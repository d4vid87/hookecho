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
    fn matches(self, in_view: impl FnOnce() -> bool, inside_warning: impl FnOnce() -> bool,
        stream_link: impl FnOnce() -> bool) -> bool {
        (!self.in_view || in_view())
            && (!self.inside_warnings || inside_warning())
            && (!self.stream_links || stream_link())
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

fn full_report_tooltip(response: egui::Response, text: &str) {
    let width = (response.ctx.content_rect().width() - 32.0).clamp(1.0, 450.0);
    response.on_hover_ui(|ui| {
        ui.set_max_width(width);
        ui.add(egui::Label::new(text).wrap());
    });
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
            Self::Spotter(sp) => sp.youtube_url.clone(),
            Self::Peer(peer) => stream_link(&peer.video_url),
        }
    }
}

impl HookEchoApp {
    pub(crate) fn request_chaser_status(&mut self, url: &str, ctx: &egui::Context) {
        let Some(target) = wxdata::youtube::Target::parse(url) else { return; };
        let key = self.settings.youtube_key.trim().to_string();
        if key.is_empty() { return; }
        let url = target.url();
        if self.chaser_video_checks.get(&url).is_some_and(|check| check.requested.elapsed().as_secs() < 60 || check.result.is_none()) { return; }
        if self.chaser_video_checks.values().filter(|check| check.result.is_none()).count() >= 4 { return; }
        if self.chaser_video_checks.len() >= 64 {
            if let Some(oldest) = self.chaser_video_checks.iter().min_by_key(|(_, check)| check.requested).map(|(url, _)| url.clone()) {
                self.chaser_video_checks.remove(&oldest);
            }
        }
        let requested = Instant::now();
        self.chaser_video_checks.insert(url.clone(), ChaserVideoCheck { requested, result: None });
        let http = self.http.clone();
        let tx = self.overlay_tx.clone();
        let ctx = ctx.clone();
        self.spawner.spawn(async move {
            let result = wxdata::task::timeout(std::time::Duration::from_secs(25), wxdata::youtube::check(&http, target, &key)).await
                .unwrap_or_else(Err).map_err(|error| error.to_string());
            let _ = tx.send(OverlayDelivery::Immediate(OverlayMsg::ChaserVideo { url, requested, result }));
            ctx.request_repaint();
        });
    }

    pub(crate) fn chaser_video_state(&self, url: &str) -> wxdata::youtube::State {
        let Some(target) = wxdata::youtube::Target::parse(url) else { return wxdata::youtube::State::Unknown; };
        self.chaser_video_checks.get(&target.url()).and_then(|check| check.result.as_ref())
            .and_then(|result| result.as_ref().ok()).map_or(wxdata::youtube::State::Unknown, |status| status.current_state(Utc::now()))
    }

    pub(crate) fn chaser_watch_url(&self, url: &str) -> String {
        let Some(target) = wxdata::youtube::Target::parse(url) else { return url.into(); };
        self.chaser_video_checks.get(&target.url()).and_then(|check| check.result.as_ref())
            .and_then(|result| result.as_ref().ok()).filter(|status| status.current_state(Utc::now()) == wxdata::youtube::State::Live)
            .and_then(|status| status.watch_url.clone()).unwrap_or_else(|| target.url())
    }

    pub(crate) fn chaser_video_label(&self, url: &str) -> String {
        let Some(target) = wxdata::youtube::Target::parse(url) else { return "Live status unknown".into(); };
        let Some(check) = self.chaser_video_checks.get(&target.url()) else {
            return if self.settings.youtube_key.trim().is_empty() { "Live status unknown · optional YouTube API key required".into() } else { "Live status unknown · not checked".into() };
        };
        match &check.result {
            None => "Checking YouTube…".into(),
            Some(Err(error)) => format!("Live status unknown · {error}"),
            Some(Ok(status)) => {
                let channel = status.channel_title.as_deref().map(|title| format!(" · {title}")).unwrap_or_default();
                format!("{}{} · checked {} ago", status.current_state(Utc::now()).label(), channel, humanize((Utc::now() - status.checked_at).num_seconds().max(0)))
            }
        }
    }

