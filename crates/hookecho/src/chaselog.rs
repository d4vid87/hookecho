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
             creator=\"HookEcho\" xmlns=\"http://www.topografix.com/GPX/1/1\">\n",
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

/// Read a GPX file back into a track: every `<trkpt>` with a time, plus the `<wpt>` marks
/// snapped to the nearest point in time.
///
/// A hand-rolled scan rather than an XML parser, because this reads one shape of file — the one
/// [`Track::to_gpx`] writes, or the one a chase logger or a phone app exports, which is the same
/// three attributes in the same order. Anything it does not understand is skipped rather than
/// rejected: half a drive is still a drive.
//
// ponytail: attribute scan, no namespaces, no extensions (heart rate, elevation, speed). The
// ceiling is "the points and their times". A real GPX reader is a dependency, and this is the
// only file the app ever reads.
pub fn from_gpx(xml: &str) -> Track {
    fn attr(tag: &str, name: &str) -> Option<f64> {
        let at = tag.find(name)?;
        let rest = &tag[at + name.len()..];
        let rest = rest.trim_start().strip_prefix('=')?.trim_start();
        let quote = rest.chars().next()?;
        let body = rest[1..].split(quote).next()?;
        body.trim().parse().ok()
    }
    fn between<'a>(s: &'a str, open: &str, close: &str) -> Option<&'a str> {
        let a = s.find(open)? + open.len();
        let b = s[a..].find(close)? + a;
        Some(&s[a..b])
    }

    /// The text of one element, from just after its name to whichever comes first: its closing
    /// tag, or the `/>` that says it has no children (and so no `<time>`).
    fn body<'a>(rest: &'a str, close: &str) -> &'a str {
        let end = match (rest.find(close), rest.find("/>")) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (None, None) => rest.len(),
        };
        &rest[..end]
    }

    let mut track = Track::default();
    let mut pending: Vec<(i64, String)> = Vec::new();
    // Two passes, one per element kind: a `<time>` is a child element rather than an attribute,
    // so each point has to be read as a span of the file rather than as a single tag.
    for (open, close, is_point) in [("<trkpt", "</trkpt>", true), ("<wpt", "</wpt>", false)] {
        for piece in xml.split(open).skip(1) {
            let piece = body(piece, close);
            let (Some(lat), Some(lon)) = (attr(piece, "lat"), attr(piece, "lon")) else {
                continue;
            };
            let ts = between(piece, "<time>", "</time>")
                .and_then(|t| t.parse::<chrono::DateTime<chrono::Utc>>().ok())
                .map(|t| t.timestamp())
                .unwrap_or(0);
            if is_point {
                // Straight onto the vec: a recorded file has already been thinned, and dropping
                // points here would move the marks the waypoints refer to.
                track.points.push(Fix { lon, lat, ts });
            } else if let Some(name) = between(piece, "<name>", "</name>") {
                pending.push((ts, unescape(name)));
            }
        }
    }
    for (ts, label) in pending {
        // Nearest point in time, since a waypoint carries no index — which is also how it
        // survives a file whose points were thinned by whatever wrote it.
        if let Some((i, _)) = track
            .points
            .iter()
            .enumerate()
            .min_by_key(|(_, p)| (p.ts - ts).abs())
        {
            track.waypoints.push((i, label));
        }
    }
    track
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

/// …and back, for the labels [`from_gpx`] reads.
fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
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
    fn a_gpx_file_reads_back_as_the_track_that_wrote_it() {
        let mut t = Track::default();
        t.push(-97.5, 35.4, 1_700_000_000);
        t.push(-97.6, 35.4, 1_700_000_300);
        t.mark("wall cloud & \"hook\"");
        let back = from_gpx(&t.to_gpx());
        assert_eq!(back.points.len(), 2);
        assert!((back.points[0].lon + 97.5).abs() < 1e-6);
        assert_eq!(back.points[1].ts, 1_700_000_300);
        // The mark was on the newest point and lands there again, with its text unescaped.
        assert_eq!(
            back.waypoints,
            vec![(1, "wall cloud & \"hook\"".to_string())]
        );
        // Distance survives the round trip.
        assert!((back.miles() - t.miles()).abs() < 0.01);
    }

    #[test]
    fn a_file_it_does_not_understand_is_skipped_not_rejected() {
        // No times, single quotes, an element it has never seen.
        let xml = "<gpx><metadata><name>x</name></metadata><trk><trkseg>\
                   <trkpt lat='35.4' lon='-97.5'/><trkpt lat='bogus' lon='-97.6'/>\
                   </trkseg></trk></gpx>";
        let t = from_gpx(xml);
        assert_eq!(t.points.len(), 1);
        assert_eq!(t.points[0].ts, 0);
        assert!(from_gpx("not xml at all").points.is_empty());
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
