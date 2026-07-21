use nexrad_data::result;

#[test]
fn test_error_display_formatting() {
    use std::error::Error;

    let io_error = std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "test error");
    let file_error = result::Error::Io(io_error);
    let display_output = format!("{}", file_error);
    assert!(display_output.contains("data file IO error"));
    assert!(file_error.source().is_some());
}

#[test]
fn test_uncompressed_data_error() {
    let uncompressed_data = vec![0, 1, 2, 3, 4, 5, 6, 7];
    let record = nexrad_data::volume::Record::new(uncompressed_data);
    assert!(!record.compressed());

    let decompress_result = record.decompress();
    assert!(
        decompress_result.is_err(),
        "Should fail to decompress uncompressed data"
    );

    let error = decompress_result.unwrap_err();
    match error {
        result::Error::UncompressedData => {
            let display = format!("{}", error);
            assert!(display.contains("error decompressing uncompressed data"));
        }
        _ => panic!("Expected UncompressedData, got: {:?}", error),
    }
}

#[test]
fn test_compressed_data_decode_error() {
    const TEST_NEXRAD_FILE: &[u8] = include_bytes!("../../downloads/KDMX20220305_232324_V06");

    let volume = nexrad_data::volume::File::new(TEST_NEXRAD_FILE.to_vec());
    let records = volume.records().expect("records");
    let compressed_record = &records[0];
    assert!(
        compressed_record.compressed(),
        "Test record should be compressed"
    );

    let messages_result = compressed_record.messages();
    assert!(
        messages_result.is_err(),
        "Should fail to decode compressed data"
    );

    let error = messages_result.unwrap_err();
    match error {
        result::Error::CompressedData => {
            let display = format!("{}", error);
            assert!(display.contains("compressed data cannot be decoded"));
        }
        _ => panic!("Expected CompressedData, got: {:?}", error),
    }
}
