/// The possible RDA system auxiliary power generator states.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum AuxiliaryPowerGeneratorState {
    /// System is running on auxiliary power.
    SwitchedToAuxiliaryPower,
    /// Utility (main) power is available.
    UtilityPowerAvailable,
    /// Generator is currently running.
    GeneratorOn,
    /// Transfer switch is set to manual mode.
    TransferSwitchSetToManual,
    /// Switchover to auxiliary power was commanded.
    CommandedSwitchover,
    /// Unknown auxiliary power generator state value for forward compatibility.
    Unknown(u16),
}
