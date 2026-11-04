/// AME operational mode.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum AmeMode {
    /// AME is ready for operation.
    Ready,
    /// AME is in maintenance mode.
    Maintenance,
    /// Unknown mode value for forward compatibility.
    Unknown(u16),
}
