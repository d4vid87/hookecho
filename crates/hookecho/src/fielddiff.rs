//! Subtract one model's field from another's.
//!
//! Two models disagreeing is the forecast information — a 6 °C spread over the warm sector says
//! more about tomorrow than either model's own number does. Everything the difference layer needs
//! beyond the ordinary field path lives here: resample the two grids onto one lattice, subtract,
//! and hand back an [`MrmsField`] the existing upload/draw code cannot tell from any other.
//!
//! ponytail: the coarser grid wins, rather than interpolating both onto something finer. A
//! difference is never sharper than its blurriest input, and pretending otherwise costs memory to
//! draw detail that is not there. GFS and ECMWF already share one lattice, so that pair does not
//! resample at all.

use wxdata::field::{
    DataClass, DataStamp, FieldDescriptor, FieldFamily, FieldFrame, FieldId, MissingData,
    QualitySummary, SamplingPolicy, ValueKind,
};
use wxdata::global::GlobalField;
use wxdata::mrms::MrmsField;

macro_rules! descriptor {
    ($name:ident, $id:literal, $label:literal, $units:literal) => {
        static $name: FieldDescriptor = FieldDescriptor {
            id: FieldId($id),
            source: "Model comparison",
            family: FieldFamily::Model,
            display_name: $label,
            short_name: $label,
            search_aliases: &["difference", "comparison"],
            units: $units,
            value_kind: ValueKind::Scalar,
            palette_key: "derived.model-difference",
            sampling: SamplingPolicy::Bilinear,
            missing: MissingData::Nan,
            time_policy: None,
            supports_contours: false,
            supports_difference: false,
        };
    };
}

pub static MODEL_DIFF_DESCRIPTOR: FieldDescriptor = FieldDescriptor {
    id: FieldId("derived.model-difference"),
    source: "Model comparison",
    family: FieldFamily::Model,
    display_name: "Model difference",
    short_name: "Model difference",
    search_aliases: &["spread", "comparison"],
    units: "",
    value_kind: ValueKind::Scalar,
    palette_key: "derived.model-difference",
    sampling: SamplingPolicy::Bilinear,
    missing: MissingData::Nan,
    time_policy: None,
    supports_contours: false,
    supports_difference: false,
};
descriptor!(
    DIFF_MSLP,
    "derived.model-difference.mslp",
    "MSLP difference",
    "Pa"
);
descriptor!(
    DIFF_HEIGHT,
    "derived.model-difference.height-500",
    "500 hPa height difference",
    "m"
);
descriptor!(
    DIFF_TEMP,
    "derived.model-difference.temperature-2m",
    "2 m temperature difference",
    "K"
);
descriptor!(
    DIFF_DEWPOINT,
    "derived.model-difference.dewpoint-2m",
    "2 m dewpoint difference",
    "K"
);
descriptor!(
    DIFF_WIND,
    "derived.model-difference.wind-10m",
    "10 m wind difference",
    "m s-1"
);
descriptor!(
    DIFF_PRECIP,
    "derived.model-difference.precipitable-water",
    "Precipitable water difference",
    "kg m-2"
);
descriptor!(
    DIFF_CAPE,
    "derived.model-difference.cape",
    "CAPE difference",
    "J/kg"
);
descriptor!(
    DIFF_SRH,
    "derived.model-difference.srh",
    "SRH difference",
    "m2/s2"
);

/// Comparisons are scientific only when both operands describe the same valid instant.
pub fn same_valid_time(a: chrono::DateTime<chrono::Utc>, b: chrono::DateTime<chrono::Utc>) -> bool {
    a == b
}

/// What the difference layer is differencing, and therefore which two models it asks for.
///
/// ponytail: a fixed pair per field rather than two free model pickers. These are the two
/// comparisons forecasters actually make — global against global for the synoptic pattern, and
/// the two convection-scale models against each other — and each extra picker is a way to ask
/// for a pair that has no shared valid time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum DiffField {
    /// GFS − ECMWF, on the shared 0.3° lattice.
    Global(GlobalFieldKind),
    /// HRRR − RAP surface CAPE.
    Cape,
    /// HRRR − RAP storm-relative helicity.
    Srh,
}

/// `GlobalField` again, because that one is not `Hash`/`Serialize` and this is used as a settings
/// value and a map key. Converts both ways.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum GlobalFieldKind {
    Mslp,
    Height500,
    Temp2m,
    Dewpoint2m,
    Wind10m,
    Precip,
}

