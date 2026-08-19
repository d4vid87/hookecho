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
pub fn show(ctx: &egui::Context, open: &mut bool, update: &UpdateState, accent: egui::Color32) {
    crate::ui::phone_surface(ctx, egui::Window::new("About Hook Echo-WX"))
        .open(open)
        .default_size([380.0, 260.0])
        .show(ctx, |ui| {
            ui.label(
                egui::RichText::new("Hook Echo-WX")
                    .size(style::FONT_TITLE)
                    .color(accent)
                    .strong(),
            );
            ui.label(egui::RichText::new(format!("Version {VERSION}")).weak());
            ui.add_space(8.0);
            ui.label("An advanced NEXRAD weather radar viewer. Free and MIT licensed.");
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
                                .color(style::OMEGA_GREEN),
                        );
                        ui.hyperlink_to("Download", format!("{REPO}/releases/latest"));
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

/// `(major, minor, patch)` from a release tag like `v0.5.0`. `None` if it isn't one.
pub fn parse_version(tag: &str) -> Option<(u64, u64, u64)> {
    let mut it = tag.trim().trim_start_matches('v').split('.');
    let mut next = || it.next()?.split('-').next()?.parse::<u64>().ok();
    let (a, b, c) = (next()?, next()?, next()?);
    Some((a, b, c))
}

/// The state a fetched `tag_name` implies for the running build.
pub fn compare(tag: &str) -> UpdateState {
    match (parse_version(tag), parse_version(VERSION)) {
        (Some(remote), Some(local)) if remote > local => UpdateState::Newer(tag.to_string()),
        (Some(_), Some(_)) => UpdateState::UpToDate,
        _ => UpdateState::Failed,
    }
}

/// The newest version tag in a GitHub `/releases` response.
///
/// Not `/releases/latest`, which returns whatever was published most recently regardless of what
/// it is called — here that is the rolling `latest` CI build, whose tag is not a version at all,
/// so the check reported "couldn't check for updates" forever. Scanning the list and taking the
/// highest tag that actually parses as a version ignores rolling and named builds by
/// construction. Drafts and prereleases are skipped; the API omits drafts from unauthenticated
/// callers anyway, which is every copy of this app.
pub fn pick_latest_tag(json: &str) -> Option<String> {
    let releases: Vec<serde_json::Value> = serde_json::from_str(json).ok()?;
    releases
        .iter()
        .filter(|r| {
            !r.get("draft").and_then(|v| v.as_bool()).unwrap_or(false)
                && !r.get("prerelease").and_then(|v| v.as_bool()).unwrap_or(false)
        })
        .filter_map(|r| r.get("tag_name")?.as_str())
        .filter_map(|tag| Some((parse_version(tag)?, tag.to_string())))
        .max()
        .map(|(_, tag)| tag)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_parse_and_order() {
        assert_eq!(parse_version("v0.5.0"), Some((0, 5, 0)));
        assert_eq!(parse_version("1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("v1.2.3-rc1"), Some((1, 2, 3)));
        assert_eq!(parse_version("nightly"), None);
        assert!(parse_version("v0.6.0") > parse_version("v0.5.9"));
    }

    #[test]
    fn the_rolling_build_is_not_mistaken_for_a_release() {
        // Shape of the real response as of 0.10.0: a rolling `latest` that is neither a draft nor
        // a prerelease, sitting above the version tags. `/releases/latest` hands back this one.
        let json = r##"[
            {"tag_name":"latest","draft":false,"prerelease":false},
            {"tag_name":"v0.9.0","draft":false,"prerelease":false},
            {"tag_name":"v0.10.0","draft":false,"prerelease":false},
            {"tag_name":"v0.11.0","draft":true,"prerelease":false},
            {"tag_name":"v0.12.0","draft":false,"prerelease":true}
        ]"##;
        // Highest *version*, not the newest entry, and not the draft or prerelease above it.
        assert_eq!(pick_latest_tag(json).as_deref(), Some("v0.10.0"));
    }

    #[test]
    fn a_repo_with_nothing_published_yields_no_tag() {
        assert_eq!(pick_latest_tag("[]"), None);
        // Every version still a draft: this is what the repo looks like today to anyone but its
        // owner, and it must not read as "you're on the latest release".
        let drafts = r##"[
            {"tag_name":"latest","draft":false,"prerelease":false},
            {"tag_name":"v0.10.0","draft":true,"prerelease":false}
        ]"##;
        assert_eq!(pick_latest_tag(drafts), None);
        assert_eq!(pick_latest_tag("not json"), None);
    }

    #[test]
    fn compares_against_the_running_build() {
        assert_eq!(compare(VERSION), UpdateState::UpToDate);
        assert_eq!(compare("v999.0.0"), UpdateState::Newer("v999.0.0".into()));
        assert_eq!(compare("what"), UpdateState::Failed);
    }
}
