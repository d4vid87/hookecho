use insta::assert_debug_snapshot;

use crate::messages::decode_messages;

const TEST_DATA: &[u8] =
    include_bytes!("../../../tests/data/messages/performance_maintenance_data.bin");

/// Tests decoding of a Performance/Maintenance Data message (type 3).
#[test]
fn test_decode_performance_maintenance_data() {
    let messages = decode_messages(TEST_DATA).expect("decodes successfully");

    assert_eq!(messages.len(), 1, "expected exactly one message");
    assert_debug_snapshot!(messages[0]);
}
