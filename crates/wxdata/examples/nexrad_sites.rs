//! Emit the WSR-88D site table the website builds its per-site radar pages from:
//! `cargo run -p wxdata --example nexrad_sites > site/src/data/nexrad-sites.json`.
//!
//! The site builds on Node alone (no Rust in .github/workflows/site.yml), so the JSON is
//! committed and CI re-runs this to check it hasn't drifted.
//!
//! Separate from `sites_json.rs` on purpose: that one dumps every network in a schema WeatherDesk
//! vendors, and this one is NEXRAD-only with the timezone folded in.

fn round4(v: f32) -> f64 {
    (v as f64 * 1e4).round() / 1e4
}

fn main() {
    let sites = wxdata::sites::sites();

    // ponytail: the registry lists Alaska twice — a legacy K-prefixed row beside the real P id
    // (KABC/PABC, both "Bethel" at the same coordinates). RIDGE serves the P id, so drop any K row
    // whose P twin exists rather than hand-maintaining an alias list.
    let mut out: Vec<_> = sites
        .iter()
        .filter(|s| {
            !(s.id.starts_with('K') && sites.iter().any(|o| o.id == format!("P{}", &s.id[1..])))
        })
        .map(|s| {
            serde_json::json!({
                "id": s.id,
                "city": s.city,
                "state": s.state,
                // f32 registry values print as 35.333099365234375 otherwise; 4 decimals is ~10 m.
                "lat": round4(s.latitude),
                "lon": round4(s.longitude),
                "elev_m": s.elevation_meters,
                "tz": wxdata::tz::site_tz(s.id).map(|tz| tz.name()),
            })
        })
        .collect();
    out.sort_by_key(|s| s["id"].as_str().unwrap().to_string());

    println!("{}", serde_json::to_string_pretty(&out).unwrap());
}
