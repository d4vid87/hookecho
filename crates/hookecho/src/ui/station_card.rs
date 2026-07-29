//! Floating live-telemetry cards: one window per station, camera on top, sparklines below.
//!
//! Each card owns a ring buffer that starts empty when the card opens and fills as the poller
//! brings observations in. That is the honest shape of it: the networks publish *current*
//! conditions, not history, so a card that has been open ninety seconds can draw ninety seconds.
//! Windows longer than the card's own life read "—" rather than a flat line pretending to be data.

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use egui::{Color32, RichText};
use std::collections::VecDeque;
use wxdata::stations::StationOb;

/// One polled reading. Everything is optional because every network omits something.
#[derive(Debug, Clone, Copy)]
pub struct Sample {
    pub t: DateTime<Utc>,
    pub temp_c: Option<f32>,
    pub dewp_c: Option<f32>,
    pub rh_pct: Option<f32>,
    pub wspd_kt: Option<f32>,
    pub gust_kt: Option<f32>,
    pub pressure_mb: Option<f32>,
    /// Electric field, in whatever unit [`EField::unit`] says — the two sources are not the same
    /// quantity and are never mixed in one card.
    pub efield: Option<f32>,
}

/// Which electric-field source a card is charting, so the label can never lie about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EFieldKind {
    /// NOAA's PPEF model: equatorial ionospheric field, mV/m, ~300 km up.
    Ppef,
    /// A ground field mill: local thunderstorm charge, kV/m.
    Mill,
}

impl EFieldKind {
    pub fn unit(self) -> &'static str {
        match self {
            EFieldKind::Ppef => "mV/m",
            EFieldKind::Mill => "kV/m",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            EFieldKind::Ppef => "Electric field (ionospheric, PPEF)",
            EFieldKind::Mill => "Electric field (field mill)",
        }
    }

    pub fn note(self) -> &'static str {
        match self {
            EFieldKind::Ppef => {
                "NOAA's equatorial ionospheric model, predicted from solar wind — space weather, \
                 not the storm overhead"
            }
            EFieldKind::Mill => "Ground field mill: past a few kV/m, charge is building nearby",
        }
    }
}

/// How long a card remembers. An hour at a one-minute poll is 60 samples — nothing, and it keeps
/// the longer wind windows honestly bounded.
const HISTORY: usize = 720;

/// A live station card.
pub struct Card {
    /// Ring-buffer key, `network:id`.
    pub key: String,
    pub ob: StationOb,
    pub history: VecDeque<Sample>,
    /// Which camera is showing, if any: its label, its owner, and the stream itself.
    pub cam_name: String,
    pub cam_owner: String,
    /// Live video. Desktop only — a phone gets the newest still instead, refreshed on the poll
    /// clock, because HLS needs an ffmpeg it cannot spawn and decoding video per open card is not
    /// what a handset's battery is for.
    #[cfg(not(target_os = "android"))]
    pub cam: Option<crate::cam::Stream>,
    /// The still-image texture key, for cameras that post frames instead of streaming.
    pub still_key: Option<String>,
    pub tex: Option<egui::TextureHandle>,
    pub last_seq: u64,
    pub efield_kind: EFieldKind,
    /// Set false by the window's close button; the app drops the card (and its stream) after.
    pub open: bool,
}

impl Card {
    pub fn new(ob: StationOb, efield_kind: EFieldKind) -> Card {
        Card {
            key: ob.key(),
            ob,
            history: VecDeque::new(),
            cam_name: String::new(),
            cam_owner: String::new(),
            #[cfg(not(target_os = "android"))]
            cam: None,
            still_key: None,
            tex: None,
            last_seq: 0,
            efield_kind,
            open: true,
        }
    }

    /// Record a poll. Sampled at the time we asked, not the time the station reported: an
    /// airport METAR is an hour old the moment it lands and repeats until the next one, so the
    /// series steps and holds through the gap rather than staying a single point forever. How
    /// stale the underlying report is stays visible on the card's own "updated N ago" line.
    pub fn push(&mut self, ob: StationOb, efield: Option<f32>) {
        let t = Utc::now();
        self.history.push_back(Sample {
            t,
            temp_c: ob.temp_c,
            dewp_c: ob.dewp_c,
            rh_pct: ob.rh_pct,
            wspd_kt: ob.wspd_kt,
            gust_kt: ob.gust_kt,
            pressure_mb: ob.pressure_mb,
            efield,
        });
        while self.history.len() > HISTORY {
            self.history.pop_front();
        }
        self.ob = ob;
    }

