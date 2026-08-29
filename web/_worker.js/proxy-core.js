// The CORS proxy the browser build needs, host-agnostic.
//
// The wasm build rewrites every feed URL to `{origin}/proxy/{host}/{path}` (crates/wxdata/src/
// net.rs), because NOAA's buckets and the NWS API send no `Access-Control-Allow-Origin`. A static
// host alone therefore serves a demo that draws nothing. This is the same trust boundary the
// native `--serve` proxy enforces (crates/hookecho/src/serve.rs): the host must be in the
// allowlist exactly, only GET is issued upstream, no client header is forwarded, and the response
// is capped and stripped down to a known content type.
//
// web/_worker.js/index.js (Cloudflare Pages) imports this. It is kept separate from the wiring so
// another host's function can reuse the same allowlist and checks rather than restating them.
//
// The ALLOWED_HOSTS list below is checked against serve.rs by .github/workflows/demo.yml — the two
// must stay identical, and the deploy fails if they drift.

export const ALLOWED_HOSTS = [
  // NEXRAD / TDWR archives and the live chunk stream.
  "unidata-nexrad-level2.s3.amazonaws.com",
  "unidata-nexrad-level2-chunks.s3.amazonaws.com",
  "unidata-nexrad-level3.s3.amazonaws.com",
  // Gridded and satellite feeds.
  "noaa-mrms-pds.s3.amazonaws.com",
  "noaa-hrrr-bdp-pds.s3.amazonaws.com",
  "noaa-rap-pds.s3.amazonaws.com",
  "noaa-nam-pds.s3.amazonaws.com",
  "noaa-nbm-grib2-pds.s3.amazonaws.com",
  "noaa-goes19.s3.amazonaws.com",
  "noaa-goes18.s3.amazonaws.com",
  "noaa-gfs-bdp-pds.s3.amazonaws.com",
  "data.ecmwf.int",
  "mrms.ncep.noaa.gov",
  "www.nohrsc.noaa.gov",
  // NWS and friends.
  "api.weather.gov",
  "tgftp.nws.noaa.gov",
  "mapservices.weather.noaa.gov",
  "www.spc.noaa.gov",
  "www.nhc.noaa.gov",
  "www.ndbc.noaa.gov",
  "api.water.noaa.gov",
  "aviationweather.gov",
  "tfr.faa.gov",
  "services.dat.noaa.gov",
  "apps.dat.noaa.gov",
  "mesonet.agron.iastate.edu",
  "weather.uwyo.edu",
  "mping.ou.edu",
  "www.spotternetwork.org",
  "api.open-meteo.com",
  "gibs.earthdata.nasa.gov",
  // MeteoAlarm: the CAP warnings every European met service publishes in common.
  "feeds.meteoalarm.org",
  // European radar.
  "opendata.dwd.de",
  // DWD's WMS: the German radar composites (RV nowcast, WN analysis) as rendered tiles.
  "maps.dwd.de",
  // ECCC's GeoMet WMS: the Canadian 1-km radar composites (rain, snow).
  "geo.weather.gc.ca",
  // EUMETNET OpenRadarData: the ODIM volumes the OPERA network publishes, plus the
  // bucket listing that names the newest one.
  "s3.waw3-1.cloudferro.com",
  // Basemap and imagery tiles.
  "api.mapbox.com",
  "api.maptiler.com",
  "basemaps.cartocdn.com",
  "basemap.nationalmap.gov",
  "server.arcgisonline.com",
  "services3.arcgis.com",
  "tiles.openfreemap.org",
  "tile.openstreetmap.org",
  "a.tile.openstreetmap.fr",
  "a.tile-cyclosm.openstreetmap.fr",
  "a.tile.opentopomap.org",
  // Cameras.
  "weathercams.faa.gov",
  "images.wcams-static.faa.gov",
  "cwwp2.dot.ca.gov",
];

export const MAX_BYTES = 64 * 1024 * 1024;

// Live feeds go stale in seconds; everything else can sit in the CDN cache for five minutes,
// which is what keeps a front-page demo off NOAA's rate limits.
// ponytail: two classes, not a per-host map — add one if a specific feed complains.
export const LIVE_HOSTS = new Set([
  "unidata-nexrad-level2-chunks.s3.amazonaws.com",
  "api.weather.gov",
  // DWD republishes every `-LATEST-` sweep on a five-minute cycle at a URL that never changes,
  // so the default five-minute TTL can hand out the previous volume for the whole of the next one.
  "opendata.dwd.de",
]);

