// HookEcho Lite: fetch, loop and chrome. The wasm next door does the decoding and the drawing.
//
// Everything here is plain DOM and fetch on purpose. The page has to run on a machine that cannot
// run the real app, so it carries no framework, no map library and no build step of its own — the
// basemap is <img> tiles positioned by the same Web Mercator math the wasm uses for the radar,
// which is what keeps the two registered without either one knowing about the other.
//
// Every request is same-origin: /proxy/<host>/… through the existing data proxy (same allowlist
// the app uses), plus /geo.json for the opening site. Nothing is sent anywhere else.

const S3 = "/proxy/unidata-nexrad-level3.s3.amazonaws.com";
// USGS The National Map: keyless, US-only (which is exactly the radar network's footprint) and
// under no tile-usage policy this page could violate. Carto's keyless tiles now come back
// watermarked "API KEY REQUIRED", so they are not an option here.
const TILES = "/proxy/basemap.nationalmap.gov/arcgis/rest/services/USGSTopo/MapServer/tile";
const ALERTS =
  "/proxy/api.weather.gov/alerts/active?status=actual&event=" +
  encodeURIComponent("Tornado Warning,Severe Thunderstorm Warning,Flash Flood Warning");
const WARN_MS = 90000;
const FRAMES = 6; // scans in the loop, ~15 minutes of weather
const FRAME_MS = 400;
const DWELL_MS = 1200; // extra time on the newest frame, so the loop reads as "now"
const REFRESH_MS = 120000;
// The CPU paints every pixel of every frame, so the canvas is capped well below a modern desktop
// window. Full screen makes the map bigger up to this, then stops growing.
const MAX_CANVAS = 1400;
// Zoom range: 5 is most of the country, 12 is a few neighbourhoods. The radar itself runs out at
// about 300 km, so anything wider than 6 is mostly basemap.
const MIN_ZOOM = 5;
const MAX_ZOOM = 12;

const el = (id) => document.getElementById(id);
const clampZoom = (z) => Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, Math.round(z) || 9));
const params = new URLSearchParams(location.search);
// Hero mode: the site embeds this page as the backdrop of its landing hero, so the chrome goes
// away and the page stops touching the parent's history. The radar itself is unchanged — it is
// the real, current scan, which is the whole point of using it as artwork.
const hero = params.get("hero") === "1";

const state = {
  site: null, // { id, city, state, lat, lon }
  product: params.get("prod") === "vel" ? "vel" : "ref",
  zoom: clampZoom(Number(params.get("zoom")) || 9),
  frame: 0,
  times: [],
  playing: true,
  loading: false,
  warnings: [], // [{ event, rings }] already filtered to the current view
};

let allSites = [];
let viewer = null;
let ctx = null;

// --- Mercator, the one piece of geography this page needs -------------------------------------

const worldPx = (z) => 256 * 2 ** z;
const lonToX = (lon, z) => ((lon + 180) / 360) * worldPx(z);
const latToY = (lat, z) => {
  const s = Math.sin((lat * Math.PI) / 180);
  return (0.5 - Math.log((1 + s) / (1 - s)) / (4 * Math.PI)) * worldPx(z);
};

// --- Chrome -----------------------------------------------------------------------------------

function canvasSize() {
  const w = Math.min(el("map").clientWidth, MAX_CANVAS);
  const h = Math.min(el("map").clientHeight, MAX_CANVAS);
  // ponytail: devicePixelRatio is ignored. The whole point of this page is the machine that
  // struggles; a retina-resolution CPU raster is four times the work for a nicer-looking loop.
  return [Math.max(w, 1), Math.max(h, 1)];
}

function drawTiles(w, h) {
  const z = state.zoom;
  const host = el("tiles");
  host.textContent = "";
  const cx = lonToX(state.site.lon, z);
  const cy = latToY(state.site.lat, z);
  const left = cx - w / 2;
  const top = cy - h / 2;
  const n = 2 ** z;
  for (let ty = Math.floor(top / 256); ty <= Math.floor((top + h) / 256); ty++) {
    for (let tx = Math.floor(left / 256); tx <= Math.floor((left + w) / 256); tx++) {
      if (ty < 0 || ty >= n) continue;
      const img = new Image();
      img.loading = "lazy";
      img.alt = "";
      // ArcGIS tile URLs are z/y/x, not z/x/y.
      img.src = `${TILES}/${z}/${ty}/${((tx % n) + n) % n}`;
      img.style.left = `${tx * 256 - left}px`;
      img.style.top = `${ty * 256 - top}px`;
      host.append(img);
    }
  }
}

