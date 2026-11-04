use crate::messages::primitive_aliases::{Code2, Integer2, SInteger2};
use std::fmt::Debug;
use zerocopy::{FromBytes, Immutable, KnownLayout};

/// Raw RDA Control Commands message (Table X) to be read directly from the Archive II file.
///
/// This 26-halfword (52-byte) structure contains commands sent from the RPG to control the RDA
/// system's state, scanning strategy, and various operational parameters.
#[repr(C)]
#[derive(Clone, PartialEq, Eq, Hash, Debug, FromBytes, Immutable, KnownLayout)]
pub struct Message {
    /// RDA State Command.
    ///
    /// Values:
    ///       0 = No change
    ///   32769 = Stand-By
    ///   32772 = Operate
    ///   32776 = Restart
    pub rda_state_command: Code2,

    /// RDA Log Command.
    ///
    /// Values:
    ///   0 = No change
    ///   1 = Enable
    ///   2 = Disable
    pub rda_log_command: Code2,

    /// Auxiliary Power Generator Control.
    ///
    /// Values:
    ///       0 = No change
    ///   32770 = Switch to utility power
    ///   32772 = Switch to auxiliary power
    pub auxiliary_power_generator_control: Code2,

    /// RDA Control Authorization.
    ///
    /// Values:
    ///    0 = No change
    ///    2 = Command clear
    ///    4 = Local control
    ///    8 = Remote control accepted
    ///   16 = Remote control requested
    pub rda_control_authorization: Code2,

    /// Restart VCP or Elevation Cut.
    ///
    /// Values:
    ///       0 = None
    ///   32768 = Restart VCP
    ///   32768 + cut_number = Restart elevation cut
    pub restart_vcp_or_elevation_cut: Code2,

    /// Select Local VCP Number.
    ///
    /// Values:
    ///       0 = No change
    ///   1-767 = Pattern number
    ///   32767 = Use remote
    pub select_local_vcp_number: Integer2,

    /// Spare (halfword 7).
    pub spare_7: Integer2,

    /// Super Resolution Control.
    ///
    /// Values:
    ///   0 = No change
    ///   2 = Enable
    ///   4 = Disable
    pub super_resolution_control: Code2,

    /// Clutter Mitigation Decision Control.
    ///
    /// Values:
    ///   0 = No change
    ///   2 = Enable
    ///   4 = Disable
    pub clutter_mitigation_decision_control: Code2,

    /// AVSET Control.
    ///
    /// Values:
    ///   0 = No change
    ///   2 = Enable
    ///   4 = Disable
    pub avset_control: Code2,

    /// Spare (halfword 11).
    pub spare_11: Integer2,

    /// Channel Control Command.
    ///
    /// Values:
    ///   0 = No change
    ///   1 = Set to controlling
    ///   2 = Set to non-controlling
    pub channel_control_command: Code2,

    /// Performance Check Control.
    ///
    /// Values:
    ///   0 = No change
    ///   1 = Force check
    pub performance_check_control: Code2,

    /// ZDR Bias Estimate.
    ///
    /// Values:
    ///      0 = Not available
    ///      1 = No change
    ///   2-1058 = Coded value
    pub zdr_bias_estimate: SInteger2,

    /// Spare (halfwords 15-20).
    pub spare_15_20: [Integer2; 6],

    /// Spot Blanking.
    ///
    /// Values:
    ///   0 = No change
    ///   2 = Enable
    ///   4 = Disable
    pub spot_blanking: Code2,

    /// Spare (halfwords 22-26).
    pub spare_22_26: [Integer2; 5],
}
