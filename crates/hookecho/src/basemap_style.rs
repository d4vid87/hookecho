//! Color/width styling for the OpenMapTiles vector basemap (dark + light).
//!
//! Pure lookups keyed by MVT layer name + a per-layer "class" discriminator (the caller pulls
//! the right property: `class` for transportation, `admin_level` for boundary, empty otherwise).
//! Colors are opaque sRGB `[r,g,b,a]`; the tessellator converts to linear. Only styled features
//! produce geometry — an unstyled layer/class returns `None` and is skipped.

/// 0xRRGGBB -> opaque RGBA.
const fn rgb(hex: u32) -> [u8; 4] {
    [(hex >> 16) as u8, (hex >> 8) as u8, hex as u8, 255]
}

/// Whole-tile constants (drawn before per-feature geometry).
pub struct VecStyle {
    /// Land quad under everything.
    pub background: [u8; 4],
    /// City/town label text.
    pub label: [u8; 4],
    /// Label halo/outline for contrast.
    pub label_halo: [u8; 4],
}

const DARK: VecStyle =
    VecStyle { background: rgb(0x111318), label: rgb(0xc8d0da), label_halo: rgb(0x0b0d11) };
const LIGHT: VecStyle =
    VecStyle { background: rgb(0xf2efe9), label: rgb(0x3a3a3a), label_halo: rgb(0xf7f5f0) };

pub fn style(dark: bool) -> &'static VecStyle {
    if dark {
        &DARK
    } else {
        &LIGHT
    }
}

/// Fill color for a polygon feature, or `None` to skip it.
pub fn fill(dark: bool, layer: &str, class: &str) -> Option<[u8; 4]> {
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
pub fn stroke(dark: bool, layer: &str, class: &str) -> Option<([u8; 4], f32)> {
    let (motorway, primary, secondary, minor, rail, water, admin2, admin4) = if dark {
        (0x3d4450, 0x353c47, 0x2c323b, 0x23282f, 0x2a2f36, 0x24333f, 0x5a6470, 0x3a424c)
    } else {
        (0xf6d3a0, 0xf9dfb0, 0xe9e3d5, 0xdedacf, 0xcdc9c0, 0xa9c4d6, 0x9aa0a8, 0xc4c0b8)
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
        assert_ne!(style(true).background, style(false).background);
        // Every documented class resolves in both themes.
        for dark in [true, false] {
            assert!(fill(dark, "water", "").is_some());
            assert!(fill(dark, "landcover", "wood").is_some());
            assert!(fill(dark, "landuse", "residential").is_some());
            assert!(fill(dark, "park", "").is_some());
            assert!(stroke(dark, "transportation", "motorway").is_some());
            assert!(stroke(dark, "transportation", "minor").is_some());
            assert!(stroke(dark, "waterway", "").is_some());
            assert!(stroke(dark, "boundary", "2").is_some());
            assert!(stroke(dark, "boundary", "4").is_some());
        }
        // Unstyled -> skipped.
        assert!(fill(true, "building", "").is_none());
        assert!(stroke(true, "transportation", "path").is_none());
        assert!(stroke(true, "boundary", "8").is_none());
    }
}
