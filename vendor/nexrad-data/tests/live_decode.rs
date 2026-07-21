//! Live NEXRAD data decode integration tests.
//!
//! These tests download real data from the AWS archive bucket and verify
//! that the library can decode current production NEXRAD data correctly.
//!
//! Run with: cargo test --package nexrad-data --test live_decode --features aws -- --ignored

#![cfg(feature = "aws")]

use chrono::{Duration, Utc};
use nexrad_data::aws::archive::{download_file, list_files};
use nexrad_data::volume;
use nexrad_decode::messages::rda_status_data::RDABuildNumber;
use nexrad_decode::messages::{decode_messages, MessageContents};

/// Expected RDA build numbers per site.
/// Update these values when sites receive software upgrades.
///
/// To update after a build change:
/// 1. Run the test to see the new build number
/// 2. Verify the library decodes correctly with the new build
/// 3. Update the expected value here
const EXPECTED_BUILDS: &[(&str, RDABuildNumber)] = &[
    ("KDMX", RDABuildNumber::Build23_0), // Des Moines, IA - Central US
    ("KTLX", RDABuildNumber::Build24_0), // Norman, OK - Tornado Alley
    ("KLOT", RDABuildNumber::Build23_0), // Chicago/Romeoville, IL - Great Lakes
    ("KJAX", RDABuildNumber::Build23_0), // Jacksonville, FL - Southeast US
    ("KATX", RDABuildNumber::Build23_0), // Seattle, WA - Pacific Northwest
];

/// Minimum number of radar data messages expected in a complete volume.
const MIN_RADAR_DATA_MESSAGES: usize = 1000;

/// Result of decoding a volume file.
struct DecodeResult {
    site: String,
    file_name: String,
    build_number: RDABuildNumber,
    rda_status_count: usize,
    vcp_count: usize,
    radar_data_count: usize,
    has_volume_start: bool,
    has_volume_end: bool,
}

impl DecodeResult {
    fn print_report(&self, expected_build: &RDABuildNumber) {
        let build_status = if self.build_number == *expected_build {
            "PASS"
        } else {
            "FAIL"
        };

        println!("=== Decode Results for {} ===", self.site);
        println!("File: {}", self.file_name);
        println!(
            "Build Number: {:?} (expected: {:?}) - {}",
            self.build_number, expected_build, build_status
        );
        println!("RDA Status Messages: {}", self.rda_status_count);
        println!("VCP Messages: {}", self.vcp_count);
        println!("Radar Data Messages: {}", self.radar_data_count);
        println!("Volume Start Found: {}", self.has_volume_start);
        println!("Volume End Found: {}", self.has_volume_end);
        println!();
    }
}

