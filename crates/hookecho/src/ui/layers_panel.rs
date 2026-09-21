//! The Layers panel: a searchable, categorized list of every layer/product/tool in the app,
//! browsed through category tiles or the active list. Desktop uses floating cards; Android
//! hosts the same body in a bottom sheet. Both read the one action registry
//! (`HookEchoApp::palette_entries`), so they can never drift apart.

use crate::app::{HealthState, PaletteAction, PaletteEntry, SourceHealth};
use crate::ui::a11y::Named as _;
use egui::{vec2, Color32, RichText, Stroke};

/// Category order in the panel (anything else falls to the bottom, in registry order).
pub(crate) const CATEGORIES: [&str; 9] = [
    "Radar",
    "National",
    "Severe",
    "Obs",
    "Models",
    "Reference",
    "Tools",
    "MRMS",
    "Settings",
];

/// Case-insensitive subsequence match with a compactness score: lower is a tighter match.
/// `None` = no match. Empty needle matches everything at score 0.
pub(crate) fn fuzzy(needle: &str, hay: &str) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    let hay: Vec<char> = hay.to_lowercase().chars().collect();
    let mut score = 0usize;
    let mut at = 0usize;
    for nc in needle.to_lowercase().chars() {
        if nc == ' ' {
            continue;
        }
        let found = hay[at..].iter().position(|h| *h == nc)?;
        score += found; // characters skipped between matches — tighter matches score lower
        at += found + 1;
    }
    Some(score)
}

/// Filter + sort entry indices for `query` (best match first, registry order within a tie).
pub(crate) fn matches(entries: &[PaletteEntry], query: &str) -> Vec<usize> {
    let mut hits: Vec<(usize, bool, usize, usize)> = entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            let descriptor = match e.action {
                PaletteAction::ToggleField(layer) => layer.descriptor(),
                _ => None,
            };
            std::iter::once(e.label.as_str())
                .chain(descriptor.into_iter().flat_map(|d| {
                    std::iter::once(d.display_name)
                        .chain(std::iter::once(d.short_name))
                        .chain(d.search_aliases.iter().copied())
                        .chain(std::iter::once(d.units))
                }))
                .filter_map(|term| fuzzy(query, term))
                .min()
                .map(|score| (score, !e.favorite, e.recent.unwrap_or(usize::MAX), i))
        })
        .collect();
    hits.sort_unstable();
    hits.into_iter().map(|(_, _, _, i)| i).collect()
}

/// Row height: one line, tall enough to scan without turning the panel into a wall.
const ROW_H: f32 = 32.0;

fn category_name(category: &str) -> &'static str {
    match category {
        "Radar" => "Radar products",
        "National" => "National weather",
        "Severe" => "Severe weather",
        "Obs" => "Observations",
        "Models" => "Forecast models",
        "Reference" => "Map reference",
        "Tools" => "Tools",
        "MRMS" => "MRMS",
        _ => "Settings",
    }
}

fn category_glyph(category: &str) -> &'static str {
    use egui_phosphor::regular as ph;
    match category {
        "Radar" => ph::BROADCAST,
        "National" => ph::GLOBE,
        "Severe" => ph::WARNING,
        "Obs" => ph::THERMOMETER,
        "Models" => ph::CHART_LINE,
        "Reference" => ph::MAP_TRIFOLD,
        "Tools" => ph::WRENCH,
        "MRMS" => ph::GLOBE,
        _ => ph::GEAR,
    }
}

/// The row's icon, picked from the label and falling back to the category.
///
/// Derived rather than stored: a per-entry `icon` field would be ~100 registry edits to keep in
/// sync by hand, and the labels already say what the thing is.
pub(crate) fn glyph(e: &PaletteEntry) -> &'static str {
    use egui_phosphor::regular as ph;
    let l = e.label.to_lowercase();
    let has = |w: &str| l.contains(w);
    match () {
        _ if has("velocity")
            || has("azshear")
            || has("rotation")
            || has("srv")
            || has("srh")
            || has("spin") =>
        {
            ph::ARROWS_CLOCKWISE
        }
        _ if has("hail") || has("mesh") => ph::CIRCLE,
        _ if has("tornado") || has("tds") => ph::TORNADO,
        _ if has("lightning") || has("glm") => ph::LIGHTNING,
        _ if has("snow") || has("winter") || has("ice") => ph::SNOWFLAKE,
        _ if has("rain")
            || has("qpe")
            || has("precip")
            || has("flood")
            || has("vil")
            || has("moisture") =>
        {
            ph::DROP
        }
        _ if has("wind") => ph::WIND,
        _ if has("temp") || has("dewpoint") => ph::THERMOMETER,
        _ if has("satellite") || has("cloud") || has("smoke") => ph::CLOUD,
        _ if has("surge") || has("buoy") || has("wave") || has("river") => ph::WAVES,
        _ if has("pirep") || has("sigmet") || has("airmet") || has("recon") => ph::AIRPLANE_TILT,
        _ if has("warning") || has("alert") || has("outlook") || has("watch") => ph::WARNING,
        _ if has("sounding") || has("vad") || has("cape") || has("chart") => ph::CHART_LINE,
        _ if has("cross-section") || has("3d") || has("cappi") || has("volume") => ph::CUBE,
        _ if has("measure") || has("range") || has("distance") => ph::RULER,
        _ if has("marker") || has("place") || has("gauge") || has("spotter") => ph::MAP_PIN,
        _ if has("basemap") || has("map") || has("terrain") => ph::MAP_TRIFOLD,
        _ if has("site") || has("radar site") || has("mosaic") => ph::BROADCAST,
        _ if has("camera") || has("webcam") => ph::CAMERA,
        _ if has("setting") || has("preference") => ph::GEAR,
        _ => match e.category {
            "Radar" => ph::RADIO_BUTTON,
            "National" => ph::GLOBE,
            "Severe" => ph::WARNING,
            "Obs" => ph::THERMOMETER,
            "Models" => ph::CHART_LINE,
            "Reference" => ph::MAP_TRIFOLD,
            _ => ph::CROSSHAIR,
        },
    }
}

