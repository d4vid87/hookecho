//! Optional GPS position source via `gpsd` (localhost:2947). Connects, enables JSON watch, and
//! streams the latest fix `(lon, lat)` to the app over a channel for chase-mode follow-me.
//! Best-effort: if `gpsd` isn't running, `spawn` returns `None` and chase mode stays manual.

use std::io::{BufRead, BufReader, Write};
use std::sync::mpsc::{self, Receiver};

/// Parse a gpsd JSON line, returning `(lon, lat)` for a TPV report with a 2D+ fix.
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
pub fn spawn() -> Option<Receiver<(f64, f64)>> {
    let stream = std::net::TcpStream::connect(("127.0.0.1", 2947)).ok()?;
    stream.set_read_timeout(None).ok()?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut stream = stream;
        // Enable JSON streaming.
        if stream.write_all(b"?WATCH={\"enable\":true,\"json\":true}\n").is_err() {
            return;
        }
        let reader = BufReader::new(stream.try_clone().expect("clone gpsd stream"));
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

#[cfg(test)]
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
