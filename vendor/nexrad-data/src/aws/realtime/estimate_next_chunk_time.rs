use crate::aws::realtime::{
    ChunkCharacteristics, ChunkIdentifier, ChunkTimingModel, ChunkTimingStats, ChunkType,
    ElevationChunkMapper,
};
use chrono::Duration as ChronoDuration;
use chrono::{DateTime, Utc};
use log::debug;
use nexrad_decode::messages::volume_coverage_pattern;

/// Attempts to estimate the time at which the next chunk will be available given the previous
/// chunk. Requires an [ElevationChunkMapper] to describe the relationship between chunk sequence
/// and VCP elevations. A None result indicates that the chunk is already available or that an
/// estimate cannot be made.
///
/// The estimate is anchored to the previous chunk's upload time rather than the current time,
/// so querying late will correctly yield a past time (indicating the chunk should already be
/// available) rather than pushing the estimate forward.
pub fn estimate_chunk_availability_time(
    chunk: &ChunkIdentifier,
    vcp: &volume_coverage_pattern::Message,
    elevation_chunk_mapper: &ElevationChunkMapper,
    timing_stats: Option<&ChunkTimingStats>,
) -> Option<DateTime<Utc>> {
    let processing_time =
        estimate_chunk_processing_time(chunk, vcp, elevation_chunk_mapper, timing_stats)?;

    let anchor = chunk.upload_date_time().unwrap_or_else(Utc::now);
    let availability_time = anchor + processing_time;

    Some(availability_time)
}

/// Attempts to estimate the time the given chunk will take to become available in the real-time S3
/// bucket following the previous chunk. Requires an [ElevationChunkMapper] to describe the
/// relationship between chunk sequence and VCP elevations. A None result indicates that an estimate
/// cannot be made.
pub fn estimate_chunk_processing_time(
    chunk: &ChunkIdentifier,
    vcp: &volume_coverage_pattern::Message,
    elevation_chunk_mapper: &ElevationChunkMapper,
    timing_stats: Option<&ChunkTimingStats>,
) -> Option<ChronoDuration> {
    // Start chunks: use the inter-volume gap model
    if chunk.chunk_type() == ChunkType::Start {
        let gap_ms = (ChunkTimingModel::inter_volume_gap_secs() * 1000.0) as i64;
        return Some(ChronoDuration::milliseconds(gap_ms));
    }

    // Try to use the physics-based model via chunk metadata
    if let Some(next_metadata) = elevation_chunk_mapper.get_chunk_metadata(chunk.sequence() + 1) {
        let current_metadata = elevation_chunk_mapper.get_chunk_metadata(chunk.sequence());

        // Check for historical timing data first
        if let Some(elevation) = elevation_chunk_mapper
            .get_sequence_elevation_number(chunk.sequence())
            .and_then(|elevation_number| vcp.elevations().get(elevation_number - 1))
        {
            let characteristics = ChunkCharacteristics {
                chunk_type: chunk.chunk_type(),
                waveform_type: elevation.waveform_type(),
                channel_configuration: elevation.channel_configuration(),
            };

            let average_timing =
                timing_stats.and_then(|stats| stats.get_average_timing(&characteristics));
            let average_attempts =
                timing_stats.and_then(|stats| stats.get_average_attempts(&characteristics));

            if let (Some(avg_timing), Some(avg_attempts)) = (average_timing, average_attempts) {
                let mut wait_time = avg_timing;
                wait_time += chrono::Duration::seconds(avg_attempts as i64 - 1);

                debug!(
                    "Using historical average timing of {}ms and {} attempts for {}ms",
                    avg_timing.num_milliseconds(),
                    avg_attempts,
                    wait_time.num_milliseconds()
                );

                return Some(wait_time);
            }
        }

        // Fall back to physics-based model
        if let Some(current) = current_metadata {
            let interval_secs =
                ChunkTimingModel::estimate_chunk_interval_secs(current, next_metadata);
            let interval_ms = (interval_secs * 1000.0) as i64;

            debug!(
                "Using physics model: interval={}ms (az_rate={:.1} dps, first_in_sweep={})",
                interval_ms,
                next_metadata.azimuth_rate_dps(),
                next_metadata.is_first_in_sweep()
            );

            return Some(ChronoDuration::milliseconds(interval_ms));
        }
    }

    // Final fallback: use old static estimation for edge cases where metadata is unavailable
    if let Some(elevation) = elevation_chunk_mapper
        .get_sequence_elevation_number(chunk.sequence())
        .and_then(|elevation_number| vcp.elevations().get(elevation_number - 1))
    {
        let wait_time = get_legacy_default_wait_time(
            elevation.waveform_type(),
            elevation.channel_configuration(),
        );

        debug!(
            "No metadata available, using legacy static estimation of {}ms",
            wait_time.num_milliseconds()
        );

        return Some(wait_time);
    }

    None
}

/// Legacy default wait time based on waveform type and channel configuration.
///
/// Only used as a last resort when chunk metadata is unavailable (should be rare).
fn get_legacy_default_wait_time(
    waveform_type: nexrad_decode::messages::volume_coverage_pattern::WaveformType,
    channel_config: nexrad_decode::messages::volume_coverage_pattern::ChannelConfiguration,
) -> ChronoDuration {
    use nexrad_decode::messages::volume_coverage_pattern::{ChannelConfiguration, WaveformType};

    if waveform_type == WaveformType::CS {
        ChronoDuration::seconds(11)
    } else if channel_config == ChannelConfiguration::ConstantPhase {
        ChronoDuration::seconds(7)
    } else {
        ChronoDuration::seconds(4)
    }
}
