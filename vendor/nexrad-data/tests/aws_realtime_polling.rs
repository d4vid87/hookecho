#![cfg(feature = "aws")]

use chrono::Duration;
use nexrad_data::aws::realtime::{
    ChunkCharacteristics, ChunkTimingModel, ChunkTimingStats, ChunkType, ElevationChunkMapper,
};
use nexrad_decode::messages::volume_coverage_pattern::{ChannelConfiguration, WaveformType};

// Test data file
const TEST_NEXRAD_FILE: &[u8] = include_bytes!("../../downloads/KDMX20220305_232324_V06");

// Get a VCP from the test file
fn get_test_vcp() -> nexrad_decode::messages::volume_coverage_pattern::Message<'static> {
    let volume_file = nexrad_data::volume::File::new(TEST_NEXRAD_FILE.to_vec());

    for mut record in volume_file.records().expect("records") {
        if record.compressed() {
            record = record.decompress().unwrap();
        }
        for message in record.messages().unwrap() {
            if let nexrad_decode::messages::MessageContents::VolumeCoveragePattern(vcp) =
                message.contents()
            {
                return vcp.clone().into_owned();
            }
        }
    }
    panic!("No VCP found in test file");
}

#[test]
fn test_elevation_chunk_mapper_creation() {
    let vcp = get_test_vcp();
    let mapper = ElevationChunkMapper::new(&vcp);

    // Mapper should have a valid final sequence
    let final_seq = mapper.final_sequence();
    assert!(final_seq > 0, "Final sequence should be positive");

    // For typical VCPs, final sequence is usually in the 40-80 range
    assert!(
        final_seq < 200,
        "Final sequence seems unreasonably large: {}",
        final_seq
    );
}

#[test]
fn test_elevation_chunk_mapper_sequence_1_is_metadata() {
    let vcp = get_test_vcp();
    let mapper = ElevationChunkMapper::new(&vcp);

    // Sequence 1 is the metadata chunk, not an elevation scan
    let elevation = mapper.get_sequence_elevation_number(1);
    assert_eq!(elevation, None, "Sequence 1 should not map to an elevation");
}

#[test]
fn test_elevation_chunk_mapper_sequence_2_is_first_elevation() {
    let vcp = get_test_vcp();
    let mapper = ElevationChunkMapper::new(&vcp);

    // Sequence 2 should be the first elevation scan
    let elevation = mapper.get_sequence_elevation_number(2);
    assert_eq!(
        elevation,
        Some(1),
        "Sequence 2 should map to first elevation"
    );
}

#[test]
fn test_elevation_chunk_mapper_all_sequences_valid() {
    let vcp = get_test_vcp();
    let mapper = ElevationChunkMapper::new(&vcp);

    let final_seq = mapper.final_sequence();

    // All sequences from 2 to final should map to some elevation
    for seq in 2..=final_seq {
        let elevation = mapper.get_sequence_elevation_number(seq);
        assert!(
            elevation.is_some(),
            "Sequence {} should map to an elevation",
            seq
        );

        let elev_num = elevation.unwrap();
        assert!(
            elev_num >= 1 && elev_num <= vcp.elevations().len(),
            "Elevation number {} out of range for sequence {}",
            elev_num,
            seq
        );
    }
}

#[test]
fn test_elevation_chunk_mapper_final_sequence() {
    let vcp = get_test_vcp();
    let mapper = ElevationChunkMapper::new(&vcp);

    let final_seq = mapper.final_sequence();

    // Final sequence should map to last elevation
    let elevation = mapper.get_sequence_elevation_number(final_seq);
    assert!(
        elevation.is_some(),
        "Final sequence should map to an elevation"
    );
    assert_eq!(
        elevation.unwrap(),
        vcp.elevations().len(),
        "Final sequence should map to last elevation"
    );
}

#[test]
fn test_elevation_chunk_mapper_beyond_final_sequence() {
    let vcp = get_test_vcp();
    let mapper = ElevationChunkMapper::new(&vcp);

    let final_seq = mapper.final_sequence();

    // Sequences beyond final should return None
    let elevation = mapper.get_sequence_elevation_number(final_seq + 1);
    assert_eq!(
        elevation, None,
        "Sequence beyond final should not map to elevation"
    );

    let elevation = mapper.get_sequence_elevation_number(final_seq + 100);
    assert_eq!(
        elevation, None,
        "Far beyond final sequence should not map to elevation"
    );
}

