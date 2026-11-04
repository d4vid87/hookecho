use insta::assert_debug_snapshot;

use crate::messages::decode_messages;

const TEST_DATA: &[u8] = include_bytes!("../../../tests/data/messages/clutter_filter_map.bin");

/// Tests decoding of a Clutter Filter Map message (type 15).
///
/// CFM is a segmented message spanning multiple 2432-byte segments.
/// The test data contains 5 segments extracted from the KDMX volume.
///
/// The full CFM structure (5 elevations x 360 azimuths x range zones) is too
/// large for a complete debug snapshot, so we snapshot a summary and verify
/// structural properties programmatically.
#[test]
fn test_decode_clutter_filter_map() {
    let messages = decode_messages(TEST_DATA).expect("decodes successfully");

    let cfm_messages: Vec<_> = messages
        .iter()
        .filter(|m| {
            matches!(
                m.contents(),
                crate::messages::MessageContents::ClutterFilterMap(_)
            )
        })
        .collect();

    assert_eq!(cfm_messages.len(), 1, "expected exactly one CFM message");
    let msg = cfm_messages[0];
    assert!(msg.is_segmented(), "CFM should be a segmented message");

    let cfm = match msg.contents() {
        crate::messages::MessageContents::ClutterFilterMap(cfm) => cfm,
        _ => panic!("expected ClutterFilterMap"),
    };

    // Snapshot header and per-elevation summary (full data is too large).
    #[derive(Debug)]
    #[allow(dead_code)]
    struct CfmSummary {
        map_generation_date: u16,
        map_generation_time: u16,
        elevation_segment_count: u16,
        elevations: Vec<ElevationSummary>,
    }

    #[derive(Debug)]
    #[allow(dead_code)]
    struct ElevationSummary {
        segment_number: u8,
        azimuth_count: usize,
        first_azimuth_range_zones: Vec<(u16, u16)>,
        last_azimuth_range_zones: Vec<(u16, u16)>,
    }

    let summary = CfmSummary {
        map_generation_date: cfm.map_generation_date(),
        map_generation_time: cfm.map_generation_time(),
        elevation_segment_count: cfm.elevation_segment_count(),
        elevations: cfm
            .elevation_segments()
            .iter()
            .map(|elev| {
                let azimuths = elev.azimuth_segments();
                ElevationSummary {
                    segment_number: elev.elevation_segment_number(),
                    azimuth_count: azimuths.len(),
                    first_azimuth_range_zones: azimuths
                        .first()
                        .map(|az| {
                            az.range_zones()
                                .iter()
                                .map(|rz| (rz.raw_op_code(), rz.end_range()))
                                .collect()
                        })
                        .unwrap_or_default(),
                    last_azimuth_range_zones: azimuths
                        .last()
                        .map(|az| {
                            az.range_zones()
                                .iter()
                                .map(|rz| (rz.raw_op_code(), rz.end_range()))
                                .collect()
                        })
                        .unwrap_or_default(),
                }
            })
            .collect(),
    };

    assert_debug_snapshot!(summary);
}
