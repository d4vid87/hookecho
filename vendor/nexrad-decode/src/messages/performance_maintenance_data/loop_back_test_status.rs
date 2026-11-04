/// Loop back test status.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum LoopBackTestStatus {
    /// Test passed.
    Pass,
    /// Test failed.
    Fail,
    /// Test timed out.
    Timeout,
    /// Test was not performed.
    NotTested,
    /// Unknown status value for forward compatibility.
    Unknown(u16),
}
