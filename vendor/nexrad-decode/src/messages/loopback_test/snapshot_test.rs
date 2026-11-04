use insta::assert_debug_snapshot;

use crate::messages::decode_messages;

const TEST_DATA: &[u8] = include_bytes!("../../../tests/data/messages/loopback_test.bin");

/// Tests decoding of a Loopback Test message (type 11).
#[test]
fn test_decode_loopback_test() {
    let messages = decode_messages(TEST_DATA).expect("decodes successfully");

    assert_eq!(messages.len(), 1, "expected exactly one message");
    assert_debug_snapshot!(messages[0]);
}
