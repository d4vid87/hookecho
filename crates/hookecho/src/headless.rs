//! Headless verify harness: render the radar or overlay layers to a PNG with no window.
//!
//! `--headless <out.png> [SITE] [--moment M] [--tilt N]` renders one real radar sweep.
//! `--headless-overlay <out.png>` fetches live NWS alerts + SPC Day 1 outlook and renders
//! the vector overlay over CONUS. Both use the exact pipelines the GUI uses.

use crate::overlay_build;
use crate::render::{mercator::Camera, MapCallback, OverlayUpload, RenderResources};
use crate::tiles::{BasemapStyle, TileManager};
use wxdata::level2::{self, Moment};

const SIZE: u32 = 1000;

/// `HOOKECHO_CAM=lon,lat,zoom` overrides any headless camera — framing knob for screenshots.
fn cam_or_env(lon: f64, lat: f64, zoom: f64) -> Camera {
    if let Ok(v) = std::env::var("HOOKECHO_CAM") {
        let p: Vec<f64> = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        if p.len() == 3 {
            return Camera::at_lonlat(p[0], p[1], p[2]);
        }
    }
    Camera::at_lonlat(lon, lat, zoom)
}

/// Basemap for the national-layer renders, so field mosaics sit over a real map instead of
/// the bare clear color. Default: dark vector tiles (same fetch path as `run`).
/// `HOOKECHO_BASEMAP=<slug>` switches to any raster style, e.g. `mapbox-satellite-streets`
/// (provider keys come from the saved Settings — never logged).
fn national_basemap(
    rt: &tokio::runtime::Runtime,
    camera: &Camera,
) -> (
    Vec<crate::render::PendingTile>,
    Vec<crate::render::VisibleTile>,
    Vec<crate::render::PendingVectorTile>,
    Vec<crate::render::TileId>,
) {
    let vp = (SIZE as f32, SIZE as f32);
    let client = reqwest::Client::new();
    if let Ok(slug) = std::env::var("HOOKECHO_BASEMAP") {
        let style = crate::tiles::BasemapStyle::from_slug(&slug);
        if style.is_raster() {
            let settings = crate::settings::Settings::load();
            let mut tm = TileManager::new(rt.handle().clone());
            tm.set_style(style);
            let vis = tm.visible(camera, vp);
            let tiles = rt.block_on(crate::tiles::fetch_visible(
                &client, style, &vis, &settings.mapbox_key, &settings.maptiler_key,
            ));
            println!("basemap {}: {} raster tiles", style.label(), tiles.len());
            return (tiles, vis, Vec::new(), Vec::new());
        }
    }
    let vis = crate::tiles::tile_cover(camera, vp, 14);
    let tiles = rt.block_on(async {
        let template = crate::vector_tiles::fetch_tilejson(&client, None).await?;
        Some(crate::vector_tiles::fetch_visible_vector(&client, &template, true, camera.zoom, &vis).await.0)
    });
    match tiles {
        Some(t) => {
            println!("basemap: {} vector tiles", t.len());
            (Vec::new(), Vec::new(), t, vis.iter().map(|v| v.id).collect())
        }
        None => (Vec::new(), Vec::new(), Vec::new(), Vec::new()),
    }
}

/// Render one real radar sweep for `site` to a PNG.
///
/// `pal` optionally overrides the moment's colormap with a GRLevelX `.pal` file (verifies the
/// custom color-table path end to end).
#[allow(clippy::too_many_arguments)]
pub fn run(
    out_path: &str,
    site: &str,
    moment: Moment,
    tilt: usize,
    smooth: bool,
    pal: Option<&str>,
    storm_uv: Option<(f32, f32)>,
    date: Option<chrono::NaiveDate>,
    hhmm: Option<&str>,
    basemap: BasemapStyle,
    dealias: bool,
) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;

    let table = match pal {
        Some(path) => crate::colormap::parse_pal(&std::fs::read_to_string(path)?)?,
        None => crate::colormap::default_table(moment).clone(),
    };

    let sweep = rt.block_on(async {
        // Archive mode: list a specific UTC day and pick the volume nearest `hhmm` — the exact
        // path the timeline uses when scrubbing (list_volumes -> download_scan by identifier).
        if let Some(day) = date {
            let frames = level2::list_volumes(site, day).await?;
            anyhow::ensure!(!frames.is_empty(), "no volumes for {site} on {day}");
            let target_min = hhmm.and_then(parse_hhmm);
            let id = match target_min {
                Some(tm) => frames
                    .into_iter()
                    .min_by_key(|f| {
                        let m = f.date_time().map(|d| d.time().signed_duration_since(chrono::NaiveTime::MIN).num_minutes()).unwrap_or(0);
                        (m - tm).abs()
                    })
                    .unwrap(),
                None => frames.into_iter().next_back().unwrap(),
            };
            eprintln!("archive frame: {}", id.name());
            let scan = level2::download_scan(id).await?;
            return level2::bin_scan_opts(&scan, moment, tilt, dealias);
        }

        let mut day = chrono::Utc::now().date_naive();
        for _ in 0..3 {
            match level2::download_latest_scan(site, day).await {
                Ok(scan) => return level2::bin_scan_opts(&scan, moment, tilt, dealias),
                Err(e) => {
                    eprintln!("{day}: {e}");
                    day = day.pred_opt().unwrap();
                }
            }
        }
        anyhow::bail!("no volumes for {site} in last 3 days")
    })?;
    let echo_gates = sweep.data.iter().filter(|&&v| v > 1).count();
    println!(
        "sweep: {} {:.2}deg {}x{} grid, {} echo gates, radar {:.3},{:.3}",
        site, sweep.elevation_deg, sweep.gate_count, sweep.az_bins, echo_gates,
        sweep.radar_lat, sweep.radar_lon
    );

    let camera = cam_or_env(sweep.radar_lon as f64, sweep.radar_lat as f64, 7.0);
    let (center, scale) = camera.world_to_clip_uniform((SIZE as f32, SIZE as f32));

    let vp = (SIZE as f32, SIZE as f32);
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (compatible; hookecho/0.0; +github.com/d4vid87/hookecho)")
        .build()?;

    // Basemap under the radar. Satellite + provider styles are raster; Dark/Light are vector MVT.
    // Provider keys come from the saved Settings (never logged).
    let settings = crate::settings::Settings::load();
    let is_vector = matches!(basemap, BasemapStyle::Dark | BasemapStyle::Light);
    let (new_tiles, visible) = if basemap.is_raster() {
        let mut tm = TileManager::new(rt.handle().clone());
        tm.set_style(basemap); // so the zoom cap matches this source (GOES layers top out early)
        let vis = tm.visible(&camera, vp);
        let tiles = rt.block_on(crate::tiles::fetch_visible(
            &client, basemap, &vis, &settings.mapbox_key, &settings.maptiler_key,
        ));
        println!("basemap {}: {} tiles fetched", basemap.label(), tiles.len());
        (tiles, vis)
    } else {
        (Vec::new(), Vec::new())
    };
    let (new_vector_tiles, visible_vector) = if is_vector {
        let dark = basemap == BasemapStyle::Dark;
        let tess_zoom = camera.zoom;
        let vis = crate::tiles::tile_cover(&camera, vp, 14);
        let (tiles, labels) = rt.block_on(async {
            let template = crate::vector_tiles::fetch_tilejson(&client, None)
                .await
                .ok_or_else(|| anyhow::anyhow!("no tilejson template"))?;
            println!("tilejson template: {template}");
            Ok::<_, anyhow::Error>(
                crate::vector_tiles::fetch_visible_vector(&client, &template, dark, tess_zoom, &vis).await,
            )
        })?;
        let verts: usize = tiles.iter().map(|t| t.vertices.len()).sum();
        println!(
            "vector basemap {}: {} tiles, {} verts, {} labels",
            basemap.label(),
            tiles.len(),
            verts,
            labels.len()
        );
        for l in labels.iter().take(8) {
            println!("  label: {} (rank {}, city {})", l.name, l.rank, l.city);
        }
        let ids: Vec<crate::render::TileId> = vis.iter().map(|v| v.id).collect();
        (tiles, ids)
    } else {
        (Vec::new(), Vec::new())
    };

    let cb = MapCallback {
        pane: 0,
        camera_center: center,
        camera_scale: scale,
        new_tiles,
        visible,
        radar_upload: Some(crate::app::to_upload(&sweep, &table, None, smooth, storm_uv)),
        draw_radar: true,
        overlay_upload: None,
        draw_overlay: false,
        field_uploads: Vec::new(),
        field_draws: Vec::new(),
        clear_tiles: false,
        new_vector_tiles,
        visible_vector,
        clear_vector: false,
    };
    render_to_png(&rt, cb, out_path)
}

