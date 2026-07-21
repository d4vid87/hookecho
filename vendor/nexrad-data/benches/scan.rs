use criterion::{criterion_group, criterion_main, Criterion};
use nexrad_data::volume;
use std::hint::black_box;

const TEST_NEXRAD_FILE: &[u8] = include_bytes!("../../downloads/KDMX20220305_232324_V06");

fn benchmark_scan(c: &mut Criterion) {
    let volume = volume::File::new(TEST_NEXRAD_FILE.to_vec());

    c.bench_function("scan", |b| {
        b.iter(|| {
            let scan = volume.scan().expect("scan succeeds");
            black_box(scan);
        })
    });
}

criterion_group!(benches, benchmark_scan);
criterion_main!(benches);
