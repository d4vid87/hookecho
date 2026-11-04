/// The possible RDA system operability statuses.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum OperabilityStatus {
    /// System is online and operational.
    OnLine,
    /// Maintenance action is required but system is operational.
    MaintenanceActionRequired,
    /// Maintenance action is mandatory.
    MaintenanceActionMandatory,
    /// System has been commanded to shut down.
    CommandedShutDown,
    /// System is inoperable.
    Inoperable,
    /// Unknown operability status value for forward compatibility.
    Unknown(u16),
}
