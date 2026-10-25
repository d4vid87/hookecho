//! wgpu rendering: a slippy-map tile layer plus a polar radar layer, both drawn
//! inside egui's render pass via [`egui_wgpu::CallbackTrait`].

pub mod field_ramps;
pub mod mercator;
mod radar_surface;
pub use radar_surface::radar_surface_supported;

use std::collections::{HashMap, VecDeque};

use std::num::NonZeroU64;
use wgpu::util::DeviceExt;

/// XYZ tile id.
pub type TileId = (u8, u32, u32);

/// A tile plus which basemap style it came from. Panes can show different basemaps at once, so
/// the same `(z, x, y)` may be resident twice with different imagery; the style key keeps them
/// apart in the GPU cache.
pub type TileKey = (u64, TileId);

const MAX_TILE_VERTS: u64 = 512 * 6 * 16 * 16; // up to 512 visible tiles per frame

/// A decoded RGBA tile the app wants uploaded this frame.
pub struct PendingTile {
    pub id: TileId,
    /// Basemap style this tile was fetched for ([`crate::tiles::BasemapStyle::key`]).
    pub style: u64,
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// A tile to draw this frame (world-space rect, top-left origin).
#[derive(Clone, Copy)]
pub struct VisibleTile {
    pub id: TileId,
    pub world_min: [f32; 2],
    pub world_max: [f32; 2],
}

/// A binned sweep plus the world-space quad covering its range disk.
pub struct RadarUpload {
    pub upload_token: u64,
    /// Animation may revisit recent scans; retain bounded GPU textures for that loop.
    pub cache_loop: bool,
    pub az_bins: u32,
    pub gate_count: u32,
    pub data: Vec<u8>,
    pub data16: Option<Vec<u16>>,
    /// [radar_lat, radar_lon, first_gate_km, gate_interval_km, az_bins, gate_count,
    ///  smoothing, srv, motion_e, motion_n, tint, flag_nx, flag_ny, flag_west, flag_north,
    ///  flag_east, flag_south, blend_width, code_max, multipass] (see `shaders/radar.wgsl`).
    pub uniform: [f32; 20],
    /// Half-degree azimuth opacity; absent means all sectors are fully opaque.
    pub sector_opacity: Option<[f32; 720]>,
    /// 256×3 RGBA color LUT indexed by the sweep's `u8`: row 0 rain (the user's own table),
    /// row 1 snow, row 2 mix. Rows 1 and 2 are copies of row 0 unless the precipitation-type
    /// tint is on (see `colormap::tint_lut`).
    pub lut: Vec<u8>,
    /// MRMS surface precipitation-type classes on their own lat/lon grid, one byte per cell
    /// (0 rain, 1 snow, 2 mix). Empty when the tint is off; a 1×1 dummy is bound instead,
    /// because every binding has to exist whether or not it is read.
    pub precip_flag: Vec<u8>,
    /// World-space quad corners covering the disk (min/max box).
    pub world_min: [f32; 2],
    pub world_max: [f32; 2],
    /// Only the color table changed: `data` and `precip_flag` are empty and the retained
    /// textures keep their contents. A palette drag writes 3 KB instead of re-uploading the
    /// ~1.3 MB gate texture it was already showing.
    pub lut_only: bool,
}

/// A national gridded field layer (all share the MRMS warp pipeline; they differ only in data,
/// LUT, and draw order). `below_radar` layers paint under the single-site radar; the rest above.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum FieldLayer {
    GoesC13,
    GoesWaterVapor,
    GoesMidWaterVapor,
    GoesLongwaveIr,
    GoesVisible,
    GoesTrueColor,
    GoesCatalog(u8),
    Mrms,
    MrmsLowLevel,
    Hrrr,
    Rotation,
    Mesh,
    AzShear,
    AzShearMid,
    Posh,
    MrmsEchoTop18,
    MrmsVil,
    MrmsCatalog(u8),
    Lightning,
    /// Instantaneous precipitation rate (mm/hr) — how hard it is coming down right now,
    /// as against the QPE layers' how much has fallen.
    PrecipRate,
    Qpe1h,
    Qpe3h,
    Qpe6h,
    Qpe12h,
    Qpe24h,
    /// Single-site Level III digital one-hour accumulation.
    Daa,
    /// Single-site Level III 16-band one-hour precipitation total.
    Oha,
    /// Single-site Level III digital storm-total accumulation.
    Dsa,
    /// HRRR surface CAPE (environment suite).
    Cape,
    /// HRRR storm-relative helicity (environment suite).
    Srh,
    /// MRMS surface precipitation type (categorical context layer).
    PrecipType,
    /// MRMS FLASH flash-flood average recurrence interval.
    FlashFlood,
    /// Gridded Digital VIL (L3 DVL packet-16 product).
    Vil,
    /// Enhanced Echo Tops (L3 EET packet-16 product).
    EchoTops,
    /// 24-hour MESH max — hail swaths / damage tracks (MRMS).
    HailSwath,
    /// Hybrid Hydrometeor Classification (L3 HHC packet-16 product, categorical).
    Hca,
    /// HRRR max updraft helicity — forecast rotation tracks.
    UpdraftHelicity,
    /// HRRR near-surface smoke (mass density).
    Smoke,
    /// Multi-radar base-reflectivity composite (L3 N0B from every radar covering the view).
    Mosaic,
    /// Column-maximum reflectivity from this radar's own volume — the strongest echo anywhere
    /// above each point. Unlike the MRMS composite this is one site at its own resolution, and
    /// it works in archive replay.
    CompositeLocal,
    /// VIL integrated locally from the Level 2 volume (works in archive replay, unlike L3 DVL).
    VilLocal,
    /// VIL density — VIL over echo-top height, the classic large-hail discriminator.
    VilDensity,
    /// Echo tops integrated locally from the Level 2 volume, at a user-set dBZ threshold.
    EtopLocal,
    /// Maximum Expected Hail Size derived from the volume (Witt et al. 1998).
    HailMehs,
    /// Probability of Severe Hail derived from the volume (Witt et al. 1998).
    HailPosh,
    /// HRRR accumulated snowfall through the scrubbed forecast hour.
    Snowfall,
    /// NOHRSC observed snowfall analysis over the last 6/24/48/72 hours.
    SnowAnalysis,
    /// Global model mean sea-level pressure (GFS or ECMWF — see `settings.global_model`).
    GlobalMslp,
    /// Global model 500 hPa geopotential height.
    GlobalHeight500,
    /// Global model 2 m temperature.
    GlobalTemp2m,
    /// Global model 2 m dewpoint.
    GlobalDewpoint2m,
    /// Global model 10 m wind speed.
    GlobalWind10m,
    /// Global model precipitable water / total precipitation.
    GlobalPrecip,
    /// NOAA real-time surface analyses.
    RtmaTemp2m,
    RtmaDewpoint2m,
    RtmaPressure,
    RtmaWindU10m,
    RtmaWindV10m,
    RtmaGust10m,
    RtmaVisibility,
    RtmaPrecip1h,
    /// Moving-window maximum of MRMS reflectivity with contributor history.
    MrmsReflectivityTrail,
    /// Banded precipitation from the MRMS mosaic, narrowed to snow — the snow-squall layer.
    SnowBands,
    /// NBM calibrated probability of thunder over the hour ending at the scrubbed forecast hour.
    ThunderProb,
    /// REFS neighborhood probability of composite reflectivity above the selected threshold.
    RefsReflectivityProb,
    /// GLM flash-extent density — the recent satellite flashes gridded into a density field.
    GlmFed,
    /// One model minus another — which field, and therefore which pair, is `app.diff_field`.
    ModelDiff,
}

impl FieldLayer {
    /// Painting under the single-site radar (national context) vs. over it (severe signals).
    pub fn below_radar(self) -> bool {
        matches!(
            self,
            FieldLayer::GoesC13
                | FieldLayer::GoesWaterVapor
                | FieldLayer::GoesMidWaterVapor
                | FieldLayer::GoesLongwaveIr
                | FieldLayer::GoesVisible
                | FieldLayer::GoesTrueColor
                | FieldLayer::GoesCatalog(_)
                | FieldLayer::Mrms
                | FieldLayer::MrmsLowLevel
                | FieldLayer::Mosaic
                | FieldLayer::Hrrr
                | FieldLayer::Cape
                | FieldLayer::Srh
                | FieldLayer::PrecipType
                | FieldLayer::ThunderProb
                | FieldLayer::RefsReflectivityProb
                | FieldLayer::Smoke
                | FieldLayer::Snowfall
                | FieldLayer::SnowAnalysis
                | FieldLayer::GlobalMslp
                | FieldLayer::GlobalHeight500
                | FieldLayer::GlobalTemp2m
                | FieldLayer::GlobalDewpoint2m
                | FieldLayer::GlobalWind10m
                | FieldLayer::GlobalPrecip
                | FieldLayer::RtmaTemp2m
                | FieldLayer::RtmaDewpoint2m
                | FieldLayer::RtmaPressure
                | FieldLayer::RtmaWindU10m
                | FieldLayer::RtmaWindV10m
                | FieldLayer::RtmaGust10m
                | FieldLayer::RtmaVisibility
                | FieldLayer::RtmaPrecip1h
                | FieldLayer::MrmsReflectivityTrail
                | FieldLayer::ModelDiff
        )
    }

    /// Fixed bottom-to-top paint order within each band.
    pub const BASE_DRAW_ORDER: [FieldLayer; 74] = [
        // Below-radar context band (bottom to top). The global models sit at the very bottom:
        // they are the synoptic backdrop everything else is drawn against.
        FieldLayer::GoesC13,
        FieldLayer::GoesWaterVapor,
        FieldLayer::GoesMidWaterVapor,
        FieldLayer::GoesLongwaveIr,
        FieldLayer::GoesVisible,
        FieldLayer::GoesTrueColor,
        FieldLayer::GoesCatalog(0),
        FieldLayer::GoesCatalog(1),
        FieldLayer::GoesCatalog(2),
        FieldLayer::GoesCatalog(3),
        FieldLayer::GoesCatalog(4),
        FieldLayer::GoesCatalog(5),
        FieldLayer::GoesCatalog(6),
        FieldLayer::GoesCatalog(7),
        FieldLayer::GoesCatalog(8),
        FieldLayer::GoesCatalog(9),
        FieldLayer::GoesCatalog(10),
        FieldLayer::GlobalMslp,
        FieldLayer::GlobalHeight500,
        FieldLayer::GlobalTemp2m,
        FieldLayer::GlobalDewpoint2m,
        FieldLayer::GlobalWind10m,
        FieldLayer::GlobalPrecip,
        FieldLayer::RtmaTemp2m,
        FieldLayer::RtmaDewpoint2m,
        FieldLayer::RtmaPressure,
        FieldLayer::RtmaWindU10m,
        FieldLayer::RtmaWindV10m,
        FieldLayer::RtmaGust10m,
        FieldLayer::RtmaVisibility,
        FieldLayer::RtmaPrecip1h,
        FieldLayer::ModelDiff,
        FieldLayer::Mrms,
        FieldLayer::MrmsReflectivityTrail,
        FieldLayer::MrmsLowLevel,
        FieldLayer::Mosaic,
        FieldLayer::Hrrr,
        FieldLayer::Cape,
        FieldLayer::Srh,
        FieldLayer::Smoke,
        FieldLayer::Snowfall,
        FieldLayer::SnowAnalysis,
        FieldLayer::PrecipType,
        FieldLayer::ThunderProb,
        FieldLayer::RefsReflectivityProb,
        // Above-radar severe-signal band.
        FieldLayer::SnowBands,
        FieldLayer::PrecipRate,
        FieldLayer::Qpe1h,
        FieldLayer::Qpe3h,
        FieldLayer::Qpe6h,
        FieldLayer::Qpe12h,
        FieldLayer::Qpe24h,
        FieldLayer::Daa,
        FieldLayer::Oha,
        FieldLayer::Dsa,
        FieldLayer::FlashFlood,
        FieldLayer::HailSwath,
        FieldLayer::CompositeLocal,
        FieldLayer::Vil,
        FieldLayer::VilLocal,
        FieldLayer::EchoTops,
        FieldLayer::EtopLocal,
        FieldLayer::VilDensity,
        FieldLayer::HailMehs,
        FieldLayer::HailPosh,
        FieldLayer::Hca,
        FieldLayer::UpdraftHelicity,
        FieldLayer::Rotation,
        FieldLayer::Mesh,
        FieldLayer::AzShear,
        FieldLayer::AzShearMid,
        FieldLayer::Posh,
        FieldLayer::MrmsEchoTop18,
        FieldLayer::MrmsVil,
    ];

