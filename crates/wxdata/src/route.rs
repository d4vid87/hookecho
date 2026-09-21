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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StormRouteAnalysis {
    pub closest: ClosestApproach,
    pub intersection: Option<RouteIntersection>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClosestApproach {
    pub vehicle_point: [f64; 2],
    pub storm_point: [f64; 2],
    pub separation_m: f64,
    pub eta_s: f64,
    /// Storm bearing relative to vehicle heading, -180° left through +180° right.
    pub relative_bearing_deg: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RouteIntersection {
    pub point: [f64; 2],
    pub route_distance_m: f64,
    pub vehicle_eta_s: f64,
    pub storm_eta_s: f64,
}

/// Convert distances along a route into vehicle arrival times using the provider ETA.
pub fn arrival_window(
    route: &Route,
    start_m: f64,
    end_m: f64,
    departure: chrono::DateTime<chrono::Utc>,
) -> Option<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)> {
    if route.distance_m <= 0.0
        || route.duration_s <= 0.0
        || !start_m.is_finite()
        || !end_m.is_finite()
    {
        return None;
    }
    let at = |distance: f64| {
        departure
            + chrono::Duration::milliseconds(
                (route.duration_s * distance.clamp(0.0, route.distance_m) / route.distance_m
                    * 1_000.0)
                    .round() as i64,
            )
    };
    Some((at(start_m.min(end_m)), at(start_m.max(end_m))))
}

/// Compare a provider route with a constant-motion storm forecast for up to `horizon_s`.
pub fn analyze_storm_route(
    route: &Route,
    storm_origin: [f64; 2],
    storm_bearing_deg: f64,
    storm_speed_kt: f64,
    horizon_s: f64,
) -> Option<StormRouteAnalysis> {
    if route.points.len() < 2
        || route.duration_s <= 0.0
        || !storm_bearing_deg.is_finite()
        || storm_speed_kt <= 0.0
        || horizon_s <= 0.0
    {
        return None;
    }
    let route_length: f64 = route
        .points
        .windows(2)
        .map(|segment| distance_m(segment[0], segment[1]))
        .sum();
    if route_length <= 0.0 {
        return None;
    }
    let profile = sample_profile(&route.points, 1_000.0, |lon, lat| Some([lon, lat]));
    let mut closest = None;
    for (index, (route_distance, vehicle_point)) in profile.iter().enumerate() {
        let eta_s = route.duration_s * route_distance / route_length;
        if eta_s > horizon_s {
            break;
        }
        let storm_point = destination(
            storm_origin,
            storm_bearing_deg,
            storm_speed_kt * 0.514_444 * eta_s / 1_000.0,
        );
        let separation_m = distance_m(*vehicle_point, storm_point);
        let neighbor = profile
            .get(index + 1)
            .or_else(|| index.checked_sub(1).and_then(|i| profile.get(i)))?
            .1;
        let vehicle_heading = bearing_deg(*vehicle_point, neighbor);
        let relative_bearing_deg = normalize_bearing(
            bearing_deg(*vehicle_point, storm_point) - vehicle_heading,
        );
        let candidate = ClosestApproach {
            vehicle_point: *vehicle_point,
            storm_point,
            separation_m,
            eta_s,
            relative_bearing_deg,
        };
        if closest
            .as_ref()
            .is_none_or(|best: &ClosestApproach| separation_m < best.separation_m)
        {
            closest = Some(candidate);
        }
    }

    let storm_end = destination(
        storm_origin,
        storm_bearing_deg,
        storm_speed_kt * 0.514_444 * horizon_s / 1_000.0,
    );
    let mut traveled = 0.0;
    let mut intersection = None;
    for segment in route.points.windows(2) {
        let length = distance_m(segment[0], segment[1]);
        if let Some((route_fraction, storm_fraction)) =
            crossing_fractions(segment[0], segment[1], storm_origin, storm_end)
        {
            let route_distance_m = traveled + length * route_fraction;
            intersection = Some(RouteIntersection {
                point: [
                    segment[0][0] + (segment[1][0] - segment[0][0]) * route_fraction,
                    segment[0][1] + (segment[1][1] - segment[0][1]) * route_fraction,
                ],
                route_distance_m,
                vehicle_eta_s: route.duration_s * route_distance_m / route_length,
                storm_eta_s: horizon_s * storm_fraction,
            });
            break;
        }
        traveled += length;
    }
    Some(StormRouteAnalysis {
        closest: closest?,
        intersection,
    })
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
    crossing_fractions(a, b, c, d).map(|(t, _)| t)
}

fn crossing_fractions(
    a: [f64; 2],
    b: [f64; 2],
    c: [f64; 2],
    d: [f64; 2],
) -> Option<(f64, f64)> {
    let (rx, ry) = (b[0] - a[0], b[1] - a[1]);
    let (sx, sy) = (d[0] - c[0], d[1] - c[1]);
    let denominator = rx * sy - ry * sx;
    if denominator.abs() < 1e-12 {
        return None;
    }
    let (qx, qy) = (c[0] - a[0], c[1] - a[1]);
    let t = (qx * sy - qy * sx) / denominator;
    let u = (qx * ry - qy * rx) / denominator;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)).then_some((t, u))
}

