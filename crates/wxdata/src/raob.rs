//! Observed upper-air soundings (RAOB) from the University of Wyoming's text service.
//!
//! What the balloon actually measured, beside what the model thinks. HRRR profiles are a forecast
//! of the atmosphere; a radiosonde is a sample of it, and on a chase day the difference between
//! the two is the story. UWyo is the source that serves both: the current 00Z/12Z within an hour
//! or two, and every ascent back to the 1970s — which is what makes the Event Library dates
//! (Moore, Joplin, El Reno) come with their real soundings attached.
//!
//! The service returns an HTML page wrapping one `<PRE>` block of fixed-width columns. Fixed-width
//! is load-bearing: high-altitude levels routinely drop the wind columns entirely, so splitting on
//! whitespace silently shifts theta into the wind field. Columns are 7 characters each.
//!
//! Observations are immutable, so a fetched profile can be cached by (station, time) forever.

use crate::alerts::USER_AGENT;
use crate::sounding::{Sounding, SoundingLevel};
use chrono::{DateTime, Datelike, TimeZone, Timelike, Utc};

const URL: &str = "https://weather.uwyo.edu/wsgi/sounding";

/// A radiosonde launch site: WMO number, a human label, and where it stands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RaobStation {
    pub id: &'static str,
    pub name: &'static str,
    pub lat: f64,
    pub lon: f64,
}

