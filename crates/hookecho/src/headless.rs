//! Headless verify harness: render the radar or overlay layers to a PNG with no window.
//!
//! `--headless <out.png> [SITE] [--moment M] [--tilt N]` renders one real radar sweep.
//! `--headless-overlay <out.png>` fetches live NWS alerts + SPC Day 1 outlook and renders
//! the vector overlay over CONUS. Both use the exact pipelines the GUI uses.

use crate::overlay_build;
use crate::render::{mercator::Camera, MapCallback, OverlayUpload, RenderResources};
use crate::tiles::{BasemapStyle, TileManager};
use wxdata::level2::{self, Moment};

/// Output edge length in pixels, and the zoom override, if either was asked for.
///
/// Process-global rather than threaded through the dozen render entry points, because every
/// caller of those is already serialized: the CLI renders once and exits, and the server holds a
/// render mutex for the whole call. Set it inside that lock or not at all.
static SIZE_PX: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1000);
/// The zoom override as `f64` bits, or `u64::MAX` for "no override" — no float is that pattern.
static ZOOM_OVERRIDE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(u64::MAX);

/// Edge length of the rendered PNG. Square: every camera the harness builds is square, and a
/// non-square viewport would need the world-to-clip uniform to carry an aspect ratio it doesn't.
fn size() -> u32 {
    SIZE_PX.load(std::sync::atomic::Ordering::Relaxed)
}

/// Whether the renders that follow carry warning polygons, a caption, a color bar and city
/// labels. Off for the CLI verifiers and the golden tests, which want the bare pipeline; on for
/// the server, whose renders are shared as pictures and have to say what they are.
///
/// Process-global for the same reason as the two knobs above, and set under the same lock.
static EXTRAS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Ask for warnings + chrome (or not) on the renders that follow.
pub fn set_extras(on: bool) {
    EXTRAS.store(on, std::sync::atomic::Ordering::Relaxed);
}

fn extras() -> bool {
    EXTRAS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Ask for a different output size (256..=2048 px) and/or zoom for the renders that follow.
///
/// `px: None` leaves the size where it was, but **`zoom: None` clears the override** rather than
/// leaving it: the caller is saying "frame this the way the site deserves", and on a server the
/// previous caller was somebody else's request. The national mosaic asks for a continental zoom
/// every four minutes, and a sticky override handed that framing to every site snapshot after it
/// — a radar page showing the whole continent with a `KFWS · REF 0.5°` caption on it.
pub fn set_output(px: Option<u32>, zoom: Option<f64>) {
    if let Some(px) = px {
        SIZE_PX.store(px.clamp(256, 2048), std::sync::atomic::Ordering::Relaxed);
    }
    ZOOM_OVERRIDE.store(
        zoom.map_or(u64::MAX, |z| z.clamp(1.0, 14.0).to_bits()),
        std::sync::atomic::Ordering::Relaxed,
    );
}

/// Built-in alternate palette for the renders that follow, or `None` for each moment's default.
///
/// Same process-global shape as [`set_output`], and set under the same render lock — and for the
/// same reason it clears rather than persists: the next render is somebody else's request, and a
/// sticky palette would hand one caller's colorblind-safe table to every snapshot after it.
static PALETTE_NAME: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Ask for a built-in alternate palette (by its [`crate::colormap::alt_names`] name) on the
/// renders that follow. `None` restores the moment's default table.
pub fn set_palette(name: Option<String>) {
    if let Ok(mut p) = PALETTE_NAME.lock() {
        *p = name;
    }
}

/// `HOOKECHO_CAM=lon,lat,zoom` overrides any headless camera — framing knob for screenshots.
/// An explicit `--zoom`/`?zoom=` beats both, since it was typed for this render.
fn cam_or_env(lon: f64, lat: f64, zoom: f64) -> Camera {
    let asked = ZOOM_OVERRIDE.load(std::sync::atomic::Ordering::Relaxed);
    let asked = (asked != u64::MAX).then(|| f64::from_bits(asked));
    if let Ok(v) = std::env::var("HOOKECHO_CAM") {
        let p: Vec<f64> = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        if p.len() == 3 {
            return Camera::at_lonlat(p[0], p[1], asked.unwrap_or(p[2]));
        }
    }
    Camera::at_lonlat(lon, lat, asked.unwrap_or(zoom))
}

/// Raster tiles, vector tiles and the place names that came with them.
type Basemap = (
    Vec<crate::render::PendingTile>,
    Vec<crate::render::VisibleTile>,
    Vec<crate::render::PendingVectorTile>,
    Vec<crate::render::TileId>,
    Vec<crate::vector_tiles::PlaceLabel>,
);

/// Basemap for the national-layer renders, so field mosaics sit over a real map instead of
/// the bare clear color. Default: dark vector tiles (same fetch path as `run`).
/// `HOOKECHO_BASEMAP=<slug>` switches to any raster style, e.g. `mapbox-satellite-streets`
/// (provider keys come from the saved Settings — never logged).
fn national_basemap(rt: &tokio::runtime::Runtime, camera: &Camera) -> Basemap {
    let vp = (size() as f32, size() as f32);
    let client = reqwest::Client::new();
    if let Ok(slug) = std::env::var("HOOKECHO_BASEMAP") {
        let style = crate::tiles::BasemapStyle::from_slug(&slug);
        if style.is_raster() {
            let settings = crate::settings::Settings::load();
            let tm = TileManager::new(crate::rt::Spawner::new(rt.handle().clone()));
            let vis = tm.visible(style, camera, vp, 0.0);
            let tiles = rt.block_on(crate::tiles::fetch_visible(
                &client,
                style,
                &vis,
                &settings.mapbox_key,
                &settings.maptiler_key,
            ));
            println!("basemap {}: {} raster tiles", style.label(), tiles.len());
            return (tiles, vis, Vec::new(), Vec::new(), Vec::new());
        }
    }
    let vis = crate::tiles::tile_cover(camera, vp, 14, 0.0);
    let tiles = rt.block_on(async {
        let template = crate::vector_tiles::fetch_tilejson(&client, None).await?;
        Some(
            crate::vector_tiles::fetch_visible_vector(
                &client,
                &template,
                crate::basemap_style::Palette::Dark,
                camera.zoom,
                &vis,
            )
            .await,
        )
    });
    match tiles {
        Some((t, labels)) => {
            println!("basemap: {} vector tiles", t.len());
            (
                Vec::new(),
                Vec::new(),
                t,
                vis.iter().map(|v| v.id).collect(),
                labels,
            )
        }
        None => (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()),
    }
}

/// The active warning polygons, tessellated for the overlay layer — or `None` when nothing is
/// warned and there is nothing to draw.
///
/// Only the three polygon warnings the app leads with: watches and advisories are county-sized
/// wash that hides the radar underneath, which is the thing the picture is of.
fn warnings_overlay(
    rt: &tokio::runtime::Runtime,
    client: &reqwest::Client,
    zoom: f64,
) -> Option<OverlayUpload> {
    let warned = cached_warnings(rt, client)?;
    if warned.is_empty() {
        return None;
    }
    let geom = overlay_build::build(&warned, zoom);
    Some(OverlayUpload {
        vertices: geom.vertices,
        indices: geom.indices,
    })
}

/// How long one alerts fetch serves every render that follows. A server rendering a wall of
/// sites otherwise asks api.weather.gov once per picture for a feed that changes once a minute.
const WARNING_TTL: std::time::Duration = std::time::Duration::from_secs(60);

/// The lead warnings, from cache when they are fresh enough. `None` means "nothing to draw",
/// whether that is because nothing is warned or because the feed could not be reached — a picture
/// with no warnings on it is worth more than no picture.
fn cached_warnings(
    rt: &tokio::runtime::Runtime,
    client: &reqwest::Client,
) -> Option<Vec<wxdata::overlay::GeoFeature>> {
    static CACHE: std::sync::Mutex<Option<(std::time::Instant, Vec<wxdata::overlay::GeoFeature>)>> =
        std::sync::Mutex::new(None);
    let mut slot = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, warned)) = slot.as_ref() {
        if at.elapsed() < WARNING_TTL {
            return (!warned.is_empty()).then(|| warned.clone());
        }
    }
    let warned = match rt.block_on(wxdata::alerts::fetch_polygon_alerts(client)) {
        Ok(f) => f.into_iter().filter(is_lead_warning).collect::<Vec<_>>(),
        Err(e) => {
            log::warn!("alerts for render unavailable: {e}");
            return None;
        }
    };
    *slot = Some((std::time::Instant::now(), warned.clone()));
    (!warned.is_empty()).then_some(warned)
}

/// Tornado, severe thunderstorm and flash flood warnings — the three the app's overlay leads with.
fn is_lead_warning(f: &wxdata::overlay::GeoFeature) -> bool {
    f.kind == wxdata::overlay::FeatureKind::Warning
        && matches!(
            f.title.as_str(),
            "Tornado Warning" | "Severe Thunderstorm Warning" | "Flash Flood Warning"
        )
}

/// The one line under the picture: what radar, what product, what tilt, when, and whose render.
fn caption(site: &str, moment: Moment, elevation_deg: f32, at: Option<&str>) -> String {
    let when = at.unwrap_or("latest");
    format!(
        "{site} · {} {elevation_deg:.1}° · {when} · hookecho.io",
        moment.short_name()
    )
}

/// City names, projected to pixels, biggest places first so the collision pass keeps those.
fn screen_labels(
    labels: &[crate::vector_tiles::PlaceLabel],
    camera: &Camera,
    vp: (f32, f32),
) -> Vec<(f32, f32, String)> {
    let mut cities: Vec<_> = labels.iter().filter(|l| l.city).collect();
    cities.sort_by_key(|l| l.rank);
    // One name, one label. A place sits in every tile that overlaps it, and a frame covers
    // dozens of tiles, so the same city arrives many times over — and since they are all at the
    // same pixel, the copies collided with each other and ate the budget. Dallas, Houston and
    // Oklahoma City were the only names surviving on a frame that had five thousand labels to
    // choose from.
    let mut seen = std::collections::HashSet::new();
    cities.retain(|l| seen.insert(l.name.clone()));
    cities
        .iter()
        .map(|l| {
            let (x, y) = camera.world_to_screen((l.world[0] as f64, l.world[1] as f64), vp);
            (x, y, l.name.clone())
        })
        .filter(|(x, y, _)| *x > 0.0 && *y > 0.0 && *x < vp.0 && *y < vp.1)
        .take(25)
        .collect()
}

