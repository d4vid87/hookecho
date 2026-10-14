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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
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
    /// Black land, neutral geography and bright labels for weather presentations.
    Presentation,
}

impl Palette {
    /// Every selectable vector look, in menu order.
    pub const ALL: [Palette; 7] = [
        Palette::Dark,
        Palette::Light,
        Palette::Liberty,
        Palette::Bright,
        Palette::Positron,
        Palette::Midnight,
        Palette::Presentation,
    ];
}

/// Optional overrides for one administrative boundary class. Width is in base screen pixels;
/// the accessibility theme may enlarge it. `None` keeps the selected palette's own appearance.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct BoundaryAppearance {
    pub enabled: bool,
    pub color: Option<[u8; 3]>,
    pub width: Option<f32>,
}

impl Default for BoundaryAppearance {
    fn default() -> Self { Self { enabled: true, color: None, width: None } }
}

impl BoundaryAppearance {
    fn normalized(mut self) -> Self {
        self.width = self.width.filter(|v| v.is_finite()).map(|v| v.clamp(0.25, 6.0));
        self
    }

    fn apply(self, base: Option<([u8; 4], f32)>) -> Option<([u8; 4], f32)> {
        if !self.enabled { return None; }
        let (mut color, mut width) = base?;
        if let Some([r, g, b]) = self.color { color = [r, g, b, 255]; }
        if let Some(value) = self.normalized().width { width = value; }
        Some((color, width))
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CityRoadVisibility {
    #[default]
    Always,
    /// Show geographic labels and roads only when the radar-site markers are hidden.
    WithoutRadarSites,
    Hidden,
}

/// Persisted map-display preferences. Defaults keep every existing palette unchanged.
#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct MapDetailSettings {
    pub states: BoundaryAppearance,
    pub counties: BoundaryAppearance,
    pub city_roads: CityRoadVisibility,
}

impl MapDetailSettings {
    /// Resolve the conditional visibility before sending a geometry job to a worker. This
    /// depends on tower-marker visibility, never on whether a radar station is selected.
    pub fn resolve(self, show_radar_sites: bool) -> MapDetailStyle {
        MapDetailStyle {
            states: self.states.normalized(),
            counties: self.counties.normalized(),
            show_city_roads: match self.city_roads {
                CityRoadVisibility::Always => true,
                CityRoadVisibility::WithoutRadarSites => !show_radar_sites,
                CityRoadVisibility::Hidden => false,
            },
        }
    }
}

/// Fully resolved geometry-job configuration; travels with both native and browser worker jobs.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct MapDetailStyle {
    pub states: BoundaryAppearance,
    pub counties: BoundaryAppearance,
    pub show_city_roads: bool,
}

impl Default for MapDetailStyle {
    fn default() -> Self { MapDetailSettings::default().resolve(true) }
}

impl MapDetailStyle {
    pub fn normalized(self) -> Self {
        Self { states: self.states.normalized(), counties: self.counties.normalized(), ..self }
    }
}

/// Apply administrative-boundary and road visibility preferences to actual vector geometry.
/// Country borders and weather overlays remain independent of these controls.
pub fn stroke_with_details(p: Palette, layer: &str, class: &str, details: MapDetailStyle)
    -> Option<([u8; 4], f32)>
{
    if layer == "transportation" && !details.show_city_roads { return None; }
    let base = stroke(p, layer, class);
    if layer == "boundary" {
        match class {
            "3" | "4" => return details.states.apply(base),
            "6" => return details.counties.apply(base),
            _ => {}
        }
    }
    base
}

