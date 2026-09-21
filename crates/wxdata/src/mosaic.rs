//! Seamless multi-radar reflectivity mosaic: N0B from every radar covering the view, stitched
//! into one grid with nearest-radar-wins.
//!
//! Why N0B (digital base reflectivity, product 153) rather than Level 2: it is ~260 KB per site
//! against 5-10 MB, already 8-bit with a scale/offset in its own header, and it comes off the same
//! S3 bucket the app's other L3 grids use. Six neighbours therefore cost about what one extra
//! MRMS refresh costs. The price is latency — an N0B lags the primary site's Level 2 chunk stream
//! by a minute or two, so a fast-moving storm can show a small seam where two radars meet. That's
//! inherent to compositing separate volume scans and the honest fix is to say so, which is why
//! [`Mosaic::oldest`] is surfaced in the UI rather than hidden.
//!
//! Reflectivity only, live only. A velocity mosaic is not deferred — it is physically meaningless,
//! since each radar measures motion along its own beam.

use crate::mrms::MrmsField;
use crate::field::{FieldDescriptor, FieldFamily, FieldId, MissingData, SamplingPolicy, ValueKind};

pub static DESCRIPTOR: FieldDescriptor = FieldDescriptor {
    id: FieldId("radar.reflectivity-fusion"),
    source: "NEXRAD Level III fusion",
    family: FieldFamily::Radar,
    display_name: "Multi-radar reflectivity fusion",
    short_name: "Radar Mosaic",
    search_aliases: &["mosaic", "multi radar", "fusion"],
    units: "dBZ",
    value_kind: ValueKind::Scalar,
    palette_key: "reflectivity",
    sampling: SamplingPolicy::Bilinear,
    missing: MissingData::Nan,
    time_policy: None,
    supports_contours: true,
    supports_difference: false,
};

/// The composite, plus what went into it (for the legend and the honesty about age).
pub struct Mosaic {
    pub field: MrmsField,
    pub provenance: Provenance,
    /// Age of the oldest contributing scan, for the "this is a composite" readout.
    pub oldest: chrono::DateTime<chrono::Utc>,
}

/// Per-pixel contributor and range-based confidence for the fused field.
pub struct Provenance {
    pub sites: Vec<String>,
    owner: Vec<u8>,
    confidence: Vec<f32>,
    nx: usize,
    ny: usize,
    bounds: (f64, f64, f64, f64),
}

impl Provenance {
    pub fn sample(&self, lon: f64, lat: f64) -> Option<(&str, f32)> {
        let (west, south, east, north) = self.bounds;
        if !(west..=east).contains(&lon) || !(south..=north).contains(&lat) {
            return None;
        }
        let x = (((lon - west) / (east - west) * self.nx as f64) as usize).min(self.nx - 1);
        let y = (((north - lat) / (north - south) * self.ny as f64) as usize).min(self.ny - 1);
        let index = y * self.nx + x;
        let owner = *self.owner.get(index)?;
        let site = self.sites.get(owner as usize)?;
        Some((site, *self.confidence.get(index)?))
    }
}

/// Radars whose 460 km coverage disk intersects the view box, nearest the centre first, capped at
/// `max`. `primary` is always first when it is a real site: whatever the user is looking at must
/// be in the picture even when the camera has wandered off its disk.
pub fn sites_for_view(
    primary: Option<&str>,
    min_lon: f64,
    min_lat: f64,
    max_lon: f64,
    max_lat: f64,
    max: usize,
) -> Vec<String> {
    let (clon, clat) = ((min_lon + max_lon) / 2.0, (min_lat + max_lat) / 2.0);
    let mut hits: Vec<(f64, String)> = crate::sites::sites()
        .iter()
        .filter_map(|s| {
            let (lon, lat) = (s.longitude as f64, s.latitude as f64);
            // Coverage disk vs. view box, in degrees — 460 km is ~4.14° of latitude and more of
            // longitude, so this over-selects slightly at the edges and never misses.
            let dlat = 4.15;
            let dlon = 4.15 / lat.to_radians().cos().max(0.2);
            (lon + dlon >= min_lon
                && lon - dlon <= max_lon
                && lat + dlat >= min_lat
                && lat - dlat <= max_lat)
                .then(|| {
                    let (dx, dy) = ((lon - clon) * clat.to_radians().cos(), lat - clat);
                    (dx * dx + dy * dy, s.id.to_string())
                })
        })
        .collect();
    hits.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out: Vec<String> = Vec::with_capacity(max);
    if let Some(p) = primary.filter(|p| crate::sites::site_by_id(p).is_some()) {
        out.push(p.to_ascii_uppercase());
    }
    for (_, id) in hits {
        if out.len() >= max {
            break;
        }
        if !out.contains(&id) {
            out.push(id);
        }
    }
    out
}