    /// Peak gust within the trailing `secs`. `None` when the card has not been open that long —
    /// the card says "—" rather than reporting a short window as if it were a long one.
    pub fn peak_gust(&self, secs: i64) -> Option<f32> {
        let (first, last) = (self.history.front()?, self.history.back()?);
        // One sample covers no window at all, so a fresh card reports every window as unknown
        // rather than repeating its only reading across all six.
        if (last.t - first.t).num_seconds() < secs {
            return None;
        }
        let cutoff = last.t - ChronoDuration::seconds(secs);
        self.history
            .iter()
            .filter(|s| s.t >= cutoff)
            .filter_map(|s| s.gust_kt.or(s.wspd_kt))
            .fold(None, |acc: Option<f32>, v| Some(acc.map_or(v, |a| a.max(v))))
    }

    fn series(&self, f: impl Fn(&Sample) -> Option<f32>) -> Vec<f32> {
        self.history.iter().filter_map(f).collect()
    }
}

fn c_to_f(c: f32) -> f32 {
    c * 9.0 / 5.0 + 32.0
}

/// The trailing-window ladder from the storm-telemetry dashboards this card follows.
const GUST_WINDOWS: [(&str, i64); 6] = [
    ("10 s", 10),
    ("30 s", 30),
    ("60 s", 60),
    ("5 min", 300),
    ("12 h", 43_200),
    ("24 h", 86_400),
];

/// Draw one card. Returns `false` when the user closed it.
pub fn show(
    ctx: &egui::Context,
    card: &mut Card,
    tz: Option<wxdata::tz::Tz>,
    index: usize,
) -> bool {
    let mut open = card.open;
    let title = format!("{} — {}", card.ob.name, card.ob.network.label());
    // Cascade the first placement so several cards don't land on top of each other.
    let offset = 24.0 * (index % 8) as f32;
    let w = crate::ui::fit_phone(
        ctx,
        egui::Window::new(RichText::new(&title).size(13.0))
            .id(egui::Id::new(("station-card", &card.key)))
            .open(&mut open)
            .default_pos([80.0 + offset, 90.0 + offset]),
    );
    // fit_phone pins the width on Android; on desktop the card sizes itself.
    let w = if cfg!(target_os = "android") {
        w
    } else {
        w.default_width(330.0).resizable(true)
    };
    w.show(ctx, |ui| {
            camera_pane(ui, card);
            header(ui, card, tz);
            ui.separator();
            temperature_block(ui, card);
            ui.separator();
            wind_block(ui, card);
            ui.separator();
            efield_block(ui, card);
            still_note(ui);
        });
    card.open = open;
    open
}

/// The video (or still) pane, its owner chip, and the permission banner. Agency cameras are
/// published for looking at, not for putting on a stream — the banner says so where a viewer
/// will actually read it.
fn camera_pane(ui: &mut egui::Ui, card: &mut Card) {
    #[cfg(not(target_os = "android"))]
    let status = card.cam.as_ref().map(|c| c.status());
    #[cfg(target_os = "android")]
    let status: Option<()> = None;

    if card.tex.is_none() && status.is_none() && card.still_key.is_none() {
        return;
    }
    let w = ui.available_width();
    match &card.tex {
        Some(tex) => {
            let size = tex.size_vec2();
            let h = if size.x > 0.0 { w * size.y / size.x } else { 180.0 };
            ui.add(egui::Image::new(tex).fit_to_exact_size(egui::vec2(w, h)));
        }
        None => {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 120.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 4.0, Color32::from_gray(24));
            let msg = camera_status_text(&status);
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                msg,
                egui::FontId::proportional(12.0),
                Color32::from_gray(190),
            );
        }
    }
    ui.horizontal_wrapped(|ui| {
        if !card.cam_owner.is_empty() {
            chip(ui, &card.cam_owner, Color32::from_rgb(60, 90, 130));
        }
        if !card.cam_name.is_empty() {
            ui.label(RichText::new(&card.cam_name).size(11.0).weak());
        }
    });
    ui.label(
        RichText::new("NOT FOR BROADCAST WITHOUT OWNER PERMISSION")
            .size(9.5)
            .color(Color32::from_rgb(220, 120, 90)),
    );
    ui.add_space(2.0);
}