/// Move `drag` to sit where `before` is inside `seq`, and record the result in `pref` — the
/// persisted, cross-category label order. Only the moved category's labels are rewritten, so
/// reordering Radar leaves an earlier drag in Obs alone.
pub(crate) fn reorder(pref: &mut Vec<String>, seq: &[String], drag: &str, before: &str) {
    if drag == before {
        return;
    }
    let mut next: Vec<String> = seq.iter().filter(|s| *s != drag).cloned().collect();
    let at = next.iter().position(|s| s == before).unwrap_or(next.len());
    next.insert(at, drag.to_string());
    pref.retain(|s| !seq.iter().any(|q| q == s));
    pref.extend(next);
}

/// One full-width row: the name, a state dot on the right, the description on hover. It used to
/// be a two-line 52 px card, which turned a category into a wall and pushed everything below the
/// fold; the description is a hint, not something you read twenty times in a row.
/// `draggable` puts the icon on a drag handle; the returned response covers the whole row and is
/// what the caller tests for a drop.
/// What a row click did: toggled the layer, or asked what the label's abbreviation means.
struct Hit {
    clicked: bool,
    favorite: Option<crate::render::FieldLayer>,
    /// Index into [`crate::ui::glossary::ENTRIES`], when the ⓘ was the thing clicked.
    explain: Option<usize>,
    resp: egui::Response,
}

fn compact_age(age: std::time::Duration) -> String {
    let seconds = age.as_secs();
    match seconds {
        0..=4 => "now".into(),
        5..=59 => format!("{seconds}s"),
        60..=3599 => format!("{}m", seconds / 60),
        3600..=86_399 => format!("{}h", seconds / 3600),
        _ => format!("{}d", seconds / 86_400),
    }
}

pub(crate) fn health_look(state: HealthState) -> (&'static str, Color32) {
    match state {
        HealthState::Fresh => ("Fresh", Color32::from_rgb(70, 200, 120)),
        HealthState::Fetching => ("Fetching", Color32::from_rgb(80, 160, 240)),
        HealthState::Stale => ("Stale", Color32::from_rgb(235, 180, 70)),
        HealthState::Failed => ("Failed", Color32::from_rgb(230, 90, 90)),
        HealthState::Waiting => ("Waiting", Color32::from_gray(110)),
    }
}

fn age_line(age: Option<std::time::Duration>) -> String {
    age.map_or_else(|| "never".into(), |d| format!("{} ago", compact_age(d)))
}

fn health_popup(ui: &mut egui::Ui, health: &SourceHealth) {
    let state = health.state();
    let (label, color) = health_look(state);
    ui.set_min_width(250.0);
    ui.strong(&health.source);
    ui.colored_label(
        color,
        if state == HealthState::Failed && health.last_success.is_some() {
            "Failed — showing previous data (degraded)"
        } else {
            label
        },
    );
    egui::Grid::new(("source_health", &health.source))
        .num_columns(2)
        .show(ui, |ui| {
            ui.weak("Data valid");
            ui.label(age_line(health.data_age));
            ui.end_row();
            ui.weak("Last success");
            ui.label(age_line(health.last_success));
            ui.end_row();
            ui.weak("Last attempt");
            ui.label(age_line(health.last_attempt));
            ui.end_row();
            ui.weak("Next retry");
            ui.label(if health.fetching {
                "in progress".into()
            } else {
                health
                    .next_retry()
                    .map_or_else(|| "waiting".into(), |d| format!("in {}", compact_age(d)))
            });
            ui.end_row();
            ui.weak("Requests");
            ui.label(format!("{} ok · {} failed", health.successes, health.failures));
            ui.end_row();
        });
    if let Some(error) = &health.error {
        ui.separator();
        ui.weak("Last error");
        ui.colored_label(Color32::from_rgb(230, 120, 120), error);
    }
}

