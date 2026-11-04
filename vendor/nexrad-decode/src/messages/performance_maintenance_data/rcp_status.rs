/// RCP (Radar Control Processor) status.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum RcpStatus {
    /// RCP is operating normally.
    Ok,
    /// RCP is not operating normally.
    NotOk,
    /// Unknown status value for forward compatibility.
    Unknown(u16),
}
