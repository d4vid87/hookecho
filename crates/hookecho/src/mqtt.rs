//! MQTT publishing: what this app knows, pushed onto the broker the rest of the house listens to.
//!
//! `--serve` answers questions; nothing there pushes. Home Assistant, Node-RED and a shelf of
//! ESPHome things are all already subscribed to a broker, and an automation that waits for a
//! tornado warning cannot poll for it. Three topics, under a prefix the user picks:
//!
//! - `<prefix>/status` — the whole [`crate::status`] report as JSON, retained.
//! - `<prefix>/nearest` — the closest tracked storm cell to the home spot, retained.
//! - `<prefix>/alerts` — one message per delivered alert, not retained (an alert is an event; a
//!   subscriber that connects tomorrow must not be handed yesterday's tornado).
//!
//! Native only: a browser has no TCP socket, and the Android alert path is its own service.
//!
//! Credentials are a user secret — settings.json only, never the shots template, never logged.
//
// ponytail: one broker, one thread each way. Commands are three verbs (site, product, mute), not
// a remote-control protocol — anything richer belongs behind the HTTP server, which already
// answers questions.

use crate::settings::Settings;
use crate::status::Spot;
use rumqttc::{Client, MqttOptions, QoS, Transport};
use std::sync::OnceLock;
use std::time::Duration;

/// A command that arrived from the broker. The app drains these once a frame and applies them,
/// so a house automation can point the window at the storm it just noticed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cmd {
    /// Show this radar site in the active pane.
    Site(String),
    /// Switch the active pane to this product code (`REF`, `VEL`, …).
    Product(String),
    /// Mute or unmute alert sound.
    Mute(bool),
}

/// Parse one command from its topic suffix and payload.
///
/// Everything arriving here came off a broker, which is a trust boundary even when the broker is
/// on the same machine: a payload is whatever anyone with publish rights felt like sending. Site
/// ids are validated to the shape a site id has, products go through `Moment::from_code`, and
/// anything else is `None` rather than a guess.
pub fn parse_cmd(suffix: &str, payload: &str) -> Option<Cmd> {
    let payload = payload.trim();
    match suffix {
        "site" => {
            let id = payload.to_ascii_uppercase();
            let ok = (3..=5).contains(&id.len()) && id.chars().all(|c| c.is_ascii_alphanumeric());
            ok.then_some(Cmd::Site(id))
        }
        "product" => {
            let code = payload.to_ascii_uppercase();
            wxdata::level2::Moment::from_code(&code).map(|_| Cmd::Product(code))
        }
        "mute" => match payload.to_ascii_lowercase().as_str() {
            "on" | "1" | "true" | "mute" => Some(Cmd::Mute(true)),
            "off" | "0" | "false" | "unmute" => Some(Cmd::Mute(false)),
            _ => None,
        },
        _ => None,
    }
}

/// Commands parked by the event loop for the app to drain. A bounded channel would drop the
/// command a user is waiting on; an unbounded one grows only if nobody drains it, which is
/// exactly why [`spawn`] takes `subscribe`.
static CMD_RX: std::sync::Mutex<Option<std::sync::mpsc::Receiver<Cmd>>> =
    std::sync::Mutex::new(None);

/// Take everything the broker has sent since the last frame.
pub fn drain() -> Vec<Cmd> {
    CMD_RX
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|rx| rx.try_iter().collect()))
        .unwrap_or_default()
}