/// Every fixed North American site the UWyo service currently carries, taken from its own station
/// list rather than a hand-typed table — it is the authority on what can actually be fetched.
pub const STATIONS: [RaobStation; 120] = [
    RaobStation {
        id: "70026",
        name: "Barrow",
        lat: 71.323,
        lon: -156.618,
    },
    RaobStation {
        id: "70200",
        name: "Nome, AK",
        lat: 64.507,
        lon: -165.434,
    },
    RaobStation {
        id: "70219",
        name: "Bethel, AK",
        lat: 60.785,
        lon: -161.840,
    },
    RaobStation {
        id: "70231",
        name: "Mcgrath, AK",
        lat: 62.955,
        lon: -155.601,
    },
    RaobStation {
        id: "70261",
        name: "Fairbanks, AK",
        lat: 64.816,
        lon: -147.876,
    },
    RaobStation {
        id: "70273",
        name: "Anchorage, AK",
        lat: 61.157,
        lon: -149.987,
    },
    RaobStation {
        id: "70316",
        name: "Cold Bay, AK",
        lat: 55.212,
        lon: -162.724,
    },
    RaobStation {
        id: "70326",
        name: "King Salmon, AK",
        lat: 58.679,
        lon: -156.669,
    },
    RaobStation {
        id: "70361",
        name: "Yakutat, AK",
        lat: 59.511,
        lon: -139.672,
    },
    RaobStation {
        id: "70398",
        name: "Annette Island, AK",
        lat: 55.045,
        lon: -131.587,
    },
    RaobStation {
        id: "71043",
        name: "Norman Wells Ua, Canada",
        lat: 65.288,
        lon: -126.753,
    },
    RaobStation {
        id: "71081",
        name: "Hall Beach Ua, Canada",
        lat: 68.765,
        lon: -81.225,
    },
    RaobStation {
        id: "71109",
        name: "Port Hardy Ua, Canada",
        lat: 50.685,
        lon: -127.376,
    },
    RaobStation {
        id: "71119",
        name: "Edmonton Stony Plain, Canada",
        lat: 53.548,
        lon: -114.109,
    },
    RaobStation {
        id: "71169",
        name: "",
        lat: 52.174,
        lon: -106.719,
    },
    RaobStation {
        id: "71603",
        name: "Yarmouth, Canada",
        lat: 43.831,
        lon: -66.088,
    },
    RaobStation {
        id: "71722",
        name: "Maniwaki, Canada",
        lat: 46.142,
        lon: -76.081,
    },
    RaobStation {
        id: "71811",
        name: "Sept-Iles Ua, Canada",
        lat: 50.219,
        lon: -66.241,
    },
    RaobStation {
        id: "71816",
        name: "Goose Bay, Canada",
        lat: 53.306,
        lon: -60.363,
    },
    RaobStation {
        id: "71867",
        name: "The Pas, Canada",
        lat: 53.974,
        lon: -101.088,
    },
    RaobStation {
        id: "71906",
        name: "Kuujjuaq, Canada",
        lat: 58.109,
        lon: -68.411,
    },
    RaobStation {
        id: "71907",
        name: "Inukjuak, Canada",
        lat: 58.467,
        lon: -78.078,
    },
    RaobStation {
        id: "71908",
        name: "Prince George, Canada",
        lat: 53.900,
        lon: -122.791,
    },
    RaobStation {
        id: "71909",
        name: "Iqaluit, Canada",
        lat: 63.747,
        lon: -68.544,
    },
    RaobStation {
        id: "71924",
        name: "Resolute Ua, Canada",
        lat: 74.705,
        lon: -94.969,
    },
    RaobStation {
        id: "71925",
        name: "Cambridge Bay Ua, Canada",
        lat: 69.129,
        lon: -105.058,
    },
    RaobStation {
        id: "71926",
        name: "Baker Lake Ua, Canada",
        lat: 64.319,
        lon: -96.002,
    },
    RaobStation {
        id: "71934",
        name: "Fort Smith Ua, Canada",
        lat: 60.026,
        lon: -111.929,
    },
    RaobStation {
        id: "71945",
        name: "Fort Nelson Ua, Canada",
        lat: 58.841,
        lon: -122.573,
    },
    RaobStation {
        id: "71957",
        name: "Inuvik Ua, Canada",
        lat: 68.318,
        lon: -133.534,
    },
    RaobStation {
        id: "71964",
        name: "Whitehorse, Canada",
        lat: 60.733,
        lon: -135.098,
    },
    RaobStation {
        id: "72201",
        name: "Key West, FL",
        lat: 24.553,
        lon: -81.789,
    },
    RaobStation {
        id: "72202",
        name: "Miami, FL",
        lat: 25.755,
        lon: -80.384,
    },
    RaobStation {
        id: "72206",
        name: "Jacksonville, FL",
        lat: 30.483,
        lon: -81.701,
    },
    RaobStation {
        id: "72208",
        name: "Charleston, SC",
        lat: 32.895,
        lon: -80.028,
    },
    RaobStation {
        id: "72210",
        name: "Tampa Bay Area, FL",
        lat: 27.705,
        lon: -82.401,
    },
    RaobStation {
        id: "72215",
        name: "Peachtree City, GA",
        lat: 33.356,
        lon: -84.567,
    },
    RaobStation {
        id: "72230",
        name: "Birmingham, AL",
        lat: 33.180,
        lon: -86.783,
    },
    RaobStation {
        id: "72233",
        name: "Slidell",
        lat: 30.338,
        lon: -89.825,
    },
    RaobStation {
        id: "72235",
        name: "Jackson, MS",
        lat: 32.320,
        lon: -90.080,
    },
    RaobStation {
        id: "72240",
        name: "Lake Charles, LA",
        lat: 30.126,
        lon: -93.217,
    },
    RaobStation {
        id: "72248",
        name: "Shreveport, LA",
        lat: 32.452,
        lon: -93.842,
    },
    RaobStation {
        id: "72249",
        name: "Fort Worth, TX",
        lat: 32.835,
        lon: -97.298,
    },
    RaobStation {
        id: "72250",
        name: "Brownsville, TX",
        lat: 25.916,
        lon: -97.420,
    },
    RaobStation {
        id: "72251",
        name: "Corpus Christi, TX",
        lat: 27.779,
        lon: -97.505,
    },
    RaobStation {
        id: "72261",
        name: "Del Rio, TX",
        lat: 29.374,
        lon: -100.918,
    },
    RaobStation {
        id: "72265",
        name: "Midland, TX",
        lat: 31.943,
        lon: -102.190,
    },
    RaobStation {
        id: "72293",
        name: "San Diego, CA",
        lat: 32.845,
        lon: -117.124,
    },
    RaobStation {
        id: "72317",
        name: "Greensboro, NC",
        lat: 36.098,
        lon: -79.943,
    },
    RaobStation {
        id: "72318",
        name: "Blacksburg, VA",
        lat: 37.206,
        lon: -80.414,
    },
    RaobStation {
        id: "72327",
        name: "Nashville, TN",
        lat: 36.247,
        lon: -86.562,
    },
    RaobStation {
        id: "72340",
        name: "Little Rock, AR",
        lat: 34.836,
        lon: -92.260,
    },
    RaobStation {
        id: "72357",
        name: "Norman, OK",
        lat: 35.181,
        lon: -97.438,
    },
    RaobStation {
        id: "72363",
        name: "Amarillo, TX",
        lat: 35.233,
        lon: -101.709,
    },
    RaobStation {
        id: "72364",
        name: "Santa Teresa, NM",
        lat: 31.873,
        lon: -106.697,
    },
    RaobStation {
        id: "72365",
        name: "Albuquerque, NM",
        lat: 35.038,
        lon: -106.623,
    },
    RaobStation {
        id: "72376",
        name: "Flagstaff, AZ",
        lat: 35.231,
        lon: -111.820,
    },
    RaobStation {
        id: "72388",
        name: "Las Vegas",
        lat: 36.047,
        lon: -115.185,
    },
    RaobStation {
        id: "72393",
        name: "Vandenberg Afb, CA",
        lat: 34.750,
        lon: -120.560,
    },
    RaobStation {
        id: "72403",
        name: "Sterling, VA",
        lat: 38.977,
        lon: -77.486,
    },
    RaobStation {
        id: "72426",
        name: "Wilmington, OH",
        lat: 39.421,
        lon: -83.821,
    },
    RaobStation {
        id: "72440",
        name: "Springfield, MO",
        lat: 37.236,
        lon: -93.402,
    },
    RaobStation {
        id: "72451",
        name: "Dodge City, KS",
        lat: 37.762,
        lon: -99.969,
    },
    RaobStation {
        id: "72456",
        name: "Topeka, KS",
        lat: 39.073,
        lon: -95.630,
    },
    RaobStation {
        id: "72476",
        name: "Grand Junction, CO",
        lat: 39.120,
        lon: -108.524,
    },
    RaobStation {
        id: "72489",
        name: "Reno, NV",
        lat: 39.568,
        lon: -119.795,
    },
    RaobStation {
        id: "72493",
        name: "Oakland, CA",
        lat: 37.744,
        lon: -122.224,
    },
    RaobStation {
        id: "72501",
        name: "Upton, NY",
        lat: 40.865,
        lon: -72.863,
    },
    RaobStation {
        id: "72518",
        name: "Albany County Airport, NY",
        lat: 42.680,
        lon: -73.816,
    },
    RaobStation {
        id: "72520",
        name: "Pittsburgh, PA",
        lat: 40.532,
        lon: -80.217,
    },
    RaobStation {
        id: "72528",
        name: "Buffalo, NY",
        lat: 42.940,
        lon: -78.725,
    },
    RaobStation {
        id: "72558",
        name: "Valley, NE",
        lat: 41.319,
        lon: -96.383,
    },
    RaobStation {
        id: "72562",
        name: "North Platte, NE",
        lat: 41.134,
        lon: -100.700,
    },
    RaobStation {
        id: "72572",
        name: "Salt Lake City",
        lat: 40.772,
        lon: -111.955,
    },
    RaobStation {
        id: "72582",
        name: "Elko, NV",
        lat: 40.860,
        lon: -115.741,
    },
    RaobStation {
        id: "72597",
        name: "Medford, OR",
        lat: 42.377,
        lon: -122.882,
    },
    RaobStation {
        id: "72632",
        name: "White Lake, MI",
        lat: 42.699,
        lon: -83.472,
    },
    RaobStation {
        id: "72634",
        name: "Gaylord, MI",
        lat: 44.908,
        lon: -84.719,
    },
    RaobStation {
        id: "72645",
        name: "Green Bay, WI",
        lat: 44.498,
        lon: -88.112,
    },
    RaobStation {
        id: "72649",
        name: "Chanhassen, MN",
        lat: 44.849,
        lon: -93.564,
    },
    RaobStation {
        id: "72659",
        name: "Aberdeen, SD",
        lat: 45.455,
        lon: -98.414,
    },
    RaobStation {
        id: "72662",
        name: "Rapid City Wfo, SD",
        lat: 44.073,
        lon: -103.210,
    },
    RaobStation {
        id: "72672",
        name: "Riverton, WY",
        lat: 43.065,
        lon: -108.477,
    },
    RaobStation {
        id: "72681",
        name: "Boise, ID",
        lat: 43.568,
        lon: -116.211,
    },
    RaobStation {
        id: "72694",
        name: "Salem, OR",
        lat: 44.910,
        lon: -123.009,
    },
    RaobStation {
        id: "72712",
        name: "Caribou, ME",
        lat: 46.868,
        lon: -68.014,
    },
    RaobStation {
        id: "72747",
        name: "Int.Falls",
        lat: 48.565,
        lon: -93.396,
    },
    RaobStation {
        id: "72764",
        name: "Bismarck, ND",
        lat: 46.772,
        lon: -100.762,
    },
    RaobStation {
        id: "72768",
        name: "Glasgow, MT",
        lat: 48.206,
        lon: -106.626,
    },
    RaobStation {
        id: "72797",
        name: "Quillayute, WA",
        lat: 47.935,
        lon: -124.559,
    },
    RaobStation {
        id: "72800",
        name: "Arm Amf3",
        lat: 34.340,
        lon: -87.340,
    },
    RaobStation {
        id: "73033",
        name: "Vernon, Canada",
        lat: 50.239,
        lon: -119.290,
    },
    RaobStation {
        id: "74389",
        name: "Gray, ME",
        lat: 43.892,
        lon: -70.257,
    },
    RaobStation {
        id: "74455",
        name: "Quad City, IA",
        lat: 41.612,
        lon: -90.580,
    },
    RaobStation {
        id: "74560",
        name: "Lincoln, IL",
        lat: 40.151,
        lon: -89.338,
    },
    RaobStation {
        id: "74626",
        name: "Wfo Phoenix, AZ",
        lat: 33.450,
        lon: -111.950,
    },
    RaobStation {
        id: "74646",
        name: "Lamont, OK",
        lat: 36.620,
        lon: -97.480,
    },
    RaobStation {
        id: "74794",
        name: "Cape Kennedy, FL",
        lat: 28.460,
        lon: -80.550,
    },
    RaobStation {
        id: "76225",
        name: "Chihuahua, Mexico",
        lat: 28.670,
        lon: -106.020,
    },
    RaobStation {
        id: "76256",
        name: "Empalme, Mexico",
        lat: 27.950,
        lon: -110.750,
    },
    RaobStation {
        id: "76394",
        name: "Monterrey, Mexico",
        lat: 25.870,
        lon: -100.230,
    },
    RaobStation {
        id: "76405",
        name: "La Paz, Mexico",
        lat: 24.170,
        lon: -110.300,
    },
    RaobStation {
        id: "76458",
        name: "Colonia Juan Carrasco Mazatlan, Mexico",
        lat: 23.200,
        lon: -106.420,
    },
    RaobStation {
        id: "76526",
        name: "Zacatecas, Mexico",
        lat: 22.750,
        lon: -102.500,
    },
    RaobStation {
        id: "76595",
        name: "Cancun, Mexico",
        lat: 21.020,
        lon: -86.850,
    },
    RaobStation {
        id: "76612",
        name: "Guadalajara, Mexico",
        lat: 20.670,
        lon: -103.380,
    },
    RaobStation {
        id: "76644",
        name: "Aerop.Internacional Merida, Mexico",
        lat: 20.980,
        lon: -89.650,
    },
    RaobStation {
        id: "76654",
        name: "Manzanillo, Mexico",
        lat: 19.050,
        lon: -104.320,
    },
    RaobStation {
        id: "76679",
        name: "Aerop. Internacional Mexico, Mexico",
        lat: 19.400,
        lon: -99.200,
    },
    RaobStation {
        id: "76692",
        name: "Hacienda Ylang Ylang Veracruz, Mexico",
        lat: 19.150,
        lon: -96.120,
    },
    RaobStation {
        id: "76805",
        name: "Acapulco, Mexico",
        lat: 16.750,
        lon: -99.750,
    },
    RaobStation {
        id: "78016",
        name: "Bermuda Naval Air Station Kindley, Bermuda",
        lat: 32.370,
        lon: -64.680,
    },
    RaobStation {
        id: "78397",
        name: "Kingston, Jamaica",
        lat: 17.939,
        lon: -76.781,
    },
    RaobStation {
        id: "78486",
        name: "Santo Domingo",
        lat: 18.430,
        lon: -69.880,
    },
    RaobStation {
        id: "78526",
        name: "San Juan",
        lat: 18.431,
        lon: -65.992,
    },
    RaobStation {
        id: "78583",
        name: "Belize, Belize",
        lat: 17.534,
        lon: -88.313,
    },
    RaobStation {
        id: "78866",
        name: "Juliana Airport",
        lat: 18.050,
        lon: -63.120,
    },
    RaobStation {
        id: "78897",
        name: "Le Raizet",
        lat: 16.262,
        lon: -61.528,
    },
    RaobStation {
        id: "91165",
        name: "Lihue",
        lat: 21.987,
        lon: -159.338,
    },
    RaobStation {
        id: "91285",
        name: "Hilo, HI",
        lat: 19.717,
        lon: -155.049,
    },
];

