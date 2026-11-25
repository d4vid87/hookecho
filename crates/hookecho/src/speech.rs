//! Speaking warnings out loud.
//!
//! Two layers. The words come from [`wxdata::spoken`] — hazard, place and heading first, NWS
//! shorthand expanded. Piper is the only voice engine; failure stays visible instead of silently
//! falling back to a robotic platform synthesizer.
//!
//! Chasing is an eyes-on-the-road activity: a warning you have to read is a warning you read at
//! the wrong moment. Every call is fire-and-forget on one background worker and every failure is
//! logged and dropped — a machine with no speech engine is a normal machine, not a broken one.

static STATUS: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
static CANCEL_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn stop() {
    stop_queued(true);
}

/// Interrupt speech without muting alert chimes.
pub fn stop_voice() {
    stop_queued(false);
}

fn stop_queued(include_tones: bool) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut state = speech_queue().state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        discard_queued(&mut state.jobs, include_tones);
        if include_tones { crate::audio::stop_tones(); }
        // The worker captures its tickets while taking the next job under this same lock.
        cancel_voice();
    }
    #[cfg(target_arch = "wasm32")]
    {
        WEB_QUEUE.with(|queue| discard_queued(&mut queue.borrow_mut().jobs, include_tones));
        cancel_voice();
    }
}

fn cancel_voice() {
    CANCEL_EPOCH.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
    imp::stop();
    crate::platform::stop_speech();
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsCast;
        if let Ok(stop) = js_sys::Reflect::get(&js_sys::global(), &"__hookechoAmyStop".into()) {
            if let Some(stop) = stop.dyn_ref::<js_sys::Function>() {
                let _ = stop.call0(&js_sys::global());
            }
        }
    }
}

fn voice_epoch() -> u64 {
    CANCEL_EPOCH.load(std::sync::atomic::Ordering::Acquire)
}

pub fn status() -> String {
    #[cfg(target_arch = "wasm32")]
    if let Some(value) = js_sys::Reflect::get(&js_sys::global(), &"__hookechoAmyStatus".into())
        .ok()
        .and_then(|value| value.as_string())
    {
        return value;
    }
    STATUS.lock().map(|status| status.clone()).unwrap_or_default()
}

#[cfg(target_arch = "wasm32")]
pub fn retry() {
    use wasm_bindgen::JsCast;
    if let Ok(retry) = js_sys::Reflect::get(&js_sys::global(), &"__hookechoAmyRetry".into()) {
        if let Some(retry) = retry.dyn_ref::<js_sys::Function>() {
            let _ = retry.call0(&js_sys::global());
        }
    }
}

fn set_status(message: impl Into<String>) {
    if let Ok(mut status) = STATUS.lock() {
        *status = message.into();
    }
}

/// Browsers require an explicit interaction before automatic audio. Native apps use their
/// existing persisted speech preference; browser activation lasts for this page session.
pub fn enable() {
    set_status("");
    #[cfg(target_arch = "wasm32")]
    {
        WEB_ENABLED.with(|enabled| enabled.set(true));
        // Defer the voice model on every browser until speech is requested.
        if js_sys::Reflect::get(&js_sys::global(), &"__hookechoAmyStatus".into())
            .ok()
            .and_then(|value| value.as_string())
            .is_none()
        {
            retry();
        }
    }
}

