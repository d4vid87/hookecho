// Advanced-mode Pages worker. Two jobs: the www redirect, and a /geo.json that hands the page the
// visitor's rough location so the landing page can say which radar is nearest without asking the
// browser for a permission the visitor has no reason to grant yet.
// ponytail: a _redirects rule cannot do the redirect — Pages matches those on the path only,
// hostnames are a Netlify feature. Everything else falls straight through to the static assets.
export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (url.hostname.startsWith("www.")) {
      url.hostname = url.hostname.slice(4);
      return Response.redirect(url.toString(), 301);
    }
    // Same shape as the app's own /geo.json (web/_worker.js/index.js), plus the city name the
    // chip prints. Free and exact enough: request.cf is the edge's own geo-IP lookup.
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
