// Miles between two lat/lon pairs. ponytail: great-circle on a sphere — the error against WGS84 is
// under half a percent, and this number is only ever read as "which radars are near me".
// Lives in its own module because Astro hoists getStaticPaths away from the rest of the
// frontmatter: only imported bindings are in scope there.
export function milesBetween(a: { lat: number; lon: number }, b: { lat: number; lon: number }) {
  const rad = Math.PI / 180;
  const dLat = (b.lat - a.lat) * rad;
  const dLon = (b.lon - a.lon) * rad;
  const h =
    Math.sin(dLat / 2) ** 2 +
    Math.cos(a.lat * rad) * Math.cos(b.lat * rad) * Math.sin(dLon / 2) ** 2;
  return 3958.8 * 2 * Math.asin(Math.sqrt(h));
}
