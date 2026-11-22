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

## Done when

- [ ] All eight tests exist and pass.
- [ ] Your report includes timings for generating one Chunk and a full Active Region in a release build.

## Scope boundary

This step ends with a world that can be generated and simulated. Showing it on screen starts in step 09.
