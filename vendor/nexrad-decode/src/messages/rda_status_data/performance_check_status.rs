/// The RDA system's performance check status.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum PerformanceCheckStatus {
    /// No performance check command is pending.
    NoCommandPending,
    /// A forced performance check is pending.
    ForcePerformanceCheckPending,
    /// Performance check is currently in progress.
    InProgress,
    /// Unknown performance check status value for forward compatibility.
    Unknown(u16),
}
