/// Pedestal interlock switch position.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum PedestalInterlockSwitch {
    /// Switch is in operational position.
    Operational,
    /// Switch is in safe position.
    Safe,
    /// Unknown position value for forward compatibility.
    Unknown(u16),
}
