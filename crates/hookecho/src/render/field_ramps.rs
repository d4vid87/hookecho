//! One table describing how every gridded field layer is colored — and what its colors mean.
//!
//! Before this, each layer's value→index mapping and color stops lived inline in a `match` arm in
//! `app.rs`, which made a legend impossible: nothing outside that function knew a layer's range or
//! units. The table is now the single source for BOTH the LUT the GPU samples and the legend the
//! user reads, so a scale can't drift from its own key.

use super::FieldLayer;

/// How raw grid values map onto the 2..=255 index range the LUT is baked over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RampScale {
    /// `(v - lo) / (hi - lo)`.
    Linear,
    /// Linear on `|v|` — signed fields (azimuthal shear) whose sign is direction, not magnitude.
    Abs,
    /// `log10` between `lo` and `hi`; rainfall and recurrence intervals span decades.
    Log,
}

/// A continuous color scale, or a set of labeled category slots.
pub enum FieldScale {
    Ramp {
        lo: f32,
        hi: f32,
        scale: RampScale,
        /// `(t, rgb)` with `t` in 0..=1, interpolated.
        stops: &'static [(f32, [u8; 3])],
    },
    /// `(raw grid value, rgb, label)` — drawn as discrete swatches, no interpolation.
    Categorical(&'static [(u8, [u8; 3], &'static str)]),
}

/// How one field layer is colored and labeled.
pub struct FieldRamp {
    /// Legend heading.
    pub label: &'static str,
    /// Physical units, empty when the values are categories or already unitless.
    pub units: &'static str,
    pub scale: FieldScale,
    /// Opacity for non-zero indices; environment overlays sit translucent over the basemap.
    pub alpha: u8,
    /// Multiplier applied to raw grid values before anything else, when the wire units aren't the
    /// units anyone reads (HRRR smoke arrives in kg/m³; people talk in µg/m³).
    pub input_scale: f32,
}

impl FieldRamp {
    /// Raw grid value in this layer's display units (applies [`Self::input_scale`]).
    pub fn display(&self, v: f32) -> f32 {
        v * self.input_scale
    }

    /// Raw grid value → LUT index. Index 0 is "nothing here" (fully transparent).
    pub fn index(&self, v: f32) -> u8 {
        let v = self.display(v);
        match &self.scale {
            FieldScale::Categorical(_) => v as u8,
            FieldScale::Ramp {
                lo,
                hi,
                scale,
                stops: _,
            } => {
                let v = if *scale == RampScale::Abs { v.abs() } else { v };
                if v < *lo {
                    return 0;
                }
                let t = match scale {
                    RampScale::Log => (v.log10() - lo.log10()) / (hi.log10() - lo.log10()),
                    _ => (v - lo) / (hi - lo),
                };
                (2.0 + t.clamp(0.0, 1.0) * 253.0) as u8
            }
        }
    }
}

macro_rules! ramp {
    ($label:expr, $units:expr, $lo:expr, $hi:expr, $scale:expr, $alpha:expr, $stops:expr) => {
        FieldRamp {
            label: $label,
            units: $units,
            alpha: $alpha,
            input_scale: 1.0,
            scale: FieldScale::Ramp {
                lo: $lo,
                hi: $hi,
                scale: $scale,
                stops: $stops,
            },
        }
    };
}

static ROTATION: FieldRamp = ramp!(
    "Rotation",
    "\u{d7}10\u{207b}\u{b3} s\u{207b}\u{b9}",
    4.0,
    40.0,
    RampScale::Abs,
    255,
    &[
        (0.0, [40, 90, 200]),
        (0.4, [40, 200, 200]),
        (0.7, [240, 230, 60]),
        (1.0, [230, 40, 40]),
    ]
);

