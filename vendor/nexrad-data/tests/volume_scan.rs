#![cfg(feature = "nexrad-model")]

use nexrad_data::volume;
use nexrad_model::data::{DataMoment, RadialStatus};

const TEST_NEXRAD_FILE: &[u8] = include_bytes!("../../downloads/KDMX20220305_232324_V06");

#[test]
fn test_scan_basic_structure() {
    let volume = volume::File::new(TEST_NEXRAD_FILE.to_vec());
    let scan = volume.scan().expect("Scan conversion should succeed");

    // Verify coverage pattern number
    assert_eq!(scan.coverage_pattern_number().number(), 212);

    // Verify number of sweeps (VCP 212 has 23 elevation cuts in this file, with split-cuts)
    assert_eq!(scan.sweeps().len(), 23);
}

#[test]
fn test_scan_sweep_properties() {
    let volume = volume::File::new(TEST_NEXRAD_FILE.to_vec());
    let scan = volume.scan().expect("Scan conversion should succeed");

    let sweeps = scan.sweeps();
    assert_eq!(sweeps.len(), 23);

    for (i, sweep) in sweeps.iter().enumerate() {
        assert_eq!(
            sweep.elevation_number(),
            (i + 1) as u8,
            "Sweep {} elevation number should be sequential",
            i
        );

        // Sweeps 0-5 and 11-16 have 0.5° spacing (720 radials)
        // Sweeps 6-10, 17-22 have 1.0° spacing (360 radials)
        let expected_radial_count = if (0..=5).contains(&i) || (11..=16).contains(&i) {
            720
        } else {
            360
        };

        assert_eq!(
            sweep.radials().len(),
            expected_radial_count,
            "Sweep {} radial count mismatch",
            i
        );
    }
}

#[test]
fn test_scan_radial_properties() {
    let volume = volume::File::new(TEST_NEXRAD_FILE.to_vec());
    let scan = volume.scan().expect("Scan conversion should succeed");

    let sweeps = scan.sweeps();

    // Test first sweep (elevation 1) with 720 radials
    let sweep0 = &sweeps[0];
    let first_radial = sweep0.radials().first().expect("First radial should exist");

    assert_eq!(first_radial.azimuth_number(), 1);
    assert!((first_radial.azimuth_angle_degrees() - 155.22).abs() < 0.01);
    assert!((first_radial.azimuth_spacing_degrees() - 0.50).abs() < 0.01);
    assert!((first_radial.elevation_angle_degrees() - 0.61).abs() < 0.01);
    assert_eq!(first_radial.radial_status(), RadialStatus::ScanStart);
    assert_eq!(first_radial.elevation_number(), 1);

    let last_radial = sweep0.radials().last().expect("Last radial should exist");
    assert_eq!(last_radial.azimuth_number(), 720);
    assert!((last_radial.azimuth_angle_degrees() - 154.68).abs() < 0.01);

    // Test a sweep with 1-degree spacing (360 radials)
    let sweep6 = &sweeps[6];
    let first_radial_360 = sweep6.radials().first().expect("First radial should exist");

    assert_eq!(first_radial_360.azimuth_number(), 1);
    assert!((first_radial_360.azimuth_spacing_degrees() - 1.00).abs() < 0.01);
    assert!((first_radial_360.elevation_angle_degrees() - 1.86).abs() < 0.01);
    assert_eq!(
        first_radial_360.radial_status(),
        RadialStatus::ElevationStart
    );
    assert_eq!(first_radial_360.elevation_number(), 7);
}

#[test]
fn test_scan_moment_data_presence() {
    let volume = volume::File::new(TEST_NEXRAD_FILE.to_vec());
    let scan = volume.scan().expect("Scan conversion should succeed");

    let sweeps = scan.sweeps();

    // Sweep 0: Dual-pol only (no velocity/spectrum width)
    let sweep0_radial = sweeps[0].radials().first().unwrap();
    assert!(sweep0_radial.reflectivity().is_some());
    assert!(sweep0_radial.velocity().is_none());
    assert!(sweep0_radial.spectrum_width().is_none());
    assert!(sweep0_radial.differential_reflectivity().is_some());
    assert!(sweep0_radial.differential_phase().is_some());
    assert!(sweep0_radial.correlation_coefficient().is_some());
    assert!(sweep0_radial.clutter_filter_power().is_some());

    // Sweep 1: Doppler only (no dual-pol)
    let sweep1_radial = sweeps[1].radials().first().unwrap();
    assert!(sweep1_radial.reflectivity().is_some());
    assert!(sweep1_radial.velocity().is_some());
    assert!(sweep1_radial.spectrum_width().is_some());
    assert!(sweep1_radial.differential_reflectivity().is_none());
    assert!(sweep1_radial.differential_phase().is_none());
    assert!(sweep1_radial.correlation_coefficient().is_none());
    assert!(sweep1_radial.clutter_filter_power().is_none());

    // Sweep 6: All moment types present
    let sweep6_radial = sweeps[6].radials().first().unwrap();
    assert!(sweep6_radial.reflectivity().is_some());
    assert!(sweep6_radial.velocity().is_some());
    assert!(sweep6_radial.spectrum_width().is_some());
    assert!(sweep6_radial.differential_reflectivity().is_some());
    assert!(sweep6_radial.differential_phase().is_some());
    assert!(sweep6_radial.correlation_coefficient().is_some());
    assert!(sweep6_radial.clutter_filter_power().is_some());
}

