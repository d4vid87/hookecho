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
    fn compares_against_the_running_build() {
        assert_eq!(compare(VERSION), UpdateState::UpToDate);
        assert_eq!(compare("v999.0.0"), UpdateState::Newer("v999.0.0".into()));
        assert_eq!(compare("what"), UpdateState::Failed);
    }
}