impl From<GlobalFieldKind> for GlobalField {
    fn from(k: GlobalFieldKind) -> GlobalField {
        match k {
            GlobalFieldKind::Mslp => GlobalField::Mslp,
            GlobalFieldKind::Height500 => GlobalField::Height500,
            GlobalFieldKind::Temp2m => GlobalField::Temp2m,
            GlobalFieldKind::Dewpoint2m => GlobalField::Dewpoint2m,
            GlobalFieldKind::Wind10m => GlobalField::Wind10m,
            GlobalFieldKind::Precip => GlobalField::Precip,
        }
    }
}

impl Default for DiffField {
    fn default() -> Self {
        DiffField::Global(GlobalFieldKind::Mslp)
    }
}

impl DiffField {
    pub fn descriptor(self) -> &'static FieldDescriptor {
        match self {
            DiffField::Global(GlobalFieldKind::Mslp) => &DIFF_MSLP,
            DiffField::Global(GlobalFieldKind::Height500) => &DIFF_HEIGHT,
            DiffField::Global(GlobalFieldKind::Temp2m) => &DIFF_TEMP,
            DiffField::Global(GlobalFieldKind::Dewpoint2m) => &DIFF_DEWPOINT,
            DiffField::Global(GlobalFieldKind::Wind10m) => &DIFF_WIND,
            DiffField::Global(GlobalFieldKind::Precip) => &DIFF_PRECIP,
            DiffField::Cape => &DIFF_CAPE,
            DiffField::Srh => &DIFF_SRH,
        }
    }
    /// Every pair uses the same physical quantity and native units. The column-moisture pair is
    /// GFS precipitable water against ECMWF total-column water, both kg/m² (numerically mm).
    pub const ALL: [DiffField; 8] = [
        DiffField::Global(GlobalFieldKind::Mslp),
        DiffField::Global(GlobalFieldKind::Height500),
        DiffField::Global(GlobalFieldKind::Temp2m),
        DiffField::Global(GlobalFieldKind::Dewpoint2m),
        DiffField::Global(GlobalFieldKind::Wind10m),
        DiffField::Global(GlobalFieldKind::Precip),
        DiffField::Cape,
        DiffField::Srh,
    ];

    /// Native units → display units, the same conversion the single-model ramps apply. Grids
    /// arrive as the model published them: pressure in Pa, height in m, wind in m/s.
    pub fn input_scale(self) -> f32 {
        match self {
            DiffField::Global(GlobalFieldKind::Mslp) => 0.01, // Pa → hPa
            DiffField::Global(GlobalFieldKind::Height500) => 0.1, // m → dam
            DiffField::Global(GlobalFieldKind::Wind10m) => 1.943_844, // m/s → kt
            // A difference of two Kelvin fields is already a difference in °C.
            _ => 1.0,
        }
    }

    /// Which two models, in the order they are subtracted.
    pub fn pair(self) -> (&'static str, &'static str) {
        match self {
            DiffField::Global(_) => ("GFS", "ECMWF"),
            DiffField::Cape | DiffField::Srh => ("HRRR", "RAP"),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            DiffField::Global(k) => GlobalField::from(k).label(),
            DiffField::Cape => "Surface CAPE",
            DiffField::Srh => "Storm-relative helicity",
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            DiffField::Global(k) => GlobalField::from(k).slug(),
            DiffField::Cape => "cape",
            DiffField::Srh => "srh",
        }
    }

    /// Full-scale difference and the deadband inside which the two models count as agreeing, in
    /// the field's own units. Both are eyeballed from what a meaningful spread looks like — an
    /// 8 hPa MSLP split is a different low, a 200 J/kg CAPE split is noise.
    pub fn range(self) -> (f32, f32) {
        match self {
            DiffField::Global(GlobalFieldKind::Mslp) => (8.0, 0.5),
            DiffField::Global(GlobalFieldKind::Height500) => (12.0, 1.0),
            DiffField::Global(GlobalFieldKind::Temp2m) => (6.0, 0.5),
            DiffField::Global(GlobalFieldKind::Dewpoint2m) => (6.0, 0.5),
            DiffField::Global(GlobalFieldKind::Wind10m) => (20.0, 2.0),
            DiffField::Global(GlobalFieldKind::Precip) => (20.0, 1.0),
            DiffField::Cape => (1500.0, 200.0),
            DiffField::Srh => (150.0, 25.0),
        }
    }

    /// Units, for the legend and the hover text.
    pub fn units(self) -> &'static str {
        match self {
            DiffField::Global(GlobalFieldKind::Mslp) => "hPa",
            DiffField::Global(GlobalFieldKind::Height500) => "dam",
            DiffField::Global(GlobalFieldKind::Temp2m) => "°C",
            DiffField::Global(GlobalFieldKind::Dewpoint2m) => "°C",
            DiffField::Global(GlobalFieldKind::Wind10m) => "kt",
            DiffField::Global(GlobalFieldKind::Precip) => "mm",

            DiffField::Cape => "J/kg",
            DiffField::Srh => "m²/s²",
        }
    }
}

