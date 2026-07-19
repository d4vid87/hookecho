//! Warning audio cue: a short two-tone chime synthesized with rodio (no bundled asset).
//!
//! Playback runs on a detached thread that owns the output stream for its lifetime, so the
//! call returns immediately. A missing/!busy audio device is logged, never fatal — headless
//! and audio-less machines keep working.

use rodio::source::{SineWave, Source};
use std::time::Duration;

/// Play the new-warning chime once (non-blocking). Best-effort: failures are logged.
pub fn warning_chime() {
    std::thread::spawn(|| {
        let (_stream, handle) = match rodio::OutputStream::try_default() {
            Ok(s) => s,
            Err(e) => {
                log::warn!("no audio output for warning chime: {e}");
                return;
            }
        };
        let sink = match rodio::Sink::try_new(&handle) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("audio sink failed: {e}");
                return;
            }
        };
        // Two-tone alert: high then lower, gentle fade so it isn't harsh.
        let hi = SineWave::new(880.0)
            .take_duration(Duration::from_millis(280))
            .amplify(0.20)
            .fade_in(Duration::from_millis(20));
        let lo = SineWave::new(660.0)
            .take_duration(Duration::from_millis(320))
            .amplify(0.20);
        sink.append(hi);
        sink.append(lo);
        sink.sleep_until_end(); // keep the stream alive until playback finishes
    });
}
