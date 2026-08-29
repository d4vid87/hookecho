// The site table carries two-letter codes; the URLs and headings want the name people type.
// ponytail: a plain record, not a package — the list is 46 rows and never changes.
export const STATE_NAMES: Record<string, string> = {
  AK: "Alaska", AL: "Alabama", AR: "Arkansas", AZ: "Arizona", CA: "California",
  CO: "Colorado", CT: "Connecticut", DC: "Washington, DC", DE: "Delaware",
  FL: "Florida", GA: "Georgia",
  GU: "Guam", HI: "Hawaii", IA: "Iowa", ID: "Idaho", IL: "Illinois",
  IN: "Indiana", KS: "Kansas", KY: "Kentucky", LA: "Louisiana", MA: "Massachusetts",
  MD: "Maryland", ME: "Maine", MI: "Michigan", MN: "Minnesota", MO: "Missouri",
  MS: "Mississippi", MT: "Montana", NC: "North Carolina", ND: "North Dakota",
  NE: "Nebraska", NH: "New Hampshire", NJ: "New Jersey", NM: "New Mexico",
  NV: "Nevada", NY: "New York", OH: "Ohio", OK: "Oklahoma", OR: "Oregon",
  PA: "Pennsylvania", PR: "Puerto Rico", RI: "Rhode Island", SC: "South Carolina",
  SD: "South Dakota", TN: "Tennessee", TX: "Texas", UT: "Utah", VA: "Virginia",
  VI: "Virgin Islands", VT: "Vermont", WA: "Washington", WI: "Wisconsin",
  WV: "West Virginia", WY: "Wyoming",
};

export const stateName = (code: string) => STATE_NAMES[code] ?? code;
// "Washington, D.C." has to survive the trip into a URL, so punctuation goes and runs of
// separators collapse to one hyphen.
const slugify = (name: string) =>
  name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "");

export const stateSlug = (code: string) => slugify(stateName(code));

// The two international networks name their regions differently: a DWD row's `state` is a German
// Land, an OPERA row's is the country itself. Both get their own page, keyed on the *name* rather
// than the code — codes collide across borders (BE is both Berlin and Belgium, MT both Montana and
// Malta) and names do not.
export const LAND_NAMES: Record<string, string> = {
  BB: "Brandenburg", BE: "Berlin", BW: "Baden-Wurttemberg", BY: "Bavaria",
  HB: "Bremen", HE: "Hesse", HH: "Hamburg", MV: "Mecklenburg-Vorpommern",
  NI: "Lower Saxony", NW: "North Rhine-Westphalia", RP: "Rhineland-Palatinate",
  SH: "Schleswig-Holstein", SL: "Saarland", SN: "Saxony", ST: "Saxony-Anhalt",
  TH: "Thuringia",
};

export const COUNTRY_NAMES: Record<string, string> = {
  BE: "Belgium", CZ: "Czechia", DE: "Germany", DK: "Denmark", HR: "Croatia",
  IE: "Ireland", IS: "Iceland", NL: "Netherlands", PL: "Poland", RO: "Romania",
  SI: "Slovenia", US: "United States",
};

export const countryName = (code: string) => COUNTRY_NAMES[code] ?? code;

// A site's sub-national label, for the "Boostedt, Schleswig-Holstein" line: the state name in the
// US, the Land name in Germany, and nothing at all for OPERA, whose `state` is the country and
// would otherwise read "Helchteren, Belgium, Belgium".
export const areaName = (site: { state: string; country: string }) =>
  site.country === "US"
    ? stateName(site.state)
    : site.country === "DE"
      ? (LAND_NAMES[site.state] ?? site.state)
      : "";

// The region page a site belongs to: its state in the US, its country everywhere else. One page
// per German Land would be three sites at best, and the country page is the one a reader wants.
export const regionName = (site: { state: string; country: string }) =>
  site.country === "US" ? stateName(site.state) : countryName(site.country);

export const regionSlug = (site: { state: string; country: string }) => slugify(regionName(site));

