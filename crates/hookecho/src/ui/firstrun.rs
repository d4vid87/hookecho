//! First run: one card, one decision — which radar you open to — and a way to skip making it.
//!
//! This replaces a four-card setup wizard that asked for a site, a basemap, a theme, keys, alert
//! sounds and saved locations before showing a single pixel of weather. Everything on those other
//! cards is a preference with a good default and a home in Settings; none of it is worth standing
//! between someone and the radar. Teaching the chrome is the tour's job ([`crate::ui::tour`]).
//!
//! The fast path is not this card at all: if a position source answers, the nearest site is picked
//! and the card closes itself. The list is what's left for everyone whose machine has no idea
//! where it is — which, on a desktop without gpsd, is most of them.

use crate::settings::Settings;

/// What the card hands back once the user is done with it.
pub struct Finish {
    /// The chosen home site, to load and fly to.
    pub site: String,
    pub take_tour: bool,
    /// The site was picked from a position fix rather than chosen. The caller says so in a toast,
    /// because a radar the user never selected should explain itself.
    pub located: bool,
}

#[derive(Default)]
pub struct FirstRun {
    pub open: bool,
    filter: String,
    /// Live position source, once asked for. Dropped when the card closes: this is a one-shot
    /// lookup, not chase mode, and chase mode starts its own.
    rx: Option<std::sync::mpsc::Receiver<(f64, f64)>>,
    /// A source was asked for and said no (no gpsd, no permission). Stops the button from looking
    /// like it does nothing.
    refused: bool,
}

impl FirstRun {
    pub fn start(&mut self) {
        self.open = true;
        self.filter.clear();
        self.rx = None;
        self.refused = false;
    }

    /// Ask the platform where we are. The same sources chase mode uses — Android polls the system
    /// LocationManager over JNI, desktop streams from a local gpsd, the web watches the browser's
    /// own Geolocation.
    fn locate(&mut self) {
        self.rx = if cfg!(target_os = "android") {
            crate::platform::start_location()
        } else {
            crate::gps::spawn()
        };
        self.refused = self.rx.is_none();
    }

    /// The nearest site to the first fix that arrives, if one has.
    fn fix_site(&mut self) -> Option<String> {
        let rx = self.rx.as_ref()?;
        let (lon, lat) = rx.try_recv().ok()?;
        crate::geo::nearest_site_id(lon, lat)
    }
}

/// Show the card. `Some` once there's a site to open — the caller marks `setup_done`, saves, flies
/// there, and starts the tour if it was asked for.
pub fn show(ctx: &egui::Context, fr: &mut FirstRun, settings: &mut Settings) -> Option<Finish> {
    if !fr.open {
        return None;
    }
    // A fix ends the card wherever the user is in it: they asked for the nearest radar, and this
    // is the whole of the ten-second start.
    if let Some(site) = fr.fix_site() {
        settings.default_site = site.clone();
        fr.open = false;
        fr.rx = None;
        return Some(Finish {
            site,
            take_tour: false,
            located: true,
        });
    }

    let mut finished = None;
    let mut open = true;
    crate::ui::phone_surface(ctx, egui::Window::new("Welcome to Hook Echo-WX"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.set_width(420.0_f32.min(ctx.content_rect().width() - 40.0));
            ui.label("Which radar should this open to? Everything else has a sensible default and lives in Settings.");
            ui.add_space(8.0);

            ui.horizontal(|ui| {
                if fr.rx.is_some() {
                    // getLastKnownLocation is null until the first fix lands, and gpsd can take a
                    // few seconds to see a satellite. Say so rather than look stuck.
                    ui.spinner();
                    ui.weak("Finding the nearest radar\u{2026}");
                    ctx.request_repaint_after(std::time::Duration::from_millis(250));
                } else if ui
                    .button(format!(
                        "{} Use my location",
                        egui_phosphor::regular::CROSSHAIR
                    ))
                    .on_hover_text(if cfg!(any(target_os = "android", target_arch = "wasm32")) {
                        "Asks for the location permission, picks the nearest radar, and gets out of the way"
                    } else {
                        "Reads a local gpsd on :2947 and picks the nearest radar"
                    })
                    .clicked()
                {
                    fr.locate();
                }
            });
            if fr.refused {
                ui.small(if cfg!(any(target_os = "android", target_arch = "wasm32")) {
                    "No position yet \u{2014} pick a radar below instead."
                } else {
                    "No gpsd on this machine \u{2014} pick a radar below instead."
                });
            }

            ui.add_space(8.0);
            ui.add(
                egui::TextEdit::singleline(&mut fr.filter)
                    .hint_text("Search by ID, city, or state\u{2026}"),
            );
            let needle = fr.filter.to_ascii_uppercase();
            egui::ScrollArea::vertical()
                .max_height(220.0)
                .show(ui, |ui| {
                    for s in wxdata::sites::sites() {
                        if !needle.is_empty()
                            && !s.id.to_ascii_uppercase().contains(&needle)
                            && !s.city.to_ascii_uppercase().contains(&needle)
                            && !s.state.to_ascii_uppercase().contains(&needle)
                        {
                            continue;
                        }
                        let label = format!("{}  \u{2014}  {}, {}", s.id, s.city, s.state);
                        if ui
                            .selectable_label(settings.default_site == s.id, label)
                            .clicked()
                        {
                            settings.default_site = s.id.to_string();
                        }
                    }
                });

            ui.add_space(8.0);
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Show me the radar").clicked() {
                    finished = Some(false);
                }
                if ui
                    .button("\u{2026}with the 60-second tour")
                    .on_hover_text("Four stops on the live map: the timeline, the products, where everything lives, and how to read a storm")
                    .clicked()
                {
                    finished = Some(true);
                }
            });
        });

    let finished = finished.map(|take_tour| Finish {
        site: settings.default_site.clone(),
        take_tour,
        located: false,
    });
    if finished.is_some() || !open {
        fr.open = false;
        fr.rx = None;
    }
    finished
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fix_picks_the_nearest_site_and_closes() {
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send((-97.5, 35.47)).unwrap(); // Oklahoma City, right by KTLX
        let mut fr = FirstRun {
            open: true,
            rx: Some(rx),
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let mut settings = Settings::default();
        // No frame needed: the fix short-circuits `show` before anything is drawn.
        let out = show(&ctx, &mut fr, &mut settings).expect("a fix finishes the card");
        assert_eq!(out.site, "KTLX");
        assert!(out.located && !out.take_tour);
        assert!(!fr.open);
        assert_eq!(settings.default_site, "KTLX");
    }

    #[test]
    fn no_fix_leaves_the_card_up() {
        let mut fr = FirstRun {
            open: true,
            ..Default::default()
        };
        let mut settings = Settings::default();
        let ctx = egui::Context::default();
        ctx.begin_pass(Default::default());
        assert!(show(&ctx, &mut fr, &mut settings).is_none());
        let _ = ctx.end_pass();
        assert!(fr.open, "nothing was chosen, so the card stays");
    }
}