pub fn diff_frames(
    a: &FieldFrame,
    b: &FieldFrame,
    descriptor: &'static FieldDescriptor,
) -> Option<FieldFrame> {
    if !same_valid_time(a.stamp.valid_time, b.stamp.valid_time) {
        return None;
    }
    let field = diff(a.field(), b.field())?;
    Some(FieldFrame::new(
        descriptor,
        field,
        DataStamp {
            source_identity: format!("{} − {}", a.stamp.source_identity, b.stamp.source_identity),
            issue_time: None,
            run_time: None,
            valid_time: a.stamp.valid_time,
            received_time: a.stamp.received_time.max(b.stamp.received_time),
            class: DataClass::Derived,
            quality: QualitySummary::Unknown,
            available_members: None,
        },
    ))
}

/// Deterministic forecast error over the common native domain.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct VerificationMetrics {
    pub samples: usize,
    pub bias: f64,
    pub mae: f64,
    pub rmse: f64,
}

pub fn verify_forecast(
    forecast: &FieldFrame,
    analysis: &FieldFrame,
) -> anyhow::Result<VerificationMetrics> {
    anyhow::ensure!(
        forecast.stamp.class == DataClass::Forecast,
        "first field is not a forecast"
    );
    anyhow::ensure!(
        matches!(analysis.stamp.class, DataClass::Analysis | DataClass::Observed),
        "reference field is not an analysis or observation"
    );
    anyhow::ensure!(
        forecast.stamp.valid_time == analysis.stamp.valid_time,
        "forecast and reference valid times differ"
    );
    anyhow::ensure!(
        forecast.descriptor.units == analysis.descriptor.units,
        "forecast and reference units differ"
    );
    let errors = diff(forecast.field(), analysis.field())
        .ok_or_else(|| anyhow::anyhow!("forecast and reference domains do not overlap"))?;
    metrics(errors.values)
}

/// Score a surface forecast or analysis against temporally matched METAR stations in native units.
pub fn verify_stations(
    field: &FieldFrame,
    observations: &[wxdata::metar::SurfaceOb],
    tolerance: chrono::Duration,
) -> anyhow::Result<VerificationMetrics> {
    anyhow::ensure!(
        matches!(field.stamp.class, DataClass::Forecast | DataClass::Analysis),
        "field is not a forecast or analysis"
    );
    let id = field.descriptor.id.0;
    anyhow::ensure!(
        matches!(
            id,
            "model.global.temperature-2m"
                | "model.global.dewpoint-2m"
                | "model.global.mslp"
                | "model.global.wind-10m"
                | "analysis.rtma.temperature-2m"
                | "analysis.rtma.dewpoint-2m"
                | "analysis.rtma.surface-pressure"
                | "analysis.rtma.wind-u-10m"
                | "analysis.urma.temperature-2m"
                | "analysis.urma.dewpoint-2m"
                | "analysis.urma.surface-pressure"
                | "analysis.urma.wind-u-10m"
        ),
        "field has no METAR verification mapping"
    );
    anyhow::ensure!(tolerance > chrono::Duration::zero(), "invalid time tolerance");
    metrics(observations.iter().filter_map(|ob| {
        let observed_at = chrono::DateTime::from_timestamp(ob.obs_time?, 0)?;
        if (observed_at - field.stamp.valid_time).abs() > tolerance {
            return None;
        }
        let observed = match id {
            "model.global.temperature-2m" | "analysis.rtma.temperature-2m" | "analysis.urma.temperature-2m" => ob.temp_c? + 273.15,
            "model.global.dewpoint-2m" | "analysis.rtma.dewpoint-2m" | "analysis.urma.dewpoint-2m" => ob.dewp_c? + 273.15,
            "model.global.mslp" | "analysis.rtma.surface-pressure" | "analysis.urma.surface-pressure" => ob.altim_mb? * 100.0,
            "model.global.wind-10m" | "analysis.rtma.wind-u-10m" | "analysis.urma.wind-u-10m" => {
                -ob.wspd_kt / 1.943_844 * ob.wdir_deg?.to_radians().sin()
            }
            _ => return None,
        };
        Some(field.sample(ob.lon, ob.lat).value? - observed)
    }))
}