/// The station nearest `(lon, lat)`, or `None` when the nearest is over 800 km away (open ocean).
/// Flat-earth distance is fine at this scale: the answer only has to beat the runner-up.
pub fn nearest_station(lon: f64, lat: f64) -> Option<&'static RaobStation> {
    let cos = lat.to_radians().cos().abs().max(0.01);
    let best = STATIONS.iter().min_by(|a, b| {
        let d = |s: &RaobStation| {
            let (dx, dy) = ((s.lon - lon) * cos, s.lat - lat);
            dx * dx + dy * dy
        };
        d(a).total_cmp(&d(b))
    })?;
    let (dx, dy) = ((best.lon - lon) * cos * 111.32, (best.lat - lat) * 111.32);
    ((dx * dx + dy * dy).sqrt() <= 800.0).then_some(best)
}

/// The synoptic launch time at or before `when`. Balloons go up at 00Z and 12Z; a 21Z click wants
/// the 12Z ascent, not tomorrow's.
pub fn synoptic_before(when: DateTime<Utc>) -> DateTime<Utc> {
    let hour = if when.hour() >= 12 { 12 } else { 0 };
    Utc.with_ymd_and_hms(when.year(), when.month(), when.day(), hour, 0, 0)
        .single()
        .unwrap_or(when)
}

