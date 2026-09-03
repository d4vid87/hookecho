//! Speaking warnings out loud.
//!
//! Two layers. The words come from [`wxdata::spoken`] — hazard, place and heading first, NWS
//! shorthand expanded. The voice is whatever the machine has: a local neural engine (Piper) when
//! one is configured, otherwise the platform's own synthesizer.
//!
//! Chasing is an eyes-on-the-road activity: a warning you have to read is a warning you read at
//! the wrong moment. Every call is fire-and-forget on a background thread and every failure is
//! logged and dropped — a machine with no speech engine is a normal machine, not a broken one.

/// Piper binary and voice model, as configured. Empty binary means "look on PATH"; empty voice
/// means Piper is off, since a neural engine with no model has nothing to say.
///
/// A global rather than a threaded-through parameter: `speak` is called from a dozen places that
/// have no reason to know about settings, and this is one string pair that changes when the user
/// edits it.
#[cfg(not(target_arch = "wasm32"))]
static PIPER: std::sync::RwLock<(String, String)> =
    std::sync::RwLock::new((String::new(), String::new()));

/// Point the speech path at a Piper binary and voice model. Called at startup and whenever the
/// user edits either field.
#[cfg(not(target_arch = "wasm32"))]
pub fn set_piper(bin: &str, voice: &str) {
    if let Ok(mut g) = PIPER.write() {
        *g = (bin.trim().to_string(), voice.trim().to_string());
    }
}

/// Where a downloaded voice lands. One file per voice id, so downloading a second voice does not
/// destroy the first — but only one is ever active, which is what `settings.piper_voice` says.
#[cfg(not(target_arch = "wasm32"))]
pub fn voice_path(id: &str) -> Option<std::path::PathBuf> {
    crate::paths::data_dir().map(|d| d.join("voices").join(format!("{id}.onnx")))
}

/// Where the default voice lands — the one the old single-slot download wrote, so an install that
/// already has it keeps working without downloading anything.
#[cfg(not(target_arch = "wasm32"))]
pub fn default_voice_path() -> Option<std::path::PathBuf> {
    voice_path(DEFAULT_VOICE)
}

/// The voice preselected in the picker: a mid-quality US English model, the smallest one that
/// does not sound worse than espeak.
#[cfg(not(target_arch = "wasm32"))]
pub const DEFAULT_VOICE: &str = "en_US-lessac-medium";

/// The voices the picker offers, by Piper id (`{lang}_{REGION}-{name}-{quality}`).
///
/// ponytail: a curated handful, not the whole repository. Piper publishes hundreds across dozens
/// of languages, and listing them means shipping (and refreshing) an index of a repository that
/// is not ours. Anything not here is still reachable — the voice field takes a path to any
/// `.onnx` the user downloaded themselves.
#[cfg(not(target_arch = "wasm32"))]
pub const VOICES: &[&str] = &[
    "en_US-lessac-medium",
    "en_US-amy-medium",
    "en_US-ryan-high",
    "en_US-joe-medium",
    "en_US-kusal-medium",
    "en_US-hfc_female-medium",
    "en_US-hfc_male-medium",
    "en_GB-alba-medium",
    "en_GB-jenny_dioco-medium",
    "en_GB-northern_english_male-medium",
    "es_ES-davefx-medium",
    "de_DE-thorsten-medium",
    "fr_FR-siwis-medium",
    "it_IT-riccardo-x_low",
    "pt_BR-faber-medium",
];

/// Upstream `.onnx` for a voice id, in Piper's own voice repository. The layout is derived from
/// the id itself — `en_US-lessac-medium` lives at `en/en_US/lessac/medium/` — and the
/// `.onnx.json` beside it is required, since Piper reads its sample rate and phoneme table
/// from there.
///
/// `None` for an id that is not shaped like a Piper id, which is how a hand-typed value is
/// refused before it becomes a URL.
/// The `rhasspy/piper-voices` revision voices are fetched from.
///
/// A commit, not `main`. These files are ONNX models this app downloads and then runs, and
/// `resolve/main` means whatever is at the head of someone else's repository at the moment the
/// button is pressed — a different model tomorrow, and a different one for two users on the same
/// build. Pinning makes the download reproducible and makes a change to it a change to this file.
///
/// Bumping it is deliberate: pick a commit from
/// `https://huggingface.co/api/models/rhasspy/piper-voices` and paste its `sha` here.
#[cfg(not(target_arch = "wasm32"))]
const VOICES_REVISION: &str = "39ab474be869e9181350af6a65e4953eef67aaa0";

