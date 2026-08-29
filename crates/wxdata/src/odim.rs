//! ODIM_H5 polar volumes — the format every European weather service speaks.
//!
//! ODIM_H5 is OPERA's exchange format: an HDF5 file with one group per elevation (`dataset1`,
//! `dataset2`, …), one group per moment inside it (`data1`, `data2`, …), and all the metadata in
//! `what`/`where`/`how` group attributes. Gates are stored packed, `value = gain × raw + offset`,
//! with two reserved raw levels: `undetect` for "looked, saw nothing" and `nodata` for "didn't
//! look". Decoding it lands on the same [`Scan`] the NEXRAD path produces, so rendering, palettes,
//! SRV and thresholds treat a European radar exactly like a WSR-88D.
//!
//! [`crate::dwd`] is the live consumer: `opendata.dwd.de` publishes single-sweep ODIM files
//! without registration, one per elevation, and hands the bytes here. Other European volumes are
//! harder to reach — Environment Canada publishes only rendered GIF/CAPPI imagery on Datamart and
//! keeps its ODIM volumes behind HTTP basic auth, and OPERA's own archive is licensed.
//!
//! # Azimuth registration
//!
//! Rays are decoded in file order, azimuth `(i + 0.5) × 360/nrays`, and `a1gate` is deliberately
//! ignored. ODIM's `a1gate` names the ray radiated *first in time*, not the ray pointing north:
//! DWD files carry values like 55 or 247 while `/dataset<n>/how/startazA[0]` is ~0.01°, so ray 0
//! already starts at north. Rotating the sweep by `a1gate` would misregister it by that many
//! degrees. Verified against live DWD volumes; do not "fix" this without checking `startazA`.

use chrono::{DateTime, NaiveDate, NaiveTime, TimeZone, Utc};
use hdf5lite::Value;
use nexrad_model::data::{
    MomentData, PulseWidth, Radial, RadialStatus, Scan, Sweep, VolumeCoveragePattern,
};
use nexrad_model::meta::Site;
use std::collections::HashMap;

/// Attributes of one HDF5 group, the shape ODIM metadata always arrives in.
type Attrs = HashMap<String, Value>;

fn num(a: &Attrs, key: &str) -> Option<f64> {
    a.get(key).and_then(Value::as_f64)
}

fn text<'a>(a: &'a Attrs, key: &str) -> Option<&'a str> {
    a.get(key).and_then(Value::as_str)
}

/// Parse ODIM's split date and time attributes (`20130429`, `043000`) into one instant.
fn timestamp(a: &Attrs, date_key: &str, time_key: &str) -> Option<DateTime<Utc>> {
    let d = NaiveDate::parse_from_str(text(a, date_key)?, "%Y%m%d").ok()?;
    let t = NaiveTime::parse_from_str(text(a, time_key)?, "%H%M%S").ok()?;
    Utc.from_utc_datetime(&d.and_time(t)).into()
}

/// Which [`Radial`] slot an ODIM `quantity` belongs in.
///
/// Unlisted quantities (`SQI`, `SNR`, the quality layers, dual-PRF diagnostics) are dropped: the
/// model has nowhere to put them and carrying them as a wrong moment would be worse than nothing.
fn moment_slot(quantity: &str) -> Option<usize> {
    Some(match quantity {
        "DBZH" | "DBZ" | "TH" | "DBZV" | "TV" => 0,
        "VRAD" | "VRADH" | "VRADV" | "VRADDH" => 1,
        "WRAD" | "WRADH" => 2,
        "ZDR" => 3,
        "PHIDP" | "UPHIDP" | "KDP" => 4,
        "RHOHV" => 5,
        _ => return None,
    })
}

/// One moment of one sweep, already packed into the byte layout [`MomentData`] wants.
struct Moment {
    slot: usize,
    /// `nrays × nbins` gates, big-endian when `word_bits` is 16.
    raw: Vec<u8>,
    scale: f32,
    offset: f32,
    word_bits: u8,
}

