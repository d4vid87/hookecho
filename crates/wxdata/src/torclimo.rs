//! Historical tornado climatology from the SPC "Severe Weather Database" tornado tracks CSV
//! (1950–present, one row per tornado). Used to answer "has a tornado ever hit near here?".
//!
//! Source: <https://www.spc.noaa.gov/wcm/data/1950-2022_actual_tornadoes.csv>. Columns of
//! interest: `yr`, `mag` (F/EF scale, −9 = unknown), `slat`/`slon` (start), `elat`/`elon` (end,
//! `0` when not recorded), `len` (path length, mi), `wid` (width, yd).

const TRACKS_URL: &str = "https://www.spc.noaa.gov/wcm/data/1950-2022_actual_tornadoes.csv";

/// One historical tornado track.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TornadoTrack {
    pub year: i32,
    /// F/EF scale 0–5, or `-1` when unknown (the source uses `-9`).
    pub mag: i8,
    pub slat: f64,
    pub slon: f64,
    /// End point; equals the start when the source recorded no path (stored `0,0`).
    pub elat: f64,
    pub elon: f64,
}

/// Fetch the full tornado-track database (~7.6 MB CSV, tens of thousands of rows).
pub async fn fetch_tracks(client: &reqwest::Client) -> anyhow::Result<Vec<TornadoTrack>> {
    let body = client
        .get(TRACKS_URL)
        .header("User-Agent", crate::alerts::USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(parse_tracks(&body))
}

/// Parse the SPC tornado CSV. Column order is fixed; the header row is skipped. Rows with an
/// unparseable start latitude/longitude are dropped.
pub fn parse_tracks(csv: &str) -> Vec<TornadoTrack> {
    // Header: om,yr,mo,dy,date,time,tz,st,stf,stn,mag,inj,fat,loss,closs,slat,slon,elat,elon,...
    const YR: usize = 1;
    const MAG: usize = 10;
    const SLAT: usize = 15;
    const SLON: usize = 16;
    const ELAT: usize = 17;
    const ELON: usize = 18;
    let mut out = Vec::new();
    for (i, line) in csv.lines().enumerate() {
        if i == 0 || line.is_empty() {
            continue; // header / blank
        }
        let f: Vec<&str> = line.split(',').collect();
        if f.len() <= ELON {
            continue;
        }
        let (Ok(slat), Ok(slon)) = (f[SLAT].trim().parse::<f64>(), f[SLON].trim().parse::<f64>()) else {
            continue;
        };
        if slat == 0.0 && slon == 0.0 {
            continue; // no usable location
        }
        let year = f[YR].trim().parse::<i32>().unwrap_or(0);
        let mag = match f[MAG].trim().parse::<i8>() {
            Ok(m) if (0..=5).contains(&m) => m,
            _ => -1, // -9 unknown, or garbage
        };
        let elat = f[ELAT].trim().parse::<f64>().unwrap_or(0.0);
        let elon = f[ELON].trim().parse::<f64>().unwrap_or(0.0);
        // Fall back to the start point when no path end was recorded.
        let (elat, elon) = if elat == 0.0 && elon == 0.0 { (slat, slon) } else { (elat, elon) };
        out.push(TornadoTrack { year, mag, slat, slon, elat, elon });
    }
    out
}

/// Great-circle distance in km between two lat/lon points (haversine).
fn haversine_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let (dp, dl) = ((lat2 - lat1).to_radians(), (lon2 - lon1).to_radians());
    let a = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    6371.0 * 2.0 * a.sqrt().asin()
}

/// Tracks whose start or end point lies within `radius_km` of `(lon, lat)`, strongest first.
pub fn near(tracks: &[TornadoTrack], lon: f64, lat: f64, radius_km: f64) -> Vec<TornadoTrack> {
    let mut hits: Vec<TornadoTrack> = tracks
        .iter()
        .filter(|t| {
            haversine_km(lat, lon, t.slat, t.slon) <= radius_km
                || haversine_km(lat, lon, t.elat, t.elon) <= radius_km
        })
        .copied()
        .collect();
    hits.sort_by(|a, b| b.mag.cmp(&a.mag).then(b.year.cmp(&a.year)));
    hits
}

/// Counts by magnitude bucket: index 0–5 = F/EF0–5, index 6 = unknown.
pub fn mag_histogram(hits: &[TornadoTrack]) -> [usize; 7] {
    let mut h = [0usize; 7];
    for t in hits {
        let idx = if (0..=5).contains(&t.mag) { t.mag as usize } else { 6 };
        h[idx] += 1;
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "om,yr,mo,dy,date,time,tz,st,stf,stn,mag,inj,fat,loss,closs,slat,slon,elat,elon,len,wid\n\
        1,2011,4,27,2011-04-27,,3,AL,1,0,4,10,5,0,0,33.10,-87.50,33.40,-87.10,20,500\n\
        2,1999,5,3,1999-05-03,,3,OK,40,0,5,100,36,0,0,35.10,-97.50,0,0,10,300\n\
        3,2020,3,3,2020-03-03,,3,TN,47,0,-9,0,0,0,0,36.10,-86.70,36.2,-86.6,5,100\n\
        4,1974,4,3,1974-04-03,,3,OH,39,0,5,0,0,0,0,40.00,-84.00,40.10,-83.90,15,400\n";

    #[test]
    fn parses_and_queries_tracks() {
        let t = parse_tracks(SAMPLE);
        assert_eq!(t.len(), 4);
        // Row 2 had no end point (0,0) → end falls back to start.
        let ok = t.iter().find(|x| x.year == 1999).unwrap();
        assert_eq!((ok.elat, ok.elon), (ok.slat, ok.slon));
        // Row 3 mag -9 → unknown (-1).
        assert_eq!(t.iter().find(|x| x.year == 2020).unwrap().mag, -1);

        // Near the AL 2011 start (33.1,-87.5): only that track within 50 km.
        let hits = near(&t, -87.5, 33.1, 50.0);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].year, 2011);

        let hist = mag_histogram(&t);
        assert_eq!(hist[5], 2, "two F5/EF5 tornadoes");
        assert_eq!(hist[6], 1, "one unknown-magnitude");
    }
}
