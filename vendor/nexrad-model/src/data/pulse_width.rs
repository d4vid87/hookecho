use std::fmt::Display;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Radar pulse width configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum PulseWidth {
    /// Short pulse width.
    Short,
    /// Long pulse width.
    Long,
    /// Unknown pulse width value.
    Unknown,
}

impl Display for PulseWidth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PulseWidth::Short => write!(f, "Short"),
            PulseWidth::Long => write!(f, "Long"),
            PulseWidth::Unknown => write!(f, "Unknown"),
        }
    }
}
