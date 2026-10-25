//! Discover reported positions and explicitly shared stream links without guessing identities.
use super::*;

#[derive(Clone, Copy)]
struct Filters {
    in_view: bool,
    inside_warnings: bool,
    stream_links: bool,
}

impl Default for Filters {
    fn default() -> Self {
        Self {
            in_view: true,
            inside_warnings: false,
            stream_links: false,
        }
    }
}

impl Filters {
    fn matches(self, in_view: bool, inside_warning: bool, stream_link: bool) -> bool {
        (!self.in_view || in_view)
            && (!self.inside_warnings || inside_warning)
            && (!self.stream_links || stream_link)
    }
}

fn stream_link(url: &str) -> Option<String> {
    let url = reqwest::Url::parse(url.trim()).ok()?;
    (matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none())
    .then(|| url.to_string())
}

fn position_age(now: i64, reported: i64) -> String {
    if reported > now || reported <= 0 {
        "Position time unavailable".into()
    } else {
        let age = now.saturating_sub(reported);
        if age < 60 {
            "Position <1 min ago".into()
        } else if age < 3600 {
            format!(
                "Position {} min ago{}",
                age / 60,
                if age > 1800 { " · stale" } else { "" }
            )
        } else {
            format!("Position {} h ago · stale", age / 3600)
        }
    }
}

#[derive(Clone)]
enum Chaser {
    Spotter(wxdata::spotters::Spotter),
    Peer(crate::share::Peer),
}

impl Chaser {
    fn info(&self) -> (&str, f64, f64, i64, &'static str) {
        match self {
            Self::Spotter(sp) => (
                &sp.name,
                sp.lon,
                sp.lat,
                sp.time.timestamp(),
                "Spotter Network",
            ),
            Self::Peer(peer) => (
                &peer.name,
                peer.lon,
                peer.lat,
                peer.ts,
                "Shared chase partner",
            ),
        }
    }

    fn stream_link(&self) -> Option<String> {
        match self {
            Self::Spotter(_) => None,
            Self::Peer(peer) => stream_link(&peer.video_url),
        }
    }
}