fn metrics(errors: impl IntoIterator<Item = f32>) -> anyhow::Result<VerificationMetrics> {
    let (mut samples, mut sum, mut absolute, mut squared) = (0usize, 0.0, 0.0, 0.0);
    for error in errors.into_iter().filter(|value| value.is_finite()) {
        let error = f64::from(error);
        samples += 1;
        sum += error;
        absolute += error.abs();
        squared += error * error;
    }
    anyhow::ensure!(samples > 0, "common domain contains no valid samples");
    let count = samples as f64;
    Ok(VerificationMetrics {
        samples,
        bias: sum / count,
        mae: absolute / count,
        rmse: (squared / count).sqrt(),
    })
}

/// Bolton-style equivalent potential temperature from surface temperature, dewpoint, and pressure.
pub fn theta_e_k(temp_k: f32, dewpoint_k: f32, pressure_pa: f32) -> Option<f32> {
    if !(temp_k > 150.0 && dewpoint_k > 150.0 && pressure_pa > 10_000.0) {
        return None;
    }
    let pressure_hpa = pressure_pa / 100.0;
    let dewpoint_c = dewpoint_k - 273.15;
    let vapor_hpa = 6.112 * (17.67 * dewpoint_c / (dewpoint_c + 243.5)).exp();
    if vapor_hpa >= pressure_hpa {
        return None;
    }
    let mixing_ratio = 0.622 * vapor_hpa / (pressure_hpa - vapor_hpa);
    let lcl_k = 1.0 / (1.0 / (dewpoint_k - 56.0) + (temp_k / dewpoint_k).ln() / 800.0) + 56.0;
    let theta_e = temp_k
        * (1000.0 / pressure_hpa).powf(0.2854 * (1.0 - 0.28 * mixing_ratio))
        * ((3376.0 / lcl_k - 2.54) * mixing_ratio * (1.0 + 0.81 * mixing_ratio)).exp();
    theta_e.is_finite().then_some(theta_e)
}

