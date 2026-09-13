# Bundled Piper voice

HookEcho uses Piper `2023.11.14-2` with `en_US-amy-medium`. `assets.json` pins every
download URL and SHA-256. `fetch.sh` refuses mismatched bytes and copies upstream notices.
Piper is MIT; Amy is redistributed under CC BY-SA 4.0 with MycroftAI attribution.

Assets are fetched while packaging, not committed to Git. Installed packages remain complete
and offline-capable without bloating Git history.