// === ChunkMetadata tests ===

#[test]
fn test_chunk_metadata_sequence_1_is_start() {
    let vcp = get_test_vcp();
    let mapper = ElevationChunkMapper::new(&vcp);

    let meta = mapper.get_chunk_metadata(1).expect("sequence 1 metadata");
    assert!(meta.is_start_chunk());
    assert_eq!(meta.elevation_number(), None);
    assert_eq!(meta.sequence(), 1);
}

#[test]
fn test_chunk_metadata_first_elevation() {
    let vcp = get_test_vcp();
    let mapper = ElevationChunkMapper::new(&vcp);

    let meta = mapper.get_chunk_metadata(2).expect("sequence 2 metadata");
    assert!(!meta.is_start_chunk());
    assert_eq!(meta.elevation_number(), Some(1));
    assert_eq!(meta.chunk_index_in_sweep(), 0);
    assert!(meta.is_first_in_sweep());
    assert!(!meta.is_last_in_sweep());
    assert!(meta.azimuth_rate_dps() > 0.0);

    // First elevation of VCP 212 is super-res (0.5 deg azimuth), so 6 chunks
    let first_elev = &vcp.elevations()[0];
    if first_elev.super_resolution_half_degree_azimuth() {
        assert_eq!(meta.chunks_in_sweep(), 6);
    } else {
        assert_eq!(meta.chunks_in_sweep(), 3);
    }
}

#[test]
fn test_chunk_metadata_sweep_boundaries() {
    let vcp = get_test_vcp();
    let mapper = ElevationChunkMapper::new(&vcp);

    let all = mapper.all_chunk_metadata();

    // Skip Start chunk (index 0)
    let mut current_elev = None;
    for meta in &all[1..] {
        if meta.elevation_number() != current_elev {
            // New sweep started
            assert!(
                meta.is_first_in_sweep(),
                "Chunk seq {} should be first in sweep for elev {:?}",
                meta.sequence(),
                meta.elevation_number()
            );
            current_elev = meta.elevation_number();
        }

        if meta.chunk_index_in_sweep() == meta.chunks_in_sweep() - 1 {
            assert!(
                meta.is_last_in_sweep(),
                "Chunk seq {} should be last in sweep",
                meta.sequence()
            );
        }
    }
}

#[test]
fn test_chunk_metadata_total_matches_final_sequence() {
    let vcp = get_test_vcp();
    let mapper = ElevationChunkMapper::new(&vcp);

    // total_chunks includes the Start chunk
    assert_eq!(mapper.total_chunks(), mapper.final_sequence());

    // Last metadata entry should have sequence == final_sequence
    let all = mapper.all_chunk_metadata();
    assert_eq!(all.last().unwrap().sequence(), mapper.final_sequence());
}

#[test]
fn test_chunk_metadata_out_of_range() {
    let vcp = get_test_vcp();
    let mapper = ElevationChunkMapper::new(&vcp);

    assert!(mapper.get_chunk_metadata(0).is_none());
    assert!(mapper
        .get_chunk_metadata(mapper.final_sequence() + 1)
        .is_none());
}

#[test]
fn test_chunk_metadata_azimuth_rates_match_vcp() {
    let vcp = get_test_vcp();
    let mapper = ElevationChunkMapper::new(&vcp);

    // For each elevation, all chunks should have the same azimuth rate from the VCP
    for (elev_idx, elev_data) in vcp.elevations().iter().enumerate() {
        let elev_num = elev_idx + 1;
        let expected_rate = elev_data.azimuth_rate();

        for meta in mapper.all_chunk_metadata() {
            if meta.elevation_number() == Some(elev_num) {
                assert!(
                    (meta.azimuth_rate_dps() - expected_rate).abs() < 0.001,
                    "Elevation {} azimuth rate mismatch: meta={}, vcp={}",
                    elev_num,
                    meta.azimuth_rate_dps(),
                    expected_rate
                );
            }
        }
    }
}

// === ChunkCharacteristics tests ===

