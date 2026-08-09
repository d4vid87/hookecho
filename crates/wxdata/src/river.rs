//! River flood gauges from NOAA's National Water Prediction Service (NWPS), for chase-relevant
//! flood awareness. A bounding-box query returns the gauges in view; each becomes a [`Gauge`] the
//! app draws as a flood-category colored droplet with a stage / forecast tooltip.
//!
//! Endpoint (no API key): `GET .../nwps/v1/gauges?bbox.xmin=<lon0>&bbox.ymin=<lat0>&
//! bbox.xmax=<lon1>&bbox.ymax=<lat1>&srid=EPSG_4326`. Stage/forecast come back as `-999` when
//! missing; flood category is a string (`action|minor|moderate|major|no_flooding|not_defined|
//! obs_not_current|fcst_not_current`).

use crate::alerts::USER_AGENT;

const GAUGES_URL: &str = "https://api.water.noaa.gov/nwps/v1/gauges";

/// NWS flood severity for a gauge. Anything not-defined / stale maps to [`FloodCat::Unknown`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloodCat {
    Major,
    Moderate,
    Minor,
    Action,
    NoFlooding,
    Unknown,
}

impl FloodCat {
    fn from_str(s: &str) -> Self {
        match s {
            "major" => FloodCat::Major,
            "moderate" => FloodCat::Moderate,
            "minor" => FloodCat::Minor,
            "action" => FloodCat::Action,
            "no_flooding" => FloodCat::NoFlooding,
            _ => FloodCat::Unknown, // not_defined, obs_not_current, fcst_not_current, ...
        }
    }
    /// Major-first sort key (0 = worst) so the app can prioritize / draw serious flooding on top.
    fn severity(self) -> u8 {
        match self {
            FloodCat::Major => 0,
            FloodCat::Moderate => 1,
            FloodCat::Minor => 2,
            FloodCat::Action => 3,
            FloodCat::NoFlooding => 4,
            FloodCat::Unknown => 5,
        }
    }
}

/// One river gauge with its current observed stage and (when available) forecast crest.
#[derive(Debug, Clone, PartialEq)]
pub struct Gauge {
    pub lid: String,
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    pub cat: FloodCat,
    pub stage_ft: Option<f64>,
    pub forecast_ft: Option<f64>,
    pub forecast_cat: FloodCat,
}

/// `-999` is NWPS's missing sentinel; treat it (and any negative) as no reading.
fn stage(v: Option<f64>) -> Option<f64> {
    v.filter(|f| *f > -900.0)
}

/// Parse an NWPS `gauges` JSON payload. Tolerant: skips entries missing lat/lon.
pub fn parse(json: &str) -> Vec<Gauge> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(arr) = v.get("gauges").and_then(|g| g.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for g in arr {
        let (Some(lat), Some(lon)) = (num(g, "latitude"), num(g, "longitude")) else {
            continue;
        };
        let obs = g.get("status").and_then(|s| s.get("observed"));
        let fcst = g.get("status").and_then(|s| s.get("forecast"));
        out.push(Gauge {
            lid: str_of(g, "lid"),
            name: str_of(g, "name"),
            lat,
            lon,
            cat: cat_of(obs),
            stage_ft: stage(obs.and_then(|o| num(o, "primary"))),
            forecast_ft: stage(fcst.and_then(|f| num(f, "primary"))),
            forecast_cat: cat_of(fcst),
        });
    }
    out
}

fn cat_of(node: Option<&serde_json::Value>) -> FloodCat {
    node.and_then(|n| n.get("floodCategory"))
        .and_then(|c| c.as_str())
        .map_or(FloodCat::Unknown, FloodCat::from_str)
}

fn str_of(m: &serde_json::Value, k: &str) -> String {
    m.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

fn num(m: &serde_json::Value, k: &str) -> Option<f64> {
    m.get(k).and_then(|v| v.as_f64())
}

/// Fetch the gauges within a lat/lon bounding box `(lat0, lon0, lat1, lon1)`, sorted worst-first
/// and capped at 300. `USER_AGENT` identifies the app to NOAA.
pub async fn fetch_bbox(
    client: &reqwest::Client,
    lat0: f64,
    lon0: f64,
    lat1: f64,
    lon1: f64,
) -> anyhow::Result<Vec<Gauge>> {
    let body = client
        .get(crate::net::fetch_url(GAUGES_URL))
        .query(&[
            ("bbox.xmin", lon0.to_string()),
            ("bbox.ymin", lat0.to_string()),
            ("bbox.xmax", lon1.to_string()),
            ("bbox.ymax", lat1.to_string()),
            ("srid", "EPSG_4326".to_string()),
        ])
        .header("User-Agent", USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let mut gauges = parse(&body);
    gauges.sort_by_key(|g| g.cat.severity());
    gauges.truncate(300);
    Ok(gauges)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_drops_missing_sentinels() {
        // One flooding gauge with a real stage + forecast, one stale gauge with -999 sentinels.
        let json = r#"{"gauges":[
            {"lid":"MAJT2","name":"Big River","latitude":33.1,"longitude":-97.2,
             "status":{"observed":{"primary":28.4,"floodCategory":"major"},
                       "forecast":{"primary":31.0,"floodCategory":"moderate"}}},
            {"lid":"DRYT2","name":"Dry Creek","latitude":32.5,"longitude":-96.8,
             "status":{"observed":{"primary":-999,"floodCategory":"obs_not_current"},
                       "forecast":{"primary":-999,"floodCategory":"fcst_not_current"}}}
        ]}"#;
        let g = parse(json);
        assert_eq!(g.len(), 2);
        // Find them by id (parse order is preserved; fetch_bbox is what sorts).
        let maj = g.iter().find(|x| x.lid == "MAJT2").unwrap();
        assert_eq!(maj.cat, FloodCat::Major);
        assert_eq!(maj.stage_ft, Some(28.4));
        assert_eq!(maj.forecast_ft, Some(31.0));
        assert_eq!(maj.forecast_cat, FloodCat::Moderate);
        let dry = g.iter().find(|x| x.lid == "DRYT2").unwrap();
        assert_eq!(dry.cat, FloodCat::Unknown);
        assert_eq!(dry.stage_ft, None);
        assert_eq!(dry.forecast_ft, None);
    }

    // Live network check (NWPS is public; be gentle).
    #[tokio::test]
    #[ignore = "network"]
    async fn fetches_real_gauges() {
        let client = reqwest::Client::new();
        let g = fetch_bbox(&client, 32.0, -98.5, 33.5, -96.5).await.unwrap();
        eprintln!("{} gauges; first: {:?}", g.len(), g.first());
        assert!(!g.is_empty());
    }
}
