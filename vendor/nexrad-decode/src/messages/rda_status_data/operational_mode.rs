/// The possible RDA system operational modes.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum OperationalMode {
    /// Normal operational mode.
    Operational,
    /// Maintenance mode for service and diagnostics.
    Maintenance,
    /// Unknown operational mode value for forward compatibility.
    Unknown(u16),
}
