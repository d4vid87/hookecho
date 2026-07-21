use nexrad_data::volume;

const TEST_NEXRAD_FILE: &[u8] = include_bytes!("../../downloads/KDMX20220305_232324_V06");

#[test]
fn test_header_read_success() {
    let file = volume::File::new(TEST_NEXRAD_FILE.to_vec());
    let header = file.header().expect("Header read should succeed");

    let expected_volume_datetime = chrono::DateTime::parse_from_rfc3339("2022-03-05T23:23:24.299Z")
        .unwrap()
        .with_timezone(&chrono::Utc);

    assert_eq!(header.tape_filename(), Some("AR2V0006.".to_string()));
    assert_eq!(header.extension_number(), Some("879".to_string()));
    assert_eq!(header.date_time(), Some(expected_volume_datetime));
    assert_eq!(header.icao_of_radar(), Some("KDMX".to_string()));
}

#[test]
fn test_header_read_insufficient_data() {
    let short_data = vec![0u8; 10];
    let file = volume::File::new(short_data);

    let result = file.header();
    assert!(result.is_none(), "Should fail with insufficient data");
}