    pub fn is_native_abi(self) -> bool {
        matches!(self, Self::GoesC13 | Self::GoesWaterVapor | Self::GoesMidWaterVapor | Self::GoesLongwaveIr | Self::GoesVisible | Self::GoesTrueColor | Self::GoesCatalog(_))
    }

    pub fn is_l3_grid(self) -> bool {
        matches!(self, Self::Vil | Self::EchoTops | Self::Hca | Self::Daa | Self::Oha | Self::Dsa)
    }

    pub fn is_pane_field(self) -> bool {
        self.is_native_abi() || self.is_l3_grid()
    }

    pub fn draw_order() -> impl DoubleEndedIterator<Item = FieldLayer> {
        Self::BASE_DRAW_ORDER
            .into_iter()
            .chain((0..wxdata::mrms::CATALOG.len()).map(|index| FieldLayer::MrmsCatalog(index as u8)))
            .chain([FieldLayer::Lightning, FieldLayer::GlmFed])
    }

    /// Stable name for saved files — a workspace records which layers were on by slug, so a file
    /// written by a newer build names a layer this one skips rather than failing to load.
    pub fn slug(self) -> &'static str {
        match self {
            FieldLayer::GoesC13 => "goes-c13",
            FieldLayer::GoesWaterVapor => "goes-water-vapor",
            FieldLayer::GoesMidWaterVapor => "goes-mid-water-vapor",
            FieldLayer::GoesLongwaveIr => "goes-longwave-ir",
            FieldLayer::GoesVisible => "goes-visible",
            FieldLayer::GoesTrueColor => "goes-true-color",
            FieldLayer::GoesCatalog(index) => wxdata::abi::CATALOG
                .get(index as usize)
                .map_or("unknown-goes-catalog", |entry| entry.slug),
            FieldLayer::Mrms => "mrms",
            FieldLayer::MrmsLowLevel => "mrms-low-level",
            FieldLayer::Hrrr => "hrrr",
            FieldLayer::Rotation => "rotation",
            FieldLayer::Mesh => "mesh",
            FieldLayer::AzShear => "azshear",
            FieldLayer::AzShearMid => "azshear-mid",
            FieldLayer::Posh => "posh",
            FieldLayer::MrmsEchoTop18 => "mrms-echo-top-18",
            FieldLayer::MrmsVil => "mrms-vil",
            FieldLayer::MrmsCatalog(index) => wxdata::mrms::CATALOG
                .get(index as usize)
                .map_or("unknown-mrms-catalog", |entry| entry.slug),
            FieldLayer::Lightning => "lightning",
            FieldLayer::PrecipRate => "preciprate",
            FieldLayer::Qpe1h => "qpe1h",
            FieldLayer::Qpe3h => "qpe3h",
            FieldLayer::Qpe6h => "qpe6h",
            FieldLayer::Qpe12h => "qpe12h",
            FieldLayer::Qpe24h => "qpe24h",
            FieldLayer::Daa => "daa",
            FieldLayer::Oha => "oha",
            FieldLayer::Dsa => "dsa",
            FieldLayer::Cape => "cape",
            FieldLayer::Srh => "srh",
            FieldLayer::PrecipType => "preciptype",
            FieldLayer::FlashFlood => "flashflood",
            FieldLayer::Vil => "vil",
            FieldLayer::EchoTops => "echotops",
            FieldLayer::HailSwath => "hailswath",
            FieldLayer::SnowBands => "snowbands",
            FieldLayer::ThunderProb => "thunderprob",
            FieldLayer::RefsReflectivityProb => "refs-reflectivity-probability",
            FieldLayer::Hca => "hca",
            FieldLayer::UpdraftHelicity => "updrafthelicity",
            FieldLayer::Smoke => "smoke",
            FieldLayer::Mosaic => "mosaic",
            FieldLayer::CompositeLocal => "composite-local",
            FieldLayer::VilLocal => "vil-local",
            FieldLayer::VilDensity => "vil-density",
            FieldLayer::EtopLocal => "etop-local",
            FieldLayer::HailMehs => "hail-mehs",
            FieldLayer::HailPosh => "hail-posh",
            FieldLayer::Snowfall => "snowfall",
            FieldLayer::SnowAnalysis => "snow-analysis",
            FieldLayer::GlobalMslp => "global-mslp",
            FieldLayer::GlobalHeight500 => "global-height500",
            FieldLayer::GlobalTemp2m => "global-temp2m",
            FieldLayer::GlobalDewpoint2m => "global-dewpoint2m",
            FieldLayer::GlobalWind10m => "global-wind10m",
            FieldLayer::GlobalPrecip => "global-precip",
            FieldLayer::RtmaTemp2m => "rtma-temp2m",
            FieldLayer::RtmaDewpoint2m => "rtma-dewpoint2m",
            FieldLayer::RtmaPressure => "rtma-pressure",
            FieldLayer::RtmaWindU10m => "rtma-wind-u10m",
            FieldLayer::RtmaWindV10m => "rtma-wind-v10m",
            FieldLayer::RtmaGust10m => "rtma-gust-10m",
            FieldLayer::RtmaVisibility => "rtma-visibility",
            FieldLayer::RtmaPrecip1h => "rtma-precip-1h",
            FieldLayer::MrmsReflectivityTrail => "mrms-reflectivity-trail",
            FieldLayer::ModelDiff => "model-diff",
            FieldLayer::GlmFed => "glm-fed",
        }
    }

    /// The inverse of [`slug`](Self::slug), or `None` for a name this build doesn't have.
    pub fn from_slug(s: &str) -> Option<FieldLayer> {
        Self::draw_order().find(|f| f.slug() == s)
    }

    /// Common metadata for migrated MRMS fields.
    pub fn descriptor(self) -> Option<&'static wxdata::field::FieldDescriptor> {
        use FieldLayer as FL;
        Some(match self {
            FL::GoesC13 => &wxdata::abi::C13_DESCRIPTOR,
            FL::GoesWaterVapor => &wxdata::abi::C08_DESCRIPTOR,
            FL::GoesMidWaterVapor => &wxdata::abi::C09_DESCRIPTOR,
            FL::GoesLongwaveIr => &wxdata::abi::C14_DESCRIPTOR,
            FL::GoesVisible => &wxdata::abi::C02_DESCRIPTOR,
            FL::GoesTrueColor => &wxdata::abi::TRUE_COLOR_DESCRIPTOR,
            FL::GoesCatalog(index) => wxdata::abi::CATALOG.get(index as usize)?.descriptor,
            FL::Mrms => &wxdata::mrms::REFLECTIVITY_DESCRIPTOR,
            FL::MrmsLowLevel => &wxdata::mrms::LOW_LEVEL_REFLECTIVITY_DESCRIPTOR,
            FL::Lightning => &wxdata::mrms::LIGHTNING_DESCRIPTOR,
            FL::Mesh => &wxdata::mrms::MESH_DESCRIPTOR,
            FL::HailSwath => &wxdata::mrms::HAIL_SWATH_DESCRIPTOR,
            FL::AzShear => &wxdata::mrms::AZSHEAR_DESCRIPTOR,
            FL::AzShearMid => &wxdata::mrms::AZSHEAR_MID_DESCRIPTOR,
            FL::Posh => &wxdata::mrms::POSH_DESCRIPTOR,
            FL::MrmsEchoTop18 => &wxdata::mrms::ECHO_TOP_18_DESCRIPTOR,
            FL::MrmsVil => &wxdata::mrms::VIL_DESCRIPTOR,
            FL::MrmsCatalog(index) => wxdata::mrms::CATALOG.get(index as usize)?.descriptor,
            FL::Rotation => &wxdata::mrms::ROTATION_DESCRIPTOR,
            FL::Qpe1h => &wxdata::mrms::QPE_01H_DESCRIPTOR,
            FL::Qpe3h => &wxdata::mrms::QPE_03H_DESCRIPTOR,
            FL::Qpe6h => &wxdata::mrms::QPE_06H_DESCRIPTOR,
            FL::Qpe12h => &wxdata::mrms::QPE_12H_DESCRIPTOR,
            FL::Qpe24h => &wxdata::mrms::QPE_24H_DESCRIPTOR,
            FL::Daa => &wxdata::level3::DAA_DESCRIPTOR,
            FL::Oha => &wxdata::level3::OHA_DESCRIPTOR,
            FL::Dsa => &wxdata::level3::DSA_DESCRIPTOR,
            FL::PrecipRate => &wxdata::mrms::PRECIP_RATE_DESCRIPTOR,
            FL::PrecipType => &wxdata::mrms::PRECIP_TYPE_DESCRIPTOR,
            FL::FlashFlood => &wxdata::mrms::FLASH_ARI30_DESCRIPTOR,
            FL::Hrrr => &wxdata::hrrr::REFLECTIVITY_DESCRIPTOR,
            FL::Cape => &wxdata::hrrr::CAPE_DESCRIPTOR,
            FL::Srh => &wxdata::hrrr::SRH_DESCRIPTOR,
            FL::UpdraftHelicity => &wxdata::hrrr::UPDRAFT_HELICITY_DESCRIPTOR,
            FL::Snowfall => &wxdata::hrrr::SNOWFALL_DESCRIPTOR,
            FL::Smoke => &wxdata::hrrr::SMOKE_DESCRIPTOR,
            FL::ThunderProb => &wxdata::hrrr::THUNDER_PROBABILITY_DESCRIPTOR,
            FL::RefsReflectivityProb => &wxdata::refs::REFLECTIVITY_40_DESCRIPTOR,
            FL::GlobalMslp => &wxdata::global::MSLP_DESCRIPTOR,
            FL::GlobalHeight500 => &wxdata::global::HEIGHT_500_DESCRIPTOR,
            FL::GlobalTemp2m => &wxdata::global::TEMP_2M_DESCRIPTOR,
            FL::GlobalDewpoint2m => &wxdata::global::DEWPOINT_2M_DESCRIPTOR,
            FL::GlobalWind10m => &wxdata::global::WIND_10M_DESCRIPTOR,
            FL::GlobalPrecip => &wxdata::global::PRECIP_DESCRIPTOR,
            FL::RtmaTemp2m => &wxdata::rtma::TEMP_DESCRIPTOR,
            FL::RtmaDewpoint2m => &wxdata::rtma::DEWPOINT_DESCRIPTOR,
            FL::RtmaPressure => &wxdata::rtma::PRESSURE_DESCRIPTOR,
            FL::RtmaWindU10m => &wxdata::rtma::WIND_U_DESCRIPTOR,
            FL::RtmaWindV10m => &wxdata::rtma::WIND_V_DESCRIPTOR,
            FL::RtmaGust10m => &wxdata::rtma::GUST_DESCRIPTOR,
            FL::RtmaVisibility => &wxdata::rtma::VISIBILITY_DESCRIPTOR,
            FL::RtmaPrecip1h => &wxdata::rtma::PRECIP_DESCRIPTOR,
            FL::MrmsReflectivityTrail => &wxdata::trail::REFLECTIVITY_TRAIL_DESCRIPTOR,
            FL::SnowAnalysis => &wxdata::nohrsc::SNOWFALL_DESCRIPTOR,
            FL::SnowBands => &wxdata::banding::SNOW_BANDS_DESCRIPTOR,
            FL::ModelDiff => &crate::fielddiff::MODEL_DIFF_DESCRIPTOR,
            FL::Vil => &wxdata::level3::VIL_DESCRIPTOR,
            FL::EchoTops => &wxdata::level3::ECHO_TOPS_DESCRIPTOR,
            FL::Hca => &wxdata::level3::HCA_DESCRIPTOR,
            FL::GlmFed => &wxdata::glm::FLASH_DENSITY_DESCRIPTOR,
            FL::Mosaic => &wxdata::mosaic::DESCRIPTOR,
            _ => return None,
        })
    }

    /// Registry identity for migrated products; legacy layers keep their established slug.
    pub fn stable_id(self) -> &'static str {
        self.descriptor().map_or_else(|| self.slug(), |d| d.id.0)
    }

    /// Read current registry IDs and every legacy layer slug.
    pub fn from_stable_id(id: &str) -> Option<FieldLayer> {
        Self::draw_order()
            .find(|field| field.stable_id() == id || field.slug() == id)
    }
}

