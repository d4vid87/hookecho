//! GRLevelX `.pal` color tables and LUT baking.
//!
//! A [`ColorTable`] is parsed from a GRLevelX `.pal` v2 file (`Product/Units/Scale/Offset/
//! Step`, `Color:/Color4:/SolidColor[4]:/RF:/ND:`, `;` comments). [`bake_lut`] turns a table
//! into a 256-entry RGBA LUT the radar shader indexes by the sweep's `u8`: index 0 =
//! transparent (below floor / below threshold), 1 = range-folded (`RF` color), 2..=255 = the
//! value band. The legend (`ui::legend`) samples the SAME table, so map and legend agree.
//!
//! The six built-in tables live as real `.pal` files in `data/colortables/` and go through
//! the exact same parser, so there is one color-table code path. User files replace them via
//! the Palettes settings tab (U3).

use std::sync::LazyLock;
use wxdata::level2::Moment;

const VALUE_ALPHA: u8 = 217; // ~0.85, matches pre-U3 built-in opacity
const FOLD_ALPHA: u8 = 179; // ~0.70
const DEFAULT_RF: [u8; 3] = [128, 128, 128];

/// One color stop, values already converted to the moment's INTERNAL units (see
/// [`parse_pal`] — `Scale`/`Offset` are applied at parse time so downstream is unit-agnostic).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PalStop {
    /// Lowest physical value (internal units) this stop covers.
    pub value: f32,
    /// Primary color (sRGB, alpha 255 unless the file gave one).
    pub rgba: [u8; 4],
    /// Second color for a two-color `Color:` line: a hard break reached at the next stop.
    pub end: Option<[u8; 4]>,
    /// `SolidColor` line: flat fill across the band, no interpolation.
    pub solid: bool,
}

/// A parsed GRLevelX color table.
#[derive(Debug, Clone, PartialEq)]
pub struct ColorTable {
    pub product: Option<String>,
    pub units: Option<String>,
    /// Legend tick spacing (internal units), if the file declared `Step:`.
    pub step: Option<f32>,
    /// Range-fold color (`RF:` line) — LUT index 1.
    pub rf: [u8; 3],
    /// Stops sorted ascending by value.
    pub stops: Vec<PalStop>,
}

impl ColorTable {
    /// Color for a physical value (internal units), or `None` below the lowest stop.
    ///
    /// GR semantics: the lowest stop is the display floor. `SolidColor` stops are flat; a
    /// plain `Color:` stop interpolates toward its second color (hard break) or the next
    /// stop's color across the band. Above the top stop clamps to the top color.
    pub fn sample(&self, v: f32) -> Option<[u8; 4]> {
        if self.stops.is_empty() || v < self.stops[0].value {
            return None;
        }
        // Last stop whose value is <= v.
        let idx = self.stops.partition_point(|s| s.value <= v) - 1;
        let s = &self.stops[idx];
        if s.solid {
            return Some(s.rgba);
        }
        let next = self.stops.get(idx + 1);
        let (hi_val, hi_col) = match (s.end, next) {
            (Some(e), Some(n)) => (n.value, e), // two-color line: hard break at next stop
            (Some(e), None) => return Some(e),  // top stop, clamp to its end color
            (None, Some(n)) => (n.value, n.rgba),
            (None, None) => return Some(s.rgba), // lone top stop
        };
        let span = (hi_val - s.value).abs().max(f32::EPSILON);
        let t = ((v - s.value) / span).clamp(0.0, 1.0);
        Some(lerp_rgba(s.rgba, hi_col, t))
    }
}

/// Bake a 256×1 RGBA LUT (row-major, 4 bytes/entry) for `table` over the data `range`.
///
/// `range` is the moment's fixed `value_range` (data quantization is set at bin time); each
/// raw index 2..=255 maps linearly into it, then through `table.sample`. `threshold`
/// (internal units) forces sub-cutoff entries transparent. Index 0 stays transparent, index
/// 1 is the range-fold color.
pub fn bake_lut(table: &ColorTable, range: (f32, f32), threshold: Option<f32>) -> [u8; 1024] {
    let (vmin, vmax) = range;
    let span = (vmax - vmin).max(f32::EPSILON);
    let cutoff = threshold.unwrap_or(f32::NEG_INFINITY);

    let mut lut = [0u8; 1024];
    lut[4..8].copy_from_slice(&[table.rf[0], table.rf[1], table.rf[2], FOLD_ALPHA]);
    for raw in 2u32..=255 {
        let t = (raw as f32 - 2.0) / 253.0;
        let value = vmin + t * span;
        let Some(rgba) = table.sample(value) else { continue }; // below floor -> transparent
        let alpha = if value < cutoff {
            0
        } else {
            (rgba[3] as u16 * VALUE_ALPHA as u16 / 255) as u8
        };
        let base = (raw * 4) as usize;
        lut[base..base + 4].copy_from_slice(&[rgba[0], rgba[1], rgba[2], alpha]);
    }
    lut
}