fn row(ui: &mut egui::Ui, e: &PaletteEntry, accent: Color32, draggable: bool) -> Hit {
    let on = e.on.unwrap_or(false);
    let glass = true;
    let (fg, bg) = if on {
        (
            accent,
            Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 24),
        )
    } else {
        (ui.visuals().text_color(), ui.visuals().faint_bg_color)
    };
    let icon = RichText::new(glyph(e)).size(14.0).color(if on {
        accent
    } else {
        ui.visuals().weak_text_color()
    });
    let mut clicked = false;
    let outer = egui::Frame::new()
        .fill(if glass {
            ui.visuals().faint_bg_color
        } else {
            Color32::TRANSPARENT
        })
        .stroke(if glass {
            Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color)
        } else {
            Stroke::NONE
        })
        .corner_radius(if glass { 12.0 } else { 0.0 })
        .inner_margin(if glass { 6 } else { 0 })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
            // The icon doubles as the grip: a separate handle column costs width the label needs,
            // and dragging from the label itself would fight the click that toggles the layer.
            if draggable {
                ui.dnd_drag_source(
                    egui::Id::new(("layer_drag", &e.label)),
                    e.label.clone(),
                    |ui| ui.label(icon),
                )
                .response
                .on_hover_cursor(egui::CursorIcon::Grab)
                .on_hover_text("Drag to reorder");
            } else {
                ui.label(icon);
            }
            let w = ui.available_width();
            // A justified child layout, not a plain `add`: inside a horizontal row egui centers a
            // button's text, and a column of centered labels is unreadable.
            let mut resp = ui
                .allocate_ui_with_layout(
                    vec2(w, ROW_H),
                    egui::Layout::top_down_justified(egui::Align::LEFT),
                    |ui| {
                        ui.add(
                            egui::Button::new(RichText::new(&e.label).size(13.0).color(fg))
                                .min_size(vec2(w, ROW_H))
                                .fill(if glass && !on { Color32::TRANSPARENT } else { bg })
                                .corner_radius(if glass { 9.0 } else { 7.0 })
                                .stroke(if on {
                                    Stroke::new(1.0, accent.gamma_multiply(0.7))
                                } else {
                                    Stroke::NONE
                                }),
                        )
                    },
                )
                .inner;
            if let Some(key) = &e.key {
                resp = resp.on_hover_text(format!("{}\nShortcut: {key}", e.desc));
            } else if !e.desc.is_empty() {
                resp = resp.on_hover_text(e.desc);
            }
            clicked = resp.clicked();
            resp
            })
            .inner
        })
        .inner;
    // The button's rect, not the whole strip: it's what the chips are drawn against and what a
    // drop is tested on, and it covers everything but the grip.
    let resp = outer;
    let mut favorite = None;
    if let PaletteAction::ToggleField(layer) = e.action {
        if layer.descriptor().is_some() {
            resp.context_menu(|ui| {
                let label = if e.favorite {
                    "Remove from favorites"
                } else {
                    "Add to favorites"
                };
                if ui.button(label).clicked() {
                    favorite = Some(layer);
                    ui.close();
                }
            });
            if e.favorite {
                ui.painter().text(
                    resp.rect.right_center() + vec2(-46.0, 0.0),
                    egui::Align2::CENTER_CENTER,
                    egui_phosphor::regular::STAR,
                    egui::FontId::proportional(12.0),
                    accent,
                );
            }
        }
    }
    // ⓘ for a row whose label names a term the glossary defines, drawn over the button the same
    // way the state dot is. Clicking it explains instead of toggling: the
    // person who doesn't know what MESH is is not the person who wants it turned on yet.
    let mut explain = None;
    if let Some(term) = crate::ui::glossary::explains(&e.label) {
        let has_state = e.on.is_some() || e.health.is_some();
        let at = resp.rect.right_center() + vec2(if has_state { -30.0 } else { -12.0 }, 0.0);
        ui.painter().text(
            at,
            egui::Align2::CENTER_CENTER,
            egui_phosphor::regular::INFO,
            egui::FontId::proportional(13.0),
            Color32::from_gray(150),
        );
        let hit = egui::Rect::from_center_size(at, vec2(18.0, ROW_H));
        if clicked
            && ui
                .ctx()
                .input(|i| i.pointer.interact_pos())
                .is_some_and(|p| hit.contains(p))
        {
            clicked = false;
            explain = Some(term);
        }
    }
    // Network health replaces the ordinary on-dot. Age and errors live in the click popup instead
    // of making every row carry a miniature status report.
    if let Some(health) = &e.health {
        let state = health.state();
        let (_, color) = health_look(state);
        let dot = resp.rect.right_center() + vec2(-10.0, 0.0);
        ui.painter().circle_filled(dot, 3.5, color);
        let hit_rect = egui::Rect::from_min_max(
            egui::pos2(resp.rect.right() - 24.0, resp.rect.top()),
            resp.rect.right_bottom(),
        );
        let health_resp = ui
            .interact(hit_rect, resp.id.with("health"), egui::Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text("Source freshness — click for details");
        if health_resp.clicked() {
            clicked = false;
        }
        egui::Popup::menu(&health_resp).show(|ui| health_popup(ui, health));
    } else if on {
        // Only enabled rows need a state dot; gray dots on every disabled row were visual noise.
        ui.painter()
            .circle_filled(resp.rect.right_center() + vec2(-10.0, 0.0), 3.5, accent);
    }
    Hit {
        clicked,
        favorite,
        explain,
        resp,
    }
}

/// Registry selection also describes tools and layouts; those are not visible map layers.
fn active_layer(e: &PaletteEntry) -> bool {
    use crate::app::{ContourKind, OverlayToggle as T};
    e.on == Some(true)
        && match e.action {
            PaletteAction::SetMoment(..) | PaletteAction::ToggleField(_) => true,
            PaletteAction::SetContours(kind) => kind != ContourKind::Off,
            PaletteAction::ToggleOverlay(toggle) => {
                !matches!(toggle, T::AlertPanel | T::LinkCameras | T::MiniLoop)
            }
            _ => false,
        }
}

fn active_row(ui: &mut egui::Ui, e: &PaletteEntry, accent: Color32) -> Option<PaletteAction> {
    use egui_phosphor::regular as ph;
    let mut chosen = None;
    ui.horizontal(|ui| {
        ui.set_min_height(38.0);
        let label = e.label.split(" (").next().unwrap_or(&e.label);
        let warning = e
            .health
            .as_ref()
            .filter(|h| h.state() != HealthState::Fresh);
        let controls = if warning.is_some() { 102.0 } else { 52.0 };
        ui.allocate_ui_with_layout(
            vec2((ui.available_width() - controls).max(80.0), 38.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.add(egui::Label::new(RichText::new(label).size(14.0)).wrap())
                    .on_hover_text(format!("{}\n{}", e.label, e.desc));
            },
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if matches!(e.action, PaletteAction::SetMoment(..)) {
                ui.weak("Current");
            } else if ui
                .add(
                    egui::Button::new(RichText::new(ph::TOGGLE_RIGHT).size(28.0).color(accent))
                        .frame(false)
                        .min_size(vec2(36.0, 32.0)),
                )
                .named_toggle(&format!("Show {}", e.label), true)
                .clicked()
            {
                chosen = Some(match e.action {
                    PaletteAction::SetContours(_) => {
                        PaletteAction::SetContours(crate::app::ContourKind::Off)
                    }
                    action => action,
                });
            }
            if let Some(health) = warning {
                let (label, color) = health_look(health.state());
                let status = ui
                    .small_button(RichText::new(label).size(10.0).color(color))
                    .on_hover_text("Source status — click for details");
                egui::Popup::menu(&status).show(|ui| health_popup(ui, health));
            }
        });
    });
    chosen
}

/// Quiet, keyboard-accessible navigation for optional categories.
fn category_tile(ui: &mut egui::Ui, cat: &str, width: f32) -> egui::Response {
    ui.add_sized(
        vec2(width, 44.0),
        egui::Button::new(format!("{}  {}", category_glyph(cat), category_name(cat)))
            .frame(false),
    ).named(category_name(cat))
}

