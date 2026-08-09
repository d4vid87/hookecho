//! Speaking warnings out loud.
//!
//! Chasing is an eyes-on-the-road activity: a warning you have to read is a warning you read at
//! the wrong moment. Every call is fire-and-forget on a background thread and every failure is
//! logged and dropped — a machine with no speech engine is a normal machine, not a broken one.

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
}

#[cfg(target_os = "android")]
mod imp {
    /// Android's TextToSpeech is JNI, and all JNI lives in `platform`.
    pub fn speak_blocking(text: &str) -> Result<(), String> {
        crate::platform::speak(text)
    }
}
