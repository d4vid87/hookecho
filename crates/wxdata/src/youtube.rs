//! Public, explicitly supplied YouTube links and optional official-API availability checks.

use chrono::{DateTime, Utc};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target { Video(String), Channel(String), Handle(String) }

impl Target {
    pub fn parse(value: &str) -> Option<Self> {
        let url = reqwest::Url::parse(value.trim()).ok()?;
        if !matches!(url.scheme(), "http" | "https") || !url.username().is_empty()
            || url.password().is_some() || url.port().is_some() { return None; }
        let host = url.host_str()?;
        let path: Vec<_> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
        let video = |id: &str| (id.len() == 11 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'))
            .then(|| Self::Video(id.to_string()));
        if host == "youtu.be" { return (path.len() == 1).then(|| video(path[0])).flatten(); }
        if !matches!(host, "youtube.com" | "www.youtube.com" | "m.youtube.com") { return None; }
        match path.as_slice() {
            ["watch"] => video(url.query_pairs().find(|(key, _)| key == "v")?.1.as_ref()),
            ["live" | "shorts" | "embed", id] => video(id),
            ["channel", id] | ["channel", id, "live"] if id.starts_with("UC") && id.len() == 24
                && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') => Some(Self::Channel((*id).into())),
            [handle] | [handle, "live"] if handle.starts_with('@') && handle.len() > 1 && handle.len() <= 200
                && !handle.bytes().any(|b| b.is_ascii_control() || matches!(b, b'?' | b'#' | b'\\')) => Some(Self::Handle((*handle).into())),
            _ => None,
        }
    }

    pub fn url(&self) -> String {
        match self {
            Self::Video(id) => format!("https://www.youtube.com/watch?v={id}"),
            Self::Channel(id) => format!("https://www.youtube.com/channel/{id}/live"),
            Self::Handle(handle) => format!("https://www.youtube.com/{handle}/live"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State { Unknown, Live, Offline, Upcoming }

impl State {
    pub fn label(self) -> &'static str {
        match self { Self::Unknown => "Live status unknown", Self::Live => "Live on YouTube", Self::Offline => "Not live on YouTube", Self::Upcoming => "Upcoming on YouTube" }
    }
}

#[derive(Clone, Debug)]
pub struct Status {
    pub state: State,
    pub checked_at: DateTime<Utc>,
    pub watch_url: Option<String>,
    pub channel_title: Option<String>,
}

impl Status {
    pub fn current_state(&self, now: DateTime<Utc>) -> State {
        let age = (now - self.checked_at).num_seconds();
        if (0..=120).contains(&age) { self.state } else { State::Unknown }
    }
}

fn video_status(item: &serde_json::Value, now: DateTime<Utc>) -> Status {
    let live = item.pointer("/snippet/liveBroadcastContent").and_then(|v| v.as_str());
    let ended = item.pointer("/liveStreamingDetails/actualEndTime").and_then(|v| v.as_str());
    let started = item.pointer("/liveStreamingDetails/actualStartTime").and_then(|v| v.as_str());
    let state = match (live, started, ended) {
        (_, _, Some(_)) => State::Offline,
        (Some("live"), Some(_), None) => State::Live,
        (Some("upcoming"), _, None) => State::Upcoming,
        (Some("none"), _, _) => State::Offline,
        _ => State::Unknown,
    };
    Status { state, checked_at: now,
        watch_url: item.get("id").and_then(|v| v.as_str()).and_then(|id| Target::parse(&format!("https://youtu.be/{id}"))).map(|target| target.url()),
        channel_title: item.pointer("/snippet/channelTitle").and_then(|v| v.as_str()).map(str::to_owned),
    }
}

async fn get(client: &reqwest::Client, resource: &str, key: &str, query: &[(&str, &str)]) -> anyhow::Result<serde_json::Value> {
    // Never include reqwest's URL-bearing error in UI/logs: it contains the user's API key.
    let response = client.get(format!("https://www.googleapis.com/youtube/v3/{resource}"))
        .query(query).query(&[("key", key)])
        .timeout(std::time::Duration::from_secs(8)).send().await
        .map_err(|_| anyhow::anyhow!("YouTube status request failed"))?;
    anyhow::ensure!(response.status().is_success(), "YouTube status unavailable (HTTP {})", response.status());
    let body = response.bytes().await.map_err(|_| anyhow::anyhow!("YouTube status response failed"))?;
    serde_json::from_slice(&body).map_err(|_| anyhow::anyhow!("Invalid YouTube status response"))
}

/// Uses only documented public YouTube Data API endpoints, with a user-supplied key.
pub async fn check(client: &reqwest::Client, target: Target, key: &str) -> anyhow::Result<Status> {
    anyhow::ensure!(!key.trim().is_empty(), "Add a YouTube API key to check live status");
    let id = match target {
        Target::Video(id) => Some(id),
        target => {
            let channel = match target {
                Target::Channel(id) => id,
                Target::Handle(handle) => {
                    let response = get(client, "channels", key, &[("part", "id"), ("forHandle", &handle)]).await?;
                    response.pointer("/items/0/id").and_then(|v| v.as_str())
                        .ok_or_else(|| anyhow::anyhow!("YouTube channel unavailable"))?.to_string()
                }
                Target::Video(_) => unreachable!(),
            };
            let response = get(client, "search", key, &[("part", "snippet"), ("channelId", &channel), ("eventType", "live"), ("type", "video"), ("maxResults", "1")]).await?;
            let items = response.get("items").and_then(|v| v.as_array()).ok_or_else(|| anyhow::anyhow!("Invalid YouTube search response"))?;
            if items.is_empty() { None } else {
                Some(response.pointer("/items/0/id/videoId").and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("YouTube live video unavailable"))?.to_string())
            }
        }
    };
    let Some(id) = id else { return Ok(Status { state: State::Offline, checked_at: Utc::now(), watch_url: None, channel_title: None }); };
    let response = get(client, "videos", key, &[("part", "snippet,liveStreamingDetails"), ("id", &id)]).await?;
    let item = response.pointer("/items/0").ok_or_else(|| anyhow::anyhow!("YouTube video unavailable"))?;
    Ok(video_status(item, Utc::now()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn youtube_links_are_allowlisted_and_canonicalized() {
        assert_eq!(Target::parse("https://youtu.be/abcdefghijk?t=2").unwrap().url(), "https://www.youtube.com/watch?v=abcdefghijk");
        assert_eq!(Target::parse("https://youtube.com/@storm/live?feature=share").unwrap().url(), "https://www.youtube.com/@storm/live");
        for bad in ["javascript:alert(1)", "https://youtube.com.evil.test/@storm", "https://youtube.com@evil.test/@storm", "https://user:pass@youtube.com/@storm", "https://youtube.com/redirect?q=https://evil.test", "https://youtu.be/no", "https://youtube.com:123/@storm", "https://youtube.com/@storm/other"] { assert!(Target::parse(bad).is_none(), "{bad}"); }
    }
    #[test]
    fn youtube_status_needs_broadcast_evidence_and_expires() {
        let now = Utc::now();
        let mut item = serde_json::json!({"id":"abcdefghijk","snippet":{"liveBroadcastContent":"live","channelTitle":"Storm"}});
        assert_eq!(video_status(&item, now).state, State::Unknown);
        item["liveStreamingDetails"] = serde_json::json!({"actualStartTime":"2026-11-03T10:00:00Z"});
        let status = video_status(&item, now);
        assert_eq!(status.current_state(now), State::Live);
        assert_eq!(status.current_state(now + chrono::Duration::seconds(121)), State::Unknown);
        assert_eq!(status.current_state(now - chrono::Duration::seconds(1)), State::Unknown);
        item["liveStreamingDetails"]["actualEndTime"] = "2026-11-03T11:00:00Z".into();
        assert_eq!(video_status(&item, now).state, State::Offline);
        assert_eq!(video_status(&serde_json::json!({"snippet":{"liveBroadcastContent":"upcoming"}}), now).state, State::Upcoming);
    }
}
