//! Sign in with Google, and keep `settings.json` the same on every machine.
//!
//! The data lives in **your own Drive**, in the hidden per-app folder (`appDataFolder`) that only
//! this app and you can see — there is no Hook Echo server, no account, and nothing to pay for.
//! Sign-in is the OAuth 2.0 **loopback redirect with PKCE**: the app listens on a random port on
//! 127.0.0.1, opens your browser at Google, and Google redirects back to that port with the code.
//! (The device flow would be less machinery, but Google does not allow Drive scopes on it — it
//! answers `invalid device flow scope` — so loopback it is. It works on Android too: the phone's
//! browser reaches the phone's own localhost.)
//!
//! You supply the OAuth client (see `docs/sync.md`) — shipping a shared client id in an
//! open-source binary would put every user's quota, and Google's trust in it, in one basket.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const FILES_URL: &str = "https://www.googleapis.com/drive/v3/files";
const UPLOAD_URL: &str = "https://www.googleapis.com/upload/drive/v3/files";
/// Only the app's own hidden folder — this grant cannot read the rest of your Drive.
const SCOPE: &str = "https://www.googleapis.com/auth/drive.appdata";
/// The synced blob's name inside `appDataFolder`.
const REMOTE_NAME: &str = "settings.json";

/// Settings fields that belong to *this* machine and must survive a sync from another one.
/// Everything else — markers, placefiles, palettes, theme, API keys — is shared.
pub const DEVICE_LOCAL: [&str; 4] = ["ui_scale", "share_name", "background_alerts", "share_relay"];

/// What Google gave us. Stored beside settings.json but never inside it: these are credentials,
/// and the whole point of the settings blob is that it travels.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Tokens {
    pub refresh_token: String,
    #[serde(default)]
    pub access_token: String,
    /// Unix seconds. Refresh once we're within a minute of it.
    #[serde(default)]
    pub expires_at: i64,
}

impl Tokens {
    pub fn path() -> Option<PathBuf> {
        crate::paths::config_dir().map(|d| d.join("google-tokens.json"))
    }

    pub fn load() -> Option<Self> {
        let s = std::fs::read_to_string(Self::path()?).ok()?;
        let t: Self = serde_json::from_str(&s).ok()?;
        (!t.refresh_token.is_empty()).then_some(t)
    }

    pub fn save(&self) {
        let Some(path) = Self::path() else { return };
        if let Some(p) = path.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        match serde_json::to_string(self) {
            Ok(j) => {
                if let Err(e) = crate::settings::atomic_write(&path, j.as_bytes()) {
                    log::warn!("token save failed: {e}");
                }
            }
            Err(e) => log::warn!("token serialize failed: {e}"),
        }
    }

    pub fn forget() {
        if let Some(p) = Self::path() {
            let _ = std::fs::remove_file(p);
        }
    }

    fn fresh(&self) -> bool {
        !self.access_token.is_empty() && self.expires_at - 60 > crate::share::now()
    }
}

/// A sign-in in flight: the URL to visit, the PKCE verifier the token exchange needs, the
/// redirect the listener is serving, and the channel the listener reports the code on.
pub struct Pending {
    pub url: String,
    pub verifier: String,
    pub redirect: String,
    pub rx: std::sync::mpsc::Receiver<Result<String, String>>,
}

/// Bind a loopback port, and build the URL that sends the browser to Google. The returned
/// listener thread serves exactly one request — the redirect — and then ends.
pub fn start_login(client_id: &str) -> Result<Pending, String> {
    let listener =
        std::net::TcpListener::bind(("127.0.0.1", 0)).map_err(|e| format!("no local port: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect = format!("http://127.0.0.1:{port}");
    let verifier = pkce_verifier();
    let url = format!(
        "{AUTH_URL}?client_id={}&redirect_uri={}&response_type=code&scope={}\
         &code_challenge={}&code_challenge_method=S256&access_type=offline&prompt=consent",
        urlencode(client_id),
        urlencode(&redirect),
        urlencode(SCOPE),
        pkce_challenge(&verifier),
    );
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(serve_redirect(&listener));
    });
    Ok(Pending {
        url,
        verifier,
        redirect,
        rx,
    })
}

