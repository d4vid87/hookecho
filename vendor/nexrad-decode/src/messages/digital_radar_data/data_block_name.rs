/// The named identity of a data block.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum DataBlockName {
    /// Volume metadata block ("VOL").
    Volume,
    /// Elevation metadata block ("ELV").
    Elevation,
    /// Radial metadata block ("RAD").
    Radial,
    /// Reflectivity data moment ("REF").
    Reflectivity,
    /// Velocity data moment ("VEL").
    Velocity,
    /// Spectrum width data moment ("SW\0").
    SpectrumWidth,
    /// Differential reflectivity data moment ("ZDR").
    DifferentialReflectivity,
    /// Differential phase data moment ("PHI").
    DifferentialPhase,
    /// Correlation coefficient data moment ("RHO").
    CorrelationCoefficient,
    /// Clutter filter power data moment ("CFP").
    ClutterFilterPower,
    /// Unknown data block name for forward compatibility.
    Unknown([u8; 3]),
}
