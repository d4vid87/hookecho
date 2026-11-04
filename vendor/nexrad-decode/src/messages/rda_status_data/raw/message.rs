use crate::messages::primitive_aliases::{Code2, Integer2, SInteger2, ScaledInteger2};
use std::fmt::Debug;
use zerocopy::{FromBytes, Immutable, KnownLayout};

/// The RDA status data message includes various information about the current RDA system's state,
/// including system operating status, performance parameters, and active alarms.
#[repr(C)]
#[derive(Clone, PartialEq, Eq, Hash, Debug, FromBytes, Immutable, KnownLayout)]
pub struct Message {
    /// The RDA system's status.
    ///
    /// Statuses:
    ///   2 (bit 1) = Start-up
    ///   4 (bit 2) = Standby
    ///   8 (bit 3) = Restart
    ///  16 (bit 4) = Operate
    ///  32 (bit 5) = Spare
    ///  64 (bit 6) = Spare
    pub rda_status: Code2,

    /// The RDA system's operability status.
    ///
    /// Statuses:
    ///   2 (bit 1) = On-line
    ///   4 (bit 2) = Maintenance action required
    ///   8 (bit 3) = Maintenance action mandatory
    ///  16 (bit 4) = Commanded shut down
    ///  32 (bit 5) = Inoperable
    pub operability_status: Code2,

    /// The RDA system's control status.
    ///
    /// Statuses:
    ///   2 (bit 1) = Local control only
    ///   4 (bit 2) = Remote (RPG) control only
    ///   8 (bit 3) = Either local or remote control
    pub control_status: Code2,

    /// The RDA system's auxiliary power generator state.
    ///
    /// States:
    ///   1 (bit 0) = Switched to auxiliary power
    ///   2 (bit 1) = Utility power available
    ///   4 (bit 2) = Generator on
    ///   8 (bit 3) = Transfer switch set to manual
    ///  16 (bit 4) = Commanded switchover
    pub auxiliary_power_generator_state: Code2,

    /// The average transmitter power in watts calculated over a range of samples.
    pub average_transmitter_power: Integer2,

    /// Difference from adaptation data (delta dBZ0) in dB. Scaling is two decimal places, e.g.
    /// a value of -19800 represents -198.00 dB.
    pub horizontal_reflectivity_calibration_correction: ScaledInteger2,

    /// Which types of data have transmission enabled.
    ///
    /// Types:
    ///   1 (bit 1) = None
    ///   2 (bit 2) = Reflectivity
    ///   4 (bit 3) = Velocity
    ///   8 (bit 4) = Spectrum width
    pub data_transmission_enabled: Code2,

    /// The radar's volume coverage pattern number.
    ///
    /// The magnitude of the value identifies the pattern, and the sign indicates whether it was
    /// specified locally or remotely. Zero indicates no pattern.
    pub volume_coverage_pattern: SInteger2,

    /// The RDA system's mode of control.
    ///
    /// Modes:
    ///   0 (none)  = No action
    ///   2 (bit 1) = Local control requested
    ///   4 (bit 2) = Remote control requested (local released)
    pub rda_control_authorization: Code2,

    /// The RDA system's major and minor build numbers.
    ///
    /// If the value divided by 100 is greater than 2 then the build version is the value divided
    /// by 100, otherwise it is divided by 10.
    pub rda_build_number: ScaledInteger2,

    /// Whether the RDA system is operational.
    ///
    /// Modes:
    ///   4 (bit 2) = Operational
    ///   8 (bit 3) = Maintenance
    pub operational_mode: Code2,

    /// Whether the RDA system has super resolution enabled.
    ///
    /// Statuses:
    ///   2 (bit 1) = Enabled
    ///   4 (bit 2) = Disabled
    pub super_resolution_status: Code2,

    /// The RDA system's clutter mitigation status.
    ///
    /// Bits 1-5 indicate which elevation segments of the bypass map are applied.
    ///
    /// Statuses:
    ///   0 (none)  = Disabled
    ///   1 (bit 0) = Enabled
    ///   2 (bit 1) = Bypass map elevation 1 applied
    ///   4 (bit 2) = Bypass map elevation 2 applied
    ///   8 (bit 3) = Bypass map elevation 3 applied
    ///  16 (bit 4) = Bypass map elevation 4 applied
    ///  32 (bit 5) = Bypass map elevation 5 applied
    pub clutter_mitigation_decision_status: Code2,