/// A volume time as it appears in a caption: `2026-08-29 20:32Z`, always UTC.
fn stamp_time(t: &chrono::DateTime<chrono::Utc>) -> String {
    t.format("%Y-%m-%d %H:%MZ").to_string()
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
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    // A `.pal` file on the command line beats everything: it is the caller holding the table in
    // their hand. Otherwise an alternate may have been requested by name — never by path, so a
    // request can never name a file.
    let table = match pal {
        Some(path) => crate::colormap::parse_pal(&std::fs::read_to_string(path)?)?,
        None => PALETTE_NAME
            .lock()
            .ok()
            .and_then(|p| p.as_deref().and_then(crate::colormap::builtin_alt))
            .unwrap_or_else(|| crate::colormap::default_table(moment).clone()),
    };

    // The volume's own time, formatted, for the caption — carried as a string because each
    // network hands back a different chrono type for it and the caption is all any of them feed.
    let (sweep, scan_time) = rt.block_on(async {
        // Only NEXRAD has a volume archive to scrub; every other network publishes its current
        // volume and nothing else. Saying so is better than quietly rendering "now" for a request
        // that asked for last Tuesday.
        if !wxdata::sites::is_nexrad(site) && (date.is_some() || hhmm.is_some()) {
            anyhow::bail!("{site} has no volume archive — only the current scan is available");
        }
        // The live path for those networks: one call assembles the newest volume from the site's
        // own files, and it feeds the same binner the Level 2 path uses.
        if !wxdata::sites::is_nexrad(site) {
            let http = reqwest::Client::builder()
                .user_agent(wxdata::alerts::USER_AGENT)
                .build()?;
            let fetched = match wxdata::sites::network(site) {
                wxdata::sites::Network::Tdwr => wxdata::tdwr::fetch_volume(&http, site, None).await,
                wxdata::sites::Network::Dwd => wxdata::dwd::fetch_volume(&http, site, None).await,
                wxdata::sites::Network::Opera => {
                    wxdata::opera::fetch_volume(&http, site, None).await
                }
                wxdata::sites::Network::Nexrad => unreachable!("guarded above"),
            }?;
            let (name, time, scan) =
                fetched.ok_or_else(|| anyhow::anyhow!("no current volume for {site}"))?;
            eprintln!("live volume: {name}");
            return Ok((
                level2::bin_scan_opts(&scan, moment, tilt, dealias)?,
                Some(stamp_time(&time)),
            ));
        }
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
                        let m = f
                            .date_time()
                            .map(|d| {
                                d.time()
                                    .signed_duration_since(chrono::NaiveTime::MIN)
                                    .num_minutes()
                            })
                            .unwrap_or(0);
                        (m - tm).abs()
                    })
                    .unwrap(),
                None => frames.into_iter().next_back().unwrap(),
            };
            eprintln!("archive frame: {}", id.name());
            let at = id.date_time().map(|t| stamp_time(&t));
            let scan = level2::download_scan(id, None).await?;
            return Ok((level2::bin_scan_opts(&scan, moment, tilt, dealias)?, at));
        }

        let mut day = chrono::Utc::now().date_naive();
        for _ in 0..3 {
            // `download_latest_scan` throws the volume identifier away, and the identifier is
            // where the volume's time lives — so the same two calls are made here by hand.
            let latest = level2::list_volumes(site, day)
                .await
                .map(|mut v| v.pop())
                .unwrap_or_default();
            match latest {
                Some(id) => {
                    let at = id.date_time().map(|t| stamp_time(&t));
                    let scan = level2::download_scan(id, None).await?;
                    return Ok((level2::bin_scan_opts(&scan, moment, tilt, dealias)?, at));
                }
                None => {
                    eprintln!("{day}: no volumes");
                    day = day.pred_opt().unwrap();
                }
            }
        }
        anyhow::bail!("no volumes for {site} in last 3 days")
    })?;
    let echo_gates = sweep.data.iter().filter(|&&v| v > 1).count();
    println!(
        "sweep: {} {:.2}deg {}x{} grid, {} echo gates, radar {:.3},{:.3}",
        site,
        sweep.elevation_deg,
        sweep.gate_count,
        sweep.az_bins,
        echo_gates,
        sweep.radar_lat,
        sweep.radar_lon
    );

    let camera = cam_or_env(sweep.radar_lon as f64, sweep.radar_lat as f64, 7.0);
    let (center, scale) = camera.world_to_clip_uniform((size() as f32, size() as f32));

    let vp = (size() as f32, size() as f32);
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (compatible; hookecho/0.0; +github.com/d4vid87/hookecho)")
        .build()?;

    // Basemap under the radar. Satellite + provider styles are raster; Dark/Light are vector MVT.
    // Provider keys come from the saved Settings (never logged).
    let settings = crate::settings::Settings::load();
    let is_vector = matches!(basemap, BasemapStyle::Dark | BasemapStyle::Light);
    let (new_tiles, visible) = if basemap.is_raster() {
        let tm = TileManager::new(crate::rt::Spawner::new(rt.handle().clone()));
        // Style is passed per call so the zoom cap matches this source (GOES layers top out early).
        let vis = tm.visible(basemap, &camera, vp, 0.0);
        let tiles = rt.block_on(crate::tiles::fetch_visible(
            &client,
            basemap,
            &vis,
            &settings.mapbox_key,
            &settings.maptiler_key,
        ));
        println!("basemap {}: {} tiles fetched", basemap.label(), tiles.len());
        (tiles, vis)
    } else {
        (Vec::new(), Vec::new())
    };
    let mut place_labels: Vec<crate::vector_tiles::PlaceLabel> = Vec::new();
    let (new_vector_tiles, visible_vector) = if is_vector {
        let dark = basemap == BasemapStyle::Dark;
        let tess_zoom = camera.zoom;
        let vis = crate::tiles::tile_cover(&camera, vp, 14, 0.0);
        let (tiles, labels) = rt.block_on(async {
            let template = crate::vector_tiles::fetch_tilejson(&client, None)
                .await
                .ok_or_else(|| anyhow::anyhow!("no tilejson template"))?;
            println!("tilejson template: {template}");
            Ok::<_, anyhow::Error>(
                crate::vector_tiles::fetch_visible_vector(
                    &client,
                    &template,
                    if dark {
                        crate::basemap_style::Palette::Dark
                    } else {
                        crate::basemap_style::Palette::Light
                    },
                    tess_zoom,
                    &vis,
                )
                .await,
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
        place_labels = labels;
        (tiles, ids)
    } else {
        (Vec::new(), Vec::new())
    };

    // Warnings and chrome, when the caller asked for them (the server does; the verifiers do not).
    let overlay = extras()
        .then(|| warnings_overlay(&rt, &client, camera.zoom))
        .flatten();
    let stamp = extras().then(|| crate::chrome::Stamp {
        caption: caption(site, moment, sweep.elevation_deg, scan_time.as_deref()),
        bar: Some(crate::chrome::Bar {
            table: table.clone(),
            unit: moment.units(),
        }),
        labels: screen_labels(&place_labels, &camera, vp),
    });

    let cb = MapCallback {
        pane: 0,
        camera_center: center,
        camera_scale: scale,
        new_tiles,
        visible,
        basemap_key: basemap.key(),
        vector_over_raster: false,
        radar_upload: Some(crate::app::to_upload(
            &sweep, &table, None, smooth, storm_uv, None, false,
        )),
        draw_radar: true,
        draw_overlay: overlay.is_some(),
        overlay_upload: overlay,
        field_uploads: Vec::new(),
        field_draws: Vec::new(),
        clear_tiles: false,
        drop_tiles: Vec::new(),
        drop_fields: Vec::new(),
        new_vector_tiles,
        visible_vector,
        clear_vector: false,
        drop_vector_tiles: Vec::new(),
        wind_upload: None,
        wind: None,
    };
    render_to_png_stamped(&rt, cb, out_path, stamp.as_ref())
}

/// Verify the multi-pane render path: prepare two panes with different cameras (pane 1 last),
/// then draw each. Proves per-pane camera state survives the all-prepare-then-paint order — the
/// core U9 correctness risk. Writes two PNGs and asserts pane 0 is unaffected by pane 1's prepare.
pub fn run_multipane(site: &str, out_a: &str, out_b: &str) -> anyhow::Result<()> {
    use crate::render::MapCallback;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
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
    let vp = (size() as f32, size() as f32);

    // Pane 0 centered on the radar; pane 1 offset well to the east (different camera).
    let cam_a = Camera::at_lonlat(sweep.radar_lon as f64, sweep.radar_lat as f64, 7.0);
    let cam_b = Camera::at_lonlat(sweep.radar_lon as f64 + 2.5, sweep.radar_lat as f64, 7.0);
    let mk = |pane: u32, cam: &Camera| {
        let (center, scale) = cam.world_to_clip_uniform(vp);
        MapCallback {
            pane,
            camera_center: center,
            camera_scale: scale,
            basemap_key: 0,
            vector_over_raster: false,
            new_tiles: Vec::new(),
            visible: Vec::new(),
            radar_upload: Some(crate::app::to_upload(
                &sweep, &table, None, false, None, None, false,
            )),
            draw_radar: true,
            overlay_upload: None,
            draw_overlay: false,
            field_uploads: Vec::new(),
            field_draws: Vec::new(),
            clear_tiles: false,
            drop_tiles: Vec::new(),
            drop_fields: Vec::new(),
            new_vector_tiles: Vec::new(),
            visible_vector: Vec::new(),
            clear_vector: false,
            drop_vector_tiles: Vec::new(),
            wind_upload: None,
            wind: None,
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
    anyhow::ensure!(
        differ,
        "FAIL: panes with different cameras rendered identically"
    );
    println!("multi-pane render PASS");
    Ok(())
}

/// Fetch + decode + project Level 3 storm cells for `site` and print them (verifies the
/// bucket fetch, the from-scratch L3 decoder, and lon/lat projection windowless).
pub fn run_cells(site: &str) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
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

/// Multi-radar mosaic verify: pick the radars covering a box around `site`, fetch each one's N0B,
/// composite them, and print the grid plus what went into it. Proves the whole F2 path — site
/// selection, L3 decode, nearest-radar compositing — against live data without a GPU.
pub fn run_mosaic(site: &str) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let s = wxdata::sites::site_by_id(site)
        .ok_or_else(|| anyhow::anyhow!("unknown radar site {site}"))?;
    let (lon, lat) = (s.longitude as f64, s.latitude as f64);
    // ~2.5° box, which is roughly a regional view at zoom 7.
    let sites =
        wxdata::mosaic::sites_for_view(Some(site), lon - 2.5, lat - 2.5, lon + 2.5, lat + 2.5, 6);
    println!("{site}: mosaic over {sites:?}");
    let m = rt
        .block_on(async {
            let http = reqwest::Client::new();
            wxdata::mosaic::fetch(&http, &sites).await
        })
        .ok_or_else(|| anyhow::anyhow!("no N0B came back for any of {sites:?}"))?;
    let f = &m.field;
    let data = f.values.iter().filter(|v| !v.is_nan()).count();
    let max = f.values.iter().cloned().fold(f32::MIN, f32::max);
    println!(
        "  contributed: {}
  grid {}x{}  lon {:.2}..{:.2}  lat {:.2}..{:.2}",
        m.sites.join(", "),
        f.nx,
        f.ny,
        f.lon_west,
        f.lon_east,
        f.lat_south,
        f.lat_north
    );
    println!(
        "  {data} cells with data ({:.1}%), max {max:.1} dBZ, oldest scan {}",
        100.0 * data as f64 / (f.nx * f.ny) as f64,
        m.oldest.format("%Y-%m-%d %H:%M:%SZ")
    );
    anyhow::ensure!(data > 0, "composite is entirely empty");
    Ok(())
}

/// Warning-verification lab, headless: `--headless-verify WFO START END` (RFC3339 or `YYYY-MM-DD`).
/// Prints the skill table and the warning list, which is checkable against IEM's own Cow web UI.
pub fn run_verify(wfo: &str, start: &str, end: &str) -> anyhow::Result<()> {
    let parse_when = |s: &str| -> anyhow::Result<chrono::DateTime<chrono::Utc>> {
        if let Ok(t) = chrono::DateTime::parse_from_rfc3339(s) {
            return Ok(t.with_timezone(&chrono::Utc));
        }
        let d = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")?;
        Ok(d.and_hms_opt(0, 0, 0)
            .ok_or_else(|| anyhow::anyhow!("bad date {s}"))?
            .and_utc())
    };
    let (start, end) = (parse_when(start)?, parse_when(end)?);
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let v = rt.block_on(async {
        let http = reqwest::Client::new();
        wxdata::verify::fetch(&http, wfo, start, end).await
    })?;
    let s = &v.stats;
    println!(
        "{} {} → {}",
        v.wfo,
        start.format("%Y-%m-%d %H:%MZ"),
        end.format("%Y-%m-%d %H:%MZ")
    );
    println!(
        "  POD {:.2}  FAR {:.2}  CSI {:.2}   lead avg {:.1} / max {} min",
        s.pod, s.far, s.csi, s.avg_lead_min, s.max_lead_min
    );
    println!(
        "  warnings {}/{} verified   reports {} ({} warned, {} missed)   avg polygon {:.0} km²",
        s.events_verified,
        s.events_total,
        s.reports_total,
        s.warned_reports,
        s.unwarned_reports,
        s.avg_size_km2
    );
    for w in v.warnings.iter().take(20) {
        println!(
            "  {} {}-{:04}  {}  {}  {}",
            if w.verified { "✓" } else { "✗" },
            w.phenomena,
            w.eventid,
            w.issue.format("%d %H:%MZ"),
            w.lead_min
                .map(|m| format!("lead {m:>3} min"))
                .unwrap_or_else(|| "  no verify  ".into()),
            w.counties.join(", ")
        );
    }
    Ok(())
}

/// TDS verify: download the latest dual-pol volume, bin reflectivity + CC at the lowest tilt, run
/// the debris-signature detector, and print any hits. Proves the detection pipeline on real data.
pub fn run_tds(site: &str) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let (z, cc) = rt.block_on(async {
        let scan = level2::download_latest_scan(site, chrono::Utc::now().date_naive()).await?;
        let z = level2::bin_scan(&scan, Moment::Reflectivity, 0)?;
        let cc = level2::bin_scan(&scan, Moment::CorrelationCoefficient, 0)?;
        anyhow::Ok((z, cc))
    })?;
    println!(
        "{site}: Z {}x{} @ {:.2}°, CC {}x{} @ {:.2}°",
        z.az_bins, z.gate_count, z.elevation_deg, cc.az_bins, cc.gate_count, cc.elevation_deg
    );
    let hits = wxdata::tds::detect(&z, &cc, 0.80, 40.0, 150.0, 4);
    println!(
        "TDS clusters (CC<0.80, Z>=40 dBZ, >=4 gates): {}",
        hits.len()
    );
    for h in hits.iter().take(8) {
        println!(
            "  {:.3},{:.3}  {} gates  min CC {:.2}",
            h.lat, h.lon, h.gates, h.min_cc
        );
    }
    Ok(())
}

/// Dual-pol signature verify: download the latest volume and run all three of the
/// [`wxdata::dualpol`] detectors on it — three-body scatter spikes at the lowest tilt, ZDR columns
/// through every tilt, and the CC bright band. `h0_km` is the freezing level above radar level;
/// there is no model fetch here, so it is a CLI argument.
pub fn run_dualpol(site: &str, h0_km: f64) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let (z0, cc0, zdr_tilts, z_tilts, cc_tilts) = rt.block_on(async {
        let scan = level2::download_latest_scan(site, chrono::Utc::now().date_naive()).await?;
        let n = level2::elevation_angles(&scan).len();
        let z0 = level2::bin_scan(&scan, Moment::Reflectivity, 0)?;
        let cc0 = level2::bin_scan(&scan, Moment::CorrelationCoefficient, 0)?;
        let take = |m: Moment| -> Vec<wxdata::level2::BinnedSweep> {
            (0..n)
                .filter_map(|t| level2::bin_scan(&scan, m, t).ok())
                .collect()
        };
        anyhow::Ok((
            z0,
            cc0,
            take(Moment::DifferentialReflectivity),
            take(Moment::Reflectivity),
            take(Moment::CorrelationCoefficient),
        ))
    })?;
    println!(
        "{site}: {} ZDR tilts, {} Z tilts, {} CC tilts; freezing level {h0_km:.1} km ARL",
        zdr_tilts.len(),
        z_tilts.len(),
        cc_tilts.len()
    );

    let spikes = wxdata::dualpol::tbss(&z0, &cc0, 60.0, 20.0, 0.8, 4.0, 150.0);
    println!(
        "TBSS (hail spikes) at {:.2}deg: {}",
        z0.elevation_deg,
        spikes.len()
    );
    for h in spikes.iter().take(8) {
        println!(
            "  {:.3},{:.3}  core {:.0} dBZ  spike {:.1} km  min CC {:.2}",
            h.lat, h.lon, h.core_dbz, h.len_km, h.min_cc
        );
    }

    let columns = wxdata::dualpol::zdr_columns(&zdr_tilts, &z_tilts, h0_km, 1.0, 1.0, 40.0, 100.0);
    println!(
        "ZDR columns (>1 dB, >=1 km above the freezing level): {}",
        columns.len()
    );
    for h in columns.iter().take(8) {
        println!(
            "  {:.3},{:.3}  depth {:.1} km  top {:.1} km  max {:.1} dB",
            h.lat, h.lon, h.depth_km, h.top_km, h.max_zdr
        );
    }

    // The low tilts see the band only at long range, where the beam is too wide to say anything.
    let mid: Vec<wxdata::level2::BinnedSweep> = cc_tilts
        .into_iter()
        .filter(|s| (2.0..=10.0).contains(&s.elevation_deg))
        .collect();
    let mid_z: Vec<wxdata::level2::BinnedSweep> = z_tilts
        .iter()
        .filter(|s| (2.0..=10.0).contains(&s.elevation_deg))
        .cloned()
        .collect();
    match wxdata::dualpol::bright_band(&mid, &mid_z, 6.0) {
        Some(bb) => println!(
            "bright band: {:.2} km ARL, mean CC {:.2}, {} gates",
            bb.height_km, bb.mean_cc, bb.samples
        ),
        None => println!("bright band: none found (no melting layer in view, or clean CC)"),
    }
    Ok(())
}

/// Rule verify: run the user's own alert rules against the latest volume and say, for each one,
/// whether it would fire and why not if it wouldn't.
///
/// A rule that never goes off is indistinguishable from a rule that is wrong, and waiting for
/// real weather to find out is a bad way to learn that a threshold was set too high. This replays
/// the scan triggers against a live scan and prints the verdict.
///
/// ponytail: scan triggers only. ProbSevere and lightning density need their national feeds
/// polling, which is the app's job, not a one-shot verifier's.
pub fn run_rules(site: &str) -> anyhow::Result<()> {
    use crate::rules::Detection;
    use crate::settings::RuleTrigger as T;

    let settings = crate::settings::Settings::load();
    let rules: Vec<&crate::settings::AlertRule> = settings.alert_rules.iter().collect();
    if rules.is_empty() {
        println!("no rules configured (Ctrl+K ▸ \"Alert rules…\")");
        return Ok(());
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let (z, cc, vel, zdr_tilts, z_tilts) = rt.block_on(async {
        let scan = level2::download_latest_scan(site, chrono::Utc::now().date_naive()).await?;
        let n = level2::elevation_angles(&scan).len();
        let take = |m: Moment| -> Vec<wxdata::level2::BinnedSweep> {
            (0..n)
                .filter_map(|t| level2::bin_scan(&scan, m, t).ok())
                .collect()
        };
        anyhow::Ok((
            level2::bin_scan(&scan, Moment::Reflectivity, 0)?,
            level2::bin_scan(&scan, Moment::CorrelationCoefficient, 0)?,
            level2::bin_scan_opts(&scan, Moment::Velocity, 0, true)?,
            take(Moment::DifferentialReflectivity),
            take(Moment::Reflectivity),
        ))
    })?;
    println!(
        "{site}: {} tilts, lowest {:.2}°",
        z_tilts.len(),
        z.elevation_deg
    );

    let d = &settings.detectors;
    let tds = wxdata::tds::detect(&z, &cc, 0.80, 40.0, 150.0, 4);
    let tbss = wxdata::dualpol::tbss(&z, &cc, d.tbss_core_dbz, 20.0, 0.8, 4.0, 150.0);
    // No model freezing level out here; 4 km ARL is the usual warm-season figure and the same
    // default `--headless-dualpol` takes.
    let zdr = wxdata::dualpol::zdr_columns(
        &zdr_tilts,
        &z_tilts,
        4.0,
        d.zdr_min_db,
        d.zdr_min_depth_km,
        40.0,
        100.0,
    );
    // Same thresholds the app's own couplet layer uses, so the verdict matches the app.
    let couplets = wxdata::rotation::detect(&vel, 25.0, 15.0, 150.0, 3);
    println!(
        "detections: {} TDS, {} TBSS, {} ZDR columns, {} couplets",
        tds.len(),
        tbss.len(),
        zdr.len(),
        couplets.len()
    );

    for rule in rules {
        let label = format!("{} [{}]", rule.title(), rule.trigger.label());
        if !rule.enabled {
            println!("  {label}: disabled");
            continue;
        }
        if !rule.trigger.is_scan() {
            println!("  {label}: needs a live feed — not replayable here");
            continue;
        }
        if !crate::rules::place_exists(&rule.place, &settings) {
            println!("  {label}: its place no longer exists — can never fire");
            continue;
        }
        let hits: Vec<Detection> = match rule.trigger {
            T::Tds => tds.iter().map(|h| Detection::at(h.lon, h.lat)).collect(),
            T::Tbss => tbss.iter().map(|h| Detection::at(h.lon, h.lat)).collect(),
            T::ZdrColumn => zdr.iter().map(|h| Detection::at(h.lon, h.lat)).collect(),
            _ => couplets
                .iter()
                .map(|h| Detection::with_strength(h.lon, h.lat, h.vrot_ms as f64 * 1.943_844))
                .collect(),
        };
        let place = crate::rules::place_label(&rule.place, &settings);
        match hits
            .iter()
            .find(|h| crate::rules::matches(rule, h, &settings))
        {
            Some(h) => {
                let strength = h.strength.map(|v| format!(" ({v:.0})")).unwrap_or_default();
                println!(
                    "  {label}: FIRES at {:.3},{:.3}{strength} — {place}",
                    h.lat, h.lon
                );
            }
            // Say which of the two reasons it was: nothing detected at all, or nothing that got
            // past the threshold and the place.
            None if hits.is_empty() => println!("  {label}: quiet (nothing detected)"),
            None => println!(
                "  {label}: quiet ({} detections, none past the threshold at {place})",
                hits.len()
            ),
        }
    }
    Ok(())
}

/// Rotation verify: download the latest volume, bin the dealiased velocity at the lowest tilt,
/// run the couplet detector, and print any hits. Proves the detection pipeline on real data.
pub fn run_rotation(site: &str) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let vel = rt.block_on(async {
        let scan = level2::download_latest_scan(site, chrono::Utc::now().date_naive()).await?;
        level2::bin_scan_opts(&scan, Moment::Velocity, 0, true)
    })?;
    println!(
        "{site}: V {}x{} @ {:.2}°",
        vel.az_bins, vel.gate_count, vel.elevation_deg
    );
    let hits = wxdata::rotation::detect(&vel, 25.0, 15.0, 150.0, 3);
    println!(
        "rotation couplets (>=25 m/s gate-to-gate, 15-150 km, >=3 gates): {}",
        hits.len()
    );
    for h in hits.iter().take(8) {
        println!(
            "  {:.3},{:.3}  vrot {:.0} kt  g2g {:.0} m/s  {:.0} km  {} gates",
            h.lat,
            h.lon,
            h.vrot_ms * 1.943_844,
            h.g2g_ms,
            h.range_km,
            h.gates
        );
    }
    Ok(())
}

/// Fetch the VAD wind profile for a site and print the levels (altitude, dir/speed, u/v).
pub fn run_vwp(site: &str) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
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
    let s =
        wxdata::sites::site_by_id(site).ok_or_else(|| anyhow::anyhow!("unknown site {site}"))?;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let station = rt.block_on(async {
        let http = reqwest::Client::new();
        wxdata::obs::fetch_nearest(&http, s.latitude as f64, s.longitude as f64).await
    })?;
    println!(
        "{site} -> station {} ({}), {} obs",
        station.station_id,
        station.name,
        station.obs.len()
    );
    if let Some(o) = station.obs.first() {
        let f = |v: Option<f32>| v.map(|x| format!("{x:.1}")).unwrap_or_else(|| "—".into());
        println!(
            "  latest {}: temp {}C dew {}C rh {}% wind {}km/h gust {} dir {} pres {}Pa",
            o.time
                .map(|t| t.format("%H:%MZ").to_string())
                .unwrap_or_default(),
            f(o.temp_c),
            f(o.dewpoint_c),
            f(o.rh),
            f(o.wind_kmh),
            f(o.gust_kmh),
            f(o.wind_dir_deg),
            f(o.pressure_pa),
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
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let feats = rt.block_on(async {
        let http = reqwest::Client::new();
        wxdata::alerts::fetch_active(&http, &[]).await
    })?;
    // Dedupe by alert id (MultiPolygon alerts emit one feature per part).
    let mut seen = std::collections::HashSet::new();
    let alerts: Vec<_> = feats
        .iter()
        .filter_map(|f| f.alert.as_ref())
        .filter(|a| seen.insert(a.id.clone()))
        .collect();
    println!(
        "{} alert polygons, {} unique alerts",
        feats.len(),
        alerts.len()
    );
    // Prefer ones with severe-weather parameters populated.
    for a in alerts
        .iter()
        .filter(|a| a.max_hail_in.is_some() || a.max_wind.is_some())
        .take(12)
    {
        println!(
            "  {:<32} hail {}  wind {}  tor {}  dmg {}  expires {}",
            a.event,
            a.max_hail_in
                .map(|h| format!("{h:.2}in"))
                .unwrap_or_else(|| "—".into()),
            a.max_wind.as_deref().unwrap_or("—"),
            a.tornado_detection.as_deref().unwrap_or("—"),
            a.damage_threat.as_deref().unwrap_or("—"),
            a.expires
                .map(|e| e.format("%H:%MZ").to_string())
                .unwrap_or_else(|| "—".into()),
        );
    }
    // Storm motion + escalation (feature S).
    for a in alerts
        .iter()
        .filter(|a| a.motion.is_some() || wxdata::alerts::escalation(a) > 0)
    {
        let esc = wxdata::alerts::escalation(a);
        match &a.motion {
            Some(m) => println!(
                "  motion: {:>3.0}° {:>2.0}kt ({} pts) esc={esc}  [{}]",
                m.deg,
                m.kt,
                m.points.len(),
                a.event
            ),
            None => println!("  motion:    —          esc={esc}  [{}]", a.event),
        }
    }
    Ok(())
}

/// Fetch + tally the storm-based warnings archived at an instant (feature W).
pub fn run_archwarn(ts: &str) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
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
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let sweep = rt.block_on(async {
        // Seed with the SECOND-newest archived volume so the in-progress live volume the
        // stream joins genuinely differs (an up-to-date base would correctly yield no update).
        let day = chrono::Utc::now().date_naive();
        let mut ids = level2::list_volumes(site, day).await?;
        let seed = ids.pop().and_then(|_| ids.pop()).or_else(|| ids.pop());
        let base = match seed {
            Some(id) => level2::download_scan(id, None).await?,
            None => level2::download_latest_scan(site, day).await?,
        };
        println!("base volume: {} sweeps", base.sweeps().len());
        let base = std::sync::Arc::new(base);

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let site_owned = site.to_string();
        let handle = tokio::spawn(async move {
            let _ = wxdata::live::stream(
                site_owned,
                base,
                || true,
                move |u| {
                    let _ = tx.send(u);
                },
            )
            .await;
        });

        // First update should arrive within a couple minutes (backfill emits immediately).
        let update = tokio::time::timeout(std::time::Duration::from_secs(180), rx.recv())
            .await
            .map_err(|_| anyhow::anyhow!("no live update within 180s"))?
            .ok_or_else(|| anyhow::anyhow!("stream closed before first update"))?;
        handle.abort();
        println!(
            "live update: {} ({} sweeps, {} changed tilts)",
            update.name,
            update.scan.sweeps().len(),
            update.changed.len()
        );
        level2::bin_scan(&update.scan, moment, 0)
    })?;

    println!(
        "sweep: {} {:.2}deg {}x{}",
        site, sweep.elevation_deg, sweep.gate_count, sweep.az_bins
    );
    let camera = Camera::at_lonlat(sweep.radar_lon as f64, sweep.radar_lat as f64, 7.0);
    let (center, scale) = camera.world_to_clip_uniform((size() as f32, size() as f32));
    let table = crate::colormap::default_table(moment).clone();
    let cb = MapCallback {
        pane: 0,
        camera_center: center,
        camera_scale: scale,
        basemap_key: 0,
        vector_over_raster: false,
        new_tiles: Vec::new(),
        visible: Vec::new(),
        radar_upload: Some(crate::app::to_upload(
            &sweep, &table, None, false, None, None, false,
        )),
        draw_radar: true,
        overlay_upload: None,
        draw_overlay: false,
        field_uploads: Vec::new(),
        field_draws: Vec::new(),
        clear_tiles: false,
        drop_tiles: Vec::new(),
        drop_fields: Vec::new(),
        new_vector_tiles: Vec::new(),
        visible_vector: Vec::new(),
        clear_vector: false,
        drop_vector_tiles: Vec::new(),
        wind_upload: None,
        wind: None,
    };
    render_to_png(&rt, cb, out_path)
}

/// Parse a local GRLevelX placefile, tessellate its lines/polygons, and render them centered on
/// their bounding box to a PNG (verifies the parser + overlay tessellation windowless).
pub fn run_placefile(path: &str, out_path: &str) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let text = std::fs::read_to_string(path)?;
    let pf = wxdata::placefile::parse(&text);
    println!(
        "placefile '{}': {} items, refresh {}s",
        pf.title,
        pf.items.len(),
        pf.refresh_secs
    );

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
        // Object bodies are pixel offsets, not positions — their anchor is the coordinate.
        if let Some(a) = it.anchor {
            acc(a[0], a[1]);
            continue;
        }
        match &it.kind {
            PlaceKind::Line { pts, .. } => pts.iter().for_each(|p| acc(p[0], p[1])),
            PlaceKind::Triangles { verts } => verts.iter().for_each(|(p, _)| acc(p[0], p[1])),
            PlaceKind::Image { verts, .. } => verts.iter().for_each(|(p, _)| acc(p[0], p[1])),
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
    let items: Vec<(&wxdata::placefile::PlaceItem, f32)> =
        pf.items.iter().map(|i| (i, 1.0)).collect();
    let mut geom = overlay_build::OverlayGeom::default();
    overlay_build::append_placefiles(&mut geom, &items, zoom);
    println!(
        "tessellated {} verts / {} indices",
        geom.vertices.len(),
        geom.indices.len()
    );

    let (center, scale) = camera.world_to_clip_uniform((size() as f32, size() as f32));
    let cb = MapCallback {
        pane: 0,
        camera_center: center,
        camera_scale: scale,
        basemap_key: 0,
        vector_over_raster: false,
        new_tiles: Vec::new(),
        visible: Vec::new(),
        radar_upload: None,
        draw_radar: false,
        overlay_upload: Some(OverlayUpload {
            vertices: geom.vertices,
            indices: geom.indices,
        }),
        draw_overlay: true,
        field_uploads: Vec::new(),
        field_draws: Vec::new(),
        clear_tiles: false,
        drop_tiles: Vec::new(),
        drop_fields: Vec::new(),
        new_vector_tiles: Vec::new(),
        visible_vector: Vec::new(),
        clear_vector: false,
        drop_vector_tiles: Vec::new(),
        wind_upload: None,
        wind: None,
    };
    render_to_png(&rt, cb, out_path)
}

/// Fetch live severe-weather overlays and render them over CONUS to a PNG.
pub fn run_overlay(out_path: &str) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let (alerts, outlook) = rt.block_on(async {
        let client = reqwest::Client::new();
        let alerts = wxdata::alerts::fetch_active(&client, &[])
            .await
            .unwrap_or_default();
        let outlook = wxdata::spc::fetch_outlook(&client, 1)
            .await
            .unwrap_or_default();
        (alerts, outlook)
    });
    let mut features = outlook;
    features.extend(alerts);
    println!("overlay features: {}", features.len());

    let zoom = 4.0;
    let camera = cam_or_env(-97.0, 38.0, zoom); // CONUS center
    let (new_tiles, visible, new_vector_tiles, visible_vector, _place_labels) =
        national_basemap(&rt, &camera);
    let geom = overlay_build::build(&features, zoom);
    println!(
        "tessellated {} verts / {} indices",
        geom.vertices.len(),
        geom.indices.len()
    );

    let (center, scale) = camera.world_to_clip_uniform((size() as f32, size() as f32));
    let cb = MapCallback {
        pane: 0,
        camera_center: center,
        camera_scale: scale,
        basemap_key: 0,
        vector_over_raster: false,
        new_tiles,
        visible,
        radar_upload: None,
        draw_radar: false,
        overlay_upload: Some(OverlayUpload {
            vertices: geom.vertices,
            indices: geom.indices,
        }),
        draw_overlay: true,
        field_uploads: Vec::new(),
        field_draws: Vec::new(),
        clear_tiles: false,
        drop_tiles: Vec::new(),
        drop_fields: Vec::new(),
        new_vector_tiles,
        visible_vector,
        clear_vector: false,
        drop_vector_tiles: Vec::new(),
        wind_upload: None,
        wind: None,
    };
    render_to_png(&rt, cb, out_path)
}

