//! Utility to trim NEXRAD Archive II volume files into smaller test fixtures.
//!
//! This tool extracts a subset of elevation sweeps from a full volume file,
//! preserving metadata records (RDA Status, VCP, Clutter Filter Map) and
//! producing a valid Archive II file suitable for testing.
//!
//! # Usage
//!
//! ```bash
//! cargo run --example trim_fixture -- \
//!     --input downloads/KDMX20220305_232324_V06 \
//!     --output tests/fixtures/convective/KDMX20220305_232324.bin \
//!     --sweeps 1,2,3
//! ```

use bzip2::write::BzEncoder;
use bzip2::Compression;
use clap::Parser;
use nexrad_data::volume::{File as VolumeFile, Header};
use nexrad_decode::messages::{decode_messages, MessageContents};
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

#[derive(Parser)]
#[command(author, version, about = "Trim NEXRAD volume files into test fixtures")]
struct Cli {
    /// Input NEXRAD Archive II volume file
    #[arg(short, long)]
    input: PathBuf,

    /// Output trimmed fixture file
    #[arg(short, long)]
    output: PathBuf,

    /// Comma-separated list of sweep/elevation numbers to include (1-indexed)
    #[arg(short, long, value_delimiter = ',')]
    sweeps: Vec<u8>,

    /// Print statistics about the trimmed file
    #[arg(long, default_value = "true")]
    stats: bool,
}

/// Statistics about the trimming operation
struct TrimStats {
    input_size: usize,
    output_size: usize,
    input_records: usize,
    output_records: usize,
    input_messages: usize,
    output_messages: usize,
    sweeps_kept: HashSet<u8>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    if cli.sweeps.is_empty() {
        eprintln!("Error: Must specify at least one sweep number with --sweeps");
        std::process::exit(1);
    }

    let target_sweeps: HashSet<u8> = cli.sweeps.iter().copied().collect();
    println!(
        "Trimming {:?} to keep sweeps: {:?}",
        cli.input, target_sweeps
    );

    // Read input file
    let input_data = fs::read(&cli.input)?;
    let input_size = input_data.len();
    println!(
        "Input file size: {} bytes ({:.2} MB)",
        input_size,
        input_size as f64 / 1024.0 / 1024.0
    );

    // Parse the volume file (transparently decompresses gzip if needed)
    let volume = VolumeFile::new(input_data);

    // Extract header (24 bytes) - we'll copy this unchanged
    let header = volume.header().ok_or("Failed to parse volume header")?;
    println!("Volume header: {:?}", header.icao_of_radar());

    // Get header bytes from the (possibly decompressed) volume data
    let header_bytes = &volume.data()[..size_of::<Header>()];

    // Process records
    let records = volume.records()?;
    let input_records = records.len();
    println!("Processing {} records...", input_records);

    let mut output_records: Vec<Vec<u8>> = Vec::new();
    let mut total_input_messages = 0;
    let mut total_output_messages = 0;
    let mut sweeps_found: HashSet<u8> = HashSet::new();
    let mut kept_metadata = false;

    for (record_idx, record) in records.iter().enumerate() {
        // Decompress record if needed
        let decompressed = if record.compressed() {
            record.decompress()?
        } else {
            record.clone()
        };

        // Decode messages in this record
        let messages = decode_messages(decompressed.data())?;
        total_input_messages += messages.len();

        // Check what types of messages are in this record
        let mut has_metadata = false;
        let mut has_target_sweep = false;
        let mut record_sweeps: HashSet<u8> = HashSet::new();

        for message in &messages {
            match message.contents() {
                MessageContents::RDAStatusData(_)
                | MessageContents::VolumeCoveragePattern(_)
                | MessageContents::ClutterFilterMap(_) => {
                    has_metadata = true;
                }
                MessageContents::DigitalRadarData(drd) => {
                    let elev = drd.header().elevation_number();
                    record_sweeps.insert(elev);
                    if target_sweeps.contains(&elev) {
                        has_target_sweep = true;
                        sweeps_found.insert(elev);
                    }
                }
                MessageContents::DigitalRadarDataLegacy(drd) => {
                    let elev = drd.elevation_number() as u8;
                    record_sweeps.insert(elev);
                    if target_sweeps.contains(&elev) {
                        has_target_sweep = true;
                        sweeps_found.insert(elev);
                    }
                }
                _ => {}
            }
        }

        // Decide whether to keep this record
        let keep_record = if has_metadata && !kept_metadata {
            // Always keep the first metadata record
            kept_metadata = true;
            println!(
                "  Record {}: keeping (metadata: RDA Status, VCP, etc.)",
                record_idx + 1
            );
            true
        } else if has_target_sweep {
            println!(
                "  Record {}: keeping (contains sweeps {:?})",
                record_idx + 1,
                record_sweeps
                    .intersection(&target_sweeps)
                    .collect::<Vec<_>>()
            );
            true
        } else {
            false
        };

        if keep_record {
            // For now, we keep the entire record if it has any target sweeps.
            // A more aggressive trimming could filter individual messages.
            total_output_messages += messages.len();

            // Re-compress the record
            let compressed = compress_record(decompressed.data())?;
            output_records.push(compressed);
        }
    }

