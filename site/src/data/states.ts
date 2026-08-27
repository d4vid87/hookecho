// The site table carries two-letter codes; the URLs and headings want the name people type.
// ponytail: a plain record, not a package — the list is 46 rows and never changes.
export const STATE_NAMES: Record<string, string> = {
  AK: "Alaska", AL: "Alabama", AR: "Arkansas", AZ: "Arizona", CA: "California",
  CO: "Colorado", CT: "Connecticut", DE: "Delaware", FL: "Florida", GA: "Georgia",
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
export const stateSlug = (code: string) => stateName(code).toLowerCase().replace(/ /g, "-");
