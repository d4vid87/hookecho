use insta::assert_debug_snapshot;

use crate::messages::decode_messages;

const TEST_DATA: &[u8] =
    include_bytes!("../../../tests/data/messages/clutter_filter_bypass_map.bin");

/// Tests decoding of a Clutter Filter Bypass Map message (type 13).
///
/// The bypass map contains 360 radials × 512 range bins per elevation, so we
/// snapshot a summary rather than the full data.
#[test]
fn test_decode_clutter_filter_bypass_map() {
    let messages = decode_messages(TEST_DATA).expect("decodes successfully");

    let bpm_messages: Vec<_> = messages
        .iter()
        .filter(|m| {
            matches!(
                m.contents(),
                crate::messages::MessageContents::ClutterFilterBypassMap(_)
            )
        })
        .collect();

    assert_eq!(
        bpm_messages.len(),
        1,
        "expected exactly one bypass map message"
    );
    let msg = bpm_messages[0];
    assert!(
        msg.is_segmented(),
        "bypass map should be a segmented message"
    );

    let bpm = match msg.contents() {
        crate::messages::MessageContents::ClutterFilterBypassMap(bpm) => bpm,
        _ => panic!("expected ClutterFilterBypassMap"),
    };

    #[derive(Debug)]
    #[allow(dead_code)]
    struct BypassMapSummary {
        generation_date: u16,
        generation_time: u16,
        elevation_segment_count: u16,
        elevations: Vec<ElevationSummary>,
    }

    #[derive(Debug)]
    #[allow(dead_code)]
    struct ElevationSummary {
        segment_number: u16,
        range_bin_bytes: usize,
        first_radial_first_halfword: (u8, u8),
        radial_180_first_halfword: (u8, u8),
    }

    let summary = BypassMapSummary {
        generation_date: bpm.bypass_map_generation_date(),
        generation_time: bpm.bypass_map_generation_time(),
        elevation_segment_count: bpm.number_of_elevation_segments(),
        elevations: bpm
            .elevation_segments()
            .iter()
            .map(|elev| {
                let bins = elev.range_bins();
                let radial_180_offset = 180 * 32 * 2;
                ElevationSummary {
                    segment_number: elev.segment_number(),
                    range_bin_bytes: bins.len(),
                    first_radial_first_halfword: (bins[0], bins[1]),
                    radial_180_first_halfword: (
                        bins[radial_180_offset],
                        bins[radial_180_offset + 1],
                    ),
                }
            })
            .collect(),
    };

    assert_debug_snapshot!(summary);
}
