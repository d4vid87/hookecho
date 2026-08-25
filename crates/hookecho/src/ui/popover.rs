//! Cards that answer a click on the map, anchored where the click was.
//!
//! A storm cell, a warning polygon, a marker, a point forecast: none of these is a place you go,
//! they are all a thing you pointed at. Answering in a window that opens wherever egui last left
//! one — halfway across the display, over the storm you were looking at — makes the user connect
//! the answer to the question themselves. So these open next to the pointer instead, and stay
//! inside the map area rather than under the chrome that surrounds it.
//!
//! The anchor is latched the first frame a card draws and thrown away when it stops drawing, so
//! the card does not follow the mouse and a reopened card takes the new click, not the old one.
//! Nothing about that needs the caller's help, which is why it lives here and not in `app`.
//!
//! ponytail: `egui::Window` again, same reason as [`crate::ui::drawer`] — it already has the title
//! row, the ✕ wired to the caller's `open` flag, and a body. All this adds is where it lands.

use egui::{pos2, Pos2, Rect};

/// Gap between the click and the card's corner, so the card never covers what was clicked.
const OFFSET: f32 = 14.0;
/// Keep-out margins for the floating chrome: the search pill on top, the control column on the
/// right, the scrubber along the bottom. Same numbers the chrome itself uses.
const KEEPOUT_TOP: f32 = 58.0;
const KEEPOUT_RIGHT: f32 = 74.0;
const KEEPOUT_BOTTOM: f32 = 96.0;
const KEEPOUT_SIDE: f32 = 10.0;
/// How far a card rises into place when it opens.
const RISE: f32 = 12.0;

#[derive(Default)]
pub struct Popovers {
    /// Where each open card was summoned from, and when, keyed by the id its caller passes.
    at: Vec<(String, Pos2, f64)>,
    /// Ids that drew this frame; anything else has closed and loses its anchor.
    seen: Vec<String>,
    frame: u64,
}

impl Popovers {
    /// Place `w` at the click that opened it. Call once per frame per open card.
    pub fn card<'a>(
        &mut self,
        ctx: &egui::Context,
        id: &str,
        w: egui::Window<'a>,
    ) -> egui::Window<'a> {
        // A phone has no room to put a card beside anything: Material 3's compact width class
        // wants the whole screen, which is what `phone_surface` already gives it.
        if cfg!(target_os = "android") {
            return crate::ui::phone_surface(ctx, w);
        }
        let frame = ctx.cumulative_pass_nr();
        if frame != self.frame {
            self.at.retain(|(k, ..)| self.seen.iter().any(|s| s == k));
            self.seen.clear();
            self.frame = frame;
        }
        self.seen.push(id.to_string());
        let field = field(ctx);
        let now = ctx.input(|i| i.time);
        let (anchor, born) = match self.at.iter().find(|(k, ..)| k == id) {
            Some((_, p, t)) => (*p, *t),
            None => {
                // No pointer at all (a card opened by a hotkey, or a touch that already lifted):
                // the middle of the map is the least wrong place left.
                let p = ctx
                    .pointer_interact_pos()
                    .map(|p| p + egui::vec2(OFFSET, OFFSET))
                    .unwrap_or_else(|| field.center());
                self.at.push((id.to_string(), p, now));
                (p, now)
            }
        };
        // A card that appears at full size looks like it was already there; one that rises the
        // last few pixels into place looks like it came from the click. Two frames of work.
        let t = ((now - born) / crate::ui::m3::DUR_SHORT as f64) as f32;
        let anchor = if crate::ui::motion::reduced() || t >= 1.0 {
            anchor
        } else {
            ctx.request_repaint();
            anchor + egui::vec2(0.0, (1.0 - crate::ui::motion::ease_out_cubic(t)) * RISE)
        };
        w.fixed_pos(anchor).constrain_to(field)
    }
}

/// The part of the screen a card may occupy: the map, minus the floating chrome around it.
fn field(ctx: &egui::Context) -> Rect {
    let r = ctx.content_rect();
    let min = pos2(r.left() + KEEPOUT_SIDE, r.top() + KEEPOUT_TOP);
    let max = pos2(r.right() - KEEPOUT_RIGHT, r.bottom() - KEEPOUT_BOTTOM);
    // A window small enough to matter still needs a rect that isn't inside-out.
    Rect::from_min_max(min, max.max(min + egui::vec2(200.0, 200.0)))
}