pub fn casing_with_details(p: Palette, layer: &str, class: &str, details: MapDetailStyle)
    -> Option<([u8; 4], f32)>
{
    if layer == "transportation" && !details.show_city_roads { return None; }
    casing(p, layer, class)
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
    water: 0x142536,
    wood: 0x121a18,
    park: 0x14201b,
    residential: 0x171a20,
    building: 0x20242c,
    motorway: 0x7a7164,
    primary: 0x696258,
    secondary: 0x77828e,
    minor: 0x4c5661,
    rail: 0x303842,
    admin2: 0x8995a3,
    admin4: 0x46515e,
    county: 0x303945,
    casing: 0x090c11,
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

// Independently authored for the OpenMapTiles schema; no provider style assets required.
const C_PRESENTATION: Colors = Colors {
    water: 0x06121b,
    wood: 0x000000,
    park: 0x000000,
    residential: 0x000000,
    building: 0x101010,
    motorway: 0x686868,
    primary: 0x555555,
    secondary: 0x454545,
    minor: 0x303030,
    rail: 0x242424,
    admin2: 0x8c8c8c,
    admin4: 0x707070,
    county: 0x383838,
    casing: 0x000000,
};

fn colors(p: Palette) -> &'static Colors {
    match p {
        Palette::Dark => &C_DARK,
        Palette::Light => &C_LIGHT,
        Palette::Liberty => &C_LIBERTY,
        Palette::Bright => &C_BRIGHT,
        Palette::Positron => &C_POSITRON,
        Palette::Midnight => &C_MIDNIGHT,
        Palette::Presentation => &C_PRESENTATION,
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
    background: Some(rgb(0x0f1217)),
    label: rgb(0xd5dbe3),
    label_halo: rgb(0x07090d),
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

const PRESENTATION: VecStyle = VecStyle {
    background: Some(rgb(0x000000)),
    label: rgb(0xffffff),
    label_halo: rgb(0x000000),
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
        Palette::Presentation => &PRESENTATION,
    }
}

/// Regional roads stay hairlines; street-level views retain the full road hierarchy.
/// Apply to both the road and its casing so the outline never overwhelms the fill.
pub fn road_scale(zoom: f64) -> f64 {
    0.25 + 0.75 * ((zoom - 5.0) / 8.0).clamp(0.0, 1.0)
}

/// Casing (outline) color + width for a road, drawn in a pass *under* the road itself so majors
/// read as ribbons rather than bare lines. `None` for classes that get no casing.
///
/// Width here is the total width of the casing stroke, so it has to exceed the road's own or the
/// road would cover it entirely.
pub fn casing(p: Palette, layer: &str, class: &str) -> Option<([u8; 4], f32)> {
    if layer != "transportation" || p == Palette::Presentation {
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
                "motorway" | "trunk" => (0xffe9a8, 4.6),
                "primary" => (0xfff4d2, 3.8),
                "secondary" | "tertiary" => (0xf0f0f0, 3.0),
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
            "motorway" => (motorway, 4.0),
            "trunk" => (motorway, 3.6),
            "primary" => (primary, 3.2),
            "secondary" => (secondary, 2.8),
            "tertiary" => (secondary, 2.4),
            "minor" | "street" => (minor, 2.0),
            "service" | "track" => (minor, 1.5),
            "path" | "footway" | "cycleway" | "pedestrian" => (minor, 0.9),
            "rail" | "transit" => (rail, 0.9),
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
    let w = if p == Palette::Presentation {
        match layer {
            "boundary" => match class {
                "2" => 1.8,
                "3" | "4" => 1.4,
                _ => 0.5,
            },
            "transportation" => w * 0.6,
            _ => w * 0.7,
        }
    } else {
        w
    };
    Some((rgb(c), w))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detail_preferences_preserve_defaults_and_roundtrip_partial_json() {
        let defaults = MapDetailSettings::default().resolve(true);
        for p in Palette::ALL {
            for (layer, class) in [("boundary", "2"), ("boundary", "4"), ("boundary", "6"),
                ("transportation", "motorway"), ("waterway", "")] {
                assert_eq!(stroke_with_details(p, layer, class, defaults), stroke(p, layer, class));
            }
        }
        let settings: MapDetailSettings = serde_json::from_str(
            r#"{"states":{"enabled":false},"city_roads":"WithoutRadarSites"}"#).unwrap();
        assert!(!settings.states.enabled);
        assert!(settings.counties.enabled);
        assert!(!settings.resolve(true).show_city_roads);
        assert!(settings.resolve(false).show_city_roads);
        assert_eq!(serde_json::from_str::<MapDetailSettings>(&serde_json::to_string(&settings).unwrap()).unwrap(), settings);
        let resolved = settings.resolve(true);
        assert_eq!(postcard::from_bytes::<MapDetailStyle>(&postcard::to_allocvec(&resolved).unwrap()).unwrap(), resolved);
    }

    #[test]
    fn administrative_overrides_are_independent_and_bounded() {
        let details = MapDetailStyle {
            states: BoundaryAppearance { enabled: false, ..Default::default() },
            counties: BoundaryAppearance { enabled: true, color: Some([40, 90, 150]), width: Some(2.5) },
            show_city_roads: false,
        };
        for class in ["3", "4"] { assert!(stroke_with_details(Palette::Presentation, "boundary", class, details).is_none()); }
        assert_eq!(stroke_with_details(Palette::Presentation, "boundary", "6", details), Some(([40,90,150,255], 2.5)));
        assert!(stroke_with_details(Palette::Presentation, "boundary", "2", details).is_some());
        assert!(stroke_with_details(Palette::Dark, "transportation", "motorway", details).is_none());
        assert!(casing_with_details(Palette::Dark, "transportation", "motorway", details).is_none());
        assert!(stroke_with_details(Palette::Dark, "waterway", "", details).is_some());
        for (input, expected) in [(Some(f32::NAN), None), (Some(-5.0), Some(0.25)), (Some(90.0), Some(6.0))] {
            let boundary = BoundaryAppearance { width: input, ..Default::default() };
            assert_eq!(boundary.normalized().width, expected);
        }
    }

    #[test]
    fn regional_roads_are_thin_and_grow_to_street_scale() {
        assert_eq!(road_scale(3.0), 0.25);
        let width = stroke(Palette::Dark, "transportation", "motorway").unwrap().1;
        assert!(road_scale(5.3) * f64::from(width) < 1.2);
        assert!(road_scale(9.0) > road_scale(5.3));
        assert_eq!(road_scale(13.0), 1.0);
        assert_eq!(road_scale(18.0), 1.0);
    }

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
