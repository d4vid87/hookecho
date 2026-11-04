/// AME Peltier cooler status.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum AmePeltierStatus {
    /// Peltier cooler is off.
    Off,
    /// Peltier cooler is on.
    On,
    /// Unknown status value for forward compatibility.
    Unknown(u16),
}
