//! Level 3 storm-cell products: fetch, decode, and project cell positions + SCIT forecast
//! tracks to lon/lat for the clickable-overlay and storm-attributes layers.
//!
//! Uses the from-scratch [`nexrad_level3`] decoder. Sources:
//! - **NST** (storm tracking, Unidata bucket, archive-capable): cell dots, plus the tabular
//!   `STORM POSITION/FORECAST` table (current AZ/RAN, movement, 15/30/45/60-min forecast, error)
//!   and the graphic `DBZM HGT` block (max dBZ + height).
//! - **NMD** (mesocyclone, Unidata): TVS / mesocyclone detections.
//! - **SS** (storm structure) and **HI** (hail) via the tgftp `sn.last` feed — the Unidata
//!   bucket stopped carrying these products in ~2021. tgftp payloads are the same WMO container
//!   but are not zlib-compressed; the decoder's zlib sniff-fallback handles both.
//!
//! Tabular parsers are token-walkers (not fixed columns) so they survive RPG-build column drift
//! and page splits; an unparsed row degrades to `None` and never panics.

use nexrad_level3::{decode, Level3Product};
use std::collections::HashMap;

const BUCKET: &str = "https://unidata-nexrad-level3.s3.amazonaws.com";
const TGFTP: &str = "https://tgftp.nws.noaa.gov/SL.us008001/DF.of/DC.radar";

/// What a clickable storm marker represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellKind {
    Storm,
    Hail,
    Meso,
}

/// A forecast track point (SCIT position at `minutes` in the future).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackPoint {
    pub minutes: u16,
    pub lon: f64,
    pub lat: f64,
}

/// A clickable storm-cell marker with typed SCIT attributes. All attribute fields are optional
/// (a product may be missing, or a row unparsable) and render as `—`.
#[derive(Debug, Clone)]
pub struct Cell {
    pub kind: CellKind,
    pub lon: f64,
    pub lat: f64,
    /// SCIT cell id (e.g. "O7"); empty for a standalone detection with no parent cell.
    pub id: String,
    /// Display title (id for storm cells, kind label for standalone detections).
    pub title: String,
    // Current position (from the STORM POSITION/FORECAST table).
    pub az_deg: Option<f32>,
    pub range_nm: Option<f32>,
    // Movement.
    pub mvt_deg: Option<f32>,
    pub mvt_kt: Option<f32>,
    // Intensity & structure.
    pub max_dbz: Option<f32>,
    pub max_dbz_hgt_kft: Option<f32>,
    pub top_kft: Option<f32>,
    pub base_kft: Option<f32>,
    /// The cell base is below the lowest tilt (`<` marker in the structure table).
    pub base_below: bool,
    pub vil: Option<f32>,
    // Hail potential.
    pub poh: Option<i32>,
    pub posh: Option<i32>,
    pub hail_in: Option<f32>,
    // Features.
    pub tvs: Option<String>,
    pub meso: Option<String>,
    // Error metrics (nm).
    pub fcst_err_nm: Option<f32>,
    pub mean_err_nm: Option<f32>,
    /// SCIT forecast track (15/30/45/60-min positions).
    pub track: Vec<TrackPoint>,
    /// SCIT past-position polyline in `(lon, lat)` (oldest→current), from symbology packet 23.
    pub past_track: Vec<(f64, f64)>,
}

impl Cell {
    fn new(kind: CellKind, lon: f64, lat: f64, id: String, title: String) -> Self {
        Cell {
            kind,
            lon,
            lat,
            id,
            title,
            az_deg: None,
            range_nm: None,
            mvt_deg: None,
            mvt_kt: None,
            max_dbz: None,
            max_dbz_hgt_kft: None,
            top_kft: None,
            base_kft: None,
            base_below: false,
            vil: None,
            poh: None,
            posh: None,
            hail_in: None,
            tvs: None,
            meso: None,
            fcst_err_nm: None,
            mean_err_nm: None,
            track: Vec::new(),
            past_track: Vec::new(),
        }
    }

