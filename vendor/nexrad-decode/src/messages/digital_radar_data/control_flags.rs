/// Flags indicating special control features.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum ControlFlags {
    /// No special control features applied.
    None,
    /// Azimuthal radials have been recombined.
    RecombinedAzimuthalRadials,
    /// Range gates have been recombined.
    RecombinedRangeGates,
    /// Both radials and range gates recombined to legacy resolution.
    RecombinedRadialsAndRangeGatesToLegacyResolution,
    /// Unknown control flag value for forward compatibility.
    Unknown(u8),
}
