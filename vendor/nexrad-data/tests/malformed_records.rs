//! Negative tests for malformed record handling.
//!
//! These tests verify that record splitting and decompression functions
//! gracefully handle invalid, truncated, and corrupted input data.

use nexrad_data::result::Error;
use nexrad_data::volume::{split_compressed_records, Record};

#[test]
fn test_split_records_empty() {
    let result = split_compressed_records(&[]);
    assert!(result.is_ok());
    assert!(result.unwrap().is_empty());
}

#[test]
fn test_split_records_truncated_size_field() {
    // Less than 4 bytes available for size field
    let data = vec![0u8, 0u8];
    let result = split_compressed_records(&data);
    assert!(result.is_err());
    match result.unwrap_err() {
        Error::TruncatedRecord { expected, actual } => {
            assert_eq!(expected, 4);
            assert_eq!(actual, 2);
        }
        other => panic!("Expected TruncatedRecord error, got: {:?}", other),
    }
}

#[test]
fn test_split_records_three_bytes() {
    // Exactly 3 bytes - still not enough for size field
    let data = vec![0u8; 3];
    let result = split_compressed_records(&data);
    assert!(result.is_err());
    match result.unwrap_err() {
        Error::TruncatedRecord { .. } => {}
        other => panic!("Expected TruncatedRecord error, got: {:?}", other),
    }
}

#[test]
fn test_split_records_size_exceeds_data() {
    // Size field claims 1000 bytes, only 10 available
    let mut data = vec![0u8, 0u8, 0x03, 0xe8]; // 1000 in big-endian
    data.extend_from_slice(&[0u8; 10]);
    let result = split_compressed_records(&data);
    assert!(result.is_err());
    match result.unwrap_err() {
        Error::TruncatedRecord { expected, actual } => {
            assert!(expected > actual);
        }
        other => panic!("Expected TruncatedRecord error, got: {:?}", other),
    }
}

#[test]
fn test_split_records_zero_size() {
    // Data starting with all-zero first 4 bytes is detected as legacy CTM format
    // (the first 12 bytes of CTM frames are the rpg_unknown field, which is zeros).
    // This returns Ok with one record containing the full data.
    let data = vec![0u8, 0u8, 0u8, 0u8, 1u8, 2u8, 3u8, 4u8];
    let result = split_compressed_records(&data);
    assert!(result.is_ok());
    let records = result.unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].data().len(), 8);
}

#[test]
fn test_split_records_ldm_zero_size() {
    // LDM record with zero size after a valid first record should error.
    // Uses non-zero first 4 bytes to avoid CTM detection.
    let mut data = vec![0u8, 0u8, 0u8, 4u8]; // First record: size=4
    data.extend_from_slice(&[1u8; 4]); // First record data
    data.extend_from_slice(&[0u8, 0u8, 0u8, 0u8]); // Second record: size=0 (invalid)
    let result = split_compressed_records(&data);
    assert!(result.is_err());
    match result.unwrap_err() {
        Error::InvalidRecordSize { size, offset } => {
            assert_eq!(size, 0);
            assert_eq!(offset, 8);
        }
        other => panic!("Expected InvalidRecordSize error, got: {:?}", other),
    }
}

#[test]
fn test_split_records_negative_size() {
    // Size field is negative (high bit set) - should work due to unsigned_abs
    let data = vec![0xFF, 0xFF, 0xFF, 0xFF]; // -1 in i32
    let result = split_compressed_records(&data);
    // This will be interpreted as a very large positive number, causing truncation error
    assert!(result.is_err());
}

#[test]
fn test_decompress_uncompressed_data() {
    let data = vec![0u8, 1u8, 2u8, 3u8, 4u8, 5u8, 6u8, 7u8];
    let record = Record::new(data);
    assert!(!record.compressed());

    let result = record.decompress();
    assert!(result.is_err());
    match result.unwrap_err() {
        Error::UncompressedData => {}
        other => panic!("Expected UncompressedData, got: {:?}", other),
    }
}

