/// The possible RDA system statuses.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum RDAStatus {
    /// System is starting up.
    StartUp,
    /// System is in standby mode.
    Standby,
    /// System is restarting.
    Restart,
    /// System is in normal operating mode.
    Operate,
    /// Spare status (reserved).
    Spare,
    /// Unknown status value for forward compatibility.
    Unknown(u16),
}
