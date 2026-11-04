//! Saved-place cards reuse the live alert feed and the point-forecast cache.
use super::*;
use crate::ui::marker_window::{ForecastKey, PlaceBriefing};

fn forecast_key(lat: f64, lon: f64) -> Option<ForecastKey> {
    (lat.is_finite() && lon.is_finite() && (-90.0..=90.0).contains(&lat)
        && (-180.0..=180.0).contains(&lon))
        .then(|| ((lat * 20.0).round() as i32, (lon * 20.0).round() as i32))
}

fn place_alerts(features: &[GeoFeature], lon: f64, lat: f64, now: chrono::DateTime<Utc>) -> Vec<String> {
    let mut alerts: Vec<_> = features.iter().filter_map(|feature| {
        let alert = feature.alert.as_ref()?;
        // An alert without an expiry cannot establish that it is currently in force.
        (alert.expires.is_some_and(|expiry| expiry > now) && feature.contains(lon, lat)).then_some(alert)
    }).collect();
    alerts.sort_by_key(|alert| std::cmp::Reverse(crate::ui::alert_panel::severity_rank(&alert.event)));
    let mut seen = std::collections::HashSet::new();
    alerts.into_iter().filter(|alert| seen.insert(alert.dedupe_key()))
        .map(|alert| alert.event.clone()).collect()
}

fn forecast_summary(forecast: &wxdata::forecast::PointForecast, now: chrono::DateTime<Utc>, unit: crate::settings::TempUnit) -> (String, String) {
    let current = forecast.hourly.iter().filter(|period| period.start <= now
        && period.start > now - chrono::Duration::hours(1)).max_by_key(|period| period.start)
        .or_else(|| forecast.hourly.iter().filter(|period| period.start > now).min_by_key(|period| period.start))
        .or_else(|| forecast.daily.iter().filter(|period| period.start <= now
            && period.start > now - chrono::Duration::hours(12)).max_by_key(|period| period.start))
        .or_else(|| forecast.daily.iter().filter(|period| period.start > now).min_by_key(|period| period.start));
    let Some(period) = current else { return ("Forecast unavailable for the current period".into(), forecast.office.clone()); };
    let temp = unit.from_c((period.temp_f - 32.0) * 5.0 / 9.0);
    let rain = period.precip_pct.map_or_else(String::new, |chance| format!(" · {chance}% precipitation"));
    let condition = if period.short.trim().is_empty() { String::new() } else { format!("{} · ", period.short) };
    let temperature = if temp.is_finite() { format!("{temp:.0}{}", unit.label()) } else { "temperature unavailable".into() };
    (format!("Forecast: {condition}{temperature}{rain}"),
        format!("{} · valid {} UTC", forecast.office, period.start.format("%b %-d %H:%M")))
}

