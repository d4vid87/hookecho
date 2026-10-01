//! Priority dock: one relevant bulletin, with full alerts and real SCIT tracks on demand.
use super::*;
use crate::ui::{a11y::Named as _, priority};

fn national_signal_counts(features: &[GeoFeature]) -> [usize; 5] {
    let mut seen: [std::collections::HashSet<String>; 5] = std::array::from_fn(|_| Default::default());
    for feature in features {
        let text = feature.title.to_ascii_uppercase();
        let group = match feature.kind {
            wxdata::overlay::FeatureKind::Warning if text.contains("EMERGENCY")
                || feature.alert.as_ref().is_some_and(|a| {
                    a.headline.to_ascii_uppercase().contains("EMERGENCY")
                        || a.description.to_ascii_uppercase().contains("EMERGENCY")
                }) => 0,
            wxdata::overlay::FeatureKind::Warning if text.contains("FLOOD") => 3,
            wxdata::overlay::FeatureKind::Warning => 1,
            wxdata::overlay::FeatureKind::Watch | wxdata::overlay::FeatureKind::WatchBox => 2,
            wxdata::overlay::FeatureKind::MesoDiscussion => 4,
            _ => continue,
        };
        let key = feature.alert.as_ref().map_or(feature.title.as_str(), |a| a.id.as_str());
        seen[group].insert(key.to_owned());
    }
    seen.map(|group| group.len())
}

#[cfg(test)]
mod signal_tests {
    use super::*;

    #[test]
    fn counts_unique_signals_by_type() {
        let feature = |kind, title: &str| GeoFeature {
            rings: Vec::new(), fill: [0; 4], stroke: [0; 4], kind,
            title: title.into(), detail: String::new(), alert: None,
        };
        let watch = feature(wxdata::overlay::FeatureKind::WatchBox, "Tornado Watch 123");
        let features = [
            watch.clone(), watch,
            feature(wxdata::overlay::FeatureKind::Warning, "Tornado Emergency"),
            feature(wxdata::overlay::FeatureKind::Warning, "Flood Warning"),
            feature(wxdata::overlay::FeatureKind::MesoDiscussion, "Mesoscale Discussion 234"),
        ];
        assert_eq!(national_signal_counts(&features), [1, 0, 1, 1, 1]);
    }
}

