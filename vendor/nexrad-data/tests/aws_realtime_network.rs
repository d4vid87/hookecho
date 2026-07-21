#![cfg(feature = "aws")]

use nexrad_data::aws::realtime::{self, ChunkIdentifier, ChunkType, VolumeIndex};

// These are integration tests that require network access to AWS
// They are marked with #[ignore] so they don't run by default

#[tokio::test]
#[ignore = "requires AWS access"]
async fn test_get_latest_volume_basic() {
    let site = "KDMX";

    let result = realtime::get_latest_volume(site).await;

    assert!(
        result.is_ok(),
        "Failed to get latest volume: {:?}",
        result.err()
    );
    let latest = result.unwrap();

    // Should have found a volume
    assert!(
        latest.volume.is_some(),
        "No latest volume found for {}",
        site
    );

    // Should have made some network calls
    assert!(latest.calls > 0, "Expected at least one network call");

    // Network calls should be reasonable (binary search should be efficient)
    assert!(
        latest.calls < 20,
        "Too many network calls: {}",
        latest.calls
    );
}

#[tokio::test]
#[ignore = "requires AWS access"]
async fn test_get_latest_volume_different_sites() {
    let sites = vec!["KDMX", "KABR", "KATX"];

    for site in sites {
        let result = realtime::get_latest_volume(site).await;
        assert!(result.is_ok(), "Failed to get latest volume for {}", site);

        let latest = result.unwrap();
        if let Some(volume) = latest.volume {
            // Volume should be in valid range
            assert!(volume.as_number() >= 1 && volume.as_number() <= 999);
        }
    }
}

#[tokio::test]
#[ignore = "requires AWS access"]
async fn test_list_chunks_in_volume_basic() {
    let site = "KDMX";

    // First get the latest volume
    let latest_result = realtime::get_latest_volume(site).await;
    assert!(latest_result.is_ok());
    let latest = latest_result.unwrap();
    assert!(latest.volume.is_some());

    let volume = latest.volume.unwrap();

    // List chunks in that volume
    let chunks_result = realtime::list_chunks_in_volume(site, volume, 100).await;
    assert!(
        chunks_result.is_ok(),
        "Failed to list chunks: {:?}",
        chunks_result.err()
    );

    let chunks = chunks_result.unwrap();

    // Should have some chunks (active volumes typically have chunks)
    if !chunks.is_empty() {
        // All chunks should be for the requested site and volume
        for chunk in &chunks {
            assert_eq!(chunk.site(), site);
            assert_eq!(chunk.volume(), &volume);
        }

        // Chunks should be ordered by sequence
        for i in 0..chunks.len() - 1 {
            assert!(
                chunks[i].sequence() <= chunks[i + 1].sequence(),
                "Chunks not ordered by sequence"
            );
        }

        // First chunk should be a Start chunk
        assert_eq!(chunks[0].chunk_type(), ChunkType::Start);
        assert_eq!(chunks[0].sequence(), 1);
    }
}

#[tokio::test]
#[ignore = "requires AWS access"]
async fn test_list_chunks_in_volume_with_limit() {
    let site = "KDMX";

    let latest_result = realtime::get_latest_volume(site).await;
    assert!(latest_result.is_ok());
    let latest = latest_result.unwrap();
    assert!(latest.volume.is_some());

    let volume = latest.volume.unwrap();

    // Request a limited number of chunks
    let limit = 5;
    let chunks_result = realtime::list_chunks_in_volume(site, volume, limit).await;
    assert!(chunks_result.is_ok());

    let chunks = chunks_result.unwrap();

    // Should not exceed the limit
    assert!(chunks.len() <= limit, "Returned more chunks than requested");
}

#[tokio::test]
#[ignore = "requires AWS access"]
async fn test_list_chunks_empty_volume() {
    let site = "KDMX";

    // Try to list chunks from a volume that's likely empty
    // Volume 1 is often empty or very old
    let empty_volume = VolumeIndex::new(500); // Pick a random volume that's likely empty

    let chunks_result = realtime::list_chunks_in_volume(site, empty_volume, 10).await;
    assert!(chunks_result.is_ok());

    let _chunks = chunks_result.unwrap();
    // Empty volume should return empty list (no error)
    // _chunks.is_empty() is expected for empty volumes
}