/// Serialize a color table back to GRLevelX `.pal` text (identity scale/offset — values are
/// already internal units). Round-trips through [`parse_pal`] for the editor's Save.
pub fn to_pal_string(t: &ColorTable) -> String {
    let mut out = String::new();
    if let Some(p) = &t.product {
        out.push_str(&format!("Product: {p}\n"));
    }
    if let Some(u) = &t.units {
        out.push_str(&format!("Units: {u}\n"));
    }
    if let Some(s) = t.step {
        out.push_str(&format!("Step: {s}\n"));
    }
    out.push_str(&format!("RF: {} {} {}\n", t.rf[0], t.rf[1], t.rf[2]));
    let rgba = |c: [u8; 4]| {
        if c[3] == 255 {
            format!("{} {} {}", c[0], c[1], c[2])
        } else {
            format!("{} {} {} {}", c[0], c[1], c[2], c[3])
        }
    };
    for s in &t.stops {
        if s.solid {
            out.push_str(&format!("SolidColor: {} {}\n", s.value, rgba(s.rgba)));
        } else if let Some(end) = s.end {
            out.push_str(&format!("Color: {} {} {}\n", s.value, rgba(s.rgba), rgba(end)));
        } else {
            out.push_str(&format!("Color: {} {}\n", s.value, rgba(s.rgba)));
        }
    }
    out
}

fn lerp_rgba(a: [u8; 4], b: [u8; 4], t: f32) -> [u8; 4] {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    [l(a[0], b[0]), l(a[1], b[1]), l(a[2], b[2]), l(a[3], b[3])]
}

/// Parse a GRLevelX `.pal` v2 file. Lenient: `;` comments, whitespace/comma-separated
/// tokens, case-insensitive keys; malformed lines are warned and skipped. Errors only if the
/// result has zero stops.
///
/// `Scale`/`Offset` are applied here so every downstream value is in the moment's internal
/// units. GRLevelX convention: the *data* value is mapped through `data * scale + offset`
/// before lookup, so a file threshold converts to internal units via
/// `internal = (file - offset) / scale` (e.g. a velocity table authored in knots carries
/// `Scale: 1.9426` and its stops divide back to m/s). The built-in tables use the identity
/// (`scale=1, offset=0`).
pub fn parse_pal(text: &str) -> anyhow::Result<ColorTable> {
    let mut product = None;
    let mut units = None;
    let mut step: Option<f32> = None;
    let mut scale = 1.0f32;
    let mut offset = 0.0f32;
    let mut rf = DEFAULT_RF;
    let mut stops: Vec<PalStop> = Vec::new();

    for (lineno, raw_line) in text.lines().enumerate() {
        let line = raw_line.split(';').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut toks = line.split([' ', '\t', ',']).filter(|t| !t.is_empty());
        let Some(key) = toks.next() else { continue };
        let key = key.trim_end_matches(':').to_ascii_lowercase();
        let rest: Vec<&str> = toks.collect();
        let nums = |slice: &[&str]| -> Vec<f32> {
            slice.iter().filter_map(|t| t.parse::<f32>().ok()).collect()
        };
        let byte = |f: f32| f.round().clamp(0.0, 255.0) as u8;
        let warn = |what: &str| log::warn!(".pal line {}: malformed {} — {:?}", lineno + 1, what, line);

        match key.as_str() {
            "product" => product = Some(rest.join(" ")),
            "units" => units = Some(rest.join(" ")),
            "scale" => {
                if let Some(&v) = nums(&rest).first() {
                    scale = v;
                }
            }
            "offset" => {
                if let Some(&v) = nums(&rest).first() {
                    offset = v;
                }
            }
            "step" => step = nums(&rest).first().copied(),
            "rf" => {
                let n = nums(&rest);
                if n.len() >= 3 {
                    rf = [byte(n[0]), byte(n[1]), byte(n[2])];
                } else {
                    warn("RF");
                }
            }
            "nd" => { /* no-data: index 0 stays transparent; parsed and ignored */ }
            "color" | "solidcolor" | "color4" | "solidcolor4" => {
                let solid = key.starts_with("solid");
                let has_alpha = key.ends_with('4');
                let n = nums(&rest);
                let cw = if has_alpha { 4 } else { 3 }; // color width
                if n.len() < 1 + cw {
                    warn("color");
                    continue;
                }
                let value = n[0];
                let read = |off: usize| -> Option<[u8; 4]> {
                    if n.len() < off + cw {
                        return None;
                    }
                    Some(if has_alpha {
                        [byte(n[off]), byte(n[off + 1]), byte(n[off + 2]), byte(n[off + 3])]
                    } else {
                        [byte(n[off]), byte(n[off + 1]), byte(n[off + 2]), 255]
                    })
                };
                let rgba = read(1).unwrap();
                let end = if solid { None } else { read(1 + cw) };
                stops.push(PalStop { value, rgba, end, solid });
            }
            _ => { /* unknown key (incl. v3-only) ignored */ }
        }
    }

    if stops.is_empty() {
        anyhow::bail!("no color stops in .pal");
    }
    // Apply Scale/Offset, then sort ascending (files are often authored high-to-low).
    if scale == 0.0 {
        scale = 1.0; // lenient: a zero Scale would divide by zero
    }
    for s in &mut stops {
        s.value = (s.value - offset) / scale;
    }
    stops.sort_by(|a, b| a.value.total_cmp(&b.value));
    let step = step.map(|s| (s / scale).abs());

    Ok(ColorTable { product, units, step, rf, stops })
}