#[cfg(test)]
mod field_slug_tests {
    use super::FieldLayer;

    #[test]
    fn site_radar_grids_and_satellite_grids_have_pane_local_gpu_keys() {
        for layer in [
            FieldLayer::Vil, FieldLayer::EchoTops, FieldLayer::Hca,
            FieldLayer::Daa, FieldLayer::Oha, FieldLayer::Dsa,
        ] {
            assert!(layer.is_l3_grid() && layer.is_pane_field());
            assert!(!layer.is_native_abi());
        }
        assert!(FieldLayer::GoesC13.is_pane_field());
        assert!(!FieldLayer::Mrms.is_pane_field());
    }

    #[test]
    fn every_layer_has_a_slug_that_parses_back() {
        for l in FieldLayer::draw_order() {
            assert_eq!(FieldLayer::from_slug(l.slug()), Some(l), "{}", l.slug());
        }
        let mut slugs: Vec<&str> = FieldLayer::draw_order().map(|l| l.slug()).collect();
        slugs.sort_unstable();
        let n = slugs.len();
        slugs.dedup();
        assert_eq!(slugs.len(), n, "two layers share a slug");
        assert_eq!(FieldLayer::from_slug("not-a-layer"), None);
    }

    #[test]
    fn draw_order_includes_the_entire_mrms_catalog() {
        let layers: Vec<_> = FieldLayer::draw_order().collect();
        assert_eq!(
            layers.len(),
            FieldLayer::BASE_DRAW_ORDER.len() + wxdata::mrms::CATALOG.len() + 2
        );
        for index in 0..wxdata::mrms::CATALOG.len() {
            assert!(layers.contains(&FieldLayer::MrmsCatalog(index as u8)));
        }
    }

    #[test]
    fn migrated_ids_and_legacy_slugs_both_restore() {
        assert_eq!(FieldLayer::Mrms.stable_id(), "mrms.composite-reflectivity");
        assert_eq!(
            FieldLayer::from_stable_id("mrms.composite-reflectivity"),
            Some(FieldLayer::Mrms)
        );
        assert_eq!(FieldLayer::from_stable_id("mrms"), Some(FieldLayer::Mrms));
        assert_eq!(FieldLayer::Hrrr.stable_id(), "model.hrrr.composite-reflectivity");
        assert_eq!(FieldLayer::from_stable_id("hrrr"), Some(FieldLayer::Hrrr));
        assert_eq!(FieldLayer::from_stable_id("cape"), Some(FieldLayer::Cape));
        assert_eq!(FieldLayer::from_stable_id("srh"), Some(FieldLayer::Srh));
        assert_eq!(
            FieldLayer::SnowBands.stable_id(),
            "derived.mrms.snow-bands"
        );
        assert_eq!(
            FieldLayer::from_stable_id("snowbands"),
            Some(FieldLayer::SnowBands)
        );
        assert_eq!(
            FieldLayer::GlmFed.stable_id(),
            "satellite.glm.flash-extent-density"
        );
        for layer in [
            FieldLayer::UpdraftHelicity,
            FieldLayer::Snowfall,
            FieldLayer::Smoke,
            FieldLayer::ThunderProb,
        ] {
            assert_eq!(FieldLayer::from_stable_id(layer.slug()), Some(layer));
            assert!(layer.stable_id().contains('.'));
        }
        assert_eq!(FieldLayer::from_stable_id("future.unknown"), None);
        assert_eq!(FieldLayer::MrmsCatalog(255).slug(), "unknown-mrms-catalog");
        assert_eq!(FieldLayer::GoesCatalog(255).slug(), "unknown-goes-catalog");
        assert_eq!(
            FieldLayer::Lightning.descriptor().unwrap().search_aliases,
            &["nldn", "lightning"]
        );
    }

    #[test]
    fn every_mrms_descriptor_has_one_layer() {
        for descriptor in wxdata::mrms::DESCRIPTORS {
            let layers: Vec<_> = FieldLayer::draw_order()
                .filter(|layer| layer.descriptor().is_some_and(|d| d.id == descriptor.id))
                .collect();
            assert_eq!(layers.len(), 1, "{}", descriptor.id.0);
        }
    }
}

/// A national MRMS mosaic to upload: an R8 index grid + LUT, warped plate-carrée→mercator.
pub struct MrmsUpload {
    pub data: Vec<u8>,
    /// `data` contains RGBA bytes instead of one LUT index per cell.
    pub rgba: bool,
    pub nx: u32,
    pub ny: u32,
    /// World-space quad (mercator bbox of the grid).
    pub world_min: [f32; 2],
    pub world_max: [f32; 2],
    /// [lon_west, lat_north, lon_east, lat_south, nx, ny, opacity, +5 pad] (see
    /// `shaders/mrms.wgsl`). Opacity is rewritten each frame from `field_draws`, so the value
    /// here only matters until the first draw.
    pub uniform: [f32; 12],
    pub lut: Vec<u8>,
}

/// A tessellated vertex for the vector overlay layer.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable, serde::Serialize, serde::Deserialize)]
pub struct OverlayVertex {
    pub world: [f32; 2],
    pub color: [f32; 4],
    /// XY: screen-pixel stroke extrusion. Z: apply the road width scale (0 or 1).
    pub offset: [f32; 3],
}

/// Pre-tessellated overlay geometry to upload this frame.
pub struct OverlayUpload {
    pub vertices: Vec<OverlayVertex>,
    pub indices: Vec<u32>,
}

/// A tessellated vector basemap tile to upload this frame (reuses the overlay pipeline).
pub struct PendingVectorTile {
    pub id: TileId,
    pub vertices: Vec<OverlayVertex>,
    pub indices: Vec<u32>,
    /// Motorway/trunk/primary strokes repeated over radar for geographic orientation.
    pub foreground_vertices: Vec<OverlayVertex>,
    pub foreground_indices: Vec<u32>,
}

