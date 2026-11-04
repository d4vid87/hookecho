use crate::data::{Sweep, VCPNumber, VolumeCoveragePattern};
use crate::meta::Site;
use std::fmt::Display;

#[cfg(feature = "chrono")]
use chrono::{DateTime, Utc};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// A single radar scan composed of a series of sweeps at different elevations. The pattern of
/// sweeps, including elevations and resolution, is determined by the scanning strategy of the
/// radar. This is referred to as the Volume Coverage Pattern (VCP).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Scan {
    site: Option<Site>,
    coverage_pattern: VolumeCoveragePattern,
    sweeps: Vec<Sweep>,
}

impl Scan {
    /// Create a new radar scan with the given coverage pattern and sweeps.
    pub fn new(coverage_pattern: VolumeCoveragePattern, sweeps: Vec<Sweep>) -> Self {
        Self {
            site: None,
            coverage_pattern,
            sweeps,
        }
    }

    /// Create a new radar scan with site metadata, coverage pattern, and sweeps.
    pub fn with_site(
        site: Site,
        coverage_pattern: VolumeCoveragePattern,
        sweeps: Vec<Sweep>,
    ) -> Self {
        Self {
            site: Some(site),
            coverage_pattern,
            sweeps,
        }
    }

    /// The radar site that produced this scan, if available.
    pub fn site(&self) -> Option<&Site> {
        self.site.as_ref()
    }

    /// This scan's coverage pattern number.
    pub fn coverage_pattern_number(&self) -> VCPNumber {
        self.coverage_pattern.pattern_number()
    }

    /// The coverage pattern configuration for this scan.
    ///
    /// This contains detailed configuration about how the radar performs the scan, including
    /// per-elevation settings, SAILS/MRLE/MPDA capabilities, and various thresholds.
    pub fn coverage_pattern(&self) -> &VolumeCoveragePattern {
        &self.coverage_pattern
    }

    /// The elevation sweeps comprising this scan.
    pub fn sweeps(&self) -> &[Sweep] {
        &self.sweeps
    }

    /// All sweeps at the given elevation angle in degrees, in scan order.
    ///
    /// Uses a 0.05-degree tolerance for matching. SAILS and MRLE VCPs repeat certain
    /// elevation angles at multiple points during a scan, so this may yield more than
    /// one sweep.
    pub fn sweeps_at_elevation(&self, elevation_degrees: f32) -> impl Iterator<Item = &Sweep> {
        self.sweeps.iter().filter(move |s| {
            s.elevation_angle_degrees()
                .map(|a| (a - elevation_degrees).abs() < 0.05)
                .unwrap_or(false)
        })
    }

    /// The time range of this scan as `(earliest, latest)` collection times across all sweeps.
    ///
    /// Returns `None` if the scan has no radials or no valid timestamps.
    #[cfg(feature = "chrono")]
    pub fn time_range(&self) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
        let mut earliest: Option<DateTime<Utc>> = None;
        let mut latest: Option<DateTime<Utc>> = None;

        for sweep in &self.sweeps {
            if let Some((sweep_start, sweep_end)) = sweep.time_range() {
                earliest = Some(match earliest {
                    Some(e) => e.min(sweep_start),
                    None => sweep_start,
                });
                latest = Some(match latest {
                    Some(l) => l.max(sweep_end),
                    None => sweep_end,
                });
            }
        }

        earliest.zip(latest)
    }
}

impl Display for Scan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Scan ({}, {} sweeps)",
            self.coverage_pattern.pattern_number(),
            self.sweeps.len()
        )
    }
}
