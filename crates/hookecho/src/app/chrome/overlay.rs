//! The floating map-first chrome: search pill, right-edge control column, and the panels that
//! slide over the map (layers/alerts, basemap).
//!
//! The map runs edge to edge underneath all of it. Nothing here is docked, so a closed panel
//! costs the map nothing.

use super::*;

/// The floating panel's geometry: left margin, top offset (clear of the search pill), width.
const PANEL_X: f32 = 10.0;
const PANEL_TOP: f32 = 58.0;
const PANEL_W: f32 = 300.0;
/// The control column sits inboard of the pane's color scale (`ui::legend`: a 16 px bar, its
/// inset, and the value labels to its left), so the two never share pixels.
const CONTROLS: egui::Vec2 = egui::vec2(-70.0, 44.0);
const RIGHT_PANEL: egui::Vec2 = egui::vec2(-130.0, 44.0);
/// What the scrubber pill needs along the bottom edge, plus a margin.
const SCRUBBER_CLEARANCE: f32 = 84.0;

impl HookEchoApp {
    /// The main panel: everything that isn't the map, floating over the map's left edge.
    ///
    /// Holds the whole action registry (products, layers, tools, windows — searchable) with the
    /// app's own commands below it, plus the alerts tab. Closed by default; the search pill and
    /// the control column are the ways in.
    pub(crate) fn panel(&mut self, ctx: &egui::Context) {
        if !self.panel_open || self.drawer.is_open() {
            return;
        }
        crate::prof_scope!("panel");
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
        // Height budget: the search pill above, the scrubber pill below (which is centred and
        // grows with the window, so on a narrow one it would otherwise run under this panel).
        let max_h = (self.chrome_rect.height() - PANEL_TOP - SCRUBBER_CLEARANCE).max(160.0);
        let panel_rect = egui::Area::new(egui::Id::new("panel"))
            .constrain_to(self.chrome_rect)
            .anchor(egui::Align2::LEFT_TOP, egui::vec2(PANEL_X, PANEL_TOP))
            .show(ctx, |ui| {
                // Denser than the chips: this one carries a wall of small text, and a basemap
                // label showing through a list row is unreadable, not tasteful.
                crate::ui::style::glass(ui, 250).show(ui, |ui| {
                    ui.set_width(PANEL_W);
                    ui.set_max_height(max_h);
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
                                        egui::RichText::new(egui_phosphor::regular::X).size(14.0),
                                    )
                                    .fill(egui::Color32::TRANSPARENT)
                                    .stroke(egui::Stroke::NONE),
                                )
                                .on_hover_text("Close this panel")
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
                                        || self
                                            .fields
                                            .get(&crate::render::FieldLayer::GlmFed)
                                            .is_some_and(|s| s.show);
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
                });
            })
            .response
            .rect;
        self.tour_anchors.menu = Some(panel_rect);
        self.tour_anchors.alerts = alerts_rect;
        self.show_alert_panel = alerts_tab;
        self.settings.mute_alerts = muted;
        if hide {
            self.panel_open = false;
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

    /// The way into the panel: one pill in the corner the map can spare.
    ///
    /// ponytail: the pill is a button, not a second search field. One query lives in the panel;
    /// two would need two states to keep in sync for no extra reach.
    pub(crate) fn search_pill(&mut self, ctx: &egui::Context) {
        let accent = crate::theme::accent(self.settings.theme);
        egui::Area::new(egui::Id::new("search_pill"))
            .constrain_to(self.chrome_rect)
            .anchor(egui::Align2::LEFT_TOP, egui::vec2(PANEL_X, 10.0))
            .show(ctx, |ui| {
                crate::ui::style::glass(ui, 238).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let menu = ui.add(
                            egui::Button::new(
                                egui::RichText::new(egui_phosphor::regular::LIST)
                                    .size(16.0)
                                    .color(if self.panel_open {
                                        accent
                                    } else {
                                        ui.visuals().text_color()
                                    }),
                            )
                            .fill(egui::Color32::TRANSPARENT)
                            .stroke(egui::Stroke::NONE),
                        );
                        if menu.on_hover_text("Show or hide the panel").clicked() {
                            self.panel_open = !self.panel_open;
                        }
                        let hint = egui::RichText::new(format!(
                            "{}  Search layers, tools, places",
                            egui_phosphor::regular::MAGNIFYING_GLASS
                        ))
                        .size(crate::ui::style::FONT_BASE)
                        .weak();
                        let search = ui.add(
                            egui::Button::new(hint)
                                .min_size(egui::vec2(PANEL_W - 74.0, 26.0))
                                .fill(egui::Color32::TRANSPARENT)
                                .stroke(egui::Stroke::NONE),
                        );
                        if search.clicked() {
                            self.panel_open = true;
                            self.show_alert_panel = false;
                            self.sidebar_focus_search = true;
                        }
                    });
                });
            });
    }

    /// The right-edge control column: the buttons that open what floats over the map.
    pub(crate) fn control_column(&mut self, ctx: &egui::Context) {
        use crate::ui::style::square_btn;
        let accent = crate::theme::accent(self.settings.theme);
        let (alert_count, esc) = self.alert_badge();
        let layers_on = self.panel_open && !self.show_alert_panel;
        let alerts_on = self.panel_open && self.show_alert_panel;
        egui::Area::new(egui::Id::new("control_column"))
            .constrain_to(self.chrome_rect)
            .anchor(egui::Align2::RIGHT_TOP, CONTROLS)
            .show(ctx, |ui| {
                ui.vertical(|ui| {
                    if square_btn(ui, egui_phosphor::regular::STACK, layers_on, accent)
                        .on_hover_text("Layers, products and tools")
                        .clicked()
                    {
                        self.panel_open = !layers_on;
                        self.show_alert_panel = false;
                    }
                    if square_btn(
                        ui,
                        egui_phosphor::regular::MAP_TRIFOLD,
                        self.basemap_open,
                        accent,
                    )
                    .on_hover_text("Background map")
                    .clicked()
                    {
                        self.basemap_open = !self.basemap_open;
                    }
                    let bell = square_btn(ui, egui_phosphor::regular::BELL, alerts_on, accent)
                        .on_hover_text("Active alerts in view");
                    if bell.clicked() {
                        self.panel_open = !alerts_on;
                        self.show_alert_panel = true;
                    }
                    // Count over the bell's top-right corner, coloured by the worst alert in
                    // view — the same escalation the alert panel sorts by.
                    if alert_count > 0 {
                        let c = match esc {
                            0 => crate::ui::style::OMEGA_ORANGE,
                            1 => egui::Color32::from_rgb(230, 120, 60),
                            _ => egui::Color32::from_rgb(200, 20, 20),
                        };
                        let at = bell.rect.right_top() + egui::vec2(-4.0, 4.0);
                        ui.painter().circle_filled(at, 8.0, c);
                        ui.painter().text(
                            at,
                            egui::Align2::CENTER_CENTER,
                            alert_count.min(99).to_string(),
                            egui::FontId::proportional(10.0),
                            egui::Color32::BLACK,
                        );
                    }
                });
            });
    }

    /// Background picker, slid in beside the control column.
    pub(crate) fn basemap_panel(&mut self, ctx: &egui::Context) {
        if !self.basemap_open {
            return;
        }
        let mut picked = None;
        egui::Area::new(egui::Id::new("basemap_panel"))
            .constrain_to(self.chrome_rect)
            .anchor(egui::Align2::RIGHT_TOP, RIGHT_PANEL)
            .show(ctx, |ui| {
                crate::ui::style::glass(ui, 238).show(ui, |ui| {
                    ui.set_max_width(460.0);
                    egui::ScrollArea::vertical()
                        .max_height((self.chrome_rect.height() - 120.0).max(200.0))
                        .show(ui, |ui| {
                            let current = self.views[self.active].basemap;
                            picked = crate::ui::basemap_picker::grid(
                                ui,
                                &mut self.tiles,
                                current,
                                &self.settings,
                            );
                        });
                });
            });
        if let Some(s) = picked {
            self.set_basemap(s);
            self.basemap_open = false;
        }
    }
}
