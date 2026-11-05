//! Sign in with Google, and keep `settings.json` the same on every machine.
//!
//! The data lives in **your own Drive**, in the hidden per-app folder (`appDataFolder`) that only
//! this app and you can see — there is no HookEcho server, no account, and nothing to pay for.
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
/// Keep ordinary HookEcho's existing remote object unchanged. The standalone workspace
/// shares an OAuth client/appDataFolder when the user chooses, but never its settings object.
fn remote_name_for_profile(radar_workspace: bool) -> &'static str {
    if radar_workspace { "radar-workspace-settings.json" } else { "settings.json" }
}

fn remote_name() -> &'static str {
    remote_name_for_profile(crate::paths::is_radar_workspace())
}

#[derive(Deserialize)]
struct RemoteFile {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(rename = "modifiedTime")]
    modified_time: String,
}

fn select_remote_file(files: Vec<RemoteFile>, radar_workspace: bool) -> Option<RemoteFile> {
    // Defend against an unexpected list response as well as filtering in the Drive query.
    // Never fall back to the first file: it may belong to the other local application.
    files.into_iter().find(|file| file.name == remote_name_for_profile(radar_workspace))
}

/// Settings fields that belong to *this* machine and must survive a sync from another one.
/// Everything else — markers, placefiles, palettes, theme, API keys — is shared.
pub const DEVICE_LOCAL: [&str; 10] = [
    // Permission to execute a command is granted separately on each machine.
    "plugins",
    "ui_scale",
    "share_name",
    "background_alerts",
    "share_relay",
    // Pushes this machine held back during quiet hours. They are owed to whoever is sitting at
    // this device, and would arrive as somebody else's stale summary if they travelled.
    "quiet_pending",
    // This machine's server credential. Copying it to a laptop would mean two boxes sharing one
    // token, and one of them changing it locks the other out.
    "serve_token",
    // Account setup and scheduling belong to the signed-in device.
    "sync_client_id",
    "sync_client_secret",
    "sync_enabled",
];

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
        Ok(_) => "<h2>Signed in.</h2><p>You can close this tab and go back to HookEcho.</p>",
        Err(_) => "<h2>Sign-in failed.</h2><p>Go back to HookEcho for the reason.</p>",
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
pub(crate) fn param(query: &str, key: &str) -> Option<String> {
    let raw = query.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == key).then_some(v)
    })?;
    Some(percent_decode(&raw.replace('+', " ")))
}

pub(crate) fn percent_decode(s: &str) -> String {
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
pub(crate) fn urlencode(s: &str) -> String {
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

/// A usable access token. The UI saves refreshed tokens only while this session is still active.
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
    Ok(t.access_token.clone())
}

/// The remote blob: its Drive file id, when Drive last changed it, and the JSON itself.
#[derive(Debug, Clone)]
pub struct Remote {
    pub id: String,
    pub modified: String,
    pub body: String,
}

/// Worker results carry the snapshot hash so a late pull cannot erase newer edits.
pub enum SyncMsg {
    Signed(Tokens),
    Pulled { remote: Remote, started_revision: u64, conflict: bool, hash: u64, resolve_conflict: bool },
    Pushed { modified: String, hash: u64 },
    UpToDate,
    Error(String),
}

/// Each operation has its own inbox. Signing out drops queued results as well as aborting IO;
/// tokens are never saved by workers, including workers finishing concurrently with sign-out.
#[derive(Default)]
pub struct SyncSession {
    rx: Option<std::sync::mpsc::Receiver<SyncMsg>>,
    abort: Option<futures_util::future::AbortHandle>,
    pub conflict: Option<Remote>,
}

impl SyncSession {
    pub fn busy(&self) -> bool { self.rx.is_some() }

    pub fn start(&mut self, rx: std::sync::mpsc::Receiver<SyncMsg>, abort: futures_util::future::AbortHandle) {
        assert!(!self.busy(), "sync operations must not overlap");
        self.rx = Some(rx);
        self.abort = Some(abort);
    }

    pub fn cancel(&mut self) {
        if let Some(abort) = self.abort.take() { abort.abort(); }
        self.rx = None;
        self.conflict = None;
    }

    pub fn next_message(&mut self) -> Option<SyncMsg> {
        match self.rx.as_ref()?.try_recv() {
            Ok(msg) => Some(msg),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.rx = None;
                self.abort = None;
                None
            }
        }
    }

    pub fn receive_pull(&mut self, remote: Remote, started_revision: u64, current_revision: u64, conflict: bool) -> Option<Remote> {
        if conflict || started_revision != current_revision || self.conflict.is_some() {
            self.conflict.get_or_insert(remote);
            None
        } else {
            Some(remote)
        }
    }
}

