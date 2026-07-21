//! Pull-based chunk iterator for real-time NEXRAD data.
//!
//! This module provides a pull-based iterator that allows callers to control
//! timing externally, making it suitable for environments without tokio or
//! where manual timing control is preferred.

use crate::aws::realtime::{
    download_chunk, estimate_chunk_availability_time, get_latest_volume, list_chunks_in_volume,
    project_scan_timing, Chunk, ChunkIdentifier, ChunkMetadata, ChunkTimingStats, ChunkType,
    ElevationChunkMapper, NextChunk, RetryPolicy, RetryState, ScanTimingProjection, VolumeIndex,
};
use crate::result::{aws::AWSError, Error, Result};
use chrono::{DateTime, Duration, Utc};
use log::debug;
use nexrad_decode::messages::volume_coverage_pattern;

/// A downloaded chunk with metadata about the download process.
#[derive(Debug)]
pub struct DownloadedChunk {
    /// The chunk identifier.
    pub identifier: ChunkIdentifier,
    /// The chunk data.
    pub chunk: Chunk<'static>,
    /// Number of attempts required to download this chunk.
    pub attempts: usize,
}

/// Result of initializing a [`ChunkIterator`].
///
/// When creating a new iterator, the latest chunk in the volume is fetched and returned
/// in `latest_chunk`. If joining mid-volume (the latest chunk is not a Start chunk),
/// the Start chunk is also fetched to extract the VCP and is returned in `start_chunk`.
#[derive(Debug)]
pub struct ChunkIteratorInit {
    /// The initialized iterator, ready for subsequent `try_next()` calls.
    pub iterator: ChunkIterator,
    /// The latest chunk in the volume (the chunk the iterator joined at).
    pub latest_chunk: DownloadedChunk,
    /// The Start chunk, if it was fetched separately from the latest chunk.
    /// This is `Some` when joining mid-volume and contains the VCP metadata.
    /// When the latest chunk IS the Start chunk, this is `None`.
    pub start_chunk: Option<DownloadedChunk>,
}

/// Iterator state for tracking what to fetch next.
#[derive(Debug, Clone, PartialEq, Eq)]
enum IteratorState {
    /// Need to fetch the start chunk for a new volume.
    NeedVolumeStart(VolumeIndex),
    /// Ready to fetch the next chunk in sequence.
    Ready(ChunkIdentifier),
}

/// Pull-based iterator for real-time NEXRAD chunks.
///
/// This iterator allows manual control over timing, making it suitable for
/// environments without tokio or where caller-controlled scheduling is preferred.
/// Instead of blocking, callers can use [`next_expected_time`](Self::next_expected_time)
/// to determine when to call [`try_next`](Self::try_next).
#[derive(Debug)]
pub struct ChunkIterator {
    site: String,
    state: IteratorState,
    elevation_mapper: Option<ElevationChunkMapper>,
    vcp: Option<volume_coverage_pattern::Message<'static>>,
    timing_stats: ChunkTimingStats,
    download_policy: RetryPolicy,
    discovery_policy: RetryPolicy,
    last_chunk_time: Option<DateTime<Utc>>,
    requests_made: usize,
    bytes_downloaded: u64,
}

impl ChunkIterator {
    /// Starts a new chunk iterator at the latest available volume.
    ///
    /// This will make network requests to discover the latest volume and download
    /// the most recent chunk. The returned [`ChunkIteratorInit`] contains both the
    /// iterator and the initial chunk(s), including the Start chunk if joining mid-volume.
    ///
    /// # Returns
    ///
    /// A [`ChunkIteratorInit`] containing:
    /// - `iterator`: The initialized iterator ready for `try_next()` calls
    /// - `latest_chunk`: The most recent chunk in the volume
    /// - `start_chunk`: The Start chunk if it differs from `latest_chunk` (contains VCP metadata)
    pub async fn start(site: &str) -> Result<ChunkIteratorInit> {
        Self::start_with_policies(
            site,
            RetryPolicy::default_download(),
            RetryPolicy::default_discovery(),
        )
        .await
    }

