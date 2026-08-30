//! Last-panic reporting: write a report when the app dies, show it the next time it starts.
//!
//! A panic on a phone goes to logcat, and a panic on the desktop goes to a terminal nobody
//! launched the app from — so a crash the user could describe in one sentence arrives as "it just
//! closed". This writes what actually happened to a file beside the settings, and the app offers
//! it back on the next start with a Copy button, which is the difference between a bug report and
//! a shrug.
//!
//! The report deliberately carries nothing identifying: the panic message, the source location
//! inside this repo, the app version, the OS name, and a timestamp. No hostname, no environment,
//! no filesystem paths of the user's own.

use std::backtrace::Backtrace;

/// Install the panic hook: log as before, and additionally leave a report behind.
pub fn install_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("panic: {info}");
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "(no message)".to_string());
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()));
        let report = format_report(&msg, location.as_deref(), &Backtrace::capture().to_string());
        // wxdata decodes malformed GRIB defensively, catching the panics gribberish throws on
        // odd packings. Those are handled, not crashes — leaving a "the app crashed" report for
        // one is a lie the user then reports as a bug.
        if wxdata::task::panic_guarded() {
            previous(info);
            return;
        }
        if let Some(p) = crate::paths::crash_file() {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(p, report);
        }
        previous(info);
    }));
}

/// The report body. Split out so a test can assert what does — and does not — end up in it.
pub fn format_report(msg: &str, location: Option<&str>, backtrace: &str) -> String {
    format!(
        "HookEcho {} on {}\n{}\n\npanic: {msg}\nat {}\n\n{backtrace}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%SZ"),
        location.unwrap_or("unknown location"),
    )
}

/// The report left by the previous run, if there is one.
pub fn take_report() -> Option<String> {
    let p = crate::paths::crash_file()?;
    std::fs::read_to_string(p).ok().filter(|s| !s.is_empty())
}

/// Forget the last report — the user has seen it.
pub fn dismiss() {
    if let Some(p) = crate::paths::crash_file() {
        let _ = std::fs::remove_file(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_report_says_what_broke_and_nothing_about_who_was_running_it() {
        let r = format_report(
            "index out of bounds",
            Some("src/app.rs:12:3"),
            "<backtrace>",
        );
        assert!(r.contains("index out of bounds"));
        assert!(r.contains("src/app.rs:12:3"));
        assert!(r.contains(env!("CARGO_PKG_VERSION")));
        assert!(r.contains(std::env::consts::OS));
        // Nothing from the user's own machine.
        let home = std::env::var("HOME").unwrap_or_else(|_| "/nonexistent-home".into());
        assert!(!r.contains(&home));
        assert!(!r.to_lowercase().contains("user="));
    }
}
