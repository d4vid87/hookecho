//! Reference-inspired desktop workspace, backed by HookEcho's existing actions and public feeds.
//! This is independent chrome: no vendor branding, account UI, or unavailable services.

use super::*;
use crate::ui::a11y::Named as _;

const ACCENT: egui::Color32 = egui::Color32::from_rgb(247, 202, 50);
const SURFACE: egui::Color32 = egui::Color32::from_rgb(16, 24, 39);
const MENU_ID: &str = "omega_workspace_menu";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SourceMode {
    Radar,
    Mrms,
    Satellite,
    Forecasts,
}

impl SourceMode {
    const ALL: [Self; 4] = [Self::Radar, Self::Mrms, Self::Satellite, Self::Forecasts];
    fn label(self) -> &'static str {
        match self {
            Self::Radar => "Radar",
            Self::Mrms => "MRMS",
            Self::Satellite => "Satellite",
            Self::Forecasts => "Forecasts",
        }
    }
    fn for_action(action: PaletteAction) -> Option<Self> {
        use crate::render::FieldLayer as F;
        use wxdata::field::FieldFamily;
        match action {
            PaletteAction::SetMoment(..) => Some(Self::Radar),
            PaletteAction::ToggleField(field) => {
                if matches!(field, F::MrmsReflectivityTrail | F::SnowBands) {
                    return Some(Self::Mrms);
                }
                match field.descriptor()?.family {
                    FieldFamily::Radar => Some(Self::Radar),
                    FieldFamily::Mrms => Some(Self::Mrms),
                    FieldFamily::Satellite => Some(Self::Satellite),
                    FieldFamily::Model => Some(Self::Forecasts),
                    _ => None,
                }
            }
            _ => None,
        }
    }
    fn animation_source(self) -> crate::settings::AnimationSource {
        use crate::settings::AnimationSource as A;
        match self {
            Self::Radar => A::Radar,
            Self::Mrms => A::Mrms,
            Self::Satellite => A::Satellite,
            Self::Forecasts => A::Forecasts,
        }
    }
    fn description(self) -> &'static str {
        match self {
            Self::Radar => "Selected station's observed radar",
            Self::Mrms => "NOAA national radar mosaics and derived fields",
            Self::Satellite => "Satellite imagery and lightning products",
            Self::Forecasts => "NOAA NDFD forecasts and model guidance",
        }
    }
}

/// Keep the main products ahead of extended catalogs, regardless of registry insertion order.
/// Search metadata as well as friendly labels so terms such as "reflectivity" find the mosaic.
fn source_product_rows<'a>(
    entries: &'a [PaletteEntry],
    mode: SourceMode,
    query: &str,
) -> Vec<&'a PaletteEntry> {
    use crate::render::FieldLayer as F;
    let query = query.trim().to_lowercase();
    let mut rows: Vec<_> = entries
        .iter()
        .filter(|entry| {
            if SourceMode::for_action(entry.action) != Some(mode) {
                return false;
            }
            if query.is_empty()
                || entry.label.to_lowercase().contains(&query)
                || entry.desc.to_lowercase().contains(&query)
            {
                return true;
            }
            match entry.action {
                PaletteAction::ToggleField(field) => field.descriptor().is_some_and(|descriptor| {
                    descriptor.display_name.to_lowercase().contains(&query)
                        || descriptor
                            .search_aliases
                            .iter()
                            .any(|alias| alias.to_lowercase().contains(&query))
                }),
                _ => false,
            }
        })
        .collect();
    rows.sort_by_key(|entry| {
        matches!(
            entry.action,
            PaletteAction::ToggleField(F::MrmsCatalog(_) | F::GoesCatalog(_))
        )
    });
    rows
}

/// A source choice replaces only other selectable source products. Analysis and auxiliary
/// overlays remain intact. Selecting the already active product is idempotent.
fn source_field_changes(
    enabled: &std::collections::HashSet<crate::render::FieldLayer>,
    choice: PaletteAction,
) -> Vec<crate::render::FieldLayer> {
    let target = match choice {
        PaletteAction::ToggleField(field) => Some(field),
        _ => None,
    };
    let mut changes: Vec<_> = enabled
        .iter()
        .copied()
        .filter(|field| {
            Some(*field) != target
                && SourceMode::for_action(PaletteAction::ToggleField(*field)).is_some()
        })
        .collect();
    if let Some(target) = target {
        if !enabled.contains(&target) {
            changes.push(target);
        }
    }
    changes
}

