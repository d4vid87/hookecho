//! Skew-T / hodograph window for a point sounding. A simplified Skew-T (temperature + dewpoint
//! vs log-pressure, temperature skewed) beside a hodograph of the wind profile.
//!
//! Two profiles can share the plot: the HRRR forecast (solid) and, when a radiosonde site is near
//! enough, the observed ascent from that site (dashed). Seeing them together is the point — the
//! model's idea of the atmosphere against a sample of the real one.

use wxdata::sounding::Sounding;

pub struct SoundingWindow {
    pub open: bool,
    pub busy: bool,
    pub sounding: Option<Sounding>,
    pub error: Option<String>,
    /// The observed radiosonde ascent nearest the clicked point, and the station it came from.
    pub observed: Option<Sounding>,
    pub observed_station: String,
    /// Why there's no observed profile — no station in range, or the balloon didn't fly.
    pub observed_error: Option<String>,
    pub show_observed: bool,
}

impl Default for SoundingWindow {
    fn default() -> Self {
        Self {
            open: false,
            busy: false,
            sounding: None,
            error: None,
            observed: None,
            observed_station: String::new(),
            observed_error: None,
            // On by default: an observed profile is the more trustworthy of the two, and hiding it
            // behind a toggle nobody finds would waste the fetch.
            show_observed: true,
        }
    }
}

impl SoundingWindow {
    pub fn show(&mut self, ctx: &egui::Context, tz: Option<wxdata::tz::Tz>) {
        if !self.open {
            return;
        }
        let mut open = self.open;
        crate::ui::fit_phone(ctx, egui::Window::new("Point Sounding"))
            .open(&mut open)
            .default_size([560.0, 460.0])
            .show(ctx, |ui| {
                if self.busy {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.weak("fetching HRRR profile…");
                    });
                    return;
                }
                if let Some(e) = &self.error {
                    ui.colored_label(egui::Color32::from_rgb(230, 120, 120), format!("Sounding unavailable: {e}"));
                    return;
                }
                let Some(s) = &self.sounding else {
                    ui.weak("Press Ctrl+K, pick \"Tool: Sounding\", then click a point on the map.");
                    return;
                };
                ui.horizontal(|ui| {
                    ui.strong(format!("{:.2}, {:.2}", s.lat, s.lon));
                    ui.separator();
                    ui.weak(format!("run {}", crate::timefmt::fmt_date_clock(s.run, tz)));
                    if let Some(sh) = s.bulk_shear_kt() {
                        ui.separator();
                        ui.label(format!("0–6 km shear ≈ {sh:.0} kt"));
                    }
                });
                // Fixed-layer composite indices (feature FF): the numbers a chaser scans first.
                if let Some(ix) = s.indices() {
                    ui.horizontal_wrapped(|ui| {
                        crate::theme::stat_card(ui, "SBCAPE", &format!("{:.0} J/kg", ix.sbcape));
                        crate::theme::stat_card(ui, "LCL", &format!("{:.0} m", ix.lcl_m));
                        crate::theme::stat_card(ui, "SRH 0–1", &format!("{:.0}", ix.srh1));
                        crate::theme::stat_card(ui, "SRH 0–3", &format!("{:.0}", ix.srh3));
                        crate::theme::stat_card(ui, "SCP", &format!("{:.1}", ix.scp));
                        crate::theme::stat_card(ui, "STP", &format!("{:.1}", ix.stp));
                        crate::theme::stat_card(ui, "EHI 0–1", &format!("{:.1}", ix.ehi1));
                    });
                    ui.weak("Fixed-layer forms from 10 mandatory levels — coarser than SPC mesoanalysis.");
                }
                // Observed ascent: a line about where it came from, and the toggle.
                ui.horizontal(|ui| match (&self.observed, &self.observed_error) {
                    (Some(o), _) => {
                        ui.checkbox(&mut self.show_observed, "Observed (RAOB)");
                        ui.weak(format!(
                            "{} · {}",
                            self.observed_station,
                            crate::timefmt::fmt_date_clock(o.run, tz)
                        ));
                    }
                    (None, Some(e)) => {
                        ui.weak(format!("Observed sounding: {e}"));
                    }
                    (None, None) => {
                        ui.weak("Observed sounding: fetching\u{2026}");
                    }
                });
                let observed = self.show_observed.then_some(self.observed.as_ref()).flatten();
                ui.separator();
                // Phone: the fixed-width plots (300 + 240 px) side by side overflow the screen —
                // stack them vertically inside a scroll instead (fixed-width content overrides
                // fit_phone's max_width; same pattern as cell_window's grid).
                if cfg!(target_os = "android") {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        skewt(ui, s, observed);
                        ui.add_space(6.0);
                        hodograph(ui, s, observed);
                    });
                } else {
                    ui.horizontal(|ui| {
                        skewt(ui, s, observed);
                        hodograph(ui, s, observed);
                    });
                }
            });
        self.open = open;
    }
}