    /// Starts a new chunk iterator with custom retry policies.
    ///
    /// See [`start`](Self::start) for details on the return value.
    pub async fn start_with_policies(
        site: &str,
        download_policy: RetryPolicy,
        discovery_policy: RetryPolicy,
    ) -> Result<ChunkIteratorInit> {
        let latest_volume_result = get_latest_volume(site).await?;
        let volume = latest_volume_result
            .volume
            .ok_or(AWSError::LatestVolumeNotFound)?;

        debug!(
            "ChunkIterator initialized for site {} at volume {}",
            site,
            volume.as_number()
        );

        let mut iterator = Self {
            site: site.to_string(),
            state: IteratorState::NeedVolumeStart(volume),
            elevation_mapper: None,
            vcp: None,
            timing_stats: ChunkTimingStats::new(),
            download_policy,
            discovery_policy,
            last_chunk_time: None,
            requests_made: latest_volume_result.calls,
            bytes_downloaded: 0,
        };

        // Fetch the initial chunk(s)
        let (latest_chunk, start_chunk) = iterator.fetch_initial_chunks(volume).await?;

        Ok(ChunkIteratorInit {
            iterator,
            latest_chunk,
            start_chunk,
        })
    }

    /// Creates a new chunk iterator starting at a specific chunk.
    ///
    /// This is useful for resuming from a known position. Note that the VCP and
    /// elevation mapper will not be available until a Start chunk is encountered.
    pub fn from_chunk(
        site: &str,
        chunk_id: ChunkIdentifier,
        download_policy: RetryPolicy,
        discovery_policy: RetryPolicy,
    ) -> Self {
        Self {
            site: site.to_string(),
            state: IteratorState::Ready(chunk_id),
            elevation_mapper: None,
            vcp: None,
            timing_stats: ChunkTimingStats::new(),
            download_policy,
            discovery_policy,
            last_chunk_time: None,
            requests_made: 0,
            bytes_downloaded: 0,
        }
    }

    /// Fetches the initial chunks during iterator construction.
    ///
    /// Returns the latest chunk and optionally the Start chunk (if they differ).
    async fn fetch_initial_chunks(
        &mut self,
        volume: VolumeIndex,
    ) -> Result<(DownloadedChunk, Option<DownloadedChunk>)> {
        // Fetch the latest chunk in the volume
        let latest_chunk = self
            .fetch_latest_chunk_in_volume(volume)
            .await?
            .ok_or(Error::AWS(AWSError::ExpectedChunkNotFound))?;

        let mut start_chunk = None;

        // If the latest chunk is a Start chunk, extract VCP from it
        if latest_chunk.identifier.chunk_type() == ChunkType::Start {
            if let Ok(vcp) = Self::extract_vcp(&latest_chunk.chunk) {
                self.elevation_mapper = Some(ElevationChunkMapper::new(&vcp));
                self.vcp = Some(vcp);
            }
        } else {
            // Joined mid-volume: fetch the Start chunk for VCP
            let start_id = ChunkIdentifier::new(
                self.site.clone(),
                volume,
                *latest_chunk.identifier.date_time_prefix(),
                1,
                ChunkType::Start,
                None,
            );

            if let Ok((identifier, chunk)) = download_chunk(&self.site, &start_id).await {
                self.requests_made += 1;
                self.bytes_downloaded += chunk.data().len() as u64;

                if let Ok(vcp) = Self::extract_vcp(&chunk) {
                    self.elevation_mapper = Some(ElevationChunkMapper::new(&vcp));
                    self.vcp = Some(vcp);
                }

                start_chunk = Some(DownloadedChunk {
                    identifier,
                    chunk,
                    attempts: 1,
                });
            }
        }

        self.last_chunk_time = latest_chunk.identifier.upload_date_time();
        self.state = IteratorState::Ready(latest_chunk.identifier.clone());

        Ok((latest_chunk, start_chunk))
    }

