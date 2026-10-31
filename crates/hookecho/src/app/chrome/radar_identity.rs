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
        if self.views[idx].basemap.ndfd_layer().is_some() && !self.views[idx].show_radar {
            self.ndfd_forecast_controls(ui);
            return;
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
    /// The NDFD map is a forecast source, so a radar moment/elevation picker would be misleading.
    fn ndfd_forecast_controls(&mut self, ui: &mut egui::Ui) {
        let idx = self.active;
        let current = self.views[idx].basemap;
        let mut pick = None;
        ui.menu_button(format!("{} {}", current.label(), egui_phosphor::regular::CARET_DOWN), |ui| {
            ui.set_min_width(220.0);
            ui.spacing_mut().interact_size.y = 44.0;
            for style in [
                crate::tiles::BasemapStyle::NdfdRainChance,
                crate::tiles::BasemapStyle::NdfdMaxTemp,
                crate::tiles::BasemapStyle::NdfdMinTemp,
                crate::tiles::BasemapStyle::NdfdAlaskaRainChance,
                crate::tiles::BasemapStyle::NdfdAlaskaMaxTemp,
                crate::tiles::BasemapStyle::NdfdAlaskaMinTemp,
                crate::tiles::BasemapStyle::NdfdHawaiiRainChance,
                crate::tiles::BasemapStyle::NdfdHawaiiMaxTemp,
                crate::tiles::BasemapStyle::NdfdHawaiiMinTemp,

            ] {
                if ui.selectable_label(style == current, style.label()).clicked() {
                    pick = Some(style);
                    ui.close();
                }
            }
        }).response.named("Choose NDFD forecast product");
        if let Some(style) = pick {
            self.views[idx].show_radar = false;
            self.set_basemap(style);
        }
    }
    /// Active-pane product, elevation and appearance choices for every radar toolbar.
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
        let available_tilts = self.views[idx]
            .volume
            .as_ref()
            .map(|v| v.available_tilts(self.views[idx].moment))
            .unwrap_or_default();
        let tilt = self.views[idx].tilt;
        ui.horizontal(|ui| {
            let mut pick = None;
            let menu = egui::containers::menu::MenuButton::from_button(egui::Button::new(format!(
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
            )).min_size(egui::vec2(0.0, 44.0)).wrap())
            .config(
                egui::containers::menu::MenuConfig::new()
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside),
            )
            .ui(ui, |ui| {
                let width = (ui.ctx().content_rect().width() - 48.0).clamp(220.0, 360.0);
                ui.set_min_width(width);
                ui.set_max_width(width);
                ui.spacing_mut().interact_size.y = 44.0;
                ui.strong(format!(
                    "Pane {} · {}",
                    idx + 1,
                    self.views[idx].site.as_deref().unwrap_or("No site")
                ));
                ui.label(&product);
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt(("radar_product_menu", idx))
                    .max_height((ui.ctx().content_rect().height() - 180.0).clamp(120.0, 560.0))
                    .show(ui, |ui| {
                        ui.strong("Elevation");
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
                                    if !available_tilts.contains(&i) {
                                        continue;
                                    }
                                    if ui.selectable_label(i == tilt, format!("{a:.1}°")).clicked()
                                    {
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
                        ui.collapsing("Appearance", |ui| self.radar_appearance_controls(ui));
                        ui.separator();
                        ui.strong("Products");
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
                                        .add_sized(
                                            [
                                                (ui.available_width()
                                                    - 44.0
                                                    - ui.spacing().item_spacing.x)
                                                    .max(80.0),
                                                44.0,
                                            ],
                                            egui::Button::new(label).wrap().selected(
                                                self.views[idx].custom_product.is_none()
                                                    && self.views[idx].moment == moment
                                                    && (moment != Moment::Velocity
                                                        || self.views[idx].srv == relative),
                                            ),
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
                                .add_sized(
                                    [ui.available_width(), 44.0],
                                    egui::Button::new(&p.name).wrap().selected(
                                        self.views[idx].custom_product.as_deref()
                                            == Some(p.name.as_str()),
                                    ),
                                )
                                .clicked()
                            {
                                self.views[idx].custom_product = Some(p.name.clone());
                                ui.close();
                            }
                        }
                    });
            });
            self.tour_anchors.product = Some(menu.0.rect);
            menu.0.named("Choose radar product").on_hover_text(&product);
            if let Some((m, srv)) = pick {
                self.apply_palette(PaletteAction::SetMoment(m, srv), ui.ctx());
            }
        });
    }

    /// The active pane uses the same appearance controls in settings and its product picker.
    #[inline(never)]
    pub(crate) fn radar_appearance_controls(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().interact_size.y = 44.0;
        ui.label(egui::RichText::new("Radar appearance").strong());
        let mut smooth = self.views[self.active].smooth;
        ui.horizontal(|ui| {
            let width = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
            for (label, value, hint) in [
                ("Crisp", false, "Show each radar gate with a sharp edge"),
                ("Smooth", true, "Blend neighboring radar gates"),
            ] {
                if ui
                    .add_sized(
                        egui::vec2(width, 44.0),
                        egui::Button::new(label)
                            .selected(smooth == value)
                            .corner_radius(9.0),
                    )
                    .on_hover_text(hint)
                    .clicked()
                {
                    smooth = value;
                }
            }
        });
        self.views[self.active].smooth = smooth;
        let pane = &mut self.views[self.active];
        let palette_moment = pane
            .custom_product
            .as_ref()
            .and_then(|name| {
                self.settings
                    .radar_products
                    .iter()
                    .find(|p| &p.name == name)
            })
            .and_then(|p| Moment::from_code(&p.palette))
            .unwrap_or(pane.moment);
        ui.checkbox(&mut pane.appearance.precision, "Higher-precision radar display").on_hover_text("Preserve finer native values in the display texture; uses additional memory. Available for native moments, KDP and gate-local custom products. Native probes remain unchanged.");
        aurora_frame().show(ui, |ui| {
            radar_opacity_contents(ui, &mut pane.appearance.opacity, false);
        });
        if smooth {
            ui.checkbox(&mut pane.appearance.show_smoothed_range_folding, "Show range-folded areas")
                .on_hover_text("Keep ambiguous radar returns in their range-fold color instead of blending valid neighbors over them.");
            ui.checkbox(&mut pane.appearance.multipass, "Two-stage smoothing").on_hover_text("Adds separable gate and radial smoothing before interpolation. Missing and folded gates stay classified; native probes are unchanged. Caches extra GPU data.");
            ui.add(egui::Slider::new(&mut pane.appearance.blend_width, 0.05..=1.0)
                .text("Blend width")).on_hover_text("Narrow keeps gate edges crisp; wide uses full bilinear blending. Native probes are unchanged.");
        }
        ui.checkbox(&mut pane.appearance.fade_old_sectors, "Fade retained sectors from the previous scan").on_hover_text("Live radar only: older sectors remain visible at 35% opacity until replaced. Measurements and archive playback are unchanged.");
        let moment = palette_moment;
        let key = moment.short_name().to_string();
        let current = if pane.appearance.palettes.contains_key(&key) {
            "Pane palette"
        } else {
            "App palette"
        };
        ui.menu_button(format!("{current} · Change…"), |ui| {
            if ui.button("Follow app palette").clicked() {
                pane.appearance.palettes.remove(&key);
                ui.close();
            }
            if ui.button("Keep current app palette in this pane").clicked() {
                pane.appearance.palettes.insert(
                    key.clone(),
                    crate::colormap::effective_table(&self.palettes, moment, self.settings.theme),
                );
                ui.close();
            }
            if ui.button("Built-in default").clicked() {
                pane.appearance
                    .palettes
                    .insert(key.clone(), crate::colormap::default_table(moment).clone());
                ui.close();
            }
            for name in crate::colormap::alt_names(moment) {
                if ui.button(name).clicked() {
                    if let Some(table) = crate::colormap::builtin_alt(name) {
                        pane.appearance.palettes.insert(key.clone(), table);
                    }
                    ui.close();
                }
            }
        });
        ui.label("Appearance applies to the active pane and is saved with its workspace.");
    }

    /// The same per-pane control is reachable without opening the product picker.
    pub(super) fn radar_opacity_menu(&mut self, ui: &mut egui::Ui) {
        let button = ui.add_sized([68.0, 44.0], egui::Button::new("Opacity"))
            .named("Radar opacity").on_hover_text("Fade the active pane's radar");
        egui::Popup::menu(&button)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .frame(aurora_frame())
            .show(|ui| {
                ui.set_width((ui.ctx().content_rect().width() - 64.0).clamp(200.0, 300.0));
                radar_opacity_contents(ui, &mut self.views[self.active].appearance.opacity, true);
            });
    }
}