// --- Built-in defaults: real .pal files parsed through the one code path above ---

const BUILTIN_SRC: [(&str, &str); 6] = [
    ("REF", include_str!("../data/colortables/REF.pal")),
    ("VEL", include_str!("../data/colortables/VEL.pal")),
    ("SW", include_str!("../data/colortables/SW.pal")),
    ("ZDR", include_str!("../data/colortables/ZDR.pal")),
    ("PHI", include_str!("../data/colortables/PHI.pal")),
    ("RHO", include_str!("../data/colortables/RHO.pal")),
];

static BUILTINS: LazyLock<[ColorTable; 6]> = LazyLock::new(|| {
    BUILTIN_SRC.map(|(name, src)| parse_pal(src).unwrap_or_else(|e| panic!("built-in {name}.pal: {e}")))
});

/// The built-in default table for `moment`.
pub fn default_table(moment: Moment) -> &'static ColorTable {
    &BUILTINS[moment.index()]
}

/// App-owned color-table registry: one active table per moment, plus per-moment load errors.
///
/// `gen` bumps on every reload so the render sync can detect a table change and re-bake LUTs.
pub struct Palettes {
    pub tables: [ColorTable; 6],
    pub errors: [Option<String>; 6],
    pub gen: u64,
}

impl Default for Palettes {
    fn default() -> Self {
        Self { tables: BUILTINS.clone(), errors: [const { None }; 6], gen: 0 }
    }
}

impl Palettes {
    /// Table for `moment` (the loaded custom table or the built-in default).
    pub fn table(&self, moment: Moment) -> &ColorTable {
        &self.tables[moment.index()]
    }

    /// Reload each moment's table from `paths` (`None` = built-in default). A parse/read
    /// failure keeps the built-in and records the error. Bumps `gen`.
    pub fn reload(&mut self, paths: &[Option<std::path::PathBuf>; 6]) {
        for (i, moment) in Moment::ALL.into_iter().enumerate() {
            let (table, err) = match &paths[i] {
                None => (default_table(moment).clone(), None),
                Some(path) => match std::fs::read_to_string(path).map_err(|e| e.to_string()).and_then(|s| {
                    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pal3")) {
                        Err(".pal3 (GRLevelX v3) not supported".to_string())
                    } else {
                        parse_pal(&s).map_err(|e| e.to_string())
                    }
                }) {
                    Ok(t) => (t, None),
                    Err(e) => (default_table(moment).clone(), Some(e)),
                },
            };
            self.tables[i] = table;
            self.errors[i] = err;
        }
        self.gen = self.gen.wrapping_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_builtins_parse() {
        for (name, src) in BUILTIN_SRC {
            let t = parse_pal(src).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(!t.stops.is_empty(), "{name} has stops");
        }
        // And the LazyLock array builds without panicking.
        assert_eq!(default_table(Moment::Reflectivity).stops.len(), 9);
    }