/// Accept the browser's one request and pull `code` (or `error`) out of its query string.
fn serve_redirect(listener: &std::net::TcpListener) -> Result<String, String> {
    use std::io::{BufRead, BufReader, Write};
    let (mut stream, _) = listener.accept().map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(stream.try_clone().map_err(|e| e.to_string())?)
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    // "GET /?code=…&scope=… HTTP/1.1"
    let query = line
        .split_whitespace()
        .nth(1)
        .and_then(|p| p.split_once('?').map(|(_, q)| q.to_string()))
        .unwrap_or_default();
    let result = match param(&query, "code") {
        Some(code) => Ok(code),
        None => Err(param(&query, "error").unwrap_or_else(|| "no code in redirect".into())),
    };
    let body = match &result {
        Ok(_) => "<h2>Signed in.</h2><p>You can close this tab and go back to Hook Echo-WX.</p>",
        Err(_) => "<h2>Sign-in failed.</h2><p>Go back to Hook Echo-WX for the reason.</p>",
    };
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    result
}

/// One `a=b` value out of a query string, percent-decoded. Google's auth codes contain a `/`,
/// which arrives as `%2F` — handing that to the token endpoint verbatim earns an `invalid_grant`.
fn param(query: &str, key: &str) -> Option<String> {
    let raw = query.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == key).then_some(v)
    })?;
    Some(percent_decode(&raw.replace('+', " ")))
}

