//! Integration tests using curated NEXRAD fixtures.
//!
//! These tests verify invariant properties across all fixtures to ensure
//! decoder correctness and regression protection.
//!
//! Fixtures are trimmed Archive II files containing 2-3 elevation sweeps,
//! stored in tests/fixtures/ and documented in fixture_catalog.toml.

#![cfg(feature = "nexrad-model")]

use chrono::Datelike;
use nexrad_data::volume;
use nexrad_decode::messages::{decode_messages, MessageContents};
use nexrad_model::data::DataMoment;

// =============================================================================
// FIXTURE LOADING
// =============================================================================

/// Metadata for a test fixture
struct FixtureMeta {
    id: &'static str,
    site: &'static str,
    expected_vcp: u16,
    expected_sweeps: usize,
    data: &'static [u8],
}

/// Available fixtures for testing
const FIXTURES: &[FixtureMeta] = &[
    FixtureMeta {
        id: "existing_kdmx",
        site: "KDMX",
        expected_vcp: 212,
        expected_sweeps: 3,
        data: include_bytes!("../../tests/fixtures/convective/KDMX20220305_232324.bin"),
    },
    FixtureMeta {
        id: "hurricane_harvey",
        site: "KCRP",
        expected_vcp: 212,
        expected_sweeps: 3,
        data: include_bytes!("../../tests/fixtures/tropical/KCRP20170826_044114.bin"),
    },
    FixtureMeta {
        id: "modern_build_kdmx",
        site: "KDMX",
        expected_vcp: 212,
        expected_sweeps: 3,
        data: include_bytes!("../../tests/fixtures/clear_air/KDMX20250314_175512.bin"),
    },
    FixtureMeta {
        id: "clear_air_plains",
        site: "KTLX",
        expected_vcp: 32,
        expected_sweeps: 3,
        data: include_bytes!("../../tests/fixtures/clear_air/KTLX20190715_120037.bin"),
    },
    FixtureMeta {
        id: "tornado_may2019",
        site: "KTLX",
        expected_vcp: 212,
        expected_sweeps: 3,
        data: include_bytes!("../../tests/fixtures/convective/KTLX20190525_222419.bin"),
    },
    FixtureMeta {
        id: "supercell_texas",
        site: "KFWS",
        expected_vcp: 212,
        expected_sweeps: 3,
        data: include_bytes!("../../tests/fixtures/convective/KFWS20190609_222523.bin"),
    },
    FixtureMeta {
        id: "winter_greatlakes",
        site: "KMKX",
        expected_vcp: 215,
        expected_sweeps: 3,
        data: include_bytes!("../../tests/fixtures/winter/KMKX20200115_150035.bin"),
    },
    FixtureMeta {
        id: "stratiform_noreaster",
        site: "KLWX",
        expected_vcp: 21,
        expected_sweeps: 3,
        data: include_bytes!("../../tests/fixtures/stratiform/KLWX20180302_115347.bin"),
    },
];

// =============================================================================
// INVARIANT TESTS: Station ID Consistency
// =============================================================================

/// Verifies that all radar data messages report the same station ID
/// as the volume header.
#[test]
fn test_station_id_consistency_all_fixtures() {
    for fixture in FIXTURES {
        let volume = volume::File::new(fixture.data.to_vec());

        let header = volume.header().expect("should have header");
        let header_site = header.icao_of_radar().expect("should have ICAO");

        assert_eq!(
            header_site, fixture.site,
            "Fixture {} header site mismatch",
            fixture.id
        );

        for mut record in volume.records().expect("should have records") {
            if record.compressed() {
                record = record.decompress().expect("should decompress");
            }

            for message in decode_messages(record.data()).expect("should decode") {
                if let MessageContents::DigitalRadarData(drd) = message.contents() {
                    let msg_site = drd.header().radar_identifier();
                    let msg_site_trimmed = msg_site.trim_end_matches('\0');
                    assert_eq!(
                        msg_site_trimmed,
                        fixture.site,
                        "Fixture {} message site mismatch at elevation {}",
                        fixture.id,
                        drd.header().elevation_number()
                    );
                }
            }
        }
    }
}

// =============================================================================
// INVARIANT TESTS: Timestamp Monotonicity
// =============================================================================