/// Verify the multi-pane render path: prepare two panes with different cameras (pane 1 last),
/// then draw each. Proves per-pane camera state survives the all-prepare-then-paint order — the
/// core U9 correctness risk. Writes two PNGs and asserts pane 0 is unaffected by pane 1's prepare.
pub fn run_multipane(site: &str, out_a: &str, out_b: &str) -> anyhow::Result<()> {
    use crate::render::MapCallback;
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let sweep = rt.block_on(async {
        let day = chrono::Utc::now().date_naive();
        for d in 0..3 {
            let day = day.checked_sub_days(chrono::Days::new(d)).unwrap_or(day);
            if let Ok(scan) = level2::download_latest_scan(site, day).await {
                return level2::bin_scan(&scan, Moment::Reflectivity, 0);
            }
        }
        anyhow::bail!("no volume for {site}")
    })?;
    let table = crate::colormap::default_table(Moment::Reflectivity).clone();
    let vp = (SIZE as f32, SIZE as f32);

    // Pane 0 centered on the radar; pane 1 offset well to the east (different camera).
    let cam_a = Camera::at_lonlat(sweep.radar_lon as f64, sweep.radar_lat as f64, 7.0);
    let cam_b = Camera::at_lonlat(sweep.radar_lon as f64 + 2.5, sweep.radar_lat as f64, 7.0);
    let mk = |pane: u32, cam: &Camera| {
        let (center, scale) = cam.world_to_clip_uniform(vp);
        MapCallback {
            pane,
            camera_center: center,
            camera_scale: scale,
            new_tiles: Vec::new(),
            visible: Vec::new(),
            radar_upload: Some(crate::app::to_upload(&sweep, &table, None, false, None)),
            draw_radar: true,
            overlay_upload: None,
            draw_overlay: false,
            field_uploads: Vec::new(),
            field_draws: Vec::new(),
            clear_tiles: false,
            new_vector_tiles: Vec::new(),
            visible_vector: Vec::new(),
            clear_vector: false,
        }
    };

    let (device, queue, _adapter) = init_gpu(&rt)?;
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let mut res = RenderResources::new(&device, format);

    // Prepare BOTH panes (pane 1 last) before drawing either — the clobber test.
    res.prepare_pane(&device, &queue, &mk(0, &cam_a));
    res.prepare_pane(&device, &queue, &mk(1, &cam_b));

    let a = draw_and_read(&device, &queue, &res, 0);
    let b = draw_and_read(&device, &queue, &res, 1);
    save_rgba(&a, out_a)?;
    save_rgba(&b, out_b)?;

    // Reference: pane 0 alone in a fresh renderer.
    let mut res2 = RenderResources::new(&device, format);
    res2.prepare_pane(&device, &queue, &mk(0, &cam_a));
    let a_ref = draw_and_read(&device, &queue, &res2, 0);

    let identical = a == a_ref;
    let differ = a != b;
    println!("pane0 unaffected by pane1 prepare: {identical}; pane0 != pane1: {differ}");
    anyhow::ensure!(identical, "FAIL: pane 0 was clobbered by pane 1's prepare");
    anyhow::ensure!(differ, "FAIL: panes with different cameras rendered identically");
    println!("multi-pane render PASS");
    Ok(())
}

/// Fetch + decode + project Level 3 storm cells for `site` and print them (verifies the
/// bucket fetch, the from-scratch L3 decoder, and lon/lat projection windowless).
pub fn run_cells(site: &str) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let cells = rt.block_on(async {
        let http = reqwest::Client::new();
        wxdata::level3::fetch_cells(&http, site).await
    });
    println!("{site}: {} storm cells", cells.len());
    let f = |v: Option<f32>| v.map(|x| format!("{x:.1}")).unwrap_or_else(|| "—".into());
    let i = |v: Option<i32>| v.map(|x| x.to_string()).unwrap_or_else(|| "—".into());
    for c in cells.iter().take(16) {
        println!(
            "  {:<3} {:?} {:.3},{:.3}  mvt {}°/{}kt  dBZ {}@{}kft  top {} base {}{}  VIL {}  POH {}/{} hail {}in  TVS {} meso {}  err {}/{}",
            if c.id.is_empty() { "—" } else { &c.id },
            c.kind, c.lat, c.lon,
            f(c.mvt_deg), f(c.mvt_kt),
            f(c.max_dbz), f(c.max_dbz_hgt_kft),
            f(c.top_kft), if c.base_below { "<" } else { "" }, f(c.base_kft),
            f(c.vil),
            i(c.poh), i(c.posh), f(c.hail_in),
            c.tvs.as_deref().unwrap_or("—"), c.meso.as_deref().unwrap_or("—"),
            f(c.fcst_err_nm), f(c.mean_err_nm),
        );
        for tp in &c.track {
            println!("        T+{:>2}m  {:.3},{:.3}", tp.minutes, tp.lat, tp.lon);
        }
        if !c.past_track.is_empty() {
            println!("        past: {} pts", c.past_track.len());
        }
    }
    Ok(())
}

/// TDS verify: download the latest dual-pol volume, bin reflectivity + CC at the lowest tilt, run
/// the debris-signature detector, and print any hits. Proves the detection pipeline on real data.
pub fn run_tds(site: &str) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let (z, cc) = rt.block_on(async {
        let scan = level2::download_latest_scan(site, chrono::Utc::now().date_naive()).await?;
        let z = level2::bin_scan(&scan, Moment::Reflectivity, 0)?;
        let cc = level2::bin_scan(&scan, Moment::CorrelationCoefficient, 0)?;
        anyhow::Ok((z, cc))
    })?;
    println!("{site}: Z {}x{} @ {:.2}°, CC {}x{} @ {:.2}°",
        z.az_bins, z.gate_count, z.elevation_deg, cc.az_bins, cc.gate_count, cc.elevation_deg);
    let hits = wxdata::tds::detect(&z, &cc, 0.80, 40.0, 150.0, 4);
    println!("TDS clusters (CC<0.80, Z>=40 dBZ, >=4 gates): {}", hits.len());
    for h in hits.iter().take(8) {
        println!("  {:.3},{:.3}  {} gates  min CC {:.2}", h.lat, h.lon, h.gates, h.min_cc);
    }
    Ok(())
}

/// Fetch the VAD wind profile for a site and print the levels (altitude, dir/speed, u/v).
pub fn run_vwp(site: &str) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let levels = rt.block_on(async {
        let http = reqwest::Client::new();
        wxdata::level3::fetch_vwp(&http, site).await
    });
    println!("{site}: {} VAD levels", levels.len());
    for l in &levels {
        println!(
            "  {:>5.1} kft  {:>3.0}° {:>3.0} kt   u {:>6.1} v {:>6.1} m/s  rms {:.1}",
            l.alt_kft, l.dir_deg, l.speed_kt, l.u_ms, l.v_ms, l.rms_kt
        );
    }
    Ok(())
}