#[test]
fn test_chunk_characteristics_equality() {
    let char1 = ChunkCharacteristics {
        chunk_type: ChunkType::Intermediate,
        waveform_type: WaveformType::CS,
        channel_configuration: ChannelConfiguration::ConstantPhase,
    };

    let char2 = ChunkCharacteristics {
        chunk_type: ChunkType::Intermediate,
        waveform_type: WaveformType::CS,
        channel_configuration: ChannelConfiguration::ConstantPhase,
    };

    let char3 = ChunkCharacteristics {
        chunk_type: ChunkType::End,
        waveform_type: WaveformType::CS,
        channel_configuration: ChannelConfiguration::ConstantPhase,
    };

    assert_eq!(char1, char2);
    assert_ne!(char1, char3);
}

#[test]
fn test_chunk_characteristics_hash() {
    use std::collections::HashSet;

    let char1 = ChunkCharacteristics {
        chunk_type: ChunkType::Intermediate,
        waveform_type: WaveformType::CS,
        channel_configuration: ChannelConfiguration::ConstantPhase,
    };

    let char2 = ChunkCharacteristics {
        chunk_type: ChunkType::Intermediate,
        waveform_type: WaveformType::CS,
        channel_configuration: ChannelConfiguration::ConstantPhase,
    };

    let mut set = HashSet::new();
    set.insert(char1);
    set.insert(char2);

    // char1 and char2 are equal, so set should have only 1 element
    assert_eq!(set.len(), 1);
}

// === ChunkTimingStats tests ===

#[test]
fn test_chunk_timing_stats_new() {
    let stats = ChunkTimingStats::new();

    // New stats should be empty
    let all_stats = stats.get_statistics();
    assert_eq!(all_stats.len(), 0, "New stats should be empty");
}

#[test]
fn test_chunk_timing_stats_add_and_retrieve() {
    let mut stats = ChunkTimingStats::new();

    let characteristics = ChunkCharacteristics {
        chunk_type: ChunkType::Intermediate,
        waveform_type: WaveformType::CS,
        channel_configuration: ChannelConfiguration::ConstantPhase,
    };

    // Add a timing sample
    let duration = Duration::seconds(5);
    stats.add_timing(characteristics, duration, 1);

    // Should be able to retrieve it via get_statistics
    let all_stats = stats.get_statistics();
    assert_eq!(all_stats.len(), 1);

    // Verify the returned stats
    let (char, avg_timing, avg_attempts) = &all_stats[0];
    assert_eq!(char, &characteristics);
    assert_eq!(*avg_timing, Some(Duration::seconds(5)));
    assert_eq!(*avg_attempts, Some(1.0));
}

#[test]
fn test_chunk_timing_stats_average() {
    let mut stats = ChunkTimingStats::new();

    let characteristics = ChunkCharacteristics {
        chunk_type: ChunkType::Intermediate,
        waveform_type: WaveformType::CS,
        channel_configuration: ChannelConfiguration::ConstantPhase,
    };

    // Add multiple samples
    stats.add_timing(characteristics, Duration::seconds(4), 1);
    stats.add_timing(characteristics, Duration::seconds(6), 1);
    stats.add_timing(characteristics, Duration::seconds(5), 1);

    // Get statistics
    let all_stats = stats.get_statistics();
    assert_eq!(all_stats.len(), 1);

    let (_char, avg_timing, avg_attempts) = &all_stats[0];

    // Average should be 5 seconds
    assert_eq!(*avg_timing, Some(Duration::seconds(5)));
    // Average attempts should be 1.0
    assert_eq!(*avg_attempts, Some(1.0));
}

#[test]
fn test_chunk_timing_stats_different_characteristics() {
    let mut stats = ChunkTimingStats::new();

    let char1 = ChunkCharacteristics {
        chunk_type: ChunkType::Intermediate,
        waveform_type: WaveformType::CS,
        channel_configuration: ChannelConfiguration::ConstantPhase,
    };

    let char2 = ChunkCharacteristics {
        chunk_type: ChunkType::End,
        waveform_type: WaveformType::CS,
        channel_configuration: ChannelConfiguration::ConstantPhase,
    };

    stats.add_timing(char1, Duration::seconds(5), 1);
    stats.add_timing(char2, Duration::seconds(10), 2);

    // Should track them separately
    let all_stats = stats.get_statistics();
    assert_eq!(all_stats.len(), 2);

    // Find stats for each characteristic
    let mut found_char1 = false;
    let mut found_char2 = false;

    for (char, avg_timing, _avg_attempts) in all_stats {
        if char == char1 {
            assert_eq!(avg_timing, Some(Duration::seconds(5)));
            found_char1 = true;
        } else if char == char2 {
            assert_eq!(avg_timing, Some(Duration::seconds(10)));
            found_char2 = true;
        }
    }

    assert!(
        found_char1 && found_char2,
        "Should find both characteristics"
    );
}

