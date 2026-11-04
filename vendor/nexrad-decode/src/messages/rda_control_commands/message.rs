use crate::messages::rda_control_commands::raw;
use crate::result::Result;
use crate::segmented_slice_reader::SegmentedSliceReader;
use std::borrow::Cow;
use std::fmt::Debug;

/// The RDA Control Commands message contains commands sent from the RPG to control the RDA
/// system's state, scanning strategy, and various operational parameters.
///
/// This message's contents correspond to ICD 2620002AA section 3.2.4.6 Table X.
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

    /// RDA State Command.
    ///
    /// Values:
    ///       0 = No change
    ///   32769 = Stand-By
    ///   32772 = Operate
    ///   32776 = Restart
    pub fn rda_state_command(&self) -> u16 {
        self.inner.rda_state_command.get()
    }

    /// RDA Log Command.
    ///
    /// Values:
    ///   0 = No change
    ///   1 = Enable
    ///   2 = Disable
    pub fn rda_log_command(&self) -> u16 {
        self.inner.rda_log_command.get()
    }

    /// Auxiliary Power Generator Control.
    ///
    /// Values:
    ///       0 = No change
    ///   32770 = Switch to utility power
    ///   32772 = Switch to auxiliary power
    pub fn auxiliary_power_generator_control(&self) -> u16 {
        self.inner.auxiliary_power_generator_control.get()
    }

    /// RDA Control Authorization.
    ///
    /// Values:
    ///    0 = No change
    ///    2 = Command clear
    ///    4 = Local control
    ///    8 = Remote control accepted
    ///   16 = Remote control requested
    pub fn rda_control_authorization(&self) -> u16 {
        self.inner.rda_control_authorization.get()
    }

    /// Restart VCP or Elevation Cut.
    ///
    /// Values:
    ///       0 = None
    ///   32768 = Restart VCP
    ///   32768 + cut_number = Restart elevation cut
    pub fn restart_vcp_or_elevation_cut(&self) -> u16 {
        self.inner.restart_vcp_or_elevation_cut.get()
    }

    /// Select Local VCP Number.
    ///
    /// Values:
    ///       0 = No change
    ///   1-767 = Pattern number
    ///   32767 = Use remote
    pub fn select_local_vcp_number(&self) -> u16 {
        self.inner.select_local_vcp_number.get()
    }

    /// Super Resolution Control.
    ///
    /// Values:
    ///   0 = No change
    ///   2 = Enable
    ///   4 = Disable
    pub fn super_resolution_control(&self) -> u16 {
        self.inner.super_resolution_control.get()
    }

    /// Clutter Mitigation Decision Control.
    ///
    /// Values:
    ///   0 = No change
    ///   2 = Enable
    ///   4 = Disable
    pub fn clutter_mitigation_decision_control(&self) -> u16 {
        self.inner.clutter_mitigation_decision_control.get()
    }

    /// AVSET Control.
    ///
    /// Values:
    ///   0 = No change
    ///   2 = Enable
    ///   4 = Disable
    pub fn avset_control(&self) -> u16 {
        self.inner.avset_control.get()
    }

    /// Channel Control Command.
    ///
    /// Values:
    ///   0 = No change
    ///   1 = Set to controlling
    ///   2 = Set to non-controlling
    pub fn channel_control_command(&self) -> u16 {
        self.inner.channel_control_command.get()
    }

    /// Performance Check Control.
    ///
    /// Values:
    ///   0 = No change
    ///   1 = Force check
    pub fn performance_check_control(&self) -> u16 {
        self.inner.performance_check_control.get()
    }

    /// ZDR Bias Estimate.
    ///
    /// Values:
    ///      0 = Not available
    ///      1 = No change
    ///   2-1058 = Coded value
    pub fn zdr_bias_estimate(&self) -> i16 {
        self.inner.zdr_bias_estimate.get()
    }

    /// Spot Blanking.
    ///
    /// Values:
    ///   0 = No change
    ///   2 = Enable
    ///   4 = Disable
    pub fn spot_blanking(&self) -> u16 {
        self.inner.spot_blanking.get()
    }

    /// Convert this message to an owned version with `'static` lifetime.
    pub fn into_owned(self) -> Message<'static> {
        Message {
            inner: Cow::Owned(self.inner.into_owned()),
        }
    }
}
