//! Verify that the public MRMS archive exposes recent frames that can be replayed.
//! `cargo run -p wxdata --example mrms_history_probe -- CARIB/MergedReflectivityQCComposite_00.50`

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let product = std::env::args().nth(1).unwrap_or_else(|| wxdata::mrms::REFLECTIVITY.to_owned());
    let http = reqwest::Client::new();
    let keys = wxdata::mrms::recent_keys(&http, &product, 4).await?;
    let mut frames = Vec::new();
    for key in keys {
        let frame = wxdata::mrms::fetch_frame_key(&http, &product, &key).await?;
        frames.push(serde_json::json!({
            "key": key,
            "valid_time": frame.stamp.valid_time,
            "grid": [frame.grid.nx, frame.grid.ny],
            "finite_values": frame.field().values.iter().filter(|value| value.is_finite()).count(),
        }));
    }
    println!("{}", serde_json::to_string_pretty(&frames)?);
    Ok(())
}
