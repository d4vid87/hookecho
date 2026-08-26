# Settings sync with Google

Sign in once and your settings — saved locations, placefiles, palette choices, theme, alert
configuration and API keys — stay the same on every machine you run HookEcho on.

The data lives in **your own Google Drive**, in the hidden per-app folder (`appDataFolder`) that
only this app and you can see. There is no HookEcho server, no account, and nothing to pay for.
The OAuth grant covers exactly one scope, `drive.appdata`: it cannot read the rest of your Drive.

Three things stay local to each machine, because they describe the machine and not you:
screen scale (`ui_scale`), your position-sharing device name, and whether Android background
alerts are on. Everything else follows the sync.

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
| yes | yes | **pull**, and the status line says so |

The both-changed case keeps the synced copy on purpose: your local settings are still on disk
in `settings.json` if you need them back, but an unpulled remote edit would be gone for good.

## Signing out

**Sign out** forgets the tokens and the sync bookkeeping on this machine only. The Drive copy is
left alone — signing out of a laptop must not wipe the phone.

To delete the synced data entirely, revoke the app at
[myaccount.google.com/permissions](https://myaccount.google.com/permissions); that removes its
app-data folder with it.
