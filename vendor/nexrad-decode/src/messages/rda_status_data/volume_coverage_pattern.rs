use crate::messages::primitive_aliases::SInteger2;
use std::fmt::Debug;

/// The RDA system's volume coverage pattern number.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct VolumeCoveragePatternNumber(SInteger2);

impl VolumeCoveragePatternNumber {
    pub(crate) fn new(value: SInteger2) -> Self {
        Self(value)
    }

    /// The volume coverage pattern number.
    pub fn number(&self) -> i16 {
        self.0.get().abs()
    }

    /// Whether the volume coverage pattern number was specified locally.
    pub fn local(&self) -> bool {
        self.0 < 0
    }

    /// Whether the volume coverage pattern number was specified remotely.
    pub fn remote(&self) -> bool {
        self.0 > 0
    }
}