/// Per-frame draw instructions handed to the render callback.
pub struct MapCallback {
    /// Which pane this callback draws (indexes into `RenderResources.panes`).
    pub pane: u32,
    /// On a phone, free GPU allocations belonging to hidden analyst panes.
    pub retain_only_pane: Option<u32>,
    /// Drop resources for panes closed by the current layout.
    pub live_pane_count: Option<u32>,
    pub camera_center: [f32; 2],
    pub camera_scale: [f32; 2],
    pub globe: [f32; 4],
    pub planar: [f32; 4],
    pub world_per_pixel: f32,
    pub new_tiles: Vec<PendingTile>,
    pub visible: Vec<VisibleTile>,
    /// Which basemap style this pane draws ([`crate::tiles::BasemapStyle::key`]).
    pub basemap_key: u64,
    /// Draw the vector basemap *after* the raster tiles instead of under them — the hybrid
    /// satellite style, where the vector geometry is roads over imagery.
    pub vector_over_raster: bool,
    pub radar_upload: Option<RadarUpload>,
    pub draw_radar: bool,
    /// Raised reflectivity-value visualization; never a meteorological altitude.
    pub radar_surface: bool,
    /// `Some` only when the overlay geometry changed (else the last upload is reused).
    pub overlay_upload: Option<OverlayUpload>,
    pub draw_overlay: bool,
    /// Drop all cached GPU tiles before uploading (basemap style changed).
    pub clear_tiles: bool,
    /// Individual tiles the tile manager evicted; freed before this frame's uploads. Eviction is
    /// decided there, not here, so both sides agree on what is resident.
    pub drop_tiles: Vec<TileKey>,
    /// Field layers whose grid changed this frame (uploaded now); others reuse the last upload.
    pub field_uploads: Vec<(FieldLayer, MrmsUpload)>,
    /// Which field layers to paint this frame, with their opacity (0..1).
    pub field_draws: Vec<(FieldLayer, f32)>,
    /// Field layers the app has stopped drawing for long enough to free; same contract as
    /// `drop_tiles` — the app decides, we free, and it re-uploads on the next enable.
    pub drop_fields: Vec<FieldLayer>,
    /// Newly tessellated vector basemap tiles to upload this frame.
    pub new_vector_tiles: Vec<PendingVectorTile>,
    /// Vector tile ids to draw this frame (drawn first, under the raster/radar layers).
    pub visible_vector: Vec<TileId>,
    /// Drop all cached vector tiles before uploading (style or tess-zoom changed).
    pub clear_vector: bool,
    /// Individual vector tiles the manager evicted; freed before this frame's uploads.
    pub drop_vector_tiles: Vec<TileId>,
    /// A wind field to (re)upload for the GPU particle layer, with its mercator world bbox.
    pub wind_upload: Option<Box<crate::wind_gpu::WindGrid>>,
    /// Advance and draw the GPU wind particles this frame.
    pub wind: Option<crate::wind_gpu::Frame>,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TileVertex {
    world: [f32; 2],
    uv: [f32; 2],
    transform: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct RadarVertex {
    world: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CameraUniform {
    center: [f32; 2],
    scale: [f32; 2],
    world_per_pixel: f32,
    road_scale: f32,
    // WebGL uniform bindings must occupy a multiple of 16 bytes.
    _pad: [f32; 2],
    globe: [f32; 4],
    planar: [f32; 4],
}

struct TileGpu {
    _tex: wgpu::Texture,
    bind_group: wgpu::BindGroup,
}

struct RadarGpu {
    tex: wgpu::Texture,
    smooth: Option<wgpu::Texture>,
    smooth_dirty: bool,
    flag: wgpu::Texture,
    lut: wgpu::Texture,
    uni: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    /// (gate_count, az_bins) and the precipitation-flag grid size, so a later upload of the same
    /// shape writes into these textures instead of building new ones with a new bind group.
    dims: (u32, u32),
    flag_dims: (u32, u32),
    precision: bool,
    lut_width: u32,
    gate_bytes: Vec<u8>,
    flag_bytes: Vec<u8>,
}

const RADAR_GPU_CACHE_ENTRIES: usize = 24;
const RADAR_GPU_CACHE_BYTES: usize = 32 * 1024 * 1024;

impl RadarGpu {
    fn shadow_bytes(&self) -> usize {
        self.gate_bytes.len() + self.flag_bytes.len() + self.lut_width as usize * 12
            + self.smooth.as_ref().map_or(0, |_| self.dims.0 as usize * self.dims.1 as usize * 4)
    }

    fn matches_upload(&self, r: &RadarUpload, gates: &[u8], flags: &[u8]) -> bool {
        self.dims == (r.gate_count, r.az_bins)
            && self.flag_dims == flag_dims(r)
            && self.precision == r.data16.is_some()
            && self.lut_width as usize * 12 == r.lut.len()
            && self.gate_bytes == gates
            && self.flag_bytes == flags
    }
}

struct OverlayGpu {
    vbuf: wgpu::Buffer,
    ibuf: wgpu::Buffer,
    index_count: u32,
}

struct VectorGpu {
    base: OverlayGpu,
    foreground: Option<OverlayGpu>,
}

struct MrmsGpu {
    _tex: wgpu::Texture,
    _lut: wgpu::Texture,
    /// Kept (not `_`-prefixed) so `prepare` can rewrite the opacity word each frame.
    uni: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    vbuf: wgpu::Buffer,
}

/// Per-pane GPU state: its own camera uniform, radar sweep, tile/radar quad buffers, and the
/// draw lists staged during `prepare` and consumed during `paint`. (All `prepare`s run before
/// any `paint`, so shared per-frame buffers would clobber across panes — hence per-pane.)
type TileQuadsKey = (u64, u64, u32, u32, u32, u32, usize, bool, [u32; 4]);

struct PaneGpu {
    id: u32,
    upload_token: u64,
    camera_buf: wgpu::Buffer,
    camera_bg: wgpu::BindGroup,
    tile_vbuf: wgpu::Buffer,
    tile_capacity: u64,
    radar_vbuf: wgpu::Buffer,
    radar: Option<RadarGpu>,
    radar_cache: VecDeque<RadarGpu>,
    surface: Option<radar_surface::Target>,
    /// Ids of the tiles this frame's quads draw, in quad order. Not the same as the visible list:
    /// a missing tile is stood in for by resident children or an ancestor.
    frame_visible: Vec<(TileKey, std::ops::Range<u32>)>,
    /// What the tile quads in `tile_vbuf` were built from: tile-cache generation, basemap, camera
    /// and visible count. Unchanged means the quads are still the right ones, so a still map
    /// re-uploads nothing.
    // ponytail: the visible *count* rather than the list — camera plus generation already decide
    // which tiles are asked for; compare the ids if a case ever shows a stale quad.
    quads_key: Option<TileQuadsKey>,
    frame_visible_vector: Vec<TileId>,
    vector_over_raster: bool,
    frame_draw_radar: bool,
    frame_draw_wind: bool,
    frame_draw_overlay: bool,
    /// Field layers this pane draws this frame. Per-pane, not shared: two panes are how you look
    /// at two fields at once.
    field_draws: Vec<FieldLayer>,
}

impl PaneGpu {
    /// Keep recent sweeps for animation; return an evicted texture that can be rewritten.
    fn cache_radar(&mut self, radar: RadarGpu) -> Option<RadarGpu> {
        self.radar_cache.push_back(radar);
        let mut bytes: usize = self.radar_cache.iter().map(RadarGpu::shadow_bytes).sum();
        let mut reusable = None;
        while self.radar_cache.len() > RADAR_GPU_CACHE_ENTRIES || bytes > RADAR_GPU_CACHE_BYTES {
            let old = self.radar_cache.pop_front().unwrap();
            bytes = bytes.saturating_sub(old.shadow_bytes());
            reusable = Some(old);
        }
        reusable
    }
}

/// Long-lived GPU resources, stored in egui's `CallbackResources` type-map.
pub struct RenderResources {
    tile_pipeline: wgpu::RenderPipeline,
    radar_pipeline: wgpu::RenderPipeline,
    smooth_pipeline: wgpu::RenderPipeline,
    smooth_empty: wgpu::BindGroup,
    surface_renderer: radar_surface::Renderer,
    overlay_pipeline: wgpu::RenderPipeline,
    mrms_pipeline: wgpu::RenderPipeline,
    camera_bgl: wgpu::BindGroupLayout,
    tile_bgl: wgpu::BindGroupLayout,
    radar_bgl: wgpu::BindGroupLayout,
    mrms_bgl: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    // Shared across panes: tile image cache, vector tile geometry, and the world-space overlay
    // (severe weather + placefiles) — the overlay is camera-independent, drawn per-pane camera.
    //
    // Plain maps, bounded from the CPU side: `TileManager` owns the eviction policy and hands us
    // the ids to free (`drop_tiles`). Evicting here instead was a bug — the manager went on
    // believing an evicted tile was still uploaded and never re-sent it, so zooming back to an
    // area left black squares where those tiles used to be.
    tiles: HashMap<TileKey, TileGpu>,
    vector_tiles: HashMap<(u32, TileId), VectorGpu>,
    overlay: Option<OverlayGpu>,
    fields: HashMap<(Option<u32>, FieldLayer), MrmsGpu>,
    // One entry per live pane.
    panes: HashMap<u32, PaneGpu>,
    /// Bumped whenever the shared tile cache gains or loses a texture, so a pane can tell that
    /// its quad list is still current.
    tiles_gen: u64,
    gpu_timers: HashMap<u32, crate::gpu_metrics::Timer>,
    /// GPU wind particles, built on first use. `None` when the CPU path is in charge.
    wind: HashMap<u32, crate::wind_gpu::WindGpu>,
    wind_grid: Option<Box<crate::wind_gpu::WindGrid>>,
    target_format: wgpu::TextureFormat,
}

impl RenderResources {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        crate::gpu_metrics::set_uploads([0; 4]);
        let tile_shader = device.create_shader_module(wgpu::include_wgsl!("../shaders/tiles.wgsl"));
        let radar_shader =
            device.create_shader_module(wgpu::include_wgsl!("../shaders/radar.wgsl"));
        let overlay_shader =
            device.create_shader_module(wgpu::include_wgsl!("../shaders/overlay.wgsl"));
        let mrms_shader = device.create_shader_module(wgpu::include_wgsl!("../shaders/mrms.wgsl"));

        // group 0: camera uniform (shared).
        let camera_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(std::mem::size_of::<CameraUniform>() as u64),
                },
                count: None,
            }],
        });
        // group 1 (tiles): texture + sampler.
        let tile_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tile_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        // group 1 (radar): uniform + u32 texture.
        let radar_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("radar_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(80 + 720 * 4),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // The precipitation-type grid. Always bound — a binding cannot be conditional —
                // and 1×1 when the tint is off.
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

        // Integer float-bit storage keeps precision without optional float-attachment support.
        let smooth_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("radar smoothing"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &radar_shader,
                entry_point: Some("vs_smooth"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &radar_shader,
                entry_point: Some("fs_smooth"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::R32Uint,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let smooth_empty = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("radar smoothing empty camera"),
            layout: &smooth_pipeline.get_bind_group_layout(0),
            entries: &[],
        });

        let blend = Some(wgpu::BlendState::ALPHA_BLENDING);
        let color_target = wgpu::ColorTargetState {
            format: target_format,
            blend,
            write_mask: wgpu::ColorWrites::ALL,
        };

        let tile_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("tile_layout"),
            bind_group_layouts: &[Some(&camera_bgl), Some(&tile_bgl)],
            immediate_size: 0,
        });
        let tile_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("tile_pipeline"),
            layout: Some(&tile_layout),
            vertex: wgpu::VertexState {
                module: &tile_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<TileVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &tile_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(color_target.clone())],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let radar_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("radar_layout"),
            bind_group_layouts: &[Some(&camera_bgl), Some(&radar_bgl)],
            immediate_size: 0,
        });
        let radar_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("radar_pipeline"),
            layout: Some(&radar_layout),
            vertex: wgpu::VertexState {
                module: &radar_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<RadarVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &radar_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(color_target)],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // The mosaic shares radar.wgsl's shape — uniform, index grid, LUT — but not its layout:
        // radar_bgl has since grown a precipitation-type texture and an 80-byte uniform, and
        // borrowing it meant the mosaic had to carry a dummy texture and dead padding that
        // nobody remembered to keep in step. It got out of step, and every MRMS draw died in
        // bind-group validation. Its own three-binding layout cannot drift.
        let mrms_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mrms_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(48),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let mrms_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mrms_layout"),
            bind_group_layouts: &[Some(&camera_bgl), Some(&mrms_bgl)],
            immediate_size: 0,
        });
        let mrms_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("mrms_pipeline"),
            layout: Some(&mrms_layout),
            vertex: wgpu::VertexState {
                module: &mrms_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<RadarVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &mrms_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // Overlay pipeline: camera-transformed colored triangles (group 0 only).
        let overlay_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("overlay_layout"),
            bind_group_layouts: &[Some(&camera_bgl)],
            immediate_size: 0,
        });
        let overlay_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("overlay_pipeline"),
            layout: Some(&overlay_layout),
            vertex: wgpu::VertexState {
                module: &overlay_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<OverlayVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4, 2 => Float32x3],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &overlay_shader,
                entry_point: Some(if target_format.is_srgb() {
                    "fs_main"
                } else {
                    "fs_main_gamma"
                }),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("tile_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let surface_renderer = radar_surface::Renderer::new(device, target_format, &camera_bgl, &radar_bgl, &radar_shader);
        Self {
            surface_renderer,
            tile_pipeline,
            radar_pipeline,
            smooth_pipeline,
            smooth_empty,
            overlay_pipeline,
            mrms_pipeline,
            camera_bgl,
            tile_bgl,
            radar_bgl,
            mrms_bgl,
            sampler,
            tiles: HashMap::new(),
            vector_tiles: HashMap::new(),
            overlay: None,
            fields: HashMap::new(),
            panes: HashMap::new(),
            tiles_gen: 0,
            gpu_timers: HashMap::new(),
            wind: HashMap::new(),
            wind_grid: None,
            target_format,
        }
    }

    /// Get or create the per-pane GPU state for `id`.
    fn pane_mut(&mut self, device: &wgpu::Device, id: u32) -> &mut PaneGpu {
        let camera_bgl = &self.camera_bgl;
        self.panes.entry(id).or_insert_with(|| {
            let camera_buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("camera_buf"),
                size: std::mem::size_of::<CameraUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let camera_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("camera_bg"),
                layout: camera_bgl,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: camera_buf.as_entire_binding(),
                }],
            });
            let tile_vbuf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("tile_vbuf"),
                size: 512 * 6 * std::mem::size_of::<TileVertex>() as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let radar_vbuf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("radar_vbuf"),
                size: 6 * std::mem::size_of::<RadarVertex>() as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            PaneGpu {
                id,
                upload_token: 0,
                camera_buf,
                camera_bg,
                tile_vbuf,
                tile_capacity: 512 * 6,
                radar_vbuf,
                radar: None,
                radar_cache: VecDeque::new(),
                surface: None,
                frame_visible: Vec::new(),
                quads_key: None,
                frame_visible_vector: Vec::new(),
                vector_over_raster: false,
                frame_draw_radar: false,
                frame_draw_wind: false,
                frame_draw_overlay: false,
                field_draws: Vec::new(),
            }
        })
    }

    fn upload_tile(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, t: &PendingTile) {
        let size = wgpu::Extent3d {
            width: t.width,
            height: t.height,
            depth_or_array_layers: 1,
        };
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("tile_tex"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &t.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * t.width),
                rows_per_image: Some(t.height),
            },
            size,
        );
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tile_bg"),
            layout: &self.tile_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        self.tiles.insert(
            (t.style, t.id),
            TileGpu {
                _tex: tex,
                bind_group,
            },
        );
    }

    /// Upload one sweep for a pane, reusing `existing` when it is the same shape.
    ///
    /// Returns `None` only for a LUT-only upload with nothing retained to write into — which
    /// the app does not ask for (it only sends one when the same key is already shown), but is
    /// answered by leaving the pane's radar alone rather than by a panic.
    fn build_radar(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        r: &RadarUpload,
        existing: Option<RadarGpu>,
        cached_data: bool,
    ) -> Option<RadarGpu> {
        let flag_dims = flag_dims(r);
        let precision = r.data16.is_some();
        let bytes_per_gate = if precision { 2 } else { 1 };
        let gate_data: &[u8] = r.data16.as_ref().map_or(r.data.as_slice(), |d| bytemuck::cast_slice(d));
        let lut_width = (r.lut.len() / (3 * 4)) as u32;
        // Same shape: write into the retained textures and keep the bind group. Dimensions are
        // the only thing a bind group depends on here, so nothing else can go stale.
        if let Some(mut g) = existing {
            if g.dims == (r.gate_count, r.az_bins) && g.flag_dims == flag_dims && g.precision == precision && g.lut_width == lut_width {
                if !r.lut_only && !cached_data {
                    g.smooth_dirty |= g.gate_bytes != gate_data;
                    write_texture_changes(queue, &g.tex, g.dims, bytes_per_gate, &mut g.gate_bytes, gate_data);
                    write_texture_changes(queue, &g.flag, g.flag_dims, 1, &mut g.flag_bytes, &flag_bytes(r, g.flag_dims));
                }
                queue.write_buffer(&g.uni, 0, bytemuck::cast_slice(&r.uniform));
                queue.write_buffer(&g.uni, 80, bytemuck::cast_slice(&r.sector_opacity.unwrap_or([1.0; 720])));
                write_lut(queue, &g.lut, &r.lut);
                if r.uniform[19] > 0.5 { self.smooth_radar(device, queue, &mut g); }
                return Some(g);
            }
        }
        if r.lut_only {
            return None;
        }
        wxdata::stats::bump(wxdata::stats::Counter::RadarTexturesBuilt);
        let size = wgpu::Extent3d {
            width: r.gate_count,
            height: r.az_bins,
            depth_or_array_layers: 1,
        };
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("sweep_tex"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: if precision { wgpu::TextureFormat::R16Uint } else { wgpu::TextureFormat::R8Uint },
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        wxdata::stats::add(wxdata::stats::Counter::RadarUploadBytes, gate_data.len() as u64);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            gate_data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(r.gate_count * bytes_per_gate),
                rows_per_image: Some(r.az_bins),
            },
            size,
        );
        let mut uniform = r.uniform.to_vec();
        uniform.extend_from_slice(&r.sector_opacity.unwrap_or([1.0; 720]));
        let uni = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("radar_uniform"),
            contents: bytemuck::cast_slice(&uniform),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // 256×3 color LUT, indexed by the sweep u8 across and the precipitation type down.
        let lut_size = wgpu::Extent3d {
            width: lut_width,
            height: 3,
            depth_or_array_layers: 1,
        };
        let lut_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("radar_lut"),
            size: lut_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        wxdata::stats::add(wxdata::stats::Counter::RadarUploadBytes, r.lut.len() as u64);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &lut_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &r.lut,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(lut_width * 4),
                rows_per_image: Some(3),
            },
            lut_size,
        );

        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        let lut_view = lut_tex.create_view(&wgpu::TextureViewDescriptor::default());

        // Precipitation-type grid. One byte per cell; 1×1 of zeroes when the tint is off, which
        // the shader never reads because the tint flag in the uniform is zero too.
        let (fnx, fny) = flag_dims;
        let flag_data = flag_bytes(r, flag_dims);
        let flag_size = wgpu::Extent3d {
            width: fnx,
            height: fny,
            depth_or_array_layers: 1,
        };
        let flag_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("radar_precip_flag"),
            size: flag_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        wxdata::stats::add(wxdata::stats::Counter::RadarUploadBytes, flag_data.len() as u64);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &flag_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &flag_data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(fnx),
                rows_per_image: Some(fny),
            },
            flag_size,
        );
        let flag_view = flag_tex.create_view(&wgpu::TextureViewDescriptor::default());

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("radar_bg"),
            layout: &self.radar_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uni.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&lut_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&flag_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
            ],
        });
        let mut gpu = RadarGpu {
            tex,
            smooth: None,
            smooth_dirty: true,
            flag: flag_tex,
            lut: lut_tex,
            uni,
            bind_group,
            dims: (r.gate_count, r.az_bins),
            flag_dims,
            precision,
            lut_width,
            gate_bytes: gate_data.to_vec(),
            flag_bytes: flag_bytes(r, flag_dims),
        };
        if r.uniform[19] > 0.5 { self.smooth_radar(device, queue, &mut gpu); }
        Some(gpu)
    }

    /// Cache only data-dependent filtering; palette, camera and storm motion stay live.
    fn smooth_radar(&self, device: &wgpu::Device, queue: &wgpu::Queue, radar: &mut RadarGpu) {
        if radar.smooth.is_none() {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("cached radar smoothing"),
                size: wgpu::Extent3d { width: radar.dims.0, height: radar.dims.1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R32Uint,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let sweep = radar.tex.create_view(&Default::default());
            let lut = radar.lut.create_view(&Default::default());
            let flag = radar.flag.create_view(&Default::default());
            let smooth = texture.create_view(&Default::default());
            radar.bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("radar with cached smoothing"),
                layout: &self.radar_bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: radar.uni.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&sweep) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&lut) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&flag) },
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&smooth) },
                ],
            });
            radar.smooth = Some(texture);
            radar.smooth_dirty = true;
        }
        if !radar.smooth_dirty { return; }
        let sweep = radar.tex.create_view(&Default::default());
        let input = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("radar smoothing input"),
            layout: &self.smooth_pipeline.get_bind_group_layout(1),
            entries: &[wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&sweep) }],
        });
        let target = radar.smooth.as_ref().unwrap().create_view(&Default::default());
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("cache radar smoothing") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cache radar smoothing"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.smooth_pipeline);
            pass.set_bind_group(0, &self.smooth_empty, &[]);
            pass.set_bind_group(1, &input, &[]);
            pass.draw(0..3, 0..1);
        }
        queue.submit(Some(encoder.finish()));
        radar.smooth_dirty = false;
    }

    /// Upload camera/tiles/radar for `cb` and stage its pane's draw list. Shared caches (tiles,
    /// vector tiles, overlay) update once; per-pane state (camera, radar, tile quads) is keyed
    /// by `cb.pane`. Shared by the egui callback and the headless renderer.
    /// Advance the GPU wind particles for this pane, building the layer on first use. Separate
    /// from `upload_frame` because it needs an encoder of its own — the advection is a render
    /// pass, and egui's `prepare` is the one place a callback may record one.
    fn step_wind(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        cb: &MapCallback,
    ) {
        if cb.wind.is_none() && cb.wind_upload.is_none() {
            return;
        }
        if let Some(up) = &cb.wind_upload {
            for wind in self.wind.values_mut() {
                wind.upload_grid(queue, up);
            }
            self.wind_grid = Some(up.clone());
        }
        let format = self.target_format;
        if !self.wind.contains_key(&cb.pane) {
            let mut wind = crate::wind_gpu::WindGpu::new(device, queue, format);
            if let Some(grid) = &self.wind_grid {
                wind.upload_grid(queue, grid);
            }
            self.wind.insert(cb.pane, wind);
        }
        if let Some(frame) = &cb.wind {
            self.wind
                .get_mut(&cb.pane)
                .unwrap()
                .step(device, queue, encoder, cb.pane, frame);
        }
    }

    /// The tile quads for this frame, and the tile ids they draw in quad order.
    fn tile_verts(
        &self,
        cb: &MapCallback,
    ) -> (Vec<TileVertex>, Vec<(TileKey, std::ops::Range<u32>)>) {
        wxdata::stats::bump(wxdata::stats::Counter::TileQuadsBuilt);
        let mut tverts: Vec<TileVertex> = Vec::new();
        let mut visible = Vec::new();
        for v in &cb.visible {
            for (id, wmin, wmax, uvmin, uvmax) in raster_tile_quads(cb.basemap_key, v, |id| self.tiles.contains_key(id)) {
                let steps = tile_subdivisions(cb.globe, cb.world_per_pixel, wmin, wmax);
                let first = tverts.len() as u32;
                if tverts.len() as u64 + 6 * steps * steps > MAX_TILE_VERTS {
                    break;
                }
                let sx = (uvmax[0] - uvmin[0]) / (wmax[0] - wmin[0]);
                let sy = (uvmax[1] - uvmin[1]) / (wmax[1] - wmin[1]);
                let transform = [wmin[0] - uvmin[0] / sx, wmin[1] - uvmin[1] / sy, sx, sy];
                for y in 0..steps {
                    for x in 0..steps {
                        for (dx, dy) in [(0, 0), (1, 0), (1, 1), (0, 0), (1, 1), (0, 1)] {
                            let u = (x + dx) as f32 / steps as f32;
                            let v = (y + dy) as f32 / steps as f32;
                            tverts.push(TileVertex {
                                world: [
                                    wmin[0] + u * (wmax[0] - wmin[0]),
                                    wmin[1] + v * (wmax[1] - wmin[1]),
                                ],
                                uv: [
                                    uvmin[0] + u * (uvmax[0] - uvmin[0]),
                                    uvmin[1] + v * (uvmax[1] - uvmin[1]),
                                ],
                                transform,
                            });
                        }
                    }
                }
                visible.push((id, first..tverts.len() as u32));
            }
        }
        (tverts, visible)
    }

    fn upload_frame(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, cb: &MapCallback) {
        // --- Shared caches ---
        if cb.clear_tiles {
            self.tiles.clear();
        }
        if cb.clear_vector {
            self.vector_tiles.retain(|(pane, _), _| *pane != cb.pane);
        }
        for id in &cb.drop_vector_tiles {
            self.vector_tiles.remove(&(cb.pane, *id));
        }
        for t in &cb.new_vector_tiles {
            self.upload_vector_tile(device, cb.pane, t);
        }
        for t in &cb.new_tiles {
            self.upload_tile(device, queue, t);
        }
        if let Some(o) = &cb.overlay_upload {
            self.upload_overlay(device, o);
        }
        // Grow to fit the frame before touching anything. A zoomed-out view can need more tiles
        // than the resting cap, and promoting a visible tile into a full cache evicts another
        // visible one — which showed up as a band of basemap with the rest of the map bare.
        // The cap is a resting size, not a limit on what one frame may hold.
        for id in &cb.drop_tiles {
            self.tiles.remove(id);
        }
        if cb.clear_tiles || !cb.new_tiles.is_empty() || !cb.drop_tiles.is_empty() {
            self.tiles_gen = self.tiles_gen.wrapping_add(1);
        }
        // Touch everything this frame draws so the LRU evicts what is off screen, not what is in
        // front of the user. Visible entries can't be evicted mid-frame: the caches only shrink
        // here, before any drawing.
        for layer in &cb.drop_fields {
            self.fields.remove(&(layer.is_pane_field().then_some(cb.pane), *layer));
        }
        for (layer, up) in &cb.field_uploads {
            let gpu = self.build_field_layer(device, queue, up);
            self.fields.insert((layer.is_pane_field().then_some(cb.pane), *layer), gpu);
        }
        // Draw only the requested layers that actually have GPU data. Opacity rides in the grid
        // uniform's first pad word, so it costs one 4-byte write per drawn layer — no LUT re-bake.
        let mut field_draws = Vec::new();
        for (layer, opacity) in &cb.field_draws {
            if let Some(f) = self.fields.get(&(layer.is_pane_field().then_some(cb.pane), *layer)) {
                queue.write_buffer(&f.uni, 24, &opacity.to_le_bytes());
                field_draws.push(*layer);
            }
        }

        // --- Per-pane state ---
        let new_radar = match cb.radar_upload.as_ref() {
            Some(r) => {
                let (existing, cached_data) = if let Some(pane) = self.panes.get_mut(&cb.pane) {
                    if !r.cache_loop {
                        pane.radar_cache.clear();
                        (pane.radar.take(), false)
                    } else if r.lut_only {
                        (pane.radar.take(), false)
                    } else {
                        let gates: &[u8] = r.data16.as_ref().map_or(r.data.as_slice(), |d| bytemuck::cast_slice(d));
                        let flags = flag_bytes(r, flag_dims(r));
                        if pane.radar.as_ref().is_some_and(|gpu| gpu.matches_upload(r, gates, &flags)) {
                            (pane.radar.take(), true)
                        } else {
                            let hit = pane.radar_cache.iter()
                                .position(|gpu| gpu.matches_upload(r, gates, &flags))
                                .and_then(|idx| pane.radar_cache.remove(idx));
                            let recycled = pane.radar.take().and_then(|gpu| pane.cache_radar(gpu));
                            let cached_data = hit.is_some();
                            (hit.or(recycled), cached_data)
                        }
                    }
                } else {
                    (None, false)
                };
                if cached_data {
                    wxdata::stats::bump(wxdata::stats::Counter::RadarCacheHits);
                }
                self.build_radar(device, queue, r, existing, cached_data)
            }
            None => None,
        };
        // Build the tile quad list against the shared tile cache before mutably borrowing the pane
        // — a tile with no texture yet borrows one from the tiles around it (see `tile_quads`).
        // Skipped outright when nothing it depends on moved: a still map rebuilt up to 512 tiles
        // worth of vertices and re-uploaded them every heartbeat frame.
        let quads_key = (
            self.tiles_gen,
            cb.basemap_key,
            cb.camera_center[0].to_bits(),
            cb.camera_center[1].to_bits(),
            cb.camera_scale[0].to_bits(),
            cb.camera_scale[1].to_bits(),
            cb.visible.len(),
            cb.globe[2] != 0.0,
            cb.planar.map(f32::to_bits),
        );
        let quads = (self.panes.get(&cb.pane).map(|p| p.quads_key) != Some(Some(quads_key)))
            .then(|| self.tile_verts(cb));
        let visible_vector = vector_draw_tiles(&cb.visible_vector, self.vector_tiles.keys().filter_map(|(pane, id)| (*pane == cb.pane).then_some(*id)));
        let overlay_present = self.overlay.is_some();

        let pane = self.pane_mut(device, cb.pane);
        queue.write_buffer(
            &pane.camera_buf,
            0,
            bytemuck::bytes_of(&CameraUniform {
                center: cb.camera_center,
                scale: cb.camera_scale,
                world_per_pixel: cb.world_per_pixel,
                road_scale: crate::basemap_style::road_scale(
                    -(256.0 * cb.world_per_pixel as f64).log2(),
                ) as f32,
                _pad: [0.0; 2],
                globe: cb.globe,
                planar: cb.planar,
            }),
        );
        if let Some(r) = &cb.radar_upload {
            let [x0, y0] = r.world_min;
            let [x1, y1] = r.world_max;
            let verts = [
                RadarVertex { world: [x0, y0] },
                RadarVertex { world: [x1, y0] },
                RadarVertex { world: [x1, y1] },
                RadarVertex { world: [x0, y0] },
                RadarVertex { world: [x1, y1] },
                RadarVertex { world: [x0, y1] },
            ];
            queue.write_buffer(&pane.radar_vbuf, 0, bytemuck::cast_slice(&verts));
        }
        if let Some(radar) = new_radar {
            pane.radar = Some(radar);
            pane.upload_token = cb.radar_upload.as_ref().map_or(0, |upload| upload.upload_token);
        }
        if let Some((tverts, visible)) = quads {
            if tverts.len() as u64 > pane.tile_capacity {
                pane.tile_capacity = (tverts.len() as u64).next_power_of_two();
                pane.tile_vbuf = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("globe_tile_vertices"),
                    size: pane.tile_capacity * std::mem::size_of::<TileVertex>() as u64,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
            }
            if !tverts.is_empty() {
                queue.write_buffer(&pane.tile_vbuf, 0, bytemuck::cast_slice(&tverts));
            }
            pane.frame_visible = visible;
            pane.quads_key = Some(quads_key);
        }
        pane.frame_visible_vector = visible_vector;
        pane.vector_over_raster = cb.vector_over_raster;
        pane.frame_draw_wind = cb.wind.is_some();
        pane.frame_draw_radar = cb.draw_radar && pane.radar.is_some();
        pane.frame_draw_overlay = cb.draw_overlay && overlay_present;
        pane.field_draws = field_draws;
    }

    fn upload_overlay(&mut self, device: &wgpu::Device, o: &OverlayUpload) {
        if o.indices.is_empty() {
            self.overlay = None;
            return;
        }
        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("overlay_vbuf"),
            contents: bytemuck::cast_slice(&o.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let ibuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("overlay_ibuf"),
            contents: bytemuck::cast_slice(&o.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        self.overlay = Some(OverlayGpu {
            vbuf,
            ibuf,
            index_count: o.indices.len() as u32,
        });
    }

    /// Build a national field-layer (MRMS mosaic or lightning): R8 index texture + LUT texture +
    /// grid uniform + full-grid quad. Shared by both layers; they differ only in data + LUT.
    fn build_field_layer(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        m: &MrmsUpload,
    ) -> MrmsGpu {
        let size = wgpu::Extent3d {
            width: m.nx,
            height: m.ny,
            depth_or_array_layers: 1,
        };
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("mrms_tex"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: if m.rgba {
                wgpu::TextureFormat::Rgba8Uint
            } else {
                wgpu::TextureFormat::R8Uint
            },
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &m.data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(m.nx * if m.rgba { 4 } else { 1 }),
                rows_per_image: Some(m.ny),
            },
            size,
        );
        let uni = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mrms_uniform"),
            contents: bytemuck::cast_slice(&m.uniform),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let lut_size = wgpu::Extent3d {
            width: 256,
            height: 1,
            depth_or_array_layers: 1,
        };
        let lut_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("mrms_lut"),
            size: lut_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &lut_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &m.lut,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256 * 4),
                rows_per_image: Some(1),
            },
            lut_size,
        );
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        let lut_view = lut_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mrms_bg"),
            layout: &self.mrms_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uni.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&lut_view),
                },
            ],
        });
        let [x0, y0] = m.world_min;
        let [x1, y1] = m.world_max;
        let verts = [
            RadarVertex { world: [x0, y0] },
            RadarVertex { world: [x1, y0] },
            RadarVertex { world: [x1, y1] },
            RadarVertex { world: [x0, y0] },
            RadarVertex { world: [x1, y1] },
            RadarVertex { world: [x0, y1] },
        ];
        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mrms_vbuf"),
            contents: bytemuck::cast_slice(&verts),
            usage: wgpu::BufferUsages::VERTEX,
        });
        MrmsGpu {
            _tex: tex,
            _lut: lut_tex,
            uni,
            bind_group,
            vbuf,
        }
    }

    fn upload_vector_tile(&mut self, device: &wgpu::Device, pane: u32, t: &PendingVectorTile) {
        if t.indices.is_empty() {
            self.vector_tiles.remove(&(pane, t.id));
            return;
        }
        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("vector_vbuf"),
            contents: bytemuck::cast_slice(&t.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let ibuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("vector_ibuf"),
            contents: bytemuck::cast_slice(&t.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let foreground = (!t.foreground_indices.is_empty()).then(|| OverlayGpu {
            vbuf: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("vector_foreground_vbuf"),
                contents: bytemuck::cast_slice(&t.foreground_vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            ibuf: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("vector_foreground_ibuf"),
                contents: bytemuck::cast_slice(&t.foreground_indices),
                usage: wgpu::BufferUsages::INDEX,
            }),
            index_count: t.foreground_indices.len() as u32,
        });
        self.vector_tiles.insert((pane, t.id), VectorGpu {
            base: OverlayGpu { vbuf, ibuf, index_count: t.indices.len() as u32 },
            foreground,
        });
    }

    /// Paint the active field layers in the requested band (below/above the radar), in the fixed
    /// bottom-to-top order, using this pane's camera.
    fn draw_fields(&self, pane: &PaneGpu, pass: &mut wgpu::RenderPass<'_>, below: bool) {
        let cam = &pane.camera_bg;
        for layer in FieldLayer::draw_order() {
            if layer.below_radar() != below || !pane.field_draws.contains(&layer) {
                continue;
            }
            if let Some(f) = self.fields.get(&(layer.is_pane_field().then_some(pane.id), layer)) {
                pass.set_pipeline(&self.mrms_pipeline);
                pass.set_bind_group(0, cam, &[]);
                pass.set_bind_group(1, &f.bind_group, &[]);
                pass.set_vertex_buffer(0, f.vbuf.slice(..));
                pass.draw(0..6, 0..1);
            }
        }
    }

    /// Record one pane's draws (vector basemap → raster tiles → radar → overlay), all using
    /// that pane's camera bind group.
    fn record_pane(&self, id: u32, pass: &mut wgpu::RenderPass<'_>) {
        let Some(pane) = self.panes.get(&id) else {
            return;
        };
        let cam = &pane.camera_bg;
        // Vector basemap first (opaque, under everything) — unless it is the hybrid overlay, in
        // which case it is roads drawn *onto* the raster imagery and has to come after it.
        if !pane.vector_over_raster {
            self.draw_vector_basemap(pane, cam, pass);
        }
        pass.set_pipeline(&self.tile_pipeline);
        pass.set_bind_group(0, cam, &[]);
        pass.set_vertex_buffer(0, pane.tile_vbuf.slice(..));
        for (id, vertices) in &pane.frame_visible {
            if let Some(tile) = self.tiles.get(id) {
                pass.set_bind_group(1, &tile.bind_group, &[]);
                pass.draw(vertices.clone(), 0..1);
            }
        }
        if pane.vector_over_raster {
            self.draw_vector_basemap(pane, cam, pass);
        }
        // Field layers under the radar (national mosaic context).
        self.draw_fields(pane, pass, true);
        if pane.frame_draw_radar {
            if let Some(radar) = &pane.radar {
                if let Some(surface) = &pane.surface {
                    self.surface_renderer.draw(surface, pass);
                } else {
                    pass.set_pipeline(&self.radar_pipeline);
                    pass.set_bind_group(0, cam, &[]);
                    pass.set_bind_group(1, &radar.bind_group, &[]);
                    pass.set_vertex_buffer(0, pane.radar_vbuf.slice(..));
                    pass.draw(0..6, 0..1);
                }
            }
        }
        // All road strokes are drawn here above radar; labels and route shields follow in egui.
        self.draw_vector_foreground(pane, cam, pass);
        // Field layers over the radar (rotation/hail/shear/lightning signals).
        self.draw_fields(pane, pass, false);
        // Wind particles under the overlay, not over it: the CPU path paints in egui's own layer,
        // which is above everything, and a warning polygon disappearing under a particle trail
        // was the one complaint that layer ever attracted.
        if pane.frame_draw_wind {
            if let Some(wind) = self.wind.get(&id) { wind.draw(id, pass); }
        }
        if pane.frame_draw_overlay {
            if let Some(overlay) = &self.overlay {
                pass.set_pipeline(&self.overlay_pipeline);
                pass.set_bind_group(0, cam, &[]);
                pass.set_vertex_buffer(0, overlay.vbuf.slice(..));
                pass.set_index_buffer(overlay.ibuf.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..overlay.index_count, 0, 0..1);
            }
        }
    }

    /// Stage a pane's uploads (mirrors the egui `prepare` phase). Public for the headless harness.
    /// Draw one pane's resident vector-basemap tiles. Extracted so the caller can put it either
    /// side of the raster tiles (see [`MapCallback::vector_over_raster`]).
    fn draw_vector_basemap(
        &self,
        pane: &PaneGpu,
        cam: &wgpu::BindGroup,
        pass: &mut wgpu::RenderPass<'_>,
    ) {
        if pane.frame_visible_vector.is_empty() {
            return;
        }
        pass.set_pipeline(&self.overlay_pipeline);
        pass.set_bind_group(0, cam, &[]);
        for tid in &pane.frame_visible_vector {
            if let Some(t) = self.vector_tiles.get(&(pane.id, *tid)) {
                pass.set_vertex_buffer(0, t.base.vbuf.slice(..));
                pass.set_index_buffer(t.base.ibuf.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..t.base.index_count, 0, 0..1);
            }
        }
    }

    fn draw_vector_foreground(
        &self,
        pane: &PaneGpu,
        cam: &wgpu::BindGroup,
        pass: &mut wgpu::RenderPass<'_>,
    ) {
        pass.set_pipeline(&self.overlay_pipeline);
        pass.set_bind_group(0, cam, &[]);
        for tid in &pane.frame_visible_vector {
            let Some(t) = self.vector_tiles.get(&(pane.id, *tid)).and_then(|t| t.foreground.as_ref()) else {
                continue;
            };
            pass.set_vertex_buffer(0, t.vbuf.slice(..));
            pass.set_index_buffer(t.ibuf.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..t.index_count, 0, 0..1);
        }
    }

    pub fn prepare_pane(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, cb: &MapCallback) {
        if let Some(count) = cb.live_pane_count {
            self.panes.retain(|id, _| *id < count);
            self.wind.retain(|id, _| *id < count);
            self.gpu_timers.retain(|id, _| *id < count);
            self.vector_tiles.retain(|(id, _), _| *id < count);
            self.fields.retain(|(id, _), _| id.is_none_or(|id| id < count));
        }
        if let Some(visible) = cb.retain_only_pane {
            self.panes.retain(|id, _| *id == visible);
            self.wind.retain(|id, _| *id == visible);
            self.gpu_timers.retain(|id, _| *id == visible);
        }
        self.upload_frame(device, queue, cb);
        self.surface_renderer.prepare(device, queue, self.panes.get_mut(&cb.pane).unwrap(), cb);
        if let std::collections::hash_map::Entry::Vacant(entry) = self.gpu_timers.entry(cb.pane) {
            if let Some(timer) = crate::gpu_metrics::Timer::new(device) { entry.insert(timer); }
        }
        let (gpu, shadows) = self.memory_bytes();
        crate::gpu_metrics::set_memory(gpu, shadows);
        crate::gpu_metrics::set_uploads(self.upload_tokens());
    }

    pub fn upload_tokens(&self) -> [u64; 4] {
        std::array::from_fn(|id| self.panes.get(&(id as u32)).filter(|pane| pane.radar.is_some()).map_or(0, |pane| pane.upload_token))
    }

    /// Nominal resource sizes; excludes driver padding, framework targets and allocation overhead.
    pub fn memory_bytes(&self) -> (u64, u64) {
        let texture = |tex: &wgpu::Texture| {
            let size = tex.size();
            size.width as u64
                * size.height as u64
                * size.depth_or_array_layers as u64
                * u64::from(tex.format().block_copy_size(None).unwrap_or(4))
        };
        let overlay = |geometry: &OverlayGpu| geometry.vbuf.size() + geometry.ibuf.size();
        let mut gpu = self
            .tiles
            .values()
            .map(|tile| texture(&tile._tex))
            .sum::<u64>()
            + self
                .vector_tiles
                .values()
                .map(|tile| overlay(&tile.base) + tile.foreground.as_ref().map_or(0, overlay))
                .sum::<u64>()
            + self.overlay.as_ref().map_or(0, overlay)
            + self
                .fields
                .values()
                .map(|field| {
                    texture(&field._tex)
                        + texture(&field._lut)
                        + field.uni.size()
                        + field.vbuf.size()
                })
                .sum::<u64>()
            + self
                .wind
                .values()
                .map(|wind| wind.estimated_bytes())
                .sum::<u64>();
        gpu += self.gpu_timers.len() as u64 * 272;
        let mut shadows = 0;
        for pane in self.panes.values() {
            gpu += pane.camera_buf.size() + pane.tile_vbuf.size() + pane.radar_vbuf.size();
            if let Some(surface) = &pane.surface { gpu += surface.memory_bytes(); }
            for radar in pane.radar.iter().chain(pane.radar_cache.iter()) {
                gpu += radar.smooth.as_ref().map_or(0, texture) + texture(&radar.tex)
                    + texture(&radar.flag)
                    + texture(&radar.lut)
                    + radar.uni.size();
                shadows += (radar.gate_bytes.len() + radar.flag_bytes.len()) as u64;
            }
        }
        (gpu, shadows)
    }

    /// Draw a previously-prepared pane to `view` (mirrors the egui `paint` phase).
    pub fn draw_pane(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        view: &wgpu::TextureView,
        pane: u32,
        clear: wgpu::Color,
    ) {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("headless"),
        });
        if let Some(timer) = self.gpu_timers.get(&pane) { timer.collect_into(&mut encoder, queue, pane); }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("headless_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if let Some(timer) = self.gpu_timers.get(&pane) { timer.begin(&mut pass); }
            self.record_pane(pane, &mut pass);
            if let Some(timer) = self.gpu_timers.get(&pane) { timer.end(&mut pass); }
        }
        queue.submit(Some(encoder.finish()));
    }

    /// Render one frame to `view` without a window (used by the headless verify harness).
    pub fn render_once(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        view: &wgpu::TextureView,
        cb: &MapCallback,
        clear: wgpu::Color,
    ) {
        self.prepare_pane(device, queue, cb);
        self.draw_pane(device, queue, view, cb.pane, clear);
    }
}