#[test]
fn test_scan_moment_data_details() {
    let volume = volume::File::new(TEST_NEXRAD_FILE.to_vec());
    let scan = volume.scan().expect("Scan conversion should succeed");

    let sweeps = scan.sweeps();

    // Test reflectivity moment data from sweep 0
    let sweep0_radial = sweeps[0].radials().first().unwrap();
    let reflectivity = sweep0_radial
        .reflectivity()
        .expect("Reflectivity should be present");

    assert_eq!(reflectivity.gate_count(), 1832);
    assert!((reflectivity.first_gate_range_km() - 2.125).abs() < 0.001);
    assert!((reflectivity.gate_interval_km() - 0.250).abs() < 0.001);

    // Test velocity moment data from sweep 1
    let sweep1_radial = sweeps[1].radials().first().unwrap();
    let velocity = sweep1_radial
        .velocity()
        .expect("Velocity should be present");

    assert_eq!(velocity.gate_count(), 1192);
    assert!((velocity.first_gate_range_km() - 2.125).abs() < 0.001);
    assert!((velocity.gate_interval_km() - 0.250).abs() < 0.001);

    // Test different gate counts across sweeps with varying elevation angles
    let sweep17_radial = sweeps[17].radials().first().unwrap();
    let ref_sweep17 = sweep17_radial
        .reflectivity()
        .expect("Reflectivity should be present");
    assert_eq!(ref_sweep17.gate_count(), 680);

    let sweep21_radial = sweeps[21].radials().first().unwrap();
    let ref_sweep21 = sweep21_radial
        .reflectivity()
        .expect("Reflectivity should be present");
    assert_eq!(ref_sweep21.gate_count(), 296);
}

#[test]
fn test_scan_elevation_angles() {
    let volume = volume::File::new(TEST_NEXRAD_FILE.to_vec());
    let scan = volume.scan().expect("Scan conversion should succeed");

    let sweeps = scan.sweeps();

    // Verify elevation angles for each sweep (VCP 212 with 23 cuts)
    #[allow(clippy::approx_constant)]
    let expected_elevations = vec![
        0.61, 0.53, 0.84, 0.92, 1.27, 1.36, 1.86, 2.25, 2.95, 3.85, 4.91, 0.27, 0.53, 0.76, 0.92,
        1.27, 1.36, 6.28, 7.83, 9.88, 12.32, 15.41, 19.41,
    ];

    for (i, sweep) in sweeps.iter().enumerate() {
        let first_radial = sweep.radials().first().unwrap();
        let actual_elev = first_radial.elevation_angle_degrees();
        let expected_elev = expected_elevations[i];

        assert!(
            (actual_elev - expected_elev).abs() < 0.1,
            "Sweep {} elevation angle mismatch: expected {:.2}, got {:.2}",
            i,
            expected_elev,
            actual_elev
        );
    }
}

#[test]
fn test_scan_azimuth_spacing_patterns() {
    let volume = volume::File::new(TEST_NEXRAD_FILE.to_vec());
    let scan = volume.scan().expect("Scan conversion should succeed");

    let sweeps = scan.sweeps();

    // Sweeps 0-5, 11-16 should have 0.5 degree spacing (720 radials)
    for i in [0, 1, 2, 3, 4, 5, 11, 12, 13, 14, 15, 16] {
        let first_radial = sweeps[i].radials().first().unwrap();
        assert!((first_radial.azimuth_spacing_degrees() - 0.50).abs() < 0.01);
        assert_eq!(sweeps[i].radials().len(), 720);
    }

    // Sweeps 6-10, 17-22 should have 1.0 degree spacing (360 radials)
    for i in [6, 7, 8, 9, 10, 17, 18, 19, 20, 21, 22] {
        let first_radial = sweeps[i].radials().first().unwrap();
        assert!((first_radial.azimuth_spacing_degrees() - 1.00).abs() < 0.01);
        assert_eq!(sweeps[i].radials().len(), 360);
    }
}