/// Fetch the latest MRMS national mosaic and render it over CONUS.
pub fn run_mrms(out_path: &str) -> anyhow::Result<()> {
    use crate::render::{mercator::lonlat_to_world, MrmsUpload};
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let field = rt.block_on(async {
        let client = reqwest::Client::new();
        wxdata::mrms::fetch_latest(&client, wxdata::mrms::REFLECTIVITY).await
    })?;
    println!(
        "mrms grid {}x{}  lon [{:.2},{:.2}]  lat [{:.2},{:.2}]  time {}",
        field.nx,
        field.ny,
        field.lon_west,
        field.lon_east,
        field.lat_south,
        field.lat_north,
        field.time
    );
    let valid = field.values.iter().filter(|v| !v.is_nan()).count();
    let vmax = field
        .values
        .iter()
        .cloned()
        .filter(|v| !v.is_nan())
        .fold(f32::MIN, f32::max);
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
            field.lon_west as f32,
            field.lat_north as f32,
            field.lon_east as f32,
            field.lat_south as f32,
            field.nx as f32,
            field.ny as f32,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
        ],
        lut: crate::colormap::bake_lut(table, (vmin, vspan_max), None).to_vec(),
    };

    let camera = cam_or_env(-97.0, 38.0, 4.0);
    let (new_tiles, visible, new_vector_tiles, visible_vector, place_labels) =
        national_basemap(&rt, &camera);
    let (center, scale) = camera.world_to_clip_uniform((size() as f32, size() as f32));
    // Same opt-in as the site renders: bare mosaic for the CLI verifier, warnings and a caption
    // for the one the server publishes.
    let client = reqwest::Client::new();
    let overlay = extras()
        .then(|| warnings_overlay(&rt, &client, camera.zoom))
        .flatten();
    let stamp = extras().then(|| crate::chrome::Stamp {
        caption: format!(
            "MRMS composite reflectivity · {} · hookecho.io",
            stamp_time(&field.time)
        ),
        bar: Some(crate::chrome::Bar {
            table: table.clone(),
            unit: Moment::Reflectivity.units(),
        }),
        // The same city names the site frames carry — a continental mosaic with no place on it
        // is a shape, not a map.
        labels: screen_labels(&place_labels, &camera, (size() as f32, size() as f32)),
    });
    let cb = MapCallback {
        pane: 0,
        camera_center: center,
        camera_scale: scale,
        basemap_key: 0,
        vector_over_raster: false,
        new_tiles,
        visible,
        radar_upload: None,
        draw_radar: false,
        draw_overlay: overlay.is_some(),
        overlay_upload: overlay,
        field_uploads: vec![(crate::render::FieldLayer::Mrms, upload)],
        field_draws: vec![(crate::render::FieldLayer::Mrms, 1.0)],
        clear_tiles: false,
        drop_tiles: Vec::new(),
        drop_fields: Vec::new(),
        new_vector_tiles,
        visible_vector,
        clear_vector: false,
        drop_vector_tiles: Vec::new(),
        wind_upload: None,
        wind: None,
    };
    render_to_png_stamped(&rt, cb, out_path, stamp.as_ref())
}

