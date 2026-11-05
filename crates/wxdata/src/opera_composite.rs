//! Open OPERA CIRRUS reflectivity composites, retaining their native LAEA registration.
//! Source: EUMETNET Open Radar Data, CC BY 4.0; attribution must accompany the displayed layer.

use chrono::{DateTime, Utc};
use hdf5lite::Value;

const BUCKET: &str = "https://s3.waw3-1.cloudferro.com/openradar-24h";
const MAX_CELLS: usize = 20_000_000;
pub const ATTRIBUTION: &str = "EUMETNET OPERA · CC BY 4.0";

#[derive(Debug, Clone)]
pub struct CompositeFrame {
    pub time: DateTime<Utc>,
    pub url: String,
}

fn frames_in(xml: &str, from: DateTime<Utc>, to: DateTime<Utc>) -> Vec<CompositeFrame> {
    xml.split("<Key>").skip(1).filter_map(|rest| {
        let key = rest.split_once("</Key>")?.0;
        let (directory, name) = key.rsplit_once('/')?;
        let stamp = name.strip_prefix("OPERA@")?.strip_suffix("@0@DBZH.h5")?;
        let time = chrono::NaiveDateTime::parse_from_str(stamp, "%Y%m%dT%H%M").ok()?.and_utc();
        if directory != format!("{}/OPERA/COMP", time.format("%Y/%m/%d")) || time < from || time > to { return None; }
        Some(CompositeFrame { time, url: format!("{BUCKET}/{key}") })
    }).collect()
}

/// Timestamped frames in a bounded rolling window (at most 24 hours), oldest first.
pub async fn list_composite_frames(
    http: &reqwest::Client, from: DateTime<Utc>, to: DateTime<Utc>,
) -> anyhow::Result<Vec<CompositeFrame>> {
    use futures_util::{stream, StreamExt};
    anyhow::ensure!(to >= from && to - from <= chrono::Duration::hours(24), "OPERA composite window must be 0–24 hours");
    let first = from.timestamp().div_euclid(3600);
    let last = to.timestamp().div_euclid(3600);
    let results: Vec<_> = stream::iter((first..=last).map(|hour| async move {
        let at = DateTime::<Utc>::from_timestamp(hour * 3600, 0).ok_or_else(|| anyhow::anyhow!("invalid OPERA date"))?;
        let url = format!("{BUCKET}?list-type=2&prefix={}/OPERA/COMP/OPERA@{}", at.format("%Y/%m/%d"), at.format("%Y%m%dT%H"));
        let xml = String::from_utf8(crate::opera::bounded_get(http, &url, 256 * 1024).await?)?;
        anyhow::ensure!(!xml.contains("<IsTruncated>true</IsTruncated>"), "OPERA composite listing truncated");
        Ok::<_, anyhow::Error>(frames_in(&xml, from, to))
    })).buffer_unordered(3).collect().await;
    let mut frames = Vec::new();
    let mut successes = 0;
    for result in results.into_iter().flatten() { successes += 1; frames.extend(result); }
    anyhow::ensure!(successes > 0, "OPERA composite listings unavailable");
    frames.sort_by_key(|frame| frame.time);
    frames.dedup_by_key(|frame| frame.time);
    Ok(frames)
}

/// Two-byte physical reflectivity storage. 0 is outside coverage, 1 is observed below threshold.
/// Sampled pixels retain their actual native centres; thinning does not claim new precision.
pub struct CompositeGrid {
    pub time: DateTime<Utc>,
    pub width: usize,
    pub height: usize,
    pub native_resolution_m: f64,
    pub display_resolution_m: f64,
    values: Vec<u16>,
    #[cfg(any(target_arch = "wasm32", test))]
    projection_definition: String,
    geographic: proj4rs::Proj,
    projected: proj4rs::Proj,
    x_left: f64,
    y_top: f64,
    dx: f64,
    dy: f64,
    native_width: usize,
    native_height: usize,
    stride: usize,
}

