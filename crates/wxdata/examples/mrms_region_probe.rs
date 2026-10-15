//! Probe NOAA's public regional MRMS composite grids and their geographic bounds.
//! `cargo run -p wxdata --example mrms_region_probe`

#[tokio::main]
async fn main() {
    let http = reqwest::Client::new();
    let mut results = Vec::new();
    for region in ["ALASKA", "HAWAII", "GUAM", "CARIB"] {
        let product = format!("{region}/MergedReflectivityQCComposite_00.50");
        let result = match wxdata::mrms::fetch_latest(&http, &product).await {
            Ok(grid) => serde_json::json!({
                "region": region,
                "product": product,
                "nx": grid.nx,
                "ny": grid.ny,
                "lon_west": grid.lon_west,
                "lon_east": grid.lon_east,
                "lat_south": grid.lat_south,
                "lat_north": grid.lat_north,
                "finite_values": grid.values.iter().filter(|v| v.is_finite()).count(),
                "time": grid.time,
            }),
            Err(error) => serde_json::json!({"region": region, "error": error.to_string()}),
        };
        results.push(result);
    }
    println!("{}", serde_json::to_string_pretty(&results).unwrap());
}