#[test]
fn test_chunk_timing_stats_rolling_window() {
    let mut stats = ChunkTimingStats::new();

    let characteristics = ChunkCharacteristics {
        chunk_type: ChunkType::Intermediate,
        waveform_type: WaveformType::CS,
        channel_configuration: ChannelConfiguration::ConstantPhase,
    };

    // Add more than MAX_TIMING_SAMPLES (which is 10)
    for i in 1..=15 {
        stats.add_timing(characteristics, Duration::seconds(i), 1);
    }

    // Should only keep the last 10 samples
    let all_stats = stats.get_statistics();
    assert_eq!(all_stats.len(), 1);

    let (_char, avg_timing, _avg_attempts) = &all_stats[0];
    assert!(avg_timing.is_some());

    // Average of last 10 samples (6-15) is 10.5 seconds
    // (6 + 7 + 8 + 9 + 10 + 11 + 12 + 13 + 14 + 15) / 10 = 105 / 10 = 10.5
    assert_eq!(*avg_timing, Some(Duration::milliseconds(10500)));
}

#[test]
fn test_chunk_timing_stats_attempts_tracking() {
    let mut stats = ChunkTimingStats::new();

    let characteristics = ChunkCharacteristics {
        chunk_type: ChunkType::Intermediate,
        waveform_type: WaveformType::CS,
        channel_configuration: ChannelConfiguration::ConstantPhase,
    };

    // Add samples with different attempt counts
    stats.add_timing(characteristics, Duration::seconds(5), 1);
    stats.add_timing(characteristics, Duration::seconds(5), 2);
    stats.add_timing(characteristics, Duration::seconds(5), 3);

    // Average attempts should be 2.0
    let all_stats = stats.get_statistics();
    let (_char, _avg_timing, avg_attempts) = &all_stats[0];
    assert_eq!(*avg_attempts, Some(2.0));
}

#[test]
fn test_chunk_timing_stats_clone() {
    let mut stats = ChunkTimingStats::new();

    let characteristics = ChunkCharacteristics {
        chunk_type: ChunkType::Intermediate,
        waveform_type: WaveformType::CS,
        channel_configuration: ChannelConfiguration::ConstantPhase,
    };

    stats.add_timing(characteristics, Duration::seconds(5), 1);

    let cloned = stats.clone();
    let cloned_stats = cloned.get_statistics();

    assert_eq!(cloned_stats.len(), 1);
    let (_char, avg_timing, _avg_attempts) = &cloned_stats[0];
    assert_eq!(*avg_timing, Some(Duration::seconds(5)));
}

// === Timing estimation tests ===

#[test]
fn test_estimate_chunk_processing_time_start_chunk() {
    use chrono::NaiveDateTime;
    use nexrad_data::aws::realtime::{
        estimate_chunk_processing_time, ChunkIdentifier, VolumeIndex,
    };

    let vcp = get_test_vcp();
    let mapper = ElevationChunkMapper::new(&vcp);

    let start_chunk = ChunkIdentifier::new(
        "KDMX".to_string(),
        VolumeIndex::new(1),
        NaiveDateTime::parse_from_str("20220305_120000", "%Y%m%d_%H%M%S").unwrap(),
        1,
        ChunkType::Start,
        None,
    );

    let estimate = estimate_chunk_processing_time(&start_chunk, &vcp, &mapper, None);

    // Start chunks use the inter-volume gap model (8.5 seconds)
    assert!(estimate.is_some());
    assert_eq!(estimate.unwrap(), Duration::milliseconds(8500));
}

