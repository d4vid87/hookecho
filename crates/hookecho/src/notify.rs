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

#[cfg(test)]
mod tests {
    use super::*;

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
        .appname("Hook Echo-WX")
        .show();
    if let Err(e) = res {
        log::warn!("desktop notification failed: {e}");
    }
}

/// No-op on Android (the alert service posts its own) and on the web.
#[cfg(any(target_os = "android", target_arch = "wasm32"))]
pub fn desktop(_title: &str, _body: &str) {}
