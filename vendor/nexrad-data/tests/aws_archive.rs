#![cfg(feature = "aws")]

use chrono::{Datelike, NaiveDate, Timelike};
use nexrad_data::aws::archive::{self, Identifier};

#[test]
fn test_identifier_new() {
    let name = "KDMX20220305_232324_V06".to_string();
    let identifier = Identifier::new(name.clone());

    assert_eq!(identifier.name(), name);
}

#[test]
fn test_identifier_site() {
    let identifier = Identifier::new("KDMX20220305_232324_V06".to_string());
    assert_eq!(identifier.site(), Some("KDMX"));

    let identifier = Identifier::new("KABR20220305_120000_V06".to_string());
    assert_eq!(identifier.site(), Some("KABR"));

    // Test with name too short
    let identifier = Identifier::new("KDM".to_string());
    assert_eq!(identifier.site(), None);
}

#[test]
fn test_identifier_date_time() {
    // Valid identifier
    let identifier = Identifier::new("KDMX20220305_232324_V06".to_string());
    let date_time = identifier.date_time();

    assert!(date_time.is_some());
    let date_time = date_time.unwrap();

    assert_eq!(date_time.year(), 2022);
    assert_eq!(date_time.month(), 3);
    assert_eq!(date_time.day(), 5);
    assert_eq!(date_time.hour(), 23);
    assert_eq!(date_time.minute(), 23);
    assert_eq!(date_time.second(), 24);
}

#[test]
fn test_identifier_date_time_various_dates() {
    // Test different valid dates
    let test_cases = vec![
        ("KDMX20220101_000000_V06", 2022, 1, 1, 0, 0, 0),
        ("KDMX20221231_235959_V06", 2022, 12, 31, 23, 59, 59),
        ("KDMX20200229_120000_V06", 2020, 2, 29, 12, 0, 0), // Leap year
        ("KDMX20230615_153045_V06", 2023, 6, 15, 15, 30, 45),
    ];

    for (name, year, month, day, hour, minute, second) in test_cases {
        let identifier = Identifier::new(name.to_string());
        let date_time = identifier.date_time();

        assert!(date_time.is_some(), "Failed to parse date from: {}", name);
        let date_time = date_time.unwrap();

        assert_eq!(date_time.year(), year);
        assert_eq!(date_time.month(), month);
        assert_eq!(date_time.day(), day);
        assert_eq!(date_time.hour(), hour);
        assert_eq!(date_time.minute(), minute);
        assert_eq!(date_time.second(), second);
    }
}

#[test]
fn test_identifier_date_time_invalid() {
    // Invalid date format
    let identifier = Identifier::new("KDMX2022030_232324_V06".to_string()); // Too short
    assert_eq!(identifier.date_time(), None);

    // Invalid date values
    let identifier = Identifier::new("KDMX20221301_120000_V06".to_string()); // Month 13
    assert_eq!(identifier.date_time(), None);

    let identifier = Identifier::new("KDMX20220230_120000_V06".to_string()); // Feb 30
    assert_eq!(identifier.date_time(), None);

    // Invalid time format
    let identifier = Identifier::new("KDMX20220305_2323_V06".to_string()); // Too short
    assert_eq!(identifier.date_time(), None);

    // Invalid time values
    let identifier = Identifier::new("KDMX20220305_256000_V06".to_string()); // Hour 25
    assert_eq!(identifier.date_time(), None);

    // Name too short overall
    let identifier = Identifier::new("KDMX".to_string());
    assert_eq!(identifier.date_time(), None);
}

#[test]
fn test_identifier_ordering() {
    let id1 = Identifier::new("KDMX20220305_120000_V06".to_string());
    let id2 = Identifier::new("KDMX20220305_130000_V06".to_string());
    let id3 = Identifier::new("KDMX20220306_120000_V06".to_string());

    assert!(id1 < id2);
    assert!(id2 < id3);
    assert!(id1 < id3);
}

#[test]
fn test_identifier_equality() {
    let id1 = Identifier::new("KDMX20220305_232324_V06".to_string());
    let id2 = Identifier::new("KDMX20220305_232324_V06".to_string());
    let id3 = Identifier::new("KDMX20220305_232325_V06".to_string());

    assert!(id1 == id2);
    assert!(id1 != id3);
}

#[test]
fn test_identifier_clone() {
    let original = Identifier::new("KDMX20220305_232324_V06".to_string());
    let cloned = original.clone();

    assert!(original == cloned);
    assert_eq!(original.name(), cloned.name());
    assert_eq!(original.site(), cloned.site());
    assert_eq!(original.date_time(), cloned.date_time());
}

// Integration tests that require network access
// These are marked with #[ignore] so they don't run by default

#[tokio::test]
#[ignore = "requires AWS access"]
async fn test_list_files_basic() {
    let site = "KDMX";
    let date = NaiveDate::from_ymd_opt(2022, 3, 5).unwrap();

    let result = archive::list_files(site, &date).await;

    assert!(result.is_ok(), "Failed to list files: {:?}", result.err());
    let files = result.unwrap();

    // There should be files for this date/site (bucket may be empty during testing)
    if !files.is_empty() {
        // All files should have the correct site
        for file in &files {
            assert_eq!(file.site(), Some(site));
        }
    } else {
        println!(
            "Warning: No files found for KDMX on 2022-03-05, bucket may be empty or inaccessible"
        );
    }
}