// Every other NEXRAD in the current view, as a button that switches to it. This is the map
// equivalent of the site picker: on a wide view you can see the neighbouring radar's storms
// arriving and go straight to the site that has them close in.
function drawMarkers(w, h) {
  const host = el("markers");
  host.textContent = "";
  if (!allSites.length) return;
  const z = state.zoom;
  const left = lonToX(state.site.lon, z) - w / 2;
  const top = latToY(state.site.lat, z) - h / 2;
  for (const s of allSites) {
    if (s.id === state.site.id) continue;
    const x = lonToX(s.lon, z) - left;
    const y = latToY(s.lat, z) - top;
    // A margin keeps a marker from hanging half off the edge where it cannot be read.
    if (x < 12 || y < 12 || x > w - 12 || y > h - 12) continue;
    const b = document.createElement("button");
    b.type = "button";
    b.textContent = s.id;
    b.title = `Switch to ${s.city} (${s.id})`;
    b.style.left = `${x}px`;
    b.style.top = `${y}px`;
    b.addEventListener("click", () => goToSite(s));
    host.append(b);
  }
}

async function goToSite(site) {
  state.site = site;
  el("site").value = site.id;
  applyView();
  permalink();
  await Promise.all([loadLoop(), loadWarnings()]);
}

function applyView() {
  const [w, h] = canvasSize();
  el("view").style.width = `${w}px`;
  el("view").style.height = `${h}px`;
  for (const id of ["radar", "warn"]) {
    const c = el(id);
    c.width = w;
    c.height = h;
  }
  showRange(w);
  ctx = el("radar").getContext("2d");
  viewer.set_view(state.site.lat, state.site.lon, w, h, state.zoom);
  drawTiles(w, h);
  drawMarkers(w, h);
}

// How wide the view actually is, which is the thing a zoom control is really setting. Web Mercator
// scale is latitude-dependent, so this is computed rather than labelled per preset.
function showRange(widthPx) {
  const kmPerPx =
    ((40075.017 * Math.cos((state.site.lat * Math.PI) / 180)) / worldPx(state.zoom)) * 1;
  const km = Math.round(widthPx * kmPerPx);
  el("range").textContent = km >= 100 ? `${Math.round(km / 10) * 10} km` : `${km} km`;
}

// The one place a zoom change happens, however it was asked for.
function setZoom(z) {
  const next = clampZoom(z);
  if (next === state.zoom) return;
  state.zoom = next;
  applyView();
  permalink();
  if (ctx) viewer.render(ctx, state.frame);
  loadWarnings();
}

function permalink() {
  if (hero) return; // an iframe rewriting its own URL is noise nobody can see
  const u = new URL(location.href);
  u.searchParams.set("site", state.site.id);
  u.searchParams.set("prod", state.product);
  u.searchParams.set("zoom", String(state.zoom));
  history.replaceState(null, "", u);
}

function showTime() {
  const t = state.times[state.frame];
  const total = viewer.frame_count();
  el("time").textContent = t
    ? `${t.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })} · ${state.frame + 1}/${total}`
    : state.loading
      ? "Loading…"
      : "No scans available";
}

// --- Data -------------------------------------------------------------------------------------

// The Level 3 site id: CONUS ICAO ids drop their leading K (KTLX -> TLX), everything else is
// already three or four characters the bucket uses verbatim.
const l3Site = (id) => (id.length === 4 && id.startsWith("K") ? id.slice(1) : id);

const pad = (n) => String(n).padStart(2, "0");
const dayPrefix = (d) => `${d.getUTCFullYear()}_${pad(d.getUTCMonth() + 1)}_${pad(d.getUTCDate())}`;