/// Horizontal gradient magnitude at a point, expressed as native field units per 100 km.
pub fn gradient_per_100km(field: &MrmsField, lon: f64, lat: f64) -> Option<f32> {
    if field.nx < 3 || field.ny < 3 {
        return None;
    }
    let dlon = (field.lon_east - field.lon_west) / field.nx as f64;
    let dlat = (field.lat_north - field.lat_south) / field.ny as f64;
    let west = field.sample_bilinear(lon - dlon, lat)?;
    let east = field.sample_bilinear(lon + dlon, lat)?;
    let south = field.sample_bilinear(lon, lat - dlat)?;
    let north = field.sample_bilinear(lon, lat + dlat)?;
    let dx_km = 2.0 * dlon.abs() * 111.32 * lat.to_radians().cos().abs().max(0.01);
    let dy_km = 2.0 * dlat.abs() * 111.32;
    Some((((east - west) / dx_km as f32).hypot((north - south) / dy_km as f32)) * 100.0)
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObjectiveSurfacePoint {
    pub station: String,
    pub distance_km: f64,
    pub weight: f32,
    pub temperature_k: Option<f32>,
    pub dewpoint_k: Option<f32>,
}

/// Blend the nearest recent METAR innovation into an analysis background at one point.
/// The weight decays as `exp(-(distance / 75 km)^2)` and is zero beyond 150 km.
pub fn objective_surface_point(
    temperature: &FieldFrame,
    dewpoint: &FieldFrame,
    observations: &[wxdata::metar::SurfaceOb],
    lon: f64,
    lat: f64,
) -> Option<ObjectiveSurfacePoint> {
    if temperature.stamp.class != DataClass::Analysis
        || dewpoint.stamp.class != DataClass::Analysis
        || temperature.stamp.valid_time != dewpoint.stamp.valid_time
    {
        return None;
    }
    let background_t = temperature.sample(lon, lat).value;
    let background_td = dewpoint.sample(lon, lat).value;
    let (station, distance_km) = observations
        .iter()
        .filter(|ob| ob.obs_time
            .and_then(|time| chrono::DateTime::from_timestamp(time, 0))
            .is_some_and(|time| (time - temperature.stamp.valid_time).abs() <= chrono::Duration::minutes(90)))
        .map(|ob| {
            let dlat = (ob.lat - lat).to_radians();
            let dlon = (ob.lon - lon).to_radians();
            let a = (dlat / 2.0).sin().powi(2)
                + lat.to_radians().cos() * ob.lat.to_radians().cos() * (dlon / 2.0).sin().powi(2);
            (ob, 6371.0 * 2.0 * a.sqrt().atan2((1.0 - a).sqrt()))
        })
        .filter(|(_, distance)| *distance <= 150.0)
        .min_by(|a, b| a.1.total_cmp(&b.1))?;
    let weight = (-(distance_km / 75.0).powi(2)).exp() as f32;
    let blend = |background: Option<f32>, observed_c: Option<f32>| {
        background.zip(observed_c).map(|(background, observed)| {
            background + weight * (observed + 273.15 - background)
        })
    };
    Some(ObjectiveSurfacePoint {
        station: station.icao.clone(), distance_km, weight,
        temperature_k: blend(background_t, station.temp_c),
        dewpoint_k: blend(background_td, station.dewp_c),
    })
}

/// `a - b`, on the coarser of the two lattices, over the part of the world both cover.
///
/// The time is `a`'s: a difference is only meaningful for one instant, and the caller is
/// responsible for saying which two cycles it compared (see the layer options row).
pub fn diff(a: &MrmsField, b: &MrmsField) -> Option<MrmsField> {
    let lon_west = a.lon_west.max(b.lon_west);
    let lon_east = a.lon_east.min(b.lon_east);
    let lat_south = a.lat_south.max(b.lat_south);
    let lat_north = a.lat_north.min(b.lat_north);
    if lon_east <= lon_west || lat_north <= lat_south {
        return None; // disjoint domains: HRRR over CONUS against a regional model elsewhere
    }

    // Cell size of each input, then the coarser one, then how many of those fit in the overlap.
    let step = |f: &MrmsField| {
        (
            (f.lon_east - f.lon_west) / f.nx.max(2).saturating_sub(1) as f64,
            (f.lat_north - f.lat_south) / f.ny.max(2).saturating_sub(1) as f64,
        )
    };
    let (adx, ady) = step(a);
    let (bdx, bdy) = step(b);
    let (dx, dy) = (adx.max(bdx), ady.max(bdy));
    if dx <= 0.0 || dy <= 0.0 {
        return None;
    }
    let nx = ((lon_east - lon_west) / dx).round() as usize + 1;
    let ny = ((lat_north - lat_south) / dy).round() as usize + 1;
    if nx < 2 || ny < 2 {
        return None;
    }

    let mut values = Vec::with_capacity(nx * ny);
    for row in 0..ny {
        let lat = lat_north - row as f64 * dy;
        for col in 0..nx {
            let lon = lon_west + col as f64 * dx;
            values.push(match (sample(a, lon, lat), sample(b, lon, lat)) {
                (Some(x), Some(y)) => x - y,
                // Either model missing here means there is no difference to state. NaN is what
                // the rest of the field pipeline already reads as "no data".
                _ => f32::NAN,
            });
        }
    }
    Some(MrmsField {
        values,
        nx,
        ny,
        lon_west,
        lon_east: lon_west + (nx - 1) as f64 * dx,
        lat_north,
        lat_south: lat_north - (ny - 1) as f64 * dy,
        time: a.time,
    })
}

/// Bilinear sample at a lat/lon, or `None` outside the grid or against missing data.
fn sample(f: &MrmsField, lon: f64, lat: f64) -> Option<f32> {
    if f.nx < 2 || f.ny < 2 {
        return None;
    }
    let dx = (f.lon_east - f.lon_west) / (f.nx - 1) as f64;
    let dy = (f.lat_north - f.lat_south) / (f.ny - 1) as f64;
    if dx <= 0.0 || dy <= 0.0 {
        return None;
    }
    // Row 0 is the northernmost latitude, so y counts downward from lat_north.
    let x = (lon - f.lon_west) / dx;
    let y = (f.lat_north - lat) / dy;
    if x < 0.0 || y < 0.0 || x > (f.nx - 1) as f64 || y > (f.ny - 1) as f64 {
        return None;
    }
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(f.nx - 1), (y0 + 1).min(f.ny - 1));
    let (tx, ty) = ((x - x0 as f64) as f32, (y - y0 as f64) as f32);
    let at = |r: usize, c: usize| {
        let v = f.values[r * f.nx + c];
        if v.is_finite() {
            Some(v)
        } else {
            None
        }
    };
    // One missing corner poisons the cell rather than being treated as zero — a hole in a model
    // field is not a value of zero, and a difference against zero is a fabricated gradient.
    let (v00, v01, v10, v11) = (at(y0, x0)?, at(y0, x1)?, at(y1, x0)?, at(y1, x1)?);
    let top = v00 + (v01 - v00) * tx;
    let bottom = v10 + (v11 - v10) * tx;
    Some(top + (bottom - top) * ty)
}