/// One field of the fixed-width table, or `None` when the column is blank (a missing measurement).
fn field(line: &str, i: usize) -> Option<f64> {
    let (lo, hi) = (i * 7, (i + 1) * 7);
    line.get(lo..hi)?.trim().parse::<f64>().ok()
}

/// Parse the `<PRE>` table into levels, surface first. Levels missing pressure, temperature or
/// dewpoint are dropped — the profile is what the Skew-T draws. Missing *wind* is kept as calm,
/// since dropping the level would put a hole in the temperature trace to save a hodograph point.
///
/// `knots` says the file reports wind speed in knots rather than m/s; the units row is read from
/// the header rather than assumed, because the service has served both.
pub fn parse_table(text: &str) -> Vec<SoundingLevel> {
    let knots = text
        .lines()
        .find(|l| l.contains("hPa") && l.contains("deg"))
        .is_some_and(|l| l.contains("knot"));
    let mut out = Vec::new();
    for line in text.lines() {
        // Data rows start with a right-aligned pressure; headers and rules never parse as one.
        let (Some(p), Some(t), Some(td)) = (field(line, 0), field(line, 2), field(line, 3)) else {
            continue;
        };
        if !(1.0..=1100.0).contains(&p) {
            continue;
        }
        // Wind is (direction from, speed). Meteorological convention: 270° means *from* the west,
        // so the vector points east — hence the negated sine/cosine.
        let (dir, spd) = (field(line, 6), field(line, 7));
        let (u, v) = match (dir, spd) {
            (Some(d), Some(s)) => {
                let ms = if knots { s * 0.514_444 } else { s };
                let r = d.to_radians();
                (-ms * r.sin(), -ms * r.cos())
            }
            _ => (0.0, 0.0),
        };
        out.push(SoundingLevel {
            pressure_hpa: p,
            temp_c: t,
            dewpt_c: td,
            u_ms: u,
            v_ms: v,
        });
    }
    out
}

