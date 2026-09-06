import { createHash } from "node:crypto";

export const PROMOTION_START = "2026-09-08";
export const CHANNELS = Object.freeze(["bluesky", "mastodon", "youtube", "facebook", "instagram"]);

export const PRODUCTS = Object.freeze({
  hookecho: {
    name: "HookEcho",
    repo: "d4vid87/hookecho",
    landing: "https://hookecho.io",
    media: "docs/shots/hero.gif",
  },
  weatherdesk: {
    name: "WeatherDesk",
    repo: "d4vid87/weatherdesk",
    landing: "https://hookecho.io/weatherdesk/",
    media: "site/public/shots/weatherdesk-dashboard-1200.webp",
  },
});

const hookecho = [
  ["See the rain move", "HookEcho opens the nearest radar already running—no account, ads, or setup screen.", "docs/shots/hero.gif", "ARCHIVE"],
  ["Warnings belong on the map", "Follow official warnings, lightning, and projected storm tracks without leaving the radar.", "docs/shots/alerts.jpg", "ARCHIVE"],
  ["Velocity shows the wind", "Storm-relative velocity makes inbound and outbound motion visible beside reflectivity.", "docs/shots/velocity.jpg", "ARCHIVE"],
  ["Dual-pol when it matters", "Correlation coefficient, differential reflectivity, and derived hail products are one tap away.", "docs/shots/derived.jpg", "ARCHIVE"],
  ["Every elevation angle", "Compare four radar tilts at once instead of guessing what is happening above the lowest scan.", "docs/shots/alltilts.jpg", "ARCHIVE"],
  ["Radar plus the environment", "Open forecast soundings and severe-weather parameters beside the storm you are studying.", "docs/shots/forecast.jpg", "ARCHIVE"],
  ["Replay storms back to 1991", "Open a historic event at the exact scan and product, then step through the radar volume.", "docs/shots/hero.gif", "ARCHIVE"],
  ["Built for the road", "Offline chase packs keep selected radar and map data available when the connection does not.", "docs/shots/layers.jpg", "ARCHIVE"],
  ["When will the rain arrive?", "HookEcho estimates rain arrival from the motion already visible in the radar scan.", "docs/shots/stormtable.jpg", "ARCHIVE"],
  ["Warnings can speak first", "Urgent warning alerts can interrupt lower-priority audio when your eyes are elsewhere.", "docs/shots/alerts.jpg", "ARCHIVE"],
  ["One national radar view", "MRMS turns the radar network into a seamless national mosaic, still sourced directly from NOAA.", "docs/shots/mrms.jpg", "ARCHIVE"],
  ["Terminal radar counts too", "HookEcho includes TDWR sites around major airports alongside the WSR-88D network.", "docs/shots/products.jpg", "ARCHIVE"],
  ["Weather software without surveillance", "The app talks to public weather feeds from your machine and sends us no telemetry.", "docs/shots/reflectivity.jpg", "ARCHIVE"],
];

const weatherdesk = [
  ["Your station on every screen", "WeatherDesk serves one whole-home dashboard from the desktop app to tablets and other LAN devices."],
  ["A better Tempest dashboard", "Connect a Tempest station and put observations, forecast context, alerts, and radar on one screen."],
  ["Ecowitt, without a cloud dashboard", "Bring Ecowitt observations into a local dashboard built for a wall tablet."],
  ["Ambient Weather, room to breathe", "Turn Ambient Weather station data into a calm whole-home display instead of a phone-sized panel."],
  ["Davis data around the house", "Use a Davis station as the source for a dashboard every device on the LAN can open."],
  ["Older stations still deserve a screen", "AcuRite and La Crosse hardware can feed the same current dashboard as newer stations."],
  ["WeeWX fits right in", "Weather Underground protocol and WeeWX make WeatherDesk useful without replacing a working station stack."],
  ["HookEcho radar is built in", "Open local NEXRAD radar inside the dashboard, then switch to the full Forecast Lab when needed."],
  ["Made for the wall tablet", "The host app serves the dashboard locally, so an old tablet can become the household weather display."],
  ["Past, present, and next", "The Timeline joins recent station changes, official alerts, and the next 48 hours in one view."],
  ["Forecast Lab, not forecast clutter", "Compare radar, models, and severe-weather parameters when a simple daily forecast is not enough."],
  ["Know when the station is stale", "Station health exposes battery, signal, sensor faults, and time since the latest report."],
  ["No station required", "Start with a location forecast today, then connect a supported personal station whenever you are ready."],
].map(([title, body], i) => [
  title,
  body,
  i % 2 ? "site/public/shots/weatherdesk-wide-1600.webp" : "site/public/shots/weatherdesk-dashboard-1200.webp",
  "DEMO DATA",
]);