fn percent_decode(s: &str) -> String {
    let mut out = Vec::with_capacity(s.len());
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match (b[i], b.get(i + 1), b.get(i + 2)) {
            (b'%', Some(h), Some(l)) => {
                match u8::from_str_radix(&format!("{}{}", *h as char, *l as char), 16) {
                    Ok(byte) => {
                        out.push(byte);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(b[i]);
                        i += 1;
                    }
                }
            }
            _ => {
                out.push(b[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Swap the code for tokens. The verifier is what proves this is the same app that started the
/// flow, which is the whole point of PKCE on a public client.
pub async fn exchange(
    client_id: &str,
    client_secret: &str,
    code: &str,
    verifier: &str,
    redirect: &str,
) -> Result<Tokens, String> {
    let r = client()
        .post(TOKEN_URL)
        .form(&[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("code", code),
            ("code_verifier", verifier),
            ("redirect_uri", redirect),
            ("grant_type", "authorization_code"),
        ])
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let t: TokenReply = parse(r).await?;
    if t.refresh_token.is_empty() {
        return Err("Google returned no refresh token — revoke the app at \
                    myaccount.google.com/permissions and sign in again"
            .into());
    }
    Ok(Tokens {
        refresh_token: t.refresh_token,
        access_token: t.access_token,
        expires_at: crate::share::now() + t.expires_in,
    })
}

/// A high-entropy PKCE verifier: 32 random bytes, base64url. `getrandom` is already in the tree
/// via rustls, but the runtime's own randomness is enough here and needs no new import path —
/// time and address entropy mixed through the hasher we already use for sync state.
fn pkce_verifier() -> String {
    use base64::Engine;
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("system randomness");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn pkce_challenge(verifier: &str) -> String {
    use base64::Engine;
    use sha2::Digest;
    let digest = sha2::Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

/// Percent-encode a query parameter value. Only the characters Google's URLs actually need.
fn urlencode(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '.' | '_' | '~' => c.to_string(),
            c => c
                .to_string()
                .into_bytes()
                .iter()
                .map(|b| format!("%{b:02X}"))
                .collect(),
        })
        .collect()
}

#[derive(Deserialize)]
struct TokenReply {
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    #[serde(default)]
    expires_in: i64,
}

/// A usable access token, refreshing (and re-saving) when the old one is about to expire.
pub async fn access_token(
    client_id: &str,
    client_secret: &str,
    t: &mut Tokens,
) -> Result<String, String> {
    if t.fresh() {
        return Ok(t.access_token.clone());
    }
    let r = client()
        .post(TOKEN_URL)
        .form(&[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("refresh_token", &t.refresh_token),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let reply: TokenReply = parse(r).await?;
    t.access_token = reply.access_token;
    t.expires_at = crate::share::now() + reply.expires_in;
    t.save();
    Ok(t.access_token.clone())
}

/// The remote blob: its Drive file id, when Drive last changed it, and the JSON itself.
pub struct Remote {
    pub id: String,
    pub modified: String,
    pub body: String,
}

/// Fetch the synced settings, or `None` when this account has never synced.
pub async fn fetch(access: &str) -> Result<Option<Remote>, String> {
    #[derive(Deserialize)]
    struct File {
        id: String,
        #[serde(rename = "modifiedTime")]
        modified_time: String,
    }
    #[derive(Deserialize)]
    struct List {
        #[serde(default)]
        files: Vec<File>,
    }
    let list: List = parse(
        client()
            .get(FILES_URL)
            .bearer_auth(access)
            .query(&[
                ("spaces", "appDataFolder"),
                ("q", &format!("name = '{REMOTE_NAME}'")),
                ("fields", "files(id,modifiedTime)"),
            ])
            .send()
            .await
            .map_err(|e| e.to_string())?,
    )
    .await?;
    let Some(f) = list.files.into_iter().next() else {
        return Ok(None);
    };
    let body = client()
        .get(format!("{FILES_URL}/{}", f.id))
        .bearer_auth(access)
        .query(&[("alt", "media")])
        .send()
        .await
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    Ok(Some(Remote {
        id: f.id,
        modified: f.modified_time,
        body,
    }))
}

/// Write `body` back, creating the file on the first sync. Returns the new `modifiedTime`.
pub async fn push(access: &str, id: Option<&str>, body: String) -> Result<String, String> {
    #[derive(Deserialize)]
    struct Saved {
        #[serde(rename = "modifiedTime")]
        modified_time: String,
    }
    let r = match id {
        // Updating content only: a plain media upload, no metadata part needed.
        Some(id) => client()
            .patch(format!("{UPLOAD_URL}/{id}"))
            .query(&[("uploadType", "media"), ("fields", "modifiedTime")])
            .bearer_auth(access)
            .header("content-type", "application/json")
            .body(body),
        // Creating: multipart, so the metadata can put it in appDataFolder.
        None => {
            let meta = serde_json::json!({"name": REMOTE_NAME, "parents": ["appDataFolder"]});
            let boundary = "hookecho-sync-boundary";
            let payload = format!(
                "--{b}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{meta}\r\n\
                 --{b}\r\nContent-Type: application/json\r\n\r\n{body}\r\n--{b}--\r\n",
                b = boundary
            );
            client()
                .post(UPLOAD_URL)
                .query(&[("uploadType", "multipart"), ("fields", "modifiedTime")])
                .bearer_auth(access)
                .header(
                    "content-type",
                    format!("multipart/related; boundary={boundary}"),
                )
                .body(payload)
        }
    }
    .send()
    .await
    .map_err(|e| e.to_string())?;
    let saved: Saved = parse(r).await?;
    Ok(saved.modified_time)
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(wxdata::alerts::USER_AGENT)
        .build()
        .unwrap_or_default()
}

/// Decode a JSON reply, turning Google's `{"error": …}` bodies into readable errors instead of
/// "missing field" noise.
async fn parse<T: serde::de::DeserializeOwned>(r: reqwest::Response) -> Result<T, String> {
    let status = r.status();
    let body = r.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        let msg = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| {
                v.pointer("/error/message")
                    .or_else(|| v.get("error_description"))
                    .or_else(|| v.get("error"))
                    .and_then(|m| m.as_str().map(str::to_string))
            })
            .unwrap_or(body);
        return Err(format!("{status}: {msg}"));
    }
    serde_json::from_str(&body).map_err(|e| e.to_string())
}

/// Take the shared fields from `remote`, keep this machine's own. Returns the settings JSON to
/// deserialize, or `Err` if the remote blob isn't an object at all.
pub fn merge_in(local: &serde_json::Value, remote: &str) -> Result<serde_json::Value, String> {
    let mut merged: serde_json::Value = serde_json::from_str(remote).map_err(|e| e.to_string())?;
    let (Some(m), Some(l)) = (merged.as_object_mut(), local.as_object()) else {
        return Err("synced settings are not an object".into());
    };
    for k in DEVICE_LOCAL {
        match l.get(k) {
            Some(v) => m.insert(k.to_string(), v.clone()),
            None => m.remove(k),
        };
    }
    Ok(merged)
}

/// The blob we upload: local settings minus the device-local fields, so another machine's screen
/// scale and device name are never clobbered by ours.
pub fn shareable(local: &serde_json::Value) -> serde_json::Value {
    let mut out = local.clone();
    if let Some(o) = out.as_object_mut() {
        for k in DEVICE_LOCAL {
            o.remove(k);
        }
    }
    out
}

/// What a sync should do, given whether each side changed since they last agreed.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Action {
    Push,
    Pull,
    Nothing,
    /// Both sides changed. The caller resolves it — we pull, because the local copy is still on
    /// disk and in the UI, while an unpulled remote edit would be silently lost.
    Conflict,
}

pub fn decide(local_changed: bool, remote_changed: bool, remote_exists: bool) -> Action {
    match (local_changed, remote_changed, remote_exists) {
        (_, _, false) => Action::Push,
        (true, false, _) => Action::Push,
        (false, true, _) => Action::Pull,
        (true, true, _) => Action::Conflict,
        (false, false, _) => Action::Nothing,
    }
}

/// The bookkeeping that makes "changed since we last agreed" answerable: the remote timestamp and
/// the local content hash as of the last successful sync. Local file, never synced.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct SyncState {
    #[serde(default)]
    pub remote_modified: String,
    #[serde(default)]
    pub local_hash: u64,
    #[serde(default)]
    pub last_sync: i64,
}

impl SyncState {
    pub fn path() -> Option<PathBuf> {
        crate::paths::config_dir().map(|d| d.join("sync-state.json"))
    }

    pub fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(path) = Self::path() else { return };
        if let Some(p) = path.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        if let Ok(j) = serde_json::to_string(self) {
            let _ = crate::settings::atomic_write(&path, j.as_bytes());
        }
    }
}

/// Content hash of the shareable settings. Only used to answer "did this machine edit anything
/// since the last sync", so any stable hash will do.
pub fn hash(v: &serde_json::Value) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_string(v).unwrap_or_default().hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn loopback_listener_hands_back_the_code() {
        use std::io::Write;
        let p = start_login("test-client").unwrap();
        assert!(p.url.starts_with(AUTH_URL));
        let port: u16 = p.redirect.rsplit(':').next().unwrap().parse().unwrap();
        // Play the browser: request the redirect exactly as Google would.
        let mut c = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(
            c,
            "GET /?code=4%2Fabc&scope=x HTTP/1.1\r\nHost: localhost\r\n\r\n"
        )
        .unwrap();
        let got =
            p.rx.recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
        assert_eq!(got.unwrap(), "4/abc"); // %2F decoded — Google's codes contain a slash
    }

    #[test]
    fn pkce_challenge_matches_rfc7636_vector() {
        // Appendix B of RFC 7636 — if this drifts, Google rejects every sign-in.
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn verifier_is_long_and_unique() {
        let (a, b) = (pkce_verifier(), pkce_verifier());
        assert_ne!(a, b);
        assert!(a.len() >= 43, "{a}"); // RFC 7636 minimum
    }

    #[test]
    fn redirect_query_yields_code_or_error() {
        assert_eq!(
            param("code=4%2Fabc&scope=x", "code").as_deref(),
            Some("4/abc")
        );
        assert_eq!(
            param("error=access_denied", "error").as_deref(),
            Some("access_denied")
        );
        assert!(param("error=access_denied", "code").is_none());
    }

    #[test]
    fn urlencode_escapes_what_google_urls_need() {
        assert_eq!(
            urlencode("https://www.googleapis.com/auth/drive.appdata"),
            "https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fdrive.appdata"
        );
    }

    #[test]
    fn merge_keeps_device_local_fields() {
        let local = json!({"ui_scale": 1.5, "share_name": "laptop", "default_site": "KFWS"});
        let remote = r#"{"ui_scale": 0.8, "share_name": "phone", "default_site": "KTLX"}"#;
        let merged = merge_in(&local, remote).unwrap();
        assert_eq!(merged["default_site"], "KTLX"); // shared field follows the remote
        assert_eq!(merged["ui_scale"], 1.5); // this machine's screen scale survives
        assert_eq!(merged["share_name"], "laptop");
    }

    #[test]
    fn merge_drops_device_local_fields_absent_locally() {
        let merged = merge_in(&json!({"default_site": "KFWS"}), r#"{"ui_scale": 2.0}"#).unwrap();
        assert!(merged.get("ui_scale").is_none());
    }

    #[test]
    fn shareable_strips_device_local_fields() {
        let s = shareable(&json!({"ui_scale": 1.5, "mapbox_key": "pk.x"}));
        assert!(s.get("ui_scale").is_none());
        assert_eq!(s["mapbox_key"], "pk.x"); // keys do sync — the user asked for that
    }

    #[test]
    fn decisions() {
        assert_eq!(decide(false, false, false), Action::Push); // first sync ever
        assert_eq!(decide(false, false, true), Action::Nothing);
        assert_eq!(decide(true, false, true), Action::Push);
        assert_eq!(decide(false, true, true), Action::Pull);
        assert_eq!(decide(true, true, true), Action::Conflict);
    }

    #[test]
    fn hash_tracks_content_not_order() {
        let a = json!({"a": 1, "b": 2});
        let b = json!({"b": 2, "a": 1});
        assert_eq!(hash(&a), hash(&b)); // serde_json maps are ordered, so this is stable
        assert_ne!(hash(&a), hash(&json!({"a": 2, "b": 2})));
    }

    #[test]
    fn error_bodies_become_readable() {
        // The shape Google returns for a bad client id.
        let v: serde_json::Value =
            serde_json::from_str(r#"{"error":{"message":"Invalid Credentials"}}"#).unwrap();
        assert_eq!(v.pointer("/error/message").unwrap(), "Invalid Credentials");
    }
}