/// Fetch the nearest-station observations for a radar site and print latest + 24h min/max.
pub fn run_obs(site: &str) -> anyhow::Result<()> {
    let s = wxdata::sites::site_by_id(site)
        .ok_or_else(|| anyhow::anyhow!("unknown site {site}"))?;
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let station = rt.block_on(async {
        let http = reqwest::Client::new();
        wxdata::obs::fetch_nearest(&http, s.latitude as f64, s.longitude as f64).await
    })?;
    println!("{site} -> station {} ({}), {} obs", station.station_id, station.name, station.obs.len());
    if let Some(o) = station.obs.first() {
        let f = |v: Option<f32>| v.map(|x| format!("{x:.1}")).unwrap_or_else(|| "—".into());
        println!(
            "  latest {}: temp {}C dew {}C rh {}% wind {}km/h gust {} dir {} pres {}Pa",
            o.time.map(|t| t.format("%H:%MZ").to_string()).unwrap_or_default(),
            f(o.temp_c), f(o.dewpoint_c), f(o.rh), f(o.wind_kmh), f(o.gust_kmh), f(o.wind_dir_deg), f(o.pressure_pa),
        );
    }
    // 24h min/max per series.
    let minmax = |vals: Vec<f32>| {
        if vals.is_empty() {
            "—".to_string()
        } else {
            let lo = vals.iter().cloned().fold(f32::INFINITY, f32::min);
            let hi = vals.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
            format!("{lo:.1}..{hi:.1} (n={})", vals.len())
        }
    };
    let s_temp: Vec<f32> = station.obs.iter().filter_map(|o| o.temp_c).collect();
    let s_rh: Vec<f32> = station.obs.iter().filter_map(|o| o.rh).collect();
    let s_wind: Vec<f32> = station.obs.iter().filter_map(|o| o.wind_kmh).collect();
    println!("  24h temp C {}", minmax(s_temp));
    println!("  24h rh %   {}", minmax(s_rh));
    println!("  24h wind   {}", minmax(s_wind));
    Ok(())
}

/// Fetch live NWS alerts and print typed metadata for warnings/watches carrying `parameters`.
pub fn run_alerts() -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let feats = rt.block_on(async {
        let http = reqwest::Client::new();
        wxdata::alerts::fetch_active(&http).await
    })?;
    // Dedupe by alert id (MultiPolygon alerts emit one feature per part).
    let mut seen = std::collections::HashSet::new();
    let alerts: Vec<_> = feats
        .iter()
        .filter_map(|f| f.alert.as_ref())
        .filter(|a| seen.insert(a.id.clone()))
        .collect();
    println!("{} alert polygons, {} unique alerts", feats.len(), alerts.len());
    // Prefer ones with severe-weather parameters populated.
    for a in alerts.iter().filter(|a| a.max_hail_in.is_some() || a.max_wind.is_some()).take(12) {
        println!(
            "  {:<32} hail {}  wind {}  tor {}  dmg {}  expires {}",
            a.event,
            a.max_hail_in.map(|h| format!("{h:.2}in")).unwrap_or_else(|| "—".into()),
            a.max_wind.as_deref().unwrap_or("—"),
            a.tornado_detection.as_deref().unwrap_or("—"),
            a.damage_threat.as_deref().unwrap_or("—"),
            a.expires.map(|e| e.format("%H:%MZ").to_string()).unwrap_or_else(|| "—".into()),
        );
    }
    // Storm motion + escalation (feature S).
    for a in alerts.iter().filter(|a| a.motion.is_some() || wxdata::alerts::escalation(a) > 0) {
        let esc = wxdata::alerts::escalation(a);
        match &a.motion {
            Some(m) => println!("  motion: {:>3.0}° {:>2.0}kt ({} pts) esc={esc}  [{}]", m.deg, m.kt, m.points.len(), a.event),
            None => println!("  motion:    —          esc={esc}  [{}]", a.event),
        }
    }
    Ok(())
}

/// Fetch + tally the storm-based warnings archived at an instant (feature W).
pub fn run_archwarn(ts: &str) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let feats = rt.block_on(async {
        let http = reqwest::Client::new();
        wxdata::archive_warnings::fetch(&http, ts).await
    })?;
    let mut tally: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for f in &feats {
        *tally.entry(f.title.clone()).or_default() += 1;
    }
    println!("{}: {} archived warning polygons", ts, feats.len());
    for (event, n) in &tally {
        println!("  {event:<32} {n}");
    }
    Ok(())
}

/// Parse `HH:MM` into minutes-since-midnight.
fn parse_hhmm(s: &str) -> Option<i64> {
    let (h, m) = s.split_once(':')?;
    Some(h.parse::<i64>().ok()? * 60 + m.parse::<i64>().ok()?)
}

/// Wait for the first live chunk-stream update for `site` and render it to a PNG.
///
/// Verifies the full chunks -> assemble -> merge -> bin -> render path windowless.
pub fn run_live(out_path: &str, site: &str, moment: Moment) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;

    let sweep = rt.block_on(async {
        // Seed with the SECOND-newest archived volume so the in-progress live volume the
        // stream joins genuinely differs (an up-to-date base would correctly yield no update).
        let day = chrono::Utc::now().date_naive();
        let mut ids = level2::list_volumes(site, day).await?;
        let seed = ids.pop().and_then(|_| ids.pop()).or_else(|| ids.pop());
        let base = match seed {
            Some(id) => level2::download_scan(id).await?,
            None => level2::download_latest_scan(site, day).await?,
        };
        println!("base volume: {} sweeps", base.sweeps().len());

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let site_owned = site.to_string();
        let handle = tokio::spawn(async move {
            let _ = wxdata::live::stream(site_owned, base, move |u| {
                let _ = tx.send(u);
            })
            .await;
        });

        // First update should arrive within a couple minutes (backfill emits immediately).
        let update = tokio::time::timeout(std::time::Duration::from_secs(180), rx.recv())
            .await
            .map_err(|_| anyhow::anyhow!("no live update within 180s"))?
            .ok_or_else(|| anyhow::anyhow!("stream closed before first update"))?;
        handle.abort();
        println!("live update: {} ({} sweeps, {} changed tilts)", update.name, update.scan.sweeps().len(), update.changed.len());
        level2::bin_scan(&update.scan, moment, 0)
    })?;

    println!("sweep: {} {:.2}deg {}x{}", site, sweep.elevation_deg, sweep.gate_count, sweep.az_bins);
    let camera = Camera::at_lonlat(sweep.radar_lon as f64, sweep.radar_lat as f64, 7.0);
    let (center, scale) = camera.world_to_clip_uniform((SIZE as f32, SIZE as f32));
    let table = crate::colormap::default_table(moment).clone();
    let cb = MapCallback {
        pane: 0,
        camera_center: center,
        camera_scale: scale,
        new_tiles: Vec::new(),
        visible: Vec::new(),
        radar_upload: Some(crate::app::to_upload(&sweep, &table, None, false, None)),
        draw_radar: true,
        overlay_upload: None,
        draw_overlay: false,
        field_uploads: Vec::new(),
        field_draws: Vec::new(),
        clear_tiles: false,
        new_vector_tiles: Vec::new(),
        visible_vector: Vec::new(),
        clear_vector: false,
    };
    render_to_png(&rt, cb, out_path)
}

