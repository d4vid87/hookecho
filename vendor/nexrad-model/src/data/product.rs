use crate::data::{CFPMomentData, MomentData, Radial};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Identifies a radar data product (moment type).
///
/// Each product corresponds to a different type of moment data captured by the radar.
/// This enum is shared across the entire crate ecosystem — it identifies which moment
/// field to extract from a radial, which processing algorithm to apply, or which color
/// scale to use for rendering.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Product {
    /// Base reflectivity (dBZ). Measures the intensity of precipitation.
    Reflectivity,
    /// Radial velocity (m/s). Measures motion toward or away from the radar.
    Velocity,
    /// Spectrum width (m/s). Measures turbulence within the radar beam.
    SpectrumWidth,
    /// Differential reflectivity (dB). Compares horizontal and vertical reflectivity.
    DifferentialReflectivity,
    /// Differential phase (degrees). Phase difference between polarizations.
    DifferentialPhase,
    /// Correlation coefficient. Correlation between polarizations (0-1).
    CorrelationCoefficient,
    /// Clutter filter power (CFP). Difference between clutter-filtered and unfiltered reflectivity.
    ClutterFilterPower,
}

impl Product {
    /// Returns the moment data for this product from a radial, if present.
    ///
    /// For all products except [`Product::ClutterFilterPower`], this returns the standard
    /// [`MomentData`]. For CFP, use [`cfp_moment_data`](Self::cfp_moment_data) instead.
    pub fn moment_data<'a>(&self, radial: &'a Radial) -> Option<&'a MomentData> {
        match self {
            Product::Reflectivity => radial.reflectivity(),
            Product::Velocity => radial.velocity(),
            Product::SpectrumWidth => radial.spectrum_width(),
            Product::DifferentialReflectivity => radial.differential_reflectivity(),
            Product::DifferentialPhase => radial.differential_phase(),
            Product::CorrelationCoefficient => radial.correlation_coefficient(),
            Product::ClutterFilterPower => None,
        }
    }

    /// Returns the CFP moment data from a radial, if this product is
    /// [`Product::ClutterFilterPower`] and the data is present.
    pub fn cfp_moment_data<'a>(&self, radial: &'a Radial) -> Option<&'a CFPMomentData> {
        match self {
            Product::ClutterFilterPower => radial.clutter_filter_power(),
            _ => None,
        }
    }

    /// Returns the typical value range `(min, max)` for this product.
    ///
    /// These ranges cover the expected data values for each product type and are
    /// used for color mapping and normalization.
    pub fn value_range(&self) -> (f32, f32) {
        match self {
            Product::Reflectivity => (-32.0, 95.0),
            Product::Velocity => (-64.0, 64.0),
            Product::SpectrumWidth => (0.0, 30.0),
            Product::DifferentialReflectivity => (-2.0, 6.0),
            Product::DifferentialPhase => (0.0, 360.0),
            Product::CorrelationCoefficient => (0.0, 1.0),
            Product::ClutterFilterPower => (-20.0, 20.0),
        }
    }

    /// Returns the standard unit string for this product.
    pub fn unit(&self) -> &'static str {
        match self {
            Product::Reflectivity => "dBZ",
            Product::Velocity => "m/s",
            Product::SpectrumWidth => "m/s",
            Product::DifferentialReflectivity => "dB",
            Product::DifferentialPhase => "°",
            Product::CorrelationCoefficient => "",
            Product::ClutterFilterPower => "dB",
        }
    }

    /// Returns a human-readable label for this product.
    pub fn label(&self) -> &'static str {
        match self {
            Product::Reflectivity => "Reflectivity",
            Product::Velocity => "Velocity",
            Product::SpectrumWidth => "Spectrum Width",
            Product::DifferentialReflectivity => "Differential Reflectivity",
            Product::DifferentialPhase => "Differential Phase",
            Product::CorrelationCoefficient => "Correlation Coefficient",
            Product::ClutterFilterPower => "Clutter Filter Power",
        }
    }
}
