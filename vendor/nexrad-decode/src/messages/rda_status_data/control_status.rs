/// The possible RDA system control statuses.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum ControlStatus {
    /// Only local control is permitted.
    LocalControlOnly,
    /// Only remote control is permitted.
    RemoteControlOnly,
    /// Either local or remote control is permitted.
    EitherLocalOrRemoteControl,
    /// Unknown control status value for forward compatibility.
    Unknown(u16),
}
