// The comparison matrix, shared by /compare/ and the five per-competitor pages so a corrected row
// is corrected everywhere at once.
//
// An honest matrix or none at all: every row a competitor wins is marked as won. Prices are tiers,
// not dollar amounts — those change without telling us, and a stale number here would be the one
// thing readers were right to disbelieve.
//
// Sources, checked 2026-08: each product's own pricing/feature pages and store listings.
// RadarScope: base.radarscope.app tier list (Pro Tier One / Tier Two).
// MyRadar: myradar.com premium features; free tier carries ads.
// Windy: windy.com Premium page; radar composite is not Level 2.
// RadarOmega: radaromega.com feature and subscription pages, including the paid add-on packs.
// Clime and RainViewer are grouped: both are mosaic-imagery apps on a subscription.
// HookEcho's own column comes from README.md and docs/GUIDE.md.
export const COLS = [
  "HookEcho",
  "RadarScope",
  "MyRadar",
  "Windy",
  "RadarOmega",
  "Clime / RainViewer",
];

/** v: "yes" | "no" | "part" — decides the mark; t is the qualifier that makes the mark honest. */
export const ROWS = [
  {
    label: "Price",
    cells: [
      { v: "yes", t: "Free, MIT licensed" },
      { v: "part", t: "Paid app, subscription tiers" },
      { v: "part", t: "Free with ads, subscription" },
      { v: "part", t: "Free, optional Premium" },
      { v: "part", t: "Paid app, subscription add-ons" },
      { v: "part", t: "Free trial, subscription" },
    ],
  },
  {
    label: "Ads",
    cells: [
      { v: "yes", t: "None" },
      { v: "yes", t: "None" },
      { v: "no", t: "Unless you subscribe" },
      { v: "yes", t: "None" },
      { v: "yes", t: "None" },
      { v: "no", t: "Yes" },
    ],
  },
  {
    label: "Account required",
    cells: [
      { v: "yes", t: "Never" },
      { v: "part", t: "For the paid tiers" },
      { v: "part", t: "For the paid tier" },
      { v: "yes", t: "Optional" },
      { v: "part", t: "For the subscription" },
      { v: "part", t: "For the paid tier" },
    ],
  },
  {
    label: "Open source",
    cells: [
      { v: "yes", t: "Every line" },
      { v: "no", t: "" },
      { v: "no", t: "" },
      { v: "no", t: "" },
      { v: "no", t: "" },
      { v: "no", t: "" },
    ],
  },
  {
    label: "Level 2, every tilt",
    cells: [
      { v: "yes", t: "All elevations, full resolution" },
      { v: "yes", t: "All elevations" },
      { v: "no", t: "Mosaic imagery" },
      { v: "no", t: "Composite only" },
      { v: "yes", t: "All elevations" },
      { v: "no", t: "Mosaic imagery" },
    ],
  },
  {
    label: "Dual-pol products",
    cells: [
      { v: "yes", t: "ZDR, CC, KDP, ΦDP" },
      { v: "yes", t: "Full set" },
      { v: "no", t: "" },
      { v: "no", t: "" },
      { v: "yes", t: "Full set" },
      { v: "no", t: "" },
    ],
  },
  {
    label: "Velocity and dealiasing",
    cells: [
      { v: "yes", t: "With storm-relative" },
      { v: "yes", t: "With storm-relative" },
      { v: "no", t: "" },
      { v: "no", t: "" },
      { v: "yes", t: "With storm-relative" },
      { v: "no", t: "" },
    ],
  },
  {
    label: "National MRMS mosaic",
    cells: [
      { v: "yes", t: "Full resolution" },
      { v: "part", t: "Paid tier" },
      { v: "yes", t: "" },
      { v: "yes", t: "Composite" },
      { v: "yes", t: "" },
      { v: "yes", t: "" },
    ],
  },
  {
    label: "Lightning",
    cells: [
      { v: "yes", t: "GOES GLM" },
      { v: "part", t: "Paid tier" },
      { v: "part", t: "Paid tier" },
      { v: "yes", t: "" },
      { v: "part", t: "Paid add-on" },
      { v: "part", t: "Paid tier" },
    ],
  },
  {
    label: "Forecast models",
    cells: [
      { v: "part", t: "HRRR, NAM, soundings" },
      { v: "no", t: "" },
      { v: "part", t: "Limited" },
      { v: "yes", t: "The best in the field" },
      { v: "part", t: "Paid add-on" },
      { v: "no", t: "" },
    ],
  },
  {
    label: "Tropical tracks",
    cells: [
      { v: "yes", t: "NHC cones and advisories" },
      { v: "no", t: "" },
      { v: "yes", t: "" },
      { v: "yes", t: "" },
      { v: "yes", t: "" },
      { v: "yes", t: "" },
    ],
  },
  {
    label: "Archive playback",
    cells: [
      { v: "yes", t: "Back to 1991" },
      { v: "part", t: "Paid tier, recent only" },
      { v: "no", t: "" },
      { v: "no", t: "" },
      { v: "part", t: "Paid add-on, recent only" },
      { v: "no", t: "" },
    ],
  },
  {
    label: "Works offline",
    cells: [
      { v: "yes", t: "Chase packs, prefetched" },
      { v: "no", t: "" },
      { v: "no", t: "" },
      { v: "no", t: "" },
      { v: "no", t: "" },
      { v: "no", t: "" },
    ],
  },
  {
    label: "Coverage outside the US",
    cells: [
      { v: "part", t: "US plus German DWD radar" },
      { v: "part", t: "Several countries" },
      { v: "part", t: "Mostly US" },
      { v: "yes", t: "Global" },
      { v: "part", t: "Several countries" },
      { v: "yes", t: "Global" },
    ],
  },
  {
    label: "Runs in a browser",
    cells: [
      { v: "yes", t: "The whole app, no install" },
      { v: "no", t: "" },
      { v: "part", t: "Cut-down web view" },
      { v: "yes", t: "Web first" },
      { v: "no", t: "" },
      { v: "no", t: "" },
    ],
  },
  {
    label: "iPhone and iPad",
    cells: [
      { v: "no", t: "Not yet" },
      { v: "yes", t: "" },
      { v: "yes", t: "" },
      { v: "yes", t: "" },
      { v: "yes", t: "" },
      { v: "yes", t: "" },
    ],
  },
  {
    label: "Desktop app",
    cells: [
      { v: "yes", t: "Windows, macOS, Linux" },
      { v: "part", t: "Windows, macOS" },
      { v: "part", t: "Windows" },
      { v: "no", t: "" },
      { v: "part", t: "Windows" },
      { v: "no", t: "" },
    ],
  },
  {
    label: "Mobile polish",
    cells: [
      { v: "part", t: "Android only, and younger" },
      { v: "yes", t: "The bar everyone measures against" },
      { v: "yes", t: "" },
      { v: "yes", t: "" },
      { v: "yes", t: "Deep, and heavily configurable" },
      { v: "yes", t: "" },
    ],
  },
  {
    label: "Telemetry",
    cells: [
      { v: "yes", t: "None, at all" },
      { v: "no", t: "Analytics" },
      { v: "no", t: "Ads and analytics" },
      { v: "no", t: "Analytics" },
      { v: "no", t: "Analytics" },
      { v: "no", t: "Ads and analytics" },
    ],
  },
];

