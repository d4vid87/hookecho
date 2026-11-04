/// The types of data messages transferred between the RDA and RPG.
#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy, Ord, PartialOrd)]
#[repr(u8)]
pub enum MessageType {
    /// Replaced by message type 31.
    RDADigitalRadarData = 1,

    /// Metadata.
    RDAStatusData = 2,

    /// Metadata.
    RDAPerformanceMaintenanceData = 3,

    /// Console message from RDA.
    RDAConsoleMessage = 4,

    /// Metadata.
    RDAVolumeCoveragePattern = 5,

    /// Control commands to RDA.
    RDAControlCommands = 6,

    /// VCP from RPG.
    RPGVolumeCoveragePattern = 7,

    /// Clutter censor zones from RPG.
    RPGClutterCensorZones = 8,

    /// Data request from RPG.
    RPGRequestForData = 9,

    /// Console message from RPG.
    RPGConsoleMessage = 10,

    /// Loopback test from RDA.
    RDALoopBackTest = 11,

    /// Loopback test from RPG.
    RPGLoopBackTest = 12,

    /// No longer sent.
    RDAClutterFilterBypassMap = 13,

    /// Spare message type.
    Spare1 = 14,

    /// Metadata.
    RDAClutterFilterMap = 15,

    /// Reserved for FAA RMS use.
    ReservedFAARMSOnly1 = 16,

    /// Reserved for FAA RMS use.
    ReservedFAARMSOnly2 = 17,

    /// Metadata.
    RDAAdaptationData = 18,

    /// Reserved message type.
    Reserved1 = 20,

    /// Reserved message type.
    Reserved2 = 21,

    /// Reserved message type.
    Reserved3 = 22,

    /// Reserved message type.
    Reserved4 = 23,

    /// Reserved for FAA RMS use.
    ReservedFAARMSOnly3 = 24,

    /// Reserved for FAA RMS use.
    ReservedFAARMSOnly4 = 25,

    /// Reserved for FAA RMS use.
    ReservedFAARMSOnly5 = 26,

    /// Reserved message type.
    Reserved5 = 29,

    /// Generic format digital radar data (primary data format).
    RDADigitalRadarDataGenericFormat = 31,

    /// PRF data from RDA.
    RDAPRFData = 32,

    /// Log data from RDA.
    RDALogData = 33,

    /// Unknown message type for forward compatibility.
    Unknown(u8),
}