/// On a phone the picture is the newest still, not video. Say so rather than letting a frame
/// that updates once a minute read as a frozen stream.
#[cfg(target_os = "android")]
fn still_note(ui: &mut egui::Ui) {
    ui.label(
        RichText::new("Camera shows the newest still; live video is desktop-only")
            .size(9.5)
            .weak(),
    );
}

#[cfg(not(target_os = "android"))]
fn still_note(_ui: &mut egui::Ui) {}

/// The line shown in place of a picture, in whatever terms that platform can honour.
#[cfg(not(target_os = "android"))]
fn camera_status_text(status: &Option<crate::cam::Status>) -> String {
    match status {
        Some(crate::cam::Status::NeedsFfmpeg) => {
            "This camera streams HLS — install ffmpeg to watch it".to_string()
        }
        Some(crate::cam::Status::Offline(e)) => format!("Camera offline: {e}"),
        Some(crate::cam::Status::Connecting) | Some(crate::cam::Status::Playing) => {
            "Connecting\u{2026}".to_string()
        }
        _ => "Waiting for a frame\u{2026}".to_string(),
    }
}

#[cfg(target_os = "android")]
fn camera_status_text(_status: &Option<()>) -> String {
    "Waiting for a frame\u{2026}".to_string()
}

fn chip(ui: &mut egui::Ui, text: &str, bg: Color32) {
    egui::Frame::new()
        .fill(bg)
        .corner_radius(3.0)
        .inner_margin(egui::Margin::symmetric(5, 1))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(10.5).color(Color32::WHITE));
        });
}

/// Where the station is, what time it is there, and how old the reading is.
fn header(ui: &mut egui::Ui, card: &Card, tz: Option<wxdata::tz::Tz>) {
    ui.horizontal_wrapped(|ui| {
        chip(ui, card.ob.network.label(), Color32::from_rgb(70, 70, 80));
        ui.label(
            RichText::new(format!("{:.3}, {:.3}", card.ob.lat, card.ob.lon))
                .size(10.5)
                .weak(),
        );
        if let Some(e) = card.ob.elev_m {
            ui.label(RichText::new(format!("{:.0} m", e)).size(10.5).weak());
        }
    });
    ui.horizontal_wrapped(|ui| {
        let now = Utc::now();
        let clock = match tz {
            Some(z) => now.with_timezone(&z).format("%H:%M:%S %Z").to_string(),
            None => now.format("%H:%M:%SZ").to_string(),
        };
        ui.label(RichText::new(clock).size(12.0).strong());
        if let Some(t) = card.ob.time {
            let age = (now - t).num_seconds().max(0);
            let (txt, col) = if age < 300 {
                (format!("updated {age}s ago"), Color32::from_rgb(120, 200, 130))
            } else {
                (
                    format!("updated {} min ago", age / 60),
                    Color32::from_rgb(220, 180, 90),
                )
            };
            ui.label(RichText::new(txt).size(10.5).color(col));
        }
    });
}

fn temperature_block(ui: &mut egui::Ui, card: &Card) {
    ui.label(RichText::new("Outdoor temperature").size(11.0).weak());
    ui.horizontal(|ui| {
        match card.ob.temp_c {
            Some(t) => ui.label(RichText::new(format!("{:.1}°F", c_to_f(t))).size(28.0).strong()),
            None => ui.label(RichText::new("—").size(28.0).weak()),
        };
        ui.vertical(|ui| {
            ui.label(
                RichText::new(match card.ob.rh_pct {
                    Some(h) => format!("Humidity {h:.0}%"),
                    None => "Humidity —".into(),
                })
                .size(11.0),
            );
            ui.label(
                RichText::new(match card.ob.dewp_c {
                    Some(d) => format!("Dewpoint {:.1}°F", c_to_f(d)),
                    None => "Dewpoint —".into(),
                })
                .size(11.0),
            );
        });
    });
    let temps: Vec<f32> = card.series(|s| s.temp_c).iter().map(|c| c_to_f(*c)).collect();
    sparkline(ui, &temps, Color32::from_rgb(230, 140, 90));
}

