//! `--serve`: the same answers `--status` prints, over HTTP, plus a radar PNG.
//!
//! For the machine on a shelf with no display attached — Home Assistant polls it, a dashboard
//! embeds the snapshot, `curl` answers "is anything warned at home". It binds loopback unless told
//! otherwise: this speaks for your saved locations, and that is not something to put on a network
//! by accident.
//!
//! Hand-rolled over `std::net::TcpListener`, the same shape as the OAuth loopback in
//! [`crate::cloud`]. A handful of pollers do not justify an async HTTP stack, and the app already
//! carries `reqwest` as a client only.
//!
// ponytail: thread-per-connection blocking server; ceiling is a handful of pollers (Home
// Assistant plus curl). Bring in hyper's server features if concurrency ever matters.

use crate::status::{self, Spot};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long a JSON answer is reused before the feeds are asked again.
const JSON_TTL: Duration = Duration::from_secs(60);
/// How long a rendered radar PNG is reused. A volume is ~5 minutes wide, so is this.
const SNAPSHOT_TTL: Duration = Duration::from_secs(300);

struct Server {
    spots: Vec<Spot>,
    /// Bearer token every request must carry, or empty for the open behaviour. A user secret:
    /// it lives in settings.json (or the flag) and is never committed.
    token: String,
    /// Directory of static files to serve (the browser build), if this was started with one.
    web_root: Option<std::path::PathBuf>,
    rt: tokio::runtime::Runtime,
    http: reqwest::Client,
    /// path (plus query, for the snapshot) -> when it was fetched and what it was.
    cache: Mutex<HashMap<String, (Instant, Vec<u8>)>>,
    /// One radar render at a time; a render is seconds long and the JSON routes stay responsive.
    render: Mutex<()>,
}

/// Serve until killed. `bind` is an address like `127.0.0.1` or `0.0.0.0`.
pub fn run(
    spots: Vec<Spot>,
    bind: &str,
    port: u16,
    web_root: Option<std::path::PathBuf>,
    token: String,
) -> anyhow::Result<()> {
    let _ = STARTED.set(Instant::now());
    let listener = TcpListener::bind((bind, port))?;
    let server = Arc::new(Server {
        spots,
        token,
        web_root,
        // One runtime and one client for the process, not one per request like the headless
        // verifiers build — this one stays up.
        rt: tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?,
        http: reqwest::Client::new(),
        cache: Mutex::new(HashMap::new()),
        render: Mutex::new(()),
    });
    log::info!("serving on http://{bind}:{port}");
    if bind == "0.0.0.0" {
        if server.token.is_empty() {
            log::warn!("bound to all interfaces — anyone on this network can read your locations");
        } else {
            log::info!("bound to all interfaces, bearer token required");
        }
    }
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let server = Arc::clone(&server);
                std::thread::spawn(move || {
                    if let Err(e) = handle(&server, stream) {
                        log::debug!("connection ended: {e}");
                    }
                });
            }
            Err(e) => log::warn!("accept failed: {e}"),
        }
    }
    Ok(())
}

fn handle(server: &Server, mut stream: TcpStream) -> anyhow::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    // "GET /status.json?x=1 HTTP/1.1"
    let target = line.split_whitespace().nth(1).unwrap_or("/");
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p, q),
        None => (target, ""),
    };

    // The headers were ignored until there was a token to read out of them. Bounded: a client
    // that never sends the blank line, or sends a header wall, is hung up on rather than humoured.
    let mut authorized = server.token.is_empty();
    if !authorized {
        let supplied = query_token(query);
        for _ in 0..64 {
            let mut h = String::new();
            if reader.read_line(&mut h)? == 0 || h.trim().is_empty() {
                break;
            }
            if let Some((name, value)) = h.split_once(':') {
                if name.trim().eq_ignore_ascii_case("authorization") {
                    if let Some(bearer) = value.trim().strip_prefix("Bearer ") {
                        authorized |= constant_time_eq(bearer.trim(), &server.token);
                    }
                }
            }
        }
        // A dashboard that can only put things in a URL (a picture element, a widget) has no
        // header to set, so `?token=` is accepted too — on loopback, and for a token the user
        // chose to hand out.
        if let Some(t) = supplied {
            authorized |= constant_time_eq(&t, &server.token);
        }
    }
    let (status, ctype, body) = if authorized {
        route(server, path, query)
    } else {
        count("denied");
        (
            "401 Unauthorized",
            "application/json",
            br#"{"error":"missing or bad bearer token"}"#.to_vec(),
        )
    };
    stream.write_all(
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\n\
             Cache-Control: no-cache\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .as_bytes(),
    )?;
    stream.write_all(&body)?;
    Ok(())
}

