//! Speaking warnings out loud.
//!
//! Two layers. The words come from [`wxdata::spoken`] — hazard, place and heading first, NWS
//! shorthand expanded. The voice is whatever the machine has: a local neural engine (Piper) when
//! one is configured, otherwise the platform's own synthesizer.
//!
//! Chasing is an eyes-on-the-road activity: a warning you have to read is a warning you read at
//! the wrong moment. Every call is fire-and-forget on one background worker and every failure is
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

/// Internal urgency of queued speech. Only an emergency may discard anything already waiting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Priority {
    Background,
    Warning,
    Emergency,
}

struct SpeechJob {
    priority: Priority,
    tone: Option<(crate::settings::AlertSound, f32)>,
    lines: Vec<String>,
}

impl SpeechJob {
    fn new(
        priority: Priority,
        tone: Option<(crate::settings::AlertSound, f32)>,
        lines: Vec<String>,
    ) -> Self {
        Self {
            priority,
            tone,
            lines: lines
                .into_iter()
                .flat_map(|line| split_sentences(&line))
                .collect(),
        }
    }

    fn is_empty(&self) -> bool {
        self.tone.is_none() && self.lines.is_empty()
    }
}

/// Break generated scripts at sentence endings so an emergency never waits behind a whole
/// multi-sentence warning after the voice reaches a natural stopping point.
fn split_sentences(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut start = 0;
    for (i, b) in bytes.iter().enumerate() {
        if matches!(b, b'.' | b'!' | b'?') && bytes.get(i + 1).is_none_or(u8::is_ascii_whitespace) {
            let sentence = text[start..=i].trim();
            if !sentence.is_empty() {
                out.push(sentence.to_string());
            }
            start = i + 1;
        }
    }
    let tail = text[start..].trim();
    if !tail.is_empty() {
        out.push(tail.to_string());
    }
    out
}

fn enqueue(queue: &mut std::collections::VecDeque<SpeechJob>, job: SpeechJob) {
    if job.priority == Priority::Emergency {
        queue.retain(|queued| queued.priority == Priority::Emergency);
    }
    queue.push_back(job);
}

fn emergency_waiting(queue: &std::collections::VecDeque<SpeechJob>) -> bool {
    queue.iter().any(|job| job.priority == Priority::Emergency)
}

fn should_preempt(current: Priority, queue: &std::collections::VecDeque<SpeechJob>) -> bool {
    current != Priority::Emergency && emergency_waiting(queue)
}

#[cfg(not(target_arch = "wasm32"))]
struct SpeechQueue {
    jobs: std::sync::Mutex<std::collections::VecDeque<SpeechJob>>,
    ready: std::sync::Condvar,
}

