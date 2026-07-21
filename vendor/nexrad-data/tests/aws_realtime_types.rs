#![cfg(feature = "aws")]

use chrono::NaiveDateTime;
use nexrad_data::aws::realtime::{Chunk, ChunkIdentifier, ChunkType, VolumeIndex};

// Test data: a minimal valid AR2 volume header
const MINIMAL_AR2_HEADER: &[u8] = b"AR2V0006.";

// Test data: a minimal BZ compressed record (just the BZ marker with some padding)
const MINIMAL_BZ_RECORD: &[u8] = &[0, 0, 0, 0, b'B', b'Z', 0, 0, 0, 0];

#[test]
fn test_volume_index_creation() {
    let volume = VolumeIndex::new(1);
    assert_eq!(volume.as_number(), 1);

    let volume = VolumeIndex::new(500);
    assert_eq!(volume.as_number(), 500);

    let volume = VolumeIndex::new(999);
    assert_eq!(volume.as_number(), 999);
}

#[test]
fn test_volume_index_next() {
    // Normal progression
    let volume = VolumeIndex::new(1);
    assert_eq!(volume.next().as_number(), 2);

    let volume = VolumeIndex::new(500);
    assert_eq!(volume.next().as_number(), 501);

    // Wraparound at 999
    let volume = VolumeIndex::new(999);
    assert_eq!(volume.next().as_number(), 1);
}

#[test]
fn test_volume_index_ordering() {
    let vol1 = VolumeIndex::new(1);
    let vol2 = VolumeIndex::new(2);
    let vol999 = VolumeIndex::new(999);

    assert!(vol1 < vol2);
    assert!(vol2 > vol1);
    assert!(vol1 == vol1);
    assert!(vol999 > vol1);
    assert!(vol999 > vol2);
}

#[test]
fn test_chunk_type_from_abbreviation() {
    assert_eq!(ChunkType::from_abbreviation('S').unwrap(), ChunkType::Start);
    assert_eq!(
        ChunkType::from_abbreviation('I').unwrap(),
        ChunkType::Intermediate
    );
    assert_eq!(ChunkType::from_abbreviation('E').unwrap(), ChunkType::End);

    // Invalid abbreviations should error
    assert!(ChunkType::from_abbreviation('X').is_err());
    assert!(ChunkType::from_abbreviation('s').is_err()); // lowercase
    assert!(ChunkType::from_abbreviation('1').is_err());
}

#[test]
fn test_chunk_type_abbreviation() {
    assert_eq!(ChunkType::Start.abbreviation(), 'S');
    assert_eq!(ChunkType::Intermediate.abbreviation(), 'I');
    assert_eq!(ChunkType::End.abbreviation(), 'E');
}

#[test]
fn test_chunk_type_roundtrip() {
    for chunk_type in [ChunkType::Start, ChunkType::Intermediate, ChunkType::End] {
        let abbreviation = chunk_type.abbreviation();
        let parsed = ChunkType::from_abbreviation(abbreviation).unwrap();
        assert_eq!(parsed, chunk_type);
    }
}

#[test]
fn test_chunk_identifier_new() {
    let site = "KDMX".to_string();
    let volume = VolumeIndex::new(123);
    let date_time_prefix =
        NaiveDateTime::parse_from_str("20220305_232324", "%Y%m%d_%H%M%S").unwrap();
    let sequence = 42;
    let chunk_type = ChunkType::Intermediate;

    let chunk_id = ChunkIdentifier::new(
        site.clone(),
        volume,
        date_time_prefix,
        sequence,
        chunk_type,
        None,
    );

    assert_eq!(chunk_id.site(), "KDMX");
    assert_eq!(chunk_id.volume().as_number(), 123);
    assert_eq!(chunk_id.sequence(), 42);
    assert_eq!(chunk_id.chunk_type(), ChunkType::Intermediate);
    assert_eq!(chunk_id.name(), "20220305-232324-042-I");
    assert_eq!(chunk_id.upload_date_time(), None);
}

#[test]
fn test_chunk_identifier_name_format() {
    let volume = VolumeIndex::new(1);
    let date_time = NaiveDateTime::parse_from_str("20220305_123456", "%Y%m%d_%H%M%S").unwrap();

    // Test Start chunk name
    let chunk_id = ChunkIdentifier::new(
        "KDMX".to_string(),
        volume,
        date_time,
        1,
        ChunkType::Start,
        None,
    );
    assert_eq!(chunk_id.name(), "20220305-123456-001-S");

    // Test Intermediate chunk name
    let chunk_id = ChunkIdentifier::new(
        "KDMX".to_string(),
        volume,
        date_time,
        25,
        ChunkType::Intermediate,
        None,
    );
    assert_eq!(chunk_id.name(), "20220305-123456-025-I");

    // Test End chunk name
    let chunk_id = ChunkIdentifier::new(
        "KDMX".to_string(),
        volume,
        date_time,
        55,
        ChunkType::End,
        None,
    );
    assert_eq!(chunk_id.name(), "20220305-123456-055-E");
}