/// Parse a local GRLevelX placefile, tessellate its lines/polygons, and render them centered on
/// their bounding box to a PNG (verifies the parser + overlay tessellation windowless).
pub fn run_placefile(path: &str, out_path: &str) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let text = std::fs::read_to_string(path)?;
    let pf = wxdata::placefile::parse(&text);
    println!("placefile '{}': {} items, refresh {}s", pf.title, pf.items.len(), pf.refresh_secs);

    // Center on the mean of all vertex coordinates.
    use wxdata::placefile::PlaceKind;
    let mut sum = [0.0f64; 2];
    let mut n = 0u32;
    let mut acc = |lon: f64, lat: f64| {
        sum[0] += lon;
        sum[1] += lat;
        n += 1;
    };
    for it in &pf.items {
        match &it.kind {
            PlaceKind::Line { pts, .. } => pts.iter().for_each(|p| acc(p[0], p[1])),
            PlaceKind::Polygon { rings, .. } => {
                rings.iter().flatten().for_each(|p| acc(p[0], p[1]))
            }
            PlaceKind::Text { pos, .. } | PlaceKind::Icon { pos, .. } => acc(pos[0], pos[1]),
        }
    }
    anyhow::ensure!(n > 0, "placefile has no coordinates");
    let (clon, clat) = (sum[0] / n as f64, sum[1] / n as f64);

    let zoom = 8.0;
    let camera = Camera::at_lonlat(clon, clat, zoom);
    let items: Vec<&wxdata::placefile::PlaceItem> = pf.items.iter().collect();
    let mut geom = overlay_build::OverlayGeom::default();
    overlay_build::append_placefiles(&mut geom, &items, zoom);
    println!("tessellated {} verts / {} indices", geom.vertices.len(), geom.indices.len());

    let (center, scale) = camera.world_to_clip_uniform((SIZE as f32, SIZE as f32));
    let cb = MapCallback {
        pane: 0,
        camera_center: center,
        camera_scale: scale,
        new_tiles: Vec::new(),
        visible: Vec::new(),
        radar_upload: None,
        draw_radar: false,
        overlay_upload: Some(OverlayUpload { vertices: geom.vertices, indices: geom.indices }),
        draw_overlay: true,
        field_uploads: Vec::new(),
        field_draws: Vec::new(),
        clear_tiles: false,
        new_vector_tiles: Vec::new(),
        visible_vector: Vec::new(),
        clear_vector: false,
    };
    render_to_png(&rt, cb, out_path)
}

/// Fetch live severe-weather overlays and render them over CONUS to a PNG.
pub fn run_overlay(out_path: &str) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;

    let (alerts, outlook) = rt.block_on(async {
        let client = reqwest::Client::new();
        let alerts = wxdata::alerts::fetch_active(&client).await.unwrap_or_default();
        let outlook = wxdata::spc::fetch_outlook(&client, 1).await.unwrap_or_default();
        (alerts, outlook)
    });
    let mut features = outlook;
    features.extend(alerts);
    println!("overlay features: {}", features.len());

    let zoom = 4.0;
    let camera = cam_or_env(-97.0, 38.0, zoom); // CONUS center
    let (new_tiles, visible, new_vector_tiles, visible_vector) = national_basemap(&rt, &camera);
    let geom = overlay_build::build(&features, zoom);
    println!("tessellated {} verts / {} indices", geom.vertices.len(), geom.indices.len());

    let (center, scale) = camera.world_to_clip_uniform((SIZE as f32, SIZE as f32));
    let cb = MapCallback {
        pane: 0,
        camera_center: center,
        camera_scale: scale,
        new_tiles,
        visible,
        radar_upload: None,
        draw_radar: false,
        overlay_upload: Some(OverlayUpload { vertices: geom.vertices, indices: geom.indices }),
        draw_overlay: true,
        field_uploads: Vec::new(),
        field_draws: Vec::new(),
        clear_tiles: false,
        new_vector_tiles,
        visible_vector,
        clear_vector: false,
    };
    render_to_png(&rt, cb, out_path)
}

/// Fetch the latest MRMS national mosaic and render it over CONUS.
pub fn run_mrms(out_path: &str) -> anyhow::Result<()> {
    use crate::render::{mercator::lonlat_to_world, MrmsUpload};
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;

    let field = rt.block_on(async {
        let client = reqwest::Client::new();
        wxdata::mrms::fetch_latest(&client, wxdata::mrms::REFLECTIVITY).await
    })?;
    println!(
        "mrms grid {}x{}  lon [{:.2},{:.2}]  lat [{:.2},{:.2}]  time {}",
        field.nx, field.ny, field.lon_west, field.lon_east, field.lat_south, field.lat_north, field.time
    );
    let valid = field.values.iter().filter(|v| !v.is_nan()).count();
    let vmax = field.values.iter().cloned().filter(|v| !v.is_nan()).fold(f32::MIN, f32::max);
    println!("valid gates: {valid}  max dBZ: {vmax:.1}");

    let (vmin, vspan_max) = Moment::Reflectivity.value_range();
    let span = (vspan_max - vmin).max(f32::EPSILON);
    let data: Vec<u8> = field
        .values
        .iter()
        .map(|&v| {
            if v.is_nan() {
                0
            } else {
                (2.0 + ((v - vmin) / span).clamp(0.0, 1.0) * 253.0) as u8
            }
        })
        .collect();
    let table = crate::colormap::default_table(Moment::Reflectivity);
    let (wx0, wy0) = lonlat_to_world(field.lon_west, field.lat_north);
    let (wx1, wy1) = lonlat_to_world(field.lon_east, field.lat_south);
    let upload = MrmsUpload {
        data,
        nx: field.nx as u32,
        ny: field.ny as u32,
        world_min: [wx0 as f32, wy0 as f32],
        world_max: [wx1 as f32, wy1 as f32],
        uniform: [
            field.lon_west as f32, field.lat_north as f32, field.lon_east as f32, field.lat_south as f32,
            field.nx as f32, field.ny as f32, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        ],
        lut: crate::colormap::bake_lut(table, (vmin, vspan_max), None).to_vec(),
    };

    let camera = cam_or_env(-97.0, 38.0, 4.0);
    let (new_tiles, visible, new_vector_tiles, visible_vector) = national_basemap(&rt, &camera);
    let (center, scale) = camera.world_to_clip_uniform((SIZE as f32, SIZE as f32));
    let cb = MapCallback {
        pane: 0,
        camera_center: center,
        camera_scale: scale,
        new_tiles,
        visible,
        radar_upload: None,
        draw_radar: false,
        overlay_upload: None,
        draw_overlay: false,
        field_uploads: vec![(crate::render::FieldLayer::Mrms, upload)],
        field_draws: vec![crate::render::FieldLayer::Mrms],
        clear_tiles: false,
        new_vector_tiles,
        visible_vector,
        clear_vector: false,
    };
    render_to_png(&rt, cb, out_path)
}

/// Fetch the latest MRMS lightning-density mosaic, print stats, and render it over CONUS.
pub fn run_lightning(out_path: &str) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let field = rt.block_on(async {
        let client = reqwest::Client::new();
        wxdata::mrms::fetch_latest(&client, wxdata::mrms::LIGHTNING).await
    })?;
    let nonzero = field.values.iter().filter(|v| !v.is_nan() && **v > 0.0).count();
    let vmax = field.values.iter().cloned().filter(|v| !v.is_nan()).fold(f32::MIN, f32::max);
    println!(
        "lightning grid {}x{}  nonzero cells: {}  max density: {:.3} strikes/km2/min  time {}",
        field.nx, field.ny, nonzero, vmax, field.time
    );

    let upload = crate::app::lightning_upload(&field);
    let camera = cam_or_env(-97.0, 38.0, 4.0);
    let (new_tiles, visible, new_vector_tiles, visible_vector) = national_basemap(&rt, &camera);
    let (center, scale) = camera.world_to_clip_uniform((SIZE as f32, SIZE as f32));
    let cb = MapCallback {
        pane: 0,
        camera_center: center,
        camera_scale: scale,
        new_tiles,
        visible,
        radar_upload: None,
        draw_radar: false,
        overlay_upload: None,
        draw_overlay: false,
        field_uploads: vec![(crate::render::FieldLayer::Lightning, upload)],
        field_draws: vec![crate::render::FieldLayer::Lightning],
        clear_tiles: false,
        new_vector_tiles,
        visible_vector,
        clear_vector: false,
    };
    render_to_png(&rt, cb, out_path)
}

