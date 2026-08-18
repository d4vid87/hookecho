//! One frame-level label placer, shared by every layer that draws text or a symbol on the map.
//!
//! Each layer used to keep its own `Vec<Rect>` and greedily skip anything that overlapped
//! something *it* had already drawn. Three of those existed, so a METAR plot could sit on top of
//! a city name and a river gauge on top of both: each declutter was correct and the map still
//! came out overlapped.
//!
//! This is the same greedy algorithm with two things added. The occupancy is shared, so a layer
//! sees what every earlier layer took. And a label that was drawn last frame is offered its slot
//! before any newcomer in its tier, which is what stops names flickering in and out while you pan
//! — without stickiness a label at the edge of a collision wins and loses on alternate frames.
//!
//! ponytail: greedy by priority, no annealing and no nudging labels off their anchors. A label
//! either fits where it belongs or it is not drawn.

use std::collections::HashSet;

/// Draw priority. Higher-priority layers reserve first, so a warning label can never be squeezed
/// out by a river gauge that happened to be drawn earlier.
///
/// The order here *is* the order layers must call [`Placer::place`] in; [`Placer::place`] asserts
/// it in debug builds, so getting the draw order wrong is a test failure rather than a subtle
/// visual regression.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    /// Warnings and storm cells — the things the app exists to show.
    Warning,
    /// City and town names from the vector basemap.
    Place,
    /// Surface observations.
    Station,
    /// User placefile text.
    Placefile,
    /// River gauges, points of interest: useful, never at the expense of the above.
    Minor,
}

/// A stable identity for one label across frames. Anything hashable that names the *object*, not
/// its position — the position is what moves.
pub type LabelKey = u64;

/// Hash a string id into a [`LabelKey`].
pub fn key(s: &str) -> LabelKey {
    // ponytail: FNV-1a. This only has to separate labels within one frame's occupancy set.
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

#[derive(Default)]
pub struct Placer {
    taken: Vec<egui::Rect>,
    /// Keys that were drawn on the previous frame.
    shown_last: HashSet<LabelKey>,
    /// Keys drawn so far this frame; becomes `shown_last` at the next [`Self::begin`].
    shown_now: HashSet<LabelKey>,
    /// Highest priority seen this frame, to catch a layer reserving out of order.
    last_priority: Option<Priority>,
}

impl Placer {
    /// Start a frame. Call once, before any layer places anything.
    pub fn begin(&mut self) {
        self.taken.clear();
        std::mem::swap(&mut self.shown_last, &mut self.shown_now);
        self.shown_now.clear();
        self.last_priority = None;
    }

    /// Was this label drawn on the previous frame? Layers sort their candidates by this first, so
    /// a label that is already on screen gets its slot back before a newcomer takes it.
    pub fn was_shown(&self, key: LabelKey) -> bool {
        self.shown_last.contains(&key)
    }

    /// Reserve `rect` for `key`. Returns false if it collides with something already reserved, in
    /// which case the caller draws nothing.
    pub fn place(&mut self, key: LabelKey, rect: egui::Rect, priority: Priority) -> bool {
        debug_assert!(
            self.last_priority.is_none_or(|p| p <= priority),
            "label layers must reserve in priority order: {priority:?} after {:?}",
            self.last_priority
        );
        self.last_priority = Some(priority);
        if self.taken.iter().any(|r| r.intersects(rect)) {
            return false;
        }
        self.taken.push(rect);
        self.shown_now.insert(key);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: f32, y: f32) -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(10.0, 10.0))
    }

    #[test]
    fn overlapping_labels_lose_and_disjoint_ones_do_not() {
        let mut p = Placer::default();
        p.begin();
        assert!(p.place(key("a"), r(0.0, 0.0), Priority::Place));
        assert!(!p.place(key("b"), r(5.0, 5.0), Priority::Place));
        assert!(p.place(key("c"), r(50.0, 50.0), Priority::Place));
    }

    /// The whole point of sharing one placer: a later layer must see what an earlier one took.
    #[test]
    fn occupancy_is_shared_across_layers() {
        let mut p = Placer::default();
        p.begin();
        assert!(p.place(key("city"), r(0.0, 0.0), Priority::Place));
        assert!(!p.place(key("metar"), r(4.0, 4.0), Priority::Station));
        assert!(!p.place(key("gauge"), r(2.0, 2.0), Priority::Minor));
    }

    /// Stickiness: the caller asks about the previous frame and offers returning labels first,
    /// which is what keeps a contested label from alternating frame to frame.
    #[test]
    fn a_label_drawn_last_frame_is_known_next_frame() {
        let mut p = Placer::default();
        p.begin();
        assert!(!p.was_shown(key("a")));
        p.place(key("a"), r(0.0, 0.0), Priority::Place);
        // Still the same frame: `was_shown` is about the previous one.
        assert!(!p.was_shown(key("a")));
        p.begin();
        assert!(p.was_shown(key("a")));
        // Dropped for a frame, and it is no longer sticky.
        p.begin();
        assert!(!p.was_shown(key("a")));
    }

    #[test]
    fn a_frame_starts_empty() {
        let mut p = Placer::default();
        p.begin();
        assert!(p.place(key("a"), r(0.0, 0.0), Priority::Place));
        p.begin();
        assert!(p.place(key("b"), r(0.0, 0.0), Priority::Place));
    }

    #[test]
    #[should_panic(expected = "priority order")]
    fn reserving_out_of_priority_order_is_a_bug() {
        let mut p = Placer::default();
        p.begin();
        p.place(key("gauge"), r(0.0, 0.0), Priority::Minor);
        p.place(key("city"), r(50.0, 50.0), Priority::Place);
    }
}
