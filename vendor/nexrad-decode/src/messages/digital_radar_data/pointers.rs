use crate::messages::primitive_aliases::Integer4;

/// A pointer to a data moment within a digital radar data message.
pub struct DataMomentPointer {
    /// The type of data moment that the pointer references.
    pub data_moment_type: DataMomentPointerType,

    /// The pointer to the data moment as a byte offset from the start of the message.
    pub pointer: Integer4,
}

/// The type of data moment that the pointer references.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DataMomentPointerType {
    /// Volume data block containing scan-level metadata.
    Volume,
    /// Elevation data block containing elevation-specific metadata.
    Elevation,
    /// Radial data block containing radial-specific metadata.
    Radial,
    /// Generic data block containing moment-specific data.
    Generic(DataMomentGenericPointerType),
}

/// The type of generic data moment that the pointer references.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DataMomentGenericPointerType {
    /// Reflectivity (dBZ) moment data.
    Reflectivity,
    /// Radial velocity (m/s) moment data.
    Velocity,
    /// Spectrum width (m/s) moment data.
    SpectrumWidth,
    /// Differential reflectivity (dB) moment data.
    DifferentialReflectivity,
    /// Differential phase (degrees) moment data.
    DifferentialPhase,
    /// Correlation coefficient (unitless) moment data.
    CorrelationCoefficient,
    /// Clutter filter power (CFP) moment data.
    ClutterFilterPower,
}
