//! Deterministic moving-window extrema for radar, MRMS, and user-defined grids.

use crate::mrms::MrmsField;
use chrono::{DateTime, Duration, Utc};

pub static REFLECTIVITY_TRAIL_DESCRIPTOR: crate::field::FieldDescriptor =
    crate::field::FieldDescriptor {
        id: crate::field::FieldId("derived.trail.mrms-reflectivity"),
        source: "HookEcho derived from NOAA MRMS",
        family: crate::field::FieldFamily::ObservationDerived,
        display_name: "MRMS reflectivity trail",
        short_name: "Reflectivity Trail",
        search_aliases: &["storm trail", "reflectivity history", "maximum"],
        units: "dBZ",
        value_kind: crate::field::ValueKind::Scalar,
        palette_key: "reflectivity",
        sampling: crate::field::SamplingPolicy::Nearest,
        missing: crate::field::MissingData::Nan,
        time_policy: None,
        supports_contours: false,
        supports_difference: false,
    };

const MAX_FRAMES: usize = 256;
const MAX_CELLS: usize = 4_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Maximum,
    Minimum,
}

#[derive(Clone)]
pub struct TrailResult {
    pub field: MrmsField,
    /// Age in seconds of the frame contributing each cell. `NaN` follows a missing value.
    pub age_seconds: Vec<f32>,
    pub frames: usize,
}

impl TrailResult {
    /// Portable scientific grid with coordinates, extrema, contributor age, and valid time.
    pub fn to_csv(&self) -> String {
        let mut csv = String::from("lon,lat,value,age_seconds,valid_time\n");
        for y in 0..self.field.ny {
            let lat = self.field.lat_north
                - (y as f64 + 0.5) / self.field.ny as f64
                    * (self.field.lat_north - self.field.lat_south);
            for x in 0..self.field.nx {
                let at = y * self.field.nx + x;
                let value = self.field.values[at];
                if !value.is_finite() {
                    continue;
                }
                let lon = self.field.lon_west
                    + (x as f64 + 0.5) / self.field.nx as f64
                        * (self.field.lon_east - self.field.lon_west);
                csv.push_str(&format!(
                    "{lon:.6},{lat:.6},{value},{:.1},{}\n",
                    self.age_seconds[at],
                    self.field.time.to_rfc3339()
                ));
            }
        }
        csv
    }
}

pub struct ExtremaTrail {
    window: Duration,
    threshold: f32,
    mode: Mode,
    reset_at: Option<DateTime<Utc>>,
    frames: Vec<MrmsField>,
}

impl ExtremaTrail {
    pub fn new(window_minutes: u16, threshold: f32, mode: Mode) -> anyhow::Result<Self> {
        anyhow::ensure!(window_minutes > 0, "trail window must be positive");
        Ok(Self {
            window: Duration::minutes(window_minutes.into()),
            threshold,
            mode,
            reset_at: None,
            frames: Vec::new(),
        })
    }

    /// Start a new reproducible trail at an archive time.
    pub fn reset_at(&mut self, time: DateTime<Utc>) {
        self.reset_at = Some(time);
        self.frames.clear();
    }

    pub fn push(&mut self, frame: MrmsField) -> anyhow::Result<TrailResult> {
        anyhow::ensure!(frame.values.len() <= MAX_CELLS, "trail grid exceeds cell budget");
        if let Some(first) = self.frames.first() {
            anyhow::ensure!(same_grid(first, &frame), "trail frame grid changed");
        }
        if self.reset_at.is_some_and(|start| frame.time < start) {
            return self.result();
        }
        self.frames.retain(|saved| saved.time != frame.time);
        self.frames.push(frame);
        self.frames.sort_by_key(|saved| saved.time);
        anyhow::ensure!(self.frames.len() <= MAX_FRAMES, "trail exceeds frame budget");
        let newest = self.frames.last().expect("just pushed").time;
        let oldest = newest - self.window;
        self.frames.retain(|saved| saved.time >= oldest);
        self.result()
    }