/// Verifies that timestamps within each sweep are monotonically increasing
/// (radials collected sequentially in time).
#[test]
fn test_timestamp_monotonicity_within_sweeps() {
    for fixture in FIXTURES {
        let volume = volume::File::new(fixture.data.to_vec());
        let scan = volume.scan().expect("should convert to scan");

        for (sweep_idx, sweep) in scan.sweeps().iter().enumerate() {
            let radials = sweep.radials();
            if radials.is_empty() {
                continue;
            }

            let mut prev_timestamp = radials[0].collection_timestamp();

            for (radial_idx, radial) in radials.iter().enumerate().skip(1) {
                let curr_timestamp = radial.collection_timestamp();
                assert!(
                    curr_timestamp >= prev_timestamp,
                    "Fixture {} sweep {} radial {}: timestamp went backwards ({} < {})",
                    fixture.id,
                    sweep_idx,
                    radial_idx,
                    curr_timestamp,
                    prev_timestamp
                );
                prev_timestamp = curr_timestamp;
            }
        }
    }
}

// =============================================================================
// INVARIANT TESTS: Sweep and Radial Counts
// =============================================================================

/// Verifies sweep counts match expected values for trimmed fixtures.
#[test]
fn test_sweep_count_matches_expected() {
    for fixture in FIXTURES {
        let volume = volume::File::new(fixture.data.to_vec());
        let scan = volume.scan().expect("should convert to scan");

        assert_eq!(
            scan.sweeps().len(),
            fixture.expected_sweeps,
            "Fixture {} sweep count mismatch",
            fixture.id
        );
    }
}

/// Verifies each sweep has reasonable radial count (360 or 720 typically).
#[test]
fn test_radial_counts_valid() {
    for fixture in FIXTURES {
        let volume = volume::File::new(fixture.data.to_vec());
        let scan = volume.scan().expect("should convert to scan");

        for (sweep_idx, sweep) in scan.sweeps().iter().enumerate() {
            let radial_count = sweep.radials().len();
            // Valid counts: 360 (1-degree), 720 (0.5-degree)
            assert!(
                radial_count == 360 || radial_count == 720,
                "Fixture {} sweep {} has unexpected radial count: {} (expected 360 or 720)",
                fixture.id,
                sweep_idx,
                radial_count
            );
        }
    }
}

/// Verifies azimuth numbers are sequential within sweeps.
#[test]
fn test_azimuth_numbers_sequential() {
    for fixture in FIXTURES {
        let volume = volume::File::new(fixture.data.to_vec());
        let scan = volume.scan().expect("should convert to scan");

        for (sweep_idx, sweep) in scan.sweeps().iter().enumerate() {
            for (radial_idx, radial) in sweep.radials().iter().enumerate() {
                let expected_azimuth = (radial_idx + 1) as u16;
                assert_eq!(
                    radial.azimuth_number(),
                    expected_azimuth,
                    "Fixture {} sweep {} radial {} has non-sequential azimuth number",
                    fixture.id,
                    sweep_idx,
                    radial_idx
                );
            }
        }
    }
}

// =============================================================================
// INVARIANT TESTS: Message Ordering
// =============================================================================

/// Verifies metadata messages (RDA Status, VCP) appear in the volume.
#[test]
fn test_metadata_messages_present() {
    for fixture in FIXTURES {
        let volume = volume::File::new(fixture.data.to_vec());

        let mut found_status = false;
        let mut found_vcp = false;
        let mut found_clutter_map = false;

        for mut record in volume.records().expect("should have records") {
            if record.compressed() {
                record = record.decompress().expect("should decompress");
            }

            for message in decode_messages(record.data()).expect("should decode") {
                match message.contents() {
                    MessageContents::RDAStatusData(_) => found_status = true,
                    MessageContents::VolumeCoveragePattern(_) => found_vcp = true,
                    MessageContents::ClutterFilterMap(_) => found_clutter_map = true,
                    _ => {}
                }
            }
        }

        assert!(
            found_status,
            "Fixture {} missing RDA Status Data message",
            fixture.id
        );
        assert!(
            found_vcp,
            "Fixture {} missing Volume Coverage Pattern message",
            fixture.id
        );
        // Note: ClutterFilterMap may not be present in all trimmed fixtures
        // as it's typically in the first LDM record but may be segmented across records
        if found_clutter_map {
            println!("Fixture {} has ClutterFilterMap", fixture.id);
        }
    }
}

