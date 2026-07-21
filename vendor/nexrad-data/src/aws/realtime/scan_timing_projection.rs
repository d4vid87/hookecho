use crate::aws::realtime::{
    ChunkCharacteristics, ChunkIdentifier, ChunkTimingModel, ChunkTimingStats, ElevationChunkMapper,
};
use chrono::{DateTime, Duration, Utc};
use nexrad_decode::messages::volume_coverage_pattern;

/// A projected timeline for all remaining chunks in a volume scan.
///
/// Built from the VCP's azimuth rates and elevation angles using a physics-based model,
/// optionally refined with historical timing observations. This enables UIs to show
/// projected timelines for the entire remaining scan, not just the next chunk.
#[derive(Debug, Clone)]
pub struct ScanTimingProjection {
    /// The sequence number of the anchor chunk (the last observed chunk).
    anchor_sequence: usize,
    /// The anchor time (upload time of the anchor chunk, or current time).
    anchor_time: DateTime<Utc>,
    /// Projected timing for each future chunk, in sequence order.
    chunks: Vec<ChunkProjection>,
    /// Projected time when the final chunk becomes available.
    volume_end_time: DateTime<Utc>,
    /// Projected total remaining duration from anchor to volume end.
    remaining_duration: Duration,
}

impl ScanTimingProjection {
    /// The sequence number of the anchor chunk this projection is relative to.
    pub fn anchor_sequence(&self) -> usize {
        self.anchor_sequence
    }

    /// The anchor time this projection is relative to.
    pub fn anchor_time(&self) -> DateTime<Utc> {
        self.anchor_time
    }

    /// Projected timing for each future chunk, in sequence order.
    pub fn chunks(&self) -> &[ChunkProjection] {
        &self.chunks
    }

    /// Projected time when the final chunk becomes available.
    pub fn volume_end_time(&self) -> DateTime<Utc> {
        self.volume_end_time
    }

    /// Projected remaining duration from anchor to volume end.
    pub fn remaining_duration(&self) -> Duration {
        self.remaining_duration
    }
}

/// Projection for a single future chunk.
#[derive(Debug, Clone)]
pub struct ChunkProjection {
    /// The chunk's sequence number.
    sequence: usize,
    /// The elevation number (1-based), or None for the Start chunk.
    elevation_number: Option<usize>,
    /// Elevation angle in degrees (0.0 for the Start chunk).
    elevation_angle_deg: f64,
    /// Projected time this chunk becomes available in S3.
    projected_time: DateTime<Utc>,
    /// Duration from the anchor to this chunk's projected availability.
    offset_from_anchor: Duration,
    /// Duration from the previous chunk to this chunk.
    interval_from_previous: Duration,
    /// Whether this chunk starts a new sweep (useful for UI grouping).
    starts_new_sweep: bool,
}

impl ChunkProjection {
    /// The chunk's sequence number.
    pub fn sequence(&self) -> usize {
        self.sequence
    }

    /// The elevation number (1-based), or None for the Start chunk.
    pub fn elevation_number(&self) -> Option<usize> {
        self.elevation_number
    }

    /// Elevation angle in degrees.
    pub fn elevation_angle_deg(&self) -> f64 {
        self.elevation_angle_deg
    }

    /// Projected time this chunk becomes available in S3.
    pub fn projected_time(&self) -> DateTime<Utc> {
        self.projected_time
    }

    /// Duration from the anchor to this chunk's projected availability.
    pub fn offset_from_anchor(&self) -> Duration {
        self.offset_from_anchor
    }

    /// Duration from the previous chunk to this chunk.
    pub fn interval_from_previous(&self) -> Duration {
        self.interval_from_previous
    }

    /// Whether this chunk starts a new sweep.
    pub fn starts_new_sweep(&self) -> bool {
        self.starts_new_sweep
    }
}

