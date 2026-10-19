//! A map-centered transport for the radar workspace, sharing the existing timeline and actions.
use super::*;
use chrono::Timelike as _;
use crate::ui::a11y::Named as _;

impl HookEchoApp {
    pub(super) fn zeta_transport(&mut self, ctx: &egui::Context) {
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
                && !loop_state.loading && loop_state.error.is_none()
                && loop_state.last_step.elapsed() >= mrms_interval
        });
        if advance_mrms { if let Some(layer) = active_mrms {
            let direction = self.mrms_loops.iter().find(|state| state.layer == layer)
                .map_or(1, |state| if state.reverse_playback { -1 } else { 1 });
            let next_window = self.mrms_loops.iter().find(|state| state.layer == layer)
                .filter(|state| state.follow_through_archive && state.requested_at.is_some()
                    && if direction < 0 { state.index == 0 }
                       else { state.index + 1 == state.frames.len() })
                .is_some();
            if next_window { self.request_mrms_adjacent_window(layer, direction, true); }
            else { self.step_mrms_loop(layer, direction); }
        } }
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
        let mrms_archive_at = self.mrms_loops.iter()
            .find(|loop_state| !radar_visible && Some(loop_state.layer) == field)
            .and_then(|loop_state| loop_state.requested_at);
        let mrms_follow_archive = self.mrms_loops.iter()
            .find(|loop_state| !radar_visible && Some(loop_state.layer) == field)
            .is_some_and(|loop_state| loop_state.follow_through_archive);
        let mrms_reverse_playback = self.mrms_loops.iter()
            .find(|loop_state| !radar_visible && Some(loop_state.layer) == field)
            .is_some_and(|loop_state| loop_state.reverse_playback);
        let mrms_last_time = self.mrms_loops.iter()
            .find(|loop_state| !radar_visible && Some(loop_state.layer) == field)
            .and_then(|loop_state| loop_state.frames.last().map(|frame| frame.stamp.valid_time));
        let mrms_first_time = self.mrms_loops.iter()
            .find(|loop_state| !radar_visible && Some(loop_state.layer) == field)
            .and_then(|loop_state| loop_state.frames.first().map(|frame| frame.stamp.valid_time));
        let mrms_error = self.mrms_loops.iter()
            .find(|loop_state| !radar_visible && Some(loop_state.layer) == field)
            .and_then(|loop_state| loop_state.error.clone());
        let mrms_loading = self.mrms_loops.iter()
            .any(|loop_state| !radar_visible && Some(loop_state.layer) == field && loop_state.loading);
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
        #[cfg(target_arch = "wasm32")]
        let mut radar_memory_mib = self.settings.web_radar_memory_mib;
        let effective_loop = loop_frames.clamp(1, max_loop);
        let listed_loop = effective_loop.min(v.timeline.frames.len());
        let cached_loop = cached_loop_tail(&v.timeline.frames, listed_loop, |name| {
            self.scan_cache.contains(&name.to_string())
        });
        let mut action = None;
        let mut layout_choice = None;
        let mut show_layers = false;
        let mut mrms_step = 0isize;
        let mut mrms_boundary_step = 0isize;
        let mut mrms_seek = None;
        let mut mrms_toggle = false;
        let mut mrms_archive_request = None;
        let mut mrms_follow_change = None;
        let mut mrms_reverse_change = None;
        #[cfg(target_arch = "wasm32")]
        let (packs, pack_status) = (self.packs(), self.pack_status());
        #[cfg(target_arch = "wasm32")]
        let mut save_pack = false;
        #[cfg(target_arch = "wasm32")]
        let mut load_pack = None;
        let accent = egui::Color32::from_rgb(255, 190, 42);
        let expanded_id = egui::Id::new("zeta_transport_expanded");
        let split = self.chrome_rect.width() >= 1100.0;
        let mut expanded = !split || ctx.data_mut(|d| d.get_temp::<bool>(expanded_id).unwrap_or(false));
        let area = if expanded { egui::Area::new(egui::Id::new("zeta_transport"))
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
                            egui::ComboBox::from_id_salt("zeta_frame_count")
                                .width(92.0)
                                .height(600.0)
                                .selected_text(egui::RichText::new(format!("{effective_loop} Frames")).color(accent))
                                .show_ui(ui, |ui| {
                                    #[cfg(target_arch = "wasm32")]
                                    {
                                        super::scrubber::browser_radar_memory_controls(ui, &mut radar_memory_mib, max_loop);
                                        ui.separator();
                                    }
                                    let choices: &[usize] = &[2, 3, 4, 6, 7, 10, 14, 21, 30, 50, 75, 100, 150, 200, 250];
                                    for &count in choices {
                                        ui.add_enabled_ui(count <= max_loop, |ui| {
                                            ui.selectable_value(&mut loop_frames, count, format!("{count} Frames"));
                                        });
                                    }
                                    if !cfg!(target_arch = "wasm32") && max_loop < 7 {
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
                                    #[cfg(target_arch = "wasm32")] &mut radar_memory_mib,
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
                                if let Some(layer) = field.filter(|_| mrms_loop.is_some()) {
                                    egui::containers::menu::MenuButton::new("History")
                                        .config(egui::containers::menu::MenuConfig::new()
                                            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside))
                                        .ui(ui, |ui| {
                                        ui.set_min_width(360.0_f32.min((self.chrome_rect.width() - 48.0).max(240.0)));
                                        ui.strong("Public MRMS archive");
                                        ui.weak("Seek anywhere in a UTC day; each position loads six mosaics.");
                                        let default_at = mrms_archive_at.or(mrms_last_time).unwrap_or_else(Utc::now);
                                        let id = egui::Id::new(("mrms-archive-window", layer.stable_id()));
                                        let mut draft: (String, u32, Option<chrono::DateTime<Utc>>) = ui.data_mut(|d| d.get_temp(id))
                                            .unwrap_or_else(|| (default_at.format("%Y-%m-%d").to_string(),
                                                default_at.hour() * 60 + default_at.minute(), mrms_archive_at));
                                        if draft.2 != mrms_archive_at {
                                            draft = (default_at.format("%Y-%m-%d").to_string(),
                                                default_at.hour() * 60 + default_at.minute(), mrms_archive_at);
                                        }
                                        ui.horizontal(|ui| {
                                            ui.label("Date");
                                            ui.add(egui::TextEdit::singleline(&mut draft.0).desired_width(100.0))
                                                .on_hover_text("UTC date, YYYY-MM-DD");
                                        });
                                        ui.label(format!("UTC time {:02}:{:02}", draft.1 / 60, draft.1 % 60));
                                        let seek = ui.horizontal(|ui| {
                                            ui.weak("00:00");
                                            let width = (ui.available_width() - 55.0).clamp(140.0, 300.0);
                                            ui.spacing_mut().slider_width = width;
                                            let response = ui.add(egui::Slider::new(&mut draft.1, 0..=1439).show_value(false))
                                                .on_hover_text("Seek through the selected UTC day; release to load six frames");
                                            ui.weak("23:59");
                                            response
                                        }).inner;
                                        let parsed = chrono::NaiveDate::parse_from_str(&draft.0, "%Y-%m-%d").ok();
                                        let seek_finished = seek.drag_stopped() || (seek.changed() && !seek.dragged());
                                        let load_clicked = ui.add_enabled(parsed.is_some(), egui::Button::new("Load selected time"))
                                            .clicked();
                                        if (seek_finished || load_clicked) && parsed.is_some() {
                                            if let Some(at) = parsed.and_then(|day| day.and_hms_opt(draft.1 / 60, draft.1 % 60, 59)) {
                                                mrms_archive_request = Some(Some(at.and_utc().min(Utc::now())));
                                                ui.close();
                                            }
                                        }
                                        ui.horizontal(|ui| {
                                            if ui.add_enabled(mrms_first_time.is_some() && !mrms_loading && mrms_error.is_none(),
                                                egui::Button::new("← Earlier six"))
                                                .on_hover_text("Load the six mosaics before this window")
                                                .clicked() {
                                                mrms_archive_request = mrms_first_time.map(|time|
                                                    Some(time - chrono::Duration::seconds(1)));
                                                if let Some(Some(at)) = mrms_archive_request {
                                                    draft = (at.format("%Y-%m-%d").to_string(), at.hour() * 60 + at.minute(), Some(at));
                                                }
                                                ui.close();
                                            }
                                            if ui.add_enabled(mrms_archive_at.is_some() && mrms_last_time.is_some()
                                                && !mrms_loading && mrms_error.is_none()
                                                && mrms_last_time.is_some_and(|time| time < Utc::now() - chrono::Duration::minutes(12)),
                                                egui::Button::new("Later six →"))
                                                .on_hover_text("Load the next six mosaics toward live")
                                                .clicked() {
                                                mrms_archive_request = mrms_last_time.map(|time|
                                                    Some((time + chrono::Duration::minutes(13)).min(Utc::now())));
                                                if let Some(Some(at)) = mrms_archive_request {
                                                    draft = (at.format("%Y-%m-%d").to_string(), at.hour() * 60 + at.minute(), Some(at));
                                                }
                                                ui.close();
                                            }
                                        });
                                        if parsed.is_none() { ui.weak("Enter a complete YYYY-MM-DD date."); }
                                        if ui.button("Return to latest").clicked() {
                                            mrms_archive_request = Some(None);
                                            let now = Utc::now();
                                            draft = (now.format("%Y-%m-%d").to_string(), now.hour() * 60 + now.minute(), None);
                                            ui.close();
                                        }
                                        if mrms_archive_at.is_some() {
                                            let mut follow = mrms_follow_archive;
                                            if ui.checkbox(&mut follow, "Continue playback across archive windows").changed() {
                                                mrms_follow_change = Some(follow);
                                            }
                                        }
                                        let mut reverse = mrms_reverse_playback;
                                        if ui.checkbox(&mut reverse, "Play backward").on_hover_text(
                                            "Replay older mosaics first. With continuous archive playback enabled, cross into earlier six-frame windows."
                                        ).changed() {
                                            mrms_reverse_change = Some(reverse);
                                        }
                                        if let Some(error) = mrms_error.as_deref() {
                                            let message = if error.contains("no MRMS frames found before") {
                                                "No mosaics for this date and time. Choose another window."
                                            } else { error };
                                            ui.add(egui::Label::new(
                                                egui::RichText::new(message).color(egui::Color32::from_rgb(255, 190, 42))
                                            ).wrap());
                                        }
                                        ui.data_mut(|d| d.insert_temp(id, draft));
                                    });
                                }
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
                                    _ if mrms_loading => "Loading archive",
                                    _ if mrms_error.is_some() => "Archive unavailable",
                                    _ if mrms_archive_at.is_some() => "Archive",
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
                            layout_choice = zeta_layout_menu(ui, self.active_workspace.as_deref(),
                                self.analyst_open, self.views.len(), &self.settings.workspaces);
                            if split {
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.button(format!("{} Minimal controls", ph::CARET_DOWN))
                                        .on_hover_text("Collapse playback controls")
                                        .clicked() { expanded = false; }
                                });
                            }
                        });
                        let t = &mut self.views[idx].timeline;
                        if !radar_visible {
                            ui.weak(if mrms_loading {
                                "Loading selected history · previous mosaic remains visible".to_owned()
                            } else if let Some(at) = mrms_archive_at {
                                format!("Public MRMS archive · window ending {} UTC", at.format("%Y-%m-%d %H:%M"))
                            } else if field.is_some() {
                                if mrms_loop.is_some() { "Public MRMS mosaic history".to_owned() }
                                else { "Latest field only · mosaic history is not available".to_owned() }
                            } else if imagery_map {
                                "Radar timeline · imagery has separate time controls".to_owned()
                            } else {
                                "Radar timeline · radar hidden".to_owned()
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
                            if zeta_button(ui, ph::SKIP_BACK, "Previous frame", false).clicked() { self.views[idx].timeline.step_cut(-1); }
                            let playing = self.views[idx].timeline.playing;
                            if zeta_button(ui, if playing { ph::PAUSE } else { ph::PLAY }, if playing { "Pause" } else { "Play" }, playing).clicked() { self.views[idx].timeline.toggle_play(); }
                            if zeta_button(ui, ph::SKIP_FORWARD, "Next frame", false).clicked() { self.views[idx].timeline.step_cut(1); }
                            if zeta_button(ui, ph::BROADCAST, "Return to live", self.views[idx].timeline.following).clicked() { action = Some(PaletteAction::GoLive); }
                            ui.separator();
                            }
                            if let Some((len, index, playing)) = mrms_loop.filter(|(len, _, _)| *len > 1) {
                                if zeta_button(ui, ph::SKIP_BACK, "Previous mosaic", false).clicked() {
                                    if mrms_archive_at.is_some() && index == 0 { mrms_boundary_step = -1; }
                                    else { mrms_step = -1; }
                                }
                                if zeta_button(ui, if playing { ph::PAUSE } else { ph::PLAY },
                                    if playing { "Pause mosaic" } else if mrms_reverse_playback { "Play mosaic backward" }
                                    else { "Play mosaic forward" }, playing).clicked() { mrms_toggle = true; }
                                if zeta_button(ui, ph::SKIP_FORWARD, "Next mosaic", false).clicked() {
                                    if mrms_archive_at.is_some() && index + 1 == len { mrms_boundary_step = 1; }
                                    else { mrms_step = 1; }
                                }
                                if zeta_button(ui, ph::BROADCAST, "Latest mosaic", index + 1 == len).clicked() { mrms_seek = Some(len - 1); }
                                ui.weak(format!("{}/{len}", index + 1));
                                ui.separator();
                            }
                            if zeta_button(ui, ph::MAP_PIN, "Set chase location", self.tool == MapTool::Chase).clicked() { action = Some(PaletteAction::Tool(MapTool::Chase)); }
                            if zeta_button(ui, ph::CROSSHAIR, "Inspect radar value", self.tool == MapTool::Interrogate).clicked() { action = Some(PaletteAction::Tool(MapTool::Interrogate)); }
                            if zeta_button(ui, ph::STACK, "Layers", self.panel_open && self.panel_section == PanelSection::Overlays).clicked() { show_layers = true; }
                            if zeta_button(ui, ph::RULER, "Measure distance and bearing", self.tool == MapTool::Measure).clicked() { action = Some(PaletteAction::Tool(MapTool::Measure)); }
                            if zeta_button(ui, ph::PATH, "Draw a manual storm track", self.tool == MapTool::StormTrack).clicked() { action = Some(PaletteAction::Tool(MapTool::StormTrack)); }
                            if zeta_button(ui, ph::PENCIL_SIMPLE, "Draw on the map", self.tool == MapTool::Draw).clicked() { action = Some(PaletteAction::Tool(MapTool::Draw)); }
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
                        else { age.clone().unwrap_or_default() };
                        if !status.is_empty() {
                            ui.add(egui::Label::new(egui::RichText::new(status).size(11.0).color(egui::Color32::from_gray(190))).wrap())
                                .on_hover_text(format!("Displayed scan: {}\nRequested scan: {}", loaded_time.map_or_else(|| "unavailable".into(), |t| t.to_rfc3339()), requested_time.map_or_else(|| "unavailable".into(), |t| t.to_rfc3339())));
                        }
                    });
            })
        } else {
            egui::Area::new(egui::Id::new("zeta_transport_compact"))
                .order(egui::Order::Foreground)
                .constrain_to(self.chrome_rect)
                .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -18.0))
                .show(ctx, |ui| {
                    egui::Frame::new()
                        .fill(egui::Color32::from_rgba_unmultiplied(8, 11, 17, 244))
                        .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(68)))
                        .corner_radius(18.0)
                        .inner_margin(egui::Margin::symmetric(10, 8))
                        .show(ui, |ui| {
                            ui.spacing_mut().interact_size.y = 30.0;
                            ui.horizontal(|ui| {
                                if radar_visible {
                                    if zeta_button(ui, ph::SKIP_BACK, "Previous frame", false).clicked() { self.views[idx].timeline.step_cut(-1); }
                                    let playing = self.views[idx].timeline.playing;
                                    if zeta_button(ui, if playing { ph::PAUSE } else { ph::PLAY }, if playing { "Pause" } else { "Play" }, playing).clicked() { self.views[idx].timeline.toggle_play(); }
                                    if zeta_button(ui, ph::SKIP_FORWARD, "Next frame", false).clicked() { self.views[idx].timeline.step_cut(1); }
                                    ui.separator();
                                }
                                layout_choice = zeta_layout_menu(ui, self.active_workspace.as_deref(),
                                    self.analyst_open, self.views.len(), &self.settings.workspaces);
                                if zeta_button(ui, ph::MAP_PIN, "Set chase location", self.tool == MapTool::Chase).clicked() { action = Some(PaletteAction::Tool(MapTool::Chase)); }
                                if zeta_button(ui, ph::CROSSHAIR, "Inspect radar value", self.tool == MapTool::Interrogate).clicked() { action = Some(PaletteAction::Tool(MapTool::Interrogate)); }
                                if zeta_button(ui, ph::STACK, "Layers", self.panel_open && self.panel_section == PanelSection::Overlays).clicked() { show_layers = true; }
                                if zeta_button(ui, ph::RULER, "Measure distance and bearing", self.tool == MapTool::Measure).clicked() { action = Some(PaletteAction::Tool(MapTool::Measure)); }
                                if zeta_button(ui, ph::PATH, "Draw a manual storm track", self.tool == MapTool::StormTrack).clicked() { action = Some(PaletteAction::Tool(MapTool::StormTrack)); }
                                if zeta_button(ui, ph::PENCIL_SIMPLE, "Draw on the map", self.tool == MapTool::Draw).clicked() { action = Some(PaletteAction::Tool(MapTool::Draw)); }
                                if zeta_button(ui, ph::SLIDERS_HORIZONTAL, "Full playback controls", false).clicked() { expanded = true; }
                            });
                        });
                })
        };
        if split && !expanded {
            let history = egui::Area::new(egui::Id::new("zeta_history_compact"))
                .order(egui::Order::Foreground)
                .constrain_to(self.chrome_rect)
                .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(12.0, -18.0))
                .show(ctx, |ui| {
                    egui::Frame::new().fill(egui::Color32::from_rgba_unmultiplied(8, 11, 17, 244))
                        .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(68)))
                        .corner_radius(14.0).inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
                            if ui.button(format!("{} Frames  ·  {}", effective_loop, age.as_deref().unwrap_or("Waiting for scan")))
                                .on_hover_text("Open full radar playback and history").clicked() { expanded = !expanded; }
                        });
                });
            self.mobile_occlusion.push(history.response.rect);
            let product = egui::Area::new(egui::Id::new("zeta_product_compact"))
                .order(egui::Order::Foreground)
                .constrain_to(self.chrome_rect)
                .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-12.0, -18.0))
                .show(ctx, |ui| {
                    egui::Frame::new().fill(egui::Color32::from_rgba_unmultiplied(8, 11, 17, 244))
                        .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(68)))
                        .corner_radius(14.0).inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
                            self.radar_product_controls(ui, true);
                        });
                });
            self.mobile_occlusion.push(product.response.rect);
        }
        if split { ctx.data_mut(|d| d.insert_temp(expanded_id, expanded)); }
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
        if mrms_boundary_step != 0 {
            if let Some(layer) = active_mrms {
                self.request_mrms_adjacent_window(layer, mrms_boundary_step, false);
            }
        }
        if mrms_toggle {
            if let Some(loop_state) = self.mrms_loops.iter_mut().find(|loop_state| Some(loop_state.layer) == active_mrms) {
                loop_state.playing = !loop_state.playing;
                loop_state.last_step = Instant::now();
            }
        }
        if let (Some(layer), Some(follow)) = (active_mrms, mrms_follow_change) {
            if let Some(loop_state) = self.mrms_loops.iter_mut().find(|state| state.layer == layer) {
                loop_state.follow_through_archive = follow;
            }
        }
        if let (Some(layer), Some(reverse)) = (active_mrms, mrms_reverse_change) {
            if let Some(loop_state) = self.mrms_loops.iter_mut().find(|state| state.layer == layer) {
                loop_state.reverse_playback = reverse;
                loop_state.last_step = Instant::now();
            }
        }
        if let (Some(layer), Some(at)) = (active_mrms, mrms_archive_request) {
            if let Some(loop_state) = self.mrms_loops.iter_mut().find(|state| state.layer == layer) {
                loop_state.requested_at = at;
                loop_state.follow_from = None;
                loop_state.backward_from = None;
                if at.is_none() { loop_state.follow_through_archive = false; }
                loop_state.index = loop_state.frames.len().saturating_sub(1);
                loop_state.playing = false;
                loop_state.error = None;
                loop_state.loading = true;
            }
            self.fields.entry(layer).or_default().last_fetch = None;
        }
        self.settings.live_loop_frames = loop_frames;
        #[cfg(target_arch = "wasm32")]
        { self.settings.web_radar_memory_mib = radar_memory_mib; }
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
        match layout_choice {
            Some(ZetaLayoutChoice::Radar) => self.return_to_radar(ctx),
            Some(ZetaLayoutChoice::Workspace(index)) => self.activate_workspace_view(index, ctx),
            Some(ZetaLayoutChoice::Panes(count)) => {
                self.switch_mode(ViewMode::Analyst, ctx);
                self.set_pane_count(count);
                self.active_workspace = None;
            }
            None => {}
        }
    }
}

