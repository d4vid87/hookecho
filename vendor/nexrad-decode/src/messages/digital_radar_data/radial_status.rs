/// Possible statuses for a radial describing its position within the larger scan.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum RadialStatus {
    /// First radial of an elevation sweep (not the first sweep in the volume).
    ElevationStart,
    /// A radial within an elevation sweep (not the first or last radial).
    IntermediateRadialData,
    /// Last radial of an elevation sweep.
    ElevationEnd,
    /// First radial of the first elevation sweep in a volume scan.
    VolumeScanStart,
    /// Last radial of the last elevation sweep in a volume scan.
    VolumeScanEnd,
    /// Start of new elevation which is the last in the VCP.
    ElevationStartVCPFinal,
    /// An unrecognized radial status value.
    Unknown(u8),
}