/// Fetch + render one index-mapped national field layer (rotation / MESH / AzShear) over CONUS,
/// printing grid stats. `slug` = rotation30|rotation60|rotation120|mesh|azshear.
pub fn run_field(slug: &str, out_path: &str) -> anyhow::Result<()> {
    use crate::render::FieldLayer as FL;
    let (product, layer): (String, FL) = match slug {
        "rotation30" => (wxdata::mrms::rotation_track(30).to_string(), FL::Rotation),
        "rotation60" => (wxdata::mrms::rotation_track(60).to_string(), FL::Rotation),
        "rotation120" => (wxdata::mrms::rotation_track(120).to_string(), FL::Rotation),
        "mesh" => (wxdata::mrms::MESH.to_string(), FL::Mesh),
        "azshear" => (wxdata::mrms::AZSHEAR.to_string(), FL::AzShear),
        "qpe1h" => (wxdata::mrms::QPE_01H.to_string(), FL::Qpe1h),
        "qpe24h" => (wxdata::mrms::QPE_24H.to_string(), FL::Qpe24h),
        "preciptype" => (wxdata::mrms::PRECIP_TYPE.to_string(), FL::PrecipType),
        "flashflood" => (wxdata::mrms::FLASH_ARI30.to_string(), FL::FlashFlood),
        "hailswath" => (wxdata::mrms::MESH_1440.to_string(), FL::HailSwath),
        other => anyhow::bail!("unknown field slug '{other}' (rotation30|rotation60|rotation120|mesh|azshear|qpe1h|qpe24h|preciptype|flashflood|hailswath)"),
    };
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let field = rt.block_on(async {
        let client = reqwest::Client::new();
        wxdata::mrms::fetch_latest(&client, &product).await
    })?;
    let nonzero = field.values.iter().filter(|v| !v.is_nan() && v.abs() > 0.0).count();
    let vmax = field.values.iter().cloned().filter(|v| !v.is_nan()).fold(f32::MIN, f32::max);
    println!("{slug} grid {}x{}  nonzero: {}  max: {:.4}  time {}", field.nx, field.ny, nonzero, vmax, field.time);

    let field = field.decimated(8192); // fit oversized (14000×7000) rotation/AzShear grids
    let upload = crate::app::field_upload_indexed(layer, &field);
    let camera = cam_or_env(-97.0, 38.0, 4.0);
    let (new_tiles, visible, new_vector_tiles, visible_vector) = national_basemap(&rt, &camera);
    let (center, scale) = camera.world_to_clip_uniform((SIZE as f32, SIZE as f32));
    let cb = MapCallback {
        pane: 0,
        camera_center: center,
        camera_scale: scale,
        new_tiles,
        visible,
        radar_upload: None,
        draw_radar: false,
        overlay_upload: None,
        draw_overlay: false,
        field_uploads: vec![(layer, upload)],
        field_draws: vec![layer],
        clear_tiles: false,
        new_vector_tiles,
        visible_vector,
        clear_vector: false,
    };
    render_to_png(&rt, cb, out_path)
}

/// Fetch + print the active NHC tropical cyclones (feature V). Exits 0 with a note when none.
pub fn run_tropical() -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let data = rt.block_on(async {
        let client = reqwest::Client::new();
        wxdata::tropical::fetch_active(&client).await
    })?;
    if data.storms.is_empty() {
        println!("no active tropical storms");
        return Ok(());
    }
    println!("{} active storm(s), {} cone polygon(s)", data.storms.len(), data.cones.len());
    for s in &data.storms {
        let (cat, _) = wxdata::tropical::saffir_simpson(s.intensity_kt);
        let cone_verts: usize = data.cones.iter().flat_map(|c| c.rings.iter().map(|r| r.len())).sum();
        println!(
            "  {} ({}) {} — {:.0} kt {} at {:.1},{:.1}  {} track pts, {} cone verts",
            s.name, s.id, s.classification, s.intensity_kt, cat, s.lat, s.lon, s.points.len(), cone_verts
        );
    }
    Ok(())
}

/// Fetch + print surface obs (METAR) near a site (feature U).
pub fn run_metar(site: &str) -> anyhow::Result<()> {
    let s = wxdata::sites::site_by_id(site).ok_or_else(|| anyhow::anyhow!("unknown site {site}"))?;
    let (lat, lon) = (s.latitude as f64, s.longitude as f64);
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let obs = rt.block_on(async {
        let client = reqwest::Client::new();
        wxdata::metar::fetch_bbox(&client, lat - 2.5, lon - 2.5, lat + 2.5, lon + 2.5).await
    })?;
    println!("{site}: {} surface obs within ±2.5°", obs.len());
    for ob in obs.iter().take(3) {
        println!(
            "  {:<5} {:>6.2},{:>7.2}  {}kt @ {}  T {} Td {}  [{}]",
            ob.icao, ob.lat, ob.lon, ob.wspd_kt,
            ob.wdir_deg.map(|d| format!("{d:.0}")).unwrap_or_else(|| "VRB".into()),
            ob.temp_c.map(|t| format!("{t:.0}C")).unwrap_or_else(|| "—".into()),
            ob.dewp_c.map(|t| format!("{t:.0}C")).unwrap_or_else(|| "—".into()),
            ob.flt_cat,
        );
    }
    Ok(())
}

/// Fetch a gridded L3 product (DVL/EET), print stats, render centered on the site (feature X).
pub fn run_l3grid(kind: &str, site: &str, out_path: &str) -> anyhow::Result<()> {
    use crate::render::FieldLayer as FL;
    let layer = match kind {
        "dvl" => FL::Vil,
        "eet" => FL::EchoTops,
        "hhc" => FL::Hca,
        other => anyhow::bail!("unknown l3grid kind '{other}' (dvl|eet|hhc)"),
    };
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let field = rt.block_on(async {
        let client = reqwest::Client::new();
        match layer {
            FL::Vil => wxdata::level3::fetch_dvl(&client, site).await,
            FL::Hca => wxdata::level3::fetch_hhc(&client, site).await,
            _ => wxdata::level3::fetch_eet(&client, site).await,
        }
    });
    let field = field.ok_or_else(|| anyhow::anyhow!("no {kind} grid for {site}"))?;
    let filled = field.values.iter().filter(|v| !v.is_nan()).count();
    let vmax = field.values.iter().cloned().filter(|v| !v.is_nan()).fold(f32::MIN, f32::max);
    println!(
        "{kind} {site} grid {}x{}  lon[{:.2},{:.2}] lat[{:.2},{:.2}]  filled {}  max {:.2}",
        field.nx, field.ny, field.lon_west, field.lon_east, field.lat_south, field.lat_north, filled, vmax
    );
    let (clon, clat) = ((field.lon_west + field.lon_east) * 0.5, (field.lat_north + field.lat_south) * 0.5);
    let upload = crate::app::field_upload_indexed(layer, &field);
    let camera = cam_or_env(clon, clat, 7.0);
    let (new_tiles, visible, new_vector_tiles, visible_vector) = national_basemap(&rt, &camera);
    let (center, scale) = camera.world_to_clip_uniform((SIZE as f32, SIZE as f32));
    let cb = MapCallback {
        pane: 0,
        camera_center: center,
        camera_scale: scale,
        new_tiles,
        visible,
        radar_upload: None,
        draw_radar: false,
        overlay_upload: None,
        draw_overlay: false,
        field_uploads: vec![(layer, upload)],
        field_draws: vec![layer],
        clear_tiles: false,
        new_vector_tiles,
        visible_vector,
        clear_vector: false,
    };
    render_to_png(&rt, cb, out_path)
}

