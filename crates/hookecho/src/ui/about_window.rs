//! About: what this is, which version you're running, and whether there's a newer one.

use crate::ui::style;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const REPO: &str = "https://github.com/d4vid87/hookecho";

/// Result of the once-per-session update check.
#[derive(Debug, Clone, PartialEq)]
pub enum UpdateState {
    Idle,
    Checking,
    UpToDate,
    Newer(String),
    /// The repo has no complete published release to compare against. Distinct from `Failed`:
    /// the request worked, there was simply nothing tagged. A repo whose releases are all still
    /// drafts looks like this to everyone but its owner.
    NoRelease,
    Failed,
}

/// Draw the window. Returns false once the user closed it.
pub fn show(
    ctx: &egui::Context,
    open: &mut bool,
    update: &UpdateState,
    accent: egui::Color32,
    drawer: &mut crate::ui::drawer::Drawer,
) {
    let Some(window) = drawer.page(
        ctx,
        "About HookEcho",
        open,
        false,
        egui::Window::new("About HookEcho"),
    ) else {
        return;
    };
    let logo = crate::icon::texture(ctx, 128);
    window.show(ctx, |ui| {
        // The mark beside the name, which is the one place in the app the logo is looked at
        // rather than glanced at in a taskbar.
        ui.horizontal(|ui| {
            ui.add(egui::Image::new(&logo).fit_to_exact_size(egui::vec2(64.0, 64.0)));
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new("HookEcho")
                        .size(style::FONT_TITLE)
                        .color(accent)
                        .strong(),
                );
                ui.label(egui::RichText::new(format!("Version {VERSION}")).weak());
                ui.label(egui::RichText::new(format!("{} · {}", env!("HOOKECHO_BUILD_CHANNEL"),
                    env!("HOOKECHO_BUILD_REVISION"))).weak());
                ui.label(egui::RichText::new(format!("Built {}", crate::build_date())).weak());
            });
        });
        ui.add_space(8.0);
        ui.label("An advanced NEXRAD weather radar viewer. Free and GPL-3.0-only licensed.");
        ui.add_space(8.0);
        ui.hyperlink_to("Source, releases and issues", REPO);
        ui.add_space(10.0);
        ui.separator();
        match update {
            UpdateState::Idle | UpdateState::Checking => {
                crate::ui::loading(ui, "Checking for updates…")
            }
            UpdateState::UpToDate => {
                ui.label(egui::RichText::new("You're on the latest release.").weak());
            }
            UpdateState::Newer(tag) => {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("{} is available", release_label(tag)))
                            .color(style::ZETA_GREEN),
                    );
                    ui.hyperlink_to("Download", format!("{REPO}/releases/tag/{tag}"));
                });
            }
            UpdateState::NoRelease => {
                ui.label(egui::RichText::new("No published release to compare against.").weak());
            }
            UpdateState::Failed => {
                ui.label(egui::RichText::new("Couldn't check for updates.").weak());
            }
        }
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new(
                "Radar and warning data from NOAA/NWS and Iowa State's IEM archive. Built \
                     with egui and wgpu.",
            )
            .size(style::FONT_SM)
            .weak(),
        );
    });
}

/// Parse the complete release version, including prerelease precedence.
pub fn parse_version(tag: &str) -> Option<semver::Version> {
    semver::Version::parse(tag.trim().strip_prefix('v').unwrap_or(tag.trim())).ok()
}

pub fn release_label(tag: &str) -> String {
    match build_revision(tag) {
        Some(revision) => format!("HookEcho build {}", &revision[..7]),
        None => format!("HookEcho {tag}"),
    }
}

fn build_revision(tag: &str) -> Option<&str> {
    let revision = tag.strip_prefix("build-")?;
    (revision.len() == 40 && revision.bytes().all(|c| c.is_ascii_hexdigit()))
        .then_some(revision)
}

fn stable_tag(json: &str) -> Option<String> {
    let release: serde_json::Value = serde_json::from_str(json).ok()?;
    if release.get("draft")?.as_bool()? || release.get("prerelease")?.as_bool()? {
        return None;
    }
    let tag = release.get("tag_name")?.as_str()?;
    if build_revision(tag).is_some() {
        // Match the publisher's all-platform gate, including its checksum manifest.
        let assets = release.get("assets")?.as_array()?;
        for name in ["HookEcho-x86_64.AppImage", "HookEcho-amd64.deb",
            "hookecho-windows-x86_64.zip", "HookEcho-x86_64.msi", "HookEcho-setup-x86_64.exe",
            "HookEcho-macos.zip", "HookEcho-arm64-v8a.apk", "HookEcho-arm64-v8a.aab", "SHA256SUMS"] {
            if !assets.iter().any(|asset| asset.get("name").and_then(|v| v.as_str()) == Some(name)
                && asset.get("size").and_then(|v| v.as_u64()).is_some_and(|size| size > 0)) {
                return None;
            }
        }
    } else if !parse_version(tag)?.pre.is_empty() {
        return None;
    }
    Some(tag.to_owned())
}