#[test]
fn test_chunk_identifier_from_name() {
    let site = "KDMX".to_string();
    let volume = VolumeIndex::new(456);
    let name = "20220305-232324-042-I".to_string();

    let chunk_id = ChunkIdentifier::from_name(site.clone(), volume, name, None).unwrap();

    assert_eq!(chunk_id.site(), "KDMX");
    assert_eq!(chunk_id.volume().as_number(), 456);
    assert_eq!(chunk_id.sequence(), 42);
    assert_eq!(chunk_id.chunk_type(), ChunkType::Intermediate);
    assert_eq!(
        chunk_id.date_time_prefix(),
        &NaiveDateTime::parse_from_str("20220305-232324", "%Y%m%d-%H%M%S").unwrap()
    );
}

#[test]
fn test_chunk_identifier_from_name_all_types() {
    let volume = VolumeIndex::new(1);

    let chunk_id = ChunkIdentifier::from_name(
        "KDMX".to_string(),
        volume,
        "20220305-120000-001-S".to_string(),
        None,
    )
    .unwrap();
    assert_eq!(chunk_id.chunk_type(), ChunkType::Start);

    let chunk_id = ChunkIdentifier::from_name(
        "KDMX".to_string(),
        volume,
        "20220305-120000-010-I".to_string(),
        None,
    )
    .unwrap();
    assert_eq!(chunk_id.chunk_type(), ChunkType::Intermediate);

    let chunk_id = ChunkIdentifier::from_name(
        "KDMX".to_string(),
        volume,
        "20220305-120000-055-E".to_string(),
        None,
    )
    .unwrap();
    assert_eq!(chunk_id.chunk_type(), ChunkType::End);
}

#[test]
fn test_chunk_identifier_from_name_invalid() {
    let volume = VolumeIndex::new(1);

    // Invalid date format
    let result = ChunkIdentifier::from_name(
        "KDMX".to_string(),
        volume,
        "20220305232324-042-I".to_string(), // Missing dash separator
        None,
    );
    assert!(result.is_err());

    // Invalid sequence format
    let result = ChunkIdentifier::from_name(
        "KDMX".to_string(),
        volume,
        "20220305-232324-ABC-I".to_string(), // Non-numeric sequence
        None,
    );
    assert!(result.is_err());

    // Invalid chunk type
    let result = ChunkIdentifier::from_name(
        "KDMX".to_string(),
        volume,
        "20220305-232324-042-X".to_string(), // Invalid type
        None,
    );
    assert!(result.is_err());

    // Name with valid length but invalid format (year part)
    let result = ChunkIdentifier::from_name(
        "KDMX".to_string(),
        volume,
        "ABCD0305-123456-042-S".to_string(),
        None,
    );
    assert!(result.is_err());
}

#[test]
fn test_chunk_identifier_from_name_too_short() {
    let volume = VolumeIndex::new(1);
    // String too short - should return an error
    let result = ChunkIdentifier::from_name("KDMX".to_string(), volume, "short".to_string(), None);
    assert!(result.is_err(), "Expected error for short name");
}

#[test]
fn test_chunk_identifier_roundtrip() {
    let original = ChunkIdentifier::new(
        "KDMX".to_string(),
        VolumeIndex::new(789),
        NaiveDateTime::parse_from_str("20220305_232324", "%Y%m%d_%H%M%S").unwrap(),
        42,
        ChunkType::Intermediate,
        None,
    );

    let name = original.name().to_string();
    let parsed =
        ChunkIdentifier::from_name(original.site().to_string(), *original.volume(), name, None)
            .unwrap();

    assert_eq!(parsed.site(), original.site());
    assert_eq!(parsed.volume(), original.volume());
    assert_eq!(parsed.sequence(), original.sequence());
    assert_eq!(parsed.chunk_type(), original.chunk_type());
    assert_eq!(parsed.date_time_prefix(), original.date_time_prefix());
}

#[test]
fn test_chunk_new_start_type() {
    // Create data that starts with AR2 header
    let mut data = Vec::new();
    data.extend_from_slice(MINIMAL_AR2_HEADER);
    data.extend_from_slice(&[0u8; 100]); // Padding

    let chunk = Chunk::new(data.clone()).unwrap();

    match chunk {
        Chunk::Start(file) => {
            assert_eq!(file.data(), data.as_slice());
        }
        _ => panic!("Expected Chunk::Start variant"),
    }
}

#[test]
fn test_chunk_new_intermediate_or_end_type() {
    // Create data that starts with BZ marker at position 4-5
    let data = MINIMAL_BZ_RECORD.to_vec();

    let chunk = Chunk::new(data.clone()).unwrap();

    match chunk {
        Chunk::IntermediateOrEnd(record) => {
            assert_eq!(record.data(), data.as_slice());
        }
        _ => panic!("Expected Chunk::IntermediateOrEnd variant"),
    }
}

