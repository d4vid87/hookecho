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
// ponytail: one broker, one publisher thread, publish-only — no subscribe side and no Home
// Assistant discovery config. The ceiling is "a house automates on this"; if someone wants
// commands back (change site, start a loop) that is a subscribe loop on the same client.

use crate::settings::Settings;
use crate::status::Spot;
use rumqttc::{Client, MqttOptions, QoS, Transport};
use std::sync::OnceLock;
use std::time::Duration;

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
pub fn spawn(settings: &Settings) {
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

    std::thread::spawn(move || {
        // Iterating the connection *is* the reconnect loop — rumqttc yields the error and then
        // tries again, so this only ends when the process does.
        for event in conn.iter() {
            if let Err(e) = event {
                log::debug!("mqtt: {e}");
                std::thread::sleep(Duration::from_secs(5));
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