/// Fetch + regrid an HRRR environment field (CAPE/SRH), print stats, render over CONUS (feature T).
pub fn run_env(slug: &str, out_path: &str) -> anyhow::Result<()> {
    use crate::render::FieldLayer as FL;
    let (var, level, min_valid, layer): (&str, &str, f64, FL) = match slug {
        "sbcape" => ("CAPE", "surface", 0.0, FL::Cape),
        "mlcape" => ("CAPE", "90-0 mb above ground", 0.0, FL::Cape),
        "srh1" => ("HLCY", "1000-0 m above ground", f64::NEG_INFINITY, FL::Srh),
        "srh3" => ("HLCY", "3000-0 m above ground", f64::NEG_INFINITY, FL::Srh),
        other => anyhow::bail!("unknown env slug '{other}' (sbcape|mlcape|srh1|srh3)"),
    };
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let fc = rt.block_on(async {
        let client = reqwest::Client::new();
        wxdata::hrrr::fetch_field(&client, var, level, 0, min_valid).await
    })?;
    let f = &fc.field;
    let filled = f.values.iter().filter(|v| !v.is_nan()).count();
    let vmax = f.values.iter().cloned().filter(|v| !v.is_nan()).fold(f32::MIN, f32::max);
    let vmin = f.values.iter().cloned().filter(|v| !v.is_nan()).fold(f32::MAX, f32::min);
    println!(
        "{slug} ({var}:{level}) regrid {}x{}  filled {}  range [{:.1},{:.1}]  run {} valid {}",
        f.nx, f.ny, filled, vmin, vmax, fc.run.format("%Y-%m-%d %HZ"), fc.valid().format("%Y-%m-%d %H:%MZ")
    );

    let upload = crate::app::field_upload_indexed(layer, f);
    let camera = cam_or_env(-97.0, 38.0, 4.0);
    let (new_tiles, visible, new_vector_tiles, visible_vector) = national_basemap(&rt, &camera);
    let (center, scale) = camera.world_to_clip_uniform((SIZE as f32, SIZE as f32));
    let cb = MapCallback {
        pane: 0,
        camera_center: center,
        camera_scale: scale,
        new_tiles,
        visible,
        radar_upload: None,
        draw_radar: false,
        overlay_upload: None,
        draw_overlay: false,
        field_uploads: vec![(layer, upload)],
        field_draws: vec![layer],
        clear_tiles: false,
        new_vector_tiles,
        visible_vector,
        clear_vector: false,
    };
    render_to_png(&rt, cb, out_path)
}

/// Fetch + regrid an HRRR reflectivity forecast for `fcst_hour`, print stats, render over CONUS.
pub fn run_hrrr(fcst_hour: u8, out_path: &str) -> anyhow::Result<()> {
    use crate::render::{mercator::lonlat_to_world, FieldLayer, MrmsUpload};
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let fc = rt.block_on(async {
        let client = reqwest::Client::new();
        wxdata::hrrr::fetch_forecast(&client, fcst_hour).await
    })?;
    let f = &fc.field;
    let valid = f.values.iter().filter(|v| !v.is_nan()).count();
    let vmax = f.values.iter().cloned().filter(|v| !v.is_nan()).fold(f32::MIN, f32::max);
    println!(
        "HRRR F+{}h regrid {}x{}  lon[{:.1},{:.1}] lat[{:.1},{:.1}]  filled {}  max {:.1} dBZ  run {} valid {}",
        fc.fcst_hour, f.nx, f.ny, f.lon_west, f.lon_east, f.lat_south, f.lat_north, valid, vmax,
        fc.run.format("%Y-%m-%d %HZ"), fc.valid().format("%Y-%m-%d %H:%MZ")
    );

    // Reflectivity index mapping (mirrors app::mrms_upload) with the default REF palette.
    let (vmin, vspan_max) = Moment::Reflectivity.value_range();
    let span = (vspan_max - vmin).max(f32::EPSILON);
    let data: Vec<u8> = f.values.iter().map(|&v| {
        if v.is_nan() { 0 } else { (2.0 + ((v - vmin) / span).clamp(0.0, 1.0) * 253.0) as u8 }
    }).collect();
    let table = crate::colormap::default_table(Moment::Reflectivity);
    let (wx0, wy0) = lonlat_to_world(f.lon_west, f.lat_north);
    let (wx1, wy1) = lonlat_to_world(f.lon_east, f.lat_south);
    let upload = MrmsUpload {
        data,
        nx: f.nx as u32,
        ny: f.ny as u32,
        world_min: [wx0 as f32, wy0 as f32],
        world_max: [wx1 as f32, wy1 as f32],
        uniform: [
            f.lon_west as f32, f.lat_north as f32, f.lon_east as f32, f.lat_south as f32,
            f.nx as f32, f.ny as f32, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        ],
        lut: crate::colormap::bake_lut(table, (vmin, vspan_max), None).to_vec(),
    };
    let camera = cam_or_env(-97.0, 38.0, 4.0);
    let (new_tiles, visible, new_vector_tiles, visible_vector) = national_basemap(&rt, &camera);
    let (center, scale) = camera.world_to_clip_uniform((SIZE as f32, SIZE as f32));
    let cb = MapCallback {
        pane: 0,
        camera_center: center,
        camera_scale: scale,
        new_tiles,
        visible,
        radar_upload: None,
        draw_radar: false,
        overlay_upload: None,
        draw_overlay: false,
        field_uploads: vec![(FieldLayer::Hrrr, upload)],
        field_draws: vec![FieldLayer::Hrrr],
        clear_tiles: false,
        new_vector_tiles,
        visible_vector,
        clear_vector: false,
    };
    render_to_png(&rt, cb, out_path)
}

/// Reconstruct a vertical cross-section for `site` along `a`→`b` (`(lon,lat)`) and save the panel
/// PNG. Prints coverage stats.
pub fn run_xsection(site: &str, a: (f64, f64), b: (f64, f64), out_path: &str) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let scan = rt.block_on(async {
        let mut day = chrono::Utc::now().date_naive();
        for _ in 0..3 {
            match level2::download_latest_scan(site, day).await {
                Ok(s) => return Ok(s),
                Err(_) => day = day.pred_opt().unwrap(),
            }
        }
        anyhow::bail!("no volume for {site}")
    })?;

    // Bin every reflectivity tilt, then reconstruct the section.
    let elevs = level2::elevation_angles(&scan);
    let sweeps: Vec<_> = (0..elevs.len())
        .filter_map(|t| level2::bin_scan_opts(&scan, Moment::Reflectivity, t, false).ok())
        .collect();
    let xs = wxdata::xsection::build(&sweeps, a, b, 300, 120, 18.0)
        .ok_or_else(|| anyhow::anyhow!("no sweeps to build cross-section"))?;
    let filled = xs.dbz.iter().filter(|c| c.is_some()).count();
    let vmax = xs.dbz.iter().flatten().cloned().fold(f32::MIN, f32::max);
    println!(
        "cross-section {} tilts, {}x{} panel, length {:.0} km, filled {}/{}, max {:.1} dBZ",
        sweeps.len(), xs.cols, xs.rows, xs.length_km, filled, xs.cols * xs.rows, vmax
    );

    let table = crate::colormap::default_table(Moment::Reflectivity);
    let img = crate::ui::xsection_window::to_image(&xs, table);
    let buf: Vec<u8> = img.pixels.iter().flat_map(|p| [p.r(), p.g(), p.b(), p.a()]).collect();
    image::save_buffer(out_path, &buf, xs.cols as u32, xs.rows as u32, image::ColorType::Rgba8)?;
    println!("wrote {out_path}");
    Ok(())
}

