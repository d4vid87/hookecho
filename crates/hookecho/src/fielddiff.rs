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
    let vapor_hpa = vapor_pressure_hpa(dewpoint_k)?;
    if vapor_hpa >= pressure_hpa { return None; }
    let mixing_ratio = 0.622 * vapor_hpa / (pressure_hpa - vapor_hpa);
    let lcl_k = 1.0 / (1.0 / (dewpoint_k - 56.0) + (temp_k / dewpoint_k).ln() / 800.0) + 56.0;
    let theta_e = temp_k
        * (1000.0 / pressure_hpa).powf(0.2854 * (1.0 - 0.28 * mixing_ratio))
        * ((3376.0 / lcl_k - 2.54) * mixing_ratio * (1.0 + 0.81 * mixing_ratio)).exp();
    theta_e.is_finite().then_some(theta_e)
}

fn vapor_pressure_hpa(dewpoint_k: f32) -> Option<f32> {
    if !(150.0..350.0).contains(&dewpoint_k) { return None; }
    let c = dewpoint_k - 273.15;
    let vapor = 6.112 * (17.67 * c / (c + 243.5)).exp();
    (vapor.is_finite() && vapor > 0.0).then_some(vapor)
}

/// Specific humidity from 2 m dewpoint and surface pressure, kg/kg.
pub fn specific_humidity_kg_kg(dewpoint_k: f32, pressure_pa: f32) -> Option<f32> {
    let pressure_hpa = pressure_pa / 100.0;
    if !(200.0..1200.0).contains(&pressure_hpa) { return None; }
    let vapor = vapor_pressure_hpa(dewpoint_k)?;
    if vapor >= pressure_hpa { return None; }
    let q = 0.622 * vapor / (pressure_hpa - 0.378 * vapor);
    (q.is_finite() && (0.0..0.1).contains(&q)).then_some(q)
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

/// Different fields occupy different byte ranges of the same analysis GRIB object.
/// Compare the immutable object and valid hour, while keeping each exact range in provenance.
pub fn same_analysis_object(a: &FieldFrame, b: &FieldFrame) -> bool {
    a.stamp.valid_time == b.stamp.valid_time
        && same_analysis_identity(&a.stamp.source_identity, &b.stamp.source_identity)
}

fn same_analysis_identity(a: &str, b: &str) -> bool {
    a.split_once("#bytes=").map_or(a, |(object, _)| object)
        == b.split_once("#bytes=").map_or(b, |(object, _)| object)
}

pub fn same_native_analysis(a: &wxdata::rtma::NativeSamplePair, b: &wxdata::rtma::NativeSamplePair) -> bool {
    a.valid_time == b.valid_time && same_analysis_identity(&a.source_identity, &b.source_identity)
}

/// At least a 30 km centered span avoids treating one RTMA grid-cell fluctuation as a
/// mesoscale gradient. Coarser native grids keep their own cell spacing.
fn surface_derivative_step(field: &MrmsField, lat: f64) -> Option<(f64, f64, f32, f32)> {
    if field.nx < 3 || field.ny < 3 { return None; }
    let cos_lat = lat.to_radians().cos().abs();
    if cos_lat < 0.01 { return None; }
    let dlon = ((field.lon_east - field.lon_west) / field.nx as f64).abs()
        .max(15_000.0 / (111_320.0 * cos_lat));
    let dlat = ((field.lat_north - field.lat_south) / field.ny as f64).abs()
        .max(15_000.0 / 111_320.0);
    let dx_m = (2.0 * dlon * 111_320.0 * cos_lat) as f32;
    let dy_m = (2.0 * dlat * 111_320.0) as f32;
    (dx_m.is_finite() && dy_m.is_finite() && dx_m >= 1.0 && dy_m >= 1.0)
        .then_some((dlon, dlat, dx_m, dy_m))
}

/// Horizontal temperature advection by a matched 10 m wind, in K/h (also °C/h).
/// This is a surface diagnostic, not a parcel trajectory or a forecast tendency.
pub fn temperature_advection_k_per_h(
    temperature: &MrmsField, lon: f64, lat: f64, u_ms: f32, v_ms: f32,
) -> Option<f32> {
    if !u_ms.is_finite() || !v_ms.is_finite() { return None; }
    let (dlon, dlat, dx_m, dy_m) = surface_derivative_step(temperature, lat)?;
    let west = temperature.sample_bilinear(lon - dlon, lat)?;
    let east = temperature.sample_bilinear(lon + dlon, lat)?;
    let south = temperature.sample_bilinear(lon, lat - dlat)?;
    let north = temperature.sample_bilinear(lon, lat + dlat)?;
    let value = -(u_ms * (east - west) / dx_m
        + v_ms * (north - south) / dy_m) * 3600.0;
    value.is_finite().then_some(value)
}

/// Horizontal convergence of q·wind, expressed as g/kg/h at the map point.
/// Uses 2 m humidity with 10 m wind, so this is a near-surface proxy rather than a
/// vertically integrated moisture budget or an observed humidity tendency.
pub fn moisture_flux_convergence_g_kg_h(
    dewpoint: &MrmsField, pressure: &MrmsField, u: &MrmsField, v: &MrmsField,
    lon: f64, lat: f64,
) -> Option<f32> {
    let (dlon, dlat, dx_m, dy_m) = surface_derivative_step(dewpoint, lat)?;
    let flux = |x, y| -> Option<(f32, f32)> {
        let q = specific_humidity_kg_kg(
            dewpoint.sample_bilinear(x, y)?, pressure.sample_bilinear(x, y)?,
        )?;
        Some((q * u.sample_bilinear(x, y)?, q * v.sample_bilinear(x, y)?))
    };
    let (west, _) = flux(lon - dlon, lat)?;
    let (east, _) = flux(lon + dlon, lat)?;
    let (_, south) = flux(lon, lat - dlat)?;
    let (_, north) = flux(lon, lat + dlat)?;
    let convergence = -((east - west) / dx_m
        + (north - south) / dy_m) * 3_600_000.0;
    convergence.is_finite().then_some(convergence)
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObjectiveSurfacePoint {
    pub station: String,
    pub analysis_source: &'static str,
    pub analysis_identity: String,
    pub analysis_received_time: chrono::DateTime<chrono::Utc>,
    pub distance_km: f64,
    pub weight: f32,
    pub temperature_k: Option<f32>,
    pub dewpoint_k: Option<f32>,
    /// Observation minus native analysis at the station, in kelvin (same increment as °C).
    pub temperature_residual_k: Option<f32>,
    pub dewpoint_residual_k: Option<f32>,
}

/// The observation chosen for a surface adjustment. A sounding requires both T and Td.
#[derive(Debug, Clone)]
pub struct ChosenSurfaceObservation {
    pub id: String,
    pub lon: f64,
    pub lat: f64,
    pub temp_c: Option<f32>,
    pub dewpoint_c: Option<f32>,
    pub distance_km: f64,
}

impl ChosenSurfaceObservation {
    fn weight(&self) -> f32 { (-(self.distance_km / 75.0).powi(2)).exp() as f32 }
}

/// Native-grid temperature and dewpoint at the point and observation station.
pub fn objective_surface_native(
    temperature: &wxdata::rtma::NativeSamplePair,
    dewpoint: &wxdata::rtma::NativeSamplePair,
    chosen: &ChosenSurfaceObservation,
    source: wxdata::rtma::Source,
) -> Option<ObjectiveSurfacePoint> {
    if temperature.valid_time != dewpoint.valid_time
        || !same_analysis_identity(&temperature.source_identity, &dewpoint.source_identity)
    { return None; }
    let observed_t = chosen.temp_c?;
    let observed_td = chosen.dewpoint_c?;
    let weight = chosen.weight();
    Some(ObjectiveSurfacePoint {
        station: chosen.id.clone(), analysis_source: wxdata::rtma::SurfaceField::Temperature2m.descriptor_for(source).source,
        analysis_identity: temperature.source_identity.clone(), analysis_received_time: temperature.received_time,
        distance_km: chosen.distance_km, weight,
        temperature_k: Some(temperature.point + weight * (observed_t + 273.15 - temperature.point)),
        dewpoint_k: Some(dewpoint.point + weight * (observed_td + 273.15 - dewpoint.point)),
        temperature_residual_k: Some(observed_t + 273.15 - temperature.station),
        dewpoint_residual_k: Some(observed_td + 273.15 - dewpoint.station),
    })
}

pub fn nearest_surface_observation(
    valid: chrono::DateTime<chrono::Utc>,
    observations: &[wxdata::metar::SurfaceOb],
    stations: &[wxdata::stations::StationOb],
    lon: f64,
    lat: f64,
    require_both: bool,
) -> Option<ChosenSurfaceObservation> {
    // The analysis poll carries METARs in `stations` even when its layer is off.
    let all = stations.iter()
        .map(|ob| (ob.id.as_str(), Some(ob.network), ob.lon, ob.lat, ob.temp_c, ob.dewp_c, ob.time))
        .chain(observations.iter().map(|ob| (ob.icao.as_str(), None, ob.lon, ob.lat,
            ob.temp_c, ob.dewp_c,
            ob.obs_time.and_then(|time| chrono::DateTime::from_timestamp(time, 0)))));
    let ((id, network, station_lon, station_lat, temp_c, dewpoint_c, _), distance_km) = all
        .filter(|(_, _, _, _, temp, dewpoint, time)| {
            (if require_both {
                temp.is_some_and(f32::is_finite) && dewpoint.is_some_and(f32::is_finite)
            } else {
                temp.is_some_and(f32::is_finite) || dewpoint.is_some_and(f32::is_finite)
            }) && time.is_some_and(|time| (time - valid).abs() <= chrono::Duration::minutes(90))
        })
        .map(|ob| {
            let dlat = (ob.3 - lat).to_radians();
            let dlon = (ob.2 - lon).to_radians();
            let a = (dlat / 2.0).sin().powi(2)
                + lat.to_radians().cos() * ob.3.to_radians().cos() * (dlon / 2.0).sin().powi(2);
            (ob, 6371.0 * 2.0 * a.sqrt().atan2((1.0 - a).sqrt()))
        })
        .filter(|(_, distance)| *distance <= 150.0)
        .min_by(|a, b| a.1.total_cmp(&b.1))?;
    Some(ChosenSurfaceObservation {
        id: network.map_or_else(|| id.to_string(), |network| format!("{}:{id}", network.label())),
        lon: station_lon, lat: station_lat, temp_c, dewpoint_c, distance_km,
    })
}

/// Blend the nearest recent surface observation into an analysis background at one point.
/// The weight decays as `exp(-(distance / 75 km)^2)` and is zero beyond 150 km.
pub fn objective_surface_point(
    temperature: &FieldFrame,
    dewpoint: &FieldFrame,
    observations: &[wxdata::metar::SurfaceOb],
    stations: &[wxdata::stations::StationOb],
    lon: f64,
    lat: f64,
) -> Option<ObjectiveSurfacePoint> {
    objective_surface_point_with(temperature, dewpoint, observations, stations, lon, lat, false)
}

fn objective_surface_point_with(
    temperature: &FieldFrame,
    dewpoint: &FieldFrame,
    observations: &[wxdata::metar::SurfaceOb],
    stations: &[wxdata::stations::StationOb],
    lon: f64,
    lat: f64,
    require_both: bool,
) -> Option<ObjectiveSurfacePoint> {
    if temperature.stamp.class != DataClass::Analysis
        || dewpoint.stamp.class != DataClass::Analysis
        || !same_analysis_object(temperature, dewpoint)
    {
        return None;
    }
    let background_t = temperature.sample(lon, lat).value;
    let background_td = dewpoint.sample(lon, lat).value;
    let chosen = nearest_surface_observation(
        temperature.stamp.valid_time, observations, stations, lon, lat, require_both,
    )?;
    let weight = chosen.weight();
    let blend = |background: Option<f32>, observed_c: Option<f32>| {
        background.zip(observed_c).map(|(background, observed)| {
            background + weight * (observed + 273.15 - background)
        })
    };
    Some(ObjectiveSurfacePoint {
        station: chosen.id.clone(), distance_km: chosen.distance_km, weight,
        analysis_source: temperature.descriptor.source,
        analysis_identity: temperature.stamp.source_identity.clone(), analysis_received_time: temperature.stamp.received_time,
        temperature_k: blend(background_t, chosen.temp_c),
        dewpoint_k: blend(background_td, chosen.dewpoint_c),
        temperature_residual_k: chosen.temp_c.zip(temperature.sample(chosen.lon, chosen.lat).value)
            .map(|(observed, analysis)| observed + 273.15 - analysis),
        dewpoint_residual_k: chosen.dewpoint_c.zip(dewpoint.sample(chosen.lon, chosen.lat).value)
            .map(|(observed, analysis)| observed + 273.15 - analysis),
    })
}

/// Same boundary replacement for fields acquired at the clicked point without map layers.
pub fn objective_sounding_surface(
    model: &wxdata::sounding::Sounding,
    point: ObjectiveSurfacePoint,
    pressure_pa: f32,
    wind_u_ms: f32,
    wind_v_ms: f32,
) -> Option<(wxdata::sounding::Sounding, ObjectiveSurfacePoint)> {
    if model.fh != 0 { return None; }
    let temp_k = point.temperature_k?;
    let dewpoint_k = point.dewpoint_k?;
    let pressure_hpa = (pressure_pa / 100.0) as f64;
    let u = wind_u_ms as f64;
    let v = wind_v_ms as f64;
    if !(650.0..=1050.0).contains(&pressure_hpa)
        || ![temp_k, dewpoint_k].into_iter().all(f32::is_finite)
        || !u.is_finite()
        || !v.is_finite()
    {
        return None;
    }
    let mut levels = vec![wxdata::sounding::SoundingLevel {
        pressure_hpa,
        temp_c: (temp_k - 273.15) as f64,
        dewpt_c: (dewpoint_k.min(temp_k) - 273.15) as f64,
        u_ms: u,
        v_ms: v,
    }];
    levels.extend(
        model
            .levels
            .iter()
            .copied()
            .filter(|level| level.pressure_hpa < pressure_hpa - 1.0),
    );
    (levels.len() >= 3).then_some((
        wxdata::sounding::Sounding {
            lon: model.lon,
            lat: model.lat,
            run: model.run,
            fh: model.fh,
            levels,
        },
        point,
    ))
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
    fn analysis_fields_match_the_object_not_their_distinct_byte_ranges() {
        let valid = chrono::Utc::now();
        let frame = |identity: &str, at| FieldFrame::new(
            &wxdata::rtma::TEMP_DESCRIPTOR,
            grid(2, 2, -100.0, -99.0, 39.0, 40.0, 300.0),
            DataStamp {
                source_identity: identity.into(), issue_time: None, run_time: None,
                valid_time: at, received_time: at, class: DataClass::Analysis,
                quality: QualitySummary::Good, available_members: None,
            },
        );
        let a = frame("https://example.test/rtma.grb2#bytes=0-10", valid);
        let b = frame("https://example.test/rtma.grb2#bytes=11-20", valid);
        assert!(same_analysis_object(&a, &b));
        assert!(!same_analysis_object(&a, &frame("https://example.test/urma.grb2#bytes=11-20", valid)));
        assert!(!same_analysis_object(&a, &frame("https://example.test/rtma.grb2#bytes=11-20", valid + chrono::Duration::hours(1))));
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
        let adv = temperature_advection_k_per_h(&field, 0.0, 0.0, 10.0, 0.0).unwrap();
        assert!((-0.33..-0.31).contains(&adv), "{adv}");
        let mut north_warmer = field.clone();
        north_warmer.values = (0..5).flat_map(|row| [4.0 - row as f32; 5]).collect();
        let meridional = temperature_advection_k_per_h(&north_warmer, 0.0, 0.0, 0.0, 10.0).unwrap();
        assert!((-0.33..-0.31).contains(&meridional), "{meridional}");
        assert!(temperature_advection_k_per_h(&field, 0.0, 0.0, f32::NAN, 0.0).is_none());
        let mut missing = field.clone();
        missing.values.fill(f32::NAN);
        assert!(temperature_advection_k_per_h(&missing, 0.0, 0.0, 10.0, 0.0).is_none());
    }

    #[test]
    fn surface_moisture_convergence_uses_signed_flux_and_rejects_missing_values() {
        let fine = grid(5, 5, -0.25, 0.25, -0.25, 0.25, 293.15);
        let (_, _, dx, dy) = surface_derivative_step(&fine, 0.0).unwrap();
        assert!((29_999.0..30_001.0).contains(&dx));
        assert!((29_999.0..30_001.0).contains(&dy));
        let q = specific_humidity_kg_kg(293.15, 100_000.0).unwrap();
        assert!((0.014..0.015).contains(&q), "{q}");
        assert!(specific_humidity_kg_kg(293.15, 0.0).is_none());
        let td = grid(5, 5, -2.5, 2.5, -2.5, 2.5, 293.15);
        let pressure = grid(5, 5, -2.5, 2.5, -2.5, 2.5, 100_000.0);
        let v = grid(5, 5, -2.5, 2.5, -2.5, 2.5, 0.0);
        let mut u = grid(5, 5, -2.5, 2.5, -2.5, 2.5, 0.0);
        for row in u.values.chunks_mut(5) {
            row.copy_from_slice(&[4.0, 3.0, 2.0, 1.0, 0.0]);
        }
        let convergence = moisture_flux_convergence_g_kg_h(&td, &pressure, &u, &v, 0.0, 0.0).unwrap();
        assert!((0.4..0.6).contains(&convergence), "{convergence}");
        for row in u.values.chunks_mut(5) { row.reverse(); }
        let divergence = moisture_flux_convergence_g_kg_h(&td, &pressure, &u, &v, 0.0, 0.0).unwrap();
        assert!((-0.6..-0.4).contains(&divergence), "{divergence}");
        let mut missing = td.clone();
        missing.values.fill(f32::NAN);
        assert!(moisture_flux_convergence_g_kg_h(&missing, &pressure, &u, &v, 0.0, 0.0).is_none());
    }

    #[tokio::test]
    #[ignore = "network"]
    async fn live_surface_moisture_contract_uses_one_valid_analysis_object() {
        use wxdata::rtma::{fetch_latest_rtma, fetch_rtma, SurfaceField};
        let http = reqwest::Client::new();
        let v = fetch_latest_rtma(&http, SurfaceField::WindV10m).await.unwrap();
        let at = v.stamp.valid_time;
        let td = fetch_rtma(&http, SurfaceField::Dewpoint2m, at).await.unwrap();
        let p = fetch_rtma(&http, SurfaceField::Pressure, at).await.unwrap();
        let u = fetch_rtma(&http, SurfaceField::WindU10m, at).await.unwrap();
        for frame in [&td, &p, &u] { assert!(same_analysis_object(&v, frame)); }
        let value = moisture_flux_convergence_g_kg_h(
            td.field(), p.field(), u.field(), v.field(), -97.3, 32.6,
        ).expect("native analysis covers Dallas–Fort Worth");
        assert!(value.is_finite());
        eprintln!("RTMA near-surface convergence {value:+.2} g/kg/h at {at}");
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
        let metar_station = wxdata::stations::from_metars(std::slice::from_ref(&observation));
        let blend = objective_surface_point(
            &frame(&wxdata::rtma::TEMP_DESCRIPTOR, 300.0),
            &frame(&wxdata::rtma::DEWPOINT_DESCRIPTOR, 290.0),
            &[observation], &[], -99.5, 39.5,
        ).unwrap();
        assert_eq!(blend.station, "KTEST");
        assert_eq!(blend.weight, 1.0);
        assert!((blend.temperature_k.unwrap() - 303.15).abs() < 0.001);
        assert!((blend.dewpoint_k.unwrap() - 293.15).abs() < 0.001);
        assert!((blend.temperature_residual_k.unwrap() - 3.15).abs() < 0.001);
        assert!((blend.dewpoint_residual_k.unwrap() - 3.15).abs() < 0.001);
        let hidden_layer_blend = objective_surface_point(
            &frame(&wxdata::rtma::TEMP_DESCRIPTOR, 300.0),
            &frame(&wxdata::rtma::DEWPOINT_DESCRIPTOR, 290.0),
            &[], &metar_station, -99.5, 39.5,
        ).unwrap();
        assert_eq!(hidden_layer_blend.station, "METAR:KTEST");
        assert_eq!(hidden_layer_blend.temperature_k, blend.temperature_k);
        let mut unmatched = frame(&wxdata::rtma::DEWPOINT_DESCRIPTOR, 290.0);
        unmatched.stamp.source_identity = "urma".into();
        assert!(objective_surface_point(
            &frame(&wxdata::rtma::TEMP_DESCRIPTOR, 300.0), &unmatched,
            &[], &[], -99.5, 39.5,
        ).is_none());

        let personal = wxdata::stations::StationOb {
            id: "home".into(), name: "Home".into(),
            network: wxdata::stations::Network::Tempest,
            lat: 39.5, lon: -99.5, time: Some(valid),
            temp_c: Some(31.0), dewp_c: Some(21.0), rh_pct: None,
            wdir_deg: None, wspd_kt: None, gust_kt: None, pressure_mb: None,
            precip_rate_mmh: None, elev_m: None,
        };
        let blend = objective_surface_point(
            &frame(&wxdata::rtma::TEMP_DESCRIPTOR, 300.0),
            &frame(&wxdata::rtma::DEWPOINT_DESCRIPTOR, 290.0),
            &[], &[personal], -99.5, 39.5,
        ).unwrap();
        assert_eq!(blend.station, "Tempest:home");
        assert!((blend.temperature_k.unwrap() - 304.15).abs() < 0.001);
    }
    #[test]
    fn objective_sounding_replaces_only_matching_surface_boundary() {
        let valid = chrono::Utc::now();
        let frame = |descriptor, value| {
            FieldFrame::new(
                descriptor,
                grid(2, 2, -100.0, -99.0, 39.0, 40.0, value),
                DataStamp {
                    source_identity: "rtma#bytes=0-10".into(),
                    issue_time: None,
                    run_time: None,
                    valid_time: valid,
                    received_time: valid,
                    class: DataClass::Analysis,
                    quality: QualitySummary::Good,
                    available_members: None,
                },
            )
        };
        let t = frame(&wxdata::rtma::TEMP_DESCRIPTOR, 300.0);
        let td = frame(&wxdata::rtma::DEWPOINT_DESCRIPTOR, 290.0);
        let model_level = |pressure_hpa| wxdata::sounding::SoundingLevel {
            pressure_hpa,
            temp_c: 20.0,
            dewpt_c: 10.0,
            u_ms: 1.0,
            v_ms: 1.0,
        };
        let model = wxdata::sounding::Sounding {
            lon: -99.5,
            lat: 39.5,
            run: valid,
            fh: 0,
            levels: vec![
                model_level(1000.0),
                model_level(925.0),
                model_level(850.0),
                model_level(700.0),
            ],
        };
        let ob = wxdata::metar::SurfaceOb {
            icao: "KTEST".into(),
            name: "Test".into(),
            lat: 39.5,
            lon: -99.5,
            temp_c: Some(30.0),
            dewp_c: Some(20.0),
            wdir_deg: None,
            wspd_kt: 0.0,
            wgst_kt: None,
            altim_mb: None,
            elev_m: None,
            obs_time: Some(valid.timestamp()),
            flt_cat: String::new(),
            wvht_ft: None,
            dpd_s: None,
            raw: String::new(),
        };
        let point = objective_surface_point(&t, &td, std::slice::from_ref(&ob), &[], model.lon, model.lat).unwrap();
        let (adjusted, _) = objective_sounding_surface(&model, point.clone(), 95_000.0, 8.0, -2.0).unwrap();
        assert_eq!(adjusted.levels.len(), 4);
        assert_eq!(adjusted.levels[0].pressure_hpa, 950.0);
        assert!((adjusted.levels[0].temp_c - 30.0).abs() < 0.001);
        assert_eq!(adjusted.levels[0].u_ms, 8.0);
        assert_eq!(adjusted.levels[1].pressure_hpa, 925.0);
        assert_eq!(model.levels[0].pressure_hpa, 1000.0);
        assert_eq!(point.station, "KTEST");
        let mut partial = wxdata::stations::from_metars(&[wxdata::metar::SurfaceOb {
            icao: "PARTIAL".into(), name: "Partial".into(), lat: 39.5, lon: -99.5,
            temp_c: Some(31.0), dewp_c: None, wdir_deg: None, wspd_kt: 0.0,
            wgst_kt: None, altim_mb: None, elev_m: None, obs_time: Some(valid.timestamp()),
            flt_cat: String::new(), wvht_ft: None, dpd_s: None, raw: String::new(),
        }]);
        let mut complete = partial[0].clone();
        complete.id = "COMPLETE".into();
        complete.lon = -99.4;
        complete.dewp_c = Some(20.0);
        partial.push(complete);
        let chosen = nearest_surface_observation(valid, &[], &partial, model.lon, model.lat, true).unwrap();
        assert_eq!(chosen.id, "METAR:COMPLETE");
        let native = |point, station| wxdata::rtma::NativeSamplePair {
            source_identity: "rtma#bytes=0-10".into(), valid_time: valid,
            received_time: valid, point, station,
        };
        let adjusted_point = objective_surface_native(
            &native(300.0, 299.0), &native(290.0, 289.0), &chosen,
            wxdata::rtma::Source::Rtma,
        ).unwrap();
        assert_eq!(adjusted_point.analysis_source, "NOAA RTMA");
        assert!((adjusted_point.temperature_residual_k.unwrap() - 5.15).abs() < 0.001);
        let (native_profile, _) = objective_sounding_surface(&model, adjusted_point, 95_000.0, 8.0, -2.0).unwrap();
        assert_eq!(native_profile.levels[0].pressure_hpa, 950.0);
        assert_eq!(native_profile.levels[1].pressure_hpa, 925.0);
        let forecast = wxdata::sounding::Sounding { fh: 1, ..model };
        assert!(objective_sounding_surface(&forecast, point, 95_000.0, 8.0, -2.0).is_none());
    }
}
