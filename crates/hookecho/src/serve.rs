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
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long a JSON answer is reused before the feeds are asked again.
const JSON_TTL: Duration = Duration::from_secs(60);
/// How long a rendered radar PNG is reused. A volume is ~5 minutes wide, so is this.
const SNAPSHOT_TTL: Duration = Duration::from_secs(300);
/// Edge length of the national mosaic. Wider than a site frame because it carries the whole
/// country, and it is the one image on the landing page.
const NATIONAL_PX: u32 = 1400;
/// Zoom for that frame. The verifier's 4.0 frames CONUS at 1000 px; a wider image at the same
/// zoom just adds ocean, so this buys the extra pixels back as detail.
const NATIONAL_ZOOM: f64 = 4.5;
/// How many built answers are kept. The snapshot key carries `px` and `zoom`, which the client
/// picks, so this cannot be a map that only grows — a poller walking zoom levels would otherwise
/// pin every render it ever asked for.
const ANSWER_CACHE: usize = 64;
/// How many proxied responses are kept, and how many bytes they may take between them. An archive
/// volume is a few MB, so the byte bound is what actually binds; the entry count stops a run of
/// tiny tiles from holding a long tail.
const PROXY_CACHE_ENTRIES: usize = 64;
const PROXY_CACHE_BYTES: usize = 256 * 1024 * 1024;