/// Decode an ODIM_H5 polar volume into a [`Scan`], with the volume's nominal start time.
///
/// Sweeps come out in file order, which ODIM requires to be ascending elevation.
pub fn decode(bytes: Vec<u8>) -> anyhow::Result<(DateTime<Utc>, Scan)> {
    let f = hdf5lite::File::open(bytes).map_err(|e| anyhow::anyhow!("odim: {e}"))?;

    let root_what = f.attributes("what").map_err(|e| anyhow::anyhow!("{e}"))?;
    let object = text(&root_what, "object").unwrap_or_default();
    if object != "PVOL" && object != "SCAN" && object != "AZIM" {
        anyhow::bail!("odim: {object:?} is not a polar volume");
    }
    let time = timestamp(&root_what, "date", "time")
        .ok_or_else(|| anyhow::anyhow!("odim: no usable /what/date and /what/time"))?;
    let root_where = f.attributes("where").map_err(|e| anyhow::anyhow!("{e}"))?;

    // Sweep groups are `dataset<n>`, and `dataset10` must not sort before `dataset2`.
    let mut groups: Vec<(u32, String)> = f
        .names()
        .iter()
        .filter_map(|n| Some((n.strip_prefix("dataset")?.parse().ok()?, n.to_string())))
        .collect();
    groups.sort_unstable();

    let mut sweeps = Vec::new();
    for (i, (_, group)) in groups.iter().enumerate() {
        match sweep(&f, group, i as u8 + 1) {
            Ok(Some(s)) => sweeps.push(s),
            Ok(None) => {}
            // One unreadable elevation shouldn't cost the whole volume.
            Err(e) => log::warn!("odim: skipping {group}: {e}"),
        }
    }
    if sweeps.is_empty() {
        anyhow::bail!("odim: no decodable sweeps");
    }

    let site = Site::new(
        site_identifier(text(&root_what, "source").unwrap_or_default()),
        num(&root_where, "lat").unwrap_or(0.0) as f32,
        num(&root_where, "lon").unwrap_or(0.0) as f32,
        num(&root_where, "height").unwrap_or(0.0) as i16,
        // ODIM's `height` is already the antenna height above sea level; there is no separate
        // tower figure to report.
        0,
    );
    // ODIM carries no VCP: the scan strategy is a free-text `how/task`. Pattern 0 says so rather
    // than borrowing a NEXRAD number that would imply elevations this radar never scanned.
    let vcp = VolumeCoveragePattern::new(
        0,
        1,
        0.5,
        PulseWidth::Short,
        false,
        0,
        false,
        0,
        false,
        false,
        0,
        false,
        false,
        Vec::new(),
    );
    Ok((time, Scan::with_site(site, vcp, sweeps)))
}

/// The four-letter id from an ODIM `source` string, which is comma-separated `KEY:value` pairs.
///
/// The model's identifier is four bytes, so the five-character `NOD` code (`bewid`) is truncated
/// to `BEWI`. That is lossy but stable and legible; the full source string stays in the file.
fn site_identifier(source: &str) -> [u8; 4] {
    let pick = |key: &str| {
        source
            .split(',')
            .find_map(|p| p.strip_prefix(key))
            .map(str::trim)
            .filter(|s| !s.is_empty())
    };
    let id = pick("NOD:").or_else(|| pick("RAD:")).unwrap_or("????");
    let mut out = [b'?'; 4];
    for (o, c) in out.iter_mut().zip(id.bytes()) {
        *o = c.to_ascii_uppercase();
    }
    out
}