/// The frequently used controls stay above alerts and the optional browser on every platform.
pub(crate) fn primary_controls(
    ui: &mut egui::Ui,
    entries: &[PaletteEntry],
    outlook_day: u8,
    outlook_kind: wxdata::spc::OutlookKind,
) -> Option<PaletteAction> {
    use crate::app::{AppWindow, OverlayToggle};
    let mut chosen = None;
    let open_id = ui.make_persistent_id("primary_spc_open");
    let mut spc_open = ui.ctx().data_mut(|d| d.get_temp::<bool>(open_id).unwrap_or(true));
    ui.columns(4, |columns| {
        for (column, (label, action)) in columns.iter_mut().zip([
            ("Storm\ntracks", PaletteAction::ToggleOverlay(OverlayToggle::Tracks)),
            ("MRMS", PaletteAction::ToggleField(crate::render::FieldLayer::Mrms)),
            ("Storm\nattributes", PaletteAction::OpenWindow(AppWindow::StormTable)),
            ("SPC\nOutlook", PaletteAction::OpenOutlooks),
        ]) {
            let on = if action == PaletteAction::OpenOutlooks { spc_open } else {
                entries.iter().any(|e| e.action == action && e.on == Some(true))
            };
            if column.add_sized([column.available_width(), 64.0], egui::Button::new(RichText::new(label).size(12.0)).selected(on)).clicked() {
                if action == PaletteAction::OpenOutlooks { spc_open = !spc_open; }
                else { chosen = Some(action); }
            }
        }
    });
    ui.ctx().data_mut(|d| d.insert_temp(open_id, spc_open));
    if spc_open {
        ui.add_space(12.0);
        ui.label(RichText::new("SPC Convective Outlook").strong());
        ui.label(RichText::new("Forecast day").size(12.0).strong());
        ui.horizontal_wrapped(|ui| {
            for day in 1u8..=8 {
                if ui.selectable_label(outlook_day == day, format!("Day {day}")).clicked() {
                    chosen = Some(PaletteAction::SetOutlookDay(if outlook_day == day { 0 } else { day }));
                }
            }
        });
        ui.add_space(6.0);
        ui.label(RichText::new("Layer").size(12.0).strong());
        ui.horizontal_wrapped(|ui| {
            for (index, kind) in wxdata::spc::OutlookKind::ALL.into_iter().enumerate() {
                if ui.selectable_label(outlook_kind == kind, kind.label()).clicked() {
                    chosen = Some(PaletteAction::SetOutlookKind(index as u8));
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            for (label, color) in [
                ("TSTM", Color32::from_rgb(85, 170, 85)),
                ("MRGL", Color32::from_rgb(65, 145, 75)),
                ("SLGT", Color32::from_rgb(235, 210, 45)),
                ("ENH", Color32::from_rgb(235, 145, 45)),
                ("MDT", Color32::from_rgb(220, 60, 55)),
                ("HIGH", Color32::from_rgb(220, 70, 190)),
            ] {
                ui.colored_label(color, RichText::new(format!("● {label}")).small());
            }
        });
    }
    chosen
}

pub(crate) fn workspace_shortcuts(ui: &mut egui::Ui, entries: &[PaletteEntry]) -> Option<PaletteAction> {
    let mut chosen = None;
    for entry in entries.iter().filter(|e| matches!(e.action, PaletteAction::ApplyWorkspace(_))) {
        if ui.add_sized([ui.available_width(), 44.0], egui::Button::new(format!("{}  {}", egui_phosphor::regular::MAP_TRIFOLD, entry.label))).clicked() {
            chosen = Some(entry.action);
        }
    }
    chosen
}

/// Observation actions are grouped without duplicating the registry or losing specialist entries.
fn observation_group(action: PaletteAction) -> u8 {
    use crate::app::OverlayToggle as T;
    match action {
        PaletteAction::ToggleOverlay(
            T::Metar | T::Webcams | T::Spotters | T::Gauges | T::Aqi | T::Tropical,
        ) => 0,
        PaletteAction::ToggleOverlay(T::Pireps | T::Recon | T::Aviation) => 1,
        _ => 2,
    }
}
fn observation_row(ui: &mut egui::Ui, e: &PaletteEntry, accent: Color32) -> bool {
    let title = e.label.split(" (").next().unwrap_or(&e.label);
    let on = e.on == Some(true);
    let r = ui
        .add_sized(
            [ui.available_width(), 66.0],
            egui::Button::new("").corner_radius(10.0),
        )
        .named_toggle(&e.label, on)
        .on_hover_text(e.desc);
    let p = ui.painter();
    let rect = r.rect;
    p.text(
        rect.left_top() + vec2(12.0, 10.0),
        egui::Align2::LEFT_TOP,
        title,
        egui::FontId::proportional(15.0),
        ui.visuals().text_color(),
    );
    let desc = e.desc.split('—').next().unwrap_or(e.desc).trim();
    let galley = p.layout(
        desc.to_string(),
        egui::FontId::proportional(10.0),
        ui.visuals().weak_text_color(),
        (rect.width() - 68.0).max(100.0),
    );
    p.galley(
        rect.left_top() + vec2(12.0, 34.0),
        galley,
        ui.visuals().weak_text_color(),
    );
    p.text(
        rect.right_center() - vec2(12.0, 0.0),
        egui::Align2::RIGHT_CENTER,
        if on {
            egui_phosphor::regular::TOGGLE_RIGHT
        } else {
            egui_phosphor::regular::TOGGLE_LEFT
        },
        egui::FontId::proportional(27.0),
        if on {
            accent
        } else {
            ui.visuals().weak_text_color()
        },
    );
    r.clicked()
}

/// The panel body: search box + categorized rows. Returns the clicked action, if any.
/// `focus_search` grabs the search field this frame (Ctrl+K opens the drawer typing-ready).
/// `pref` is the persisted drag order and is rewritten in place when a row is dropped.
#[allow(clippy::too_many_arguments)] // two call sites, both flat; a params struct buys nothing
pub(crate) fn body(
    ui: &mut egui::Ui,
    entries: &[PaletteEntry],
    query: &mut String,
    accent: Color32,
    _max_height: f32,
    focus_search: bool,
    pref: &mut Vec<String>,
    _outlook_day: u8,
    _outlook_kind: wxdata::spc::OutlookKind,
    mut after_radar: impl FnMut(&mut egui::Ui),
) -> Option<PaletteAction> {
    let mut chosen = None;
    let nav_id = ui.make_persistent_id("layer_navigation");
    let (mut active_only, mut category) = ui.ctx().data_mut(|d| {
        d.get_temp::<(bool, Option<String>)>(nav_id)
            .unwrap_or_default()
    });
    // (dragged label, label it was dropped on) — applied after the loop so the borrow of `pref`
    // doesn't have to live inside the scroll area.
    let mut moved: Option<(String, String)> = None;
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(egui_phosphor::regular::MAGNIFYING_GLASS)
                .size(15.0)
                .color(Color32::from_gray(170)),
        );
        let field = ui.add(
            egui::TextEdit::singleline(query)
                .hint_text(if active_only {
                    "Filter active layers…"
                } else {
                    "Find a setting, tool, or place…"
                })
                .margin(egui::vec2(8.0, 8.0))
                .desired_width(ui.available_width() - 4.0),
        );
        if focus_search {
            field.request_focus();
        }
        // Only reveal on an explicit search request or when the keyboard shrinks the
        // viewport. Revealing every clipped frame traps scrolling near the search box.
        let height = ui.ctx().content_rect().height();
        let previous_height = ui.ctx().data_mut(|data| {
            let id = field.id.with("viewport_height");
            let previous = data.get_temp::<f32>(id).unwrap_or(height);
            data.insert_temp(id, height);
            previous
        });
        if focus_search || (field.has_focus() && height < previous_height
            && !ui.clip_rect().contains_rect(field.rect)) {
            field.scroll_to_me(Some(egui::Align::Center));
        }
    });
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        let count = entries.iter().filter(|e| active_layer(e)).count();
        for (label, value) in [("Browse".to_string(), false), (format!("{count} active"), true)] {
            if ui.selectable_label(active_only == value, label).clicked() {
                active_only = value;
                category = None;
            }
        }
    });
    ui.add_space(10.0);
    let order: Vec<_> = matches(entries, query)
        .into_iter()
        .filter(|i| !active_only || active_layer(&entries[*i]))
        .collect();
    // Enter runs the top-ranked match. Type-and-Enter was the whole point of the command palette
    // this drawer replaced; without it the search box is a filter, not a launcher.
    if !query.is_empty() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        if let Some(i) = order.first() {
            return Some(entries[*i].action);
        }
    }
    ui.scope(|ui| {
            if order.is_empty() {
                ui.add_space(8.0);
                ui.weak(if active_only && query.is_empty() {
                    "No active layers."
                } else {
                    "No matches."
                });
                return;
            }
            if active_only {
                for cat in CATEGORIES {
                    let group: Vec<_> = order
                        .iter()
                        .copied()
                        .filter(|i| entries[*i].category == cat)
                        .collect();
                    if group.is_empty() {
                        continue;
                    }
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(category_name(cat))
                            .size(11.0)
                            .color(ui.visuals().weak_text_color()),
                    );
                    for i in group {
                        if let Some(action) = active_row(ui, &entries[i], accent) {
                            chosen = Some(action);
                        }
                    }
                    ui.separator();
                }
                after_radar(ui);
                return;
            }
            if !query.is_empty() {
                // Searching: one flat best-first list — categories only add noise here.
                for i in &order {
                    // No dragging in search results: the order you're looking at is the ranking,
                    // not the list you'd be reordering.
                    let hit = row(ui, &entries[*i], accent, false);
                    if hit.clicked {
                        chosen = Some(entries[*i].action);
                    }
                    if let Some(layer) = hit.favorite {
                        chosen = Some(PaletteAction::ToggleFavorite(layer));
                    }
                    if let Some(t) = hit.explain {
                        chosen = Some(PaletteAction::Explain(t));
                    }
                    ui.add_space(2.0);
                }
                return;
            }
            if category.is_none() {
                let width = (ui.available_width() - 8.0) * 0.5;
                let categories: Vec<_> = CATEGORIES
                    .into_iter()
                    .filter(|cat| !["Radar", "MRMS", "Tools", "Settings"].contains(cat))
                    .filter(|cat| entries.iter().any(|e| e.category == *cat))
                    .collect();
                for pair in categories.chunks(2) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        for cat in pair {
                            if category_tile(ui, cat, width).clicked() {
                                category = Some((*cat).to_string());
                            }
                        }
                    });
                    ui.add_space(4.0);
                }
                ui.separator();
                ui.horizontal(|ui| {
                    for cat in ["Tools", "Settings"] {
                        if category_tile(ui, cat, width).clicked() {
                            if cat == "Settings" {
                                chosen = Some(PaletteAction::OpenWindow(crate::app::AppWindow::Settings));
                            } else {
                                category = Some(cat.to_string());
                            }
                        }
                    }
                });
                return;
            }
            let selected = category.clone().unwrap();
            if ui
                .button(format!("‹  {}", category_name(&selected)))
                .clicked()
            {
                category = None;
            }
            ui.add_space(6.0);
            if selected == "Obs" {
                let id = ui.id().with("observation_group");
                let mut group = ui.ctx().data_mut(|d| d.get_temp::<u8>(id).unwrap_or(0));
                if group > 0 && ui.button("‹ Everyday observations").clicked() {
                    group = 0;
                }
                ui.weak(match group {
                    1 => "Aviation & flight data",
                    2 => "Advanced observations",
                    _ => "Everyday observations",
                });
                for i in order.iter().filter(|i| {
                    entries[**i].category == "Obs"
                        && observation_group(entries[**i].action) == group
                }) {
                    if observation_row(ui, &entries[*i], accent) {
                        chosen = Some(entries[*i].action);
                    }
                    ui.add_space(4.0);
                }
                if group == 0 {
                    for (label, g) in [
                        ("Aviation & flight data  ›", 1),
                        ("Advanced observations  ›", 2),
                    ] {
                        if ui
                            .add_sized([ui.available_width(), 48.0], egui::Button::new(label))
                            .clicked()
                        {
                            group = g;
                        }
                    }
                }
                ui.ctx().data_mut(|d| d.insert_temp(id, group));
                return;
            }
            for cat in CATEGORIES.into_iter().filter(|cat| *cat == selected || (selected == "National" && *cat == "MRMS")) {
                let mut in_cat: Vec<usize> = order
                    .iter()
                    .copied()
                    .filter(|i| entries[*i].category == cat)
                    .collect();
                if in_cat.is_empty() {
                    continue;
                }
                // Dragged rows first, in the order they were dropped; then everyday entries;
                // then registry order. A row that was never dragged still has a stable place.
                in_cat.sort_by_key(|i| {
                    let dragged = pref.iter().position(|s| *s == entries[*i].label);
                    (
                        dragged.unwrap_or(usize::MAX),
                        !entries[*i].favorite,
                        entries[*i].recent.unwrap_or(usize::MAX),
                        !entries[*i].common,
                    )
                });
                let seq: Vec<String> = in_cat.iter().map(|i| entries[*i].label.clone()).collect();
                for i in in_cat {
                    let Hit {
                        clicked,
                        favorite,
                        explain,
                        resp,
                    } = row(ui, &entries[i], accent, true);
                    if clicked {
                        chosen = Some(entries[i].action);
                    }
                    if let Some(t) = explain {
                        chosen = Some(PaletteAction::Explain(t));
                    }
                    if let Some(layer) = favorite {
                        chosen = Some(PaletteAction::ToggleFavorite(layer));
                    }
                    // Insertion line above the row the pointer is over, so a drop lands
                    // where the preview says it will.
                    if resp.dnd_hover_payload::<String>().is_some() {
                        let r = resp.rect;
                        ui.painter()
                            .hline(r.x_range(), r.top() - 1.0, Stroke::new(2.0, accent));
                    }
                    if let Some(drag) = resp.dnd_release_payload::<String>() {
                        moved = Some(((*drag).clone(), entries[i].label.clone()));
                    }
                    ui.add_space(2.0);
                }
                // The knobs for the products right above them, not at the bottom of the panel:
                // a threshold or a forecast hour is read together with the layer it belongs to.
                if cat == "Radar" {
                    ui.add_space(2.0);
                    after_radar(ui);
                    ui.add_space(2.0);
                }
                if let Some((drag, before)) = moved.take() {
                    // Only the category the row was dropped in is rewritten — a cross-category
                    // drag would move a layer out of the group its label says it's in.
                    if seq.contains(&drag) {
                        reorder(pref, &seq, &drag, &before);
                    }
                }
            }
        });
    ui.ctx()
        .data_mut(|d| d.insert_temp(nav_id, (active_only, category)));
    chosen
}

