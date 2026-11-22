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

## Seams and tests

- **Seam:** `Simulation`, plus the benchmark.
- **New test**, in `crates/worldgen/tests/determinism.rs`: run the busy scenario for 20 Ticks with 1 thread and with 4 threads, and compare every loaded Cell. They must be identical.
- Every existing test keeps passing unchanged.

## Done when

- [ ] `cargo bench -p worldgen` runs both scenarios.
- [ ] Both targets are met, or measurements and a bottleneck analysis are reported.
- [ ] The determinism test passes.
- [ ] The README status row for step 08 includes the measured numbers.

## Scope boundary

This step ends at a fast simulation. Running it on a thread inside the game comes in step 09.
