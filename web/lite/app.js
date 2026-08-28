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
const FRAMES = 6; // scans in the loop, ~15 minutes of weather
const FRAME_MS = 400;
const DWELL_MS = 1200; // extra time on the newest frame, so the loop reads as "now"
const REFRESH_MS = 120000;
const MAX_CANVAS = 1024;

const el = (id) => document.getElementById(id);
const params = new URLSearchParams(location.search);

const state = {
  site: null, // { id, city, state, lat, lon }
  product: params.get("prod") === "vel" ? "vel" : "ref",
  zoom: Number(params.get("zoom")) || 9,
  frame: 0,
  times: [],
  playing: true,
  loading: false,
};

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

function applyView() {
  const [w, h] = canvasSize();
  for (const id of ["radar", "warn"]) {
    const c = el(id);
    c.width = w;
    c.height = h;
  }
  ctx = el("radar").getContext("2d");
  viewer.set_view(state.site.lat, state.site.lon, w, h, state.zoom);
  drawTiles(w, h);
}

function permalink() {
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
    viewer.clear_frames();
    const times = [];
    for (const key of keys) {
      const res = await fetch(`${S3}/${key}`).catch(() => null);
      if (!res || !res.ok) continue;
      const bytes = new Uint8Array(await res.arrayBuffer());
      if (viewer.add_frame(bytes)) times.push(keyTime(key));
    }
    state.times = times;
    state.frame = Math.max(0, times.length - 1);
    if (ctx) viewer.render(ctx, state.frame);
  } finally {
    state.loading = false;
    showTime();
  }
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
  fillPicker(sites);

  const glue = await import(globalThis.__liteGlue);
  await glue.default();
  viewer = new glue.Viewer();
  viewer.set_product(state.product);

  state.site = await pickSite(sites);
  el("site").value = state.site.id;
  el("zoom").value = String(state.zoom);
  for (const p of ["ref", "vel"]) el(p).setAttribute("aria-pressed", String(state.product === p));
  applyView();
  permalink();
  tick();
  await loadLoop();

  el("site").addEventListener("change", async (e) => {
    state.site = sites.find((s) => s.id === e.target.value);
    applyView();
    permalink();
    await loadLoop();
  });
  el("zoom").addEventListener("change", (e) => {
    state.zoom = Number(e.target.value);
    applyView();
    permalink();
    if (ctx) viewer.render(ctx, state.frame);
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
  });

  // A hidden tab is a tab nobody is watching: stop asking S3 for scans, and catch up on return.
  setInterval(() => {
    if (document.visibilityState === "visible") loadLoop();
  }, REFRESH_MS);
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "visible") loadLoop();
  });
}

main().catch((e) => {
  el("time").textContent = "Failed to start: " + e;
  console.error(e);
});
