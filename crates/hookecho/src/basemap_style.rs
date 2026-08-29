//! Color/width styling for the OpenMapTiles vector basemap.
//!
//! Pure lookups keyed by MVT layer name + a per-layer "class" discriminator (the caller pulls
//! the right property: `class` for transportation, `admin_level` for boundary, empty otherwise).
//! Colors are opaque sRGB `[r,g,b,a]`; the tessellator converts to linear. Only styled features
//! produce geometry — an unstyled layer/class returns `None` and is skipped.

/// 0xRRGGBB -> opaque RGBA.
const fn rgb(hex: u32) -> [u8; 4] {
    [(hex >> 16) as u8, (hex >> 8) as u8, hex as u8, 255]
}

/// Which look the vector pipeline tessellates for.
///
/// This used to be a bare `dark: bool`. It is an enum because the hybrid satellite basemap needs
/// a third look — roads and boundaries only, no land or water fills — drawn *over* raster imagery
/// rather than instead of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Palette {
    #[default]
    Dark,
    Light,
    /// Roads/boundaries over satellite imagery: no fills, no background, high-contrast strokes.
    HybridOverlay,
    /// Warm, high-contrast street map in the OSM Liberty vein.
    Liberty,
    /// Pale, colourful, low-ink — reads well under heavy radar fill.
    Bright,
    /// Near-monochrome grey; the most radar gets out of the way.
    Positron,
    /// Very dark, desaturated navy — the night-drive look.
    Midnight,
}

impl Palette {
    /// Every selectable vector look, in menu order.
    pub const ALL: [Palette; 6] = [
        Palette::Dark,
        Palette::Light,
        Palette::Liberty,
        Palette::Bright,
        Palette::Positron,
        Palette::Midnight,
    ];
}

/// The per-palette color table. One struct rather than a pile of parallel `match` arms: adding a
/// look is then a new `Colors` value, not an edit in five places.
struct Colors {
    water: u32,
    wood: u32,
    park: u32,
    residential: u32,
    building: u32,
    motorway: u32,
    primary: u32,
    secondary: u32,
    minor: u32,
    rail: u32,
    admin2: u32,
    admin4: u32,
    county: u32,
    casing: u32,
}

const C_DARK: Colors = Colors {
    water: 0x1b2733,
    wood: 0x151c17,
    park: 0x152018,
    residential: 0x16181d,
    building: 0x1d2028,
    motorway: 0x3d4450,
    primary: 0x353c47,
    secondary: 0x2c323b,
    minor: 0x23282f,
    rail: 0x2a2f36,
    admin2: 0x5a6470,
    admin4: 0x3a424c,
    county: 0x2f353d,
    casing: 0x0d1015,
};
const C_LIGHT: Colors = Colors {
    water: 0xc3d6e3,
    wood: 0xd6e0cf,
    park: 0xd9e8d2,
    residential: 0xece8e0,
    building: 0xe3ded4,
    motorway: 0xf6d3a0,
    primary: 0xf9dfb0,
    secondary: 0xe9e3d5,
    minor: 0xdedacf,
    rail: 0xcdc9c0,
    admin2: 0x9aa0a8,
    admin4: 0xc4c0b8,
    county: 0xd2cec6,
    casing: 0xd8cfc0,
};
// Warmer paper, saturated road ladder, strong casings — the OSM Liberty look.
const C_LIBERTY: Colors = Colors {
    water: 0xa8ccdf,
    wood: 0xc8dcb4,
    park: 0xd2e8c0,
    residential: 0xf0ece2,
    building: 0xdfd8c8,
    motorway: 0xf29a6a,
    primary: 0xf7bd7a,
    secondary: 0xfbdc9e,
    minor: 0xf7f4ec,
    rail: 0xb9b2a6,
    admin2: 0x8c8478,
    admin4: 0xb6afa2,
    county: 0xcac3b6,
    casing: 0xc08a5a,
};
// Pale and colourful but low-ink, so heavy radar fill still reads over it.
const C_BRIGHT: Colors = Colors {
    water: 0xd0e6f2,
    wood: 0xdcecd4,
    park: 0xe2f2da,
    residential: 0xf6f4f0,
    building: 0xeae5dc,
    motorway: 0xffd9b0,
    primary: 0xffe8c8,
    secondary: 0xf4f0e6,
    minor: 0xfbfaf6,
    rail: 0xd8d2c8,
    admin2: 0xb2aca4,
    admin4: 0xd0cac2,
    county: 0xdedad2,
    casing: 0xe0d6c6,
};
// Near-monochrome. The look that gets furthest out of the radar's way.
const C_POSITRON: Colors = Colors {
    water: 0xd6dde2,
    wood: 0xe4e7e4,
    park: 0xe7eae7,
    residential: 0xf2f2f0,
    building: 0xe6e6e4,
    motorway: 0xdcdcdc,
    primary: 0xe4e4e4,
    secondary: 0xececec,
    minor: 0xf4f4f4,
    rail: 0xdadada,
    admin2: 0xb0b0b0,
    admin4: 0xcccccc,
    county: 0xdcdcdc,
    casing: 0xc4c4c4,
};
// Darker and bluer than Dark, with slightly hotter roads — the night-drive look.
const C_MIDNIGHT: Colors = Colors {
    water: 0x0d1826,
    wood: 0x0c1410,
    park: 0x0d1712,
    residential: 0x0e1118,
    building: 0x141824,
    motorway: 0x4a5570,
    primary: 0x3c4560,
    secondary: 0x2e3548,
    minor: 0x232838,
    rail: 0x282e3c,
    admin2: 0x5a6480,
    admin4: 0x38405a,
    county: 0x2a3040,
    casing: 0x05070c,
};

