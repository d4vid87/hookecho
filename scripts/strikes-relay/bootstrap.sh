#!/usr/bin/env bash
# Build the strikes relay and install it as a user service.
#
# One command from a fresh checkout: build, drop the binary in ~/.local/bin, install the unit,
# start it. Broker defaults to 127.0.0.1:1883; edit the unit if yours lives elsewhere.
set -euo pipefail
cd "$(dirname "$0")"

mkdir -p "$HOME/.local/bin" "$HOME/.config/systemd/user"
cargo build --release
install -m 755 target/release/strikes-relay "$HOME/.local/bin/strikes-relay"
install -m 644 strikes-relay.service "$HOME/.config/systemd/user/strikes-relay.service"

systemctl --user daemon-reload
systemctl --user enable --now strikes-relay.service
systemctl --user --no-pager status strikes-relay.service | head -5

cat <<'MSG'

Running. Check it with:
  mosquitto_sub -h 127.0.0.1 -t 'blitzortung/#' -v
Then in HookEcho: Settings -> MQTT -> Strikes topic: blitzortung/1.1/#
MSG
