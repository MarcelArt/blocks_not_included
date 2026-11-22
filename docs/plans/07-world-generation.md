# Step 07: World generation

> Read `docs/plans/CONVENTIONS.md` first.

**Prerequisites:** step 06 is done (`ChunkSource` and world limits).

## Goal

A new Bevy-free crate, `crates/worldgen`, generates the Sim Sandbox world one Chunk at a time. The output is deterministic per seed and Chunk, and it is already settled, so a newly generated Chunk does not churn.

## Design

- **Crate.** `crates/worldgen` depends on `sim` and one noise crate. Choose a maintained one, such as `noise` or `fastnoise-lite`, and confirm its latest version. Add the crate to the workspace members.
- **Generator.** `pub struct Generator` implements `sim::ChunkSource`. `Generator::new(seed: u64, elements: &ElementTable) -> Result<Generator, …>` looks up the Elements it needs by name and returns an error naming any that are missing.
- **Determinism.** A Chunk's Cells depend only on the seed and its `ChunkPos`. There is no shared mutable state, and generation order has no effect.
- **Layout.** These values come from the design session; make them named constants.
  - **Surface:** a 2D noise heightmap of rolling hills around y = 0, varying by about ±40 Cells.
  - **Atmosphere:** above the surface, Oxygen at 1,430 g per Cell (Density × 1 m³) and 293 K.
  - **Ground:** a 3–5 Cell layer of Dirt on top, Granite below.
  - **Caves:** 3D noise carves caves below the surface. They are filled with Oxygen at Atmosphere Mass and the local temperature.
  - **Temperature:** rises with depth, from about 288 K at the surface to about 1500 K at `WORLD_BOTTOM_Y`, roughly 1.2 K per Cell.
  - **Floor:** the bottom Chunk layer is Magma.
  - **Pockets:** scattered underground blobs from 3D noise, of Water, Carbon Dioxide and Hydrogen.
- **Settled at generation.** Choose the intended Element, then follow its Phase Transitions for the local temperature until it is stable. For example, a deep "Water" pocket comes out as Steam, and Granite near the floor comes out as Magma.
- **Masses.** Generated solids and liquids are full (Density × 1 m³). Generated gases hold Density × 1 m³ too, which is roughly one atmosphere.

### Decisions made during implementation

- **Noise crate:** `fastnoise-lite` 1.1.1 with its `f64` feature, so noise coordinates stay exact far from the origin. Upstream FastNoiseLite was last pushed in October 2026; `noise` 0.9.0's repository had nothing since February 2025.
- **Temperature** is measured from the *local* surface: 288 K + 1.2 K × depth. The Atmosphere is a flat 293 K. Even under the lowest surface, the floor layer is at least 953 Cells deep, so about 1432 K or more, safely above Magma's 1400 K freezing point.
- **What comes out of the Phase Transitions:** Water pockets deeper than 70 Cells (373.2 K at 71) come out as Steam. Granite deeper than 1010 Cells comes out as Magma, just above the floor layer under the higher ground. Mass follows the Element a Cell ends up as, so a Steam pocket holds 600 g, not Water's 1,000,000 g. The code calls this *following* Phase Transitions, not settling, so it doesn't clash with **Settled Chunk**.
- **Which rule wins** below the surface: the floor first, then caves, then the Dirt layer, then pockets. Pockets replace only Granite. Caves can cut into the Dirt, but they fade out within 16 Cells of the surface, so few break through to the Atmosphere.
- **Shapes** (all in `crates/worldgen/src/layout.rs`):
  - Surface: 3 octaves of OpenSimplex2 at 1/256, × 40 Cells. Over 16,384 sampled columns it spanned y = −37 to 34.
  - Caves: 2 octaves at 1/48 above 0.4, flattened 2×. About 12 % of the ground.
  - Pockets: blobs of 1/16 noise above 0.7, with a 1/64 noise choosing Water, Carbon Dioxide or Hydrogen. About 1.6 % of the ground each.
- **Not entirely still.** No Phase Transition happens on the first Tick (test 8), but a pocket that touches a cave still flows: Water drains in and gases displace. Heat also creeps up the temperature gradient.
- **New public items:** `worldgen::Generator` and `worldgen::MissingElements`, which lists the missing names in `names`. There is also an example, `cargo run --release -p worldgen --example generation_timings`.
- **Release timings on this machine:** one Chunk takes 0.25 ms in the sky, 0.61 ms at the surface, 4.6 ms underground, and 0.23 ms in the floor layer. A full 4 × 2 × 4 Active Region takes 75 ms around the surface and 139 ms underground. The cost is the 3D noise: about three OpenSimplex2 samples per underground Cell, for caves and pockets.

## Seams and tests

**Seam:** `Generator`, through `ChunkSource::generate`, plus a `Simulation` built with it. Put the tests in `crates/worldgen/tests/generation.rs`, in this order:

1. The same seed and `ChunkPos` give identical Cells, and different seeds give different Cells.
2. Generating a set of Chunks in two different orders gives identical results.
3. A Chunk well above the surface is all Oxygen at 1,430 g and about 293 K.
4. Going down, the average temperature of solid Cells rises with every Chunk layer.
5. The bottom Chunk layer is Magma.
6. Every generated Cell lies within its Element's Phase Transition range. Sample many Chunks.
7. The invariants hold: Vacuum Cells have zero Mass, other Cells hold at least 1 g, and liquids are within capacity.
8. On the first Tick of a freshly generated, fully active region, no Phase Transition happens.

Added during implementation, at the same seam:

- Underground there are Oxygen caves and pockets of Water, Steam, Carbon Dioxide and Hydrogen.
- `Generator::new` names every Element the layout needs that the table lacks.

## Done when

- [x] All eight tests exist and pass.
- [x] Your report includes timings for generating one Chunk and a full Active Region in a release build.

## Scope boundary

This step ends with a world that can be generated and simulated. Showing it on screen starts in step 09.
