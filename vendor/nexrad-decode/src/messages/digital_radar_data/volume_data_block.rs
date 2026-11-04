use super::raw;
use super::{ProcessingStatus, VolumeCoveragePattern};
use std::borrow::Cow;

#[cfg(feature = "uom")]
use uom::si::f64::{Angle, Information, Length, Power};

/// Internal representation of the volume data block, supporting both legacy and modern formats.
///
/// The format expanded at Build 20.0 (ICD 2620002U, July 2021).
#[derive(Clone, PartialEq, Debug)]
enum VolumeDataBlockInner<'a> {
    /// Legacy format (Build 10.0–19.0, 40 bytes, lrtup = 44).
    Legacy(Cow<'a, raw::VolumeDataBlockLegacy>),
    /// Modern format (Build 20.0+, 48 bytes, lrtup = 52).
    Modern(Cow<'a, raw::VolumeDataBlock>),
}

/// A volume data moment block.
///
/// This type provides access to volume metadata from digital radar data messages.
/// It supports both the legacy 40-byte format (Build 19.0 and earlier) and the
/// modern 48-byte format (Build 20.0 and later).
///
/// Fields that were added in Build 20.0 (`zdr_bias_estimate_weighted_mean` and `spare`)
/// return `Option` types that are `None` for legacy data.
#[derive(Clone, PartialEq, Debug)]
pub struct VolumeDataBlock<'a> {
    inner: VolumeDataBlockInner<'a>,
}

impl<'a> VolumeDataBlock<'a> {
    /// Create a new VolumeDataBlock wrapper from a raw VolumeDataBlock reference (modern format).
    pub(crate) fn new(inner: &'a raw::VolumeDataBlock) -> Self {
        Self {
            inner: VolumeDataBlockInner::Modern(Cow::Borrowed(inner)),
        }
    }

    /// Create a new VolumeDataBlock wrapper from a raw VolumeDataBlockLegacy reference.
    pub(crate) fn new_legacy(inner: &'a raw::VolumeDataBlockLegacy) -> Self {
        Self {
            inner: VolumeDataBlockInner::Legacy(Cow::Borrowed(inner)),
        }
    }

    /// Returns true if this is a legacy format block (Build 19.0 and earlier).
    pub fn is_legacy(&self) -> bool {
        matches!(self.inner, VolumeDataBlockInner::Legacy(..))
    }