/// Fetch the latest MRMS lightning-density mosaic, print stats, and render it over CONUS.
pub fn run_lightning(out_path: &str) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let field = rt.block_on(async {
        let client = reqwest::Client::new();
        wxdata::mrms::fetch_latest(&client, wxdata::mrms::LIGHTNING).await
    })?;
    let nonzero = field
        .values
        .iter()
        .filter(|v| !v.is_nan() && **v > 0.0)
        .count();
    let vmax = field
        .values
        .iter()
        .cloned()
        .filter(|v| !v.is_nan())
        .fold(f32::MIN, f32::max);
    println!(
        "lightning grid {}x{}  nonzero cells: {}  max density: {:.3} strikes/km2/min  time {}",
        field.nx, field.ny, nonzero, vmax, field.time
    );

    let upload = crate::app::lightning_upload(&field);
    let camera = cam_or_env(-97.0, 38.0, 4.0);
    let (new_tiles, visible, new_vector_tiles, visible_vector, _place_labels) =
        national_basemap(&rt, &camera);
    let (center, scale) = camera.world_to_clip_uniform((size() as f32, size() as f32));
    let cb = MapCallback {
        pane: 0,
        camera_center: center,
        camera_scale: scale,
        basemap_key: 0,
        vector_over_raster: false,
        new_tiles,
        visible,
        radar_upload: None,
        draw_radar: false,
        overlay_upload: None,
        draw_overlay: false,
        field_uploads: vec![(crate::render::FieldLayer::Lightning, upload)],
        field_draws: vec![(crate::render::FieldLayer::Lightning, 1.0)],
        clear_tiles: false,
        drop_tiles: Vec::new(),
        drop_fields: Vec::new(),
        new_vector_tiles,
        visible_vector,
        clear_vector: false,
        drop_vector_tiles: Vec::new(),
        wind_upload: None,
        wind: None,
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
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let field = rt.block_on(async {
        let client = reqwest::Client::new();
        wxdata::mrms::fetch_latest(&client, &product).await
    })?;
    let nonzero = field
        .values
        .iter()
        .filter(|v| !v.is_nan() && v.abs() > 0.0)
        .count();
    let vmax = field
        .values
        .iter()
        .cloned()
        .filter(|v| !v.is_nan())
        .fold(f32::MIN, f32::max);
    println!(
        "{slug} grid {}x{}  nonzero: {}  max: {:.4}  time {}",
        field.nx, field.ny, nonzero, vmax, field.time
    );

    let field = field.decimated(8192); // fit oversized (14000×7000) rotation/AzShear grids
    let upload = crate::app::field_upload_indexed(layer, &field);
    let camera = cam_or_env(-97.0, 38.0, 4.0);
    let (new_tiles, visible, new_vector_tiles, visible_vector, _place_labels) =
        national_basemap(&rt, &camera);
    let (center, scale) = camera.world_to_clip_uniform((size() as f32, size() as f32));
    let cb = MapCallback {
        pane: 0,
        camera_center: center,
        camera_scale: scale,
        basemap_key: 0,
        vector_over_raster: false,
        new_tiles,
        visible,
        radar_upload: None,
        draw_radar: false,
        overlay_upload: None,
        draw_overlay: false,
        field_uploads: vec![(layer, upload)],
        field_draws: vec![(layer, 1.0)],
        clear_tiles: false,
        drop_tiles: Vec::new(),
        drop_fields: Vec::new(),
        new_vector_tiles,
        visible_vector,
        clear_vector: false,
        drop_vector_tiles: Vec::new(),
        wind_upload: None,
        wind: None,
    };
    render_to_png(&rt, cb, out_path)
}