#[test]
fn test_chunk_new_unrecognized_format() {
    // Data that doesn't match either format
    let data = vec![0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];

    let result = Chunk::new(data);
    assert!(result.is_err());
}

#[test]
fn test_chunk_data_accessor() {
    // Test Start chunk
    let mut start_data = Vec::new();
    start_data.extend_from_slice(MINIMAL_AR2_HEADER);
    start_data.extend_from_slice(&[1, 2, 3, 4]);

    let chunk = Chunk::new(start_data.clone()).unwrap();
    assert_eq!(chunk.data(), start_data.as_slice());

    // Test IntermediateOrEnd chunk
    let intermediate_data = MINIMAL_BZ_RECORD.to_vec();
    let chunk = Chunk::new(intermediate_data.clone()).unwrap();
    assert_eq!(chunk.data(), intermediate_data.as_slice());
}

#[test]
fn test_chunk_too_small_for_ar2_check() {
    // Test with data too small for AR2 header (needs at least 3 bytes)
    let tiny_data = vec![b'A', b'R'];
    let result = Chunk::new(tiny_data);
    assert!(
        result.is_err(),
        "Expected error for data too small for AR2 check"
    );
}

#[test]
fn test_chunk_too_small_for_bz_check() {
    // Test with data too small for BZ check (needs at least 6 bytes to check positions 4-5)
    let small_data = vec![0, 0, 0, 0, b'B'];
    let result = Chunk::new(small_data);
    assert!(
        result.is_err(),
        "Expected error for data too small for BZ check"
    );
}

#[test]
fn test_chunk_identifier_next_chunk_intermediate() {
    use nexrad_data::aws::realtime::{ElevationChunkMapper, NextChunk};

    // This test requires a real VCP message to create the mapper
    // For now, we'll test the basic structure
    const TEST_NEXRAD_FILE: &[u8] = include_bytes!("../../downloads/KDMX20220305_232324_V06");
    let volume_file = nexrad_data::volume::File::new(TEST_NEXRAD_FILE.to_vec());

    // Get VCP from the file
    let mut vcp = None;
    for mut record in volume_file.records().expect("records") {
        if record.compressed() {
            record = record.decompress().unwrap();
        }
        for message in record.messages().unwrap() {
            if let nexrad_decode::messages::MessageContents::VolumeCoveragePattern(vcp_msg) =
                message.contents()
            {
                vcp = Some(vcp_msg.clone().into_owned());
                break;
            }
        }
        if vcp.is_some() {
            break;
        }
    }

    let vcp = vcp.expect("VCP should be found in test file");
    let mapper = ElevationChunkMapper::new(&vcp);
    let final_seq = mapper.final_sequence();

    // Test non-final chunk progresses to next sequence
    let chunk_id = ChunkIdentifier::new(
        "KDMX".to_string(),
        VolumeIndex::new(1),
        NaiveDateTime::parse_from_str("20220305_120000", "%Y%m%d_%H%M%S").unwrap(),
        2,
        ChunkType::Intermediate,
        None,
    );

    match chunk_id.next_chunk(&mapper) {
        Some(NextChunk::Sequence(next)) => {
            assert_eq!(next.sequence(), 3);
            assert_eq!(next.site(), "KDMX");
            assert_eq!(next.volume().as_number(), 1);
        }
        _ => panic!("Expected NextChunk::Sequence for intermediate chunk"),
    }

    // Test final chunk progresses to next volume
    let chunk_id = ChunkIdentifier::new(
        "KDMX".to_string(),
        VolumeIndex::new(1),
        NaiveDateTime::parse_from_str("20220305_120000", "%Y%m%d_%H%M%S").unwrap(),
        final_seq,
        ChunkType::End,
        None,
    );

    match chunk_id.next_chunk(&mapper) {
        Some(NextChunk::Volume(next_volume)) => {
            assert_eq!(next_volume.as_number(), 2);
        }
        _ => panic!("Expected NextChunk::Volume for final chunk"),
    }
}

#[test]
fn test_volume_index_equality() {
    let vol1a = VolumeIndex::new(100);
    let vol1b = VolumeIndex::new(100);
    let vol2 = VolumeIndex::new(200);

    assert_eq!(vol1a, vol1b);
    assert_ne!(vol1a, vol2);
}

#[test]
fn test_chunk_type_equality() {
    assert_eq!(ChunkType::Start, ChunkType::Start);
    assert_eq!(ChunkType::Intermediate, ChunkType::Intermediate);
    assert_eq!(ChunkType::End, ChunkType::End);

    assert_ne!(ChunkType::Start, ChunkType::Intermediate);
    assert_ne!(ChunkType::Intermediate, ChunkType::End);
    assert_ne!(ChunkType::Start, ChunkType::End);
}