fn compare_build(tag: &str, comparison: &str) -> UpdateState {
    let json = serde_json::from_str::<serde_json::Value>(comparison).ok();
    match json.as_ref().and_then(|value| value.get("status")).and_then(|v| v.as_str()) {
        Some("ahead") => UpdateState::Newer(tag.to_owned()),
        Some("identical" | "behind") => UpdateState::UpToDate,
        // Divergent/unknown local builds cannot safely be ordered by release publication date.
        _ => UpdateState::Failed,
    }
}

/// Stable builds follow GitHub's designated Latest release; prereleases keep SemVer ordering.
pub async fn check(http: &reqwest::Client) -> UpdateState {
    async fn request(http: &reqwest::Client) -> Result<UpdateState, reqwest::Error> {
        let preview = parse_version(VERSION).is_some_and(|version| !version.pre.is_empty());
        let base = "https://api.github.com/repos/d4vid87/hookecho";
        let endpoint = if preview { "releases?per_page=30" } else { "releases/latest" };
        let response = http.get(format!("{base}/{endpoint}"))
            .timeout(wxdata::net::FEED_TIMEOUT)
            .header("User-Agent", "hookecho").send().await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(UpdateState::NoRelease);
        }
        let body = response.error_for_status()?.text().await?;
        if serde_json::from_str::<serde_json::Value>(&body).is_err() {
            return Ok(UpdateState::Failed);
        }
        let tag = if preview { pick_latest_tag(&body) } else { stable_tag(&body) };
        let Some(tag) = tag else { return Ok(UpdateState::NoRelease) };
        let Some(remote) = build_revision(&tag) else { return Ok(compare(&tag)) };
        let local = crate::BUILD_REVISION;
        if !(7..=40).contains(&local.len()) || !local.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Ok(UpdateState::Failed);
        }
        if remote.starts_with(local) { return Ok(UpdateState::UpToDate); }
        let comparison = http.get(format!("{base}/compare/{local}...{remote}"))
            .timeout(wxdata::net::FEED_TIMEOUT)
            .header("User-Agent", "hookecho").send().await?
            .error_for_status()?.text().await?;
        Ok(compare_build(&tag, &comparison))
    }
    wxdata::task::timeout(wxdata::net::FEED_TIMEOUT, request(http)).await
        .ok().and_then(Result::ok).unwrap_or(UpdateState::Failed)
}

/// The state a fetched release tag implies for the running build.
pub fn compare(tag: &str) -> UpdateState {
    compare_versions(tag, VERSION)
}

fn compare_versions(tag: &str, current: &str) -> UpdateState {
    match (parse_version(tag), parse_version(current)) {
        (Some(remote), Some(local)) if remote.cmp_precedence(&local).is_gt() => UpdateState::Newer(tag.to_string()),
        (Some(_), Some(_)) => UpdateState::UpToDate,
        _ => UpdateState::Failed,
    }
}

/// Choose a SemVer release for the prerelease channel; stable builds use GitHub's Latest.
pub fn pick_latest_tag(json: &str) -> Option<String> {
    pick_latest_tag_for(json, VERSION)
}

