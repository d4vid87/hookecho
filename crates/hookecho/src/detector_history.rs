//! Bounded, exportable histories for client-side radar detectors.

pub const ALGORITHM_VERSION: &str = "hookecho-radar-detectors-v1";

#[derive(Debug, Clone, serde::Serialize)]
pub struct Snapshot {
    pub algorithm_version: &'static str,
    pub source_object: String,
    pub valid_time: chrono::DateTime<chrono::Utc>,
    pub sweep_count: usize,
    pub thresholds: Thresholds,
    pub tds: Vec<wxdata::tds::TdsHit>,
    pub tbss: Vec<wxdata::dualpol::TbssHit>,
    pub zdr_columns: Vec<wxdata::dualpol::ZdrColumnHit>,
    pub rotation: Vec<wxdata::rotation::CoupletHit>,
    pub reason_codes: [&'static str; 4],
}

#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct Thresholds {
    pub tds_cc_max: f32,
    pub tds_reflectivity_min_dbz: f32,
    pub tds_min_gates: usize,
    pub tbss_core_min_dbz: f32,
    pub zdr_min_db: f32,
    pub zdr_min_depth_km: f64,
    pub rotation_gate_to_gate_min_ms: f32,
    pub usable_range_km: [f32; 2],
}

impl Thresholds {
    pub fn from_settings(settings: &crate::settings::DetectorTuning) -> Self {
        Self {
            tds_cc_max: 0.8,
            tds_reflectivity_min_dbz: 40.0,
            tds_min_gates: 4,
            tbss_core_min_dbz: settings.tbss_core_dbz,
            zdr_min_db: settings.zdr_min_db,
            zdr_min_depth_km: settings.zdr_min_depth_km,
            rotation_gate_to_gate_min_ms: 25.0,
            usable_range_km: [15.0, 150.0],
        }
    }
}

pub fn to_json(history: &std::collections::VecDeque<Snapshot>) -> anyhow::Result<String> {
    Ok(serde_json::to_string_pretty(history)?)
}

#[cfg(test)]
mod tests {
    #[test]
    fn export_names_version_thresholds_and_reason_codes() {
        let snapshot = super::Snapshot {
            algorithm_version: super::ALGORITHM_VERSION,
            source_object: "KTLX20240526_013000_V06".into(),
            valid_time: "2024-05-26T01:30:00Z".parse().unwrap(),
            sweep_count: 1,
            thresholds: super::Thresholds::from_settings(&Default::default()),
            tds: Vec::new(),
            tbss: Vec::new(),
            zdr_columns: Vec::new(),
            rotation: Vec::new(),
            reason_codes: ["LOW_CC_HIGH_Z", "HAIL_CORE_SPIKE", "ZDR_ABOVE_FREEZING", "OPPOSITE_SIGN_SHEAR"],
        };
        let json = super::to_json(&std::collections::VecDeque::from([snapshot])).unwrap();
        for required in ["algorithm_version", "tds_cc_max", "reason_codes", "source_object"] {
            assert!(json.contains(required));
        }
    }
}
