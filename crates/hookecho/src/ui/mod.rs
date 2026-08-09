//! egui UI: the drawer's layer list and options, site picker, settings window, color legend.

/// A spinner with a label. A bare spinner says "something is happening"; the label says what,
/// which is the difference between a professional app and a hung one.
pub(crate) fn loading(ui: &mut egui::Ui, what: &str) {
    ui.horizontal(|ui| {
        ui.spinner();
        ui.label(egui::RichText::new(what).weak());
    });
}

/// Turn a desktop tool window into a phone surface (no-op elsewhere).
///
/// Material 3's rule for the compact width class is that a secondary screen takes the whole
/// screen — no floating panel, no dialog margins, no dragging a window around a display it barely
/// fits on. So on Android every one of these windows becomes a full-screen surface: pinned to the
/// content rect (which already excludes the system bars and grows to clear the keyboard), not
/// draggable, not resizable, not collapsible, scrolling its body, with square corners because
/// there is nothing behind it to round against. The title bar stays: its ✕ is the surface's back
/// affordance, and it is already wired to each caller's `open` flag.
///
/// One function, because there is exactly one right answer for all twenty-odd of these windows,
/// and a per-window conversion would be twenty chances to get it slightly different.
///
/// ponytail: `Window` rather than a hand-rolled surface widget — it already does the title row,
/// the close button, the open flag and the scroll body, and reusing it keeps the desktop and
/// phone call sites identical.
pub(crate) fn phone_surface<'a>(ctx: &egui::Context, w: egui::Window<'a>) -> egui::Window<'a> {
    if cfg!(target_os = "android") {
        let r = ctx.content_rect();
        let frame = egui::Frame::window(&ctx.style_of(ctx.theme()))
            .corner_radius(0)
            .shadow(egui::epaint::Shadow::NONE)
            .inner_margin(egui::Margin::symmetric(
                crate::ui::m3::SP_4 as i8,
                crate::ui::m3::SP_3 as i8,
            ));
        // `fixed_rect` beats the `.default_size(…)` / `.anchor(…)` that most call sites chain on
        // after this — the old width-pinning trick had to fight those and lost more than once.
        w.fixed_rect(r)
            .resizable(false)
            .collapsible(false)
            .vscroll(true)
            .frame(frame)
    } else {
        w
    }
}

pub mod about_window;
pub mod afd_window;
pub mod alert_panel;
pub mod cappi_window;
pub mod cell_window;
pub mod cells_window;
pub mod cheatsheet;
pub mod detail_window;
pub mod digest_window;
pub mod event_window;
pub mod forecast_window;
pub mod hodograph_window;
pub mod layer_options;
pub mod layer_window;
pub mod layers_panel;
pub mod legend;
/// Material 3 design tokens for the mobile chrome.
pub mod m3;
pub mod marker_popup;
pub mod marker_window;
pub mod palette_editor;
pub mod placefile_window;
pub mod sensor_window;
pub mod settings_window;
pub mod site_dialog;
pub mod sounding_window;
/// Live station telemetry cards.
pub mod station_card;
pub mod style;
pub mod verify_window;
pub mod video_window;
pub mod volume3d_window;
pub mod warning_window;
pub mod wizard;
pub mod xsection_window;