/// Simplified Skew-T: temperature (red) and dewpoint (green) plotted against log-pressure, with
/// temperature skewed 45° to the right (the classic emagram layout).
fn skewt(ui: &mut egui::Ui, s: &Sounding, observed: Option<&Sounding>) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(300.0, 380.0), egui::Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 4.0, ui.visuals().extreme_bg_color);
    let grid = ui
        .visuals()
        .widgets
        .noninteractive
        .bg_stroke
        .color
        .gamma_multiply(0.6);

    // Vertical axis: log pressure 1000 (bottom) → 200 (top). Horizontal: temperature -40..40 C.
    let (p_bot, p_top) = (1000f64.ln(), 200f64.ln());
    let (t_min, t_max) = (-40.0f64, 40.0f64);
    let y_of = |hpa: f64| {
        let f = (hpa.ln() - p_bot) / (p_top - p_bot);
        rect.bottom() - 26.0 - f as f32 * (rect.height() - 40.0)
    };
    // Skew: shift temperature right as pressure decreases (higher up).
    let x_of = |temp_c: f64, hpa: f64| {
        let f = (temp_c - t_min) / (t_max - t_min);
        let skew = (p_bot - hpa.ln()) / (p_bot - p_top) * 60.0; // px of rightward skew at top
        rect.left() + 34.0 + f as f32 * (rect.width() - 44.0) + skew as f32
    };

    // Pressure gridlines + labels.
    for &hpa in &[1000.0, 850.0, 700.0, 500.0, 300.0, 200.0] {
        let y = y_of(hpa);
        p.hline(rect.x_range(), y, egui::Stroke::new(1.0, grid));
        p.text(
            egui::pos2(rect.left() + 3.0, y),
            egui::Align2::LEFT_CENTER,
            format!("{hpa:.0}"),
            egui::FontId::proportional(9.0),
            ui.visuals().weak_text_color(),
        );
    }

    let trace = |src: &Sounding,
                 color: egui::Color32,
                 dashed: bool,
                 pick: &dyn Fn(&wxdata::sounding::SoundingLevel) -> f64| {
        let pts: Vec<egui::Pos2> = src
            .levels
            .iter()
            .filter(|l| l.pressure_hpa >= 195.0) // the plot stops at 200 hPa
            .map(|l| egui::pos2(x_of(pick(l), l.pressure_hpa), y_of(l.pressure_hpa)))
            .collect();
        if pts.len() < 2 {
            return;
        }
        let stroke = egui::Stroke::new(2.0, color);
        if dashed {
            p.extend(egui::Shape::dashed_line(&pts, stroke, 5.0, 4.0));
        } else {
            p.add(egui::Shape::line(pts, stroke));
        }
    };
    let (green, red) = (
        egui::Color32::from_rgb(120, 230, 120),
        egui::Color32::from_rgb(240, 90, 90),
    );
    // Observed underneath, so the forecast trace stays readable where they overlap.
    if let Some(o) = observed {
        trace(o, green.gamma_multiply(0.85), true, &|l| l.dewpt_c);
        trace(o, red.gamma_multiply(0.85), true, &|l| l.temp_c);
    }
    trace(s, green, false, &|l| l.dewpt_c);
    trace(s, red, false, &|l| l.temp_c);
    p.text(
        rect.center_top() + egui::vec2(0.0, 10.0),
        egui::Align2::CENTER_TOP,
        "Skew-T",
        egui::FontId::proportional(11.0),
        ui.visuals().weak_text_color(),
    );
}

/// Hodograph: wind (u, v) at each level, connected surface→top, in knots.
fn hodograph(ui: &mut egui::Ui, s: &Sounding, observed: Option<&Sounding>) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(240.0, 380.0), egui::Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 4.0, ui.visuals().extreme_bg_color);
    let grid = ui
        .visuals()
        .widgets
        .noninteractive
        .bg_stroke
        .color
        .gamma_multiply(0.6);
    let center = rect.center();
    let max_kt = 80.0f32;
    let r_px = (rect.width().min(rect.height()) / 2.0) - 20.0;
    // Range rings every 20 kt.
    for kt in [20.0, 40.0, 60.0, 80.0] {
        p.circle_stroke(center, r_px * kt / max_kt, egui::Stroke::new(1.0, grid));
    }
    p.line_segment(
        [
            egui::pos2(center.x - r_px, center.y),
            egui::pos2(center.x + r_px, center.y),
        ],
        egui::Stroke::new(1.0, grid),
    );
    p.line_segment(
        [
            egui::pos2(center.x, center.y - r_px),
            egui::pos2(center.x, center.y + r_px),
        ],
        egui::Stroke::new(1.0, grid),
    );

    let to_px = |u_ms: f64, v_ms: f64| {
        let (u_kt, v_kt) = (u_ms * 1.943_844, v_ms * 1.943_844);
        // East = +x, North = +y (up on screen).
        egui::pos2(
            center.x + r_px * (u_kt as f32 / max_kt),
            center.y - r_px * (v_kt as f32 / max_kt),
        )
    };
    // The observed hodograph is dashed and dimmer, drawn first so the forecast sits on top. Its
    // levels are thinned to the plotted layer: a 111-level radiosonde otherwise draws a hairball
    // of stratospheric wind above everything a chaser cares about.
    if let Some(o) = observed {
        let opts: Vec<egui::Pos2> = o
            .levels
            .iter()
            .filter(|l| l.pressure_hpa >= 250.0)
            .map(|l| to_px(l.u_ms, l.v_ms))
            .collect();
        if opts.len() >= 2 {
            p.extend(egui::Shape::dashed_line(
                &opts,
                egui::Stroke::new(2.0, egui::Color32::from_rgb(120, 180, 255).gamma_multiply(0.7)),
                5.0,
                4.0,
            ));
        }
    }
    let pts: Vec<egui::Pos2> = s.levels.iter().map(|l| to_px(l.u_ms, l.v_ms)).collect();
    if pts.len() >= 2 {
        p.add(egui::Shape::line(
            pts.clone(),
            egui::Stroke::new(2.0, egui::Color32::from_rgb(120, 180, 255)),
        ));
    }
    if let Some(sfc) = pts.first() {
        p.circle_filled(*sfc, 3.0, egui::Color32::WHITE); // surface marker
    }
    p.text(
        rect.center_top() + egui::vec2(0.0, 10.0),
        egui::Align2::CENTER_TOP,
        "Hodograph (kt)",
        egui::FontId::proportional(11.0),
        ui.visuals().weak_text_color(),
    );
}
