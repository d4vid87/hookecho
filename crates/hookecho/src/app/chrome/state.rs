//! What the chrome looked like, saved and restored with a workspace.
//!
//! A workspace is an arrangement, and after R18 the arrangement includes the floating surfaces:
//! restoring "KTLX beside KDMX" but dropping the user back onto a bare map loses half of what
//! they saved. Only which surface was showing is recorded — not scroll offsets, not the drawer's
//! back-stack, which is a history rather than a layout.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PrimarySurface { Map, Panel, Drawer }

pub(crate) fn primary_surface(panel_open: bool, drawer_open: bool) -> PrimarySurface {
    if drawer_open { PrimarySurface::Drawer }
    else if panel_open { PrimarySurface::Panel }
    else { PrimarySurface::Map }
}

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
        "About HookEcho" => AppWindow::About,
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

fn window_from_chrome(c: &crate::workspace::Chrome) -> Option<AppWindow> {
    c.drawer_id.as_deref()
        .and_then(|id| serde_json::from_value::<AppWindow>(serde_json::Value::String(id.to_string())).ok())
        .or_else(|| c.drawer.as_deref().and_then(window_for_page))
}

impl HookEchoApp {
    pub(crate) fn primary_surface(&self) -> PrimarySurface {
        primary_surface(self.panel_open, self.drawer.is_open())
    }
    pub(crate) fn capture_chrome(&self) -> crate::workspace::Chrome {
        crate::workspace::Chrome {
            panel_open: self.panel_open,
            analyst_open: self.analyst_open,
            analyst_inspector_open: self.analyst_inspector_open,
            alerts_tab: self.show_alert_panel,
            basemap_open: self.basemap_open,
            drawer: self.drawer.top().map(str::to_string),
            drawer_id: self.drawer.top()
                .and_then(window_for_page)
                .and_then(|window| serde_json::to_value(window).ok())
                .and_then(|value| value.as_str().map(str::to_string)),
        }
    }

    pub(crate) fn apply_chrome(&mut self, c: &crate::workspace::Chrome, ctx: &egui::Context) {
        self.panel_open = c.panel_open;
        self.analyst_open = c.analyst_open;
        self.analyst_inspector_open = c.analyst_inspector_open;
        self.show_alert_panel = c.alerts_tab;
        self.basemap_open = c.basemap_open;
        // The page opens the same way clicking its row in the panel opens it: one dispatch path,
        // so a page with side effects (a fetch, a rebuild) gets them here too.
        if let Some(w) = window_from_chrome(c) {
            self.apply_palette(PaletteAction::OpenWindow(w), ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn drawer_takes_priority_and_menu_recovers_after_close() {
        use super::PrimarySurface as S;
        assert_eq!(super::primary_surface(false, false), S::Map);
        assert_eq!(super::primary_surface(true, false), S::Panel);
        assert_eq!(super::primary_surface(true, true), S::Drawer);
        assert_eq!(super::primary_surface(true, false), S::Panel);
    }

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

    #[test]
    fn stable_drawer_id_wins_over_legacy_title() {
        let chrome = crate::workspace::Chrome {
            drawer: Some("Settings".into()),
            drawer_id: Some("StormTable".into()),
            ..Default::default()
        };
        assert_eq!(super::window_from_chrome(&chrome), Some(super::AppWindow::StormTable));
    }
}
