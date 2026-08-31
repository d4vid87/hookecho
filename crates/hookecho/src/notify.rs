//! Alert payload formatting for the chat webhooks (Discord, Slack, Matrix).
//!
//! Pure string building only — the HTTP fan-out lives in `App::notify_alert`. Keeping the bodies
//! here makes them unit-testable without a runtime, and keeps the secrets (URLs, tokens) in the
//! caller where they came out of settings.

/// JSON-escape a string body (the only escaping these tiny payloads need).
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Discord incoming webhook: bold title, body on the next line.
pub fn discord_body(title: &str, body: &str) -> String {
    format!("{{\"content\":\"**{}**\\n{}\"}}", esc(title), esc(body))
}

/// Slack incoming webhook: same text, no markup beyond Slack's own bold.
pub fn slack_body(title: &str, body: &str) -> String {
    format!("{{\"text\":\"*{}*\\n{}\"}}", esc(title), esc(body))
}

/// Matrix `m.room.message` event content.
pub fn matrix_body(title: &str, body: &str) -> String {
    format!(
        "{{\"msgtype\":\"m.text\",\"body\":\"{}\\n{}\"}}",
        esc(title),
        esc(body)
    )
}

/// Percent-encode a path segment (room ids carry `!`, `:` and `#`).
fn pct(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// `PUT` URL for a Matrix message send, with `txn` as the transaction id (unix millis).
pub fn matrix_url(homeserver: &str, room: &str, txn: u128) -> String {
    format!(
        "{}/_matrix/client/v3/rooms/{}/send/m.room.message/{}",
        homeserver.trim_end_matches('/'),
        pct(room),
        txn
    )
}

/// Post a notification to the OS notification centre (desktop only).
///
/// The in-app banner only exists while the window is on screen; this is the one that reaches
/// someone who alt-tabbed away, and it is what the tray-based background alerting was missing.
/// Best effort — a desktop with no notification daemon logs and carries on.
#[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
pub fn desktop(title: &str, body: &str) {
    use notify_rust::Notification;
    let res = Notification::new()
        .summary(title)
        .body(body)
        .appname("HookEcho")
        .show();
    if let Err(e) = res {
        log::warn!("desktop notification failed: {e}");
    }
}

/// No-op on Android — the alert service posts its own, through the system channel.
#[cfg(target_os = "android")]
pub fn desktop(_title: &str, _body: &str) {}

/// Post a browser notification, so an alert reaches someone whose tab is behind something else.
///
/// Best effort in the same way the desktop one is: no permission, or a browser that refuses,
/// logs and carries on. The in-app banner and the alert list never depended on this.
#[cfg(target_arch = "wasm32")]
pub fn desktop(title: &str, body: &str) {
    use web_sys::NotificationPermission as P;
    match web_sys::Notification::permission() {
        P::Granted => {
            let opts = web_sys::NotificationOptions::new();
            opts.set_body(body);
            opts.set_icon("/icon-192.png");
            // One notification per event, replacing rather than stacking: an outbreak should not
            // leave forty cards in the tray.
            opts.set_tag("hookecho-alert");
            if let Err(e) = web_sys::Notification::new_with_options(title, &opts) {
                log::warn!("browser notification failed: {e:?}");
            }
        }
        // Never prompt from here. An alert firing is not a moment to interrupt someone with a
        // permission dialog, and the ask already happened when they turned the setting on.
        _ => log::debug!("browser notification skipped: no permission"),
    }
}

/// Ask the browser for notification permission, if it has not already been answered.
///
/// Called from the one place that makes it contextual — the moment the user turns alert
/// notifications on — never at load. A denial is final and is not asked about again; the browser
/// enforces that anyway.
#[cfg(target_arch = "wasm32")]
pub fn ask_permission() {
    if web_sys::Notification::permission() != web_sys::NotificationPermission::Default {
        return;
    }
    match web_sys::Notification::request_permission() {
        Ok(p) => wasm_bindgen_futures::spawn_local(async move {
            match wasm_bindgen_futures::JsFuture::from(p).await {
                Ok(v) => log::info!("notification permission: {v:?}"),
                Err(e) => log::warn!("notification permission request failed: {e:?}"),
            }
        }),
        Err(e) => log::warn!("cannot ask for notification permission: {e:?}"),
    }
}

/// Nothing to ask for: desktop notifications need no permission, and Android's are the service's.
#[cfg(not(target_arch = "wasm32"))]
pub fn ask_permission() {}

/// Waits before each retry. A webhook that fails is usually failing for seconds (a Discord blip,
/// a laptop's wifi coming back after a lid open), so the first retry is quick and the last one is
/// far enough out to outlast a short outage.
const BACKOFF_SECS: [u64; 3] = [2, 8, 30];

/// How many alert deliveries may be sitting in backoff at once. An outbreak plus a dead webhook
/// should not accumulate tasks without bound.
const MAX_RETRYING: usize = 32;

static RETRYING: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Is this worth trying again? A transport error or a 5xx/429 is; a 404 on a deleted webhook or a
/// 401 on a stale token will fail identically forever, and retrying it only delays the log line.
fn worth_retrying(res: &Result<reqwest::Response, reqwest::Error>) -> bool {
    match res {
        Err(_) => true,
        Ok(r) => {
            r.status().is_server_error() || r.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
        }
    }
}

/// Send a notification request, retrying a transient failure up to three times with backoff.
///
/// `make` builds the request fresh each attempt — `RequestBuilder` is consumed by `send`, and a
/// body may not be cloneable. The queue is the spawned tasks themselves, bounded by a counter.
///
// ponytail: in memory, so a crash mid-backoff loses the delivery; persist the queue if anyone
// reports losing alerts to a restart rather than to a dead webhook.
pub async fn send_retrying(what: &str, make: impl Fn() -> reqwest::RequestBuilder) {
    use std::sync::atomic::Ordering;
    let first = make().send().await;
    if log_outcome(what, "delivery", &first) {
        return;
    }
    if !worth_retrying(&first) {
        return;
    }
    // Taking a retry slot is what the cap counts; the first attempt is always free.
    if RETRYING.fetch_add(1, Ordering::Relaxed) >= MAX_RETRYING {
        RETRYING.fetch_sub(1, Ordering::Relaxed);
        log::warn!("{what}: retry queue full, dropping");
        return;
    }
    struct Slot;
    impl Drop for Slot {
        fn drop(&mut self) {
            std::sync::atomic::AtomicUsize::fetch_sub(&RETRYING, 1, Ordering::Relaxed);
        }
    }
    let _slot = Slot;
    for secs in BACKOFF_SECS {
        sleep_secs(secs).await;
        let res = make().send().await;
        if log_outcome(what, "retry", &res) {
            log::info!("{what} delivery succeeded on retry");
            return;
        }
        if !worth_retrying(&res) {
            return;
        }
    }
    log::warn!("{what}: giving up after {} retries", BACKOFF_SECS.len());
}

/// Log what happened; `true` means it landed and there is nothing more to do.
fn log_outcome(what: &str, stage: &str, res: &Result<reqwest::Response, reqwest::Error>) -> bool {
    match res {
        Ok(r) if r.status().is_success() => true,
        Ok(r) => {
            log::warn!("{what} {stage} returned {}", r.status());
            false
        }
        Err(e) => {
            log::warn!("{what} {stage} failed: {e}");
            false
        }
    }
}

/// Both targets have a timer: tokio natively, `setTimeout` in the browser. `wxdata::task::sleep`
/// is already the thing that knows which.
async fn sleep_secs(secs: u64) {
    wxdata::task::sleep(std::time::Duration::from_secs(secs)).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tiny HTTP server answering `codes` in order (then repeating the last), counting requests.
    #[cfg(not(target_arch = "wasm32"))]
    fn mock_sink(codes: Vec<u16>) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        use std::io::{Read, Write};
        use std::sync::atomic::Ordering;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/hook", listener.local_addr().unwrap());
        let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let n = hits.clone();
        std::thread::spawn(move || {
            for sock in listener.incoming() {
                let Ok(mut sock) = sock else { break };
                let i = n.fetch_add(1, Ordering::SeqCst);
                let code = codes[i.min(codes.len() - 1)];
                let mut buf = [0u8; 2048];
                let _ = sock.read(&mut buf);
                let _ = sock.write_all(
                    format!("HTTP/1.1 {code} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                        .as_bytes(),
                );
            }
        });
        (url, hits)
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn a_transient_failure_is_retried_and_a_permanent_one_is_not() {
        use std::sync::atomic::Ordering;
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let http = reqwest::Client::new();

        // 503 then 200: the second attempt lands, so the sink sees exactly two requests.
        let (url, hits) = mock_sink(vec![503, 200]);
        rt.block_on(send_retrying("test", || http.post(&url).body("x")));
        assert_eq!(hits.load(Ordering::SeqCst), 2, "a 503 should be retried");

        // 404 is a dead webhook — retrying it forever only delays the log line. `send_retrying`
        // returns without ever sleeping, which is what makes this assertion immediate.
        let (url, hits) = mock_sink(vec![404]);
        rt.block_on(send_retrying("test", || http.post(&url).body("x")));
        assert_eq!(
            hits.load(Ordering::SeqCst),
            1,
            "a 404 should not be retried"
        );
    }

    #[test]
    fn backoff_grows_and_is_bounded() {
        assert_eq!(BACKOFF_SECS, [2, 8, 30]);
        assert!(BACKOFF_SECS.windows(2).all(|w| w[1] > w[0]));
    }

    #[test]
    fn payloads_are_valid_json_and_escape() {
        let t = "Tornado \"Warning\"";
        let b = "line1\nline2\\end";
        for s in [discord_body(t, b), slack_body(t, b), matrix_body(t, b)] {
            let v: serde_json::Value = serde_json::from_str(&s).expect("valid json");
            let text = v["content"]
                .as_str()
                .or_else(|| v["text"].as_str())
                .or_else(|| v["body"].as_str())
                .unwrap();
            assert!(text.contains("Tornado \"Warning\""));
            assert!(text.contains("line1\nline2\\end"));
        }
    }

    #[test]
    fn matrix_url_encodes_room_and_trims_slash() {
        let u = matrix_url("https://matrix.org/", "!abc:matrix.org", 1234);
        assert_eq!(
            u,
            "https://matrix.org/_matrix/client/v3/rooms/%21abc%3Amatrix.org/send/m.room.message/1234"
        );
    }
}
