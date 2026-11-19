//! Bounded document preparation. Native runs on the blocking pool; web uses the decode worker.

use crate::dialog::{Import, ImportKind};
use wxdata::placefile::{PlaceKind, Placefile};

const MAX_FEATURES: usize = 50_000;
const MAX_VERTICES: usize = 500_000;
const MAX_PREPARED_BYTES: usize = 128 * 1024 * 1024;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Gis {
    pub key: String,
    pub file: Placefile,
    pub attributes: Vec<String>,
    pub config: Option<crate::settings::PlacefileConfig>,
}

impl Gis {
    pub fn matches_config(&self, current: Option<&crate::settings::PlacefileConfig>) -> bool {
        let options = |cfg: &crate::settings::PlacefileConfig| {
            (
                cfg.gis_label_field.clone(),
                cfg.gis_color_field.clone(),
                cfg.gis_valid_start_field.clone(),
                cfg.gis_valid_end_field.clone(),
            )
        };
        self.config.as_ref().map(options) == current.map(options)
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub enum Prepared {
    Settings {
        settings: Box<crate::settings::Settings>,
        gis: Vec<Gis>,
    },
    Gpx {
        name: String,
        track: crate::chaselog::Track,
    },
    Gis {
        content: String,
        layer: Gis,
        #[cfg(target_arch = "wasm32")]
        #[serde(skip)]
        stored_reference: Option<String>,
    },
    Case(crate::casefile::CaseManifest),
    Palette {
        import: Import,
        text: String,
    },
    Icon {
        marker_id: String,
        name: String,
    },
    Asset(Import),
}

/// The UI keeps exactly one receiver; the dialog gate stays held through application.
pub fn start(
    import: Import,
    gis_config: Option<crate::settings::PlacefileConfig>,
    spawner: &crate::rt::Spawner,
    ctx: egui::Context,
) -> std::sync::mpsc::Receiver<Result<Prepared, String>> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    #[cfg(not(target_arch = "wasm32"))]
    spawner.spawn_blocking(move || {
        let result = std::panic::catch_unwind(|| prepare_with_options(import, gis_config.as_ref()))
            .unwrap_or_else(|_| Err("Document preparation failed".into()));
        let _ = tx.send(result);
        ctx.request_repaint();
    });
    #[cfg(target_arch = "wasm32")]
    spawner.spawn(async move {
        let result = prepare_browser(import, gis_config).await;
        let _ = tx.send(result);
        ctx.request_repaint();
    });
    rx
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Request {
    kind: ImportKind,
    name: String,
    tag: String,
    bytes: Vec<u8>,
    gis_config: Option<crate::settings::PlacefileConfig>,
}

#[cfg(target_arch = "wasm32")]
async fn prepare_browser(
    import: Import,
    gis_config: Option<crate::settings::PlacefileConfig>,
) -> Result<Prepared, String> {
    let request = Request {
        kind: import.kind,
        name: import.name(),
        tag: import.tag.clone(),
        bytes: import.into_content()?,
        gis_config,
    };
    let payload = postcard::to_allocvec(&request).map_err(|e| e.to_string())?;
    drop(request);
    let bytes = wxdata::wasm_worker::prepare_import(payload)
        .await
        .map_err(|e| {
            format!(
                "Import could not use the background worker: {e}. Retry after reloading the app."
            )
        })?;
    let mut prepared = decode_prepared(&bytes)?;
    drop(bytes);
    match &mut prepared {
        Prepared::Gis { content, layer, stored_reference } => {
            let pending = std::collections::BTreeMap::from([(layer.key.clone(), std::mem::take(content))]);
            let mut writes = crate::webcache::write_gis_sources(pending).await
                .map_err(|error| format!("Could not save browser GIS source: {error}"))?;
            let (reference, saved_content) = writes.remove(&layer.key)
                .ok_or_else(|| "Browser storage did not return the GIS source".to_string())?;
            *content = saved_content;
            *stored_reference = Some(reference);
        }
        Prepared::Settings { settings, .. } => {
            settings.migrate_inline_browser_gis().await?;
        }
        _ => {}
    }
    Ok(prepared)
}

/// Called by the existing decode worker, never by the browser UI as an inline fallback.
pub fn worker_prepare(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.len() > ImportKind::Gis.max_bytes() + 4096 {
        return Err("Import request exceeds its limit".into());
    }
    let request: Request = postcard::from_bytes(bytes).map_err(|e| e.to_string())?;
    request.kind.check_size(request.bytes.len() as u64)?;
    let prepared = prepare_with_options(
        Import {
            kind: request.kind,
            tag: request.tag,
            path: request.name.into(),
            bytes: Some(request.bytes),
        },
        request.gis_config.as_ref(),
    )?;
    encode_prepared(&prepared)
}

fn encode_prepared(prepared: &Prepared) -> Result<Vec<u8>, String> {
    struct Limited(Vec<u8>);
    impl std::io::Write for Limited {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > MAX_PREPARED_BYTES.saturating_sub(self.0.len()) {
                return Err(std::io::Error::other("Prepared import exceeds 128 MiB"));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut output = Limited(Vec::new());
    ciborium::into_writer(prepared, &mut output).map_err(|e| e.to_string())?;
    Ok(output.0)
}

#[cfg(any(target_arch = "wasm32", test))]
fn decode_prepared(bytes: &[u8]) -> Result<Prepared, String> {
    if bytes.len() > MAX_PREPARED_BYTES {
        return Err("Prepared import exceeds 128 MiB".into());
    }
    ciborium::from_reader(bytes).map_err(|e| e.to_string())
}

pub fn prepare(import: Import) -> Result<Prepared, String> {
    prepare_with_options(import, None)
}

fn prepare_with_options(
    import: Import,
    cfg: Option<&crate::settings::PlacefileConfig>,
) -> Result<Prepared, String> {
    let name = import.name();
    let kind = import.kind;
    if matches!(kind, ImportKind::MarkerIcon | ImportKind::AlertSound) {
        // Keep native asset references; validate size before their existing loaders use them.
        let size = match &import.bytes {
            Some(bytes) => bytes.len() as u64,
            None => {
                let metadata = std::fs::metadata(&import.path).map_err(|e| e.to_string())?;
                if !metadata.is_file() {
                    return Err("Choose a regular file".into());
                }
                metadata.len()
            }
        };
        kind.check_size(size)?;
        if kind == ImportKind::MarkerIcon {
            #[cfg(target_arch = "wasm32")]
            return Err("Marker icon uploads are not available in the browser".into());
            #[cfg(not(target_arch = "wasm32"))]
            return Ok(Prepared::Icon {
                marker_id: import.tag,
                name: crate::ui::marker_window::store_icon(&import.path)?,
            });
        }
        return Ok(Prepared::Asset(import));
    }
    if kind == ImportKind::Palette {
        let text = import.text()?;
        crate::colormap::parse_pal(&text).map_err(|e| e.to_string())?;
        return Ok(Prepared::Palette { import, text });
    }
    let bytes = import.into_content()?;
    match kind {
        ImportKind::Gis => {
            let archive = name.to_ascii_lowercase().ends_with(".kmz")
                || name.to_ascii_lowercase().ends_with(".zip");
            let (content, file, attributes) = if archive {
                use base64::Engine as _;
                let (file, attributes) = wxdata::gis::archive_with_options(
                    &name,
                    &bytes,
                    wxdata::gis::GisImportOptions {
                        label_field: cfg.and_then(|c| c.gis_label_field.as_deref()),
                        color_field: cfg.and_then(|c| c.gis_color_field.as_deref()),
                        valid_start_field: cfg.and_then(|c| c.gis_valid_start_field.as_deref()),
                        valid_end_field: cfg.and_then(|c| c.gis_valid_end_field.as_deref()),
                    },
                )
                .map_err(|e| e.to_string())?;
                (
                    format!(
                        "base64:{}",
                        base64::engine::general_purpose::STANDARD.encode(&bytes)
                    ),
                    file,
                    attributes,
                )
            } else {
                let content = String::from_utf8(bytes).map_err(|e| e.to_string())?;
                let file = wxdata::gis::parse(&name, &content).map_err(|e| e.to_string())?;
                (content, file, Vec::new())
            };
            validate_gis(&file, &mut (0, 0))?;
            Ok(Prepared::Gis {
                content,
                #[cfg(target_arch = "wasm32")]
                stored_reference: None,
                layer: Gis {
                    key: format!("gis:{name}"),
                    file,
                    attributes,
                    config: cfg.cloned(),
                },
            })
        }
        ImportKind::ChaseGpx => {
            let xml = String::from_utf8(bytes).map_err(|e| e.to_string())?;
            if !xml.contains("<gpx") || !xml.contains("</gpx>") {
                return Err("That file is not a GPX document".into());
            }
            if xml.match_indices("<trkpt").take(20_001).count() > 20_000
                || xml.match_indices("<wpt").take(2_001).count() > 2_000
            {
                return Err("GPX exceeds 20,000 track points or 2,000 waypoints".into());
            }
            let track = crate::chaselog::from_gpx(&xml);
            if track.points.len() < 2 {
                return Err("No track points in that file".into());
            }
            if track.points.iter().any(|p| !valid_position([p.lon, p.lat])) {
                return Err("GPX contains invalid coordinates".into());
            }
            Ok(Prepared::Gpx { name, track })
        }
        ImportKind::SettingsBundle => {
            let json = String::from_utf8(bytes).map_err(|e| e.to_string())?;
            let bundle = crate::settings::Settings::parse_import_bundle(&json)?;
            let mut gis = Vec::new();
            let mut geometry_budget = (0, 0);
            for cfg in &bundle.settings.placefiles {
                if let Some(name) = cfg.url.strip_prefix("gis:") {
                    if let Some(content) = bundle.settings.web_files.get(&cfg.url) {
                        let (file, attributes) =
                            stored_gis(name, content, Some(cfg)).map_err(|e| e.to_string())?;
                        validate_gis(&file, &mut geometry_budget)?;
                        gis.push(Gis {
                            key: cfg.url.clone(),
                            file,
                            attributes,
                            config: Some(cfg.clone()),
                        });
                    }
                }
            }
            // All embedded documents passed validation before palettes can replace local files.
            let settings = crate::settings::Settings::install_import_bundle(bundle)?;
            Ok(Prepared::Settings {
                settings: Box::new(settings),
                gis,
            })
        }
        ImportKind::CaseManifest => {
            let json = String::from_utf8(bytes).map_err(|e| e.to_string())?;
            let case =
                crate::casefile::CaseManifest::from_json(&json).map_err(|e| e.to_string())?;
            if case
                .annotations
                .iter()
                .map(|s| s.points.len())
                .sum::<usize>()
                > MAX_VERTICES
            {
                return Err("Case exceeds 500,000 annotation points".into());
            }
            if case
                .annotations
                .iter()
                .flat_map(|s| &s.points)
                .any(|p| !valid_position(*p))
            {
                return Err("Case contains invalid annotation coordinates".into());
            }
            Ok(Prepared::Case(case))
        }
        _ => unreachable!("assets and palettes are handled before document parsing"),
    }
}

fn valid_position(p: [f64; 2]) -> bool {
    p[0].is_finite()
        && p[1].is_finite()
        && (-180.0..=180.0).contains(&p[0])
        && (-90.0..=90.0).contains(&p[1])
}

fn validate_gis(file: &Placefile, used: &mut (usize, usize)) -> Result<(), String> {
    used.0 = used.0.saturating_add(file.items.len());
    if used.0 > MAX_FEATURES {
        return Err("GIS exceeds 50,000 drawable features".into());
    }
    let vertices = file
        .items
        .iter()
        .map(|item| match &item.kind {
            PlaceKind::Line { pts, .. } => pts.len(),
            PlaceKind::Polygon { rings, .. } => rings.iter().map(Vec::len).sum(),
            PlaceKind::Triangles { verts } => verts.len(),
            PlaceKind::Image { verts, .. } => verts.len(),
            _ => 1,
        })
        .sum::<usize>();
    used.1 = used.1.saturating_add(vertices);
    if used.1 > MAX_VERTICES {
        return Err("GIS exceeds 500,000 vertices".into());
    }
    Ok(())
}

pub(crate) fn stored_gis(
    name: &str,
    content: &str,
    cfg: Option<&crate::settings::PlacefileConfig>,
) -> anyhow::Result<(Placefile, Vec<String>)> {
    if name.to_ascii_lowercase().ends_with(".kmz") || name.to_ascii_lowercase().ends_with(".zip") {
        use base64::Engine as _;
        let encoded = content
            .strip_prefix("base64:")
            .ok_or_else(|| anyhow::anyhow!("saved GIS archive is corrupt"))?;
        let bytes = base64::engine::general_purpose::STANDARD.decode(encoded)?;
        wxdata::gis::archive_with_options(
            name,
            &bytes,
            wxdata::gis::GisImportOptions {
                label_field: cfg.and_then(|c| c.gis_label_field.as_deref()),
                color_field: cfg.and_then(|c| c.gis_color_field.as_deref()),
                valid_start_field: cfg.and_then(|c| c.gis_valid_start_field.as_deref()),
                valid_end_field: cfg.and_then(|c| c.gis_valid_end_field.as_deref()),
            },
        )
    } else {
        Ok((wxdata::gis::parse(name, content)?, Vec::new()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document(kind: ImportKind, name: &str, bytes: impl Into<Vec<u8>>) -> Import {
        Import {
            kind,
            tag: String::new(),
            path: name.into(),
            bytes: Some(bytes.into()),
        }
    }

    fn worker_roundtrip(kind: ImportKind, name: &str, bytes: Vec<u8>) -> Prepared {
        let payload = postcard::to_allocvec(&Request {
            kind,
            name: name.into(),
            tag: String::new(),
            bytes,
            gis_config: None,
        })
        .unwrap();
        decode_prepared(&worker_prepare(&payload).unwrap()).unwrap()
    }

    #[test]
    fn worker_roundtrip_preserves_normalized_gis_and_nested_properties() {
        let json = br#"{"type":"Feature","geometry":{"type":"LineString","coordinates":[[-97,32],[-98,33]]},"properties":{"label":"Route","nested":{"hazard":[true,4,null]}}}"#;
        let Prepared::Gis { content, layer, .. } =
            worker_roundtrip(ImportKind::Gis, "route.geojson", json.to_vec())
        else {
            panic!()
        };
        assert_eq!(content.as_bytes(), json);
        assert_eq!(layer.key, "gis:route.geojson");
        assert!(!layer.file.items.is_empty());
        let exported: serde_json::Value =
            serde_json::from_str(&wxdata::gis::export_geojson(&layer.file).unwrap()).unwrap();
        assert_eq!(
            exported["features"][0]["properties"]["nested"]["hazard"],
            serde_json::json!([true, 4, null])
        );
    }

    #[test]
    fn worker_roundtrip_preserves_gpx_and_case() {
        let track = crate::chaselog::Track {
            points: vec![
                crate::chaselog::Fix {
                    lon: -97.0,
                    lat: 32.0,
                    ts: 100,
                },
                crate::chaselog::Fix {
                    lon: -98.0,
                    lat: 33.0,
                    ts: 200,
                },
            ],
            waypoints: vec![(1, "Storm".into())],
        };
        let Prepared::Gpx {
            name,
            track: loaded,
        } = worker_roundtrip(
            ImportKind::ChaseGpx,
            "route.gpx",
            track.to_gpx().into_bytes(),
        )
        else {
            panic!()
        };
        assert_eq!(name, "route.gpx");
        assert_eq!(loaded.points, track.points);
        assert_eq!(loaded.waypoints, track.waypoints);
        let workspace = crate::workspace::starters().remove(0);
        let panes = workspace
            .panes
            .iter()
            .map(|_| crate::casefile::CasePane {
                site: "KFWS".into(),
                selected_time: None,
                radar_objects: vec![],
            })
            .collect();
        let case = crate::casefile::CaseManifest::new(
            "Storm case".into(),
            workspace,
            panes,
            vec![crate::casefile::CaseStroke {
                points: vec![[-97.0, 32.0]],
                rgba: [255, 0, 0, 255],
                width: 2.5,
            }],
            vec![],
        )
        .unwrap();
        let Prepared::Case(loaded) = worker_roundtrip(
            ImportKind::CaseManifest,
            "case.json",
            case.to_json().unwrap().into_bytes(),
        ) else {
            panic!()
        };
        assert_eq!(loaded, case);
    }

    #[test]
    fn worker_roundtrip_settings_preserves_schemas_and_disables_commands() {
        let json = serde_json::json!({"settings":{"default_site":"KFWS","plugins":[{
            "name":"Test","command":"do-not-run","enabled":true,"manifest":{
                "config_schema":{"type":"object","properties":{"flags":{"enum":[null,true,12,"x"]}}}
            }
        }],"web_files":{"reference":"contents"}}, "palette_files":{}});
        let Prepared::Settings { settings, gis } = worker_roundtrip(
            ImportKind::SettingsBundle,
            "settings.json",
            json.to_string().into_bytes(),
        ) else {
            panic!()
        };
        assert_eq!(settings.default_site, "KFWS");
        assert!(!settings.plugins[0].enabled);
        assert_eq!(
            settings.plugins[0].manifest.config_schema.as_ref().unwrap()["properties"]["flags"]
                ["enum"],
            serde_json::json!([null, true, 12, "x"])
        );
        assert_eq!(settings.web_files["reference"], "contents");
        assert!(gis.is_empty());
    }

    #[test]
    fn malformed_or_oversized_documents_never_produce_prepared_state() {
        for kind in [
            ImportKind::SettingsBundle,
            ImportKind::Gis,
            ImportKind::ChaseGpx,
            ImportKind::CaseManifest,
        ] {
            assert!(prepare(document(kind, "broken.json", b"not a document".to_vec())).is_err());
        }
        let payload = postcard::to_allocvec(&Request {
            kind: ImportKind::Palette,
            name: "large.pal".into(),
            tag: String::new(),
            bytes: vec![0; ImportKind::Palette.max_bytes() + 1],
            gis_config: None,
        })
        .unwrap();
        assert!(worker_prepare(&payload).unwrap_err().contains("limit"));
        let bad_gis = serde_json::json!({"settings":{"placefiles":[{"url":"gis:broken.json","enabled":true}],
            "web_files":{"gis:broken.json":"not geometry"}},"palette_files":{"REF":"Color: 0 1 2 3"}});
        // The existing palette installer runs only after every embedded GIS file validates.
        assert!(prepare(document(
            ImportKind::SettingsBundle,
            "settings.json",
            bad_gis.to_string().into_bytes()
        ))
        .is_err());
        let asset = Import {
            kind: ImportKind::MarkerIcon,
            tag: String::new(),
            path: std::env::temp_dir(),
            bytes: None,
        };
        assert!(prepare(asset).unwrap_err().contains("regular file"));
    }

    #[test]
    fn geometry_and_gpx_counts_bound_preparation() {
        let file = Placefile {
            items: vec![wxdata::placefile::PlaceItem {
                threshold_nmi: 0.0,
                time: None,
                anchor: None,
                properties: None,
                kind: PlaceKind::Line {
                    color: [0; 4],
                    width: 1.0,
                    pts: vec![[-97.0, 32.0]; MAX_VERTICES + 1],
                },
            }],
            ..Default::default()
        };
        assert!(validate_gis(&file, &mut (0, 0))
            .unwrap_err()
            .contains("vertices"));
        let mut budget = (0, MAX_VERTICES - 1);
        let small = Placefile {
            items: vec![wxdata::placefile::PlaceItem {
                threshold_nmi: 0.0,
                time: None,
                anchor: None,
                properties: None,
                kind: PlaceKind::Line {
                    color: [0; 4],
                    width: 1.0,
                    pts: vec![[-97.0, 32.0]; 2],
                },
            }],
            ..Default::default()
        };
        assert!(validate_gis(&small, &mut budget)
            .unwrap_err()
            .contains("vertices"));
        let xml = format!(
            "<gpx>{}</gpx>",
            "<trkpt lat='32' lon='-97'/>".repeat(20_001)
        );
        assert!(prepare(document(
            ImportKind::ChaseGpx,
            "large.gpx",
            xml.into_bytes()
        ))
        .unwrap_err()
        .contains("20,000"));
        let xml = b"<gpx><trkpt lat='NaN' lon='-97'/><trkpt lat='32' lon='-98'/></gpx>";
        assert!(
            prepare(document(ImportKind::ChaseGpx, "invalid.gpx", xml.to_vec()))
                .unwrap_err()
                .contains("coordinates")
        );
        let xml =
            "<gpx><trkpt lat=\u{1f4a3}32\u{1f4a3} lon='-97'/><trkpt lat='32' lon='-98'/></gpx>";
        assert!(prepare(document(
            ImportKind::ChaseGpx,
            "malformed.gpx",
            xml.as_bytes().to_vec()
        ))
        .is_err());
    }

    #[test]
    fn prepared_geometry_cannot_be_installed_under_changed_style_options() {
        let config = crate::settings::PlacefileConfig {
            url: "gis:places.zip".into(),
            enabled: true,
            opacity: 1.0,
            gis_label_field: Some("name".into()),
            gis_color_field: None,
            gis_valid_start_field: None,
            gis_valid_end_field: None,
        };
        let layer = Gis {
            key: config.url.clone(),
            file: Placefile::default(),
            attributes: vec![],
            config: Some(config.clone()),
        };
        assert!(layer.matches_config(Some(&config)));
        let mut changed = config;
        changed.enabled = false;
        assert!(
            layer.matches_config(Some(&changed)),
            "visibility does not affect normalization"
        );
        changed.gis_label_field = Some("city".into());
        assert!(!layer.matches_config(Some(&changed)));
        assert!(
            !layer.matches_config(None),
            "removed layers are not silently restored"
        );
    }

    /// Paired with scripts/web/import-worker.mjs: real browser results use the production codec.
    #[test]
    #[ignore = "requires generated browser worker results; see scripts/web/import-worker.mjs"]
    fn browser_worker_fixtures() {
        let dir = std::path::PathBuf::from(
            std::env::var("HOOKECHO_IMPORT_FIXTURES").expect("fixture output directory"),
        );
        std::fs::create_dir_all(&dir).unwrap();
        if std::env::var_os("HOOKECHO_IMPORT_VERIFY").is_some() {
            let read = |name| {
                decode_prepared(&std::fs::read(dir.join(format!("{name}.cbor"))).unwrap()).unwrap()
            };
            let Prepared::Settings { settings, .. } = read("settings") else {
                panic!()
            };
            assert_eq!(settings.default_site, "KFWS");
            assert!(!settings.plugins[0].enabled);
            assert_eq!(
                settings.plugins[0].manifest.config_schema.as_ref().unwrap()["enum"],
                serde_json::json!([true, 3, null])
            );
            assert!(settings.web_files[&settings.palettes["REF"]].contains("Color: 0 1 2 3"));
            let Prepared::Gis { layer, .. } = read("gis") else {
                panic!()
            };
            assert!(layer.file.items.iter().any(|i| i
                .properties
                .as_ref()
                .is_some_and(|p| p["nested"]["flags"] == serde_json::json!([true, null, 3]))));
            assert!(layer
                .file
                .items
                .iter()
                .any(|i| matches!(&i.kind, PlaceKind::Line { pts, .. } if pts.len() == 200_000)));
            let Prepared::Gpx { track, .. } = read("gpx") else {
                panic!()
            };
            assert_eq!(track.points.len(), 2);
            assert_eq!(track.waypoints, [(1, "Storm".into())]);
            let Prepared::Case(case) = read("case") else {
                panic!()
            };
            assert_eq!(case.name, "Browser case");
            assert_eq!(case.annotations[0].points, [[-97.0, 32.0], [-98.0, 33.0]]);
            let report: serde_json::Value =
                serde_json::from_slice(&std::fs::read(dir.join("report.json")).unwrap()).unwrap();
            for name in [
                "bad-settings",
                "bad-gis",
                "bad-gpx",
                "bad-case",
                "oversized",
            ] {
                assert!(
                    report[name]["error"]
                        .as_str()
                        .is_some_and(|e| !e.is_empty()),
                    "{name}"
                );
            }
            return;
        }
        let mut fixtures = Vec::new();
        let mut save = |name: &str, kind, filename: &str, bytes: Vec<u8>, error: bool| {
            let payload = postcard::to_allocvec(&Request {
                kind,
                name: filename.into(),
                tag: String::new(),
                bytes,
                gis_config: None,
            })
            .unwrap();
            std::fs::write(dir.join(format!("{name}.bin")), payload).unwrap();
            fixtures.push(serde_json::json!({"name":name,"error":error}));
        };
        let settings = serde_json::json!({"settings":{"default_site":"KFWS","plugins":[{"name":"test","command":"never-run","enabled":true,
            "manifest":{"config_schema":{"enum":[true,3,null]}}}]},"palette_files":{"REF":"Color: 0 1 2 3\nColor: 80 250 251 252\n"}});
        save(
            "settings",
            ImportKind::SettingsBundle,
            "settings.json",
            settings.to_string().into_bytes(),
            false,
        );
        let coordinates = (0..200_000)
            .map(|n| [-97.0 + f64::from(n % 100) / 1000.0, 32.0])
            .collect::<Vec<_>>();
        let gis = serde_json::json!({"type":"Feature","geometry":{"type":"LineString","coordinates":coordinates},"properties":{"nested":{"flags":[true,null,3]}}});
        save(
            "gis",
            ImportKind::Gis,
            "large.geojson",
            gis.to_string().into_bytes(),
            false,
        );
        let track = crate::chaselog::Track {
            points: vec![
                crate::chaselog::Fix {
                    lon: -97.0,
                    lat: 32.0,
                    ts: 100,
                },
                crate::chaselog::Fix {
                    lon: -98.0,
                    lat: 33.0,
                    ts: 200,
                },
            ],
            waypoints: vec![(1, "Storm".into())],
        };
        save(
            "gpx",
            ImportKind::ChaseGpx,
            "route.gpx",
            track.to_gpx().into_bytes(),
            false,
        );
        let workspace = crate::workspace::starters().remove(0);
        let panes = workspace
            .panes
            .iter()
            .map(|_| crate::casefile::CasePane {
                site: "KFWS".into(),
                selected_time: None,
                radar_objects: vec![],
            })
            .collect();
        let case = crate::casefile::CaseManifest::new(
            "Browser case".into(),
            workspace,
            panes,
            vec![crate::casefile::CaseStroke {
                points: vec![[-97.0, 32.0], [-98.0, 33.0]],
                rgba: [255, 0, 0, 255],
                width: 2.5,
            }],
            vec![],
        )
        .unwrap();
        save(
            "case",
            ImportKind::CaseManifest,
            "case.json",
            case.to_json().unwrap().into_bytes(),
            false,
        );
        for (name, kind) in [
            ("bad-settings", ImportKind::SettingsBundle),
            ("bad-gis", ImportKind::Gis),
            ("bad-gpx", ImportKind::ChaseGpx),
            ("bad-case", ImportKind::CaseManifest),
        ] {
            save(name, kind, "broken.json", b"not a document".to_vec(), true);
        }
        save(
            "oversized",
            ImportKind::SettingsBundle,
            "oversized.json",
            vec![0; ImportKind::SettingsBundle.max_bytes() + 1],
            true,
        );
        std::fs::write(
            dir.join("fixtures.json"),
            serde_json::to_vec(&fixtures).unwrap(),
        )
        .unwrap();
    }
}