fn colors(p: Palette) -> &'static Colors {
    match p {
        Palette::Dark => &C_DARK,
        Palette::Light => &C_LIGHT,
        Palette::Liberty => &C_LIBERTY,
        Palette::Bright => &C_BRIGHT,
        Palette::Positron => &C_POSITRON,
        Palette::Midnight => &C_MIDNIGHT,
        // Never asked for: `fill` and `stroke` handle the overlay before they get here.
        Palette::HybridOverlay => &C_DARK,
    }
}

/// Whole-tile constants (drawn before per-feature geometry).
pub struct VecStyle {
    /// Land quad under everything. `None` draws no quad at all — the overlay palette wants
    /// whatever is underneath (satellite imagery) to show through.
    pub background: Option<[u8; 4]>,
    /// City/town label text.
    pub label: [u8; 4],
    /// Label halo/outline for contrast.
    pub label_halo: [u8; 4],
}

const DARK: VecStyle = VecStyle {
    background: Some(rgb(0x111318)),
    label: rgb(0xc8d0da),
    label_halo: rgb(0x0b0d11),
};
const LIGHT: VecStyle = VecStyle {
    background: Some(rgb(0xf2efe9)),
    label: rgb(0x3a3a3a),
    label_halo: rgb(0xf7f5f0),
};
// Imagery is busy and mid-tone in both directions, so the overlay leans on contrast rather than
// hue: near-white lines and text, near-black halo.
// ponytail: one fixed overlay palette. Per-style overlay tuning if a second raster basemap ever
// wants a different one.
const HYBRID: VecStyle = VecStyle {
    background: None,
    label: rgb(0xffffff),
    label_halo: rgb(0x000000),
};

const LIBERTY: VecStyle = VecStyle {
    background: Some(rgb(0xf7f4ec)),
    label: rgb(0x30302c),
    label_halo: rgb(0xfbf9f3),
};
const BRIGHT: VecStyle = VecStyle {
    background: Some(rgb(0xfbfaf6)),
    label: rgb(0x44443e),
    label_halo: rgb(0xfdfdfa),
};
const POSITRON: VecStyle = VecStyle {
    background: Some(rgb(0xf4f4f2)),
    label: rgb(0x555555),
    label_halo: rgb(0xfafafa),
};
const MIDNIGHT: VecStyle = VecStyle {
    background: Some(rgb(0x070a12)),
    label: rgb(0xb8c4d8),
    label_halo: rgb(0x03050a),
};

