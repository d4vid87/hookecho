use insta::assert_debug_snapshot;

use crate::messages::decode_messages;

const TEST_DATA: &[u8] = include_bytes!("../../../tests/data/messages/rda_adaptation_data.bin");

/// Tests decoding of an RDA Adaptation Data message (type 18).
///
/// Adaptation data spans 4 segments (~9468 bytes). We snapshot a summary of key
/// fields rather than the full raw data buffer.
#[test]
fn test_decode_rda_adaptation_data() {
    let messages = decode_messages(TEST_DATA).expect("decodes successfully");

    let adap_messages: Vec<_> = messages
        .iter()
        .filter(|m| {
            matches!(
                m.contents(),
                crate::messages::MessageContents::RDAAdaptationData(_)
            )
        })
        .collect();

    assert_eq!(
        adap_messages.len(),
        1,
        "expected exactly one adaptation data message"
    );
    let msg = adap_messages[0];
    assert!(
        msg.is_segmented(),
        "adaptation data should be a segmented message"
    );

    let adap = match msg.contents() {
        crate::messages::MessageContents::RDAAdaptationData(a) => a,
        _ => panic!("expected RDAAdaptationData"),
    };

    #[derive(Debug)]
    #[allow(dead_code)]
    struct AdaptationSummary {
        adap_file_name: Option<String>,
        adap_format: Option<String>,
        adap_revision: Option<String>,
        adap_date: Option<String>,
        adap_time: Option<String>,
        site_latitude: Option<f64>,
        site_longitude: Option<f64>,
        site_name: Option<String>,
        antenna_gain: Option<f32>,
    }

    let summary = AdaptationSummary {
        adap_file_name: adap.adap_file_name(),
        adap_format: adap.adap_format(),
        adap_revision: adap.adap_revision(),
        adap_date: adap.adap_date(),
        adap_time: adap.adap_time(),
        site_latitude: adap.site_latitude(),
        site_longitude: adap.site_longitude(),
        site_name: adap.site_name(),
        antenna_gain: adap.antenna_gain(),
    };

    assert_debug_snapshot!(summary);
}
