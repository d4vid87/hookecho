use insta::assert_debug_snapshot;

use crate::messages::{decode_messages, MessageContents};

const TEST_DATA: &[u8] =
    include_bytes!("../../../tests/data/messages/digital_radar_data_legacy.bin");

/// Tests decoding of a single Digital Radar Data Legacy message (type 1).
#[test]
fn test_decode_digital_radar_data_legacy() {
    let messages = decode_messages(TEST_DATA).expect("decodes successfully");

    assert_eq!(messages.len(), 1, "expected exactly one message");
    assert_debug_snapshot!(messages[0]);
}

/// Tests converting a legacy Message Type 1 to a model Radial.
#[cfg(feature = "nexrad-model")]
#[test]
fn test_legacy_radial_conversion() {
    let messages = decode_messages(TEST_DATA).expect("decodes successfully");
    let message = &messages[0];

    let drd = match message.contents() {
        MessageContents::DigitalRadarDataLegacy(drd) => drd,
        other => panic!("expected DigitalRadarDataLegacy, got {:?}", other),
    };

    let radial = drd.radial().expect("converts to radial");
    assert_debug_snapshot!(radial);
}