// Archived Level 2 volumes: the same four volumes are what every visitor on a given radar loads,
// and each is tens of MB, so caching them at the edge is the difference between one S3 fetch per
// hour and one per visitor.
const ARCHIVE_BUCKET = "unidata-nexrad-level2.s3.amazonaws.com";

export const cacheSeconds = (host, search = "") => {
  // A bucket listing is how the app finds the newest volume — cache that like a live feed or the
  // loop stops advancing.
  if (search.includes("list-type=")) return 15;
  // A WMS GetMap with no TIME is whatever the layer's default frame happens to be, which turns
  // over every five minutes; one naming a frame is that frame forever. Not in LIVE_HOSTS, because
  // that would put the whole loop — every tile of which names its frame — on a 15-second TTL.
  // The Canadian alerts ride the same rule: a WFS GetFeature carries no TIME and wants the short
  // TTL, since what it returns is the set of alerts in force this minute.
  if (host === "maps.dwd.de" || host === "geo.weather.gc.ca") {
    return search.includes("TIME=") ? 300 : 15;
  }
  // ponytail: an hour, not forever. The newest archive object is re-uploaded while the radar is
  // still writing it, so a long TTL can pin a truncated volume at the edge; an hour bounds how
  // long that can last. Drop it to 600 if it is ever seen to bite.
  if (host === ARCHIVE_BUCKET) return 3600;
  return LIVE_HOSTS.has(host) ? 15 : 300;
};

// The User-Agent the app identifies itself with everywhere else (wxdata::alerts::USER_AGENT);
// api.weather.gov refuses a request without one.
export const USER_AGENT = "hookecho (github.com/d4vid87/hookecho, davidmay87@gmail.com)";

export function contentType(upstream) {
  switch ((upstream.split(";")[0] || "").trim()) {
    case "application/json":
    case "application/geo+json":
      return "application/json";
    case "application/xml":
    case "text/xml":
      return "application/xml";
    case "text/plain":
      return "text/plain; charset=utf-8";
    // HTML is deliberately downgraded: proxied bytes are served from our own origin.
    case "text/html":
      return "text/plain; charset=utf-8";
    case "image/png":
      return "image/png";
    case "image/jpeg":
      return "image/jpeg";
    case "image/webp":
      return "image/webp";
    case "application/x-protobuf":
    case "application/vnd.mapbox-vector-tile":
      return "application/x-protobuf";
    default:
      return "application/octet-stream";
  }
}

const refused = (why) =>
  new Response(JSON.stringify({ error: "host not proxyable", why }), {
    status: 403,
    headers: { "content-type": "application/json" },
  });

const badGateway = () =>
  new Response(JSON.stringify({ error: "upstream fetch failed" }), {
    status: 502,
    headers: { "content-type": "application/json" },
  });

// Stop a hostile or broken upstream mid-stream rather than after buffering it.
function capped(body) {
  let seen = 0;
  return body.pipeThrough(
    new TransformStream({
      transform(chunk, controller) {
        seen += chunk.byteLength;
        if (seen > MAX_BYTES) throw new Error("response over cap");
        controller.enqueue(chunk);
      },
    }),
  );
}

/// Handle one `/proxy/{host}/{path}` request. `fetchInit(host)` lets a platform add its own cache
/// hints to the upstream call, `extraHeaders(host, search)` the same for the response we send back.
export async function handleProxy(request, { fetchInit = () => ({}), extraHeaders = () => ({}) } = {}) {
  const url = new URL(request.url);
  const rest = url.pathname.slice("/proxy/".length);
  const slash = rest.indexOf("/");
  if (slash < 0) return refused("no path after host");
  const host = rest.slice(0, slash);
  if (!ALLOWED_HOSTS.includes(host)) return refused("host not in allowlist");
  if (request.method !== "GET") return refused("GET only");

  const target = `https://${host}/${rest.slice(slash + 1)}${url.search}`;
  let upstream;
  try {
    // No client header is forwarded — this is a fresh request, not a rewrite of theirs.
    upstream = await fetch(target, {
      headers: { "user-agent": USER_AGENT },
      ...fetchInit(host, url.search),
    });
  } catch {
    return badGateway();
  }
  if (!upstream.ok) return badGateway();

  const length = Number(upstream.headers.get("content-length") || 0);
  if (length > MAX_BYTES) return refused("response over cap");

  return new Response(upstream.body ? capped(upstream.body) : null, {
    headers: {
      "content-type": contentType(upstream.headers.get("content-type") || ""),
      ...extraHeaders(host, url.search),
    },
  });
}
