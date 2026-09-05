import { milesBetween } from "../data/geo";

interface Site {
  id: string;
  city: string;
  state: string;
  lat: number;
  lon: number;
}

export interface NearbySite extends Site {
  visitorCity: string | null;
}

let registry: Promise<Site[]> | null = null;

function sites(): Promise<Site[]> {
  registry ??= fetch("/nexrad-sites.json")
    .then((response) => (response.ok ? response.json() : []))
    .catch(() => [] as Site[]);
  return registry;
}

function nearest(list: Site[], point: { lat: number; lon: number }) {
  let best: Site | null = list[0] ?? null;
  let bestMiles = Infinity;
  for (const site of list) {
    const miles = milesBetween(point, site);
    if (miles < bestMiles) {
      best = site;
      bestMiles = miles;
    }
  }
  return best;
}

/** The visitor's nearest radar, using only the rough location already known by the edge. */
export async function nearbySite(): Promise<NearbySite | null> {
  const list = await sites();
  try {
    const response = await fetch("/geo.json");
    const geo = response.ok ? await response.json() : null;
    if (typeof geo?.lat === "number" && typeof geo?.lon === "number") {
      const site = nearest(list, geo);
      if (site) return { ...site, visitorCity: typeof geo.city === "string" ? geo.city : null };
    }
  } catch {
    // The fallback below keeps the hero useful when edge location is unavailable.
  }

  const fallback = list.find((site) => site.id === "KTLX") ?? list[0] ?? null;
  return fallback ? { ...fallback, visitorCity: null } : null;
}