impl egui_wgpu::CallbackTrait for MapCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        crate::prof_scope!("render prepare");
        let res: &mut RenderResources = resources.get_mut().unwrap();
        res.prepare_pane(device, queue, self);
        if let Some(timer) = res.gpu_timers.get(&self.pane) { timer.collect_into(_encoder, queue, self.pane); }
        res.step_wind(device, queue, _encoder, self);
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        crate::prof_scope!("render paint");
        let res: &RenderResources = resources.get().unwrap();
        if let Some(timer) = res.gpu_timers.get(&self.pane) { timer.begin(pass); }
        res.record_pane(self.pane, pass);
        if let Some(timer) = res.gpu_timers.get(&self.pane) { timer.end(pass); }
    }
}

/// Prefer exact imagery, then cached children; fill their missing quarters from an ancestor.
#[allow(clippy::type_complexity)] // one call site; a struct here would be ceremony
fn raster_tile_quads(
    style: u64,
    v: &VisibleTile,
    contains: impl Fn(&TileKey) -> bool,
) -> Vec<(TileKey, [f32; 2], [f32; 2], [f32; 2], [f32; 2])> {
    const FULL: ([f32; 2], [f32; 2]) = ([0.0, 0.0], [1.0, 1.0]);
    if contains(&(style, v.id)) {
        return vec![((style, v.id), v.world_min, v.world_max, FULL.0, FULL.1)];
    }
    let (z, x, y) = v.id;
    let [x0, y0] = v.world_min;
    let [x1, y1] = v.world_max;
    let (mx, my) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let mut kids: Vec<_> = [(0u32, 0u32), (1, 0), (0, 1), (1, 1)]
        .into_iter()
        .filter_map(|(dx, dy)| {
            let id = (style, (z + 1, x * 2 + dx, y * 2 + dy));
            contains(&id).then(|| {
                let (qx0, qx1) = if dx == 0 { (x0, mx) } else { (mx, x1) };
                let (qy0, qy1) = if dy == 0 { (y0, my) } else { (my, y1) };
                (id, [qx0, qy0], [qx1, qy1], FULL.0, FULL.1)
            })
        })
        .collect();
    if kids.len() == 4 {
        return kids;
    }
    for up in 1..=3u8 {
        if up > z {
            break;
        }
        let id = (style, (z - up, x >> up, y >> up));
        if contains(&id) {
            if kids.is_empty() {
                let (uv_min, uv_max) = ancestor_uv(x, y, up);
                return vec![(id, v.world_min, v.world_max, uv_min, uv_max)];
            }
            // Fill only missing quarters, so translucent imagery is never blended twice.
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (cx, cy) = (x * 2 + dx, y * 2 + dy);
                if !contains(&(style, (z + 1, cx, cy))) {
                    let (qx0, qx1) = if dx == 0 { (x0, mx) } else { (mx, x1) };
                    let (qy0, qy1) = if dy == 0 { (y0, my) } else { (my, y1) };
                    let (uv_min, uv_max) = ancestor_uv(cx, cy, up + 1);
                    kids.push((id, [qx0, qy0], [qx1, qy1], uv_min, uv_max));
                }
            }
            break;
        }
    }
    kids
}

