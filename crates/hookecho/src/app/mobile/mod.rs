//! What the phone adds to the shared floating chrome, and nothing else.
//!
//! Android used to have a chrome of its own: a persistent bottom sheet with three snap points, a
//! docked five-slot toolbar, its own menu, its own alerts list, its own capture sheet. All of it
//! duplicated surfaces the shared registry already fed, and all of it is gone — the phone now
//! draws the same search pill, control column, scrubber and panel the desktop does
//! ([`crate::app::chrome::overlay`]), laid out for a thumb and presented in a bottom sheet.
//!
//! Three things stay here, because they exist only on a phone: the color scale painted along the
//! top edge (there is no room for the desktop legend box), the hide-all-chrome eye, and the
//! system-back chain that decides what a back gesture dismisses.

pub(crate) mod sheet;

use egui::{pos2, vec2, Align2, Color32, Id, Mesh, Rect, Shape};
use egui_phosphor::regular as ph;
use wxdata::level2::Moment;

pub(crate) use crate::ui::style::{glass, square_btn, OMEGA_GREEN, OMEGA_ORANGE};

/// A slim full-width strip for a gridded field layer, with its name and range inline.
fn paint_field_strip(painter: &egui::Painter, rect: Rect, layer: crate::render::FieldLayer) {
    use crate::render::field_ramps::{ramp_for, FieldScale};
    let Some(r) = ramp_for(layer) else { return };
    let col = |c: [u8; 3]| Color32::from_rgb(c[0], c[1], c[2]);
    match r.scale {
        FieldScale::Ramp { lo, hi, stops, .. } => {
            let mut mesh = Mesh::default();
            for w in stops.windows(2) {
                let (t0, c0) = w[0];
                let (t1, c1) = w[1];
                let q = Rect::from_min_max(
                    pos2(rect.left() + t0 * rect.width(), rect.top()),
                    pos2(rect.left() + t1 * rect.width(), rect.bottom()),
                );
                let i = mesh.vertices.len() as u32;
                for (p, c) in [
                    (q.left_top(), col(c0)),
                    (q.right_top(), col(c1)),
                    (q.right_bottom(), col(c1)),
                    (q.left_bottom(), col(c0)),
                ] {
                    mesh.colored_vertex(p, c);
                }
                mesh.add_triangle(i, i + 1, i + 2);
                mesh.add_triangle(i, i + 2, i + 3);
            }
            painter.add(Shape::mesh(mesh));
            let label = if r.units.is_empty() {
                format!("{}  {lo:.0}\u{2013}{hi:.0}", r.label)
            } else {
                format!("{}  {lo:.0}\u{2013}{hi:.0} {}", r.label, r.units)
            };
            painter.text(
                pos2(rect.left() + 4.0, rect.bottom() + 1.0),
                Align2::LEFT_TOP,
                label,
                egui::FontId::proportional(9.0),
                Color32::from_gray(215),
            );
        }
        FieldScale::Categorical(cats) => {
            // Equal slices, one per class, with the layer name beneath.
            let w = rect.width() / cats.len() as f32;
            for (i, (_, rgb, _)) in cats.iter().enumerate() {
                let x = rect.left() + i as f32 * w;
                painter.rect_filled(
                    Rect::from_min_size(pos2(x, rect.top()), vec2(w, rect.height())),
                    0.0,
                    col(*rgb),
                );
            }
            painter.text(
                pos2(rect.left() + 4.0, rect.bottom() + 1.0),
                Align2::LEFT_TOP,
                r.label,
                egui::FontId::proportional(9.0),
                Color32::from_gray(215),
            );
        }
    }
}