#[cfg(not(target_arch = "wasm32"))]
pub fn voice_url(id: &str) -> Option<String> {
    let mut parts = id.split('-');
    let (lang, name, quality) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some()
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return None;
    }
    let base = lang.split('_').next()?;
    Some(format!(
        "https://huggingface.co/rhasspy/piper-voices/resolve/{VOICES_REVISION}/{base}/{lang}/{name}/{quality}/{id}.onnx"
    ))
}

/// What the settings row shows about the voice download.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Default)]
pub struct VoiceStatus {
    /// A download is in flight, so the button hides rather than starting a second one.
    pub busy: bool,
    /// Progress or outcome, already phrased for the user. Empty means nothing to say.
    pub text: String,
}

#[cfg(not(target_arch = "wasm32"))]
static VOICE: std::sync::RwLock<Option<VoiceStatus>> = std::sync::RwLock::new(None);
#[cfg(not(target_arch = "wasm32"))]
static WANT_VOICE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

#[cfg(not(target_arch = "wasm32"))]
pub fn voice_status() -> VoiceStatus {
    VOICE
        .read()
        .ok()
        .and_then(|g| g.clone())
        .unwrap_or_default()
}

#[cfg(not(target_arch = "wasm32"))]
pub fn set_voice_status(busy: bool, text: impl Into<String>) {
    if let Ok(mut g) = VOICE.write() {
        *g = Some(VoiceStatus {
            busy,
            text: text.into(),
        });
    }
}

/// Ask for a voice download by id. The settings window has no HTTP client or runtime of its own,
/// so it parks the request here and the app's frame loop does the work on the shared spawner.
#[cfg(not(target_arch = "wasm32"))]
pub fn request_voice_download(id: &str) {
    if let Ok(mut g) = WANT_VOICE.lock() {
        *g = Some(id.to_string());
    }
    set_voice_status(true, "starting…");
}

/// Drained once per frame by the app.
#[cfg(not(target_arch = "wasm32"))]
pub fn take_voice_request() -> Option<String> {
    WANT_VOICE.lock().ok().and_then(|mut g| g.take())
}

/// Speak `text` aloud, if the platform can. Returns immediately.
#[cfg(not(target_arch = "wasm32"))]
pub fn speak(text: &str) {
    let text = text.to_string();
    std::thread::spawn(move || {
        if let Err(e) = imp::speak_blocking(&text) {
            log::warn!("speech failed: {e}");
        }
    });
}

/// The browser speaks asynchronously on its own, so there is no thread to spawn — and no thread to
/// spawn with, since wasm32 has no `std::thread`.
#[cfg(target_arch = "wasm32")]
pub fn speak(text: &str) {
    if let Err(e) = imp::speak_blocking(text) {
        log::warn!("speech failed: {e}");
    }
}

#[cfg(target_arch = "wasm32")]
mod imp {
    /// `speechSynthesis` is in every browser we build for, so the web arm needs no engine of its
    /// own. A browser with speech disabled throws, which lands in the same warn-and-drop path as a
    /// desktop with no espeak.
    pub fn speak_blocking(text: &str) -> Result<(), String> {
        let synth = web_sys::window()
            .ok_or("no window")?
            .speech_synthesis()
            .map_err(|_| "no speechSynthesis")?;
        let utter = web_sys::SpeechSynthesisUtterance::new_with_text(text)
            .map_err(|_| "utterance failed")?;
        synth.speak(&utter);
        Ok(())
    }
}

#[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
mod imp {
    use std::process::{Command, Stdio};

    /// Desktop speech goes through whatever the system already has: speech-dispatcher or espeak on
    /// Linux, PowerShell's System.Speech on Windows, `say` on macOS. No new dependency, and on a
    /// box with none of them installed this is a no-op rather than a failed build.
    pub fn speak_blocking(text: &str) -> Result<(), String> {
        // Piper first when it is configured and present: a neural voice is the difference between
        // a warning you listen to and one you turn off. Anything wrong with it — missing binary,
        // missing model, a crash — falls through to the platform engine rather than going silent.
        match piper(text) {
            Ok(true) => return Ok(()),
            Ok(false) => {}
            Err(e) => log::warn!("piper failed, falling back: {e}"),
        }
        #[cfg(target_os = "linux")]
        let candidates: Vec<(&str, Vec<String>)> = vec![
            ("spd-say", vec!["-w".into(), text.to_string()]),
            ("espeak-ng", vec![text.to_string()]),
            ("espeak", vec![text.to_string()]),
        ];
        #[cfg(target_os = "macos")]
        let candidates: Vec<(&str, Vec<String>)> = vec![("say", vec![text.to_string()])];
        #[cfg(target_os = "windows")]
        let candidates: Vec<(&str, Vec<String>)> = vec![(
            "powershell",
            vec![
                "-NoProfile".into(),
                "-Command".into(),
                format!(
                    "Add-Type -AssemblyName System.Speech; \
                     (New-Object System.Speech.Synthesis.SpeechSynthesizer).Speak('{}')",
                    text.replace('\'', "''")
                ),
            ],
        )];
        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        let candidates: Vec<(&str, Vec<String>)> = Vec::new();

        for (bin, args) in &candidates {
            let mut cmd = Command::new(bin);
            crate::platform::no_window(&mut cmd);
            match cmd
                .args(args)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
            {
                Ok(s) if s.success() => return Ok(()),
                Ok(_) => continue,
                Err(_) => continue, // not installed; try the next one
            }
        }
        Err("no speech engine found (tried spd-say / espeak)".into())
    }