fn wind_block(ui: &mut egui::Ui, card: &Card) {
    ui.label(RichText::new("Wind").size(11.0).weak());
    ui.horizontal(|ui| {
        dial(ui, card.ob.wdir_deg);
        ui.vertical(|ui| {
            ui.label(
                RichText::new(match card.ob.wspd_kt {
                    Some(s) => format!("{s:.0} kt sustained"),
                    None => "— sustained".into(),
                })
                .size(15.0)
                .strong(),
            );
            ui.label(
                RichText::new(match (card.ob.gust_kt, card.ob.wdir_deg) {
                    (Some(g), Some(d)) => format!("gusting {g:.0} kt from {d:.0}°"),
                    (Some(g), None) => format!("gusting {g:.0} kt"),
                    (None, Some(d)) => format!("from {d:.0}°"),
                    (None, None) => "no gust reported".into(),
                })
                .size(11.0),
            );
        });
    });
    egui::Grid::new(("gusts", &card.key))
        .num_columns(6)
        .spacing([8.0, 2.0])
        .show(ui, |ui| {
            for (label, _) in GUST_WINDOWS {
                ui.label(RichText::new(label).size(9.5).weak());
            }
            ui.end_row();
            for (_, secs) in GUST_WINDOWS {
                match card.peak_gust(secs) {
                    Some(g) => ui.label(RichText::new(format!("{g:.0}")).size(12.0).strong()),
                    None => ui.label(RichText::new("—").size(12.0).weak()),
                };
            }
            ui.end_row();
        });
    sparkline(
        ui,
        &card.series(|s| s.gust_kt.or(s.wspd_kt)),
        Color32::from_rgb(110, 180, 240),
    );
}

fn efield_block(ui: &mut egui::Ui, card: &Card) {
    let kind = card.efield_kind;
    ui.label(RichText::new(kind.title()).size(11.0).weak());
    let latest = card.history.iter().rev().find_map(|s| s.efield);
    ui.horizontal(|ui| {
        match latest {
            Some(v) => ui.label(
                RichText::new(format!("{v:+.3} {}", kind.unit()))
                    .size(16.0)
                    .strong(),
            ),
            None => ui.label(RichText::new("—").size(16.0).weak()),
        };
    });
    sparkline(
        ui,
        &card.series(|s| s.efield),
        Color32::from_rgb(190, 150, 240),
    );
    ui.label(RichText::new(kind.note()).size(9.5).weak());
}

/// A compass dial with a needle pointing the way the wind is coming from.
fn dial(ui: &mut egui::Ui, dir: Option<f32>) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(46.0, 46.0), egui::Sense::hover());
    let p = ui.painter();
    let c = rect.center();
    let r = rect.width() * 0.45;
    p.circle_stroke(c, r, egui::Stroke::new(1.0, Color32::from_gray(110)));
    for (i, tick) in ["N", "E", "S", "W"].iter().enumerate() {
        let a = (i as f32) * std::f32::consts::FRAC_PI_2 - std::f32::consts::FRAC_PI_2;
        p.text(
            c + egui::vec2(a.cos(), a.sin()) * (r + 5.0),
            egui::Align2::CENTER_CENTER,
            tick,
            egui::FontId::proportional(8.0),
            Color32::from_gray(140),
        );
    }
    if let Some(d) = dir {
        // Meteorological convention: the needle points at where the wind comes from.
        let a = (d - 90.0).to_radians();
        let tip = c + egui::vec2(a.cos(), a.sin()) * r;
        p.line_segment(
            [c, tip],
            egui::Stroke::new(2.0, Color32::from_rgb(110, 180, 240)),
        );
        p.circle_filled(c, 2.0, Color32::from_rgb(110, 180, 240));
    }
}

