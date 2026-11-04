use crate::data::Radial;
use crate::result::{Error, Result};
use std::fmt::Display;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[cfg(feature = "chrono")]
use chrono::{DateTime, Utc};

#[cfg(feature = "uom")]
use uom::si::f32::Angle;

/// A single radar sweep composed of a series of radials. This represents a full rotation of the
/// radar at some elevation angle and contains the Level II data (reflectivity, velocity, and
/// spectrum width) for each azimuth angle in that sweep. The resolution of the sweep dictates the
/// azimuthal distance between rays and thus and number of rays in the sweep. Multiple sweeps are
/// taken at different elevation angles to create a complete scan.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Sweep {
    elevation_number: u8,
    radials: Vec<Radial>,
}

impl Sweep {
    /// Create a new radar sweep with the given elevation number and radials.
    pub fn new(elevation_number: u8, radials: Vec<Radial>) -> Self {
        Self {
            elevation_number,
            radials,
        }
    }

    /// Create a new radar sweep from a list of radials by splitting them by elevation.
    pub fn from_radials(radials: Vec<Radial>) -> Vec<Self> {
        let mut sweeps = Vec::new();

        let mut sweep_elevation_number = None;
        let mut sweep_radials = Vec::new();

        for radial in radials {
            if let Some(elevation_number) = sweep_elevation_number {
                if elevation_number != radial.elevation_number() {
                    sweeps.push(Sweep::new(elevation_number, sweep_radials));
                    sweep_radials = Vec::new();
                }
            }

            sweep_elevation_number = Some(radial.elevation_number());
            sweep_radials.push(radial);
        }

        // Push the final sweep if there are remaining radials
        if let Some(elevation_number) = sweep_elevation_number {
            if !sweep_radials.is_empty() {
                sweeps.push(Sweep::new(elevation_number, sweep_radials));
            }
        }

        sweeps
    }

    /// The index number for this radial's elevation in the scan. The precise elevation angle
    /// varies and can be found in individual radials.
    pub fn elevation_number(&self) -> u8 {
        self.elevation_number
    }

    /// The elevation angle in degrees for this sweep, derived from the median of all radial
    /// elevation angles.
    ///
    /// The median is used rather than the first radial because radials at the beginning and end
    /// of a sweep may report transitional elevation angles as the antenna moves between sweeps.
    /// The median is robust against these outliers.
    ///
    /// Returns `None` if the sweep has no radials.
    pub fn elevation_angle_degrees(&self) -> Option<f32> {
        if self.radials.is_empty() {
            return None;
        }

        let mut angles: Vec<f32> = self
            .radials
            .iter()
            .map(|r| r.elevation_angle_degrees())
            .collect();
        angles.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let mid = angles.len() / 2;
        if angles.len() % 2 == 0 {
            Some((angles[mid - 1] + angles[mid]) / 2.0)
        } else {
            Some(angles[mid])
        }
    }

    /// The elevation angle for this sweep.
    ///
    /// Uses the median of all radial elevation angles for robustness against transitional
    /// radials at sweep boundaries. See [`elevation_angle_degrees`](Self::elevation_angle_degrees)
    /// for details.
    ///
    /// Returns `None` if the sweep has no radials.
    #[cfg(feature = "uom")]
    pub fn elevation_angle(&self) -> Option<Angle> {
        self.elevation_angle_degrees()
            .map(Angle::new::<uom::si::angle::degree>)
    }

    /// The radials comprising this sweep.
    pub fn radials(&self) -> &[Radial] {
        &self.radials
    }

    /// The time range of this sweep as `(earliest, latest)` collection times.
    ///
    /// Returns `None` if the sweep has no radials or no valid timestamps.
    #[cfg(feature = "chrono")]
    pub fn time_range(&self) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
        let mut earliest: Option<DateTime<Utc>> = None;
        let mut latest: Option<DateTime<Utc>> = None;

        for radial in &self.radials {
            if let Some(time) = radial.collection_time() {
                earliest = Some(match earliest {
                    Some(e) => e.min(time),
                    None => time,
                });
                latest = Some(match latest {
                    Some(l) => l.max(time),
                    None => time,
                });
            }
        }

        earliest.zip(latest)
    }

    /// Merges this sweep with another sweep, combining their radials into a single sweep. The
    /// sweeps must be at the same elevation, and they should not have duplicate azimuth radials.
    pub fn merge(self, other: Self) -> Result<Self> {
        if self.elevation_number != other.elevation_number {
            return Err(Error::ElevationMismatch);
        }

        let mut radials = self.radials;
        radials.extend(other.radials);
        radials.sort_by_key(|radial| radial.azimuth_number());

        Ok(Self {
            elevation_number: self.elevation_number,
            radials,
        })
    }
}

impl Display for Sweep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let (Some(first), Some(last)) = (self.radials.first(), self.radials.last()) {
            write!(
                f,
                "Sweep ({:.1}-{:.1} deg, {} radials, {} deg spacing)",
                first.azimuth_angle_degrees(),
                last.azimuth_angle_degrees(),
                self.radials.len(),
                first.azimuth_spacing_degrees()
            )
        } else {
            write!(f, "Sweep (no radials)")
        }
    }
}