fn workspace_style(ui: &mut egui::Ui) {
    ui.visuals_mut().override_text_color = Some(egui::Color32::from_rgb(237, 240, 245));
    ui.visuals_mut().selection.bg_fill = ACCENT.gamma_multiply(0.25);
    ui.visuals_mut().selection.stroke = egui::Stroke::new(1.0, ACCENT);
    ui.visuals_mut().widgets.inactive.bg_fill = egui::Color32::from_rgb(24, 33, 47);
    ui.visuals_mut().widgets.hovered.bg_fill = egui::Color32::from_rgb(43, 52, 65);
    ui.spacing_mut().interact_size.y = 36.0;
    ui.spacing_mut().button_padding = egui::vec2(10.0, 7.0);
}

fn surface() -> egui::Frame {
    egui::Frame::new()
        .fill(SURFACE)
        .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(48, 57, 71)))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(8, 5))
}

fn menu_row(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    ui.add_sized(
        [ui.available_width(), 40.0],
        egui::Button::new(egui::RichText::new(label).size(14.0).color(if selected {
            ACCENT
        } else {
            egui::Color32::WHITE
        }))
        .fill(if selected {
            ACCENT.gamma_multiply(0.13)
        } else {
            egui::Color32::TRANSPARENT
        })
        .stroke(egui::Stroke::NONE)
        .corner_radius(4.0),
    )
    .named_toggle(label, selected)
}

fn section(ui: &mut egui::Ui, label: &str) {
    ui.add_space(16.0);
    ui.label(
        egui::RichText::new(label)
            .size(11.0)
            .strong()
            .color(egui::Color32::from_rgb(150, 158, 173)),
    );
    ui.add_space(6.0);
}

impl HookEchoApp {
    pub(super) fn omega_animation_family(&self) -> Option<crate::settings::AnimationSource> {
        let mut modes = SourceMode::ALL.into_iter().filter(|mode| {
            (*mode == SourceMode::Radar && self.views[self.active].show_radar)
                || (*mode == SourceMode::Forecasts && self.views[self.active].basemap.ndfd_layer().is_some())
                || self.views[self.active].fields_on.iter().any(|field| {
                    SourceMode::for_action(PaletteAction::ToggleField(*field)) == Some(*mode)
                })
        });
        let first = modes.next()?;
        modes.next().is_none().then(|| first.animation_source())
    }

    fn sync_omega_animation_speed(&mut self, ctx: &egui::Context) {
        let key = egui::Id::new(("omega_animation_source", self.active));
        let current = self.omega_animation_family();
        let previous =
            ctx.data_mut(|d| d.get_temp::<Option<crate::settings::AnimationSource>>(key));
        let before = self.settings.source_animation_speeds;
        match previous {
            None => {
                if let Some(current) = current {
                    self.views[self.active].timeline.speed =
                        self.settings.source_animation_speeds.get(current);
                }
            }
            Some(previous) if previous != current => {
                self.views[self.active].timeline.speed = self
                    .settings
                    .source_animation_speeds
                    .switch(previous, current, self.views[self.active].timeline.speed);
            }
            _ => {
                if let Some(current) = current {
                    // History controls can also change speed. Save that edit even if the
                    // user closes the app before switching sources.
                    self.settings.source_animation_speeds.set(current, self.views[self.active].timeline.speed);
                }
            }
        }
        if before != self.settings.source_animation_speeds {
            self.settings.save();
        }
        ctx.data_mut(|d| d.insert_temp(key, current));
    }