impl HookEchoApp {
    pub(crate) fn show_chaser_dashboard(&mut self, ctx: &egui::Context) {
        let id = egui::Id::new("chaser_dashboard_filters");
        let mut filters = ctx.data_mut(|data| data.get_temp::<Filters>(id).unwrap_or_default());
        let now = Utc::now();
        let camera = self.views[self.active].camera;
        let viewport = self.last_viewport;
        let view_rect =
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(viewport.0, viewport.1));
        let site = self.views[self.active]
            .site
            .as_deref()
            .and_then(wxdata::sites::site_by_id)
            .map(|s| [s.longitude as f64, s.latitude as f64]);
        let range = self.settings.spotter_range_km.max(0.0);
        let warnings: Vec<_> = self
            .alert_features
            .iter()
            .filter(|feature| feature.kind == wxdata::overlay::FeatureKind::Warning
                && feature.alert.as_ref().is_none_or(|alert| alert.expires.is_none_or(|expiry| expiry > now)))
            .collect();
        let mut entries: Vec<_> = self
            .spotters
            .iter()
            .filter(|sp| sp.within_active_timeframe(now, self.settings.spotter_max_age_minutes))
            .filter(|sp| {
                site.is_some_and(|site| {
                    range <= 0.0 || crate::geo::great_circle(site, [sp.lon, sp.lat]).0 <= range
                })
            })
            .cloned()
            .map(Chaser::Spotter)
            .chain(self.peers.values().cloned().map(Chaser::Peer))
            .filter(|entry| {
                let (_, lon, lat, _, _) = entry.info();
                lon.is_finite()
                    && lat.is_finite()
                    && (-180.0..=180.0).contains(&lon)
                    && (-90.0..=90.0).contains(&lat)
            })
            .collect();
        entries.sort_by(|a, b| a.info().0.cmp(b.info().0).then(a.info().4.cmp(b.info().4)));
        let mut open = self.show_chaser_dashboard;
        let mut close = false;
        let mut refresh = false;
        let mut selected = None;
        let mut watch = None;
        let width = (ctx.content_rect().width() - 24.0).clamp(220.0, 430.0);
        egui::Window::new("Storm chasers")
            .open(&mut open)
            .title_bar(false)
            .default_width(width)
            .max_width(width)
            .max_height((ctx.content_rect().height() - 48.0).max(180.0))
            .show(ctx, |ui| {
                ui.spacing_mut().interact_size.y = 44.0;
                ui.horizontal(|ui| {
                    ui.heading("Storm chasers");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        close = ui.button("Close ×").clicked();
                    });
                });
                ui.horizontal_wrapped(|ui| {
                    ui.checkbox(&mut filters.in_view, "In view");
                    ui.checkbox(&mut filters.inside_warnings, "Inside current warnings");
                    ui.checkbox(&mut filters.stream_links, "Stream links only");
                });
                ui.small("Position age is separate from video availability. Stream links are not verified live.");
                ui.separator();
                let visible: Vec<_> = entries.iter().filter(|entry| {
                    let (_, lon, lat, _, _) = entry.info();
                    let world = crate::render::mercator::lonlat_to_world(lon, lat);
                    let (x, y) = camera.world_to_screen(world, viewport);
                    let in_view = camera.point_visible(world, viewport)
                        && view_rect.contains(egui::pos2(x, y));
                    let inside = warnings.iter().any(|warning| warning.contains(lon, lat));
                    filters.matches(in_view, inside, entry.stream_link().is_some())
                }).collect();
                ui.label(format!("{} reported positions", visible.len()));
                if visible.is_empty() {
                    if filters.stream_links {
                        ui.label("No shared stream links match these filters.");
                    } else if self.spotters.is_empty() && self.spotters_last_fetch
                        .is_none_or(|at| at.elapsed().as_secs() < 10) {
                        ui.horizontal(|ui| { ui.spinner(); ui.label("Checking Spotter Network positions…"); });
                    } else {
                        ui.label("No reported positions match these filters.");
                    }
                    if site.is_none() {
                        ui.small("Choose a radar site to display Spotter Network beacons.");
                    }
                    ui.small("Spotter Network does not supply video links. Shared chase partners can publish their own link.");
                }
                egui::ScrollArea::vertical()
                    .max_height((ctx.content_rect().height() - 300.0).max(120.0))
                    .show(ui, |ui| {
                        for entry in visible {
                            let (name, _, _, reported, source) = entry.info();
                            ui.strong(if name.is_empty() { "Unnamed chaser" } else { name });
                            ui.small(format!("{source} · {}", position_age(now.timestamp(), reported)));
                            if let Chaser::Spotter(sp) = entry { ui.small(&sp.status); }
                            let link = entry.stream_link();
                            ui.horizontal_wrapped(|ui| {
                                if ui.button("View on map").clicked() { selected = Some(entry.clone()); }
                                if let Some(url) = link {
                                    if ui.button("Watch stream").clicked() { watch = Some((name.to_string(), url)); }
                                } else {
                                    ui.weak("No stream link supplied");
                                }
                            });
                            ui.separator();
                        }
                    });
                ui.horizontal_wrapped(|ui| {
                    refresh = ui.button("Refresh positions").clicked();
                    ui.small("Network positions follow your report-age and range settings.");
                });
            });
        ctx.data_mut(|data| data.insert_temp(id, filters));
        self.show_chaser_dashboard = open && !close;
        if refresh {
            self.spotters_last_fetch = None;
            ctx.request_repaint();
        }
        if let Some(entry) = selected {
            let (_, lon, lat, _, _) = entry.info();
            self.views[self.active].camera.center =
                crate::render::mercator::lonlat_to_world(lon, lat);
            self.show_chaser_dashboard = false;
            match entry {
                Chaser::Spotter(sp) => self.open_spotter(&sp),
                Chaser::Peer(peer) => {
                    let stream = stream_link(&peer.video_url);
                    self.cell_popup = None;
                    self.detail = Some(Detail {
                        title: peer.name,
                        body: format!(
                            "SHARED CHASE PARTNER\n{}\n{:.4}°, {:.4}°\n{}",
                            position_age(now.timestamp(), peer.ts),
                            peer.lat,
                            peer.lon,
                            if stream.is_some() {
                                "Stream link supplied by this partner; live status is unverified."
                            } else {
                                "No stream link supplied."
                            }
                        ),
                        color: [255, 180, 60, 255],
                        image: None,
                        link: None,
                        stream,
                    });
                }
            }
            ctx.request_repaint();
        }
        if let Some((name, url)) = watch {
            self.watch_stream(name, url);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chaser_filters_combine_without_requiring_a_stream_by_default() {
        assert!(Filters::default().matches(true, false, false));
        assert!(!Filters::default().matches(false, true, true));
        let filters = Filters {
            in_view: false,
            inside_warnings: true,
            stream_links: true,
        };
        assert!(filters.matches(false, true, true));
        assert!(!filters.matches(true, false, true));
        assert!(!filters.matches(true, true, false));
    }

    #[test]
    fn chaser_stream_links_allow_web_only_without_embedded_credentials() {
        assert_eq!(
            stream_link(" https://www.youtube.com/watch?v=example "),
            Some("https://www.youtube.com/watch?v=example".into())
        );
        assert!(stream_link("http://example.com/stream.m3u8").is_some());
        for url in [
            "javascript:alert(1)",
            "file:///tmp/video",
            "data:text/html,test",
            "",
            "https://",
            "https://user:pass@example.com/live",
        ] {
            assert!(stream_link(url).is_none(), "accepted {url}");
        }
    }

    #[test]
    fn chaser_position_age_does_not_call_future_or_missing_reports_fresh() {
        assert_eq!(position_age(10000, 10030), "Position time unavailable");
        assert_eq!(position_age(10000, 0), "Position time unavailable");
        assert_eq!(position_age(10000, 9990), "Position <1 min ago");
        assert_eq!(position_age(10000, 9880), "Position 2 min ago");
        assert_eq!(position_age(10000, 8080), "Position 32 min ago · stale");
    }
}