/// The Home Assistant discovery configs, as `(topic, payload)` pairs.
///
/// Published retained so Home Assistant finds the device whenever it restarts, and republished on
/// every reconnect — which is idempotent, since a retained config is overwritten rather than
/// appended.
fn discovery(prefix: &str) -> Vec<(String, String)> {
    let device = serde_json::json!({
        "identifiers": ["hookecho"],
        "name": "HookEcho",
        "manufacturer": "HookEcho",
        "model": "radar",
    });
    vec![
        (
            "homeassistant/sensor/hookecho_status/config".to_string(),
            serde_json::json!({
                "name": "HookEcho status",
                "unique_id": "hookecho_status",
                "state_topic": format!("{prefix}/status"),
                "value_template": "{{ value_json | length }}",
                "json_attributes_topic": format!("{prefix}/nearest"),
                "device": device,
            })
            .to_string(),
        ),
        (
            "homeassistant/sensor/hookecho_nearest/config".to_string(),
            serde_json::json!({
                "name": "HookEcho nearest cell",
                "unique_id": "hookecho_nearest",
                "state_topic": format!("{prefix}/nearest"),
                // A cell that is not there is `null`, which has no distance — the sensor goes
                // unavailable rather than reporting a made-up zero.
                "value_template": "{{ value_json.distance_km | default('unavailable') }}",
                "unit_of_measurement": "km",
                "device": device,
            })
            .to_string(),
        ),
        (
            "homeassistant/sensor/hookecho_alerts/config".to_string(),
            serde_json::json!({
                "name": "HookEcho last alert",
                "unique_id": "hookecho_alerts",
                "state_topic": format!("{prefix}/alerts"),
                "value_template": "{{ value_json.title }}",
                "device": device,
            })
            .to_string(),
        ),
        (
            "homeassistant/switch/hookecho_mute/config".to_string(),
            serde_json::json!({
                "name": "HookEcho mute",
                "unique_id": "hookecho_mute",
                "command_topic": format!("{prefix}/cmd/mute"),
                "payload_on": "on",
                "payload_off": "off",
                "optimistic": true,
                "device": device,
            })
            .to_string(),
        ),
    ]
}

/// How often the retained status/nearest topics are refreshed. A volume is ~5 minutes wide and
/// the observation feeds update slower than that, so anything quicker only spends network.
const POLL: Duration = Duration::from_secs(300);

/// The connected client, once [`spawn`] has one. `None` until then, and permanently absent when
/// no broker is configured — [`publish_alert`] is a no-op in both cases.
static CLIENT: OnceLock<Client> = OnceLock::new();

/// A topic prefix the user typed, made safe to concatenate onto.
///
/// **Trust boundary**: `#` and `+` are MQTT wildcards and are illegal in a topic being published
/// to; a broker drops the whole publish (or the connection) rather than guessing. Empty segments
/// from a stray `/` are legal but publish to a topic nobody can type back, so they go too.
fn clean_prefix(raw: &str) -> String {
    let cleaned: Vec<&str> = raw
        .split('/')
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.contains(['#', '+']))
        .collect();
    if cleaned.is_empty() {
        "hookecho".to_string()
    } else {
        cleaned.join("/")
    }
}

/// The JSON one alert is published as. A subscriber wants the fields apart, not a sentence to
/// parse back out — `title` is the warning, `body` is where and until when.
fn alert_payload(title: &str, body: &str, urgent: bool) -> String {
    serde_json::json!({
        "title": title,
        "body": body,
        "urgent": urgent,
        "time": chrono::Utc::now().to_rfc3339(),
    })
    .to_string()
}

