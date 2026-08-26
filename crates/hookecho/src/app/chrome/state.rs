//! What the chrome looked like, saved and restored with a workspace.
//!
//! A workspace is an arrangement, and after R18 the arrangement includes the floating surfaces:
//! restoring "KTLX beside KDMX" but dropping the user back onto a bare map loses half of what
//! they saved. Only which surface was showing is recorded — not scroll offsets, not the drawer's
//! back-stack, which is a history rather than a layout.

use super::*;

/// Which window a drawer page belongs to, so a saved page can be reopened through the registry
/// instead of through a second table of `open` flags.
///
/// ponytail: one arm per page that has a registry action to reopen it. Pages opened by the map
/// rather than by a menu — a cross-section, a hodograph, the sounding, the station sensors — have
/// no such action and no meaningful thing to restore into, so they are absent on purpose: a title
/// this build cannot reopen simply doesn't reopen.
pub(crate) fn window_for_page(title: &str) -> Option<AppWindow> {
    Some(match title {
        "Settings" => AppWindow::Settings,
        "Event Library" => AppWindow::Events,
        "Chase Replay" => AppWindow::ChaseReplay,
        "Alert rules" => AppWindow::AlertRules,
        "Help" => AppWindow::Help,
        "About Hook Echo-WX" => AppWindow::About,
        "Forecast Discussion" => AppWindow::Afd,
        "CAPPI slice" => AppWindow::Cappi,
        "Storm attributes" => AppWindow::StormTable,
        "Storm Digest" => AppWindow::Digest,
        "Layer Manager" => AppWindow::LayerManager,
        "Location Markers" => AppWindow::Markers,
        "Color-Table Editor" => AppWindow::Palettes,
        "Placefile Manager" => AppWindow::Placefiles,
        "Select Radar Site" => AppWindow::Site,
        "Warning Verification" => AppWindow::Verify,
        "3D Reflectivity" => AppWindow::Volume3d,
        "Tornado climatology" => AppWindow::Climatology,
        _ => return None,
    })
}

impl HookEchoApp {
    pub(crate) fn capture_chrome(&self) -> crate::workspace::Chrome {
        crate::workspace::Chrome {
            panel_open: self.panel_open,
            alerts_tab: self.show_alert_panel,
            basemap_open: self.basemap_open,
            drawer: self.drawer.top().map(str::to_string),
        }
    }

    pub(crate) fn apply_chrome(&mut self, c: &crate::workspace::Chrome, ctx: &egui::Context) {
        self.panel_open = c.panel_open;
        self.show_alert_panel = c.alerts_tab;
        self.basemap_open = c.basemap_open;
        // The page opens the same way clicking its row in the panel opens it: one dispatch path,
        // so a page with side effects (a fetch, a rebuild) gets them here too.
        if let Some(w) = c.drawer.as_deref().and_then(window_for_page) {
            self.apply_palette(PaletteAction::OpenWindow(w), ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn known_pages_map_back_to_their_window() {
        assert_eq!(
            super::window_for_page("Settings"),
            Some(super::AppWindow::Settings)
        );
        assert_eq!(
            super::window_for_page("Tornado climatology"),
            Some(super::AppWindow::Climatology)
        );
        // A page from a newer build, or one that isn't a window at all: skipped, not fatal.
        assert_eq!(super::window_for_page("Storm 42 Attributes"), None);
        // Map-click pages are deliberately absent — nothing to restore them into.
        assert_eq!(super::window_for_page("Cross-section"), None);
    }
}