pub fn style(p: Palette) -> &'static VecStyle {
    match p {
        Palette::Dark => &DARK,
        Palette::Light => &LIGHT,
        Palette::HybridOverlay => &HYBRID,
        Palette::Liberty => &LIBERTY,
        Palette::Bright => &BRIGHT,
        Palette::Positron => &POSITRON,
        Palette::Midnight => &MIDNIGHT,
    }
}

/// Casing (outline) color + width for a road, drawn in a pass *under* the road itself so majors
/// read as ribbons rather than bare lines. `None` for classes that get no casing.
///
/// Width here is the total width of the casing stroke, so it has to exceed the road's own or the
/// road would cover it entirely.
pub fn casing(p: Palette, layer: &str, class: &str) -> Option<([u8; 4], f32)> {
    if layer != "transportation" {
        return None;
    }
    // Only the classes wide enough for a casing to be visible. Below tertiary the casing and the
    // road would be a pixel apart and the result is mud.
    let widen = match class {
        "motorway" | "trunk" => 2.2,
        "primary" => 1.8,
        "secondary" => 1.4,
        _ => return None,
    };
    // Over imagery the casing is what makes a pale road legible at all.
    let c = if p == Palette::HybridOverlay {
        [0, 0, 0, 255]
    } else {
        rgb(colors(p).casing)
    };
    let (_, w) = stroke(p, layer, class)?;
    Some((c, w + widen))
}

/// Fill color for a polygon feature, or `None` to skip it.
pub fn fill(p: Palette, layer: &str, class: &str) -> Option<[u8; 4]> {
    // Nothing is filled over imagery — the imagery *is* the land cover.
    if p == Palette::HybridOverlay {
        return None;
    }
    let k = colors(p);
    let (water, wood, park, residential) = (k.water, k.wood, k.park, k.residential);
    let c = match layer {
        "water" | "ocean" => water,
        "waterway" => water,
        "landcover" => match class {
            "wood" | "forest" | "tree" => wood,
            "grass" | "meadow" | "park" | "scrub" | "farmland" => park,
            _ => return None,
        },
        "landuse" => match class {
            "park" | "cemetery" | "recreation_ground" | "pitch" | "golf_course" | "grass"
            | "wood" | "forest" | "meadow" => park,
            "residential" | "suburb" | "neighbourhood" | "commercial" | "industrial" => residential,
            _ => return None,
        },
        "park" => park,
        // Flat footprints, a touch off the background so blocks read without shouting.
        // ponytail: flat fills, no extrusion — `render_height` is in the tile if that ever
        // becomes worth the depth buffer.
        "building" => k.building,
        _ => return None,
    };
    Some(rgb(c))
}