/// Start publishing, if a broker is configured and one is not running already.
///
/// Two threads: rumqttc's event loop must be driven by someone, and the status poll blocks on
/// feeds for seconds at a time, so it cannot be the same one.
pub fn spawn(settings: &Settings, subscribe: bool) {
    let host = settings.mqtt_host.trim().to_string();
    if host.is_empty() || CLIENT.get().is_some() {
        return;
    }
    let prefix = clean_prefix(&settings.mqtt_prefix);
    // A fixed client id would fight itself if the app and a `--serve` container both published;
    // the pid keeps two on one machine apart without asking the user for a name.
    let mut opts = MqttOptions::new(
        format!("hookecho-{}", std::process::id()),
        host,
        settings.mqtt_port,
    );
    opts.set_keep_alive(Duration::from_secs(30));
    if settings.mqtt_tls {
        opts.set_transport(Transport::tls_with_default_config());
    }
    let (user, pass) = (settings.mqtt_user.trim(), settings.mqtt_pass.trim());
    if !user.is_empty() {
        opts.set_credentials(user, pass);
    }
    // Capacity is the publish queue, not a message history: an outbreak is a few dozen alerts.
    let (client, mut conn) = Client::new(opts, 64);
    if CLIENT.set(client.clone()).is_err() {
        return;
    }
    log::info!("mqtt: publishing under {prefix}/");

    let discovery_on = settings.mqtt_discovery;
    let (tx, rx) = std::sync::mpsc::channel();
    if subscribe {
        if let Ok(mut g) = CMD_RX.lock() {
            *g = Some(rx);
        }
    }
    let cmd_prefix = format!("{prefix}/cmd/");
    let sub_client = client.clone();
    let sub_prefix = prefix.clone();
    std::thread::spawn(move || {
        // Iterating the connection *is* the reconnect loop — rumqttc yields the error and then
        // tries again, so this only ends when the process does.
        for event in conn.iter() {
            match event {
                // A fresh connection is where subscriptions and retained configs go: this fires
                // again after every reconnect, and both are idempotent.
                Ok(rumqttc::Event::Incoming(rumqttc::Packet::ConnAck(_))) => {
                    if subscribe {
                        let topic = format!("{sub_prefix}/cmd/#");
                        if let Err(e) = sub_client.subscribe(&topic, QoS::AtLeastOnce) {
                            log::warn!("mqtt: subscribing to {topic} failed: {e}");
                        }
                    }
                    if discovery_on {
                        for (topic, payload) in discovery(&sub_prefix) {
                            publish(&sub_client, &topic, payload, true);
                        }
                    }
                }
                Ok(rumqttc::Event::Incoming(rumqttc::Packet::Publish(p))) => {
                    let Some(suffix) = p.topic.strip_prefix(&cmd_prefix) else {
                        continue;
                    };
                    let payload = String::from_utf8_lossy(&p.payload);
                    match parse_cmd(suffix, &payload) {
                        Some(cmd) => {
                            let _ = tx.send(cmd);
                        }
                        None => log::warn!("mqtt: ignoring {} = {payload:?}", p.topic),
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    log::debug!("mqtt: {e}");
                    std::thread::sleep(Duration::from_secs(5));
                }
            }
        }
    });

    let spots = crate::status::spots(settings, None);
    std::thread::spawn(move || status_loop(client, prefix, spots));
}

/// Publish the retained topics forever. Builds its own runtime: the callers are the GUI (whose
/// runtime is busy drawing) and `--serve` (whose runtime answers requests).
fn status_loop(client: Client, prefix: String, spots: Vec<Spot>) {
    let Ok(rt) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        log::warn!("mqtt: no runtime for the status poll");
        return;
    };
    let http = reqwest::Client::new();
    loop {
        match rt.block_on(crate::status::collect(&http, &spots)) {
            Ok(report) => {
                if let Ok(json) = serde_json::to_string(&report) {
                    publish(&client, &format!("{prefix}/status"), json, true);
                }
                // The home spot's cell is the one an automation acts on ("is anything coming
                // here"); the rest of the report is in `/status` for whoever wants it.
                let home = report.iter().find(|s| s.home).or_else(|| report.first());
                let nearest = home.and_then(|s| s.nearest_cell.as_ref());
                let payload = match nearest {
                    Some(c) => serde_json::to_string(c).unwrap_or_else(|_| "null".to_string()),
                    // Explicitly null, not silence: "nothing is being tracked" is an answer, and a
                    // retained topic that stops updating looks the same as a dead app otherwise.
                    None => "null".to_string(),
                };
                publish(&client, &format!("{prefix}/nearest"), payload, true);
            }
            Err(e) => log::warn!("mqtt: status collect failed: {e}"),
        }
        std::thread::sleep(POLL);
    }
}

/// One publish, with the failure logged and swallowed — a broker being down is not a reason for
/// the app to stop drawing radar.
fn publish(client: &Client, topic: &str, payload: String, retain: bool) {
    if let Err(e) = client.publish(topic, QoS::AtLeastOnce, retain, payload) {
        log::debug!("mqtt: publish to {topic} failed: {e}");
    }
}