/// Wind speed, for the animated particle layer.
///
/// Public and deliberately absent from [`ramp_for`]: wind is not a [`FieldLayer`] — nothing is
/// uploaded to the GPU as a scalar grid — but the particles still need a value→color rule and the
/// legend still needs to explain it. `input_scale` converts the model's m/s to the knots people
/// actually read, so both get the same conversion for free.
pub static WIND: FieldRamp = FieldRamp {
    input_scale: 1.943_844,
    ..ramp!(
        "Wind",
        "kt",
        0.0,
        // 50, not 80: surface wind is 5-20 kt almost everywhere almost always, and a scale topping
        // out at jet speeds leaves every particle in the pale bottom tenth of the palette. The
        // live decode test peaked at 39 kt over the whole of CONUS.
        50.0,
        RampScale::Linear,
        255,
        &[
            (0.0, [120, 165, 205]),
            (0.25, [95, 200, 175]),
            (0.5, [235, 225, 120]),
            (0.7, [240, 155, 70]),
            (0.85, [225, 80, 70]),
            (1.0, [230, 95, 200]),
        ]
    )
};

static MESH: FieldRamp = ramp!(
    "Hail size",
    "mm",
    10.0,
    75.0,
    RampScale::Linear,
    255,
    &[
        (0.0, [60, 200, 90]),
        (0.4, [240, 230, 60]),
        (0.7, [240, 150, 30]),
        (1.0, [230, 60, 200]),
    ]
);

static HAIL_SWATH: FieldRamp = ramp!(
    "Hail swaths (24 h)",
    "mm",
    19.0,
    75.0,
    RampScale::Linear,
    255,
    &[
        (0.0, [60, 200, 90]),
        (0.4, [240, 230, 60]),
        (0.7, [240, 150, 30]),
        (1.0, [230, 60, 200]),
    ]
);

static QPE_STOPS: &[(f32, [u8; 3])] = &[
    (0.0, [40, 180, 90]),
    (0.3, [230, 220, 60]),
    (0.55, [230, 110, 40]),
    (0.8, [220, 40, 60]),
    (1.0, [230, 220, 240]),
];
// Rate, not accumulation: the top of the scale is a rain rate rather than a storm total, so
// it shares the QPE colours but not its bounds. 100 mm/hr is a tropical downpour.
static PRECIP_RATE: FieldRamp = ramp!(
    "Rain rate",
    "mm/hr",
    0.25,
    100.0,
    RampScale::Log,
    255,
    QPE_STOPS
);
static QPE_1H: FieldRamp = ramp!(
    "Rain (1 h)",
    "mm",
    0.25,
    100.0,
    RampScale::Log,
    255,
    QPE_STOPS
);
static QPE_24H: FieldRamp = ramp!(
    "Rain (24 h)",
    "mm",
    0.25,
    250.0,
    RampScale::Log,
    255,
    QPE_STOPS
);

static CAPE: FieldRamp = ramp!(
    "CAPE",
    "J/kg",
    100.0,
    5000.0,
    RampScale::Linear,
    150,
    &[
        (0.0, [0, 200, 200]),
        (0.25, [40, 200, 90]),
        (0.5, [240, 230, 60]),
        (0.75, [240, 150, 30]),
        (1.0, [230, 60, 200]),
    ]
);

static SRH: FieldRamp = ramp!(
    "Helicity",
    "m\u{b2}/s\u{b2}",
    50.0,
    500.0,
    RampScale::Linear,
    150,
    &[
        (0.0, [40, 90, 200]),
        (0.5, [240, 230, 60]),
        (1.0, [230, 40, 40]),
    ]
);

static FLASH_FLOOD: FieldRamp = ramp!(
    "Flood recurrence",
    "yr",
    1.0,
    100.0,
    RampScale::Log,
    255,
    &[
        (0.0, [240, 230, 60]),
        (0.3, [240, 150, 30]),
        (0.6, [230, 40, 40]),
        (0.85, [150, 40, 200]),
        (1.0, [240, 240, 240]),
    ]
);

static VIL: FieldRamp = ramp!(
    "Water aloft (VIL)",
    "kg/m\u{b2}",
    0.1,
    80.0,
    RampScale::Linear,
    255,
    &[
        (0.0, [60, 200, 90]),
        (0.35, [240, 230, 60]),
        (0.6, [240, 150, 30]),
        (0.85, [230, 60, 200]),
        (1.0, [240, 240, 240]),
    ]
);

static ECHO_TOPS: FieldRamp = ramp!(
    "Storm tops",
    "kft",
    5.0,
    70.0,
    RampScale::Linear,
    255,
    &[
        (0.0, [40, 90, 200]),
        (0.4, [40, 200, 90]),
        (0.75, [240, 230, 60]),
        (1.0, [240, 240, 240]),
    ]
);

