//! Counts retained encoded gate payloads once per shared scan allocation, excluding metadata.
use std::collections::HashMap;
use std::sync::{Arc, Weak};
use wxdata::level2::Scan;
#[derive(Default)]
pub struct Inventory {
    scans: HashMap<usize, (Weak<Scan>, usize)>,
}
impl Inventory {
    pub fn bytes<'a>(&mut self, scans: impl Iterator<Item = &'a Arc<Scan>>) -> usize {
        self.scans.retain(|_, (scan, _)| scan.strong_count() > 0);
        let mut seen = std::collections::HashSet::new();
        scans
            .map(|scan| {
                let address = Arc::as_ptr(scan) as usize;
                if !seen.insert(address) {
                    return 0;
                }
                let (_, bytes) = self.scans.entry(address).or_insert_with(|| {
                    (Arc::downgrade(scan), wxdata::level2::scan_gate_bytes(scan))
                });
                *bytes
            })
            .sum()
    }
}
