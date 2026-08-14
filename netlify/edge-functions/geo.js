// Where the visitor is, as the CDN edge already knows it — no third-party lookup, no browser
// permission prompt, no IP ever leaving the edge. The app asks once at boot and opens on the
// nearest radar instead of Oklahoma.
//
// ponytail: city-level accuracy is plenty; picking the nearest NEXRAD only needs ~50 km. Swap in
// the browser Geolocation API if someone wants street-level, and pay the permission prompt.

export default async (request, context) => {
  const { latitude, longitude } = context.geo ?? {};
  if (!latitude || !longitude) return new Response("null", jsonHeaders());
  return new Response(JSON.stringify([longitude, latitude]), jsonHeaders());
};

// Per-visitor answer: it must never be held in a shared cache.
const jsonHeaders = () => ({
  headers: { "content-type": "application/json", "cache-control": "private, no-store" },
});

export const config = { path: "/geo.json" };
