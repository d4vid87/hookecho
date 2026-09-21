//! Common metadata and sampling for gridded weather fields.

use chrono::{DateTime, Utc};
use std::sync::Arc;

/// Stable product identity. IDs are persisted; display names are not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FieldId(pub &'static str);

/// Stable provider identity for diagnostics and cache namespaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DataSourceId(pub &'static str);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldFamily {
    Radar,
    Mrms,
    Satellite,
    Model,
    Analysis,
    ObservationDerived,
    UserDefined,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    Scalar,
    Categorical,
    Vector,
    Probability,
    Accumulation,
    Mask,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SamplingPolicy {
    Nearest,
    Bilinear,
}

impl SamplingPolicy {
    pub fn label(self) -> &'static str {
        match self {
            Self::Nearest => "Nearest neighbor",
            Self::Bilinear => "Bilinear",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MissingData {
    Nan,
    Value(f32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitSystem {
    Metric,
    Us,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DisplayValue {
    pub value: f32,
    pub units: &'static str,
}

/// Static product behavior used by acquisition, UI, legends, and analysis.
#[derive(Debug)]
pub struct FieldDescriptor {
    pub id: FieldId,
    pub source: &'static str,
    pub family: FieldFamily,
    pub display_name: &'static str,
    pub short_name: &'static str,
    pub search_aliases: &'static [&'static str],
    pub units: &'static str,
    pub value_kind: ValueKind,
    pub palette_key: &'static str,
    pub sampling: SamplingPolicy,
    pub missing: MissingData,
    /// Override the default policy implied by [`DataClass`] when a product has stricter semantics.
    pub time_policy: Option<crate::timecoord::TimePolicy>,
    pub supports_contours: bool,
    pub supports_difference: bool,
}

/// Source and schedule metadata shared by model adapters and diagnostics.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelDefinition {
    pub id: &'static str,
    pub label: &'static str,
    pub provider: &'static str,
    pub base_url: &'static str,
    pub cycle_hours: u32,
    pub max_forecast_hour: u16,
    pub grid: &'static str,
    pub regrid_resolution_deg: f64,
    pub index_suffix: &'static str,
    pub expected_latency_minutes: Option<u16>,
    pub domain: &'static str,
    pub ensemble: bool,
}

impl FieldDescriptor {
    pub fn source_id(&self) -> DataSourceId {
        DataSourceId(match self.source {
            "NOAA MRMS" => "noaa.mrms",
            "NOAA MRMS derived" => "noaa.mrms.derived",
            "NOAA GOES ABI" => "noaa.goes.abi",
            "NOAA GOES GLM" => "noaa.goes.glm",
            "NOAA GFS / ECMWF IFS" => "model.global",
            "NOAA HRRR" => "noaa.hrrr",
            "NOAA NBM" => "noaa.nbm",
            "NOAA regional models" => "noaa.regional-models",
            "NOAA REFS v1 parallel" => "noaa.refs-v1",
            "NOAA RTMA" => "noaa.rtma",
            source => source,
        })
    }

    pub fn time_policy(&self, class: DataClass) -> crate::timecoord::TimePolicy {
        self.time_policy
            .unwrap_or_else(|| crate::timecoord::policy_for(class))
    }

    /// Convert a native scientific value for display. Sampling and exports remain native.
    pub fn display_value(&self, value: f32, system: UnitSystem) -> DisplayValue {
        match (self.units, system) {
            ("K", UnitSystem::Metric) => DisplayValue {
                value: value - 273.15,
                units: "°C",
            },
            ("K", UnitSystem::Us) => DisplayValue {
                value: (value - 273.15) * 1.8 + 32.0,
                units: "°F",
            },
            ("Pa", UnitSystem::Metric) => DisplayValue {
                value: value / 100.0,
                units: "hPa",
            },
            ("Pa", UnitSystem::Us) => DisplayValue {
                value: value / 3386.389,
                units: "inHg",
            },
            ("m s-1", UnitSystem::Metric) => DisplayValue {
                value: value * 3.6,
                units: "km/h",
            },
            ("m s-1", UnitSystem::Us) => DisplayValue {
                value: value * 2.236_936_3,
                units: "mph",
            },
            ("kg m-2", UnitSystem::Metric) => DisplayValue { value, units: "mm" },
            ("kg m-2", UnitSystem::Us) => DisplayValue {
                value: value / 25.4,
                units: "in",
            },
            _ => DisplayValue {
                value,
                units: self.units,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GridSpec {
    pub nx: usize,
    pub ny: usize,
    pub projection: &'static str,
    pub lon_west: f64,
    pub lon_east: f64,
    pub lat_north: f64,
    pub lat_south: f64,
    pub native_resolution_m: Option<f64>,
    pub missing: MissingData,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataClass {
    Observed,
    Analysis,
    Forecast,
    Derived,
}

impl DataClass {
    pub fn label(self) -> &'static str {
        match self {
            Self::Observed => "Observed",
            Self::Analysis => "Analysis",
            Self::Forecast => "Forecast",
            Self::Derived => "Derived",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualitySummary {
    Unknown,
    Good,
    Suspect,
}

impl QualitySummary {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "Unknown",
            Self::Good => "Good",
            Self::Suspect => "Suspect",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataStamp {
    pub source_identity: String,
    pub issue_time: Option<DateTime<Utc>>,
    pub run_time: Option<DateTime<Utc>>,
    pub valid_time: DateTime<Utc>,
    pub received_time: DateTime<Utc>,
    pub class: DataClass,
    pub quality: QualitySummary,
    /// Members contributing to an ensemble product, when the source reports it.
    pub available_members: Option<u16>,
}

/// Immutable decoded values plus the metadata needed to identify and sample them.
#[derive(Clone)]
pub struct FieldFrame {
    pub descriptor: &'static FieldDescriptor,
    pub grid: GridSpec,
    pub stamp: DataStamp,
    field: Arc<crate::mrms::MrmsField>,
    native_abi: Option<Arc<crate::abi::Image>>,
}

impl FieldFrame {
    pub fn new(
        descriptor: &'static FieldDescriptor,
        field: crate::mrms::MrmsField,
        stamp: DataStamp,
    ) -> Self {
        let grid = GridSpec {
            nx: field.nx,
            ny: field.ny,
            projection: "EPSG:4326",
            lon_west: field.lon_west,
            lon_east: field.lon_east,
            lat_north: field.lat_north,
            lat_south: field.lat_south,
            // MRMS files do not carry one reliable scalar metre resolution for the whole
            // latitude/longitude grid. Keep it unknown instead of inventing precision.
            native_resolution_m: None,
            missing: descriptor.missing,
        };
        Self {
            descriptor,
            grid,
            stamp,
            field: Arc::new(field),
            native_abi: None,
        }
    }

    pub fn from_abi(
        descriptor: &'static FieldDescriptor,
        native: crate::abi::Image,
        display: crate::mrms::MrmsField,
        stamp: DataStamp,
    ) -> Self {
        let (lon_west, lon_east, lat_north, lat_south) = (
            display.lon_west,
            display.lon_east,
            display.lat_north,
            display.lat_south,
        );
        let grid = GridSpec {
            nx: native.width,
            ny: native.height,
            projection: "GOES-R fixed grid",
            lon_west,
            lon_east,
            lat_north,
            lat_south,
            native_resolution_m: None,
            missing: descriptor.missing,
        };
        Self {
            descriptor,
            grid,
            stamp,
            field: Arc::new(display),
            native_abi: Some(Arc::new(native)),
        }
    }

    pub fn field(&self) -> &crate::mrms::MrmsField {
        &self.field
    }

    pub fn native_abi(&self) -> Option<&crate::abi::Image> {
        self.native_abi.as_deref()
    }

    pub fn sample(&self, lon: f64, lat: f64) -> SampleResult {
        let value = self.native_abi.as_ref().map_or_else(
            || match self.descriptor.sampling {
                SamplingPolicy::Bilinear => self.field.sample_bilinear(lon, lat),
                SamplingPolicy::Nearest => self.field.sample_nearest(lon, lat),
            },
            |image| image.sample_nearest(lon, lat),
        );
        SampleResult {
            value,
            units: self.descriptor.units,
            quality: self.stamp.quality,
            valid_time: self.stamp.valid_time,
            method: self.descriptor.sampling,
        }
    }

    /// Exact native-value statistics inside a lon/lat box. Display textures are never sampled.
    pub fn statistics_in_box(&self, a: [f64; 2], b: [f64; 2]) -> Option<RegionStats> {
        let bounds = (
            a[0].min(b[0]),
            a[1].min(b[1]),
            a[0].max(b[0]),
            a[1].max(b[1]),
        );
        let mut count = 0usize;
        let mut mean = 0.0f64;
        let mut m2 = 0.0f64;
        let mut min = f32::INFINITY;
        let mut max = f32::NEG_INFINITY;
        self.for_each_in_box(bounds, |_, _, value| {
            count += 1;
            min = min.min(value);
            max = max.max(value);
            let delta = f64::from(value) - mean;
            mean += delta / count as f64;
            m2 += delta * (f64::from(value) - mean);
        });
        if count == 0 {
            return None;
        }
        let mut histogram = [0u32; 16];
        let span = (max - min).max(f32::EPSILON);
        self.for_each_in_box(bounds, |_, _, value| {
            let bin = (((value - min) / span * 16.0) as usize).min(15);
            histogram[bin] = histogram[bin].saturating_add(1);
        });
        Some(RegionStats {
            count,
            min,
            max,
            mean: mean as f32,
            std_dev: (m2 / count as f64).sqrt() as f32,
            histogram,
        })
    }

    /// Pair this field's native cells with another field's declared point sampler.
    /// Exact valid-time matching prevents correlations across different weather states.
    pub fn correlation_in_box(
        &self,
        other: &Self,
        a: [f64; 2],
        b: [f64; 2],
    ) -> Option<CorrelationStats> {
        if self.stamp.valid_time != other.stamp.valid_time {
            return None;
        }
        let bounds = (
            a[0].min(b[0]),
            a[1].min(b[1]),
            a[0].max(b[0]),
            a[1].max(b[1]),
        );
        let mut count = 0usize;
        let (mut mean_x, mut mean_y, mut m2_x, mut m2_y, mut covariance) =
            (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
        self.for_each_in_box(bounds, |lon, lat, x| {
            let Some(y) = other.sample(lon, lat).value else {
                return;
            };
            count += 1;
            let dx = f64::from(x) - mean_x;
            mean_x += dx / count as f64;
            let dy = f64::from(y) - mean_y;
            mean_y += dy / count as f64;
            m2_x += dx * (f64::from(x) - mean_x);
            m2_y += dy * (f64::from(y) - mean_y);
            covariance += dx * (f64::from(y) - mean_y);
        });
        if count < 2 || m2_x <= 0.0 || m2_y <= 0.0 {
            return None;
        }
        let slope = covariance / m2_x;
        Some(CorrelationStats {
            count,
            pearson_r: (covariance / (m2_x * m2_y).sqrt()) as f32,
            slope: slope as f32,
            intercept: (mean_y - slope * mean_x) as f32,
        })
    }

    fn for_each_in_box(&self, bounds: (f64, f64, f64, f64), mut visit: impl FnMut(f64, f64, f32)) {
        let (west, south, east, north) = bounds;
        if let Some(image) = &self.native_abi {
            for (row, &y) in image.y.iter().enumerate() {
                for (col, &x) in image.x.iter().enumerate() {
                    let index = row * image.width + col;
                    let value = image.values[index];
                    if image.quality.get(index).copied().unwrap_or(3) >= 2 || !value.is_finite() {
                        continue;
                    }
                    if let Some((lon, lat)) = image.projection.lon_lat(x, y) {
                        if (west..=east).contains(&lon) && (south..=north).contains(&lat) {
                            visit(lon, lat, value);
                        }
                    }
                }
            }
            return;
        }
        let field = &self.field;
        let dlon = (field.lon_east - field.lon_west) / field.nx.max(1) as f64;
        let dlat = (field.lat_north - field.lat_south) / field.ny.max(1) as f64;
        for row in 0..field.ny {
            let lat = field.lat_north - (row as f64 + 0.5) * dlat;
            if !(south..=north).contains(&lat) {
                continue;
            }
            for col in 0..field.nx {
                let lon = field.lon_west + (col as f64 + 0.5) * dlon;
                let value = field.values[row * field.nx + col];
                if (west..=east).contains(&lon) && value.is_finite() {
                    visit(lon, lat, value);
                }
            }
        }
    }

    /// Stream native scalar values with enough metadata to reproduce their meaning.
    pub fn write_csv(&self, mut output: impl std::io::Write) -> std::io::Result<usize> {
        let metadata = serde_json::json!({
            "schema": "hookecho.field/v1",
            "product_id": self.descriptor.id.0,
            "source_id": self.descriptor.source_id().0,
            "source_identity": self.stamp.source_identity,
            "units": self.descriptor.units,
            "valid_time": self.stamp.valid_time.to_rfc3339(),
            "received_time": self.stamp.received_time.to_rfc3339(),
            "quality": self.stamp.quality.label(),
            "sampling": self.descriptor.sampling.label(),
            "projection": self.grid.projection,
        });
        writeln!(output, "# {metadata}")?;
        writeln!(output, "lon,lat,value")?;
        let mut rows = 0;
        if let Some(image) = &self.native_abi {
            for (row, &y) in image.y.iter().enumerate() {
                for (col, &x) in image.x.iter().enumerate() {
                    let index = row * image.width + col;
                    let value = image.values[index];
                    if image.quality.get(index).copied().unwrap_or(3) >= 2 || !value.is_finite() {
                        continue;
                    }
                    let Some((lon, lat)) = image.projection.lon_lat(x, y) else {
                        continue;
                    };
                    writeln!(output, "{lon:.6},{lat:.6},{value}")?;
                    rows += 1;
                }
            }
        } else {
            let field = &self.field;
            let dlon = (field.lon_east - field.lon_west) / field.nx.max(1) as f64;
            let dlat = (field.lat_north - field.lat_south) / field.ny.max(1) as f64;
            for row in 0..field.ny {
                let lat = field.lat_north - (row as f64 + 0.5) * dlat;
                for col in 0..field.nx {
                    let value = field.values[row * field.nx + col];
                    if !value.is_finite() {
                        continue;
                    }
                    let lon = field.lon_west + (col as f64 + 0.5) * dlon;
                    writeln!(output, "{lon:.6},{lat:.6},{value}")?;
                    rows += 1;
                }
            }
        }
        Ok(rows)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SampleResult {
    pub value: Option<f32>,
    pub units: &'static str,
    pub quality: QualitySummary,
    pub valid_time: DateTime<Utc>,
    pub method: SamplingPolicy,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RegionStats {
    pub count: usize,
    pub min: f32,
    pub max: f32,
    pub mean: f32,
    pub std_dev: f32,
    pub histogram: [u32; 16],
}

#[derive(Debug, Clone, PartialEq)]
pub struct CorrelationStats {
    pub count: usize,
    pub pearson_r: f32,
    pub slope: f32,
    pub intercept: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    static TEST: FieldDescriptor = FieldDescriptor {
        id: FieldId("test.scalar"),
        source: "test",
        family: FieldFamily::Analysis,
        display_name: "Test",
        short_name: "TEST",
        search_aliases: &[],
        units: "unit",
        value_kind: ValueKind::Scalar,
        palette_key: "test",
        sampling: SamplingPolicy::Nearest,
        missing: MissingData::Nan,
        time_policy: None,
        supports_contours: false,
        supports_difference: false,
    };

    #[test]
    fn frame_samples_native_values_with_provenance() {
        let valid = Utc::now();
        let frame = FieldFrame::new(
            &TEST,
            crate::mrms::MrmsField {
                values: vec![1.0, 2.0, 3.0, 4.0],
                nx: 2,
                ny: 2,
                lon_west: -100.0,
                lon_east: -98.0,
                lat_north: 40.0,
                lat_south: 38.0,
                time: valid,
            },
            DataStamp {
                source_identity: "fixture".into(),
                issue_time: None,
                run_time: None,
                valid_time: valid,
                received_time: valid,
                class: DataClass::Analysis,
                quality: QualitySummary::Good,
                available_members: None,
            },
        );
        let sample = frame.sample(-99.5, 39.5);
        assert_eq!(sample.value, Some(1.0));
        assert_eq!(sample.units, "unit");
        assert_eq!(sample.valid_time, valid);
        let mut csv = Vec::new();
        assert_eq!(frame.write_csv(&mut csv).unwrap(), 4);
        let csv = String::from_utf8(csv).unwrap();
        assert!(csv.contains("\"product_id\":\"test.scalar\""));
        assert!(csv.contains("\"source_id\":\"test\""));
        assert!(csv.contains("\"source_identity\":\"fixture\""));
        assert!(csv.contains("-99.500000,39.500000,1"));

        let stats = frame
            .statistics_in_box([-100.0, 38.0], [-99.0, 40.0])
            .unwrap();
        assert_eq!(stats.count, 2);
        assert_eq!((stats.min, stats.max, stats.mean), (1.0, 3.0, 2.0));
        assert!((stats.std_dev - 1.0).abs() < 1e-6);
        assert_eq!(stats.histogram.iter().sum::<u32>(), 2);

        let mut doubled = frame.field().clone();
        doubled.values.iter_mut().for_each(|value| *value *= 2.0);
        let paired = FieldFrame::new(&TEST, doubled, frame.stamp.clone());
        let correlation = frame
            .correlation_in_box(&paired, [-100.0, 38.0], [-98.0, 40.0])
            .unwrap();
        assert_eq!(correlation.count, 4);
        assert!((correlation.pearson_r - 1.0).abs() < 1e-6);
        assert!((correlation.slope - 2.0).abs() < 1e-6);
        assert!(correlation.intercept.abs() < 1e-6);
        assert_eq!(crate::mrms::REFLECTIVITY_DESCRIPTOR.source_id().0, "noaa.mrms");
        let c = crate::global::TEMP_2M_DESCRIPTOR.display_value(273.15, UnitSystem::Metric);
        assert_eq!((c.value, c.units), (0.0, "°C"));
        let f = crate::global::TEMP_2M_DESCRIPTOR.display_value(273.15, UnitSystem::Us);
        assert!((f.value - 32.0).abs() < 1e-5);
        assert_eq!(f.units, "°F");
        let pressure = crate::global::MSLP_DESCRIPTOR.display_value(101_325.0, UnitSystem::Us);
        assert!((pressure.value - 29.92).abs() < 0.01);
        assert_eq!(pressure.units, "inHg");
    }
}
