/// Transmitter maintenance mode status.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum MaintenanceMode {
    /// Not in maintenance mode.
    No,
    /// In maintenance mode.
    Yes,
    /// Unknown status value for forward compatibility.
    Unknown(u16),
}