fn pick_latest_tag_for(json: &str, current: &str) -> Option<String> {
    let preview = !parse_version(current)?.pre.is_empty();
    let releases: Vec<serde_json::Value> = serde_json::from_str(json).ok()?;
    releases.iter()
        .filter(|release| !release.get("draft").and_then(|value| value.as_bool()).unwrap_or(false))
        .filter_map(|release| {
            let tag = release.get("tag_name")?.as_str()?;
            let version = parse_version(tag)?;
            let prerelease = !version.pre.is_empty()
                || release.get("prerelease").and_then(|value| value.as_bool()).unwrap_or(false);
            (preview || !prerelease).then(|| (version, tag.to_string()))
        })
        .max_by(|a, b| a.0.cmp_precedence(&b.0))
        .map(|(_, tag)| tag)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_build_requires_a_complete_published_release() {
        let tag = format!("build-{}", "a".repeat(40));
        let mut release = serde_json::json!({"tag_name": tag, "draft": false, "prerelease": false,
            "assets": (["HookEcho-x86_64.AppImage", "HookEcho-amd64.deb",
                "hookecho-windows-x86_64.zip", "HookEcho-x86_64.msi", "HookEcho-setup-x86_64.exe",
                "HookEcho-macos.zip", "HookEcho-arm64-v8a.apk", "HookEcho-arm64-v8a.aab", "SHA256SUMS"]
                .map(|name| serde_json::json!({"name": name, "size": 100})))});
        assert_eq!(stable_tag(&release.to_string()), Some(tag.clone()));
        release["draft"] = true.into();
        assert_eq!(stable_tag(&release.to_string()), None);
        release["draft"] = false.into();
        release["prerelease"] = true.into();
        assert_eq!(stable_tag(&release.to_string()), None);
        release["prerelease"] = false.into();
        release["assets"][0]["size"] = 0.into();
        assert_eq!(stable_tag(&release.to_string()), None);
        release["assets"][0]["size"] = 100.into();
        release["assets"].as_array_mut().unwrap().pop();
        assert_eq!(stable_tag(&release.to_string()), None);
        for bad in ["latest", "build-deadbeef", "build-../../compare", "v0.12.0-beta.1"] {
            release["tag_name"] = bad.into();
            assert_eq!(stable_tag(&release.to_string()), None);
        }
        release["tag_name"] = "v0.12.0".into();
        assert_eq!(stable_tag(&release.to_string()).as_deref(), Some("v0.12.0"));
    }

    #[test]
    fn commit_comparison_never_recommends_an_older_or_divergent_build() {
        let tag = format!("build-{}", "b".repeat(40));
        assert_eq!(compare_build(&tag, r#"{"status":"ahead"}"#), UpdateState::Newer(tag.clone()));
        for status in ["identical", "behind"] {
            assert_eq!(compare_build(&tag, &serde_json::json!({"status": status}).to_string()), UpdateState::UpToDate);
        }
        for body in [r#"{"status":"diverged"}"#, "{}", "not json"] {
            assert_eq!(compare_build(&tag, body), UpdateState::Failed);
        }
        assert_eq!(release_label(&tag), "HookEcho build bbbbbbb");
    }

    #[test]
    fn prereleases_and_final_releases_compare_in_semver_order() {
        for (older, newer) in [
            ("v0.12.0-alpha.1", "v0.12.0-beta.2"),
            ("v0.12.0-beta.2", "v0.12.0-beta.11"),
            ("v0.12.0-rc.1", "v0.12.0"),
        ] {
            assert_eq!(compare_versions(newer, older), UpdateState::Newer(newer.into()));
            assert_eq!(compare_versions(older, newer), UpdateState::UpToDate);
        }
        assert_eq!(compare(VERSION), UpdateState::UpToDate);
        assert_eq!(compare("v999.0.0"), UpdateState::Newer("v999.0.0".into()));
        assert_eq!(compare("what"), UpdateState::Failed);
        assert_eq!(compare_versions("0.12.0+new", "0.12.0+old"), UpdateState::UpToDate);
        assert!(parse_version("v1.2.3.4").is_none());
    }

    #[test]
    fn updates_follow_the_installed_channel_and_exclude_drafts_and_rolling_tags() {
        let json = r#"[
            {"tag_name":"latest","draft":false,"prerelease":false},
            {"tag_name":"v0.11.0","draft":false,"prerelease":false},
            {"tag_name":"v0.12.0-beta.2","draft":false,"prerelease":true},
            {"tag_name":"v0.12.0-beta.11","draft":false,"prerelease":true},
            {"tag_name":"v0.12.0","draft":true,"prerelease":false},
            {"tag_name":"v0.13.0-alpha.1","draft":false,"prerelease":false}
        ]"#;
        assert_eq!(pick_latest_tag_for(json, "0.11.0").as_deref(), Some("v0.11.0"));
        assert_eq!(pick_latest_tag_for(json, "0.12.0-beta.2").as_deref(), Some("v0.13.0-alpha.1"));
        let published_beta = r#"[{"tag_name":"v0.12.0-beta.2","prerelease":true}]"#;
        assert_eq!(pick_latest_tag_for(published_beta, "0.12.0-beta.1").as_deref(), Some("v0.12.0-beta.2"));
        assert!(pick_latest_tag_for(published_beta, "0.11.0").is_none());
        assert!(pick_latest_tag("[]").is_none());
        assert!(pick_latest_tag("not json").is_none());
    }
}