/// Keep cached geography visible while a new zoom level loads. Coarse tiles draw first,
/// then finer fallbacks, then the requested tiles so current detail always wins.
pub(crate) fn vector_draw_tiles(visible: &[TileId], resident: impl Iterator<Item = TileId>) -> Vec<TileId> {
    if visible.is_empty() {
        return Vec::new();
    }
    let resident: Vec<_> = resident.collect();
    let mut fallback = Vec::new();
    let mut exact = Vec::new();
    for &(z, x, y) in visible {
        if resident.contains(&(z, x, y)) {
            exact.push((z, x, y));
            continue;
        }
        // Select the nearest ancestor and cached descendants in the same pass.
        let mut parent: Option<TileId> = None;
        for &(rz, rx, ry) in &resident {
            if rz < z && (x >> (z - rz), y >> (z - rz)) == (rx, ry)
                && parent.is_none_or(|p| rz > p.0) {
                parent = Some((rz, rx, ry));
            } else if rz > z && (rx >> (rz - z), ry >> (rz - z)) == (x, y) {
                fallback.push((rz, rx, ry));
            }
        }
        if let Some(parent) = parent {
            fallback.push(parent);
        }
    }
    fallback.sort_unstable();
    fallback.dedup();
    if fallback.is_empty() {
        return exact;
    }
    fallback.extend(exact);
    fallback
}

