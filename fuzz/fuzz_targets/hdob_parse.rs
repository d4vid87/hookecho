//! Hurricane-hunter HDOB bulletins: fixed-column text, parsed by slicing.
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = wxdata::recon::parse_hdob(&String::from_utf8_lossy(data));
});
