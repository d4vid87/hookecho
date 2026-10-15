//! A map-centered transport for the radar workspace, sharing the existing timeline and actions.
use super::*;
use crate::ui::a11y::Named as _;

impl HookEchoApp {
    pub(super) fn omega_transport(&mut self, ctx: &egui::Context) {
        use egui_phosphor::regular as ph;
        let idx = self.active;
        let active_field = crate::render::FieldLayer::draw_order().rev()
            .find(|field| self.views[idx].fields_on.contains(field));
        let active_mrms = if self.views[idx].show_radar { None } else { active_field };
        let mrms_interval = std::time::Duration::from_secs_f32(
            1.0 / crate::settings::SourceAnimationSpeeds::sanitize(self.views[idx].timeline.speed),
        );
        let advance_mrms = self.mrms_loops.iter().find(|loop_state| Some(loop_state.layer) == active_mrms)
            .is_some_and(|loop_state| {
            loop_state.playing && loop_state.frames.len() > 1
                && loop_state.last_step.elapsed() >= mrms_interval
        });
        if advance_mrms { if let Some(layer) = active_mrms { self.step_mrms_loop(layer, 1); } }
        if self.mrms_loops.iter().any(|loop_state| Some(loop_state.layer) == active_mrms && loop_state.playing) {
            ctx.request_repaint_after(mrms_interval.min(std::time::Duration::from_millis(100)));
        }
        let before = (
            self.views[idx].timeline.playhead,
            self.views[idx].timeline.playing,
            self.views[idx].timeline.following,
        );
        let statuses = self.timeline_frame_statuses();
        let tz = self.active_tz();
        let max_loop = self.live_loop_limit(idx);
        let v = &self.views[idx];
        let radar_visible = v.show_radar;
        let imagery_map = v.basemap.timed() || v.abi_scene.is_some();
        let field = active_field;
        let mrms_loop = self.mrms_loops.iter()
            .find(|loop_state| !radar_visible && Some(loop_state.layer) == field)
            .map(|loop_state| (loop_state.frames.len(), loop_state.index, loop_state.playing));
        let field_name = field
            .and_then(|field| field.descriptor())
            .map_or("Radar controls", |d| d.display_name);
        let field_time = field.and_then(|field| self.field_time_status(idx, field));
        let latest_cut = v.volume.as_ref().and_then(|volume| {
            volume
                .live_status
                .as_ref()
                .filter(|s| s.stream_active)
                .and_then(|_| volume.cuts.last())
                .and_then(|cut| chrono::DateTime::from_timestamp_millis(cut.ended_at_ms))
        });
        let loaded_time = v.displayed_time();
        let selected_cut = v
            .selected_cut_ms()
            .and_then(chrono::DateTime::from_timestamp_millis);
        let requested_time = v.timeline.selected_time();
        let site = v.site.clone().unwrap_or_else(|| "No site".into());
        let retained = v.retained_station.clone();
        let error = v.error.clone();
        let loading = v.loading;
        let buffering = v.buffering;
        let skipped = v
            .last_skipped_at
            .is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(5));
        let fresh = latest_cut
            .or(loaded_time)
            .is_some_and(|time| (Utc::now() - time).num_seconds() < 900);
        let age = latest_cut.or(loaded_time).map(|time| {
            format!(
                "{} {} ago",
                if latest_cut.is_some() { "Cut" } else { "Scan" },
                humanize((Utc::now() - time).num_seconds().max(0))
            )
        });
        let load_label = match v.load_phase {
            Some(level2::LoadPhase::Downloading) => "Downloading radar…",
            Some(level2::LoadPhase::Decoding) => "Decoding radar…",
            None => "Loading radar…",
        };
        let archived = v.site.as_deref().is_some_and(wxdata::sites::is_nexrad);
        let streaming = self
            .live_stream
            .as_ref()
            .is_some_and(|(pane, _, _)| *pane == idx);
        let was_cut_playback = v.timeline.cut_playback;
        let mut loop_frames = self.settings.live_loop_frames;
        let effective_loop = loop_frames.clamp(1, max_loop);
        let listed_loop = effective_loop.min(v.timeline.frames.len());
        let cached_loop = cached_loop_tail(&v.timeline.frames, listed_loop, |name| {
            self.scan_cache.contains(&name.to_string())
        });
        let mut action = None;
        let mut show_layers = false;
        let mut mrms_step = 0isize;
        let mut mrms_seek = None;
        let mut mrms_toggle = false;
        #[cfg(target_arch = "wasm32")]
        let (packs, pack_status) = (self.packs(), self.pack_status());
        #[cfg(target_arch = "wasm32")]
        let mut save_pack = false;
        #[cfg(target_arch = "wasm32")]
        let mut load_pack = None;
        let accent = egui::Color32::from_rgb(255, 190, 42);
        let area = egui::Area::new(egui::Id::new("omega_transport"))
            .constrain_to(self.chrome_rect)
            .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -24.0))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(egui::Color32::from_rgba_unmultiplied(8, 11, 12, 242))
                    .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(58)))
                    .corner_radius(16.0)
                    .inner_margin(egui::Margin::symmetric(12, 9))
                    .show(ui, |ui| {
                        ui.set_width(440.0_f32.min((self.chrome_rect.width() - 48.0).max(240.0)));
                        ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                        ui.spacing_mut().interact_size.y = 28.0;
                        ui.visuals_mut().override_text_color = Some(egui::Color32::from_gray(235));
                        ui.visuals_mut().widgets.inactive.weak_bg_fill = egui::Color32::from_rgb(30, 33, 33);
                        ui.visuals_mut().selection.bg_fill = egui::Color32::from_rgb(88, 65, 17);
                        ui.horizontal(|ui| {
                            if radar_visible {
                            egui::ComboBox::from_id_salt("omega_frame_count")
                                .width(92.0)
                                .selected_text(egui::RichText::new(format!("{effective_loop} Frames")).color(accent))
                                .show_ui(ui, |ui| {
                                    let choices: &[usize] = if cfg!(target_arch = "wasm32") {
                                        &[2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]
                                    } else if max_loop < 7 { &[2, 3, 4, 6] }
                                        else { &[7, 14, 21, 30, 50, 75, 100, 150, 200, 250] };
                                    for &count in choices.iter().filter(|&&n| n <= max_loop) {
                                        ui.selectable_value(&mut loop_frames, count, format!("{count} Frames"));
                                    }
                                    if cfg!(target_arch = "wasm32") {
                                        ui.weak(format!("Current radar memory allows up to {max_loop} live-loop frames."));
                                    } else if max_loop < 7 {
                                        ui.weak(format!("This device supports up to {max_loop} live-loop frames."));
                                    }
                                    ui.weak("Limits the live loop; archive frames remain available.");
                                })
                                .response.named("Live radar loop frame count");
                            egui::containers::menu::MenuButton::new(egui::RichText::new(ph::CLOCK_COUNTER_CLOCKWISE).color(accent))
                                .config(egui::containers::menu::MenuConfig::new().close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside))
                                .ui(ui, |ui| {
                                ui.set_min_width(260.0);
                                ui.strong("History & playback");
                                ui.label("Archive date (UTC)");
                                let result = super::scrubber::history_controls(ui, &mut self.views[idx].timeline, &mut loop_frames, max_loop,
                                    #[cfg(target_arch = "wasm32")] &packs,
                                    #[cfg(target_arch = "wasm32")] pack_status.as_deref(),
                                );
                                if result.go_live { action = Some(PaletteAction::GoLive); }
                                #[cfg(target_arch = "wasm32")]
                                {
                                    save_pack |= result.save_pack;
                                    if result.load_pack.is_some() { load_pack = result.load_pack; }
                                }
                            }).0.named("Open radar history and playback settings").on_hover_text("History & playback");
                            self.radar_product_controls(ui, true);
                            }
                            if !radar_visible {
                                ui.label(egui::RichText::new(field_name).size(12.0).color(accent));
                                if field == Some(crate::render::FieldLayer::HailSwath) {
                                    let size = if self.hail_threshold_inches == 0 {
                                        "All".to_owned()
                                    } else {
                                        format!("{}+ in", self.hail_threshold_inches)
                                    };
                                    ui.menu_button(format!("Hail: {size}"), |ui| {
                                        ui.strong("Public MRMS estimated hail size");
                                        ui.label("Size filter");
                                        let mut changed = false;
                                        for (inches, label) in [(0, "All"), (1, "1+ in"), (2, "2+ in"), (3, "3+ in")] {
                                            changed |= ui.selectable_value(&mut self.hail_threshold_inches, inches, label).changed();
                                        }
                                        if changed {
                                            if let Some(state) = self.fields.get_mut(&crate::render::FieldLayer::HailSwath) {
                                                if let Some(frame) = state.frame.as_ref() {
                                                    state.pending = Some(crate::app::field_upload_indexed_filtered(
                                                        crate::render::FieldLayer::HailSwath,
                                                        frame.field(), self.hail_threshold_inches,
                                                    ));
                                                }
                                            }
                                        }
                                        ui.separator();
                                        ui.label("Maximum over");
                                        for (minutes, label) in [(30, "30m"), (60, "1h"), (120, "2h"), (360, "6h"), (1440, "24h")] {
                                            if ui.selectable_value(&mut self.hail_minutes, minutes, label).changed() {
                                                if let Some(state) = self.fields.get_mut(&crate::render::FieldLayer::HailSwath) {
                                                    state.last_fetch = None;
                                                }
                                            }
                                        }
                                    }).response.named("Hail swath size and time window");
                                }
                            }
                        });
                        ui.horizontal(|ui| {
                            let t = &self.views[idx].timeline;
                            let (color, status) = if !radar_visible {
                                (accent, match field_time {
                                    Some((_, _, _, true)) if mrms_loop.is_some_and(|(len, index, _)| index + 1 < len) => "Replay",
                                    Some((_, _, _, true)) => "Field valid",
                                    Some((_, _, _, false)) => "Field hidden",
                                    None if field.is_some() => "Awaiting field",
                                    None => "Radar hidden",
                                })
                            } else if retained.is_some() || error.is_some() {
                                (accent, if error.is_some() { "Unavailable" } else { "Loading" })
                            } else if !t.following { (egui::Color32::from_rgb(170, 205, 250), "Archive") }
                            else if loading && loaded_time.is_none() { (accent, "Loading") }
                            else if fresh { (egui::Color32::from_rgb(69, 210, 110), "Live") }
                            else { (accent, "Stale") };
                            ui.label(egui::RichText::new(format!("● {status}")).size(11.0).color(color))
                                .on_hover_text(if retained.is_some() { "The map retains the previous station while the selected station is unavailable or loading." } else if !t.following { "Browsing historical data. Return to live follows the latest scan." } else if streaming { "Newest scans arrive through the live stream." } else { "Updates use the station poller. Data age is shown below." });
                            let clock = if !radar_visible {
                                field_time.map_or_else(String::new, |(valid, _, _, _)| crate::timefmt::fmt_clock(valid, tz, false))
                            } else { t.forecast_hour().map(|h| format!("F+{h}h")).unwrap_or_else(|| {
                                selected_cut.or_else(|| latest_cut.filter(|_| t.following && !t.playing)).or(loaded_time)
                                    .map(|time| crate::timefmt::fmt_clock(time, tz, t.cut_playback))
                                    .unwrap_or_else(|| if t.listing { "Listing volumes…" } else if archived { "No volumes" } else { "Live only" }.into())
                            }) };
                            ui.label(egui::RichText::new(clock).size(12.0));

                        });
                        let t = &mut self.views[idx].timeline;
                        if !radar_visible {
                            ui.weak(if field.is_some() {
                                if mrms_loop.is_some() { "Public MRMS mosaic history" }
                                else { "Latest field only · mosaic history is not available" }
                            } else if imagery_map {
                                "Radar timeline · imagery has separate time controls"
                            } else {
                                "Radar timeline · radar hidden"
                            });
                        }
                        if radar_visible && t.slot_count() > 0 {
                            let window = t.live_window;
                            super::scrubber::track(ui, t, tz, accent, window, false, &statuses);
                        }
                        if let Some((len, index, _)) = mrms_loop.filter(|(len, _, _)| *len > 1) {
                            let mut selected = index;
                            if ui.add(egui::Slider::new(&mut selected, 0..=len - 1).show_value(false))
                                .on_hover_text("Scrub public MRMS mosaic frames").changed() {
                                mrms_seek = Some(selected);
                            }
                        }
                        if radar_visible && t.following && t.playing && listed_loop > 0 && cached_loop < listed_loop {
                            ui.weak(format!("Loop: {cached_loop}/{listed_loop} recent frames ready"));
                        }
                        ui.horizontal(|ui| {
                            if radar_visible {
                            if omega_button(ui, ph::SKIP_BACK, "Previous frame", false).clicked() { self.views[idx].timeline.step_cut(-1); }
                            let playing = self.views[idx].timeline.playing;
                            if omega_button(ui, if playing { ph::PAUSE } else { ph::PLAY }, if playing { "Pause" } else { "Play" }, playing).clicked() { self.views[idx].timeline.toggle_play(); }
                            if omega_button(ui, ph::SKIP_FORWARD, "Next frame", false).clicked() { self.views[idx].timeline.step_cut(1); }
                            if omega_button(ui, ph::BROADCAST, "Return to live", self.views[idx].timeline.following).clicked() { action = Some(PaletteAction::GoLive); }
                            ui.separator();
                            }
                            if let Some((len, index, playing)) = mrms_loop.filter(|(len, _, _)| *len > 1) {
                                if omega_button(ui, ph::SKIP_BACK, "Previous mosaic", false).clicked() { mrms_step = -1; }
                                if omega_button(ui, if playing { ph::PAUSE } else { ph::PLAY },
                                    if playing { "Pause mosaic" } else { "Play mosaic" }, playing).clicked() { mrms_toggle = true; }
                                if omega_button(ui, ph::SKIP_FORWARD, "Next mosaic", false).clicked() { mrms_step = 1; }
                                if omega_button(ui, ph::BROADCAST, "Latest mosaic", index + 1 == len).clicked() { mrms_seek = Some(len - 1); }
                                ui.weak(format!("{}/{len}", index + 1));
                                ui.separator();
                            }
                            if omega_button(ui, ph::MAP_PIN, "Set chase location", self.tool == MapTool::Chase).clicked() { action = Some(PaletteAction::Tool(MapTool::Chase)); }
                            if omega_button(ui, ph::CROSSHAIR, "Inspect radar value", self.tool == MapTool::Interrogate).clicked() { action = Some(PaletteAction::Tool(MapTool::Interrogate)); }
                            if omega_button(ui, ph::STACK, "Layers", self.panel_open && self.panel_section == PanelSection::Overlays).clicked() { show_layers = true; }
                            if omega_button(ui, ph::RULER, "Measure distance and bearing", self.tool == MapTool::Measure).clicked() { action = Some(PaletteAction::Tool(MapTool::Measure)); }
                            if omega_button(ui, ph::PATH, "Draw a manual storm track", self.tool == MapTool::StormTrack).clicked() { action = Some(PaletteAction::Tool(MapTool::StormTrack)); }
                            if omega_button(ui, ph::PENCIL_SIMPLE, "Draw on the map", self.tool == MapTool::Draw).clicked() { action = Some(PaletteAction::Tool(MapTool::Draw)); }
                        });
                        let status = if !radar_visible {
                            field_time.map_or_else(|| if field.is_some() { "Field data has not loaded yet".into() } else { String::new() }, |(valid, _, _, aligned)| format!("{}{}", crate::timefmt::fmt_date_clock(valid, tz), if aligned { "" } else { " · outside selected time tolerance" }))
                        } else if let Some(previous) = &retained {
                            format!("Showing {previous} · {site} {}", if error.is_some() { "unavailable" } else { "loading" })
                        } else if let Some(error) = &error { format!("{site}: {error}") }
                        else if buffering { "Buffering next radar frame…".into() }
                        else if loading { load_label.into() }
                        else if skipped { "Skipped unavailable radar frame".into() }
                        else if !self.views[idx].timeline.following { self.views[idx].timeline.date.format("Archive · %Y-%m-%d").to_string() }
                        else { age.unwrap_or_default() };
                        if !status.is_empty() {
                            ui.add(egui::Label::new(egui::RichText::new(status).size(11.0).color(egui::Color32::from_gray(190))).wrap())
                                .on_hover_text(format!("Displayed scan: {}\nRequested scan: {}", loaded_time.map_or_else(|| "unavailable".into(), |t| t.to_rfc3339()), requested_time.map_or_else(|| "unavailable".into(), |t| t.to_rfc3339())));
                        }
                    });
            });
        if let (Some(layer), Some(index)) = (active_mrms, mrms_seek) {
            if let Some(loop_state) = self.mrms_loops.iter_mut().find(|loop_state| loop_state.layer == layer) {
                loop_state.index = index;
                loop_state.playing = false;
            }
            self.show_mrms_loop_frame(layer);
        } else if mrms_step != 0 {
            if let Some(layer) = active_mrms {
                if let Some(loop_state) = self.mrms_loops.iter_mut().find(|loop_state| loop_state.layer == layer) { loop_state.playing = false; }
                self.step_mrms_loop(layer, mrms_step);
            }
        }
        if mrms_toggle {
            if let Some(loop_state) = self.mrms_loops.iter_mut().find(|loop_state| Some(loop_state.layer) == active_mrms) {
                loop_state.playing = !loop_state.playing;
                loop_state.last_step = Instant::now();
            }
        }
        self.settings.live_loop_frames = loop_frames;
        if was_cut_playback && !self.views[idx].timeline.cut_playback {
            self.views[idx].cut_selection = None;
        }
        if let Some(action) = action {
            self.apply_palette(action, ctx);
        }
        if show_layers {
            self.panel_open = true;
            self.panel_section = PanelSection::Overlays;
            self.show_alert_panel = false;
            self.sidebar_focus_search = false;
            self.layers_query.clear();
            self.drawer.show_search();
            ctx.data_mut(|d| {
                d.remove::<Option<&'static str>>(egui::Id::new("panel_settings_page"));
                d.insert_temp(egui::Id::new(super::clearview::HOME_KEY), false);
                d.insert_temp(egui::Id::new("context_all_controls"), false);
            });
        }
        #[cfg(target_arch = "wasm32")]
        {
            if save_pack {
                self.save_offline_pack(ctx);
            }
            if let Some(pack) = load_pack {
                self.load_offline_pack(&pack);
            }
        }
        self.mobile_occlusion.push(area.response.rect);
        self.tour_anchors.timeline = Some(area.response.rect);
        ctx.data_mut(|d| d.insert_temp(egui::Id::new("corner_scrubber_rect"), area.response.rect));
        let after = (
            self.views[idx].timeline.playhead,
            self.views[idx].timeline.playing,
            self.views[idx].timeline.following,
        );
        if before != after {
            self.record_interaction("timeline_transport");
        }
    }
}

fn omega_button(ui: &mut egui::Ui, glyph: &str, label: &str, active: bool) -> egui::Response {
    ui.add_sized(
        [36.0, 34.0],
        egui::Button::new(egui::RichText::new(glyph).size(18.0).color(if active {
            egui::Color32::from_rgb(255, 190, 42)
        } else {
            egui::Color32::from_gray(210)
        }))
        .fill(if active {
            egui::Color32::from_rgba_unmultiplied(92, 64, 10, 155)
        } else {
            egui::Color32::TRANSPARENT
        })
        .stroke(egui::Stroke::NONE)
        .corner_radius(16.0),
    )
    .named_toggle(label, active)
    .on_hover_text(label)
}
