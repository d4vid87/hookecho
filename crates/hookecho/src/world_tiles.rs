//! Observed international composites on the existing timed raster tile path.
use crate::tiles::BasemapStyle;
use chrono::{DateTime, Utc};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use wxdata::world_radar::{CwaGrid, RadarSample};

static KOREA_AVAILABLE: AtomicBool = AtomicBool::new(false);
pub(crate) fn korea_available() -> bool {
    KOREA_AVAILABLE.load(Ordering::Relaxed)
}

/// Discovery runs independently of imagery loading; no key is ever sent to the client.
pub(crate) async fn discover_providers(client: &reqwest::Client) {
    let url = wxdata::world_radar::regional_service_url("status");
    let enabled = match wxdata::task::timeout(
        std::time::Duration::from_secs(5),
        fetch_bytes(client, &url, 1024),
    )
    .await
    {
        Ok(Ok(bytes)) => serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .and_then(|value| value.get("kma").and_then(|value| value.as_bool()))
            .unwrap_or(false),
        _ => false,
    };
    KOREA_AVAILABLE.store(enabled, Ordering::Relaxed);
}

#[derive(Default)]
struct Catalogs {
    japan: Vec<DateTime<Utc>>,
    japan_checked: Option<wxdata::clock::Instant>,
    taiwan_checked: Option<wxdata::clock::Instant>,
    europe: Vec<wxdata::opera_composite::CompositeFrame>,
    europe_windows: VecDeque<CatalogWindow>,
    korea: Vec<DateTime<Utc>>,
    korea_windows: VecDeque<CatalogWindow>,
    grids: VecDeque<Arc<Grid>>,
}

const MAX_CATALOG_FRAMES: usize = 600;
const MAX_CATALOG_WINDOWS: usize = 8;

struct CatalogWindow {
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    checked: wxdata::clock::Instant,
}

fn recently_checked(
    windows: &VecDeque<CatalogWindow>,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> bool {
    windows.iter().any(|window| {
        window.from <= from && window.to >= to && window.checked.elapsed().as_secs() < 60
    })
}

fn remember_window(
    windows: &mut VecDeque<CatalogWindow>,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    trimmed_before: Option<DateTime<Utc>>,
) {
    windows.retain(|window| {
        window.checked.elapsed().as_secs() < 60 && !(from <= window.from && to >= window.to)
    });
    windows.push_back(CatalogWindow {
        from,
        to,
        checked: wxdata::clock::Instant::now(),
    });
    // A recently checked window is no longer complete if the bounded catalog evicted its frames.
    if let Some(first) = trimmed_before {
        windows.retain(|window| window.from >= first);
    }
    while windows.len() > MAX_CATALOG_WINDOWS {
        windows.pop_front();
    }
}

/// Catalogs are shared across panes: discovering live frames must not erase another pane's history.
fn merge_frames<T>(
    saved: &mut Vec<T>,
    incoming: impl IntoIterator<Item = T>,
    time: impl Fn(&T) -> DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    saved.extend(incoming);
    saved.sort_by_key(&time);
    saved.dedup_by_key(|frame| time(frame));
    let remove = saved.len().saturating_sub(MAX_CATALOG_FRAMES);
    saved.drain(..remove);
    (remove > 0).then(|| time(&saved[0]))
}

enum Grid {
    Taiwan(CwaGrid),
    Europe(wxdata::opera_composite::CompositeGrid),
    Korea(wxdata::world_radar::KmaGrid),
}
impl Grid {
    fn time(&self) -> DateTime<Utc> {
        match self {
            Self::Taiwan(grid) => grid.time,
            Self::Europe(grid) => grid.time,
            Self::Korea(grid) => grid.time,
        }
    }
    fn source(&self) -> BasemapStyle {
        match self {
            Self::Taiwan(_) => BasemapStyle::TaiwanRadar,
            Self::Europe(_) => BasemapStyle::OperaRadar,
            Self::Korea(_) => BasemapStyle::KoreaRadar,
        }
    }
    fn sample(&self, lon: f64, lat: f64) -> Option<f32> {
        match self {
            Self::Taiwan(grid) => match grid.sample_wgs84(lon, lat) {
                RadarSample::Echo(value) => Some(value),
                _ => None,
            },
            Self::Europe(grid) => grid.sample_nearest(lon, lat),
            Self::Korea(grid) => match grid.sample_wgs84(lon, lat) {
                RadarSample::Echo(value) => Some(value),
                _ => None,
            },
        }
    }
}

fn catalogs() -> &'static Mutex<Catalogs> {
    static CATALOGS: OnceLock<Mutex<Catalogs>> = OnceLock::new();
    CATALOGS.get_or_init(Default::default)
}

