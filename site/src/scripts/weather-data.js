export function temperature(value, source = 'F', unit = 'F') {
  if (typeof value !== 'number' || !Number.isFinite(value)) return '—';
  const converted = source === unit ? value : unit === 'C' ? (value - 32) * 5 / 9 : value * 9 / 5 + 32;
  return `${Math.round(converted)}°`;
}

export function percent(value) {
  return typeof value === 'number' && value >= 0 && value <= 100 ? `${Math.round(value)}%` : '—';
}

export function weatherSymbol(description = '', daytime = true) {
  if (/thunder/i.test(description)) return 'ϟ';
  if (/snow|sleet|ice/i.test(description)) return '❄';
  if (/rain|shower|drizzle/i.test(description)) return '☂';
  if (/cloud|fog|haze/i.test(description)) return '☁';
  if (/sun|clear|fair/i.test(description)) return daytime ? '☀' : '☾';
  return '◌';
}

export function currentPeriods(periods, now = Date.now()) {
  return Array.isArray(periods) ? periods.filter(p => p && typeof p.name === 'string' && typeof p.shortForecast === 'string' && Number.isFinite(Date.parse(p.startTime)) && Date.parse(p.endTime) > now) : [];
}

export function validLocation(place) {
  return !!place && typeof place.label === 'string' && place.label.length < 160 && Number.isFinite(place.lat) && Math.abs(place.lat) <= 90 && Number.isFinite(place.lon) && Math.abs(place.lon) <= 180 && typeof place.site === 'string' && /^K[A-Z0-9]{3}$|^P[A-Z0-9]{3}$|^T[A-Z0-9]{3}$/.test(place.site);
}

export function alertState(features, now = Date.now()) {
  if (!Array.isArray(features) || features.some(f => !f?.properties || typeof f.properties.event !== 'string' || !Number.isFinite(Date.parse(f.properties.expires)))) throw new Error('Invalid alert response');
  const active = features.map(f => f.properties).filter(p => p.status === 'Actual' && p.messageType !== 'Cancel' && Date.parse(p.ends || p.expires) > now);
  const rank = { Extreme: 0, Severe: 1, Moderate: 2, Minor: 3, Unknown: 4 };
  return active.sort((a,b) => (rank[a.severity] ?? 4) - (rank[b.severity] ?? 4));
}
