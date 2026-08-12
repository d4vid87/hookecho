//! Archive II volumes come off a public S3 bucket, byte-for-byte as NOAA wrote them, and a
//! truncated or half-written object at the live head is a normal event rather than an attack.
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: Vec<u8>| {
    let _ = wxdata::level2::decode_volume(data);
});