#[cfg(not(target_arch = "wasm32"))]
fn grid_pool() -> &'static wxdata::shared_read::Pool<Grid> {
    static POOL: OnceLock<wxdata::shared_read::Pool<Grid>> = OnceLock::new();
    POOL.get_or_init(Default::default)
}
#[cfg(target_arch = "wasm32")]
thread_local! { static GRID_POOL: Arc<wxdata::shared_read::Pool<Grid>> = Arc::new(Default::default()); }
#[cfg(target_arch = "wasm32")]
fn grid_pool() -> Arc<wxdata::shared_read::Pool<Grid>> {
    GRID_POOL.with(Arc::clone)
}

async fn fetch_bytes(
    client: &reqwest::Client,
    url: &str,
    limit: usize,
) -> anyhow::Result<Arc<Vec<u8>>> {
    use futures_util::StreamExt;
    let http = client.clone();
    let url = if url.contains("/regional-radar/") {
        url.to_owned()
    } else {
        wxdata::net::fetch_url(url)
    };
    let bytes = wxdata::shared_read::bytes(url.clone(), async move {
        let response = http
            .get(url)
            .timeout(wxdata::net::FEED_TIMEOUT)
            .send()
            .await?
            .error_for_status()?;
        let mut stream = response.bytes_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            anyhow::ensure!(
                bytes.len() + chunk.len() <= limit,
                "Regional radar response exceeds size limit"
            );
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    })
    .await?;
    Ok(bytes)
}

impl Catalogs {
    fn retain_grid(&mut self, grid: Arc<Grid>) {
        let source = grid.source();
        self.grids
            .retain(|old| !(old.source() == source && old.time() == grid.time()));
        self.grids.push_back(grid);
        // Separate two-frame budgets keep another pane from evicting Taiwan's unrecoverable
        // latest-only snapshots. Continent grids remain bounded even when several panes are open.
        while self
            .grids
            .iter()
            .filter(|grid| grid.source() == source)
            .count()
            > 2
        {
            if let Some(index) = self.grids.iter().position(|grid| grid.source() == source) {
                self.grids.remove(index);
            }
        }
    }
}

fn retain_grid(grid: Arc<Grid>) {
    catalogs()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .retain_grid(grid);
}

fn cached_grid(style: BasemapStyle, time: DateTime<Utc>) -> Option<Arc<Grid>> {
    catalogs()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .grids
        .iter()
        .find(|grid| grid.source() == style && grid.time() == time)
        .cloned()
}

