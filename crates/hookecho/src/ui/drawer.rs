//! One slide-over drawer, one page at a time, with a back-stack.
//!
//! Every browsable tool (settings, events, rules, verify, palettes…) used to be its own free
//! floating window: draggable, overlappable, and on a phone a full-screen surface pretending to
//! be a window. They are all the same thing — a page you open, read, and leave — so they all get
//! the same surface now. The map keeps the rest of the screen.
//!
//! The stack is titles, not pages: each tool still owns its `open` flag and still draws its own
//! body. Opening a second page hides the first without closing it, and leaving the second brings
//! it back. That is the whole router.
//!
//! ponytail: still `egui::Window` under the header, because it already does the fixed rect, the
//! scrolling body and the frame — the drawer only takes over the chrome and the ordering. If
//! pages ever need transitions of their own, that's the seam to cut at.

use crate::ui::a11y::Named as _;
use egui::{pos2, vec2, Rect};

/// The drawer's geometry on a desktop-sized screen. On a phone it takes the whole content rect,
/// which is Material 3's answer for the compact width class and what the old `phone_surface` did.
const X: f32 = 10.0;
const TOP: f32 = 58.0;
const WIDTH: f32 = 380.0;
/// Room along the bottom edge for the floating scrubber.
const BOTTOM_CLEARANCE: f32 = 96.0;
const HEADER_H: f32 = 44.0;

#[derive(Default)]
pub struct Drawer {
    /// Titles of every page currently open, oldest first. The last one is what shows.
    stack: Vec<String>,
    /// Titles that asked to draw this frame; a page that closed itself simply stops asking.
    seen: Vec<String>,
    /// Which frame `seen` belongs to, so the stack can prune itself without the app calling us.
    frame: u64,
    /// App time the drawer last went from empty to showing something, so the slide-in animates
    /// from where the drawer actually came from rather than from wherever egui last latched it.
    opened_at: f64,
    /// Is the top page's quick-settings row expanded? Per-page state would outlive the page it
    /// belongs to; a single flag reset on every page change is the honest scope.
    pub gear: bool,
}

impl Drawer {
    /// Is a page showing? The floating panel steps aside when one is: they share the same lane,
    /// and the drawer is where the panel just sent the user.
    pub fn is_open(&self) -> bool {
        !self.stack.is_empty()
    }

    /// The page on top, if any. What a workspace saves; the pages underneath it are a back-stack,
    /// which is a history, not an arrangement.
    pub fn top(&self) -> Option<&str> {
        self.stack.last().map(String::as_str)
    }

