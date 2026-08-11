//! The chase log: where you have been, and a GPX file of it.
//!
//! Every GPS fix the app already receives is offered here; the track keeps the ones that mean
//! something (far enough apart, or long enough after the last one) so a stationary hour is one
//! point rather than three thousand. The export is plain GPX 1.1, which every mapping tool and
//! every video overlay reads.

/// One recorded fix.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fix {
    pub lon: f64,
    pub lat: f64,
    /// Unix seconds.
    pub ts: i64,
}

/// Minimum movement between kept points. Below this a fix is GPS noise around a parked car.
const MIN_MOVE_M: f64 = 40.0;
/// …unless this long has passed, so a long stop still leaves evidence it happened.
const MIN_GAP_S: i64 = 120;
/// Ceiling on kept points. At one point every few seconds this is many hours of driving, and it
/// bounds both the memory and the polyline the map draws.
const MAX_POINTS: usize = 20_000;

#[derive(Debug, Default, Clone)]
pub struct Track {
    pub points: Vec<Fix>,
    /// Named marks the user dropped along the way (index into `points`, label).
    pub waypoints: Vec<(usize, String)>,
}

impl Track {
    /// Offer a fix. Returns true when it was kept, so the caller can repaint only then.
    pub fn push(&mut self, lon: f64, lat: f64, ts: i64) -> bool {
        if let Some(last) = self.points.last() {
            let moved = haversine_m(last.lon, last.lat, lon, lat);
            if moved < MIN_MOVE_M && ts - last.ts < MIN_GAP_S {
                return false;
            }
        }
        self.points.push(Fix { lon, lat, ts });
        if self.points.len() > MAX_POINTS {
            // Drop the oldest half rather than one point per fix: a memmove of 20k points on
            // every fix would be the most expensive thing in the log.
            let cut = self.points.len() / 2;
            self.points.drain(..cut);
            self.waypoints.retain(|(i, _)| *i >= cut);
            for (i, _) in &mut self.waypoints {
                *i -= cut;
            }
        }
        true
    }

    /// Mark the newest point with a label ("wall cloud", "first tornado").
    pub fn mark(&mut self, label: impl Into<String>) {
        if self.points.is_empty() {
            return;
        }
        self.waypoints.push((self.points.len() - 1, label.into()));
    }

    pub fn clear(&mut self) {
        self.points.clear();
        self.waypoints.clear();
    }

    /// Distance travelled along the track, in miles.
    pub fn miles(&self) -> f64 {
        self.points
            .windows(2)
            .map(|w| haversine_m(w[0].lon, w[0].lat, w[1].lon, w[1].lat))
            .sum::<f64>()
            / 1609.344
    }

    /// GPX 1.1: the waypoints first (that is the order the schema wants), then one track.
    pub fn to_gpx(&self) -> String {
        let time = |ts: i64| {
            chrono::DateTime::from_timestamp(ts, 0)
                .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
                .unwrap_or_default()
        };
        let mut s = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<gpx version=\"1.1\" \
             creator=\"Hook Echo-WX\" xmlns=\"http://www.topografix.com/GPX/1/1\">\n",
        );
        for (idx, label) in &self.waypoints {
            let Some(p) = self.points.get(*idx) else {
                continue;
            };
            s.push_str(&format!(
                "  <wpt lat=\"{:.6}\" lon=\"{:.6}\"><time>{}</time><name>{}</name></wpt>\n",
                p.lat,
                p.lon,
                time(p.ts),
                escape(label)
            ));
        }
        s.push_str("  <trk><name>Chase</name><trkseg>\n");
        for p in &self.points {
            s.push_str(&format!(
                "    <trkpt lat=\"{:.6}\" lon=\"{:.6}\"><time>{}</time></trkpt>\n",
                p.lat,
                p.lon,
                time(p.ts)
            ));
        }
        s.push_str("  </trkseg></trk>\n</gpx>\n");
        s
    }
}

/// The five characters XML cannot carry raw. A waypoint label is user text and lands in a file
/// other programs parse.
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Great-circle distance in metres.
fn haversine_m(lon0: f64, lat0: f64, lon1: f64, lat1: f64) -> f64 {
    const R: f64 = 6_371_000.0;
    let (p0, p1) = (lat0.to_radians(), lat1.to_radians());
    let (dp, dl) = ((lat1 - lat0).to_radians(), (lon1 - lon0).to_radians());
    let a = (dp / 2.0).sin().powi(2) + p0.cos() * p1.cos() * (dl / 2.0).sin().powi(2);
    2.0 * R * a.sqrt().clamp(0.0, 1.0).asin()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_while_parked_is_not_a_track() {
        let mut t = Track::default();
        assert!(t.push(-97.5, 35.4, 0));
        // A few metres a second later: the same place.
        assert!(!t.push(-97.5001, 35.4, 1));
        // Same place, but two minutes on: worth a point, so a stop is visible in the file.
        assert!(t.push(-97.5001, 35.4, 200));
        // Driving.
        assert!(t.push(-97.6, 35.4, 260));
        assert_eq!(t.points.len(), 3);
        assert!(t.miles() > 5.0 && t.miles() < 7.0, "{}", t.miles());
    }

    #[test]
    fn gpx_carries_points_and_escaped_waypoints() {
        let mut t = Track::default();
        t.push(-97.5, 35.4, 1_700_000_000);
        t.mark("wall cloud & \"hook\"");
        let gpx = t.to_gpx();
        assert!(gpx.contains("<trkpt lat=\"35.400000\" lon=\"-97.500000\">"));
        assert!(gpx.contains("wall cloud &amp; &quot;hook&quot;"), "{gpx}");
        assert!(gpx.contains("2023-11-14T"), "{gpx}");
        // Waypoints come before the track, per the GPX schema's element order.
        assert!(gpx.find("<wpt").unwrap() < gpx.find("<trk>").unwrap());
    }

    #[test]
    fn the_track_is_bounded() {
        let mut t = Track::default();
        for i in 0..(MAX_POINTS + 100) {
            t.push(-97.5 + i as f64 * 0.01, 35.4, i as i64 * 10);
        }
        assert!(t.points.len() <= MAX_POINTS);
        // And a waypoint on a dropped point does not become a waypoint on some other point.
        assert!(t.waypoints.iter().all(|(i, _)| *i < t.points.len()));
    }
}