impl HookEchoApp {
    pub(crate) fn national_signal_strip(&mut self, ctx: &egui::Context) {
        let counts = national_signal_counts(&self.overlays);
        let width = (self.chrome_rect.width() - super::clearview::CORNER_WIDTH - 36.0)
            .clamp(440.0, 1100.0);
        let mut open_alerts = false;
        let area = egui::Area::new("national_signal_strip".into())
            .constrain_to(self.chrome_rect)
            .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(12.0, -74.0))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(egui::Color32::from_rgba_premultiplied(20, 37, 46, 244))
                    .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(85, 124, 147)))
                    .corner_radius(9.0)
                    .inner_margin(10.0)
                    .show(ui, |ui| {
                        ui.set_width(width);
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.set_width(140.0);
                                ui.small("NATIONAL SIGNALS");
                                ui.strong("Weather at a glance");
                            });
                            ui.separator();
                            let item_width = (width - 270.0) / 5.0;
                            for (label, count, color) in [
                                ("Emergency", counts[0], egui::Color32::from_rgb(255, 112, 120)),
                                ("Warnings", counts[1], egui::Color32::from_rgb(238, 202, 100)),
                                ("Watches", counts[2], egui::Color32::from_rgb(233, 191, 106)),
                                ("Flood", counts[3], egui::Color32::from_rgb(0, 188, 118)),
                                ("Discussions", counts[4], egui::Color32::from_rgb(143, 172, 255)),
                            ] {
                                ui.vertical(|ui| {
                                    ui.set_width(item_width);
                                    ui.colored_label(color, egui::RichText::new(count.to_string()).size(19.0).strong());
                                    ui.small(label);
                                });
                                ui.separator();
                            }
                            open_alerts = ui.button("All alerts ↗").clicked();
                        });
                    });
            });
        self.mobile_occlusion.push(area.response.rect);
        if open_alerts {
            self.apply_action(BindableAction::ToggleAlertPanel, ctx);
        }
    }

    pub(crate) fn priority_reference(&self) -> ((f64, f64), String) {
        if let Some(home) = self
            .settings
            .markers
            .iter()
            .find(|m| m.home && m.lon.is_finite() && m.lat.is_finite())
        {
            ((home.lon, home.lat), home.name.clone())
        } else if let Some(site) =
            wxdata::sites::all().find(|s| Some(s.id) == self.views[self.active].site.as_deref())
        {
            (
                (f64::from(site.longitude), f64::from(site.latitude)),
                site.id.to_string(),
            )
        } else {
            let center = self.views[self.active].camera.center;
            (
                crate::render::mercator::world_to_lonlat(center.0, center.1),
                "map center".into(),
            )
        }
    }
    pub(crate) fn priority_time(&self) -> chrono::DateTime<Utc> {
        if self.archive_bucket().is_some() {
            self.placefile_time()
        } else {
            Utc::now()
        }
    }
    pub(crate) fn map_alert_visible(&self, feature: &GeoFeature) -> bool {
        alerts::event_enabled(&feature.title, &self.settings.optional_alert_events)
            && self.filters.alert_cats[alerts::category(&feature.title).index()]
            && priority::map_visible(feature, self.priority_time())
    }
    pub(crate) fn refresh_priority_overlays(&mut self, ctx: &egui::Context) {
        // Rebuild on reference/settings changes, and once a minute for expiry. All alert feeds,
        // notification rules and the full bulletin list retain their original data.
        let point = self.priority_reference().0;
        let signature = (
            (point.0 * 10000.).round() as i64,
            (point.1 * 10000.).round() as i64,
            self.priority_time().timestamp() / 60,
            self.settings.priority_rules.clone(),
        );
        let id = egui::Id::new("priority_overlay_signature");
        let changed = ctx.data_mut(|d| {
            if d.get_temp::<(i64, i64, i64, crate::settings::PriorityRules)>(id)
                .as_ref()
                == Some(&signature)
            {
                false
            } else {
                d.insert_temp(id, signature);
                true
            }
        });
        if changed {
            self.rebuild_overlays();
        }
    }
    pub(crate) fn priority_dock(&mut self, ctx: &egui::Context) {
        let (point, reference) = self.priority_reference();
        let rows = priority::rows(
            self.active_alert_features(),
            &self.settings.priority_rules,
            point,
            self.priority_time(),
        );
        let count = rows.len();
        let first = rows.first().map(|f| {
            (
                f.alert.as_ref().unwrap().clone(),
                priority::category(f),
                f.contains(point.0, point.1),
                f.distance_km(point.0, point.1),
            )
        });
        let track_count = if self.cells_site.as_deref() == self.views[self.active].site.as_deref() {
            self.active_storm_cells()
                .iter()
                .filter(|c| {
                    !c.track.is_empty()
                        && priority::track_visible(c, &self.settings.priority_rules, Utc::now())
                })
                .count()
        } else {
            0
        };
        let mut bulletin = None;
        let mut alerts = false;
        let mut tracks = false;
        let rules_id = egui::Id::new("priority_rules_open");
        let tracks_id = egui::Id::new("priority_tracks_open");
        let mut rules_open = ctx.data_mut(|d| d.get_temp::<bool>(rules_id).unwrap_or(false));
        let mut tracks_open = ctx.data_mut(|d| d.get_temp::<bool>(tracks_id).unwrap_or(false));
        let width = super::clearview::CORNER_WIDTH - 30.0;
        let playback_height = ctx.data_mut(|d| d.get_temp::<egui::Rect>(egui::Id::new("corner_scrubber_rect"))).map_or(74.0, |r| r.height());
        let area = egui::Area::new("priority_dock".into())
            .constrain_to(self.chrome_rect)
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-super::clearview::CORNER_RIGHT, -(super::clearview::CORNER_BOTTOM + playback_height + 8.0)))
            .show(ctx, |ui| {
                crate::ui::style::glass(ui, 252)
                    .inner_margin(14)
                    .corner_radius(13.)
                    .show(ui, |ui| {
                        ui.set_width(width);
                        ui.set_max_width(width);
                        if let Some((a, level, inside, km)) = &first {
                            let color = priority::COLORS[*level];
                            egui::Frame::new()
                                .fill(color.gamma_multiply(0.09))
                                .stroke(egui::Stroke::new(1., color.gamma_multiply(0.6)))
                                .corner_radius(8.)
                                .inner_margin(12)
                                .show(ui, |ui| {
                                    ui.set_max_width(width - 26.);
                                    let place = if *inside {
                                        format!("covers {reference}")
                                    } else {
                                        format!(
                                            "{} from {reference}",
                                            crate::geo::fmt_distance(*km, self.metric(), 0)
                                        )
                                    };
                                    ui.colored_label(
                                        color,
                                        format!("{} · {place}", priority::LABELS[*level]),
                                    );
                                    ui.label(egui::RichText::new(&a.event).size(18.).strong());
                                    let expiry = a
                                        .expires
                                        .map(|t| {
                                            crate::timefmt::fmt_clock(t, self.active_tz(), false)
                                        })
                                        .unwrap_or_else(|| "unavailable".into());
                                    ui.horizontal_wrapped(|ui| {
                                        let short: String = a.area.chars().take(72).collect();
                                        ui.small(format!("{short} · expiry {expiry}"));
                                        if ui
                                            .button("Read bulletin ↗")
                                            .named("Read priority alert bulletin")
                                            .clicked()
                                        {
                                            bulletin = Some(a.id.clone());
                                        }
                                    });
                                    if let Some(tag) =
                                        a.damage_threat.as_deref().filter(|s| !s.is_empty())
                                    {
                                        ui.small(format!("Official impact: {tag}"));
                                    }
                                });
                            ui.add_space(7.);
                        } else {
                            ui.strong("No matching alert bulletins");
                            ui.small(format!("Around {reference} · dock thresholds applied"));
                            ui.add_space(5.);
                        }
                        ui.horizontal_wrapped(|ui| {
                            alerts = ui
                                .button(format!("Alerts · {count}"))
                                .named("Open full alerts list")
                                .clicked();
                            tracks = ui.button(format!("Tracks · {track_count}")).clicked();
                            if ui.button("Rules").named("Display rules").clicked() {
                                rules_open = true;
                            }
                            if self.archive_bucket().is_some() {
                                ui.weak("Archive");
                            }
                        });
                    });
            });
        self.mobile_occlusion.push(area.response.rect);
        if let Some(id) = bulletin {
            self.open_alert_popup(&id);
        }
        if alerts {
            self.apply_action(BindableAction::ToggleAlertPanel, ctx);
        }
        if tracks {
            tracks_open = true;
            self.filters.show_cells = true;
            self.filters.show_tracks = true;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            rules_open = false;
            tracks_open = false;
        }
        if rules_open {
            egui::Window::new("Priority display rules")
                .open(&mut rules_open)
                .default_width(430.)
                .resizable(false)
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.small(format!("Reference: {reference}"));
                    priority::controls(ui, &mut self.settings.priority_rules);
                });
        }
        if tracks_open {
            let station_matches =
                self.cells_site.as_deref() == self.views[self.active].site.as_deref();
            let cells: Vec<_> = self
                .active_storm_cells()
                .iter()
                .filter(|c| {
                    station_matches
                        && !c.track.is_empty()
                        && priority::track_visible(c, &self.settings.priority_rules, Utc::now())
                })
                .cloned()
                .collect();
            let mut selected = None;
            egui::Window::new("Storm tracks").open(&mut tracks_open).default_width(420.).collapsible(false)
                .show(ctx,|ui| {
                    ui.weak("Estimated SCIT paths. These do not issue or upgrade official warnings.");
                    ui.checkbox(&mut self.filters.show_tracks,"Draw qualifying tracks on the map");
                    if cells.is_empty() {ui.label("No tracks meet the current age and error limits."); ui.weak("Data may be unavailable, stale, or still loading. Use Layers → Radar to choose a radar site.");}
                    egui::ScrollArea::vertical().max_height(360.).show(ui,|ui| {
                        for cell in &cells {
                            ui.separator();
                            ui.horizontal(|ui| {
                                if ui.button(format!("Inspect {}",cell.id)).clicked() {selected=Some(cell.clone());}
                                let age=cell.time.map(|t|(Utc::now()-t).num_minutes().max(0)).unwrap_or(0);
                                ui.label(format!("{age} min old · error {:.1} nm",ui::cell_window::error_km(cell).unwrap_or(0.)/1.852));
                            });
                            if let (Some(speed),Some(dir))=(cell.mvt_kt,cell.mvt_deg) {ui.small(format!("Motion {speed:.0} kt toward {dir:.0}° · {} min projection",self.settings.priority_rules.track_horizon_min));}
                        }
                    });
                    if ui.button("Edit track limits").clicked() {rules_open=true;}
                });
            if let Some(cell) = selected {
                self.views[self.active].camera.center =
                    crate::render::mercator::lonlat_to_world(cell.lon, cell.lat);
                self.cell_popup = Some(cell);
                tracks_open = false;
            }
        }
        ctx.data_mut(|d| {
            d.insert_temp(rules_id, rules_open);
            d.insert_temp(tracks_id, tracks_open);
        });
    }
}
