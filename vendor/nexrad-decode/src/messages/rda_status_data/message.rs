use crate::messages::rda_status_data::alarm;
use crate::messages::rda_status_data::alarm::Summary;
use crate::messages::rda_status_data::raw;
use crate::messages::rda_status_data::{
    AuxiliaryPowerGeneratorState, ClutterMitigationDecisionStatus, CommandAcknowledgement,
    ControlAuthorization, ControlStatus, DataTransmissionEnabled, OperabilityStatus,
    OperationalMode, PerformanceCheckStatus, RDABuildNumber, RDAStatus, RMSControlStatus,
    ScanDataFlags, SpotBlankingStatus, SuperResolutionStatus, TransitionPowerSourceStatus,
    VolumeCoveragePatternNumber,
};
use crate::result::Result;
use crate::segmented_slice_reader::SegmentedSliceReader;
use crate::util::get_datetime;
use chrono::{DateTime, Duration, Utc};
use std::borrow::Cow;
use std::fmt::Debug;

/// The RDA status data message includes various information about the current RDA system's state,
/// including system operating status, performance parameters, and active alarms.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Message<'a> {
    inner: Cow<'a, raw::Message>,
}

impl<'a> Message<'a> {
    pub(crate) fn parse(reader: &mut SegmentedSliceReader<'a, '_>) -> Result<Self> {
        let inner = reader.take_ref::<raw::Message>()?;
        Ok(Self {
            inner: Cow::Borrowed(inner),
        })
    }

    /// The RDA system's status.
    ///
    /// Statuses:
    ///   2 (bit 1) = Start-up
    ///   4 (bit 2) = Standby
    ///   8 (bit 3) = Restart
    ///  16 (bit 4) = Operate
    ///  32 (bit 5) = Spare
    ///  64 (bit 6) = Spare
    pub fn raw_rda_status(&self) -> u16 {
        self.inner.rda_status.get()
    }

    /// The RDA system's operability status.
    ///
    /// Statuses:
    ///   2 (bit 1) = On-line
    ///   4 (bit 2) = Maintenance action required
    ///   8 (bit 3) = Maintenance action mandatory
    ///  16 (bit 4) = Commanded shut down
    ///  32 (bit 5) = Inoperable
    pub fn raw_operability_status(&self) -> u16 {
        self.inner.operability_status.get()
    }

    /// The RDA system's control status.
    ///
    /// Statuses:
    ///   2 (bit 1) = Local control only
    ///   4 (bit 2) = Remote (RPG) control only
    ///   8 (bit 3) = Either local or remote control
    pub fn raw_control_status(&self) -> u16 {
        self.inner.control_status.get()
    }

    /// The RDA system's auxiliary power generator state.
    ///
    /// States:
    ///   1 (bit 0) = Switched to auxiliary power
    ///   2 (bit 1) = Utility power available
    ///   4 (bit 2) = Generator on
    ///   8 (bit 3) = Transfer switch set to manual
    ///  16 (bit 4) = Commanded switchover
    pub fn raw_auxiliary_power_generator_state(&self) -> u16 {
        self.inner.auxiliary_power_generator_state.get()
    }

    /// The average transmitter power in watts calculated over a range of samples.
    pub fn average_transmitter_power(&self) -> u16 {
        self.inner.average_transmitter_power.get()
    }

    /// Difference from adaptation data (delta dBZ0) in dB. Scaling is two decimal places, e.g.
    /// a value of -19800 represents -198.00 dB.
    pub fn raw_horizontal_reflectivity_calibration_correction(&self) -> u16 {
        self.inner
            .horizontal_reflectivity_calibration_correction
            .get()
    }

    /// Which types of data have transmission enabled.
    ///
    /// Types:
    ///   1 (bit 1) = None
    ///   2 (bit 2) = Reflectivity
    ///   4 (bit 3) = Velocity
    ///   8 (bit 4) = Spectrum width
    pub fn raw_data_transmission_enabled(&self) -> u16 {
        self.inner.data_transmission_enabled.get()
    }

    /// The radar's volume coverage pattern number.
    ///
    /// The magnitude of the value identifies the pattern, and the sign indicates whether it was
    /// specified locally or remotely. Zero indicates no pattern.
    pub fn raw_volume_coverage_pattern(&self) -> i16 {
        self.inner.volume_coverage_pattern.get()
    }

    /// The RDA system's mode of control.
    ///
    /// Modes:
    ///   0 (none)  = No action
    ///   2 (bit 1) = Local control requested
    ///   4 (bit 2) = Remote control requested (local released)
    pub fn raw_rda_control_authorization(&self) -> u16 {
        self.inner.rda_control_authorization.get()
    }