#[test]
fn test_estimate_chunk_processing_time_with_stats() {
    use chrono::NaiveDateTime;
    use nexrad_data::aws::realtime::{
        estimate_chunk_processing_time, ChunkIdentifier, VolumeIndex,
    };

    let vcp = get_test_vcp();
    let mapper = ElevationChunkMapper::new(&vcp);

    // Get characteristics for sequence 2
    let elevation = mapper.get_sequence_elevation_number(2).unwrap();
    let elev_data = &vcp.elevations()[elevation - 1];

    let characteristics = ChunkCharacteristics {
        chunk_type: ChunkType::Intermediate,
        waveform_type: elev_data.waveform_type(),
        channel_configuration: elev_data.channel_configuration(),
    };

    // Create stats with known timing
    let mut stats = ChunkTimingStats::new();
    stats.add_timing(characteristics, Duration::seconds(5), 2);

    let chunk = ChunkIdentifier::new(
        "KDMX".to_string(),
        VolumeIndex::new(1),
        NaiveDateTime::parse_from_str("20220305_120000", "%Y%m%d_%H%M%S").unwrap(),
        2,
        ChunkType::Intermediate,
        None,
    );

    let estimate = estimate_chunk_processing_time(&chunk, &vcp, &mapper, Some(&stats));

    // Should use historical timing (5 seconds + 1 second for the extra attempt)
    assert!(estimate.is_some());
    assert_eq!(estimate.unwrap(), Duration::seconds(6)); // 5 + (2-1)
}

#[test]
fn test_estimate_chunk_processing_time_without_stats_uses_physics_model() {
    use chrono::NaiveDateTime;
    use nexrad_data::aws::realtime::{
        estimate_chunk_processing_time, ChunkIdentifier, VolumeIndex,
    };

    let vcp = get_test_vcp();
    let mapper = ElevationChunkMapper::new(&vcp);

    let chunk = ChunkIdentifier::new(
        "KDMX".to_string(),
        VolumeIndex::new(1),
        NaiveDateTime::parse_from_str("20220305_120000", "%Y%m%d_%H%M%S").unwrap(),
        2,
        ChunkType::Intermediate,
        None,
    );

    let estimate = estimate_chunk_processing_time(&chunk, &vcp, &mapper, None);

    assert!(estimate.is_some());
    let duration_secs = estimate.unwrap().num_milliseconds() as f64 / 1000.0;

    // Physics model: chunk duration = (360 / azimuth_rate - 0.67) / chunks_in_sweep
    // For VCP 212 first elevation, azimuth rate ~21 dps, super-res (6 chunks)
    // Expected: (360/21.149 - 0.67) / 6 ≈ 2.73s
    // With inter-sweep gap for first-in-sweep: ~2.73 + 0.7 = ~3.43s
    // The exact value depends on whether this is first-in-sweep or intra-sweep
    assert!(
        duration_secs > 1.0 && duration_secs < 15.0,
        "Physics-model estimate should be reasonable, got: {:.2}s",
        duration_secs
    );
}

// === ChunkTimingModel tests ===

#[test]
fn test_chunk_timing_model_sweep_duration_precip() {
    // VCP 212 low elevation: ~21.149 dps
    let duration = ChunkTimingModel::sweep_duration_secs(21.149).unwrap();
    // Expected: (360 / 21.149) - 0.67 ≈ 16.36s
    assert!(
        (duration - 16.36).abs() < 0.1,
        "Expected ~16.36s, got {:.2}s",
        duration
    );
}

#[test]
fn test_chunk_timing_model_sweep_duration_clear_air() {
    // VCP 35 low elevation: ~4.966 dps
    let duration = ChunkTimingModel::sweep_duration_secs(4.966).unwrap();
    // Expected: (360 / 4.966) - 0.67 ≈ 71.82s
    assert!(
        (duration - 71.82).abs() < 0.1,
        "Expected ~71.82s, got {:.2}s",
        duration
    );
}

#[test]
fn test_chunk_timing_model_sweep_duration_zero_rate() {
    assert!(ChunkTimingModel::sweep_duration_secs(0.0).is_none());
    assert!(ChunkTimingModel::sweep_duration_secs(-5.0).is_none());
}

