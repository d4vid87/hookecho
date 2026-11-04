/// Transmitter air filter condition.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum TransmitterAirFilter {
    /// Air filter is dirty.
    Dirty,
    /// Air filter is OK.
    Ok,
    /// Unknown status value for forward compatibility.
    Unknown(u16),
}