    pub fn result(&self) -> anyhow::Result<TrailResult> {
        let newest = self.frames.last().ok_or_else(|| anyhow::anyhow!("trail has no frames"))?;
        let mut values = vec![f32::NAN; newest.values.len()];
        let mut age_seconds = vec![f32::NAN; newest.values.len()];
        for frame in &self.frames {
            let age = (newest.time - frame.time).num_milliseconds() as f32 / 1000.0;
            for (index, value) in frame.values.iter().copied().enumerate() {
                if !value.is_finite() || !passes(value, self.threshold, self.mode) {
                    continue;
                }
                let replace = !values[index].is_finite()
                    || match self.mode {
                        Mode::Maximum => value > values[index],
                        Mode::Minimum => value < values[index],
                    };
                if replace {
                    values[index] = value;
                    age_seconds[index] = age;
                }
            }
        }
        Ok(TrailResult {
            field: MrmsField { values, ..newest.clone() },
            age_seconds,
            frames: self.frames.len(),
        })
    }
}

fn passes(value: f32, threshold: f32, mode: Mode) -> bool {
    match mode {
        Mode::Maximum => value >= threshold,
        Mode::Minimum => value <= threshold,
    }
}

fn same_grid(a: &MrmsField, b: &MrmsField) -> bool {
    a.nx == b.nx
        && a.ny == b.ny
        && a.lon_west == b.lon_west
        && a.lon_east == b.lon_east
        && a.lat_north == b.lat_north
        && a.lat_south == b.lat_south
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn frame(minute: i64, values: &[f32]) -> MrmsField {
        MrmsField {
            values: values.to_vec(),
            nx: values.len(),
            ny: 1,
            lon_west: -100.0,
            lon_east: -99.0,
            lat_north: 40.0,
            lat_south: 39.0,
            time: Utc.with_ymd_and_hms(2026, 9, 20, 12, minute as u32, 0).unwrap(),
        }
    }

    #[test]
    fn maximum_is_order_independent_and_reports_contributor_age() {
        let mut trail = ExtremaTrail::new(30, 10.0, Mode::Maximum).unwrap();
        trail.push(frame(10, &[20.0, 15.0])).unwrap();
        let result = trail.push(frame(0, &[30.0, 5.0])).unwrap();
        assert_eq!(result.field.values, [30.0, 15.0]);
        assert_eq!(result.age_seconds, [600.0, 0.0]);
    }

    #[test]
    fn window_threshold_and_reset_are_reproducible() {
        let mut trail = ExtremaTrail::new(15, 0.5, Mode::Minimum).unwrap();
        trail.push(frame(0, &[0.1, 0.7])).unwrap();
        let result = trail.push(frame(20, &[0.4, 0.3])).unwrap();
        assert_eq!(result.frames, 1);
        assert_eq!(result.field.values, [0.4, 0.3]);
        trail.reset_at(frame(25, &[0.0]).time);
        assert!(trail.push(frame(20, &[0.2, 0.2])).is_err());
        assert_eq!(trail.push(frame(30, &[0.2, 0.2])).unwrap().frames, 1);
    }

    #[test]
    fn changed_grid_is_rejected() {
        let mut trail = ExtremaTrail::new(60, 0.0, Mode::Maximum).unwrap();
        trail.push(frame(0, &[1.0])).unwrap();
        assert!(trail.push(frame(1, &[1.0, 2.0])).is_err());
    }

    #[test]
    fn csv_preserves_coordinates_time_value_and_age() {
        let mut trail = ExtremaTrail::new(60, 0.0, Mode::Maximum).unwrap();
        let result = trail.push(frame(0, &[12.5])).unwrap();
        let csv = result.to_csv();
        assert!(csv.contains("lon,lat,value,age_seconds,valid_time"));
        assert!(csv.contains("12.5,0.0,2026-09-20T12:00:00+00:00"));
    }
}
