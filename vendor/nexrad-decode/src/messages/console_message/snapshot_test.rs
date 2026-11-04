use insta::assert_debug_snapshot;

use crate::messages::decode_messages;

const TEST_DATA: &[u8] = include_bytes!("../../../tests/data/messages/console_message.bin");

/// Tests decoding of a Console Message (type 4).
#[test]
fn test_decode_console_message() {
    let messages = decode_messages(TEST_DATA).expect("decodes successfully");

    assert_eq!(messages.len(), 1, "expected exactly one message");
    assert_debug_snapshot!(messages[0]);
}
