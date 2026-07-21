use crate::aws::realtime::ChunkMetadata;

/// Sweep duration bias correction in seconds.
///
/// Sweeps consistently finish ~0.67s before a full 360-degree rotation would predict,
/// because the last radial at ~359.5 degrees means the sweep is slightly short of a
/// full circle. Derived from analysis of 801 sweep observations across 59 volumes.
const SWEEP_DURATION_BIAS_SECS: f64 = 0.67;

/// Base overhead for inter-sweep transitions in seconds.
///
/// This represents the minimum time for mode switching between sweeps when the
/// antenna doesn't need to change elevation (e.g., CS to CDW at same angle).
const INTER_SWEEP_BASE_GAP_SECS: f64 = 0.7;

/// Seconds per degree of elevation change during inter-sweep transitions.
///
/// Represents the antenna slew rate during transitions between sweeps at different
/// elevation angles. Combined with the base gap: gap = 0.7 + (|delta_elev| * 0.08).
const INTER_SWEEP_ELEVATION_RATE_SECS_PER_DEG: f64 = 0.08;

/// Inter-volume gap in seconds.
///
/// Time between the last radial of one volume and the first radial of the next.
/// Includes antenna return to starting elevation plus initialization overhead.
/// Derived from analysis: mean ~8.5s, range 7-10s.
const INTER_VOLUME_GAP_SECS: f64 = 8.5;

/// Physics-based timing model for predicting chunk and sweep timing from VCP parameters.
///
/// All predictions are derived from analysis of 59 archive volumes across 12 NEXRAD sites,
/// 8 VCP types, and diverse meteorological scenarios. The azimuth rate from the VCP is the
/// dominant predictor, achieving a mean absolute error of 0.33s for sweep duration prediction.
pub struct ChunkTimingModel;

impl ChunkTimingModel {
    /// Predicted sweep duration in seconds based on azimuth rotation rate.
    ///
    /// Formula: `(360 / azimuth_rate_dps) - 0.67`
    ///
    /// Returns `None` if `azimuth_rate_dps` is zero or negative.
    pub fn sweep_duration_secs(azimuth_rate_dps: f64) -> Option<f64> {
        if azimuth_rate_dps <= 0.0 {
            return None;
        }
        Some((360.0 / azimuth_rate_dps) - SWEEP_DURATION_BIAS_SECS)
    }

    /// Predicted duration for a single chunk within a sweep.
    ///
    /// Chunks divide a sweep evenly: `sweep_duration / chunks_in_sweep`.
    ///
    /// Returns `None` if `azimuth_rate_dps` is zero or negative, or `chunks_in_sweep` is zero.
    pub fn chunk_duration_secs(azimuth_rate_dps: f64, chunks_in_sweep: usize) -> Option<f64> {
        if chunks_in_sweep == 0 {
            return None;
        }
        Self::sweep_duration_secs(azimuth_rate_dps).map(|d| d / chunks_in_sweep as f64)
    }

    /// Predicted inter-sweep gap in seconds based on elevation angle change.
    ///
    /// Formula: `0.7 + (|from_elevation - to_elevation| * 0.08)`
    ///
    /// The base 0.7s represents mode switching overhead. The 0.08s per degree represents
    /// antenna slew rate during transitions (much faster than the survey rotation rate).
    pub fn inter_sweep_gap_secs(from_elevation_deg: f64, to_elevation_deg: f64) -> f64 {
        let elevation_change = (to_elevation_deg - from_elevation_deg).abs();
        INTER_SWEEP_BASE_GAP_SECS + (elevation_change * INTER_SWEEP_ELEVATION_RATE_SECS_PER_DEG)
    }

    /// Predicted inter-volume gap in seconds (constant 8.5s).
    pub fn inter_volume_gap_secs() -> f64 {
        INTER_VOLUME_GAP_SECS
    }

    /// Estimate the time interval in seconds between two consecutive chunks.
    ///
    /// Three cases:
    /// 1. **Start chunk** (inter-volume): Returns the inter-volume gap (~8.5s).
    /// 2. **First chunk in a new sweep** (inter-sweep): Chunk duration + inter-sweep gap.
    /// 3. **Intra-sweep chunk**: Pure chunk duration (sweep_duration / chunks_in_sweep).
    ///
    /// Falls back to static defaults if the azimuth rate is zero or unavailable.
    pub fn estimate_chunk_interval_secs(previous: &ChunkMetadata, next: &ChunkMetadata) -> f64 {
        // Case 1: Start chunk (beginning of new volume)
        if next.is_start_chunk() {
            return Self::inter_volume_gap_secs();
        }

        // Get the chunk duration for the next chunk's sweep
        let chunk_duration =
            Self::chunk_duration_secs(next.azimuth_rate_dps(), next.chunks_in_sweep());

        // Case 2: First chunk in a new sweep (inter-sweep transition)
        if next.is_first_in_sweep() {
            let gap = Self::inter_sweep_gap_secs(
                previous.elevation_angle_deg(),
                next.elevation_angle_deg(),
            );

            return match chunk_duration {
                Some(d) => d + gap,
                None => gap + Self::fallback_chunk_duration_secs(),
            };
        }

        // Case 3: Intra-sweep chunk (same elevation, continuous rotation)
        chunk_duration.unwrap_or(Self::fallback_chunk_duration_secs())
    }

    /// Fallback chunk duration when azimuth rate is unavailable.
    ///
    /// Uses the midpoint of observed chunk durations (~4s) as a conservative default.
    fn fallback_chunk_duration_secs() -> f64 {
        4.0
    }
}