#[test]
fn test_chunk_timing_model_chunk_duration() {
    // 21.149 dps, 6 chunks (super-res)
    let chunk_dur = ChunkTimingModel::chunk_duration_secs(21.149, 6).unwrap();
    assert!(
        (chunk_dur - 2.73).abs() < 0.1,
        "Expected ~2.73s, got {:.2}s",
        chunk_dur
    );

    // 21.149 dps, 3 chunks (standard)
    let chunk_dur = ChunkTimingModel::chunk_duration_secs(21.149, 3).unwrap();
    assert!(
        (chunk_dur - 5.45).abs() < 0.1,
        "Expected ~5.45s, got {:.2}s",
        chunk_dur
    );

    // Zero chunks
    assert!(ChunkTimingModel::chunk_duration_secs(21.149, 0).is_none());
}

#[test]
fn test_chunk_timing_model_inter_sweep_gap_same_elevation() {
    let gap = ChunkTimingModel::inter_sweep_gap_secs(0.5, 0.5);
    assert!(
        (gap - 0.7).abs() < 0.001,
        "Same elevation gap should be 0.7s"
    );
}

#[test]
fn test_chunk_timing_model_inter_sweep_gap_small_change() {
    let gap = ChunkTimingModel::inter_sweep_gap_secs(0.5, 0.9);
    // 0.7 + (0.4 * 0.08) = 0.732
    assert!(
        (gap - 0.732).abs() < 0.01,
        "Expected ~0.732s, got {:.3}s",
        gap
    );
}

#[test]
fn test_chunk_timing_model_inter_sweep_gap_large_change() {
    let gap = ChunkTimingModel::inter_sweep_gap_secs(6.4, 15.7);
    // 0.7 + (9.3 * 0.08) = 1.444
    assert!(
        (gap - 1.444).abs() < 0.01,
        "Expected ~1.444s, got {:.3}s",
        gap
    );
}

#[test]
fn test_chunk_timing_model_inter_volume_gap() {
    assert!((ChunkTimingModel::inter_volume_gap_secs() - 8.5).abs() < 0.001);
}

// === ScanTimingProjection tests ===

#[test]
fn test_scan_timing_projection_basic() {
    use chrono::{NaiveDateTime, TimeZone, Utc};
    use nexrad_data::aws::realtime::{project_scan_timing, ChunkIdentifier, VolumeIndex};

    let vcp = get_test_vcp();
    let mapper = ElevationChunkMapper::new(&vcp);

    let anchor_time = Utc.with_ymd_and_hms(2022, 3, 5, 23, 23, 24).unwrap();
    let anchor_chunk = ChunkIdentifier::new(
        "KDMX".to_string(),
        VolumeIndex::new(1),
        NaiveDateTime::parse_from_str("20220305_232324", "%Y%m%d_%H%M%S").unwrap(),
        5,
        ChunkType::Intermediate,
        Some(anchor_time),
    );

    let projection = project_scan_timing(&anchor_chunk, &vcp, &mapper, None);
    assert!(projection.is_some(), "Projection should succeed");

    let proj = projection.unwrap();
    assert_eq!(proj.anchor_sequence(), 5);
    assert!(!proj.chunks().is_empty());

    // All projected chunks should have monotonically increasing times
    let mut prev_time = anchor_time;
    for chunk in proj.chunks() {
        assert!(
            chunk.projected_time() > prev_time,
            "Chunk {} projected time should be after previous",
            chunk.sequence()
        );
        prev_time = chunk.projected_time();
    }

    // Volume end should be the last projected chunk's time
    assert_eq!(
        proj.volume_end_time(),
        proj.chunks().last().unwrap().projected_time()
    );

    // Remaining duration should be positive
    assert!(
        proj.remaining_duration().num_seconds() > 0,
        "Remaining duration should be positive"
    );
}