#[cfg(test)]
mod vector_fallback_tests {
    use super::vector_draw_tiles;

    #[test]
    fn zooming_both_ways_keeps_cached_tiles_and_current_detail_wins() {
        let parent = (4, 3, 5);
        let child = (5, 6, 10);
        let sibling = (5, 7, 10);
        let far = (5, 20, 20);
        let grandparent = (3, 1, 2);
        assert_eq!(vector_draw_tiles(&[child], [grandparent, parent].into_iter()), vec![parent]);
        assert_eq!(vector_draw_tiles(&[child], [parent, far].into_iter()), vec![parent]);
        assert_eq!(vector_draw_tiles(&[parent], [child, far].into_iter()), vec![child]);
        assert_eq!(vector_draw_tiles(&[child, sibling], [parent, child].into_iter()), vec![parent, child]);
        assert_eq!(vector_draw_tiles(&[child], [parent, child].into_iter()), vec![child]);
        assert!(vector_draw_tiles(&[child], [far].into_iter()).is_empty());
    }

    #[test]
    fn mixed_zoom_fallbacks_keep_order_and_nearest_parent_for_any_cache_order() {
        let grandparent = (3, 1, 2);
        let parent = (4, 3, 5);
        let missing = (5, 6, 10);
        let exact = (5, 7, 10);
        let child = (6, 12, 20);
        let mut resident = vec![grandparent, parent, exact, child, (5, 20, 20)];
        for _ in 0..resident.len() {
            assert_eq!(vector_draw_tiles(&[missing, exact], resident.iter().copied()),
                vec![parent, child, exact]);
            resident.reverse();
            assert_eq!(vector_draw_tiles(&[missing, exact], resident.iter().copied()),
                vec![parent, child, exact]);
            resident.reverse();
            resident.rotate_left(1);
        }
        assert!(vector_draw_tiles(&[missing], std::iter::empty()).is_empty());
        assert!(vector_draw_tiles(&[], resident.into_iter()).is_empty());
    }
}

/// Where tile `(x, y)` sits inside the ancestor `up` levels above it, in that ancestor's UV space.
/// `up = 1` gives one of four quadrants, `up = 2` one of sixteen, and so on.
fn ancestor_uv(x: u32, y: u32, up: u8) -> ([f32; 2], [f32; 2]) {
    let n = (1u32 << up) as f32;
    let fx = (x & ((1 << up) - 1)) as f32 / n;
    let fy = (y & ((1 << up) - 1)) as f32 / n;
    ([fx, fy], [fx + 1.0 / n, fy + 1.0 / n])
}

#[cfg(test)]
mod tests {
    use super::ancestor_uv;

    #[test]
    fn ancestor_uv_picks_the_right_quadrant() {
        // Direct parent: (x, y) = (3, 2) is the odd column, even row -> right/top quadrant.
        assert_eq!(ancestor_uv(3, 2, 1), ([0.5, 0.0], [1.0, 0.5]));
        assert_eq!(ancestor_uv(2, 3, 1), ([0.0, 0.5], [0.5, 1.0]));
        // Two levels up: sixteenths, and only the low bits matter.
        assert_eq!(ancestor_uv(0, 0, 2), ([0.0, 0.0], [0.25, 0.25]));
        assert_eq!(ancestor_uv(7, 5, 2), ([0.75, 0.25], [1.0, 0.5]));
        // Three levels up: the whole ancestor is still covered exactly.
        let (min, max) = ancestor_uv(7, 7, 3);
        assert_eq!(min, [0.875, 0.875]);
        assert_eq!(max, [1.0, 1.0]);
    }

    #[test]
    fn partial_raster_children_keep_complete_non_overlapping_coverage() {
        let tile = super::VisibleTile { id: (4, 3, 5), world_min: [0.0; 2], world_max: [1.0; 2] };
        let parent = (17, (3, 1, 2));
        for mask in 0..16 {
            let mut resident = std::collections::HashSet::from([parent]);
            for dy in 0..2 {
                for dx in 0..2 {
                    if mask & (1 << (dy * 2 + dx)) != 0 {
                        resident.insert((17, (5, 6 + dx, 10 + dy)));
                    }
                }
            }
            let quads = super::raster_tile_quads(17, &tile, |id| resident.contains(id));
            let area: f32 = quads.iter().map(|(_, min, max, _, _)| (max[0] - min[0]) * (max[1] - min[1])).sum();
            assert_eq!(area, 1.0, "partial child mask {mask} must cover the entire tile once");
            for dy in 0..2 {
                for dx in 0..2 {
                    let point = [0.25 + dx as f32 * 0.5, 0.25 + dy as f32 * 0.5];
                    let covering: Vec<_> = quads.iter().filter(|(_, min, max, _, _)|
                        (0..2).all(|i| min[i] <= point[i] && point[i] < max[i])).collect();
                    assert_eq!(covering.len(), 1);
                    let expected = if mask & (1 << (dy * 2 + dx)) != 0 { (17, (5, 6 + dx, 10 + dy)) } else { parent };
                    assert_eq!(covering[0].0, expected);
                    if expected == parent {
                        let (_, min, max, uvmin, uvmax) = covering[0];
                        let uv = std::array::from_fn::<_, 2, _>(|i|
                            uvmin[i] + (point[i] - min[i]) / (max[i] - min[i]) * (uvmax[i] - uvmin[i]));
                        assert_eq!(uv, [0.5 + point[0] * 0.5, 0.5 + point[1] * 0.5]);
                    }
                }
            }
        }
        assert!(super::raster_tile_quads(18, &tile, |id| *id == parent).is_empty());
        let exact = (17, tile.id);
        assert_eq!(super::raster_tile_quads(17, &tile, |id| *id == parent || *id == exact).len(), 1);
    }
}

/// The precipitation-flag grid size for an upload: the grid the uniform declares when the tint
/// is on and the bytes match it, else the 1×1 dummy every binding needs whether or not it is read.
fn flag_dims(r: &RadarUpload) -> (u32, u32) {
    let (nx, ny) = (r.uniform[11] as u32, r.uniform[12] as u32);
    match r.precip_flag.len() == (nx * ny) as usize && nx > 0 {
        true => (nx, ny),
        false => (1, 1),
    }
}

/// The precipitation-flag bytes to write for `dims` (the dummy grid is a single zero).
fn flag_bytes(r: &RadarUpload, dims: (u32, u32)) -> Vec<u8> {
    match dims == (1, 1) && r.precip_flag.len() != 1 {
        true => vec![0u8],
        false => r.precip_flag.clone(),
    }
}

