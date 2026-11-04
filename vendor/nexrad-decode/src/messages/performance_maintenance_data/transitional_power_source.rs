/// Transitional power source status.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum TransitionalPowerSource {
    /// Transitional power source is OK.
    Ok,
    /// Transitional power source is off.
    Off,
    /// Unknown status value for forward compatibility.
    Unknown(u16),
}
