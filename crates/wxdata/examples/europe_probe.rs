//! Opt-in provider smoke check: cargo run -p wxdata --example europe_probe -- [composite-file.h5]
use chrono::Utc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if let Some(path) = std::env::args().nth(1) {
        let grid = wxdata::opera_composite::decode_composite(std::fs::read(path)?)?;
        println!("composite {} {}x{} {}m display", grid.time, grid.width, grid.height, grid.display_resolution_m);
        for (city, lon, lat) in [("Paris", 2.35, 48.85), ("Berlin", 13.40, 52.52), ("Madrid", -3.70, 40.42), ("Sydney", 151.2, -33.9)] {
            println!("{city}: {:?} dBZ", grid.sample_nearest(lon, lat));
        }
        anyhow::ensure!(grid.sample_nearest(151.2, -33.9).is_none(), "outside coverage must be missing");
        return Ok(());
    }
    let http = reqwest::Client::new();
    let now = Utc::now();
    let composites = wxdata::opera_composite::list_composite_frames(&http, now - chrono::Duration::hours(1), now).await?;
    let latest = composites.last().ok_or_else(|| anyhow::anyhow!("no recent composites"))?;
    let grid = wxdata::opera_composite::fetch_composite(&http, latest).await?;
    println!("{} composites, newest {} ({}x{})", composites.len(), grid.time, grid.width, grid.height);
    for site in ["BEWID", "DEBO"] {
        let date = now.date_naive();
        let frames = if site == "DEBO" { wxdata::dwd::list_volumes(&http, site, date).await? }
            else { wxdata::opera::list_volumes(&http, site, date).await? };
        let frame = frames.iter().rev().nth(3).ok_or_else(|| anyhow::anyhow!("fewer than four recent {site} frames"))?;
        let (name, time, scan) = if site == "DEBO" { wxdata::dwd::fetch_volume_at(&http, frame).await? }
            else { wxdata::opera::fetch_volume_at(&http, frame).await? };
        anyhow::ensure!(name == frame.name(), "history frame changed identity");
        anyhow::ensure!((time - frame.date_time().unwrap()).num_seconds().abs() < 60, "history time differs from selected cycle");
        println!("{site}: {} frames, selected {name}, observed {time}, {} sweeps", frames.len(), scan.sweeps().len());
    }
    Ok(())
}
