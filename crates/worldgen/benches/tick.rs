//! How long one Tick takes over a full Active Region (4 × 2 × 4 Chunks, about 1M Cells), at the
//! default thread count. Run it with:
//!
//! ```text
//! cargo bench -p worldgen
//! ```
//!
//! - **busy:** a freshly generated region with hot spots and gas releases in every Chunk, so
//!   nearly every Chunk is awake. It is generated again every [`BUSY_TICKS`] Ticks, so it stays
//!   busy however many Ticks are measured.
//! - **settled:** the same region as generated, without hot spots or gas releases, after 300
//!   Ticks. It never comes entirely to rest in a practical time: most of its Chunks stay awake
//!   while Water and gas level out a few grams at a time (see `tests/common/mod.rs`).
//! - **at_rest:** a region of nothing but still Atmosphere, in which every Chunk is Settled. It
//!   shows what a Tick costs once a region really has come to rest.
//!
//! Only the Ticks are timed, never the generation or the run-up. Targets on the development
//! laptop: busy ≤ 100 ms, settled ≤ 10 ms (`docs/plans/08-performance.md`).

#[path = "../tests/common/mod.rs"]
mod common;

use std::time::{Duration, Instant};

use criterion::{Criterion, SamplingMode, criterion_group, criterion_main};
use sim::Simulation;

/// How many Ticks the busy scenario runs before it is generated again.
const BUSY_TICKS: u64 = 40;

fn busy(c: &mut Criterion) {
    let elements = common::shipped_elements();
    let mut sim = common::busy(&elements);
    let mut ticks_run = 0;
    let mut group = c.benchmark_group("tick");
    group
        .sampling_mode(SamplingMode::Flat)
        .sample_size(10)
        .measurement_time(Duration::from_secs(30));
    group.bench_function("busy", |b| {
        b.iter_custom(|iters| {
            let mut timed = Duration::ZERO;
            for _ in 0..iters {
                if ticks_run == BUSY_TICKS {
                    sim = common::busy(&elements);
                    ticks_run = 0;
                }
                timed += time_tick(&mut sim);
                ticks_run += 1;
            }
            timed
        });
    });
    group.finish();
}

fn settled(c: &mut Criterion) {
    let elements = common::shipped_elements();
    bench_steady(c, "settled", common::settled(&elements));
    bench_steady(c, "at_rest", common::at_rest(&elements));
}

/// Times Ticks of `sim`, which stays much as it is from one Tick to the next.
fn bench_steady(c: &mut Criterion, name: &str, mut sim: Simulation) {
    let mut group = c.benchmark_group("tick");
    group
        .sample_size(20)
        .measurement_time(Duration::from_secs(10));
    group.bench_function(name, |b| {
        b.iter_custom(|iters| (0..iters).map(|_| time_tick(&mut sim)).sum());
    });
    group.finish();
}

fn time_tick(sim: &mut Simulation) -> Duration {
    let start = Instant::now();
    sim.tick();
    start.elapsed()
}

criterion_group!(benches, busy, settled);
criterion_main!(benches);