    /// Synthesize through Piper and play the result. `Ok(false)` means Piper is not configured,
    /// which is not an error — it is the default state of every machine.
    ///
    // ponytail: subprocess, not in-process ONNX. `ort` would put a C++ runtime and a licence
    // question into a build that currently cross-compiles to four targets without one; the
    // process boundary costs a fork per warning, which is nothing against a network fetch.
    fn piper(text: &str) -> Result<bool, String> {
        use std::io::Write;
        let (bin, voice) = super::PIPER
            .read()
            .map(|g| g.clone())
            .map_err(|_| "piper config poisoned")?;
        if voice.is_empty() || !std::path::Path::new(&voice).exists() {
            return Ok(false);
        }
        let bin = if bin.is_empty() {
            "piper".to_string()
        } else {
            bin
        };
        let mut cmd = Command::new(&bin);
        crate::platform::no_window(&mut cmd);
        // `--output_file -` gives a WAV on stdout, which rodio decodes directly. Writing to a
        // temp file would mean choosing a directory and cleaning it up for no gain.
        let mut child = match cmd
            .args(["--model", &voice, "--output_file", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            // Not installed is the common case, and it is not a failure worth a warning.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(format!("spawn {bin}: {e}")),
        };
        child
            .stdin
            .take()
            .ok_or("no stdin")?
            .write_all(text.as_bytes())
            .map_err(|e| e.to_string())?;
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        if !out.status.success() || out.stdout.is_empty() {
            return Err(format!("piper exited {}", out.status));
        }
        crate::audio::play_wav(out.stdout);
        Ok(true)
    }
}

#[cfg(target_os = "android")]
mod imp {
    /// Android's TextToSpeech is JNI, and all JNI lives in `platform`.
    pub fn speak_blocking(text: &str) -> Result<(), String> {
        crate::platform::speak(text)
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    /// The repository layout is derived from the id rather than listed, so the derivation is the
    /// thing that can be wrong. Every id in [`VOICES`] was checked live when it was added.
    #[test]
    fn a_voice_id_derives_its_own_download_url() {
        assert_eq!(
            voice_url("en_US-lessac-medium"),
            Some(format!("https://huggingface.co/rhasspy/piper-voices/resolve/{VOICES_REVISION}/en/en_US/lessac/medium/en_US-lessac-medium.onnx"))
        );
        assert_eq!(
            voice_url("pt_BR-faber-medium"),
            Some(format!("https://huggingface.co/rhasspy/piper-voices/resolve/{VOICES_REVISION}/pt/pt_BR/faber/medium/pt_BR-faber-medium.onnx"))
        );
        // A commit, never a branch: `resolve/main` is whatever someone else's head happens to be.
        assert!(
            VOICES_REVISION.len() == 40 && VOICES_REVISION.chars().all(|c| c.is_ascii_hexdigit()),
            "VOICES_REVISION must be a full commit sha, got {VOICES_REVISION}"
        );
        // Underscores are part of names and qualities both, and must survive.
        assert!(voice_url("en_US-hfc_female-medium")
            .unwrap()
            .ends_with("/en/en_US/hfc_female/medium/en_US-hfc_female-medium.onnx"));
        assert!(voice_url("it_IT-riccardo-x_low")
            .unwrap()
            .ends_with("/it/it_IT/riccardo/x_low/it_IT-riccardo-x_low.onnx"));

        // Anything not shaped like a Piper id is refused before it becomes a URL.
        assert_eq!(voice_url("lessac"), None);
        assert_eq!(voice_url("en_US-lessac-medium-extra"), None);
        assert_eq!(voice_url("en_US-../../etc-medium"), None);
        assert!(VOICES.iter().all(|id| voice_url(id).is_some()));
        assert!(VOICES.contains(&DEFAULT_VOICE));
    }
}
