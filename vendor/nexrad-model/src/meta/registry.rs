//! Static registry of NEXRAD radar sites.
//!
//! This module provides a compile-time registry of all operational NEXRAD WSR-88D radar
//! sites in the United States and its territories. Site data is sourced from NOAA's
//! official radar network documentation.

/// Metadata for a NEXRAD radar site in the static registry.
#[derive(Debug, Clone, PartialEq)]
pub struct SiteEntry {
    /// Four-letter ICAO identifier (e.g., "KTLX").
    pub id: &'static str,
    /// City or location name (e.g., "Oklahoma City").
    pub city: &'static str,
    /// Two-letter state/territory abbreviation (e.g., "OK").
    pub state: &'static str,
    /// Latitude in degrees.
    pub latitude: f32,
    /// Longitude in degrees.
    pub longitude: f32,
    /// Elevation above sea level in meters.
    pub elevation_meters: i16,
}

impl SiteEntry {
    /// Create a [`super::Site`] from this registry entry.
    pub fn to_site(&self) -> super::Site {
        let mut id = [0u8; 4];
        let bytes = self.id.as_bytes();
        let len = bytes.len().min(4);
        id[..len].copy_from_slice(&bytes[..len]);

        super::Site::new(id, self.latitude, self.longitude, self.elevation_meters, 0)
    }
}

impl std::fmt::Display for SiteEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({}, {})", self.id, self.city, self.state)
    }
}

/// Returns all NEXRAD radar sites in the registry.
pub fn sites() -> &'static [SiteEntry] {
    SITES
}

/// Look up a NEXRAD radar site by its ICAO identifier (case-insensitive).
///
/// # Example
///
/// ```
/// use nexrad_model::meta::registry;
///
/// let site = registry::site_by_id("KTLX").unwrap();
/// assert_eq!(site.city, "Oklahoma City");
/// assert_eq!(site.state, "OK");
/// ```
pub fn site_by_id(id: &str) -> Option<&'static SiteEntry> {
    let id_upper = id.to_uppercase();
    SITES.iter().find(|s| s.id == id_upper)
}

/// Find the nearest NEXRAD site to a given latitude/longitude.
///
/// Uses great-circle distance approximation. Returns `None` only if the
/// registry is empty (which should never happen).
///
/// # Example
///
/// ```
/// use nexrad_model::meta::registry;
///
/// // Find the nearest radar to downtown Oklahoma City
/// let site = registry::nearest_site(35.4676, -97.5164).unwrap();
/// assert_eq!(site.id, "KTLX");
/// ```
pub fn nearest_site(latitude: f32, longitude: f32) -> Option<&'static SiteEntry> {
    SITES.iter().min_by(|a, b| {
        let dist_a = approx_distance_squared(latitude, longitude, a.latitude, a.longitude);
        let dist_b = approx_distance_squared(latitude, longitude, b.latitude, b.longitude);
        dist_a
            .partial_cmp(&dist_b)
            .unwrap_or(std::cmp::Ordering::Equal)
    })
}

/// Approximate squared distance for comparison purposes (not actual distance).
/// Uses equirectangular approximation, which is adequate for nearest-neighbor queries
/// within CONUS-scale distances.
fn approx_distance_squared(lat1: f32, lon1: f32, lat2: f32, lon2: f32) -> f32 {
    let dlat = lat2 - lat1;
    let dlon = (lon2 - lon1) * ((lat1 + lat2) * 0.5).to_radians().cos();
    dlat * dlat + dlon * dlon
}