/// Taiwan's URL names the newest file, so only actually retained snapshots enter its timeline.
pub(crate) async fn frame_times(
    client: &reqwest::Client,
    style: BasemapStyle,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    limit: usize,
) -> anyhow::Result<Vec<DateTime<Utc>>> {
    if style == BasemapStyle::KoreaRadar {
        anyhow::ensure!(
            korea_available(),
            "South Korea radar requires KMA_API_KEY on the server"
        );
        anyhow::ensure!(
            to >= from && to - from <= chrono::Duration::hours(24),
            "KMA window must be 0–24 hours"
        );
        let fresh = recently_checked(
            &catalogs()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .korea_windows,
            from,
            to,
        );
        if !fresh {
            let mut frames = Vec::new();
            let mut date = (from + chrono::Duration::hours(9)).date_naive();
            let last = (to + chrono::Duration::hours(9)).date_naive();
            while date <= last {
                let url = wxdata::world_radar::regional_service_url(&format!(
                    "kma/catalog?day={}",
                    date.format("%Y%m%d")
                ));
                let bytes = fetch_bytes(client, &url, 1024 * 1024).await?;
                frames.extend(wxdata::world_radar::parse_kma_frames(&bytes)?);
                let Some(next) = date.succ_opt() else { break };
                date = next;
            }
            let mut catalogs = catalogs()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let trimmed_before = merge_frames(
                &mut catalogs.korea,
                window(&frames, from, to, MAX_CATALOG_FRAMES),
                |time| *time,
            );
            remember_window(&mut catalogs.korea_windows, from, to, trimmed_before);
        }
        let catalogs = catalogs()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        return Ok(window(&catalogs.korea, from, to, limit));
    }
    if style == BasemapStyle::OperaRadar {
        let fresh = recently_checked(
            &catalogs()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .europe_windows,
            from,
            to,
        );
        if !fresh {
            let frames = wxdata::opera_composite::list_composite_frames(client, from, to).await?;
            let mut catalogs = catalogs()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let trimmed_before = merge_frames(&mut catalogs.europe, frames, |frame| frame.time);
            remember_window(&mut catalogs.europe_windows, from, to, trimmed_before);
        }
        let catalogs = catalogs()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        return Ok(window(
            &catalogs
                .europe
                .iter()
                .map(|frame| frame.time)
                .collect::<Vec<_>>(),
            from,
            to,
            limit,
        ));
    }
    if style == BasemapStyle::JmaRadar {
        let fresh = catalogs()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .japan_checked
            .is_some_and(|time| time.elapsed().as_secs() < 60);
        if !fresh {
            let bytes =
                fetch_bytes(client, wxdata::world_radar::JMA_CATALOG_URL, 64 * 1024).await?;
            let frames = wxdata::world_radar::parse_jma_frames(&bytes)?;
            let mut catalogs = catalogs()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            catalogs.japan = frames;
            catalogs.japan_checked = Some(wxdata::clock::Instant::now());
        }
        let catalogs = catalogs()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        return Ok(window(&catalogs.japan, from, to, limit));
    }
    if style == BasemapStyle::TaiwanRadar {
        let fresh = {
            let catalogs = catalogs()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            catalogs
                .taiwan_checked
                .is_some_and(|time| time.elapsed().as_secs() < 60)
                && catalogs.grids.iter().any(|grid| grid.source() == style)
        };
        if !fresh {
            let http = client.clone();
            let pool = grid_pool();
            let grid = pool
                .read("taiwan-latest".into(), async move {
                    let bytes = fetch_bytes(
                        &http,
                        wxdata::world_radar::CWA_GRID_URL,
                        wxdata::world_radar::CWA_MAX_BYTES,
                    )
                    .await?;
                    let grid = wxdata::world_radar::decode_cwa_grid(bytes).await?;
                    anyhow::ensure!(
                        grid.time <= Utc::now() + chrono::Duration::minutes(10),
                        "CWA observation is in the future"
                    );
                    Ok(Grid::Taiwan(grid))
                })
                .await?;
            retain_grid(grid);
            catalogs()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .taiwan_checked = Some(wxdata::clock::Instant::now());
        }
        let catalogs = catalogs()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let times = catalogs
            .grids
            .iter()
            .filter(|grid| grid.source() == style)
            .map(|grid| grid.time())
            .collect::<Vec<_>>();
        return Ok(window(&times, from, to, limit));
    }
    anyhow::bail!("Unsupported regional radar source")
}

fn window(
    times: &[DateTime<Utc>],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    limit: usize,
) -> Vec<DateTime<Utc>> {
    let mut times = times
        .iter()
        .copied()
        .filter(|time| *time >= from && *time <= to)
        .collect::<Vec<_>>();
    times.sort_unstable();
    times.dedup();
    times.drain(..times.len().saturating_sub(limit));
    times
}

