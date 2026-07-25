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
}

impl FieldRamp {
    /// Raw grid value → LUT index. Index 0 is "nothing here" (fully transparent).
    pub fn index(&self, v: f32) -> u8 {
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

static PRECIP_TYPE: FieldRamp = FieldRamp {
    label: "Precip type",
    units: "",
    alpha: 200,
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

/// The color scale for `layer`, or `None` for layers colored some other way: the reflectivity
/// palette (`Mrms`/`Hrrr`, which follow the user's `.pal` table) and `Lightning` (own upload fn).
pub fn ramp_for(layer: FieldLayer) -> Option<&'static FieldRamp> {
    use FieldLayer as FL;
    Some(match layer {
        FL::Rotation | FL::AzShear => &ROTATION,
        FL::Mesh => &MESH,
        FL::HailSwath => &HAIL_SWATH,
        FL::Qpe1h => &QPE_1H,
        FL::Qpe24h => &QPE_24H,
        FL::Cape => &CAPE,
        FL::Srh => &SRH,
        FL::FlashFlood => &FLASH_FLOOD,
        FL::Vil => &VIL,
        FL::EchoTops => &ECHO_TOPS,
        FL::PrecipType => &PRECIP_TYPE,
        FL::Hca => &HCA,
        FL::Mrms | FL::Hrrr | FL::Lightning => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Layers colored outside this table. A new `FieldLayer` must join the table or this list —
    /// forgetting both silently ships a layer with no legend.
    const NO_RAMP: [FieldLayer; 3] = [FieldLayer::Mrms, FieldLayer::Hrrr, FieldLayer::Lightning];

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
}