/// Build one sweep from a `dataset<n>` group, or `None` when it holds no moment we can use.
fn sweep(f: &hdf5lite::File, group: &str, elevation_number: u8) -> anyhow::Result<Option<Sweep>> {
    let attrs = |p: String| f.attributes(&p).map_err(|e| anyhow::anyhow!("{e}"));
    let w = attrs(format!("{group}/where"))?;
    let (nrays, nbins) = (
        num(&w, "nrays").unwrap_or(0.0) as usize,
        num(&w, "nbins").unwrap_or(0.0) as usize,
    );
    let elevation = num(&w, "elangle").unwrap_or(0.0) as f32;
    let rscale = num(&w, "rscale").unwrap_or(0.0);
    // `rstart` is the one ODIM range field in kilometres, not metres.
    let rstart_m = num(&w, "rstart").unwrap_or(0.0) * 1000.0;
    if nrays == 0 || nbins == 0 || rscale <= 0.0 {
        anyhow::bail!("{group}: nrays={nrays} nbins={nbins} rscale={rscale}");
    }
    // Gate ranges in the model are to the gate's centre; ODIM's are to its near edge.
    let first_gate = (rstart_m + rscale / 2.0) as u16;
    let gate_interval = rscale as u16;

    let what = attrs(format!("{group}/what")).unwrap_or_default();
    let start = timestamp(&what, "startdate", "starttime");

    let mut moments: Vec<Moment> = Vec::new();
    for child in f.children(group).map_err(|e| anyhow::anyhow!("{e}"))? {
        if !child.starts_with("data") || child.starts_with("dataset") {
            continue;
        }
        match moment(f, &format!("{group}/{child}"), nrays * nbins) {
            Ok(Some(m)) => moments.push(m),
            Ok(None) => {}
            Err(e) => log::warn!("odim: skipping {group}/{child}: {e}"),
        }
    }
    if moments.is_empty() {
        return Ok(None);
    }

    let spacing = 360.0 / nrays as f32;
    let radials = (0..nrays)
        .map(|i| {
            let mut slots: [Option<MomentData>; 6] = Default::default();
            for m in &moments {
                let stride = nbins * (m.word_bits as usize / 8);
                let Some(gates) = m.raw.get(i * stride..(i + 1) * stride) else {
                    continue;
                };
                slots[m.slot] = Some(MomentData::from_fixed_point(
                    nbins as u16,
                    first_gate,
                    gate_interval,
                    m.word_bits,
                    m.scale,
                    m.offset,
                    gates.to_vec(),
                ));
            }
            let [refl, vel, sw, zdr, phi, rho] = slots;
            Radial::new(
                start.map(|t| t.timestamp_millis()).unwrap_or(0),
                i as u16,
                // Ray `i` spans one azimuth step starting at `i × spacing`; the model wants its
                // centre.
                (i as f32 + 0.5) * spacing,
                spacing,
                RadialStatus::ScanStart,
                elevation_number,
                elevation,
                refl,
                vel,
                sw,
                zdr,
                phi,
                rho,
                None,
            )
        })
        .collect();
    Ok(Some(Sweep::new(elevation_number, radials)))
}

