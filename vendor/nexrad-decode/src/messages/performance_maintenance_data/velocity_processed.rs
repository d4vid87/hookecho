/// Velocity processing calibration status.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum VelocityProcessed {
    /// Velocity processing is good.
    Good,
    /// Velocity processing has failed.
    Fail,
    /// Unknown status value for forward compatibility.
    Unknown(u16),
}
