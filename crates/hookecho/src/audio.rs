//! Alert audio cues: five short synthesized tones (no bundled assets) plus optional user files.
//!
//! Playback runs on a detached thread that owns the output stream for its lifetime, so the
//! call returns immediately. A missing/busy audio device or an undecodable file is logged,
//! never fatal — headless and audio-less machines keep working.
//!
//! The web build has no rodio (cpal's WebAudio backend needs a user-gesture resume and a stream
//! to keep alive); it renders the same tones into a one-shot `<audio>` element instead — see the
//! bottom of this file.

use crate::settings::AlertSound;
#[cfg(not(target_arch = "wasm32"))]
use rodio::source::{SineWave, Source};
#[cfg(not(target_arch = "wasm32"))]
use rodio::Sink;
#[cfg(not(target_arch = "wasm32"))]
use std::io::BufReader;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;

/// Play an alert sound once at `volume` (0.0..=1.0), non-blocking. Best-effort: failures logged.
#[cfg(not(target_arch = "wasm32"))]
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
#[cfg(not(target_arch = "wasm32"))]
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
#[cfg(not(target_arch = "wasm32"))]
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
        // The EAS/NWS Attention Signal: 853 Hz and 960 Hz sounded together. Eight seconds is the
        // broadcast length; two is plenty on a desktop, and unmistakable. Amplitudes are halved
        // so summing the pair doesn't clip.
        AlertSound::Eas => {
            let a = SineWave::new(853.0).amplify(0.5);
            let b = SineWave::new(960.0).amplify(0.5);
            sink.append(
                a.mix(b)
                    .take_duration(Duration::from_millis(2000))
                    .fade_in(Duration::from_millis(30)),
            );
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

// ---------------------------------------------------------------------------------------------
// Web
// ---------------------------------------------------------------------------------------------

/// Sample rate of the generated WAVs. 16 kHz is plenty for tones under ~1.1 kHz and keeps the
/// longest cue (the two-second EAS signal) at ~32 KB before base64.
#[cfg(any(target_arch = "wasm32", test))]
const WEB_RATE: u32 = 16_000;

/// Play an alert sound once at `volume` (0.0..=1.0) through a throwaway `<audio>` element.
///
/// The tone is synthesized to an 8-bit WAV and handed over as a `data:` URL, so nothing is
/// bundled and nothing has to be served. `Custom` has no meaning here — there is no filesystem
/// to read a user's file from — so it falls back to the chime, exactly as a failed open does
/// natively.
///
// ponytail: no rodio and no NWR streaming on the web, and a browser may refuse to play a cue
// that no user gesture asked for (autoplay policy) — the alert is still on screen. A WebAudio
// graph behind a "enable sound" click is the upgrade path if that turns out to matter.
#[cfg(target_arch = "wasm32")]
pub fn play(sound: &AlertSound, volume: f32) {
    use base64::Engine as _;
    let src = format!(
        "data:audio/wav;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(wav(segments(sound)))
    );
    match web_sys::HtmlAudioElement::new_with_src(&src) {
        Ok(el) => {
            el.set_volume(volume.clamp(0.0, 1.0) as f64);
            // The promise is dropped: a rejection (autoplay blocked) is not worth a task to log.
            let _ = el.play();
        }
        Err(e) => log::warn!("alert sound element failed: {e:?}"),
    }
}

/// The built-in cues as `(hz, ms)` runs, `0.0` meaning silence. Mirrors `append_builtin`'s
/// shapes without its fades, and flattens the EAS two-tone mix to its lower note.
#[cfg(any(target_arch = "wasm32", test))]
fn segments(sound: &AlertSound) -> Vec<(f32, u64)> {
    match sound {
        AlertSound::Chime | AlertSound::Custom(_) => vec![(880.0, 280), (660.0, 320)],
        AlertSound::Ding => vec![(1047.0, 220)],
        AlertSound::Siren => [(600.0, 200), (900.0, 200)].repeat(2),
        AlertSound::Alarm => [(950.0, 160), (0.0, 90)].repeat(3),
        AlertSound::Eas => vec![(853.0, 2000)],
        AlertSound::Pulse => [(440.0, 70), (0.0, 50)].repeat(4),
    }
}

/// Render `segs` to a mono 8-bit PCM WAV. Each run is amplitude-ramped over its first and last
/// ~5 ms so the joins don't click.
#[cfg(any(target_arch = "wasm32", test))]
fn wav(segs: Vec<(f32, u64)>) -> Vec<u8> {
    let mut pcm: Vec<u8> = Vec::new();
    for (hz, ms) in segs {
        let n = (WEB_RATE as u64 * ms / 1000) as usize;
        let ramp = (WEB_RATE as usize / 200).max(1); // ~5 ms
        for i in 0..n {
            let env = (i.min(n - i - 1) as f32 / ramp as f32).min(1.0);
            let t = i as f32 / WEB_RATE as f32;
            let s = if hz > 0.0 {
                (t * hz * std::f32::consts::TAU).sin() * env
            } else {
                0.0
            };
            pcm.push((128.0 + s * 120.0) as u8);
        }
    }
    let mut out = Vec::with_capacity(44 + pcm.len());
    let data_len = pcm.len() as u32;
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // PCM fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&WEB_RATE.to_le_bytes());
    out.extend_from_slice(&WEB_RATE.to_le_bytes()); // byte rate = rate * 1 ch * 1 byte
    out.extend_from_slice(&1u16.to_le_bytes()); // block align
    out.extend_from_slice(&8u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(&pcm);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header_and_length_match_the_tone() {
        // 220 ms at 16 kHz mono 8-bit = 3520 sample bytes after a 44-byte header.
        let bytes = wav(segments(&AlertSound::Ding));
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(&bytes[8..16], b"WAVEfmt ");
        assert_eq!(&bytes[36..40], b"data");
        let data_len = u32::from_le_bytes(bytes[40..44].try_into().unwrap()) as usize;
        assert_eq!(data_len, 3520);
        assert_eq!(bytes.len(), 44 + data_len);
        // Silence sits at the 8-bit midpoint, and the ramp keeps the first sample there.
        assert_eq!(bytes[44], 128);
    }
}