/// Stroke color + pixel width for a line feature, or `None` to skip it.
pub fn stroke(p: Palette, layer: &str, class: &str) -> Option<([u8; 4], f32)> {
    if p == Palette::HybridOverlay {
        // Wider than the vector basemap's own roads: a 0.8 px hairline vanishes against imagery.
        // Only the classes worth having over aerial photography — the minor-road mesh would turn
        // a city into a white smear.
        let (c, w) = match layer {
            "transportation" => match class {
                "motorway" | "trunk" => (0xffe9a8, 2.6),
                "primary" => (0xfff4d2, 2.0),
                "secondary" | "tertiary" => (0xf0f0f0, 1.4),
                _ => return None,
            },
            "boundary" => match class {
                "2" => (0xffd9d9, 1.6),
                "3" | "4" => (0xe8c8c8, 1.0),
                _ => return None,
            },
            _ => return None,
        };
        return Some((rgb(c), w));
    }
    let k = colors(p);
    let (motorway, primary, secondary, minor, rail) =
        (k.motorway, k.primary, k.secondary, k.minor, k.rail);
    let (admin2, admin4, county) = (k.admin2, k.admin4, k.county);
    // Waterway lines lean on the water fill so the two agree.
    let water = k.water;
    let (c, w) = match layer {
        "transportation" => match class {
            "motorway" => (motorway, 2.2),
            "trunk" => (motorway, 1.9),
            "primary" => (primary, 1.6),
            "secondary" => (secondary, 1.3),
            "tertiary" => (secondary, 1.05),
            "minor" | "street" => (minor, 0.85),
            "service" | "track" => (minor, 0.55),
            "path" | "footway" | "cycleway" | "pedestrian" => (minor, 0.4),
            "rail" | "transit" => (rail, 0.8),
            _ => return None,
        },
        "aeroway" => (rail, 1.4),
        "waterway" => (water, 1.0),
        // caller passes admin_level as the class string; maritime boundaries filtered upstream.
        "boundary" => match class {
            "2" => (admin2, 1.2),
            "3" | "4" => (admin4, 0.8),
            // Counties: hairline, and only drawn once the caller is zoomed in enough
            // (see the zoom gate in vector_tiles.rs) or they smear the CONUS view.
            "6" => (county, 0.5),
            _ => return None,
        },
        _ => return None,
    };
    Some((rgb(c), w))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_and_light_differ_and_resolve() {
        assert_ne!(
            style(Palette::Dark).background,
            style(Palette::Light).background
        );
        // Every documented class resolves in both themes.
        for p in [Palette::Dark, Palette::Light] {
            assert!(fill(p, "water", "").is_some());
            assert!(fill(p, "landcover", "wood").is_some());
            assert!(fill(p, "landuse", "residential").is_some());
            assert!(fill(p, "park", "").is_some());
            assert!(stroke(p, "transportation", "motorway").is_some());
            assert!(stroke(p, "boundary", "2").is_some());
            assert!(stroke(p, "waterway", "").is_some());
            assert!(fill(p, "building", "").is_some());
            // Unstyled input is skipped, not defaulted.
            assert!(fill(p, "aeroway", "").is_none());
            assert!(stroke(p, "transportation", "elevator").is_none());
        }
    }

    /// The hybrid overlay's whole job is to add lines to imagery without hiding it. If it ever
    /// starts returning fills or a background quad, it paints over the satellite photo.
    #[test]
    fn hybrid_overlay_covers_nothing_it_should_not() {
        let p = Palette::HybridOverlay;
        assert!(style(p).background.is_none());
        for (layer, class) in [
            ("water", ""),
            ("landcover", "wood"),
            ("landuse", "residential"),
            ("park", ""),
        ] {
            assert!(
                fill(p, layer, class).is_none(),
                "{layer}/{class} fills over imagery"
            );
        }
        // It still draws the roads it exists for, and thicker than the vector basemap's.
        let (_, hybrid_w) = stroke(p, "transportation", "motorway").unwrap();
        let (_, dark_w) = stroke(Palette::Dark, "transportation", "motorway").unwrap();
        assert!(hybrid_w > dark_w);
        // No minor-road mesh over imagery.
        assert!(stroke(p, "transportation", "minor").is_none());
    }

    /// A casing only works if it is wider than the road it sits under and darker than it — get
    /// either backwards and the road disappears into its own outline.
    #[test]
    fn casings_sit_under_the_roads_they_outline() {
        for p in [Palette::Dark, Palette::Light, Palette::HybridOverlay] {
            for class in ["motorway", "trunk", "primary", "secondary"] {
                let Some((_, road_w)) = stroke(p, "transportation", class) else {
                    continue;
                };
                let (_, case_w) = casing(p, "transportation", class)
                    .unwrap_or_else(|| panic!("{p:?}/{class} should have a casing"));
                assert!(case_w > road_w, "{p:?}/{class}: casing must be wider");
            }
            // Small roads get none: at a pixel apart the two strokes are just mud.
            assert!(casing(p, "transportation", "minor").is_none());
            // And nothing else in the tile has a casing at all.
            assert!(casing(p, "boundary", "2").is_none());
            assert!(casing(p, "waterway", "").is_none());
        }
    }

    /// The road ladder has to be monotonic, or a residential street outdraws an interstate.
    #[test]
    fn road_widths_descend_by_importance() {
        for p in [Palette::Dark, Palette::Light] {
            let w = |c: &str| stroke(p, "transportation", c).unwrap().1;
            assert!(w("motorway") > w("trunk"));
            assert!(w("trunk") > w("primary"));
            assert!(w("primary") > w("secondary"));
            assert!(w("secondary") > w("tertiary"));
            assert!(w("tertiary") > w("minor"));
            assert!(w("minor") > w("service"));
            assert!(w("service") > w("path"));
        }
    }
}