/// Paint the active product's color table as a full-width gradient strip (the top scale bar).
fn paint_colorbar(
    painter: &egui::Painter,
    rect: Rect,
    moment: Moment,
    table: &crate::colormap::ColorTable,
) {
    let (vmin, vmax) = moment.value_range();
    let span = (vmax - vmin).max(f32::EPSILON);
    let x_of = |v: f32| rect.left() + ((v - vmin) / span).clamp(0.0, 1.0) * rect.width();
    let col = |c: [u8; 4]| Color32::from_rgb(c[0], c[1], c[2]);
    let mut mesh = Mesh::default();
    let mut quad = |x0: f32, x1: f32, c0: Color32, c1: Color32| {
        if x1 <= x0 {
            return;
        }
        let i = mesh.vertices.len() as u32;
        mesh.colored_vertex(pos2(x0, rect.top()), c0);
        mesh.colored_vertex(pos2(x1, rect.top()), c1);
        mesh.colored_vertex(pos2(x1, rect.bottom()), c1);
        mesh.colored_vertex(pos2(x0, rect.bottom()), c0);
        mesh.add_triangle(i, i + 1, i + 2);
        mesh.add_triangle(i, i + 2, i + 3);
    };
    for (i, s) in table.stops.iter().enumerate() {
        let x0 = x_of(s.value);
        match table.stops.get(i + 1) {
            Some(n) => {
                let x1 = x_of(n.value);
                if s.solid {
                    quad(x0, x1, col(s.rgba), col(s.rgba));
                } else {
                    quad(x0, x1, col(s.rgba), col(s.end.unwrap_or(n.rgba)));
                }
            }
            None => quad(
                x0,
                rect.right(),
                col(s.end.unwrap_or(s.rgba)),
                col(s.end.unwrap_or(s.rgba)),
            ),
        }
    }
    painter.add(Shape::mesh(mesh));
}
impl super::HookEchoApp {
    /// Whether back would dismiss something in-app rather than leaving the app.
    fn mobile_has_dismissable(&self) -> bool {
        self.marker_popup.is_some()
            || self.detail.is_some()
            || self.warning_popup.is_some()
            || self.cell_popup.is_some()
            || self.xsection.is_some()
            || self.site_dialog.is_some()
            || self.sounding_window.open
            || self.show_hodo
            || self.show_cappi
            || self.show_3d
            || self.show_sensors
            || self.climo_open
            || self.afd_open
            || self.digest_window.open
            || self.verify_window.open
            || self.event_window.open
            || self.marker_window.open
            || self.placefile_window.open
            || self.palette_editor.open
            || self.layer_window_open
            || self.show_cheatsheet
            || self.about_open
            || self.firstrun.open
            || self.tour.open
            || self.cells_window.open
            || self.forecast_open
            || self.settings_window.open
            || self.basemap_open
            || self.panel_open
            || self.mobile_chrome_hidden
    }

    /// What Android's back gesture dismisses, innermost first. Returns without doing anything when
    /// nothing is open, which lets the OS handle it (leave the app).
    fn mobile_back(&mut self) {
        // Full-screen surfaces first, then the sheets over the map, and only then nothing. Every
        // surface the phone can open is in here; one that isn't is a one-way door, because a
        // full-screen surface hides the map you'd otherwise tap to get out.
        macro_rules! close {
            ($($flag:expr),* $(,)?) => {
                $(if $flag { $flag = false; return; })*
            };
        }
        macro_rules! clear {
            ($($opt:expr),* $(,)?) => {
                $(if $opt.is_some() { $opt = None; return; })*
            };
        }
        clear!(
            self.marker_popup,
            self.detail,
            self.warning_popup,
            self.cell_popup,
            self.xsection,
            self.site_dialog,
        );
        close!(
            self.sounding_window.open,
            self.show_hodo,
            self.show_cappi,
            self.show_3d,
            self.show_sensors,
            self.climo_open,
            self.afd_open,
            self.digest_window.open,
            self.verify_window.open,
            self.event_window.open,
            self.marker_window.open,
            self.placefile_window.open,
            self.palette_editor.open,
            self.layer_window_open,
            self.show_cheatsheet,
            self.about_open,
            self.firstrun.open,
            self.tour.open,
            self.cells_window.open,
            self.forecast_open,
            self.settings_window.open,
            self.basemap_open,
            self.panel_open,
            self.mobile_chrome_hidden,
        );
    }

