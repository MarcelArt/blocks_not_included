# Step 08: Performance

> Read `docs/plans/CONVENTIONS.md` first, then `docs/adr/0003-bevy-free-background-sim.md`.

**Prerequisites:** step 07 is done, since realistic worlds are needed to measure against.

## Goal

A Tick over a full Active Region (about 1M Cells) fits comfortably inside the 200 ms Tick budget, and a benchmark proves it.

## Design

- **Benchmark.** Use `criterion`, in `crates/worldgen/benches/tick.rs`. It lives in `worldgen` so it can use `Generator` without a dev-dependency cycle on `sim`. Two scenarios:
  - **busy:** a freshly generated, fully active region, plus scattered hot spots and gas releases so most Chunks are awake.
  - **settled:** the same region after it comes to rest.
- **Parallel Tick.** Split each phenomenon's work across Chunks on a thread pool. If you use rayon, size it explicitly, for example to 3 threads. The game's Bevy pools share this 8-thread laptop (see `docs/plans/BEVY-0.20-NOTES.md`, "Threads and tasks"). Results must be identical whatever the thread count.
- **Freedom inside the crate.** You may change data layout (for example structure-of-arrays), avoid per-Tick allocations, and skip all-Vacuum or all-solid Chunks cheaply. The public interface and every existing test must stay unchanged.
- **Targets.** Release build on this laptop: **busy ≤ 100 ms per Tick, settled ≤ 10 ms per Tick.** If a target is out of reach, stop. Report the measurements and the bottleneck, and leave the simulation rules as they are.

### Decisions made during implementation

- **Threads.** `rayon` 1.12. Each `Simulation` owns a pool, started by its first Tick and sized by the new `Simulation::set_threads` (default 3; `Simulation::threads` reads it). Every pass plans its moves one task per Chunk, reading only, then carries them out one task per Chunk, each changing only its own Cells. Results are gathered in the order of the Chunks' positions (`crates/sim/src/grid.rs`). Displacement still settles its candidates one at a time, because its pseudo-random order chains decisions across the region, but everything it needs is prepared in parallel.
- **The rules are unchanged, bit for bit.** Before the rewrite, every Cell was hashed after each of 300 busy Ticks. The new code gives the same hashes at 1 and at 4 threads. Heat keeps each Cell's f32 additions in their old order, and `apply` mixes each Cell's arrivals in the order of where they come from.
- **What made it fast**, from about 1.5 s per busy Tick:
  - Cells are found through a flat box of Chunks instead of a `HashMap` lookup per Cell, and a Cell in the Chunk being planned is read directly.
  - Passes no longer build a list of every Cell's position.
  - Face counts are only worked out where something might move.
  - Layer swaps only look above Cells that something could sink through.
  - Displacement doesn't allocate per candidate.
  - `apply` sorts the moves into Chunks in parallel, then groups them by Cell with a counting sort.
- **Scenarios** (`crates/worldgen/tests/common/mod.rs`, shared by the benchmark and the determinism test). Both use seed 2026, the default 4 × 2 × 4 Active Region around the origin, and the game's planned `conduction_multiplier` of 1000 (step 09).
  - **busy:** the region as generated. At four spots in every Chunk, a 3 × 3 × 3 block of ground is warmed by 1500 K, or a block of Atmosphere is replaced by Carbon Dioxide or Hydrogen at 20 kg a Cell. The benchmark generates it again every 40 Ticks; all 32 Chunks stay awake.
  - **settled:** the region as generated, without hot spots or releases, after 300 Ticks. See below for why it isn't at rest.
  - **at_rest:** an extra case. It is a region high in the Atmosphere where every Chunk is Settled, and shows what a Tick costs once a region really rests.
- **Measured** with `cargo bench -p worldgen` on this laptop (i5-10300H), release build, 3 threads:

  | Scenario | Per Tick | Target |
  |---|---|---|
  | busy | 75.6 ms | ≤ 100 ms ✅ |
  | settled | 44.1 ms | ≤ 10 ms ❌ |
  | at_rest | 0.07 ms | — |

  Over 300 busy Ticks: 1.5 s per Tick before; now 190 ms on 1 thread and 67 ms on 4.
- **Why settled misses, and the bottleneck.** The generated region never comes to rest in a practical time.
  - In a 4,000-Tick run at default settings, about 20 of its 32 Chunks were still awake at the end.
  - Each Tick, about 2,400 Oxygen Cells and 1,900 Water Cells still changed Mass. This is Water draining from pockets and levelling out across cave floors and the region's sealed floor, and gas evening out through the caves, a few grams at a time.
  - Settling works per Chunk, so a Chunk with a single changing Cell costs as much as a busy one: every pass reads all 32,768 of its Cells. At 0.07 ms for a region that does rest, the Settled machinery itself is cheap.
  - The rules are left as they are. Ways forward, for the human to choose:
    1. Skip Cells inside awake Chunks whose neighbourhood hasn't changed since they last moved nothing. This keeps results identical, so work follows activity rather than Chunks.
    2. Change the rules so that levelling stops sooner.
    3. Accept 44 ms, which is well inside the 200 ms Tick.
- **New public items:** `Simulation::set_threads` and `Simulation::threads`. Nothing else in the public interface changed.

## Seams and tests

- **Seam:** `Simulation`, plus the benchmark.
- **New test**, in `crates/worldgen/tests/determinism.rs`: run the busy scenario for 20 Ticks with 1 thread and with 4 threads, and compare every loaded Cell. They must be identical.
- Every existing test keeps passing unchanged.

## Done when

- [x] `cargo bench -p worldgen` runs both scenarios.
- [x] Both targets are met, or measurements and a bottleneck analysis are reported. Busy is met; settled is not, see above.
- [x] The determinism test passes.
- [x] The README status row for step 08 includes the measured numbers.

## Scope boundary

This step ends at a fast simulation. Running it on a thread inside the game comes in step 09.
