use insta::assert_debug_snapshot;

use crate::messages::decode_messages;

const TEST_DATA: &[u8] = include_bytes!("../../../tests/data/messages/volume_coverage_pattern.bin");

/// Tests decoding of a single Volume Coverage Pattern message (type 5).
#[test]
fn test_decode_volume_coverage_pattern() {
    let messages = decode_messages(TEST_DATA).expect("decodes successfully");

    assert_eq!(messages.len(), 1, "expected exactly one message");
    assert_debug_snapshot!(messages[0]);
}