struct Server {
    spots: Vec<Spot>,
    /// Bearer token every request must carry, or empty for the open behaviour. A user secret:
    /// it lives in settings.json (or the flag) and is never committed.
    token: String,
    /// Whether unauthenticated callers may have the preset frames (`--public`). Off by default:
    /// a server with a token answers nothing without it.
    public: bool,
    /// Directory of static files to serve (the browser build), if this was started with one.
    web_root: Option<std::path::PathBuf>,
    rt: tokio::runtime::Runtime,
    http: reqwest::Client,
    /// path (plus query, for the snapshot) -> when it was fetched and what it was.
    cache: Mutex<lru::LruCache<String, (Instant, Vec<u8>)>>,
    /// Proxied upstream responses, so one visitor's volume download is the next visitor's cache
    /// hit — which is what the note in `wxdata::net` promised and this server was not doing.
    proxy_cache: Mutex<lru::LruCache<String, ProxyHit>>,
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
    public: bool,
) -> anyhow::Result<()> {
    let _ = STARTED.set(Instant::now());
    let listener = TcpListener::bind((bind, port))?;
    let server = Arc::new(Server {
        spots,
        token,
        public,
        web_root,
        // One runtime and one client for the process, not one per request like the headless
        // verifiers build — this one stays up.
        rt: tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?,
        http: reqwest::Client::new(),
        cache: Mutex::new(lru::LruCache::new(
            std::num::NonZeroUsize::new(ANSWER_CACHE).unwrap(),
        )),
        proxy_cache: Mutex::new(lru::LruCache::new(
            std::num::NonZeroUsize::new(PROXY_CACHE_ENTRIES).unwrap(),
        )),
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

    // The headers were ignored until there was a token to read out of them, and now a proxied
    // response can be revalidated too. Bounded: a client that never sends the blank line, or sends
    // a header wall, is hung up on rather than humoured.
    let mut authorized = server.token.is_empty();
    let mut if_none_match = None;
    {
        let supplied = query_token(query);
        for _ in 0..64 {
            let mut h = String::new();
            if reader.read_line(&mut h)? == 0 || h.trim().is_empty() {
                break;
            }
            let Some((name, value)) = h.split_once(':') else {
                continue;
            };
            let name = name.trim();
            if name.eq_ignore_ascii_case("authorization") {
                if let Some(bearer) = value.trim().strip_prefix("Bearer ") {
                    authorized |= constant_time_eq(bearer.trim(), &server.token);
                }
            } else if name.eq_ignore_ascii_case("if-none-match") {
                if_none_match = Some(value.trim().to_string());
            }
        }
        // A dashboard that can only put things in a URL (a picture element, a widget) has no
        // header to set, so `?token=` is accepted too — on loopback, and for a token the user
        // chose to hand out.
        if let Some(t) = supplied {
            authorized |= constant_time_eq(&t, &server.token);
        }
    }
    let reply = if authorized {
        route(server, path, query, if_none_match.as_deref())
    } else if server.public {
        // The public hostname serves the fixed frames the site embeds and nothing else. Anything
        // else — another size, another product, a status page — needs the token, which is how the
        // owner keeps the full parameter surface without handing it to the internet.
        if is_preset(path, query) {
            route(server, path, query, if_none_match.as_deref())
        } else {
            count("denied");
            (
                "403 Forbidden",
                "application/json",
                br#"{"error":"presets only"}"#.to_vec(),
            )
                .into()
        }
    } else {
        count("denied");
        (
            "401 Unauthorized",
            "application/json",
            br#"{"error":"missing or bad bearer token"}"#.to_vec(),
        )
            .into()
    };
    let Reply {
        status,
        ctype,
        body,
        headers,
    } = reply;
    let mut head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    // `no-cache` stays the default for everything this server builds itself — those answers speak
    // for saved locations and go stale in a minute. A route that knows better says so.
    if !headers.iter().any(|(n, _)| *n == "Cache-Control") {
        head.push_str("Cache-Control: no-cache\r\n");
    }
    // Every name and value here is ours (a TTL, a hex digest) — no upstream text reaches a header,
    // which is the same rule `proxy_content_type` enforces for the content type.
    for (name, value) in &headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(&body)?;
    Ok(())
}

/// A response, before it is written.
///
/// Most routes have only three things to say and stay tuples; `headers` exists because `/proxy/`
/// has a `Cache-Control` and an `ETag` of its own, and defaulting it keeps every other arm as it
/// was.
struct Reply {
    status: &'static str,
    ctype: &'static str,
    body: Vec<u8>,
    headers: Vec<(&'static str, String)>,
}

impl From<(&'static str, &'static str, Vec<u8>)> for Reply {
    fn from((status, ctype, body): (&'static str, &'static str, Vec<u8>)) -> Self {
        Self {
            status,
            ctype,
            body,
            headers: Vec::new(),
        }
    }
}

/// Whether an unauthenticated request on a `--public` server is one of the frames the site
/// embeds.
///
/// The site registry is the allowlist: one default frame per known radar, plus the national
/// mosaic and the health page. There is no preset file to keep in step, and no way to ask for a
/// 2048 px render of an arbitrary product by guessing a URL — every knob but `site` has to be
/// absent, and `site` has to be a radar we know.
fn is_preset(path: &str, query: &str) -> bool {
    // `t=` is the cache-buster the page appends; it changes no pixel and is ignored everywhere.
    let keys: Vec<&str> = query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| p.split_once('=').map_or(p, |(k, _)| k))
        .filter(|k| *k != "t")
        .collect();
    match path {
        "/health.json" | "/national.png" | "/national.mp4" => keys.is_empty(),
        "/snapshot.png" | "/loop.gif" | "/loop.mp4" => {
            // `product` is the one knob the site actually turns, and only between the two
            // moments a viewer reads: what it looks like, and which way it's moving.
            if keys != ["site"] && keys != ["site", "product"] && keys != ["product", "site"] {
                return false;
            }
            if let Some(product) = crate::cloud::param(query, "product") {
                if !matches!(product.as_str(), "REF" | "VEL") {
                    return false;
                }
            }
            let Some(site) = crate::cloud::param(query, "site") else {
                return false;
            };
            if wxdata::sites::site_by_id(&site).is_none() {
                return false;
            }
            // A loop steps the volume archive, which only NEXRAD has.
            if path.starts_with("/loop.") && !wxdata::sites::is_nexrad(&site) {
                return false;
            }
            true
        }
        _ => false,
    }
}

/// A rendered image, with the one header that lets a CDN in front of this server do its job.
///
/// The default is `no-cache` (see `respond`), which is right for the JSON routes and wrong for a
/// render: the same frame is reused for `SNAPSHOT_TTL` here, so saying so out loud costs nothing
/// and keeps repeat views off the box entirely.
fn image_reply(ctype: &'static str, body: Vec<u8>) -> Reply {
    Reply {
        status: "200 OK",
        ctype,
        body,
        headers: vec![(
            "Cache-Control",
            format!("public, max-age={}", SNAPSHOT_TTL.as_secs()),
        )],
    }
}

fn route(server: &Server, path: &str, query: &str, if_none_match: Option<&str>) -> Reply {
    count(match path {
        "/" => "index",
        "/status.json" | "/alerts.json" | "/obs.json" | "/health.json" => "json",
        "/cells.json" => "cells",
        "/snapshot.png" => "snapshot",
        "/national.png" | "/national.mp4" => "national",
        "/loop.gif" | "/loop.mp4" => "loop",
        "/metrics" => "metrics",
        _ if path.starts_with("/proxy/") => "proxy",
        _ => "other",
    });
    match path {
        "/" if server.web_root.is_some() => static_file(server, "/index.html").into(),
        "/" => (
            "200 OK",
            "text/html; charset=utf-8",
            index(server).into_bytes(),
        )
            .into(),
        "/status.json" | "/alerts.json" | "/obs.json" => match cached_json(server, path) {
            Ok(body) => ("200 OK", "application/json", body).into(),
            Err(e) => error_json(e).into(),
        },
        "/cells.json" => match cells_json(server, query) {
            Ok(body) => ("200 OK", "application/json", body).into(),
            Err(e) => error_json(e).into(),
        },
        "/health.json" => match health_json(server) {
            Ok(body) => ("200 OK", "application/json", body).into(),
            Err(e) => error_json(e).into(),
        },
        "/snapshot.png" => match snapshot(server, query) {
            Ok(png) => image_reply("image/png", png),
            Err(e) => error_json(e).into(),
        },
        "/national.png" => match national(server) {
            Ok(png) => image_reply("image/png", png),
            Err(e) => error_json(e).into(),
        },
        "/national.mp4" => match national_clip(server) {
            Ok(body) => image_reply("video/mp4", body),
            Err(e) => error_json(e).into(),
        },
        "/loop.gif" => match loop_clip(server, query, crate::loopexport::LoopFormat::Gif) {
            Ok(body) => image_reply("image/gif", body),
            Err(e) => error_json(e).into(),
        },
        "/loop.mp4" => match loop_clip(server, query, crate::loopexport::LoopFormat::Mp4) {
            Ok(body) => image_reply("video/mp4", body),
            Err(e) => error_json(e).into(),
        },
        "/metrics" => (
            "200 OK",
            "text/plain; version=0.0.4; charset=utf-8",
            metrics(server).into_bytes(),
        )
            .into(),
        _ if path.starts_with("/proxy/") => proxy(server, path, query, if_none_match),
        _ if server.web_root.is_some() => static_file(server, path).into(),
        _ => not_found().into(),
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
    // A directory has an index.html or it has nothing — `--web-root web` could not serve /lite/
    // at all before this, because reading a directory is an error and that read as a 404.
    let file = match rel.is_empty() || rel.ends_with('/') {
        true => root.join(rel).join("index.html"),
        false => root.join(rel),
    };
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
        .put(path.to_string(), (Instant::now(), body.clone()));
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
        .put(key, (Instant::now(), body.clone()));
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
            .peek(path)
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
            .iter()
            .filter(|(k, _)| k.starts_with("/snapshot.png"))
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
    /// Built-in alternate palette by name, or `None` for the product's default table.
    palette: Option<String>,
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
        // Palettes are chosen by exact name from the built-in alternates for this product, and
        // an unknown one is a 400 rather than a silent fallback — the caller asked for a specific
        // picture. A name is never a path: this must not become a way to read a file off the
        // server, so nothing here touches the filesystem.
        let palette = match crate::cloud::param(query, "palette") {
            Some(name) => {
                if !crate::colormap::alt_names(moment).any(|n| n == name) {
                    anyhow::bail!("unknown palette '{name}' for {}", moment.short_name());
                }
                Some(name)
            }
            None => None,
        };
        Ok(Frame {
            site,
            moment,
            basemap,
            px: size.unwrap_or(1000).clamp(256, 2048),
            zoom,
            zoom_tag: zoom.map_or_else(|| "auto".to_string(), |z| format!("{z:.2}")),
            tilt,
            palette,
        })
    }

    /// Everything that changes the pixels, in one string — the cache key and the filename both
    /// have to carry all of it. The basemap used to be missing from the filename, so two styles
    /// of the same site raced over one file on disk.
    fn tag(&self) -> String {
        // The palette belongs here for the same reason the basemap does: two callers asking for
        // the same site with different tables would otherwise race over one file on disk.
        let palette = self.palette.as_deref().map_or("std".to_string(), |p| {
            p.chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
                .collect()
        });
        format!(
            "{}-{}-{}-{}-{}-{}-{}",
            self.site,
            self.moment.short_name(),
            self.basemap.slug(),
            self.px,
            self.zoom_tag,
            self.tilt,
            palette
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
        let Some(_one_at_a_time) = try_render(server, out)? else {
            return Ok(std::fs::read(out)?);
        };
        // Global knobs on the renderer, set under the same lock that serializes the render.
        crate::headless::set_output(Some(f.px), f.zoom);
        // Anything this server hands out is a picture someone will look at away from the app, so
        // it carries its warnings, its caption and its scale. The CLI verifiers do not.
        crate::headless::set_extras(true);
        // Set on every render, `Some` or `None`: an unset palette has to clear whatever the
        // previous caller left behind, exactly like the zoom override.
        crate::headless::set_palette(f.palette.clone());
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

/// The render lock, or `None` when somebody else has it and this frame already exists on disk.
///
/// A crawler walking the site directory asks for a few hundred renders at once. Queued on one
/// mutex the last of them waits an hour — long past the point where the CDN in front gives up, so
/// one crawl turns into an error page for everybody. A request that finds the renderer busy
/// therefore does not wait: it serves whatever is on disk, however old, and is fast about it. A
/// frame that has never been rendered at all is the only case left that can fail.
fn try_render<'a>(
    server: &'a Server,
    out: &std::path::Path,
) -> anyhow::Result<Option<std::sync::MutexGuard<'a, ()>>> {
    match server.render.try_lock() {
        Ok(guard) => Ok(Some(guard)),
        Err(_) if out.is_file() => {
            log::debug!("renderer busy; serving stale {}", out.display());
            Ok(None)
        }
        Err(_) => anyhow::bail!("renderer busy, and this frame has never been rendered"),
    }
}

/// Where rendered frames are kept between requests.
fn snapshot_dir() -> anyhow::Result<std::path::PathBuf> {
    let dir = crate::paths::cache_dir()
        .ok_or_else(|| anyhow::anyhow!("no cache directory"))?
        .join("snapshots");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// The bytes of a rendered file that is younger than `SNAPSHOT_TTL`, if there is one.
///
/// Anything unreadable, empty, or without a usable modified time is treated as absent — a missing
/// cache entry is a re-render, never an error.
fn fresh_on_disk(path: &std::path::Path) -> Option<Vec<u8>> {
    let age = std::fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .elapsed()
        .ok()?;
    if age >= SNAPSHOT_TTL {
        return None;
    }
    std::fs::read(path).ok().filter(|b| !b.is_empty())
}

/// A radar PNG through the same off-screen renderer the `--headless` verifier uses.
///
// ponytail: one render at a time. `palette=` selects a built-in alternate by name; a caller's
// own `.pal` file is not accepted, because a name can never become a path.
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
    let dir = snapshot_dir()?;
    prune_snapshots_hourly(&dir);
    let out = dir.join(format!("snapshot-{}.png", f.tag()));
    // Disk is the cache that survives a restart, and the loop path already reuses frames off it.
    // Without this a process that has just started — or one whose LRU dropped this key — re-renders
    // a file written seconds ago, which is the expensive half of the request.
    let png = match fresh_on_disk(&out) {
        Some(bytes) => bytes,
        None => render_png(server, &f, None, &out)?,
    };
    server
        .cache
        .lock()
        .unwrap()
        .put(key, (Instant::now(), png.clone()));
    Ok(png)
}

/// The MRMS national mosaic, rendered here rather than hotlinked from the NWS.
///
/// One frame for everybody — there is nothing to vary — so the key, the file and the TTL are all
/// fixed. MRMS publishes every couple of minutes and this holds a render for five, which is still
/// fresher than the ten-minute GIF it replaces.
fn national(server: &Server) -> anyhow::Result<Vec<u8>> {
    let key = "/national.png".to_string();
    if let Some(hit) = server
        .cache
        .lock()
        .unwrap()
        .get(&key)
        .filter(|(when, _)| when.elapsed() < SNAPSHOT_TTL)
    {
        return Ok(hit.1.clone());
    }
    let out = snapshot_dir()?.join("national.png");
    let png = match fresh_on_disk(&out) {
        Some(bytes) => bytes,
        None => match try_render(server, &out)? {
            Some(_one_at_a_time) => {
                crate::headless::set_output(Some(NATIONAL_PX), Some(NATIONAL_ZOOM));
                crate::headless::set_extras(true);
                crate::headless::set_palette(None);
                crate::headless::run_mrms(out.to_string_lossy().as_ref())?;
                archive_national_frame(&out);
                std::fs::read(&out)?
            }
            None => std::fs::read(&out)?,
        },
    };
    server
        .cache
        .lock()
        .unwrap()
        .put(key, (Instant::now(), png.clone()));
    Ok(png)
}

/// How many national renders the ring keeps. The warm timer runs every four minutes, so fifteen
/// is about an hour of weather — long enough to see a line move, short enough to encode quickly.
const NATIONAL_FRAMES: usize = 15;

fn national_frame_dir() -> anyhow::Result<std::path::PathBuf> {
    let dir = snapshot_dir()?.join("national-frames");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// The sorted ring, oldest first. Names carry the UTC minute, so sorting them is sorting by time.
fn national_frames() -> anyhow::Result<Vec<std::path::PathBuf>> {
    let mut frames: Vec<_> = std::fs::read_dir(national_frame_dir()?)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "png"))
        .collect();
    frames.sort();
    Ok(frames)
}

/// Keep a copy of a freshly rendered mosaic, and drop the oldest once the ring is full.
///
/// Best effort on purpose: the mosaic itself is already on disk and answered, and a full disk or
/// a racing render must not turn `/national.png` into an error page.
fn archive_national_frame(rendered: &std::path::Path) {
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M");
    let copy = || -> anyhow::Result<()> {
        let dir = national_frame_dir()?;
        std::fs::copy(rendered, dir.join(format!("national-{stamp}.png")))?;
        let frames = national_frames()?;
        for old in frames.iter().rev().skip(NATIONAL_FRAMES) {
            let _ = std::fs::remove_file(old);
        }
        Ok(())
    };
    if let Err(e) = copy() {
        log::warn!("national frame not archived: {e}");
    }
}

/// The national mosaic as an hour of motion — the same renders `/national.png` already made,
/// muxed rather than re-rendered, so the loop costs a video encode and nothing else.
fn national_clip(server: &Server) -> anyhow::Result<Vec<u8>> {
    let key = "/national.mp4".to_string();
    if let Some(hit) = server
        .cache
        .lock()
        .unwrap()
        .get(&key)
        .filter(|(when, _)| when.elapsed() < SNAPSHOT_TTL)
    {
        return Ok(hit.1.clone());
    }
    let frames = national_frames()?;
    if frames.len() < 2 {
        anyhow::bail!("not enough national frames yet");
    }
    let out = snapshot_dir()?.join("national.mp4");
    // Re-encode only when the ring has moved on since the last one.
    let newest = frames
        .last()
        .and_then(|p| p.metadata().ok()?.modified().ok());
    let encoded = out.metadata().ok().and_then(|m| m.modified().ok());
    if !matches!((encoded, newest), (Some(e), Some(n)) if e >= n) {
        let images: Vec<_> = frames
            .iter()
            .filter_map(|p| {
                Some(
                    image::load_from_memory(&std::fs::read(p).ok()?)
                        .ok()?
                        .to_rgba8(),
                )
            })
            .collect();
        crate::loopexport::encode_mp4(&images, 6, &out)?;
    }
    let body = std::fs::read(&out)?;
    server
        .cache
        .lock()
        .unwrap()
        .put(key, (Instant::now(), body.clone()));
    Ok(body)
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
    prune_stale(&dir, "snapshot-", SNAPSHOT_FILE_TTL);

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
        .put(key, (Instant::now(), body.clone()));
    Ok(body)
}

/// How long a rendered snapshot stays on disk. Longer than a loop frame's TTL because a snapshot
/// URL is shareable and may be fetched again; short enough that a headless box does not keep every
/// render it has ever served (186 MB observed).
const SNAPSHOT_FILE_TTL: std::time::Duration = std::time::Duration::from_secs(24 * 3600);

/// Drop loop frames that have slid out of every window. Without this the cache directory grows by
/// a render every five minutes for as long as the box is up.
fn prune_loop_frames(dir: &std::path::Path) {
    prune_stale(dir, "loop-", LOOP_FRAME_TTL);
}

/// Sweep stale snapshot renders, at most once an hour. Reading the directory on every request
/// would be the wrong shape for a route this hot.
fn prune_snapshots_hourly(dir: &std::path::Path) {
    static LAST: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
    let Ok(mut last) = LAST.lock() else { return };
    if last.is_some_and(|t| t.elapsed() < Duration::from_secs(3600)) {
        return;
    }
    *last = Some(Instant::now());
    prune_stale(dir, "snapshot-", SNAPSHOT_FILE_TTL);
}

/// Remove files in `dir` named `prefix*` last modified more than `ttl` ago.
///
/// Snapshots needed this too: only the GUI ever swept that directory, so a `--serve` box with no
/// window grew without bound.
fn prune_stale(dir: &std::path::Path, prefix: &str, ttl: std::time::Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        if !e.file_name().to_string_lossy().starts_with(prefix) {
            continue;
        }
        let stale = e
            .metadata()
            .and_then(|m| m.modified())
            .map(|t| t.elapsed().unwrap_or_default() > ttl)
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

    out.push_str("# HELP hookecho_stats_total Perf counters (wxdata::stats).\n");
    out.push_str("# TYPE hookecho_stats_total counter\n");
    for (label, n) in wxdata::stats::snapshot() {
        let label = escape_label(label);
        out.push_str(&format!(
            "hookecho_stats_total{{counter=\"{label}\"}} {n}\n"
        ));
    }

    // What the proxy cache is actually saving: a hit is an upstream fetch that did not happen.
    let (hits, misses) = (
        PROXY_HITS.load(Ordering::Relaxed),
        PROXY_MISSES.load(Ordering::Relaxed),
    );
    let (entries, bytes) = {
        let cache = server.proxy_cache.lock().unwrap();
        (
            cache.len(),
            cache.iter().map(|(_, h)| h.body.len()).sum::<usize>(),
        )
    };
    out.push_str("# HELP hookecho_proxy_cache_hits_total Proxied responses served without an upstream fetch.\n");
    out.push_str("# TYPE hookecho_proxy_cache_hits_total counter\n");
    out.push_str(&format!("hookecho_proxy_cache_hits_total {hits}\n"));
    out.push_str("# HELP hookecho_proxy_cache_misses_total Proxied responses that needed an upstream fetch.\n");
    out.push_str("# TYPE hookecho_proxy_cache_misses_total counter\n");
    out.push_str(&format!("hookecho_proxy_cache_misses_total {misses}\n"));
    out.push_str("# HELP hookecho_proxy_cache_bytes Bytes the proxy cache is holding.\n");
    out.push_str("# TYPE hookecho_proxy_cache_bytes gauge\n");
    out.push_str(&format!("hookecho_proxy_cache_bytes {bytes}\n"));
    out.push_str("# HELP hookecho_proxy_cache_entries Responses the proxy cache is holding.\n");
    out.push_str("# TYPE hookecho_proxy_cache_entries gauge\n");
    out.push_str(&format!("hookecho_proxy_cache_entries {entries}\n"));

    let Ok(body) = cached_json(server, "/status.json") else {
        // A feed being down is not a reason to fail the scrape — the counters above still answer.
        return out;
    };
    let age = server
        .cache
        .lock()
        .unwrap()
        .peek("/status.json")
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
    // MeteoAlarm: the CAP warnings every European met service publishes in common.
    "feeds.meteoalarm.org",
    // European radar.
    "opendata.dwd.de",
    // DWD's WMS: the German radar composites (RV nowcast, WN analysis) as rendered tiles.
    "maps.dwd.de",
    // ECCC's GeoMet WMS: the Canadian 1-km radar composites (rain, snow).
    "geo.weather.gc.ca",
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

/// The feeds that go stale in seconds. Mirrors `LIVE_HOSTS` in web/_worker.js/proxy-core.js.
const LIVE_HOSTS: &[&str] = &[
    "unidata-nexrad-level2-chunks.s3.amazonaws.com",
    "api.weather.gov",
    // DWD republishes every `-LATEST-` sweep on a five-minute cycle at a URL that never changes,
    // so the default five-minute TTL can hand out the previous volume for the whole of the next.
    "opendata.dwd.de",
];

/// The archived Level 2 bucket: the same handful of volumes every client on a radar asks for.
const ARCHIVE_BUCKET: &str = "unidata-nexrad-level2.s3.amazonaws.com";

/// How long a proxied response may be reused, in seconds.
///
/// The same policy the edge worker applies (`cacheSeconds`, web/_worker.js/proxy-core.js) — the
/// two are kept in step by `cache_ttls_match_the_edge_worker` below, because a client that gets
/// one answer from the Pages deployment and a different one from `--serve` is a bug that only
/// shows up as a loop that will not advance.
fn cache_seconds(host: &str, query: &str) -> u64 {
    // A bucket listing is how the app finds the newest volume — cache that like a live feed or
    // the loop stops advancing.
    if query.contains("list-type=") {
        return 15;
    }
    // A WMS GetMap with no TIME is whatever the layer's default frame happens to be, which turns
    // over every five minutes; one naming a frame is that frame forever.
    if host == "maps.dwd.de" || host == "geo.weather.gc.ca" {
        return if query.contains("TIME=") { 300 } else { 15 };
    }
    // An hour, not forever: the newest archive object is re-uploaded while the radar is still
    // writing it, so a long TTL can pin a truncated volume.
    if host == ARCHIVE_BUCKET {
        return 3600;
    }
    if LIVE_HOSTS.contains(&host) {
        15
    } else {
        300
    }
}

/// One proxied response, kept for its TTL.
struct ProxyHit {
    at: Instant,
    ctype: &'static str,
    /// Shared so the cache holds one copy however many requests are reading it.
    body: std::sync::Arc<Vec<u8>>,
    /// Hashed once at insert, so a client that already holds these bytes can be told so.
    etag: String,
}

/// A cheap content hash. Not a checksum anyone trusts — an ETag only has to change when the bytes
/// change, and this is compared against a value we ourselves issued.
fn etag_of(body: &[u8]) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    body.len().hash(&mut h);
    body.hash(&mut h);
    format!("\"{:016x}\"", h.finish())
}

/// Counters behind the `hookecho_proxy_cache_*` metrics.
static PROXY_HITS: AtomicU64 = AtomicU64::new(0);
static PROXY_MISSES: AtomicU64 = AtomicU64::new(0);

/// Nothing bigger than this comes back through the proxy. An archive volume is a few MB; this is
/// generous for every feed on the list and still bounds a hostile or broken upstream.
const PROXY_MAX_BYTES: usize = 64 * 1024 * 1024;

/// `GET /proxy/{host}/{rest}` → `https://{host}/{rest}`, for the browser build.
///
/// The trust boundary, in four rules: the host must be in [`ALLOWED_HOSTS`] exactly, only GET is
/// ever issued upstream (this server never speaks another method), no client header reaches the
/// upstream, and the response is capped and stripped down to a known content type. Same-origin by
/// construction, so no CORS header of our own is needed.
fn proxy(server: &Server, path: &str, query: &str, if_none_match: Option<&str>) -> Reply {
    let forbidden = |why: &str| -> Reply {
        log::warn!("proxy refused {path}: {why}");
        (
            "403 Forbidden",
            "application/json",
            br#"{"error":"host not proxyable"}"#.to_vec(),
        )
            .into()
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

    let ttl = cache_seconds(host, query);
    let fresh = |h: &ProxyHit| h.at.elapsed() < Duration::from_secs(ttl);
    // Cloned out from under the lock: an upstream fetch must not be holding the cache shut, and
    // the clone is an `Arc` bump, not the body.
    let hit = {
        let mut cache = server.proxy_cache.lock().unwrap();
        cache
            .get(&url)
            .filter(|h| fresh(h))
            .map(|h| (h.ctype, h.body.clone(), h.etag.clone()))
    };
    if let Some((ctype, body, etag)) = hit {
        PROXY_HITS.fetch_add(1, Ordering::Relaxed);
        return proxy_reply(ctype, body, etag, ttl, if_none_match);
    }

    PROXY_MISSES.fetch_add(1, Ordering::Relaxed);
    match server.rt.block_on(fetch_capped(&server.http, &url)) {
        Ok((ctype, body)) => {
            let body = std::sync::Arc::new(body);
            let etag = etag_of(&body);
            let mut cache = server.proxy_cache.lock().unwrap();
            cache.put(
                url,
                ProxyHit {
                    at: Instant::now(),
                    ctype,
                    body: body.clone(),
                    etag: etag.clone(),
                },
            );
            // ponytail: re-sums the held bytes per insert. At 64 entries that is 64 adds; a
            // running total would have to be kept correct across every eviction path instead.
            while cache.iter().map(|(_, h)| h.body.len()).sum::<usize>() > PROXY_CACHE_BYTES
                && cache.len() > 1
            {
                cache.pop_lru();
            }
            drop(cache);
            proxy_reply(ctype, body, etag, ttl, if_none_match)
        }
        Err(e) => {
            log::warn!("proxy fetch of {url} failed: {e}");
            (
                "502 Bad Gateway",
                "application/json",
                br#"{"error":"upstream fetch failed"}"#.to_vec(),
            )
                .into()
        }
    }
}

/// The response for a proxied body, 304 if the client already holds it.
///
/// The client's `If-None-Match` is compared here and never forwarded — the upstream call is the
/// same header-free GET it has always been, which is the rule the whole proxy is built on.
fn proxy_reply(
    ctype: &'static str,
    body: std::sync::Arc<Vec<u8>>,
    etag: String,
    ttl: u64,
    if_none_match: Option<&str>,
) -> Reply {
    let headers = vec![
        ("Cache-Control", format!("public, max-age={ttl}")),
        ("ETag", etag.clone()),
    ];
    if if_none_match.is_some_and(|t| t.split(',').any(|t| t.trim() == etag)) {
        return Reply {
            status: "304 Not Modified",
            ctype,
            body: Vec::new(),
            headers,
        };
    }
    Reply {
        status: "200 OK",
        ctype,
        // The bytes are handed to one socket; the cache keeps its own copy through the `Arc`.
        body: (*body).clone(),
        headers,
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
    fn a_palette_is_an_exact_alternate_name_or_a_refusal() {
        let name = crate::colormap::alt_names(wxdata::level2::Moment::Reflectivity)
            .next()
            .expect("reflectivity has an alternate");
        let q = format!("site=KTLX&palette={}", crate::cloud::urlencode(name));
        let f = Frame::parse(&q).unwrap();
        assert_eq!(f.palette.as_deref(), Some(name));
        // The palette changes the pixels, so it has to change the filename too.
        assert_ne!(f.tag(), Frame::parse("site=KTLX").unwrap().tag());
        assert!(
            !f.tag().contains(['/', '\\', ' ']),
            "tag becomes a filename"
        );

        // Not a name of a table this product has, not a path, not a near miss.
        assert!(Frame::parse("site=KTLX&palette=viridis").is_err());
        assert!(Frame::parse("site=KTLX&palette=../../etc/passwd").is_err());
        // The alternate belongs to reflectivity; spectrum width has none at all.
        assert!(Frame::parse(&format!("site=KTLX&product=SW&palette={name}")).is_err());
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
            public: false,
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
            cache: Mutex::new(lru::LruCache::new(
                std::num::NonZeroUsize::new(ANSWER_CACHE).unwrap(),
            )),
            proxy_cache: Mutex::new(lru::LruCache::new(
                std::num::NonZeroUsize::new(PROXY_CACHE_ENTRIES).unwrap(),
            )),
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
            public: false,
            spots: Vec::new(),
            web_root: None,
            rt: tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap(),
            http: reqwest::Client::new(),
            cache: Mutex::new(lru::LruCache::new(
                std::num::NonZeroUsize::new(ANSWER_CACHE).unwrap(),
            )),
            proxy_cache: Mutex::new(lru::LruCache::new(
                std::num::NonZeroUsize::new(PROXY_CACHE_ENTRIES).unwrap(),
            )),
            render: Mutex::new(()),
        };
        let Reply {
            status,
            ctype,
            body,
            ..
        } = route(&server, "/etc/passwd", "", None);
        assert_eq!(status, "404 Not Found");
        assert_eq!(ctype, "application/json");
        assert!(String::from_utf8_lossy(&body).contains("no such endpoint"));

        let status = route(&server, "/", "", None).status;
        assert_eq!(status, "200 OK");
    }

    #[test]
    fn static_serving_refuses_to_walk_out_of_the_web_root() {
        let server = Server {
            token: String::new(),
            public: false,
            spots: Vec::new(),
            web_root: Some(std::path::PathBuf::from("/var/empty")),
            rt: tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap(),
            http: reqwest::Client::new(),
            cache: Mutex::new(lru::LruCache::new(
                std::num::NonZeroUsize::new(ANSWER_CACHE).unwrap(),
            )),
            proxy_cache: Mutex::new(lru::LruCache::new(
                std::num::NonZeroUsize::new(PROXY_CACHE_ENTRIES).unwrap(),
            )),
            render: Mutex::new(()),
        };
        for path in [
            "/../../etc/passwd",
            "/dist/../../../etc/shadow",
            "/..%2f..%2fetc/passwd",
            "//etc/passwd",
        ] {
            let status = route(&server, path, "", None).status;
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

    /// The TTL a proxied response gets here must be the one the edge worker gives it. These are
    /// the values in `cacheSeconds` (web/_worker.js/proxy-core.js) written out — if that table
    /// moves, this fails and says so, instead of two deployments quietly disagreeing about how
    /// long a volume is good for.
    /// `--web-root web` has to be able to serve `/lite/`, which is a directory. Reading one is an
    /// error, and that error read as a 404 — the lite viewer was simply unreachable this way.
    #[test]
    fn a_directory_is_served_as_its_index() {
        let dir = std::env::temp_dir().join(format!("hookecho-serve-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("lite")).unwrap();
        std::fs::write(dir.join("lite/index.html"), b"<!doctype html>lite").unwrap();
        let server = Server {
            token: String::new(),
            public: false,
            spots: Vec::new(),
            web_root: Some(dir.clone()),
            rt: tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap(),
            http: reqwest::Client::new(),
            cache: Mutex::new(lru::LruCache::new(
                std::num::NonZeroUsize::new(ANSWER_CACHE).unwrap(),
            )),
            proxy_cache: Mutex::new(lru::LruCache::new(
                std::num::NonZeroUsize::new(PROXY_CACHE_ENTRIES).unwrap(),
            )),
            render: Mutex::new(()),
        };
        let body = route(&server, "/lite/", "", None).body;
        assert_eq!(String::from_utf8_lossy(&body), "<!doctype html>lite");
        // Still no way out of the root, trailing slash or not.
        assert_eq!(route(&server, "/../", "", None).status, "404 Not Found");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn cache_ttls_match_the_edge_worker() {
        assert_eq!(cache_seconds(ARCHIVE_BUCKET, ""), 3600);
        assert_eq!(cache_seconds(ARCHIVE_BUCKET, "list-type=2"), 15);
        assert_eq!(cache_seconds("maps.dwd.de", "LAYERS=rv&TIME=2026"), 300);
        assert_eq!(cache_seconds("maps.dwd.de", "LAYERS=rv"), 15);
        assert_eq!(cache_seconds("geo.weather.gc.ca", "SERVICE=WFS"), 15);
        assert_eq!(cache_seconds("api.weather.gov", ""), 15);
        assert_eq!(cache_seconds("opendata.dwd.de", ""), 15);
        assert_eq!(cache_seconds("tgftp.nws.noaa.gov", ""), 300);
        assert_eq!(cache_seconds("basemaps.cartocdn.com", ""), 300);
    }

    /// The byte bound is what stops a handful of archive volumes from being the whole heap. It has
    /// to evict on total size, not only on entry count — 64 volumes is 64 × tens of MB.
    #[test]
    fn the_proxy_cache_evicts_on_bytes_not_just_entries() {
        let mut cache =
            lru::LruCache::new(std::num::NonZeroUsize::new(PROXY_CACHE_ENTRIES).unwrap());
        let held = |c: &lru::LruCache<String, ProxyHit>| {
            c.iter().map(|(_, h)| h.body.len()).sum::<usize>()
        };
        // Ten entries of 64 MB: well inside the entry cap, well past the byte cap.
        for i in 0..10 {
            cache.put(
                format!("https://example.invalid/{i}"),
                ProxyHit {
                    at: Instant::now(),
                    ctype: "application/octet-stream",
                    body: std::sync::Arc::new(vec![0u8; 64 * 1024 * 1024]),
                    etag: String::new(),
                },
            );
            while held(&cache) > PROXY_CACHE_BYTES && cache.len() > 1 {
                cache.pop_lru();
            }
        }
        assert!(held(&cache) <= PROXY_CACHE_BYTES, "byte bound not honoured");
        assert!(cache.len() < 10, "nothing was evicted");
        // The newest survives — the one a second client is most likely to ask for next.
        assert!(cache.peek("https://example.invalid/9").is_some());
    }

    /// A client already holding the bytes is told so and gets no body; a stale tag gets the body.
    #[test]
    fn a_matching_etag_is_answered_304() {
        let body = std::sync::Arc::new(b"hello".to_vec());
        let etag = etag_of(&body);
        let hit = proxy_reply("text/plain", body.clone(), etag.clone(), 15, Some(&etag));
        assert_eq!(hit.status, "304 Not Modified");
        assert!(hit.body.is_empty());
        assert!(hit
            .headers
            .contains(&("Cache-Control", "public, max-age=15".to_string())));
        let miss = proxy_reply("text/plain", body, etag, 15, Some("\"stale\""));
        assert_eq!(miss.status, "200 OK");
        assert_eq!(miss.body, b"hello");
        // Different bytes, different tag — an ETag that never moves is worse than none.
        assert_ne!(etag_of(b"hello"), etag_of(b"hellp"));
    }

    #[test]
    fn proxy_refuses_hosts_that_are_not_on_the_list() {
        let server = Server {
            token: String::new(),
            public: false,
            spots: Vec::new(),
            web_root: None,
            rt: tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap(),
            http: reqwest::Client::new(),
            cache: Mutex::new(lru::LruCache::new(
                std::num::NonZeroUsize::new(ANSWER_CACHE).unwrap(),
            )),
            proxy_cache: Mutex::new(lru::LruCache::new(
                std::num::NonZeroUsize::new(PROXY_CACHE_ENTRIES).unwrap(),
            )),
            render: Mutex::new(()),
        };
        for path in [
            "/proxy/evil.example.com/x",
            // Suffix and prefix games against a host that *is* allowed: match is exact.
            "/proxy/api.weather.gov.evil.example.com/alerts",
            "/proxy/evil.example.com#api.weather.gov/alerts",
            "/proxy/api.weather.gov",
        ] {
            let status = route(&server, path, "", None).status;
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
            public: false,
            spots: Vec::new(),
            web_root: None,
            rt: tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap(),
            http: reqwest::Client::new(),
            cache: Mutex::new(lru::LruCache::new(
                std::num::NonZeroUsize::new(ANSWER_CACHE).unwrap(),
            )),
            proxy_cache: Mutex::new(lru::LruCache::new(
                std::num::NonZeroUsize::new(PROXY_CACHE_ENTRIES).unwrap(),
            )),
            render: Mutex::new(()),
        };
        let i = ROUTES.iter().position(|r| *r == "index").unwrap();
        let before = REQUESTS[i].load(Ordering::Relaxed);
        route(&server, "/", "", None);
        // Counters are process-wide and the test threads share them, so this asserts movement
        // rather than an exact delta — another test routing "/" must not fail this one.
        assert!(REQUESTS[i].load(Ordering::Relaxed) > before);
    }
}

#[cfg(test)]
mod cache_hygiene_tests {
    use super::*;

    /// A render is reusable for its whole TTL, and the answer says so — without this header a CDN
    /// in front of the server treats every view as a fresh origin request.
    #[test]
    fn image_answers_are_cacheable() {
        let reply = image_reply("image/png", vec![1, 2, 3]);
        assert_eq!(reply.status, "200 OK");
        assert!(reply
            .headers
            .contains(&("Cache-Control", "public, max-age=300".to_string())));
    }

    /// A file just written is reused; one backdated past the TTL is not; a missing one is absent.
    #[test]
    fn only_young_files_are_reused() {
        let dir = std::env::temp_dir().join(format!("hookecho-fresh-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("snapshot.png");

        assert!(fresh_on_disk(&path).is_none(), "no file is no answer");
        std::fs::write(&path, b"pixels").unwrap();
        assert_eq!(fresh_on_disk(&path).as_deref(), Some(&b"pixels"[..]));

        // An empty file is what a half-finished render leaves behind, and it is not an answer.
        std::fs::write(&path, b"").unwrap();
        assert!(fresh_on_disk(&path).is_none());

        // Backdated past the TTL: the volume has moved on, so the pixels have to be rebuilt.
        std::fs::write(&path, b"pixels").unwrap();
        let old = std::time::SystemTime::now() - SNAPSHOT_TTL - Duration::from_secs(1);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(old)
            .unwrap();
        assert!(fresh_on_disk(&path).is_none(), "stale file was reused");

        std::fs::remove_dir_all(&dir).ok();
    }
}

#[cfg(test)]
mod preset_gate_tests {
    use super::*;

    /// The frames the site embeds are served without a token; everything else is not. This is the
    /// whole public surface of the origin, so it is worth spelling out.
    #[test]
    fn only_the_site_frames_are_public() {
        // Allowed: a known radar's default frame, with or without the page's cache-buster.
        assert!(is_preset("/snapshot.png", "site=KTLX"));
        assert!(is_preset("/snapshot.png", "site=KTLX&t=123"));
        assert!(is_preset("/snapshot.png", "site=TOKC")); // TDWR — a still, not a loop
        assert!(is_preset("/loop.gif", "site=KTLX"));
        assert!(is_preset("/loop.mp4", "site=KTLX"));
        assert!(is_preset("/snapshot.png", "site=KTLX&product=VEL"));
        assert!(is_preset("/loop.mp4", "product=VEL&site=KTLX"));
        assert!(is_preset("/national.png", ""));
        assert!(is_preset("/national.png", "t=99"));
        assert!(is_preset("/national.mp4", ""));
        assert!(is_preset("/health.json", ""));

        // Refused: any other knob, however harmless it looks.
        assert!(!is_preset("/snapshot.png", "site=KTLX&size=2048"));
        assert!(!is_preset("/snapshot.png", "site=KTLX&product=CC"));
        assert!(!is_preset("/snapshot.png", "product=VEL"));
        assert!(!is_preset("/snapshot.png", "site=KTLX&zoom=12"));
        assert!(!is_preset("/snapshot.png", "site=KTLX&basemap=satellite"));
        assert!(!is_preset("/loop.gif", "site=KTLX&frames=12"));
        assert!(!is_preset("/national.png", "size=2048"));
        assert!(!is_preset("/national.mp4", "frames=30"));

        // Refused: a site nobody has heard of, and a loop of a radar with no archive to step.
        assert!(!is_preset("/snapshot.png", "site=ZZZZ"));
        assert!(!is_preset("/loop.gif", "site=TOKC"));
        assert!(!is_preset("/loop.mp4", "site=TOKC"));
        assert!(!is_preset("/snapshot.png", ""));

        // Refused: everything that is not an image the site embeds.
        assert!(!is_preset("/status.json", ""));
        assert!(!is_preset("/metrics", ""));
        assert!(!is_preset("/proxy/https://example.invalid/x", ""));
        assert!(!is_preset("/", ""));
    }
}
