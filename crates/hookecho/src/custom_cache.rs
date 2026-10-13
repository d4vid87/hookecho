//! Identity of a derived sweep. A weak reference prevents decoded-volume retention and ABA reuse.
use std::sync::{Arc, Weak};
pub struct Input<T> {
    scan: Weak<T>,
    tilt: usize,
    cut_end_ms: Option<i64>,
    precision: bool,
    recipe: u64,
}
impl<T> Input<T> {
    pub fn new(
        scan: &Arc<T>,
        tilt: usize,
        cut_end_ms: Option<i64>,
        precision: bool,
        recipe: u64,
    ) -> Self {
        Self {
            scan: Arc::downgrade(scan),
            tilt,
            cut_end_ms,
            precision,
            recipe,
        }
    }
    pub fn matches(
        &self,
        scan: &Arc<T>,
        tilt: usize,
        cut_end_ms: Option<i64>,
        precision: bool,
        recipe: u64,
    ) -> bool {
        self.cut_end_ms == cut_end_ms
            && self.tilt == tilt
            && self.precision == precision
            && self.recipe == recipe
            && self
                .scan
                .upgrade()
                .is_some_and(|previous| Arc::ptr_eq(&previous, scan))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn changed_sources_and_display_dependencies_invalidate_output() {
        let scan = Arc::new(vec![1_u8, 2]);
        let key = Input::new(&scan, 0, None, false, 17);
        assert!(key.matches(&scan.clone(), 0, None, false, 17));
        assert!(!key.matches(&Arc::new(vec![1_u8, 2]), 0, None, false, 17));
        assert!(!key.matches(&scan, 1, None, false, 17));
        assert!(!key.matches(&scan, 0, None, true, 17));
        assert!(!key.matches(&scan, 0, None, false, 18));
        assert!(!key.matches(&scan, 0, Some(1000), false, 17));
        let weak = Arc::downgrade(&scan);
        drop(scan);
        assert!(
            weak.upgrade().is_none(),
            "cache identity must not retain decoded data"
        );
        assert!(!key.matches(&Arc::new(vec![1_u8, 2]), 0, None, false, 17));
    }
}