#[cfg(not(target_arch = "wasm32"))]
fn speech_queue() -> &'static std::sync::Arc<SpeechQueue> {
    static QUEUE: std::sync::OnceLock<std::sync::Arc<SpeechQueue>> = std::sync::OnceLock::new();
    QUEUE.get_or_init(|| {
        let queue = std::sync::Arc::new(SpeechQueue {
            jobs: std::sync::Mutex::new(std::collections::VecDeque::new()),
            ready: std::sync::Condvar::new(),
        });
        let worker = std::sync::Arc::clone(&queue);
        std::thread::spawn(move || speech_worker(&worker));
        queue
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn speech_worker(queue: &SpeechQueue) {
    loop {
        let mut jobs = queue
            .jobs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while jobs.is_empty() {
            jobs = queue
                .ready
                .wait(jobs)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        let job = jobs.pop_front().expect("queue was not empty");
        drop(jobs);

        if let Some((sound, volume)) = job.tone {
            crate::audio::play_blocking(&sound, volume);
        }
        for line in job.lines {
            let jobs = queue
                .jobs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let preempt = should_preempt(job.priority, &jobs);
            drop(jobs);
            if preempt {
                break;
            }
            if let Err(e) = imp::speak_blocking(&line) {
                log::warn!("speech failed: {e}");
                break;
            }
        }
    }
}

/// The tone, then the words — one announcement at a time.
///
/// Returns immediately; one process-wide worker owns the queue and all platform speech calls.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn announce(
    priority: Priority,
    tone: Option<(crate::settings::AlertSound, f32)>,
    lines: Vec<String>,
) {
    let job = SpeechJob::new(priority, tone, lines);
    if job.is_empty() {
        return;
    }
    let queue = speech_queue();
    let mut jobs = queue
        .jobs
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    enqueue(&mut jobs, job);
    queue.ready.notify_one();
}

/// Speak `text` aloud, if the platform can. Returns immediately.
///
/// Goes through [`announce`], so a chase position update waits for a warning to finish rather
/// than interrupting it.
pub fn speak(text: &str) {
    announce(Priority::Background, None, vec![text.to_string()]);
}

#[cfg(target_arch = "wasm32")]
#[derive(Default)]
struct WebQueue {
    jobs: std::collections::VecDeque<SpeechJob>,
    running: bool,
}

#[cfg(target_arch = "wasm32")]
std::thread_local! {
    static WEB_QUEUE: std::cell::RefCell<WebQueue> = std::cell::RefCell::new(WebQueue::default());
}

/// The same single FIFO on the browser's one thread. Only one utterance is handed to
/// `speechSynthesis` at a time, so its `speaking` state is the completion signal and the next
/// sentence is a preemption boundary.
#[cfg(target_arch = "wasm32")]
pub(crate) fn announce(
    priority: Priority,
    tone: Option<(crate::settings::AlertSound, f32)>,
    lines: Vec<String>,
) {
    let job = SpeechJob::new(priority, tone, lines);
    if job.is_empty() {
        return;
    }
    let start = WEB_QUEUE.with(|queue| {
        let mut queue = queue.borrow_mut();
        enqueue(&mut queue.jobs, job);
        if queue.running {
            false
        } else {
            queue.running = true;
            true
        }
    });
    if start {
        wasm_bindgen_futures::spawn_local(web_speech_worker());
    }
}

#[cfg(target_arch = "wasm32")]
async fn web_speech_worker() {
    loop {
        let Some(job) = WEB_QUEUE.with(|queue| queue.borrow_mut().jobs.pop_front()) else {
            WEB_QUEUE.with(|queue| queue.borrow_mut().running = false);
            return;
        };
        if let Some((sound, volume)) = &job.tone {
            crate::audio::play(sound, *volume);
            crate::fonts::sleep_ms(crate::audio::duration_ms(sound)).await;
        }
        for line in job.lines {
            let preempt =
                WEB_QUEUE.with(|queue| should_preempt(job.priority, &queue.borrow().jobs));
            if preempt {
                break;
            }
            if let Err(e) = imp::speak_blocking(&line) {
                log::warn!("speech failed: {e}");
                break;
            }
            // Let the browser start the queued utterance before observing its state.
            crate::fonts::sleep_ms(20).await;
            let mut polls = 0;
            while imp::is_speaking() && polls < 4_800 {
                crate::fonts::sleep_ms(25).await;
                polls += 1;
            }
            if polls == 4_800 {
                log::warn!("browser speech did not finish after 120 seconds");
            }
        }
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

    pub fn is_speaking() -> bool {
        web_sys::window()
            .and_then(|w| w.speech_synthesis().ok())
            .is_some_and(|s| s.speaking())
    }
}

/// What is wrong with the configured Piper binary, once anything is known to be.
///
/// `(binary probed, problem)`. Keyed by path so editing the field re-probes, and written by a real
/// synthesis run too — what actually happened beats what `--help` implied.
#[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
static PROBE: std::sync::Mutex<Option<(String, Option<String>)>> = std::sync::Mutex::new(None);

/// The nudge for a machine with a voice selected and no engine to speak it.
///
/// Named for Arch on purpose: `extra/piper` is a GTK mouse-configuration tool that installs
/// `/usr/bin/piper`, so "install piper" is advice that leads to a program for setting DPI on a
/// gaming mouse. The text-to-speech one is in the AUR.
#[cfg(target_os = "linux")]
pub const NOT_FOUND: &str = "Piper is not installed — on Arch: `yay -S piper-tts-bin` \
                            (not `extra/piper`, which is a mouse tool). Or point the field above \
                            at the binary.";

/// The same nudge where there is no package manager to name.
#[cfg(not(any(target_os = "linux", target_os = "android", target_arch = "wasm32")))]
pub const NOT_FOUND: &str = "Piper is not installed — download piper-tts from \
                            github.com/rhasspy/piper/releases and point the field above at the \
                            binary.";

#[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
fn set_piper_problem(problem: Option<String>) {
    let bin = PIPER.read().map(|g| g.0.clone()).unwrap_or_default();
    if let Ok(mut g) = PROBE.lock() {
        *g = Some((bin, problem));
    }
}

/// What Settings should warn about, or `None` while all is well or not yet known.
///
/// The first call for a given binary starts a probe on a background thread and returns `None`
/// until it lands: this is read every frame, and forking a process per frame to answer it would
/// be worse than the problem.
#[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
pub fn piper_problem() -> Option<String> {
    let bin = PIPER.read().map(|g| g.0.clone()).unwrap_or_default();
    let mut guard = PROBE.lock().ok()?;
    if let Some((probed, problem)) = guard.as_ref() {
        if *probed == bin {
            return problem.clone();
        }
    }
    // Claim the slot before spawning, so a UI redrawing at 60 Hz starts one probe, not sixty.
    *guard = Some((bin.clone(), None));
    drop(guard);
    std::thread::spawn(move || {
        let exe = if bin.is_empty() { "piper" } else { &bin };
        let mut cmd = std::process::Command::new(exe);
        crate::platform::no_window(&mut cmd);
        let problem = match cmd.arg("--help").output() {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(NOT_FOUND.to_string()),
            Err(e) => Some(format!("{exe}: {e}")),
            // `--model` is what tells the synthesizer from the mouse tool, which also answers
            // `--help` and also mentions its own name in the output.
            Ok(out) => {
                let text = String::from_utf8_lossy(&out.stdout).to_lowercase()
                    + &String::from_utf8_lossy(&out.stderr).to_lowercase();
                (!text.contains("--model")).then(|| {
                    format!("`{exe}` is not the text-to-speech Piper — it takes no `--model`.")
                })
            }
        };
        set_piper_problem(problem);
    });
    None
}

/// How loud a spoken warning is. Piper's own output has no volume of its own, and speech that is
/// louder than the tone that introduced it is a jump-scare.
#[cfg(not(target_arch = "wasm32"))]
static VOLUME: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(1.0_f32.to_bits());

/// Tell the speech path how loud to be. Takes the same slider the alert tones use.
#[cfg(not(target_arch = "wasm32"))]
pub fn set_volume(v: f32) {
    VOLUME.store(v.clamp(0.0, 1.0).to_bits(), std::sync::atomic::Ordering::Relaxed);
}

/// The browser mixes speech itself and `SpeechSynthesisUtterance` carries its own volume, so
/// there is nothing here to set. A stub rather than a `cfg` at every call site.
#[cfg(target_arch = "wasm32")]
pub fn set_volume(_v: f32) {}

/// Only the desktop arm plays its own audio; Android hands the words to TextToSpeech, which owns
/// the level itself.
#[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
fn speech_volume() -> f32 {
    f32::from_bits(VOLUME.load(std::sync::atomic::Ordering::Relaxed))
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
        if voice.is_empty() {
            return Ok(false);
        }
        if !std::path::Path::new(&voice).exists() {
            // Configured and gone: a cleaned cache, a moved data directory. Without this the
            // fallback voice takes over and nothing says why the good one stopped.
            super::set_piper_problem(Some(format!("voice model not found: {voice}")));
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
            // Not installed is the common case, and it is not a failure worth a warning — but it
            // is worth saying so in Settings, where the user asked for this voice.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                super::set_piper_problem(Some(super::NOT_FOUND.to_string()));
                return Ok(false);
            }
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
            // The likeliest cause on Arch is the wrong `piper`: `extra/piper` is a mouse
            // configuration tool that installs the same binary name.
            super::set_piper_problem(Some(format!(
                "{bin} exited {} with no audio — is this the text-to-speech piper?",
                out.status
            )));
            return Err(format!("piper exited {}", out.status));
        }
        super::set_piper_problem(None);
        // Blocking: this call is already inside `announce`'s one thread, and the next line of the
        // warning must not start until this one has been heard.
        crate::audio::play_wav_blocking(out.stdout, super::speech_volume());
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

    fn job(priority: Priority, text: &str) -> SpeechJob {
        SpeechJob::new(priority, None, vec![text.to_string()])
    }

    #[test]
    fn an_emergency_discards_pending_lower_priority_speech() {
        let mut queue = std::collections::VecDeque::new();
        enqueue(&mut queue, job(Priority::Background, "chase update"));
        enqueue(&mut queue, job(Priority::Warning, "ordinary warning"));
        enqueue(&mut queue, job(Priority::Emergency, "tornado emergency"));

        assert_eq!(queue.len(), 1);
        assert_eq!(queue.front().unwrap().priority, Priority::Emergency);
        assert!(emergency_waiting(&queue));
        assert!(should_preempt(Priority::Warning, &queue));
        assert!(should_preempt(Priority::Background, &queue));
        assert!(!should_preempt(Priority::Emergency, &queue));
    }

    #[test]
    fn ordinary_speech_stays_fifo_and_scripts_split_at_sentences() {
        let mut queue = std::collections::VecDeque::new();
        enqueue(&mut queue, job(Priority::Background, "first"));
        enqueue(
            &mut queue,
            job(
                Priority::Warning,
                "Hail to 1.75 inches. Take cover now! Stay there?",
            ),
        );

        assert_eq!(queue.pop_front().unwrap().priority, Priority::Background);
        assert_eq!(
            queue.pop_front().unwrap().lines,
            ["Hail to 1.75 inches.", "Take cover now!", "Stay there?"]
        );
    }

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