/// Fade the last few pixels of the scroll viewport into the card colour when there's more below.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_footer_opens_settings_with_one_click() {
        let ctx = egui::Context::default();
        let mut query = String::new();
        let mut pref = Vec::new();
        let mut pos = egui::Pos2::ZERO;
        let entries = [PaletteEntry {
            label: "Settings…".into(), category: "Settings",
            action: PaletteAction::OpenWindow(crate::app::AppWindow::Settings),
            on: None, desc: "", common: true, key: None, health: None,
            favorite: false, recent: None,
        }];
        let mut action = None;
        for frame in 0..5 {
            let mut input = egui::RawInput::default();
            if frame >= 3 {
                input.events = vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton {
                    pos, button: egui::PointerButton::Primary, pressed: frame == 3,
                    modifiers: egui::Modifiers::default(),
                }];
            }
            let out = ctx.run_ui(input, |ui| {
                ui.set_width(340.0);
                action = body(ui, &entries, &mut query, Color32::WHITE, 700.0, false,
                    &mut pref, 0, wxdata::spc::OutlookKind::default(), |_| {});
            });
            if frame == 2 {
                pos = out.shapes.iter().find_map(|shape| match &shape.shape {
                    egui::Shape::Text(t) if t.galley.job.text.ends_with("Settings") =>
                        Some(t.pos + t.galley.rect.size() * 0.5),
                    _ => None,
                }).expect("Settings footer is visible");
            }
        }
        assert!(matches!(action, Some(PaletteAction::OpenWindow(crate::app::AppWindow::Settings))));
    }

    #[test]
    fn active_list_excludes_commands_and_off_contours() {
        use crate::app::{ContourKind, OverlayToggle as T};
        let mut entry = PaletteEntry {
            label: "test".into(),
            category: "Reference",
            action: PaletteAction::ToggleOverlay(T::RadarSites),
            on: Some(true),
            desc: "",
            common: true,
            key: None,
            health: None,
            favorite: false,
            recent: None,
        };
        assert!(active_layer(&entry));
        for action in [
            PaletteAction::SetContours(ContourKind::Off),
            PaletteAction::TogglePanel,
            PaletteAction::SetPanes(1),
            PaletteAction::ToggleOverlay(T::AlertPanel),
            PaletteAction::ToggleOverlay(T::LinkCameras),
            PaletteAction::ToggleOverlay(T::MiniLoop),
        ] {
            entry.action = action;
            assert!(!active_layer(&entry), "{action:?}");
        }
        entry.action = PaletteAction::ToggleOverlay(T::Tracks);
        entry.on = Some(false);
        assert!(!active_layer(&entry));
        entry.on = Some(true);
        assert!(active_layer(&entry));
    }

    #[test]
    fn floating_navigation_keeps_categories_active_layers_and_search_reachable() {
        let entries: Vec<_> = CATEGORIES
            .iter()
            .map(|cat| PaletteEntry {
                label: format!("{cat} layer"),
                category: cat,
                action: PaletteAction::ToggleOverlay(if *cat == "Obs" {
                    crate::app::OverlayToggle::Metar
                } else {
                    crate::app::OverlayToggle::RadarSites
                }),
                on: Some(*cat == "Radar"),
                desc: "",
                common: true,
                key: None,
                health: None,
                favorite: false,
                recent: None,
            })
            .collect();
        let render = |active: bool, category: Option<&str>, search: &str| {
            let ctx = egui::Context::default();
            let mut query = search.to_string();
            let mut pref = Vec::new();
            let mut text = Vec::new();
            for _ in 0..3 {
                let out = ctx.run_ui(egui::RawInput::default(), |ui| {
                    ui.set_width(308.0);
                    let id = ui.make_persistent_id("layer_navigation");
                    ui.ctx()
                        .data_mut(|d| d.insert_temp(id, (active, category.map(str::to_string))));
                    body(
                        ui,
                        &entries,
                        &mut query,
                        Color32::WHITE,
                        700.0,
                        false,
                        &mut pref,
                        0,
                        wxdata::spc::OutlookKind::default(),
                        |_| {},
                    );
                });
                text = out
                    .shapes
                    .iter()
                    .filter_map(|s| match &s.shape {
                        egui::Shape::Text(t) => Some(t.galley.job.text.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
            }
            text
        };
        let browse = render(false, None, "");
        for cat in CATEGORIES {
            if !["Radar", "MRMS"].contains(&cat) { assert!(
                browse.iter().any(|s| s.ends_with(category_name(cat))),
                "missing {cat}: {browse:?}"
            ); }
            assert!(render(false, Some(cat), "")
                .iter()
                .any(|s| s == &format!("{cat} layer")));
        }
        let active = render(true, None, "");
        assert!(active.iter().any(|s| s == "Radar layer"));
        assert!(!active.iter().any(|s| s == "National layer"));
        assert!(render(false, Some("Radar"), "National")
            .iter()
            .any(|s| s == "National layer"));
    }

    #[test]
    fn focused_search_does_not_pull_the_menu_back_while_scrolling() {
        let ctx = egui::Context::default();
        let mut query = String::new();
        let mut pref = Vec::new();
        let mut offset = 0.0;
        let mut focused_offset = 0.0;
        for frame in 0..30 {
            let mut events = vec![egui::Event::PointerMoved(egui::pos2(100.0, 100.0))];
            if frame >= 10 {
                events.push(egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point,
                    phase: egui::TouchPhase::Move, delta: egui::vec2(0.0, 80.0),
                    modifiers: egui::Modifiers::default() });
            }
            let _ = ctx.run_ui(egui::RawInput { events, time: Some(frame as f64 / 10.0),
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(360.0, 400.0))),
                ..Default::default() }, |ui| {
                let out = egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_space(600.0);
                    body(ui, &[], &mut query, Color32::WHITE, 100.0, frame == 0,
                        &mut pref, 0, wxdata::spc::OutlookKind::default(), |_| {});
                    ui.add_space(200.0);
                });
                offset = out.state.offset.y;
            });
            if frame == 9 { focused_offset = offset; }
        }
        assert!(focused_offset > 300.0, "search must initially be revealed");
        assert!(offset < 10.0, "user must be able to scroll back to radar products: {offset}");
    }

    #[test]
    fn focused_search_scrolls_above_keyboard() {
        let ctx = egui::Context::default();
        let mut query = String::new();
        let mut pref = Vec::new();
        let mut offset = 0.0;
        for frame in 0..12 {
            let height = if frame < 3 { 800.0 } else { 300.0 };
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(360.0, height),
                )),
                time: Some(frame as f64 / 10.0),
                ..Default::default()
            };
            let _ = ctx.run_ui(input, |ui| {
                let out = egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_space(400.0); // product controls above the search field
                    body(
                        ui,
                        &[],
                        &mut query,
                        Color32::WHITE,
                        100.0,
                        frame == 0,
                        &mut pref,
                        0,
                        wxdata::spc::OutlookKind::default(),
                        |_| {},
                    );
                    ui.add_space(200.0);
                });
                offset = out.state.offset.y;
            });
        }
        assert!(
            offset > 150.0,
            "focused search stayed behind the keyboard: {offset}"
        );
    }

    #[test]
    fn browse_shows_spc_day_and_layer_controls() {
        let ctx = egui::Context::default();
        let entries = [PaletteEntry {
            label: "SPC convective outlook".into(),
            category: "Severe",
            action: PaletteAction::OpenOutlooks,
            on: Some(true),
            desc: "",
            common: true,
            key: None,
            health: None,
            favorite: false,
            recent: None,
        }];
        let out = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(400.0);
            primary_controls(ui, &entries, 1, wxdata::spc::OutlookKind::Categorical);
        });
        let text: Vec<_> = out
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.job.text.as_str()),
                _ => None,
            })
            .collect();
        for expected in ["Forecast day", "Day 8", "Layer", "Hail", "● HIGH"] {
            assert!(text.contains(&expected), "missing {expected}: {text:?}");
        }
    }

    #[test]
    fn outlook_days_toggle_off_and_switch_directly() {
        let ctx = egui::Context::default();
        let mut active = 0;
        let mut frame = |events| {
            ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
                ui.set_width(340.0);
                if let Some(PaletteAction::SetOutlookDay(day)) =
                    primary_controls(ui, &[], active, wxdata::spc::OutlookKind::Categorical) {
                    active = day;
                }
                ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("test_outlook_day"), active));
            })
        };
        for day in 1u8..=8 {
            for expected in [day, 0, day] {
                let output = frame(Vec::new());
                assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
                    egui::Shape::Text(t) if t.galley.job.text == "Show outlook on map")));
                let point = output.shapes.iter().find_map(|shape| match &shape.shape {
                    egui::Shape::Text(t) if t.galley.job.text == format!("Day {day}") =>
                        Some(t.pos + t.galley.rect.size() * 0.5),
                    _ => None,
                }).expect("forecast day is visible");
                for pressed in [true, false] {
                    frame(vec![egui::Event::PointerMoved(point), egui::Event::PointerButton {
                        pos: point, button: egui::PointerButton::Primary, pressed,
                        modifiers: egui::Modifiers::NONE,
                    }]);
                }
                assert_eq!(ctx.data(|d| d.get_temp::<u8>(egui::Id::new("test_outlook_day"))), Some(expected));
            }
        }
    }

    #[test]
    fn primary_controls_dispatch_actions_and_keep_workspaces_reachable() {
        let ctx = egui::Context::default();
        let entries = [PaletteEntry {
            label: "Workspace: Chase".into(), category: "Reference",
            action: PaletteAction::ApplyWorkspace(0), on: None, desc: "",
            common: true, key: None, health: None,
            favorite: false, recent: None,
        }];
        let mut action = None;
        let mut frame = |events| {
            ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
                ui.set_width(340.0);
                action = primary_controls(ui, &entries, 1, wxdata::spc::OutlookKind::Categorical);
                if let Some(workspace) = workspace_shortcuts(ui, &entries) { action = Some(workspace); }
                ui.ctx().data_mut(|d| {
                    d.remove::<PaletteAction>(egui::Id::new("test_action"));
                    if let Some(action) = action { d.insert_temp(egui::Id::new("test_action"), action); }
                });
            })
        };
        for (label, expected) in [
            ("MRMS", PaletteAction::ToggleField(crate::render::FieldLayer::Mrms)),
            ("Storm\ntracks", PaletteAction::ToggleOverlay(crate::app::OverlayToggle::Tracks)),
            ("Storm\nattributes", PaletteAction::OpenWindow(crate::app::AppWindow::StormTable)),
            ("Day 8", PaletteAction::SetOutlookDay(8)),
            ("Hail", PaletteAction::SetOutlookKind(3)),
            ("Workspace: Chase", PaletteAction::ApplyWorkspace(0)),
        ] {
            let output = frame(Vec::new());
            let point = output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::Shape::Text(t) if t.galley.job.text.ends_with(label) =>
                    Some(t.pos + t.galley.rect.size() * 0.5),
                _ => None,
            }).unwrap_or_else(|| panic!("missing {label}"));
            frame(vec![egui::Event::PointerMoved(point), egui::Event::PointerButton {
                pos: point, button: egui::PointerButton::Primary, pressed: true,
                modifiers: egui::Modifiers::NONE,
            }]);
            frame(vec![egui::Event::PointerButton {
                pos: point, button: egui::PointerButton::Primary, pressed: false,
                modifiers: egui::Modifiers::NONE,
            }]);
            // Inspect outside the renderer's mutable capture.
            assert_eq!(ctx.data(|d| d.get_temp::<PaletteAction>(egui::Id::new("test_action"))), Some(expected));
        }
    }

    /// Grouping must never hide a row for good: every specialist entry remains searchable.
    #[test]
    fn every_entry_is_reachable_from_the_registry() {
        let entries = [
            PaletteEntry {
                label: "Echo tops (L3)".into(),
                category: "National",
                action: PaletteAction::CycleBasemap,
                on: None,
                desc: "How tall the storm is",
                common: false,
                key: None,
                health: None,
                favorite: false,
                recent: None,
            },
            PaletteEntry {
                label: "MRMS Mosaic".into(),
                category: "National",
                action: PaletteAction::CycleBasemap,
                on: None,
                desc: "Every radar stitched together",
                common: true,
                key: None,
                health: None,
                favorite: false,
                recent: None,
            },
        ];
        // Empty query = the full list, common or not.
        assert_eq!(matches(&entries, "").len(), entries.len());
        // And an uncommon row is still findable by name.
        assert_eq!(matches(&entries, "echo"), vec![0]);
    }

    #[test]
    fn migrated_fields_are_searchable_by_descriptor_metadata() {
        let entries = [
            PaletteEntry {
                label: "National mosaic".into(),
                category: "National",
                action: PaletteAction::ToggleField(crate::render::FieldLayer::Mrms),
                on: None,
                desc: "",
                common: true,
                key: None,
                health: None,
                favorite: false,
                recent: None,
            },
            PaletteEntry {
                label: "Ground strikes".into(),
                category: "National",
                action: PaletteAction::ToggleField(crate::render::FieldLayer::Lightning),
                on: None,
                desc: "",
                common: true,
                key: None,
                health: None,
                favorite: false,
                recent: None,
            },
        ];
        assert_eq!(matches(&entries, "dbz"), vec![0]);
        assert_eq!(matches(&entries, "nldn"), vec![1]);
        assert_eq!(matches(&entries, "strikes/km"), vec![1]);
    }

    #[test]
    fn favorites_and_recents_break_equal_search_ties() {
        let entry = |favorite, recent| PaletteEntry {
            label: "Hail".into(),
            category: "National",
            action: PaletteAction::ToggleField(crate::render::FieldLayer::Mesh),
            on: None,
            desc: "",
            common: true,
            key: None,
            health: None,
            favorite,
            recent,
        };
        assert_eq!(matches(&[entry(false, Some(0)), entry(true, None)], "hail"), vec![1, 0]);
        assert_eq!(matches(&[entry(false, Some(2)), entry(false, Some(0))], "hail"), vec![1, 0]);
    }

    /// The drawer's Enter key runs `matches(...)[0]`, so the ranking has to put the obvious
    /// answer first for the labels people actually type.
    #[test]
    fn top_match_is_the_obvious_one() {
        let e = |label: &str| PaletteEntry {
            label: label.into(),
            category: "Radar",
            action: PaletteAction::CycleBasemap,
            on: None,
            desc: "",
            common: true,
            key: None,
            health: None,
            favorite: false,
            recent: None,
        };
        let entries = [
            e("Storm-Relative Velocity"),
            e("Velocity"),
            e("Reflectivity"),
        ];
        assert_eq!(matches(&entries, "velocity").first(), Some(&1));
        assert_eq!(matches(&entries, "refl").first(), Some(&2));
    }

    #[test]
    fn dropping_a_row_puts_it_where_the_preview_said() {
        let seq: Vec<String> = ["Reflectivity", "Velocity", "Spectrum Width"]
            .map(String::from)
            .to_vec();
        let mut pref = vec!["Some other category's row".to_string()];
        // Drop Spectrum Width onto Velocity: it lands *above* Velocity, matching the line drawn
        // along the hovered row's top edge.
        reorder(&mut pref, &seq, "Spectrum Width", "Velocity");
        assert_eq!(
            pref,
            [
                "Some other category's row",
                "Reflectivity",
                "Spectrum Width",
                "Velocity",
            ]
        );
        // A second drag rewrites the same labels rather than appending them twice.
        let seq2: Vec<String> = ["Reflectivity", "Spectrum Width", "Velocity"]
            .map(String::from)
            .to_vec();
        reorder(&mut pref, &seq2, "Velocity", "Reflectivity");
        assert_eq!(
            pref,
            [
                "Some other category's row",
                "Velocity",
                "Reflectivity",
                "Spectrum Width",
            ]
        );
        // Dropping a row on itself is a no-op, not a reshuffle.
        let before = pref.clone();
        reorder(&mut pref, &seq2, "Velocity", "Velocity");
        assert_eq!(pref, before);
    }

    #[test]
    fn fuzzy_subsequence_and_ranking() {
        assert!(fuzzy("vel", "Velocity").is_some());
        assert!(fuzzy("srv", "Storm-Relative Velocity").is_some());
        assert!(fuzzy("gau", "River gauges (NWPS)").is_some());
        assert!(fuzzy("zzz", "Velocity").is_none());
        // Empty query matches everything.
        assert_eq!(fuzzy("", "anything"), Some(0));
        // A tighter (contiguous) match must rank ahead of a scattered one.
        let tight = fuzzy("cape", "CAPE").unwrap();
        let loose = fuzzy("cape", "Cell arrival probability estimate").unwrap();
        assert!(tight < loose, "tight {tight} should beat loose {loose}");
    }
}