    println!("\nWriting output file...");

    // Build output file: header + compressed records
    let mut output_data = Vec::new();
    output_data.extend_from_slice(header_bytes);

    for record_data in &output_records {
        output_data.extend_from_slice(record_data);
    }

    // Create output directory if needed
    if let Some(parent) = cli.output.parent() {
        fs::create_dir_all(parent)?;
    }

    // Write output file
    fs::write(&cli.output, &output_data)?;

    let output_size = output_data.len();

    // Print statistics
    if cli.stats {
        let stats = TrimStats {
            input_size,
            output_size,
            input_records,
            output_records: output_records.len(),
            input_messages: total_input_messages,
            output_messages: total_output_messages,
            sweeps_kept: sweeps_found.clone(),
        };

        println!("\n=== Trimming Statistics ===");
        println!(
            "Input:  {} bytes ({:.2} MB), {} records, {} messages",
            stats.input_size,
            stats.input_size as f64 / 1024.0 / 1024.0,
            stats.input_records,
            stats.input_messages
        );
        println!(
            "Output: {} bytes ({:.2} KB), {} records, {} messages",
            stats.output_size,
            stats.output_size as f64 / 1024.0,
            stats.output_records,
            stats.output_messages
        );
        println!(
            "Reduction: {:.1}%",
            (1.0 - stats.output_size as f64 / stats.input_size as f64) * 100.0
        );
        println!("Sweeps kept: {:?}", stats.sweeps_kept);

        // Verify output is valid
        println!("\nVerifying output file...");
        let verify_data = fs::read(&cli.output)?;
        let verify_volume = VolumeFile::new(verify_data);

        if let Some(h) = verify_volume.header() {
            println!("  Header: OK (site: {:?})", h.icao_of_radar());
        } else {
            println!("  Header: FAILED");
        }

        match verify_volume.records() {
            Ok(records) => {
                println!("  Records: OK ({} records)", records.len());

                // Count messages and sweeps in output
                let mut verify_sweeps: HashSet<u8> = HashSet::new();
                let mut verify_messages = 0;
                for record in records {
                    let decompressed = if record.compressed() {
                        record.decompress()?
                    } else {
                        record
                    };
                    let messages = decode_messages(decompressed.data())?;
                    verify_messages += messages.len();
                    for msg in messages {
                        match msg.contents() {
                            MessageContents::DigitalRadarData(drd) => {
                                verify_sweeps.insert(drd.header().elevation_number());
                            }
                            MessageContents::DigitalRadarDataLegacy(drd) => {
                                verify_sweeps.insert(drd.elevation_number() as u8);
                            }
                            _ => {}
                        }
                    }
                }
                println!("  Messages: {} total", verify_messages);
                println!("  Sweeps in output: {:?}", verify_sweeps);
            }
            Err(e) => {
                println!("  Records: FAILED ({:?})", e);
            }
        }
    }

    println!("\nDone! Output written to {:?}", cli.output);
    Ok(())
}

/// Compress data into an LDM record format (4-byte size prefix + bzip2 data)
fn compress_record(data: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    // Compress with bzip2
    let mut encoder = BzEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(data)?;
    let compressed = encoder.finish()?;

    // Build record: 4-byte big-endian size (negative) + compressed data
    let record_size = compressed.len() as i32;
    let mut record = Vec::with_capacity(4 + compressed.len());
    record.extend_from_slice(&(-record_size).to_be_bytes()); // Negative for LDM format
    record.extend_from_slice(&compressed);

    Ok(record)
}