    /// Claim the drawer for `title`, drawing its header. Returns the window to render the page
    /// body in, or `None` when another page is on top — the caller draws nothing that frame but
    /// stays open, and comes back when the page above it closes.
    ///
    /// `open` is the caller's own flag: the header's back arrow and ✕ clear it, which is the same
    /// thing the old title bar's ✕ did.
    pub fn page<'a>(
        &mut self,
        ctx: &egui::Context,
        title: &str,
        open: &mut bool,
        gear: bool,
        w: egui::Window<'a>,
    ) -> Option<egui::Window<'a>> {
        self.page_sized(ctx, title, open, gear, WIDTH, w)
    }

    /// Same, for a page that carries a drawing rather than a list — a hodograph, a cross-section,
    /// a 3D volume. `width` is what the old floating window asked for; the drawer still clamps it
    /// to the screen, and the phone still takes the whole thing.
    pub fn page_sized<'a>(
        &mut self,
        ctx: &egui::Context,
        title: &str,
        open: &mut bool,
        gear: bool,
        width: f32,
        w: egui::Window<'a>,
    ) -> Option<egui::Window<'a>> {
        // A page that isn't open doesn't get a slot: several callers reach `page` before their
        // own `open` check, and a closed page silently holding the top of the stack would hide
        // the one the user actually asked for.
        if !*open {
            return None;
        }
        let frame = ctx.cumulative_pass_nr();
        if frame != self.frame {
            // A page that stopped drawing has closed itself (its own ✕, a hotkey, an action).
            self.stack.retain(|t| self.seen.contains(t));
            self.seen.clear();
            self.frame = frame;
        }
        self.seen.push(title.to_string());
        if self.stack.is_empty() {
            self.opened_at = ctx.input(|i| i.time);
        }
        if !self.stack.iter().any(|t| t == title) {
            self.stack.push(title.to_string());
            self.gear = false;
        }
        if self.stack.last().map(String::as_str) != Some(title) {
            return None;
        }

        let (head, body) = rects(ctx, width);
        let (head, body) = self.slide(ctx, head, body);
        let depth = self.stack.len();
        let mut gear_on = self.gear;
        let mut close = false;
        egui::Area::new(egui::Id::new("drawer_header"))
            .fixed_pos(head.min)
            .show(ctx, |ui| {
                crate::ui::style::glass(ui, 250).show(ui, |ui| {
                    ui.set_width(head.width() - 24.0);
                    ui.horizontal(|ui| {
                        // One button, two meanings: with a page underneath it goes back, without
                        // one it closes the drawer. Both leave this page, so both clear `open`.
                        let (glyph, hint) = if depth > 1 {
                            (egui_phosphor::regular::CARET_LEFT, "Back")
                        } else {
                            (egui_phosphor::regular::X, "Close")
                        };
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new(glyph).size(crate::ui::style::FONT_LG),
                                )
                                .fill(egui::Color32::TRANSPARENT)
                                .stroke(egui::Stroke::NONE),
                            )
                            .named(hint)
                            .clicked()
                        {
                            close = true;
                        }
                        ui.label(
                            egui::RichText::new(title)
                                .size(crate::ui::style::FONT_LG)
                                .strong(),
                        );
                        if gear {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                egui::RichText::new(egui_phosphor::regular::GEAR)
                                                    .size(crate::ui::style::FONT_LG),
                                            )
                                            .fill(egui::Color32::TRANSPARENT)
                                            .stroke(egui::Stroke::NONE),
                                        )
                                        .named_toggle("Quick settings for this page", gear_on)
                                        .clicked()
                                    {
                                        gear_on = !gear_on;
                                    }
                                },
                            );
                        }
                    });
                });
            });
        self.gear = gear_on;
        if close {
            *open = false;
            return None;
        }

        let frame = egui::Frame::window(&ctx.style_of(ctx.theme()))
            .corner_radius(if cfg!(target_os = "android") {
                0
            } else {
                crate::ui::style::RADIUS_LG as u8
            })
            .shadow(egui::epaint::Shadow::NONE)
            .inner_margin(egui::Margin::symmetric(
                crate::ui::m3::SP_3 as i8,
                crate::ui::m3::SP_2 as i8,
            ));
        Some(
            w.title_bar(false)
                .fixed_rect(body)
                .resizable(false)
                .collapsible(false)
                .vscroll(true)
                .frame(frame),
        )
    }

    /// Walk the drawer in from the edge it lives on, overshooting a hair before it settles.
    ///
    /// ponytail: translation only — no fade, no scale. The drawer is opaque glass over a map, so
    /// sliding is the only one of the three that reads at all, and it is the one that says where
    /// the thing came from.
    fn slide(&self, ctx: &egui::Context, head: Rect, body: Rect) -> (Rect, Rect) {
        let dur = crate::ui::m3::DUR_MED as f64;
        let t = (ctx.input(|i| i.time) - self.opened_at) / dur;
        if crate::ui::motion::reduced() || t >= 1.0 {
            return (head, body);
        }
        ctx.request_repaint();
        let off = (1.0 - crate::ui::motion::ease_out_back(t as f32)) * (head.width() + X);
        (
            head.translate(vec2(-off, 0.0)),
            body.translate(vec2(-off, 0.0)),
        )
    }
}

/// Header and body rectangles for the current screen.
fn rects(ctx: &egui::Context, width: f32) -> (Rect, Rect) {
    let full = ctx.content_rect();
    let (x, w, top, bottom) = if cfg!(target_os = "android") {
        (full.left(), full.width(), full.top(), full.bottom())
    } else {
        (
            full.left() + X,
            width.min(full.width() - 2.0 * X),
            full.top() + TOP,
            (full.bottom() - BOTTOM_CLEARANCE).max(full.top() + TOP + 200.0),
        )
    };
    let head = Rect::from_min_size(pos2(x, top), vec2(w, HEADER_H));
    let body = Rect::from_min_max(pos2(x, head.bottom() + 4.0), pos2(x + w, bottom));
    (head, body)
}
