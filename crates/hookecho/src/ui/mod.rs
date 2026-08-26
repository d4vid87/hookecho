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
/// fits on. So on Android these become full-screen surfaces: pinned to the content rect (which
/// already excludes the system bars and grows to clear the keyboard), not draggable, not
/// resizable, not collapsible, scrolling their body, with square corners because there is nothing
/// behind them to round against.
///
/// What is left after B4b is the handful of surfaces that are not drawer pages: the first-run
/// card, the anchored map-click popovers, and the station cards. Every browsable tool now goes
/// through [`drawer::Drawer::page`], which owns its own chrome.
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

/// Hand this product to the platform's reader — Android's WebView, the desktop's browser.
///
/// Drawn only where there is one: the web build has neither, and keeps the monospace view it was
/// already showing. See [`crate::textview`] for why the desktop half is the system browser and not
/// an embedded webview.
pub(crate) fn reader_button(ui: &mut egui::Ui, title: &str, issued: &str, text: &str) {
    if !crate::textview::available() {
        return;
    }
    if ui
        .button(format!("{} Read", egui_phosphor::regular::BOOK_OPEN))
        .on_hover_text("Opens this product typeset for reading \u{2014} paragraphs instead of teletype columns")
        .clicked()
    {
        if let Err(e) = crate::textview::open(title, issued, text) {
            log::warn!("reader failed: {e}");
        }
    }
}

/// The "Copy CSV" / "Save CSV…" pair every table window ends up wanting. `csv` is only called
/// when a button is clicked, so building the text stays off the per-frame path.
pub(crate) fn csv_buttons(
    ui: &mut egui::Ui,
    default_name: &str,
    hover: &str,
    csv: impl Fn() -> String,
) {
    if ui.button("Save CSV…").on_hover_text(hover).clicked() {
        if let crate::dialog::Saved::Failed(e) =
            crate::dialog::save_bytes(default_name, "csv", csv().as_bytes())
        {
            log::warn!("CSV export failed: {e}");
        }
    }
    if ui.button("Copy CSV").on_hover_text(hover).clicked() {
        ui.ctx().copy_text(csv());
    }
}

/// Accessible names for icon-only chrome.
pub mod a11y;
pub mod about_window;
pub mod afd_window;
pub mod tropical_window;
pub mod alert_panel;
pub mod drawer;
pub mod basemap_picker;
pub mod cappi_window;
pub mod cell_window;
pub mod cells_window;
pub mod cheatsheet;
pub mod detail_window;
pub mod digest_window;
pub mod event_window;
pub mod firstrun;
pub mod forecast_window;
pub mod glossary;
pub mod help_hub;
pub mod hodograph_window;
pub mod layer_options;
pub mod layer_window;
pub mod layers_panel;
pub mod legend;
/// Material 3 design tokens for the mobile chrome.
pub mod m3;
pub mod marker_popup;
/// Easing and the reduced-motion brake.
pub mod motion;
pub mod marker_window;
pub mod palette_editor;
pub mod placefile_window;
pub mod popover;
pub mod rules_window;
pub mod sensor_window;
pub mod settings_window;
pub mod site_dialog;
pub mod sounding_window;
/// Live station telemetry cards.
pub mod station_card;
pub mod style;
/// The optional spotlight tour of the live chrome.
pub mod tour;
pub mod verify_window;
pub mod video_window;
pub mod volume3d_window;
pub mod warning_window;
pub mod xsection_window;
