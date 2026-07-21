# NEXRAD Data

[![Crates.io](https://img.shields.io/crates/v/nexrad-data)](https://crates.io/crates/nexrad-data)
[![Docs.rs](https://docs.rs/nexrad-data/badge.svg)](https://docs.rs/nexrad-data)
[![Rust CI](https://github.com/danielway/nexrad/actions/workflows/ci.yml/badge.svg)](https://github.com/danielway/nexrad/actions/workflows/ci.yml)

Provides structure definitions and decoding functions for NEXRAD Archive II volume files, along with functions for 
downloading both archival and real-time data from open cloud providers like AWS OpenData.

## Volume Definitions

The `nexrad-data::volume` module provides model definitions for the NEXRAD Archive II volume file format described in
the Radar Operations Center's ICD 2620010J for the Archive II/User (Build 23.0, June 25, 2024). A `volume::File`
can be constructed with archive or real-time data. It can decode the archive volume header and provide access to LDM
`volume::Record`s which can be decompressed and decoded into a series of messages.

## AWS Open Data

NOAA uploads archive and real-time NEXRAD data to AWS Open Data S3 buckets which are publicly available. The
`nexrad-data::aws` module provides functions for listing and downloading NEXRAD data from these buckets.

### Archive Data

Historical volumes are archived by date and radar site in the `noaa-nexrad-level2` bucket. The
`nexrad-data::aws::archive` module provides functions for accessing these volumes. The `archive::list_files` function
queries volumes for a given date and radar site, returning identifiers for each volume. The `archive::download_file`
function downloads a volume file by its identifier.

### Real-Time Data

Real-time volume data is uploaded in chunks to the `unidata-nexrad-level2-chunks` bucket. 999 volume directories are
rotated through with chunks being added to each directory until they comprise a full volume. The
`nexrad-data::aws::realtime` module provides functions for accessing these chunks. The `realtime::list_chunks_in_volume`
function queries a volume for its chunks, returning identifiers for each chunk. The `realtime::download_chunk` function
downloads a chunk by its identifier. The `realtime::get_latest_volume` function can be used to identify which of the 999
volume directories contain the latest data, and the `realtime::estimate_next_chunk_time` function can be used to
estimate when the next chunk will be uploaded. The `realtime::chunk_stream` function returns an async stream that
continuously yields chunks as they become available. For environments without tokio, `realtime::ChunkIterator` provides
a pull-based interface with manual timing control.

## Features

The APIs in this crate should be configured to require only the dependencies they need, leaving the consumer to include
as much or little as they desire. By default, all features are included. The following named features are helpful
groupings of dependencies/behavior:

- `decode` - Enables both decoding of the volume headers and of decoding the LDM records' NEXRAD messages using `nexrad-decode`.
- `aws` - Enables accessing archive and real-time NEXRAD data from AWS Open Data.
- `nexrad-model` - Provides mappings to a common radar data model, particularly for mapping `volume::File` into a `Scan`.
- `parallel` - Parallelize LDM record decompression and decoding using Rayon.
