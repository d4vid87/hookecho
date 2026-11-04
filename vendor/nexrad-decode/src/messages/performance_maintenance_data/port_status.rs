/// Status of a network port.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum PortStatus {
    /// Port is up and operational.
    Up,
    /// Port is down.
    Down,
    /// Port is in test mode.
    Test,
    /// Unknown status value for forward compatibility.
    Unknown(u16),
}
