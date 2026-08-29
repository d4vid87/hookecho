//! Emit the radar site table the website builds its per-site radar pages from:
//! `cargo run -p wxdata --example nexrad_sites > site/src/data/nexrad-sites.json`.
//!
//! The site builds on Node alone (no Rust in .github/workflows/site.yml), so the JSON is
//! committed and CI re-runs this to check it hasn't drifted.
//!
//! Separate from `sites_json.rs` on purpose: that one dumps every network in a schema WeatherDesk
//! vendors, and this one carries the timezone and the network the page template branches on.

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
                "network": "nexrad",
                "country": "US",
            })
        })
        .collect();

    // The 44 TDWRs come from their own table (bare `sites()` is WSR-88D only). No K-alias problem
    // here — the ids are unique — but the website renders them differently, hence the field above.
    out.extend(wxdata::tdwr::SITES.iter().map(|s| {
        serde_json::json!({
            "id": s.id,
            "city": s.city,
            "state": s.state,
            "lat": round4(s.latitude),
            "lon": round4(s.longitude),
            "elev_m": s.elevation_meters,
            "tz": wxdata::tz::site_tz(s.id).map(|tz| tz.name()),
            "network": "tdwr",
            "country": "US",
        })
    }));

    // The two international networks. `state` means something different in each — a German Land
    // for the DWD rows, the country itself for OPERA's — so `country` is its own field rather
    // than something the website is left to infer from an id prefix.
    out.extend(wxdata::dwd::SITES.iter().map(|s| {
        serde_json::json!({
            "id": s.id,
            "city": s.city,
            "state": s.state,
            "lat": round4(s.latitude),
            "lon": round4(s.longitude),
            "elev_m": s.elevation_meters,
            "tz": wxdata::tz::site_tz(s.id).map(|tz| tz.name()),
            "network": "dwd",
            "country": "DE",
        })
    }));
    out.extend(wxdata::opera::SITES.iter().map(|s| {
        serde_json::json!({
            "id": s.id,
            "city": s.city,
            "state": s.state,
            "lat": round4(s.latitude),
            "lon": round4(s.longitude),
            "elev_m": s.elevation_meters,
            "tz": wxdata::tz::site_tz(s.id).map(|tz| tz.name()),
            "network": "opera",
            "country": s.state,
        })
    }));
    out.sort_by_key(|s| s["id"].as_str().unwrap().to_string());
    // The registry also carries a straight duplicate row (KCCX appears twice, identical), which
    // would give the website two pages fighting over one URL.
    out.dedup_by_key(|s| s["id"].as_str().unwrap().to_string());

    println!("{}", serde_json::to_string_pretty(&out).unwrap());
}