    #[test]
    fn pal_string_roundtrips() {
        // Serialize a built-in table and re-parse: the stops must survive.
        let orig = default_table(Moment::Reflectivity);
        let text = to_pal_string(orig);
        let back = parse_pal(&text).expect("reparse");
        assert_eq!(back.stops, orig.stops, "stops round-trip through .pal text");
        assert_eq!(back.rf, orig.rf);
    }

    #[test]
    fn lut_layout_and_threshold() {
        let table = default_table(Moment::Reflectivity);
        let range = Moment::Reflectivity.value_range();
        let lut = bake_lut(table, range, None);
        assert_eq!(&lut[0..4], &[0, 0, 0, 0], "index 0 transparent");
        assert_eq!(&lut[4..8], &[128, 128, 128, FOLD_ALPHA], "index 1 range-fold");
        assert_eq!(lut[255 * 4 + 3], VALUE_ALPHA, "top opaque");

        let lut = bake_lut(table, range, Some(40.0));
        assert_eq!(lut[2 * 4 + 3], 0, "below-threshold entry transparent");
        assert_eq!(lut[255 * 4 + 3], VALUE_ALPHA, "top still opaque");
    }

    #[test]
    fn solid_step_function() {
        let table = default_table(Moment::Reflectivity);
        // 47 dBZ falls in the FOXweather 45-dBZ red band.
        assert_eq!(table.sample(47.0), Some([255, 0, 0, 255]));
        // Below the 10-dBZ floor -> transparent (None).
        assert_eq!(table.sample(-100.0), None);
        assert_eq!(table.sample(5.0), None);
        // Exactly at the first (gradient) stop -> its start color.
        assert_eq!(table.sample(10.0), Some([50, 230, 165, 255]));
    }

    #[test]
    fn parses_community_style_table() {
        // Descending order, six-value Color line (gradient to second color), a Color4 with
        // alpha, a SolidColor, RF, comments, and Scale/Offset.
        let src = "\
; a community table
Product: BR
Units: dBZ
Step: 10
Scale: 1
Offset: 0
RF: 100 100 100
Color: 70 255 0 255 255 255 255
Color4: 50 255 0 0 200
SolidColor: 20 0 255 0
";
        let t = parse_pal(src).unwrap();
        assert_eq!(t.rf, [100, 100, 100]);
        assert_eq!(t.step, Some(10.0));
        assert_eq!(t.stops.len(), 3);
        // Sorted ascending: 20, 50, 70.
        assert_eq!(t.stops[0].value, 20.0);
        assert!(t.stops[0].solid);
        assert_eq!(t.stops[2].value, 70.0);
        assert_eq!(t.stops[2].end, Some([255, 255, 255, 255]));
        // Color4 alpha preserved.
        assert_eq!(t.stops[1].rgba, [255, 0, 0, 200]);
    }

    #[test]
    fn interpolates_midpoint() {
        // Two-color Color line 0..10 blending black->white; midpoint ~= gray.
        let src = "Color: 0 0 0 0 255 255 255\nColor: 10 255 255 255\n";
        let t = parse_pal(src).unwrap();
        let mid = t.sample(5.0).unwrap();
        assert!((mid[0] as i16 - 128).abs() <= 2, "midpoint ~gray, got {mid:?}");
    }

    #[test]
    fn scale_offset_applied() {
        // File authored in knots, Scale 2 (data*2 = file units) -> stop at 100 kt -> 50 m/s.
        let src = "Scale: 2\nOffset: 0\nSolidColor: 100 1 2 3\nSolidColor: 0 4 5 6\n";
        let t = parse_pal(src).unwrap();
        assert_eq!(t.stops.last().unwrap().value, 50.0);
        assert_eq!(t.sample(60.0), Some([1, 2, 3, 255]));
    }

    #[test]
    fn empty_table_errors() {
        assert!(parse_pal("; only a comment\nUnits: dBZ\n").is_err());
    }
}
