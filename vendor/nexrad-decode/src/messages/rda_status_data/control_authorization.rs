/// The possible RDA system control authorizations.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum ControlAuthorization {
    /// No control action requested.
    NoAction,
    /// Local control has been requested.
    LocalControlRequested,
    /// Remote control has been requested.
    RemoteControlRequested,
    /// Unknown control authorization value for forward compatibility.
    Unknown(u16),
}
