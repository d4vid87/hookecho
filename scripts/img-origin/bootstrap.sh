#!/usr/bin/env bash
# Stand up the img.hookecho.io origin on a fresh box: the serve process, the Cloudflare
# tunnel that reaches it, and the timer that keeps the national mosaic warm.
#
# Idempotent — every step checks before it acts, so re-running after a reinstall, a
# rebuild or a half-finished attempt is the supported way to use it. Nothing here is a
# secret in the repo: the token is generated on the box on first run.
#
# Prerequisites you have to do yourself (both open a browser):
#   rustup default stable && cargo build --release
#   cloudflared tunnel login
#
# Usage: scripts/img-origin/bootstrap.sh [--port 8087] [--hostname img.hookecho.io]
set -euo pipefail

port=8087
hostname=img.hookecho.io
tunnel=hookecho-img
metrics=127.0.0.1:20241
repo="$(cd "$(dirname "$0")/../.." && pwd)"
units="$HOME/.config/systemd/user"
conf="$HOME/.config/hookecho"

while [ $# -gt 0 ]; do
  case "$1" in
    --port) port="$2"; shift 2 ;;
    --hostname) hostname="$2"; shift 2 ;;
    --tunnel) tunnel="$2"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

need() { command -v "$1" >/dev/null || { echo "missing $1 — $2" >&2; exit 1; }; }
need cloudflared "pacman -S cloudflared, then: cloudflared tunnel login"
need ffmpeg "pacman -S ffmpeg — /loop.mp4 needs it"
need curl "pacman -S curl"

binary="$repo/target/release/hookecho"
[ -x "$binary" ] || { echo "no build at $binary — cargo build --release" >&2; exit 1; }

mkdir -p "$conf" "$units"

# The token. Generated once, 0600, and mirrored into settings.json so the serve process
# and anything asking for the full surface agree without the token living on a command line.
token_file="$conf/img-token"
if [ ! -s "$token_file" ]; then
  ( umask 077; head -c 32 /dev/urandom | base64 | tr -d '=+/\n' > "$token_file" )
  echo "generated $token_file"
fi
chmod 600 "$token_file"
token="$(cat "$token_file")"
python3 - "$conf/settings.json" "$token" <<'PY'
import json, os, sys
path, token = sys.argv[1], sys.argv[2]
settings = json.load(open(path)) if os.path.exists(path) else {}
if settings.get("serve_token") != token:
    settings["serve_token"] = token
    json.dump(settings, open(path, "w"), indent=2)
    print("wrote serve_token into", path)
PY

# The tunnel. `tunnel create` is the only step that needs the browser login; the credentials file
# it writes is what the run command reads — and that file is on the box, not at Cloudflare. A
# reinstall therefore leaves a tunnel that exists remotely and cannot be run here, which looks
# like "already configured" and fails at startup. Existing-but-unrunnable is deleted and remade.
tunnel_id_of() {
  cloudflared tunnel list --output json 2>/dev/null | python3 -c '
import json, sys
name = sys.argv[1]
# A live tunnel carries the zero timestamp rather than no field at all.
deleted = lambda t: not t.get("deleted_at", "").startswith("0001-01-01")
print(next((t["id"] for t in json.load(sys.stdin) if t["name"] == name and not deleted(t)), ""))
' "$1"
}
existing="$(tunnel_id_of "$tunnel")"
if [ -n "$existing" ] && [ ! -s "$HOME/.cloudflared/$existing.json" ]; then
  echo "tunnel $tunnel exists but its credentials are not on this box — recreating"
  cloudflared tunnel cleanup "$tunnel" || true
  cloudflared tunnel delete "$tunnel"
  existing=""
fi
[ -n "$existing" ] || cloudflared tunnel create "$tunnel"
cloudflared tunnel route dns --overwrite-dns "$tunnel" "$hostname"
tunnel_id="$(tunnel_id_of "$tunnel")"
cat > "$HOME/.cloudflared/config.yml" <<YAML
tunnel: $tunnel_id
credentials-file: $HOME/.cloudflared/$tunnel_id.json
metrics: $metrics
ingress:
  - hostname: $hostname
    service: http://127.0.0.1:$port
  - service: http_status:404
YAML

cat > "$units/hookecho-img.service" <<UNIT
[Unit]
Description=hookecho image origin (loopback, presets public)
After=network-online.target

[Service]
ExecStart=$binary --serve $port --public
Restart=always
RestartSec=5

[Install]
WantedBy=default.target
UNIT

cat > "$units/cloudflared-hookecho-img.service" <<UNIT
[Unit]
Description=Cloudflare tunnel for $hostname
After=network-online.target hookecho-img.service

[Service]
ExecStart=$(command -v cloudflared) --no-autoupdate tunnel run $tunnel
Restart=always
RestartSec=5

[Install]
WantedBy=default.target
UNIT

cat > "$units/hookecho-national.service" <<UNIT
[Unit]
Description=Keep the national mosaic warm

[Service]
Type=oneshot
ExecStart=$(command -v curl) -fsS -o /dev/null http://127.0.0.1:$port/national.png
UNIT

cat > "$units/hookecho-national.timer" <<'UNIT'
[Unit]
Description=Re-render the national mosaic every four minutes

[Timer]
OnBootSec=2min
OnUnitActiveSec=4min

[Install]
WantedBy=timers.target
UNIT

loginctl enable-linger "$USER" >/dev/null
systemctl --user daemon-reload
systemctl --user enable --now hookecho-img.service cloudflared-hookecho-img.service hookecho-national.timer
systemctl --user restart hookecho-img.service cloudflared-hookecho-img.service

for _ in $(seq 30); do
  curl -fsS -o /dev/null "http://127.0.0.1:$port/health.json" && break
  sleep 1
done
curl -fsS -o /dev/null "http://127.0.0.1:$port/health.json" || { echo "origin did not come up on $port" >&2; exit 1; }
echo "origin healthy on 127.0.0.1:$port, tunnel $tunnel_id -> $hostname"
echo "warm the disk once: scripts/warm-presets.sh"
