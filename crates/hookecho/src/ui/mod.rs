//! egui UI: the radar toolbox, site picker, settings window, and color legend.

/// Clamp a floating window to the screen on Android (no-op elsewhere): egui windows size to
/// their content, and desktop-sized content overflows a ~360-pt-wide portrait phone display.
/// Height overflow gets a scrollbar instead of a clip.
pub(crate) fn fit_phone<'a>(ctx: &egui::Context, w: egui::Window<'a>) -> egui::Window<'a> {
    if cfg!(target_os = "android") {
        let r = ctx.content_rect();
        w.max_width(r.width() - 12.0)
            .max_height(r.height() * 0.82)
            .vscroll(true)
    } else {
        w
    }
}

pub mod afd_window;
pub mod alert_panel;
pub mod cappi_window;
pub mod cell_window;
pub mod detail_window;
pub mod digest_window;
pub mod event_window;
pub mod hodograph_window;
pub mod legend;
pub mod marker_window;
pub mod palette_editor;
pub mod placefile_window;
pub mod sensor_window;
pub mod settings_window;
pub mod site_dialog;
pub mod sounding_window;
pub mod toolbox;
pub mod volume3d_window;
pub mod warning_window;
pub mod wizard;
pub mod xsection_window;
