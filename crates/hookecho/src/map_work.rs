//! One upload budget shared by all panes, so quad view cannot multiply gesture-frame work.

/// Bound optional geography text work without changing the cap when a gesture starts.
pub const PLACE_LABELS_PER_PANE: usize = 192;
/// Route shields have many duplicate candidates. Reserve at least half the text work for
/// names so an interstate cannot consume every layout slot before nearby towns are considered.
#[derive(Default)]
pub struct LabelBudget {
    total: usize,
    shields: usize,
}
impl LabelBudget {
    pub fn take(&mut self, shield: bool) -> bool {
        if self.total >= PLACE_LABELS_PER_PANE
            || (shield && self.shields >= PLACE_LABELS_PER_PANE / 2)
        {
            return false;
        }
        self.total += 1;
        if shield {
            self.shields += 1;
        }
        true
    }
    pub fn exhausted(&self) -> bool {
        self.total >= PLACE_LABELS_PER_PANE
    }
}

#[derive(Debug)]
pub struct UploadBudget {
    remaining: usize,
    bytes: usize,
    used: usize,
}
impl UploadBudget {
    pub fn frame(moving: bool) -> Self {
        Self {
            remaining: if moving { 2 } else { 8 },
            bytes: if moving { 2 << 20 } else { 8 << 20 },
            used: 0,
        }
    }
    pub fn legacy_batch() -> Self {
        Self {
            remaining: 2,
            bytes: usize::MAX,
            used: 0,
        }
    }
    /// Permit one oversized tile to make progress, but never a second in the same frame.
    pub fn take(&mut self, bytes: usize) -> bool {
        if self.remaining == 0 || (bytes > self.bytes && self.used > 0) {
            return false;
        }
        self.remaining -= 1;
        self.used += 1;
        self.bytes = self.bytes.saturating_sub(bytes);
        true
    }
    pub fn exhausted(&self) -> bool {
        self.remaining == 0
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn panes_share_count_and_bytes_and_an_oversized_tile_can_progress() {
        let mut b = UploadBudget::frame(true);
        assert!(b.take(1 << 20));
        assert!(b.take(1 << 20));
        assert!(!b.take(1));
        let mut b = UploadBudget::frame(false);
        assert!(b.take(9 << 20));
        assert!(!b.take(1));
        let mut b = UploadBudget::frame(true);
        assert!(b.take(1 << 20));
        assert!(!b.take(2 << 20));
        assert!(b.take(1 << 20));
    }
    #[test]
    fn repeated_shields_cannot_starve_town_names() {
        let mut b = LabelBudget::default();
        for _ in 0..96 {
            assert!(b.take(true));
        }
        assert!(!b.take(true));
        for _ in 0..96 {
            assert!(b.take(false));
        }
        assert!(b.exhausted());
        assert!(!b.take(false));
    }
}