/// Build the 3D reflectivity volume for `site` and raymarch it from a fixed orbit camera to a PNG.
/// Fetch a volume, slice a CAPPI at `alt_km`, print filled-cell count, and save the slice PNG.
pub fn run_cappi(site: &str, alt_km: f32, out_path: &str) -> anyhow::Result<()> {
    const N: usize = 256;
    const HALF_KM: f32 = 150.0;
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let scan = rt.block_on(async {
        let mut day = chrono::Utc::now().date_naive();
        for _ in 0..3 {
            match level2::download_latest_scan(site, day).await {
                Ok(s) => return Ok(s),
                Err(_) => day = day.pred_opt().unwrap(),
            }
        }
        anyhow::bail!("no volume for {site}")
    })?;
    let elevs = level2::elevation_angles(&scan);
    let sweeps: Vec<_> = (0..elevs.len())
        .filter_map(|t| level2::bin_scan_opts(&scan, Moment::Reflectivity, t, false).ok())
        .collect();
    let c = wxdata::volume3d::cappi(&sweeps, alt_km, N, HALF_KM)
        .ok_or_else(|| anyhow::anyhow!("no sweeps for CAPPI"))?;
    let filled = c.dbz.iter().filter(|v| v.is_some()).count();
    println!("CAPPI {site} @ {alt_km:.1} km  {N}x{N}  filled {}/{}", filled, c.dbz.len());
    let table = crate::colormap::default_table(Moment::Reflectivity);
    let img = crate::ui::cappi_window::to_image(&c, table);
    let rgba: Vec<u8> = img.pixels.iter().flat_map(|p| [p.r(), p.g(), p.b(), p.a()]).collect();
    image::save_buffer(out_path, &rgba, N as u32, N as u32, image::ColorType::Rgba8)?;
    println!("wrote {out_path}");
    Ok(())
}

pub fn run_3d(site: &str, out_path: &str) -> anyhow::Result<()> {
    const N: usize = 192;
    const NZ: usize = 48;
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let scan = rt.block_on(async {
        let mut day = chrono::Utc::now().date_naive();
        for _ in 0..3 {
            match level2::download_latest_scan(site, day).await {
                Ok(s) => return Ok(s),
                Err(_) => day = day.pred_opt().unwrap(),
            }
        }
        anyhow::bail!("no volume for {site}")
    })?;

    let elevs = level2::elevation_angles(&scan);
    let sweeps: Vec<_> = (0..elevs.len())
        .filter_map(|t| level2::bin_scan_opts(&scan, Moment::Reflectivity, t, false).ok())
        .collect();
    let v3 = wxdata::volume3d::build(&sweeps, N, NZ, 150.0, 18.0)
        .ok_or_else(|| anyhow::anyhow!("no sweeps for 3D volume"))?;
    let filled = v3.data.iter().filter(|&&b| b >= 2).count();
    println!(
        "3D volume {} tilts, {}x{}x{}, filled voxels {}/{}",
        sweeps.len(), v3.n, v3.n, v3.nz, filled, v3.data.len()
    );

    let table = crate::colormap::default_table(Moment::Reflectivity);
    let lut = crate::colormap::bake_lut(table, (v3.value_min, v3.value_max), None).to_vec();
    let upload = crate::render3d::Volume3dUpload { data: v3.data, n: v3.n as u32, nz: v3.nz as u32, lut };
    let uniform = crate::render3d::orbit_uniform(30.0, 25.0, 3.0, 1.0, N as u32, NZ as u32, 256);

    let (device, queue, adapter) = init_gpu(&rt)?;
    println!("adapter: {}", adapter.get_info().name);
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let mut res = crate::render3d::Volume3dResources::new(&device, format);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("headless_3d_target"),
        size: wgpu::Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    res.render_once(&device, &queue, &view, &upload, uniform, wgpu::Color { r: 0.03, g: 0.03, b: 0.05, a: 1.0 });

    let rgba = read_target(&device, &queue, &target);
    image::save_buffer(out_path, &rgba, SIZE, SIZE, image::ColorType::Rgba8)?;
    // Echo pixels = those differing from the uniform sRGB background clear (~48,48,63).
    let echo = rgba
        .chunks_exact(4)
        .filter(|p| (p[0] as i16 - 48).abs() + (p[1] as i16 - 48).abs() + (p[2] as i16 - 63).abs() > 30)
        .count();
    println!("wrote {out_path}  ({echo} echo pixels over background)");
    Ok(())
}

/// Fetch + print today's SPC storm reports (textual gate; markers are painter-drawn).
pub fn run_reports(window: Option<(&str, &str)>) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let reports = rt.block_on(async {
        let client = reqwest::Client::new();
        wxdata::lsr::fetch(&client, window).await
    })?;
    use wxdata::spc::ReportKind;
    let (mut t, mut w, mut h, mut f, mut o) = (0, 0, 0, 0, 0);
    for r in &reports {
        match r.kind {
            ReportKind::Tornado => t += 1,
            ReportKind::Wind => w += 1,
            ReportKind::Hail => h += 1,
            ReportKind::Flood => f += 1,
            ReportKind::Other => o += 1,
        }
    }
    let span = window.map(|(a, b)| format!("{a}..{b}")).unwrap_or_else(|| "last 6 h".into());
    println!(
        "LSRs ({span}): {} total ({t} tornado, {w} wind, {h} hail, {f} flood, {o} other)",
        reports.len()
    );
    for r in reports.iter().take(5) {
        println!("  {} {} @ {:.2},{:.2} — {} {}", r.kind.label(), r.magnitude, r.lat, r.lon, r.location, r.state);
    }
    Ok(())
}

/// AFD verify: fetch + print the head of the active-site WFO discussion (feature DD).
pub fn run_afd(site: &str) -> anyhow::Result<()> {
    let s = wxdata::sites::site_by_id(site).ok_or_else(|| anyhow::anyhow!("unknown site {site}"))?;
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let afd = rt.block_on(async {
        let client = reqwest::Client::new();
        wxdata::afd::fetch(&client, s.latitude as f64, s.longitude as f64).await
    })?;
    println!("AFD {} issued {} — {} chars", afd.office, afd.issued, afd.text.len());
    for line in afd.text.lines().take(12) {
        println!("  {line}");
    }
    Ok(())
}

/// Aviation verify: fetch SIGMETs/AIRMETs, print per-hazard tallies (feature GG).
pub fn run_aviation() -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let feats = rt.block_on(async {
        let client = reqwest::Client::new();
        wxdata::aviation::fetch_airsigmet(&client).await
    })?;
    let mut tally: std::collections::BTreeMap<String, usize> = Default::default();
    for f in &feats {
        *tally.entry(f.title.clone()).or_default() += 1;
    }
    println!("Aviation hazards: {} polygons", feats.len());
    for (k, n) in tally {
        println!("  {n}× {k}");
    }
    Ok(())
}