fn route(server: &Server, path: &str, query: &str) -> (&'static str, &'static str, Vec<u8>) {
    count(match path {
        "/" => "index",
        "/status.json" | "/alerts.json" | "/obs.json" | "/health.json" => "json",
        "/cells.json" => "cells",
        "/snapshot.png" => "snapshot",
        "/loop.gif" | "/loop.mp4" => "loop",
        "/metrics" => "metrics",
        _ if path.starts_with("/proxy/") => "proxy",
        _ => "other",
    });
    match path {
        "/" if server.web_root.is_some() => static_file(server, "/index.html"),
        "/" => (
            "200 OK",
            "text/html; charset=utf-8",
            index(server).into_bytes(),
        ),
        "/status.json" | "/alerts.json" | "/obs.json" => match cached_json(server, path) {
            Ok(body) => ("200 OK", "application/json", body),
            Err(e) => error_json(e),
        },
        "/cells.json" => match cells_json(server, query) {
            Ok(body) => ("200 OK", "application/json", body),
            Err(e) => error_json(e),
        },
        "/health.json" => match health_json(server) {
            Ok(body) => ("200 OK", "application/json", body),
            Err(e) => error_json(e),
        },
        "/snapshot.png" => match snapshot(server, query) {
            Ok(png) => ("200 OK", "image/png", png),
            Err(e) => error_json(e),
        },
        "/loop.gif" => match loop_clip(server, query, crate::loopexport::LoopFormat::Gif) {
            Ok(body) => ("200 OK", "image/gif", body),
            Err(e) => error_json(e),
        },
        "/loop.mp4" => match loop_clip(server, query, crate::loopexport::LoopFormat::Mp4) {
            Ok(body) => ("200 OK", "video/mp4", body),
            Err(e) => error_json(e),
        },
        "/metrics" => (
            "200 OK",
            "text/plain; version=0.0.4; charset=utf-8",
            metrics(server).into_bytes(),
        ),
        _ if path.starts_with("/proxy/") => proxy(server, path, query),
        _ if server.web_root.is_some() => static_file(server, path),
        _ => not_found(),
    }
}

/// The `token=` query parameter, if the request carried one.
fn query_token(query: &str) -> Option<String> {
    crate::cloud::param(query, "token")
}

/// Compare without leaking where the two differ through timing. Length is not a secret here (the
/// user chose it), but the content is.
fn constant_time_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes()
        .zip(b.bytes())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

fn not_found() -> (&'static str, &'static str, Vec<u8>) {
    (
        "404 Not Found",
        "application/json",
        br#"{"error":"no such endpoint"}"#.to_vec(),
    )
}

/// Serve a file from `--web-root`. This is the trust boundary: a request path is attacker-chosen
/// text, so anything containing `..` is refused outright rather than normalized and hoped about.
fn static_file(server: &Server, path: &str) -> (&'static str, &'static str, Vec<u8>) {
    let Some(root) = &server.web_root else {
        return not_found();
    };
    let rel = path.trim_start_matches('/');
    if rel.contains("..") || rel.starts_with('/') || rel.contains('\\') {
        log::warn!("refused traversal attempt: {path}");
        return not_found();
    }
    let file = root.join(rel);
    match std::fs::read(&file) {
        Ok(body) => ("200 OK", content_type(&file), body),
        Err(_) => not_found(),
    }
}

/// Extension to content type. `application/wasm` is the load-bearing one — browsers refuse to
/// stream-compile a module served as anything else.
fn content_type(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("webmanifest") => "application/manifest+json",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        _ => "application/octet-stream",
    }
}

fn error_json(e: anyhow::Error) -> (&'static str, &'static str, Vec<u8>) {
    log::warn!("request failed: {e}");
    let body = serde_json::json!({ "error": e.to_string() }).to_string();
    ("503 Service Unavailable", "application/json", body.into())
}

/// The three JSON views, all built from one status report and cached together.
fn cached_json(server: &Server, path: &str) -> anyhow::Result<Vec<u8>> {
    if let Some(hit) = server
        .cache
        .lock()
        .unwrap()
        .get(path)
        .filter(|(when, _)| when.elapsed() < JSON_TTL)
    {
        return Ok(hit.1.clone());
    }
    let report = server
        .rt
        .block_on(status::collect(&server.http, &server.spots))?;
    let body = match path {
        "/alerts.json" => serde_json::to_vec(
            &report
                .iter()
                .map(|s| serde_json::json!({ "name": s.name, "alerts": s.alerts }))
                .collect::<Vec<_>>(),
        )?,
        "/obs.json" => serde_json::to_vec(
            &report
                .iter()
                .map(|s| {
                    let mut v = serde_json::to_value(s).unwrap_or_default();
                    if let Some(map) = v.as_object_mut() {
                        map.remove("alerts");
                    }
                    v
                })
                .collect::<Vec<_>>(),
        )?,
        _ => serde_json::to_vec(&report)?,
    };
    server
        .cache
        .lock()
        .unwrap()
        .insert(path.to_string(), (Instant::now(), body.clone()));
    Ok(body)
}

/// `GET /cells.json?site=KTLX` — every SCIT storm cell the radar's own algorithms are tracking.
///
/// The same four Level 3 products the storm-attributes table reads, in the same fields, plus the
/// position and forecast track a map needs. What `/status.json` cannot answer: it speaks for
/// saved locations, and "what storms exist near this radar" is a different question.
fn cells_json(server: &Server, query: &str) -> anyhow::Result<Vec<u8>> {
    let site = crate::cloud::param(query, "site").unwrap_or_else(|| "KTLX".to_string());
    if !site.chars().all(|c| c.is_ascii_alphanumeric()) {
        anyhow::bail!("bad site");
    }
    let site = site.to_ascii_uppercase();
    let key = format!("/cells.json?{site}");
    if let Some(hit) = server
        .cache
        .lock()
        .unwrap()
        .get(&key)
        .filter(|(when, _)| when.elapsed() < JSON_TTL)
    {
        return Ok(hit.1.clone());
    }
    let cells = server
        .rt
        .block_on(wxdata::level3::fetch_cells(&server.http, &site));
    let body = serde_json::to_vec(&serde_json::json!({
        "site": site,
        "cells": cells.iter().map(cell_json).collect::<Vec<_>>(),
    }))?;
    server
        .cache
        .lock()
        .unwrap()
        .insert(key, (Instant::now(), body.clone()));
    Ok(body)
}

