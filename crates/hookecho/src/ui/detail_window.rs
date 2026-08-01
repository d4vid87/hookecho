//! Detail window shown when a map overlay feature (warning, watch, outlook, MD) is clicked.

/// The currently-open detail popup.
pub struct Detail {
    pub title: String,
    pub body: String,
    pub color: [u8; 4],
    /// Texture-cache key of a picture to show above the text (webcam stills). `None` for the
    /// text-only features this window was built for.
    pub image: Option<String>,
    /// `(label, url)` for a "see this on its own site" button. Windy's webcam terms require every
    /// image to link back to the camera's page, so for those this is not decoration.
    pub link: Option<(String, String)>,
}

/// Show the detail window. Returns `false` when it should close. `image` is the resolved texture
/// for `detail.image`, if it has finished loading.
pub fn show(ctx: &egui::Context, detail: &Detail, image: Option<&egui::TextureHandle>) -> bool {
    let mut open = true;
    crate::ui::fit_phone(ctx, egui::Window::new("Feature Details"))
        .open(&mut open)
        .default_size([380.0, 320.0])
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                let c = detail.color;
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                ui.painter()
                    .rect_filled(rect, 2.0, egui::Color32::from_rgb(c[0], c[1], c[2]));
                ui.heading(&detail.title);
            });
            ui.separator();
            if detail.image.is_some() {
                match image {
                    Some(tex) => {
                        let size = tex.size_vec2();
                        // Never wider than the image really is. Windy's webcam terms allow their
                        // stills at original size or smaller and forbid stretching, and their
                        // previews are often narrower than this window.
                        let w = ui.available_width().min(size.x);
                        let h = if size.x > 0.0 { w * size.y / size.x } else { 0.0 };
                        ui.add(egui::Image::new(tex).fit_to_exact_size(egui::vec2(w, h)));
                    }
                    // Offline, or the camera posted nothing recently: the text below still stands.
                    None => {
                        ui.weak("loading image\u{2026}");
                    }
                }
                ui.separator();
            }
            egui::ScrollArea::vertical().show(ui, |ui| {
                // Monospace keeps L3 attribute-table columns aligned.
                ui.add(egui::Label::new(egui::RichText::new(&detail.body).monospace()).wrap());
            });
            // A button rather than `ui.hyperlink_to`: Android needs the JNI ACTION_VIEW path in
            // platform::open_url, which egui's own hyperlink does not go through.
            if let Some((label, url)) = &detail.link {
                ui.add_space(6.0);
                if ui.button(label).clicked() {
                    if let Err(e) = crate::platform::open_url(url) {
                        log::warn!("could not open {url}: {e}");
                    }
                }
            }
        });
    open
}
