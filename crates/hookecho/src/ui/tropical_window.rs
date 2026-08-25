//! NHC text products for an active storm: the public advisory and the forecast discussion.
//!
//! The app drew the cone, the track and the wind field but carried none of the words that come
//! with them. The discussion is where the hurricane specialist says what the guidance disagrees
//! about and how confident the track really is — the part a cone cannot express.

use wxdata::tropical::{Advisory, TropicalStorm};

/// Which text product to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Product {
    /// Public advisory (TCP): watches, warnings, hazards, the numbers.
    Advisory,
    /// Forecast discussion (TCD): the forecaster's reasoning.
    Discussion,
}

impl Product {
    pub fn label(self) -> &'static str {
        match self {
            Product::Advisory => "Advisory",
            Product::Discussion => "Discussion",
        }
    }

    /// This product's page for `storm`, if the feed published one.
    pub fn url(self, storm: &TropicalStorm) -> Option<&str> {
        match self {
            Product::Advisory => storm.advisory_url.as_deref(),
            Product::Discussion => storm.discussion_url.as_deref(),
        }
    }
}

/// What the window is currently pointed at.
pub struct TropicalWindow {
    pub open: bool,
    /// Storm id the text belongs to; `None` until one is chosen.
    pub storm_id: Option<String>,
    pub product: Product,
    pub text: Option<Advisory>,
    pub busy: bool,
    pub error: Option<String>,
}

impl Default for TropicalWindow {
    fn default() -> Self {
        Self {
            open: false,
            storm_id: None,
            product: Product::Discussion,
            text: None,
            busy: false,
            error: None,
        }
    }
}

/// Show the window. Returns `Some((storm id, product))` when the user asks for a fetch — either
/// by picking a different storm or product, or by hitting refresh.
pub fn show(
    w: &mut TropicalWindow,
    ctx: &egui::Context,
    storms: &[TropicalStorm],
) -> Option<(String, Product)> {
    if !w.open {
        return None;
    }
    let mut want: Option<(String, Product)> = None;
    let mut open = w.open;
    crate::ui::phone_surface(ctx, egui::Window::new("Tropical products"))
        .open(&mut open)
        .default_size([600.0, 540.0])
        .show(ctx, |ui| {
            if storms.is_empty() {
                ui.weak("No active tropical cyclones.");
                ui.small("The NHC publishes these only while a storm is being advised on.");
                return;
            }
            ui.horizontal_wrapped(|ui| {
                for s in storms {
                    let selected = w.storm_id.as_deref() == Some(s.id.as_str());
                    if ui
                        .selectable_label(selected, format!("{} {}", s.classification, s.name))
                        .clicked()
                        && !selected
                    {
                        w.storm_id = Some(s.id.clone());
                        want = Some((s.id.clone(), w.product));
                    }
                }
            });
            ui.horizontal(|ui| {
                for p in [Product::Discussion, Product::Advisory] {
                    if ui.selectable_label(w.product == p, p.label()).clicked() && w.product != p {
                        w.product = p;
                        if let Some(id) = w.storm_id.clone() {
                            want = Some((id, p));
                        }
                    }
                }
                if w.busy {
                    crate::ui::loading(ui, "Fetching…");
                } else if ui.button("⟳ Refresh").clicked() {
                    if let Some(id) = w.storm_id.clone() {
                        want = Some((id, w.product));
                    }
                }
            });
            if let Some(e) = &w.error {
                ui.colored_label(egui::Color32::from_rgb(230, 90, 90), e);
            }
            ui.separator();
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| match &w.text {
                    // The products are column-formatted plain text; monospace or they lose it.
                    Some(a) => {
                        ui.add(
                            egui::Label::new(egui::RichText::new(&a.text).monospace().size(12.0))
                                .wrap(),
                        );
                    }
                    None if w.busy => {
                        ui.weak("Fetching…");
                    }
                    None => {
                        ui.weak("Pick a storm to read its latest product.");
                    }
                });
        });
    w.open = open;
    want
}
