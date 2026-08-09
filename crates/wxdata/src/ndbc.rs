//! NDBC moored buoys and coastal stations.
//!
//! The surface-obs layer is airport METARs, which stop at the shoreline — exactly where a
//! lake-effect band or a landfalling squall line is still over water. NDBC's `latest_obs.txt`
//! is one fixed-width file covering every station, so the whole network costs a single request.
//!
//! Buoys decode into [`metar::SurfaceOb`], the same type the station plots already draw, so the
//! wind barbs and decluttering work on them with no renderer changes at all.

use crate::metar::SurfaceOb;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const LATEST: &str = "https://www.ndbc.noaa.gov/data/latest_obs/latest_obs.txt";

/// Column order of `latest_obs.txt`, from its own header:
/// `STN LAT LON YYYY MM DD hh mm WDIR WSPD GST WVHT DPD APD MWD PRES PTDY ATMP WTMP DEWP VIS TIDE`
const COL_LAT: usize = 1;
const COL_LON: usize = 2;
const COL_WDIR: usize = 8;
const COL_WSPD: usize = 9;
const COL_GST: usize = 10;
const COL_WVHT: usize = 11;
const COL_DPD: usize = 12;
const COL_PRES: usize = 15;
const COL_ATMP: usize = 17;
const COL_DEWP: usize = 19;

/// `MM` is NDBC's missing marker.
fn num(fields: &[&str], i: usize) -> Option<f32> {
    fields.get(i).and_then(|s| s.parse::<f32>().ok())
}

/// Parse `latest_obs.txt` into surface observations.
pub fn parse(text: &str) -> Vec<SurfaceOb> {
    let mut out = Vec::new();
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() <= COL_DEWP {
            continue;
        }
        let (Some(lat), Some(lon)) = (num(&f, COL_LAT), num(&f, COL_LON)) else {
            continue;
        };
        // A buoy with no wind and no temperature has nothing to plot.
        let wspd_ms = num(&f, COL_WSPD);
        let temp_c = num(&f, COL_ATMP);
        if wspd_ms.is_none() && temp_c.is_none() {
            continue;
        }
        // Columns 3..8 are the observation's Y M D h m; a station missing any of them keeps
        // its reading and loses only the timestamp.
        let ymdhm = (3..=7).map(|i| num(&f, i)).collect::<Option<Vec<_>>>();
        let obs_time = ymdhm.and_then(|v| {
            chrono::NaiveDate::from_ymd_opt(v[0] as i32, v[1] as u32, v[2] as u32)
                .and_then(|d| d.and_hms_opt(v[3] as u32, v[4] as u32, 0))
                .map(|d| d.and_utc().timestamp())
        });
        out.push(SurfaceOb {
            icao: f[0].to_string(),
            name: format!("NDBC {}", f[0]),
            lat: lat as f64,
            // NDBC writes east longitude positive in −180..180 already, but a few Pacific
            // stations report past 180; wrap so they don't land off the map.
            lon: if lon > 180.0 {
                (lon - 360.0) as f64
            } else {
                lon as f64
            },
            temp_c,
            dewp_c: num(&f, COL_DEWP),
            wdir_deg: num(&f, COL_WDIR),
            // The file is metric; the station plot is in knots like the METARs beside it.
            wspd_kt: wspd_ms.map_or(0.0, |v| v * 1.943_844),
            wgst_kt: num(&f, COL_GST).map(|v| v * 1.943_844),
            altim_mb: num(&f, COL_PRES),
            elev_m: Some(0.0),
            obs_time,
            flt_cat: String::new(),
            // Wave height is metres in the file; feet is what mariners and the forecasts use.
            wvht_ft: num(&f, COL_WVHT).map(|m| m * 3.280_84),
            dpd_s: num(&f, COL_DPD),
            raw: line.trim().to_string(),
        });
    }
    out
}

/// The whole network, cached: one file serves every station, and it updates hourly.
static CACHE: Mutex<Option<(Instant, Vec<SurfaceOb>)>> = Mutex::new(None);

