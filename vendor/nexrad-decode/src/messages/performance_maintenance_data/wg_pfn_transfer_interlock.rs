/// Waveguide/PFN transfer interlock status.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum WgPfnTransferInterlock {
    /// Interlock is OK.
    Ok,
    /// Interlock is open.
    Open,
    /// Unknown status value for forward compatibility.
    Unknown(u16),
}