export interface Competitor {
  slug: string;
  /** Column index in COLS. */
  col: number;
  name: string;
  /** One line under the H1. */
  blurb: string;
  /** The rows they beat us on, said plainly and first. */
  theyWin: string[];
  /** The honest close, two or three sentences. */
  verdict: string;
}

// Written to be read by someone deciding between the two apps, not by a search engine: the rows
// they win go above the rows we win.
export const COMPETITORS: Competitor[] = [
  {
    slug: "radarscope",
    col: 1,
    name: "RadarScope",
    blurb: "The chaser standard, and the app HookEcho is measured against most often.",
    theyWin: [
      "The phone app is more polished than ours, and has been for a decade.",
      "There is an iPhone and iPad build. We have Android and desktop.",
      "Its data plumbing — SPC outlooks, extra products, the pro tiers — is deeper in places.",
    ],
    verdict:
      "RadarScope is a good app that costs money, and the things you buy on top of it — lightning, the mosaic, archive playback — are things HookEcho gives away because they come from public NOAA feeds. If the polish of the phone app is what you care about, buy RadarScope. If you want every Level 2 product, the archive back to 1991 and no bill, take ours for a drive in the browser first.",
  },
  {
    slug: "myradar",
    col: 2,
    name: "MyRadar",
    blurb: "The popular free radar app, paid for with ads.",
    theyWin: [
      "A far bigger phone app team, and an iPhone build.",
      "Route and aviation extras HookEcho does not attempt.",
    ],
    verdict:
      "MyRadar shows mosaic imagery: one national picture, one product, already rendered. That is fine for whether it is about to rain. It is not enough to tell a rotating storm from a heavy one, which needs velocity and dual-pol at the radar's own resolution — what HookEcho reads directly, with no ads and no subscription.",
  },
  {
    slug: "windy",
    col: 3,
    name: "Windy",
    blurb: "The best model visualisation on the web. Its radar is the weak part.",
    theyWin: [
      "Global forecast models, and nobody in this list is close.",
      "Genuinely global coverage, and a beautiful web app.",
    ],
    verdict:
      "Use Windy for the models — we say so on the comparison page too. But Windy's radar is a composite: one blended image, no tilts, no velocity, no dual-pol. When a storm is over your house, that is the moment HookEcho is for. They are complementary, and both run in a browser, so keep both tabs open.",
  },
  {
    slug: "rainviewer",
    col: 5,
    name: "RainViewer",
    blurb: "Global rain mosaics on a subscription. Clime is the same shape of product.",
    theyWin: [
      "Worldwide coverage: HookEcho is the US plus the German DWD network.",
      "iPhone builds, and a simpler app for a simpler question.",
    ],
    verdict:
      "RainViewer answers \"is rain coming\" well, worldwide, and if that is your question it is a reasonable app to pay for. It cannot answer \"is this storm rotating\", because mosaic imagery has thrown that information away before it reaches you. HookEcho decodes the volume on your own machine, so nothing is thrown away — inside the US, for free.",
  },
  {
    slug: "radaromega",
    col: 4,
    name: "RadarOmega",
    blurb: "The other serious Level 2 viewer, sold as an app plus add-on packs.",
    theyWin: [
      "A more configurable phone app, with a long list of paid add-on data packs.",
      "iPhone and iPad builds.",
      "More non-US radar networks than our US-plus-Germany coverage.",
    ],
    verdict:
      "RadarOmega and HookEcho read the same public Level 2 volumes and show the same tilts, velocity and dual-pol products. The difference is the bill and the source: RadarOmega charges for the app and again for the add-ons, and you cannot read its code. HookEcho is MIT licensed, runs in a browser with nothing installed, and the archive goes back to 1991 without an add-on.",
  },
];