pub(crate) fn tile_url(
    style: BasemapStyle,
    time: Option<DateTime<Utc>>,
    z: u8,
    x: u32,
    y: u32,
) -> Option<String> {
    if z > 10 || x >= 1_u32 << z || y >= 1_u32 << z {
        return None;
    }
    let catalogs = catalogs()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if style == BasemapStyle::JmaRadar {
        let time = time.or_else(|| catalogs.japan.last().copied())?;
        if !catalogs.japan.contains(&time) {
            return None;
        }
        return wxdata::world_radar::jma_tile_url(time, z, x, y);
    }
    if style == BasemapStyle::OperaRadar {
        let time = time.or_else(|| catalogs.europe.last().map(|frame| frame.time))?;
        if !catalogs.europe.iter().any(|frame| frame.time == time) {
            return None;
        }
        return Some(format!(
            "world-radar:{}/{}/{z}/{x}/{y}",
            style.slug(),
            time.timestamp()
        ));
    }
    if style == BasemapStyle::KoreaRadar {
        let time = time.or_else(|| catalogs.korea.last().copied())?;
        if !catalogs.korea.contains(&time) {
            return None;
        }
        return Some(format!(
            "world-radar:{}/{}/{z}/{x}/{y}",
            style.slug(),
            time.timestamp()
        ));
    }
    let time = time.or_else(|| {
        catalogs
            .grids
            .iter()
            .filter(|grid| grid.source() == style)
            .map(|grid| grid.time())
            .max()
    })?;
    // Internal tile identity, intercepted before HTTP; never an external URL or a guessed time.
    Some(format!(
        "world-radar:{}/{}/{z}/{x}/{y}",
        style.slug(),
        time.timestamp()
    ))
}

pub(crate) async fn load_tile(
    client: &reqwest::Client,
    url: &str,
) -> anyhow::Result<image::RgbaImage> {
    let (style, time, z, x, y) = parse_tile_url(url)?;
    let grid = if let Some(grid) = cached_grid(style, time) {
        grid
    } else if style == BasemapStyle::OperaRadar {
        let frame = catalogs()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .europe
            .iter()
            .find(|frame| frame.time == time)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("European frame is no longer advertised"))?;
        let http = client.clone();
        let pool = grid_pool();
        let grid = pool
            .read(format!("opera:{}", time.timestamp()), async move {
                Ok(Grid::Europe(
                    wxdata::opera_composite::fetch_composite(&http, &frame).await?,
                ))
            })
            .await?;
        retain_grid(Arc::clone(&grid));
        grid
    } else if style == BasemapStyle::KoreaRadar {
        anyhow::ensure!(
            catalogs()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .korea
                .contains(&time),
            "Korean frame is no longer advertised"
        );
        let http = client.clone();
        let pool = grid_pool();
        let grid = pool
            .read(format!("korea:{}", time.timestamp()), async move {
                let url = wxdata::world_radar::regional_service_url(&format!(
                    "kma/grid?time={}",
                    wxdata::world_radar::kma_time_parameter(time)
                ));
                let bytes = fetch_bytes(&http, &url, wxdata::world_radar::KMA_MAX_BYTES).await?;
                Ok(Grid::Korea(
                    wxdata::world_radar::decode_kma_grid(bytes, time).await?,
                ))
            })
            .await?;
        retain_grid(Arc::clone(&grid));
        grid
    } else {
        anyhow::bail!("This regional radar snapshot is no longer retained")
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        wxdata::task::blocking(move || render_tile(z, x, y, |lon, lat| grid.sample(lon, lat))).await
    }
    #[cfg(target_arch = "wasm32")]
    {
        Ok(render_tile_cooperative(z, x, y, |lon, lat| grid.sample(lon, lat)).await)
    }
}

