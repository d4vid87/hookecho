//! Profile the native decoded-loop cache with independent copies of one live TDWR scan.
//!
//! `cargo run -p hookecho --example tdwr_cache_profile -- TDFW 250`
//! This isolates decoded retention from 250 archive downloads and playback rendering.

use hookecho::app::{NATIVE_LOOP_WINDOW, NATIVE_RADAR_PAYLOAD_BUDGET, NATIVE_SCAN_CACHE_CAPACITY};
use hookecho::radar_memory::Inventory;
use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Instant;

fn rss_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|line| line.starts_with("VmRSS:"))?;
    line.split_whitespace()
        .nth(1)?
        .parse::<u64>()
        .ok()
        .map(|kib| kib * 1024)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let site = args.next().unwrap_or_else(|| "TDFW".into());
    let count = args.next().as_deref().unwrap_or("250").parse::<usize>()?;
    anyhow::ensure!(
        (1..=NATIVE_LOOP_WINDOW).contains(&count),
        "count must be between 1 and {NATIVE_LOOP_WINDOW}"
    );

    let http = reqwest::Client::new();
    let fetched_at = Instant::now();
    let (volume_name, _time, scan, _revision) =
        wxdata::tdwr::fetch_volume(&http, &site, None, None)
            .await?
            .ok_or_else(|| anyhow::anyhow!("no current TDWR volume"))?;
    let fetch_ms = fetched_at.elapsed().as_millis();
    let sample = Arc::new(scan);
    let mut inventory = Inventory::default();
    let sample_payload_bytes = inventory.scan_bytes(&sample);
    let baseline_rss_bytes = rss_bytes();

    let mut cache = LruCache::new(NonZeroUsize::new(NATIVE_SCAN_CACHE_CAPACITY).unwrap());
    let inserted_at = Instant::now();
    for index in 0..count {
        cache.put(index, Arc::new((*sample).clone()));
    }
    let clone_insert_ms = inserted_at.elapsed().as_millis();
    let payload_bytes = inventory.bytes(cache.iter().map(|(_, scan)| scan));
    let filled_rss_bytes = rss_bytes();

    let read_at = Instant::now();
    let hits = (0..count)
        .filter(|index| cache.get(index).is_some())
        .count();
    let full_pass_us = read_at.elapsed().as_micros();

    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "site": site,
            "volume_name": volume_name,
            "copies": count,
            "cache_capacity": NATIVE_SCAN_CACHE_CAPACITY,
            "cache_entries": cache.len(),
            "cache_hits_in_full_pass": hits,
            "sample_encoded_payload_bytes": sample_payload_bytes,
            "retained_encoded_payload_bytes": payload_bytes,
            "encoded_payload_budget_bytes": NATIVE_RADAR_PAYLOAD_BUDGET,
            "within_encoded_payload_budget": payload_bytes <= NATIVE_RADAR_PAYLOAD_BUDGET,
            "baseline_rss_bytes": baseline_rss_bytes,
            "filled_rss_bytes": filled_rss_bytes,
            "fetch_ms": fetch_ms,
            "clone_insert_ms": clone_insert_ms,
            "full_cache_pass_us": full_pass_us,
            "scope": "One live scan cloned into independent decoded allocations; no historical fetch, render, GPU or total application latency is measured."
        }))?
    );
    Ok(())
}
