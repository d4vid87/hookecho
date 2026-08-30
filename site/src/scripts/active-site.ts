/*
  "Where is the weather right now?" — one national warning query, ranked onto the radar site that
  sees the most of it. Used by the hero backdrop and (later) the storm-of-the-day tile, so the
  promise is memoized at module level: one fetch per page, whoever asks first.

  Everything is client-side and unauthenticated against api.weather.gov, which is CORS-open and
  public. No key, no proxy, no backend.
*/
import { milesBetween } from "../data/geo";

export interface ActiveSite {
  id: string;
  city: string;
  state: string;
  lat: number;
  lon: number;
  /** Warnings counted against this site. Zero when nothing is active and we fell back. */
  warnings: number;
}

// ponytail: the three warning types the app leads with. Widening the query mostly adds marine and
// hydrologic products that make for a duller backdrop.
const EVENTS = ["Tornado Warning", "Severe Thunderstorm Warning", "Flash Flood Warning"];
const WEIGHT: Record<string, number> = {
  "Tornado Warning": 5,
  "Severe Thunderstorm Warning": 2,
  "Flash Flood Warning": 1,
};

interface Site {
  id: string;
  city: string;
  state: string;
  lat: number;
  lon: number;
}

// TDWR sites are terminal radars with a short range and no RIDGE loop; they never lead the page.
// The international rows are out for a different reason: the ranking below is driven entirely by
// api.weather.gov, so pairing a German radar with a US warning count would be a lie. Both filters
// happen where the file is built (`/nexrad-sites.json`), so no page ships the rows it cannot use.
//
// Fetched rather than imported: bundling the registry put 52 KB of JSON — 38 KB of generated
// JavaScript — in front of every landing page, for a list only the hero ranking reads.
let registry: Promise<Site[]> | null = null;
function nexrad(): Promise<Site[]> {
  registry ??= fetch("/nexrad-sites.json")
    .then((r) => (r.ok ? r.json() : []))
    .catch(() => [] as Site[]);
  return registry;
}

const fallback = (list: Site[]) =>
  list.find((s) => s.id === "KTLX") ?? list[0] ?? null;

/** Mean of a polygon's outer ring — close enough to "where the warning is" for ranking. */
function centroid(feature: any): { lat: number; lon: number } | null {
  const ring = feature?.geometry?.coordinates?.[0];
  if (!Array.isArray(ring) || !ring.length) return null; // zone-only alerts carry no geometry
  let lat = 0;
  let lon = 0;
  for (const [x, y] of ring) {
    lon += x;
    lat += y;
  }
  return { lat: lat / ring.length, lon: lon / ring.length };
}

function nearest(list: Site[], point: { lat: number; lon: number }) {
  let best: Site | null = list[0] ?? null;
  let bestMiles = Infinity;
  for (const site of list) {
    const miles = milesBetween(point, site);
    if (miles < bestMiles) {
      bestMiles = miles;
      best = site;
    }
  }
  return best;
}

async function visitorSite(list: Site[]): Promise<ActiveSite | null> {
  // The site's own edge worker (site/public/_worker.js) hands back the visitor's rough location.
  try {
    const geo = await fetch("/geo.json").then((r) => (r.ok ? r.json() : null));
    if (typeof geo?.lat === "number" && typeof geo?.lon === "number") {
      const near = nearest(list, { lat: geo.lat, lon: geo.lon });
      if (near) return { ...near, warnings: 0 };
    }
  } catch {
    // fall through
  }
  const back = fallback(list);
  return back ? { ...back, warnings: 0 } : null;
}

async function pick(): Promise<ActiveSite | null> {
  const list = await nexrad();
  try {
    const url =
      "https://api.weather.gov/alerts/active?status=actual&event=" +
      // No Accept header: the API's preflight only allows API-Key and User-Agent, so asking for
      // the lighter ld+json would fail CORS.
      encodeURIComponent(EVENTS.join(","));
    const data = await fetch(url).then((r) => (r.ok ? r.json() : Promise.reject(r.status)));
    const scores = new Map<string, { score: number; count: number }>();
    for (const feature of data?.features ?? []) {
      const at = centroid(feature);
      if (!at) continue;
      const site = nearest(list, at);
      if (!site) continue;
      const weight = WEIGHT[feature?.properties?.event] ?? 1;
      const row = scores.get(site.id) ?? { score: 0, count: 0 };
      row.score += weight;
      row.count += 1;
      scores.set(site.id, row);
    }
    let bestId: string | null = null;
    let bestScore = 0;
    for (const [id, row] of scores) {
      if (row.score > bestScore) {
        bestScore = row.score;
        bestId = id;
      }
    }
    if (bestId) {
      const site = list.find((s) => s.id === bestId)!;
      return { ...site, warnings: scores.get(bestId)!.count };
    }
  } catch {
    // A quiet map or a blocked fetch both land on the visitor's own radar, which is never wrong.
  }
  return visitorSite(list);
}

let cached: Promise<ActiveSite | null> | null = null;

/**
 * The most-active radar site right now, or the visitor's nearest when the country is quiet.
 *
 * `null` only when the registry itself could not be fetched — callers already had to handle a
 * failed ranking, and a hero with no backdrop is better than one built from a site that is not
 * in the list.
 */
export function activeSite(): Promise<ActiveSite | null> {
  cached ??= pick();
  return cached;
}
