/// The RDA system's RMS control status.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum RMSControlStatus {
    /// Not an RMS-controlled site.
    NonRMS,
    /// RMS (Remote Monitoring System) is in control.
    RMSInControl,
    /// RDA is in local control.
    RDAInControl,
    /// Unknown RMS control status value for forward compatibility.
    Unknown(u16),
}
