// The CORS proxy the browser build needs, as a Cloudflare Pages Worker.
//
// The wasm build rewrites every feed URL to `{origin}/proxy/{host}/{path}` (crates/wxdata/src/
// net.rs), because NOAA's buckets and the NWS API send no `Access-Control-Allow-Origin`. A static
// host alone therefore serves a demo that draws nothing. This is the same trust boundary the
// native `--serve` proxy enforces (crates/hookecho/src/serve.rs): the host must be in the
// allowlist exactly, only GET is issued upstream, no client header is forwarded, and the response
// is capped and stripped down to a known content type.
//
// The ALLOWED_HOSTS list below is checked against serve.rs by .github/workflows/demo.yml — the two
// must stay identical, and the deploy fails if they drift.

const ALLOWED_HOSTS = [
  // NEXRAD / TDWR archives and the live chunk stream.
  "unidata-nexrad-level2.s3.amazonaws.com",
  "unidata-nexrad-level2-chunks.s3.amazonaws.com",
  "unidata-nexrad-level3.s3.amazonaws.com",
  // Gridded and satellite feeds.
  "noaa-mrms-pds.s3.amazonaws.com",
  "noaa-hrrr-bdp-pds.s3.amazonaws.com",
  "noaa-rap-pds.s3.amazonaws.com",
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
  "services.dat.noaa.gov",
  "apps.dat.noaa.gov",
  "mesonet.agron.iastate.edu",
  "weather.uwyo.edu",
  "mping.ou.edu",
  "www.spotternetwork.org",
  "api.open-meteo.com",
  "gibs.earthdata.nasa.gov",
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

const MAX_BYTES = 64 * 1024 * 1024;

// Live feeds go stale in seconds; everything else can sit in Cloudflare's cache for five minutes,
// which is what keeps a front-page demo off NOAA's rate limits.
// ponytail: two classes, not a per-host map — add one if a specific feed complains.
const LIVE_HOSTS = new Set([
  "unidata-nexrad-level2-chunks.s3.amazonaws.com",
  "api.weather.gov",
]);

function contentType(upstream) {
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

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (!url.pathname.startsWith("/proxy/")) return env.ASSETS.fetch(request);

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
        cf: { cacheTtl: LIVE_HOSTS.has(host) ? 15 : 300, cacheEverything: true },
      });
    } catch (e) {
      return new Response(JSON.stringify({ error: "upstream fetch failed" }), {
        status: 502,
        headers: { "content-type": "application/json" },
      });
    }
    if (!upstream.ok) {
      return new Response(JSON.stringify({ error: "upstream fetch failed" }), {
        status: 502,
        headers: { "content-type": "application/json" },
      });
    }

    const length = Number(upstream.headers.get("content-length") || 0);
    if (length > MAX_BYTES) return refused("response over cap");

    return new Response(upstream.body ? capped(upstream.body) : null, {
      headers: { "content-type": contentType(upstream.headers.get("content-type") || "") },
    });
  },
};