#[tokio::test]
#[ignore = "requires AWS access"]
async fn test_list_files_no_data() {
    let site = "KDMX";
    // Use a date far in the future where no data should exist
    let date = NaiveDate::from_ymd_opt(2099, 12, 31).unwrap();

    let result = archive::list_files(site, &date).await;

    assert!(result.is_ok());
    let files = result.unwrap();

    // Should return empty list for future date
    assert!(
        files.is_empty(),
        "Expected no files for future date, got: {}",
        files.len()
    );
}

#[tokio::test]
#[ignore = "requires AWS access"]
async fn test_list_files_sorted() {
    let site = "KDMX";
    let date = NaiveDate::from_ymd_opt(2022, 3, 5).unwrap();

    let result = archive::list_files(site, &date).await;
    assert!(result.is_ok());
    let files = result.unwrap();

    if files.len() > 1 {
        // Files should be sorted by time
        for i in 0..files.len() - 1 {
            let curr_time = files[i].date_time();
            let next_time = files[i + 1].date_time();

            if let (Some(curr), Some(next)) = (curr_time, next_time) {
                assert!(
                    curr <= next,
                    "Files not sorted: {} comes after {}",
                    files[i].name(),
                    files[i + 1].name()
                );
            }
        }
    }
}

#[tokio::test]
#[ignore = "requires AWS access"]
async fn test_download_file_basic() {
    let site = "KDMX";
    let date = NaiveDate::from_ymd_opt(2022, 3, 5).unwrap();

    // First list files to get an actual file that exists
    let list_result = archive::list_files(site, &date).await;
    assert!(list_result.is_ok(), "Failed to list files");
    let files = list_result.unwrap();

    if files.is_empty() {
        // Skip test if no files are available
        println!("No files available for testing, skipping");
        return;
    }

    // Download the first available file
    let identifier = files.first().unwrap().clone();
    let result = archive::download_file(identifier.clone()).await;

    assert!(
        result.is_ok(),
        "Failed to download file {}: {:?}",
        identifier.name(),
        result.err()
    );
    let file = result.unwrap();

    // File should have data
    assert!(!file.data().is_empty(), "Downloaded file has no data");

    // File should start with AR2 header
    assert!(
        file.data().starts_with(b"AR2"),
        "File doesn't start with AR2 header"
    );
}

#[tokio::test]
#[ignore = "requires AWS access"]
async fn test_download_file_nonexistent() {
    // Use an identifier that should not exist
    let identifier = Identifier::new("KDMX20990101_000000_V06".to_string());

    let result = archive::download_file(identifier).await;

    // Should return an error for non-existent file
    assert!(result.is_err(), "Expected error for non-existent file");
}

#[tokio::test]
#[ignore = "requires AWS access"]
async fn test_download_file_invalid_identifier() {
    // Identifier without valid site
    let identifier = Identifier::new("INVALID_NAME".to_string());

    let result = archive::download_file(identifier).await;

    assert!(result.is_err(), "Expected error for invalid identifier");
}

#[tokio::test]
#[ignore = "requires AWS access"]
async fn test_list_and_download_workflow() {
    // Test a complete workflow: list files, then download one
    let site = "KDMX";
    let date = NaiveDate::from_ymd_opt(2022, 3, 5).unwrap();

    // List files
    let list_result = archive::list_files(site, &date).await;
    assert!(list_result.is_ok());
    let files = list_result.unwrap();

    if files.is_empty() {
        println!("No files available for testing workflow, skipping");
        return;
    }

    // Download the first file
    let first_file = files.first().unwrap().clone();
    let download_result = archive::download_file(first_file.clone()).await;
    assert!(
        download_result.is_ok(),
        "Failed to download: {:?}",
        download_result.err()
    );

    let file = download_result.unwrap();

    // Verify the file content matches expectations
    assert!(!file.data().is_empty());
    assert!(file.data().starts_with(b"AR2"));

    // Verify we can get records from the downloaded file
    let records = file.records().expect("records");
    assert!(!records.is_empty(), "File should contain records");
}

#[test]
fn test_identifier_mdm_file() {
    // MDM files have a different naming convention
    let identifier = Identifier::new("KDMX20220305_232324_V06_MDM".to_string());

    assert_eq!(identifier.site(), Some("KDMX"));
    // Date/time parsing should still work
    assert!(identifier.date_time().is_some());
}

#[test]
fn test_identifier_various_sites() {
    let sites = vec!["KDMX", "KABR", "KATX", "KFTG", "KGJX"];

    for site in sites {
        let name = format!("{site}20220305_120000_V06");
        let identifier = Identifier::new(name);

        assert_eq!(identifier.site(), Some(site));
        assert!(identifier.date_time().is_some());
    }
}

#[test]
fn test_identifier_hash() {
    use std::collections::HashSet;

    let id1 = Identifier::new("KDMX20220305_120000_V06".to_string());
    let id2 = Identifier::new("KDMX20220305_120000_V06".to_string());
    let id3 = Identifier::new("KDMX20220305_130000_V06".to_string());

    let mut set = HashSet::new();
    set.insert(id1.clone());
    set.insert(id2.clone());
    set.insert(id3.clone());

    // id1 and id2 are equal, so set should have 2 elements
    assert_eq!(set.len(), 2);
}

#[tokio::test]
#[ignore = "requires AWS access"]
async fn test_list_files_different_sites() {
    let date = NaiveDate::from_ymd_opt(2022, 3, 5).unwrap();
    let sites = vec!["KDMX", "KABR"];

    for site in sites {
        let result = archive::list_files(site, &date).await;
        assert!(result.is_ok(), "Failed to list files for site: {}", site);

        let files = result.unwrap();
        // All returned files should be for the requested site
        for file in files {
            assert_eq!(
                file.site(),
                Some(site),
                "File {} has wrong site",
                file.name()
            );
        }
    }
}