#[test]
fn test_scan_timing_projection_sweep_boundaries() {
    use chrono::{NaiveDateTime, TimeZone, Utc};
    use nexrad_data::aws::realtime::{project_scan_timing, ChunkIdentifier, VolumeIndex};

    let vcp = get_test_vcp();
    let mapper = ElevationChunkMapper::new(&vcp);

    let anchor_time = Utc.with_ymd_and_hms(2022, 3, 5, 23, 23, 24).unwrap();
    let anchor_chunk = ChunkIdentifier::new(
        "KDMX".to_string(),
        VolumeIndex::new(1),
        NaiveDateTime::parse_from_str("20220305_232324", "%Y%m%d_%H%M%S").unwrap(),
        1,
        ChunkType::Start,
        Some(anchor_time),
    );

    let projection = project_scan_timing(&anchor_chunk, &vcp, &mapper, None);
    assert!(projection.is_some());

    let proj = projection.unwrap();

    // Count sweep transitions
    let sweep_starts: Vec<_> = proj
        .chunks()
        .iter()
        .filter(|c| c.starts_new_sweep())
        .collect();
    assert!(
        !sweep_starts.is_empty(),
        "Should have at least one sweep start"
    );

    // Number of sweep starts should equal number of elevations in VCP
    assert_eq!(
        sweep_starts.len(),
        vcp.elevations().len(),
        "Sweep starts should match VCP elevation count"
    );
}

#[test]
fn test_scan_timing_projection_total_duration() {
    use chrono::{NaiveDateTime, TimeZone, Utc};
    use nexrad_data::aws::realtime::{project_scan_timing, ChunkIdentifier, VolumeIndex};

    let vcp = get_test_vcp();
    let mapper = ElevationChunkMapper::new(&vcp);

    let anchor_time = Utc.with_ymd_and_hms(2022, 3, 5, 23, 23, 24).unwrap();
    let anchor_chunk = ChunkIdentifier::new(
        "KDMX".to_string(),
        VolumeIndex::new(1),
        NaiveDateTime::parse_from_str("20220305_232324", "%Y%m%d_%H%M%S").unwrap(),
        1,
        ChunkType::Start,
        Some(anchor_time),
    );

    let projection = project_scan_timing(&anchor_chunk, &vcp, &mapper, None);
    assert!(projection.is_some());

    let proj = projection.unwrap();
    let duration_secs = proj.remaining_duration().num_seconds();

    // VCP 212 with SAILS typically takes 200-400 seconds
    assert!(
        duration_secs > 100 && duration_secs < 600,
        "Volume duration should be 100-600s, got {}s",
        duration_secs
    );
}

#[test]
fn test_scan_timing_projection_at_final_sequence() {
    use chrono::{NaiveDateTime, TimeZone, Utc};
    use nexrad_data::aws::realtime::{project_scan_timing, ChunkIdentifier, VolumeIndex};

    let vcp = get_test_vcp();
    let mapper = ElevationChunkMapper::new(&vcp);

    let anchor_time = Utc.with_ymd_and_hms(2022, 3, 5, 23, 23, 24).unwrap();
    let anchor_chunk = ChunkIdentifier::new(
        "KDMX".to_string(),
        VolumeIndex::new(1),
        NaiveDateTime::parse_from_str("20220305_232324", "%Y%m%d_%H%M%S").unwrap(),
        mapper.final_sequence(),
        ChunkType::End,
        Some(anchor_time),
    );

    // At the final sequence, there's nothing to project
    let projection = project_scan_timing(&anchor_chunk, &vcp, &mapper, None);
    assert!(projection.is_none());
}

// Integration tests that require network access and aws-polling feature
#[cfg(all(feature = "aws-polling", not(target_arch = "wasm32")))]
mod polling_tests {
    use futures::StreamExt;
    use nexrad_data::aws::realtime::{chunk_stream, PollConfig};
    use std::pin::pin;
    use std::time::Duration as StdDuration;

    #[tokio::test]
    #[ignore = "requires AWS access and takes time"]
    async fn test_chunk_stream_basic() {
        let config = PollConfig::new("KDMX");
        let stream = chunk_stream(config);
        let mut stream = pin!(stream);

        let mut chunk_count = 0;

        // Use timeout to limit test duration
        let result = tokio::time::timeout(StdDuration::from_secs(30), async {
            while let Some(result) = stream.next().await {
                match result {
                    Ok(downloaded) => {
                        assert!(!downloaded.identifier.name().is_empty());
                        assert!(!downloaded.chunk.data().is_empty());
                        chunk_count += 1;

                        // Stop after receiving a few chunks
                        if chunk_count >= 3 {
                            break;
                        }
                    }
                    Err(e) => {
                        panic!("Stream error: {:?}", e);
                    }
                }
            }
        })
        .await;

        assert!(result.is_ok(), "Test timed out");
        assert!(chunk_count > 0, "Expected to receive at least one chunk");
    }
}