/// Static registry of all NEXRAD WSR-88D radar sites.
///
/// Data sourced from NOAA Radar Operations Center and NWS documentation.
/// Coordinates and elevations are approximate.
static SITES: &[SiteEntry] = &[
    // Alabama
    SiteEntry {
        id: "KBMX",
        city: "Birmingham",
        state: "AL",
        latitude: 33.1722,
        longitude: -86.7698,
        elevation_meters: 197,
    },
    SiteEntry {
        id: "KEOX",
        city: "Fort Rucker",
        state: "AL",
        latitude: 31.4606,
        longitude: -85.4592,
        elevation_meters: 132,
    },
    SiteEntry {
        id: "KHTX",
        city: "Huntsville",
        state: "AL",
        latitude: 34.9306,
        longitude: -86.0833,
        elevation_meters: 536,
    },
    SiteEntry {
        id: "KMOB",
        city: "Mobile",
        state: "AL",
        latitude: 30.6794,
        longitude: -88.2397,
        elevation_meters: 63,
    },
    SiteEntry {
        id: "KMXX",
        city: "Maxwell AFB",
        state: "AL",
        latitude: 32.5367,
        longitude: -85.7897,
        elevation_meters: 122,
    },
    // Alaska
    SiteEntry {
        id: "KABC",
        city: "Bethel",
        state: "AK",
        latitude: 60.7919,
        longitude: -161.8764,
        elevation_meters: 48,
    },
    SiteEntry {
        id: "KAKQ",
        city: "Wakefield",
        state: "VA",
        latitude: 36.9839,
        longitude: -77.0072,
        elevation_meters: 34,
    },
    SiteEntry {
        id: "KAPX",
        city: "Gaylord",
        state: "MI",
        latitude: 44.9072,
        longitude: -84.7197,
        elevation_meters: 446,
    },
    SiteEntry {
        id: "KACG",
        city: "Sitka",
        state: "AK",
        latitude: 56.8525,
        longitude: -135.5294,
        elevation_meters: 63,
    },
    SiteEntry {
        id: "KAIH",
        city: "Middleton Island",
        state: "AK",
        latitude: 59.4614,
        longitude: -146.3031,
        elevation_meters: 20,
    },
    SiteEntry {
        id: "KAKC",
        city: "King Salmon",
        state: "AK",
        latitude: 58.6794,
        longitude: -156.6297,
        elevation_meters: 19,
    },
    SiteEntry {
        id: "PABC",
        city: "Bethel",
        state: "AK",
        latitude: 60.7919,
        longitude: -161.8764,
        elevation_meters: 48,
    },
    SiteEntry {
        id: "PACG",
        city: "Sitka",
        state: "AK",
        latitude: 56.8525,
        longitude: -135.5294,
        elevation_meters: 63,
    },
    SiteEntry {
        id: "PAEC",
        city: "Nome",
        state: "AK",
        latitude: 64.5114,
        longitude: -165.295,
        elevation_meters: 16,
    },
    SiteEntry {
        id: "PAHG",
        city: "Anchorage",
        state: "AK",
        latitude: 60.7258,
        longitude: -151.3514,
        elevation_meters: 74,
    },
    SiteEntry {
        id: "PAIH",
        city: "Middleton Island",
        state: "AK",
        latitude: 59.4614,
        longitude: -146.3031,
        elevation_meters: 20,
    },
    SiteEntry {
        id: "PAKC",
        city: "King Salmon",
        state: "AK",
        latitude: 58.6794,
        longitude: -156.6297,
        elevation_meters: 19,
    },
    // Arizona
    SiteEntry {
        id: "KEMX",
        city: "Tucson",
        state: "AZ",
        latitude: 31.8939,
        longitude: -110.6303,
        elevation_meters: 1586,
    },
    SiteEntry {
        id: "KFSX",
        city: "Flagstaff",
        state: "AZ",
        latitude: 34.5744,
        longitude: -111.1983,
        elevation_meters: 2261,
    },
    SiteEntry {
        id: "KIWA",
        city: "Phoenix",
        state: "AZ",
        latitude: 33.2892,
        longitude: -111.67,
        elevation_meters: 413,
    },
    SiteEntry {
        id: "KYUX",
        city: "Yuma",
        state: "AZ",
        latitude: 32.4953,
        longitude: -114.6567,
        elevation_meters: 53,
    },
    // Arkansas
    SiteEntry {
        id: "KLZK",
        city: "Little Rock",
        state: "AR",
        latitude: 34.8364,
        longitude: -92.2622,
        elevation_meters: 173,
    },
    SiteEntry {
        id: "KSRX",
        city: "Fort Smith",
        state: "AR",
        latitude: 35.2906,
        longitude: -94.3619,
        elevation_meters: 195,
    },
    // California
    SiteEntry {
        id: "KBBX",
        city: "Beale AFB",
        state: "CA",
        latitude: 39.4961,
        longitude: -121.6317,
        elevation_meters: 53,
    },
    SiteEntry {
        id: "KEYX",
        city: "Edwards AFB",
        state: "CA",
        latitude: 35.0978,
        longitude: -117.5608,
        elevation_meters: 840,
    },
    SiteEntry {
        id: "KHNX",
        city: "Hanford",
        state: "CA",
        latitude: 36.3142,
        longitude: -119.6322,
        elevation_meters: 74,
    },
    SiteEntry {
        id: "KMUX",
        city: "San Francisco",
        state: "CA",
        latitude: 37.1553,
        longitude: -121.8983,
        elevation_meters: 1057,
    },
    SiteEntry {
        id: "KNKX",
        city: "San Diego",
        state: "CA",
        latitude: 32.9189,
        longitude: -117.0419,
        elevation_meters: 291,
    },
    SiteEntry {
        id: "KSOX",
        city: "Santa Ana Mountains",
        state: "CA",
        latitude: 33.8178,
        longitude: -117.6358,
        elevation_meters: 923,
    },
    SiteEntry {
        id: "KVBX",
        city: "Vandenberg AFB",
        state: "CA",
        latitude: 34.8383,
        longitude: -120.3978,
        elevation_meters: 373,
    },
    SiteEntry {
        id: "KVTX",
        city: "Los Angeles",
        state: "CA",
        latitude: 34.4114,
        longitude: -119.1794,
        elevation_meters: 831,
    },
    SiteEntry {
        id: "KDAX",
        city: "Sacramento",
        state: "CA",
        latitude: 38.5011,
        longitude: -121.6778,
        elevation_meters: 9,
    },
    // Colorado
    SiteEntry {
        id: "KFTG",
        city: "Denver",
        state: "CO",
        latitude: 39.7867,
        longitude: -104.5458,
        elevation_meters: 1675,
    },
    SiteEntry {
        id: "KGJX",
        city: "Grand Junction",
        state: "CO",
        latitude: 39.0622,
        longitude: -108.2139,
        elevation_meters: 3045,
    },
    SiteEntry {
        id: "KPUX",
        city: "Pueblo",
        state: "CO",
        latitude: 38.4594,
        longitude: -104.1817,
        elevation_meters: 1600,
    },
    // Connecticut / New England
    SiteEntry {
        id: "KOKX",
        city: "Upton",
        state: "NY",
        latitude: 40.8656,
        longitude: -72.8639,
        elevation_meters: 26,
    },
    // Florida
    SiteEntry {
        id: "KAMX",
        city: "Miami",
        state: "FL",
        latitude: 25.6111,
        longitude: -80.4128,
        elevation_meters: 4,
    },
    SiteEntry {
        id: "KEVX",
        city: "Eglin AFB",
        state: "FL",
        latitude: 30.5644,
        longitude: -85.9214,
        elevation_meters: 43,
    },
    SiteEntry {
        id: "KJAX",
        city: "Jacksonville",
        state: "FL",
        latitude: 30.4847,
        longitude: -81.7019,
        elevation_meters: 10,
    },
    SiteEntry {
        id: "KMLB",
        city: "Melbourne",
        state: "FL",
        latitude: 28.1133,
        longitude: -80.6542,
        elevation_meters: 11,
    },
    SiteEntry {
        id: "KTBW",
        city: "Tampa Bay",
        state: "FL",
        latitude: 27.7056,
        longitude: -82.4017,
        elevation_meters: 13,
    },
    SiteEntry {
        id: "KTLH",
        city: "Tallahassee",
        state: "FL",
        latitude: 30.3975,
        longitude: -84.3289,
        elevation_meters: 19,
    },
    SiteEntry {
        id: "KBYX",
        city: "Key West",
        state: "FL",
        latitude: 24.5975,
        longitude: -81.7031,
        elevation_meters: 3,
    },
    // Georgia
    SiteEntry {
        id: "KFFC",
        city: "Atlanta",
        state: "GA",
        latitude: 33.3636,
        longitude: -84.5658,
        elevation_meters: 262,
    },
    SiteEntry {
        id: "KJGX",
        city: "Robins AFB",
        state: "GA",
        latitude: 32.6756,
        longitude: -83.3511,
        elevation_meters: 159,
    },
    SiteEntry {
        id: "KVAX",
        city: "Moody AFB",
        state: "GA",
        latitude: 30.8900,
        longitude: -83.0019,
        elevation_meters: 54,
    },
    // Hawaii
    SiteEntry {
        id: "PHKI",
        city: "South Kauai",
        state: "HI",
        latitude: 21.8939,
        longitude: -159.5522,
        elevation_meters: 55,
    },
    SiteEntry {
        id: "PHKM",
        city: "Kamuela",
        state: "HI",
        latitude: 20.1253,
        longitude: -155.7781,
        elevation_meters: 1161,
    },
    SiteEntry {
        id: "PHMO",
        city: "Molokai",
        state: "HI",
        latitude: 21.1328,
        longitude: -157.1803,
        elevation_meters: 416,
    },
    SiteEntry {
        id: "PHWA",
        city: "South Shore",
        state: "HI",
        latitude: 19.0950,
        longitude: -155.5689,
        elevation_meters: 421,
    },
    // Idaho
    SiteEntry {
        id: "KCBX",
        city: "Boise",
        state: "ID",
        latitude: 43.4908,
        longitude: -116.2358,
        elevation_meters: 933,
    },
    SiteEntry {
        id: "KSFX",
        city: "Pocatello",
        state: "ID",
        latitude: 43.1058,
        longitude: -112.6861,
        elevation_meters: 1364,
    },
    // Illinois
    SiteEntry {
        id: "KILX",
        city: "Lincoln",
        state: "IL",
        latitude: 40.1506,
        longitude: -89.3369,
        elevation_meters: 177,
    },
    SiteEntry {
        id: "KLOT",
        city: "Chicago",
        state: "IL",
        latitude: 41.6044,
        longitude: -88.0847,
        elevation_meters: 202,
    },
    // Indiana
    SiteEntry {
        id: "KIND",
        city: "Indianapolis",
        state: "IN",
        latitude: 39.7075,
        longitude: -86.2803,
        elevation_meters: 241,
    },
    SiteEntry {
        id: "KIWX",
        city: "Fort Wayne",
        state: "IN",
        latitude: 41.3586,
        longitude: -85.7000,
        elevation_meters: 293,
    },
    SiteEntry {
        id: "KVWX",
        city: "Evansville",
        state: "IN",
        latitude: 38.2603,
        longitude: -87.7247,
        elevation_meters: 155,
    },
    // Iowa
    SiteEntry {
        id: "KDMX",
        city: "Des Moines",
        state: "IA",
        latitude: 41.7311,
        longitude: -93.7228,
        elevation_meters: 299,
    },
    SiteEntry {
        id: "KDVN",
        city: "Davenport",
        state: "IA",
        latitude: 41.6117,
        longitude: -90.5808,
        elevation_meters: 230,
    },
    // Kansas
    SiteEntry {
        id: "KDDC",
        city: "Dodge City",
        state: "KS",
        latitude: 37.7608,
        longitude: -99.9689,
        elevation_meters: 790,
    },
    SiteEntry {
        id: "KGLD",
        city: "Goodland",
        state: "KS",
        latitude: 39.3669,
        longitude: -101.7003,
        elevation_meters: 1113,
    },
    SiteEntry {
        id: "KICT",
        city: "Wichita",
        state: "KS",
        latitude: 37.6544,
        longitude: -97.4428,
        elevation_meters: 407,
    },
    SiteEntry {
        id: "KTWX",
        city: "Topeka",
        state: "KS",
        latitude: 38.9969,
        longitude: -96.2325,
        elevation_meters: 417,
    },
    // Kentucky
    SiteEntry {
        id: "KHPX",
        city: "Fort Campbell",
        state: "KY",
        latitude: 36.7369,
        longitude: -87.2850,
        elevation_meters: 177,
    },
    SiteEntry {
        id: "KJKL",
        city: "Jackson",
        state: "KY",
        latitude: 37.5908,
        longitude: -83.3131,
        elevation_meters: 415,
    },
    SiteEntry {
        id: "KLVX",
        city: "Louisville",
        state: "KY",
        latitude: 37.9753,
        longitude: -85.9439,
        elevation_meters: 219,
    },
    SiteEntry {
        id: "KPAH",
        city: "Paducah",
        state: "KY",
        latitude: 37.0683,
        longitude: -88.7722,
        elevation_meters: 119,
    },
    // Louisiana
    SiteEntry {
        id: "KHGX",
        city: "Houston",
        state: "TX",
        latitude: 29.4719,
        longitude: -95.0792,
        elevation_meters: 5,
    },
    SiteEntry {
        id: "KLCH",
        city: "Lake Charles",
        state: "LA",
        latitude: 30.1253,
        longitude: -93.2158,
        elevation_meters: 4,
    },
    SiteEntry {
        id: "KLIX",
        city: "New Orleans",
        state: "LA",
        latitude: 30.3367,
        longitude: -89.8256,
        elevation_meters: 7,
    },
    SiteEntry {
        id: "KPOE",
        city: "Fort Polk",
        state: "LA",
        latitude: 31.1556,
        longitude: -92.9756,
        elevation_meters: 124,
    },
    SiteEntry {
        id: "KSHV",
        city: "Shreveport",
        state: "LA",
        latitude: 32.4508,
        longitude: -93.8411,
        elevation_meters: 83,
    },
    // Maine
    SiteEntry {
        id: "KCBW",
        city: "Caribou",
        state: "ME",
        latitude: 46.0392,
        longitude: -67.8067,
        elevation_meters: 227,
    },
    SiteEntry {
        id: "KGYX",
        city: "Portland",
        state: "ME",
        latitude: 43.8914,
        longitude: -70.2567,
        elevation_meters: 125,
    },
    // Maryland
    SiteEntry {
        id: "KLWX",
        city: "Sterling",
        state: "VA",
        latitude: 38.9753,
        longitude: -77.4778,
        elevation_meters: 83,
    },
    // Massachusetts
    SiteEntry {
        id: "KBOX",
        city: "Boston",
        state: "MA",
        latitude: 41.9558,
        longitude: -71.1369,
        elevation_meters: 36,
    },
    // Michigan
    SiteEntry {
        id: "KDTX",
        city: "Detroit",
        state: "MI",
        latitude: 42.6997,
        longitude: -83.4717,
        elevation_meters: 327,
    },
    SiteEntry {
        id: "KGRR",
        city: "Grand Rapids",
        state: "MI",
        latitude: 42.8939,
        longitude: -85.5447,
        elevation_meters: 237,
    },
    SiteEntry {
        id: "KMQT",
        city: "Marquette",
        state: "MI",
        latitude: 46.5314,
        longitude: -87.5486,
        elevation_meters: 430,
    },
    // Minnesota
    SiteEntry {
        id: "KDLH",
        city: "Duluth",
        state: "MN",
        latitude: 46.8369,
        longitude: -92.2097,
        elevation_meters: 435,
    },
    SiteEntry {
        id: "KMPX",
        city: "Minneapolis",
        state: "MN",
        latitude: 44.8489,
        longitude: -93.5653,
        elevation_meters: 288,
    },
    // Mississippi
    SiteEntry {
        id: "KDGX",
        city: "Brandon",
        state: "MS",
        latitude: 32.2800,
        longitude: -89.9844,
        elevation_meters: 149,
    },
    SiteEntry {
        id: "KGWX",
        city: "Columbus AFB",
        state: "MS",
        latitude: 33.8967,
        longitude: -88.3289,
        elevation_meters: 145,
    },
    // Missouri
    SiteEntry {
        id: "KEAX",
        city: "Kansas City",
        state: "MO",
        latitude: 38.8103,
        longitude: -94.2644,
        elevation_meters: 303,
    },
    SiteEntry {
        id: "KLSX",
        city: "St. Louis",
        state: "MO",
        latitude: 38.6986,
        longitude: -90.6828,
        elevation_meters: 185,
    },
    SiteEntry {
        id: "KSGF",
        city: "Springfield",
        state: "MO",
        latitude: 37.2353,
        longitude: -93.4006,
        elevation_meters: 390,
    },
    // Montana
    SiteEntry {
        id: "KBLX",
        city: "Billings",
        state: "MT",
        latitude: 45.8536,
        longitude: -108.6069,
        elevation_meters: 1097,
    },
    SiteEntry {
        id: "KGGW",
        city: "Glasgow",
        state: "MT",
        latitude: 48.2064,
        longitude: -106.6253,
        elevation_meters: 694,
    },
    SiteEntry {
        id: "KMSX",
        city: "Missoula",
        state: "MT",
        latitude: 47.0411,
        longitude: -113.9864,
        elevation_meters: 2394,
    },
    SiteEntry {
        id: "KTFX",
        city: "Great Falls",
        state: "MT",
        latitude: 47.4597,
        longitude: -111.3853,
        elevation_meters: 1132,
    },
    // Nebraska
    SiteEntry {
        id: "KLNX",
        city: "North Platte",
        state: "NE",
        latitude: 41.9578,
        longitude: -100.5761,
        elevation_meters: 906,
    },
    SiteEntry {
        id: "KOAX",
        city: "Omaha",
        state: "NE",
        latitude: 41.3203,
        longitude: -96.3667,
        elevation_meters: 350,
    },
    SiteEntry {
        id: "KUEX",
        city: "Hastings",
        state: "NE",
        latitude: 40.3208,
        longitude: -98.4419,
        elevation_meters: 602,
    },
    // Nevada
    SiteEntry {
        id: "KLRX",
        city: "Elko",
        state: "NV",
        latitude: 40.7400,
        longitude: -116.8025,
        elevation_meters: 2056,
    },
    SiteEntry {
        id: "KESX",
        city: "Las Vegas",
        state: "NV",
        latitude: 35.7011,
        longitude: -114.8914,
        elevation_meters: 1483,
    },
    SiteEntry {
        id: "KRGX",
        city: "Reno",
        state: "NV",
        latitude: 39.7542,
        longitude: -119.4622,
        elevation_meters: 2530,
    },
    // New Mexico
    SiteEntry {
        id: "KABX",
        city: "Albuquerque",
        state: "NM",
        latitude: 35.1497,
        longitude: -106.8239,
        elevation_meters: 1789,
    },
    SiteEntry {
        id: "KFDX",
        city: "Cannon AFB",
        state: "NM",
        latitude: 34.6350,
        longitude: -103.6297,
        elevation_meters: 1417,
    },
    SiteEntry {
        id: "KHDX",
        city: "Holloman AFB",
        state: "NM",
        latitude: 33.0769,
        longitude: -106.12,
        elevation_meters: 1287,
    },
    // New York
    SiteEntry {
        id: "KBGM",
        city: "Binghamton",
        state: "NY",
        latitude: 42.1997,
        longitude: -75.9847,
        elevation_meters: 490,
    },
    SiteEntry {
        id: "KBUF",
        city: "Buffalo",
        state: "NY",
        latitude: 42.9486,
        longitude: -78.7369,
        elevation_meters: 211,
    },
    SiteEntry {
        id: "KENX",
        city: "Albany",
        state: "NY",
        latitude: 42.5864,
        longitude: -74.0639,
        elevation_meters: 557,
    },
    SiteEntry {
        id: "KTYX",
        city: "Montague",
        state: "NY",
        latitude: 43.7558,
        longitude: -75.6800,
        elevation_meters: 562,
    },
    // North Carolina
    SiteEntry {
        id: "KMHX",
        city: "Morehead City",
        state: "NC",
        latitude: 34.7761,
        longitude: -76.8764,
        elevation_meters: 9,
    },
    SiteEntry {
        id: "KRAX",
        city: "Raleigh",
        state: "NC",
        latitude: 35.6656,
        longitude: -78.4903,
        elevation_meters: 106,
    },
    SiteEntry {
        id: "KLTX",
        city: "Wilmington",
        state: "NC",
        latitude: 33.9892,
        longitude: -78.4292,
        elevation_meters: 19,
    },
    // North Dakota
    SiteEntry {
        id: "KBIS",
        city: "Bismarck",
        state: "ND",
        latitude: 46.7708,
        longitude: -100.7603,
        elevation_meters: 505,
    },
    SiteEntry {
        id: "KMVX",
        city: "Fargo",
        state: "ND",
        latitude: 47.5281,
        longitude: -97.3256,
        elevation_meters: 301,
    },
    SiteEntry {
        id: "KMBX",
        city: "Minot AFB",
        state: "ND",
        latitude: 48.3925,
        longitude: -100.8644,
        elevation_meters: 455,
    },
    // Ohio
    SiteEntry {
        id: "KCLE",
        city: "Cleveland",
        state: "OH",
        latitude: 41.4131,
        longitude: -81.8597,
        elevation_meters: 233,
    },
    SiteEntry {
        id: "KILN",
        city: "Wilmington",
        state: "OH",
        latitude: 39.4203,
        longitude: -83.8217,
        elevation_meters: 322,
    },
    // Oklahoma
    SiteEntry {
        id: "KFDR",
        city: "Frederick",
        state: "OK",
        latitude: 34.3622,
        longitude: -98.9764,
        elevation_meters: 386,
    },
    SiteEntry {
        id: "KINX",
        city: "Tulsa",
        state: "OK",
        latitude: 36.1750,
        longitude: -95.5647,
        elevation_meters: 204,
    },
    SiteEntry {
        id: "KTLX",
        city: "Oklahoma City",
        state: "OK",
        latitude: 35.3331,
        longitude: -97.2778,
        elevation_meters: 370,
    },
    SiteEntry {
        id: "KVNX",
        city: "Vance AFB",
        state: "OK",
        latitude: 36.7406,
        longitude: -98.1278,
        elevation_meters: 369,
    },
    // Oregon
    SiteEntry {
        id: "KMAX",
        city: "Medford",
        state: "OR",
        latitude: 42.0811,
        longitude: -122.7167,
        elevation_meters: 2290,
    },
    SiteEntry {
        id: "KPDT",
        city: "Pendleton",
        state: "OR",
        latitude: 45.6906,
        longitude: -118.8528,
        elevation_meters: 462,
    },
    SiteEntry {
        id: "KRTX",
        city: "Portland",
        state: "OR",
        latitude: 45.7150,
        longitude: -122.9653,
        elevation_meters: 479,
    },
    // Pennsylvania
    SiteEntry {
        id: "KCCX",
        city: "State College",
        state: "PA",
        latitude: 40.9231,
        longitude: -78.0039,
        elevation_meters: 733,
    },
    SiteEntry {
        id: "KDIX",
        city: "Philadelphia",
        state: "PA",
        latitude: 39.9469,
        longitude: -74.4108,
        elevation_meters: 45,
    },
    SiteEntry {
        id: "KPBZ",
        city: "Pittsburgh",
        state: "PA",
        latitude: 40.5317,
        longitude: -80.0183,
        elevation_meters: 361,
    },
    // Puerto Rico
    SiteEntry {
        id: "TJUA",
        city: "San Juan",
        state: "PR",
        latitude: 18.1156,
        longitude: -66.0781,
        elevation_meters: 867,
    },
    // South Carolina
    SiteEntry {
        id: "KCAE",
        city: "Columbia",
        state: "SC",
        latitude: 33.9486,
        longitude: -81.1186,
        elevation_meters: 70,
    },
    SiteEntry {
        id: "KCLX",
        city: "Charleston",
        state: "SC",
        latitude: 32.6556,
        longitude: -81.0425,
        elevation_meters: 30,
    },
    SiteEntry {
        id: "KGSP",
        city: "Greenville",
        state: "SC",
        latitude: 34.8833,
        longitude: -82.2200,
        elevation_meters: 296,
    },
    // South Dakota
    SiteEntry {
        id: "KABR",
        city: "Aberdeen",
        state: "SD",
        latitude: 45.4558,
        longitude: -98.4131,
        elevation_meters: 397,
    },
    SiteEntry {
        id: "KUDX",
        city: "Rapid City",
        state: "SD",
        latitude: 44.1250,
        longitude: -102.8297,
        elevation_meters: 919,
    },
    SiteEntry {
        id: "KFSD",
        city: "Sioux Falls",
        state: "SD",
        latitude: 43.5878,
        longitude: -96.7289,
        elevation_meters: 436,
    },
    // Tennessee
    SiteEntry {
        id: "KMRX",
        city: "Knoxville",
        state: "TN",
        latitude: 36.1686,
        longitude: -83.4017,
        elevation_meters: 408,
    },
    SiteEntry {
        id: "KNQA",
        city: "Memphis",
        state: "TN",
        latitude: 35.3447,
        longitude: -89.8733,
        elevation_meters: 86,
    },
    SiteEntry {
        id: "KOHX",
        city: "Nashville",
        state: "TN",
        latitude: 36.2472,
        longitude: -86.5625,
        elevation_meters: 176,
    },
    // Texas
    SiteEntry {
        id: "KAMA",
        city: "Amarillo",
        state: "TX",
        latitude: 35.2333,
        longitude: -101.7092,
        elevation_meters: 1093,
    },
    SiteEntry {
        id: "KBRO",
        city: "Brownsville",
        state: "TX",
        latitude: 25.9158,
        longitude: -97.4189,
        elevation_meters: 7,
    },
    SiteEntry {
        id: "KCRP",
        city: "Corpus Christi",
        state: "TX",
        latitude: 27.7842,
        longitude: -97.5111,
        elevation_meters: 14,
    },
    SiteEntry {
        id: "KDFX",
        city: "Laughlin AFB",
        state: "TX",
        latitude: 29.2722,
        longitude: -100.2803,
        elevation_meters: 345,
    },
    SiteEntry {
        id: "KDYX",
        city: "Dyess AFB",
        state: "TX",
        latitude: 32.5386,
        longitude: -99.2544,
        elevation_meters: 463,
    },
    SiteEntry {
        id: "KEPZ",
        city: "El Paso",
        state: "TX",
        latitude: 31.8731,
        longitude: -106.6978,
        elevation_meters: 1251,
    },
    SiteEntry {
        id: "KEWX",
        city: "San Antonio",
        state: "TX",
        latitude: 29.7039,
        longitude: -98.0286,
        elevation_meters: 193,
    },
    SiteEntry {
        id: "KFWS",
        city: "Dallas/Fort Worth",
        state: "TX",
        latitude: 32.5731,
        longitude: -97.3031,
        elevation_meters: 208,
    },
    SiteEntry {
        id: "KGRK",
        city: "Fort Hood",
        state: "TX",
        latitude: 30.7217,
        longitude: -97.3828,
        elevation_meters: 164,
    },
    SiteEntry {
        id: "KLBB",
        city: "Lubbock",
        state: "TX",
        latitude: 33.6542,
        longitude: -101.8142,
        elevation_meters: 993,
    },
    SiteEntry {
        id: "KMAF",
        city: "Midland/Odessa",
        state: "TX",
        latitude: 31.9433,
        longitude: -102.1892,
        elevation_meters: 874,
    },
    SiteEntry {
        id: "KSJT",
        city: "San Angelo",
        state: "TX",
        latitude: 31.3714,
        longitude: -100.4925,
        elevation_meters: 576,
    },
    // Utah
    SiteEntry {
        id: "KICX",
        city: "Cedar City",
        state: "UT",
        latitude: 37.5911,
        longitude: -112.8622,
        elevation_meters: 3231,
    },
    SiteEntry {
        id: "KMTX",
        city: "Salt Lake City",
        state: "UT",
        latitude: 41.2628,
        longitude: -112.4478,
        elevation_meters: 1969,
    },
    // Vermont
    SiteEntry {
        id: "KCXX",
        city: "Burlington",
        state: "VT",
        latitude: 44.5111,
        longitude: -73.1667,
        elevation_meters: 97,
    },
    // Virginia
    SiteEntry {
        id: "KFCX",
        city: "Roanoke",
        state: "VA",
        latitude: 37.0242,
        longitude: -80.2744,
        elevation_meters: 874,
    },
    // Washington
    SiteEntry {
        id: "KATX",
        city: "Seattle",
        state: "WA",
        latitude: 48.1944,
        longitude: -122.4958,
        elevation_meters: 151,
    },
    SiteEntry {
        id: "KOTX",
        city: "Spokane",
        state: "WA",
        latitude: 47.6806,
        longitude: -117.6267,
        elevation_meters: 728,
    },
    // West Virginia
    SiteEntry {
        id: "KRLX",
        city: "Charleston",
        state: "WV",
        latitude: 38.3111,
        longitude: -81.7231,
        elevation_meters: 329,
    },
    // Wisconsin
    SiteEntry {
        id: "KARX",
        city: "La Crosse",
        state: "WI",
        latitude: 43.8228,
        longitude: -91.1911,
        elevation_meters: 389,
    },
    SiteEntry {
        id: "KGRB",
        city: "Green Bay",
        state: "WI",
        latitude: 44.4986,
        longitude: -88.1111,
        elevation_meters: 208,
    },
    SiteEntry {
        id: "KMKX",
        city: "Milwaukee",
        state: "WI",
        latitude: 42.9678,
        longitude: -88.5506,
        elevation_meters: 292,
    },
    // Wyoming
    SiteEntry {
        id: "KCYS",
        city: "Cheyenne",
        state: "WY",
        latitude: 41.1519,
        longitude: -104.8061,
        elevation_meters: 1868,
    },
    SiteEntry {
        id: "KRIW",
        city: "Riverton",
        state: "WY",
        latitude: 43.0661,
        longitude: -108.4772,
        elevation_meters: 1697,
    },
    // Guam
    SiteEntry {
        id: "PGUA",
        city: "Andersen AFB",
        state: "GU",
        latitude: 13.4544,
        longitude: 144.8111,
        elevation_meters: 78,
    },
    // DoD/CONUS
    SiteEntry {
        id: "KCCX",
        city: "State College",
        state: "PA",
        latitude: 40.9231,
        longitude: -78.0039,
        elevation_meters: 733,
    },
];