    /// Returns the estimated time when the next chunk will be available.
    ///
    /// Returns `None` if timing cannot be estimated (e.g., VCP not yet known).
    /// The caller can use this to schedule when to call [`try_next`](Self::try_next).
    pub fn next_expected_time(&self) -> Option<DateTime<Utc>> {
        let chunk_id = match &self.state {
            IteratorState::NeedVolumeStart(_) => return None,
            IteratorState::Ready(id) => id,
        };

        let vcp = self.vcp.as_ref()?;
        let mapper = self.elevation_mapper.as_ref()?;

        estimate_chunk_availability_time(chunk_id, vcp, mapper, Some(&self.timing_stats))
    }

    /// Returns the estimated duration until the next chunk is available.
    ///
    /// Returns `None` if timing cannot be estimated or if the chunk should
    /// already be available.
    pub fn time_until_next(&self) -> Option<Duration> {
        let expected = self.next_expected_time()?;
        let now = Utc::now();

        if expected <= now {
            None
        } else {
            Some(expected - now)
        }
    }

    /// Attempts to fetch the next chunk.
    ///
    /// Returns:
    /// - `Ok(Some(chunk))` if a chunk was successfully downloaded
    /// - `Ok(None)` if the chunk is not yet available (caller should wait and retry)
    /// - `Err(...)` if an unrecoverable error occurred
    ///
    /// This method uses the configured retry policy for transient failures.
    pub async fn try_next(&mut self) -> Result<Option<DownloadedChunk>> {
        match &self.state {
            IteratorState::NeedVolumeStart(volume) => self.try_fetch_volume_start(*volume).await,
            IteratorState::Ready(current) => {
                let next = current
                    .next_chunk(
                        self.elevation_mapper
                            .as_ref()
                            .ok_or(AWSError::FailedToDetermineNextChunk)?,
                    )
                    .ok_or(AWSError::FailedToDetermineNextChunk)?;

                match next {
                    NextChunk::Sequence(next_id) => self.try_fetch_chunk(next_id).await,
                    NextChunk::Volume(next_volume) => {
                        self.try_fetch_volume_start(next_volume).await
                    }
                }
            }
        }
    }

    /// Attempts to fetch the start chunk of a new volume.
    async fn try_fetch_volume_start(
        &mut self,
        volume: VolumeIndex,
    ) -> Result<Option<DownloadedChunk>> {
        let mut retry_state = RetryState::new(self.discovery_policy.clone());

        while retry_state.should_retry() {
            match self.fetch_latest_chunk_in_volume(volume).await {
                Ok(Some(downloaded)) => {
                    // Update VCP and elevation mapper from start chunk
                    if downloaded.identifier.chunk_type() == ChunkType::Start {
                        if let Ok(vcp) = Self::extract_vcp(&downloaded.chunk) {
                            self.elevation_mapper = Some(ElevationChunkMapper::new(&vcp));
                            self.vcp = Some(vcp);
                        }
                    }

                    // If we joined mid-volume (not a start chunk), fetch the start chunk
                    // to get the VCP needed for elevation mapping
                    if downloaded.identifier.chunk_type() != ChunkType::Start
                        && self.elevation_mapper.is_none()
                    {
                        let start_id = ChunkIdentifier::new(
                            self.site.clone(),
                            volume,
                            *downloaded.identifier.date_time_prefix(),
                            1,
                            ChunkType::Start,
                            None,
                        );
                        if let Ok((_, start_chunk)) = download_chunk(&self.site, &start_id).await {
                            self.requests_made += 1;
                            self.bytes_downloaded += start_chunk.data().len() as u64;

                            if let Ok(vcp) = Self::extract_vcp(&start_chunk) {
                                self.elevation_mapper = Some(ElevationChunkMapper::new(&vcp));
                                self.vcp = Some(vcp);
                            }
                        }
                    }

                    // Update timing stats if we have previous chunk time
                    if let (Some(upload_time), Some(prev_time)) = (
                        downloaded.identifier.upload_date_time(),
                        self.last_chunk_time,
                    ) {
                        let duration = upload_time - prev_time;
                        self.update_timing_stats(
                            &downloaded.identifier,
                            duration,
                            downloaded.attempts,
                        );
                    }

                    self.last_chunk_time = downloaded.identifier.upload_date_time();
                    self.state = IteratorState::Ready(downloaded.identifier.clone());

                    return Ok(Some(downloaded));
                }
                Ok(None) => {
                    // Volume has no chunks yet, will retry
                }
                Err(e) => {
                    debug!("Error fetching volume start: {:?}", e);
                }
            }

            if let Some(_delay) = retry_state.next_delay() {
                // Return None to let the caller control timing.
                // The caller should check time_until_next() and schedule accordingly.
                return Ok(None);
            }
        }

        // Exhausted retries
        Err(Error::AWS(AWSError::ExpectedChunkNotFound))
    }