/// Blue-white-red across ±`range`, with everything inside `deadband` fully transparent.
///
/// The deadband is the point of the layer: models agreeing is the common case and drawing it
/// would bury the disagreement under a wash of near-white. 256 entries, RGBA, index 128 = zero —
/// the same 256×1 LUT shape every other field layer uploads.
pub fn diverging_lut(range: f32, deadband: f32) -> Vec<u8> {
    let mut lut = Vec::with_capacity(256 * 4);
    for i in 0..256 {
        let t = (i as f32 / 255.0) * 2.0 - 1.0; // −1..1
        let v = t * range;
        if v.abs() <= deadband {
            lut.extend_from_slice(&[0, 0, 0, 0]);
            continue;
        }
        // Ramp opacity in from the deadband edge so the field has no hard rim around agreement.
        let mag = ((v.abs() - deadband) / (range - deadband).max(1e-6)).clamp(0.0, 1.0);
        let alpha = (60.0 + 195.0 * mag) as u8;
        let (r, g, b) = if t < 0.0 {
            // b's value is higher: cool.
            (
                (255.0 * (1.0 - mag)) as u8,
                (255.0 * (1.0 - 0.45 * mag)) as u8,
                255,
            )
        } else {
            (
                255,
                (255.0 * (1.0 - 0.75 * mag)) as u8,
                (255.0 * (1.0 - mag)) as u8,
            )
        };
        lut.extend_from_slice(&[r, g, b, alpha]);
    }
    lut
}