/// VIL density: water aloft per unit storm depth. Above ~3.5 g/m³ is the classic large-hail
/// signature, so the scale turns hot exactly there rather than spending its range on drizzle.
static VIL_DENSITY: FieldRamp = ramp!(
    "VIL density",
    "g/m\u{b3}",
    0.5,
    5.0,
    RampScale::Linear,
    255,
    &[
        (0.0, [40, 90, 200]),
        (0.45, [60, 200, 90]),
        (0.65, [240, 230, 60]),
        (0.8, [240, 150, 30]),
        (1.0, [230, 60, 200]),
    ]
);

/// GLM flash-extent density: flashes per cell over the last 15 minutes. One flash is worth
/// showing (it is the first one), and a vigorous updraft runs into the tens, so the scale is
/// short and warms fast.
static GLM_FED: FieldRamp = ramp!(
    "Flash density",
    "flashes/15 min",
    1.0,
    30.0,
    RampScale::Linear,
    220,
    &[
        (0.0, [60, 60, 160]),
        (0.35, [90, 180, 230]),
        (0.6, [240, 230, 60]),
        (0.8, [240, 140, 40]),
        (1.0, [255, 60, 60]),
    ]
);

/// Probability of severe hail. 50% is Witt's warning threshold, so the scale turns warm there.
static POSH: FieldRamp = ramp!(
    "Severe hail probability",
    "%",
    10.0,
    100.0,
    RampScale::Linear,
    255,
    &[
        (0.0, [40, 90, 200]),
        (0.45, [240, 230, 60]),
        (0.75, [240, 150, 30]),
        (1.0, [230, 60, 200]),
    ]
);

/// Global-model mean sea-level pressure. The band is wide because this is a whole-planet field:
/// a deep low and a summer ridge have to share one scale.
static GLOBAL_MSLP: FieldRamp = FieldRamp {
    input_scale: 0.01, // Pa → hPa
    ..ramp!(
        "MSLP",
        "hPa",
        960.0,
        1040.0,
        RampScale::Linear,
        170,
        &[
            (0.0, [150, 60, 200]),
            (0.35, [60, 120, 220]),
            (0.5, [230, 230, 230]),
            (0.7, [240, 170, 60]),
            (1.0, [200, 60, 60]),
        ]
    )
};

/// 500 hPa geopotential height — the steering flow, in decametres the way charts label it.
static GLOBAL_HEIGHT_500: FieldRamp = FieldRamp {
    input_scale: 0.1, // m → dam
    ..ramp!(
        "500 hPa height",
        "dam",
        492.0,
        600.0,
        RampScale::Linear,
        170,
        &[
            (0.0, [120, 60, 190]),
            (0.3, [60, 110, 220]),
            (0.55, [90, 200, 160]),
            (0.8, [240, 200, 70]),
            (1.0, [220, 70, 50]),
        ]
    )
};

/// Global 2 m temperature, in the units most of the planet reads.
static GLOBAL_TEMP_2M: FieldRamp = FieldRamp {
    // Kelvin → °C is an offset, not a scale, so the ramp is expressed in Kelvin and labelled °C
    // by the legend's own offset handling would be a lie — keep it in Kelvin-derived °C here.
    input_scale: 1.0,
    ..ramp!(
        "2 m temp",
        "K",
        233.0,
        318.0,
        RampScale::Linear,
        170,
        &[
            (0.0, [80, 40, 160]),
            (0.25, [60, 140, 220]),
            (0.5, [230, 230, 210]),
            (0.75, [240, 160, 50]),
            (1.0, [190, 40, 40]),
        ]
    )
};

/// Global 10 m wind speed (the U component's magnitude band, which is what the layer draws).
static GLOBAL_WIND_10M: FieldRamp = FieldRamp {
    input_scale: 1.943_844, // m/s → kt
    ..ramp!(
        "10 m wind",
        "kt",
        5.0,
        80.0,
        RampScale::Abs,
        170,
        &[
            (0.0, [70, 130, 180]),
            (0.4, [90, 200, 140]),
            (0.7, [240, 200, 60]),
            (1.0, [220, 60, 60]),
        ]
    )
};

