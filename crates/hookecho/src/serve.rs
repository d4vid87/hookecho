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
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long a JSON answer is reused before the feeds are asked again.
const JSON_TTL: Duration = Duration::from_secs(60);
/// How long a rendered radar PNG is reused. A volume is ~5 minutes wide, so is this.
const SNAPSHOT_TTL: Duration = Duration::from_secs(300);

struct Server {
    spots: Vec<Spot>,
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
) -> anyhow::Result<()> {
    let listener = TcpListener::bind((bind, port))?;
    let server = Arc::new(Server {
        spots,
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
        log::warn!("bound to all interfaces — anyone on this network can read your locations");
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
    let mut line = String::new();
    BufReader::new(stream.try_clone()?).read_line(&mut line)?;
    // "GET /status.json?x=1 HTTP/1.1"
    let target = line.split_whitespace().nth(1).unwrap_or("/");
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p, q),
        None => (target, ""),
    };

    let (status, ctype, body) = route(server, path, query);
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
    match path {
        "/" if server.web_root.is_some() => static_file(server, "/index.html"),
        "/" => ("200 OK", "text/html; charset=utf-8", index().into_bytes()),
        "/status.json" | "/alerts.json" | "/obs.json" => match cached_json(server, path) {
            Ok(body) => ("200 OK", "application/json", body),
            Err(e) => error_json(e),
        },
        "/snapshot.png" => match snapshot(server, query) {
            Ok(png) => ("200 OK", "image/png", png),
            Err(e) => error_json(e),
        },
        _ if server.web_root.is_some() => static_file(server, path),
        _ => not_found(),
    }
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

/// A radar PNG through the same off-screen renderer the `--headless` verifier uses.
///
// ponytail: one render at a time, fixed 1000x1000, site and product only; add tilt/palette params
// when someone asks for them.
fn snapshot(server: &Server, query: &str) -> anyhow::Result<Vec<u8>> {
    let site = crate::cloud::param(query, "site").unwrap_or_else(|| "KTLX".to_string());
    let product = crate::cloud::param(query, "product").unwrap_or_else(|| "REF".to_string());
    if !site.chars().all(|c| c.is_ascii_alphanumeric()) {
        anyhow::bail!("bad site");
    }
    let moment = wxdata::level2::Moment::from_code(&product.to_ascii_uppercase())
        .ok_or_else(|| anyhow::anyhow!("unknown product '{product}'"))?;
    // A bare sweep on a blank field is unreadable on a dashboard; the keyless dark vector map is
    // the sane default, and `basemap=none` gets the bare sweep back.
    let basemap = crate::tiles::BasemapStyle::from_slug(
        &crate::cloud::param(query, "basemap").unwrap_or_else(|| "dark".to_string()),
    );

    let key = format!("/snapshot.png?{site}&{product}&{}", basemap.slug());
    if let Some(hit) = server
        .cache
        .lock()
        .unwrap()
        .get(&key)
        .filter(|(when, _)| when.elapsed() < SNAPSHOT_TTL)
    {
        return Ok(hit.1.clone());
    }

    let dir = crate::paths::cache_dir().ok_or_else(|| anyhow::anyhow!("no cache directory"))?;
    std::fs::create_dir_all(&dir)?;
    let out = dir.join(format!("snapshot-{site}-{product}.png"));
    {
        // The headless renderer builds its own runtime, so it can only be called from a plain
        // thread — which this is, one connection per thread.
        let _one_at_a_time = server.render.lock().unwrap();
        crate::headless::run(
            out.to_string_lossy().as_ref(),
            &site,
            moment,
            0,
            true,
            None,
            None,
            None,
            None,
            basemap,
            false,
        )?;
    }
    let png = std::fs::read(&out)?;
    server
        .cache
        .lock()
        .unwrap()
        .insert(key, (Instant::now(), png.clone()));
    Ok(png)
}

fn index() -> String {
    "<!doctype html><meta charset=utf-8><title>Hook Echo-WX</title>\
     <h1>Hook Echo-WX</h1><ul>\
     <li><a href=\"/status.json\">/status.json</a> — conditions and alerts at your locations</li>\
     <li><a href=\"/alerts.json\">/alerts.json</a> — alerts only</li>\
     <li><a href=\"/obs.json\">/obs.json</a> — conditions only</li>\
     <li><a href=\"/snapshot.png?site=KTLX\">/snapshot.png?site=KTLX&amp;product=REF</a> — radar render</li>\
     </ul>"
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_paths_404() {
        // A server with no spots: routing is independent of what it would report.
        let server = Server {
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
    }
}