/// Nearest-cell value at a lat/lon, for the spot checks the headless renderers print.
fn spot(f: &wxdata::mrms::MrmsField, lon: f64, lat: f64) -> Option<f32> {
    if f.nx < 2 || f.ny < 2 {
        return None;
    }
    let dx = (f.lon_east - f.lon_west) / (f.nx - 1) as f64;
    let dy = (f.lat_north - f.lat_south) / (f.ny - 1) as f64;
    let col = ((lon - f.lon_west) / dx).round() as isize;
    let row = ((f.lat_north - lat) / dy).round() as isize;
    if col < 0 || row < 0 || col as usize >= f.nx || row as usize >= f.ny {
        return None;
    }
    Some(f.values[row as usize * f.nx + col as usize])
}

/// Render one global-model field. `--headless-global <gfs|ecmwf> <slug> <out.png>`, with the
/// camera from `HOOKECHO_CAM` — which is how the dateline gets checked (a global grid that hasn't
/// been wrapped to −180..180 draws one quad across the entire map).
pub fn run_global(model: &str, slug: &str, out_path: &str) -> anyhow::Result<()> {
    use crate::render::FieldLayer as FL;
    use wxdata::global::{GlobalField, GlobalModel};
    let model = match model {
        "ecmwf" => GlobalModel::Ecmwf,
        "gfs" => GlobalModel::Gfs,
        other => anyhow::bail!("unknown global model '{other}' (gfs|ecmwf)"),
    };
    let gfield = GlobalField::from_slug(slug)
        .ok_or_else(|| anyhow::anyhow!("unknown global field '{slug}'"))?;
    let layer = match gfield {
        GlobalField::Mslp => FL::GlobalMslp,
        GlobalField::Height500 => FL::GlobalHeight500,
        GlobalField::Temp2m => FL::GlobalTemp2m,
        GlobalField::Dewpoint2m => FL::GlobalDewpoint2m,
        GlobalField::Wind10m => FL::GlobalWind10m,
        GlobalField::Precip => FL::GlobalPrecip,
    };
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let fc = rt.block_on(async {
        let client = reqwest::Client::new();
        wxdata::global::fetch(&client, model, gfield, 0).await
    })?;
    let field = fc.field;
    let finite = field.values.iter().filter(|v| v.is_finite()).count();
    let mean = field.values.iter().filter(|v| v.is_finite()).sum::<f32>() / finite.max(1) as f32;
    println!(
        "{} {slug}: {}x{} lon {:.2}..{:.2} lat {:.2}..{:.2} finite {finite} mean {mean:.2} valid {}",
        model.label(),
        field.nx,
        field.ny,
        field.lon_west,
        field.lon_east,
        field.lat_south,
        field.lat_north,
        fc.run
    );
    // Spot values at three well-separated points: a shifted or flipped grid has the right mean
    // and the wrong map, and only a named location catches that.
    for (lon, lat) in [(-97.0, 38.0), (0.0, 51.5), (140.0, -35.0)] {
        println!("  at {lon:.0},{lat:.0}: {:?}", spot(&field, lon, lat));
    }
    anyhow::ensure!(finite > 0, "global field decoded to nothing");
    anyhow::ensure!(
        field.lon_west >= -180.5 && field.lon_east <= 180.5,
        "global grid was not wrapped into -180..180 ({}..{})",
        field.lon_west,
        field.lon_east
    );

    let upload = crate::app::field_upload_indexed(layer, &field);
    let camera = cam_or_env(0.0, 20.0, 2.0);
    let (new_tiles, visible, new_vector_tiles, visible_vector, _place_labels) =
        national_basemap(&rt, &camera);
    let (center, scale) = camera.world_to_clip_uniform((size() as f32, size() as f32));
    let cb = MapCallback {
        pane: 0,
        camera_center: center,
        camera_scale: scale,
        basemap_key: 0,
        vector_over_raster: false,
        new_tiles,
        visible,
        radar_upload: None,
        draw_radar: false,
        overlay_upload: None,
        draw_overlay: false,
        field_uploads: vec![(layer, upload)],
        field_draws: vec![(layer, 1.0)],
        clear_tiles: false,
        drop_tiles: Vec::new(),
        drop_fields: Vec::new(),
        new_vector_tiles,
        visible_vector,
        clear_vector: false,
        drop_vector_tiles: Vec::new(),
        wind_upload: None,
        wind: None,
    };
    render_to_png(&rt, cb, out_path)
}

/// Render the model-difference layer. `--headless-diff <field-slug> <out.png>` — the same two
/// fetches and the same subtract the app does, so the layer is checkable without a window.
pub fn run_diff(slug: &str, out_path: &str) -> anyhow::Result<()> {
    use crate::fielddiff::DiffField;
    use crate::render::FieldLayer as FL;
    use wxdata::global::{GlobalField, GlobalModel};
    let field = DiffField::ALL
        .into_iter()
        .find(|f| f.slug() == slug)
        .ok_or_else(|| anyhow::anyhow!("unknown difference field '{slug}'"))?;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let (a, b, label_a, label_b) = rt.block_on(async {
        let client = reqwest::Client::new();
        match field {
            DiffField::Global(kind) => {
                let g: GlobalField = kind.into();
                let gfs = wxdata::global::fetch(&client, GlobalModel::Gfs, g, 0).await?;
                let ecmwf = wxdata::global::fetch(&client, GlobalModel::Ecmwf, g, 0).await?;
                let (va, vb) = (gfs.valid().to_string(), ecmwf.valid().to_string());
                anyhow::Ok((gfs.field, ecmwf.field, va, vb))
            }
            DiffField::Cape | DiffField::Srh => {
                let (var, level, min_valid) = match field {
                    DiffField::Srh => ("HLCY", "3000-0 m above ground", f64::NEG_INFINITY),
                    _ => ("CAPE", "surface", 0.0),
                };
                let hrrr = wxdata::hrrr::fetch_field(
                    &client,
                    wxdata::hrrr::Model::Hrrr,
                    var,
                    level,
                    0,
                    min_valid,
                )
                .await?;
                let rap = wxdata::hrrr::fetch_field(
                    &client,
                    wxdata::hrrr::Model::Rap,
                    var,
                    level,
                    0,
                    min_valid,
                )
                .await?;
                let (va, vb) = (hrrr.run.to_string(), rap.run.to_string());
                anyhow::Ok((hrrr.field, rap.field, va, vb))
            }
        }
    })?;
    let (na, nb) = field.pair();
    let d = crate::fielddiff::diff(&a, &b)
        .ok_or_else(|| anyhow::anyhow!("the two models cover nothing in common"))?;
    let mut finite: Vec<f32> = d.values.iter().copied().filter(|v| v.is_finite()).collect();
    finite.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    // Percentiles, not just the extremes: two reduction schemes always disagree wildly over high
    // terrain, and the tails say nothing about whether the field is right.
    let (lo, hi) = (
        finite.first().copied().unwrap_or(f32::NAN),
        finite.last().copied().unwrap_or(f32::NAN),
    );
    let pct = |q: f64| finite[((finite.len() - 1) as f64 * q) as usize];
    println!(
        "{na} ({label_a}) − {nb} ({label_b}) {slug}: {}x{} of {} finite, \
         spread {:.2}..{:.2}, middle 90% {:.2}..{:.2}, median {:.2} {}",
        d.nx,
        d.ny,
        finite.len(),
        lo * field.input_scale(),
        hi * field.input_scale(),
        pct(0.05) * field.input_scale(),
        pct(0.95) * field.input_scale(),
        pct(0.50) * field.input_scale(),
        field.units()
    );
    anyhow::ensure!(!finite.is_empty(), "the difference decoded to nothing");

    let (range, deadband) = field.range();
    let scale = field.input_scale();
    let upload = crate::app::field_index_upload(
        &d,
        |v| crate::fielddiff::diff_index(v * scale, range),
        crate::fielddiff::diverging_lut(range, deadband),
    );
    let camera = cam_or_env(-97.0, 38.0, 4.0);
    let (new_tiles, visible, new_vector_tiles, visible_vector, _place_labels) =
        national_basemap(&rt, &camera);
    let (center, scale) = camera.world_to_clip_uniform((size() as f32, size() as f32));
    let cb = MapCallback {
        pane: 0,
        camera_center: center,
        camera_scale: scale,
        basemap_key: 0,
        vector_over_raster: false,
        new_tiles,
        visible,
        radar_upload: None,
        draw_radar: false,
        overlay_upload: None,
        draw_overlay: false,
        field_uploads: vec![(FL::ModelDiff, upload)],
        field_draws: vec![(FL::ModelDiff, 1.0)],
        clear_tiles: false,
        drop_tiles: Vec::new(),
        drop_fields: Vec::new(),
        new_vector_tiles,
        visible_vector,
        clear_vector: false,
        drop_vector_tiles: Vec::new(),
        wind_upload: None,
        wind: None,
    };
    render_to_png(&rt, cb, out_path)
}

/// Fetch + print the active NHC tropical cyclones (feature V). Exits 0 with a note when none.
pub fn run_tropical() -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let data = rt.block_on(async {
        let client = reqwest::Client::new();
        wxdata::tropical::fetch_active(&client).await
    })?;
    if data.storms.is_empty() {
        println!("no active tropical storms");
        return Ok(());
    }
    println!(
        "{} active storm(s), {} cone polygon(s)",
        data.storms.len(),
        data.cones.len()
    );
    for s in &data.storms {
        let (cat, _) = wxdata::tropical::saffir_simpson(s.intensity_kt);
        let mb = s
            .pressure_mb
            .map(|p| format!("{p:.0} mb"))
            .unwrap_or_else(|| "no pressure".into());
        let cone_verts: usize = data
            .cones
            .iter()
            .flat_map(|c| c.rings.iter().map(|r| r.len()))
            .sum();
        println!(
            "  {} ({}) {} — {:.0} kt {} {} at {:.1},{:.1}  {} track pts, {} cone verts",
            s.name,
            s.id,
            s.classification,
            s.intensity_kt,
            cat,
            mb,
            s.lat,
            s.lon,
            s.points.len(),
            cone_verts
        );
    }
    Ok(())
}

/// Fetch + print every SPC outlook day, so a change to the Day 1-3 or the Day 4-8 URL forms
/// fails here rather than as an empty map: `hookecho --headless-outlooks`.
pub fn run_outlooks() -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let client = reqwest::Client::new();
    for day in 1u8..=8 {
        let feats = rt.block_on(wxdata::spc::fetch_outlook(&client, day));
        match feats {
            Ok(f) => {
                let labels: Vec<&str> = f.iter().map(|x| x.title.as_str()).take(4).collect();
                println!(
                    "day {day}: {} polygon(s) {}",
                    f.len(),
                    if labels.is_empty() {
                        "(nothing outlooked)".to_string()
                    } else {
                        format!("— {}", labels.join(", "))
                    }
                );
            }
            // A day with no product yet is a 404, which is data rather than a bug; anything else
            // is worth the non-zero exit.
            Err(e) => println!("day {day}: {e}"),
        }
    }
    Ok(())
}

/// Fetch + print surface obs (METAR) near a site (feature U).
pub fn run_metar(site: &str) -> anyhow::Result<()> {
    let s =
        wxdata::sites::site_by_id(site).ok_or_else(|| anyhow::anyhow!("unknown site {site}"))?;
    let (lat, lon) = (s.latitude as f64, s.longitude as f64);
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let obs = rt.block_on(async {
        let client = reqwest::Client::new();
        wxdata::metar::fetch_bbox(&client, lat - 2.5, lon - 2.5, lat + 2.5, lon + 2.5).await
    })?;
    let tafs = rt.block_on(async {
        let client = reqwest::Client::new();
        wxdata::metar::fetch_tafs(&client, lat - 2.5, lon - 2.5, lat + 2.5, lon + 2.5).await
    })?;
    println!(
        "{site}: {} surface obs, {} terminal forecasts within ±2.5°",
        obs.len(),
        tafs.len()
    );
    if let Some((icao, taf)) = tafs.iter().next() {
        println!("  TAF {icao}: {taf}");
    }
    for ob in obs.iter().take(3) {
        println!(
            "  {:<5} {:>6.2},{:>7.2}  {}kt @ {}  T {} Td {}  [{}]",
            ob.icao,
            ob.lat,
            ob.lon,
            ob.wspd_kt,
            ob.wdir_deg
                .map(|d| format!("{d:.0}"))
                .unwrap_or_else(|| "VRB".into()),
            ob.temp_c
                .map(|t| format!("{t:.0}C"))
                .unwrap_or_else(|| "—".into()),
            ob.dewp_c
                .map(|t| format!("{t:.0}C"))
                .unwrap_or_else(|| "—".into()),
            ob.flt_cat,
        );
    }
    Ok(())
}

