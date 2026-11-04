/// Volume coverage pattern (VCP) definitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VolumeCoveragePattern {
    /// VCP 12 - Precipitation mode with 14 elevations.
    VCP12,
    /// VCP 31 - Clear air mode with long dwell times.
    VCP31,
    /// VCP 35 - Clear air mode variant.
    VCP35,
    /// VCP 112 - Precipitation mode, enhanced low-level coverage.
    VCP112,
    /// VCP 212 - Precipitation mode with AVSET.
    VCP212,
    /// VCP 215 - Precipitation mode, similar to VCP 212.
    VCP215,
    /// Unknown VCP number for forward compatibility with new scan strategies.
    Unknown(u16),
}