// =============================================================================
// INVARIANT TESTS: Key Header Fields
// =============================================================================

/// Verifies volume header fields are valid.
#[test]
fn test_volume_header_fields_valid() {
    for fixture in FIXTURES {
        let volume = volume::File::new(fixture.data.to_vec());
        let header = volume.header().expect("should have header");

        // Tape filename format check
        let tape_filename = header.tape_filename().expect("should have tape filename");
        assert!(
            tape_filename.starts_with("AR2V"),
            "Fixture {} invalid tape filename: {}",
            fixture.id,
            tape_filename
        );

        // Date/time should be parseable
        let datetime = header.date_time().expect("should have datetime");
        assert!(
            datetime.year() >= 2005 && datetime.year() <= 2030,
            "Fixture {} datetime out of range: {}",
            fixture.id,
            datetime
        );

        // ICAO should be valid
        let icao = header.icao_of_radar().expect("should have ICAO");
        assert_eq!(
            icao.len(),
            4,
            "Fixture {} ICAO wrong length: {}",
            fixture.id,
            icao
        );
    }
}

/// Verifies VCP number matches expected value.
#[test]
fn test_vcp_number_matches_expected() {
    for fixture in FIXTURES {
        let volume = volume::File::new(fixture.data.to_vec());
        let scan = volume.scan().expect("should convert to scan");

        assert_eq!(
            scan.coverage_pattern_number().number(),
            fixture.expected_vcp,
            "Fixture {} VCP mismatch",
            fixture.id
        );
    }
}

// =============================================================================
// INVARIANT TESTS: Elevation Angle Validity
// =============================================================================

/// Verifies elevation angles are within valid physical range.
#[test]
fn test_elevation_angles_valid() {
    for fixture in FIXTURES {
        let volume = volume::File::new(fixture.data.to_vec());
        let scan = volume.scan().expect("should convert to scan");

        for (sweep_idx, sweep) in scan.sweeps().iter().enumerate() {
            if let Some(first_radial) = sweep.radials().first() {
                let elevation = first_radial.elevation_angle_degrees();
                // NEXRAD elevation range is approximately -1 to 60 degrees
                assert!(
                    elevation >= -2.0 && elevation <= 65.0,
                    "Fixture {} sweep {} has invalid elevation: {}",
                    fixture.id,
                    sweep_idx,
                    elevation
                );
            }
        }
    }
}

/// Verifies azimuth angles are within 0-360 degree range.
#[test]
fn test_azimuth_angles_valid() {
    for fixture in FIXTURES {
        let volume = volume::File::new(fixture.data.to_vec());
        let scan = volume.scan().expect("should convert to scan");

        for (sweep_idx, sweep) in scan.sweeps().iter().enumerate() {
            for (radial_idx, radial) in sweep.radials().iter().enumerate() {
                let azimuth = radial.azimuth_angle_degrees();
                assert!(
                    azimuth >= 0.0 && azimuth < 360.0,
                    "Fixture {} sweep {} radial {} has invalid azimuth: {}",
                    fixture.id,
                    sweep_idx,
                    radial_idx,
                    azimuth
                );
            }
        }
    }
}

// =============================================================================
// INVARIANT TESTS: Elevation Number Consistency
// =============================================================================

/// Verifies all radials within a sweep have the same elevation number.
#[test]
fn test_elevation_number_consistent_within_sweep() {
    for fixture in FIXTURES {
        let volume = volume::File::new(fixture.data.to_vec());
        let scan = volume.scan().expect("should convert to scan");

        for (sweep_idx, sweep) in scan.sweeps().iter().enumerate() {
            let sweep_elev = sweep.elevation_number();

            for (radial_idx, radial) in sweep.radials().iter().enumerate() {
                assert_eq!(
                    radial.elevation_number(),
                    sweep_elev,
                    "Fixture {} sweep {} radial {} has mismatched elevation number",
                    fixture.id,
                    sweep_idx,
                    radial_idx
                );
            }
        }
    }
}

// =============================================================================
// INVARIANT TESTS: Azimuth Spacing Consistency
// =============================================================================

