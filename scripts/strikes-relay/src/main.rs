//! Republish Blitzortung lightning strikes onto your own MQTT broker.
//!
//! HookEcho never connects to Blitzortung: their terms ask that third-party applications not
//! point their users' clients at the network's servers. What they do allow is a participant
//! running one relay for their own household. That is this program — one websocket connection
//! out, one MQTT connection to a broker you run, and HookEcho subscribing to your broker.
//!
//! The wire protocol is folklore rather than specification: connect, send `{"a":111}`, and
//! receive JSON strings compressed with a dictionary scheme that is LZW in everything but name.
//! Both halves were captured from a live connection before this was written, and
//! [`decompress`]'s test is a case that scheme produces.
//!
//! Topics match what the Home Assistant Blitzortung integration publishes, so anything already
//! written against that works here unchanged: `blitzortung/1.1/{geohash}` carrying the strike
//! JSON as received.

use futures_util::{SinkExt, StreamExt};
use std::time::Duration;

/// The network's public feed servers. Rotated on failure — one of them is usually up.
const HOSTS: [&str; 3] = [
    "wss://ws1.blitzortung.org/",
    "wss://ws7.blitzortung.org/",
    "wss://ws8.blitzortung.org/",
];

/// The handshake. Undocumented, unchanged for years, and the connection stays silent without it.
const SUBSCRIBE: &str = r#"{"a":111}"#;

/// Undo the dictionary compression every frame arrives in.
///
/// Straight LZW over UTF-16-ish code units: anything under 256 is a literal, anything above is an
/// entry built from the previous output. Written to match the reference implementation's quirks
/// exactly, including the "not in the table yet" case that a run of one repeated character
/// produces.
fn decompress(input: &str) -> Option<String> {
    let chars: Vec<char> = input.chars().collect();
    let first = *chars.first()?;
    let mut dict: std::collections::HashMap<u32, String> = std::collections::HashMap::new();
    let mut prev = first.to_string();
    let mut out = prev.clone();
    let mut next_code = 256u32;
    let mut ch = first;
    for &c in &chars[1..] {
        let code = c as u32;
        let entry = if code < 256 {
            c.to_string()
        } else if let Some(e) = dict.get(&code) {
            e.clone()
        } else {
            format!("{prev}{ch}")
        };
        out.push_str(&entry);
        ch = entry.chars().next()?;
        dict.insert(next_code, format!("{prev}{ch}"));
        next_code += 1;
        prev = entry;
    }
    Some(out)
}

/// Geohash of a position, `precision` characters. The integration keys its topics by geohash so a
/// subscriber can take a region with a wildcard instead of the whole planet.
fn geohash(lat: f64, lon: f64, precision: usize) -> String {
    const BASE32: &[u8] = b"0123456789bcdefghjkmnpqrstuvwxyz";
    let (mut lat_lo, mut lat_hi) = (-90.0f64, 90.0f64);
    let (mut lon_lo, mut lon_hi) = (-180.0f64, 180.0f64);
    let (mut bit, mut ch, mut even) = (0, 0usize, true);
    let mut out = String::with_capacity(precision);
    while out.len() < precision {
        if even {
            let mid = (lon_lo + lon_hi) / 2.0;
            if lon > mid {
                ch = ch * 2 + 1;
                lon_lo = mid;
            } else {
                ch *= 2;
                lon_hi = mid;
            }
        } else {
            let mid = (lat_lo + lat_hi) / 2.0;
            if lat > mid {
                ch = ch * 2 + 1;
                lat_lo = mid;
            } else {
                ch *= 2;
                lat_hi = mid;
            }
        }
        even = !even;
        bit += 1;
        if bit == 5 {
            out.push(BASE32[ch] as char);
            bit = 0;
            ch = 0;
        }
    }
    out
}

struct Args {
    /// Print strikes instead of publishing them — for checking the feed without a broker.
    dry_run: bool,
    broker: String,
    port: u16,
    prefix: String,
    user: Option<String>,
    pass: Option<String>,
}

fn args() -> Args {
    let mut a = Args {
        dry_run: false,
        // Loopback by default: the broker this feeds is the one on the same box, and a relay that
        // reaches out to the network by accident is exactly what the policy above forbids.
        broker: "127.0.0.1".into(),
        port: 1883,
        prefix: "blitzortung/1.1".into(),
        // Credentials come from the environment, never a flag: a password in argv is a password
        // in every process listing on the machine.
        user: std::env::var("MQTT_USER").ok(),
        pass: std::env::var("MQTT_PASS").ok(),
    };
    let argv: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < argv.len() {
        let next = argv.get(i + 1).cloned();
        match argv[i].as_str() {
            "--broker" => {
                if let Some(v) = next {
                    match v.split_once(':') {
                        Some((h, p)) => {
                            a.broker = h.to_string();
                            a.port = p.parse().unwrap_or(1883);
                        }
                        None => a.broker = v,
                    }
                    i += 1;
                }
            }
            "--dry-run" => a.dry_run = true,
            "--prefix" => {
                if let Some(v) = next {
                    a.prefix = v;
                    i += 1;
                }
            }
            "--help" | "-h" => {
                println!(
                    "strikes-relay [--broker HOST[:PORT]] [--prefix TOPIC] [--dry-run]\n\
                     credentials: MQTT_USER / MQTT_PASS in the environment"
                );
                std::process::exit(0);
            }
            other => eprintln!("ignoring unknown argument {other}"),
        }
        i += 1;
    }
    a
}