    /// Convert this volume data block to an owned version with `'static` lifetime.
    pub fn into_owned(self) -> VolumeDataBlock<'static> {
        match self.inner {
            VolumeDataBlockInner::Legacy(inner) => VolumeDataBlock {
                inner: VolumeDataBlockInner::Legacy(Cow::Owned(inner.into_owned())),
            },
            VolumeDataBlockInner::Modern(inner) => VolumeDataBlock {
                inner: VolumeDataBlockInner::Modern(Cow::Owned(inner.into_owned())),
            },
        }
    }

    /// Size of data block in bytes (raw value).
    pub fn lrtup_raw(&self) -> u16 {
        match &self.inner {
            VolumeDataBlockInner::Legacy(inner) => inner.lrtup.get(),
            VolumeDataBlockInner::Modern(inner) => inner.lrtup.get(),
        }
    }

    /// Major version number.
    pub fn major_version_number(&self) -> u8 {
        match &self.inner {
            VolumeDataBlockInner::Legacy(inner) => inner.major_version_number,
            VolumeDataBlockInner::Modern(inner) => inner.major_version_number,
        }
    }

    /// Minor version number.
    pub fn minor_version_number(&self) -> u8 {
        match &self.inner {
            VolumeDataBlockInner::Legacy(inner) => inner.minor_version_number,
            VolumeDataBlockInner::Modern(inner) => inner.minor_version_number,
        }
    }

    /// Latitude of radar in degrees (raw value).
    pub fn latitude_raw(&self) -> f32 {
        match &self.inner {
            VolumeDataBlockInner::Legacy(inner) => inner.latitude.get(),
            VolumeDataBlockInner::Modern(inner) => inner.latitude.get(),
        }
    }

    /// Longitude of radar in degrees (raw value).
    pub fn longitude_raw(&self) -> f32 {
        match &self.inner {
            VolumeDataBlockInner::Legacy(inner) => inner.longitude.get(),
            VolumeDataBlockInner::Modern(inner) => inner.longitude.get(),
        }
    }

    /// Height of site base above sea level in meters (raw value).
    pub fn site_height_raw(&self) -> i16 {
        match &self.inner {
            VolumeDataBlockInner::Legacy(inner) => inner.site_height.get(),
            VolumeDataBlockInner::Modern(inner) => inner.site_height.get(),
        }
    }

    /// Height of radar tower above ground in meters (raw value).
    pub fn tower_height_raw(&self) -> u16 {
        match &self.inner {
            VolumeDataBlockInner::Legacy(inner) => inner.tower_height.get(),
            VolumeDataBlockInner::Modern(inner) => inner.tower_height.get(),
        }
    }

    /// Reflectivity scaling factor without correction by ground noise scaling factors given in
    /// adaptation data message in dB.
    pub fn calibration_constant(&self) -> f32 {
        match &self.inner {
            VolumeDataBlockInner::Legacy(inner) => inner.calibration_constant.get(),
            VolumeDataBlockInner::Modern(inner) => inner.calibration_constant.get(),
        }
    }

    /// Transmitter power for horizontal channel in kW (raw value).
    pub fn horizontal_shv_tx_power_raw(&self) -> f32 {
        match &self.inner {
            VolumeDataBlockInner::Legacy(inner) => inner.horizontal_shv_tx_power.get(),
            VolumeDataBlockInner::Modern(inner) => inner.horizontal_shv_tx_power.get(),
        }
    }

    /// Transmitter power for vertical channel in kW (raw value).
    pub fn vertical_shv_tx_power_raw(&self) -> f32 {
        match &self.inner {
            VolumeDataBlockInner::Legacy(inner) => inner.vertical_shv_tx_power.get(),
            VolumeDataBlockInner::Modern(inner) => inner.vertical_shv_tx_power.get(),
        }
    }

    /// Calibration of system ZDR in dB.
    pub fn system_differential_reflectivity(&self) -> f32 {
        match &self.inner {
            VolumeDataBlockInner::Legacy(inner) => inner.system_differential_reflectivity.get(),
            VolumeDataBlockInner::Modern(inner) => inner.system_differential_reflectivity.get(),
        }
    }

    /// Initial DP for the system in degrees (raw value).
    pub fn initial_system_differential_phase_raw(&self) -> f32 {
        match &self.inner {
            VolumeDataBlockInner::Legacy(inner) => inner.initial_system_differential_phase.get(),
            VolumeDataBlockInner::Modern(inner) => inner.initial_system_differential_phase.get(),
        }
    }

    /// Identifies the volume coverage pattern in use (raw value).
    pub fn volume_coverage_pattern_number(&self) -> u16 {
        match &self.inner {
            VolumeDataBlockInner::Legacy(inner) => inner.volume_coverage_pattern_number.get(),
            VolumeDataBlockInner::Modern(inner) => inner.volume_coverage_pattern_number.get(),
        }
    }

    /// Processing option flags (raw value).
    pub fn processing_status_raw(&self) -> u16 {
        match &self.inner {
            VolumeDataBlockInner::Legacy(inner) => inner.processing_status.get(),
            VolumeDataBlockInner::Modern(inner) => inner.processing_status.get(),
        }
    }

    /// RPG weighted mean ZDR bias estimate in dB.
    ///
    /// Returns `None` for legacy data (Build 19.0 and earlier) as this field
    /// was added in Build 20.0.
    pub fn zdr_bias_estimate_weighted_mean(&self) -> Option<u16> {
        match &self.inner {
            VolumeDataBlockInner::Legacy(_) => None,
            VolumeDataBlockInner::Modern(inner) => {
                Some(inner.zdr_bias_estimate_weighted_mean.get())
            }
        }
    }

    /// Spare bytes.
    ///
    /// Returns `None` for legacy data (Build 19.0 and earlier) as this field
    /// was added in Build 20.0.
    pub fn spare(&self) -> Option<&[u8; 6]> {
        match &self.inner {
            VolumeDataBlockInner::Legacy(_) => None,
            VolumeDataBlockInner::Modern(inner) => Some(&inner.spare.0),
        }
    }

    /// Size of data block.
    #[cfg(feature = "uom")]
    pub fn lrtup(&self) -> Information {
        Information::new::<uom::si::information::byte>(self.lrtup_raw() as f64)
    }

    /// Latitude of radar.
    #[cfg(feature = "uom")]
    pub fn latitude(&self) -> Angle {
        Angle::new::<uom::si::angle::degree>(self.latitude_raw() as f64)
    }

    /// Longitude of radar.
    #[cfg(feature = "uom")]
    pub fn longitude(&self) -> Angle {
        Angle::new::<uom::si::angle::degree>(self.longitude_raw() as f64)
    }

    /// Height of site base above sea level.
    #[cfg(feature = "uom")]
    pub fn site_height(&self) -> Length {
        Length::new::<uom::si::length::meter>(self.site_height_raw() as f64)
    }

    /// Height of radar tower above ground.
    #[cfg(feature = "uom")]
    pub fn tower_height(&self) -> Length {
        Length::new::<uom::si::length::meter>(self.tower_height_raw() as f64)
    }

    /// Transmitter power for horizontal channel.
    #[cfg(feature = "uom")]
    pub fn horizontal_shv_tx_power(&self) -> Power {
        Power::new::<uom::si::power::kilowatt>(self.horizontal_shv_tx_power_raw() as f64)
    }

    /// Transmitter power for vertical channel.
    #[cfg(feature = "uom")]
    pub fn vertical_shv_tx_power(&self) -> Power {
        Power::new::<uom::si::power::kilowatt>(self.vertical_shv_tx_power_raw() as f64)
    }

    /// Initial DP for the system.
    #[cfg(feature = "uom")]
    pub fn initial_system_differential_phase(&self) -> Angle {
        Angle::new::<uom::si::angle::degree>(self.initial_system_differential_phase_raw() as f64)
    }

    /// Identifies the volume coverage pattern in use.
    pub fn volume_coverage_pattern(&self) -> VolumeCoveragePattern {
        let volume_coverage_pattern = self.volume_coverage_pattern_number();
        match volume_coverage_pattern {
            12 => VolumeCoveragePattern::VCP12,
            31 => VolumeCoveragePattern::VCP31,
            35 => VolumeCoveragePattern::VCP35,
            112 => VolumeCoveragePattern::VCP112,
            212 => VolumeCoveragePattern::VCP212,
            215 => VolumeCoveragePattern::VCP215,
            other => VolumeCoveragePattern::Unknown(other),
        }
    }

    /// Processing option flags.
    pub fn processing_status(&self) -> ProcessingStatus {
        let processing_status = self.processing_status_raw();
        match processing_status {
            0 => ProcessingStatus::RxRNoise,
            1 => ProcessingStatus::CBT,
            _ => ProcessingStatus::Other(processing_status),
        }
    }
}