/// Value → LUT index, symmetric about zero. `NaN` (no data on either side) maps to the deadband,
/// which the LUT draws as nothing.
pub fn diff_index(v: f32, range: f32) -> u8 {
    if !v.is_finite() {
        return 128;
    }
    (((v / range).clamp(-1.0, 1.0) + 1.0) * 127.5) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(
        nx: usize,
        ny: usize,
        west: f64,
        east: f64,
        south: f64,
        north: f64,
        v: f32,
    ) -> MrmsField {
        MrmsField {
            values: vec![v; nx * ny],
            nx,
            ny,
            lon_west: west,
            lon_east: east,
            lat_north: north,
            lat_south: south,
            time: chrono::Utc::now(),
        }
    }

    #[test]
    fn identical_lattices_subtract_cell_by_cell() {
        let a = grid(11, 11, -100.0, -90.0, 30.0, 40.0, 8.0);
        let b = grid(11, 11, -100.0, -90.0, 30.0, 40.0, 5.0);
        let d = diff(&a, &b).expect("overlapping");
        assert_eq!((d.nx, d.ny), (11, 11));
        assert!(d.values.iter().all(|v| (v - 3.0).abs() < 1e-4));
    }

    #[test]
    fn frame_difference_keeps_both_contributors_and_exact_time() {
        let valid = chrono::Utc::now();
        let make = |value, source: &str| {
            FieldFrame::new(
                &wxdata::global::MSLP_DESCRIPTOR,
                grid(2, 2, -100.0, -99.0, 39.0, 40.0, value),
                DataStamp {
                    source_identity: source.into(),
                    issue_time: None,
                    run_time: None,
                    valid_time: valid,
                    received_time: valid,
                    class: DataClass::Forecast,
                    quality: QualitySummary::Unknown,
                    available_members: None,
                },
            )
        };
        let frame = diff_frames(&make(8.0, "gfs"), &make(5.0, "ecmwf"), &DIFF_MSLP)
            .expect("compatible frames");
        assert_eq!(frame.stamp.class, DataClass::Derived);
        assert_eq!(frame.stamp.valid_time, valid);
        assert_eq!(frame.stamp.source_identity, "gfs − ecmwf");
        assert_eq!(frame.sample(-99.5, 39.5).value, Some(3.0));
    }

    #[test]
    fn the_coarser_lattice_wins_and_the_overlap_clips() {
        // Fine grid over half the domain of a coarse one.
        let fine = grid(101, 101, -100.0, -95.0, 30.0, 35.0, 10.0);
        let coarse = grid(11, 11, -100.0, -90.0, 30.0, 40.0, 4.0);
        let d = diff(&fine, &coarse).expect("overlapping");
        assert_eq!((d.lon_west, d.lon_east), (-100.0, -95.0));
        assert_eq!((d.lat_south, d.lat_north), (30.0, 35.0));
        // 1° cells from the coarse grid across a 5° overlap.
        assert_eq!((d.nx, d.ny), (6, 6));
        assert!(d.values.iter().all(|v| (v - 6.0).abs() < 1e-4));
    }

    #[test]
    fn disjoint_domains_produce_nothing() {
        let a = grid(11, 11, -100.0, -90.0, 30.0, 40.0, 1.0);
        let b = grid(11, 11, 10.0, 20.0, 30.0, 40.0, 1.0);
        assert!(diff(&a, &b).is_none());
    }

    #[test]
    fn a_hole_in_either_model_is_a_hole_in_the_difference() {
        let mut a = grid(11, 11, -100.0, -90.0, 30.0, 40.0, 8.0);
        a.values[0] = f32::NAN;
        let b = grid(11, 11, -100.0, -90.0, 30.0, 40.0, 5.0);
        let d = diff(&a, &b).unwrap();
        assert!(d.values[0].is_nan());
        assert!((d.values[d.values.len() - 1] - 3.0).abs() < 1e-4);
    }

    #[test]
    fn agreement_draws_nothing_and_disagreement_ramps() {
        let lut = diverging_lut(10.0, 1.0);
        let alpha = |v: f32| lut[diff_index(v, 10.0) as usize * 4 + 3];
        assert_eq!(alpha(0.0), 0, "models agreeing is invisible");
        assert_eq!(alpha(0.5), 0, "inside the deadband is invisible");
        assert!(
            alpha(5.0) > 0 && alpha(10.0) > alpha(5.0),
            "further apart, more opaque"
        );
        // Sign picks the side of the ramp: blue for negative, red for positive.
        let rgb = |v: f32| {
            let i = diff_index(v, 10.0) as usize * 4;
            (lut[i], lut[i + 2])
        };
        assert!(rgb(-9.0).1 > rgb(-9.0).0, "negative is blue");
        assert!(rgb(9.0).0 > rgb(9.0).1, "positive is red");
    }

    #[test]
    fn comparisons_require_the_same_valid_instant() {
        let a = chrono::Utc::now();
        assert!(same_valid_time(a, a));
        assert!(!same_valid_time(a, a + chrono::Duration::minutes(1)));
    }

    #[test]
    fn verification_scores_only_compatible_valid_native_values() {
        let valid = chrono::Utc::now();
        let make = |value, class: DataClass| {
            FieldFrame::new(
                &wxdata::global::MSLP_DESCRIPTOR,
                grid(2, 2, -100.0, -99.0, 39.0, 40.0, value),
                DataStamp {
                    source_identity: class.label().into(),
                    issue_time: None,
                    run_time: None,
                    valid_time: valid,
                    received_time: valid,
                    class,
                    quality: QualitySummary::Good,
                    available_members: None,
                },
            )
        };
        let score = verify_forecast(
            &make(8.0, DataClass::Forecast),
            &make(5.0, DataClass::Analysis),
        )
        .unwrap();
        assert_eq!(score.samples, 4);
        assert_eq!((score.bias, score.mae, score.rmse), (3.0, 3.0, 3.0));
        assert!(verify_forecast(
            &make(8.0, DataClass::Analysis),
            &make(5.0, DataClass::Analysis)
        )
        .is_err());
    }

    #[test]
    fn station_verification_converts_units_and_rejects_stale_observations() {
        let valid = chrono::Utc::now();
        let forecast = FieldFrame::new(
            &wxdata::global::TEMP_2M_DESCRIPTOR,
            grid(2, 2, -100.0, -99.0, 39.0, 40.0, 300.0),
            DataStamp {
                source_identity: "gfs".into(),
                issue_time: None,
                run_time: None,
                valid_time: valid,
                received_time: valid,
                class: DataClass::Forecast,
                quality: QualitySummary::Good,
                available_members: None,
            },
        );
        let observation = |minutes: i64| wxdata::metar::SurfaceOb {
            icao: "KTEST".into(),
            name: "Test".into(),
            lat: 39.5,
            lon: -99.5,
            temp_c: Some(25.85),
            dewp_c: None,
            wdir_deg: None,
            wspd_kt: 0.0,
            wgst_kt: None,
            altim_mb: None,
            elev_m: None,
            obs_time: Some((valid + chrono::Duration::minutes(minutes)).timestamp()),
            flt_cat: String::new(),
            wvht_ft: None,
            dpd_s: None,
            raw: String::new(),
        };
        let score = verify_stations(
            &forecast,
            &[observation(30), observation(180)],
            chrono::Duration::minutes(90),
        )
        .unwrap();
        assert_eq!(score.samples, 1);
        assert!((score.bias - 1.0).abs() < 0.001);

        let analysis = FieldFrame::new(
            &wxdata::rtma::TEMP_DESCRIPTOR,
            grid(2, 2, -100.0, -99.0, 39.0, 40.0, 299.0),
            DataStamp {
                source_identity: "rtma".into(),
                issue_time: None,
                run_time: None,
                valid_time: valid,
                received_time: valid,
                class: DataClass::Analysis,
                quality: QualitySummary::Good,
                available_members: None,
            },
        );
        let score = verify_stations(
            &analysis,
            &[observation(0)],
            chrono::Duration::minutes(90),
        )
        .unwrap();
        assert!((score.bias).abs() < 0.001);
    }

    #[test]
    fn surface_diagnostics_are_bounded_and_physical() {
        let theta_e = theta_e_k(303.15, 293.15, 100_000.0).unwrap();
        assert!((340.0..=350.0).contains(&theta_e), "{theta_e}");
        assert!(theta_e_k(f32::NAN, 293.15, 100_000.0).is_none());

        let mut values = Vec::new();
        for _row in 0..5 {
            values.extend([0.0, 1.0, 2.0, 3.0, 4.0]);
        }
        let field = grid(5, 5, -2.5, 2.5, -2.5, 2.5, 0.0);
        let field = MrmsField { values, ..field };
        let gradient = gradient_per_100km(&field, 0.0, 0.0).unwrap();
        assert!((0.89..=0.91).contains(&gradient), "{gradient}");
    }

    #[test]
    fn objective_analysis_blends_only_a_nearby_recent_observation() {
        let valid = chrono::Utc::now();
        let frame = |descriptor, value| FieldFrame::new(
            descriptor,
            grid(2, 2, -100.0, -99.0, 39.0, 40.0, value),
            DataStamp {
                source_identity: "rtma".into(), issue_time: None, run_time: None,
                valid_time: valid, received_time: valid, class: DataClass::Analysis,
                quality: QualitySummary::Good, available_members: None,
            },
        );
        let observation = wxdata::metar::SurfaceOb {
            icao: "KTEST".into(), name: "Test".into(), lat: 39.5, lon: -99.5,
            temp_c: Some(30.0), dewp_c: Some(20.0), wdir_deg: None, wspd_kt: 0.0,
            wgst_kt: None, altim_mb: None, elev_m: None, obs_time: Some(valid.timestamp()),
            flt_cat: String::new(), wvht_ft: None, dpd_s: None, raw: String::new(),
        };
        let blend = objective_surface_point(
            &frame(&wxdata::rtma::TEMP_DESCRIPTOR, 300.0),
            &frame(&wxdata::rtma::DEWPOINT_DESCRIPTOR, 290.0),
            &[observation], -99.5, 39.5,
        ).unwrap();
        assert_eq!(blend.station, "KTEST");
        assert_eq!(blend.weight, 1.0);
        assert!((blend.temperature_k.unwrap() - 303.15).abs() < 0.001);
        assert!((blend.dewpoint_k.unwrap() - 293.15).abs() < 0.001);
    }
}
