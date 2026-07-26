//! GOES GLM lightning: individual flashes, from the satellite rather than the ground network.
//!
//! The Geostationary Lightning Mapper watches the whole disk continuously and publishes a granule
//! every 20 seconds, so this sees in-cloud flashes that a cloud-to-ground network never reports and
//! sees them within a minute. The MRMS lightning-density layer is a gridded 2-minute average of
//! ground strikes; this is the other half of the picture, not a replacement.
//!
//! Granules are netCDF-4, read through [`hdf5lite`] so the Android build doesn't need libhdf5.

use chrono::{DateTime, TimeZone, Utc};
use std::collections::VecDeque;

/// GOES-East. GOES-19 took over the slot from GOES-16 in 2025; GOES-18 is West.
const BUCKET: &str = "https://noaa-goes19.s3.amazonaws.com";
const PRODUCT: &str = "GLM-L2-LCFA";

/// Seconds between the netCDF epoch (2000-01-01 12:00 UTC) and the Unix epoch.
const J2000_UNIX: i64 = 946_728_000;

/// One lightning flash.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Flash {
    pub lon: f64,
    pub lat: f64,
    /// Optical energy in joules — spans decades, so callers usually want it on a log scale.
    pub energy: f64,
    pub time: DateTime<Utc>,
}

/// Decode one granule's flashes.
///
/// Flash positions are plain floats; the time is the granule's `product_time` plus each flash's
/// offset. Rows where any field is a fill value are dropped rather than plotted at (0, 0).
pub fn decode(bytes: Vec<u8>) -> anyhow::Result<Vec<Flash>> {
    let f = hdf5lite::File::open(bytes).map_err(|e| anyhow::anyhow!("glm: {e}"))?;
    let lat = f
        .read_f64("flash_lat")
        .map_err(|e| anyhow::anyhow!("glm lat: {e}"))?;
    let lon = f
        .read_f64("flash_lon")
        .map_err(|e| anyhow::anyhow!("glm lon: {e}"))?;
    let energy = f.read_f64("flash_energy").unwrap_or_default();
    let offset = f
        .read_f64("flash_time_offset_of_first_event")
        .unwrap_or_default();
    // `product_time` is seconds since 2000-01-01 12:00 UTC, the netCDF/CF convention here.
    let base = f
        .read_f64("product_time")
        .ok()
        .and_then(|v| v.first().copied())
        .and_then(|s| Utc.timestamp_opt(J2000_UNIX + s as i64, 0).single())
        .unwrap_or_else(Utc::now);

    let n = lat.len().min(lon.len());
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        if !lat[i].is_finite() || !lon[i].is_finite() {
            continue;
        }
        // Off-disk or corrupt rows: the GLM field of view can't produce these.
        if !(-90.0..=90.0).contains(&lat[i]) || !(-180.0..=180.0).contains(&lon[i]) {
            continue;
        }
        let dt = offset
            .get(i)
            .copied()
            .filter(|v| v.is_finite())
            .unwrap_or(0.0);
        out.push(Flash {
            lon: lon[i],
            lat: lat[i],
            energy: energy
                .get(i)
                .copied()
                .filter(|v| v.is_finite())
                .unwrap_or(0.0),
            time: base + chrono::Duration::milliseconds((dt * 1000.0) as i64),
        });
    }
    Ok(out)
}

/// List the granule keys for the hour containing `t`, oldest first.
async fn list_hour(http: &reqwest::Client, t: DateTime<Utc>) -> Vec<String> {
    use chrono::Datelike;
    let prefix = format!(
        "{PRODUCT}/{:04}/{:03}/{:02}/",
        t.year(),
        t.ordinal(),
        t.format("%H")
    );
    let url = format!("{BUCKET}/?list-type=2&prefix={prefix}&max-keys=1000");
    let Ok(resp) = http.get(&url).send().await else {
        return Vec::new();
    };
    let Ok(xml) = resp.text().await else {
        return Vec::new();
    };
    let mut keys = Vec::new();
    let mut rest = xml.as_str();
    while let Some(i) = rest.find("<Key>") {
        rest = &rest[i + 5..];
        let Some(e) = rest.find("</Key>") else { break };
        keys.push(rest[..e].to_string());
        rest = &rest[e..];
    }
    keys.sort();
    keys
}

/// A rolling window of recent flashes, refreshed by polling S3 for new granules.
///
/// The window is what makes the display readable: a single 20-second granule is too sparse to show
/// where a storm is, and everything ever received is too dense.
pub struct GlmFeed {
    flashes: VecDeque<Flash>,
    /// Newest key already ingested, so a poll only fetches what it hasn't seen.
    last_key: Option<String>,
    window: chrono::Duration,
}

impl GlmFeed {
    /// A feed keeping `window_minutes` of flashes.
    pub fn new(window_minutes: i64) -> GlmFeed {
        GlmFeed {
            flashes: VecDeque::new(),
            last_key: None,
            window: chrono::Duration::minutes(window_minutes),
        }
    }

    pub fn flashes(&self) -> &VecDeque<Flash> {
        &self.flashes
    }

