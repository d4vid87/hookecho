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
}

impl Palette {
    /// The two full basemap looks share every code path; the overlay is the odd one out, so most
    /// lookups only need to know which of the two it leans on for contrast.
    fn dark(self) -> bool {
        !matches!(self, Palette::Light)
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

pub fn style(p: Palette) -> &'static VecStyle {
    match p {
        Palette::Dark => &DARK,
        Palette::Light => &LIGHT,
        Palette::HybridOverlay => &HYBRID,
    }
}

/// Fill color for a polygon feature, or `None` to skip it.
pub fn fill(p: Palette, layer: &str, class: &str) -> Option<[u8; 4]> {
    // Nothing is filled over imagery — the imagery *is* the land cover.
    if p == Palette::HybridOverlay {
        return None;
    }
    let dark = p.dark();
    let (water, wood, park, residential) = if dark {
        (0x1b2733, 0x151c17, 0x152018, 0x16181d)
    } else {
        (0xc3d6e3, 0xd6e0cf, 0xd9e8d2, 0xece8e0)
    };
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
    let dark = p.dark();
    let (motorway, primary, secondary, minor, rail, water, admin2, admin4, county) = if dark {
        (
            0x3d4450, 0x353c47, 0x2c323b, 0x23282f, 0x2a2f36, 0x24333f, 0x5a6470, 0x3a424c,
            0x2f353d,
        )
    } else {
        (
            0xf6d3a0, 0xf9dfb0, 0xe9e3d5, 0xdedacf, 0xcdc9c0, 0xa9c4d6, 0x9aa0a8, 0xc4c0b8,
            0xd2cec6,
        )
    };
    let (c, w) = match layer {
        "transportation" => match class {
            "motorway" | "trunk" => (motorway, 2.0),
            "primary" => (primary, 1.5),
            "secondary" => (secondary, 1.2),
            "tertiary" => (secondary, 1.0),
            "minor" | "service" | "street" => (minor, 0.8),
            "rail" | "transit" => (rail, 0.8),
            _ => return None,
        },
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
        assert_ne!(style(Palette::Dark).background, style(Palette::Light).background);
        // Every documented class resolves in both themes.
        for p in [Palette::Dark, Palette::Light] {
            assert!(fill(p, "water", "").is_some());
            assert!(fill(p, "landcover", "wood").is_some());
            assert!(fill(p, "landuse", "residential").is_some());
            assert!(fill(p, "park", "").is_some());
            assert!(stroke(p, "transportation", "motorway").is_some());
            assert!(stroke(p, "boundary", "2").is_some());
            assert!(stroke(p, "waterway", "").is_some());
            // Unstyled input is skipped, not defaulted.
            assert!(fill(p, "aeroway", "").is_none());
            assert!(stroke(p, "transportation", "path").is_none());
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
            assert!(fill(p, layer, class).is_none(), "{layer}/{class} fills over imagery");
        }
        // It still draws the roads it exists for, and thicker than the vector basemap's.
        let (_, hybrid_w) = stroke(p, "transportation", "motorway").unwrap();
        let (_, dark_w) = stroke(Palette::Dark, "transportation", "motorway").unwrap();
        assert!(hybrid_w > dark_w);
        // No minor-road mesh over imagery.
        assert!(stroke(p, "transportation", "minor").is_none());
    }
}