/// Push one alert, if a broker is connected. Called from the same place the webhooks are, so
/// quiet hours and outbreak rollup have already had their say.
pub fn publish_alert(settings: &Settings, title: &str, body: &str, urgent: bool) {
    let Some(client) = CLIENT.get() else {
        return;
    };
    let topic = format!("{}/alerts", clean_prefix(&settings.mqtt_prefix));
    publish(client, &topic, alert_payload(title, body, urgent), false);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A payload off a broker is whatever anyone with publish rights sent, so the parser is a
    /// trust boundary and not a convenience.
    #[test]
    fn commands_are_validated_before_they_are_believed() {
        assert_eq!(parse_cmd("site", "ktlx"), Some(Cmd::Site("KTLX".into())));
        assert_eq!(
            parse_cmd("site", " kfws \n"),
            Some(Cmd::Site("KFWS".into()))
        );
        assert_eq!(parse_cmd("site", "../../etc"), None);
        assert_eq!(parse_cmd("site", "K"), None);
        assert_eq!(parse_cmd("site", "KTLX; DROP"), None);

        assert_eq!(
            parse_cmd("product", "vel"),
            Some(Cmd::Product("VEL".into()))
        );
        assert_eq!(parse_cmd("product", "nope"), None);

        assert_eq!(parse_cmd("mute", "ON"), Some(Cmd::Mute(true)));
        assert_eq!(parse_cmd("mute", "0"), Some(Cmd::Mute(false)));
        assert_eq!(parse_cmd("mute", "maybe"), None);

        // An unknown verb is silence, not a guess.
        assert_eq!(parse_cmd("reboot", "on"), None);
    }

    /// Home Assistant keys entities by `unique_id` and the configs are retained, so a collision
    /// or a stray topic is a mess in somebody's house that outlives the app.
    #[test]
    fn discovery_configs_are_distinct_and_point_at_our_prefix() {
        let configs = discovery("home/weather");
        assert_eq!(configs.len(), 4);
        let mut ids: Vec<String> = Vec::new();
        for (topic, payload) in &configs {
            assert!(topic.starts_with("homeassistant/"), "{topic}");
            let v: serde_json::Value = serde_json::from_str(payload).expect("config is JSON");
            let id = v["unique_id"].as_str().expect("unique_id").to_string();
            assert!(!ids.contains(&id), "duplicate unique_id {id}");
            ids.push(id);
            let points_at_us = v["state_topic"]
                .as_str()
                .or_else(|| v["command_topic"].as_str())
                .expect("a topic");
            assert!(points_at_us.starts_with("home/weather/"), "{points_at_us}");
            assert_eq!(v["device"]["identifiers"][0], "hookecho");
        }
        // The switch commands the topic the subscribe loop actually listens on.
        let (_, mute) = configs.last().unwrap();
        assert!(mute.contains("home/weather/cmd/mute"));
    }

    #[test]
    fn a_prefix_cannot_carry_a_wildcard_into_a_publish() {
        assert_eq!(clean_prefix("home/weather"), "home/weather");
        assert_eq!(clean_prefix("/home//weather/"), "home/weather");
        assert_eq!(clean_prefix("  spaced  "), "spaced");
        // Wildcards are illegal in a published topic; a broker drops the publish rather than
        // interpreting it, so the segment goes instead of the whole message.
        assert_eq!(clean_prefix("home/#"), "home");
        assert_eq!(clean_prefix("home/+/x"), "home/x");
        // Nothing usable left means the default, not a publish to "".
        assert_eq!(clean_prefix(""), "hookecho");
        assert_eq!(clean_prefix("#"), "hookecho");
    }

    #[test]
    fn an_alert_publishes_its_fields_apart() {
        let v: serde_json::Value = serde_json::from_str(&alert_payload(
            "Tornado Warning",
            "Norman until 21:15",
            true,
        ))
        .unwrap();
        assert_eq!(v["title"], "Tornado Warning");
        assert_eq!(v["body"], "Norman until 21:15");
        assert_eq!(v["urgent"], true);
        assert!(v["time"].as_str().unwrap().contains('T'));
    }
}