    /// A plain-text attribute summary (used for standalone-marker / generic detail popups).
    pub fn summary(&self) -> String {
        let mut s = String::new();
        let f = |v: Option<f32>, u: &str| v.map(|x| format!("{x:.1}{u}")).unwrap_or_else(|| "—".into());
        if let (Some(a), Some(r)) = (self.az_deg, self.range_nm) {
            s.push_str(&format!("Position: {a:.0}° / {r:.0} nm\n"));
        }
        if let (Some(d), Some(k)) = (self.mvt_deg, self.mvt_kt) {
            s.push_str(&format!("Movement: {d:.0}° @ {k:.0} kt\n"));
        }
        if self.max_dbz.is_some() {
            s.push_str(&format!("Max reflectivity: {} @ {}\n", f(self.max_dbz, " dBZ"), f(self.max_dbz_hgt_kft, " kft")));
        }
        if self.top_kft.is_some() || self.base_kft.is_some() {
            s.push_str(&format!("Top / base: {} / {}\n", f(self.top_kft, " kft"), f(self.base_kft, " kft")));
        }
        if self.vil.is_some() {
            s.push_str(&format!("Cell-based VIL: {}\n", f(self.vil, " kg/m²")));
        }
        if self.poh.is_some() || self.posh.is_some() {
            s.push_str(&format!(
                "POH / POSH: {} / {}%\n",
                self.poh.map(|v| v.to_string()).unwrap_or_else(|| "—".into()),
                self.posh.map(|v| v.to_string()).unwrap_or_else(|| "—".into())
            ));
        }
        if let Some(h) = self.hail_in {
            s.push_str(&format!("Max hail size: {h:.2} in\n"));
        }
        if let Some(t) = &self.tvs {
            s.push_str(&format!("TVS: {t}\n"));
        }
        if let Some(m) = &self.meso {
            s.push_str(&format!("Mesocyclone: {m}\n"));
        }
        s.trim_end().to_string()
    }
}

/// The 3-letter Level 3 site id (drop a leading `K` from CONUS ICAO ids: `KTLX` -> `TLX`).
fn l3_site(site: &str) -> String {
    match site.strip_prefix('K') {
        Some(rest) if rest.len() == 3 => rest.to_string(),
        _ => site.to_string(),
    }
}

/// Mesocyclone/TVS (NMD) detection at a geographic position.
struct MesoFeat {
    lon: f64,
    lat: f64,
    kind: String,
}

/// Association radius: fold a meso detection into the nearest storm cell within this range.
const MERGE_KM: f64 = 10.0;

