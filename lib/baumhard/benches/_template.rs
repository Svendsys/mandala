// SPDX-License-Identifier: MPL-2.0

use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn run_bench() {
   
}

fn criterion_benchmark(c: &mut Criterion) {
   c.bench_function("template", |b| b.iter(|| run_bench()));
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
