"""Constants, and the one place the server's JSON field names are written down.

`/status.json` is produced by `crates/hookecho/src/status.rs` (`SpotStatus`). If a
field is renamed there, this mapping is what has to change with it — keeping it in
one file is deliberate.
"""

from datetime import timedelta

DOMAIN = "hookecho"

CONF_HOST = "host"
CONF_PORT = "port"

DEFAULT_PORT = 8080
UPDATE_INTERVAL = timedelta(seconds=60)

# Server JSON keys.
KEY_NAME = "name"
KEY_HOME = "home"
KEY_STATION = "station"
KEY_ALERTS = "alerts"
KEY_ALERT_EVENT = "event"
KEY_ALERT_UNTIL = "until"
KEY_ALERT_DISTANCE = "distance_km"
KEY_ALERT_ESCALATION = "escalation"

# (json key, sensor key, name suffix, unit, device class, state class)
MEASUREMENTS = [
    ("temp_f", "temperature", "temperature", "°F", "temperature", "measurement"),
    ("dewpoint_f", "dewpoint", "dewpoint", "°F", "temperature", "measurement"),
    ("rh", "humidity", "humidity", "%", "humidity", "measurement"),
    ("wind_kt", "wind", "wind", "kn", "wind_speed", "measurement"),
    ("gust_kt", "gust", "wind gust", "kn", "wind_speed", "measurement"),
    ("pressure_in", "pressure", "pressure", "inHg", "pressure", "measurement"),
]


def status_url(host: str, port: int) -> str:
    """The endpoint the coordinator polls."""
    return f"http://{host}:{port}/status.json"


def snapshot_url(host: str, port: int, site: str | None = None) -> str:
    """The radar image endpoint; `site` empty lets the server pick its default."""
    url = f"http://{host}:{port}/snapshot.png"
    return f"{url}?site={site}" if site else url
