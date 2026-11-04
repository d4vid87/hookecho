/// PFN (Pulse Forming Network) switch position.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum PfnSwitchPosition {
    /// Short pulse mode.
    ShortPulse,
    /// Long pulse mode.
    LongPulse,
    /// Unknown position value for forward compatibility.
    Unknown(u16),
}
