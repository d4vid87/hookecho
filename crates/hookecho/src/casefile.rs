//! Portable case manifests: pane layout plus exact source-object identities and selected times.

use crate::workspace::Workspace;
use chrono::{DateTime, Utc};

pub const SCHEMA_VERSION: u16 = 1;
const MAX_OBJECTS_PER_PANE: usize = 2048;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CaseManifest {
    pub schema_version: u16,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub workspace: Workspace,
    pub panes: Vec<CasePane>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CasePane {
    pub site: String,
    pub selected_time: Option<DateTime<Utc>>,
    /// Immutable upstream object names in playback order.
    pub radar_objects: Vec<String>,
}

impl CaseManifest {
    pub fn new(name: String, workspace: Workspace, panes: Vec<CasePane>) -> anyhow::Result<Self> {
        let manifest = Self {
            schema_version: SCHEMA_VERSION,
            name,
            created_at: Utc::now(),
            workspace,
            panes,
        };
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn from_json(json: &str) -> anyhow::Result<Self> {
        let manifest: Self = serde_json::from_str(json)?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn to_json(&self) -> anyhow::Result<String> {
        self.validate()?;
        Ok(serde_json::to_string_pretty(self)?)
    }

    fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.schema_version == SCHEMA_VERSION,
            "unsupported case manifest version {}",
            self.schema_version
        );
        anyhow::ensure!(!self.name.trim().is_empty(), "case name is empty");
        anyhow::ensure!(
            self.workspace.panes.len() == self.panes.len(),
            "case pane metadata does not match its workspace"
        );
        anyhow::ensure!(
            self.panes.len() <= crate::workspace::MAX_PANES,
            "case exceeds the pane resource limit"
        );
        anyhow::ensure!(
            self.panes
                .iter()
                .all(|pane| pane.radar_objects.len() <= MAX_OBJECTS_PER_PANE),
            "case contains too many radar objects"
        );
        for pane in &self.panes {
            for name in &pane.radar_objects {
                let object = wxdata::level2::Identifier::new(name.clone());
                anyhow::ensure!(
                    name.len() <= 128
                        && object.site() == Some(pane.site.as_str())
                        && object.date_time().is_some(),
                    "case contains an invalid radar object"
                );
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_round_trips_and_rejects_mismatched_panes() {
        let workspace = crate::workspace::starters().remove(0);
        let panes = workspace
            .panes
            .iter()
            .map(|_| CasePane {
                site: "KTLX".into(),
                selected_time: Some("2024-05-26T01:30:00Z".parse().unwrap()),
                radar_objects: vec!["KTLX20240526_013000_V06".into()],
            })
            .collect();
        let manifest = CaseManifest::new("May 25 outbreak".into(), workspace, panes).unwrap();
        assert_eq!(
            CaseManifest::from_json(&manifest.to_json().unwrap()).unwrap(),
            manifest
        );

        let mut bad = manifest;
        bad.panes.pop();
        assert!(bad
            .to_json()
            .unwrap_err()
            .to_string()
            .contains("pane metadata"));
    }
}
