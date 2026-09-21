//! Provider-neutral road routes, initially backed by a user-configured OSRM endpoint.

const MAX_WAYPOINTS: usize = 32;
const MAX_ROUTES: usize = 3;
const MAX_POINTS: usize = 100_000;

#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    /// Geographic line in GeoJSON order: `[longitude, latitude]`.
    pub points: Vec<[f64; 2]>,
    pub distance_m: f64,
    pub duration_s: f64,
}

/// Distance along a route to its first entry into a polygon, including interior holes.
pub fn first_intersection_m(points: &[[f64; 2]], rings: &[Vec<[f64; 2]>]) -> Option<f64> {
    let outer = rings.first()?;
    let inside = |point: [f64; 2]| {
        crate::overlay::point_in_ring(outer, point[0], point[1])
            && !rings[1..]
                .iter()
                .any(|hole| crate::overlay::point_in_ring(hole, point[0], point[1]))
    };
    let mut traveled = 0.0;
    for segment in points.windows(2) {
        let (a, b) = (segment[0], segment[1]);
        let length = distance_m(a, b);
        let mut cuts = vec![0.0, 1.0];
        for ring in rings {
            for edge in ring.windows(2) {
                if let Some(t) = crossing_fraction(a, b, edge[0], edge[1]) {
                    cuts.push(t);
                }
            }
            if let (Some(&first), Some(&last)) = (ring.first(), ring.last()) {
                if let Some(t) = crossing_fraction(a, b, last, first) {
                    cuts.push(t);
                }
            }
        }
        cuts.sort_by(f64::total_cmp);
        cuts.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
        for pair in cuts.windows(2) {
            let mid = (pair[0] + pair[1]) / 2.0;
            if inside([a[0] + (b[0] - a[0]) * mid, a[1] + (b[1] - a[1]) * mid]) {
                return Some(traveled + length * pair[0]);
            }
        }
        traveled += length;
    }
    None
}

fn crossing_fraction(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> Option<f64> {
    let (rx, ry) = (b[0] - a[0], b[1] - a[1]);
    let (sx, sy) = (d[0] - c[0], d[1] - c[1]);
    let denominator = rx * sy - ry * sx;
    if denominator.abs() < 1e-12 {
        return None;
    }
    let (qx, qy) = (c[0] - a[0], c[1] - a[1]);
    let t = (qx * sy - qy * sx) / denominator;
    let u = (qx * ry - qy * rx) / denominator;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)).then_some(t)
}

fn distance_m(a: [f64; 2], b: [f64; 2]) -> f64 {
    let (lat1, lat2) = (a[1].to_radians(), b[1].to_radians());
    let dlat = lat2 - lat1;
    let dlon = (b[0] - a[0]).to_radians();
    let h = (dlat / 2.0).sin().powi(2)
        + lat1.cos() * lat2.cos() * (dlon / 2.0).sin().powi(2);
    12_742_000.0 * h.sqrt().asin()
}

