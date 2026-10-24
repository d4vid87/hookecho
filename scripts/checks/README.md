# Website media check

Build `site/`, serve `site/dist` on port 8095, and run:

```sh
node scripts/checks/site-media.cjs
```

Requires Playwright and Chromium (`/usr/bin/chromium`). Checks both product GIFs,
pause/resume controls, reduced-motion posters, and mobile horizontal overflow.

# Linux AppImage payload

`packaging/build-appimage.sh` builds on Ubuntu 22.04 and runs
`bash scripts/checks/appimage.sh target/appimage/HookEcho.AppDir` before packaging.
The check enforces glibc <= 2.35 for bundled ELF files, verifies bundled ALSA and
licenses, and exercises display-free help/version/error handling, icon rendering
and offline English speech. A newer build host is rejected rather than silently
raising the Linux runtime requirement.