/// Read one `data<n>` group into packed gate bytes plus the scale and offset that invert
/// ODIM's packing.
fn moment(f: &hdf5lite::File, path: &str, gates: usize) -> anyhow::Result<Option<Moment>> {
    let what = f
        .attributes(&format!("{path}/what"))
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let quantity = text(&what, "quantity").unwrap_or_default();
    let Some(slot) = moment_slot(quantity) else {
        return Ok(None);
    };

    let data = format!("{path}/data");
    let ds = f.dataset(&data).map_err(|e| anyhow::anyhow!("{e}"))?;
    let word_bits = match ds.dtype.size_of() {
        1 => 8u8,
        2 => 16,
        n => anyhow::bail!("{path}: {n}-byte gates"),
    };
    let values = f.read_f64(&data).map_err(|e| anyhow::anyhow!("{e}"))?;
    if values.len() < gates {
        anyhow::bail!("{path}: {} gates, expected {gates}", values.len());
    }

    // The model decodes a gate as `(raw − offset) / scale`; ODIM packs it as `gain × raw + off`.
    // Equating the two gives `scale = 1/gain` and `offset = −off/gain`.
    let gain = num(&what, "gain").unwrap_or(1.0);
    if gain == 0.0 {
        anyhow::bail!("{path}: zero gain");
    }
    let (scale, offset) = (1.0 / gain, -num(&what, "offset").unwrap_or(0.0) / gain);

    // Raw 0 means below threshold to the model, which is exactly ODIM's `undetect`. `nodata` has
    // no separate representation, so it collapses onto the same level.
    //
    // ponytail: this also claims raw 1 as "range folded" for every moment, so a genuine gate at
    // level 1 reads as folded. At the usual DBZH packing that level is −31.5 dBZ, below anything
    // that renders. Give ODIM its own gate status if a quantity ever needs level 1 back.
    let undetect = num(&what, "undetect");
    let nodata = num(&what, "nodata");
    let mut raw = Vec::with_capacity(gates * word_bits as usize / 8);
    for &v in &values[..gates] {
        let v = if Some(v) == undetect || Some(v) == nodata || v.is_nan() {
            0
        } else {
            v as u32
        };
        if word_bits == 8 {
            raw.push(v as u8);
        } else {
            raw.extend_from_slice(&(v as u16).to_be_bytes());
        }
    }
    Ok(Some(Moment {
        slot,
        raw,
        scale: scale as f32,
        offset: offset as f32,
        word_bits,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexrad_model::data::{DataMoment, MomentValue};

    /// The same Belgian volume hdf5lite's goldens are built from — five sweeps of DBZH.
    fn fixture() -> Vec<u8> {
        let p = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../hdf5lite/tests/data/bewid.h5"
        );
        std::fs::read(p).expect("bewid.h5 fixture")
    }

    #[test]
    fn a_real_volume_decodes_to_a_scan() {
        let (time, scan) = decode(fixture()).expect("decode");
        assert_eq!(time.to_rfc3339(), "2013-04-29T04:30:00+00:00");
        assert_eq!(scan.site().unwrap().identifier_string(), "BEWI");
        assert!((scan.site().unwrap().latitude() - 49.9143).abs() < 1e-4);

        // Five elevations, ascending, exactly as the file lists them.
        let elevations: Vec<f32> = scan
            .sweeps()
            .iter()
            .map(|s| s.radials()[0].elevation_angle_degrees())
            .collect();
        assert_eq!(elevations, vec![0.3, 0.9, 1.8, 3.3, 6.0]);

        let sweep = &scan.sweeps()[0];
        assert_eq!(sweep.radials().len(), 360);
        let r = &sweep.radials()[0];
        assert_eq!(r.azimuth_angle_degrees(), 0.5);
        assert_eq!(r.azimuth_spacing_degrees(), 1.0);
        assert_eq!(r.collection_timestamp(), 1_367_209_800_000);
        assert!(r.velocity().is_none(), "the file carries DBZH only");
    }

    /// gain 0.5 / offset −32 must come back out as dBZ, and the reserved levels as gate status.
    #[test]
    fn gain_and_offset_invert_to_the_original_dbz() {
        let (_, scan) = decode(fixture()).expect("decode");
        let refl = scan.sweeps()[0].radials()[0]
            .reflectivity()
            .expect("reflectivity");
        assert_eq!(refl.gate_count(), 960);
        assert_eq!(refl.gate_interval_km(), 0.25);
        assert_eq!(refl.first_gate_range_km(), 0.125);

        // Raw gates 0,0,0,0,0,0,42,40 from the file: undetect, then 0.5×42−32 and 0.5×40−32.
        let v = refl.values();
        assert_eq!(v[0], MomentValue::BelowThreshold);
        assert_eq!(v[6], MomentValue::Value(-11.0));
        assert_eq!(v[7], MomentValue::Value(-12.0));

        // No gate may decode above the 95 dBZ the packing tops out at, which is what a scale or
        // offset applied the wrong way round would produce.
        for sweep in scan.sweeps() {
            for radial in sweep.radials() {
                for value in radial.reflectivity().unwrap().iter() {
                    if let MomentValue::Value(dbz) = value {
                        assert!((-32.0..=96.0).contains(&dbz), "{dbz} dBZ out of range");
                    }
                }
            }
        }
    }

    #[test]
    fn junk_is_rejected_rather_than_guessed_at() {
        assert!(decode(Vec::new()).is_err());
        assert!(decode(vec![0u8; 4096]).is_err());
        let mut truncated = fixture();
        truncated.truncate(2048);
        assert!(decode(truncated).is_err());
    }
}