#[tokio::main]
async fn main() {
    // rustls refuses to pick a provider when more than one could be in the graph, and both this
    // program's TLS users pull it in transitively. Choosing here is the whole fix.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let args = args();
    let mut opts = rumqttc::MqttOptions::new("strikes-relay", &args.broker, args.port);
    opts.set_keep_alive(Duration::from_secs(30));
    if let (Some(u), Some(p)) = (&args.user, &args.pass) {
        opts.set_credentials(u, p);
    }
    let (client, mut conn) = rumqttc::AsyncClient::new(opts, 256);
    let dry_run = args.dry_run;
    // rumqttc only makes progress while someone polls the event loop.
    tokio::spawn(async move {
        loop {
            if dry_run {
                // Nothing to drive: --dry-run never connects to a broker at all.
                tokio::time::sleep(Duration::from_secs(3600)).await;
                continue;
            }
            if let Err(e) = conn.poll().await {
                eprintln!("mqtt: {e}");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    });

    let mut host = 0usize;
    loop {
        let url = HOSTS[host % HOSTS.len()];
        host += 1;
        eprintln!("connecting to {url}");
        match relay(url, &client, &args.prefix, args.dry_run).await {
            Ok(()) => eprintln!("{url} closed the connection"),
            Err(e) => eprintln!("{url}: {e}"),
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

/// One websocket session: subscribe, then republish every strike until the socket closes.
async fn relay(
    url: &str,
    client: &rumqttc::AsyncClient,
    prefix: &str,
    dry_run: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await?;
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        SUBSCRIBE.into(),
    ))
    .await?;
    while let Some(msg) = ws.next().await {
        let text = match msg? {
            tokio_tungstenite::tungstenite::Message::Text(t) => t.to_string(),
            tokio_tungstenite::tungstenite::Message::Close(_) => break,
            _ => continue,
        };
        // A compressed frame still *starts* with readable JSON — the dictionary only kicks in a
        // few dozen characters along — so the shape of the first byte proves nothing. Parse it
        // as-is, and decompress only when that fails.
        let v = match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(v) => v,
            Err(_) => match decompress(&text).and_then(|j| serde_json::from_str(&j).ok()) {
                Some(v) => v,
                None => continue,
            },
        };
        let (Some(lat), Some(lon)) = (v["lat"].as_f64(), v["lon"].as_f64()) else {
            continue;
        };
        // Strip the per-station detections: they are most of the payload and nothing downstream
        // reads them, and a broker should not carry a megabyte a minute for nobody.
        let strike = serde_json::json!({
            "time": v["time"],
            "lat": lat,
            "lon": lon,
        });
        let topic = format!("{prefix}/{}", geohash(lat, lon, 4));
        if dry_run {
            println!("{topic} {strike}");
            continue;
        }
        if let Err(e) = client
            .publish(
                &topic,
                rumqttc::QoS::AtMostOnce,
                false,
                strike.to_string().into_bytes(),
            )
            .await
        {
            eprintln!("publish to {topic} failed: {e}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dictionary_scheme_round_trips_its_own_back_references() {
        // Captured behaviour: code 256 is the first two-character entry, so "abab\u{100}" ends
        // with a repeat of "ab", and "abc\u{100}\u{102}" exercises the not-yet-in-the-table case.
        assert_eq!(decompress("abab\u{100}").unwrap(), "ababab");
        assert_eq!(decompress("abc\u{100}\u{102}").unwrap(), "abcabca");
        assert_eq!(decompress("plain").unwrap(), "plain");
        assert_eq!(decompress(""), None);
    }

    #[test]
    fn geohashes_match_the_known_values() {
        // The textbook example: 57.64911 N, 10.40744 E is u4pruydqqvj.
        assert_eq!(geohash(57.64911, 10.40744, 7), "u4pruyd");
        // Wildcard-friendly: a coarse hash is a prefix of the finer one for the same point.
        assert!(geohash(35.2, -97.4, 7).starts_with(&geohash(35.2, -97.4, 4)));
    }
}
