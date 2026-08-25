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
/// ponytail: one arm per converted page. B4b converts the rest of `ui/*_window.rs`; each one adds
/// its line here, and a title this build doesn't know simply doesn't reopen.
pub(crate) fn window_for_page(title: &str) -> Option<AppWindow> {
    Some(match title {
        "Settings" => AppWindow::Settings,
        "Event Library" => AppWindow::Events,
        "Alert rules" => AppWindow::AlertRules,
        "Help" => AppWindow::Help,
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
        // A page from a newer build, or one that isn't a window at all: skipped, not fatal.
        assert_eq!(super::window_for_page("Storm 42 Attributes"), None);
    }
}