// Keys look like TLX_N0B_2026_08_27_18_30_40, in UTC.
function keyTime(key) {
  const m = key.match(/_(\d{4})_(\d\d)_(\d\d)_(\d\d)_(\d\d)_(\d\d)$/);
  if (!m) return null;
  const [, y, mo, d, hh, mm, ss] = m;
  return new Date(Date.UTC(+y, +mo - 1, +d, +hh, +mm, +ss));
}

async function listKeys() {
  const prod = state.product === "vel" ? "N0G" : "N0B";
  const site = l3Site(state.site.id);
  const now = new Date();
  const keys = [];
  // Today first, then yesterday — a site that just rolled past midnight UTC still has a loop.
  for (const d of [now, new Date(now - 864e5)]) {
    const prefix = `${site}_${prod}_${dayPrefix(d)}`;
    // start-after trims a full day of keys (a super-res product scans every couple of minutes)
    // down to the recent ones, which is all a six-frame loop can use.
    const after = new Date(d.getTime() - 3 * 3600e3);
    const start = `${site}_${prod}_${dayPrefix(after)}_${pad(after.getUTCHours())}`;
    const url = `${S3}/?list-type=2&prefix=${prefix}&start-after=${start}`;
    const res = await fetch(url).catch(() => null);
    if (!res || !res.ok) continue;
    const xml = await res.text();
    for (const m of xml.matchAll(/<Key>([^<]+)<\/Key>/g)) keys.push(m[1]);
    if (keys.length >= FRAMES) break;
  }
  return keys.slice(-FRAMES);
}

async function loadLoop() {
  if (state.loading) return;
  state.loading = true;
  showTime();
  try {
    const keys = await listKeys();
    // All six at once. Fetched one after another this was six round trips deep, which is most of
    // the wait on a slow link; the decode below is the same work either way.
    const inflight = keys.map((key) =>
      fetch(`${S3}/${key}`)
        .then((r) => (r.ok ? r.arrayBuffer() : null))
        .then((buf) => (buf ? { key, bytes: new Uint8Array(buf) } : null))
        .catch(() => null),
    );
    viewer.clear_frames();
    const times = [];
    // Awaited in order, not with Promise.all: the requests are already all in flight, and waiting
    // for the slowest one before decoding any of them is how the first frame ends up last.
    for (const pending of inflight) {
      const item = await pending;
      if (!item || !viewer.add_frame(item.bytes)) continue;
      times.push(keyTime(item.key));
      // Paint the oldest frame the moment it decodes rather than sitting on a blank map for the
      // length of five more decodes.
      if (times.length === 1 && ctx) {
        state.frame = 0;
        state.times = times;
        viewer.render(ctx, 0);
        showTime();
      }
      // Decoding six scans is a second of solid work on a slow machine. Yielding between them
      // lets the browser actually put the frame above on screen, and keeps the toolbar clickable
      // while the rest arrive.
      await new Promise((r) => setTimeout(r, 0));
    }
    state.times = times;
    state.frame = Math.max(0, times.length - 1);
    if (ctx) viewer.render(ctx, state.frame);
  } finally {
    state.loading = false;
    showTime();
  }
}

// --- Warnings ---------------------------------------------------------------------------------

// One national query, filtered here. A per-site point query would only return warnings covering
// the radar itself and miss every one in the corner of the view, which is most of them.
const WARN_COLOR = {
  "Tornado Warning": "#ff4d4d",
  "Severe Thunderstorm Warning": "#ffd24d",
  "Flash Flood Warning": "#4dd97a",
};

function viewBounds() {
  const c = el("radar");
  const z = state.zoom;
  const w = worldPx(z);
  const left = lonToX(state.site.lon, z) - c.width / 2;
  const top = latToY(state.site.lat, z) - c.height / 2;
  const lat = (y) => (Math.atan(Math.sinh(Math.PI * (1 - (2 * y) / w))) * 180) / Math.PI;
  return {
    west: (left / w) * 360 - 180,
    east: ((left + c.width) / w) * 360 - 180,
    north: lat(top),
    south: lat(top + c.height),
  };
}