pub async fn fetch_osrm(
    http: &reqwest::Client,
    endpoint: &str,
    waypoints: &[[f64; 2]],
) -> anyhow::Result<Vec<Route>> {
    let url = osrm_url(endpoint, waypoints)?;
    let response = http
        // User-configured endpoints stay direct: routing can be self-hosted and its URL must not
        // be sent through HookEcho's weather-feed proxy. Browser endpoints therefore need CORS.
        .get(url)
        .timeout(crate::net::FEED_TIMEOUT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    parse_osrm(&response)
}

pub fn osrm_url(endpoint: &str, waypoints: &[[f64; 2]]) -> anyhow::Result<String> {
    let endpoint = endpoint.trim().trim_end_matches('/');
    anyhow::ensure!(
        endpoint.starts_with("https://") || endpoint.starts_with("http://"),
        "route endpoint must be an HTTP(S) URL"
    );
    anyhow::ensure!(
        (2..=MAX_WAYPOINTS).contains(&waypoints.len()),
        "a route needs 2 to {MAX_WAYPOINTS} waypoints"
    );
    anyhow::ensure!(
        waypoints
            .iter()
            .all(|[lon, lat]| lon.is_finite()
                && lat.is_finite()
                && (-180.0..=180.0).contains(lon)
                && (-90.0..=90.0).contains(lat)),
        "route contains an invalid coordinate"
    );
    let coordinates = waypoints
        .iter()
        .map(|[lon, lat]| format!("{lon:.6},{lat:.6}"))
        .collect::<Vec<_>>()
        .join(";");
    Ok(format!(
        "{endpoint}/route/v1/driving/{coordinates}?alternatives=2&steps=false&geometries=geojson&overview=full"
    ))
}

#[derive(serde::Deserialize)]
struct OsrmResponse {
    code: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    routes: Vec<OsrmRoute>,
}

#[derive(serde::Deserialize)]
struct OsrmRoute {
    distance: f64,
    duration: f64,
    geometry: OsrmGeometry,
}

#[derive(serde::Deserialize)]
struct OsrmGeometry {
    coordinates: Vec<[f64; 2]>,
}

fn parse_osrm(json: &str) -> anyhow::Result<Vec<Route>> {
    let response: OsrmResponse = serde_json::from_str(json)?;
    anyhow::ensure!(
        response.code == "Ok",
        "route provider returned {}{}",
        response.code,
        if response.message.is_empty() {
            String::new()
        } else {
            format!(": {}", response.message)
        }
    );
    anyhow::ensure!(!response.routes.is_empty(), "route provider returned no routes");
    anyhow::ensure!(response.routes.len() <= MAX_ROUTES, "too many alternate routes");
    response
        .routes
        .into_iter()
        .map(|route| {
            anyhow::ensure!(
                route.distance.is_finite()
                    && route.distance >= 0.0
                    && route.duration.is_finite()
                    && route.duration >= 0.0,
                "route has invalid distance or duration"
            );
            anyhow::ensure!(
                (2..=MAX_POINTS).contains(&route.geometry.coordinates.len())
                    && route.geometry.coordinates.iter().all(|[lon, lat]| {
                        lon.is_finite()
                            && lat.is_finite()
                            && (-180.0..=180.0).contains(lon)
                            && (-90.0..=90.0).contains(lat)
                    }),
                "route has invalid geometry"
            );
            Ok(Route {
                points: route.geometry.coordinates,
                distance_m: route.distance,
                duration_s: route.duration,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_documented_osrm_route_request_and_parses_geojson() {
        let url = osrm_url(
            "https://router.example.test/",
            &[[-97.5164, 35.4676], [-96.7970, 32.7767]],
        )
        .unwrap();
        assert_eq!(
            url,
            "https://router.example.test/route/v1/driving/-97.516400,35.467600;-96.797000,32.776700?alternatives=2&steps=false&geometries=geojson&overview=full"
        );
        let routes = parse_osrm(
            r#"{"code":"Ok","routes":[{"distance":331000.0,"duration":12000.0,"geometry":{"coordinates":[[-97.5164,35.4676],[-96.797,32.7767]]}}]}"#,
        )
        .unwrap();
        assert_eq!(routes[0].points.len(), 2);
        assert_eq!(routes[0].distance_m, 331000.0);
    }

    #[test]
    fn rejects_provider_errors_and_unbounded_input() {
        assert!(osrm_url("file:///tmp/router", &[[0.0, 0.0], [1.0, 1.0]]).is_err());
        assert!(parse_osrm(r#"{"code":"NoRoute","message":"Impossible"}"#)
            .unwrap_err()
            .to_string()
            .contains("NoRoute"));
    }

    #[test]
    fn finds_first_polygon_entry_and_respects_holes() {
        let outer = vec![[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];
        let route = vec![[-2.0, 0.0], [2.0, 0.0]];
        let distance = first_intersection_m(&route, &[outer.clone()]).unwrap();
        assert!((distance / 1000.0 - 111.2).abs() < 1.0);

        let hole = vec![[-2.0, -0.5], [2.0, -0.5], [2.0, 0.5], [-2.0, 0.5]];
        assert!(first_intersection_m(&route, &[outer, hole]).is_none());
    }
}
