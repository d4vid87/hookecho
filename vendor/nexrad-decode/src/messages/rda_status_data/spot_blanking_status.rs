/// The RDA system's spot blanking status.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum SpotBlankingStatus {
    /// Spot blanking hardware is not installed.
    NotInstalled,
    /// Spot blanking is enabled.
    Enabled,
    /// Spot blanking is disabled.
    Disabled,
    /// Unknown spot blanking status value for forward compatibility.
    Other(u8),
}