    /// The RDA system's major and minor build numbers.
    ///
    /// If the value divided by 100 is greater than 2 then the build version is the value divided
    /// by 100, otherwise it is divided by 10.
    pub fn raw_rda_build_number(&self) -> u16 {
        self.inner.rda_build_number.get()
    }

    /// Whether the RDA system is operational.
    ///
    /// Modes:
    ///   4 (bit 2) = Operational
    ///   8 (bit 3) = Maintenance
    pub fn raw_operational_mode(&self) -> u16 {
        self.inner.operational_mode.get()
    }

    /// Whether the RDA system has super resolution enabled.
    ///
    /// Statuses:
    ///   2 (bit 1) = Enabled
    ///   4 (bit 2) = Disabled
    pub fn raw_super_resolution_status(&self) -> u16 {
        self.inner.super_resolution_status.get()
    }

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
    pub fn raw_clutter_mitigation_decision_status(&self) -> u16 {
        self.inner.clutter_mitigation_decision_status.get()
    }

    /// Multiple flags for the RDA system's scan and data status.
    ///
    /// Flags:
    ///   2 (bit 1) = AVSET enabled
    ///   4 (bit 2) = AVSET disabled
    ///   8 (bit 3) = EBC enablement
    ///  16 (bit 4) = RDA log data enablement
    ///  32 (bit 5) = Time series data recording enablement
    pub fn raw_rda_scan_and_data_flags(&self) -> u16 {
        self.inner.rda_scan_and_data_flags.get()
    }

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
    pub fn raw_rda_alarm_summary(&self) -> u16 {
        self.inner.rda_alarm_summary.get()
    }

    /// Acknowledgement of command receipt by RDA system.
    ///
    /// Codes:
    ///   0 (none)    = No acknowledgement
    ///   1 (bit 0)   = Remote VCP received
    ///   2 (bit 1)   = Clutter bypass map received
    ///   3 (bit 0&1) = Clutter censor zones received
    ///   4 (bit 2)   = Redundant channel control command accepted
    pub fn raw_command_acknowledgement(&self) -> u16 {
        self.inner.command_acknowledgement.get()
    }

    /// Indicates whether this is the RDA system's controlling channel.
    ///
    /// Values:
    ///   0 (none)  = Controlling channel
    ///   1 (bit 0) = Non-controlling channel
    pub fn raw_channel_control_status(&self) -> u16 {
        self.inner.channel_control_status.get()
    }

    /// The RDA system's spot blanking status.
    ///
    /// Statuses:
    ///   0 (none)  = Not installed
    ///   1 (bit 1) = Enabled
    ///   4 (bit 2) = Disabled
    pub fn raw_spot_blanking_status(&self) -> u16 {
        self.inner.spot_blanking_status.get()
    }

    /// The bypass map generation date represented as a count of days since 1 January 1970 00:00 GMT.
    /// It is also referred-to as a "modified Julian date" where it is the Julian date - 2440586.5.
    pub fn bypass_map_generation_date(&self) -> u16 {
        self.inner.bypass_map_generation_date.get()
    }

    /// The bypass map generation time in minutes past midnight, GMT.
    pub fn bypass_map_generation_time(&self) -> u16 {
        self.inner.bypass_map_generation_time.get()
    }

    /// The clutter filter map generation date represented as a count of days since 1 January 1970
    /// 00:00 GMT. It is also referred-to as a "modified Julian date" where it is the
    /// Julian date - 2440586.5.
    pub fn clutter_filter_map_generation_date(&self) -> u16 {
        self.inner.clutter_filter_map_generation_date.get()
    }

    /// The clutter filter map generation time in minutes past midnight, GMT.
    pub fn clutter_filter_map_generation_time(&self) -> u16 {
        self.inner.clutter_filter_map_generation_time.get()
    }

    /// The RDA system's vertical reflectivity calibration correction in dB.
    pub fn raw_vertical_reflectivity_calibration_correction(&self) -> u16 {
        self.inner
            .vertical_reflectivity_calibration_correction
            .get()
    }

    /// The RDA system's TPS.
    ///
    /// Statuses:
    ///   0 (none)    = Not installed
    ///   1 (bit 0)   = Off
    ///   3 (bit 0&1) = OK
    ///   4 (bit 2)   = Unknown
    pub fn raw_transition_power_source_status(&self) -> u16 {
        self.inner.transition_power_source_status.get()
    }

    /// The RDA system's RMS control status.
    ///
    /// Statuses:
    ///   0 (none)  = Non-RMS system
    ///   2 (bit 1) = RMS in control
    ///   4 (bit 2) = RDA in control
    pub fn raw_rms_control_status(&self) -> u16 {
        self.inner.rms_control_status.get()
    }