#[test]
fn test_decompress_invalid_bzip_header() {
    // Record starts with size prefix but has invalid magic bytes
    let mut data = vec![0u8, 0u8, 0u8, 10u8]; // Size prefix
    data.extend_from_slice(b"BX"); // Invalid magic (should be "BZ")
    data.extend_from_slice(&[0u8; 10]);
    let record = Record::new(data);
    assert!(!record.compressed()); // Should not be detected as compressed
}

#[test]
fn test_decompress_truncated_bzip_stream() {
    // Valid BZ marker but truncated bzip stream
    let mut data = vec![0u8, 0u8, 0u8, 100u8]; // Size prefix
    data.extend_from_slice(b"BZh91AY&SY"); // Valid bzip2 header start
                                           // No actual compressed data follows
    let record = Record::new(data);
    assert!(record.compressed()); // Should be detected as compressed

    let result = record.decompress();
    // Should fail gracefully, not panic
    assert!(result.is_err());
}

#[test]
fn test_decompress_corrupted_bzip_data() {
    // Valid BZ marker but corrupted data
    let mut data = vec![0u8, 0u8, 0u8, 50u8]; // Size prefix
    data.extend_from_slice(b"BZh9"); // Valid header start
    data.extend_from_slice(&[0xFFu8; 46]); // Garbage data
    let record = Record::new(data);

    let result = record.decompress();
    // Should fail gracefully, not panic
    assert!(result.is_err());
}

#[test]
fn test_record_messages_compressed_data() {
    // Try to decode messages from compressed data
    let mut data = vec![0u8, 0u8, 0u8, 10u8]; // Size prefix
    data.extend_from_slice(b"BZ"); // BZ marker makes it "compressed"
    data.extend_from_slice(&[0u8; 10]);
    let record = Record::new(data);
    assert!(record.compressed());

    let result = record.messages();
    assert!(result.is_err());
    match result.unwrap_err() {
        Error::CompressedData => {}
        other => panic!("Expected CompressedData, got: {:?}", other),
    }
}

#[test]
fn test_record_short_data_not_compressed() {
    // Data too short to check for BZ marker
    let data = vec![0u8; 5];
    let record = Record::new(data);
    assert!(!record.compressed());
}

#[test]
fn test_record_exactly_six_bytes_no_bz() {
    // Exactly 6 bytes but no BZ marker
    let data = vec![0u8, 1u8, 2u8, 3u8, b'X', b'Y'];
    let record = Record::new(data);
    assert!(!record.compressed());
}

#[test]
fn test_record_exactly_six_bytes_with_bz() {
    // Exactly 6 bytes with BZ marker at correct position
    let data = vec![0u8, 0u8, 0u8, 0u8, b'B', b'Z'];
    let record = Record::new(data);
    assert!(record.compressed());
}

#[test]
fn test_split_multiple_records_second_truncated() {
    // First record valid, second record truncated
    let mut data = vec![];
    // First record: size=10, data
    data.extend_from_slice(&[0u8, 0u8, 0u8, 10u8]);
    data.extend_from_slice(&[1u8; 10]);
    // Second record: size claims 100 but data is truncated
    data.extend_from_slice(&[0u8, 0u8, 0u8, 100u8]);
    data.extend_from_slice(&[2u8; 5]); // Only 5 bytes instead of 100

    let result = split_compressed_records(&data);
    assert!(result.is_err());
}

#[test]
fn test_split_multiple_valid_records() {
    // Multiple valid records
    let mut data = vec![];
    // First record: size=8, data
    data.extend_from_slice(&[0u8, 0u8, 0u8, 8u8]);
    data.extend_from_slice(&[1u8; 8]);
    // Second record: size=4, data
    data.extend_from_slice(&[0u8, 0u8, 0u8, 4u8]);
    data.extend_from_slice(&[2u8; 4]);

    let result = split_compressed_records(&data);
    assert!(result.is_ok());
    let records = result.unwrap();
    assert_eq!(records.len(), 2);
}
