# strikes-relay

Republishes Blitzortung lightning strikes onto **your own** MQTT broker, so HookEcho's
lightning-strikes layer has something to subscribe to.

## Why a relay at all

Blitzortung is a volunteer network. Their terms ask that third-party applications not point their
users' clients at the network's servers — what a participant may do is run a relay for their own
household. So HookEcho never connects to Blitzortung. It subscribes to a topic on a broker you
run, and this program is what fills that topic. Anything else that already publishes the same
shape (the Home Assistant Blitzortung integration, for instance) works just as well; you do not
need this program if you have that.

Personal, non-commercial use, one relay per household. If you find this useful, contribute a
station.

## Run it

```sh
./bootstrap.sh          # build, install to ~/.local/bin, enable the user service
```

Or by hand:

```sh
cargo run --release -- --broker 127.0.0.1:1883
cargo run --release -- --dry-run     # print strikes instead, no broker needed
```

Flags: `--broker HOST[:PORT]` (default `127.0.0.1:1883`), `--prefix TOPIC` (default
`blitzortung/1.1`), `--dry-run`. Broker credentials come from `MQTT_USER` and `MQTT_PASS` in the
environment — never from a flag, because argv is visible in every process listing. The systemd
unit reads them from `~/.config/strikes-relay.env` if that file exists.

Then in HookEcho: **Settings → MQTT → Strikes topic**, `blitzortung/1.1/#`.

## What it publishes

`blitzortung/1.1/{geohash}` (4-character geohash, so a subscriber can take a region with a
wildcard), payload `{"time": <nanoseconds>, "lat": <deg>, "lon": <deg>}`. The per-station
detections in the upstream frame are dropped: they are most of the payload and nothing downstream
reads them.

## Not part of the workspace

This crate is excluded from the HookEcho workspace and keeps its own `Cargo.lock`, so its
dependencies cannot move the app's. It is not built by any of the app's CI gates; build and test
it here:

```sh
cargo build && cargo test
```
