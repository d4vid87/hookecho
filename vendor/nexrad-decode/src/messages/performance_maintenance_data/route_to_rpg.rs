/// Route to RPG status.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum RouteToRPG {
    /// Normal route.
    Normal,
    /// Backup route in use.
    BackupInUse,
    /// Route down due to failure.
    DownFailure,
    /// Backup route commanded down.
    BackupCommandedDown,
    /// Route not installed.
    NotInstalled,
    /// Unknown status value for forward compatibility.
    Unknown(u16),
}
