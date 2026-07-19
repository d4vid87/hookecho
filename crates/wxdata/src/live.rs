//! Live Level 2 chunk streaming.
//!
//! NEXRAD publishes a volume as a sequence of small "chunks" to an S3 bucket during the scan
//! itself, so a display can update sweep-by-sweep instead of waiting ~5 min for the archived
//! volume. [`stream`] drives `nexrad-data`'s pull-based [`ChunkIterator`], assembles the
//! accumulated chunks into a [`Scan`] at every sweep boundary, merges it into the running
//! volume, and hands the caller a full updated [`Scan`] via `on_update`.
//!
//! All merged state lives on this task; the UI thread only ever receives a finished `Scan`.

use crate::level2::{elevation_angles, Scan};
use nexrad_data::aws::realtime::{
    assemble_volume, download_chunk, Chunk, ChunkIdentifier, ChunkIterator, ChunkType,
};
use nexrad_model::data::Sweep;
use std::time::Duration;

/// A merged live volume ready to display.
pub struct Update {
    /// A synthetic name identifying this update (volume prefix + sequence).
    pub name: String,
    pub time: chrono::DateTime<chrono::Utc>,
    pub scan: Scan,
    /// Elevation angles (deg) whose sweeps changed vs. the previous update — the app uses
    /// this to evict only the affected tilts from its binned-sweep cache.
    pub changed: Vec<f32>,
}

/// Stream live chunks for `site`, starting from `base` (the last polled volume), calling
/// `on_update` with a full merged [`Scan`] at each sweep boundary.
///
/// Returns `Ok(())` only if the iterator ends cleanly (it normally runs until aborted);
/// any error returns so the caller can fall back to interval polling.
pub async fn stream<F>(site: String, base: Scan, mut on_update: F) -> anyhow::Result<()>
where
    F: FnMut(Update),
{
    let init = ChunkIterator::start(&site)
        .await
        .map_err(|e| anyhow::anyhow!("chunk iterator start: {e}"))?;
    let mut it = init.iterator;

    // Assemble the current volume: start chunk + backfilled middle chunks + the joined chunk.
    let mut chunks: Vec<Chunk<'static>> = Vec::new();
    if let Some(sc) = init.start_chunk {
        chunks.push(sc.chunk);
    }
    let joined = &init.latest_chunk.identifier;
    let latest_seq = joined.sequence();
    let volume = *joined.volume();
    let prefix = *joined.date_time_prefix();
    // ponytail: sequential O(n) backfill downloads on start so the first frame is a full
    // volume; gaps are tolerated (a missing chunk just omits its radials).
    for seq in 2..latest_seq {
        let id = ChunkIdentifier::new(site.clone(), volume, prefix, seq, ChunkType::Intermediate, None);
        if let Ok((_, ch)) = download_chunk(&site, &id).await {
            chunks.push(ch);
        }
    }
    chunks.push(init.latest_chunk.chunk);

    let mut merged = base;
    emit(&it, &chunks, &mut merged, &mut on_update);

    loop {
        let wait = it
            .time_until_next()
            .and_then(|d| d.to_std().ok())
            .unwrap_or(Duration::from_secs(2))
            .clamp(Duration::from_secs(1), Duration::from_secs(15));
        tokio::time::sleep(wait).await;

        match it.try_next().await {
            Ok(Some(dc)) => {
                let seq = dc.identifier.sequence();
                let ctype = dc.identifier.chunk_type();
                if ctype == ChunkType::Start {
                    chunks.clear(); // volume rollover: start a fresh accumulator
                }
                chunks.push(dc.chunk);
                let sweep_done = it.chunk_metadata(seq).is_some_and(|m| m.is_last_in_sweep());
                if sweep_done || ctype == ChunkType::End {
                    emit(&it, &chunks, &mut merged, &mut on_update);
                }
            }
            Ok(None) => { /* not available yet; loop and wait again */ }
            Err(e) => return Err(anyhow::anyhow!("chunk stream: {e}")),
        }
    }
}

