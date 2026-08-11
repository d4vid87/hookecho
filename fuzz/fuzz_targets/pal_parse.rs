//! GRLevelX `.pal` colour tables — files users drop in from anywhere.
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = hookecho::colormap::parse_pal(&String::from_utf8_lossy(data));
});