const proof = {
  hookecho: [
    ["Free means the whole app", "MIT licensed, no paid tier, and the browser build is the same radar engine as desktop and Android."],
    ["The decoder is open too", "HookEcho decodes NEXRAD Level 2 and Level 3 data in the open instead of hiding the hard part behind a service."],
    ["Archive cases you can inspect", "Historic storm pages deep-link to the exact radar, product, tilt, and scan so anyone can verify the example."],
    ["One Rust codebase, four platforms", "Desktop, Android, and WebAssembly builds share the radar engine and public-data model."],
    ["Built where bugs are visible", "The roadmap, issues, source, and release history are public. Useful feedback can become the next fix."],
    ["No account-shaped dependency", "There is no login server to fail, profile to monetize, or subscription to remember."],
    ["Public data should stay useful", "HookEcho makes NOAA radar, forecasts, and warnings easier to explore without putting a tollbooth in front."],
    ["Reproducible radar education", "Archived scans make a tornado signature teachable without waiting for dangerous weather to happen again."],
    ["A real browser demo", "The live demo is the application, not a mockup or a prerecorded tour."],
    ["Privacy is an architecture choice", "Direct public-data requests and local settings mean there is no app telemetry pipeline to promise away."],
    ["Small project, broad coverage", "WSR-88D, TDWR, MRMS, forecast models, warnings, lightning, and archives live in one open project."],
    ["Contributors can run the evidence", "Tests, smoke checks, and reproducible screenshot scenes live beside the code they verify."],
    ["Useful weather tools can be free", "If HookEcho has earned a place in your weather workflow, a GitHub star helps other people find it."],
  ],
  weatherdesk: [
    ["Self-hosted means your house", "The desktop app serves the dashboard across your LAN; your station does not need our server."],
    ["A dashboard without a framework", "WeatherDesk uses native browser modules and a small local host instead of a hosted application stack."],
    ["One screen, many station brands", "Tempest, Ecowitt, Ambient, Davis, AcuRite, La Crosse, Weather Underground, and WeeWX share one UI."],
    ["A Raspberry Pi can host it", "Linux and arm64 packages make a small always-on host a first-class installation."],
    ["Home Assistant can stay home", "WeatherDesk fits alongside an existing local automation setup instead of replacing it."],
    ["Your archive stays local", "Station history and dashboard settings remain on the machines you control."],
    ["Radar is not an afterthought", "HookEcho runs inside WeatherDesk, centered on the station and ready to open into its full analysis view."],
    ["Old tablets get a second job", "Any modern browser on the LAN can become a dedicated weather display."],
    ["Forecasts need context", "WeatherDesk places station observations, official alerts, radar, and multiple models beside each other."],
    ["No station lock-in", "Changing hardware does not require changing the dashboard used around the house."],
    ["Demo data, real interface", "The screenshots and clips use the current v4 dashboard without exposing a private station or token."],
    ["Open source is supportable", "Issues, source, installers, and checksums are visible instead of disappearing behind an app-store listing."],
    ["A useful station should be visible", "If WeatherDesk improved how you use your station, a GitHub star helps another owner discover it."],
  ],
};

function entry(product, pillar, row, week, askForStar = false) {
  const [title, body, media, label] = row;
  return {
    id: `scheduled-${PROMOTION_START}-w${week + 1}-${pillar}-${product}`,
    kind: "scheduled",
    product,
    pillar,
    title,
    body,
    media,
    label,
    askForStar,
  };
}

export function campaignForDate(value, saturdayProduct = "hookecho") {
  const date = new Date(value);
  if (!Number.isFinite(date.valueOf())) throw new Error("invalid campaign date");
  const start = new Date(`${PROMOTION_START}T00:00:00Z`);
  const day = Date.UTC(date.getUTCFullYear(), date.getUTCMonth(), date.getUTCDate());
  const elapsed = Math.floor((day - start.valueOf()) / 86_400_000);
  const week = Math.floor(elapsed / 7);
  if (week < 0 || week >= 13) return null;
  if (date.getUTCDay() === 2) return entry("hookecho", "tip", hookecho[week], week);
  if (date.getUTCDay() === 4) return entry("weatherdesk", "use-case", weatherdesk[week], week);
  if (date.getUTCDay() === 6) {
    const product = saturdayProduct === "weatherdesk" ? "weatherdesk" : "hookecho";
    const row = proof[product][week];
    return entry(product, "open-source", [row[0], row[1], PRODUCTS[product].media, product === "hookecho" ? "ARCHIVE" : "DEMO DATA"], week, true);
  }
  return null;
}