enum ZetaLayoutChoice { Radar, Workspace(usize), Panes(usize) }

fn zeta_layout_menu(ui: &mut egui::Ui, active_workspace: Option<&str>, analyst_open: bool,
    pane_count: usize, workspaces: &[crate::workspace::Workspace]) -> Option<ZetaLayoutChoice> {
    use egui_phosphor::regular as ph;
    let mut choice = None;
    egui::containers::menu::MenuButton::new(
        egui::RichText::new(ph::SQUARES_FOUR).size(18.0),
    )
    .config(egui::containers::menu::MenuConfig::new()
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside))
    .ui(ui, |ui| {
        ui.set_min_width(190.0);
        ui.strong("Map layout");
        if ui.selectable_label(!analyst_open && active_workspace.is_none(), "Radar")
            .named("Single radar layout").clicked() { choice = Some(ZetaLayoutChoice::Radar); ui.close(); }
        for name in ["Chase", "National overview", "Analysis"] {
            if let Some(index) = workspaces.iter().position(|ws| ws.name == name) {
                if ui.selectable_label(active_workspace == Some(name), name)
                    .named(name).clicked() { choice = Some(ZetaLayoutChoice::Workspace(index)); ui.close(); }
            }
        }
        ui.separator();
        ui.label("Panes");
        for count in [1, 2, 4] {
            if ui.selectable_label(pane_count == count, format!("{count} pane{}", if count == 1 { "" } else { "s" }))
                .named(&format!("Show {count} map panes")).clicked() {
                choice = Some(if count == 1 { ZetaLayoutChoice::Radar } else { ZetaLayoutChoice::Panes(count) });
                ui.close();
            }
        }
    }).0.named("Choose map layout and pane count");
    choice
}

fn zeta_button(ui: &mut egui::Ui, glyph: &str, label: &str, active: bool) -> egui::Response {
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
}
