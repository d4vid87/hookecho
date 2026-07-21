use crate::aws::realtime::list_chunks_in_volume::list_chunks_in_volume;
use crate::aws::realtime::search::search;
use crate::aws::realtime::VolumeIndex;
use chrono::{DateTime, Utc};
use std::sync::atomic::AtomicI32;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;

/// Identifies the volume index with the most recent data for the specified radar site. Real-time
/// NEXRAD data is uploaded to a series of rotating volumes 0..=999, each containing ~55 chunks.
/// This function performs a binary search to find the most recent volume with data.
pub async fn get_latest_volume(site: &str) -> crate::result::Result<LatestVolumeResult> {
    let calls = Arc::new(AtomicI32::new(0));
    let latest_volume = search(998, DateTime::<Utc>::MAX_UTC, |volume| {
        calls.fetch_add(1, Relaxed);
        async move {
            let chunks = list_chunks_in_volume(site, VolumeIndex::new(volume + 1), 1).await?;
            Ok(chunks.first().and_then(|chunk| chunk.upload_date_time()))
        }
    })
    .await
    .map(|volume| volume.map(|index| VolumeIndex::new(index + 1)))?;

    Ok(LatestVolumeResult {
        volume: latest_volume,
        calls: calls.load(Relaxed) as usize,
    })
}

/// Represents the most recent volume index and the number of network calls made to find it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LatestVolumeResult {
    /// The most recent volume index, if found.
    pub volume: Option<VolumeIndex>,

    /// The number of network calls made to find the most recent volume.
    pub calls: usize,
}