/// Fetch storm cells (NST) + structure (SS) + hail (HI) + mesocyclones (NMD) for `site` and merge
/// them into one clickable marker per storm. Failed products are skipped (their fields stay `None`).
pub async fn fetch_cells(http: &reqwest::Client, site: &str) -> Vec<Cell> {
    let s3 = l3_site(site);
    let mut storms: Vec<Cell> = Vec::new();
    let mut by_id: HashMap<String, usize> = HashMap::new();

    if let Some(p) = fetch_latest(http, &s3, "NST").await {
        let (lat0, lon0) = (p.lat as f64, p.lon as f64);
        let table = p.tabular.clone().unwrap_or_default();
        let graphic = p.graphic.clone().unwrap_or_default();
        let fcst = parse_position_forecast(&table);
        let dbz = parse_graphic_attrs(&graphic);
        for c in &p.cells {
            let (lon, lat) = offset_lonlat(lon0, lat0, c.x_km, c.y_km);
            let mut cell = Cell::new(CellKind::Storm, lon, lat, c.id.clone(), c.id.clone());
            if let Some(pf) = fcst.get(&c.id) {
                cell.az_deg = Some(pf.az);
                cell.range_nm = Some(pf.range);
                cell.mvt_deg = Some(pf.mvt_deg);
                cell.mvt_kt = Some(pf.mvt_kt);
                cell.fcst_err_nm = Some(pf.fcst_err);
                cell.mean_err_nm = Some(pf.mean_err);
                cell.track = pf
                    .fcst
                    .iter()
                    .map(|&(min, az, rng)| {
                        let (lon, lat) = azran_lonlat(lon0, lat0, az, rng);
                        TrackPoint { minutes: min, lon, lat }
                    })
                    .collect();
            }
            if let Some(&(d, h)) = dbz.get(&c.id) {
                cell.max_dbz = Some(d);
                cell.max_dbz_hgt_kft = Some(h);
            }
            by_id.insert(c.id.clone(), storms.len());
            storms.push(cell);
        }
        // Attach past-track polylines (packet 23) to the nearest cell by their current endpoint.
        for poly in &p.past_tracks {
            let ll: Vec<(f64, f64)> =
                poly.iter().map(|&(x, y)| offset_lonlat(lon0, lat0, x, y)).collect();
            let Some(&(elon, elat)) = ll.last() else { continue };
            if let Some(i) = nearest_storm(&storms, elon, elat) {
                storms[i].past_track = ll;
            }
        }
    }

    if let Some(p) = fetch_tgftp(http, &s3, "p62ss").await {
        for (id, ss) in parse_storm_structure(&p.raw_text.unwrap_or_default()) {
            if let Some(&i) = by_id.get(&id) {
                storms[i].base_kft = ss.base;
                storms[i].base_below = ss.base_below;
                storms[i].top_kft = ss.top;
                storms[i].vil = ss.vil;
                if storms[i].max_dbz.is_none() {
                    storms[i].max_dbz = ss.max_ref;
                }
            }
        }
    }

    if let Some(p) = fetch_tgftp(http, &s3, "p59hi").await {
        for (id, h) in parse_hail(&p.raw_text.unwrap_or_default()) {
            if let Some(&i) = by_id.get(&id) {
                storms[i].poh = h.poh;
                storms[i].posh = h.posh;
                storms[i].hail_in = h.hail_in;
            }
        }
    }

    let mut meso = Vec::new();
    if let Some(p) = fetch_latest(http, &s3, "NMD").await {
        let (lat0, lon0) = (p.lat as f64, p.lon as f64);
        for m in &p.meso {
            let (lon, lat) = offset_lonlat(lon0, lat0, m.x_km, m.y_km);
            meso.push(MesoFeat { lon, lat, kind: m.kind.clone() });
        }
    }
    merge_meso(storms, meso)
}

/// Great-circle-ish distance in km between two lon/lat points (flat-earth, fine at radar range).
fn dist_km(lon1: f64, lat1: f64, lon2: f64, lat2: f64) -> f64 {
    let dlat = (lat2 - lat1) * 111.32;
    let dlon = (lon2 - lon1) * 111.32 * lat1.to_radians().cos();
    (dlat * dlat + dlon * dlon).sqrt()
}

