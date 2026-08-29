//! The two built-in GRLevelX `.pal` tables, trimmed to what the lite viewer needs.
//!
//! This is a deliberate copy of the parts of `hookecho::colormap` that matter here — parsing a
//! `.pal` and sampling it. Linking the app crate would drag wgpu and egui into a bundle whose
//! whole point is being small; the `.pal` files themselves stay single-source (`include_str!`
//! from the app's data directory), so the colors can never drift.

/// One color stop, values already in the product's units (`Scale`/`Offset` applied at parse).
struct Stop {
    value: f32,
    rgb: [u8; 3],
    /// Second color of a two-color `Color:` line — a hard break reached at the next stop.
    end: Option<[u8; 3]>,
    /// `SolidColor:` line — flat across the band, no interpolation.
    solid: bool,
}

pub struct Table {
    /// Range-folded color (`RF:` line), used for velocity data level 1.
    pub rf: [u8; 3],
    stops: Vec<Stop>,
}

impl Table {
    /// Color for a physical value, or `None` below the lowest stop (the display floor).
    fn sample(&self, v: f32) -> Option<[u8; 3]> {
        if self.stops.is_empty() || v < self.stops[0].value {
            return None;
        }
        let i = self.stops.partition_point(|s| s.value <= v) - 1;
        let s = &self.stops[i];
        if s.solid {
            return Some(s.rgb);
        }
        let next = self.stops.get(i + 1);
        let (hi_val, hi_col) = match (s.end, next) {
            (Some(e), Some(n)) => (n.value, e),
            (Some(e), None) => return Some(e),
            (None, Some(n)) => (n.value, n.rgb),
            (None, None) => return Some(s.rgb),
        };
        let span = (hi_val - s.value).abs().max(f32::EPSILON);
        let t = ((v - s.value) / span).clamp(0.0, 1.0);
        Some([
            lerp(s.rgb[0], hi_col[0], t),
            lerp(s.rgb[1], hi_col[1], t),
            lerp(s.rgb[2], hi_col[2], t),
        ])
    }

    /// Bake a 256-entry RGBA LUT indexed by the product's raw data level.
    ///
    /// Level 0 is below threshold (transparent) and level 1 is range-folded (the `RF` color for
    /// velocity, transparent for reflectivity, which never folds). Levels 2.. decode linearly
    /// through the tenths thresholds, so the table is baked once per frame and the render loop is
    /// a pure lookup.
    pub fn bake(&self, thr: &[i16; 16], folded: bool) -> [u8; 1024] {
        let mut lut = [0u8; 1024];
        if folded {
            lut[4..8].copy_from_slice(&[self.rf[0], self.rf[1], self.rf[2], 179]);
        }
        for level in 2u32..=255 {
            let Some(v) = nexrad_level3::n0b_value(level as u8, thr) else {
                continue;
            };
            let Some(c) = self.sample(v) else { continue };
            let b = (level * 4) as usize;
            lut[b..b + 4].copy_from_slice(&[c[0], c[1], c[2], 217]);
        }
        lut
    }
}

fn lerp(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t)
        .round()
        .clamp(0.0, 255.0) as u8
}

/// Parse a GRLevelX `.pal` v2 file. Only the keys the two built-in tables use are recognized;
/// anything else is skipped, same as the app's parser.
fn parse(text: &str) -> Table {
    let mut rf = [128u8, 128, 128];
    let (mut scale, mut offset) = (1.0f32, 0.0f32);
    let mut stops: Vec<Stop> = Vec::new();

    for raw in text.lines() {
        let line = raw.split(';').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut toks = line.split([' ', '\t', ',']).filter(|t| !t.is_empty());
        let Some(key) = toks.next() else { continue };
        let key = key.trim_end_matches(':').to_ascii_lowercase();
        let n: Vec<f32> = toks.filter_map(|t| t.parse::<f32>().ok()).collect();
        let byte = |f: f32| f.round().clamp(0.0, 255.0) as u8;
        match key.as_str() {
            "scale" => scale = n.first().copied().filter(|s| *s != 0.0).unwrap_or(1.0),
            "offset" => offset = n.first().copied().unwrap_or(0.0),
            "rf" if n.len() >= 3 => rf = [byte(n[0]), byte(n[1]), byte(n[2])],
            "color" | "solidcolor" | "color4" | "solidcolor4" => {
                let solid = key.starts_with("solid");
                let w = if key.ends_with('4') { 4 } else { 3 }; // color width (alpha ignored)
                if n.len() < 1 + w {
                    continue;
                }
                let read = |o: usize| -> Option<[u8; 3]> {
                    (n.len() >= o + w).then(|| [byte(n[o]), byte(n[o + 1]), byte(n[o + 2])])
                };
                stops.push(Stop {
                    value: n[0],
                    rgb: read(1).unwrap(),
                    end: if solid { None } else { read(1 + w) },
                    solid,
                });
            }
            _ => {}
        }
    }
    for s in &mut stops {
        s.value = (s.value - offset) / scale;
    }
    stops.sort_by(|a, b| a.value.total_cmp(&b.value));
    Table { rf, stops }
}

pub fn reflectivity() -> Table {
    parse(include_str!("../../hookecho/data/colortables/REF.pal"))
}

pub fn velocity() -> Table {
    parse(include_str!("../../hookecho/data/colortables/VEL.pal"))
}
