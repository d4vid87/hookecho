//! What you get when you tap one of your own markers on the map: rename it, or get rid of it.
//!
//! Removal used to live only in the Location Markers window, which you had to know about — so a
//! marker dropped by a stray tap was effectively permanent. Acting on the thing you tapped is the
//! obvious gesture, and it's the same one on a phone.

use crate::settings::Marker;

/// What the user asked for this frame.
#[derive(Default)]
pub struct MarkerPopupResult {
    /// The window is still open (false = closed via the title-bar ✖ or a button below).
    pub open: bool,
    pub remove: bool,
    /// Open the full Location Markers manager (icons, coordinates, the whole list).
    pub manage: bool,
}

/// Show the popup for `m`, editing its name in place.
pub fn show(ctx: &egui::Context, m: &mut Marker) -> MarkerPopupResult {
    let mut r = MarkerPopupResult {
        open: true,
        ..Default::default()
    };
    crate::ui::phone_surface(ctx, egui::Window::new("Marker"))
        .open(&mut r.open)
        .resizable(false)
        // Three fixed rows: nothing to scroll, and `phone_surface`'s vscroll would otherwise stretch
        // the frame to its 80%-of-screen cap and leave most of it empty on a phone.
        .vscroll(false)
        .default_width(260.0)
        .show(ctx, |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut m.name)
                    .hint_text("Name")
                    .desired_width(ui.available_width()),
            );
            ui.weak(format!("{:.4}, {:.4}", m.lat, m.lon));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui
                    .button("✖ Remove")
                    .on_hover_text("Delete this marker")
                    .clicked()
                {
                    r.remove = true;
                }
                if ui.button("All markers…").clicked() {
                    r.manage = true;
                }
            });
        });
    r
}
