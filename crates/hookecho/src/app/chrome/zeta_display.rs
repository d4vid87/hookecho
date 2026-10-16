//! Independently implemented display controls for the radar workspace.
use super::*;
use crate::ui::a11y::Named as _;

impl HookEchoApp {
    pub(super) fn zeta_display_settings(&mut self, ctx: &egui::Context) {
        let id = egui::Id::new("zeta_display_open");
        let mut open = ctx.data_mut(|d| d.get_temp::<bool>(id).unwrap_or(false));
        if !open {
            return;
        }
        let animation_family = self.zeta_animation_family();
        let speeds_before = self.settings.source_animation_speeds;
        let mut basemap = self.views[self.active].basemap;
        let mut sites = self.show_radar_sites;
        let mut rings = self.show_range_rings;
        let mut scan_boundary = self.show_scan_boundary;
        let before = (
            self.settings.map_details,
            self.settings.pause_while_panning,
            self.settings.time_display,
            self.settings.reduce_motion,
            self.settings.radar_workspace,
            self.settings.show_tdwr_sites,
            self.settings.full_coverage_radar,
        );
        let area = egui::Modal::new(id)
            .frame(
                egui::Frame::new()
                    .fill(egui::Color32::from_rgb(43, 53, 68))
                    .inner_margin(22),
            )
            .show(ctx, |ui| {
                ui.set_width(520.0_f32.min(self.chrome_rect.width() - 80.0));
                ui.visuals_mut().override_text_color = Some(egui::Color32::from_rgb(225, 229, 238));
                ui.spacing_mut().item_spacing.y = 14.0;
                ui.horizontal(|ui| {
                    ui.heading("Display Settings");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("×").named("Close display settings").clicked() {
                            open = false;
                        }
                    });
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .min_scrolled_height((self.chrome_rect.height() - 160.0).clamp(180.0, 720.0))
                    .max_height((self.chrome_rect.height() - 160.0).clamp(180.0, 720.0))
                    .show(ui, |ui| {
                        ui.columns(2, |cols| {
                            let left = &mut cols[0];
                            left.colored_label(
                                egui::Color32::from_rgb(90, 171, 255),
                                "MAP DISPLAY SETTINGS",
                            );
                            left.separator();
                            left.label("Map type");
                            egui::ComboBox::from_id_salt("workspace_map_type")
                                .width(220.0)
                                .selected_text(basemap.label())
                                .show_ui(left, |ui| {
                                    for style in crate::tiles::BasemapStyle::ALL {
                                        if style.available(
                                            !self.settings.mapbox_key.is_empty(),
                                            !self.settings.maptiler_key.is_empty(),
                                            !self.settings.custom_tile_url.is_empty(),
                                        ) {
                                            ui.selectable_value(&mut basemap, style, style.label());
                                        }
                                    }
                                });
                            left.checkbox(&mut sites, "Radar stations")
                                .named("Show radar stations");
                            left.add_enabled_ui(sites, |ui| {
                                ui.checkbox(&mut self.settings.show_tdwr_sites, "TDWR stations")
                                    .named("Show terminal Doppler radar stations");
                            });
                            left.checkbox(&mut rings, "Range rings")
                                .named("Show range rings");
                            left.checkbox(&mut scan_boundary, "Live scan boundary")
                                .named("Show actual scan boundary");
                            let vector = basemap
                                .resolve(left.visuals().dark_mode)
                                .vector_palette()
                                .is_some();
                            let palette = basemap
                                .resolve(left.visuals().dark_mode)
                                .vector_palette()
                                .unwrap_or_default();
                            let (state_color, state_width) =
                                crate::basemap_style::stroke(palette, "boundary", "4")
                                    .unwrap_or(([112, 112, 112, 255], 0.8));
                            let (county_color, county_width) =
                                crate::basemap_style::stroke(palette, "boundary", "6")
                                    .unwrap_or(([56, 56, 56, 255], 0.5));
                            left.add_enabled_ui(vector, |ui| {
                                boundary_controls(
                                    ui,
                                    "State / province lines",
                                    &mut self.settings.map_details.states,
                                    [state_color[0], state_color[1], state_color[2]],
                                    state_width,
                                );
                                let counties_available = crate::basemap_style::stroke(palette, "boundary", "6").is_some();
                                ui.add_enabled_ui(counties_available, |ui| {
                                    boundary_controls(
                                        ui,
                                        "County lines",
                                        &mut self.settings.map_details.counties,
                                        [county_color[0], county_color[1], county_color[2]],
                                        county_width,
                                    );
                                });
                                if !counties_available { ui.weak("This map style has no county geometry."); }
                                ui.label("Cities & roads");
                                use crate::basemap_style::CityRoadVisibility as V;
                                let mode = &mut self.settings.map_details.city_roads;
                                let label = match *mode {
                                    V::Always => "Always",
                                    V::WithoutRadarSites => "When radar stations are hidden",
                                    V::Hidden => "Hidden",
                                };
                                egui::ComboBox::from_id_salt("workspace_city_roads")
                                    .selected_text(label)
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(mode, V::Always, "Always");
                                        ui.selectable_value(
                                            mode,
                                            V::WithoutRadarSites,
                                            "When radar stations are hidden",
                                        );
                                        ui.selectable_value(mode, V::Hidden, "Hidden");
                                    });
                            });
                            if !vector {
                                left.weak(
                                    "Choose a vector map to edit boundaries, cities and roads.",
                                );
                            } else {
                                left.weak("County detail appears as you zoom in.");
                            }
                            left.separator();
                            left.checkbox(&mut self.views[self.active].show_legend, "Color scale");
                            left.checkbox(&mut self.views[self.active].smooth, "Smooth radar");
                            left.checkbox(&mut self.settings.full_coverage_radar, "Full coverage radar")
                                .on_hover_text("Show every available NEXRAD gate; when off, display the standard 230 km range.");
                            left.checkbox(
                                &mut self.views[self.active].appearance.precision,
                                "Higher-precision radar",
                            );
                            left.separator();
                            left.label("Projection");
                            let mut globe = self.views[self.active].camera.globe;
                            left.horizontal(|ui| {
                                ui.selectable_value(&mut globe, false, "2D");
                                ui.selectable_value(&mut globe, true, "Globe");
                            });
                            self.views[self.active].camera.set_globe(globe);
                            left.add_enabled_ui(!globe, |ui| {
                                let camera = &mut self.views[self.active].camera;
                                let (mut pitch, mut bearing) = (camera.pitch_deg, camera.bearing_deg);
                                ui.add(egui::Slider::new(&mut pitch, 0.0..=60.0).text("Tilt").suffix("°"));
                                ui.add(egui::Slider::new(&mut bearing, 0.0..=360.0).text("Bearing").suffix("°"));
                                if ui.button("Reset north up").clicked() { pitch = 0.0; bearing = 0.0; }
                                camera.set_orientation(pitch, bearing);
                            });
                            let right = &mut cols[1];
                            right.colored_label(
                                egui::Color32::from_rgb(90, 171, 255),
                                "DATA DISPLAY & ANIMATION",
                            );
                            right.separator();
                            right.label("Animation speed by source");
                            for source in crate::settings::AnimationSource::ALL {
                                let active = animation_family == Some(source);
                                let mut speed = if active {
                                    crate::settings::SourceAnimationSpeeds::sanitize(self.views[self.active].timeline.speed)
                                } else {
                                    self.settings.source_animation_speeds.get(source)
                                };
                                right.label(if active { format!("{} · active", source.label()) } else { source.label().to_owned() });
                                if right.add(egui::Slider::new(&mut speed, 0.5..=20.0).suffix(" fps"))
                                    .on_hover_text(format!("{} playback speed", source.label())).changed() {
                                    self.settings.source_animation_speeds.set(source, speed);
                                    if active { self.views[self.active].timeline.speed = self.settings.source_animation_speeds.get(source); }
                                }
                            }
                            if animation_family.is_none() {
                                right.weak("Mixed sources share the current timeline speed. Choose one source to apply its saved speed.");
                            } else {
                                right.weak("One timeline controls this pane; source changes restore its saved speed.");
                            }
                            right.label("Radar opacity");
                            let mut opacity = self.views[self.active].appearance.opacity * 100.0;
                            if right
                                .add(egui::Slider::new(&mut opacity, 0.0..=100.0).suffix("%"))
                                .changed()
                            {
                                self.views[self.active].appearance.opacity = opacity / 100.0;
                            }
                            right
                                .checkbox(
                                    &mut self.settings.pause_while_panning,
                                    "Hold to pause animation",
                                )
                                .on_hover_text("Pause playback while panning or zooming the map.");
                            right.label("Time display format");
                            right.horizontal(|ui| {
                                ui.selectable_value(
                                    &mut self.settings.time_display,
                                    crate::settings::TimeDisplay::Utc,
                                    "UTC",
                                );
                                ui.selectable_value(
                                    &mut self.settings.time_display,
                                    crate::settings::TimeDisplay::SiteLocal,
                                    "Local",
                                );
                            });
                            right.separator();
                            right.checkbox(
                                &mut self.settings.reduce_motion,
                                "Reduce interface motion",
                            );
                            right.checkbox(&mut self.settings.radar_workspace, "HookEcho layout");
                        });
                    });
            });
        if area.should_close() {
            open = false;
        }
        self.mobile_occlusion.push(self.chrome_rect);
        if basemap != self.views[self.active].basemap {
            self.set_basemap(basemap);
        }
        if sites != self.show_radar_sites {
            self.apply_palette(PaletteAction::ToggleOverlay(OverlayToggle::RadarSites), ctx);
        }
        if rings != self.show_range_rings {
            self.apply_palette(PaletteAction::ToggleOverlay(OverlayToggle::RangeRings), ctx);
        }
        if scan_boundary != self.show_scan_boundary {
            self.apply_palette(PaletteAction::ToggleOverlay(OverlayToggle::ScanBoundary), ctx);
        }
        if speeds_before != self.settings.source_animation_speeds
            || before
                != (
                    self.settings.map_details,
                    self.settings.pause_while_panning,
                    self.settings.time_display,
                    self.settings.reduce_motion,
                    self.settings.radar_workspace,
                    self.settings.show_tdwr_sites,
                    self.settings.full_coverage_radar,
                )
        {
            self.settings.save();
        }
        ctx.data_mut(|d| d.insert_temp(id, open));
    }
}

fn boundary_controls(
    ui: &mut egui::Ui,
    label: &str,
    style: &mut crate::basemap_style::BoundaryAppearance,
    base_color: [u8; 3],
    base_width: f32,
) {
    ui.push_id(label, |ui| {
        ui.checkbox(&mut style.enabled, label);
        ui.add_enabled_ui(style.enabled, |ui| {
            ui.horizontal(|ui| {
                let mut color = style.color.unwrap_or(base_color);
                if ui
                    .color_edit_button_srgb(&mut color)
                    .on_hover_text("Boundary color")
                    .changed()
                {
                    style.color = Some(color);
                }
                let mut width = style
                    .width
                    .filter(|v| v.is_finite())
                    .unwrap_or(base_width)
                    .clamp(0.25, 6.0);
                if ui
                    .add(egui::Slider::new(&mut width, 0.25..=6.0).suffix(" px"))
                    .changed()
                {
                    style.width = Some(width);
                }
            });
            if (style.color.is_some() || style.width.is_some())
                && ui.small_button("Use map defaults").clicked()
            {
                style.color = None;
                style.width = None;
            }
        });
    });
}
