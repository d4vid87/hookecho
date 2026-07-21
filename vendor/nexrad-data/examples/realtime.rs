#![cfg(all(feature = "aws-polling", not(target_arch = "wasm32")))]

use chrono::{DateTime, SubsecRound, Utc};
use clap::Parser;
use env_logger::{Builder, Env};
use futures::StreamExt;
use log::{info, warn, LevelFilter};
use nexrad_data::result::Result;
use nexrad_data::{
    aws::realtime::{chunk_stream, Chunk, DownloadedChunk, PollConfig},
    volume,
};
use nexrad_decode::summarize;
use std::pin::pin;
use std::time::Duration;
use tokio::time::timeout;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Site identifier (e.g., KDMX)
    #[arg(default_value = "KDMX")]
    site: String,

    /// The number of chunks to download
    #[arg(default_value = "10")]
    chunk_count: usize,
}

#[tokio::main]
async fn main() -> Result<()> {
    Builder::from_env(Env::default().default_filter_or("info"))
        .filter_module("reqwest::connect", LevelFilter::Info)
        .init();

    let cli = Cli::parse();
    let desired_chunk_count = cli.chunk_count;

    info!("Starting real-time polling for site {}", cli.site);

    println!(
        "{:<25} | {:<25} | {:<13} | {:<13} | {:<13} | {:<8}",
        "Chunk", "Downloaded", "AWS Latency", "First Radial", "Last Radial", "Attempts"
    );
    println!("{:-<110}", "");

    let config = PollConfig::new(&cli.site);
    let stream = chunk_stream(config);
    let mut stream = pin!(stream);

    let mut downloaded_chunk_count = 0;

    let result = timeout(Duration::from_secs(300), async {
        while let Some(result) = stream.next().await {
            match result {
                Ok(downloaded) => {
                    let download_time = Utc::now();
                    process_chunk(&downloaded, download_time);

                    downloaded_chunk_count += 1;
                    if downloaded_chunk_count >= desired_chunk_count {
                        info!("Downloaded {} chunks, stopping...", desired_chunk_count);
                        break;
                    }
                }
                Err(e) => {
                    warn!("Error downloading chunk: {:?}", e);
                }
            }
        }
    })
    .await;

    match result {
        Ok(()) => info!("Finished downloading chunks"),
        Err(_) => info!("Timeout reached, stopping..."),
    }

    Ok(())
}

fn process_chunk(downloaded: &DownloadedChunk, download_time: DateTime<Utc>) {
    let chunk = &downloaded.chunk;

    match chunk {
        Chunk::Start(file) => {
            let records = file.records().expect("records");
            records
                .into_iter()
                .for_each(|record| decode_record(downloaded, record, download_time));
        }
        Chunk::IntermediateOrEnd(record) => {
            decode_record(downloaded, record.clone(), download_time);
        }
    }
}

fn decode_record(
    downloaded: &DownloadedChunk,
    mut record: volume::Record,
    download_time: DateTime<Utc>,
) {
    let chunk_id = &downloaded.identifier;

    if record.compressed() {
        record = record.decompress().expect("Failed to decompress record");
    }

    let messages = match record.messages() {
        Ok(msgs) => msgs,
        Err(e) => {
            warn!("Failed to decode messages: {e}");
            return;
        }
    };

    let summary = summarize::messages(messages.as_slice());

    let first_radial_latency = summary
        .earliest_collection_time
        .map(|time| {
            format!(
                "{:.2}s",
                (download_time - time).num_milliseconds() as f64 / 1000.0
            )
        })
        .unwrap_or_else(|| "N/A".to_string());

    let last_radial_latency = summary
        .latest_collection_time
        .map(|time| {
            format!(
                "{:.2}s",
                (download_time - time).num_milliseconds() as f64 / 1000.0
            )
        })
        .unwrap_or_else(|| "N/A".to_string());

    let rounded_download_time = download_time.round_subsecs(0);
    let aws_latency = chunk_id
        .upload_date_time()
        .map(|time| {
            format!(
                "{:.2}s",
                (rounded_download_time - time).num_milliseconds() as f64 / 1000.0
            )
        })
        .unwrap_or_else(|| "N/A".to_string());

    println!(
        "{:<25} | {:<25} | {:<13} | {:<13} | {:<13} | {:<8}",
        format!("{}/{}", chunk_id.volume().as_number(), chunk_id.name()),
        download_time.format("%Y-%m-%d %H:%M:%S%.3f"),
        aws_latency,
        first_radial_latency,
        last_radial_latency,
        downloaded.attempts
    );
}