/// Buoy observations within a lat/lon bbox, from a ten-minute cache of the network file.
pub async fn fetch_bbox(
    http: &reqwest::Client,
    lat0: f64,
    lon0: f64,
    lat1: f64,
    lon1: f64,
) -> anyhow::Result<Vec<SurfaceOb>> {
    let fresh = CACHE.lock().ok().and_then(|c| {
        c.as_ref()
            .filter(|(t, _)| t.elapsed() < Duration::from_secs(600))
            .map(|(_, v)| v.clone())
    });
    let all = match fresh {
        Some(v) => v,
        None => {
            let text = http
                .get(crate::net::fetch_url(LATEST))
                .header("User-Agent", crate::alerts::USER_AGENT)
                .send()
                .await?
                .error_for_status()?
                .text()
                .await?;
            let v = parse(&text);
            if let Ok(mut c) = CACHE.lock() {
                *c = Some((Instant::now(), v.clone()));
            }
            v
        }
    };
    let (lat_lo, lat_hi) = (lat0.min(lat1), lat0.max(lat1));
    let (lon_lo, lon_hi) = (lon0.min(lon1), lon0.max(lon1));
    Ok(all
        .into_iter()
        .filter(|o| (lat_lo..=lat_hi).contains(&o.lat) && (lon_lo..=lon_hi).contains(&o.lon))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
#STN       LAT      LON  YYYY MM DD hh mm WDIR WSPD   GST WVHT  DPD APD MWD   PRES  PTDY  ATMP  WTMP  DEWP  VIS   TIDE
#text      deg      deg   yr mo day hr mn degT  m/s   m/s   m   sec sec degT   hPa   hPa  degC  degC  degC  nmi     ft
45007    42.674  -87.026 2026 02 15 17 00 340  10.0  12.0  1.5    4  MM  MM 1013.2    MM  -3.0  -1.0  -8.0   MM     MM
41001    34.724  -72.317 2026 02 15 17 00  MM    MM    MM   MM   MM  MM  MM     MM    MM    MM    MM    MM   MM     MM
51000    23.535 -153.792 2026 02 15 17 00 070   6.0    MM  2.0    9  MM  MM 1017.0    MM  24.0  25.0    MM   MM     MM";

    #[test]
    fn buoys_decode_into_surface_obs() {
        let obs = parse(SAMPLE);
        assert_eq!(obs.len(), 2, "the all-missing station is skipped");
        let lake = &obs[0];
        assert_eq!(lake.icao, "45007");
        // The file's own precision is f32, so compare to it and not to an f64 literal.
        assert!((lake.lat - 42.674).abs() < 1e-4 && (lake.lon - -87.026).abs() < 1e-4);
        assert_eq!(lake.wdir_deg, Some(340.0));
        // 1.5 m of sea, 4-second period.
        assert!((lake.wvht_ft.unwrap() - 4.921_26).abs() < 1e-3);
        assert_eq!(lake.dpd_s, Some(4.0));
        // 10 m/s is 19.4 kt — the station plot is in knots.
        assert!((lake.wspd_kt - 19.438_44).abs() < 1e-3);
        assert!((lake.wgst_kt.unwrap() - 23.326_13).abs() < 1e-3);
        assert_eq!(lake.temp_c, Some(-3.0));
        assert_eq!(lake.dewp_c, Some(-8.0));
        assert_eq!(lake.altim_mb, Some(1013.2));
        assert_eq!(
            lake.obs_time,
            Some(
                chrono::NaiveDate::from_ymd_opt(2026, 2, 15)
                    .unwrap()
                    .and_hms_opt(17, 0, 0)
                    .unwrap()
                    .and_utc()
                    .timestamp()
            )
        );
        // Missing fields stay missing rather than becoming zero.
        assert_eq!(obs[1].dewp_c, None);
    }

    #[tokio::test]
    #[ignore = "network"]
    async fn fetches_lake_michigan() {
        let http = reqwest::Client::new();
        let obs = fetch_bbox(&http, 41.0, -88.5, 46.0, -85.0).await.unwrap();
        assert!(!obs.is_empty(), "Lake Michigan has buoys");
        assert!(obs.iter().all(|o| o.lat >= 41.0 && o.lat <= 46.0));
    }
}
