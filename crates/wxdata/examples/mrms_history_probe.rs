//! Verify that the public MRMS archive exposes recent frames that can be replayed.
//! `cargo run -p wxdata --example mrms_history_probe -- CARIB/MergedReflectivityQCComposite_00.50`
//! Add an RFC 3339 UTC instant as the second argument to inspect an archived window.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let product = std::env::args().nth(1).unwrap_or_else(|| wxdata::mrms::REFLECTIVITY.to_owned());
    let http = reqwest::Client::new();
    let at = std::env::args().nth(2)
        .map(|value| chrono::DateTime::parse_from_rfc3339(&value).map(|time| time.to_utc()))
        .transpose()?;
    let keys = if let Some(at) = at {
        wxdata::mrms::history_keys_before(&http, &product, at, 4).await?
    } else {
        wxdata::mrms::recent_keys(&http, &product, 4).await?
    };
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
