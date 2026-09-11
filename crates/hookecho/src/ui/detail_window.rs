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
pub fn show(
    ctx: &egui::Context,
    detail: &Detail,
    image: Option<&egui::TextureHandle>,
    popovers: &mut crate::ui::popover::Popovers,
) -> bool {
    let mut open = true;
    let mut close = false;
    popovers
        .card(ctx, "detail", egui::Window::new("Feature Details"))
        .open(&mut open)
        .frame(crate::ui::popover::glass_frame())
        .collapsible(false)
        .title_bar(false)
        // Preserve table width for non-outage products.
        .default_size(if outage_summary(&detail.body).is_some() {
            [420.0, 320.0]
        } else {
            [560.0, 420.0]
        })
        .show(ctx, |ui| {
            ui.visuals_mut().override_text_color = Some(egui::Color32::from_rgb(225, 234, 244));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                close = ui.button("Close ×").clicked();
            });
            if let Some((place, count, rest)) = outage_summary(&detail.body) {
                ui.label(
                    egui::RichText::new(format!(
                        "{}  POWER OUTAGE",
                        egui_phosphor::regular::LIGHTNING
                    ))
                    .color(egui::Color32::from_rgb(246, 194, 105))
                    .strong(),
                );
                ui.add_space(12.0);
                ui.label(egui::RichText::new(place).size(23.0));
                ui.label(egui::RichText::new(count).size(46.0).strong());
                ui.label("Customers without power");
                ui.add_space(12.0);
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let mut lines = rest.lines();
                    let tier = lines.next().unwrap_or_default();
                    let stats = lines.next().unwrap_or_default();
                    ui.horizontal_wrapped(|ui| {
                        for stat in stats.split(", ") {
                            ui.label(egui::RichText::new(stat).size(18.0).strong());
                            ui.separator();
                        }
                        ui.label(egui::RichText::new(tier).size(16.0));
                    });
                    ui.add_space(10.0);
                    for line in lines {
                        ui.add(egui::Label::new(egui::RichText::new(line).size(13.0)).wrap());
                    }
                });
                return;
            }
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
                        let h = if size.x > 0.0 {
                            w * size.y / size.x
                        } else {
                            0.0
                        };
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
    open && !close
}

// ODIN's existing display format; other feature bodies retain their table formatting.
fn outage_summary(body: &str) -> Option<(&str, &str, String)> {
    if !body.contains("Source: ODIN (DOE/ORNL)") {
        return None;
    }
    let (place, remainder) = body.split_once('\n')?;
    let (count, remainder) = remainder.split_once(" customers without power ")?;
    let (tier, rest) = remainder.split_once('\n')?;
    Some((
        place,
        count,
        format!("{}\n{}", tier.trim_matches(['(', ')']), rest),
    ))
}

#[cfg(test)]
mod tests {
    #[test]
    fn outage_card_preserves_counts_caveats_and_optional_details() {
        let body = "Harris, Texas\n1,205 customers without power (scattered)\n15 incidents, 1 utility\nCause: storm\n\nSource: ODIN (DOE/ORNL) — participating utilities only.";
        let (place, count, rest) = super::outage_summary(body).unwrap();
        assert_eq!((place, count), ("Harris, Texas", "1,205"));
        assert!(rest.contains("15 incidents, 1 utility"));
        assert!(rest.contains("Cause: storm"));
        assert!(rest.contains("participating utilities only"));
        assert!(super::outage_summary("A different feature").is_none());
    }
}
