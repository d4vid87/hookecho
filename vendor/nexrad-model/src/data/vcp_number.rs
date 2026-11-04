use std::fmt::Display;

/// Volume Coverage Pattern (VCP) number identifying the radar's scanning strategy.
///
/// Each VCP defines the number of elevation cuts, scan speed, pulse characteristics, and
/// range-unfolding techniques. The radar operator selects a VCP based on meteorological
/// conditions.
///
/// Serializes as its raw `u16` pattern number for compatibility with archival formats.
///
/// # Sources
///
/// Definitions are drawn from official NOAA/NWS documentation:
/// - NOAA JetStream: Volume Coverage Patterns
/// - WSR-88D Radar Operations Center VCP Info Sheet
/// - RDA/RPG ICD (2620002 series)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VCPNumber {
    /// VCP 12: Precipitation mode. 14 elevation angles in ~4.5 minutes. Fastest scan;
    /// default for severe weather. Does not use SZ-2 phase coding.
    Precipitation12,

    /// VCP 31: Clear air mode, long pulse. 5 elevation angles in ~10 minutes. The longer
    /// transmitted pulse increases sensitivity, permitting detection of very weak returns.
    ClearAirLongPulse31,

    /// VCP 32: Clear air mode, short pulse. 5 elevation angles in ~10 minutes. Identical
    /// scan angles to VCP 31 but uses short pulse for higher unambiguous velocity.
    ClearAirShortPulse32,

    /// VCP 35: Clear air mode with SZ-2. 9 elevation angles in ~7 minutes. Default clear
    /// air VCP. Scans the same lower angles as VCP 12 and applies SZ-2 range unfolding.
    ClearAir35,

    /// VCP 112: Precipitation mode with MPDA. ~15 elevation angles in ~5.5 minutes.
    /// Employs both SZ-2 and Multiple PRF Dealiasing Algorithm. Designed for tropical
    /// cyclones and large-scale systems with widespread high velocities.
    PrecipitationMpda112,

    /// VCP 212: Precipitation mode with SZ-2. 14-15 elevation angles in ~4.5-5 minutes.
    /// Applies SZ-2 phase coding on the lowest tilts for second-trip echo recovery.
    /// Supports up to 3 SAILS or 4 MRLE cuts.
    PrecipitationSz2_212,

    /// VCP 215: General surveillance / standard precipitation with SZ-2. 15 elevation
    /// angles in ~6 minutes. Combines low-angle coverage of VCP 12/212 with upper-angle
    /// coverage of the former VCP 11. Default for widespread rain or snow without intense
    /// thunderstorms.
    GeneralSurveillance215,

    /// An unrecognized VCP number for forward compatibility with new scan strategies.
    Unknown(u16),
}

impl VCPNumber {
    /// Creates a `VCPNumber` from a raw `u16` pattern number.
    pub fn from_number(number: u16) -> Self {
        match number {
            12 => Self::Precipitation12,
            31 => Self::ClearAirLongPulse31,
            32 => Self::ClearAirShortPulse32,
            35 => Self::ClearAir35,
            112 => Self::PrecipitationMpda112,
            212 => Self::PrecipitationSz2_212,
            215 => Self::GeneralSurveillance215,
            other => Self::Unknown(other),
        }
    }

    /// Returns the raw `u16` pattern number.
    pub fn number(&self) -> u16 {
        match self {
            Self::Precipitation12 => 12,
            Self::ClearAirLongPulse31 => 31,
            Self::ClearAirShortPulse32 => 32,
            Self::ClearAir35 => 35,
            Self::PrecipitationMpda112 => 112,
            Self::PrecipitationSz2_212 => 212,
            Self::GeneralSurveillance215 => 215,
            Self::Unknown(n) => *n,
        }
    }

    /// A human-readable description of the scan strategy.
    pub fn description(&self) -> &'static str {
        match self {
            Self::Precipitation12 => "Precipitation, fast update",
            Self::ClearAirLongPulse31 => "Clear air, long pulse",
            Self::ClearAirShortPulse32 => "Clear air, short pulse",
            Self::ClearAir35 => "Clear air, SZ-2",
            Self::PrecipitationMpda112 => "Precipitation, MPDA + SZ-2",
            Self::PrecipitationSz2_212 => "Precipitation, SZ-2",
            Self::GeneralSurveillance215 => "General surveillance, SZ-2",
            Self::Unknown(_) => "Unknown",
        }
    }
}

impl Display for VCPNumber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(n) => write!(f, "VCP {n}"),
            known => write!(f, "VCP {} ({})", known.number(), known.description()),
        }
    }
}

impl From<u16> for VCPNumber {
    fn from(number: u16) -> Self {
        Self::from_number(number)
    }
}

impl From<VCPNumber> for u16 {
    fn from(vcp: VCPNumber) -> Self {
        vcp.number()
    }
}

// Serialize as the raw u16 pattern number for backward compatibility with archival formats
// and existing snapshot tests.
#[cfg(feature = "serde")]
impl serde::Serialize for VCPNumber {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u16(self.number())
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for VCPNumber {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let number = u16::deserialize(deserializer)?;
        Ok(Self::from_number(number))
    }
}