async function loadWarnings() {
  // No Accept header: the proxy's CORS preflight only permits API-Key and User-Agent, and adding
  // one turns every request into a failed preflight.
  const data = await fetch(ALERTS)
    .then((r) => (r.ok ? r.json() : null))
    .catch(() => null);
  if (!data) return;
  const b = viewBounds();
  const kept = [];
  for (const f of data.features ?? []) {
    // Zone-based alerts carry no geometry; there is nothing to outline.
    const g = f.geometry;
    if (!g) continue;
    const polys = g.type === "MultiPolygon" ? g.coordinates : g.type === "Polygon" ? [g.coordinates] : [];
    const rings = [];
    for (const poly of polys) {
      for (const ring of poly) {
        const hit = ring.some(
          ([lon, lat]) => lon >= b.west && lon <= b.east && lat >= b.south && lat <= b.north,
        );
        if (hit) rings.push(ring);
      }
    }
    if (rings.length) kept.push({ event: f.properties.event, rings });
  }
  state.warnings = kept;
  drawWarnings();
}

function drawWarnings() {
  const c = el("warn");
  const g = c.getContext("2d");
  g.clearRect(0, 0, c.width, c.height);
  const z = state.zoom;
  const left = lonToX(state.site.lon, z) - c.width / 2;
  const top = latToY(state.site.lat, z) - c.height / 2;
  g.lineWidth = 2;
  for (const wrn of state.warnings) {
    g.strokeStyle = WARN_COLOR[wrn.event] ?? "#ffffff";
    for (const ring of wrn.rings) {
      g.beginPath();
      ring.forEach(([lon, lat], i) => {
        const x = lonToX(lon, z) - left;
        const y = latToY(lat, z) - top;
        if (i === 0) g.moveTo(x, y);
        else g.lineTo(x, y);
      });
      g.closePath();
      g.stroke();
    }
  }
  const counts = new Map();
  for (const wrn of state.warnings) counts.set(wrn.event, (counts.get(wrn.event) ?? 0) + 1);
  el("warns").innerHTML = [...counts]
    .map(([event, n]) => `<b style="color:${WARN_COLOR[event] ?? "#fff"}">${n}</b> ${event.replace(" Warning", "")}`)
    .join(" · ");
}

// --- The loop ---------------------------------------------------------------------------------

function tick() {
  const total = viewer.frame_count();
  if (!state.playing || total === 0 || !ctx) {
    setTimeout(tick, FRAME_MS);
    return;
  }
  const last = state.frame === total - 1;
  viewer.render(ctx, state.frame);
  showTime();
  state.frame = (state.frame + 1) % total;
  setTimeout(tick, last ? DWELL_MS : FRAME_MS);
}

// Full screen on the whole document element, so the toolbar comes along — a radar with no site
// picker and no timestamp is a screensaver.
function toggleFullscreen() {
  el("full").blur(); // otherwise the button keeps focus and eats the next space bar
  if (document.fullscreenElement) document.exitFullscreen();
  else document.documentElement.requestFullscreen().catch((e) => console.warn("fullscreen:", e));
}

// --- Boot -------------------------------------------------------------------------------------

const haversine = (a, b) => {
  const r = (d) => (d * Math.PI) / 180;
  const dLat = r(b.lat - a.lat);
  const dLon = r(b.lon - a.lon);
  const s =
    Math.sin(dLat / 2) ** 2 + Math.cos(r(a.lat)) * Math.cos(r(b.lat)) * Math.sin(dLon / 2) ** 2;
  return 12742 * Math.asin(Math.sqrt(s));
};

async function pickSite(sites) {
  const wanted = (params.get("site") || "").toUpperCase();
  const named = sites.find((s) => s.id === wanted);
  if (named) return named;
  // The edge already knows roughly where the visitor is; /geo.json is [lon, lat] or null.
  const geo = await fetch("/geo.json")
    .then((r) => r.json())
    .catch(() => null);
  if (Array.isArray(geo)) {
    const me = { lat: geo[1], lon: geo[0] };
    return sites.reduce((best, s) => (haversine(me, s) < haversine(me, best) ? s : best));
  }
  return sites.find((s) => s.id === "KTLX") || sites[0];
}