#[test]
fn test_scan_radial_timestamps() {
    let volume = volume::File::new(TEST_NEXRAD_FILE.to_vec());
    let scan = volume.scan().expect("Scan conversion should succeed");

    let sweeps = scan.sweeps();

    // All radials should have valid timestamps
    for sweep in sweeps {
        for radial in sweep.radials() {
            let timestamp = radial.collection_timestamp();
            // Timestamps should be reasonable (after year 2000)
            assert!(
                timestamp > 946684800000,
                "Timestamp should be after year 2000"
            );
            // Timestamps should be before year 2100
            assert!(
                timestamp < 4102444800000,
                "Timestamp should be before year 2100"
            );
        }
    }

    // Timestamps within a sweep should be sequential
    let first_sweep = &sweeps[0];
    let first_timestamp = first_sweep
        .radials()
        .first()
        .unwrap()
        .collection_timestamp();
    let last_timestamp = first_sweep.radials().last().unwrap().collection_timestamp();
    assert!(
        last_timestamp >= first_timestamp,
        "Timestamps should be sequential within a sweep"
    );
}

#[test]
fn test_scan_moment_values() {
    let volume = volume::File::new(TEST_NEXRAD_FILE.to_vec());
    let scan = volume.scan().expect("Scan conversion should succeed");

    let sweeps = scan.sweeps();
    let first_radial = sweeps[0].radials().first().unwrap();
    let reflectivity = first_radial
        .reflectivity()
        .expect("Reflectivity should be present");

    // Get the decoded values
    let values = reflectivity.values();

    // Should have the same number of values as gate count
    assert_eq!(values.len(), reflectivity.gate_count() as usize);

    // Values should be reasonable (mix of actual values and special cases)
    let mut has_value = false;
    let mut has_below_threshold = false;

    use nexrad_model::data::MomentValue;
    for value in values.iter() {
        match value {
            MomentValue::Value(v) => {
                has_value = true;
                // Reflectivity values typically range from -32 to +94.5 dBZ
                assert!(
                    *v >= -50.0 && *v <= 100.0,
                    "Reflectivity value out of expected range: {}",
                    v
                );
            }
            MomentValue::BelowThreshold => {
                has_below_threshold = true;
            }
            MomentValue::RangeFolded => {}
        }
    }

    // Should have at least some actual values and some below threshold
    assert!(has_value, "Should have at least some reflectivity values");
    assert!(
        has_below_threshold,
        "Should have at least some below-threshold gates"
    );
}

#[test]
fn test_scan_all_radials_have_elevation() {
    let volume = volume::File::new(TEST_NEXRAD_FILE.to_vec());
    let scan = volume.scan().expect("Scan conversion should succeed");

    // Ensure all radials in all sweeps have matching elevation numbers
    for sweep in scan.sweeps() {
        let expected_elevation = sweep.elevation_number();
        for radial in sweep.radials() {
            assert_eq!(
                radial.elevation_number(),
                expected_elevation,
                "All radials in a sweep should have the same elevation number"
            );
        }
    }
}

#[test]
fn test_scan_radial_azimuth_coverage() {
    let volume = volume::File::new(TEST_NEXRAD_FILE.to_vec());
    let scan = volume.scan().expect("Scan conversion should succeed");

    // For each sweep, verify azimuth numbers are sequential
    for (i, sweep) in scan.sweeps().iter().enumerate() {
        let radials = sweep.radials();
        let first_azimuth = radials.first().unwrap().azimuth_number();
        let last_azimuth = radials.last().unwrap().azimuth_number();

        // First radial should be azimuth number 1
        assert_eq!(first_azimuth, 1, "Sweep {} first azimuth should be 1", i);

        // Last azimuth should match radial count
        assert_eq!(
            last_azimuth,
            radials.len() as u16,
            "Sweep {} last azimuth should equal radial count",
            i
        );

        // Verify all azimuth numbers are sequential
        for (j, radial) in radials.iter().enumerate() {
            assert_eq!(
                radial.azimuth_number(),
                (j + 1) as u16,
                "Sweep {} radial {} azimuth number should be sequential",
                i,
                j
            );
        }
    }
}