/// Attempt to find and download a volume file for the given site.
/// Tries today, yesterday, and 2 days ago before giving up.
async fn get_latest_volume_file(site: &str) -> Option<(String, volume::File)> {
    let today = Utc::now().date_naive();

    for days_ago in 0..3i64 {
        let date = today - Duration::days(days_ago);

        match list_files(site, &date).await {
            Ok(files) => {
                // Filter out MDM files and get the latest non-MDM file
                let volume_files: Vec<_> = files
                    .into_iter()
                    .filter(|f| !f.name().ends_with("_MDM"))
                    .collect();

                if let Some(latest) = volume_files.last() {
                    let file_name = latest.name().to_string();
                    match download_file(latest.clone()).await {
                        Ok(file) => {
                            println!(
                                "Downloaded {} from {} ({} days ago)",
                                file_name, date, days_ago
                            );
                            return Some((file_name, file));
                        }
                        Err(e) => {
                            eprintln!("Failed to download {}: {}", file_name, e);
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("Failed to list files for {} on {}: {}", site, date, e);
            }
        }
    }

    None
}

/// Decode a volume file and extract statistics.
fn decode_volume(site: &str, file_name: &str, file: &volume::File) -> Result<DecodeResult, String> {
    let mut rda_status_count = 0;
    let mut vcp_count = 0;
    let mut radar_data_count = 0;
    let mut build_number: Option<RDABuildNumber> = None;
    let mut has_volume_start = false;
    let mut has_volume_end = false;

    let records: Vec<_> = file.records().expect("records").into_iter().collect();
    if records.is_empty() {
        return Err("No records found in volume".to_string());
    }

    for mut record in records {
        if record.compressed() {
            record = record
                .decompress()
                .map_err(|e| format!("Decompression failed: {}", e))?;
        }

        let messages =
            decode_messages(record.data()).map_err(|e| format!("Message decode failed: {}", e))?;

        for message in messages {
            match message.contents() {
                MessageContents::RDAStatusData(status) => {
                    rda_status_count += 1;
                    if build_number.is_none() {
                        build_number = Some(status.build_number());
                    }
                }
                MessageContents::VolumeCoveragePattern(_) => {
                    vcp_count += 1;
                }
                MessageContents::DigitalRadarData(radar_data) => {
                    radar_data_count += 1;
                    // Check for scan boundaries
                    match radar_data.header().radial_status_raw() {
                        3 => has_volume_start = true, // ScanStart
                        4 => has_volume_end = true,   // ScanEnd
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }

    let build_number =
        build_number.ok_or_else(|| "No RDA Status Data message found".to_string())?;

    Ok(DecodeResult {
        site: site.to_string(),
        file_name: file_name.to_string(),
        build_number,
        rda_status_count,
        vcp_count,
        radar_data_count,
        has_volume_start,
        has_volume_end,
    })
}

/// Run the live decode test for a specific site.
async fn run_site_test(site: &str, expected_build: &RDABuildNumber) {
    println!("\n{}", "=".repeat(60));
    println!("Testing site: {}", site);
    println!("{}\n", "=".repeat(60));

    let (file_name, file) = match get_latest_volume_file(site).await {
        Some(result) => result,
        None => {
            panic!(
                "SKIP: No data available for {} in the last 3 days. \
                 Site may be under maintenance.",
                site
            );
        }
    };

    let result = match decode_volume(site, &file_name, &file) {
        Ok(r) => r,
        Err(e) => {
            panic!("FAIL: Decode error for {}: {}", site, e);
        }
    };

    result.print_report(expected_build);

    // Collect all assertion failures
    let mut failures = Vec::new();

    // Build number check (primary assertion)
    if result.build_number != *expected_build {
        let msg = format!(
            "Build number mismatch: expected {:?}, got {:?}. \
             Update EXPECTED_BUILDS in live_decode.rs if this is expected.",
            expected_build, result.build_number
        );
        // GitHub Actions annotation for visibility
        println!(
            "::error file=nexrad-data/tests/live_decode.rs,title=Build Mismatch::\
             Site {} build number changed from {:?} to {:?}. Update EXPECTED_BUILDS constant.",
            site, expected_build, result.build_number
        );
        failures.push(msg);
    }

    // RDA Status check
    if result.rda_status_count == 0 {
        failures.push("No RDA Status Data messages found".to_string());
    }

    // VCP check
    if result.vcp_count == 0 {
        failures.push("No Volume Coverage Pattern messages found".to_string());
    }

    // Radar data count check
    if result.radar_data_count < MIN_RADAR_DATA_MESSAGES {
        failures.push(format!(
            "Insufficient radar data messages: {} (expected >= {})",
            result.radar_data_count, MIN_RADAR_DATA_MESSAGES
        ));
    }

    // Volume boundary check
    if !result.has_volume_start {
        failures.push("Volume scan start marker not found".to_string());
    }
    if !result.has_volume_end {
        failures.push("Volume scan end marker not found".to_string());
    }

    // Final assertion
    assert!(
        failures.is_empty(),
        "Test failed for {}:\n  - {}",
        site,
        failures.join("\n  - ")
    );

    println!("PASS: {} decoded successfully\n", site);
}

fn get_expected_build(site: &str) -> &'static RDABuildNumber {
    EXPECTED_BUILDS
        .iter()
        .find(|(s, _)| *s == site)
        .map(|(_, b)| b)
        .unwrap_or_else(|| panic!("Site {} not found in EXPECTED_BUILDS", site))
}

// Individual test functions for each site (allows matrix strategy in CI)

#[tokio::test]
#[ignore = "requires AWS access - run weekly"]
async fn test_live_decode_kdmx() {
    run_site_test("KDMX", get_expected_build("KDMX")).await;
}

#[tokio::test]
#[ignore = "requires AWS access - run weekly"]
async fn test_live_decode_ktlx() {
    run_site_test("KTLX", get_expected_build("KTLX")).await;
}

#[tokio::test]
#[ignore = "requires AWS access - run weekly"]
async fn test_live_decode_klot() {
    run_site_test("KLOT", get_expected_build("KLOT")).await;
}

#[tokio::test]
#[ignore = "requires AWS access - run weekly"]
async fn test_live_decode_kjax() {
    run_site_test("KJAX", get_expected_build("KJAX")).await;
}

#[tokio::test]
#[ignore = "requires AWS access - run weekly"]
async fn test_live_decode_katx() {
    run_site_test("KATX", get_expected_build("KATX")).await;
}

/// Combined test for local development - runs all sites sequentially.
#[tokio::test]
#[ignore = "requires AWS access - run weekly"]
async fn test_live_decode_all_sites() {
    for (site, expected_build) in EXPECTED_BUILDS {
        run_site_test(site, expected_build).await;
    }
    println!("\nAll sites decoded successfully!");
}
