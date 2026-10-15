//! Point values for NOAA's NDFD graphical forecast rasters.
//!
//! The WMS layers advertise `queryable=0`. NOAA's own graphical viewer instead asks its
//! raster-point endpoint for the selected element and VTIT, so a map tap can use the same data
//! path without guessing a value from a rendered color.

use crate::alerts::USER_AGENT;
use chrono::{DateTime, Utc};

const POINT_URL: &str = "https://digital.weather.gov/scripts/get_rasterdata_xml.php";

fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let marker = format!("{name}=\"");
    tag.split(&marker).nth(1)?.split_once('"').map(|(value, _)| value)
}

pub fn parse_point(xml: &str, requested_hour: &str) -> anyhow::Result<Option<f64>> {
    let info = xml
        .split("<info ")
        .nth(1)
        .and_then(|tail| tail.split_once('>'))
        .map(|(tag, _)| tag)
        .ok_or_else(|| anyhow::anyhow!("NDFD point response has no info element"))?;
    anyhow::ensure!(attribute(info, "hour") == Some(requested_hour), "NDFD point response time does not match the selected forecast");
    let value = attribute(info, "gridval")
        .ok_or_else(|| anyhow::anyhow!("NDFD point response has no grid value"))?;
    let Ok(value) = value.parse::<f64>() else {
        return Ok(None);
    };
    Ok((value.is_finite() && value.abs() < 9999.0).then_some(value))
}

pub async fn fetch_point(
    client: &reqwest::Client,
    lon: f64,
    lat: f64,
    valid: DateTime<Utc>,
    region: &str,
    element: &str,
) -> anyhow::Result<Option<f64>> {
    anyhow::ensure!(matches!(region, "conus" | "alaska" | "hawaii"), "unsupported NDFD region");
    anyhow::ensure!(matches!(element, "maxt" | "mint" | "pop12" | "windgust"), "unsupported NDFD element");
    anyhow::ensure!(lon.is_finite() && lat.is_finite() && (-180.0..=180.0).contains(&lon) && (-85.0..=85.0).contains(&lat), "invalid point");
    let projected_lon = 6_378_137.0 * lon.to_radians();
    let projected_lat = 6_378_137.0
        * (std::f64::consts::FRAC_PI_4 + lat.to_radians() / 2.0)
            .tan()
            .ln();
    let hour = valid.format("%Y-%m-%d %H:%M:%S").to_string();
    let body = client
        .get(crate::net::fetch_url(POINT_URL))
        .timeout(crate::net::FEED_TIMEOUT)
        .query(&[
            ("grid", "ndfd"),
            ("region", region),
            ("lat", &format!("{projected_lat:.3}")),
            ("lon", &format!("{projected_lon:.3}")),
            ("latdeg", &format!("{lat:.4}")),
            ("londeg", &format!("{lon:.4}")),
            ("z", "7"),
            ("hour", &hour),
            ("elmt", element),
            ("ms", "english"),
        ])
        .header("User-Agent", USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    parse_point(&body, &hour)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_noaa_raster_point_and_missing_values() {
        let hour = "2026-10-16 00:00:00";
        assert_eq!(parse_point(r#"<stationinfo><info id="NOID" hour="2026-10-16 00:00:00" gridval="87" gridunit="" /></stationinfo>"#, hour).unwrap(), Some(87.0));
        assert_eq!(parse_point(r#"<stationinfo><info hour="2026-10-16 00:00:00" gridval="Not available" /></stationinfo>"#, hour).unwrap(), None);
        assert_eq!(parse_point(r#"<stationinfo><info hour="2026-10-16 00:00:00" gridval="9999" /></stationinfo>"#, hour).unwrap(), None);
        assert!(parse_point(r#"<stationinfo><info hour="2026-10-15 00:00:00" gridval="87" /></stationinfo>"#, hour).is_err());
        assert!(parse_point("<stationinfo/>", hour).is_err());
    }
}