/// One cell on the wire. Hand-written rather than derived: the wire format is a promise to
/// whatever is polling this, and it should not move because a struct field was renamed.
fn cell_json(c: &wxdata::level3::Cell) -> serde_json::Value {
    serde_json::json!({
        "id": c.title,
        "lon": c.lon,
        "lat": c.lat,
        "azimuth_deg": c.az_deg,
        "range_nm": c.range_nm,
        "movement_deg": c.mvt_deg,
        "movement_kt": c.mvt_kt,
        "max_dbz": c.max_dbz,
        "top_kft": c.top_kft,
        "vil": c.vil,
        "poh": c.poh,
        "posh": c.posh,
        "hail_in": c.hail_in,
        "tvs": c.tvs,
        "meso": c.meso,
        "track": c.track.iter().map(|t| serde_json::json!({
            "minutes": t.minutes,
            "lon": t.lon,
            "lat": t.lat,
        })).collect::<Vec<_>>(),
    })
}

/// `GET /health.json` — is this instance actually answering with fresh data?
///
/// A container that is up but has not reached a feed in an hour looks exactly like a healthy one
/// from the outside, which is the failure worth catching. Ages are in seconds; `null` means that
/// answer has never been built in this process.
fn health_json(server: &Server) -> anyhow::Result<Vec<u8>> {
    let age = |path: &str| -> Option<f64> {
        server
            .cache
            .lock()
            .unwrap()
            .get(path)
            .map(|(when, _)| when.elapsed().as_secs_f64())
    };
    // Ask for a status build if nothing has yet, so a fresh container reports on real feeds
    // rather than on never having tried.
    let feeds_ok = cached_json(server, "/status.json").is_ok();
    let body = serde_json::to_vec(&serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_secs": STARTED.get().map(|t| t.elapsed().as_secs()),
        "spots": server.spots.len(),
        "feeds_ok": feeds_ok,
        "status_age_secs": age("/status.json"),
        "snapshot_cached": server
            .cache
            .lock()
            .unwrap()
            .keys()
            .filter(|k| k.starts_with("/snapshot.png"))
            .count(),
    }))?;
    Ok(body)
}

/// When this process started serving, for `/health.json`.
static STARTED: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

/// The render knobs every image endpoint shares, parsed out of a query string.
struct Frame {
    site: String,
    moment: wxdata::level2::Moment,
    basemap: crate::tiles::BasemapStyle,
    px: u32,
    zoom: Option<f64>,
    zoom_tag: String,
    tilt: usize,
}

impl Frame {
    fn parse(query: &str) -> anyhow::Result<Frame> {
        let site = crate::cloud::param(query, "site").unwrap_or_else(|| "KTLX".to_string());
        let product = crate::cloud::param(query, "product").unwrap_or_else(|| "REF".to_string());
        if !site.chars().all(|c| c.is_ascii_alphanumeric()) {
            anyhow::bail!("bad site");
        }
        let moment = wxdata::level2::Moment::from_code(&product.to_ascii_uppercase())
            .ok_or_else(|| anyhow::anyhow!("unknown product '{product}'"))?;
        // A bare sweep on a blank field is unreadable on a dashboard; the keyless dark vector map
        // is the sane default, and `basemap=none` gets the bare sweep back.
        let basemap = crate::tiles::BasemapStyle::from_slug(
            &crate::cloud::param(query, "basemap").unwrap_or_else(|| "dark".to_string()),
        );
        // A dashboard wants a tile it can fit, and a widget wants its own framing; both are one
        // parameter each on a render that was already happening.
        let size: Option<u32> = crate::cloud::param(query, "size").and_then(|v| v.parse().ok());
        let zoom: Option<f64> = crate::cloud::param(query, "zoom").and_then(|v| v.parse().ok());
        // Elevation index, not degrees: the same number the app's tilt picker uses. A volume has
        // at most a couple of dozen sweeps, and asking past the end is a missing render, not a
        // crash.
        let tilt: usize = crate::cloud::param(query, "tilt")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
            .min(30);
        Ok(Frame {
            site,
            moment,
            basemap,
            px: size.unwrap_or(1000).clamp(256, 2048),
            zoom,
            zoom_tag: zoom.map_or_else(|| "auto".to_string(), |z| format!("{z:.2}")),
            tilt,
        })
    }

    /// Everything that changes the pixels, in one string — the cache key and the filename both
    /// have to carry all of it. The basemap used to be missing from the filename, so two styles
    /// of the same site raced over one file on disk.
    fn tag(&self) -> String {
        format!(
            "{}-{}-{}-{}-{}-{}",
            self.site,
            self.moment.short_name(),
            self.basemap.slug(),
            self.px,
            self.zoom_tag,
            self.tilt
        )
    }
}

/// Render one frame to `out` and hand back the PNG bytes. `at` picks an archived volume; `None`
/// is the latest one.
///
/// The renderer builds its own runtime, so it can only be called from a plain thread — which this
/// is, one connection per thread.
fn render_png(
    server: &Server,
    f: &Frame,
    at: Option<chrono::DateTime<chrono::Utc>>,
    out: &std::path::Path,
) -> anyhow::Result<Vec<u8>> {
    // `HH:MM`, with the colon: the archive picker parses on it, and a bare `1230` silently falls
    // through to "the latest volume" — which is a loop of the same frame six times.
    let hhmm = at.map(|t| t.format("%H:%M").to_string());
    {
        let _one_at_a_time = server.render.lock().unwrap();
        // Global knobs on the renderer, set under the same lock that serializes the render.
        crate::headless::set_output(Some(f.px), f.zoom);
        crate::headless::run(
            out.to_string_lossy().as_ref(),
            &f.site,
            f.moment,
            f.tilt,
            true,
            None,
            None,
            at.map(|t| t.date_naive()),
            hhmm.as_deref(),
            f.basemap,
            false,
        )?;
    }
    Ok(std::fs::read(out)?)
}

