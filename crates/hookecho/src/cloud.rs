//! Sign in with Google, and keep `settings.json` the same on every machine.
//!
//! The data lives in **your own Drive**, in the hidden per-app folder (`appDataFolder`) that only
//! this app and you can see — there is no Hook Echo server, no account, and nothing to pay for.
//! Sign-in uses the OAuth 2.0 **device flow**: the app shows a short code, you type it on any
//! browser, and Google hands back a refresh token. That one flow works identically on a desktop
//! and on a phone, which is why it is here instead of a loopback redirect.
//!
//! You supply the OAuth client (see `docs/sync.md`) — shipping a shared client id in an
//! open-source binary would put every user's quota, and Google's trust in it, in one basket.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const DEVICE_CODE_URL: &str = "https://oauth2.googleapis.com/device/code";
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
                if let Err(e) = std::fs::write(&path, j) {
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

/// The half-finished sign-in shown to the user: type `user_code` at `verification_url`.
#[derive(Debug, Clone, Deserialize)]
pub struct DeviceAuth {
    pub device_code: String,
    pub user_code: String,
    pub verification_url: String,
    pub interval: u64,
    pub expires_in: i64,
}

/// Ask Google for a code pair to show the user.
pub async fn start_device_auth(client_id: &str) -> Result<DeviceAuth, String> {
    let r = client()
        .post(DEVICE_CODE_URL)
        .form(&[("client_id", client_id), ("scope", SCOPE)])
        .send()
        .await
        .map_err(|e| e.to_string())?;
    parse(r).await
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

/// Poll until the user finishes (or refuses) the browser half. `authorization_pending` and
/// `slow_down` are the normal answers while they're still typing, not failures.
pub async fn poll_device_token(
    client_id: &str,
    client_secret: &str,
    auth: &DeviceAuth,
) -> Result<Tokens, String> {
    let deadline = crate::share::now() + auth.expires_in.min(600);
    let mut wait = auth.interval.max(1);
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
        let r = client()
            .post(TOKEN_URL)
            .form(&[
                ("client_id", client_id),
                ("client_secret", client_secret),
                ("device_code", &auth.device_code),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ])
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let body = r.text().await.map_err(|e| e.to_string())?;
        let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
        match v.get("error").and_then(|e| e.as_str()) {
            Some("authorization_pending") => {}
            Some("slow_down") => wait += 5,
            Some(e) => return Err(e.to_string()),
            None => {
                let t: TokenReply = serde_json::from_str(&body).map_err(|e| e.to_string())?;
                return Ok(Tokens {
                    refresh_token: t.refresh_token,
                    access_token: t.access_token,
                    expires_at: crate::share::now() + t.expires_in,
                });
            }
        }
        if crate::share::now() > deadline {
            return Err("sign-in timed out".into());
        }
    }
}

/// A usable access token, refreshing (and re-saving) when the old one is about to expire.
pub async fn access_token(client_id: &str, client_secret: &str, t: &mut Tokens) -> Result<String, String> {
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
            let _ = std::fs::write(path, j);
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