    /// Fetches the latest chunk in a volume.
    async fn fetch_latest_chunk_in_volume(
        &mut self,
        volume: VolumeIndex,
    ) -> Result<Option<DownloadedChunk>> {
        let chunks = list_chunks_in_volume(&self.site, volume, 100).await?;
        self.requests_made += 1;

        let latest = match chunks.last() {
            Some(id) => id,
            None => return Ok(None),
        };

        let (identifier, chunk) = download_chunk(&self.site, latest).await?;
        self.requests_made += 1;
        self.bytes_downloaded += chunk.data().len() as u64;

        Ok(Some(DownloadedChunk {
            identifier,
            chunk,
            attempts: 1,
        }))
    }

    /// Attempts to fetch a specific chunk.
    /// Attempts to fetch a specific chunk.
    ///
    /// This is a single-attempt fetch for pull-based iteration. Returns:
    /// - `Ok(Some(chunk))` if successfully downloaded
    /// - `Ok(None)` if chunk is not yet available (caller should wait and retry)
    /// - `Err(...)` for unrecoverable errors
    async fn try_fetch_chunk(
        &mut self,
        chunk_id: ChunkIdentifier,
    ) -> Result<Option<DownloadedChunk>> {
        self.requests_made += 1;

        match download_chunk(&self.site, &chunk_id).await {
            Ok((identifier, chunk)) => {
                self.bytes_downloaded += chunk.data().len() as u64;

                // Update VCP if this is a start chunk
                if identifier.chunk_type() == ChunkType::Start {
                    if let Ok(vcp) = Self::extract_vcp(&chunk) {
                        self.elevation_mapper = Some(ElevationChunkMapper::new(&vcp));
                        self.vcp = Some(vcp);
                    }
                }

                // Update timing stats
                if let (Some(upload_time), Some(prev_time)) =
                    (identifier.upload_date_time(), self.last_chunk_time)
                {
                    let duration = upload_time - prev_time;
                    self.update_timing_stats(&identifier, duration, 1);
                }

                self.last_chunk_time = identifier.upload_date_time();
                self.state = IteratorState::Ready(identifier.clone());

                Ok(Some(DownloadedChunk {
                    identifier,
                    chunk,
                    attempts: 1,
                }))
            }
            Err(Error::AWS(AWSError::S3ObjectNotFound)) => {
                // Chunk not yet available
                debug!("Chunk {} not yet available", chunk_id.name());
                Ok(None)
            }
            Err(e) => {
                debug!("Error downloading chunk: {:?}", e);
                Err(e)
            }
        }
    }

    /// Extracts VCP from a start chunk.
    fn extract_vcp(chunk: &Chunk) -> Result<volume_coverage_pattern::Message<'static>> {
        if let Chunk::Start(file) = chunk {
            for mut record in file.records()? {
                if record.compressed() {
                    record = record.decompress()?;
                }

                for message in record.messages()? {
                    if let nexrad_decode::messages::MessageContents::VolumeCoveragePattern(vcp) =
                        message.contents()
                    {
                        return Ok(vcp.clone().into_owned());
                    }
                }
            }
        }

