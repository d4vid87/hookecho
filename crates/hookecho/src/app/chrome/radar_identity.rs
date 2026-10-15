//! Routine radar choices stay together on both desktop and phone surfaces.
use super::*;
use crate::ui::a11y::Named as _;

impl HookEchoApp {
    pub(crate) fn pane_identity(&self, idx: usize) -> String {
        let v = &self.views[idx];
        format!(
            "{} · {}",
            v.custom_product
                .as_deref()
                .unwrap_or_else(|| crate::products::name(v.moment, v.srv)),
            v.site.as_deref().unwrap_or("No site")
        )
    }

    pub(crate) fn radar_identity_controls(&mut self, ui: &mut egui::Ui) {
        let idx = self.active;
        let site = self.views[idx]
            .site
            .clone()
            .unwrap_or_else(|| "Choose radar".into());
        let place = wxdata::sites::site_by_id(&site).map(|s| format!("{}, {}", s.city, s.state));
        let phone = compact(ui.ctx());
        ui.spacing_mut().interact_size.y = if phone { 44.0 } else { 32.0 };
        if !phone
            && ui
                .add(
                    egui::Button::new(format!(
                        "{site}{} {}",
                        String::new(),
                        egui_phosphor::regular::CARET_DOWN
                    ))
                    .wrap()
                    .min_size(egui::vec2(0.0, 32.0)),
                )
                .named("Choose radar station")
                .on_hover_text(place.unwrap_or_else(|| site.clone()))
                .clicked()
        {
            self.apply_palette(PaletteAction::OpenWindow(AppWindow::Site), ui.ctx());
        }
        self.radar_product_controls(ui, false);
        if self.analyst_open {
            ui.weak(if self.link_cameras && self.views.len() == 4 {
                "Products/elevations: this pane · Station: linked panes"
            } else {
                "Products/elevations/station: this pane"
            });
        }
        if let Some(previous) = &self.views[idx].retained_station {
            ui.weak(format!(
                "Showing prior scan from {previous} while {site} loads"
            ));
        }
    }
    /// Shared product/elevation chooser for the compact header and radar workspace transport.
    pub(crate) fn radar_product_controls(&mut self, ui: &mut egui::Ui, full_name: bool) {
        let idx = self.active;
        let phone = compact(ui.ctx());
        let product = self.views[idx].custom_product.clone().unwrap_or_else(|| {
            crate::products::name(self.views[idx].moment, self.views[idx].srv).into()
        });
        let elevations = self.views[idx]
            .volume
            .as_ref()
            .map(|v| v.elevations.clone())
            .unwrap_or_default();
        let tilt = self.views[idx].tilt;
        ui.horizontal(|ui| {
            let mut pick = None;
            let menu = ui.menu_button(
                format!(
                    "{} {}",
                    if phone || full_name {
                        product.as_str()
                    } else if self.views[idx].srv && self.views[idx].moment == Moment::Velocity {
                        "SRV"
                    } else {
                        self.views[idx]
                            .custom_product
                            .as_deref()
                            .unwrap_or(crate::products::info(self.views[idx].moment).short)
                    },
                    egui_phosphor::regular::CARET_DOWN
                ),
                |ui| {
                    ui.set_min_width(220.0);
                    ui.spacing_mut().interact_size.y = 44.0;
                    let available = self.available_moments();
                    let mut products: Vec<_> = crate::products::PRODUCTS
                        .iter()
                        .map(|p| (p.moment, false, p.name))
                        .chain([(Moment::Velocity, true, "Storm-relative velocity")])
                        .filter(|(m, _, _)| available[m.index()])
                        .collect();
                    products.sort_by_key(|(m, relative, _)| {
                        !self
                            .settings
                            .favorite_fields
                            .contains(&radar_favorite_id(*m, *relative))
                    });
                    let mut favorites = false;
                    for (moment, relative, label) in products {
                        let favorite = self
                            .settings
                            .favorite_fields
                            .contains(&radar_favorite_id(moment, relative));
                        if favorite && !favorites {
                            ui.strong("Favorites");
                            favorites = true;
                        }
                        if !favorite && favorites {
                            ui.separator();
                            ui.strong("All radar products");
                            favorites = false;
                        }
                        ui.push_id(("radar_product", moment.index(), relative), |ui| {
                            ui.horizontal(|ui| {
                                if ui
                                    .selectable_label(
                                        self.views[idx].custom_product.is_none()
                                            && self.views[idx].moment == moment
                                            && (moment != Moment::Velocity
                                                || self.views[idx].srv == relative),
                                        label,
                                    )
                                    .clicked()
                                {
                                    pick = Some((moment, relative));
                                    ui.close();
                                }
                                if ui
                                    .add_sized(
                                        [44.0, 44.0],
                                        egui::Button::new(egui_phosphor::regular::STAR)
                                            .selected(favorite),
                                    )
                                    .named_toggle(&format!("Favorite {label}"), favorite)
                                    .clicked()
                                {
                                    self.apply_palette(
                                        PaletteAction::ToggleRadarFavorite(moment, relative),
                                        ui.ctx(),
                                    );
                                }
                            })
                        });
                    }
                    for p in &self.settings.radar_products {
                        if ui
                            .selectable_label(
                                self.views[idx].custom_product.as_deref() == Some(p.name.as_str()),
                                &p.name,
                            )
                            .clicked()
                        {
                            self.views[idx].custom_product = Some(p.name.clone());
                            ui.close();
                        }
                    }
                },
            );
            self.tour_anchors.product = Some(menu.response.rect);
            menu.response
                .named("Choose radar product")
                .on_hover_text(&product);
            if let Some((m, srv)) = pick {
                self.apply_palette(PaletteAction::SetMoment(m, srv), ui.ctx());
            }
            egui::ComboBox::from_id_salt(("identity_elevation", idx))
                .width(if phone { 78.0 } else { 90.0 })
                .selected_text(
                    elevations
                        .get(tilt)
                        .map_or_else(|| "Elevation…".into(), |a| format!("{a:.1}°")),
                )
                .show_ui(ui, |ui| {
                    ui.spacing_mut().interact_size.y = 44.0;
                    for (i, a) in elevations.iter().enumerate() {
                        if ui.selectable_label(i == tilt, format!("{a:.1}°")).clicked() {
                            self.views[idx].tilt = i;
                            self.views[idx].follow_low_cut = false;
                            self.views[idx].error = None;
                        }
                    }
                    if elevations.is_empty() {
                        ui.weak("Elevations appear when the scan loads.");
                    }
                })
                .response
                .on_hover_text("Choose radar elevation");
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn radar_favorites_distinguish_velocity_modes_and_survive_settings_roundtrip() {
        let velocity = radar_favorite_id(Moment::Velocity, false);
        let relative = radar_favorite_id(Moment::Velocity, true);
        assert_ne!(velocity, relative);
        assert_eq!(
            radar_favorite_id(Moment::Reflectivity, true),
            radar_favorite_id(Moment::Reflectivity, false)
        );
        let mut settings = crate::settings::Settings::default();
        settings.toggle_favorite_field(&relative);
        let restored: crate::settings::Settings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert!(restored.favorite_fields.contains(&relative));
        assert!(!restored.favorite_fields.contains(&velocity));
    }
}
