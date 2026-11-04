//!
//! Message type 2 "RDA Status Data" contains information about the current RDA state, system
//! control, operating status, scanning strategy, performance parameters like transmitter power and
//! calibration, and system alarms. This message is sent upon wideband connection, after state or
//! control changes, at the beginning of each volume scan, and after an RPG request.
//!

pub mod alarm;

mod auxiliary_power_generator_state;
pub use auxiliary_power_generator_state::AuxiliaryPowerGeneratorState;

mod clutter_mitigation_decision_status;
pub use clutter_mitigation_decision_status::ClutterMitigationDecisionStatus;

mod command_acknowledgement;
pub use command_acknowledgement::CommandAcknowledgement;

mod control_authorization;
pub use control_authorization::ControlAuthorization;

mod control_status;
pub use control_status::ControlStatus;

mod data_transmission_enabled;
pub use data_transmission_enabled::DataTransmissionEnabled;

mod operability_status;
pub use operability_status::OperabilityStatus;

mod operational_mode;
pub use operational_mode::OperationalMode;

mod performance_check_status;
pub use performance_check_status::PerformanceCheckStatus;

mod rda_status;
pub use rda_status::RDAStatus;

mod rms_control_status;
pub use rms_control_status::RMSControlStatus;

mod scan_data_flags;
pub use scan_data_flags::ScanDataFlags;

mod spot_blanking_status;
pub use spot_blanking_status::SpotBlankingStatus;

mod super_resolution_status;
pub use super_resolution_status::SuperResolutionStatus;

mod transition_power_source_status;
pub use transition_power_source_status::TransitionPowerSourceStatus;

mod volume_coverage_pattern;
pub use volume_coverage_pattern::VolumeCoveragePatternNumber;

mod rda_build_number;
pub use rda_build_number::RDABuildNumber;

mod message;
pub use message::Message;

pub(crate) mod raw;

#[cfg(test)]
mod snapshot_test;
