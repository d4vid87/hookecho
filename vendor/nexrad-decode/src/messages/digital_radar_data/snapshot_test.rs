use insta::assert_debug_snapshot;

use crate::messages::decode_messages;

const TEST_DATA: &[u8] = include_bytes!("../../../tests/data/messages/digital_radar_data_full.bin");

/// Tests decoding of a single Digital Radar Data message (type 31).
#[test]
fn test_decode_digital_radar_data() {
    let messages = decode_messages(TEST_DATA).expect("decodes successfully");

    assert_eq!(messages.len(), 1, "expected exactly one message");
    assert_debug_snapshot!(messages[0]);
}