#[tokio::test]
#[ignore = "requires AWS access"]
async fn test_download_chunk_basic() {
    let site = "KDMX";

    // Get latest volume
    let latest_result = realtime::get_latest_volume(site).await;
    assert!(latest_result.is_ok());
    let latest = latest_result.unwrap();
    assert!(latest.volume.is_some());

    let volume = latest.volume.unwrap();

    // List chunks
    let chunks_result = realtime::list_chunks_in_volume(site, volume, 10).await;
    assert!(chunks_result.is_ok());
    let chunks = chunks_result.unwrap();

    if chunks.is_empty() {
        // Skip test if no chunks available
        return;
    }

    // Download the first chunk
    let first_chunk_id = &chunks[0];
    let download_result = realtime::download_chunk(site, first_chunk_id).await;

    assert!(
        download_result.is_ok(),
        "Failed to download chunk: {:?}",
        download_result.err()
    );

    let (returned_chunk_id, chunk) = download_result.unwrap();

    // Verify chunk identifier matches
    assert_eq!(returned_chunk_id.site(), site);
    assert_eq!(returned_chunk_id.volume(), &volume);
    assert_eq!(returned_chunk_id.sequence(), first_chunk_id.sequence());

    // Verify chunk has data
    assert!(!chunk.data().is_empty(), "Downloaded chunk has no data");

    // First chunk should be a Start chunk with AR2 header
    use nexrad_data::aws::realtime::Chunk;
    match chunk {
        Chunk::Start(file) => {
            assert!(
                file.data().starts_with(b"AR2"),
                "Start chunk should have AR2 header"
            );
        }
        _ => {
            panic!("First chunk should be Chunk::Start variant");
        }
    }
}

#[tokio::test]
#[ignore = "requires AWS access"]
async fn test_download_chunk_has_upload_time() {
    let site = "KDMX";

    let latest_result = realtime::get_latest_volume(site).await;
    assert!(latest_result.is_ok());
    let latest = latest_result.unwrap();
    assert!(latest.volume.is_some());

    let volume = latest.volume.unwrap();

    let chunks_result = realtime::list_chunks_in_volume(site, volume, 5).await;
    assert!(chunks_result.is_ok());
    let chunks = chunks_result.unwrap();

    if chunks.is_empty() {
        return;
    }

    // Download a chunk
    let chunk_id = &chunks[0];
    let download_result = realtime::download_chunk(site, chunk_id).await;
    assert!(download_result.is_ok());

    let (returned_chunk_id, _chunk) = download_result.unwrap();

    // Downloaded chunk should have upload time populated
    assert!(
        returned_chunk_id.upload_date_time().is_some(),
        "Downloaded chunk should have upload date/time"
    );
}

#[tokio::test]
#[ignore = "requires AWS access"]
async fn test_download_nonexistent_chunk() {
    let site = "KDMX";

    // Create a chunk identifier that doesn't exist
    use chrono::NaiveDateTime;
    let nonexistent_chunk = ChunkIdentifier::new(
        site.to_string(),
        VolumeIndex::new(999),
        NaiveDateTime::parse_from_str("20990101_000000", "%Y%m%d_%H%M%S").unwrap(),
        1,
        ChunkType::Start,
        None,
    );

    let result = realtime::download_chunk(site, &nonexistent_chunk).await;

    // Should return an error for non-existent chunk
    assert!(result.is_err(), "Expected error for non-existent chunk");
}

#[tokio::test]
#[ignore = "requires AWS access"]
async fn test_complete_workflow() {
    // Test a complete workflow: get latest volume, list chunks, download one
    let site = "KDMX";

    // Step 1: Get latest volume
    let latest_result = realtime::get_latest_volume(site).await;
    assert!(latest_result.is_ok());
    let latest = latest_result.unwrap();
    assert!(latest.volume.is_some());

    let volume = latest.volume.unwrap();
    println!("Latest volume for {}: {}", site, volume.as_number());

    // Step 2: List chunks in that volume
    let chunks_result = realtime::list_chunks_in_volume(site, volume, 10).await;
    assert!(chunks_result.is_ok());
    let chunks = chunks_result.unwrap();

    if chunks.is_empty() {
        println!("No chunks found in volume {}", volume.as_number());
        return;
    }

    println!("Found {} chunks in volume", chunks.len());

    // Step 3: Download the first chunk
    let first_chunk = &chunks[0];
    let download_result = realtime::download_chunk(site, first_chunk).await;
    assert!(download_result.is_ok());

    let (chunk_id, chunk) = download_result.unwrap();

    println!(
        "Downloaded chunk: {} (sequence {}, size {} bytes)",
        chunk_id.name(),
        chunk_id.sequence(),
        chunk.data().len()
    );

    // Verify the chunk is valid
    assert!(!chunk.data().is_empty());
    assert_eq!(chunk_id.site(), site);
    assert_eq!(chunk_id.volume(), &volume);
}