/// Transfer physical samples and their exact registration, rebuilding projection handles on the
/// receiving heap. The decoded HDF5 scratch and its full-resolution data never enter the UI heap.
#[cfg(any(target_arch = "wasm32", test))]
#[derive(serde::Serialize, serde::Deserialize)]
struct CompositeWire {
    time: DateTime<Utc>,
    values: Vec<u16>,
    projection_definition: String,
    x_left: f64,
    y_top: f64,
    dx: f64,
    dy: f64,
    native_width: usize,
    native_height: usize,
    stride: usize,
}

#[cfg(any(target_arch = "wasm32", test))]
impl From<CompositeGrid> for CompositeWire {
    fn from(grid: CompositeGrid) -> Self {
        Self { time: grid.time, values: grid.values, projection_definition: grid.projection_definition,
            x_left: grid.x_left, y_top: grid.y_top, dx: grid.dx, dy: grid.dy,
            native_width: grid.native_width, native_height: grid.native_height, stride: grid.stride }
    }
}

#[cfg(any(target_arch = "wasm32", test))]
impl TryFrom<CompositeWire> for CompositeGrid {
    type Error = anyhow::Error;
    fn try_from(wire: CompositeWire) -> anyhow::Result<Self> {
        anyhow::ensure!(wire.native_width > 0 && wire.native_height > 0
            && wire.native_width.checked_mul(wire.native_height).is_some_and(|n| n <= MAX_CELLS)
            && wire.stride > 0, "OPERA worker grid dimensions exceed limit");
        let (width, height) = (wire.native_width.div_ceil(wire.stride), wire.native_height.div_ceil(wire.stride));
        anyhow::ensure!(width <= 2200 && height <= 2200 && wire.values.len() == width * height,
            "OPERA worker grid samples disagree");
        anyhow::ensure!([wire.x_left, wire.y_top, wire.dx, wire.dy].iter().all(|value| value.is_finite())
            && wire.dx > 0.0 && wire.dy > 0.0, "OPERA worker grid registration is invalid");
        anyhow::ensure!(wire.projection_definition.contains("+proj=laea"), "OPERA unsupported composite projection");
        let projected = proj4rs::Proj::from_proj_string(&wire.projection_definition)
            .map_err(|error| anyhow::anyhow!("OPERA worker projection: {error}"))?;
        let geographic = proj4rs::Proj::from_proj_string("+proj=longlat +ellps=WGS84")
            .map_err(|error| anyhow::anyhow!("OPERA WGS84: {error}"))?;
        Ok(Self { time: wire.time, width, height,
            native_resolution_m: wire.dx.max(wire.dy), display_resolution_m: wire.dx.max(wire.dy) * wire.stride as f64,
            values: wire.values, projection_definition: wire.projection_definition, projected, geographic,
            x_left: wire.x_left, y_top: wire.y_top, dx: wire.dx, dy: wire.dy,
            native_width: wire.native_width, native_height: wire.native_height, stride: wire.stride })
    }
}

#[cfg(any(target_arch = "wasm32", test))]
fn decode_worker_result(bytes: &[u8]) -> anyhow::Result<CompositeGrid> {
    CompositeGrid::try_from(postcard::from_bytes::<CompositeWire>(bytes)?)
}

#[cfg(any(target_arch = "wasm32", test))]
pub fn decode_worker(bytes: Vec<u8>) -> anyhow::Result<Vec<u8>> {
    Ok(postcard::to_allocvec(&CompositeWire::from(decode_composite(bytes)?))?)
}

impl CompositeGrid {
    /// dBZ at the nearest retained observed cell. None means no observation, not no rain.
    pub fn sample_nearest(&self, lon: f64, lat: f64) -> Option<f32> {
        if !lon.is_finite() || !lat.is_finite() || !(-180.0..=180.0).contains(&lon) || !(-90.0..=90.0).contains(&lat) { return None; }
        let mut point = (lon.to_radians(), lat.to_radians(), 0.0);
        proj4rs::transform::transform(&self.geographic, &self.projected, &mut point).ok()?;
        self.sample_projected(point.0, point.1)
    }