/// Global moisture: GFS publishes precipitable water, ECMWF total precipitation. Both are
/// millimetres of water and both answer "how wet is this air mass".
static GLOBAL_PRECIP: FieldRamp = ramp!(
    "Precipitable water",
    "mm",
    1.0,
    70.0,
    RampScale::Log,
    170,
    &[
        (0.0, [60, 80, 120]),
        (0.4, [70, 170, 190]),
        (0.7, [90, 210, 110]),
        (1.0, [240, 230, 90]),
    ]
);

/// Forecast snowfall accumulation. The model reports metres; nobody talks in metres of snow.
static SNOWFALL: FieldRamp = FieldRamp {
    input_scale: 39.370_08, // m → in
    ..ramp!(
        "Snowfall",
        "in",
        0.1,
        24.0,
        RampScale::Log,
        220,
        &[
            (0.0, [200, 235, 255]),
            (0.35, [90, 170, 235]),
            (0.6, [60, 90, 210]),
            (0.8, [140, 60, 200]),
            (1.0, [240, 240, 255]),
        ]
    )
};

/// Observed snowfall. Same units and shape as the forecast scale, one decade taller: a 72-hour
/// analysis of a lake-effect band goes places a model run does not.
static SNOW_ANALYSIS: FieldRamp = FieldRamp {
    input_scale: 39.370_08, // m → in
    ..ramp!(
        "Snowfall (observed)",
        "in",
        0.1,
        48.0,
        RampScale::Log,
        220,
        &[
            (0.0, [200, 235, 255]),
            (0.35, [90, 170, 235]),
            (0.6, [60, 90, 210]),
            (0.8, [140, 60, 200]),
            (1.0, [240, 240, 255]),
        ]
    )
};

/// Banded snow. The values are reflectivity, but the scale is not the reflectivity palette: what
/// the layer says is "this echo is organised into a line", and it has to read as that against the
/// mosaic it was cut out of.
static SNOW_BANDS: FieldRamp = ramp!(
    "Snow bands",
    "dBZ",
    10.0,
    45.0,
    RampScale::Linear,
    230,
    &[
        (0.0, [120, 160, 210]),
        (0.5, [180, 215, 250]),
        (1.0, [255, 255, 255]),
    ]
);

/// Calibrated chance of a thunderstorm. A probability, so the scale is linear and the numbers on
/// the legend are the forecast: 30 means thirty percent.
static THUNDER_PROB: FieldRamp = ramp!(
    "Chance of thunder",
    "%",
    5.0,
    100.0,
    RampScale::Linear,
    150,
    &[
        (0.0, [70, 100, 140]),
        (0.35, [90, 180, 190]),
        (0.7, [240, 200, 90]),
        (1.0, [235, 90, 70]),
    ]
);

static PRECIP_TYPE: FieldRamp = FieldRamp {
    label: "Precip type",
    units: "",
    alpha: 200,
    input_scale: 1.0,
    scale: FieldScale::Categorical(&[
        (1, [60, 200, 90], "Rain"),
        (3, [90, 150, 240], "Snow"),
        (6, [240, 230, 60], "Convective"),
        (7, [230, 40, 40], "Hail"),
        (10, [40, 200, 200], "Cold rain"),
        (91, [80, 220, 120], "Tropical rain"),
        (96, [80, 220, 120], "Tropical convective"),
    ]),
};

static HCA: FieldRamp = FieldRamp {
    label: "Hydrometeor class",
    units: "",
    alpha: 200,
    input_scale: 1.0,
    scale: FieldScale::Categorical(&[
        (10, [140, 110, 90], "Biological"),
        (20, [95, 95, 95], "Clutter"),
        (30, [185, 220, 255], "Ice crystals"),
        (40, [110, 160, 240], "Dry snow"),
        (50, [0, 200, 255], "Wet snow"),
        (60, [90, 200, 90], "Light rain"),
        (70, [25, 145, 50], "Heavy rain"),
        (80, [240, 200, 60], "Big drops"),
        (90, [200, 120, 220], "Graupel"),
        (100, [230, 50, 50], "Hail"),
        (110, [170, 0, 0], "Large hail"),
        (120, [120, 0, 60], "Giant hail"),
        (140, [160, 160, 160], "Unknown"),
        (150, [240, 150, 200], "Range folded"),
    ]),
};