    /// Multiple flags for the RDA system's scan and data status.
    ///
    /// Flags:
    ///   2 (bit 1) = AVSET enabled
    ///   4 (bit 2) = AVSET disabled
    ///   8 (bit 3) = EBC enablement
    ///  16 (bit 4) = RDA log data enablement
    ///  32 (bit 5) = Time series data recording enablement
    pub rda_scan_and_data_flags: Code2,

    /// The RDA system's active alarm types.
    ///
    /// Types:
    ///   0 (none)  = No alarms
    ///   1 (bit 1) = Tower/utilities
    ///   2 (bit 2) = Pedestal
    ///   4 (bit 3) = Transmitter
    ///   8 (bit 4) = Receiver
    ///  16 (bit 5) = RDA control
    ///  32 (bit 6) = Communication
    ///  64 (bit 7) = Signal processor
    pub rda_alarm_summary: Code2,

    /// Acknowledgement of command receipt by RDA system.
    ///
    /// Codes:
    ///   0 (none)    = No acknowledgement
    ///   1 (bit 0)   = Remote VCP received
    ///   2 (bit 1)   = Clutter bypass map received
    ///   3 (bit 0&1) = Clutter censor zones received
    ///   4 (bit 2)   = Redundant channel control command accepted
    pub command_acknowledgement: Code2,

    /// Indicates whether this is the RDA system's controlling channel.
    ///
    /// Values:
    ///   0 (none)  = Controlling channel
    ///   1 (bit 0) = Non-controlling channel
    pub channel_control_status: Code2,

    /// The RDA system's spot blanking status.
    ///
    /// Statuses:
    ///   0 (none)  = Not installed
    ///   1 (bit 1) = Enabled
    ///   4 (bit 2) = Disabled
    pub spot_blanking_status: Code2,

    /// The bypass map generation date represented as a count of days since 1 January 1970 00:00 GMT.
    /// It is also referred-to as a "modified Julian date" where it is the Julian date - 2440586.5.
    pub bypass_map_generation_date: Integer2,

    /// The bypass map generation time in minutes past midnight, GMT.
    pub bypass_map_generation_time: Integer2,

    /// The clutter filter map generation date represented as a count of days since 1 January 1970
    /// 00:00 GMT. It is also referred-to as a "modified Julian date" where it is the
    /// Julian date - 2440586.5.
    pub clutter_filter_map_generation_date: Integer2,

    /// The clutter filter map generation time in minutes past midnight, GMT.
    pub clutter_filter_map_generation_time: Integer2,

    /// The RDA system's vertical reflectivity calibration correction in dB.
    pub vertical_reflectivity_calibration_correction: ScaledInteger2,

    /// The RDA system's TPS.
    ///
    /// Statuses:
    ///   0 (none)    = Not installed
    ///   1 (bit 0)   = Off
    ///   3 (bit 0&1) = OK
    ///   4 (bit 2)   = Unknown
    pub transition_power_source_status: Integer2,

    /// The RDA system's RMS control status.
    ///
    /// Statuses:
    ///   0 (none)  = Non-RMS system
    ///   2 (bit 1) = RMS in control
    ///   4 (bit 2) = RDA in control
    pub rms_control_status: Code2,

    /// The RDA system's performance check status.
    ///
    /// Statuses:
    ///   0 (none)  = No command pending
    ///   1 (bit 0) = Force performance check pending
    ///   2 (bit 1) = In progress
    pub performance_check_status: Code2,

    /// The RDA system's alarm codes stored per-halfword up to 14 possible codes.
    pub alarm_codes: [Integer2; 14],

    /// Flags indicating the various RDA signal processing options.
    pub signal_processor_options: Code2,

    pub spares: [Integer2; 17],

    /// The VCP number currently in use, downloaded from the RPG.
    pub downloaded_pattern_number: Integer2,

    /// Version of status message.
    pub status_version: Integer2,
}