/// A bare sparkline: the series scaled to its own range, with a flat line when it never moved.
fn sparkline(ui: &mut egui::Ui, values: &[f32], color: Color32) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 26.0), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 2.0, Color32::from_black_alpha(40));
    if values.len() < 2 {
        p.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "collecting\u{2026}",
            egui::FontId::proportional(9.0),
            Color32::from_gray(120),
        );
        return;
    }
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for v in values {
        lo = lo.min(*v);
        hi = hi.max(*v);
    }
    let span = if (hi - lo).abs() < 1e-6 { 1.0 } else { hi - lo };
    let pts: Vec<egui::Pos2> = values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let x = rect.left() + rect.width() * (i as f32) / (values.len() - 1) as f32;
            let y = rect.bottom() - rect.height() * (v - lo) / span;
            egui::pos2(x, y.clamp(rect.top() + 1.0, rect.bottom() - 1.0))
        })
        .collect();
    p.add(egui::Shape::line(pts, egui::Stroke::new(1.3, color)));
    // Electric field lives in thousandths where temperature lives in tens, so let the range
    // itself pick the precision rather than rounding a real reading away to "0.0 – 0.0".
    let dp = match hi.abs().max(lo.abs()) {
        m if m >= 10.0 => 0,
        m if m >= 1.0 => 1,
        _ => 3,
    };
    p.text(
        rect.right_top() + egui::vec2(-3.0, 1.0),
        egui::Align2::RIGHT_TOP,
        format!("{lo:.dp$} – {hi:.dp$}"),
        egui::FontId::proportional(8.5),
        Color32::from_gray(150),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use wxdata::stations::Network;

    fn ob(secs: i64, gust: f32) -> StationOb {
        StationOb {
            id: "X".into(),
            name: "Test".into(),
            network: Network::Tempest,
            lat: 34.0,
            lon: -84.0,
            time: Some(Utc.timestamp_opt(1_785_000_000 + secs, 0).unwrap()),
            temp_c: Some(20.0),
            dewp_c: Some(15.0),
            rh_pct: Some(72.0),
            wdir_deg: Some(180.0),
            wspd_kt: Some(8.0),
            gust_kt: Some(gust),
            pressure_mb: Some(1010.0),
            precip_rate_mmh: None,
            elev_m: None,
        }
    }

    #[test]
    fn a_repeated_report_holds_rather_than_stalling_the_series() {
        let mut card = Card::new(ob(0, 10.0), EFieldKind::Ppef);
        card.push(ob(0, 10.0), None);
        card.push(ob(0, 10.0), None);
        // An hourly METAR repeats between reports; the series holds its value across the gap
        // instead of freezing at one point forever.
        assert_eq!(card.history.len(), 2);
        assert_eq!(card.history[1].temp_c, Some(20.0));
    }

    #[test]
    fn gust_windows_only_report_what_the_card_has_seen() {
        let mut card = Card::new(ob(0, 10.0), EFieldKind::Ppef);
        // Samples are stamped when they are polled, so drive the clock directly here.
        let t0 = Utc::now() - ChronoDuration::seconds(60);
        for (i, g) in [10.0, 25.0, 14.0].iter().enumerate() {
            card.push(ob(0, *g), None);
            card.history.back_mut().unwrap().t = t0 + ChronoDuration::seconds(i as i64 * 30);
        }
        // One minute of samples: the 60 s window is answerable, the 12 h one is not.
        assert_eq!(card.peak_gust(60), Some(25.0));
        assert_eq!(card.peak_gust(43_200), None);
        // A shorter window keeps only the samples inside it.
        assert_eq!(card.peak_gust(30), Some(25.0));
        // A card with one reading covers no window, however short.
        let mut fresh = Card::new(ob(0, 10.0), EFieldKind::Ppef);
        fresh.push(ob(0, 10.0), None);
        assert_eq!(fresh.history.len(), 1);
        assert_eq!(fresh.peak_gust(10), None);
    }

    #[test]
    fn history_is_bounded() {
        let mut card = Card::new(ob(0, 5.0), EFieldKind::Ppef);
        for i in 0..(HISTORY + 50) {
            card.push(ob(i as i64 * 60, 5.0), None);
        }
        assert_eq!(card.history.len(), HISTORY);
    }

    #[test]
    fn efield_units_never_cross() {
        assert_eq!(EFieldKind::Ppef.unit(), "mV/m");
        assert_eq!(EFieldKind::Mill.unit(), "kV/m");
        assert!(EFieldKind::Ppef.note().contains("not the storm overhead"));
    }
}
