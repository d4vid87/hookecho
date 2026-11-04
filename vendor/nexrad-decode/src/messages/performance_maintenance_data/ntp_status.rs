/// NTP synchronization status.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum NtpStatus {
    /// NTP is operating normally.
    Ok,
    /// NTP has failed.
    Fail,
    /// Unknown status value for forward compatibility.
    Unknown(u16),
}
