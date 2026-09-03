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

/// GOES-East. GOES-19 took over the slot from GOES-16 in 2025.
pub const EAST: &str = "https://noaa-goes19.s3.amazonaws.com";
/// GOES-West (GOES-18) — the Pacific, the west coast and the Rockies, which East sees at a very
/// oblique angle or not at all.
pub const WEST: &str = "https://noaa-goes18.s3.amazonaws.com";
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
async fn list_hour(http: &reqwest::Client, bucket: &str, t: DateTime<Utc>) -> Vec<String> {
    use chrono::Datelike;
    let prefix = format!(
        "{PRODUCT}/{:04}/{:03}/{:02}/",
        t.year(),
        t.ordinal(),
        t.format("%H")
    );
    let url = format!("{bucket}/?list-type=2&prefix={prefix}&max-keys=1000");
    let Ok(resp) = http.get(crate::net::fetch_url(&url))
    .timeout(crate::net::FEED_TIMEOUT).send().await else {
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
    /// Newest key already ingested per satellite bucket, so a poll only fetches what it hasn't
    /// seen. Keyed by bucket because East and West publish independent granule streams.
    last_keys: std::collections::BTreeMap<String, String>,
    /// Which satellites to poll. East alone by default; adding West costs a second listing per
    /// cycle and buys the Pacific and the west coast.
    buckets: Vec<&'static str>,
    window: chrono::Duration,
}

impl GlmFeed {
    /// A feed keeping `window_minutes` of flashes from GOES-East.
    pub fn new(window_minutes: i64) -> GlmFeed {
        GlmFeed {
            flashes: VecDeque::new(),
            last_keys: Default::default(),
            buckets: vec![EAST],
            window: chrono::Duration::minutes(window_minutes),
        }
    }

    /// Also poll GOES-West (or stop doing so).
    pub fn set_west(&mut self, on: bool) {
        self.buckets = if on { vec![EAST, WEST] } else { vec![EAST] };
    }

    pub fn flashes(&self) -> &VecDeque<Flash> {
        &self.flashes
    }

    /// The newest granule key already ingested, per satellite bucket.
    pub fn last_keys(&self) -> &std::collections::BTreeMap<String, String> {
        &self.last_keys
    }

    /// Resume from the keys another feed left off at.
    pub fn set_last_keys(&mut self, keys: std::collections::BTreeMap<String, String>) {
        self.last_keys = keys;
    }

    /// Fold a feed polled elsewhere into this one.
    ///
    /// The UI polls on a worker so a slow fetch can't stall a frame, which means the decode
    /// happens in a throwaway feed and the result is merged back under the lock — the merge itself
    /// has to be cheap, so it's an extend and a re-sort rather than a re-poll.
    pub fn absorb(&mut self, other: GlmFeed) {
        self.last_keys.extend(other.last_keys);
        // Both sides are already in time order, and a granule that just landed is normally newer
        // than everything held — so the usual merge is an append. Only an out-of-order granule
        // pays for a re-sort of the whole 15-minute window, under the painter's mutex.
        let ordered = match (self.flashes.back(), other.flashes.front()) {
            (Some(last), Some(first)) => first.time >= last.time,
            _ => true,
        };
        self.flashes.extend(other.flashes);
        if !ordered {
            self.flashes
                .make_contiguous()
                .sort_by_key(|f| f.time.timestamp_millis());
        }
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
        let mut added = 0;
        for bucket in self.buckets.clone() {
            added += self.poll_bucket(http, bucket, now).await?;
        }
        // Granules can arrive slightly out of order; keeping the deque sorted is what lets
        // `expire` just pop from the front.
        self.flashes
            .make_contiguous()
            .sort_by_key(|f| f.time.timestamp_millis());
        self.expire(now);
        Ok(added)
    }

    /// One satellite's share of a poll.
    async fn poll_bucket(
        &mut self,
        http: &reqwest::Client,
        bucket: &'static str,
        now: DateTime<Utc>,
    ) -> anyhow::Result<usize> {
        let mut keys = list_hour(http, bucket, now).await;
        // Near the top of the hour the newest granules are still in the previous hour's prefix.
        if keys.len() < 4 {
            let mut prev = list_hour(http, bucket, now - chrono::Duration::hours(1)).await;
            prev.extend(keys);
            keys = prev;
        }
        let fresh: Vec<String> = match self.last_keys.get(bucket) {
            Some(last) => keys.into_iter().filter(|k| k > last).collect(),
            None => keys.into_iter().rev().take(6).rev().collect(),
        };
        if fresh.is_empty() {
            return Ok(0);
        }
        let mut added = 0;
        // A burst of granules after a pause shouldn't stall the app; take the newest handful.
        for key in fresh.iter().rev().take(10).rev() {
            let url = format!("{bucket}/{key}");
            let bytes = match http.get(crate::net::fetch_url(&url))
            .timeout(crate::net::FEED_TIMEOUT).send().await {
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
            self.last_keys.insert(bucket.to_string(), key.clone());
        }
        Ok(added)
    }
}

/// Grid the recent flashes into flash-extent density: how many flashes fell in each cell over the
/// last `window`.
///
/// Individual flash dots answer "is it electrified"; density answers "where is it electrified
/// *most*", which is the part that tracks an updraft and the part a jump shows up in. The result
/// is a plate-carrée [`mrms::MrmsField`], so the existing warp/draw path renders it with no new
/// pipeline — empty cells stay `NaN` and draw as nothing, per that type's convention.
///
/// `None` when no flash falls inside the window.
// ponytail: fixed cell size and window, no user knob. Both are legible-at-a-glance choices, not
// tuning parameters; add settings if anyone actually wants a different one.
pub fn flash_density(
    flashes: &VecDeque<Flash>,
    cell_deg: f64,
    window: chrono::Duration,
    now: DateTime<Utc>,
) -> Option<crate::mrms::MrmsField> {
    let cutoff = now - window;
    let recent: Vec<&Flash> = flashes.iter().filter(|f| f.time >= cutoff).collect();
    let first = recent.first()?;

    // Snap the bounds to the cell lattice so a cell covers the same ground from frame to frame —
    // otherwise the whole grid slides half a cell every time the extremes change.
    let snap = |v: f64| (v / cell_deg).floor() * cell_deg;
    let (mut w, mut e, mut s, mut n) = (first.lon, first.lon, first.lat, first.lat);
    for f in &recent {
        w = w.min(f.lon);
        e = e.max(f.lon);
        s = s.min(f.lat);
        n = n.max(f.lat);
    }
    let (lon_west, lat_south) = (snap(w), snap(s));
    let nx = (((e - lon_west) / cell_deg).floor() as usize) + 1;
    let ny = (((n - lat_south) / cell_deg).floor() as usize) + 1;
    let (lon_east, lat_north) = (
        lon_west + nx as f64 * cell_deg,
        lat_south + ny as f64 * cell_deg,
    );

    let mut values = vec![f32::NAN; nx * ny];
    for f in &recent {
        let ix = ((f.lon - lon_west) / cell_deg).floor() as usize;
        // Row 0 is the northernmost latitude, matching MRMS.
        let iy = ((lat_north - f.lat) / cell_deg).floor() as usize;
        let (ix, iy) = (ix.min(nx - 1), iy.min(ny - 1));
        let cell = &mut values[iy * nx + ix];
        *cell = if cell.is_nan() { 1.0 } else { *cell + 1.0 };
    }

    Some(crate::mrms::MrmsField {
        values,
        nx,
        ny,
        lon_west,
        lon_east,
        lat_north,
        lat_south,
        time: now,
    })
}

/// Rate of change of flash-extent density between two [`flash_density`] grids, in flashes per
/// cell per minute — the lightning jump.
///
/// A storm that is about to do something usually electrifies first: total flash rate climbs
/// sharply several minutes before the severe weather arrives. A high density on its own only says
/// the storm is already big; the *rise* is the part with lead time in it.
///
/// Both grids snap to the same lattice, so cells line up by an integer offset and no interpolation
/// is involved — but only if they were built with the same cell size, which is checked. Cells that
/// fell (or held steady) come back `NaN`: a jump layer showing decay would bury the thing it is
/// for. `None` when the grids do not line up or no time passed between them.
pub fn flash_jump(
    prev: &crate::mrms::MrmsField,
    cur: &crate::mrms::MrmsField,
) -> Option<crate::mrms::MrmsField> {
    let dt_min = (cur.time - prev.time).num_seconds() as f32 / 60.0;
    if dt_min <= 0.0 || cur.nx == 0 || cur.ny == 0 || prev.nx == 0 || prev.ny == 0 {
        return None;
    }
    let cell = (cur.lon_east - cur.lon_west) / cur.nx as f64;
    let prev_cell = (prev.lon_east - prev.lon_west) / prev.nx as f64;
    if cell <= 0.0 || (cell - prev_cell).abs() > cell * 1e-6 {
        return None;
    }
    // Where cur's origin sits in prev's index space. Rows run north to south.
    let ox = ((cur.lon_west - prev.lon_west) / cell).round() as isize;
    let oy = ((prev.lat_north - cur.lat_north) / cell).round() as isize;
    let mut values = vec![f32::NAN; cur.nx * cur.ny];
    for y in 0..cur.ny {
        for x in 0..cur.nx {
            let now = cur.values[y * cur.nx + x];
            if !now.is_finite() {
                continue;
            }
            let (px, py) = (x as isize + ox, y as isize + oy);
            let before = if px >= 0 && py >= 0 && (px as usize) < prev.nx && (py as usize) < prev.ny
            {
                let v = prev.values[py as usize * prev.nx + px as usize];
                if v.is_finite() {
                    v
                } else {
                    0.0
                }
            } else {
                // Off the edge of the older grid: the storm moved in, so all of it is new.
                0.0
            };
            let rate = (now - before) / dt_min;
            if rate > 0.0 {
                values[y * cur.nx + x] = rate;
            }
        }
    }
    Some(crate::mrms::MrmsField {
        values,
        nx: cur.nx,
        ny: cur.ny,
        lon_west: cur.lon_west,
        lon_east: cur.lon_east,
        lat_north: cur.lat_north,
        lat_south: cur.lat_south,
        time: cur.time,
    })
}

#[cfg(test)]
mod jump_tests {
    use super::*;

    fn grid(vals: &[f32], lon_west: f64, lat_north: f64, min_ago: i64) -> crate::mrms::MrmsField {
        crate::mrms::MrmsField {
            values: vals.to_vec(),
            nx: 2,
            ny: 1,
            lon_west,
            lon_east: lon_west + 2.0,
            lat_north,
            lat_south: lat_north - 1.0,
            time: Utc::now() - chrono::Duration::minutes(min_ago),
        }
    }

    #[test]
    fn a_rising_cell_reports_its_rate_and_a_falling_one_reports_nothing() {
        let prev = grid(&[10.0, 30.0], 0.0, 10.0, 5);
        let cur = grid(&[20.0, 12.0], 0.0, 10.0, 0);
        let j = flash_jump(&prev, &cur).expect("same lattice, five minutes apart");
        assert!((j.values[0] - 2.0).abs() < 1e-4, "10 flashes over 5 min");
        assert!(j.values[1].is_nan(), "decay is not a jump");
    }

    #[test]
    fn a_shifted_grid_lines_up_by_offset_and_new_ground_counts_as_all_new() {
        // cur sits one cell east of prev: cur cell 0 is prev cell 1, cur cell 1 is off the edge.
        let prev = grid(&[0.0, 5.0], 0.0, 10.0, 5);
        let cur = grid(&[10.0, 10.0], 1.0, 10.0, 0);
        let j = flash_jump(&prev, &cur).unwrap();
        assert!((j.values[0] - 1.0).abs() < 1e-4, "(10 - 5) / 5 min");
        assert!((j.values[1] - 2.0).abs() < 1e-4, "nothing there before");
    }

    #[test]
    fn mismatched_cell_sizes_and_stopped_clocks_are_refused() {
        let prev = grid(&[1.0, 1.0], 0.0, 10.0, 5);
        let mut coarse = grid(&[2.0, 2.0], 0.0, 10.0, 0);
        coarse.lon_east = coarse.lon_west + 4.0;
        assert!(flash_jump(&prev, &coarse).is_none());
        assert!(flash_jump(&prev, &grid(&[2.0, 2.0], 0.0, 10.0, 5)).is_none());
    }
}

#[cfg(test)]
mod density_tests {
    use super::*;

    fn at(lon: f64, lat: f64, min_ago: i64) -> Flash {
        Flash {
            lon,
            lat,
            energy: 1.0,
            time: Utc::now() - chrono::Duration::minutes(min_ago),
        }
    }

    #[test]
    fn flashes_in_one_cell_stack_and_stale_ones_drop_out() {
        let flashes: VecDeque<Flash> = [
            at(-97.01, 35.01, 1),
            at(-97.02, 35.02, 2),
            at(-97.03, 35.03, 3),
            at(-97.01, 35.01, 60), // outside the window
        ]
        .into_iter()
        .collect();
        let f = flash_density(&flashes, 0.05, chrono::Duration::minutes(15), Utc::now()).unwrap();
        let total: f32 = f.values.iter().filter(|v| !v.is_nan()).sum();
        assert_eq!(total, 3.0);
        assert_eq!(f.values.iter().filter(|v| **v == 3.0).count(), 1);
    }

    #[test]
    fn separate_cells_stay_separate() {
        let flashes: VecDeque<Flash> = [at(-97.0, 35.0, 1), at(-96.5, 35.4, 1)]
            .into_iter()
            .collect();
        let f = flash_density(&flashes, 0.05, chrono::Duration::minutes(15), Utc::now()).unwrap();
        assert_eq!(f.values.iter().filter(|v| **v == 1.0).count(), 2);
        assert!(f.lon_west <= -97.0 && f.lon_east >= -96.5);
        assert!(f.lat_south <= 35.0 && f.lat_north >= 35.4);
    }

    #[test]
    fn nothing_recent_is_no_grid() {
        let flashes: VecDeque<Flash> = [at(-97.0, 35.0, 90)].into_iter().collect();
        assert!(flash_density(&flashes, 0.05, chrono::Duration::minutes(15), Utc::now()).is_none());
    }
}

#[cfg(test)]
mod tests {
    /// A merge must leave the window in time order — that is what lets `expire` pop the front —
    /// but the ordinary case (a granule newer than everything held) has to reach it without
    /// re-sorting a quarter-hour of flashes under the painter's mutex.
    #[test]
    fn absorbing_keeps_the_window_in_time_order() {
        use chrono::Utc;
        // Inside the window: `absorb` expires as it goes, and a fixed epoch would be dropped.
        let base = Utc::now() - chrono::Duration::seconds(120);
        let flash = |secs: i64| super::Flash {
            lon: -97.0,
            lat: 35.0,
            energy: 1.0,
            time: base + chrono::Duration::seconds(secs),
        };
        let offsets = |f: &super::GlmFeed| -> Vec<i64> {
            f.flashes()
                .iter()
                .map(|x| (x.time - base).num_seconds())
                .collect()
        };

        let mut held = super::GlmFeed::new(15);
        held.flashes.extend([flash(10), flash(20)]);

        let mut newer = super::GlmFeed::new(15);
        newer.flashes.extend([flash(30), flash(40)]);
        held.absorb(newer);
        assert_eq!(offsets(&held), vec![10, 20, 30, 40]);

        // A granule that arrives late still lands in the right place.
        let mut late = super::GlmFeed::new(15);
        late.flashes.extend([flash(25), flash(50)]);
        held.absorb(late);
        assert_eq!(offsets(&held), vec![10, 20, 25, 30, 40, 50]);
    }

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