fn aurora_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(egui::Color32::from_rgba_unmultiplied(39, 28, 55, 235))
        .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(187, 134, 255)))
        .corner_radius(20.0)
        .inner_margin(16)
        .shadow(egui::epaint::Shadow {
            offset: [0, 0], blur: 18, spread: 1,
            color: egui::Color32::from_rgba_unmultiplied(173, 117, 255, 28),
        })
}

/// Scope the purple styling so it never recolors neighboring product controls.
fn radar_opacity_contents(ui: &mut egui::Ui, opacity: &mut f32, close: bool) {
    ui.scope(|ui| {
        let purple = egui::Color32::from_rgb(201, 160, 255);
        let text = egui::Color32::from_rgb(241, 234, 255);
        ui.visuals_mut().override_text_color = Some(text);
        ui.visuals_mut().selection.bg_fill = egui::Color32::from_rgb(112, 69, 159);
        ui.visuals_mut().selection.stroke = egui::Stroke::new(1.0, text);
        ui.visuals_mut().widgets.inactive.bg_fill = egui::Color32::from_rgb(57, 43, 75);
        ui.visuals_mut().widgets.inactive.weak_bg_fill = egui::Color32::from_rgb(57, 43, 75);
        ui.visuals_mut().widgets.hovered.bg_fill = egui::Color32::from_rgb(88, 58, 117);
        ui.visuals_mut().widgets.active.bg_fill = purple;
        ui.visuals_mut().widgets.active.fg_stroke = egui::Stroke::new(2.0, text);
        ui.spacing_mut().item_spacing.y = 10.0;
        ui.spacing_mut().interact_size.y = 44.0;
        ui.horizontal(|ui| {
            ui.strong("Radar appearance");
            if close {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add_sized([44.0, 44.0], egui::Button::new("×"))
                        .named("Close radar opacity").clicked() { ui.close(); }
                });
            }
        });
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Radar opacity");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new(format!("{:.0}%", *opacity * 100.0))
                    .size(28.0).color(purple));
            });
        });
        ui.spacing_mut().slider_width = ui.available_width();
        ui.scope(|ui| {
            ui.visuals_mut().widgets.inactive.bg_fill = text;
            ui.visuals_mut().widgets.hovered.bg_fill = text;
            let response = ui.add(egui::Slider::new(opacity, 0.0..=1.0)
                .show_value(false).step_by(0.01).trailing_fill(true)
                .handle_shape(egui::style::HandleShape::Circle))
                .named("Radar opacity percentage")
                .on_hover_text("Fade radar to reveal the map. Measurements remain unchanged.");
            if response.drag_started() || response.dragged() {
                response.request_focus();
            }
        });
        ui.horizontal(|ui| {
            ui.small("Clear map · 0%");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.small("Full radar · 100%");
            });
        });
        ui.horizontal(|ui| {
            let width = (ui.available_width() - 3.0 * ui.spacing().item_spacing.x) / 4.0;
            for percent in [25, 50, 75, 100] {
                let value = percent as f32 / 100.0;
                if ui.add_sized([width, 44.0],
                    egui::Button::new(format!("{percent}%"))
                        .selected((*opacity - value).abs() < 0.005))
                    .named(&format!("Set radar opacity to {percent}%")).clicked() {
                    *opacity = value;
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.small("Map labels stay clear");
            if ui.button("Reset · 100%").named("Reset radar opacity").clicked() {
                *opacity = 1.0;
            }
        });
    });
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

    #[test]
    fn aurora_presets_and_reset_update_the_same_opacity() {
        let ctx = egui::Context::default();
        let mut opacity = 1.0;
        let mut draw = |events| {
            ctx.run_ui(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO, egui::vec2(390.0, 844.0))),
                events, ..Default::default()
            }, |ui| {
                ui.set_width(300.0);
                aurora_frame().show(ui, |ui| {
                    radar_opacity_contents(ui, &mut opacity, false);
                });
            })
        };
        let mut output = draw(Vec::new());
        for label in ["25%", "50%", "75%", "100%", "25%", "Reset · 100%"] {
            let pos = output.shapes.iter().find_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape {
                    (text.galley.text() == label)
                        .then_some(text.pos + text.galley.rect.center().to_vec2())
                } else { None }
            }).unwrap_or_else(|| panic!("missing control: {label}"));
            let _ = draw(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton { pos, button: egui::PointerButton::Primary,
                    pressed: true, modifiers: egui::Modifiers::default() },
                egui::Event::PointerButton { pos, button: egui::PointerButton::Primary,
                    pressed: false, modifiers: egui::Modifiers::default() },
            ]);
            // Read the next rendered percentage; controls must update the bound value.
            output = draw(Vec::new());
            let expected = if label.starts_with("Reset") { "100%" } else { label };
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Text(text) if text.galley.text() == expected
                    && text.galley.rect.height() > 25.0)));
        }
        assert_eq!(opacity, 1.0);
    }

    #[test]
    fn aurora_slider_takes_keyboard_focus_after_drag() {
        let ctx = egui::Context::default();
        let mut opacity = 0.5;
        let mut draw = |events| {
            ctx.run_ui(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO, egui::vec2(390.0, 844.0))),
                events, ..Default::default()
            }, |ui| {
                ui.set_width(300.0);
                radar_opacity_contents(ui, &mut opacity, false);
            })
        };
        let output = draw(Vec::new());
        let pos = output.shapes.iter().find_map(|shape| {
            if let egui::Shape::Circle(circle) = &shape.shape {
                (circle.radius > 10.0).then_some(circle.center)
            } else { None }
        }).expect("slider thumb");
        let _ = draw(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary,
                pressed: true, modifiers: egui::Modifiers::default() },
        ]);
        let _ = draw(vec![egui::Event::PointerButton {
            pos, button: egui::PointerButton::Primary, pressed: false,
            modifiers: egui::Modifiers::default(),
        }]);
        assert!(ctx.memory(|memory| memory.focused().is_some()));
        let _ = draw(vec![egui::Event::Key {
            key: egui::Key::ArrowRight, physical_key: None, pressed: true,
            repeat: false, modifiers: egui::Modifiers::default(),
        }]);
        assert!((opacity - 0.51).abs() < 0.001);
    }
}