/// Sounding-indices verify: fetch an HRRR profile and print the composites (feature FF).
pub fn run_indices(lon: f64, lat: f64) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let s = rt.block_on(async {
        let client = reqwest::Client::new();
        wxdata::sounding::fetch(&client, lon, lat).await
    })?;
    let ix = s.indices().ok_or_else(|| anyhow::anyhow!("profile too short for indices"))?;
    println!(
        "indices @ {lat:.2},{lon:.2} (run {}): SBCAPE {:.0} J/kg  LCL {:.0} m  SRH1 {:.0}  SRH3 {:.0}  shear6 {:.0} kt  SCP {:.1}  STP {:.1}  EHI1 {:.1}",
        s.run.format("%m/%d %H:%MZ"), ix.sbcape, ix.lcl_m, ix.srh1, ix.srh3, ix.shear6_kt, ix.scp, ix.stp, ix.ehi1
    );
    Ok(())
}

/// Tornado-climatology verify: download the SPC database, query near a point, print counts.
pub fn run_climatology(lon: f64, lat: f64) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let tracks = rt.block_on(async {
        let client = reqwest::Client::new();
        wxdata::torclimo::fetch_tracks(&client).await
    })?;
    println!("Loaded {} tornado tracks (1950–2022)", tracks.len());
    let hits = wxdata::torclimo::near(&tracks, lon, lat, 40.0);
    let hist = wxdata::torclimo::mag_histogram(&hits);
    println!("Within 25 mi of {lat:.3},{lon:.3}: {} tornadoes", hits.len());
    println!("  EF0:{} EF1:{} EF2:{} EF3:{} EF4:{} EF5:{} Unk:{}",
        hist[0], hist[1], hist[2], hist[3], hist[4], hist[5], hist[6]);
    for t in hits.iter().take(5) {
        let m = if t.mag < 0 { "EF?".to_string() } else { format!("EF{}", t.mag) };
        println!("  {} {} @ {:.2},{:.2}", t.year, m, t.slat, t.slon);
    }
    Ok(())
}

/// ProbSevere verify: fetch the latest FeatureCollection, print storm count + top probabilities.
pub fn run_probsevere() -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let feats = rt.block_on(async {
        let client = reqwest::Client::new();
        wxdata::probsevere::fetch_probsevere(&client).await
    })?;
    println!("ProbSevere storms: {}", feats.len());
    let mut sorted: Vec<_> = feats.iter().collect();
    sorted.sort_by_key(|f| std::cmp::Reverse(f.title.trim_end_matches('%').rsplit(' ').next().and_then(|s| s.parse::<u8>().ok()).unwrap_or(0)));
    for f in sorted.iter().take(5) {
        let c = f.rings.first().and_then(|r| r.first()).copied().unwrap_or([0.0, 0.0]);
        println!("  {} @ {:.2},{:.2}", f.title, c[1], c[0]);
    }
    Ok(())
}

/// Spotter Network verify: fetch, then apply the same 230 km site filter the map painter uses.
pub fn run_spotters(site: &str) -> anyhow::Result<()> {
    let s = wxdata::sites::site_by_id(site)
        .ok_or_else(|| anyhow::anyhow!("unknown site {site}"))?;
    let site_pos = [s.longitude as f64, s.latitude as f64];
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let spotters = rt.block_on(async {
        let client = reqwest::Client::new();
        wxdata::spotters::fetch_spotters(&client).await
    })?;
    let now = chrono::Utc::now();
    let mut near = 0;
    let mut movers = 0;
    let mut printed = 0;
    for sp in &spotters {
        if crate::geo::great_circle(site_pos, [sp.lon, sp.lat]).0 > 230.0 {
            continue;
        }
        near += 1;
        if sp.heading.is_some() {
            movers += 1;
        }
        if printed < 5 {
            let age = (now - sp.time).num_minutes();
            println!(
                "  {} @ {:.2},{:.2} — {} ({age} min ago){}",
                sp.name, sp.lat, sp.lon, sp.status,
                sp.heading.map(|h| format!(", heading {h:.0}°")).unwrap_or_default(),
            );
            printed += 1;
        }
    }
    println!(
        "Spotter Network: {} total, {near} within 230 km of {site} ({movers} moving)",
        spotters.len()
    );
    anyhow::ensure!(!format!("{spotters:?}").contains('@'), "email leaked into parsed spotters");
    Ok(())
}

/// Create a headless GPU device/queue.
fn init_gpu(rt: &tokio::runtime::Runtime) -> anyhow::Result<(wgpu::Device, wgpu::Queue, wgpu::Adapter)> {
    let instance = wgpu::Instance::default();
    let adapter = rt.block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: None,
    }))?;
    let (device, queue) = rt.block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))?;
    Ok((device, queue, adapter))
}

/// Read a `SIZE×SIZE` RGBA render target back to a tightly-packed byte vec.
fn read_target(device: &wgpu::Device, queue: &wgpu::Queue, target: &wgpu::Texture) -> Vec<u8> {
    let bytes_per_pixel = 4u32;
    let unpadded = SIZE * bytes_per_pixel;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded = unpadded.div_ceil(align) * align;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (padded * SIZE) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(SIZE) },
        },
        wgpu::Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: 1 },
    );
    queue.submit(Some(encoder.finish()));
    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(r); });
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    let _ = rx.recv();
    let mapped = slice.get_mapped_range();
    let mut rgba = Vec::with_capacity((unpadded * SIZE) as usize);
    for row in 0..SIZE {
        let start = (row * padded) as usize;
        rgba.extend_from_slice(&mapped[start..start + unpadded as usize]);
    }
    drop(mapped);
    buffer.unmap();
    rgba
}

fn new_target(device: &wgpu::Device, format: wgpu::TextureFormat) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("headless_target"),
        size: wgpu::Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

/// Draw one prepared pane to a fresh target and read it back.
fn draw_and_read(device: &wgpu::Device, queue: &wgpu::Queue, res: &RenderResources, pane: u32) -> Vec<u8> {
    let target = new_target(device, wgpu::TextureFormat::Rgba8UnormSrgb);
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    res.draw_pane(device, queue, &view, pane, wgpu::Color { r: 0.05, g: 0.05, b: 0.08, a: 1.0 });
    read_target(device, queue, &target)
}

fn save_rgba(rgba: &[u8], out_path: &str) -> anyhow::Result<()> {
    image::save_buffer(out_path, rgba, SIZE, SIZE, image::ColorType::Rgba8)?;
    println!("wrote {out_path}");
    Ok(())
}

/// Shared: create an offscreen GPU, render `cb`, read the target back, and save a PNG.
fn render_to_png(rt: &tokio::runtime::Runtime, cb: MapCallback, out_path: &str) -> anyhow::Result<()> {
    let (device, queue, adapter) = init_gpu(rt)?;
    println!("adapter: {}", adapter.get_info().name);

    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let mut res = RenderResources::new(&device, format);

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("headless_target"),
        size: wgpu::Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    res.render_once(&device, &queue, &view, &cb, wgpu::Color { r: 0.05, g: 0.05, b: 0.08, a: 1.0 });

    let bytes_per_pixel = 4u32;
    let unpadded = SIZE * bytes_per_pixel;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded = unpadded.div_ceil(align) * align;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (padded * SIZE) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(SIZE),
            },
        },
        wgpu::Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: 1 },
    );
    queue.submit(Some(encoder.finish()));

    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(r); });
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv()??;

    let mapped = slice.get_mapped_range();
    let mut rgba = Vec::with_capacity((unpadded * SIZE) as usize);
    for row in 0..SIZE {
        let start = (row * padded) as usize;
        rgba.extend_from_slice(&mapped[start..start + unpadded as usize]);
    }
    drop(mapped);
    buffer.unmap();

    image::save_buffer(out_path, &rgba, SIZE, SIZE, image::ColorType::Rgba8)?;
    println!("wrote {out_path}");
    Ok(())
}