/// Index of the nearest storm within [`MERGE_KM`] of `(lon, lat)`, if any.
fn nearest_storm(storms: &[Cell], lon: f64, lat: f64) -> Option<usize> {
    storms
        .iter()
        .enumerate()
        .filter(|(_, c)| c.kind == CellKind::Storm)
        .map(|(i, c)| (i, dist_km(lon, lat, c.lon, c.lat)))
        .filter(|(_, d)| *d <= MERGE_KM)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// Fold meso/TVS detections into the nearest storm cell (within [`MERGE_KM`]); unmatched
/// detections become standalone markers. Pure so it can be unit-tested without the network.
fn merge_meso(mut storms: Vec<Cell>, meso: Vec<MesoFeat>) -> Vec<Cell> {
    for m in meso {
        let is_tvs = m.kind.contains("TVS");
        match nearest_storm(&storms, m.lon, m.lat) {
            Some(i) => {
                if is_tvs {
                    storms[i].tvs = Some(m.kind);
                } else {
                    storms[i].meso = Some(m.kind);
                }
            }
            None => {
                let mut c = Cell::new(CellKind::Meso, m.lon, m.lat, String::new(), m.kind.clone());
                if is_tvs {
                    c.tvs = Some(m.kind);
                } else {
                    c.meso = Some(m.kind);
                }
                storms.push(c);
            }
        }
    }
    storms
}

/// Radar-relative km east/north to lon/lat (small-offset flat-earth; fine at radar range).
fn offset_lonlat(lon0: f64, lat0: f64, x_km: f32, y_km: f32) -> (f64, f64) {
    let lat = lat0 + y_km as f64 / 111.32;
    let lon = lon0 + x_km as f64 / (111.32 * lat0.to_radians().cos().max(0.01));
    (lon, lat)
}

/// Compass azimuth (deg, 0 = north, clockwise) + range (nm) to lon/lat, relative to the radar.
fn azran_lonlat(lon0: f64, lat0: f64, az_deg: f32, range_nm: f32) -> (f64, f64) {
    let r_km = range_nm * 1.852;
    let a = (az_deg as f64).to_radians();
    let x_km = (r_km as f64 * a.sin()) as f32;
    let y_km = (r_km as f64 * a.cos()) as f32;
    offset_lonlat(lon0, lat0, x_km, y_km)
}

// --- Tabular / graphic parsers (token-walk, tolerant) -----------------------------------------

/// A whitespace field: a numeric pair (`294/ 83`), or a forecast-gap marker.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Field {
    Pair(f32, f32),
    NoData,
    New,
}

/// True if `tok` looks like a SCIT cell id (two alphanumerics with at least one letter+digit).
fn is_cell_id(tok: &str) -> bool {
    tok.len() == 2
        && tok.chars().all(|c| c.is_ascii_alphanumeric())
        && tok.chars().any(|c| c.is_ascii_digit())
        && tok.chars().any(|c| c.is_ascii_alphabetic())
}

/// Collapse `294/ 83` (any run of spaces after the slash) into `294/83` so slash pairs survive
/// whitespace tokenizing.
fn collapse_slashes(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        out.push(c);
        if c == '/' {
            while chars.peek() == Some(&' ') {
                chars.next();
            }
        }
    }
    out
}

/// Parse a collapsed line's whitespace tokens into ordered [`Field`]s (one per data column).
fn fields_of(line: &str) -> Vec<Field> {
    let mut out = Vec::new();
    for tok in collapse_slashes(line).split_whitespace() {
        if let Some((a, b)) = tok.split_once('/') {
            if let (Ok(a), Ok(b)) = (a.trim().parse::<f32>(), b.trim().parse::<f32>()) {
                out.push(Field::Pair(a, b));
                continue;
            }
        }
        match tok {
            "NEW" => out.push(Field::New),
            "DATA" => out.push(Field::NoData), // the leading "NO" is ignored
            _ => {}
        }
    }
    out
}

struct PosFcst {
    az: f32,
    range: f32,
    mvt_deg: f32,
    mvt_kt: f32,
    fcst: Vec<(u16, f32, f32)>,
    fcst_err: f32,
    mean_err: f32,
}

/// Parse the `STORM POSITION/FORECAST` table into per-cell current position, movement, forecast
/// track (15/30/45/60 min), and error metrics.
fn parse_position_forecast(tabular: &str) -> HashMap<String, PosFcst> {
    let mut out = HashMap::new();
    for line in tabular.lines() {
        let toks: Vec<&str> = line.split_whitespace().collect();
        // Rows look like: `P  O7  294/ 83  202/ 15  297/ 83 ...  0.5/ 0.6`.
        let id = toks.iter().find(|t| is_cell_id(t));
        let Some(&id) = id else { continue };
        let fields = fields_of(line);
        // cur, mvt, f15, f30, f45, f60, err = 7 columns.
        if fields.len() < 7 {
            continue;
        }
        let Field::Pair(az, range) = fields[0] else { continue };
        let Field::Pair(mvt_deg, mvt_kt) = fields[1] else { continue };
        let mut fcst = Vec::new();
        for (slot, min) in [15u16, 30, 45, 60].iter().enumerate() {
            if let Field::Pair(a, r) = fields[2 + slot] {
                fcst.push((*min, a, r));
            }
        }
        let Field::Pair(fcst_err, mean_err) = fields[6] else { continue };
        out.insert(id.to_string(), PosFcst { az, range, mvt_deg, mvt_kt, fcst, fcst_err, mean_err });
    }
    out
}