/// Pull the `<PRE>` block out of the returned page.
fn pre_block(html: &str) -> Option<&str> {
    let start = html.find("<PRE>")? + "<PRE>".len();
    let end = html[start..].find("</PRE>")? + start;
    Some(&html[start..end])
}

/// Fetch the observed sounding for `station` at the synoptic time at or before `when`.
///
/// Errors when the station didn't launch (weather, equipment, a site that only flies 12Z) — that's
/// a normal outcome, not a fault, and the caller should say so plainly.
///
/// A past ascent never changes, so with `cache_dir` set the raw table is kept on disk forever and
/// the service is asked once per station+time — which also carries us through UWyo downtime.
pub async fn fetch(
    client: &reqwest::Client,
    station: &RaobStation,
    when: DateTime<Utc>,
    cache_dir: Option<std::path::PathBuf>,
) -> anyhow::Result<Sounding> {
    let launch = synoptic_before(when);
    let cache_file = cache_dir.map(|d| {
        d.join("raob")
            .join(format!("{}-{}.txt", station.id, launch.format("%Y%m%d%H")))
    });
    let cached = cache_file
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok());
    let table = match cached {
        Some(t) => t,
        None => {
            let body = client
                .get(crate::net::fetch_url(URL))
                .query(&[
                    ("datetime", launch.format("%Y-%m-%d %H:00:00").to_string()),
                    ("id", station.id.to_string()),
                    // The form's own value for "look the source up yourself".
                    ("src", "UNKNOWN".to_string()),
                    ("type", "TEXT:LIST".to_string()),
                ])
                .header("User-Agent", USER_AGENT)
                .send()
                .await?
                .error_for_status()?
                .text()
                .await?;
            let table = pre_block(&body)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "no sounding for {} at {}",
                        station.name,
                        launch.format("%Y-%m-%d %HZ")
                    )
                })?
                .to_string();
            if let Some(p) = &cache_file {
                if let Some(dir) = p.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = std::fs::write(p, &table);
            }
            table
        }
    };
    let levels = parse_table(&table);
    anyhow::ensure!(
        levels.len() >= 5,
        "sounding for {} at {} has only {} usable levels",
        station.name,
        launch.format("%Y-%m-%d %HZ"),
        levels.len()
    );
    Ok(Sounding {
        lon: station.lon,
        lat: station.lat,
        run: launch,
        levels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The head of the real OUN 12Z ascent from 20 May 2013 — the morning of the Moore EF5 — plus
    /// a top-of-sounding row with the wind columns blank, which is where whitespace splitting goes
    /// wrong.
    const OUN: &str = "\
-----------------------------------------------------------------------------
   PRES   HGHT   TEMP   DWPT   RELH   MIXR   DRCT   SPED   THTA   THTE   THTV
    hPa      m      C      C      %   g/kg    deg    m/s      K      K      K 
-----------------------------------------------------------------------------
  966.0    345   21.6   19.7     89  15.11    160    3.6  297.7  341.7  300.4
  958.0    416   21.2   19.5     90  15.05    167    6.4  298.0  341.8  300.7
  936.6    610   20.4   19.2     93  15.09    185   13.9  299.1  343.3  301.8
  925.0    717   20.0   19.0     94  15.11    195   17.5  299.8  344.1  302.5
   15.0  28467  -44.9  -74.9      2   0.11                757.8  758.9  757.8
";

    #[test]
    fn parses_the_fixed_width_table() {
        let levels = parse_table(OUN);
        assert_eq!(levels.len(), 5, "four low levels plus the windless top one");
        assert_eq!(levels[0].pressure_hpa, 966.0);
        assert_eq!(levels[0].temp_c, 21.6);
        assert_eq!(levels[0].dewpt_c, 19.7);
        // 160° at 3.6 m/s is a south-southeasterly: blowing toward the north-northwest, so v > 0.
        assert!(levels[0].v_ms > 0.0, "v was {}", levels[0].v_ms);
        assert!(levels[0].u_ms < 0.0, "u was {}", levels[0].u_ms);
        assert!((levels[0].u_ms.hypot(levels[0].v_ms) - 3.6).abs() < 1e-6);
        // The blank-wind top level survives, as calm.
        let top = levels.last().unwrap();
        assert_eq!(top.pressure_hpa, 15.0);
        assert_eq!((top.u_ms, top.v_ms), (0.0, 0.0));
    }

    /// Splitting this row on whitespace yields 9 fields, and the code would read THTA (757.8) as
    /// the wind direction. Fixed-width reading is the whole reason the parser looks like this.
    #[test]
    fn a_missing_wind_column_is_not_read_as_wind() {
        let levels = parse_table(OUN);
        let top = levels.last().unwrap();
        assert!(top.u_ms.abs() < 1.0 && top.v_ms.abs() < 1.0);
    }

    #[test]
    fn knots_are_converted() {
        let kt = OUN.replace("    deg    m/s", "    deg   knot");
        let ms = parse_table(&kt);
        let speed = ms[0].u_ms.hypot(ms[0].v_ms);
        assert!((speed - 3.6 * 0.514_444).abs() < 1e-6, "got {speed}");
    }

    #[test]
    fn nearest_station_and_synoptic_hour() {
        // Over Oklahoma City, the nearest launch site is Norman.
        let s = nearest_station(-97.5, 35.4).unwrap();
        assert_eq!(s.id, "72357");
        // The middle of the Pacific has none.
        assert!(nearest_station(-150.0, 30.0).is_none());

        let t = |h| Utc.with_ymd_and_hms(2013, 5, 20, h, 30, 0).unwrap();
        assert_eq!(synoptic_before(t(21)).hour(), 12);
        assert_eq!(synoptic_before(t(6)).hour(), 0);
        assert_eq!(synoptic_before(t(12)).hour(), 12);
    }
}
