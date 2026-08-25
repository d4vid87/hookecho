//! The docked left sidebar and the button that collapses it.

use super::*;

impl HookEchoApp {
    /// The sidebar: everything that isn't the map.
    ///
    /// A docked left column holding the whole action registry (products, layers, tools,
    /// windows — searchable) with the app's own commands below it. It replaces a wall of parallel
    /// entry points; the map keeps nothing but transient status.
    pub(crate) fn sidebar(&mut self, root: &mut egui::Ui, ctx: &egui::Context) {
        crate::prof_scope!("sidebar");
        let accent = crate::theme::accent(self.settings.theme);
        let entries = self.palette_entries();
        let mut query = std::mem::take(&mut self.layers_query);
        let (mut chosen, mut fly_to) = (None, None);
        let mut opts = ui::layer_options::UiActions::default();
        let mut focus_search = std::mem::take(&mut self.sidebar_focus_search);
        let mut alerts_tab = self.show_alert_panel;
        let (alert_count, _) = self.alert_badge();
        let bounds = self.view_bounds();
        let feats = self.active_alert_features().to_vec();
        let mut muted = self.settings.mute_alerts;
        let mut alert_hit = None;
        // Read before the panel closure: the Layer options callback runs inside a `&mut self`
        // borrow and can only touch plain fields, not `&self` methods.
        let l3_site = self.l3grid_site.clone();
        let tz = self.active_tz();
        let mosaic = self.mosaic_status();
        let mut etop_dbz = self.settings.etop_dbz;
        let mut hide = false;
        // Tour spotlights, recorded from inside the panel (no `self` reachable in there).
        let mut alerts_rect = None;
        let sidebar_rect = egui::Panel::left("sidebar")
            .exact_size(264.0)
            .show(root, |ui| {
                // Title on the same line as the tabs: the name is branding, not a section, and
                // its own row plus separator cost 30 px of every screen height.
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Hook Echo-WX")
                            .size(13.0)
                            .strong()
                            .color(accent),
                    );
                    ui.add_space(4.0);
                    // Data | Alerts. `show_alert_panel` is the same flag the bell and the A hotkey
                    // flip, so every entry point lands on the same tab.
                    if ui.selectable_label(!alerts_tab, "Data").clicked() {
                        alerts_tab = false;
                    }
                    let label = if alert_count == 0 {
                        "Alerts".to_string()
                    } else {
                        format!("Alerts ({alert_count})")
                    };
                    let alerts_hit = ui.selectable_label(alerts_tab, label);
                    alerts_rect = Some(alerts_hit.rect);
                    if alerts_hit.clicked() {
                        alerts_tab = true;
                    }
                    // Collapse, on the row it collapses. The floating button that brings the
                    // panel back lands in the same corner this one sits in.
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new(egui_phosphor::regular::CARET_LEFT)
                                        .size(14.0),
                                )
                                .fill(egui::Color32::TRANSPARENT)
                                .stroke(egui::Stroke::NONE),
                            )
                            .on_hover_text("Hide the sidebar")
                            .clicked()
                        {
                            hide = true;
                        }
                    });
                });
                ui.separator();
                if alerts_tab {
                    alert_hit = ui::alert_panel::body(ui, &feats, bounds, &mut muted);
                    return;
                }
                self.product_section(ui, &mut opts);
                ui.separator();
                // A drag rewrites the order in place, so persist it when it moves.
                let order_was = self.settings.layer_order.clone();
                chosen = ui::layers_panel::body(
                    ui,
                    &entries,
                    &mut query,
                    accent,
                    // Leave room for the disclosures under the tree, whatever the window height.
                    (ui.available_height() - 110.0).max(120.0),
                    std::mem::take(&mut focus_search),
                    &mut self.settings.layer_order,
                    |ui| {
                        // Knobs for the layers that are already on, drawn between the Radar group
                        // and the rest. Collapsed by default: the list is still the panel's job.
                        egui::CollapsingHeader::new("Layer options")
                            .default_open(false)
                            .show(ui, |ui| {
                                let glm_options = self.show_glm
                                    || self.fields.get(&crate::render::FieldLayer::GlmFed).is_some_and(|s| s.show);
                                crate::ui::layer_options::show(
                                    ui,
                                    &mut self.filters,
                                    &mut self.fields,
                                    &mut self.rotation_minutes,
                                    &mut self.hrrr_fcst_hour,
                                    self.hrrr_valid,
                                    tz,
                                    &mut self.env_cape_ml,
                                    &mut self.env_srh_km,
                                    &mut self.env_model,
                                    &mut self.contour_kind,
                                    &mut etop_dbz,
                                    &mut self.snow_hours,
                                    &self.show_tropical,
                                    &mut self.tropical_wind_kt,
                                    &mut self.tropical_surge,
                                    l3_site.as_deref(),
                                    &mut self.global_model,
                                    &mut self.global_fcst_hour,
                                    &mut self.diff_field,
                                    self.diff_valid.as_ref(),
                                    &mut self.settings.lightning_minutes,
                                    glm_options,
                                    &mut self.settings.glm_goes_west,
                                    self.show_spotters,
                                    &mut self.settings.spotter_range_km,
                                    &mut self.settings.detectors,
                                    Some(mosaic.as_str()),
                                    &mut opts,
                                );
                            });
                    },
                );
                self.settings.etop_dbz = etop_dbz;
                if self.settings.layer_order != order_was {
                    self.settings.save();
                }
                // Place search folds in here rather than keeping a pill of its own: action
                // matches rank first, and this row is the explicit "I meant a place" answer.
                if !query.trim().is_empty() {
                    ui.add_space(4.0);
                    let w = ui.available_width();
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new(format!(
                                    "{}  Fly to \u{201c}{}\u{201d}",
                                    egui_phosphor::regular::MAP_PIN,
                                    query.trim()
                                ))
                                .size(13.0),
                            )
                            .min_size(egui::vec2(w, 34.0))
                            .corner_radius(10.0),
                        )
                        .on_hover_text("Search the place name and move the map there")
                        .clicked()
                    {
                        fly_to = Some(query.trim().to_string());
                    }
                }
                ui.add_space(4.0);
                // The set-once map knobs, and the app's own commands.
                egui::CollapsingHeader::new("Map")
                    .default_open(false)
                    .show(ui, |ui| self.map_rows(ui, &mut opts));
                egui::CollapsingHeader::new("App")
                    .default_open(false)
                    .show(ui, |ui| self.app_rows(ui));
            })
            .response
            .rect;
        self.tour_anchors.menu = Some(sidebar_rect);
        self.tour_anchors.alerts = alerts_rect;
        self.show_alert_panel = alerts_tab;
        self.settings.mute_alerts = muted;
        if hide {
            self.settings.hide_sidebar = true;
            self.settings.save();
        }
        if let Some((id, lon, lat)) = alert_hit {
            // Fly the active camera to the alert and open its bulletin.
            let cam = &mut self.views[self.active].camera;
            cam.center = crate::render::mercator::lonlat_to_world(lon, lat);
            cam.zoom = cam.zoom.max(8.0);
            self.open_alert_popup(&id);
        }
        // `query` is the live text; `self.layers_query` was taken from at the top of the frame.
        let searched = !query.trim().is_empty();
        self.layers_query = query;
        self.apply_ui_actions(opts, ctx);
        if let Some(a) = chosen {
            // Picking a search hit means you're done searching: clear the query so the tree comes
            // back. Browsing the list without a query is the opposite — you're flipping layers on
            // and off, so it stays put.
            if searched || matches!(a, PaletteAction::OpenWindow(_)) {
                self.layers_query.clear();
            }
            self.apply_palette(a, ctx);
        }
        if let Some(place) = fly_to {
            self.geocode_nav = true;
            self.save_offer = None; // a new search retires the previous offer
            self.place_status = Some(("Searching…".to_string(), Instant::now()));
            let http = self.http.clone();
            let tx = self.geocode_tx.clone();
            let ctx2 = ctx.clone();
            self.spawner.spawn(async move {
                let _ = tx.send(wxdata::geocode::search(&http, &place).await);
                ctx2.request_repaint();
            });
        }
    }

    /// Bottom-right info chip: zoom, cursor position, DVR depth, and the active tool's hint.
    ///
    /// This is what the docked status bar used to hold. A full-width bar for four short readouts
    /// cost a strip of map on every frame; the chip floats over the map instead, and the site and
    /// volume time it also carried now live in the timeline pill where the clock belongs.
    /// The way back to a hidden sidebar: one floating button in the corner it used to occupy.
    /// Nothing else on the map reaches the layer list, so this is not optional chrome.
    pub(crate) fn sidebar_button(&mut self, ctx: &egui::Context) {
        if !self.settings.hide_sidebar {
            return;
        }
        egui::Area::new(egui::Id::new("sidebar_button"))
            .constrain_to(self.chrome_rect)
            .anchor(egui::Align2::LEFT_TOP, egui::vec2(10.0, 10.0))
            .show(ctx, |ui| {
                crate::ui::style::glass(ui, 238).show(ui, |ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new(egui_phosphor::regular::LIST).size(18.0),
                            )
                            .min_size(egui::vec2(30.0, 30.0))
                            .fill(egui::Color32::TRANSPARENT)
                            .stroke(egui::Stroke::NONE),
                        )
                        .on_hover_text("Show the sidebar")
                        .clicked()
                    {
                        self.settings.hide_sidebar = false;
                        self.settings.save();
                    }
                });
            });
    }
}
