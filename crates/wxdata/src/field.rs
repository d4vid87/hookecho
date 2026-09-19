//! Common metadata and sampling for gridded weather fields.

use chrono::{DateTime, Utc};
use std::sync::Arc;

/// Stable product identity. IDs are persisted; display names are not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FieldId(pub &'static str);

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

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MissingData {
    Nan,
    Value(f32),
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
    pub supports_contours: bool,
    pub supports_difference: bool,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualitySummary {
    Unknown,
    Good,
    Suspect,
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
}

/// Immutable decoded values plus the metadata needed to identify and sample them.
#[derive(Clone)]
pub struct FieldFrame {
    pub descriptor: &'static FieldDescriptor,
    pub grid: GridSpec,
    pub stamp: DataStamp,
    field: Arc<crate::mrms::MrmsField>,
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
        }
    }

    pub fn field(&self) -> &crate::mrms::MrmsField {
        &self.field
    }

    pub fn sample(&self, lon: f64, lat: f64) -> SampleResult {
        let value = match self.descriptor.sampling {
            SamplingPolicy::Bilinear => self.field.sample_bilinear(lon, lat),
            SamplingPolicy::Nearest => self.field.sample_nearest(lon, lat),
        };
        SampleResult {
            value,
            units: self.descriptor.units,
            quality: self.stamp.quality,
            valid_time: self.stamp.valid_time,
            method: self.descriptor.sampling,
        }
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
            },
        );
        let sample = frame.sample(-99.5, 39.5);
        assert_eq!(sample.value, Some(1.0));
        assert_eq!(sample.units, "unit");
        assert_eq!(sample.valid_time, valid);
    }
}