    fn sample_projected(&self, x: f64, y: f64) -> Option<f32> {
        let (column, row) = ((x - self.x_left) / self.dx, (self.y_top - y) / self.dy);
        if !column.is_finite() || !row.is_finite() || column < 0.0 || row < 0.0
            || column >= self.native_width as f64 || row >= self.native_height as f64 { return None; }
        // Strided decoding keeps source centres 0.5, stride+0.5, …, not cell-block centres.
        let col = ((column - 0.5) / self.stride as f64).round().clamp(0.0, (self.width - 1) as f64) as usize;
        let row = ((row - 0.5) / self.stride as f64).round().clamp(0.0, (self.height - 1) as f64) as usize;
        match *self.values.get(row * self.width + col)? {
            0 => None,
            1 => Some(-32.0),
            value => Some((value - 2) as f32 / 100.0 - 100.0),
        }
    }
}

fn pack(value: f64, gain: f64, offset: f64, nodata: f64, undetect: f64) -> u16 {
    if !value.is_finite() || value == nodata { return 0; }
    if value == undetect { return 1; }
    let dbz = value.mul_add(gain, offset);
    if !dbz.is_finite() || !(-100.0..=200.0).contains(&dbz) { return 0; }
    ((dbz + 100.0) * 100.0).round() as u16 + 2
}

/// Decode with at most 2,200 pixels per dimension (~8.4MB retained for the current 1km composite).
/// shortcut: overview composites are thinned to 2km; native station data supplies detailed zoom.
pub fn decode_composite(bytes: Vec<u8>) -> anyhow::Result<CompositeGrid> {
    let file = hdf5lite::File::open(bytes)?;
    let what = file.attributes("what")?;
    anyhow::ensure!(what.get("object").and_then(Value::as_str) == Some("COMP"), "OPERA expected COMP object");
    let date = what.get("date").and_then(Value::as_str).unwrap_or_default();
    let time = what.get("time").and_then(Value::as_str).unwrap_or_default();
    let time = chrono::NaiveDateTime::parse_from_str(&format!("{date}{time}"), "%Y%m%d%H%M%S")?.and_utc();
    let where_ = file.attributes("where")?;
    let number = |key: &str| -> anyhow::Result<f64> {
        where_.get(key).and_then(Value::as_f64).filter(|value| value.is_finite())
            .ok_or_else(|| anyhow::anyhow!("OPERA missing {key}"))
    };
    let dims = file.dataset("dataset1/data1/data")?.dims;
    let [native_height, native_width] = dims.as_slice() else { anyhow::bail!("OPERA composite must be a 2D grid") };
    let (native_width, native_height) = (usize::try_from(*native_width)?, usize::try_from(*native_height)?);
    anyhow::ensure!(native_width > 0 && native_height > 0 && native_width.checked_mul(native_height).is_some_and(|n| n <= MAX_CELLS), "OPERA grid dimensions exceed limit");
    anyhow::ensure!(number("xsize")? == native_width as f64 && number("ysize")? == native_height as f64, "OPERA dimensions disagree");
    let (dx, dy) = (number("xscale")?, number("yscale")?);
    anyhow::ensure!(dx > 0.0 && dy > 0.0, "OPERA invalid cell spacing");
    let definition = where_.get("projdef").and_then(Value::as_str).ok_or_else(|| anyhow::anyhow!("OPERA missing projection"))?;
    anyhow::ensure!(definition.contains("+proj=laea"), "OPERA unsupported composite projection");
    let projected = proj4rs::Proj::from_proj_string(definition).map_err(|e| anyhow::anyhow!("OPERA projection: {e}"))?;
    let geographic = proj4rs::Proj::from_proj_string("+proj=longlat +ellps=WGS84").map_err(|e| anyhow::anyhow!("OPERA WGS84: {e}"))?;
    let mut origin = (number("UL_lon")?.to_radians(), number("UL_lat")?.to_radians(), 0.0);
    proj4rs::transform::transform(&geographic, &projected, &mut origin).map_err(|e| anyhow::anyhow!("OPERA origin: {e}"))?;
    anyhow::ensure!(origin.0.is_finite() && origin.1.is_finite(), "OPERA invalid origin");
    let data = file.attributes("dataset1/data1/what")?;
    anyhow::ensure!(data.get("quantity").and_then(Value::as_str) == Some("DBZH"), "OPERA expected measured reflectivity");
    let gain = data.get("gain").and_then(Value::as_f64).unwrap_or(1.0);
    let offset = data.get("offset").and_then(Value::as_f64).unwrap_or(0.0);
    anyhow::ensure!(gain.is_finite() && offset.is_finite(), "OPERA invalid calibration");
    let nodata = data.get("nodata").and_then(Value::as_f64).unwrap_or(f64::NAN);
    let undetect = data.get("undetect").and_then(Value::as_f64).unwrap_or(f64::NAN);
    let stride = native_width.max(native_height).div_ceil(2200).max(1);
    let values = file.read_f64_strided_2d("dataset1/data1/data", stride)?.into_iter()
        .map(|value| pack(value, gain, offset, nodata, undetect)).collect();
    Ok(CompositeGrid { time, width: native_width.div_ceil(stride), height: native_height.div_ceil(stride),
        native_resolution_m: dx.max(dy), display_resolution_m: dx.max(dy) * stride as f64,
        #[cfg(any(target_arch = "wasm32", test))]
        projection_definition: definition.to_owned(),
        values, geographic, projected, x_left: origin.0, y_top: origin.1, dx, dy,
        native_width, native_height, stride })
}

