//! Curated "time machine" event library: famous storms with a radar site, UTC time, and a map
//! target, so a click deep-links the active pane straight into the archive. The list is static;
//! user-saved bookmarks live in [`crate::settings::Settings::bookmarks`].

use chrono::{DateTime, Utc};

/// A curated archive event.
pub struct RadarEvent {
    pub name: &'static str,
    pub site: &'static str,
    /// RFC3339 UTC instant to seek the timeline to.
    pub time: &'static str,
    pub lon: f64,
    pub lat: f64,
    pub zoom: f64,
    pub blurb: &'static str,
}

impl RadarEvent {
    /// Parse the event's instant (panics only on a malformed literal, caught by the unit test).
    pub fn datetime(&self) -> DateTime<Utc> {
        self.time.parse().expect("event time is valid RFC3339")
    }
}

/// Famous, radar-archive-available severe weather events (all on the public NEXRAD L2 bucket).
pub const EVENTS: &[RadarEvent] = &[
    RadarEvent {
        name: "Moore, OK EF5 tornado",
        site: "KTLX",
        time: "2013-05-20T20:00:00Z",
        lon: -97.48,
        lat: 35.34,
        zoom: 9.0,
        blurb: "May 20 2013 — EF5 tracks through Moore; textbook hook echo south of OKC.",
    },
    RadarEvent {
        name: "El Reno, OK — widest tornado on record",
        site: "KTLX",
        time: "2013-05-31T23:00:00Z",
        lon: -97.96,
        lat: 35.53,
        zoom: 9.0,
        blurb: "May 31 2013 — 2.6-mi-wide EF3; violent, erratic couplet west of OKC.",
    },
    RadarEvent {
        name: "Joplin, MO EF5 tornado",
        site: "KSGF",
        time: "2011-05-22T22:40:00Z",
        lon: -94.51,
        lat: 37.06,
        zoom: 9.0,
        blurb: "May 22 2011 — rain-wrapped EF5 devastates Joplin.",
    },
    RadarEvent {
        name: "Tuscaloosa–Birmingham EF4",
        site: "KBMX",
        time: "2011-04-27T22:10:00Z",
        lon: -87.57,
        lat: 33.21,
        zoom: 9.0,
        blurb: "Apr 27 2011 — long-track EF4 during the Super Outbreak.",
    },
    RadarEvent {
        name: "Mayfield, KY — Quad-State supercell",
        site: "KPAH",
        time: "2021-12-11T03:30:00Z",
        lon: -88.64,
        lat: 36.74,
        zoom: 9.0,
        blurb: "Dec 10-11 2021 — nocturnal long-track tornado, Mayfield destroyed.",
    },
    RadarEvent {
        name: "Greenfield, IA EF4",
        site: "KDMX",
        time: "2024-05-21T20:30:00Z",
        lon: -94.46,
        lat: 41.31,
        zoom: 9.0,
        blurb: "May 21 2024 — high-end EF4 with a striking velocity couplet.",
    },
    RadarEvent {
        name: "Hurricane Harvey landfall",
        site: "KCRP",
        time: "2017-08-26T03:00:00Z",
        lon: -97.05,
        lat: 28.02,
        zoom: 7.0,
        blurb: "Aug 26 2017 — Cat 4 landfall near Rockport, TX.",
    },
    RadarEvent {
        name: "Hurricane Ian landfall",
        site: "KTBW",
        time: "2022-09-28T19:00:00Z",
        lon: -82.20,
        lat: 26.70,
        zoom: 7.0,
        blurb: "Sep 28 2022 — Cat 5 landfall at Cayo Costa / SW Florida.",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_event_times_parse() {
        for e in EVENTS {
            let _ = e.datetime(); // would panic on a bad literal
            assert!(e.site.len() == 4 && e.site.starts_with('K'), "site {}", e.site);
            assert!(e.lat > 20.0 && e.lat < 50.0 && e.lon < -60.0, "coords {}", e.name);
        }
    }
}
