//! Times world generation. Run it in a release build:
//!
//! ```text
//! cargo run --release -p worldgen --example generation_timings
//! ```

use std::time::{Duration, Instant};

use sim::{CellPos, ChunkPos, ChunkSource, ElementTable, Simulation, WORLD_BOTTOM_Y};
use worldgen::Generator;

const SEED: u64 = 2026;
const CHUNK_RUNS: u32 = 50;
const REGION_RUNS: u32 = 5;

fn main() {
    let elements = ElementTable::from_ron(include_str!("../../../assets/elements.ron"))
        .expect("the shipped Elements file is valid");
    let generator = Generator::new(SEED, &elements).expect("the shipped Elements are enough");

    println!("One Chunk, mean of {CHUNK_RUNS} runs:");
    for (label, chunk) in [
        ("sky (all Atmosphere)", ChunkPos::new(0, 4, 0)),
        ("surface", ChunkPos::new(0, 0, 0)),
        ("underground", ChunkPos::new(0, -10, 0)),
        (
            "floor (all Magma)",
            ChunkPos::new(0, WORLD_BOTTOM_Y / 32, 0),
        ),
    ] {
        let mean = mean_of(CHUNK_RUNS, || {
            std::hint::black_box(generator.generate(chunk));
        });
        println!("  {label:<22} {:>7.2} ms", millis(mean));
    }

    println!("A full Active Region (4 × 2 × 4 Chunks), mean of {REGION_RUNS} runs:");
    // `with_source` generates the Active Region around the origin, across the surface.
    let mean = mean_of_with_setup(
        REGION_RUNS,
        || (elements.clone(), Generator::new(SEED, &elements).unwrap()),
        |(elements, generator)| {
            std::hint::black_box(Simulation::with_source(elements, Box::new(generator)));
        },
    );
    println!("  {:<22} {:>7.2} ms", "around the surface", millis(mean));
    // Moving the focus far down generates a whole new Active Region underground.
    let mean = mean_of_with_setup(
        REGION_RUNS,
        || {
            Simulation::with_source(
                elements.clone(),
                Box::new(Generator::new(SEED, &elements).unwrap()),
            )
        },
        |mut sim| {
            sim.focus_on(CellPos::new(0, -500, 0));
            std::hint::black_box(sim);
        },
    );
    println!("  {:<22} {:>7.2} ms", "underground", millis(mean));
}

fn mean_of(runs: u32, mut work: impl FnMut()) -> Duration {
    mean_of_with_setup(runs, || (), |()| work())
}

/// The mean time `work` takes, each run on a fresh value from `setup`, which isn't timed.
fn mean_of_with_setup<T>(
    runs: u32,
    mut setup: impl FnMut() -> T,
    mut work: impl FnMut(T),
) -> Duration {
    let mut total = Duration::ZERO;
    for _ in 0..runs {
        let input = setup();
        let start = Instant::now();
        work(input);
        total += start.elapsed();
    }
    total / runs
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}
