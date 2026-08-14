//! Dump the radar site registry as JSON, for embedders that want a site picker without pulling in
//! this crate: `cargo run -p wxdata --example sites_json > sites.json`.
//!
//! WeatherDesk vendors the output. The registry changes about once a decade, so a generated file
//! beats an endpoint.

fn main() {
    let sites: Vec<_> = wxdata::sites::all()
        .map(|s| {
            serde_json::json!({
                "id": s.id,
                "name": format!("{}, {}", s.city, s.state),
                "lat": s.latitude,
                "lon": s.longitude,
            })
        })
        .collect();
    println!("{}", serde_json::to_string(&sites).unwrap());
}
