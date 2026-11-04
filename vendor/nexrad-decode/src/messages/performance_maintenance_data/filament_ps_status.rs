/// Transmitter filament power supply status.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum FilamentPsStatus {
    /// Filament power supply is on.
    On,
    /// Filament power supply is off.
    Off,
    /// Unknown status value for forward compatibility.
    Unknown(u16),
}
