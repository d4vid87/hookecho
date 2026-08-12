//! Placefiles are third-party text: users point the app at URLs other people publish.
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = wxdata::placefile::parse(&String::from_utf8_lossy(data));
});