pub async fn fetch_composite(http: &reqwest::Client, frame: &CompositeFrame) -> anyhow::Result<CompositeGrid> {
    let expected = format!("{BUCKET}/{}/OPERA/COMP/OPERA@{}@0@DBZH.h5", frame.time.format("%Y/%m/%d"), frame.time.format("%Y%m%dT%H%M"));
    anyhow::ensure!(frame.url == expected, "OPERA composite URL does not match the frame");
    let bytes = crate::opera::bounded_get(http, &frame.url, 16 * 1024 * 1024).await?;
    #[cfg(not(target_arch = "wasm32"))]
    let grid = crate::task::blocking(move || decode_composite(bytes)).await??;
    #[cfg(target_arch = "wasm32")]
    let grid = match crate::wasm_worker::decode_opera(bytes.clone()).await {
        Ok(wire) => decode_worker_result(&wire)?,
        Err(crate::wasm_worker::Error::Unavailable) => decode_composite(bytes)?,
        Err(error) => anyhow::bail!("{error}"),
    };
    anyhow::ensure!(grid.time == frame.time, "OPERA composite timestamp mismatch");
    Ok(grid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_selects_only_real_reflectivity_objects_in_window() {
        let start = "2026-11-03T23:55:00Z".parse().unwrap();
        let end = "2026-11-04T00:05:00Z".parse().unwrap();
        let xml = "<Key>2026/11/03/OPERA/COMP/OPERA@20261103T2355@0@DBZH.h5</Key>\
                   <Key>2026/11/04/OPERA/COMP/OPERA@20261104T0000@0@DBZH.h5</Key>\
                   <Key>2026/11/04/OPERA/COMP/OPERA@20261104T0000@0@RATE.h5</Key>\
                   <Key>2026/11/04/OPERA/COMP/OPERA@20261104T0010@0@DBZH.h5</Key>\
                   <Key>../../OPERA@20261104T0000@0@DBZH.h5</Key>";
        let frames = frames_in(xml, start, end);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].time, start);
        assert!(frames[1].url.starts_with(BUCKET));
    }

    #[test]
    fn no_observation_is_distinct_from_below_threshold() {
        assert_eq!(pack(-999.0, 1.0, 0.0, -999.0, -888.0), 0);
        assert_eq!(pack(-888.0, 1.0, 0.0, -999.0, -888.0), 1);
        assert_eq!(pack(f64::NAN, 1.0, 0.0, -999.0, -888.0), 0);
        assert_eq!(pack(100.0, 0.5, -32.0, -999.0, -888.0), 11802);
        assert_eq!(pack(1e30, 1.0, 0.0, -999.0, -888.0), 0);
    }

    #[test]
    fn projection_sampling_does_not_stretch_laea_as_a_latlon_rectangle() {
        let mut grid = CompositeGrid { time: Utc::now(), width: 2, height: 2, native_resolution_m: 1000.0,
            display_resolution_m: 1000.0, values: vec![0, 1, 14002, 15002],
            projection_definition: "+proj=laea +lat_0=55 +lon_0=10 +ellps=WGS84".into(),
            geographic: proj4rs::Proj::from_proj_string("+proj=longlat +ellps=WGS84").unwrap(),
            projected: proj4rs::Proj::from_proj_string("+proj=laea +lat_0=55 +lon_0=10 +ellps=WGS84").unwrap(),
            x_left: 0.0, y_top: 0.0, dx: 1000.0, dy: 1000.0, native_width: 2, native_height: 2, stride: 1 };
        assert_eq!(grid.sample_projected(500.0, -500.0), None);
        assert_eq!(grid.sample_projected(1500.0, -500.0), Some(-32.0));
        assert_eq!(grid.sample_projected(500.0, -1500.0), Some(40.0));
        assert_eq!(grid.sample_projected(2000.0, -500.0), None);
        assert_eq!(grid.sample_nearest(f64::NAN, 55.0), None);
        grid.values[0] = 16002;
        assert_eq!(grid.sample_nearest(10.001, 54.999), Some(60.0));
    }

    #[test]
    fn worker_roundtrip_preserves_projection_stride_and_missing_samples() {
        let definition = "+proj=laea +lat_0=55 +lon_0=10 +x_0=1950000 +y_0=-2100000 +units=m +ellps=WGS84";
        let grid = CompositeGrid { time: "2026-11-04T00:00:00Z".parse().unwrap(),
            width: 2, height: 2, native_resolution_m: 1000.0, display_resolution_m: 2000.0,
            values: vec![0, 1, 14002, 15002], projection_definition: definition.into(),
            geographic: proj4rs::Proj::from_proj_string("+proj=longlat +ellps=WGS84").unwrap(),
            projected: proj4rs::Proj::from_proj_string(definition).unwrap(),
            x_left: 1950000.0, y_top: -2100000.0, dx: 1000.0, dy: 1000.0,
            native_width: 4, native_height: 4, stride: 2 };
        let positions = [(1950500.0, -2100500.0), (1952500.0, -2100500.0),
            (1950500.0, -2102500.0), (1952500.0, -2102500.0), (1949500.0, -2102500.0)];
        let observations = positions.iter().map(|&(x, y)| {
            let mut point = (x, y, 0.0);
            proj4rs::transform::transform(&grid.projected, &grid.geographic, &mut point).unwrap();
            let (lon, lat) = (point.0.to_degrees(), point.1.to_degrees());
            (lon, lat, grid.sample_nearest(lon, lat))
        }).collect::<Vec<_>>();
        assert_eq!(observations.iter().map(|value| value.2).collect::<Vec<_>>(),
            vec![None, Some(-32.0), Some(40.0), Some(50.0), None]);
        let encoded = postcard::to_allocvec(&CompositeWire::from(grid)).unwrap();
        let rebuilt = decode_worker_result(&encoded).unwrap();
        assert_eq!((rebuilt.width, rebuilt.height, rebuilt.stride), (2, 2, 2));
        assert_eq!(rebuilt.native_resolution_m, 1000.0);
        assert_eq!(rebuilt.display_resolution_m, 2000.0);
        assert_eq!(rebuilt.time.to_rfc3339(), "2026-11-04T00:00:00+00:00");
        for (lon, lat, value) in observations { assert_eq!(rebuilt.sample_nearest(lon, lat), value); }
        let mut malformed = CompositeWire::from(rebuilt);
        malformed.stride = 0;
        assert!(decode_worker_result(&postcard::to_allocvec(&malformed).unwrap()).is_err());
        assert!(decode_worker(vec![0; 128]).is_err());
    }

    #[test]
    fn invalid_composite_is_rejected() { assert!(decode_composite(vec![0; 128]).is_err()); }
}