    /// The RDA system's performance check status.
    ///
    /// Statuses:
    ///   0 (none)  = No command pending
    ///   1 (bit 0) = Force performance check pending
    ///   2 (bit 1) = In progress
    pub fn raw_performance_check_status(&self) -> u16 {
        self.inner.performance_check_status.get()
    }

    /// The RDA system's alarm codes stored per-halfword up to 14 possible codes.
    pub fn raw_alarm_codes(&self) -> [u16; 14] {
        self.inner.alarm_codes.map(|c| c.get())
    }

    /// Flags indicating the various RDA signal processing options.
    pub fn raw_signal_processor_options(&self) -> u16 {
        self.inner.signal_processor_options.get()
    }

    /// The VCP number currently in use, downloaded from the RPG.
    pub fn downloaded_pattern_number(&self) -> u16 {
        self.inner.downloaded_pattern_number.get()
    }

    /// Version of status message.
    pub fn status_version(&self) -> u16 {
        self.inner.status_version.get()
    }

    /// The RDA system's status.
    pub fn rda_status(&self) -> RDAStatus {
        match self.inner.rda_status.get() {
            2 => RDAStatus::StartUp,
            4 => RDAStatus::Standby,
            8 => RDAStatus::Restart,
            16 => RDAStatus::Operate,
            32 => RDAStatus::Spare,
            other => RDAStatus::Unknown(other),
        }
    }

    /// The RDA system's operability status.
    pub fn operability_status(&self) -> OperabilityStatus {
        match self.inner.operability_status.get() {
            2 => OperabilityStatus::OnLine,
            4 => OperabilityStatus::MaintenanceActionRequired,
            8 => OperabilityStatus::MaintenanceActionMandatory,
            16 => OperabilityStatus::CommandedShutDown,
            32 => OperabilityStatus::Inoperable,
            other => OperabilityStatus::Unknown(other),
        }
    }

    /// The RDA system's control status.
    pub fn control_status(&self) -> ControlStatus {
        match self.inner.control_status.get() {
            2 => ControlStatus::LocalControlOnly,
            4 => ControlStatus::RemoteControlOnly,
            8 => ControlStatus::EitherLocalOrRemoteControl,
            other => ControlStatus::Unknown(other),
        }
    }

    /// The RDA system's auxiliary power generator state.
    pub fn auxiliary_power_generator_state(&self) -> AuxiliaryPowerGeneratorState {
        match self.inner.auxiliary_power_generator_state.get() {
            1 => AuxiliaryPowerGeneratorState::SwitchedToAuxiliaryPower,
            2 => AuxiliaryPowerGeneratorState::UtilityPowerAvailable,
            4 => AuxiliaryPowerGeneratorState::GeneratorOn,
            8 => AuxiliaryPowerGeneratorState::TransferSwitchSetToManual,
            16 => AuxiliaryPowerGeneratorState::CommandedSwitchover,
            other => AuxiliaryPowerGeneratorState::Unknown(other),
        }
    }

    /// Difference from adaptation data (delta dBZ0) in dB.
    pub fn horizontal_reflectivity_calibration_correction(&self) -> f32 {
        self.inner
            .horizontal_reflectivity_calibration_correction
            .get() as f32
            / 100.0
    }

    /// The types of data that have transmission enabled.
    pub fn data_transmission_enabled(&self) -> DataTransmissionEnabled {
        DataTransmissionEnabled::new(self.inner.data_transmission_enabled)
    }

    /// The radar's volume coverage pattern number.
    pub fn volume_coverage_pattern(&self) -> Option<VolumeCoveragePatternNumber> {
        if self.inner.volume_coverage_pattern == 0 {
            return None;
        }

        Some(VolumeCoveragePatternNumber::new(
            self.inner.volume_coverage_pattern,
        ))
    }

    /// The RDA system's mode of control.
    pub fn rda_control_authorization(&self) -> ControlAuthorization {
        match self.inner.rda_control_authorization.get() {
            0 => ControlAuthorization::NoAction,
            1 => ControlAuthorization::LocalControlRequested,
            2 => ControlAuthorization::RemoteControlRequested,
            other => ControlAuthorization::Unknown(other),
        }
    }

    /// The RDA system's major and minor build numbers.
    pub fn build_number(&self) -> RDABuildNumber {
        RDABuildNumber::from_raw(self.inner.rda_build_number.get())
    }

    /// Whether the RDA system is operational.
    pub fn operational_mode(&self) -> OperationalMode {
        match self.inner.operational_mode.get() {
            4 => OperationalMode::Operational,
            8 => OperationalMode::Maintenance,
            other => OperationalMode::Unknown(other),
        }
    }

    /// Whether the RDA system has super resolution enabled.
    pub fn super_resolution_status(&self) -> SuperResolutionStatus {
        match self.inner.super_resolution_status.get() {
            2 => SuperResolutionStatus::Enabled,
            4 => SuperResolutionStatus::Disabled,
            other => SuperResolutionStatus::Unknown(other),
        }
    }

