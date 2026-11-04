/// Transmitter recycling summary status.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum TransmitterRecyclingSummary {
    /// Normal operation.
    Normal,
    /// Transmitter is recycling.
    Recycling,
    /// Unknown status value for forward compatibility.
    Unknown(u16),
}
