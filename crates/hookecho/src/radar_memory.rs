//! Counts retained encoded gate payloads once per shared scan allocation, excluding metadata.
use std::collections::HashMap;
use std::sync::{Arc, Weak};
use wxdata::level2::Scan;
#[derive(Default)]
pub struct Inventory {
    scans: HashMap<usize, (Weak<Scan>, usize)>,
}
impl Inventory {
    /// Memoize a scan's encoded payload size so UI and loop-budget checks do not walk every
    /// radial on every frame. The weak entry expires when the last displayed/cache owner does.
    pub fn scan_bytes(&mut self, scan: &Arc<Scan>) -> usize {
        let address = Arc::as_ptr(scan) as usize;
        if self.scans.get(&address).is_some_and(|(owner, _)| owner.strong_count() == 0) {
            self.scans.remove(&address);
        }
        self.scans.entry(address).or_insert_with(|| {
            (Arc::downgrade(scan), wxdata::level2::scan_gate_bytes(scan))
        }).1
    }

    pub fn bytes<'a>(&mut self, scans: impl Iterator<Item = &'a Arc<Scan>>) -> usize {
        self.scans.retain(|_, (scan, _)| scan.strong_count() > 0);
        let mut seen = std::collections::HashSet::new();
        scans
            .map(|scan| {
                let address = Arc::as_ptr(scan) as usize;
                if !seen.insert(address) {
                    return 0;
                }
                self.scan_bytes(scan)
            })
            .sum()
    }
}