/// Verifies all radials within a sweep have consistent azimuth spacing.
#[test]
fn test_azimuth_spacing_consistent_within_sweep() {
    for fixture in FIXTURES {
        let volume = volume::File::new(fixture.data.to_vec());
        let scan = volume.scan().expect("should convert to scan");

        for (sweep_idx, sweep) in scan.sweeps().iter().enumerate() {
            if let Some(first_radial) = sweep.radials().first() {
                let expected_spacing = first_radial.azimuth_spacing_degrees();

                for (radial_idx, radial) in sweep.radials().iter().enumerate() {
                    let actual_spacing = radial.azimuth_spacing_degrees();
                    assert!(
                        (actual_spacing - expected_spacing).abs() < 0.01,
                        "Fixture {} sweep {} radial {} has inconsistent azimuth spacing: {} vs {}",
                        fixture.id,
                        sweep_idx,
                        radial_idx,
                        actual_spacing,
                        expected_spacing
                    );
                }
            }
        }
    }
}

// =============================================================================
// INVARIANT TESTS: Moment Data Structure
// =============================================================================

/// Verifies moment data (when present) has valid gate counts.
#[test]
fn test_moment_data_gate_counts_valid() {
    for fixture in FIXTURES {
        let volume = volume::File::new(fixture.data.to_vec());
        let scan = volume.scan().expect("should convert to scan");

        for (sweep_idx, sweep) in scan.sweeps().iter().enumerate() {
            for (radial_idx, radial) in sweep.radials().iter().enumerate() {
                // Check reflectivity if present
                if let Some(ref_data) = radial.reflectivity() {
                    assert!(
                        ref_data.gate_count() > 0 && ref_data.gate_count() <= 2000,
                        "Fixture {} sweep {} radial {} has invalid reflectivity gate count: {}",
                        fixture.id,
                        sweep_idx,
                        radial_idx,
                        ref_data.gate_count()
                    );
                }

                // Check velocity if present
                if let Some(vel_data) = radial.velocity() {
                    assert!(
                        vel_data.gate_count() > 0 && vel_data.gate_count() <= 2000,
                        "Fixture {} sweep {} radial {} has invalid velocity gate count: {}",
                        fixture.id,
                        sweep_idx,
                        radial_idx,
                        vel_data.gate_count()
                    );
                }
            }
        }
    }
}

/// Verifies at least one moment type is present in each radial.
#[test]
fn test_at_least_one_moment_present() {
    for fixture in FIXTURES {
        let volume = volume::File::new(fixture.data.to_vec());
        let scan = volume.scan().expect("should convert to scan");

        for (sweep_idx, sweep) in scan.sweeps().iter().enumerate() {
            for (radial_idx, radial) in sweep.radials().iter().enumerate() {
                let has_any_moment = radial.reflectivity().is_some()
                    || radial.velocity().is_some()
                    || radial.spectrum_width().is_some()
                    || radial.differential_reflectivity().is_some()
                    || radial.differential_phase().is_some()
                    || radial.correlation_coefficient().is_some()
                    || radial.clutter_filter_power().is_some();

                assert!(
                    has_any_moment,
                    "Fixture {} sweep {} radial {} has no moment data",
                    fixture.id, sweep_idx, radial_idx
                );
            }
        }
    }
}

// =============================================================================
// CROSS-FIXTURE COMPARISON TESTS
// =============================================================================

/// Verifies that different fixtures with the same VCP have similar structure.
#[test]
fn test_same_vcp_similar_structure() {
    // Group fixtures by VCP
    let vcp_212_fixtures: Vec<_> = FIXTURES.iter().filter(|f| f.expected_vcp == 212).collect();

    // All VCP 212 fixtures should have similar first-sweep properties
    for fixture in &vcp_212_fixtures {
        let volume = volume::File::new(fixture.data.to_vec());
        let scan = volume.scan().expect("should convert to scan");

        let first_sweep = &scan.sweeps()[0];
        let radial_count = first_sweep.radials().len();

        // VCP 212 typically has 720 radials in first sweep (0.5 degree spacing)
        assert!(
            radial_count == 720 || radial_count == 360,
            "Fixture {} VCP 212 first sweep has unusual radial count: {}",
            fixture.id,
            radial_count
        );
    }
}
