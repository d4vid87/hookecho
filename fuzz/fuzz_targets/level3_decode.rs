//! Level 3 products are hand-decoded from packet descriptors in this repo (crates/nexrad-level3),
//! which makes them the most likely place for an index to run off the end of a short message.
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = nexrad_level3::decode(data);
});
