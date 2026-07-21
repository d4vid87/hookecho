use crate::aws::realtime::ChunkType;
use chrono::Duration;
use nexrad_decode::messages::volume_coverage_pattern::{ChannelConfiguration, WaveformType};
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};

/// Maximum number of timing samples to keep per chunk characteristics
const MAX_TIMING_SAMPLES: usize = 10;

/// Characteristics of a chunk that affect timing
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ChunkCharacteristics {
    /// Type of the chunk
    pub chunk_type: ChunkType,
    /// Waveform type of the elevation
    pub waveform_type: WaveformType,
    /// Channel configuration of the elevation
    pub channel_configuration: ChannelConfiguration,
}

impl Hash for ChunkCharacteristics {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(&self.chunk_type).hash(state);
        std::mem::discriminant(&self.waveform_type).hash(state);
        std::mem::discriminant(&self.channel_configuration).hash(state);
    }
}

/// Statistics for a single timing sample
#[derive(Debug, Clone, Copy)]
pub(crate) struct TimingStat {
    /// Duration of the timing sample
    duration: Duration,
    /// Number of attempts to download the chunk
    attempts: usize,
}

/// Statistics for timing between chunks
#[derive(Debug, Clone, Default)]
pub struct ChunkTimingStats {
    /// Timing statistics for each chunk characteristics
    timings: HashMap<ChunkCharacteristics, VecDeque<TimingStat>>,
}

impl ChunkTimingStats {
    /// Create a new empty timing statistics
    pub fn new() -> Self {
        Self {
            timings: HashMap::new(),
        }
    }

    /// Add a timing sample for the given chunk characteristics
    pub fn add_timing(
        &mut self,
        characteristics: ChunkCharacteristics,
        duration: Duration,
        attempts: usize,
    ) {
        let entry = self.timings.entry(characteristics).or_default();

        entry.push_back(TimingStat { duration, attempts });

        // Maintain the rolling window by removing oldest if we exceed the max
        if entry.len() > MAX_TIMING_SAMPLES {
            entry.pop_front();
        }
    }

    /// Get the average timing for the given chunk characteristics
    pub(crate) fn get_average_timing(
        &self,
        characteristics: &ChunkCharacteristics,
    ) -> Option<Duration> {
        self.timings.get(characteristics).and_then(|timings| {
            if timings.is_empty() {
                return None;
            }

            let total_millis: i64 = timings
                .iter()
                .map(|timing| timing.duration.num_milliseconds())
                .sum();

            let avg_millis = total_millis / timings.len() as i64;
            Some(Duration::milliseconds(avg_millis))
        })
    }

    /// Get the average number of attempts for the given chunk characteristics
    pub(crate) fn get_average_attempts(
        &self,
        characteristics: &ChunkCharacteristics,
    ) -> Option<f64> {
        self.timings.get(characteristics).and_then(|timings| {
            if timings.is_empty() {
                return None;
            }

            let total_attempts: usize = timings.iter().map(|timing| timing.attempts).sum();
            Some(total_attempts as f64 / timings.len() as f64)
        })
    }

    /// Get all chunk statistics for display purposes
    pub fn get_statistics(&self) -> Vec<(ChunkCharacteristics, Option<Duration>, Option<f64>)> {
        self.timings
            .keys()
            .map(|characteristics| {
                (
                    *characteristics,
                    self.get_average_timing(characteristics),
                    self.get_average_attempts(characteristics),
                )
            })
            .collect()
    }
}
