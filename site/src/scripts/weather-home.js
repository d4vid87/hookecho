import sites from '../data/nexrad-sites.json';
import { milesBetween } from '../data/geo';
import { temperature, percent, weatherSymbol, currentPeriods, validLocation, alertState } from './weather-data.js';

const root = document.querySelector('.weather-home');
if (root) {
  const el = name => root.querySelector(`[data-${name}]`);
  const areas = sites.filter(s => s.network === 'nexrad' && s.country === 'US');
  const defaultSite = areas.find(s => s.id === 'KTLX');
  const fromSite = s => ({ label: `${s.city}, ${s.state}`, lat: s.lat, lon: s.lon, site: s.id, tz: s.tz });
  const read = (key, fallback) => { try { return JSON.parse(localStorage.getItem(key)) ?? fallback; } catch { return fallback; } };
  const write = (key, value) => { try { localStorage.setItem(key, JSON.stringify(value)); } catch { /* The page still works when storage is disabled. */ } };
  let unit = read('hookecho:weather-unit', 'F') === 'C' ? 'C' : 'F';
  const remembered = read('hookecho:weather-place', null);
  let place = validLocation(remembered) && areas.some(s => s.id === remembered.site) ? remembered : fromSite(defaultSite);
  const stored = read('hookecho:weather-saved', []);
  let saved = Array.isArray(stored) ? stored.filter(p => validLocation(p) && areas.some(s => s.id === p.site)).slice(0,8) : [];
  let hourly = [], daily = [], issued = null, controller;
  let timezone = areas.find(s => s.id === place.site)?.tz || 'America/Chicago';
  const text = (name, value) => { el(name).textContent = value; };
  const node = (tag, content, className = '') => { const n = document.createElement(tag); n.textContent = content; n.className = className; return n; };
  const when = (iso, options = {}) => Number.isFinite(Date.parse(iso)) ? new Date(iso).toLocaleString('en-US', { timeZone: timezone, hour: 'numeric', minute: '2-digit', timeZoneName: 'short', ...options }) : 'Time unavailable';
  const same = (a,b) => a.lat === b.lat && a.lon === b.lon;
  const radarUrl = () => `https://app.hookecho.io/#goto=${place.site},${place.lon},${place.lat},8`;
  function renderSaved() {
    el('saved').replaceChildren(...saved.map(p => {
      const b = node('button', p.label); b.type = 'button'; b.setAttribute('aria-pressed', String(same(p,place)));
      b.addEventListener('click', () => load(p)); return b;
    }));
    const selected = saved.some(p => same(p,place));
    text('save', selected ? 'Saved ✓' : 'Save location'); el('save').setAttribute('aria-pressed', String(selected));
  }
  const narrative = p => `${unit === 'C' ? 'NWS outlook (°F): ' : ''}${p.detailedForecast || p.shortForecast}`;
  function render() {
    root.querySelectorAll('[data-unit]').forEach(b => b.setAttribute('aria-pressed', String(b.dataset.unit === unit)));
    const hours = currentPeriods(hourly), days = currentPeriods(daily), current = hours[0] || days[0];
    text('temperature', temperature(current?.temperature, current?.temperatureUnit, unit));
    text('condition', current?.shortForecast || 'Forecast unavailable');
    text('condition-icon', weatherSymbol(current?.shortForecast, current?.isDaytime));
    text('rain', percent(current?.probabilityOfPrecipitation?.value));
    text('humidity', percent(current?.relativeHumidity?.value));
    text('wind', current ? `${current.windDirection || ''} ${current.windSpeed || '—'}`.trim() : '—');
    text('summary', days[0] ? narrative(days[0]) : 'Forecast data could not be loaded. Try Refresh or visit weather.gov.');
    const old = issued && Date.now() - Date.parse(issued) > 6 * 3600000;
    text('updated', issued ? `${old ? 'Older forecast · ' : 'Issued '}${when(issued)}` : 'Forecast data unavailable · NWS');
    text('local-time', when(new Date().toISOString()));
    el('hourly').replaceChildren(...(hours.length ? hours.slice(0,24).map((p,i) => {
      const item = node('div','', 'hour');
      item.append(node('span', i === 0 && Date.parse(p.startTime) <= Date.now() ? 'This hour' : new Date(p.startTime).toLocaleTimeString('en-US',{timeZone:timezone,hour:'numeric'})));
      const symbol = node('span', weatherSymbol(p.shortForecast,p.isDaytime),'symbol'); symbol.title = p.shortForecast; symbol.setAttribute('role','img'); symbol.setAttribute('aria-label',p.shortForecast);
      item.append(symbol,node('strong',temperature(p.temperature,p.temperatureUnit,unit)),node('span',percent(p.probabilityOfPrecipitation?.value),'rain'),node('span',p.windSpeed || '—','wind'));
      return item;
    }) : [node('p','Hourly forecast unavailable. Try Refresh.')]));
    el('daily').replaceChildren(...(days.length ? days.slice(0,14).map(p => {
      const detail = node('details',''), summary = node('summary','');
      summary.append(node('span',p.name),node('span',p.shortForecast,'period-description'),node('strong',temperature(p.temperature,p.temperatureUnit,unit),'period-temp'),node('span',percent(p.probabilityOfPrecipitation?.value),'period-rain'),node('span','+'));
      detail.append(summary,node('p',narrative(p))); return detail;
    }) : [node('p','Daily forecast unavailable. Try Refresh.')]));
  }
  async function json(url, signal) {
    const parsed = new URL(url);
    if (parsed.origin !== 'https://api.weather.gov') throw new Error('Unexpected weather source');
    const res = await fetch(url,{signal,headers:{Accept:'application/geo+json'}});
    if (!res.ok) throw new Error(`Weather service ${res.status}`);
    return res.json();
  }
  function renderAlerts(alerts) {
    el('alert-strip').dataset.severity = alerts.length ? 'warning' : 'none';
    if (!alerts.length) {
      el('alerts').replaceChildren(node('strong','No active NWS alerts'),node('span',`For this location · checked ${when(new Date().toISOString())}`)); return;
    }
    el('alerts').replaceChildren(...alerts.map(p => {
      const detail = node('details',''), summary = node('summary','');
      summary.append(node('strong',p.event),node('span',`${p.severity} · until ${when(p.ends || p.expires)}`));
      detail.append(summary,node('p',p.areaDesc || ''),node('p',p.description || ''),node('p',p.instruction || ''));
      return detail;
    }));
  }
  async function load(next) {
    if (!validLocation(next)) return;
    controller?.abort(); controller = new AbortController(); const {signal} = controller;
    const timeout = setTimeout(() => controller?.signal === signal && controller.abort(),18000);
    place = next; timezone = areas.find(s => s.id === place.site)?.tz || 'America/Chicago';
    write('hookecho:weather-place',place); renderSaved();
    text('place',place.label); text('location-detail',`${place.site} · hourly forecast, not an observation`);
    el('radar-link').href = radarUrl();
    const liveRadar = el('live-radar');
    el('local-radar').href = liveRadar ? radarUrl() : `https://app.hookecho.io/lite/?site=${place.site}`;
    if (liveRadar) {
      const src = radarUrl().replace('/#', '/?embed#');
      if (liveRadar.getAttribute('src') !== src) liveRadar.src = src;
    }
    hourly = []; daily = []; issued = null; render();
    text('condition','Loading forecast…'); text('summary','Connecting to the National Weather Service.'); text('updated','Updating forecast…');
    el('hourly').replaceChildren(node('p','Loading hourly forecast…'));
    el('daily').replaceChildren(node('p','Loading daily forecast…'));
    el('alerts').replaceChildren(node('strong','Official weather alerts'),node('span','Checking the National Weather Service…'));
    delete el('alert-strip').dataset.severity;
    el('refresh').disabled = true;
    const point = `${place.lat.toFixed(4)},${place.lon.toFixed(4)}`;
    const forecastTask = (async () => {
      try {
        const metadata = (await json(`https://api.weather.gov/points/${point}`,signal)).properties;
        if (signal.aborted) return;
        if (typeof metadata?.timeZone === 'string') { try { new Intl.DateTimeFormat('en-US',{timeZone:metadata.timeZone}); timezone = metadata.timeZone; } catch {} }
        const results = await Promise.allSettled([json(metadata.forecastHourly,signal),json(metadata.forecast,signal)]);
        if (signal.aborted) { if (controller?.signal === signal) render(); return; }
        const h = results[0].status === 'fulfilled' ? results[0].value.properties : null;
        const d = results[1].status === 'fulfilled' ? results[1].value.properties : null;
        hourly = currentPeriods(h?.periods); daily = currentPeriods(d?.periods);
        issued = h?.updateTime || d?.updateTime || null; render();
      } catch { if (controller?.signal === signal) render(); }
    })();
    const alertsTask = (async () => {
      try { const data = await json(`https://api.weather.gov/alerts/active?point=${point}`,signal); if (!signal.aborted) renderAlerts(alertState(data.features)); }
      catch { if (controller?.signal === signal) el('alerts').replaceChildren(node('strong','Alerts unavailable'),node('span','Check the official NWS alerts before making weather-sensitive plans.')); }
    })();
    await Promise.allSettled([forecastTask,alertsTask]); clearTimeout(timeout);
    if (controller?.signal === signal) el('refresh').disabled = false;
  }
  el('location-form').addEventListener('submit',event => {
    event.preventDefault(); const value = root.querySelector('#weather-search').value.trim().toLowerCase();
    const exact = areas.find(s => `${s.city}, ${s.state} · ${s.id}`.toLowerCase() === value || s.id.toLowerCase() === value);
    const matches = areas.filter(s => s.city.toLowerCase() === value);
    const match = exact || (matches.length === 1 ? matches[0] : null);
    if (!match) { text('search-status','Choose a radar area from the suggestions, or enter its four-letter station code.'); return; }
    text('search-status',`Forecast for the ${match.city} radar location. Select “Use my location” for your local forecast.`); load(fromSite(match));
  });
  el('locate').addEventListener('click',() => {
    if (!navigator.geolocation) { text('search-status','Location is unavailable. Search for a radar area instead.'); return; }
    text('search-status','Waiting for location permission…');
    navigator.geolocation.getCurrentPosition(({coords}) => {
      const point = {lat:Math.round(coords.latitude*100)/100,lon:Math.round(coords.longitude*100)/100};
      const closest = areas.reduce((best,s) => milesBetween(point,s) < milesBetween(point,best) ? s : best);
      text('search-status','Using your approximate location for the NWS forecast. Radar opens at the nearest station.');
      load({...fromSite(closest),...point,label:`Near ${closest.city}`});
    },() => text('search-status','Location could not be accessed. Search for a radar area instead.'),{timeout:10000,maximumAge:300000});
  });
  el('save').addEventListener('click',() => {
    if (saved.some(p => same(p,place))) saved = saved.filter(p => !same(p,place));
    else if (saved.length < 8) saved.push({...place});
    else { text('search-status','Eight locations saved. Select a saved location and press Saved to remove it first.'); return; }
    write('hookecho:weather-saved',saved); renderSaved();
  });
  root.querySelectorAll('[data-unit]').forEach(b => b.addEventListener('click',() => { unit = b.dataset.unit; write('hookecho:weather-unit',unit); render(); }));
  el('refresh').addEventListener('click',() => load(place));
  el('radar-image')?.addEventListener('error',() => { el('radar-image').hidden = true; el('radar-error').hidden = false; });

  load(place);
  // Recheck visible weather every five minutes; never leave an old all-clear on screen indefinitely.
  setInterval(() => { if (!document.hidden && !el('refresh').disabled) load(place); }, 5 * 60 * 1000);
}