fn parse_tile_url(url: &str) -> anyhow::Result<(BasemapStyle, DateTime<Utc>, u8, u32, u32)> {
    let parts = url
        .strip_prefix("world-radar:")
        .unwrap_or("")
        .split('/')
        .collect::<Vec<_>>();
    anyhow::ensure!(parts.len() == 5, "Invalid regional tile identity");
    let style = match parts[0] {
        "taiwan-radar" => BasemapStyle::TaiwanRadar,
        "opera-radar" => BasemapStyle::OperaRadar,
        "korea-radar" => BasemapStyle::KoreaRadar,
        _ => anyhow::bail!("Unknown regional grid"),
    };
    let time = DateTime::from_timestamp(parts[1].parse()?, 0)
        .ok_or_else(|| anyhow::anyhow!("Invalid regional tile time"))?;
    let z: u8 = parts[2].parse()?;
    let x: u32 = parts[3].parse()?;
    let y: u32 = parts[4].parse()?;
    anyhow::ensure!(
        z <= 10 && x < 1_u32 << z && y < 1_u32 << z,
        "Invalid regional tile coordinates"
    );
    Ok((style, time, z, x, y))
}

/// Render one bounded row block. Latitude is constant across a Web Mercator tile row.
fn render_rows(
    image: &mut image::RgbaImage,
    z: u8,
    x: u32,
    y: u32,
    rows: std::ops::Range<u32>,
    sample: &impl Fn(f64, f64) -> Option<f32>,
) {
    let palette = crate::colormap::default_table(wxdata::level2::Moment::Reflectivity);
    let scale = 256.0 * f64::from(1_u32 << z);
    let longitudes = std::array::from_fn::<_, 256, _>(|px| {
        (f64::from(x) * 256.0 + px as f64 + 0.5) / scale * 360.0 - 180.0
    });
    for py in rows {
        let merc = std::f64::consts::PI
            * (1.0 - 2.0 * (f64::from(y) * 256.0 + f64::from(py) + 0.5) / scale);
        let lat = merc.sinh().atan().to_degrees();
        for (px, lon) in longitudes.iter().enumerate() {
            let rgba = sample(*lon, lat)
                .filter(|value| value.is_finite())
                .and_then(|value| palette.sample(value))
                .map(|mut color| {
                    color[3] = 217;
                    color
                })
                .unwrap_or([0; 4]);
            image.put_pixel(px as u32, py, image::Rgba(rgba));
        }
    }
}

#[cfg(any(not(target_arch = "wasm32"), test))]
fn render_tile(
    z: u8,
    x: u32,
    y: u32,
    sample: impl Fn(f64, f64) -> Option<f32>,
) -> image::RgbaImage {
    let mut image = image::RgbaImage::new(256, 256);
    render_rows(&mut image, z, x, y, 0..256, &sample);
    image
}

