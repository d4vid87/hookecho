//! Benchmarks for the per-volume hot paths: the derived-product integrals and the sweep binning
//! every product goes through.
//!
//! Synthetic sweeps, because no Level 2 volume is committed to this repo (they are tens of MB).
//! That makes these a regression tripwire on the *algorithms* — a change that doubles the cost of
//! `derive` shows up here — rather than a claim about any real storm. Point
//! `HOOKECHO_BENCH_VOLUME` at a downloaded volume to bench decode + bin on real data.
//!
//!   cargo bench -p wxdata

use criterion::{criterion_group, criterion_main, Criterion};
use wxdata::derived::DerivedOpts;
use wxdata::level2::{BinnedSweep, Moment};

/// A full ladder of uniform reflectivity tilts — same shape the derived tests use, at a volume's
/// worth of azimuths and gates.
fn sweeps(dbz: f32) -> Vec<BinnedSweep> {
    const ELEVS: [f32; 9] = [0.5, 1.5, 2.4, 3.4, 4.3, 6.0, 9.9, 14.6, 19.5];
    ELEVS
        .iter()
        .map(|&e| BinnedSweep {
            moment: Moment::Reflectivity,
            az_bins: 720,
            gate_count: 400,
            data: vec![((dbz + 32.0) / 127.0 * 253.0 + 2.0) as u8; 720 * 400],
            first_gate_km: 2.0,
            gate_interval_km: 0.25,
            radar_lat: 35.0,
            radar_lon: -97.0,
            elevation_deg: e,
            value_min: -32.0,
            value_max: 95.0,
        })
        .collect()
}

fn bench_derived(c: &mut Criterion) {
    let s = sweeps(45.0);
    let opts = DerivedOpts {
        etop_dbz: 18.0,
        time: chrono::Utc::now(),
    };
    let mut g = c.benchmark_group("derived");
    g.sample_size(10);
    g.bench_function("derive", |b| {
        b.iter(|| wxdata::derived::derive(std::hint::black_box(&s), &opts))
    });
    g.bench_function("hail", |b| {
        b.iter(|| wxdata::derived::hail(std::hint::black_box(&s), 4.0, 7.5, &opts))
    });
    g.finish();
}

fn bench_dualpol(c: &mut Criterion) {
    let z = sweeps(45.0);
    let mut cc = sweeps(45.0);
    for s in &mut cc {
        s.moment = Moment::CorrelationCoefficient;
        s.value_min = 0.0;
        s.value_max = 1.05;
    }
    let mut g = c.benchmark_group("dualpol");
    g.sample_size(10);
    g.bench_function("tbss", |b| {
        b.iter(|| wxdata::dualpol::tbss(&z[0], &cc[0], 60.0, 20.0, 0.8, 4.0, 150.0))
    });
    g.bench_function("zdr_columns", |b| {
        b.iter(|| wxdata::dualpol::zdr_columns(&z, &z, 3.0, 1.0, 1.0, 40.0, 100.0))
    });
    g.bench_function("bright_band", |b| {
        b.iter(|| wxdata::dualpol::bright_band(&cc, &z, 6.0))
    });
    g.finish();
}

/// Decode + bin a real volume, when one is pointed at: `HOOKECHO_BENCH_VOLUME=/path/to/volume`.
/// Silently skipped otherwise — this repo commits no Level 2 fixture.
fn bench_real_volume(c: &mut Criterion) {
    let Ok(path) = std::env::var("HOOKECHO_BENCH_VOLUME") else {
        return;
    };
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("HOOKECHO_BENCH_VOLUME={path} is not readable; skipping");
        return;
    };
    let mut g = c.benchmark_group("volume");
    g.sample_size(10);
    g.bench_function("decode", |b| {
        b.iter(|| wxdata::level2::decode_volume(std::hint::black_box(bytes.clone())))
    });
    if let Ok(scan) = wxdata::level2::decode_volume(bytes) {
        g.bench_function("bin_sweep", |b| {
            b.iter(|| wxdata::level2::bin_scan(&scan, Moment::Reflectivity, 0))
        });
    }
    g.finish();
}

criterion_group!(benches, bench_derived, bench_dualpol, bench_real_volume);
criterion_main!(benches);