    pub(super) fn omega_workspace_controls(&mut self, ctx: &egui::Context) {
        use crate::render::FieldLayer as F;
        use egui_phosphor::regular as ph;
        self.sync_omega_animation_speed(ctx);
        let menu_id = egui::Id::new(MENU_ID);
        let mut menu_open = ctx.data_mut(|d| d.get_temp::<bool>(menu_id).unwrap_or(false));
        let logo = crate::icon::texture(ctx, 128);
        let menu_was_open = menu_open;
        let active_modes: Vec<_> = SourceMode::ALL
            .into_iter()
            .filter(|mode| {
                (*mode == SourceMode::Radar && self.views[self.active].show_radar)
                    || (*mode == SourceMode::Forecasts && self.views[self.active].basemap.ndfd_layer().is_some())
                    || self.views[self.active].fields_on.iter().any(|field| {
                        SourceMode::for_action(PaletteAction::ToggleField(*field)) == Some(*mode)
                    })
            })
            .collect();
        let source_label = match active_modes.as_slice() {
            [mode] => mode.label(),
            [] => "Sources",
            _ => "Mixed sources",
        };
        let mut choose_source = None;
        let mut choose_ndfd_style = None;
        let mut action = None;
        let mut display = false;
        let mut spotter_settings = false;
        let mut notification_settings = false;
        let mut return_to_radar = false;
        let mut compare_panes = None;
        let entries = self.palette_entries();
        let top = egui::Area::new("omega_workspace_top".into())
            .order(egui::Order::Foreground)
            .constrain_to(self.chrome_rect)
            .anchor(egui::Align2::LEFT_TOP, egui::vec2(12.0, 24.0))
            .show(ctx, |ui| {
                workspace_style(ui);
                surface().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui
                            .button(ph::LIST)
                            .named_toggle("Open workspace menu", menu_open)
                            .clicked()
                        {
                            menu_open = !menu_open;
                        }
                        ui.add(egui::Image::new(&logo).fit_to_exact_size(egui::vec2(40.0, 40.0)))
                            .on_hover_text("HookEcho");
                        egui::containers::menu::MenuButton::new(
                            egui::RichText::new(format!("{} {}", source_label, ph::CARET_DOWN))
                                .size(16.0),
                        )
                        .config(
                            egui::containers::menu::MenuConfig::new()
                                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside),
                        )
                        .ui(ui, |ui| {
                            ui.weak("Choose one source; use Layer manager to combine.");
                            for mode in SourceMode::ALL {
                                ui.menu_button(mode.label(), |ui| {
                                    ui.set_min_width(290.0);
                                    ui.label(mode.description());
                                    ui.separator();
                                    let search_id =
                                        egui::Id::new(("omega_source_search", mode.label()));
                                    let mut query = ctx.data_mut(|d| {
                                        d.get_temp::<String>(search_id).unwrap_or_default()
                                    });
                                    ui.add(
                                        egui::TextEdit::singleline(&mut query)
                                            .hint_text("Find a product")
                                            .desired_width(290.0),
                                    )
                                    .named("Find a source product");
                                    ctx.data_mut(|d| d.insert_temp(search_id, query.clone()));
                                    let total = source_product_rows(&entries, mode, "").len();
                                    let rows = source_product_rows(&entries, mode, &query);
                                    let ndfd_rows = if mode == SourceMode::Forecasts {
                                        [crate::tiles::BasemapStyle::NdfdRainChance,
                                         crate::tiles::BasemapStyle::NdfdMaxTemp,
                                         crate::tiles::BasemapStyle::NdfdMinTemp,
                                         crate::tiles::BasemapStyle::NdfdAlaskaRainChance,
                                         crate::tiles::BasemapStyle::NdfdAlaskaMaxTemp,
                                         crate::tiles::BasemapStyle::NdfdAlaskaMinTemp,
                                         crate::tiles::BasemapStyle::NdfdHawaiiRainChance,
                                         crate::tiles::BasemapStyle::NdfdHawaiiMaxTemp,
                                         crate::tiles::BasemapStyle::NdfdHawaiiMinTemp]
                                            .into_iter().filter(|style| style.label().to_lowercase().contains(&query.to_lowercase())).count()
                                    } else { 0 };
                                    ui.weak(format!("{} of {} products", rows.len() + ndfd_rows,
                                        total + usize::from(mode == SourceMode::Forecasts) * 9));
                                    if rows.is_empty() && ndfd_rows == 0 {
                                        ui.weak("No matching products.");
                                    }
                                    egui::ScrollArea::vertical()
                                        .id_salt(("omega_source_products", mode.label(), &query))
                                        .scroll_bar_visibility(
                                            egui::scroll_area::ScrollBarVisibility::AlwaysVisible,
                                        )
                                        .max_height(
                                            (self.chrome_rect.height() - 180.0).clamp(120.0, 420.0),
                                        )
                                        .show(ui, |ui| {
                                            if mode == SourceMode::Forecasts {
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
                                                    if !query.is_empty() && !style.label().to_lowercase().contains(&query.to_lowercase()) { continue; }
                                                    let selected = self.views[self.active].basemap == style;
                                                    if ui.selectable_label(selected, style.label())
                                                        .named_toggle(style.label(), selected).clicked() {
                                                        choose_ndfd_style = Some(style);
                                                        ui.close();
                                                    }
                                                }
                                            }
                                            for entry in rows {
                                                let selected = entry.on.unwrap_or(false)
                                                    && match entry.action {
                                                        PaletteAction::SetMoment(..) => {
                                                            self.views[self.active].show_radar
                                                                && self.views[self.active]
                                                                    .custom_product
                                                                    .is_none()
                                                        }
                                                        _ => true,
                                                    };
                                                if ui
                                                    .selectable_label(selected, &entry.label)
                                                    .named_toggle(&entry.label, selected)
                                                    .on_hover_text(entry.desc)
                                                    .clicked()
                                                {
                                                    choose_source = Some(entry.action);
                                                    ui.close();
                                                }
                                            }
                                        });
                                })
                                .response
                                .named(mode.label());
                            }
                            ui.separator();
                            if ui.button("Layer manager").named("Layer manager").clicked() {
                                action = Some(PaletteAction::OpenWindow(AppWindow::LayerManager));
                                ui.close();
                            }
                        })
                        .0
                        .named("Weather source mode");
                    });
                });
            });
        self.mobile_occlusion.push(top.response.rect);
        self.tour_anchors.menu = Some(top.response.rect);
        crate::ui::drawer::avoid_toolbar(ctx, top.response.rect);

        // On narrow desktops the station card follows the source selector instead of colliding.
        let room_for_center = self.chrome_rect.width() >= 820.0;
        let station = self.views[self.active]
            .site
            .clone()
            .unwrap_or_else(|| "Select radar".into());
        let station_status = self.radar_site_status.as_ref().map_or(
            wxdata::radar_status::Availability::Unknown,
            |report| report.availability(&station, Utc::now()),
        );
        let station_status = match station_status {
            wxdata::radar_status::Availability::Recent => "NOAA Level II receipt: recent",
            wxdata::radar_status::Availability::Delayed => "NOAA Level II receipt: delayed",
            wxdata::radar_status::Availability::Stale => "NOAA Level II receipt: stale",
            wxdata::radar_status::Availability::NotOperating => "NOAA RDA: not operating",
            wxdata::radar_status::Availability::Unknown => "NOAA network status: unavailable",
        };
        let scan_status = self.views[self.active].displayed_time().map_or_else(
            || "Displayed scan: not loaded".to_owned(),
            |time| format!("Displayed scan: {} min old", Utc::now().signed_duration_since(time).num_minutes().max(0)),
        );
        let station_details = format!("{station_status}\n{scan_status}\nPin color reflects NOAA network status.");
        let station_area = egui::Area::new("omega_workspace_station".into())
            .order(egui::Order::Foreground)
            .constrain_to(self.chrome_rect)
            .anchor(
                if room_for_center {
                    egui::Align2::CENTER_TOP
                } else {
                    egui::Align2::RIGHT_TOP
                },
                egui::vec2(if room_for_center { 0.0 } else { -12.0 }, 24.0),
            )
            .show(ctx, |ui| {
                workspace_style(ui);
                surface().show(ui, |ui| {
                    let view = &self.views[self.active];
                    if view.show_radar {
                        if let Some(volume) = view.volume.as_ref() {
                            let label = volume.vcp.split('(').next().unwrap_or(&volume.vcp).trim();
                            ui.label(
                                egui::RichText::new(if view.retained_station.is_some() {
                                    format!("Previous scan · {label}")
                                } else {
                                    label.to_owned()
                                })
                                .size(10.0)
                                .color(egui::Color32::from_gray(175)),
                            )
                            .on_hover_text(&volume.vcp);
                        }
                    }
                    ui.horizontal(|ui| {
                        if ui
                            .button(
                                egui::RichText::new(format!("{station} {}", ph::CARET_DOWN))
                                    .strong()
                                    .color(ACCENT),
                            )
                            .named("Select radar station")
                            .on_hover_text(&station_details)
                            .clicked()
                        {
                            action = Some(PaletteAction::OpenWindow(AppWindow::Site));
                        }
                        let mut globe = self.views[self.active].camera.globe;
                        let mut pitch = self.views[self.active].camera.pitch_deg;
                        let mut bearing = self.views[self.active].camera.bearing_deg;
                        egui::containers::menu::MenuButton::new(format!(
                            "{} {}",
                            if globe {
                                "Globe"
                            } else if pitch > 0.0 {
                                "3D"
                            } else {
                                "2D"
                            },
                            ph::CARET_DOWN
                        ))
                        .config(
                            egui::containers::menu::MenuConfig::new()
                                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside),
                        )
                        .ui(ui, |ui| {
                            if ui
                                .selectable_label(!globe && pitch == 0.0, "2D map · north up")
                                .named_toggle("2D map", !globe && pitch == 0.0)
                                .clicked()
                            {
                                globe = false;
                                pitch = 0.0;
                                bearing = 0.0;
                                ui.close();
                            }
                            if ui
                                .selectable_label(!globe && pitch > 0.0, "3D map")
                                .named_toggle("3D map", !globe && pitch > 0.0)
                                .clicked()
                            {
                                globe = false;
                                if pitch == 0.0 {
                                    pitch = 45.0;
                                }
                                ui.close();
                            }
                            if ui
                                .selectable_label(globe, "Globe")
                                .named_toggle("Globe projection", globe)
                                .clicked()
                            {
                                globe = true;
                                ui.close();
                            }
                            ui.separator();
                            ui.add_enabled_ui(!globe, |ui| {
                                ui.add(
                                    egui::Slider::new(&mut pitch, 0.0..=60.0)
                                        .text("Tilt")
                                        .suffix("°"),
                                );
                                ui.add(
                                    egui::Slider::new(&mut bearing, 0.0..=360.0)
                                        .text("Bearing")
                                        .suffix("°"),
                                );
                                if ui.button("Reset north up").clicked() {
                                    pitch = 0.0;
                                    bearing = 0.0;
                                }
                            });
                            ui.weak("Reflectivity height shows value, not storm altitude. Other products stay flat.");
                        })
                        .0
                        .named("Map projection");
                        if globe != self.views[self.active].camera.globe {
                            self.views[self.active].camera.set_globe(globe);
                        }
                        self.views[self.active]
                            .camera
                            .set_orientation(pitch, bearing);
                    })
                });
            });
        self.mobile_occlusion.push(station_area.response.rect);

        if menu_open && !menu_was_open {
            let previous = (self.panel_open, self.drawer.is_open());
            ctx.data_mut(|d| {
                d.insert_temp(egui::Id::new("omega_menu_previous_surface"), previous);
                d.insert_temp(egui::Id::new("omega_display_open"), false);
            });
            self.panel_open = false;
            self.drawer.show_search();
        }
        if menu_open {
            let bounds = self.chrome_rect;
            let backdrop = egui::Area::new("omega_workspace_backdrop".into())
                .order(egui::Order::Foreground)
                .fixed_pos(bounds.min)
                .show(ctx, |ui| {
                    let (rect, response) =
                        ui.allocate_exact_size(bounds.size(), egui::Sense::click());
                    ui.painter()
                        .rect_filled(rect, 0.0, egui::Color32::from_black_alpha(130));
                    response.named("Close workspace menu")
                });
            if backdrop.inner.clicked() || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                menu_open = false;
            }
            let side = egui::Area::new("omega_workspace_sidebar".into())
                .order(egui::Order::Foreground)
                .fixed_pos(bounds.min)
                .show(ctx, |ui| {
                    workspace_style(ui);
                    egui::Frame::new()
                        .fill(SURFACE)
                        .inner_margin(egui::Margin::same(16))
                        .show(ui, |ui| {
                            ui.set_width(228.0);
                            ui.set_min_height((bounds.height() - 32.0).max(100.0));
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::Image::new(&logo)
                                        .fit_to_exact_size(egui::vec2(40.0, 40.0)),
                                );
                                ui.label(
                                    egui::RichText::new("HookEcho")
                                        .strong()
                                        .size(19.0)
                                        .color(ACCENT),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui.button(ph::X).named("Close workspace menu").clicked()
                                        {
                                            menu_open = false;
                                        }
                                    },
                                );
                            });
                            ui.add_space(10.0);
                            ui.separator();
                            egui::ScrollArea::vertical()
                                .max_height((bounds.height() - 105.0).max(60.0))
                                .show(ui, |ui| {
                                    section(ui, "VIEWS");
                                    if menu_row(ui, "Radar", !self.analyst_open).clicked() {
                                        return_to_radar = true;
                                    }
                                    if menu_row(ui, "Compare radar products", self.analyst_open).clicked() {
                                        compare_panes = Some(if self.analyst_open { self.views.len() } else { 2 });
                                    }
                                    if self.analyst_open {
                                        ui.horizontal(|ui| {
                                            ui.label("Panes");
                                            for count in [2, 4] {
                                                if ui.selectable_label(self.views.len() == count, count.to_string()).clicked() {
                                                    compare_panes = Some(count);
                                                }
                                            }
                                        });
                                        ui.checkbox(&mut self.link_cameras, "Link map positions");
                                        ui.checkbox(&mut self.link_times, "Link timelines");
                                        if self.link_times {
                                            let mut limited = self.settings.linked_time_limit_seconds.is_some();
                                            if ui.checkbox(&mut limited, "Limit linked radar time difference").changed() {
                                                self.settings.linked_time_limit_seconds = limited.then_some(120);
                                                self.settings.save();
                                            }
                                            if let Some(seconds) = &mut self.settings.linked_time_limit_seconds {
                                                ui.label("Maximum difference (seconds)");
                                                if ui.add_sized([ui.available_width().min(216.0), 22.0], egui::Slider::new(seconds, 0..=1800)).changed() {
                                                    self.settings.save();
                                                }
                                            }
                                        }
                                        ui.checkbox(&mut self.analyst_inspector_open, "Show inspector");
                                        ui.separator();
                                    }
                                    section(ui, "GENERAL");
                                    if menu_row(ui, "Weather Alerts", self.show_alert_panel)
                                        .clicked()
                                    {
                                        action = Some(PaletteAction::ToggleOverlay(
                                            OverlayToggle::AlertPanel,
                                        ));
                                    }
                                    egui::CollapsingHeader::new("Data Overlays")
                                        .id_salt("omega_data_overlays")
                                        .show(ui, |ui| {
                                            for (label, mapped) in [
                                                (
                                                    "Weather warnings",
                                                    PaletteAction::ToggleOverlay(
                                                        OverlayToggle::Alerts,
                                                    ),
                                                ),
                                                (
                                                    "Storm reports",
                                                    PaletteAction::ToggleOverlay(
                                                        OverlayToggle::StormReports,
                                                    ),
                                                ),
                                                (
                                                    "Storm tracks",
                                                    PaletteAction::ToggleOverlay(
                                                        OverlayToggle::Tracks,
                                                    ),
                                                ),
                                                (
                                                    "Satellite lightning",
                                                    PaletteAction::ToggleOverlay(
                                                        OverlayToggle::GlmLightning,
                                                    ),
                                                ),
                                                (
                                                    "Surface observations",
                                                    PaletteAction::ToggleOverlay(
                                                        OverlayToggle::Metar,
                                                    ),
                                                ),
                                                ("SPC Outlook", PaletteAction::ToggleOutlook),
                                                (
                                                    "Tropical cyclones",
                                                    PaletteAction::ToggleOverlay(
                                                        OverlayToggle::Tropical,
                                                    ),
                                                ),
                                                (
                                                    "MRMS reflectivity",
                                                    PaletteAction::ToggleField(F::Mrms),
                                                ),
                                                (
                                                    "Layer manager",
                                                    PaletteAction::OpenWindow(
                                                        AppWindow::LayerManager,
                                                    ),
                                                ),
                                            ] {
                                                let on = entries
                                                    .iter()
                                                    .find(|entry| entry.action == mapped)
                                                    .and_then(|entry| entry.on)
                                                    .unwrap_or(false);
                                                if menu_row(ui, label, on).clicked() {
                                                    action = Some(mapped);
                                                }
                                            }
                                        });
                                    if menu_row(ui, "Custom Locations", false).clicked() {
                                        action =
                                            Some(PaletteAction::OpenWindow(AppWindow::Markers));
                                    }
                                    if menu_row(ui, "Sensors", self.show_sensors).clicked() {
                                        action = Some(PaletteAction::ToggleOverlay(
                                            OverlayToggle::Sensors,
                                        ));
                                    }
                                    egui::CollapsingHeader::new("Storm Chasers")
                                        .id_salt("omega_storm_chasers")
                                        .show(ui, |ui| {
                                            if menu_row(ui, "Spotter Network settings", false)
                                                .clicked()
                                            {
                                                spotter_settings = true;
                                            }
                                            for (label, mapped) in [
                                                (
                                                    "Spotter Network positions",
                                                    PaletteAction::ToggleOverlay(
                                                        OverlayToggle::Spotters,
                                                    ),
                                                ),
                                                (
                                                    "Weather & highway cameras",
                                                    PaletteAction::ToggleOverlay(
                                                        OverlayToggle::Webcams,
                                                    ),
                                                ),
                                                (
                                                    "Live station cards",
                                                    PaletteAction::ToggleOverlay(
                                                        OverlayToggle::Stations,
                                                    ),
                                                ),
                                                (
                                                    "Chase replay",
                                                    PaletteAction::OpenWindow(
                                                        AppWindow::ChaseReplay,
                                                    ),
                                                ),
                                            ] {
                                                let on = entries
                                                    .iter()
                                                    .find(|entry| entry.action == mapped)
                                                    .and_then(|entry| entry.on)
                                                    .unwrap_or(false);
                                                if menu_row(ui, label, on).clicked() {
                                                    action = Some(mapped);
                                                }
                                            }
                                        });
                                    if menu_row(ui, "Notifications", false).clicked() {
                                        notification_settings = true;
                                    }
                                    section(ui, "SETTINGS");
                                    if menu_row(ui, "Display Settings", false).clicked() {
                                        display = true;
                                    }
                                    if menu_row(ui, "Radar Settings", false).clicked() {
                                        action =
                                            Some(PaletteAction::OpenWindow(AppWindow::Settings));
                                    }
                                    ui.separator();
                                    if menu_row(ui, "Help & Support", false).clicked() {
                                        action = Some(PaletteAction::OpenWindow(AppWindow::Help));
                                    }
                                });
                        });
                });
            self.mobile_occlusion.push(side.response.rect);
        }
        if return_to_radar {
            menu_open = false;
            self.return_to_radar(ctx);
        } else if let Some(count) = compare_panes {
            menu_open = false;
            self.switch_mode(ViewMode::Analyst, ctx);
            self.set_pane_count(count);
        }
        if menu_was_open && !menu_open && action.is_none() && !display && !spotter_settings
            && !notification_settings
            && !return_to_radar && compare_panes.is_none() {
            if let Some((panel, drawer)) = ctx.data_mut(|d| {
                d.get_temp::<(bool, bool)>(egui::Id::new("omega_menu_previous_surface"))
            }) {
                self.panel_open = panel;
                if drawer {
                    self.drawer.resume();
                }
            }
        }
        if let Some(source) = choose_source {
            let speeds_before = self.settings.source_animation_speeds;
            if let Some(previous) = self.omega_animation_family() {
                self.settings
                    .source_animation_speeds
                    .set(previous, self.views[self.active].timeline.speed);
            }
            if self.views[self.active].basemap.ndfd_layer().is_some() {
                self.set_basemap(crate::tiles::BasemapStyle::Dark);
            }
            let changes = source_field_changes(&self.views[self.active].fields_on, source);
            for field in changes {
                self.apply_palette(PaletteAction::ToggleField(field), ctx);
            }
            let radar = matches!(source, PaletteAction::SetMoment(..));
            if radar {
                self.apply_palette(source, ctx);
            }
            self.views[self.active].show_radar = radar;
            self.sync_omega_animation_speed(ctx);
            if speeds_before != self.settings.source_animation_speeds {
                self.settings.save();
            }
            self.panel_open = false;
            self.show_alert_panel = false;
            self.drawer.show_search();
        }
        if let Some(style) = choose_ndfd_style {
            for field in source_field_changes(&self.views[self.active].fields_on,
                PaletteAction::SetMoment(wxdata::level2::Moment::Reflectivity, false)) {
                self.apply_palette(PaletteAction::ToggleField(field), ctx);
            }
            self.views[self.active].show_radar = false;
            self.set_basemap(style);
            self.sync_omega_animation_speed(ctx);
            self.panel_open = false;
            self.show_alert_panel = false;
            self.drawer.show_search();
        }
        if spotter_settings {
            menu_open = false;
            self.show_spotters = true;
            self.panel_open = true;
            self.show_alert_panel = false;
            self.panel_section = PanelSection::Tools;
            self.sidebar_focus_search = false;
            self.drawer.show_search();
            ctx.data_mut(|d| {
                d.insert_temp(egui::Id::new("quick_launch_home"), false);
                d.insert_temp(egui::Id::new("panel_settings_page"), Some("Spotters"));
            });
        }
        if notification_settings {
            menu_open = false;
            self.settings_window.open_notifications();
        }
        if display {
            menu_open = false;
            ctx.data_mut(|d| d.insert_temp(egui::Id::new("omega_display_open"), true));
        }
        if let Some(action) = action {
            menu_open = false;
            if matches!(action, PaletteAction::OpenWindow(_)) {
                self.panel_open = false;
                self.show_alert_panel = false;
                ctx.data_mut(|d| {
                    d.insert_temp(egui::Id::new("quick_launch_home"), false);
                    d.remove::<Option<&'static str>>(egui::Id::new("panel_settings_page"));
                });
            }
            self.apply_palette(action, ctx);
            if action == PaletteAction::ToggleOverlay(OverlayToggle::AlertPanel) {
                self.panel_open = self.show_alert_panel;
                self.panel_section = PanelSection::Alerts;
                self.drawer.show_search();
                self.sidebar_focus_search = false;
                ctx.data_mut(|d| {
                    d.insert_temp(egui::Id::new("quick_launch_home"), false);
                    d.remove::<Option<&'static str>>(egui::Id::new("panel_settings_page"));
                });
            }
        }
        ctx.data_mut(|d| d.insert_temp(menu_id, menu_open));
    }
}