/// Parse the graphic `STORM ID` / `DBZM HGT` block into per-cell (max dBZ, height kft).
fn parse_graphic_attrs(graphic: &str) -> HashMap<String, (f32, f32)> {
    let mut out = HashMap::new();
    let mut pending: Vec<String> = Vec::new();
    for line in graphic.lines() {
        if line.contains("STORM ID") {
            pending = line.split_whitespace().filter(|t| is_cell_id(t)).map(String::from).collect();
        } else if line.contains("DBZM") {
            let nums: Vec<f32> = line
                .split_whitespace()
                .filter_map(|t| t.parse::<f32>().ok())
                .collect();
            for (i, id) in pending.iter().enumerate() {
                if let (Some(&d), Some(&h)) = (nums.get(i * 2), nums.get(i * 2 + 1)) {
                    out.insert(id.clone(), (d, h));
                }
            }
            pending.clear();
        }
    }
    out
}

struct StructAttr {
    base: Option<f32>,
    base_below: bool,
    top: Option<f32>,
    vil: Option<f32>,
    max_ref: Option<f32>,
}

/// Parse the `STORM STRUCTURE` table: per-cell base/top/VIL/max-ref (tolerating `< n.n` bases).
fn parse_storm_structure(tabular: &str) -> HashMap<String, StructAttr> {
    let mut out = HashMap::new();
    for line in tabular.lines() {
        let toks: Vec<&str> = line.split_whitespace().collect();
        if toks.first() != Some(&"P") || toks.get(1).map(|t| !is_cell_id(t)).unwrap_or(true) {
            continue;
        }
        // After id, skip the AZRAN pair; then base(<) top VIL maxref height.
        let mut nums: Vec<f32> = Vec::new();
        let mut base_below = false;
        let mut rest = toks[2..].iter().peekable();
        // Drop the AZRAN pair token(s) (`294/` `83` or `294/83`).
        if let Some(t) = rest.peek() {
            if t.contains('/') {
                let t = rest.next().unwrap();
                if !t.ends_with('/') {
                    // combined `294/83` — nothing more to drop
                } else {
                    rest.next(); // the bare range number
                }
            }
        }
        let mut want_below_num = false;
        for &t in rest {
            if t == "<" {
                if nums.is_empty() {
                    base_below = true;
                }
                want_below_num = true;
                continue;
            }
            let s = t.strip_prefix('<').inspect(|_| {
                if nums.is_empty() {
                    base_below = true;
                }
            });
            let s = s.unwrap_or(t);
            if let Ok(v) = s.parse::<f32>() {
                nums.push(v);
                want_below_num = false;
            } else if want_below_num {
                want_below_num = false;
            }
        }
        if nums.is_empty() {
            continue;
        }
        out.insert(
            toks[1].to_string(),
            StructAttr {
                base: nums.first().copied(),
                base_below,
                top: nums.get(1).copied(),
                vil: nums.get(2).copied(),
                max_ref: nums.get(3).copied(),
            },
        );
    }
    out
}

struct HailAttr {
    posh: Option<i32>,
    poh: Option<i32>,
    hail_in: Option<f32>,
}