/// Fetch NWPS river gauges within ±2.5° of a site, print the count + a few samples (worst-first).
pub fn run_gauges(site: &str) -> anyhow::Result<()> {
    let s =
        wxdata::sites::site_by_id(site).ok_or_else(|| anyhow::anyhow!("unknown site {site}"))?;
    let (lat, lon) = (s.latitude as f64, s.longitude as f64);
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let gauges = rt.block_on(async {
        let client = reqwest::Client::new();
        wxdata::river::fetch_bbox(&client, lat - 2.5, lon - 2.5, lat + 2.5, lon + 2.5).await
    })?;
    println!("{site}: {} river gauges within ±2.5°", gauges.len());
    for g in gauges.iter().take(3) {
        println!(
            "  {:<6} {:>6.2},{:>7.2}  {:>7}  {:?}  {}",
            g.lid,
            g.lat,
            g.lon,
            g.stage_ft
                .map(|v| format!("{v:.1}ft"))
                .unwrap_or_else(|| "—".into()),
            g.cat,
            g.name,
        );
    }
    Ok(())
}

/// Offline chase-pack verifier: pre-download the raster basemap around `(lat, lon)` within
/// `radius_km` over a 4-level zoom span ending at `zmax`, then print how many tiles were cached vs
/// fetched. Re-running should report everything cached (the read-through disk cache is transparent).
pub fn run_chasepack(
    lat: f64,
    lon: f64,
    radius_km: f64,
    zmax: u8,
    style_slug: &str,
) -> anyhow::Result<()> {
    use crate::tiles::{start_pack_download, BasemapStyle, TileManager};
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let style = BasemapStyle::from_slug(style_slug);
    let dlat = radius_km / 111.0;
    let dlon = radius_km / (111.0 * lat.to_radians().cos().abs().max(0.01));
    let (min_lon, min_lat, max_lon, max_lat) = (lon - dlon, lat - dlat, lon + dlon, lat + dlat);
    let z_lo = zmax.saturating_sub(4).max(2);
    let mgr = TileManager::new(crate::rt::Spawner::new(rt.handle().clone()));
    let jobs = mgr.pack_jobs(style, min_lon, min_lat, max_lon, max_lat, z_lo, zmax);
    let total = jobs.len();
    if total == 0 {
        anyhow::bail!(
            "no jobs for style '{style_slug}' (pick a raster basemap slug, e.g. carto-dark)"
        );
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    start_pack_download(rt.handle(), jobs, cancel, tx);
    let (mut cached, mut fetched, mut errors, mut bytes) = (0u64, 0u64, 0u64, 0u64);
    for _ in 0..total {
        match rx.recv() {
            Ok((true, 0)) => cached += 1,
            Ok((true, n)) => {
                fetched += 1;
                bytes += n;
            }
            Ok((false, _)) => errors += 1,
            Err(_) => break,
        }
    }
    let mb = bytes as f64 / 1e6;
    println!("{total} tiles (z{z_lo}-{zmax}, {style_slug}): {cached} cached, {fetched} fetched, {errors} errors, {mb:.1} MB");
    Ok(())
}

/// HRRR contour verifier: fetch a surface field, contour it, and print the line count / level
/// range / longest polyline / valid time. No PNG — the overlay is painter-based, so line counts
/// exercise the whole fetch→regrid→marching-squares pipeline without a GPU.
pub fn run_contours(kind_token: &str) -> anyhow::Result<()> {
    let kind = crate::app::ContourKind::from_token(kind_token).ok_or_else(|| {
        anyhow::anyhow!("unknown contour kind '{kind_token}' (mslp|t2m|td2m|cape|srh|stp|scp|ehi)")
    })?;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let temp_unit = crate::settings::TempUnit::Fahrenheit;
    let interval = kind.interval(temp_unit);
    let (lines, valid) = rt.block_on(async {
        let client = reqwest::Client::new();
        // Composite parameters are built from several same-run fields; single fields fetch directly.
        let mut fc = match kind.severe() {
            Some(sk) => wxdata::severe::fetch_grid(&client, wxdata::hrrr::Model::Hrrr, sk).await?,
            None => {
                let (var, level, _) = kind.params().expect("non-Off kind has params");
                wxdata::hrrr::fetch_field(
                    &client,
                    wxdata::hrrr::Model::Hrrr,
                    var,
                    level,
                    0,
                    f64::NEG_INFINITY,
                )
                .await?
            }
        };
        for v in &mut fc.field.values {
            if v.is_finite() {
                *v = kind.to_display(*v, temp_unit);
            }
        }
        let valid = fc.valid();
        anyhow::Ok((wxdata::contour::contour_lines(&fc.field, interval), valid))
    })?;
    let n = lines.len();
    let longest = lines.iter().map(|l| l.pts.len()).max().unwrap_or(0);
    let (lo, hi) = lines
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), l| {
            (a.min(l.level), b.max(l.level))
        });
    println!(
        "{kind_token}: {n} lines, levels {lo}-{hi} step {interval}, longest {longest} pts, valid {}",
        valid.format("%H:%MZ")
    );
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
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
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
    let vmax = field
        .values
        .iter()
        .cloned()
        .filter(|v| !v.is_nan())
        .fold(f32::MIN, f32::max);
    println!(
        "{kind} {site} grid {}x{}  lon[{:.2},{:.2}] lat[{:.2},{:.2}]  filled {}  max {:.2}",
        field.nx,
        field.ny,
        field.lon_west,
        field.lon_east,
        field.lat_south,
        field.lat_north,
        filled,
        vmax
    );
    let (clon, clat) = (
        (field.lon_west + field.lon_east) * 0.5,
        (field.lat_north + field.lat_south) * 0.5,
    );
    let upload = crate::app::field_upload_indexed(layer, &field);
    let camera = cam_or_env(clon, clat, 7.0);
    let (new_tiles, visible, new_vector_tiles, visible_vector, _place_labels) =
        national_basemap(&rt, &camera);
    let (center, scale) = camera.world_to_clip_uniform((size() as f32, size() as f32));
    let cb = MapCallback {
        pane: 0,
        camera_center: center,
        camera_scale: scale,
        basemap_key: 0,
        vector_over_raster: false,
        new_tiles,
        visible,
        radar_upload: None,
        draw_radar: false,
        overlay_upload: None,
        draw_overlay: false,
        field_uploads: vec![(layer, upload)],
        field_draws: vec![(layer, 1.0)],
        clear_tiles: false,
        drop_tiles: Vec::new(),
        drop_fields: Vec::new(),
        new_vector_tiles,
        visible_vector,
        clear_vector: false,
        drop_vector_tiles: Vec::new(),
        wind_upload: None,
        wind: None,
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
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let fc = rt.block_on(async {
        let client = reqwest::Client::new();
        wxdata::hrrr::fetch_field(&client, wxdata::hrrr::Model::Hrrr, var, level, 0, min_valid)
            .await
    })?;
    let f = &fc.field;
    let filled = f.values.iter().filter(|v| !v.is_nan()).count();
    let vmax = f
        .values
        .iter()
        .cloned()
        .filter(|v| !v.is_nan())
        .fold(f32::MIN, f32::max);
    let vmin = f
        .values
        .iter()
        .cloned()
        .filter(|v| !v.is_nan())
        .fold(f32::MAX, f32::min);
    println!(
        "{slug} ({var}:{level}) regrid {}x{}  filled {}  range [{:.1},{:.1}]  run {} valid {}",
        f.nx,
        f.ny,
        filled,
        vmin,
        vmax,
        fc.run.format("%Y-%m-%d %HZ"),
        fc.valid().format("%Y-%m-%d %H:%MZ")
    );

    let upload = crate::app::field_upload_indexed(layer, f);
    let camera = cam_or_env(-97.0, 38.0, 4.0);
    let (new_tiles, visible, new_vector_tiles, visible_vector, _place_labels) =
        national_basemap(&rt, &camera);
    let (center, scale) = camera.world_to_clip_uniform((size() as f32, size() as f32));
    let cb = MapCallback {
        pane: 0,
        camera_center: center,
        camera_scale: scale,
        basemap_key: 0,
        vector_over_raster: false,
        new_tiles,
        visible,
        radar_upload: None,
        draw_radar: false,
        overlay_upload: None,
        draw_overlay: false,
        field_uploads: vec![(layer, upload)],
        field_draws: vec![(layer, 1.0)],
        clear_tiles: false,
        drop_tiles: Vec::new(),
        drop_fields: Vec::new(),
        new_vector_tiles,
        visible_vector,
        clear_vector: false,
        drop_vector_tiles: Vec::new(),
        wind_upload: None,
        wind: None,
    };
    render_to_png(&rt, cb, out_path)
}

/// Fetch + regrid an HRRR reflectivity forecast for `fcst_hour`, print stats, render over CONUS.
pub fn run_hrrr(fcst_hour: u8, out_path: &str) -> anyhow::Result<()> {
    run_hrrr_layer(crate::render::FieldLayer::Hrrr, fcst_hour, out_path)
}

/// Render any HRRR-backed field layer (future radar, rotation tracks, smoke) for `fcst_hour`,
/// printing grid stats first — decoded ranges are how a units mistake gets caught before it
/// reaches the map.
pub fn run_hrrr_layer(
    layer: crate::render::FieldLayer,
    fcst_hour: u8,
    out_path: &str,
) -> anyhow::Result<()> {
    use crate::render::{mercator::lonlat_to_world, FieldLayer, MrmsUpload};
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let fc = rt.block_on(async {
        let client = reqwest::Client::new();
        match layer {
            FieldLayer::UpdraftHelicity => {
                wxdata::hrrr::fetch_field_swath(
                    &client,
                    "MXUPHL",
                    "5000-2000 m above ground",
                    fcst_hour.max(1),
                    0.0,
                )
                .await
            }
            FieldLayer::Smoke => {
                wxdata::hrrr::fetch_field(
                    &client,
                    wxdata::hrrr::Model::Hrrr,
                    "MASSDEN",
                    "8 m above ground",
                    fcst_hour,
                    0.0,
                )
                .await
            }
            _ => wxdata::hrrr::fetch_forecast(&client, fcst_hour).await,
        }
    })?;
    let f = &fc.field;
    if layer != FieldLayer::Hrrr {
        let vmax = f
            .values
            .iter()
            .cloned()
            .filter(|v| !v.is_nan())
            .fold(f32::MIN, f32::max);
        let ramp = crate::render::field_ramps::ramp_for(layer);
        println!(
            "{:?} F+{}h  {}x{}  filled {}  max {:.3}{}  run {}",
            layer,
            fc.fcst_hour,
            f.nx,
            f.ny,
            f.values.iter().filter(|v| !v.is_nan()).count(),
            ramp.map_or(vmax, |r| r.display(vmax)),
            ramp.map(|r| format!(" {}", r.units)).unwrap_or_default(),
            fc.run.format("%Y-%m-%d %HZ"),
        );
        let data: Vec<u8> = f
            .values
            .iter()
            .map(|&v| ramp.map_or(0, |r| if v.is_nan() { 0 } else { r.index(v) }))
            .collect();
        let lut = match ramp.map(|r| &r.scale) {
            Some(crate::render::field_ramps::FieldScale::Ramp { stops, .. }) => {
                crate::render::field_ramps::bake_ramp_lut(stops, ramp.unwrap().alpha)
            }
            _ => vec![0u8; 256 * 4],
        };
        let (wx0, wy0) = lonlat_to_world(f.lon_west, f.lat_north);
        let (wx1, wy1) = lonlat_to_world(f.lon_east, f.lat_south);
        let upload = MrmsUpload {
            data,
            nx: f.nx as u32,
            ny: f.ny as u32,
            world_min: [wx0 as f32, wy0 as f32],
            world_max: [wx1 as f32, wy1 as f32],
            uniform: [
                f.lon_west as f32,
                f.lat_north as f32,
                f.lon_east as f32,
                f.lat_south as f32,
                f.nx as f32,
                f.ny as f32,
                0.0,
                0.0,
                0.0,
                0.0,
                0.0,
                0.0,
            ],
            lut,
        };
        return render_field_png(&rt, layer, upload, out_path);
    }
    let valid = f.values.iter().filter(|v| !v.is_nan()).count();
    let vmax = f
        .values
        .iter()
        .cloned()
        .filter(|v| !v.is_nan())
        .fold(f32::MIN, f32::max);
    println!(
        "HRRR F+{}h regrid {}x{}  lon[{:.1},{:.1}] lat[{:.1},{:.1}]  filled {}  max {:.1} dBZ  run {} valid {}",
        fc.fcst_hour, f.nx, f.ny, f.lon_west, f.lon_east, f.lat_south, f.lat_north, valid, vmax,
        fc.run.format("%Y-%m-%d %HZ"), fc.valid().format("%Y-%m-%d %H:%MZ")
    );

    // Reflectivity index mapping (mirrors app::mrms_upload) with the default REF palette.
    let (vmin, vspan_max) = Moment::Reflectivity.value_range();
    let span = (vspan_max - vmin).max(f32::EPSILON);
    let data: Vec<u8> = f
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
    let (wx0, wy0) = lonlat_to_world(f.lon_west, f.lat_north);
    let (wx1, wy1) = lonlat_to_world(f.lon_east, f.lat_south);
    let upload = MrmsUpload {
        data,
        nx: f.nx as u32,
        ny: f.ny as u32,
        world_min: [wx0 as f32, wy0 as f32],
        world_max: [wx1 as f32, wy1 as f32],
        uniform: [
            f.lon_west as f32,
            f.lat_north as f32,
            f.lon_east as f32,
            f.lat_south as f32,
            f.nx as f32,
            f.ny as f32,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
        ],
        lut: crate::colormap::bake_lut(table, (vmin, vspan_max), None).to_vec(),
    };
    let camera = cam_or_env(-97.0, 38.0, 4.0);
    let (new_tiles, visible, new_vector_tiles, visible_vector, _place_labels) =
        national_basemap(&rt, &camera);
    let (center, scale) = camera.world_to_clip_uniform((size() as f32, size() as f32));
    let cb = MapCallback {
        pane: 0,
        camera_center: center,
        camera_scale: scale,
        basemap_key: 0,
        vector_over_raster: false,
        new_tiles,
        visible,
        radar_upload: None,
        draw_radar: false,
        overlay_upload: None,
        draw_overlay: false,
        field_uploads: vec![(FieldLayer::Hrrr, upload)],
        field_draws: vec![(FieldLayer::Hrrr, 1.0)],
        clear_tiles: false,
        drop_tiles: Vec::new(),
        drop_fields: Vec::new(),
        new_vector_tiles,
        visible_vector,
        clear_vector: false,
        drop_vector_tiles: Vec::new(),
        wind_upload: None,
        wind: None,
    };
    render_to_png(&rt, cb, out_path)
}