#[cfg(test)]
mod source_tests {
    use super::*;
    use crate::render::FieldLayer as F;

    fn field_entry(field: F) -> PaletteEntry {
        PaletteEntry {
            label: field.descriptor().unwrap().display_name.into(),
            category: "test",
            action: PaletteAction::ToggleField(field),
            on: Some(false),
            desc: "",
            common: false,
            key: None,
            health: None,
            favorite: false,
            recent: None,
        }
    }

    #[test]
    fn mrms_menu_keeps_core_products_before_every_extended_catalog_product() {
        // The registry emits its entire 63-product catalog before the primary fields.
        let mut entries: Vec<_> = (0..wxdata::mrms::CATALOG.len())
            .map(|index| field_entry(F::MrmsCatalog(index as u8)))
            .collect();
        let core = [
            F::Mrms,
            F::MrmsLowLevel,
            F::Rotation,
            F::Mesh,
            F::Lightning,
            F::AzShear,
            F::AzShearMid,
            F::Posh,
            F::MrmsEchoTop18,
            F::MrmsVil,
            F::PrecipRate,
            F::Qpe1h,
            F::Qpe3h,
            F::Qpe6h,
            F::Qpe12h,
            F::Qpe24h,
            F::PrecipType,
            F::FlashFlood,
            F::HailSwath,
            F::MrmsReflectivityTrail,
            F::SnowBands,
        ];
        entries.extend(core.into_iter().map(field_entry));
        entries.push(field_entry(F::GoesC13));
        let rows = source_product_rows(&entries, SourceMode::Mrms, "");
        assert_eq!(rows.len(), core.len() + wxdata::mrms::CATALOG.len());
        for (row, field) in rows.iter().zip(core) {
            assert_eq!(row.action, PaletteAction::ToggleField(field));
        }
        for (row, index) in rows[core.len()..].iter().zip(0..) {
            assert_eq!(
                row.action,
                PaletteAction::ToggleField(F::MrmsCatalog(index))
            );
        }
    }