/// Parse the `HAIL` table: per-cell POSH (severe %), POH (%), max expected size (in). Rows read
/// `P  ID  <posh>  <poh>  <size>`; `UNKNOWN` fields and `<` size prefixes are tolerated.
fn parse_hail(tabular: &str) -> HashMap<String, HailAttr> {
    let mut out = HashMap::new();
    for line in tabular.lines() {
        let toks: Vec<&str> = line.split_whitespace().collect();
        if toks.first() != Some(&"P") || toks.get(1).map(|t| !is_cell_id(t)).unwrap_or(true) {
            continue;
        }
        if toks.len() < 5 {
            continue;
        }
        let posh = toks[2].parse::<i32>().ok();
        let poh = toks[3].parse::<i32>().ok();
        let hail_in = toks[4].trim_start_matches('<').parse::<f32>().ok();
        // Skip rows with nothing parsable (e.g. all-UNKNOWN).
        if posh.is_none() && poh.is_none() && hail_in.is_none() {
            continue;
        }
        out.insert(toks[1].to_string(), HailAttr { posh, poh, hail_in });
    }
    out
}

// --- VAD wind profile (VWP, product 48) -------------------------------------------------------

/// One VAD wind-profile level (from the `VAD Algorithm Output` table).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VwpLevel {
    pub alt_kft: f32,
    /// East/north wind components (m/s) — hodograph coordinates.
    pub u_ms: f32,
    pub v_ms: f32,
    pub dir_deg: f32,
    pub speed_kt: f32,
    pub rms_kt: f32,
}

/// Parse the `VAD Algorithm Output` table: rows `P ALT U V W DIR SPD RMS DIV SRNG ELEV`.
/// `NA` fields keep the column count fixed, so U/V/DIR/SPD sit at stable token indices.
fn parse_vwp(tabular: &str) -> Vec<VwpLevel> {
    let mut out = Vec::new();
    for line in tabular.lines() {
        let t: Vec<&str> = line.split_whitespace().collect();
        if t.first() != Some(&"P") || t.len() < 11 {
            continue;
        }
        // Row leads with an integer altitude in 100s of feet; header rows ("ALT", "100ft") skip.
        let Ok(alt100) = t[1].parse::<f32>() else { continue };
        let (Ok(u), Ok(v)) = (t[2].parse::<f32>(), t[3].parse::<f32>()) else { continue };
        let (Ok(dir), Ok(spd)) = (t[5].parse::<f32>(), t[6].parse::<f32>()) else { continue };
        let rms = t[7].parse::<f32>().unwrap_or(f32::NAN);
        out.push(VwpLevel { alt_kft: alt100 * 0.1, u_ms: u, v_ms: v, dir_deg: dir, speed_kt: spd, rms_kt: rms });
    }
    out
}

/// Fetch the latest VAD wind profile (VWP) for `site` (tgftp `DS.48vwp`), sorted by altitude.
pub async fn fetch_vwp(http: &reqwest::Client, site: &str) -> Vec<VwpLevel> {
    let s3 = l3_site(site);
    let Some(p) = fetch_tgftp(http, &s3, "48vwp").await else { return Vec::new() };
    let mut levels = parse_vwp(&p.raw_text.unwrap_or_default());
    levels.sort_by(|a, b| a.alt_kft.total_cmp(&b.alt_kft));
    levels.dedup_by(|a, b| (a.alt_kft - b.alt_kft).abs() < 0.01);
    levels
}

// --- Fetch helpers ----------------------------------------------------------------------------

/// List the latest object for `SITE_PRODUCT` today (with a yesterday fallback) and decode it.
async fn fetch_latest(http: &reqwest::Client, site: &str, product: &str) -> Option<Level3Product> {
    let today = chrono::Utc::now().date_naive();
    for day in [today, today.pred_opt().unwrap_or(today)] {
        let prefix = format!("{site}_{product}_{}", day.format("%Y_%m_%d"));
        let url = format!("{BUCKET}/?list-type=2&prefix={prefix}");
        let Ok(resp) = http.get(&url).send().await else { continue };
        let Ok(xml) = resp.text().await else { continue };
        if let Some(key) = last_key(&xml) {
            let obj_url = format!("{BUCKET}/{key}");
            if let Ok(resp) = http.get(&obj_url).send().await {
                if let Ok(bytes) = resp.bytes().await {
                    match decode(&bytes) {
                        Ok(p) => return Some(p),
                        Err(e) => log::warn!("level3 decode {key}: {e}"),
                    }
                }
            }
        }
    }
    None
}

