//! What the signatures on the map are called, and what they mean.
//!
//! The app draws TDS rings, hail spikes and ZDR columns, and names them the way a warning
//! meteorologist would. That is the right vocabulary — but somebody who installed this to watch
//! the storm over their own house has no way to look any of it up without leaving the app for a
//! forum post of unknown vintage. Fifteen entries, searchable, no network.

/// One term: what it is called, and the two or three sentences that make it useful.
struct Entry {
    term: &'static str,
    /// What it is, in the plainest words that stay true.
    body: &'static str,
}

const ENTRIES: &[Entry] = &[
    Entry {
        term: "Hook echo",
        body: "The comma-shaped notch on the back-right of a supercell's reflectivity, where the \
               storm's rotating updraft wraps precipitation around itself. Where the app gets its \
               name. A hook is a reason to look at velocity, not a tornado by itself.",
    },
    Entry {
        term: "TDS — tornado debris signature",
        body: "Lofted debris seen by dual-pol: high reflectivity, near-zero velocity couplet, and \
               correlation coefficient dropping below about 0.8 because chunks of building are \
               nothing like raindrops. A TDS is confirmation that something on the ground is \
               being destroyed, so it is the one detection this app treats as urgent.",
    },
    Entry {
        term: "TBSS — three-body scatter spike (hail spike)",
        body: "A spike of weak echo pointing directly away from the radar behind a very strong \
               core. Radar energy bounces off large hail down to the ground and back up, arriving \
               late, so the radar paints it further out than it is. It is an artifact — and a \
               reliable sign of big hail.",
    },
    Entry {
        term: "ZDR column",
        body: "A plume of high differential reflectivity extending above the freezing level, \
               which means liquid raindrops are being carried up where only ice belongs. That \
               takes a strong updraft, and the column usually deepens ten to twenty minutes \
               before the storm produces.",
    },
    Entry {
        term: "Bright band",
        body: "A ring of enhanced reflectivity at the melting level in stratiform rain: snow \
               melts into a wet coating that looks huge to the radar, then collapses into \
               raindrops. Reads as a false ring of heavy rain around the radar in winter.",
    },
    Entry {
        term: "BWER — bounded weak echo region",
        body: "A pocket of weak echo inside a storm with strong echo above and around it. The \
               updraft there is so strong that precipitation has not had time to form. A \
               classic supercell signature, best seen in a vertical cross-section.",
    },
    Entry {
        term: "Bow echo",
        body: "A line segment that bulges downwind, driven by a rear-inflow jet. The apex is \
               where the strongest straight-line winds are. Bows are a wind threat first; \
               tornadoes, when they happen, come from the ends.",
    },
    Entry {
        term: "MARC — mid-altitude radial convergence",
        body: "A band of strong inbound-next-to-outbound velocity a few kilometres up along a \
               squall line, where the rear-inflow jet meets the front-to-rear flow. Often shows \
               ten to thirty minutes before damaging wind reaches the ground.",
    },
    Entry {
        term: "Velocity couplet",
        body: "Inbound and outbound velocities side by side, which is rotation. What matters is \
               how strong (Vrot), how tight, and how low — a tight couplet at the lowest tilt \
               close to the radar is a very different thing from a broad one at 20,000 feet.",
    },
    Entry {
        term: "Outflow boundary",
        body: "The leading edge of cold air spreading out from a storm's downdraft, drawn as a \
               thin line of weak echo. Storms that cross one often intensify: the boundary adds \
               low-level spin the updraft can tilt upright.",
    },
    Entry {
        term: "VIL — vertically integrated liquid",
        body: "How much water the radar sees in a column, in kg/m². High VIL means a deep, wet \
               storm. VIL density (VIL divided by echo top) is the better hail discriminator, \
               because it asks whether that water is packed into a short column.",
    },
    Entry {
        term: "MESO and TVS badges",
        body: "The radar's own algorithm output: MESO flags a mesocyclone-strength circulation, \
               TVS a tornadic vortex signature. They are pattern matches, not verdicts — plenty \
               of TVS flags never produce, and plenty of tornadoes never get one.",
    },
    Entry {
        term: "SRM — storm-relative velocity",
        body: "Velocity with the storm's own motion subtracted, so a rotation that is being \
               carried along at 50 knots stops hiding inside the translation. Turn it on when a \
               couplet looks like it might just be the storm moving.",
    },
    Entry {
        term: "Dual-pol moments",
        body: "The radar transmits horizontally and vertically. ZDR (differential reflectivity) \
               says whether targets are wide or tall — big raindrops flatten, hail tumbles. CC \
               (correlation coefficient) says whether they are all the same kind of thing; low \
               CC means a mixture, like debris. KDP tracks liquid water content and sees through \
               attenuation.",
    },
    Entry {
        term: "Hail spike vs hail core",
        body: "The core is the actual hail: a small area of very high reflectivity. The spike is \
               the artifact it throws behind itself, away from the radar. If you are judging \
               hail size, judge the core; the spike only tells you the core is serious.",
    },
];

#[derive(Default)]
pub struct Glossary {
    pub open: bool,
    query: String,
}

impl Glossary {
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }
}

pub fn show(state: &mut Glossary, ctx: &egui::Context) {
    if !state.open {
        return;
    }
    let mut open = state.open;
    let window = egui::Window::new("Glossary")
        .open(&mut open)
        .default_width(430.0)
        .default_height(480.0)
        .resizable(true);
    crate::ui::phone_surface(ctx, window).show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label("Find");
            ui.text_edit_singleline(&mut state.query);
            if !state.query.is_empty() && ui.button("✕").clicked() {
                state.query.clear();
            }
        });
        ui.separator();
        let q = state.query.trim().to_ascii_lowercase();
        // Search the bodies too: someone who only knows "the debris thing" should still land on
        // TDS without knowing to type three letters.
        let hits: Vec<&Entry> = ENTRIES
            .iter()
            .filter(|e| {
                q.is_empty()
                    || e.term.to_ascii_lowercase().contains(&q)
                    || e.body.to_ascii_lowercase().contains(&q)
            })
            .collect();
        egui::ScrollArea::vertical().show(ui, |ui| {
            if hits.is_empty() {
                ui.weak("Nothing matches that.");
            }
            for e in hits {
                ui.label(egui::RichText::new(e.term).strong());
                ui.label(e.body);
                ui.add_space(8.0);
            }
        });
    });
    state.open = open;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_entry_is_filled_in_and_named_once() {
        let mut seen = std::collections::HashSet::new();
        for e in ENTRIES {
            assert!(!e.term.is_empty() && !e.body.is_empty(), "{}", e.term);
            assert!(seen.insert(e.term), "duplicate term {}", e.term);
        }
        // The signatures the app itself draws must be in here, or the glossary is decoration.
        for must in ["TDS", "TBSS", "ZDR column", "Bright band"] {
            assert!(
                ENTRIES.iter().any(|e| e.term.contains(must)),
                "missing {must}"
            );
        }
    }
}