    /// The phone-only chrome, drawn before the shared floating chrome: the back wiring, the color
    /// scale along the top edge, the hide-everything eye, and the armed-tool hint.
    ///
    /// Returns `false` when the user has hidden the chrome, which is the caller's cue to skip the
    /// shared surfaces entirely — the point of the eye is a map with nothing on it.
    pub(crate) fn mobile_chrome(&mut self, ctx: &egui::Context) -> bool {
        // Back arrives two ways: as a `BrowserBack` key event (the legacy path, which Android 16
        // stops delivering) and from the predictive-back callback in MainActivity. Either one
        // runs the same dismissal chain.
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::BrowserBack))
            || crate::platform::take_back_pressed()
        {
            self.mobile_back();
        }
        // Tell Android whether we would consume the next gesture. With nothing open the callback
        // is disabled and the OS gets to draw its live home-screen preview as the user drags.
        crate::platform::set_back_consumed(self.mobile_has_dismissable());

        let content = ctx.content_rect();
        let inset_top = (content.top() - ctx.viewport_rect().top()).max(0.0);

        // Hide/show all chrome (view the whole radar). Always drawn; when hidden it is the only
        // floating control, so the map is fully visible.
        egui::Area::new(Id::new("m_chrome_toggle"))
            .anchor(
                Align2::RIGHT_TOP,
                vec2(-crate::ui::m3::SP_3, inset_top + 26.0),
            )
            // Plain Middle order, so any surface opened afterwards (a full-screen window, a
            // modal sheet's scrim) covers it instead of leaving an eye floating over the content.
            .show(ctx, |ui| {
                let g = if self.mobile_chrome_hidden {
                    ph::EYE
                } else {
                    ph::EYE_SLASH
                };
                if square_btn(ui, g, self.mobile_chrome_hidden, OMEGA_ORANGE).clicked() {
                    self.mobile_chrome_hidden = !self.mobile_chrome_hidden;
                    self.panel_open = false;
                    self.basemap_open = false;
                }
            });
        if self.mobile_chrome_hidden {
            return false;
        }

        // ---------- FULL-WIDTH COLOR SCALE (top edge, under the status bar) ----------
        let active = self.active;
        if self.views[active].volume.is_some() {
            let moment = self.views[active].moment;
            let table = self.palettes.table(moment);
            let strip = Rect::from_min_size(
                pos2(content.left(), content.top()),
                vec2(content.width(), 9.0),
            );
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Background,
                Id::new("m_colorbar"),
            ));
            paint_colorbar(&painter, strip, moment, table);

            // Second strip for the topmost gridded layer — the phone has no room for the desktop
            // legend box, but an unlabeled MESH/QPE wash is just as cryptic here.
            if let Some(top) = crate::render::FieldLayer::DRAW_ORDER
                .iter()
                .rev()
                .find(|l| self.fields.get(l).is_some_and(|s| s.show))
            {
                paint_field_strip(&painter, strip.translate(vec2(0.0, 11.0)), *top);
            }
        }

        self.mobile_tool_hint(ctx, content);
        true
    }

    /// The armed-tool hint: the desktop says this in its status bar, which the phone hides, so a
    /// two-point tool otherwise reads as "the first tap did nothing".
    fn mobile_tool_hint(&mut self, ctx: &egui::Context, content: Rect) {
        let text = match self.tool {
            crate::app::MapTool::Measure => "Tap two points to measure",
            crate::app::MapTool::Marker => "Tap the map to drop a marker",
            crate::app::MapTool::CrossSection => "Tap two points for a cross-section",
            crate::app::MapTool::Sounding => "Tap a point for a sounding",
            crate::app::MapTool::Climatology => "Tap a point for tornado climatology",
            _ => return,
        };
        let accent = crate::theme::accent(self.settings.theme);
        egui::Area::new(Id::new("m_toolhint"))
            .anchor(Align2::CENTER_TOP, vec2(0.0, 92.0))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(accent)
                    .corner_radius(crate::ui::m3::R_FULL)
                    .inner_margin(egui::Margin::symmetric(
                        crate::ui::m3::SP_4 as i8,
                        crate::ui::m3::SP_2 as i8,
                    ))
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(text)
                                .size(crate::ui::m3::T_LABEL_LG)
                                .strong()
                                .color(Color32::BLACK),
                        );
                    });
            });
        let _ = content;
    }

    /// The basemap picker, as chips.
    ///
    /// The only way to change the map underneath the radar used to be the palette's "Cycle
    /// basemap" row, which advances one step through forty styles and closes the sheet as it goes
    /// — reaching satellite imagery from dark vector was nine round trips through the menu. These
    /// set the style directly and leave the sheet open, so trying two is two taps.
    pub(crate) fn basemap_chips(&mut self, ui: &mut egui::Ui) {
        use crate::tiles::BasemapStyle;
        let (mb, mt) = (
            !self.settings.mapbox_key.is_empty(),
            !self.settings.maptiler_key.is_empty(),
        );
        let cx = crate::tiles::valid_xyz_template(&self.settings.custom_tile_url);
        let cur = self.views[self.active].basemap;
        let mut pick = None;
        ui.label(egui::RichText::new("Basemap").size(crate::ui::m3::T_LABEL_LG));
        // The common styles stay a chip row: one tap, no scrolling, thumb-reachable. It is the
        // *other* forty-odd that needed the grid.
        ui.horizontal_wrapped(|ui| {
            for s in BasemapStyle::COMMON {
                if s.available(mb, mt, cx)
                    && crate::ui::m3::chip(ui, s.short_label(), s == cur).clicked()
                {
                    pick = Some(s);
                }
            }
        });
        egui::CollapsingHeader::new("All basemaps")
            .id_salt("m_basemap_all")
            .default_open(!BasemapStyle::COMMON.contains(&cur))
            .show(ui, |ui| {
                if let Some(s) =
                    crate::ui::basemap_picker::grid(ui, &mut self.tiles, cur, &self.settings)
                {
                    pick = Some(s);
                }
            });
        if let Some(s) = pick {
            self.set_basemap(s);
        }
    }

}
