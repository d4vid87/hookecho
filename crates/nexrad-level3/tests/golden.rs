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

#[test]
fn nmd_mesocyclone_decodes() {
    let p = decode(include_bytes!("data/nmd_tlx.l3")).expect("decode NMD");
    assert_eq!(p.code, 141, "mesocyclone product code");
    assert!((p.lat - 35.333).abs() < 0.001, "radar lat");
    // NMD carries mesocyclone/MDA point features (packet 20).
    assert!(!p.meso.is_empty(), "at least one meso feature");
}