export function releaseCampaign(product, tag, changelog) {
  if (!PRODUCTS[product] || !/^v?[0-9][0-9A-Za-z.-]*$/.test(tag)) throw new Error("invalid release");
  const version = tag.replace(/^v/, "");
  const lines = changelog.split("\n");
  const patterns = product === "weatherdesk"
    ? [new RegExp(`^## \\[${escapeRegExp(version)}\\]`), new RegExp(`^## ${escapeRegExp(version)}(?: |$)`)]
    : [new RegExp(`^## ${escapeRegExp(version)}(?: |$)`), new RegExp(`^## \\[${escapeRegExp(version)}\\]`)];
  const start = lines.findIndex((line) => patterns.some((pattern) => pattern.test(line)));
  if (start < 0) throw new Error(`no changelog section for ${version}`);
  const rest = lines.slice(start + 1);
  const end = rest.findIndex((line) => /^## /.test(line));
  const section = (end < 0 ? rest : rest.slice(0, end)).join("\n");
  const highlights = [...section.matchAll(/^[-*] (.+)$/gm)]
    .slice(0, 3)
    .map((match) => match[1].split(" — ")[0].replace(/\s+/g, " ").trim());
  return {
    id: `release-${product}-${tag}`,
    kind: "release",
    product,
    pillar: "release",
    title: `${PRODUCTS[product].name} ${version} is out`,
    body: highlights.length ? highlights.join(" • ") : "A new stable release is ready.",
    media: PRODUCTS[product].media,
    label: product === "hookecho" ? "ARCHIVE" : "DEMO DATA",
    askForStar: true,
    tag,
  };
}

export function trackedUrl(campaign, channel) {
  if (!CHANNELS.includes(channel) && channel !== "discord") throw new Error("unsupported channel");
  const source = campaign.kind !== "release" && campaign.askForStar && campaign.combinedStars < 200;
  const target = campaign.product === "hookecho"
    ? source ? "hookecho-source" : campaign.kind === "release" ? "download" : "web"
    : source ? "weatherdesk-source" : campaign.kind === "release" ? "weatherdesk-release" : "weatherdesk";
  return `https://hookecho.io/go/${target}/${channel}`;
}

export function composeText(campaign, channel) {
  const product = PRODUCTS[campaign.product];
  if (!product) throw new Error("unknown product");
  const link = campaign.officialUrl || trackedUrl(campaign, channel);
  const source = `https://hookecho.io/go/${campaign.product === "hookecho" ? "hookecho-source" : "weatherdesk-source"}/${channel}`;
  const star = campaign.askForStar && (campaign.kind === "release" || campaign.combinedStars < 200)
    ? `If ${product.name} is useful to you, please give it a star on GitHub.${campaign.kind === "release" ? ` ${source}` : ""}`
    : "";
  const disclaimer = campaign.kind === "weather"
    ? "HookEcho is not an official warning source. Follow local officials."
    : "";
  return fitForChannel(campaign.title, campaign.body, [star, disclaimer, link].filter(Boolean), channel === "bluesky" ? 300 : channel === "mastodon" ? 500 : 1900);
}

function fitForChannel(title, body, tail, limit) {
  const text = [title, body, ...tail].join("\n\n");
  if (text.length <= limit) return text;
  const suffix = `\n\n${tail.join("\n\n")}`;
  const lead = `${title}\n\n${body}`.slice(0, Math.max(0, limit - suffix.length - 1)).trimEnd();
  return `${lead}…${suffix}`;
}

export function eligibleNwsAlert(feature, now = new Date()) {
  const p = feature?.properties;
  if (!p || p.status !== "Actual" || p.messageType === "Cancel" || !p.headline || !p.areaDesc || !feature.id) return null;
  const expires = new Date(p.expires || p.ends || 0);
  const sent = new Date(p.sent || p.effective || 0);
  if (!Number.isFinite(sent.valueOf()) || !Number.isFinite(expires.valueOf()) || expires <= now || sent > now) return null;
  const emergency = /TORNADO EMERGENCY|FLASH FLOOD EMERGENCY/i.test(`${p.headline} ${p.description || ""}`);
  if (p.severity !== "Extreme" && !emergency) return null;
  const body = `${p.areaDesc}. Issued ${formatUtc(sent)}; expires ${formatUtc(expires)}.`;
  const vtec = p.parameters?.VTEC?.[0]?.replace(/^\/O\.[A-Z]{3}\./, "/O.*.");
  return weatherCampaign(vtec || feature.id, p.headline, body, feature.id, p.sent || p.effective, `${p.headline}|${p.severity}`);
}

export function eligibleNhcItems(xml, now = new Date()) {
  const areas = /United States|U\.S\.|Hawaii|Puerto Rico|Virgin Islands|Florida|Georgia|Carolinas?|Virginia|Louisiana|Texas|Mississippi|Alabama|Gulf Coast|East Coast/i;
  return [...String(xml).matchAll(/<item\b[^>]*>([\s\S]*?)<\/item>/gi)].flatMap((match) => {
    const item = match[1];
    const value = (name) => decodeXml(item.match(new RegExp(`<${name}\\b[^>]*>([\\s\\S]*?)<\\/${name}>`, "i"))?.[1] || "");
    const title = value("title").replace(/\s+/g, " ").trim();
    const description = value("description").replace(/<[^>]+>/g, " ").replace(/\s+/g, " ").trim();
    const link = value("link").trim();
    const published = new Date(value("pubDate"));
    const text = `${title} ${description}`;
    if (!title || !/^https:\/\/www\.nhc\.noaa\.gov\//.test(link) || !Number.isFinite(published.valueOf())) return [];
    if (now - published > 12 * 3_600_000 || published > now || !/(?:Hurricane|Tropical Storm)\s+(?!(?:Warning|Watch)\b)[A-Z][A-Za-z-]+/.test(text) || !/(watch|warning)/i.test(text) || !areas.test(text)) return [];
    return [weatherCampaign(link, title, `Issued ${formatUtc(published)} by the National Hurricane Center.`, link, published.toISOString(), title)];
  });
}

function weatherCampaign(sourceId, title, body, officialUrl, sent, revision) {
  const source = createHash("sha256").update(sourceId).digest("hex").slice(0, 10);
  const update = createHash("sha256").update(revision).digest("hex").slice(0, 10);
  const issued = new Date(sent);
  return {
    id: `weather-${source}-${update}`,
    kind: "weather",
    product: "hookecho",
    pillar: "official-weather",
    title,
    body,
    officialUrl,
    media: null,
    label: "LIVE",
    askForStar: false,
    sent: Number.isFinite(issued.valueOf()) ? issued.toISOString() : null,
  };
}

export function pickWeatherCampaign(nwsFeatures, nhcXmlDocuments, publishedIds = new Set(), now = new Date()) {
  const candidates = [
    ...(nwsFeatures || []).map((feature) => eligibleNwsAlert(feature, now)).filter(Boolean),
    ...(nhcXmlDocuments || []).flatMap((xml) => eligibleNhcItems(xml, now)),
  ].filter((campaign) => !publishedIds.has(campaign.id));
  candidates.sort((a, b) => String(b.sent).localeCompare(String(a.sent)) || a.id.localeCompare(b.id));
  return candidates[0] || null;
}

export function weatherCapAllows(published, now = new Date()) {
  const unique = new Map((published || []).map((item) => [item.id, new Date(item.createdAt)]));
  const recent = [...unique.values()].filter((date) => Number.isFinite(date.valueOf()) && date <= now);
  return recent.filter((date) => now - date < 6 * 3_600_000).length === 0
    && recent.filter((date) => now - date < 24 * 3_600_000).length < 2;
}

export function isPaused(channel, value, now = new Date()) {
  const entry = String(value || "").split(",").find((part) => part.startsWith(`${channel}=`));
  if (!entry) return false;
  const until = new Date(entry.slice(entry.indexOf("=") + 1));
  return Number.isFinite(until.valueOf()) && until > now;
}

export function promotionPauses(artifacts, rows, now = new Date()) {
  const until = new Date(now.valueOf() + 14 * 86_400_000).toISOString();
  return CHANNELS.flatMap((channel) => {
    const events = (artifacts || [])
      .filter(({ name }) => name.startsWith("promo-") && (name.endsWith(`-${channel}`) || name.startsWith(`promo-auth-failure-${channel}-`)))
      .sort((a, b) => String(b.created_at).localeCompare(String(a.created_at)));
    const consecutiveAuthFailures = events.findIndex(({ name }) => !name.startsWith("promo-auth-failure-"));
    const authFailures = consecutiveAuthFailures < 0 ? events.length : consecutiveAuthFailures;
    const posts = events.filter(({ name }) => !name.startsWith("promo-auth-failure-") && !name.startsWith("promo-weather-")).length;
    const clicks = (rows || []).filter((row) => row.placement === channel).reduce((sum, row) => sum + Number(row.clicks || 0), 0);
    return authFailures >= 3 || (posts >= 4 && clicks === 0) ? [`${channel}=${until}`] : [];
  }).join(",");
}

function formatUtc(date) {
  return date.toISOString().replace("T", " ").slice(0, 16) + " UTC";
}

function decodeXml(value) {
  return value
    .replace(/^<!\[CDATA\[|\]\]>$/g, "")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&amp;/g, "&")
    .replace(/&quot;/g, '"')
    .replace(/&#39;|&apos;/g, "'");
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