    #[test]
    fn product_search_finds_metadata_and_keeps_extended_products_reachable() {
        let mut mosaic = field_entry(F::Mrms);
        mosaic.label = "National mosaic (MRMS)".into();
        let entries = vec![
            field_entry(F::MrmsCatalog(0)),
            mosaic,
            field_entry(F::GoesC13),
            field_entry(F::Qpe1h),
        ];
        let rows = source_product_rows(&entries, SourceMode::Mrms, " REFLECTIVITY ");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].action, PaletteAction::ToggleField(F::Mrms));
        let rows = source_product_rows(&entries, SourceMode::Mrms, "30 dbz");
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].action,
            PaletteAction::ToggleField(F::MrmsCatalog(0))
        );
        assert!(source_product_rows(&entries, SourceMode::Mrms, "unavailable product").is_empty());
        assert_eq!(source_product_rows(&entries, SourceMode::Mrms, "").len(), 3);
    }

    #[test]
    fn catalog_selection_replaces_other_sources_and_keeps_family_for_speed() {
        let enabled = std::collections::HashSet::from([F::Mrms, F::GoesC13, F::RtmaTemp2m]);
        let choice = PaletteAction::ToggleField(F::MrmsCatalog(0));
        assert_eq!(
            SourceMode::for_action(choice).unwrap().animation_source(),
            crate::settings::AnimationSource::Mrms
        );
        let mut next = enabled.clone();
        for field in source_field_changes(&enabled, choice) {
            if !next.remove(&field) {
                next.insert(field);
            }
        }
        assert_eq!(
            next,
            std::collections::HashSet::from([F::MrmsCatalog(0), F::RtmaTemp2m])
        );
        assert!(source_field_changes(&next, choice).is_empty());
        // Point lightning is an auxiliary overlay, not a raster replacement.
        assert_eq!(
            SourceMode::for_action(PaletteAction::ToggleOverlay(OverlayToggle::GlmLightning)),
            None
        );
    }

    #[test]
    fn source_product_selection_replaces_weather_and_preserves_analysis() {
        let mut enabled = std::collections::HashSet::from([F::Mrms, F::GoesC13, F::RtmaTemp2m]);
        let choice = PaletteAction::ToggleField(F::Hrrr);
        for field in source_field_changes(&enabled, choice) {
            if !enabled.remove(&field) {
                enabled.insert(field);
            }
        }
        assert_eq!(
            enabled,
            std::collections::HashSet::from([F::Hrrr, F::RtmaTemp2m])
        );
        assert!(source_field_changes(&enabled, choice).is_empty());
        let radar = PaletteAction::SetMoment(Moment::Velocity, true);
        assert_eq!(source_field_changes(&enabled, radar), vec![F::Hrrr]);
    }

    #[test]
    fn source_groups_use_field_metadata_and_do_not_classify_tools() {
        assert_eq!(
            SourceMode::for_action(PaletteAction::ToggleField(F::GoesC13)),
            Some(SourceMode::Satellite)
        );
        assert_eq!(
            SourceMode::for_action(PaletteAction::ToggleField(F::Qpe1h)),
            Some(SourceMode::Mrms)
        );
        assert_eq!(
            SourceMode::for_action(PaletteAction::ToggleField(F::GlobalWind10m)),
            Some(SourceMode::Forecasts)
        );
        assert_eq!(
            SourceMode::for_action(PaletteAction::ToggleField(F::Vil)),
            Some(SourceMode::Radar)
        );
        assert_eq!(
            SourceMode::for_action(PaletteAction::ToggleField(F::RtmaTemp2m)),
            None
        );
        assert_eq!(
            SourceMode::for_action(PaletteAction::OpenWindow(AppWindow::Help)),
            None
        );
    }
}