/// Draw one prepared gridded field over the national basemap and save it.
fn render_field_png(
    rt: &tokio::runtime::Runtime,
    layer: crate::render::FieldLayer,
    upload: crate::render::MrmsUpload,
    out_path: &str,
) -> anyhow::Result<()> {
    let camera = cam_or_env(-97.0, 38.0, 4.0);
    let (new_tiles, visible, new_vector_tiles, visible_vector, _place_labels) =
        national_basemap(rt, &camera);
    let (center, scale) = camera.world_to_clip_uniform((size() as f32, size() as f32));
    let cb = MapCallback {
        pane: 0,
        camera_center: center,
        camera_scale: scale,
        basemap_key: 0,
        vector_over_raster: false,
        new_tiles,
        visible,
        radar_upload: None,
        draw_radar: false,
        overlay_upload: None,
        draw_overlay: false,
        field_uploads: vec![(layer, upload)],
        field_draws: vec![(layer, 1.0)],
        clear_tiles: false,
        drop_tiles: Vec::new(),
        drop_fields: Vec::new(),
        new_vector_tiles,
        visible_vector,
        clear_vector: false,
        drop_vector_tiles: Vec::new(),
        wind_upload: None,
        wind: None,
    };
    render_to_png(rt, cb, out_path)
}

/// Reconstruct a vertical cross-section for `site` along `a`→`b` (`(lon,lat)`) and save the panel
/// PNG. Prints coverage stats.
pub fn run_xsection(
    site: &str,
    a: (f64, f64),
    b: (f64, f64),
    out_path: &str,
) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
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
        sweeps.len(),
        xs.cols,
        xs.rows,
        xs.length_km,
        filled,
        xs.cols * xs.rows,
        vmax
    );

    let table = crate::colormap::default_table(Moment::Reflectivity);
    let img = crate::ui::xsection_window::to_image(&xs, table);
    let buf: Vec<u8> = img
        .pixels
        .iter()
        .flat_map(|p| [p.r(), p.g(), p.b(), p.a()])
        .collect();
    image::save_buffer(
        out_path,
        &buf,
        xs.cols as u32,
        xs.rows as u32,
        image::ColorType::Rgba8,
    )?;
    println!("wrote {out_path}");
    Ok(())
}

/// Build the 3D reflectivity volume for `site` and raymarch it from a fixed orbit camera to a PNG.
/// Fetch a volume, slice a CAPPI at `alt_km`, print filled-cell count, and save the slice PNG.
pub fn run_cappi(site: &str, alt_km: f32, out_path: &str) -> anyhow::Result<()> {
    const N: usize = 256;
    const HALF_KM: f32 = 150.0;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
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
    println!(
        "CAPPI {site} @ {alt_km:.1} km  {N}x{N}  filled {}/{}",
        filled,
        c.dbz.len()
    );
    let table = crate::colormap::default_table(Moment::Reflectivity);
    let img = crate::ui::cappi_window::to_image(&c, table);
    let rgba: Vec<u8> = img
        .pixels
        .iter()
        .flat_map(|p| [p.r(), p.g(), p.b(), p.a()])
        .collect();
    image::save_buffer(out_path, &rgba, N as u32, N as u32, image::ColorType::Rgba8)?;
    println!("wrote {out_path}");
    Ok(())
}

pub fn run_3d(site: &str, out_path: &str, threshold_dbz: Option<f32>) -> anyhow::Result<()> {
    const N: usize = 192;
    const NZ: usize = 48;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
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
        sweeps.len(),
        v3.n,
        v3.n,
        v3.nz,
        filled,
        v3.data.len()
    );

    let table = crate::colormap::default_table(Moment::Reflectivity);
    let (v3_min, v3_max) = (v3.value_min, v3.value_max);
    let lut = crate::colormap::bake_lut(table, (v3_min, v3_max), None).to_vec();
    let upload = crate::render3d::Volume3dUpload {
        data: v3.data,
        n: v3.n as u32,
        nz: v3.nz as u32,
        lut,
    };
    let view = crate::render3d::View3d {
        threshold_idx: match threshold_dbz {
            Some(dbz) => crate::render3d::threshold_index(dbz, (v3_min, v3_max)),
            None => 2.0,
        },
        ..Default::default()
    };
    let uniform =
        crate::render3d::orbit_uniform(30.0, 25.0, 3.0, 1.0, N as u32, NZ as u32, 256, view);

    let (device, queue, adapter) = init_gpu(&rt)?;
    println!("adapter: {}", adapter.get_info().name);
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let mut res = crate::render3d::Volume3dResources::new(&device, format);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("headless_3d_target"),
        size: wgpu::Extent3d {
            width: size(),
            height: size(),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    res.render_once(
        &device,
        &queue,
        &view,
        &upload,
        uniform,
        wgpu::Color {
            r: 0.03,
            g: 0.03,
            b: 0.05,
            a: 1.0,
        },
    );

    let rgba = read_target(&device, &queue, &target, size());
    image::save_buffer(out_path, &rgba, size(), size(), image::ColorType::Rgba8)?;
    // Echo pixels = those differing from the uniform sRGB background clear (~48,48,63).
    let echo = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| {
            (p[0] as i16 - 48).abs() + (p[1] as i16 - 48).abs() + (p[2] as i16 - 63).abs() > 30
        })
        .count();
    println!("wrote {out_path}  ({echo} echo pixels over background)");
    // With no threshold set, an empty image means a broken pipeline rather than a quiet day: the
    // volume above already reported filled voxels. With one set, empty is a legitimate answer —
    // there may simply be no 45 dBZ core out there.
    if echo == 0 && threshold_dbz.is_none() {
        anyhow::bail!("raymarch produced no echo pixels");
    }
    Ok(())
}

/// Fetch + print today's SPC storm reports (textual gate; markers are painter-drawn).
pub fn run_reports(window: Option<(&str, &str)>) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
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
    let span = window
        .map(|(a, b)| format!("{a}..{b}"))
        .unwrap_or_else(|| "last 6 h".into());
    println!(
        "LSRs ({span}): {} total ({t} tornado, {w} wind, {h} hail, {f} flood, {o} other)",
        reports.len()
    );
    for r in reports.iter().take(5) {
        println!(
            "  {} {} @ {:.2},{:.2} — {} {}",
            r.kind.label(),
            r.magnitude,
            r.lat,
            r.lon,
            r.location,
            r.state
        );
    }
    Ok(())
}

/// AFD verify: fetch + print the head of the active-site WFO discussion (feature DD).
pub fn run_afd(site: &str) -> anyhow::Result<()> {
    let s =
        wxdata::sites::site_by_id(site).ok_or_else(|| anyhow::anyhow!("unknown site {site}"))?;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let afd = rt.block_on(async {
        let client = reqwest::Client::new();
        wxdata::afd::fetch(&client, s.latitude as f64, s.longitude as f64).await
    })?;
    println!(
        "AFD {} issued {} — {} chars",
        afd.office,
        afd.issued,
        afd.text.len()
    );
    for line in afd.text.lines().take(12) {
        println!("  {line}");
    }
    Ok(())
}

/// Aviation verify: fetch SIGMETs/AIRMETs, print per-hazard tallies (feature GG).
pub fn run_aviation() -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
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
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let s = rt.block_on(async {
        let client = reqwest::Client::new();
        wxdata::sounding::fetch(&client, lon, lat).await
    })?;
    let ix = s
        .indices()
        .ok_or_else(|| anyhow::anyhow!("profile too short for indices"))?;
    println!(
        "indices @ {lat:.2},{lon:.2} (run {}): SBCAPE {:.0} J/kg  LCL {:.0} m  SRH1 {:.0}  SRH3 {:.0}  shear6 {:.0} kt  SCP {:.1}  STP {:.1}  EHI1 {:.1}",
        s.run.format("%m/%d %H:%MZ"), ix.sbcape, ix.lcl_m, ix.srh1, ix.srh3, ix.shear6_kt, ix.scp, ix.stp, ix.ehi1
    );
    Ok(())
}

/// Tornado-climatology verify: download the SPC database, query near a point, print counts.
pub fn run_climatology(lon: f64, lat: f64) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let tracks = rt.block_on(async {
        let client = reqwest::Client::new();
        wxdata::torclimo::fetch_tracks(&client).await
    })?;
    println!("Loaded {} tornado tracks (1950–2022)", tracks.len());
    let hits = wxdata::torclimo::near(&tracks, lon, lat, 40.0);
    let hist = wxdata::torclimo::mag_histogram(&hits);
    println!(
        "Within 25 mi of {lat:.3},{lon:.3}: {} tornadoes",
        hits.len()
    );
    println!(
        "  EF0:{} EF1:{} EF2:{} EF3:{} EF4:{} EF5:{} Unk:{}",
        hist[0], hist[1], hist[2], hist[3], hist[4], hist[5], hist[6]
    );
    for t in hits.iter().take(5) {
        let m = if t.mag < 0 {
            "EF?".to_string()
        } else {
            format!("EF{}", t.mag)
        };
        println!("  {} {} @ {:.2},{:.2}", t.year, m, t.slat, t.slon);
    }
    Ok(())
}

/// ProbSevere verify: fetch the latest FeatureCollection, print storm count + top probabilities.
pub fn run_probsevere() -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let feats = rt.block_on(async {
        let client = reqwest::Client::new();
        wxdata::probsevere::fetch_probsevere(&client).await
    })?;
    println!("ProbSevere storms: {}", feats.len());
    let mut sorted: Vec<_> = feats.iter().collect();
    sorted.sort_by_key(|f| {
        std::cmp::Reverse(
            f.title
                .trim_end_matches('%')
                .rsplit(' ')
                .next()
                .and_then(|s| s.parse::<u8>().ok())
                .unwrap_or(0),
        )
    });
    for f in sorted.iter().take(5) {
        let c = f
            .rings
            .first()
            .and_then(|r| r.first())
            .copied()
            .unwrap_or([0.0, 0.0]);
        println!("  {} @ {:.2},{:.2}", f.title, c[1], c[0]);
    }
    Ok(())
}

/// Spotter Network verify: fetch, then apply the same 230 km site filter the map painter uses.
pub fn run_spotters(site: &str) -> anyhow::Result<()> {
    let s =
        wxdata::sites::site_by_id(site).ok_or_else(|| anyhow::anyhow!("unknown site {site}"))?;
    let site_pos = [s.longitude as f64, s.latitude as f64];
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
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
                sp.name,
                sp.lat,
                sp.lon,
                sp.status,
                sp.heading
                    .map(|h| format!(", heading {h:.0}°"))
                    .unwrap_or_default(),
            );
            printed += 1;
        }
    }
    println!(
        "Spotter Network: {} total, {near} within 230 km of {site} ({movers} moving)",
        spotters.len()
    );
    anyhow::ensure!(
        !format!("{spotters:?}").contains('@'),
        "email leaked into parsed spotters"
    );
    Ok(())
}