    /// The RDA system's clutter mitigation status.
    pub fn clutter_mitigation_decision_status(&self) -> ClutterMitigationDecisionStatus {
        match self.inner.clutter_mitigation_decision_status.get() {
            0 => ClutterMitigationDecisionStatus::Disabled,
            1 => ClutterMitigationDecisionStatus::Enabled,
            _ => {
                let mut segments = Vec::new();
                for i in 0..5 {
                    if self.inner.clutter_mitigation_decision_status.get() & (1 << i) != 0 {
                        segments.push(i);
                    }
                }

                ClutterMitigationDecisionStatus::BypassMapElevationSegments(segments)
            }
        }
    }

    /// Multiple flags for the RDA system's scan and data status.
    pub fn rda_scan_and_data_flags(&self) -> ScanDataFlags {
        ScanDataFlags::new(self.inner.rda_scan_and_data_flags)
    }

    /// The RDA system's active alarm types.
    pub fn rda_alarm_summary(&self) -> Summary {
        Summary::new(self.inner.rda_alarm_summary)
    }

    /// Acknowledgement of command receipt by RDA system.
    pub fn command_acknowledgement(&self) -> Option<CommandAcknowledgement> {
        match self.inner.command_acknowledgement.get() {
            1 => Some(CommandAcknowledgement::RemoteVCPReceived),
            2 => Some(CommandAcknowledgement::ClutterBypassMapReceived),
            3 => Some(CommandAcknowledgement::ClutterCensorZonesReceived),
            4 => Some(CommandAcknowledgement::RedundantChannelControlCommandAccepted),
            _ => None,
        }
    }

    /// Indicates whether this is the RDA system's controlling channel.
    pub fn controlling_channel(&self) -> bool {
        self.inner.channel_control_status.get() & 1 == 0
    }

    /// The RDA system's spot blanking status.
    pub fn spot_blanking_status(&self) -> SpotBlankingStatus {
        match self.inner.spot_blanking_status.get() {
            0 => SpotBlankingStatus::NotInstalled,
            1 => SpotBlankingStatus::Enabled,
            4 => SpotBlankingStatus::Disabled,
            other => SpotBlankingStatus::Other(other as u8),
        }
    }

    /// The bypass map generation date and time in UTC.
    pub fn bypass_map_generation_date_time(&self) -> Option<DateTime<Utc>> {
        get_datetime(
            self.inner.bypass_map_generation_date.get(),
            Duration::minutes(self.inner.bypass_map_generation_time.get() as i64),
        )
    }

    /// The clutter filter map generation date and time in UTC.
    pub fn clutter_filter_map_generation_date_time(&self) -> Option<DateTime<Utc>> {
        get_datetime(
            self.inner.clutter_filter_map_generation_date.get(),
            Duration::minutes(self.inner.clutter_filter_map_generation_time.get() as i64),
        )
    }

    /// The RDA system's TPS.
    pub fn transition_power_source_status(&self) -> TransitionPowerSourceStatus {
        match self.inner.transition_power_source_status.get() {
            0 => TransitionPowerSourceStatus::NotInstalled,
            1 => TransitionPowerSourceStatus::Off,
            3 => TransitionPowerSourceStatus::OK,
            4 => TransitionPowerSourceStatus::Unknown,
            other => TransitionPowerSourceStatus::Other(other),
        }
    }

    /// The RDA system's RMS control status.
    pub fn rms_control_status(&self) -> RMSControlStatus {
        match self.inner.rms_control_status.get() {
            0 => RMSControlStatus::NonRMS,
            2 => RMSControlStatus::RMSInControl,
            4 => RMSControlStatus::RDAInControl,
            other => RMSControlStatus::Unknown(other),
        }
    }

    /// The RDA system's performance check status.
    pub fn performance_check_status(&self) -> PerformanceCheckStatus {
        match self.inner.performance_check_status.get() {
            0 => PerformanceCheckStatus::NoCommandPending,
            1 => PerformanceCheckStatus::ForcePerformanceCheckPending,
            2 => PerformanceCheckStatus::InProgress,
            other => PerformanceCheckStatus::Unknown(other),
        }
    }

    /// The RDA system's alarm messages.
    pub fn alarm_messages(&self) -> Vec<alarm::Message> {
        self.inner
            .alarm_codes
            .iter()
            .filter(|&code| *code != 0)
            .filter_map(|&code| alarm::get_alarm_message(code.get()))
            .collect()
    }

    /// Convert this message to an owned version with `'static` lifetime.
    pub fn into_owned(self) -> Message<'static> {
        Message {
            inner: Cow::Owned(self.inner.into_owned()),
        }
    }
}