fn distance_m(a: [f64; 2], b: [f64; 2]) -> f64 {
    let (lat1, lat2) = (a[1].to_radians(), b[1].to_radians());
    let dlat = lat2 - lat1;
    let dlon = (b[0] - a[0]).to_radians();
    let h = (dlat / 2.0).sin().powi(2)
        + lat1.cos() * lat2.cos() * (dlon / 2.0).sin().powi(2);
    12_742_000.0 * h.sqrt().asin()
}

fn bearing_deg(a: [f64; 2], b: [f64; 2]) -> f64 {
    let (lat1, lat2) = (a[1].to_radians(), b[1].to_radians());
    let dlon = (b[0] - a[0]).to_radians();
    (dlon.sin() * lat2.cos())
        .atan2(lat1.cos() * lat2.sin() - lat1.sin() * lat2.cos() * dlon.cos())
        .to_degrees()
        .rem_euclid(360.0)
}

fn normalize_bearing(value: f64) -> f64 {
    (value + 180.0).rem_euclid(360.0) - 180.0
}

fn destination(origin: [f64; 2], bearing_deg: f64, distance_km: f64) -> [f64; 2] {
    let angular = distance_km / 6_371.0;
    let bearing = bearing_deg.to_radians();
    let (lon, lat) = (origin[0].to_radians(), origin[1].to_radians());
    let out_lat = (lat.sin() * angular.cos()
        + lat.cos() * angular.sin() * bearing.cos())
    .asin();
    let out_lon = lon
        + (bearing.sin() * angular.sin() * lat.cos())
            .atan2(angular.cos() - lat.sin() * out_lat.sin());
    [
        normalize_bearing(out_lon.to_degrees()),
        out_lat.to_degrees(),
    ]
}

/// Sample a route at a bounded spacing while retaining distance from its start.
pub fn sample_profile<T>(
    points: &[[f64; 2]],
    spacing_m: f64,
    mut sample: impl FnMut(f64, f64) -> Option<T>,
) -> Vec<(f64, T)> {
    if points.len() < 2 || !spacing_m.is_finite() || spacing_m < 100.0 {
        return Vec::new();
    }
    let mut output = Vec::new();
    let mut traveled = 0.0;
    for segment in points.windows(2) {
        let (a, b) = (segment[0], segment[1]);
        let length = distance_m(a, b);
        let steps = ((length / spacing_m).ceil() as usize).max(1);
        for index in 0..steps {
            let t = index as f64 / steps as f64;
            let point = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
            if let Some(value) = sample(point[0], point[1]) {
                output.push((traveled + length * t, value));
            }
            if output.len() >= MAX_POINTS {
                return output;
            }
        }
        traveled += length;
    }
    let last = *points.last().expect("at least two points");
    if let Some(value) = sample(last[0], last[1]) {
        output.push((traveled, value));
    }
    output
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
        let distance = first_intersection_m(&route, std::slice::from_ref(&outer)).unwrap();
        assert!((distance / 1000.0 - 111.2).abs() < 1.0);

        let hole = vec![[-2.0, -0.5], [2.0, -0.5], [2.0, 0.5], [-2.0, 0.5]];
        assert!(first_intersection_m(&route, &[outer, hole]).is_none());
    }

    #[test]
    fn samples_route_at_bounded_physical_spacing() {
        let samples = sample_profile(&[[0.0, 0.0], [0.1, 0.0]], 1_000.0, |lon, _| Some(lon));
        assert!((11..=14).contains(&samples.len()), "{}", samples.len());
        assert_eq!(samples.last().unwrap().1, 0.1);
        assert!(samples.windows(2).all(|pair| pair[0].0 <= pair[1].0));
    }

    #[test]
    fn compares_vehicle_and_storm_arrival_at_route_crossing() {
        let route = Route {
            points: vec![[-0.1, 0.0], [0.1, 0.0]],
            distance_m: 22_239.0,
            duration_s: 3_600.0,
        };
        let analysis = analyze_storm_route(&route, [0.0, -0.1], 0.0, 12.0, 7_200.0).unwrap();
        let crossing = analysis.intersection.unwrap();
        assert!(crossing.point[0].abs() < 0.001 && crossing.point[1].abs() < 0.001);
        assert!((crossing.vehicle_eta_s - 1_800.0).abs() < 30.0);
        assert!((crossing.storm_eta_s - 1_800.0).abs() < 60.0);
        assert!(
            analysis.closest.separation_m < 750.0,
            "{} m",
            analysis.closest.separation_m
        );
    }

    #[test]
    fn maps_route_distance_to_provider_eta() {
        let route = Route {
            points: vec![[0.0, 0.0], [1.0, 0.0]],
            distance_m: 100_000.0,
            duration_s: 7_200.0,
        };
        let departure = "2026-05-01T20:00:00Z".parse().unwrap();
        let (start, end) = arrival_window(&route, 25_000.0, 50_000.0, departure).unwrap();
        assert_eq!(start.to_rfc3339(), "2026-05-01T20:30:00+00:00");
        assert_eq!(end.to_rfc3339(), "2026-05-01T21:00:00+00:00");
    }
}
