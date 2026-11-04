/// Waveguide switch position.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum WgSwitchPosition {
    /// Switch is in antenna position.
    Antenna,
    /// Switch is in dummy load position.
    DummyLoad,
    /// Unknown position value for forward compatibility.
    Unknown(u16),
}
