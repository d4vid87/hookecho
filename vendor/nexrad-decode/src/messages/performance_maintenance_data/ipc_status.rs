/// IPC (Inter-Process Communication) status.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum IpcStatus {
    /// IPC is operating normally.
    Ok,
    /// IPC has failed.
    Fail,
    /// IPC is not applicable.
    NotApplicable,
    /// Unknown status value for forward compatibility.
    Unknown(u16),
}
