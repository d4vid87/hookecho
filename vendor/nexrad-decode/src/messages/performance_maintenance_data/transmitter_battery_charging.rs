/// Transmitter battery charging status.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum TransmitterBatteryCharging {
    /// Battery is charging.
    Charging,
    /// Battery is not charging.
    NotCharging,
    /// Unknown status value for forward compatibility.
    Unknown(u16),
}
