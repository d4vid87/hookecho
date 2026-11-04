/// Site power source.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum PowerSource {
    /// Running on utility power.
    Utility,
    /// Running on generator power.
    Generator,
    /// Unknown value for forward compatibility.
    Unknown(u16),
}