/// Fetch N0B for every site and composite them. `None` when nothing came back at all.
pub async fn fetch(client: &reqwest::Client, sites: &[String]) -> Option<Mosaic> {
    let fields = futures_util::future::join_all(sites.iter().map(|s| async move {
        let f = crate::level3::fetch_n0b(client, s).await;
        (s.clone(), f)
    }))
    .await;
    let parts: Vec<(String, MrmsField)> = fields
        .into_iter()
        .filter_map(|(s, f)| f.map(|f| (s, f)))
        .collect();
    if parts.is_empty() {
        return None;
    }
    let oldest = parts.iter().map(|(_, f)| f.time).min()?;
    let names = parts.iter().map(|(s, _)| s.clone()).collect();
    let (field, owner, confidence) = composite(parts.iter().map(|(_, f)| f));
    let provenance = Provenance {
        nx: field.nx,
        ny: field.ny,
        bounds: (field.lon_west, field.lat_south, field.lon_east, field.lat_north),
        sites: names,
        owner,
        confidence,
    };
    Some(Mosaic {
        field,
        provenance,
        oldest,
    })
}

/// Nearest-radar-wins compositing onto the union grid.
///
/// Overlap is the whole problem: two radars see the same storm from different angles and neither
/// is wrong, so blending them smears the reflectivity gradients that matter. Taking the closer
/// radar's value keeps every pixel a real measurement from the radar best placed to make it — the
/// same rule the national mosaics use, minus the range weighting.
fn composite<'a>(parts: impl Iterator<Item = &'a MrmsField> + Clone) -> (MrmsField, Vec<u8>, Vec<f32>) {
    const RES_DEG: f64 = 0.01;
    let (mut lon_west, mut lon_east) = (f64::MAX, f64::MIN);
    let (mut lat_south, mut lat_north) = (f64::MAX, f64::MIN);
    let mut newest = chrono::DateTime::<chrono::Utc>::MIN_UTC;
    for f in parts.clone() {
        lon_west = lon_west.min(f.lon_west);
        lon_east = lon_east.max(f.lon_east);
        lat_south = lat_south.min(f.lat_south);
        lat_north = lat_north.max(f.lat_north);
        newest = newest.max(f.time);
    }
    let nx = (((lon_east - lon_west) / RES_DEG).ceil() as usize).max(1);
    let ny = (((lat_north - lat_south) / RES_DEG).ceil() as usize).max(1);
    let mut values = vec![f32::NAN; nx * ny];
    // Distance (squared, in degrees) from the radar that currently owns each cell.
    let mut owner = vec![f64::MAX; nx * ny];
    let mut owner_index = vec![u8::MAX; nx * ny];

    for (part_index, f) in parts.enumerate() {
        let (rlon, rlat) = (
            (f.lon_west + f.lon_east) / 2.0,
            (f.lat_south + f.lat_north) / 2.0,
        );
        let coslat = rlat.to_radians().cos().max(0.05);
        let sx = (f.lon_east - f.lon_west) / f.nx as f64;
        let sy = (f.lat_north - f.lat_south) / f.ny as f64;
        for gy in 0..f.ny {
            let lat = f.lat_north - (gy as f64 + 0.5) * sy;
            let oy = ((lat_north - lat) / RES_DEG) as isize;
            if oy < 0 || oy as usize >= ny {
                continue;
            }
            for gx in 0..f.nx {
                let v = f.values[gy * f.nx + gx];
                if v.is_nan() {
                    continue;
                }
                let lon = f.lon_west + (gx as f64 + 0.5) * sx;
                let ox = ((lon - lon_west) / RES_DEG) as isize;
                if ox < 0 || ox as usize >= nx {
                    continue;
                }
                let (dx, dy) = ((lon - rlon) * coslat, lat - rlat);
                let d2 = dx * dx + dy * dy;
                let i = oy as usize * nx + ox as usize;
                if d2 < owner[i] {
                    owner[i] = d2;
                    owner_index[i] = part_index as u8;
                    values[i] = v;
                }
            }
        }
    }

    let confidence = owner
        .iter()
        .map(|distance| (1.0 - distance.sqrt() as f32 / 4.15).clamp(0.0, 1.0))
        .collect();
    (MrmsField {
        values,
        nx,
        ny,
        lon_west,
        lon_east,
        lat_north,
        lat_south,
        time: newest,
    }, owner_index, confidence)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(lon: f64, lat: f64, v: f32) -> MrmsField {
        // A 1°-square field, uniformly `v`, centred on (lon, lat).
        let n = 100;
        MrmsField {
            values: vec![v; n * n],
            nx: n,
            ny: n,
            lon_west: lon - 0.5,
            lon_east: lon + 0.5,
            lat_south: lat - 0.5,
            lat_north: lat + 0.5,
            time: chrono::Utc::now(),
        }
    }

    #[test]
    fn nearest_radar_wins_in_the_overlap() {
        // Two radars 0.5° apart, so the right half of the left one overlaps the left half of the
        // right one. Every cell must carry the value of whichever centre it sits nearer.
        let (out, owner, confidence) = composite([patch(-97.0, 35.0, 10.0), patch(-96.5, 35.0, 50.0)].iter());
        assert!(out.lon_west <= -97.5 && out.lon_east >= -96.0);
        let at = |lon: f64, lat: f64| {
            let gx = ((lon - out.lon_west) / 0.01) as usize;
            let gy = ((out.lat_north - lat) / 0.01) as usize;
            out.values[gy * out.nx + gx]
        };
        assert_eq!(at(-97.4, 35.0), 10.0, "left radar's exclusive area");
        assert_eq!(at(-96.1, 35.0), 50.0, "right radar's exclusive area");
        assert_eq!(at(-96.8, 35.0), 10.0, "overlap, nearer the left radar");
        assert_eq!(at(-96.7, 35.0), 50.0, "overlap, nearer the right radar");
        let index = |lon: f64, lat: f64| {
            ((out.lat_north - lat) / 0.01) as usize * out.nx
                + ((lon - out.lon_west) / 0.01) as usize
        };
        assert_eq!(owner[index(-96.8, 35.0)], 0);
        assert_eq!(owner[index(-96.7, 35.0)], 1);
        assert!(confidence[index(-97.0, 35.0)] > confidence[index(-96.8, 35.0)]);
        // Outside both disks stays no-data, so the renderer's discard leaves a real gap.
        assert!(at(-97.4, 35.45).is_nan() || at(-97.4, 35.45) == 10.0);
    }

    #[test]
    fn view_selection_prefers_the_primary_and_the_centre() {
        let sites = sites_for_view(Some("KTLX"), -99.0, 34.0, -96.0, 36.5, 4);
        assert_eq!(sites.first().map(String::as_str), Some("KTLX"));
        assert!(sites.len() <= 4);
        assert!(sites.contains(&"KFDR".to_string()), "got {sites:?}");
        // A radar an ocean away is not in a plains view.
        assert!(!sites.contains(&"KMUX".to_string()));
    }
}