    pub(crate) fn refresh_chaser_detail(&mut self) {
        let source = self.detail.as_ref().and_then(|detail| detail.link.as_ref())
            .filter(|(label, _)| label == "Self-published YouTube").map(|(_, url)| url.clone());
        let Some(url) = source else { return; };
        let label = self.chaser_video_label(&url);
        let watch = self.chaser_watch_url(&url);
        if let Some(detail) = &mut self.detail {
            if let Some((prefix, _)) = detail.body.rsplit_once("\nVideo: ") {
                detail.body = format!("{prefix}\nVideo: {label}");
            }
            detail.stream = Some(watch);
        }
    }

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
        let mut check_video = None;
        let width = (ctx.content_rect().width() - 24.0).clamp(220.0, 430.0);
        egui::Window::new("Storm chasers")
            .order(egui::Order::Foreground)
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
                ui.small("YouTube links are self-published by the chaser. Position freshness does not establish live video or verify identity.");
                ui.separator();
                let warnings: Vec<_> = if filters.inside_warnings {
                    self.alert_features.iter().filter(|feature| feature.kind == wxdata::overlay::FeatureKind::Warning
                        && feature.alert.as_ref().is_none_or(|alert| alert.expires.is_none_or(|expiry| expiry > now))).collect()
                } else { Vec::new() };
                let visible: Vec<_> = entries.iter().filter(|entry| {
                    let (_, lon, lat, _, _) = entry.info();
                    filters.matches(|| {
                        let world = crate::render::mercator::lonlat_to_world(lon, lat);
                        let (x, y) = camera.world_to_screen(world, viewport);
                        camera.point_visible(world, viewport) && view_rect.contains(egui::pos2(x, y))
                    }, || warnings.iter().any(|warning| warning.contains(lon, lat)),
                        || entry.stream_link().is_some())
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
                    ui.small("A spotter can publish a YouTube URL in their public Web field; shared chase partners can supply a stream link.");
                }
                let body_height = ui.text_style_height(&egui::TextStyle::Body);
                let small_height = ui.text_style_height(&egui::TextStyle::Small);
                let row_height = body_height + 3.0 * small_height + 45.0
                    + 5.0 * ui.spacing().item_spacing.y;
                egui::ScrollArea::vertical()
                    .max_height((ctx.content_rect().height() - 300.0).max(120.0))
                    .show_rows(ui, row_height, visible.len(), |ui, rows| {
                        for index in rows {
                            let entry = visible[index];
                            ui.push_id(index, |ui| {
                                let (name, _, _, reported, source) = entry.info();
                                let name = if name.is_empty() { "Unnamed chaser" } else { name };
                                let position = format!("{source} · {}", position_age(now.timestamp(), reported));
                                let status = if let Chaser::Spotter(sp) = entry { sp.status.as_str() } else { "" };
                                // Fixed rows virtualize the list; full report text remains available on hover and in map details.
                                for (text, height, strong) in [(name, body_height, true), (position.as_str(), small_height, false), (status, small_height, false)] {
                                    let label = egui::RichText::new(text);
                                    let response = ui.add_sized([ui.available_width(), height], egui::Label::new(if strong { label.strong() } else { label.small() }).truncate().show_tooltip_when_elided(false).halign(egui::Align::Min));
                                    full_report_tooltip(response, text);
                                }
                                let link = entry.stream_link();
                                let youtube = link.as_deref().is_some_and(|url| wxdata::youtube::Target::parse(url).is_some());
                                let count = 1 + usize::from(link.is_some()) + usize::from(youtube);
                                let button_width = ((ui.available_width() - ui.spacing().item_spacing.x * (count - 1) as f32) / count as f32).max(1.0);
                                ui.horizontal(|ui| {
                                    if ui.add_sized([button_width, 44.0], egui::Button::new("View on map").truncate())
                                        .on_hover_text("View on map").clicked() { selected = Some(entry.clone()); }
                                    if let Some(url) = link.as_deref() {
                                        let state = self.chaser_video_state(url);
                                        let label = if state == wxdata::youtube::State::Live { "Watch live" } else { "Open stream" };
                                        if ui.add_sized([button_width, 44.0], egui::Button::new(label).truncate()).on_hover_text(label).clicked() {
                                            watch = Some((name.to_string(), self.chaser_watch_url(url)));
                                        }
                                        if youtube && ui.add_enabled_ui(!self.settings.youtube_key.trim().is_empty(), |ui| {
                                            ui.add_sized([button_width, 44.0], egui::Button::new("Check status").truncate())
                                        }).inner.on_hover_text("Check status · Uses your optional YouTube Data API key. A check is fresh for two minutes.").clicked() {
                                            check_video = Some(url.to_string());
                                        }
                                    }
                                });
                                let video = match link.as_deref() {
                                    Some(url) if youtube => self.chaser_video_label(url),
                                    Some(_) => "Live status unknown".into(),
                                    None => "No stream link supplied".into(),
                                };
                                let response = ui.add_sized([ui.available_width(), small_height], egui::Label::new(egui::RichText::new(&video).small()).truncate().show_tooltip_when_elided(false).halign(egui::Align::Min));
                                full_report_tooltip(response, &video);
                                ui.add(egui::Separator::default().spacing(1.0));
                            });
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
                Chaser::Spotter(sp) => self.open_spotter(&sp, ctx),
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
        if let Some(url) = check_video { self.request_chaser_status(&url, ctx); }
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
        assert!(Filters::default().matches(|| true, || false, || false));
        assert!(!Filters::default().matches(|| false, || true, || true));
        let filters = Filters {
            in_view: false,
            inside_warnings: true,
            stream_links: true,
        };
        assert!(filters.matches(|| false, || true, || true));
        assert!(!filters.matches(|| true, || false, || true));
        assert!(!filters.matches(|| true, || true, || false));
    }

    #[test]
    fn chaser_filters_skip_disabled_and_short_circuited_work() {
        let disabled = Filters { in_view: false, inside_warnings: false, stream_links: false };
        assert!(disabled.matches(|| panic!("camera filter disabled"), || panic!("polygon filter disabled"), || panic!("stream filter disabled")));
        assert!(!Filters::default().matches(|| false, || panic!("offscreen polygon lookup"), || panic!("offscreen stream lookup")));
        assert!(Filters::default().matches(|| true, || panic!("polygon filter disabled"), || panic!("stream filter disabled")));
        let warnings = Filters { inside_warnings: true, ..disabled };
        assert!(!warnings.matches(|| panic!("camera filter disabled"), || false, || panic!("stream filter disabled")));
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
