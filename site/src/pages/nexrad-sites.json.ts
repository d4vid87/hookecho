/*
  The site list the landing page's hero ranking needs, and nothing else.

  It used to be an `import` in `active-site.ts`, which meant the whole 52 KB registry — every
  network, every field — was bundled into the landing page's JavaScript and parsed before the
  page could do anything with it. Prerendered here instead and fetched when the ranking actually
  runs: five fields, US radars only, because that is all the ranking reads.
*/
import sites from "../data/nexrad-sites.json";

export const prerender = true;

export function GET() {
  const slim = sites
    .filter((s) => s.network === "nexrad")
    .map(({ id, city, state, lat, lon }) => ({ id, city, state, lat, lon }));
  return new Response(JSON.stringify(slim), {
    headers: {
      "content-type": "application/json",
      // The registry changes when the app ships a new one, which is a new deploy.
      "cache-control": "public, max-age=3600",
    },
  });
}