/// Assemble `chunks`, merge into `merged`, and emit if anything changed. Assembly failure
/// (e.g. a still-incomplete volume) is skipped; the next sweep boundary self-heals.
fn emit<F: FnMut(Update)>(it: &ChunkIterator, chunks: &[Chunk<'static>], merged: &mut Scan, on_update: &mut F) {
    let partial = match assemble_volume(chunks.iter().cloned()) {
        Ok(s) => s,
        Err(e) => {
            log::debug!("assemble skipped: {e}");
            return;
        }
    };
    let base = std::mem::replace(merged, empty_like(&partial));
    let (new_scan, changed) = merge_scan(base, partial);
    *merged = new_scan;
    if changed.is_empty() {
        return; // nothing new since the last emit
    }
    let (name, time) = it
        .current()
        .map(|id| (id.name().to_string(), id.upload_date_time().unwrap_or_else(chrono::Utc::now)))
        .unwrap_or_else(|| (String::from("live"), chrono::Utc::now()));
    // ponytail: one Scan clone per emit for the UI; the merge/decode cost stays on this task.
    on_update(Update { name, time, scan: merged.clone(), changed });
}

/// A cheap placeholder scan (same VCP, no sweeps) used only to move `merged` out during emit.
fn empty_like(like: &Scan) -> Scan {
    match like.site() {
        Some(s) => Scan::with_site(s.clone(), like.coverage_pattern().clone(), Vec::new()),
        None => Scan::new(like.coverage_pattern().clone(), Vec::new()),
    }
}

/// Merge `partial` into `base`, newest-wins by elevation number.
///
/// A VCP change replaces the volume wholesale (tilt set changed). Otherwise each partial
/// sweep replaces the base sweep with the same elevation number only when it actually differs
/// (`Sweep: PartialEq`), keeping split cuts and the tilt list stable mid-stream. Returns the
/// merged scan and the angles of the sweeps that changed.
pub fn merge_scan(base: Scan, partial: Scan) -> (Scan, Vec<f32>) {
    if base.coverage_pattern_number() != partial.coverage_pattern_number() {
        let changed = elevation_angles(&partial);
        return (partial, changed);
    }

    let mut sweeps: Vec<Sweep> = base.sweeps().to_vec();
    let mut changed_nums: Vec<u8> = Vec::new();
    for ps in partial.sweeps() {
        let en = ps.elevation_number();
        match sweeps.iter().position(|s| s.elevation_number() == en) {
            Some(i) => {
                if &sweeps[i] != ps {
                    sweeps[i] = ps.clone();
                    changed_nums.push(en);
                }
            }
            None => {
                sweeps.push(ps.clone());
                changed_nums.push(en);
            }
        }
    }

    let vcp = base.coverage_pattern().clone();
    let scan = match base.site() {
        Some(s) => Scan::with_site(s.clone(), vcp, sweeps),
        None => Scan::new(vcp, sweeps),
    };
    let changed = changed_nums
        .iter()
        .filter_map(|en| {
            scan.sweeps()
                .iter()
                .find(|s| s.elevation_number() == *en)
                .and_then(|s| s.elevation_angle_degrees())
        })
        .collect();
    (scan, changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexrad_model::data::{MomentData, PulseWidth, Radial, RadialStatus, VolumeCoveragePattern};

    fn vcp(n: u16) -> VolumeCoveragePattern {
        VolumeCoveragePattern::new(n, 1, 0.5, PulseWidth::Short, false, 0, false, 0, false, false, 0, false, false, Vec::new())
    }

    // A sweep at the given elevation number/angle carrying a single reflectivity value.
    fn sweep(elevation_number: u8, angle: f32, refl_raw: u8) -> Sweep {
        let data = MomentData::from_fixed_point(1, 2125, 250, 8, 2.0, 66.0, vec![refl_raw]);
        let radial = Radial::new(
            0, 90, 90.0, 0.5, RadialStatus::ScanStart, elevation_number, angle, Some(data),
            None, None, None, None, None, None,
        );
        Sweep::new(elevation_number, vec![radial])
    }

    #[test]
    fn merge_replaces_changed_sweep_keeps_others() {
        let base = Scan::new(vcp(212), vec![sweep(1, 0.5, 100), sweep(2, 1.5, 100)]);
        // Partial re-sends tilt 1 unchanged and tilt 2 with new data.
        let partial = Scan::new(vcp(212), vec![sweep(1, 0.5, 100), sweep(2, 1.5, 150)]);
        let (merged, changed) = merge_scan(base, partial);
        assert_eq!(merged.sweeps().len(), 2);
        // Only tilt 2 (~1.5deg) changed.
        assert_eq!(changed.len(), 1);
        assert!((changed[0] - 1.5).abs() < 0.01, "changed angle {:?}", changed);
    }

    #[test]
    fn merge_appends_new_tilt() {
        let base = Scan::new(vcp(212), vec![sweep(1, 0.5, 100)]);
        let partial = Scan::new(vcp(212), vec![sweep(3, 2.4, 120)]);
        let (merged, changed) = merge_scan(base, partial);
        assert_eq!(merged.sweeps().len(), 2, "new tilt appended");
        assert_eq!(changed.len(), 1);
    }

    #[test]
    fn vcp_change_replaces_wholesale() {
        let base = Scan::new(vcp(212), vec![sweep(1, 0.5, 100), sweep(2, 1.5, 100)]);
        let partial = Scan::new(vcp(35), vec![sweep(1, 0.5, 100)]);
        let (merged, _) = merge_scan(base, partial);
        assert_eq!(merged.coverage_pattern_number(), vcp(35).pattern_number());
        assert_eq!(merged.sweeps().len(), 1, "wholesale replace");
    }
}