/// Build a timing projection for all remaining chunks in the current volume.
///
/// The projection starts from `anchor_chunk` (the most recently observed chunk) and
/// projects forward through the final chunk in the volume. Each chunk's projected
/// availability time is computed using the VCP's azimuth rates, elevation angles,
/// and optionally historical timing data.
///
/// Returns `None` if the anchor chunk's metadata cannot be resolved or if there are
/// no remaining chunks to project.
pub fn project_scan_timing(
    anchor_chunk: &ChunkIdentifier,
    _vcp: &volume_coverage_pattern::Message,
    mapper: &ElevationChunkMapper,
    timing_stats: Option<&ChunkTimingStats>,
) -> Option<ScanTimingProjection> {
    let anchor_sequence = anchor_chunk.sequence();
    let anchor_time = anchor_chunk.upload_date_time().unwrap_or_else(Utc::now);
    let final_sequence = mapper.final_sequence();

    // Nothing to project if we're at or past the final chunk
    if anchor_sequence >= final_sequence {
        return None;
    }

    let anchor_metadata = mapper.get_chunk_metadata(anchor_sequence)?;

    let mut projections = Vec::new();
    let mut cumulative_offset_ms: i64 = 0;
    let mut prev_metadata = anchor_metadata;

    for seq in (anchor_sequence + 1)..=final_sequence {
        let next_metadata = mapper.get_chunk_metadata(seq)?;

        // Compute interval using physics model
        let mut interval_secs =
            ChunkTimingModel::estimate_chunk_interval_secs(prev_metadata, next_metadata);

        // Blend with historical data if available
        if let Some(stats) = timing_stats {
            if let Some(elev_num) = next_metadata.elevation_number() {
                if let Some(elev_data) = _vcp.elevations().get(elev_num - 1) {
                    let characteristics = ChunkCharacteristics {
                        chunk_type: crate::aws::realtime::ChunkType::Intermediate,
                        waveform_type: elev_data.waveform_type(),
                        channel_configuration: elev_data.channel_configuration(),
                    };

                    if let Some(avg_timing) = stats.get_average_timing(&characteristics) {
                        let historical_secs = avg_timing.num_milliseconds() as f64 / 1000.0;
                        // Blend: 70% physics, 30% historical
                        interval_secs = interval_secs * 0.7 + historical_secs * 0.3;
                    }
                }
            }
        }

        let interval_ms = (interval_secs * 1000.0) as i64;
        cumulative_offset_ms += interval_ms;

        let interval_duration = Duration::milliseconds(interval_ms);
        let offset_duration = Duration::milliseconds(cumulative_offset_ms);
        let projected_time = anchor_time + offset_duration;

        projections.push(ChunkProjection {
            sequence: seq,
            elevation_number: next_metadata.elevation_number(),
            elevation_angle_deg: next_metadata.elevation_angle_deg(),
            projected_time,
            offset_from_anchor: offset_duration,
            interval_from_previous: interval_duration,
            starts_new_sweep: next_metadata.is_first_in_sweep(),
        });

        prev_metadata = next_metadata;
    }

    let volume_end_time = projections
        .last()
        .map(|p| p.projected_time)
        .unwrap_or(anchor_time);
    let remaining_duration = Duration::milliseconds(cumulative_offset_ms);

    Some(ScanTimingProjection {
        anchor_sequence,
        anchor_time,
        chunks: projections,
        volume_end_time,
        remaining_duration,
    })
}

/// Build a timing projection for an entire volume from the Start chunk.
///
/// This projects all chunks from sequence 1 through the final sequence, useful when
/// starting a fresh volume and wanting to display the full expected timeline.
///
/// The `start_time` parameter is the time the Start chunk was uploaded (or current time).
pub fn project_full_scan_timing(
    site: &str,
    volume: crate::aws::realtime::VolumeIndex,
    start_time: DateTime<Utc>,
    vcp: &volume_coverage_pattern::Message,
    mapper: &ElevationChunkMapper,
    timing_stats: Option<&ChunkTimingStats>,
) -> Option<ScanTimingProjection> {
    let start_chunk = ChunkIdentifier::new(
        site.to_string(),
        volume,
        start_time.naive_utc(),
        1,
        crate::aws::realtime::ChunkType::Start,
        Some(start_time),
    );

    project_scan_timing(&start_chunk, vcp, mapper, timing_stats)
}