function fillPicker(sites) {
  const sel = el("site");
  const byState = new Map();
  for (const s of sites) {
    if (!byState.has(s.state)) byState.set(s.state, []);
    byState.get(s.state).push(s);
  }
  for (const [st, list] of [...byState].sort((a, b) => a[0].localeCompare(b[0]))) {
    const group = document.createElement("optgroup");
    group.label = st;
    for (const s of list.sort((a, b) => a.city.localeCompare(b.city))) {
      const opt = new Option(`${s.city} (${s.id})`, s.id);
      group.append(opt);
    }
    sel.append(group);
  }
}

async function main() {
  const sites = await fetch("./sites.json").then((r) => r.json());
  allSites = sites;
  fillPicker(sites);

  const glue = await import(globalThis.__liteGlue);
  await glue.default();
  viewer = new glue.Viewer();
  viewer.set_product(state.product);

  if (hero) {
    document.body.classList.add("hero");
    // Reduced motion still gets live radar, just not an animating one: the newest scan holds.
    if (matchMedia("(prefers-reduced-motion: reduce)").matches) state.playing = false;
  }

  state.site = await pickSite(sites);
  el("site").value = state.site.id;
  for (const p of ["ref", "vel"]) el(p).setAttribute("aria-pressed", String(state.product === p));
  applyView();
  permalink();
  tick();
  await Promise.all([loadLoop(), loadWarnings()]);

  if (hero) {
    // No chrome to wire up, and the wheel and keyboard belong to the page around the iframe.
    heroRefreshers();
    return;
  }

  el("site").addEventListener("change", (e) => goToSite(sites.find((s) => s.id === e.target.value)));
  el("in").addEventListener("click", () => setZoom(state.zoom + 1));
  el("out").addEventListener("click", () => setZoom(state.zoom - 1));
  // The wheel zooms, which is what anyone who has used a map expects. Passive: false because the
  // point is to stop the page scrolling underneath it.
  el("map").addEventListener(
    "wheel",
    (e) => {
      e.preventDefault();
      setZoom(state.zoom + (e.deltaY < 0 ? 1 : -1));
    },
    { passive: false },
  );
  addEventListener("keydown", (e) => {
    // Not while the site picker has focus: typing there jumps to a state, and stealing the keys
    // would break it.
    if (/^(INPUT|SELECT|TEXTAREA)$/.test(e.target.tagName)) return;
    if (e.key === "+" || e.key === "=") setZoom(state.zoom + 1);
    if (e.key === "-") setZoom(state.zoom - 1);
    if (e.key === "f") toggleFullscreen();
  });
  el("full").addEventListener("click", toggleFullscreen);
  document.addEventListener("fullscreenchange", () => {
    const on = !!document.fullscreenElement;
    el("full").textContent = on ? "Exit full screen" : "Full screen";
    el("full").setAttribute("aria-pressed", String(on));
    // The element resized, and on some browsers no resize event follows.
    applyView();
    if (ctx) viewer.render(ctx, state.frame);
    drawWarnings();
  });
  for (const p of ["ref", "vel"]) {
    el(p).addEventListener("click", async () => {
      if (state.product === p) return;
      state.product = p;
      for (const q of ["ref", "vel"]) el(q).setAttribute("aria-pressed", String(q === p));
      viewer.set_product(p); // clears the frames: they are one product's data levels
      permalink();
      await loadLoop();
    });
  }
  el("play").addEventListener("click", () => {
    state.playing = !state.playing;
    el("play").textContent = state.playing ? "Pause" : "Play";
    el("play").setAttribute("aria-pressed", String(state.playing));
  });
  addEventListener("resize", () => {
    applyView();
    if (ctx) viewer.render(ctx, state.frame);
    drawWarnings();
  });

  heroRefreshers();
}

// A hidden tab is a tab nobody is watching: stop asking S3 for scans, and catch up on return.
function heroRefreshers() {
  setInterval(() => {
    if (document.visibilityState === "visible") loadLoop();
  }, REFRESH_MS);
  setInterval(() => {
    if (document.visibilityState === "visible") loadWarnings();
  }, WARN_MS);
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState !== "visible") return;
    loadLoop();
    loadWarnings();
  });
}

main().catch((e) => {
  el("time").textContent = "Failed to start: " + e;
  console.error(e);
});
