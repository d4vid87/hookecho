//! egui UI: the drawer's layer list and options, site picker, settings window, color legend.

/// Clamp a floating window to the screen on Android (no-op elsewhere): egui windows size to
/// their content, and desktop-sized content overflows a ~360-pt-wide portrait phone display.
/// Height overflow gets a scrollbar instead of a clip. The window is pinned near the top so its
/// first fields stay visible when the soft keyboard covers the lower ~40% of the screen (the
/// NativeActivity draws edge-to-edge, so the IME doesn't resize the content rect). Callers that
/// set their own `.anchor` after this override the pin.
pub(crate) fn fit_phone<'a>(ctx: &egui::Context, w: egui::Window<'a>) -> egui::Window<'a> {
    if cfg!(target_os = "android") {
        let r = ctx.content_rect();
        // `resizable(false)` is load-bearing: a resizable window keeps a stored width from a wider
        // session (or grows to its content) and spills off both screen edges — the reported cell
        // popup clipping. Pinning width + disabling resize keeps every popup inside the screen.
        //
        // min == max width, not `max_width` alone, because every call site chains
        // `.default_size(…)` AFTER this — and on a non-resizable window that wider desired size
        // won, which is why the 560 pt sounding window and the 720 pt attributes table hung off
        // both edges of a 512 pt screen. Pinning both bounds leaves a later default nothing to
        // override.
        w.resizable(false)
            // The budget is for the *inner* width; the window frame's margins, stroke and shadow
            // sit outside it, so leaving only 16 px still hung the frame off both edges.
            .min_width(r.width() - 56.0)
            .max_width(r.width() - 56.0)
            .max_height(r.height() * 0.80)
            .vscroll(true)
            .anchor(egui::Align2::CENTER_TOP, [0.0, r.top() + 6.0])
    } else {
        w
    }
}

pub mod afd_window;
pub mod alert_panel;
pub mod cappi_window;
pub mod cell_window;
pub mod cells_window;
pub mod detail_window;
pub mod digest_window;
pub mod event_window;
pub mod forecast_window;
pub mod hodograph_window;
pub mod layer_window;
pub mod layers_panel;
pub mod legend;
pub mod marker_popup;
pub mod marker_window;
pub mod palette_editor;
pub mod placefile_window;
pub mod sensor_window;
pub mod settings_window;
pub mod site_dialog;
pub mod sounding_window;
pub mod style;
pub mod layer_options;
pub mod verify_window;
pub mod volume3d_window;
pub mod warning_window;
pub mod wizard;
pub mod xsection_window;