/// Create a headless GPU device/queue.
fn init_gpu(
    rt: &tokio::runtime::Runtime,
) -> anyhow::Result<(wgpu::Device, wgpu::Queue, wgpu::Adapter)> {
    let instance = wgpu::Instance::default();
    let adapter = rt.block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        // `HOOKECHO_GPU_FALLBACK=1` forces the software adapter (lavapipe in CI) so the
        // golden-image test compares the same rasterizer everywhere.
        force_fallback_adapter: std::env::var("HOOKECHO_GPU_FALLBACK").is_ok(),
        compatible_surface: None,
    }))?;
    let (device, queue) =
        rt.block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))?;
    Ok((device, queue, adapter))
}

/// Read a `size×size` RGBA render target back to a tightly-packed byte vec.
fn read_target(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &wgpu::Texture,
    size: u32,
) -> Vec<u8> {
    let bytes_per_pixel = 4u32;
    let unpadded = size * bytes_per_pixel;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded = unpadded.div_ceil(align) * align;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (padded * size) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(size),
            },
        },
        wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));
    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    let _ = rx.recv();
    let mapped = slice.get_mapped_range();
    let mut rgba = Vec::with_capacity((unpadded * size) as usize);
    for row in 0..size {
        let start = (row * padded) as usize;
        rgba.extend_from_slice(&mapped[start..start + unpadded as usize]);
    }
    drop(mapped);
    buffer.unmap();
    rgba
}

fn new_target(device: &wgpu::Device, format: wgpu::TextureFormat, size: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("headless_target"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

/// Draw one prepared pane to a fresh target and read it back.
fn draw_and_read(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    res: &RenderResources,
    pane: u32,
) -> Vec<u8> {
    let target = new_target(device, wgpu::TextureFormat::Rgba8UnormSrgb, size());
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    res.draw_pane(
        device,
        queue,
        &view,
        pane,
        wgpu::Color {
            r: 0.05,
            g: 0.05,
            b: 0.08,
            a: 1.0,
        },
    );
    read_target(device, queue, &target, size())
}

fn save_rgba(rgba: &[u8], out_path: &str) -> anyhow::Result<()> {
    image::save_buffer(out_path, rgba, size(), size(), image::ColorType::Rgba8)?;
    println!("wrote {out_path}");
    Ok(())
}

/// Shared: create an offscreen GPU, render `cb`, read the target back, and save a PNG.
fn render_to_png(
    rt: &tokio::runtime::Runtime,
    cb: MapCallback,
    out_path: &str,
) -> anyhow::Result<()> {
    render_to_png_stamped(rt, cb, out_path, None)
}

/// As [`render_to_png`], with `stamp` painted onto the pixels after readback and before the file
/// is written. The chrome is drawn on the CPU precisely so it cannot vary with the adapter — see
/// `chrome.rs`.
fn render_to_png_stamped(
    rt: &tokio::runtime::Runtime,
    cb: MapCallback,
    out_path: &str,
    stamp: Option<&crate::chrome::Stamp>,
) -> anyhow::Result<()> {
    let (device, queue, adapter) = init_gpu(rt)?;
    println!("adapter: {}", adapter.get_info().name);

    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let mut res = RenderResources::new(&device, format);

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("headless_target"),
        size: wgpu::Extent3d {
            width: size(),
            height: size(),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    res.render_once(
        &device,
        &queue,
        &view,
        &cb,
        wgpu::Color {
            r: 0.05,
            g: 0.05,
            b: 0.08,
            a: 1.0,
        },
    );

    let bytes_per_pixel = 4u32;
    let unpadded = size() * bytes_per_pixel;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded = unpadded.div_ceil(align) * align;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (padded * size()) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
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
                rows_per_image: Some(size()),
            },
        },
        wgpu::Extent3d {
            width: size(),
            height: size(),
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));

    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv()??;

    let mapped = slice.get_mapped_range();
    let mut rgba = Vec::with_capacity((unpadded * size()) as usize);
    for row in 0..size() {
        let start = (row * padded) as usize;
        rgba.extend_from_slice(&mapped[start..start + unpadded as usize]);
    }
    drop(mapped);
    buffer.unmap();

    if let Some(stamp) = stamp {
        crate::chrome::draw(&mut rgba, size(), size(), stamp);
    }
    image::save_buffer(out_path, &rgba, size(), size(), image::ColorType::Rgba8)?;
    println!("wrote {out_path}");
    Ok(())
}

// ponytail: one golden scene; per-moment/per-palette goldens if renderer churn starts
// eating bisects.
#[cfg(test)]
mod golden_tests {
    use super::*;
    use wxdata::level2::BinnedSweep;

    /// Small so the checked-in golden stays tens of KB.
    const GOLDEN_SIZE: u32 = 200;
    const GOLDEN: &str = "tests/golden/snapshot_base.png";

    /// A deterministic synthetic sweep: a 90° wedge plus three range rings.
    fn synthetic_sweep() -> BinnedSweep {
        let (az_bins, gate_count) = (360usize, 200usize);
        let mut data = vec![0u8; az_bins * gate_count];
        for az in 0..az_bins {
            for g in 0..gate_count {
                // Raw 0/1 are "no data"; 2..=255 map across value_min..value_max.
                let v = if (45..135).contains(&az) {
                    // Wedge: ramp outward so the colormap is exercised end to end.
                    2 + (g * 253 / gate_count) as u8
                } else if g % 50 < 3 {
                    120
                } else {
                    0
                };
                data[az * gate_count + g] = v;
            }
        }
        BinnedSweep {
            moment: Moment::Reflectivity,
            az_bins,
            gate_count,
            data,
            first_gate_km: 2.125,
            gate_interval_km: 0.25,
            radar_lat: 35.0,
            radar_lon: -97.0,
            elevation_deg: 0.5,
            value_min: -32.0,
            value_max: 95.0,
        }
    }

    /// A loop frame and a palette drag must not rebuild GPU state that only ever changes shape.
    ///
    /// Ten renders of the same-shaped sweep plus five palette generations: one texture set and
    /// one tile-quad list, not fifteen of each. Run under lavapipe like the golden below.
    #[test]
    #[ignore = "gpu"]
    fn a_loop_and_a_palette_drag_rebuild_nothing() {
        use wxdata::stats::Counter;
        let sweep = synthetic_sweep();
        let table = crate::colormap::default_table(Moment::Reflectivity).clone();
        let camera = Camera::at_lonlat(sweep.radar_lon as f64, sweep.radar_lat as f64, 8.5);
        let (center, scale) =
            camera.world_to_clip_uniform((GOLDEN_SIZE as f32, GOLDEN_SIZE as f32));
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let Ok((device, queue, _adapter)) = init_gpu(&rt) else {
            println!("SKIP: no wgpu adapter");
            return;
        };
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let mut res = RenderResources::new(&device, format);
        let target = new_target(&device, format, GOLDEN_SIZE);
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());

        let counter = |c: Counter| {
            let label = wxdata::stats::Counter::LABELS[c as usize];
            wxdata::stats::snapshot()
                .into_iter()
                .find(|(l, _)| *l == label)
                .expect("counter")
                .1
        };
        let before = (
            counter(Counter::RadarTexturesBuilt),
            counter(Counter::TileQuadsBuilt),
        );

        // Ten loop frames (a new sweep each time, same shape), then five LUT-only uploads.
        for i in 0..15 {
            let lut_only = i >= 10;
            let mut cb = MapCallback {
                pane: 0,
                camera_center: center,
                camera_scale: scale,
                basemap_key: 0,
                vector_over_raster: false,
                new_tiles: Vec::new(),
                visible: Vec::new(),
                radar_upload: Some(crate::app::to_upload(
                    &sweep, &table, None, false, None, None, lut_only,
                )),
                draw_radar: true,
                overlay_upload: None,
                draw_overlay: false,
                field_uploads: Vec::new(),
                field_draws: Vec::new(),
                clear_tiles: false,
                drop_tiles: Vec::new(),
                drop_fields: Vec::new(),
                new_vector_tiles: Vec::new(),
                visible_vector: Vec::new(),
                clear_vector: false,
                drop_vector_tiles: Vec::new(),
                wind_upload: None,
                wind: None,
            };
            cb.draw_radar = true;
            res.render_once(&device, &queue, &view, &cb, wgpu::Color::BLACK);
        }
        let built = counter(Counter::RadarTexturesBuilt) - before.0;
        let quads = counter(Counter::TileQuadsBuilt) - before.1;
        println!("radar_textures_built={built} tile_quads_built={quads} over 15 frames");
        assert_eq!(
            built, 1,
            "same-shaped sweeps write into the retained textures"
        );
        assert_eq!(quads, 1, "a still camera keeps its tile quads");
    }

    /// Golden-image test for the radar render pipeline. Run with
    /// `HOOKECHO_GPU_FALLBACK=1 cargo test -p hookecho -- --ignored gpu` so the
    /// software (lavapipe) adapter is used — the golden is authored under lavapipe.
    #[test]
    #[ignore = "gpu"]
    fn gpu_golden_radar_snapshot() {
        let sweep = synthetic_sweep();
        let table = crate::colormap::default_table(Moment::Reflectivity).clone();
        let camera = Camera::at_lonlat(sweep.radar_lon as f64, sweep.radar_lat as f64, 8.5);
        let (center, scale) =
            camera.world_to_clip_uniform((GOLDEN_SIZE as f32, GOLDEN_SIZE as f32));
        let cb = MapCallback {
            pane: 0,
            camera_center: center,
            camera_scale: scale,
            basemap_key: 0,
            vector_over_raster: false,
            new_tiles: Vec::new(),
            visible: Vec::new(),
            radar_upload: Some(crate::app::to_upload(
                &sweep, &table, None, false, None, None, false,
            )),
            draw_radar: true,
            overlay_upload: None,
            draw_overlay: false,
            field_uploads: Vec::new(),
            field_draws: Vec::new(),
            clear_tiles: false,
            drop_tiles: Vec::new(),
            drop_fields: Vec::new(),
            new_vector_tiles: Vec::new(),
            visible_vector: Vec::new(),
            clear_vector: false,
            drop_vector_tiles: Vec::new(),
            wind_upload: None,
            wind: None,
        };

        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let (device, queue, adapter) = init_gpu(&rt).expect("no wgpu adapter");
        println!("adapter: {}", adapter.get_info().name);
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let mut res = RenderResources::new(&device, format);
        let target = new_target(&device, format, GOLDEN_SIZE);
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        res.render_once(
            &device,
            &queue,
            &view,
            &cb,
            wgpu::Color {
                r: 0.05,
                g: 0.05,
                b: 0.08,
                a: 1.0,
            },
        );
        let actual = read_target(&device, &queue, &target, GOLDEN_SIZE);

        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let golden = dir.join(GOLDEN);
        let dump = dir.join("../../target/snapshot_base_actual.png");
        let write_actual = || {
            if let Some(p) = dump.parent() {
                let _ = std::fs::create_dir_all(p);
            }
            image::save_buffer(
                &dump,
                &actual,
                GOLDEN_SIZE,
                GOLDEN_SIZE,
                image::ColorType::Rgba8,
            )
            .expect("write actual png");
        };

        if !golden.exists() {
            write_actual();
            println!(
                "SKIP: golden {} missing — wrote actual to {}. Generate it under lavapipe \
                 (mesa-vulkan-drivers + HOOKECHO_GPU_FALLBACK=1) and check it in.",
                golden.display(),
                dump.display()
            );
            return;
        }

        let expected = image::open(&golden).expect("decode golden").to_rgba8();
        assert_eq!(
            (expected.width(), expected.height()),
            (GOLDEN_SIZE, GOLDEN_SIZE),
            "golden has the wrong dimensions"
        );
        // Per-channel delta ≤ 8, failing pixels ≤ 0.5% — absorbs driver rounding without
        // letting a real render regression through.
        let bad = expected
            .as_raw()
            .as_chunks::<4>()
            .0
            .iter()
            .zip(actual.as_chunks::<4>().0)
            .filter(|(e, a)| {
                e.iter()
                    .zip(a.iter())
                    .any(|(x, y)| (*x as i16 - *y as i16).abs() > 8)
            })
            .count();
        let total = (GOLDEN_SIZE * GOLDEN_SIZE) as usize;
        if bad * 200 > total {
            write_actual();
            panic!(
                "golden mismatch: {bad}/{total} pixels off (> 0.5%); actual written to {}",
                dump.display()
            );
        }
    }
}
