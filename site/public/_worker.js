const DOWNLOAD_FILES = [
  "HookEcho-x86_64.AppImage", "HookEcho-amd64.deb", "hookecho-windows-x86_64.zip",
  "HookEcho-x86_64.msi", "HookEcho-setup-x86_64.exe", "HookEcho-macos.zip", "HookEcho-arm64-v8a.apk",
];

export function selectDownload(releases, name) {
  if (!DOWNLOAD_FILES.includes(name)) return null;
  const complete = releases.filter(release => !release.draft &&
    DOWNLOAD_FILES.every(file => release.assets?.some(asset => asset.name === file)));
  const stable = complete.filter(release => /^v\d+\.\d+\.\d+$/.test(release.tag_name) && !release.prerelease);
  const current = release => release.assets.some(asset => asset.name === "SHA256SUMS");
  const selected = stable.find(current) ?? complete.find(current) ?? complete.find(release => release.tag_name === "latest");
  const asset = selected?.assets.find(asset => asset.name === name);
  return asset?.browser_download_url?.startsWith("https://github.com/d4vid87/hookecho/releases/download/") ? asset.browser_download_url : null;
}

export async function downloadRedirect(request) {
  const name = new URL(request.url).pathname.slice("/download-file/".length);
  if (!DOWNLOAD_FILES.includes(name)) return new Response("Not found", {status: 404});
  if (!["GET", "HEAD"].includes(request.method)) return new Response("Method not allowed", {status: 405});
  let destination = "https://github.com/d4vid87/hookecho/releases";
  try {
    const init = {
      headers: {"User-Agent": "HookEcho downloads", "Accept": "application/vnd.github+json"},
      cf: {cacheTtl: 300, cacheEverything: true},
    };
    const [latest, list] = await Promise.all([
      fetch("https://api.github.com/repos/d4vid87/hookecho/releases/latest", init),
      fetch("https://api.github.com/repos/d4vid87/hookecho/releases?per_page=30", init),
    ]);
    const releases = list.ok ? await list.json() : [];
    if (latest.ok) releases.push(await latest.json());
    destination = selectDownload(releases, name) ?? destination;
  } catch { /* The releases page remains usable when the API is unavailable. */ }
  return new Response(null, {status: 302, headers: {location: destination, "cache-control": "no-store"}});
}

export const GO_TARGETS = Object.freeze({
  web: "https://app.hookecho.io/",
  download: "/download/",
  android: "/download/#p-android",
  "hookecho-source": "https://github.com/d4vid87/hookecho",
  "hookecho-youtube": "https://www.youtube.com/channel/UCZRECVhcCwdr0Fm5Z0NUo7g",
  stormdesk: "/stormdesk/",
  "stormdesk-release": "https://github.com/d4vid87/stormdesk/releases/latest",
  "stormdesk-source": "https://github.com/d4vid87/stormdesk",
  "stormdesk-youtube": "https://www.youtube.com/channel/UCDgrO5Mn_mshCQukQm428lA",
});

export const GO_PLACEMENTS = new Set([
  "nav",
  "hero",
  "homepage",
  "stormdesk-hero",
  "stormdesk-final",
  "final",
  "footer",
  "bluesky",
  "mastodon",
  "discord",
  "facebook",
  "instagram",
  "youtube",
]);

export function trackedRedirect(request, env) {
  const url = new URL(request.url);
  const [, route, target, placement, extra] = url.pathname.split("/");
  if (route !== "go" || extra || !GO_TARGETS[target] || !GO_PLACEMENTS.has(placement)) {
    return new Response("Not found", { status: 404 });
  }
  if (request.method !== "GET" && request.method !== "HEAD") {
    return new Response("Method not allowed", { status: 405, headers: { allow: "GET, HEAD" } });
  }

  if (request.method === "GET") {
    try {
      env.CTA_ANALYTICS?.writeDataPoint({ blobs: [target, placement], doubles: [1] });
    } catch {
      // Measurement must never stand between someone and the product.
    }
  }

  return new Response(null, {
    status: 302,
    headers: {
      location: new URL(GO_TARGETS[target], url).toString(),
      "cache-control": "private, no-store",
      "referrer-policy": "no-referrer",
    },
  });
}

// Advanced-mode Pages worker. Host redirects, aggregate CTA counts and rough location for the
// nearest-radar label live here; every other request falls through to the static site.
export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (url.hostname.startsWith("www.")) {
      url.hostname = url.hostname.slice(4);
      return Response.redirect(url.toString(), 301);
    }
    if (url.pathname === `/${"weather" + "desk"}/`) {
      url.pathname = "/stormdesk/";
      return Response.redirect(url.toString(), 301);
    }
    if (url.pathname.startsWith("/go/")) return trackedRedirect(request, env);
    if (url.pathname.startsWith("/download-file/")) return downloadRedirect(request);
    if (url.pathname === "/api/place") {
      const q = url.searchParams.get("q")?.trim() ?? "";
      if (request.method !== "GET" || !q || q.length > 120) return new Response("Invalid place search", { status: 400 });
      const upstream = new URL("https://nominatim.openstreetmap.org/search");
      upstream.search = new URLSearchParams({ q, format: "json", limit: "5", countrycodes: "us", addressdetails: "1" }).toString();
      try {
        const result = await fetch(upstream, {
          headers: { "User-Agent": "hookecho (github.com/d4vid87/hookecho, davidmay87@gmail.com)" },
          cf: { cacheTtl: 3600, cacheEverything: true },
        });
        if (!result.ok) throw new Error(`Geocoder ${result.status}`);
        const matches = await result.json();
        if (!matches.length) return new Response("Place not found", { status: 404 });
        return Response.json(matches.map(match => {
          const a = match.address ?? {};
          const city = a.city ?? a.town ?? a.village ?? a.hamlet ?? a.municipality ?? a.county;
          const street = [a.house_number, a.road].filter(Boolean).join(" ");
          const label = [street, city, a.state, a.postcode].filter(Boolean).join(", ") || match.display_name;
          return { label, address: match.display_name || label, lat: Number(match.lat), lon: Number(match.lon) };
        }).filter(match => Number.isFinite(match.lat) && Number.isFinite(match.lon)), { headers: { "cache-control": "public, max-age=3600" } });
      } catch {
        return new Response("Place search unavailable", { status: 502 });
      }
    }
    if (url.pathname === "/geo.json") {
      const { latitude, longitude, city } = request.cf ?? {};
      const body =
        latitude && longitude
          ? JSON.stringify({ lon: +longitude, lat: +latitude, city: city ?? null })
          : "null";
      return new Response(body, {
        headers: { "content-type": "application/json", "cache-control": "private, no-store" },
      });
    }
    return env.ASSETS.fetch(request);
  },
};