#[cfg(any(target_arch = "wasm32", test))]
async fn render_tile_cooperative(
    z: u8,
    x: u32,
    y: u32,
    sample: impl Fn(f64, f64) -> Option<f32>,
) -> image::RgbaImage {
    let mut image = image::RgbaImage::new(256, 256);
    for first in (0..256).step_by(8) {
        // Return to the browser between 2,048 projections so pan/zoom and cancellation can run.
        wxdata::task::yield_now().await;
        render_rows(&mut image, z, x, y, first..first + 8, &sample);
    }
    image
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn regional_tiles_preserve_orientation_and_missing_coverage() {
        let image = render_tile(0, 0, 0, |lon, lat| (lon > 0.0 && lat > 0.0).then_some(40.0));
        assert_eq!(image.get_pixel(20, 20)[3], 0);
        assert_eq!(image.get_pixel(220, 220)[3], 0);
        assert_eq!(image.get_pixel(220, 20)[3], 217);
    }
    #[test]
    fn tile_identity_rejects_invalid_sources_and_coordinates() {
        assert!(parse_tile_url("world-radar:taiwan-radar/1700000000/4/14/7").is_ok());
        for url in [
            "world-radar:evil/1700000000/4/14/7",
            "world-radar:taiwan-radar/1700000000/31/0/0",
            "world-radar:taiwan-radar/1700000000/4/16/7",
        ] {
            assert!(parse_tile_url(url).is_err());
        }
    }
    #[test]
    fn frame_window_does_not_fabricate_missing_history() {
        let start = DateTime::from_timestamp(1700000000, 0).unwrap();
        assert!(window(
            &[start],
            start - chrono::Duration::hours(2),
            start - chrono::Duration::hours(1),
            10
        )
        .is_empty());
        assert!(window(&[start], start, start, 0).is_empty());
        assert_eq!(window(&[start, start], start, start, 10), vec![start]);
    }

    #[test]
    fn another_provider_cannot_evict_retained_taiwan_snapshots() {
        let time = "2026-11-04T23:40:00Z".parse::<DateTime<Utc>>().unwrap();
        let cwa = wxdata::world_radar::parse_cwa_grid(
            br#"<cwaopendata>
            <dataid>O-A0059-001</dataid><Reflectivity>dBZ</Reflectivity>
            <DateTime>2026-11-04T23:40:00Z</DateTime><GridDimensionX>2</GridDimensionX>
            <GridDimensionY>2</GridDimensionY><StartPointLongitude>121</StartPointLongitude>
            <StartPointLatitude>24</StartPointLatitude><GridResolution>0.0125</GridResolution>
            <contentDescription>TWD67</contentDescription><content>10,20,30,40</content>
            </cwaopendata>"#,
        )
        .unwrap();
        let kma = wxdata::world_radar::parse_kma_grid(
            format!("2305,2881\n{}", "0,".repeat(2305 * 2881)).as_bytes(),
            time,
        )
        .unwrap();
        let mut catalogs = Catalogs::default();
        for minute in 0..2 {
            let mut grid = cwa.clone();
            grid.time = time + chrono::Duration::minutes(minute);
            catalogs.retain_grid(Arc::new(Grid::Taiwan(grid)));
        }
        for minute in 0..3 {
            let mut grid = kma.clone();
            grid.time = time + chrono::Duration::minutes(minute);
            catalogs.retain_grid(Arc::new(Grid::Korea(grid)));
        }
        let times = |source| {
            catalogs
                .grids
                .iter()
                .filter(|grid| grid.source() == source)
                .map(|grid| grid.time())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            times(BasemapStyle::TaiwanRadar),
            vec![time, time + chrono::Duration::minutes(1)]
        );
        assert_eq!(
            times(BasemapStyle::KoreaRadar),
            vec![
                time + chrono::Duration::minutes(1),
                time + chrono::Duration::minutes(2)
            ]
        );
        let mut newest = cwa;
        newest.time = time + chrono::Duration::minutes(2);
        catalogs.retain_grid(Arc::new(Grid::Taiwan(newest)));
        assert_eq!(catalogs.grids.len(), 4);
        assert!(!catalogs
            .grids
            .iter()
            .any(|grid| grid.source() == BasemapStyle::TaiwanRadar && grid.time() == time));
    }

    #[test]
    fn separate_panes_keep_history_and_independent_freshness() {
        let start = DateTime::from_timestamp(1700000000, 0).unwrap();
        let minute = chrono::Duration::minutes(1);
        let mut frames = Vec::new();
        let mut windows = VecDeque::new();
        merge_frames(&mut frames, [start, start + minute], |time| *time);
        remember_window(&mut windows, start, start + minute, None);
        merge_frames(
            &mut frames,
            [start + minute * 10, start + minute * 11],
            |time| *time,
        );
        remember_window(&mut windows, start + minute * 10, start + minute * 11, None);
        assert_eq!(
            window(&frames, start, start + minute, 10),
            vec![start, start + minute]
        );
        assert!(recently_checked(&windows, start, start + minute));
        assert!(recently_checked(
            &windows,
            start + minute * 10,
            start + minute * 11
        ));
        // The gap between successful requests has never been checked.
        assert!(!recently_checked(&windows, start, start + minute * 11));
        windows[0].checked = wxdata::clock::Instant::now() - std::time::Duration::from_secs(61);
        assert!(!recently_checked(&windows, start, start + minute));
        assert!(recently_checked(
            &windows,
            start + minute * 10,
            start + minute * 11
        ));
    }

    #[test]
    fn merged_catalog_and_freshness_are_bounded_and_deduplicated() {
        let start = DateTime::from_timestamp(1700000000, 0).unwrap();
        let minute = chrono::Duration::minutes(1);
        let mut frames = vec![start, start];
        let mut windows = VecDeque::new();
        remember_window(&mut windows, start, start, None);
        let trimmed = merge_frames(
            &mut frames,
            (0..MAX_CATALOG_FRAMES + 2).map(|i| start + minute * i as i32),
            |time| *time,
        );
        assert_eq!(frames.len(), MAX_CATALOG_FRAMES);
        assert_eq!(trimmed, Some(start + minute * 2));
        remember_window(
            &mut windows,
            start + minute * 2,
            *frames.last().unwrap(),
            trimmed,
        );
        assert!(!recently_checked(&windows, start, start));
        assert!(recently_checked(
            &windows,
            start + minute * 2,
            *frames.last().unwrap()
        ));
        for i in 0..MAX_CATALOG_WINDOWS * 2 {
            let at = start + chrono::Duration::hours(i as i64);
            remember_window(&mut windows, at, at, None);
        }
        assert_eq!(windows.len(), MAX_CATALOG_WINDOWS);
    }

    #[tokio::test]
    async fn cooperative_render_matches_native_and_yields_before_projection() {
        use std::future::Future;
        let sample = |lon: f64, lat: f64| (lon > 0.0 && lat > 0.0).then_some(40.0);
        let mut render = std::pin::pin!(render_tile_cooperative(0, 0, 0, sample));
        let mut context = std::task::Context::from_waker(futures_util::task::noop_waker_ref());
        assert!(render.as_mut().poll(&mut context).is_pending());
        assert_eq!(render.await, render_tile(0, 0, 0, sample));
    }

    /// Run manually with network access; output is evidence, not a deterministic CI fixture.
    #[tokio::test]
    #[ignore = "fetches current JMA, CWA and OPERA observations from official services"]
    async fn live_observed_world_radar_smoke() {
        let client = reqwest::Client::builder()
            .user_agent(crate::tiles::USER_AGENT)
            .build()
            .unwrap();
        let now = Utc::now();
        for (name, style, lon, lat) in [
            ("jma", BasemapStyle::JmaRadar, 139.7_f64, 35.7_f64),
            ("taiwan", BasemapStyle::TaiwanRadar, 121.0, 24.0),
            ("opera", BasemapStyle::OperaRadar, 10.0, 51.0),
        ] {
            let started = wxdata::clock::Instant::now();
            let frames = frame_times(&client, style, now - chrono::Duration::hours(8), now, 48)
                .await
                .unwrap();
            let time = *frames
                .last()
                .expect("source advertised no recent observed frames");
            eprintln!(
                "{name}: catalog {} observed frames, latest {time}, elapsed {:?}",
                frames.len(),
                started.elapsed()
            );
            let z = 7;
            let x = ((lon + 180.0) / 360.0 * f64::from(1_u32 << z)).floor() as u32;
            let y = ((1.0 - lat.to_radians().tan().asinh() / std::f64::consts::PI) / 2.0
                * f64::from(1_u32 << z))
            .floor() as u32;
            let url = tile_url(style, Some(time), z, x, y).unwrap();
            let started = wxdata::clock::Instant::now();
            let image = if style == BasemapStyle::JmaRadar {
                let bytes = fetch_bytes(&client, &url, 4 * 1024 * 1024).await.unwrap();
                image::load_from_memory(&bytes).unwrap().to_rgba8()
            } else {
                load_tile(&client, &url).await.unwrap()
            };
            assert_eq!(image.dimensions(), (256, 256));
            eprintln!("{name}: fetch/decode/render {:?}", started.elapsed());
            image
                .save(format!("/tmp/hookecho-world-{name}.png"))
                .unwrap();
            if style != BasemapStyle::JmaRadar {
                let started = wxdata::clock::Instant::now();
                let cached = load_tile(&client, &url).await.unwrap();
                assert_eq!(cached, image);
                eprintln!("{name}: cached grid raster {:?}", started.elapsed());
            }
        }
    }
}
