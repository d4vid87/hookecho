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
    /// The repo has no published versioned release to compare against. Distinct from `Failed`:
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
                        egui::RichText::new(format!("Version {tag} is available"))
                            .color(style::ZETA_GREEN),
                    );
                    ui.hyperlink_to("Download", format!("{REPO}/releases"));
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

/// Choose a published version on the installed channel; rolling tags are not releases.
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
