use insta::assert_debug_snapshot;

use crate::messages::decode_messages;

const TEST_DATA: &[u8] = include_bytes!("../../../tests/data/messages/clutter_censor_zones.bin");

/// Tests decoding of a Clutter Censor Zones message (type 8).
#[test]
fn test_decode_clutter_censor_zones() {
    let messages = decode_messages(TEST_DATA).expect("decodes successfully");

    assert_eq!(messages.len(), 1, "expected exactly one message");
    assert_debug_snapshot!(messages[0]);
}