/// Coalesce dirty radial rows. Dense or fragmented changes use one full write;
/// sparse updates preserve unchanged rows and avoid hundreds of tiny queue calls.
fn changed_rows(previous: &[u8], next: &[u8], width: usize) -> Vec<std::ops::Range<usize>> {
    if width == 0 || !next.len().is_multiple_of(width) { return Vec::new(); }
    let height = next.len() / width;
    if previous.len() != next.len() { return std::iter::once(0..height).collect(); }
    let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
    let mut changed = 0;
    for row in 0..height {
        let bytes = row * width..(row + 1) * width;
        if previous[bytes.clone()] != next[bytes] {
            changed += 1;
            if let Some(last) = ranges.last_mut().filter(|r| r.end == row) { last.end += 1; }
            else { ranges.push(row..row + 1); }
        }
    }
    if ranges.len() > 32 || changed * 2 > height { std::iter::once(0..height).collect() } else { ranges }
}

fn write_texture_changes(queue: &wgpu::Queue, tex: &wgpu::Texture, dims: (u32, u32), bytes_per_pixel: u32, previous: &mut Vec<u8>, next: &[u8]) {
    let (width, height) = dims;
    let row_bytes = width * bytes_per_pixel;
    if next.len() != row_bytes as usize * height as usize { return; }
    let ranges = changed_rows(previous, next, row_bytes as usize);
    if ranges.is_empty() { return; }
    for rows in ranges {
        let bytes = rows.start * row_bytes as usize..rows.end * row_bytes as usize;
        wxdata::stats::add(wxdata::stats::Counter::RadarUploadBytes, bytes.len() as u64);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d { x: 0, y: rows.start as u32, z: 0 }, aspect: wgpu::TextureAspect::All },
            &next[bytes],
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row_bytes), rows_per_image: Some(rows.len() as u32) },
            wgpu::Extent3d { width, height: rows.len() as u32, depth_or_array_layers: 1 },
        );
    }
    previous.clear();
    previous.extend_from_slice(next);
}

/// Write the 256×3 RGBA color LUT.
fn write_lut(queue: &wgpu::Queue, tex: &wgpu::Texture, lut: &[u8]) {
    wxdata::stats::add(wxdata::stats::Counter::RadarUploadBytes, lut.len() as u64);
    let width = (lut.len() / (3 * 4)) as u32;
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        lut,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(3),
        },
        wgpu::Extent3d {
            width,
            height: 3,
            depth_or_array_layers: 1,
        },
    );
}

/// Bound projected chord error to about one pixel, quantized to prevent geometry churn.
/// The hard 16 subdivision ceiling and MAX_TILE_VERTS keep global allocation bounded.
fn tile_subdivisions(globe: [f32; 4], world_per_pixel: f32, min: [f32; 2], max: [f32; 2]) -> u64 {
    if globe[2] == 0.0 { return 1; }
    let latitude = |y: f32| (std::f32::consts::PI * (1.0 - 2.0 * y)).sinh().atan();
    let angle = ((max[0] - min[0]).abs() * std::f32::consts::TAU).max((latitude(max[1]) - latitude(min[1])).abs());
    let radius = 1.0 / (world_per_pixel.max(1e-12) * globe[1].abs().max(0.05) * std::f32::consts::TAU);
    let needed = (radius * angle * angle / 8.0).sqrt().ceil() as u64;
    needed.clamp(2, 16).next_power_of_two().min(16)
}

#[cfg(test)]
mod dirty_radar_rows_tests {
    use super::changed_rows;
    #[test]
    fn globe_geometry_tracks_screen_error_and_stays_bounded() {
        let globe = [0.0, 1.0, 2.0, 2.0];
        assert_eq!(super::tile_subdivisions([0.0; 4], 0.001, [0.0; 2], [1.0; 2]), 1);
        let wide = super::tile_subdivisions(globe, 0.001, [0.0; 2], [0.5; 2]);
        let narrow = super::tile_subdivisions(globe, 0.001, [0.0; 2], [0.01; 2]);
        assert!(wide > narrow);
        assert_eq!(super::tile_subdivisions(globe, 1e-12, [0.0; 2], [1.0; 2]), 16);
        assert!(super::tile_subdivisions(globe, 0.0001, [0.0; 2], [0.1; 2]) >= super::tile_subdivisions(globe, 0.001, [0.0; 2], [0.1; 2]));
    }

    #[test]
    fn sparse_changes_include_cleared_gates_and_coalesce_neighbors() {
        let previous = vec![7; 40];
        let mut next = previous.clone();
        next[8] = 0; next[12] = 8; next[28] = 9;
        assert_eq!(changed_rows(&previous, &next, 4), vec![2..4, 7..8]);
        let mut reconstructed = previous.clone();
        for rows in changed_rows(&previous, &next, 4) {
            let bytes = rows.start * 4..rows.end * 4;
            reconstructed[bytes.clone()].copy_from_slice(&next[bytes]);
        }
        assert_eq!(reconstructed, next);
        assert!(changed_rows(&next, &next, 4).is_empty());
    }
    #[test]
    fn dense_changes_and_shape_changes_use_full_upload() {
        assert_eq!(changed_rows(&[0; 16], &[1; 16], 4), vec![0..4]);
        assert_eq!(changed_rows(&[], &[1; 16], 4), vec![0..4]);
        assert!(changed_rows(&[], &[1; 3], 4).is_empty());
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod projection_shader_tests {
    #[test]
    fn camera_shaders_validate_with_the_same_uniform_layout() {
        use wgpu::naga;
        for (name,source) in [
            ("tiles",include_str!("../shaders/tiles.wgsl")),
            ("radar",include_str!("../shaders/radar.wgsl")),
            ("overlay",include_str!("../shaders/overlay.wgsl")),
            ("mrms",include_str!("../shaders/mrms.wgsl")),
            ("wind",include_str!("../shaders/wind.wgsl")),
        ] {
            let module=naga::front::wgsl::parse_str(source).unwrap_or_else(|e| panic!("{name}: {}",e.emit_to_string(source)));
            let _info=naga::valid::Validator::new(naga::valid::ValidationFlags::all(),naga::valid::Capabilities::empty()).validate(&module).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            #[cfg(target_os = "linux")]
            for entry in &module.entry_points {
                let options=naga::back::glsl::Options {version:naga::back::glsl::Version::Embedded {version:300,is_webgl:true},..Default::default()};
                let pipeline=naga::back::glsl::PipelineOptions {shader_stage:entry.stage,entry_point:entry.name.clone(),multiview:None};
                let mut output=String::new();
                naga::back::glsl::Writer::new(&mut output,&module,&_info,&options,&pipeline,Default::default()).unwrap().write().unwrap_or_else(|e| panic!("WebGL2 {name}/{}: {e:?}",entry.name));
            }
            let uniform=module.types.iter().find_map(|(_,ty)| {
                if ty.name.as_deref()==Some(if name=="wind" { "Wind" } else { "Camera" }) {
                    if let naga::TypeInner::Struct {span,..}=ty.inner { return Some(span); }
                } None
            }).unwrap();
            assert_eq!(uniform,if name=="wind" {112} else {std::mem::size_of::<super::CameraUniform>() as u32},"{name}");
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod smoothing_tests {
    use super::*;

    fn reference(gates: &[u16], width: usize, x: usize, y: usize) -> f32 {
        let height = gates.len() / width;
        let center = gates[y * width + x];
        if center < 2 { return center as f32; }
        let mut sum = 0.0;
        let mut weight = 0.0;
        for j in -1isize..=1 {
            let row = (y as isize + j).rem_euclid(height as isize) as usize;
            if gates[row * width + x] < 2 { continue; }
            let mut inner = 0.0;
            let mut inner_weight = 0.0;
            for i in -1isize..=1 {
                let col = x as isize + i;
                if col < 0 || col >= width as isize { continue; }
                let value = gates[row * width + col as usize];
                if value < 2 { continue; }
                let w = if i == 0 { 2.0 } else { 1.0 };
                inner += value as f32 * w;
                inner_weight += w;
            }
            let w = if j == 0 { 2.0 } else { 1.0 };
            sum += inner / inner_weight * w;
            weight += w;
        }
        sum / weight
    }

    fn read_cache(device: &wgpu::Device, queue: &wgpu::Queue, radar: &RadarGpu) -> Vec<f32> {
        let stride = (radar.dims.0 * 4).div_ceil(256) * 256;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None, size: u64::from(stride * radar.dims.1),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: radar.smooth.as_ref().unwrap(), mip_level: 0,
                origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(stride), rows_per_image: None } },
            wgpu::Extent3d { width: radar.dims.0, height: radar.dims.1, depth_or_array_layers: 1 });
        queue.submit(Some(encoder.finish()));
        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        rx.recv().unwrap().unwrap();
        let mapped = slice.get_mapped_range();
        mapped.chunks_exact(stride as usize).flat_map(|row| {
            row[..radar.dims.0 as usize * 4].as_chunks::<4>().0.iter().map(|word| f32::from_le_bytes(*word))
        }).collect()
    }

    #[test]
    fn cache_format_is_portable_and_preserves_reserved_values() {
        let features = wgpu::TextureFormat::R32Uint.guaranteed_format_features(wgpu::Features::empty());
        assert!(features.allowed_usages.contains(wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT));
        let gates = [0, 1, 20, 30, 40, 50];
        assert_eq!(reference(&gates, 3, 0, 0), 0.0);
        assert_eq!(reference(&gates, 3, 1, 0), 1.0);
        assert!(reference(&gates, 3, 2, 0) >= 2.0);
    }

    #[test]
    #[ignore = "gpu"]
    fn gpu_cached_smoothing_preserves_precision_classification_and_reuse() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let instance = wgpu::Instance::default();
        let adapter = rt.block_on(instance.request_adapter(&Default::default())).unwrap();
        let (device, queue) = rt.block_on(adapter.request_device(&Default::default())).unwrap();
        let resources = RenderResources::new(&device, wgpu::TextureFormat::Rgba8UnormSrgb);
        for precision in [false, true] {
            let mut gates = vec![0, 1, 20, 30, 40, 50, 0, 250, 1, 90, 100, 110];
            if precision { gates[7] = 65_535; }
            let mut uniform = [0.0; 20];
            uniform[4] = 3.0; uniform[5] = 4.0; uniform[18] = if precision { 65_535.0 } else { 255.0 };
            let mut upload = RadarUpload {
                upload_token: 1, cache_loop: true, az_bins: 3, gate_count: 4,
                data: gates.iter().map(|v| *v as u8).collect(),
                data16: precision.then(|| gates.clone()), uniform, sector_opacity: None,
                lut: vec![255; 256 * 3 * 4], precip_flag: Vec::new(),
                world_min: [0.0; 2], world_max: [1.0; 2], lut_only: false,
            };
            let mut gpu = resources.build_radar(&device, &queue, &upload, None, false).unwrap();
            assert!(gpu.smooth.is_none(), "disabled smoothing allocates no cache");
            upload.lut_only = true; upload.uniform[19] = 1.0;
            gpu = resources.build_radar(&device, &queue, &upload, Some(gpu), false).unwrap();
            let first = read_cache(&device, &queue, &gpu);
            for (i, actual) in first.iter().enumerate() {
                let expected = reference(&gates, 4, i % 4, i / 4);
                assert!((actual - expected).abs() <= expected.abs().max(1.0) * 0.000_001,
                    "{precision} gate {i}: {actual} != {expected}");
                if gates[i] < 2 { assert_eq!(actual.to_bits(), (gates[i] as f32).to_bits()); }
            }
            assert!(!gpu.smooth_dirty);
            let bytes = gpu.shadow_bytes();
            upload.uniform[8] = 42.0;
            upload.lut[8] = 0;
            gpu = resources.build_radar(&device, &queue, &upload, Some(gpu), false).unwrap();
            assert_eq!(first, read_cache(&device, &queue, &gpu), "palette and motion keep data cache");
            assert_eq!(bytes, gpu.shadow_bytes());
            gates[2] = 150; upload.data[2] = 150;
            if let Some(data) = &mut upload.data16 { data[2] = 150; }
            upload.lut_only = false;
            gpu = resources.build_radar(&device, &queue, &upload, Some(gpu), false).unwrap();
            assert_ne!(first, read_cache(&device, &queue, &gpu), "changed gates rebuild smoothing");
            assert_eq!(gpu.shadow_bytes(), gpu.gate_bytes.len() + gpu.flag_bytes.len() + 256 * 12 + gates.len() * 4);
        }
    }
}