static UPDRAFT_HELICITY: FieldRamp = ramp!(
    "Forecast rotation",
    "m\u{b2}/s\u{b2}",
    25.0,
    200.0,
    RampScale::Linear,
    220,
    &[
        (0.0, [90, 60, 190]),
        (0.35, [160, 60, 220]),
        (0.7, [230, 70, 200]),
        (1.0, [255, 210, 245]),
    ]
);

static SMOKE: FieldRamp = FieldRamp {
    input_scale: 1.0e9,
    ..ramp_smoke()
};

const fn ramp_smoke() -> FieldRamp {
    ramp!(
        "Smoke",
        "\u{b5}g/m\u{b3}",
        2.0,
        150.0,
        RampScale::Log,
        170,
        &[
            (0.0, [170, 170, 165]),
            (0.4, [160, 135, 100]),
            (0.7, [140, 95, 60]),
            (1.0, [90, 50, 30]),
        ]
    )
}

/// Bake a 256-entry RGBA LUT from ramp `stops`, with `alpha` on every data index. Indices 0 and
/// 1 stay clear — [`FieldRamp::index`] emits 0 for "no data" and 2..=255 for t in 0..=1, so the
/// bake must use the same mapping or every color sits a hair off its value.
/// Shared by the GPU upload path and the headless verifiers.
pub fn bake_ramp_lut(stops: &[(f32, [u8; 3])], alpha: u8) -> Vec<u8> {
    let mut lut = vec![0u8; 256 * 4];
    for (i, slot) in lut.as_chunks_mut::<4>().0.iter_mut().enumerate().skip(2) {
        let t = (i - 2) as f32 / 253.0;
        let mut rgb = stops[0].1;
        for w in stops.windows(2) {
            let (t0, c0) = w[0];
            let (t1, c1) = w[1];
            if t >= t0 && t <= t1 {
                let u = if (t1 - t0).abs() < f32::EPSILON {
                    0.0
                } else {
                    (t - t0) / (t1 - t0)
                };
                rgb = [
                    (c0[0] as f32 + (c1[0] as f32 - c0[0] as f32) * u) as u8,
                    (c0[1] as f32 + (c1[1] as f32 - c0[1] as f32) * u) as u8,
                    (c0[2] as f32 + (c1[2] as f32 - c0[2] as f32) * u) as u8,
                ];
                break;
            }
            if t > t1 {
                rgb = c1;
            }
        }
        slot.copy_from_slice(&[rgb[0], rgb[1], rgb[2], alpha]);
    }
    lut
}

