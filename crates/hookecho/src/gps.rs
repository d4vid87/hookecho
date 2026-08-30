//! Optional GPS position source, streaming the latest fix `(lon, lat)` to the app over a channel
//! for chase-mode follow-me. Two implementations behind one `spawn`: `gpsd` on localhost:2947 for
//! desktop, and the browser's Geolocation API on the web. Best-effort on both — no daemon, or a
//! refused permission, means `spawn` returns `None` (or the channel simply stays quiet) and chase
//! mode stays manual.
//!
//! (Android is neither: it polls the system LocationManager over JNI in `platform.rs`, and feeds
//! the same channel shape.)

#[cfg(not(target_arch = "wasm32"))]
use std::io::{BufRead, BufReader, Write};
use std::sync::mpsc::{self, Receiver};

/// Parse a gpsd JSON line, returning `(lon, lat)` for a TPV report with a 2D+ fix.
#[cfg(not(target_arch = "wasm32"))]
fn parse_tpv(line: &str) -> Option<(f64, f64)> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    if v.get("class")?.as_str()? != "TPV" {
        return None;
    }
    // mode 2 = 2D fix, 3 = 3D fix; 0/1 have no usable position.
    if v.get("mode").and_then(|m| m.as_i64()).unwrap_or(0) < 2 {
        return None;
    }
    let lat = v.get("lat")?.as_f64()?;
    let lon = v.get("lon")?.as_f64()?;
    Some((lon, lat))
}

/// Connect to `gpsd` and stream position fixes. Returns `None` if the daemon isn't reachable.
/// The reader thread runs for the process lifetime.
#[cfg(not(target_arch = "wasm32"))]
pub fn spawn() -> Option<Receiver<(f64, f64)>> {
    let stream = std::net::TcpStream::connect(("127.0.0.1", 2947)).ok()?;
    stream.set_read_timeout(None).ok()?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut stream = stream;
        // Enable JSON streaming.
        if stream
            .write_all(b"?WATCH={\"enable\":true,\"json\":true}\n")
            .is_err()
        {
            return;
        }
        let Ok(cloned) = stream.try_clone() else {
            log::warn!("gpsd: cannot clone stream");
            return;
        };
        let reader = BufReader::new(cloned);
        for line in reader.lines() {
            let Ok(line) = line else { break };
            if let Some(pos) = parse_tpv(&line) {
                if tx.send(pos).is_err() {
                    break; // app dropped the receiver
                }
            }
        }
    });
    Some(rx)
}

/// Watch the browser's own position. There is no daemon to be missing here — the thing that can
/// go wrong is the user saying no, which arrives as an error on the callback rather than as a
/// failure to start. So this returns a live receiver either way and the channel just stays empty,
/// which is the same thing chase mode already does while waiting for a first fix.
///
/// Calling `watchPosition` is what raises the permission prompt, and it is only ever called from a
/// button the user pressed — the app never asks for a location out of nowhere.
///
/// ponytail: no `clearWatch`. The watch ends with the page, and a browser tab that has the
/// permission is not spending anything meaningful to keep an idle GPS callback registered.
#[cfg(target_arch = "wasm32")]
pub fn spawn() -> Option<Receiver<(f64, f64)>> {
    use wasm_bindgen::prelude::*;
    use wasm_bindgen::JsCast;

    let geo = web_sys::window()?.navigator().geolocation().ok()?;
    let (tx, rx) = mpsc::channel();

    let on_fix = Closure::wrap(Box::new(move |pos: JsValue| {
        // `unchecked_into`, not `dyn_into`: web-sys still calls this interface `Position`, while
        // every current browser exposes it as `GeolocationPosition`, so an `instanceof Position`
        // check fails against a perfectly good fix. The getters underneath are plain property
        // reads and do not care what the constructor is called.
        let pos: web_sys::Position = pos.unchecked_into();
        let c = pos.coords();
        // Dropped receiver means the user disconnected GPS; nothing to do but stop sending.
        let _ = tx.send((c.longitude(), c.latitude()));
    }) as Box<dyn FnMut(JsValue)>);

    let on_err = Closure::wrap(Box::new(move |e: JsValue| {
        // Same renaming story as the fix callback (`GeolocationPositionError` now).
        let msg: web_sys::PositionError = e.unchecked_into();
        log::warn!("geolocation: {}", msg.message());
    }) as Box<dyn FnMut(JsValue)>);

    // High accuracy: this is chase follow-me, where the difference between a cell-tower fix and a
    // GPS fix is the difference between the right side of a storm and the wrong one.
    let opts = web_sys::PositionOptions::new();
    opts.set_enable_high_accuracy(true);
    let started = geo.watch_position_with_error_callback_and_options(
        on_fix.as_ref().unchecked_ref(),
        Some(on_err.as_ref().unchecked_ref()),
        &opts,
    );
    if let Err(e) = started {
        log::warn!("geolocation unavailable: {e:?}");
        return None;
    }
    // The callbacks outlive this function by design — they fire for as long as the watch runs.
    on_fix.forget();
    on_err.forget();
    Some(rx)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn parses_tpv_fix() {
        let line = r#"{"class":"TPV","mode":3,"lat":35.47,"lon":-97.5,"alt":300.0}"#;
        assert_eq!(parse_tpv(line), Some((-97.5, 35.47)));
    }

    #[test]
    fn rejects_no_fix_and_other_classes() {
        assert!(parse_tpv(r#"{"class":"TPV","mode":1,"lat":35.0,"lon":-97.0}"#).is_none());
        assert!(parse_tpv(r#"{"class":"SKY","satellites":[]}"#).is_none());
        assert!(parse_tpv("not json").is_none());
    }
}
