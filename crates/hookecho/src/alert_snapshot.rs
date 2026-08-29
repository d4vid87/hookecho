//! The last good alert fetch, kept on disk.
//!
//! Two things go wrong when the app starts with no alerts. The map is empty until the first fetch
//! lands, which during an outbreak is the worst possible moment to show nothing; and the warning
//! seeding pass treats every alert already on the ground as brand new, so a restart mid-event
//! re-banners and re-speaks a dozen warnings the user watched arrive ten minutes ago.
//!
//! Writing the fetch out and reading it back at startup fixes both. Expired alerts are dropped on
//! load, so a snapshot from last week seeds nothing.

use wxdata::overlay::GeoFeature;

/// Snapshot file, or `None` where there's no filesystem (web).
fn path() -> Option<std::path::PathBuf> {
    crate::paths::cache_dir().map(|d| d.join("alerts-last.json"))
}

/// Drop alerts that have expired by `now`. Features with no expiry (SPC-style, or an alert issued
/// without one) are kept — the feed decides when they leave, not us.
pub fn still_active(feats: Vec<GeoFeature>, now: chrono::DateTime<chrono::Utc>) -> Vec<GeoFeature> {
    feats
        .into_iter()
        .filter(|f| {
            f.alert
                .as_ref()
                .and_then(|a| a.expires)
                .is_none_or(|e| e > now)
        })
        .collect()
}

/// Persist the current alert set, best effort — a failed write costs the next startup a blank map
/// for one fetch, which is exactly where we were before.
pub fn save(feats: &[GeoFeature]) {
    let Some(p) = path() else { return };
    let Ok(json) = serde_json::to_vec(feats) else {
        return;
    };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(p, json);
}

/// The last saved alert set, minus anything that has since expired.
pub fn load() -> Vec<GeoFeature> {
    let Some(p) = path() else { return Vec::new() };
    let Ok(bytes) = std::fs::read(p) else {
        return Vec::new();
    };
    match serde_json::from_slice(&bytes) {
        Ok(feats) => still_active(feats, chrono::Utc::now()),
        // A snapshot from an older build whose fields no longer line up is not an error worth
        // reporting; the next fetch replaces it.
        Err(e) => {
            log::debug!("alert snapshot unreadable: {e}");
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wxdata::overlay::{AlertInfo, FeatureKind};

    fn feat(id: &str, expires: Option<chrono::DateTime<chrono::Utc>>) -> GeoFeature {
        GeoFeature {
            rings: vec![vec![
                [-98.0, 35.0],
                [-97.0, 35.0],
                [-97.0, 36.0],
                [-98.0, 35.0],
            ]],
            fill: [255, 0, 0, 60],
            stroke: [255, 0, 0, 235],
            kind: FeatureKind::Warning,
            title: "Tornado Warning".into(),
            detail: "…".into(),
            alert: Some(AlertInfo {
                id: id.into(),
                event: "Tornado Warning".into(),
                headline: String::new(),
                area: String::new(),
                description: String::new(),
                instruction: String::new(),
                expires,
                max_hail_in: None,
                max_wind: None,
                tornado_detection: None,
                damage_threat: None,
                source: None,
                vtec: None,
                motion: None,
            }),
        }
    }

    #[test]
    fn expired_alerts_do_not_come_back() {
        let now = chrono::Utc::now();
        let feats = vec![
            feat("gone", Some(now - chrono::Duration::minutes(5))),
            feat("live", Some(now + chrono::Duration::minutes(20))),
            feat("no-expiry", None),
        ];
        let kept = still_active(feats, now);
        let ids: Vec<&str> = kept
            .iter()
            .map(|f| f.alert.as_ref().unwrap().id.as_str())
            .collect();
        assert_eq!(ids, vec!["live", "no-expiry"]);
    }

    #[test]
    fn roundtrips_through_json() {
        let now = chrono::Utc::now();
        let feats = vec![feat("live", Some(now + chrono::Duration::hours(1)))];
        let json = serde_json::to_vec(&feats).unwrap();
        let back: Vec<GeoFeature> = serde_json::from_slice(&json).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].alert, feats[0].alert);
        assert_eq!(back[0].rings, feats[0].rings);
    }
}