/// Where rendered frames are kept between requests.
fn snapshot_dir() -> anyhow::Result<std::path::PathBuf> {
    let dir = crate::paths::cache_dir()
        .ok_or_else(|| anyhow::anyhow!("no cache directory"))?
        .join("snapshots");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// A radar PNG through the same off-screen renderer the `--headless` verifier uses.
///
// ponytail: one render at a time; the palette is still fixed. Add it when someone asks.
fn snapshot(server: &Server, query: &str) -> anyhow::Result<Vec<u8>> {
    let f = Frame::parse(query)?;
    let key = format!("/snapshot.png?{}", f.tag());
    if let Some(hit) = server
        .cache
        .lock()
        .unwrap()
        .get(&key)
        .filter(|(when, _)| when.elapsed() < SNAPSHOT_TTL)
    {
        return Ok(hit.1.clone());
    }
    let out = snapshot_dir()?.join(format!("snapshot-{}.png", f.tag()));
    let png = render_png(server, &f, None, &out)?;
    server
        .cache
        .lock()
        .unwrap()
        .insert(key, (Instant::now(), png.clone()));
    Ok(png)
}

/// How far apart the frames of a loop are asked for. A volume is about five minutes wide; a site
/// in clear-air mode is slower, and two targets then land on the same volume — a repeated frame,
/// not an error.
const LOOP_STEP_MIN: i64 = 5;
/// Rendered loop frames older than this are never wanted again — the window only slides forward.
const LOOP_FRAME_TTL: Duration = Duration::from_secs(2 * 3600);

/// A radar loop as a GIF (or MP4), for the dashboard that wants motion rather than a still.
///
/// This is the Home Assistant camera endpoint: point a `generic` camera or a picture card at it
/// and the last half hour animates. Each frame is a full render, so the answer is cached for the
/// same window a snapshot is, and frames already on disk are reused — a poll a minute later
/// renders one new frame, not six.
///
// ponytail: frames are picked by wall-clock steps rather than by listing the site's volumes,
// which reuses the archive path the timeline already uses. Listing volumes would give exact
// frames; it also gives a second network round trip per request, for a dashboard that cannot
// tell the difference.
fn loop_clip(
    server: &Server,
    query: &str,
    format: crate::loopexport::LoopFormat,
) -> anyhow::Result<Vec<u8>> {
    let f = Frame::parse(query)?;
    // Six frames is half an hour of storm, and the ceiling is there because each one is a full
    // render on a machine that is probably also serving snapshots.
    let count: usize = crate::cloud::param(query, "frames")
        .and_then(|v| v.parse().ok())
        .unwrap_or(6)
        .clamp(2, 12);
    let fps: u32 = crate::cloud::param(query, "fps")
        .and_then(|v| v.parse().ok())
        .unwrap_or(2)
        .clamp(1, 10);
    let ext = match format {
        crate::loopexport::LoopFormat::Gif => "gif",
        crate::loopexport::LoopFormat::Mp4 => "mp4",
    };

    let now = chrono::Utc::now();
    let key = format!("/loop.{ext}?{}-{count}-{fps}", f.tag());
    if let Some(hit) = server
        .cache
        .lock()
        .unwrap()
        .get(&key)
        .filter(|(when, _)| when.elapsed() < SNAPSHOT_TTL)
    {
        return Ok(hit.1.clone());
    }

    let dir = snapshot_dir()?;
    let mut frames = Vec::with_capacity(count);
    let mut last: Option<Vec<u8>> = None;
    for k in (0..count as i64).rev() {
        let at = now - chrono::Duration::minutes(k * LOOP_STEP_MIN);
        let out = dir.join(format!("loop-{}-{}.png", f.tag(), at.format("%Y%m%d-%H%M")));
        // An archived frame never changes, so one already on disk is the answer. Only the newest
        // step is rendered on a repeat poll.
        let png = match std::fs::read(&out) {
            Ok(bytes) if !bytes.is_empty() => bytes,
            _ => render_png(server, &f, Some(at), &out)?,
        };
        // Two steps can land on the same volume — a site scanning every 3.5 minutes has no frame
        // newer than the one three minutes old, so the head of the window repeats it. Identical
        // consecutive frames are a stutter in the loop, not information.
        if last.as_deref() == Some(png.as_slice()) {
            continue;
        }
        frames.push(image::load_from_memory(&png)?.to_rgba8());
        last = Some(png);
    }
    prune_loop_frames(&dir);

    let clip = dir.join(format!("loop-{}.{ext}", f.tag()));
    match format {
        crate::loopexport::LoopFormat::Gif => {
            crate::loopexport::encode_gif(&frames, (1000 / fps) as u16, &clip)?
        }
        crate::loopexport::LoopFormat::Mp4 => crate::loopexport::encode_mp4(&frames, fps, &clip)?,
    }
    let body = std::fs::read(&clip)?;
    server
        .cache
        .lock()
        .unwrap()
        .insert(key, (Instant::now(), body.clone()));
    Ok(body)
}

/// Drop loop frames that have slid out of every window. Without this the cache directory grows by
/// a render every five minutes for as long as the box is up.
fn prune_loop_frames(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        if !e.file_name().to_string_lossy().starts_with("loop-") {
            continue;
        }
        let stale = e
            .metadata()
            .and_then(|m| m.modified())
            .map(|t| t.elapsed().unwrap_or_default() > LOOP_FRAME_TTL)
            .unwrap_or(false);
        if stale {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// The route labels `/metrics` reports, in counter order.
const ROUTES: [&str; 9] = [
    "index", "json", "cells", "snapshot", "loop", "metrics", "proxy", "denied", "other",
];
/// One counter per label in [`ROUTES`], bumped on every request in [`route`].
///
// ponytail: flat process-lifetime counters, no histograms or per-status buckets; the ceiling is
// "how busy is this box", and a real client-latency question wants the prometheus crate anyway.
static REQUESTS: [AtomicU64; ROUTES.len()] = [const { AtomicU64::new(0) }; ROUTES.len()];

fn count(label: &str) {
    if let Some(i) = ROUTES.iter().position(|r| *r == label) {
        REQUESTS[i].fetch_add(1, Ordering::Relaxed);
    }
}

/// A Prometheus label value, escaped. **Trust boundary**: spot names are whatever the user typed
/// into the marker dialog, and a stray quote or newline there would otherwise forge label pairs —
/// or whole metric lines — in the scrape. Backslash first, so it does not double-escape the
/// escapes added after it.
fn escape_label(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

/// The text exposition format, hand-rolled.
///
/// Counters come from [`REQUESTS`]; the gauges are derived on scrape from the same status report
/// `/status.json` serves, so a scrape costs at most one round of feed requests per [`JSON_TTL`]
/// and usually costs nothing.
///
// ponytail: the report is read back out of the JSON cache as `serde_json::Value` rather than
// caching the typed `Vec<SpotStatus>` alongside it — one parse of a few KB per scrape. Cache the
// struct if the gauge set ever grows past a handful of fields.
fn metrics(server: &Server) -> String {
    let mut out = String::new();
    out.push_str("# HELP hookecho_http_requests_total Requests served, by route.\n");
    out.push_str("# TYPE hookecho_http_requests_total counter\n");
    for (label, n) in ROUTES.iter().zip(&REQUESTS) {
        let n = n.load(Ordering::Relaxed);
        let label = escape_label(label);
        out.push_str(&format!(
            "hookecho_http_requests_total{{route=\"{label}\"}} {n}\n"
        ));
    }

    let Ok(body) = cached_json(server, "/status.json") else {
        // A feed being down is not a reason to fail the scrape — the counters above still answer.
        return out;
    };
    let age = server
        .cache
        .lock()
        .unwrap()
        .get("/status.json")
        .map(|(when, _)| when.elapsed().as_secs_f64())
        .unwrap_or(0.0);
    let Ok(report) = serde_json::from_slice::<Vec<serde_json::Value>>(&body) else {
        return out;
    };

    out.push_str("# HELP hookecho_spot_temp_c Current temperature at a saved location.\n");
    out.push_str("# TYPE hookecho_spot_temp_c gauge\n");
    for spot in &report {
        let (Some(name), Some(f)) = (
            spot.get("name").and_then(|v| v.as_str()),
            spot.get("temp_f").and_then(|v| v.as_f64()),
        ) else {
            continue;
        };
        let c = (f - 32.0) * 5.0 / 9.0;
        out.push_str(&format!(
            "hookecho_spot_temp_c{{spot=\"{}\"}} {c:.2}\n",
            escape_label(name)
        ));
    }

    out.push_str("# HELP hookecho_spot_alerts Active NWS alerts within a location's radius.\n");
    out.push_str("# TYPE hookecho_spot_alerts gauge\n");
    for spot in &report {
        let (Some(name), Some(alerts)) = (
            spot.get("name").and_then(|v| v.as_str()),
            spot.get("alerts").and_then(|v| v.as_array()),
        ) else {
            continue;
        };
        out.push_str(&format!(
            "hookecho_spot_alerts{{spot=\"{}\"}} {}\n",
            escape_label(name),
            alerts.len()
        ));
    }

    out.push_str("# HELP hookecho_data_age_seconds How stale the reported conditions are.\n");
    out.push_str("# TYPE hookecho_data_age_seconds gauge\n");
    out.push_str(&format!("hookecho_data_age_seconds {age:.1}\n"));
    out
}

/// Hosts the CORS proxy will fetch from, exact match only.
///
/// The browser build cannot read most of these directly — NOAA's buckets and the NWS API send no
/// `Access-Control-Allow-Origin` — so the page asks its own origin instead. Compiled in rather
/// than configured: this is the whole security model, and a runtime knob would be one typo away
/// from an open relay.
///
// ponytail: hand-maintained list, so a new feed host means a new entry here; a
// `--proxy-host` flag is the upgrade path if third-party placefiles ever need proxying.
const ALLOWED_HOSTS: &[&str] = &[
    // NEXRAD / TDWR archives and the live chunk stream.
    "unidata-nexrad-level2.s3.amazonaws.com",
    "unidata-nexrad-level2-chunks.s3.amazonaws.com",
    "unidata-nexrad-level3.s3.amazonaws.com",
    // Gridded and satellite feeds.
    "noaa-mrms-pds.s3.amazonaws.com",
    "noaa-hrrr-bdp-pds.s3.amazonaws.com",
    "noaa-rap-pds.s3.amazonaws.com",
    "noaa-nam-pds.s3.amazonaws.com",
    "noaa-nbm-grib2-pds.s3.amazonaws.com",
    "noaa-goes19.s3.amazonaws.com",
    "noaa-goes18.s3.amazonaws.com",
    "noaa-gfs-bdp-pds.s3.amazonaws.com",
    "data.ecmwf.int",
    "mrms.ncep.noaa.gov",
    "www.nohrsc.noaa.gov",
    // NWS and friends.
    "api.weather.gov",
    "tgftp.nws.noaa.gov",
    "mapservices.weather.noaa.gov",
    "www.spc.noaa.gov",
    "www.nhc.noaa.gov",
    "www.ndbc.noaa.gov",
    "api.water.noaa.gov",
    "aviationweather.gov",
    "tfr.faa.gov",
    "services.dat.noaa.gov",
    "apps.dat.noaa.gov",
    "mesonet.agron.iastate.edu",
    "weather.uwyo.edu",
    "mping.ou.edu",
    "www.spotternetwork.org",
    "api.open-meteo.com",
    "gibs.earthdata.nasa.gov",
    // European radar.
    "opendata.dwd.de",
    // EUMETNET OpenRadarData: the ODIM volumes the OPERA network publishes, plus the
    // bucket listing that names the newest one.
    "s3.waw3-1.cloudferro.com",
    // Basemap and imagery tiles.
    "api.mapbox.com",
    "api.maptiler.com",
    "basemaps.cartocdn.com",
    "basemap.nationalmap.gov",
    "server.arcgisonline.com",
    "services3.arcgis.com",
    "tiles.openfreemap.org",
    "tile.openstreetmap.org",
    "a.tile.openstreetmap.fr",
    "a.tile-cyclosm.openstreetmap.fr",
    "a.tile.opentopomap.org",
    // Cameras.
    "weathercams.faa.gov",
    "images.wcams-static.faa.gov",
    "cwwp2.dot.ca.gov",
];

/// Nothing bigger than this comes back through the proxy. An archive volume is a few MB; this is
/// generous for every feed on the list and still bounds a hostile or broken upstream.
const PROXY_MAX_BYTES: usize = 64 * 1024 * 1024;

/// `GET /proxy/{host}/{rest}` → `https://{host}/{rest}`, for the browser build.
///
/// The trust boundary, in four rules: the host must be in [`ALLOWED_HOSTS`] exactly, only GET is
/// ever issued upstream (this server never speaks another method), no client header reaches the
/// upstream, and the response is capped and stripped down to a known content type. Same-origin by
/// construction, so no CORS header of our own is needed.
fn proxy(server: &Server, path: &str, query: &str) -> (&'static str, &'static str, Vec<u8>) {
    let forbidden = |why: &str| -> (&'static str, &'static str, Vec<u8>) {
        log::warn!("proxy refused {path}: {why}");
        (
            "403 Forbidden",
            "application/json",
            br#"{"error":"host not proxyable"}"#.to_vec(),
        )
    };
    let Some((host, rest)) = path["/proxy/".len()..].split_once('/') else {
        return forbidden("no path after host");
    };
    if !ALLOWED_HOSTS.contains(&host) {
        return forbidden("host not in allowlist");
    }
    let url = if query.is_empty() {
        format!("https://{host}/{rest}")
    } else {
        format!("https://{host}/{rest}?{query}")
    };
    match server.rt.block_on(fetch_capped(&server.http, &url)) {
        Ok((ctype, body)) => ("200 OK", ctype, body),
        Err(e) => {
            log::warn!("proxy fetch of {url} failed: {e}");
            (
                "502 Bad Gateway",
                "application/json",
                br#"{"error":"upstream fetch failed"}"#.to_vec(),
            )
        }
    }
}

/// One upstream GET, no inherited headers, stopped at [`PROXY_MAX_BYTES`] mid-stream rather than
/// after the fact.
async fn fetch_capped(
    http: &reqwest::Client,
    url: &str,
) -> anyhow::Result<(&'static str, Vec<u8>)> {
    let mut resp = http.get(url).send().await?.error_for_status()?;
    let ctype = proxy_content_type(
        resp.headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or(""),
    );
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await? {
        if body.len() + chunk.len() > PROXY_MAX_BYTES {
            anyhow::bail!("response over {PROXY_MAX_BYTES} bytes");
        }
        body.extend_from_slice(&chunk);
    }
    Ok((ctype, body))
}

/// An upstream `Content-Type` mapped onto one of ours. The upstream header is remote text going
/// into our response headers, so it is matched against a fixed set rather than copied — a copied
/// `\r\n` would let an upstream write headers of its own.
fn proxy_content_type(upstream: &str) -> &'static str {
    match upstream.split(';').next().unwrap_or("").trim() {
        "application/json" | "application/geo+json" => "application/json",
        "application/xml" | "text/xml" => "application/xml",
        "text/plain" => "text/plain; charset=utf-8",
        // HTML is deliberately downgraded: proxied bytes are served from our own origin, and no
        // feed on the list needs a page that a browser will execute.
        "text/html" => "text/plain; charset=utf-8",
        "image/png" => "image/png",
        "image/jpeg" => "image/jpeg",
        "image/webp" => "image/webp",
        "application/x-protobuf" | "application/vnd.mapbox-vector-tile" => "application/x-protobuf",
        _ => "application/octet-stream",
    }
}

/// The page `/` serves when there is no `--web-root`: a dashboard rather than a list of links.
///
/// Everything on it comes from the endpoints below it — the radar render, the conditions and the
/// alerts — and it refreshes itself, so a spare monitor on the wall stays current without anyone
/// touching it.
///
/// The rows are built from `/status.json` in the browser with `textContent`, never by pasting
/// JSON into HTML: a spot name is whatever the user typed into the marker dialog, and this page
/// is served from the same origin as the token'd endpoints.
///
// ponytail: one page, no framework, no build step — it is served as a string literal from the
// binary. If it ever wants charts, it wants `--web-root` and the real app instead.
fn index(server: &Server) -> String {
    // The nearest radar to home, so the picture is of somewhere the user cares about.
    let site = server
        .spots
        .iter()
        .find(|s| s.home)
        .or_else(|| server.spots.first())
        .and_then(|s| crate::geo::nearest_site_id(s.lon, s.lat))
        .unwrap_or_else(|| "KTLX".to_string());
    let version = env!("CARGO_PKG_VERSION");
    format!(
        r##"<!doctype html><html lang="en"><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>HookEcho</title>
<!-- No icon to serve, and without this the browser asks for /favicon.ico on every load and logs
     the 404 it gets. -->
<link rel="icon" href="data:,">
<style>
:root {{ color-scheme: dark; }}
body {{ margin:0; font:15px/1.5 system-ui,sans-serif; background:#0e1116; color:#e6edf3;
  display:grid; grid-template-columns:minmax(320px,1fr) minmax(280px,420px); gap:16px; padding:16px; }}
@media (max-width:800px) {{ body {{ grid-template-columns:1fr; }} }}
h1 {{ font-size:16px; margin:0 0 8px; font-weight:600; letter-spacing:.02em; }}
img {{ width:100%; border-radius:10px; display:block; background:#161b22; }}
.card {{ background:#161b22; border:1px solid #21262d; border-radius:10px; padding:12px; margin-bottom:12px; }}
.spot {{ font-weight:600; }}
.cond {{ color:#9aa7b4; }}
.alert {{ margin-top:6px; padding:6px 8px; border-radius:6px; background:#3d1d1d; border-left:3px solid #f85149; }}
.alert.warn {{ background:#3d3319; border-left-color:#d29922; }}
.cell {{ margin-top:6px; color:#9aa7b4; }}
footer {{ grid-column:1/-1; color:#6e7681; font-size:13px; }}
a {{ color:#58a6ff; }}
</style>
<main>
  <h1>Radar — <span id="site">{site}</span></h1>
  <img id="radar" alt="radar">
</main>
<aside>
  <h1>Conditions</h1>
  <div id="spots"><div class="card">loading…</div></div>
</aside>
<footer>
  hookecho {version} · <span id="when">—</span> ·
  <a href="/status.json">status</a> · <a href="/cells.json?site={site}">cells</a> ·
  <a href="/loop.gif?site={site}">loop</a> · <a href="/health.json">health</a> ·
  <a href="/metrics">metrics</a>
</footer>
<script>
// A token'd server is reached with `?token=`; every fetch this page makes has to carry it too.
const token = new URLSearchParams(location.search).get("token");
const q = (path, extra) => path + (path.includes("?") ? "&" : "?") +
  (token ? "token=" + encodeURIComponent(token) + "&" : "") + extra;
const site = document.getElementById("site").textContent;

function radar() {{
  // The server caches a render for five minutes; the cache-buster is what stops the browser
  // holding on to it for longer than that.
  document.getElementById("radar").src = q("/snapshot.png", "site=" + site + "&size=900&t=" + Date.now());
}}

function row(s) {{
  const card = document.createElement("div");
  card.className = "card";
  const name = document.createElement("div");
  name.className = "spot";
  name.textContent = s.name;
  card.append(name);
  const bits = [];
  if (s.temp_f != null) bits.push(Math.round(s.temp_f) + "°F");
  if (s.dewpoint_f != null) bits.push("dew " + Math.round(s.dewpoint_f) + "°");
  if (s.wind_kt != null) bits.push((s.wind_dir || "") + " " + Math.round(s.wind_kt) + " kt");
  const cond = document.createElement("div");
  cond.className = "cond";
  cond.textContent = bits.length ? bits.join(" · ") : "no observations";
  card.append(cond);
  for (const a of s.alerts || []) {{
    const el = document.createElement("div");
    el.className = "alert" + (a.escalation > 0 ? "" : " warn");
    el.textContent = a.event + (a.until ? " until " + a.until : "");
    card.append(el);
  }}
  if (s.nearest_cell) {{
    const c = document.createElement("div");
    c.className = "cell";
    c.textContent = "storm " + s.nearest_cell.id + " " + Math.round(s.nearest_cell.distance_km) +
      " km at " + Math.round(s.nearest_cell.bearing_deg) + "°";
    card.append(c);
  }}
  return card;
}}

async function refresh() {{
  const box = document.getElementById("spots");
  try {{
    const r = await fetch(q("/status.json", ""));
    if (!r.ok) throw new Error(r.status + "");
    const report = await r.json();
    box.replaceChildren(...report.map(row));
    document.getElementById("when").textContent = new Date().toLocaleTimeString();
  }} catch (e) {{
    // A failed poll must not blank the last good answer — a stale reading beats an empty page.
    document.getElementById("when").textContent = "offline (" + e.message + ")";
  }}
}}

radar();
refresh();
// The answers behind these are cached for a minute and five minutes; polling faster only spends
// the browser's battery.
setInterval(refresh, 60000);
setInterval(radar, 300000);
</script>
</html>"##
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_token_check_is_exact_and_length_safe() {
        assert!(constant_time_eq("hunter2", "hunter2"));
        assert!(!constant_time_eq("hunter2", "hunter3"));
        assert!(!constant_time_eq("hunter2", "hunter22"));
        assert!(!constant_time_eq("", "x"));
        // An empty configured token means the server is open; that decision is made before this
        // is ever called, but two empties must not compare unequal.
        assert!(constant_time_eq("", ""));
        // A dashboard's `?token=` form.
        assert_eq!(query_token("site=KTLX&token=abc"), Some("abc".to_string()));
        assert_eq!(query_token("site=KTLX"), None);
    }

    #[test]
    fn a_frame_carries_everything_that_changes_the_pixels() {
        let f = Frame::parse("site=KOUN&product=vel&size=512&zoom=8&tilt=2&basemap=light").unwrap();
        assert_eq!(f.site, "KOUN");
        assert_eq!(f.px, 512);
        assert_eq!(f.tilt, 2);
        // Two renders that differ in any knob must not share a cache key or a filename.
        let other =
            Frame::parse("site=KOUN&product=vel&size=512&zoom=8&tilt=3&basemap=light").unwrap();
        assert_ne!(f.tag(), other.tag());
        assert!(
            !f.tag().contains(['/', '\\', ' ']),
            "tag becomes a filename"
        );

        // Defaults are the dashboard case: latest reflectivity on the dark vector map.
        let d = Frame::parse("").unwrap();
        assert_eq!(d.site, "KTLX");
        assert_eq!(d.zoom_tag, "auto");

        // A site is pasted into a URL by whoever is looking; it also becomes a path segment.
        assert!(Frame::parse("site=../../etc").is_err());
        assert!(Frame::parse("product=NOPE").is_err());
    }

    #[test]
    fn the_dashboard_points_at_the_radar_nearest_home() {
        let server = Server {
            token: String::new(),
            spots: vec![Spot {
                name: "home".to_string(),
                lat: 35.22,
                lon: -97.44,
                radius_mi: 20.0,
                home: true,
            }],
            web_root: None,
            rt: tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap(),
            http: reqwest::Client::new(),
            cache: Mutex::new(HashMap::new()),
            render: Mutex::new(()),
        };
        let page = index(&server);
        assert!(page.contains("KTLX"), "Norman's nearest radar is KTLX");
        // The page is the only thing served at `/` without a web root, so it has to carry its own
        // refresh — a wall display nobody touches is the whole point.
        assert!(page.contains("setInterval"));
        // Spot names reach the page as `textContent`, never pasted into HTML.
        assert!(page.contains("name.textContent = s.name"));
    }

    #[test]
    fn unknown_paths_404() {
        // A server with no spots: routing is independent of what it would report.
        let server = Server {
            token: String::new(),
            spots: Vec::new(),
            web_root: None,
            rt: tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap(),
            http: reqwest::Client::new(),
            cache: Mutex::new(HashMap::new()),
            render: Mutex::new(()),
        };
        let (status, ctype, body) = route(&server, "/etc/passwd", "");
        assert_eq!(status, "404 Not Found");
        assert_eq!(ctype, "application/json");
        assert!(String::from_utf8_lossy(&body).contains("no such endpoint"));

        let (status, ..) = route(&server, "/", "");
        assert_eq!(status, "200 OK");
    }

    #[test]
    fn static_serving_refuses_to_walk_out_of_the_web_root() {
        let server = Server {
            token: String::new(),
            spots: Vec::new(),
            web_root: Some(std::path::PathBuf::from("/var/empty")),
            rt: tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap(),
            http: reqwest::Client::new(),
            cache: Mutex::new(HashMap::new()),
            render: Mutex::new(()),
        };
        for path in [
            "/../../etc/passwd",
            "/dist/../../../etc/shadow",
            "/..%2f..%2fetc/passwd",
            "//etc/passwd",
        ] {
            let (status, ..) = route(&server, path, "");
            assert_eq!(status, "404 Not Found", "{path} must not be served");
        }
        assert_eq!(
            content_type(std::path::Path::new("a/b.wasm")),
            "application/wasm"
        );
        // A manifest served as octet-stream is a manifest the browser ignores, and the app then
        // silently stops being installable.
        assert_eq!(
            content_type(std::path::Path::new("manifest.webmanifest")),
            "application/manifest+json"
        );
    }

    #[test]
    fn proxy_refuses_hosts_that_are_not_on_the_list() {
        let server = Server {
            token: String::new(),
            spots: Vec::new(),
            web_root: None,
            rt: tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap(),
            http: reqwest::Client::new(),
            cache: Mutex::new(HashMap::new()),
            render: Mutex::new(()),
        };
        for path in [
            "/proxy/evil.example.com/x",
            // Suffix and prefix games against a host that *is* allowed: match is exact.
            "/proxy/api.weather.gov.evil.example.com/alerts",
            "/proxy/evil.example.com#api.weather.gov/alerts",
            "/proxy/api.weather.gov",
        ] {
            let (status, ..) = route(&server, path, "");
            assert_eq!(status, "403 Forbidden", "{path} must not be proxied");
        }
    }

    #[test]
    fn label_values_cannot_forge_metric_lines() {
        assert_eq!(escape_label("Home"), "Home");
        assert_eq!(
            escape_label("a\"} 1\nhookecho_spot_alerts{spot=\"b"),
            "a\\\"} 1\\nhookecho_spot_alerts{spot=\\\"b"
        );
        // Backslash first, or the escapes below it get escaped twice.
        assert_eq!(escape_label(r#"c:\tmp"#), r"c:\\tmp");
    }

    #[test]
    fn requests_are_counted_by_route() {
        let server = Server {
            token: String::new(),
            spots: Vec::new(),
            web_root: None,
            rt: tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap(),
            http: reqwest::Client::new(),
            cache: Mutex::new(HashMap::new()),
            render: Mutex::new(()),
        };
        let i = ROUTES.iter().position(|r| *r == "index").unwrap();
        let before = REQUESTS[i].load(Ordering::Relaxed);
        route(&server, "/", "");
        // Counters are process-wide and the test threads share them, so this asserts movement
        // rather than an exact delta — another test routing "/" must not fail this one.
        assert!(REQUESTS[i].load(Ordering::Relaxed) > before);
    }
}