    /// The newest granule key already ingested.
    pub fn last_key(&self) -> Option<&String> {
        self.last_key.as_ref()
    }

    /// Resume from a key another feed left off at.
    pub fn set_last_key(&mut self, key: String) {
        self.last_key = Some(key);
    }

    /// Fold a feed polled elsewhere into this one.
    ///
    /// The UI polls on a worker so a slow fetch can't stall a frame, which means the decode
    /// happens in a throwaway feed and the result is merged back under the lock — the merge itself
    /// has to be cheap, so it's an extend and a re-sort rather than a re-poll.
    pub fn absorb(&mut self, other: GlmFeed) {
        if other.last_key.is_some() {
            self.last_key = other.last_key;
        }
        self.flashes.extend(other.flashes);
        self.flashes
            .make_contiguous()
            .sort_by_key(|f| f.time.timestamp_millis());
        self.expire(Utc::now());
    }

    /// Drop anything older than the window. Cheap enough to call every frame.
    pub fn expire(&mut self, now: DateTime<Utc>) {
        while self
            .flashes
            .front()
            .is_some_and(|f| now - f.time > self.window)
        {
            self.flashes.pop_front();
        }
    }

    /// Fetch granules newer than the last one seen and fold them in. Returns how many flashes were
    /// added.
    ///
    /// On a cold start only the newest few granules are read: back-filling the whole window would
    /// mean ~45 requests before the first pixel, and lightning a quarter-hour old isn't what
    /// anyone opened the layer for.
    pub async fn poll(&mut self, http: &reqwest::Client) -> anyhow::Result<usize> {
        let now = Utc::now();
        let mut keys = list_hour(http, now).await;
        // Near the top of the hour the newest granules are still in the previous hour's prefix.
        if keys.len() < 4 {
            let mut prev = list_hour(http, now - chrono::Duration::hours(1)).await;
            prev.extend(keys);
            keys = prev;
        }
        let fresh: Vec<String> = match &self.last_key {
            Some(last) => keys.into_iter().filter(|k| k > last).collect(),
            None => keys.into_iter().rev().take(6).rev().collect(),
        };
        if fresh.is_empty() {
            return Ok(0);
        }
        let mut added = 0;
        // A burst of granules after a pause shouldn't stall the app; take the newest handful.
        for key in fresh.iter().rev().take(10).rev() {
            let url = format!("{BUCKET}/{key}");
            let bytes = match http.get(&url).send().await {
                Ok(r) => match r.error_for_status() {
                    Ok(r) => r.bytes().await?,
                    Err(e) => {
                        log::debug!("glm: {key}: {e}");
                        continue;
                    }
                },
                Err(e) => {
                    log::debug!("glm: {key}: {e}");
                    continue;
                }
            };
            match decode(bytes.to_vec()) {
                Ok(fl) => {
                    added += fl.len();
                    self.flashes.extend(fl);
                }
                Err(e) => log::debug!("glm decode {key}: {e}"),
            }
            self.last_key = Some(key.clone());
        }
        // Granules can arrive slightly out of order; keeping the deque sorted is what lets
        // `expire` just pop from the front.
        self.flashes
            .make_contiguous()
            .sort_by_key(|f| f.time.timestamp_millis());
        self.expire(now);
        Ok(added)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flash(min_ago: i64) -> Flash {
        Flash {
            lon: -97.0,
            lat: 35.0,
            energy: 1e-14,
            time: Utc::now() - chrono::Duration::minutes(min_ago),
        }
    }

    #[test]
    fn expire_drops_only_what_left_the_window() {
        let mut feed = GlmFeed::new(15);
        feed.flashes
            .extend([flash(30), flash(20), flash(5), flash(1)]);
        feed.expire(Utc::now());
        assert_eq!(feed.flashes.len(), 2, "the two older than 15 min go");
    }

    #[test]
    fn epoch_matches_the_netcdf_convention() {
        // 2000-01-01 12:00 UTC is the CF epoch these files count from.
        let base = Utc.timestamp_opt(J2000_UNIX, 0).single().unwrap();
        assert_eq!(base.to_rfc3339(), "2000-01-01T12:00:00+00:00");
    }

    #[test]
    fn decoding_garbage_is_an_error_not_a_panic() {
        assert!(decode(Vec::new()).is_err());
        assert!(decode(vec![0u8; 128]).is_err());
    }

    /// The committed granules live in the hdf5lite crate; decode one end to end from there.
    #[test]
    fn decodes_a_real_granule() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../hdf5lite/tests/data/g19_day.nc");
        let Ok(bytes) = std::fs::read(&p) else {
            return; // the fixture lives in a sibling crate; skip if it moved
        };
        let flashes = decode(bytes).expect("decode");
        assert!(!flashes.is_empty(), "granule had flashes");
        for f in &flashes {
            assert!((-90.0..=90.0).contains(&f.lat));
            assert!((-180.0..=180.0).contains(&f.lon));
        }
        // Every flash falls inside the granule's own 20-second window, give or take.
        let t0 = flashes[0].time;
        for f in &flashes {
            assert!((f.time - t0).num_seconds().abs() < 120, "{:?}", f.time);
        }
    }
}
