/// The possible RDA system transition power source statuses.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum TransitionPowerSourceStatus {
    /// Transition power source is not installed.
    NotInstalled,
    /// Transition power source is off.
    Off,
    /// Transition power source is OK.
    OK,
    /// Transition power source status is unknown.
    Unknown,
    /// Unrecognized TPS status value for forward compatibility.
    Other(u16),
}