        Err(Error::MissingCoveragePattern)
    }

    /// Updates timing statistics for a chunk.
    fn update_timing_stats(
        &mut self,
        chunk_id: &ChunkIdentifier,
        duration: Duration,
        attempts: usize,
    ) {
        if let (Some(vcp), Some(mapper)) = (&self.vcp, &self.elevation_mapper) {
            if let Some(elevation) = mapper
                .get_sequence_elevation_number(chunk_id.sequence())
                .and_then(|n| vcp.elevations().get(n - 1))
            {
                use crate::aws::realtime::ChunkCharacteristics;

                let characteristics = ChunkCharacteristics {
                    chunk_type: chunk_id.chunk_type(),
                    waveform_type: elevation.waveform_type(),
                    channel_configuration: elevation.channel_configuration(),
                };

                self.timing_stats
                    .add_timing(characteristics, duration, attempts);
            }
        }
    }

    /// Returns the current chunk identifier, if available.
    pub fn current(&self) -> Option<&ChunkIdentifier> {
        match &self.state {
            IteratorState::Ready(id) => Some(id),
            IteratorState::NeedVolumeStart(_) => None,
        }
    }

    /// Returns a reference to the timing statistics.
    pub fn timing_stats(&self) -> &ChunkTimingStats {
        &self.timing_stats
    }

    /// Returns a mutable reference to the timing statistics.
    pub fn timing_stats_mut(&mut self) -> &mut ChunkTimingStats {
        &mut self.timing_stats
    }

    /// Returns the current VCP if available.
    pub fn vcp(&self) -> Option<&volume_coverage_pattern::Message<'static>> {
        self.vcp.as_ref()
    }

    /// Returns the current elevation chunk mapper if available.
    pub fn elevation_mapper(&self) -> Option<&ElevationChunkMapper> {
        self.elevation_mapper.as_ref()
    }

    /// Returns the site identifier.
    pub fn site(&self) -> &str {
        &self.site
    }

    /// Returns the download retry policy.
    pub fn download_policy(&self) -> &RetryPolicy {
        &self.download_policy
    }

    /// Returns the discovery retry policy.
    pub fn discovery_policy(&self) -> &RetryPolicy {
        &self.discovery_policy
    }

    /// Returns the total number of HTTP requests made by this iterator.
    ///
    /// This includes requests made during initialization and all subsequent
    /// `try_next()` calls. Useful for displaying network statistics in UIs.
    pub fn requests_made(&self) -> usize {
        self.requests_made
    }

    /// Returns the total bytes downloaded by this iterator.
    ///
    /// This includes chunk data downloaded during initialization and all
    /// subsequent `try_next()` calls.
    pub fn bytes_downloaded(&self) -> u64 {
        self.bytes_downloaded
    }

    /// Projects timing for all remaining chunks in the current volume scan.
    ///
    /// Returns `None` if the VCP or elevation mapper is not yet available, or if
    /// there are no remaining chunks to project.
    pub fn project_remaining_scan(&self) -> Option<ScanTimingProjection> {
        let chunk_id = match &self.state {
            IteratorState::Ready(id) => id,
            IteratorState::NeedVolumeStart(_) => return None,
        };

        let vcp = self.vcp.as_ref()?;
        let mapper = self.elevation_mapper.as_ref()?;

        project_scan_timing(chunk_id, vcp, mapper, Some(&self.timing_stats))
    }

    /// Returns the projected time when the current volume will complete.
    ///
    /// Returns `None` if a projection cannot be made.
    pub fn projected_volume_end_time(&self) -> Option<DateTime<Utc>> {
        self.project_remaining_scan().map(|p| p.volume_end_time())
    }

    /// Returns metadata for a specific chunk sequence number in the current volume.
    ///
    /// Returns `None` if the elevation mapper is not available or the sequence is out of range.
    pub fn chunk_metadata(&self, sequence: usize) -> Option<&ChunkMetadata> {
        self.elevation_mapper
            .as_ref()
            .and_then(|m| m.get_chunk_metadata(sequence))
    }

    /// Returns metadata for all chunks in the current volume.
    ///
    /// Returns `None` if the elevation mapper is not yet available.
    pub fn all_chunk_metadata(&self) -> Option<&[ChunkMetadata]> {
        self.elevation_mapper
            .as_ref()
            .map(|m| m.all_chunk_metadata())
    }
}
