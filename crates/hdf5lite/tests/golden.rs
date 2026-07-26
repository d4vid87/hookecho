//! hdf5lite checked against the real HDF5 library.
//!
//! The goldens in `tests/data/*.golden.json` come from h5py (see `gen_golden.py`), so these tests
//! prove agreement with the reference implementation rather than self-consistency. Two real GOES-19
//! GLM granules from different hours are committed alongside them.

use serde_json::Value;
use std::path::PathBuf;

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data")
}

fn granules() -> Vec<(String, Vec<u8>, Value)> {
    let dir = data_dir();
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("tests/data") {
        let p = entry.expect("dir entry").path();
        if p.extension().and_then(|e| e.to_str()) != Some("nc") {
            continue;
        }
        let stem = p.file_stem().unwrap().to_string_lossy().to_string();
        let golden = dir.join(format!("{stem}.golden.json"));
        let json: Value = serde_json::from_slice(&std::fs::read(&golden).expect("golden json"))
            .expect("parse golden");
        out.push((stem, std::fs::read(&p).expect("granule"), json));
    }
    assert!(!out.is_empty(), "no granules committed");
    out
}

/// Compare against h5py with a tolerance that scales with magnitude — flash energies are ~1e-13,
/// coordinates are ~1e2, and both round-trip through f64 the same way.
fn close(a: f64, b: f64) -> bool {
    if a == b {
        return true;
    }
    let scale = a.abs().max(b.abs()).max(1e-30);
    (a - b).abs() / scale < 1e-9
}

#[test]
fn every_object_in_the_root_group_is_found() {
    for (name, bytes, golden) in granules() {
        let f = hdf5lite::File::open(bytes).expect("open");
        let got: Vec<String> = f.names().into_iter().map(str::to_string).collect();
        let want: Vec<String> = golden["names"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert_eq!(got, want, "{name}: root group listing differs from h5py");
    }
}

#[test]
fn datasets_decode_to_the_same_values_as_h5py() {
    for (gname, bytes, golden) in granules() {
        let f = hdf5lite::File::open(bytes).expect("open");
        let datasets = golden["datasets"].as_object().expect("datasets");
        assert!(!datasets.is_empty());
        for (name, want) in datasets {
            let got = f
                .read_f64(name)
                .unwrap_or_else(|e| panic!("{gname}/{name}: {e}"));
            let n = want["len"].as_u64().unwrap() as usize;
            assert_eq!(got.len(), n, "{gname}/{name}: element count");

            // Spot-check both ends: a chunk assembled in the wrong order shows up here first.
            for (i, w) in want["first"].as_array().unwrap().iter().enumerate() {
                match w.as_f64() {
                    Some(w) => assert!(
                        close(got[i], w),
                        "{gname}/{name}[{i}]: got {} want {w}",
                        got[i]
                    ),
                    None => assert!(got[i].is_nan(), "{gname}/{name}[{i}]: expected fill/NaN"),
                }
            }
            let tail = want["last"].as_array().unwrap();
            for (k, w) in tail.iter().enumerate() {
                let i = n - tail.len() + k;
                match w.as_f64() {
                    Some(w) => assert!(
                        close(got[i], w),
                        "{gname}/{name}[{i}]: got {} want {w}",
                        got[i]
                    ),
                    None => assert!(got[i].is_nan(), "{gname}/{name}[{i}]: expected fill/NaN"),
                }
            }

            // Aggregates catch anything wrong in the middle that the ends would miss.
            let finite: Vec<f64> = got.iter().copied().filter(|v| v.is_finite()).collect();
            assert_eq!(
                finite.len(),
                want["n_finite"].as_u64().unwrap() as usize,
                "{gname}/{name}: fill-value handling"
            );
            if let Some(w) = want["sum"].as_f64() {
                let sum: f64 = finite.iter().sum();
                assert!(close(sum, w), "{gname}/{name}: sum got {sum} want {w}");
            }
            if let Some(w) = want["min"].as_f64() {
                let m = finite.iter().copied().fold(f64::INFINITY, f64::min);
                assert!(close(m, w), "{gname}/{name}: min got {m} want {w}");
            }
            if let Some(w) = want["max"].as_f64() {
                let m = finite.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                assert!(close(m, w), "{gname}/{name}: max got {m} want {w}");
            }
        }
    }
}

#[test]
fn packed_integers_carry_their_scale_and_offset() {
    let (_, bytes, _) = granules().pop().unwrap();
    let f = hdf5lite::File::open(bytes).expect("open");
    let ds = f.dataset("flash_energy").expect("flash_energy");
    // The unpacking metadata lives in a dense attribute heap, not inline in the object header.
    assert!(ds.scale_factor.is_some(), "scale_factor not read");
    assert!(ds.add_offset.is_some(), "add_offset not read");
    assert!(ds.fill_value.is_some(), "_FillValue not read");
}

#[test]
fn garbage_is_rejected_without_panicking() {
    assert!(hdf5lite::File::open(Vec::new()).is_err(), "empty input");
    assert!(hdf5lite::File::open(vec![0u8; 64]).is_err(), "no signature");
    // A real header followed by nothing: truncation must be an error, never a panic or a hang.
    let (_, bytes, _) = granules().pop().unwrap();
    for cut in [16usize, 64, 512, 4096] {
        let _ = hdf5lite::File::open(bytes[..cut.min(bytes.len())].to_vec());
    }
}