pub fn automatic_ready() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        WEB_ENABLED.with(|enabled| enabled.get())
            && js_sys::Reflect::get(&js_sys::global(), &"__hookechoAmyStatus".into())
                .ok().and_then(|value| value.as_string()).as_deref() == Some("Amy — ready")
    }
    #[cfg(not(target_arch = "wasm32"))]
    { true }
}

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
    let custom = !voice.trim().is_empty();
    let (bin, voice) = if voice.trim().is_empty() {
        bundled_piper().unwrap_or_else(|| (bin.trim().to_string(), String::new()))
    } else {
        (bin.trim().to_string(), voice.trim().to_string())
    };
    if let Ok(mut g) = PIPER.write() {
        set_status(if voice.is_empty() {
            "Piper unavailable"
        } else if custom {
            "Custom Piper voice — ready"
        } else {
            "Amy — ready"
        });
        *g = (bin, voice);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn bundled_piper() -> Option<(String, String)> {
    let mut roots = Vec::new();
    if let Some(dir) = std::env::var_os("HOOKECHO_PIPER_DIR") {
        roots.push(dir.into());
    }
    let exe = std::env::current_exe().ok()?;
    if let Some(dir) = exe.parent() {
        roots.push(dir.join("piper"));
        roots.push(dir.join("../Resources/piper"));
        roots.push(dir.join("../lib/hookecho/piper"));
    }
    roots.into_iter().find_map(|root: std::path::PathBuf| {
        let bin = root.join(if cfg!(target_os = "windows") {
            "piper.exe"
        } else {
            "piper"
        });
        let voice = root.join(format!("{DEFAULT_VOICE}.onnx"));
        (bin.is_file() && voice.is_file()).then(|| {
            (
                bin.to_string_lossy().into_owned(),
                voice.to_string_lossy().into_owned(),
            )
        })
    })
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
pub const DEFAULT_VOICE: &str = "en_US-amy-medium";

/// The voices the picker offers, by Piper id (`{lang}_{REGION}-{name}-{quality}`).
///
/// ponytail: a curated handful, not the whole repository. Piper publishes hundreds across dozens
/// of languages, and listing them means shipping (and refreshing) an index of a repository that
/// is not ours. Anything not here is still reachable — the voice field takes a path to any
/// `.onnx` the user downloaded themselves.
#[cfg(not(target_arch = "wasm32"))]
pub const VOICES: &[&str] = &[
    "en_US-amy-medium",
    "en_US-lessac-medium",
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
pub const AMY_SHA256: &str = "b3a6e47b57b8c7fbe6a0ce2518161a50f59a9cdd8a50835c02cb02bdd6206c18";

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

fn discard_queued(queue: &mut std::collections::VecDeque<SpeechJob>, include_tones: bool) {
    if include_tones {
        queue.clear();
    } else {
        queue.retain_mut(|job| {
            job.lines.clear();
            job.tone.is_some()
        });
    }
}

fn emergency_waiting(queue: &std::collections::VecDeque<SpeechJob>) -> bool {
    queue.iter().any(|job| job.priority == Priority::Emergency)
}

fn should_preempt(current: Priority, queue: &std::collections::VecDeque<SpeechJob>) -> bool {
    current != Priority::Emergency && emergency_waiting(queue)
}

fn interrupts_voice(current: Option<Priority>, next: Priority) -> bool {
    next == Priority::Emergency && current != Some(Priority::Emergency)
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
struct NativeQueue {
    jobs: std::collections::VecDeque<SpeechJob>,
    priority: Option<Priority>,
}

#[cfg(not(target_arch = "wasm32"))]
struct SpeechQueue {
    state: std::sync::Mutex<NativeQueue>,
    ready: std::sync::Condvar,
}

#[cfg(not(target_arch = "wasm32"))]
fn speech_queue() -> &'static std::sync::Arc<SpeechQueue> {
    static QUEUE: std::sync::OnceLock<std::sync::Arc<SpeechQueue>> = std::sync::OnceLock::new();
    QUEUE.get_or_init(|| {
        let queue = std::sync::Arc::new(SpeechQueue {
            state: std::sync::Mutex::new(NativeQueue::default()),
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
        let mut state = queue
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.priority = None;
        while state.jobs.is_empty() {
            state = queue
                .ready
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        let job = state.jobs.pop_front().expect("queue was not empty");
        state.priority = Some(job.priority);
        let epoch = voice_epoch();
        let tone_epoch = crate::audio::tone_epoch();
        drop(state);

        if let Some((sound, volume)) = job.tone {
            crate::audio::play_blocking(&sound, volume, tone_epoch);
        }
        for line in job.lines {
            if voice_epoch() != epoch { break; }
            let state = queue
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let preempt = should_preempt(job.priority, &state.jobs);
            drop(state);
            if preempt {
                break;
            }
            if let Err(e) = imp::speak_blocking(&line, epoch) {
                if voice_epoch() == epoch {
                    log::warn!("speech failed: {e}");
                    set_status(format!("Speech unavailable: {e}. Check your system voice and try the test warning."));
                }
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
    let mut state = queue
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if interrupts_voice(state.priority, priority) { cancel_voice(); }
    enqueue(&mut state.jobs, job);
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
    priority: Option<Priority>,
}

#[cfg(target_arch = "wasm32")]
std::thread_local! {
    static WEB_QUEUE: std::cell::RefCell<WebQueue> = std::cell::RefCell::new(WebQueue::default());
}

/// The same single FIFO on the browser's one thread. Only one utterance is handed to
/// One Piper utterance at a time; its speaking state is the completion signal and the next
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
        if interrupts_voice(queue.priority, priority) {
            cancel_voice();
        }
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
        let Some(job) = WEB_QUEUE.with(|queue| {
            let mut queue = queue.borrow_mut();
            let job = queue.jobs.pop_front();
            queue.priority = job.as_ref().map(|job| job.priority);
            job
        }) else {
            WEB_QUEUE.with(|queue| queue.borrow_mut().running = false);
            return;
        };
        let epoch = voice_epoch();
        if let Some((sound, volume)) = &job.tone {
            crate::audio::play(sound, *volume);
            crate::fonts::sleep_ms(crate::audio::duration_ms(sound)).await;
        }
        for line in job.lines {
            if voice_epoch() != epoch { break; }
            let preempt =
                WEB_QUEUE.with(|queue| should_preempt(job.priority, &queue.borrow().jobs));
            if preempt {
                break;
            }
            if let Err(e) = imp::speak_blocking(&line) {
                log::warn!("speech failed: {e}");
                set_status(format!("Piper unavailable: {e}. Retry the voice, then test again."));
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
                set_status("Speech timed out. Check browser audio and try the test warning.");
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use wasm_bindgen::JsCast;

    pub fn speak_blocking(text: &str) -> Result<(), String> {
        if let Ok(speak) = js_sys::Reflect::get(&js_sys::global(), &"__hookechoAmySpeak".into()) {
            if let Some(speak) = speak.dyn_ref::<js_sys::Function>() {
                let volume = super::WEB_VOLUME.with(std::cell::Cell::get);
                speak.call2(&js_sys::global(), &text.into(), &volume.into())
                    .map_err(|_| "Amy invocation failed".to_string())?;
                return Ok(());
            }
        }
        Err("Amy is not ready".into())
    }

    pub fn is_speaking() -> bool {
        js_sys::Reflect::get(&js_sys::global(), &"__hookechoAmySpeaking".into())
            .ok().and_then(|value| value.as_bool()).unwrap_or(false)
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

/// Apply the shared alert volume to each subsequently created browser utterance.
#[cfg(target_arch = "wasm32")]
pub fn set_volume(v: f32) {
    WEB_VOLUME.with(|volume| volume.set(v.clamp(0.0, 1.0)));
}

#[cfg(target_arch = "wasm32")]
std::thread_local! {
    static WEB_VOLUME: std::cell::Cell<f32> = const { std::cell::Cell::new(1.0) };
    static WEB_ENABLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Only the desktop arm plays its own audio; Android's Piper bridge owns the level itself.
#[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
fn speech_volume() -> f32 {
    f32::from_bits(VOLUME.load(std::sync::atomic::Ordering::Relaxed))
}

#[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
mod imp {
    use std::io::{Read, Write};
    use std::process::{Child, Command, Stdio};
    use std::sync::{Mutex, PoisonError};

    static PIPER_CHILD: Mutex<Option<Child>> = Mutex::new(None);

    pub(super) fn stop() {
        if let Some(child) = PIPER_CHILD.lock().unwrap_or_else(PoisonError::into_inner).as_mut() {
            let _ = child.kill();
        }
    }

    /// Piper is the only speech engine: a failed neural voice must be visible, never silently
    /// replaced by a robotic device voice.
    pub fn speak_blocking(text: &str, epoch: u64) -> Result<(), String> {
        match piper(text, epoch) {
            Ok(true) => Ok(()),
            Ok(false) => Err("Piper voice is not configured".into()),
            Err(e) => Err(e),
        }
    }

    /// Synthesize through Piper and play the result. `Ok(false)` means Piper is not configured,
    /// which is not an error — it is the default state of every machine.
    ///
    // ponytail: subprocess, not in-process ONNX. `ort` would put a C++ runtime and a licence
    // question into a build that currently cross-compiles to four targets without one; the
    // process boundary costs a fork per warning, which is nothing against a network fetch.
    fn piper(text: &str, epoch: u64) -> Result<bool, String> {
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
        cmd.args(["--model", &voice, "--output_file", "-"])
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        let cancelled = || super::voice_epoch() != epoch;
        let (mut stdin, mut stdout) = {
            let mut active = PIPER_CHILD.lock().unwrap_or_else(PoisonError::into_inner);
            // Stop either invalidates this launch or sees the registered child and kills it.
            if cancelled() { return Ok(true); }
            let mut child = match cmd.spawn() {
                Ok(child) => child,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    super::set_piper_problem(Some(super::NOT_FOUND.to_string()));
                    return Ok(false);
                }
                Err(e) => return Err(format!("spawn {bin}: {e}")),
            };
            let pipes = (child.stdin.take().expect("piped stdin"), child.stdout.take().expect("piped stdout"));
            *active = Some(child);
            pipes
        };
        let input = stdin.write_all(text.as_bytes());
        drop(stdin);
        let mut wav = Vec::new();
        let output = input.and_then(|()| stdout.read_to_end(&mut wav));
        let status = loop {
            let mut active = PIPER_CHILD.lock().unwrap_or_else(PoisonError::into_inner);
            let child = active.as_mut().expect("speech worker owns its registered child");
            if output.is_err() {
                let _ = child.kill();
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    *active = None;
                    break Ok(status);
                }
                Ok(None) => {
                    // A process can close stdout and stay alive. Leave Stop free to kill it.
                    drop(active);
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
                Err(error) => {
                    let _ = child.kill();
                    *active = None;
                    break Err(error);
                }
            }
        };
        if cancelled() { return Ok(true); }
        output.map_err(|e| e.to_string())?;
        let status = status.map_err(|e| e.to_string())?;
        if !status.success() || wav.is_empty() {
            // The likeliest cause on Arch is the wrong `piper`: `extra/piper` is a mouse
            // configuration tool that installs the same binary name.
            super::set_piper_problem(Some(format!(
                "{bin} exited {} with no audio — is this the text-to-speech piper?",
                status
            )));
            return Err(format!("piper exited {status}"));
        }
        super::set_piper_problem(None);
        // Blocking: this call is already inside `announce`'s one thread, and the next line of the
        // warning must not start until this one has been heard.
        crate::audio::play_wav_blocking(wav, super::speech_volume(), cancelled);
        Ok(true)
    }
}

#[cfg(target_os = "android")]
mod imp {
    /// Android's bundled Piper bridge lives in `platform`.
    pub fn speak_blocking(text: &str, epoch: u64) -> Result<(), String> {
        if super::voice_epoch() != epoch { return Ok(()); }
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
        assert!(interrupts_voice(Some(Priority::Warning), Priority::Emergency));
        assert!(!interrupts_voice(Some(Priority::Emergency), Priority::Emergency));
    }

    #[test]
    fn stop_voice_preserves_queued_alert_tones() {
        let mut queue = std::collections::VecDeque::from([
            job(Priority::Warning, "voice only"),
            SpeechJob::new(Priority::Warning, Some((crate::settings::AlertSound::Chime, 0.5)), vec!["with a tone".into()]),
        ]);
        discard_queued(&mut queue, false);
        assert_eq!(queue.len(), 1);
        assert!(queue[0].lines.is_empty());
        assert!(queue[0].tone.is_some());
        discard_queued(&mut queue, true);
        assert!(queue.is_empty());
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
