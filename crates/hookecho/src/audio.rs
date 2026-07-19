//! Alert audio cues: five short synthesized tones (no bundled assets) plus optional user files.
//!
//! Playback runs on a detached thread that owns the output stream for its lifetime, so the
//! call returns immediately. A missing/busy audio device or an undecodable file is logged,
//! never fatal — headless and audio-less machines keep working.

use crate::settings::AlertSound;
use rodio::source::{SineWave, Source};
use rodio::Sink;
use std::io::BufReader;
use std::time::Duration;

/// Play an alert sound once at `volume` (0.0..=1.0), non-blocking. Best-effort: failures logged.
pub fn play(sound: &AlertSound, volume: f32) {
    let sound = sound.clone();
    std::thread::spawn(move || {
        let (_stream, handle) = match rodio::OutputStream::try_default() {
            Ok(s) => s,
            Err(e) => {
                log::warn!("no audio output for alert sound: {e}");
                return;
            }
        };
        let sink = match Sink::try_new(&handle) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("audio sink failed: {e}");
                return;
            }
        };
        sink.set_volume(volume.clamp(0.0, 1.0));
        match &sound {
            AlertSound::Custom(path) => {
                if !append_file(&sink, path) {
                    // Fall back to the default chime so the alert is never silent.
                    append_builtin(&sink, &AlertSound::Chime);
                }
            }
            builtin => append_builtin(&sink, builtin),
        }
        sink.sleep_until_end(); // keep the stream alive until playback finishes
    });
}

/// Try to queue a user audio file (wav/mp3/ogg/flac). Returns false (logged) on any failure.
fn append_file(sink: &Sink, path: &str) -> bool {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) => {
            log::warn!("alert sound file open failed ({path}): {e}");
            return false;
        }
    };
    match rodio::Decoder::new(BufReader::new(file)) {
        Ok(src) => {
            sink.append(src);
            true
        }
        Err(e) => {
            log::warn!("alert sound decode failed ({path}): {e}");
            false
        }
    }
}

/// Queue a synthesized built-in tone. `Custom` is treated as `Chime` (callers route files above).
fn append_builtin(sink: &Sink, sound: &AlertSound) {
    let tone = |freq: f32, ms: u64| SineWave::new(freq).take_duration(Duration::from_millis(ms));
    match sound {
        // Two-tone alert: high then lower, gentle fade so it isn't harsh.
        AlertSound::Chime | AlertSound::Custom(_) => {
            sink.append(tone(880.0, 280).fade_in(Duration::from_millis(20)));
            sink.append(tone(660.0, 320));
        }
        // Single bright note with a quick fade tail.
        AlertSound::Ding => {
            sink.append(tone(1047.0, 220).fade_in(Duration::from_millis(10)));
        }
        // Rising/falling siren sweep, two cycles.
        AlertSound::Siren => {
            for _ in 0..2 {
                sink.append(tone(600.0, 200));
                sink.append(tone(900.0, 200));
            }
        }
        // Three urgent bursts separated by gaps (silence via near-zero-amplitude tone).
        AlertSound::Alarm => {
            for _ in 0..3 {
                sink.append(tone(950.0, 160));
                sink.append(tone(950.0, 90).amplify(0.0));
            }
        }
        // Four rapid low ticks.
        AlertSound::Pulse => {
            for _ in 0..4 {
                sink.append(tone(440.0, 70));
                sink.append(tone(440.0, 50).amplify(0.0));
            }
        }
    }
}
