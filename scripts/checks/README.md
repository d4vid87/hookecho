# Website media check

Build `site/`, serve `site/dist` on port 8095, and run:

```sh
node scripts/checks/site-media.cjs
```

Requires Playwright and Chromium (`/usr/bin/chromium`). Checks both product GIFs,
pause/resume controls, reduced-motion posters, and mobile horizontal overflow.
