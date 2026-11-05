# Settings sync with Google

Sign in once and your settings — saved locations, placefiles, palette choices, theme, alert
configuration and API keys — stay the same on every machine you run HookEcho on.

The data lives in **your own Google Drive**, in the hidden per-app folder (`appDataFolder`) that
only this app and you can see. There is no HookEcho server, no account, and nothing to pay for.
The OAuth grant covers exactly one scope, `drive.appdata`: it cannot read the rest of your Drive.

Device-specific settings, including screen scale, your position-sharing device name, Android
background alerts, and executable plugins, stay local. Custom palette contents travel with the
settings. Custom marker images and sound files stay on their original device; synchronization
preserves the receiving device's choices when those files are unavailable.

## One-time setup: your own OAuth client

HookEcho ships no client id. A shared one in an open-source binary would put every user on one
quota, and one abusive user could get it revoked for everybody. Making your own takes a few
minutes and it is yours.

1. Open the [Google Cloud console](https://console.cloud.google.com/) and create a project (any
   name).
2. **APIs & Services → Library →** enable the **Google Drive API**.
3. **APIs & Services → OAuth consent screen**: pick **External**, fill in the app name and your
   email, and add yourself under **Test users**. You do not need to publish or get verified —
   `drive.appdata` is not a restricted scope, and a testing-mode app works indefinitely for its
   own test users.
4. **APIs & Services → Credentials → Create credentials → OAuth client ID**, application type
   **Desktop app**. That type allows the loopback redirect (`http://127.0.0.1:<port>`) the app
   signs in with — no redirect URIs to configure, any port is accepted.

   (Not "TVs and Limited Input devices": Google refuses Drive scopes on the device-code flow with
   `invalid device flow scope`.)
5. Copy the **client ID** and **client secret** into **Settings → Sync** in the app.

The "secret" here is not really secret — Google issues it to installed apps that cannot keep one,
which is why the flow also uses PKCE. Treat it as an identifier.

## Signing in

**Settings → Sync → Sign in with Google.** Your browser opens at Google; approve, and the page
redirects to a port the app is listening on, which finishes the sign-in. If no browser opens
(some phones, or a desktop with no `xdg-open`), the Sync tab shows the link with a **Copy link**
button — pasting it into any browser *on the same device* works just as well. The refresh
token is stored in `google-tokens.json` next to `settings.json`, never inside it — that file is
what travels, and credentials must not.

After that the app syncs on start and every 5 minutes, and you can force one with **Sync now**.

## How conflicts resolve

Each machine remembers the Drive timestamp and a hash of its own settings as of the last
successful sync, so it can tell which side changed:

| Local changed | Remote changed | Result |
|---|---|---|
| yes | no | push |
| no | yes | pull |
| no | no | nothing |
| yes | yes | pause and ask which copy to keep |

When both copies have edits, **Settings → Sync** offers **Keep this device** and **Use Drive
copy**. Automatic synchronization stays paused until you choose. Keep this device uploads your
latest settings, including changes made after the conflict appeared; Use Drive copy replaces
the shared settings with the downloaded copy. Edits made while a download is running also
trigger this choice instead of being silently overwritten.

## Signing out

**Sign out** forgets the tokens and the sync bookkeeping on this machine only. The Drive copy is
left alone — signing out of a laptop must not wipe the phone.
It also cancels pending synchronization and discards its results, so a late response cannot
sign you back in or replace your settings.

To delete the synced data entirely, revoke the app at
[myaccount.google.com/permissions](https://myaccount.google.com/permissions); that removes its
app-data folder with it.