impl Drop for SyncSession {
    fn drop(&mut self) { self.cancel(); }
}

/// Enough for settings and custom palettes, without accepting unbounded Drive files.
pub const MAX_SYNC_BYTES: usize = 8 * 1024 * 1024;

/// Fetch the synced settings, or `None` when this account has never synced.
pub async fn fetch(access: &str) -> Result<Option<Remote>, String> {
    #[derive(Deserialize)]
    struct List {
        #[serde(default)]
        files: Vec<RemoteFile>,
    }
    let list: List = parse(
        client()
            .get(FILES_URL)
            .bearer_auth(access)
            .query(&[
                ("spaces", "appDataFolder"),
                ("q", &format!("name = '{}' and trashed = false", remote_name())),
                ("fields", "files(id,name,modifiedTime)"),
            ])
            .send()
            .await
            .map_err(|e| e.to_string())?,
    )
    .await?;
    let Some(f) = select_remote_file(list.files, crate::paths::is_radar_workspace()) else {
        return Ok(None);
    };
    let response = client()
        .get(format!("{FILES_URL}/{}", f.id))
        .bearer_auth(access)
        .query(&[("alt", "media")])
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status().map_err(|e| e.to_string())?;
    if response.content_length().is_some_and(|len| len > MAX_SYNC_BYTES as u64) {
        return Err("synced settings exceed 8 MiB".into());
    }
    use futures_util::StreamExt;
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        if chunk.len() > MAX_SYNC_BYTES.saturating_sub(bytes.len()) {
            return Err("synced settings exceed 8 MiB".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let body = String::from_utf8(bytes).map_err(|e| e.to_string())?;
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
            let meta = serde_json::json!({"name": remote_name(), "parents": ["appDataFolder"]});
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
    if remote.len() > MAX_SYNC_BYTES { return Err("synced settings exceed 8 MiB".into()); }
    let mut merged: serde_json::Value = serde_json::from_str(remote).map_err(|e| e.to_string())?;
    let (Some(m), Some(l)) = (merged.as_object_mut(), local.as_object()) else {
        return Err("synced settings are not an object".into());
    };
    for k in DEVICE_LOCAL { keep_local_field(m, l, k); }
    for field in SOUND_FIELDS {
        if m.get(field).is_none_or(custom_sound) { keep_local_field(m, l, field); }
    }
    for (collection, field) in [("markers", "icon"), ("alert_rules", "sound")] {
        if let Some(items) = m.get_mut(collection).and_then(serde_json::Value::as_array_mut) {
            for item in items {
                let Some(item) = item.as_object_mut() else { continue };
                let unresolved = item.get(field).is_none_or(|v| if field == "icon" { custom_icon(v) } else { custom_sound(v) });
                if !unresolved { continue; }
                let id = item.get("id").and_then(serde_json::Value::as_str).filter(|id| !id.is_empty());
                let previous = l.get(collection).and_then(serde_json::Value::as_array)
                    .and_then(|items| items.iter().find(|other| id.is_some() && other.get("id").and_then(serde_json::Value::as_str) == id))
                    .and_then(serde_json::Value::as_object);
                if let Some(previous) = previous { keep_local_field(item, previous, field); }
                else { item.remove(field); }
            }
        }
    }
    // A path from another computer is not a transferable asset, even if it happens to exist
    // here. Older clients sent bare paths; retain this device's palette for those entries.
    let mut palettes = m.get("palettes").and_then(serde_json::Value::as_object).cloned().unwrap_or_default();
    let mut files = m.get("web_files").and_then(serde_json::Value::as_object).cloned().unwrap_or_default();
    for (moment, path) in palettes.clone() {
        let portable = path.as_str().is_some_and(|p| p.starts_with(crate::colormap::BUILTIN_PREFIX)
            || files.get(p).and_then(serde_json::Value::as_str).is_some_and(|text| text.len() <= 2 * 1024 * 1024));
        if portable { continue; }
        if let Some(previous) = l.get("palettes").and_then(|v| v.get(&moment)) {
            palettes.insert(moment, previous.clone());
            if let Some((name, text)) = previous.as_str().and_then(|name| l.get("web_files")?.get(name).map(|text| (name, text))) {
                files.insert(name.into(), text.clone());
            }
        } else { palettes.remove(&moment); }
    }
    if m.contains_key("palettes") { m.insert("palettes".into(), palettes.into()); }
    if !files.is_empty() { m.insert("web_files".into(), files.into()); }
    Ok(merged)
}

fn keep_local_field(target: &mut serde_json::Map<String, serde_json::Value>, local: &serde_json::Map<String, serde_json::Value>, field: &str) {
    if let Some(value) = local.get(field) { target.insert(field.into(), value.clone()); }
    else { target.remove(field); }
}

const SOUND_FIELDS: [&str; 8] = ["rain_sound", "scan_sound", "warn_sound", "storm_warn_sound", "tds_sound", "rotation_sound", "lightning_sound", "emergency_sound"];
fn custom_sound(value: &serde_json::Value) -> bool { value.get("Custom").is_some() }
fn custom_icon(value: &serde_json::Value) -> bool { value.as_str().is_some_and(|s| !s.starts_with("builtin:")) }

fn shared_fields(local: &serde_json::Value) -> serde_json::Value {
    let mut out = local.clone();
    if let Some(fields) = out.as_object_mut() {
        for key in DEVICE_LOCAL { fields.remove(key); }
    }
    out
}

/// Custom sounds and marker images stay on the device until their bytes can travel too.
/// Omission lets receiving clients preserve their own asset instead of loading a foreign path.
pub fn shareable(local: &serde_json::Value) -> serde_json::Value {
    let mut out = shared_fields(local);
    if let Some(o) = out.as_object_mut() {
        for field in SOUND_FIELDS {
            if o.get(field).is_some_and(custom_sound) { o.remove(field); }
        }
        for (collection, field) in [("markers", "icon"), ("alert_rules", "sound")] {
            if let Some(items) = o.get_mut(collection).and_then(serde_json::Value::as_array_mut) {
                for item in items {
                    if let Some(item) = item.as_object_mut() {
                        if item.get(field).is_some_and(|v| if field == "icon" { custom_icon(v) } else { custom_sound(v) }) { item.remove(field); }
                    }
                }
            }
        }
    }
    out
}

pub fn settings_hash(settings: &crate::settings::Settings) -> Result<u64, String> {
    settings_snapshot(settings).map(|(_, hash)| hash)
}

/// A pull can replace custom choices with builtins even though custom asset paths are omitted
/// from uploads. Include those references when checking for edits made during a request.
pub fn settings_revision(settings: &crate::settings::Settings) -> Result<u64, String> {
    serde_json::to_value(settings).map(|v| hash(&shared_fields(&v))).map_err(|e| e.to_string())
}

#[cfg(test)]
fn settings_body(settings: &crate::settings::Settings) -> Result<String, String> {
    settings_snapshot(settings).map(|(body, _)| body)
}

/// Prepared on the sync worker: the same portable content drives both upload and dirty checks.
pub fn settings_snapshot(settings: &crate::settings::Settings) -> Result<(String, u64), String> {
    let portable = settings.portable_sync()?;
    let value = serde_json::to_value(portable).map_err(|e| e.to_string())?;
    let shared = shareable(&value);
    let body = serde_json::to_string_pretty(&shared).map_err(|e| e.to_string())?;
    if body.len() > MAX_SYNC_BYTES { return Err("synced settings exceed 8 MiB".into()); }
    Ok((body, hash(&shared)))
}

/// Compute the destination's baseline, including local fallback assets, before delivery to UI.
pub fn pulled_hash(local: &crate::settings::Settings, body: &str) -> Result<u64, String> {
    let local = serde_json::to_value(local).map_err(|e| e.to_string())?;
    let settings = serde_json::from_value(merge_in(&local, body)?).map_err(|e| e.to_string())?;
    settings_hash(&settings)
}

/// What a sync should do, given whether each side changed since they last agreed.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Action {
    Push,
    Pull,
    Nothing,
    /// Both sides changed. Keep both copies until the user chooses which one to retain.
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

    fn remote(body: &str, modified: &str) -> Remote {
        Remote { id: "drive-settings".into(), modified: modified.into(), body: body.into() }
    }

    #[test]
    fn late_pull_preserves_edits_and_stays_conflicted_on_repeat_pass() {
        let mut session = SyncSession::default();
        let start = json!({"markers": [{"id": "home", "name": "Home"}]});
        let edited = json!({"markers": [{"id": "home", "name": "New home"}]});
        assert!(session.receive_pull(remote("drive copy", "v2"), hash(&start), hash(&edited), false).is_none());
        assert_eq!(session.conflict.as_ref().unwrap().body, "drive copy");
        // A later pass's baseline already includes the edit. It must not silently clear the conflict.
        assert!(session.receive_pull(remote("another copy", "v3"), hash(&edited), hash(&edited), false).is_none());
        assert_eq!(session.conflict.as_ref().unwrap().modified, "v2");
        assert_eq!(edited["markers"][0]["name"], "New home");
    }

    #[test]
    fn late_pull_preserves_new_custom_sound_and_icon_choices() {
        let initial: crate::settings::Settings = serde_json::from_value(json!({
            "warn_sound": {"Custom":"old.wav"},
            "markers": [{"id":"home", "name":"Home", "lat":0.0, "lon":0.0, "icon":"old.png"}],
            "alert_rules": [{"id":"storm", "trigger":"Tds", "sound":{"Custom":"old-rule.wav"}}]
        })).unwrap();
        let remote_body = json!({
            "warn_sound":"Siren",
            "markers":[{"id":"home", "name":"Home", "lat":0.0, "lon":0.0, "icon":"builtin:home"}],
            "alert_rules":[{"id":"storm", "trigger":"Tds", "sound":"Chime"}]
        }).to_string();
        for changed in ["sound", "icon", "rule"] {
            let mut edited = initial.clone();
            match changed {
                "sound" => edited.warn_sound = crate::settings::AlertSound::Custom("new.wav".into()),
                "icon" => edited.markers[0].icon = Some("new.png".into()),
                _ => edited.alert_rules[0].sound = Some(crate::settings::AlertSound::Custom("new-rule.wav".into())),
            }
            // Device file changes are intentionally not uploads, but still require pull protection.
            assert_eq!(settings_hash(&initial).unwrap(), settings_hash(&edited).unwrap());
            assert_ne!(settings_revision(&initial).unwrap(), settings_revision(&edited).unwrap());
            let mut session = SyncSession::default();
            assert!(session.receive_pull(remote(&remote_body, "v2"), settings_revision(&initial).unwrap(),
                settings_revision(&edited).unwrap(), false).is_none());
            assert!(session.conflict.is_some());
        }
    }

    #[test]
    fn pull_requires_choice_when_both_sides_changed_before_the_request() {
        let mut session = SyncSession::default();
        assert!(session.receive_pull(remote("remote", "v2"), 1, 1, true).is_none());
        session.cancel();
        assert!(session.receive_pull(remote("fresh", "v3"), 2, 2, false).is_some());
    }

    #[test]
    fn signout_discards_queued_and_late_credentials_and_pending_conflicts() {
        let (tx, rx) = std::sync::mpsc::channel();
        let (abort, _registration) = futures_util::future::AbortHandle::new_pair();
        let witness = abort.clone();
        let mut session = SyncSession::default();
        session.start(rx, abort);
        tx.send(SyncMsg::Signed(Tokens { refresh_token: "old account".into(), ..Default::default() })).unwrap();
        tx.send(SyncMsg::Pulled { remote: remote("old settings", "v2"), started_revision: 1, conflict: true, hash: 1, resolve_conflict: false }).unwrap();
        session.conflict = Some(remote("pending old account", "v1"));
        session.cancel();
        assert!(witness.is_aborted());
        assert!(!session.busy());
        assert!(session.next_message().is_none());
        assert!(session.conflict.is_none());
        assert!(tx.send(SyncMsg::Signed(Tokens::default())).is_err());
        let (new_tx, new_rx) = std::sync::mpsc::channel();
        let (new_abort, _) = futures_util::future::AbortHandle::new_pair();
        session.start(new_rx, new_abort);
        new_tx.send(SyncMsg::UpToDate).unwrap();
        assert!(matches!(session.next_message(), Some(SyncMsg::UpToDate)));
        drop(new_tx);
        assert!(session.next_message().is_none());
        assert!(!session.busy());
    }

    #[test]
    fn cloud_transfers_palette_bytes_without_requiring_the_source_file() {
        let path = std::env::temp_dir().join(format!("hookecho-cloud-palette-{}.pal", std::process::id()));
        let palette = "Step: 5\nColor: 0 0 0 0\nColor: 80 255 255 255\n";
        std::fs::write(&path, palette).unwrap();
        let mut source = crate::settings::Settings::default();
        source.palettes.insert("REF".into(), path.to_string_lossy().into_owned());
        source.warn_sound = crate::settings::AlertSound::Custom("/source/warn.wav".into());
        let body = settings_body(&source).unwrap();
        std::fs::remove_file(path).unwrap();
        let target = crate::settings::Settings {
            warn_sound: crate::settings::AlertSound::Custom("/target/warn.wav".into()),
            ..Default::default()
        };
        let merged = merge_in(&serde_json::to_value(&target).unwrap(), &body).unwrap();
        let received: crate::settings::Settings = serde_json::from_value(merged).unwrap();
        assert_eq!(received.web_files[&received.palettes["REF"]], palette);
        assert!(received.palette_paths()[0].as_ref().unwrap().to_string_lossy().starts_with(crate::colormap::INLINE_PREFIX));
        assert_eq!(received.warn_sound, target.warn_sound);
        let returned = settings_body(&received).unwrap();
        assert!(returned.contains("Color: 80"));
        assert!(!returned.contains("/source/warn.wav"));
        assert!(!returned.contains("/target/warn.wav"));
    }

    #[test]
    fn palette_file_edits_at_the_same_path_trigger_a_push() {
        let path = std::env::temp_dir().join(format!("hookecho-cloud-revision-{}.pal", std::process::id()));
        std::fs::write(&path, "Color: 0 1 2 3\n").unwrap();
        let mut settings = crate::settings::Settings::default();
        settings.palettes.insert("REF".into(), path.to_string_lossy().into_owned());
        let (old_body, old_hash) = settings_snapshot(&settings).unwrap();
        let old_revision = settings_revision(&settings).unwrap();
        std::fs::write(&path, "Color: 0 4 5 6\n").unwrap();
        let (new_body, new_hash) = settings_snapshot(&settings).unwrap();
        std::fs::remove_file(path).unwrap();
        assert_ne!(old_body, new_body);
        assert_ne!(old_hash, new_hash);
        assert_eq!(old_revision, settings_revision(&settings).unwrap(), "ordinary UI fingerprint does not read files");
        assert_eq!(decide(new_hash != old_hash, false, true), Action::Push);
    }

    #[test]
    fn pulled_baseline_matches_portable_content_on_the_receiving_device() {
        let local: crate::settings::Settings = serde_json::from_value(json!({
            "palettes":{"VEL":"local.pal"}, "web_files":{"local.pal":"local palette"},
            "warn_sound":"Siren"
        })).unwrap();
        // Includes a legacy unresolved path and omitted local custom sound to exercise fallback.
        let remote = json!({
            "palettes":{"REF":"bundle-REF.pal", "VEL":"/foreign/velocity.pal"},
            "web_files":{"bundle-REF.pal":"remote palette"}, "warn_sound":{"Custom":"/foreign/sound.wav"}
        }).to_string();
        let baseline = pulled_hash(&local, &remote).unwrap();
        let received: crate::settings::Settings = serde_json::from_value(
            merge_in(&serde_json::to_value(&local).unwrap(), &remote).unwrap()).unwrap();
        assert_eq!(baseline, settings_hash(&received).unwrap());
        assert_eq!(decide(settings_hash(&received).unwrap() != baseline, false, true), Action::Nothing);
        let body = settings_body(&received).unwrap();
        let another_device = crate::settings::Settings::default();
        let other_baseline = pulled_hash(&another_device, &body).unwrap();
        let other: crate::settings::Settings = serde_json::from_value(
            merge_in(&serde_json::to_value(another_device).unwrap(), &body).unwrap()).unwrap();
        assert_eq!(baseline, other_baseline);
        assert_eq!(baseline, settings_hash(&other).unwrap());
    }

    #[test]
    fn legacy_cloud_paths_preserve_only_matching_local_assets() {
        let local = json!({
            "warn_sound": {"Custom":"/my/warn.wav"},
            "palettes":{"REF":"local-ref.pal", "VEL":"builtin:velocity"},
            "web_files":{"local-ref.pal":"local palette"},
            "markers":[{"id":"home","icon":"my-icon.png"}],
            "alert_rules":[{"id":"rain","sound":{"Custom":"/my/rain.wav"}}]
        });
        let old_remote = json!({
            "warn_sound":{"Custom":"/their/warn.wav"},
            "palettes":{"REF":"/their/ref.pal", "VEL":"/their/vel.pal", "CC":"/their/cc.pal"},
            "markers":[{"id":"home","icon":"their-icon.png"}, {"id":"new","icon":"their-icon.png"}],
            "alert_rules":[{"id":"rain","sound":{"Custom":"/their/rain.wav"}}, {"id":"new","sound":{"Custom":"/their/new.wav"}}]
        });
        for remote in [old_remote.clone(), shareable(&old_remote)] {
            let merged = merge_in(&local, &remote.to_string()).unwrap();
            assert_eq!(merged["warn_sound"], local["warn_sound"]);
            assert_eq!(merged["palettes"]["REF"], "local-ref.pal");
            assert_eq!(merged["web_files"]["local-ref.pal"], "local palette");
            assert_eq!(merged["palettes"]["VEL"], "builtin:velocity");
            assert!(merged["palettes"].get("CC").is_none());
            assert_eq!(merged["markers"][0]["icon"], "my-icon.png");
            assert!(merged["markers"][1].get("icon").is_none());
            assert_eq!(merged["alert_rules"][0]["sound"], local["alert_rules"][0]["sound"]);
            assert!(merged["alert_rules"][1].get("sound").is_none());
        }
    }

    #[test]
    fn builtin_assets_and_explicit_clear_still_sync() {
        let local = json!({"markers":[{"id":"home","icon":"my-icon.png"}],"warn_sound":{"Custom":"/my/warn.wav"}});
        for icon in [json!("builtin:home"), serde_json::Value::Null] {
            let remote = json!({"markers":[{"id":"home","icon":icon}],"warn_sound":"Siren"});
            let merged = merge_in(&local, &shareable(&remote).to_string()).unwrap();
            assert_eq!(merged["markers"][0]["icon"], icon);
            assert_eq!(merged["warn_sound"], "Siren");
        }
    }

    #[test]
    fn cloud_palette_payloads_are_bounded() {
        let mut settings = crate::settings::Settings::default();
        settings.palettes.insert("REF".into(), "large.pal".into());
        settings.web_files.insert("large.pal".into(), "x".repeat(2 * 1024 * 1024 + 1));
        assert!(settings_body(&settings).unwrap_err().contains("2 MiB"));
        assert!(merge_in(&json!({}), &" ".repeat(MAX_SYNC_BYTES + 1)).unwrap_err().contains("8 MiB"));
    }

    #[test]
    fn profile_remote_files_never_fall_back_to_the_other_app() {
        assert_eq!(remote_name_for_profile(false), "settings.json");
        assert_ne!(remote_name_for_profile(false), remote_name_for_profile(true));
        let files = || vec![
            RemoteFile { id: "ordinary".into(), name: "settings.json".into(), modified_time: "now".into() },
            RemoteFile { id: "workspace".into(), name: "radar-workspace-settings.json".into(), modified_time: "now".into() },
        ];
        assert_eq!(select_remote_file(files(), false).unwrap().id, "ordinary");
        assert_eq!(select_remote_file(files(), true).unwrap().id, "workspace");
        assert!(select_remote_file(vec![files().remove(0)], true).is_none());
        assert!(select_remote_file(vec![files().remove(1)], false).is_none());
        let unnamed = serde_json::from_str::<RemoteFile>(r#"{"id":"legacy","modifiedTime":"now"}"#).unwrap();
        assert!(select_remote_file(vec![unnamed], true).is_none());
    }

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
        // Held-back pushes belong to the machine that held them, not to the next device to sync.
        let s = shareable(&json!({"quiet_pending": [["Tornado Warning", "…"]]}));
        assert!(s.get("quiet_pending").is_none());
        let s = shareable(&json!({"ui_scale": 1.5, "mapbox_key": "pk.x"}));
        assert!(s.get("ui_scale").is_none());
        assert_eq!(s["mapbox_key"], "pk.x"); // keys do sync — the user asked for that
    }

    #[test]
    fn sync_cannot_install_or_enable_remote_commands() {
        let local = json!({"plugins": [{"command": "trusted-local", "enabled": true}]});
        let remote = r#"{"plugins":[{"command":"remote-command","enabled":true}],"theme":"Light"}"#;
        assert_eq!(merge_in(&local, remote).unwrap()["plugins"], local["plugins"]);
        assert!(merge_in(&json!({}), remote).unwrap().get("plugins").is_none());
        assert!(shareable(&local).get("plugins").is_none());
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
