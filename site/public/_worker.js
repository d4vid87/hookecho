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
