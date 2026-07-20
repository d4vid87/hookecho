//! Golden tests: the Rust decoder vs. JSON emitted by MetPy's `Level3File` (see gen_golden.py).

use nexrad_level3::decode;

#[derive(serde::Deserialize)]
struct GoldenCell {
    id: String,
    x: f32,
    y: f32,
}

#[derive(serde::Deserialize)]
struct Golden {
    prod_code: i16,
    lat: f32,
    lon: f32,
    cells: Vec<GoldenCell>,
}

#[test]
fn nst_matches_metpy_golden() {
    let bytes = include_bytes!("data/nst_tlx.l3");
    let golden: Golden = serde_json::from_str(include_str!("data/nst_tlx.golden.json")).unwrap();

    let p = decode(bytes).expect("decode NST");
    assert_eq!(p.code, golden.prod_code, "product code");
    assert!((p.lat - golden.lat).abs() < 0.001, "lat {} vs {}", p.lat, golden.lat);
    assert!((p.lon - golden.lon).abs() < 0.001, "lon {} vs {}", p.lon, golden.lon);

    // Same set of cells (id + position) as MetPy, order-independent.
    let mut mine: Vec<_> = p.cells.iter().map(|c| (c.id.clone(), c.x_km, c.y_km)).collect();
    mine.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(mine.len(), golden.cells.len(), "cell count");
    for (got, want) in mine.iter().zip(&golden.cells) {
        assert_eq!(got.0, want.id, "cell id");
        assert!((got.1 - want.x).abs() < 0.01, "cell {} x {} vs {}", want.id, got.1, want.x);
        assert!((got.2 - want.y).abs() < 0.01, "cell {} y {} vs {}", want.id, got.2, want.y);
    }

    // The tabular block should carry the storm attribute table.
    let tab = p.tabular.expect("NST tabular");
    assert!(tab.contains("STORM"), "tabular has storm table: {:?}", &tab[..tab.len().min(80)]);

    // The graphic block should carry the per-cell DBZM/HGT lines.
    let graphic = p.graphic.expect("NST graphic");
    assert!(graphic.contains("DBZM"), "graphic has DBZM line: {:?}", &graphic[..graphic.len().min(80)]);

    // Packet 23 (SCIT past position) yields past-track polylines with >= 2 points each.
    assert!(!p.past_tracks.is_empty(), "NST has past tracks");
    assert!(p.past_tracks.iter().all(|t| t.len() >= 2), "each track has >= 2 points");
}

#[derive(serde::Deserialize)]
struct GoldenSample {
    rad: usize,
    bin: usize,
    level: u8,
    value: Option<f32>,
}

#[derive(serde::Deserialize)]
struct RadialGolden {
    prod_code: i16,
    lat: f32,
    lon: f32,
    nrad: usize,
    nbins: usize,
    first: u16,
    thresholds: Vec<i32>,
    max_level: u8,
    samples: Vec<GoldenSample>,
}

#[test]
fn dvl_matches_metpy_golden() {
    let p = decode(include_bytes!("data/dvl_tlx.l3")).expect("decode DVL");
    let g: RadialGolden = serde_json::from_str(include_str!("data/dvl_tlx.golden.json")).unwrap();
    assert_eq!(p.code, g.prod_code, "product code");
    assert!((p.lat - g.lat).abs() < 0.001 && (p.lon - g.lon).abs() < 0.001, "radar location");
    let r = p.radial.as_ref().expect("DVL radial array");
    assert_eq!(r.radials.len(), g.nrad, "radial count");
    assert_eq!(r.nbins as usize, g.nbins, "bin count");
    assert_eq!(r.first_bin, g.first, "first bin");
    // Thresholds decode (u16 bit patterns preserved through i16 storage).
    for (i, t) in g.thresholds.iter().enumerate() {
        assert_eq!(p.thresholds[i] as u16, *t as u16, "threshold {i}");
    }
    let maxlvl = r.radials.iter().flat_map(|rad| rad.levels.iter().copied()).max().unwrap_or(0);
    assert_eq!(maxlvl, g.max_level, "max data level");
    for s in &g.samples {
        let level = r.radials[s.rad].levels[s.bin];
        assert_eq!(level, s.level, "sampled level rad={} bin={}", s.rad, s.bin);
        match (nexrad_level3::dvl_value(level, &p.thresholds), s.value) {
            (Some(v), Some(want)) => assert!((v - want).abs() < 0.5, "DVL value {v} vs {want}"),
            (None, None) => {}
            (a, b) => panic!("DVL missing-ness mismatch: {a:?} vs {b:?}"),
        }
    }
}

#[test]
fn eet_matches_metpy_golden() {
    let p = decode(include_bytes!("data/eet_tlx.l3")).expect("decode EET");
    let g: RadialGolden = serde_json::from_str(include_str!("data/eet_tlx.golden.json")).unwrap();
    assert_eq!(p.code, g.prod_code, "product code");
    let r = p.radial.as_ref().expect("EET radial array");
    assert_eq!(r.radials.len(), g.nrad, "radial count");
    assert_eq!(r.nbins as usize, g.nbins, "bin count");
    for (i, t) in g.thresholds.iter().enumerate() {
        assert_eq!(p.thresholds[i] as u16, *t as u16, "threshold {i}");
    }
    for s in &g.samples {
        let level = r.radials[s.rad].levels[s.bin];
        assert_eq!(level, s.level, "sampled level rad={} bin={}", s.rad, s.bin);
        match (nexrad_level3::eet_value(level, &p.thresholds), s.value) {
            (Some((kft, _topped)), Some(want)) => assert!((kft - want).abs() < 0.5, "EET value {kft} vs {want}"),
            (None, None) => {}
            (a, b) => panic!("EET missing-ness mismatch: {a:?} vs {b:?}"),
        }
    }
}

#[test]
fn icd_float16_known_vectors() {
    use nexrad_level3::icd_float16;
    // exp=0 → frac/512; 0x0000 = 0.
    assert_eq!(icd_float16(0x0000), 0.0);
    // exp=16 (bias) → 2^0 * (1 + frac/1024). 0x4000 = exp 16, frac 0 → 1.0.
    assert!((icd_float16(0x4000) - 1.0).abs() < 1e-6);
    // Sign bit set negates.
    assert!((icd_float16(0xC000) + 1.0).abs() < 1e-6);
}

#[test]
fn nmd_mesocyclone_decodes() {
    let p = decode(include_bytes!("data/nmd_tlx.l3")).expect("decode NMD");
    assert_eq!(p.code, 141, "mesocyclone product code");
    assert!((p.lat - 35.333).abs() < 0.001, "radar lat");
    // NMD carries mesocyclone/MDA point features (packet 20).
    assert!(!p.meso.is_empty(), "at least one meso feature");
}
