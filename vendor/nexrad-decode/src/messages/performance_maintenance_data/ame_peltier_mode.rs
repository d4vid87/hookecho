/// AME Peltier cooler operating mode.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum AmePeltierMode {
    /// Peltier cooler is in cooling mode.
    Cool,
    /// Peltier cooler is in heating mode.
    Heat,
    /// Unknown mode value for forward compatibility.
    Unknown(u16),
}