#[tokio::test]
#[ignore = "requires AWS access"]
async fn test_chunk_sequencing() {
    // Test that chunks in a volume follow proper sequencing
    let site = "KDMX";

    let latest_result = realtime::get_latest_volume(site).await;
    assert!(latest_result.is_ok());
    let latest = latest_result.unwrap();
    assert!(latest.volume.is_some());

    let volume = latest.volume.unwrap();

    let chunks_result = realtime::list_chunks_in_volume(site, volume, 100).await;
    assert!(chunks_result.is_ok());
    let chunks = chunks_result.unwrap();

    if chunks.is_empty() {
        return;
    }

    // First chunk should be sequence 1
    assert_eq!(chunks[0].sequence(), 1);

    // All chunks should have same date_time_prefix
    let date_time_prefix = chunks[0].date_time_prefix();
    for chunk in &chunks {
        assert_eq!(chunk.date_time_prefix(), date_time_prefix);
    }

    // Chunks should be sequential (may have gaps)
    for i in 0..chunks.len() - 1 {
        assert!(chunks[i].sequence() < chunks[i + 1].sequence());
    }

    // Last chunk should be End type if volume is complete
    let last_chunk = chunks.last().unwrap();
    if last_chunk.chunk_type() == ChunkType::End {
        // If we have an End chunk, verify it's the last sequence
        // (typically around 55 for most VCPs)
        assert!(
            last_chunk.sequence() > 20,
            "End chunk should have reasonable sequence number"
        );
    }
}

#[tokio::test]
#[ignore = "requires AWS access"]
async fn test_multiple_sites_concurrently() {
    // Test that we can query multiple sites concurrently
    let sites = vec!["KDMX", "KABR"];

    let mut handles = vec![];

    for site in sites {
        let handle = tokio::spawn(async move {
            let result = realtime::get_latest_volume(site).await;
            (site, result)
        });
        handles.push(handle);
    }

    for handle in handles {
        let (site, result) = handle.await.unwrap();
        assert!(result.is_ok(), "Failed to get latest volume for {}", site);
    }
}

#[test]
fn test_volume_index_boundaries() {
    // Test that VolumeIndex enforces proper boundaries
    let vol_min = VolumeIndex::new(1);
    let vol_max = VolumeIndex::new(999);

    assert_eq!(vol_min.as_number(), 1);
    assert_eq!(vol_max.as_number(), 999);

    // Test wraparound
    assert_eq!(vol_max.next().as_number(), 1);
}

#[tokio::test]
#[ignore = "requires AWS access and specific volume state"]
async fn test_download_intermediate_and_end_chunks() {
    // This test downloads multiple chunk types from a volume
    let site = "KDMX";

    let latest_result = realtime::get_latest_volume(site).await;
    assert!(latest_result.is_ok());
    let latest = latest_result.unwrap();
    assert!(latest.volume.is_some());

    let volume = latest.volume.unwrap();

    let chunks_result = realtime::list_chunks_in_volume(site, volume, 100).await;
    assert!(chunks_result.is_ok());
    let chunks = chunks_result.unwrap();

    if chunks.len() < 3 {
        // Need at least 3 chunks for this test
        return;
    }

    // Download a few chunks of different types
    for chunk_id in chunks.iter().take(3) {
        let download_result = realtime::download_chunk(site, chunk_id).await;
        assert!(
            download_result.is_ok(),
            "Failed to download chunk sequence {}",
            chunk_id.sequence()
        );

        let (returned_id, chunk) = download_result.unwrap();
        assert!(!chunk.data().is_empty());
        assert_eq!(returned_id.sequence(), chunk_id.sequence());
    }
}