/// The color scale for `layer`, or `None` for layers colored some other way: the reflectivity
/// palette (`Mrms`/`Hrrr`, which follow the user's `.pal` table) and `Lightning` (own upload fn).
pub fn ramp_for(layer: FieldLayer) -> Option<&'static FieldRamp> {
    use FieldLayer as FL;
    Some(match layer {
        FL::Rotation | FL::AzShear => &ROTATION,
        FL::Mesh => &MESH,
        FL::HailSwath => &HAIL_SWATH,
        FL::PrecipRate => &PRECIP_RATE,
        FL::Qpe1h => &QPE_1H,
        FL::Qpe24h => &QPE_24H,
        FL::Cape => &CAPE,
        FL::Srh => &SRH,
        FL::FlashFlood => &FLASH_FLOOD,
        // Locally derived twins share their L3 counterparts' scales — one VIL scale app-wide, so
        // a number means the same thing whichever source drew it.
        FL::Vil | FL::VilLocal => &VIL,
        FL::EchoTops | FL::EtopLocal => &ECHO_TOPS,
        FL::VilDensity => &VIL_DENSITY,
        // MEHS shares the MRMS MESH scale: one hail scale app-wide.
        FL::HailMehs => &MESH,
        FL::HailPosh => &POSH,
        FL::PrecipType => &PRECIP_TYPE,
        FL::UpdraftHelicity => &UPDRAFT_HELICITY,
        FL::Smoke => &SMOKE,
        FL::Snowfall => &SNOWFALL,
        FL::SnowAnalysis => &SNOW_ANALYSIS,
        FL::GlobalMslp => &GLOBAL_MSLP,
        FL::GlobalHeight500 => &GLOBAL_HEIGHT_500,
        FL::GlobalTemp2m => &GLOBAL_TEMP_2M,
        FL::GlobalWind10m => &GLOBAL_WIND_10M,
        FL::GlobalPrecip => &GLOBAL_PRECIP,
        FL::Hca => &HCA,
        FL::GlmFed => &GLM_FED,
        FL::SnowBands => &SNOW_BANDS,
        FL::ThunderProb => &THUNDER_PROB,
        // Composite is reflectivity in dBZ, so like the mosaic it follows the user's own
        // reflectivity `.pal` rather than a fixed ramp of its own.
        FL::Mrms | FL::Mosaic | FL::CompositeLocal | FL::Hrrr | FL::Lightning | FL::ModelDiff => {
            return None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Layers colored outside this table. A new `FieldLayer` must join the table or this list —
    /// forgetting both silently ships a layer with no legend.
    const NO_RAMP: [FieldLayer; 6] = [
        FieldLayer::Mrms,
        FieldLayer::Mosaic,
        FieldLayer::CompositeLocal,
        FieldLayer::Hrrr,
        FieldLayer::Lightning,
        // The difference layer's ramp is symmetric about zero and rebuilt whenever the field
        // changes, so it is baked in `fielddiff`, not tabulated here.
        FieldLayer::ModelDiff,
    ];

    #[test]
    fn every_layer_is_either_ramped_or_explicitly_exempt() {
        for l in FieldLayer::DRAW_ORDER {
            let exempt = NO_RAMP.contains(&l);
            assert_eq!(
                ramp_for(l).is_some(),
                !exempt,
                "{l:?} must have a ramp or be listed in NO_RAMP"
            );
        }
    }

    #[test]
    fn ramps_are_labeled_and_ordered() {
        for l in FieldLayer::DRAW_ORDER {
            let Some(r) = ramp_for(l) else { continue };
            assert!(!r.label.is_empty(), "{l:?}");
            if let FieldScale::Ramp { lo, hi, .. } = r.scale {
                assert!(lo < hi, "{l:?}: lo {lo} must be below hi {hi}");
            }
        }
    }

    #[test]
    fn index_maps_endpoints() {
        let m = ramp_for(FieldLayer::Mesh).unwrap();
        assert_eq!(m.index(9.9), 0, "below threshold is transparent");
        assert_eq!(m.index(10.0), 2, "threshold is the first visible index");
        assert_eq!(m.index(75.0), 255);
        assert_eq!(m.index(9999.0), 255, "clamps above the top");
    }

    #[test]
    fn abs_scale_ignores_sign() {
        let r = ramp_for(FieldLayer::AzShear).unwrap();
        assert_eq!(r.index(-20.0), r.index(20.0));
        assert_eq!(r.index(-1.0), 0);
    }

    #[test]
    fn log_scale_is_monotonic_across_decades() {
        let q = ramp_for(FieldLayer::Qpe1h).unwrap();
        assert_eq!(q.index(0.2), 0);
        let (a, b, c) = (q.index(1.0), q.index(10.0), q.index(100.0));
        assert!(a < b && b < c, "{a} {b} {c}");
        assert_eq!(c, 255);
    }

    #[test]
    fn categorical_index_is_the_raw_class_code() {
        let h = ramp_for(FieldLayer::Hca).unwrap();
        assert_eq!(h.index(110.0), 110);
    }

    #[test]
    fn lut_uses_the_encoder_index_mapping() {
        let stops = [(0.0, [10, 20, 30]), (1.0, [200, 210, 220])];
        let lut = bake_ramp_lut(&stops, 255);
        assert_eq!(&lut[2 * 4..2 * 4 + 3], &[10, 20, 30], "index 2 is t=0");
        assert_eq!(
            &lut[255 * 4..255 * 4 + 3],
            &[200, 210, 220],
            "index 255 is t=1"
        );
        assert_eq!(lut[4 + 3], 0, "index 1 is never emitted, stays clear");
    }
}
