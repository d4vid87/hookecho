use crate::messages::primitive_aliases::Code2;
use std::fmt::Debug;

/// The RDA system's active alarm types.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Summary(Code2);

impl Summary {
    pub(crate) fn new(code: Code2) -> Self {
        Self(code)
    }

    /// Whether no alarms are active.
    pub fn none(&self) -> bool {
        self.0 == 0
    }

    /// Whether the tower/utilities alarm is active.
    pub fn tower_utilities(&self) -> bool {
        self.0 & 0b0001 != 0
    }

    /// Whether the pedestal alarm is active.
    pub fn pedestal(&self) -> bool {
        self.0 & 0b0010 != 0
    }

    /// Whether the transmitter alarm is active.
    pub fn transmitter(&self) -> bool {
        self.0 & 0b0100 != 0
    }

    /// Whether the receiver alarm is active.
    pub fn receiver(&self) -> bool {
        self.0 & 0b1000 != 0
    }

    /// Whether the RDA control alarm is active.
    pub fn rda_control(&self) -> bool {
        self.0 & 0b10000 != 0
    }

    /// Whether the communication alarm is active.
    pub fn communication(&self) -> bool {
        self.0 & 0b100000 != 0
    }

    /// Whether the signal processor alarm is active.
    pub fn signal_processor(&self) -> bool {
        self.0 & 0b1000000 != 0
    }
}