/// Fetch the latest `DS.{ds}` product for `site` from the tgftp `sn.last` feed and decode it.
/// `ds` is the directory suffix (`p59hi`, `p62ss`); `site` is the 3-letter L3 id.
async fn fetch_tgftp(http: &reqwest::Client, site: &str, ds: &str) -> Option<Level3Product> {
    let url = format!("{TGFTP}/DS.{ds}/SI.k{}/sn.last", site.to_lowercase());
    let bytes = http.get(&url).send().await.ok()?.bytes().await.ok()?;
    match decode(&bytes) {
        Ok(p) => Some(p),
        Err(e) => {
            log::warn!("level3 tgftp decode {ds} {site}: {e}");
            None
        }
    }
}

/// The last `<Key>` in an S3 list-objects-v2 XML response (keys sort ascending, so last =
/// newest for the timestamped naming).
fn last_key(xml: &str) -> Option<String> {
    xml.rmatch_indices("<Key>").next().and_then(|(i, _)| {
        let rest = &xml[i + 5..];
        rest.find("</Key>").map(|e| rest[..e].to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn site_id_strips_k() {
        assert_eq!(l3_site("KTLX"), "TLX");
        assert_eq!(l3_site("PACG"), "PACG");
        assert_eq!(l3_site("TJUA"), "TJUA");
    }

    #[test]
    fn last_key_picks_newest() {
        let xml = "<x><Key>TLX_NST_2020_03_31_00_02_54</Key><Key>TLX_NST_2020_03_31_00_39_35</Key></x>";
        assert_eq!(last_key(xml).unwrap(), "TLX_NST_2020_03_31_00_39_35");
    }

    const POS_FCST: &str = "\
P  O7     294/ 83   202/ 15     297/ 83   299/ 84   302/ 84   304/ 85    0.5/ 0.6
P  E5     296/ 89   317/  9     295/ 87   295/ 85   NO DATA   NO DATA    1.1/ 0.7
P  M4     215/134    30/ 14     215/137   NO DATA   NO DATA   NO DATA    1.9/ 1.9
P  ID     AZRAN     MOVEMENT    15 MIN    30 MIN    45 MIN    60 MIN    FCST/MEAN";

    #[test]
    fn position_forecast_parses() {
        let m = parse_position_forecast(POS_FCST);
        let o7 = m.get("O7").expect("O7");
        assert_eq!(o7.az, 294.0);
        assert_eq!(o7.range, 83.0);
        assert_eq!(o7.mvt_deg, 202.0);
        assert_eq!(o7.mvt_kt, 15.0);
        assert_eq!(o7.fcst.len(), 4);
        assert_eq!(o7.fcst[0], (15, 297.0, 83.0));
        assert_eq!(o7.fcst[3], (60, 304.0, 85.0));
        assert_eq!(o7.fcst_err, 0.5);
        assert_eq!(o7.mean_err, 0.6);
        // E5 has two NO DATA forecast slots.
        assert_eq!(m.get("E5").unwrap().fcst.len(), 2);
        // M4 has only the 15-min slot.
        assert_eq!(m.get("M4").unwrap().fcst.len(), 1);
        // The header row is not a cell.
        assert!(!m.contains_key("ID"));
    }

    const GRAPHIC: &str = "\
 STORM ID        O7        E5        D8
) DBZM HGT   63  9.7   60 17.8   62 11.1";

    #[test]
    fn graphic_attrs_parse() {
        let m = parse_graphic_attrs(GRAPHIC);
        assert_eq!(m.get("O7"), Some(&(63.0, 9.7)));
        assert_eq!(m.get("E5"), Some(&(60.0, 17.8)));
        assert_eq!(m.get("D8"), Some(&(62.0, 11.1)));
    }

    const STRUCTURE: &str = "\
P     O7      294/ 83    < 9.7    31.7          43             63        9.7
P     B4      305/109    <14.7    18.9           2             37       14.7
P   STORM      AZRAN      BASE     TOP    CELL BASED VIL    MAX REF    HEIGHT";

    #[test]
    fn structure_parses_below_base() {
        let m = parse_storm_structure(STRUCTURE);
        let o7 = m.get("O7").expect("O7");
        assert_eq!(o7.base, Some(9.7));
        assert!(o7.base_below, "'<' base marker");
        assert_eq!(o7.top, Some(31.7));
        assert_eq!(o7.vil, Some(43.0));
        assert_eq!(o7.max_ref, Some(63.0));
        assert_eq!(m.get("B4").unwrap().base, Some(14.7));
        assert!(!m.contains_key("STORM"));
    }

    const HAIL: &str = "\
P        O7               0                  100                 0.50
P        D8               0                   70                <0.50
P        J9         UNKNOWN              UNKNOWN              UNKNOWN";

    #[test]
    fn hail_parses() {
        let m = parse_hail(HAIL);
        let o7 = m.get("O7").expect("O7");
        assert_eq!(o7.posh, Some(0));
        assert_eq!(o7.poh, Some(100));
        assert_eq!(o7.hail_in, Some(0.50));
        assert_eq!(m.get("D8").unwrap().hail_in, Some(0.50)); // '<' stripped
        assert!(!m.contains_key("J9"), "all-UNKNOWN row skipped");
    }

    const VWP: &str = "\
P    ALT      U       V       W    DIR   SPD   RMS     DIV     SRNG    ELEV
P   100ft    m/s     m/s    cm/s   deg   kts   kts    E-3/s     nm      deg
P    016    -2.5     7.3     NA    161   015   6.7      NA      5.67    0.5
P    068     9.6    12.1    -6.9   218   030   2.2   -0.0378   16.20    3.1";

    #[test]
    fn vwp_parses_levels_and_skips_headers() {
        let v = parse_vwp(VWP);
        assert_eq!(v.len(), 2);
        assert!((v[0].alt_kft - 1.6).abs() < 1e-4);
        assert_eq!(v[0].u_ms, -2.5);
        assert_eq!(v[0].v_ms, 7.3);
        assert_eq!(v[0].dir_deg, 161.0);
        assert_eq!(v[0].speed_kt, 15.0);
        assert_eq!(v[0].rms_kt, 6.7);
        assert_eq!(v[1].dir_deg, 218.0); // W='-6.9' present, columns still line up
    }

    #[test]
    fn azran_places_north_and_east() {
        // Radar at equator-ish; 0° az → due north, 90° → due east.
        let (lon_n, lat_n) = azran_lonlat(-97.0, 35.0, 0.0, 54.0); // 54 nm ~ 100 km
        assert!(lat_n > 35.0 && (lon_n + 97.0).abs() < 0.01, "north");
        let (lon_e, lat_e) = azran_lonlat(-97.0, 35.0, 90.0, 54.0);
        assert!(lon_e > -97.0 && (lat_e - 35.0).abs() < 0.01, "east");
    }

    #[test]
    fn meso_merges_tvs_into_nearby_cell() {
        let mut storm = Cell::new(CellKind::Storm, -97.5, 35.5, "A1".into(), "A1".into());
        storm.max_dbz = Some(60.0);
        let meso = vec![
            MesoFeat { lon: -97.49, lat: 35.5, kind: "TVS".into() },
            MesoFeat { lon: -96.0, lat: 35.5, kind: "Mesocyclone".into() }, // far → standalone
        ];
        let out = merge_meso(vec![storm], meso);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].tvs.as_deref(), Some("TVS"));
        assert_eq!(out[1].kind, CellKind::Meso);
    }
}