impl HookEchoApp {
    pub(in crate::app) fn refresh_place_briefings(&mut self, ctx: &egui::Context) {
        let result = self.marker_window.forecast_pending.as_ref().and_then(|(key, rx)| {
            match rx.try_recv() {
                Ok(result) => Some((*key, result)),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Some((*key, Err("Forecast request ended; retrying shortly".into()))),
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
            }
        });
        if let Some((key, result)) = result {
            self.marker_window.forecast_pending = None;
            self.marker_window.briefings_at = None;
            match result {
                Ok(forecast) => {
                    self.forecast_cache.insert(key, (Instant::now(), forecast));
                    self.marker_window.forecast_errors.remove(&key);
                }
                Err(error) => { self.marker_window.forecast_errors.insert(key, (Instant::now(), error)); }
            }
        }
        if !self.marker_window.open { return; }
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
        if self.marker_window.briefings_at.is_some_and(|at| at.elapsed().as_secs() < 1) { return; }
        self.marker_window.briefings_at = Some(Instant::now());
        let keys: std::collections::HashSet<_> = self.settings.markers.iter()
            .filter_map(|marker| forecast_key(marker.lat, marker.lon)).collect();
        self.marker_window.forecast_errors.retain(|key, _| keys.contains(key));
        if self.marker_window.forecast_pending.is_none() {
            let next = self.settings.markers.iter().find_map(|marker| {
                let key = forecast_key(marker.lat, marker.lon)?;
                let fresh = self.forecast_cache.get(&key).is_some_and(|(at, _)| at.elapsed().as_secs() < 900);
                let backed_off = self.marker_window.forecast_errors.get(&key).is_some_and(|(at, _)| at.elapsed().as_secs() < 300);
                (!fresh && !backed_off).then_some((key, marker.lat, marker.lon))
            });
            if let Some((key, lat, lon)) = next {
                let (tx, rx) = std::sync::mpsc::channel();
                self.marker_window.forecast_pending = Some((key, rx));
                let http = self.http.clone();
                let repaint = ctx.clone();
                self.spawner.spawn(async move {
                    let result = wxdata::forecast::fetch_basic(&http, lat, lon).await.map_err(|e| e.to_string());
                    let _ = tx.send(result);
                    repaint.request_repaint();
                });
            }
        }
        let now = Utc::now();
        let health = self.request_health(RequestLane::Feed("Weather alerts"));
        let feed_status = match health.last_success {
            Some(age) => format!("Loaded alert feed checked {} ago{}", humanize(age.as_secs() as i64),
                if age > health.freshness_window || health.error.is_some() { " · update unavailable" } else { "" }),
            None => "Current alert feed unavailable".into(),
        };
        let mut briefings = std::collections::HashMap::new();
        for marker in &self.settings.markers {
            let Some(key) = forecast_key(marker.lat, marker.lon) else { continue; };
            let alerts = place_alerts(&self.alert_features, marker.lon, marker.lat, now);
            let alerts_status = if alerts.is_empty() {
                format!("No matching alerts in loaded feed · {feed_status}")
            } else { feed_status.clone() };
            let radar = self.views.iter().filter_map(|view| {
                let site_id = view.displayed_station()?;
                let site = wxdata::sites::site_by_id(site_id)?;
                let time = view.displayed_time()?;
                let distance = crate::geo::great_circle([marker.lon, marker.lat], [site.longitude as f64, site.latitude as f64]).0;
                (distance <= 460.0).then_some((distance, site_id, time, view.timeline.following))
            }).min_by(|a, b| a.0.total_cmp(&b.0)).map_or_else(|| "No nearby radar loaded · open this place on the map".into(),
                |(distance, site, time, following)| format!("{site} · {}scan {} · {:.0} km from place", if following { "" } else { "historical " },
                    if time > now { "time is ahead of clock".into() } else { format!("{} ago", humanize((now - time).num_seconds())) }, distance));
            let (forecast, forecast_source) = match self.forecast_cache.get(&key) {
                Some((fetched, forecast)) => {
                    let (summary, source) = forecast_summary(forecast, now, self.settings.temp_unit);
                    let age = fetched.elapsed().as_secs();
                    let update = if age < 900 { "" } else if self.marker_window.forecast_errors.contains_key(&key) { " · update unavailable" } else { " · refresh queued" };
                    (summary, format!("{source} · retrieved {} ago{update}", humanize(age as i64)))
                }
                None if self.marker_window.forecast_errors.contains_key(&key) => ("Forecast temporarily unavailable".into(), "Automatic retry in a few minutes · Forecast details to retry now".into()),
                None => ("Loading forecast…".into(), "Forecasts load one place at a time".into()),
            };
            briefings.insert(marker.id.clone(), PlaceBriefing { alerts, alerts_status, radar, forecast, forecast_source });
        }
        self.marker_window.briefings = briefings;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_places_and_expired_forecasts_do_not_look_current() {
        assert_eq!(forecast_key(f64::NAN, 1.0), None);
        assert_eq!(forecast_key(91.0, 1.0), None);
        assert_eq!(forecast_key(32.0, -97.0), Some((640, -1940)));
        let now = Utc::now();
        let mut forecast = wxdata::forecast::PointForecast::default();
        assert!(forecast_summary(&forecast, now, crate::settings::TempUnit::Fahrenheit).0.contains("unavailable"));
        let period = wxdata::forecast::Period {
            start: now - chrono::Duration::hours(2), name: String::new(), temp_f: 68.,
            precip_pct: Some(20), short: "Sunny".into(), wind: String::new(),
            wind_mph: None, wind_deg: None, is_day: true,
        };
        forecast.hourly.push(period);
        assert!(forecast_summary(&forecast, now, crate::settings::TempUnit::Fahrenheit).0.contains("unavailable"));
        forecast.hourly[0].start = now - chrono::Duration::minutes(30);
        let (summary, _) = forecast_summary(&forecast, now, crate::settings::TempUnit::Celsius);
        assert!(summary.contains("20°C") && summary.contains("20% precipitation"));
        forecast.hourly[0].temp_f = f32::NAN;
        forecast.hourly[0].short.clear();
        let summary = forecast_summary(&forecast, now, crate::settings::TempUnit::Fahrenheit).0;
        assert!(summary.starts_with("Forecast: temperature unavailable"));
        assert!(!summary.contains("NaN"));
        forecast.hourly[0].start = now + chrono::Duration::hours(1);
        assert!(forecast_summary(&forecast, now, crate::settings::TempUnit::Fahrenheit).1.contains(&forecast.hourly[0].start.format("%b %-d %H:%M").to_string()));
    }

    #[test]
    fn place_alerts_respect_polygon_expiry_and_deduplication() {
        let now = Utc::now();
        let feature = GeoFeature {
            rings: vec![vec![[-98., 31.], [-96., 31.], [-96., 33.], [-98., 33.], [-98., 31.]]],
            fill: [0; 4], stroke: [0; 4], kind: wxdata::overlay::FeatureKind::Warning,
            title: "Tornado Warning".into(), detail: String::new(),
            alert: Some(wxdata::overlay::AlertInfo {
                id: "test".into(), event: "Tornado Warning".into(), headline: String::new(), area: String::new(),
                description: String::new(), instruction: String::new(), expires: Some(now + chrono::Duration::minutes(5)),
                max_hail_in: None, max_wind: None, tornado_detection: None, damage_threat: None, source: None, motion: None, vtec: None,
            }),
        };
        assert_eq!(place_alerts(&[feature.clone(), feature.clone()], -97., 32., now), ["Tornado Warning"]);
        assert!(place_alerts(std::slice::from_ref(&feature), -90., 32., now).is_empty());
        assert!(place_alerts(&[feature], -97., 32., now + chrono::Duration::minutes(5)).is_empty());
    }
}
