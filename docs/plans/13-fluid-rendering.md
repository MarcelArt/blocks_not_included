# Step 13: Liquids and gases on screen

> Read `docs/plans/CONVENTIONS.md` and `docs/plans/BEVY-0.20-NOTES.md` first.

**Prerequisites:** step 12 is done.

## Goal

Liquids are drawn as translucent volumes whose surface height shows how full each Cell is. Gases whose Element is marked as visible get a faint tint. Every other gas stays invisible until step 14's Overlays.

## Design

- **Liquid mesh.** Each Chunk gets a second, translucent mesh.
  - A liquid Cell's top face sits at a height of `Mass / capacity`, clamped to between 0.05 and 1, unless the Cell above holds the same liquid.
  - Side faces are drawn wherever the neighbouring Cell is neither the same liquid nor solid.
  - The colour is the Element's colour at about 0.6 alpha, with `AlphaMode::Blend`.
  - If faces overlapping inside one mesh look wrong, try `Material::enable_oit`.
- **Gas tint.**
  - Add a `tint: f32` field to `Element`, ranging from 0 to 1 with a default of 0. Validate the range in `sim`, and give it a builder method and a test in `crates/sim/tests/elements.rs`.
  - Set Steam's tint to 0.15 in `assets/elements.ron`; every other gas stays at 0.
  - A gas with tint above 0 is drawn as translucent faces wherever it borders a different Element, using its colour with alpha equal to `tint`.
- Rebuild a Chunk's translucent meshes using the same change rules as solids (step 10).

### Decisions made during implementation

- **Modules.**
  - `src/render/fluids.rs` holds the pure functions: `FluidLooks`, `mesh_fluids` and `FluidGeometry` (a liquid and a gas `ChunkGeometry`).
  - `ChunkNeighbourhood::cell` in `src/render/mesher.rs` looks up a Cell up to one step outside the Chunk, in a face neighbour.
  - `src/render/test_support.rs` holds what both meshers' tests share: the test Elements, worlds built Cell by Cell, and `quads_of`, which checks winding.
- **Layers.**
  - `ChunkMesh { pos, layer: ChunkLayer }` replaces step 10's `SolidMesh(ChunkPos)`. A Chunk has up to three entities: `Solid`, `Liquid` and `Gas`. Each layer shares one material across all Chunks.
  - A single meshing task builds all three layers, so fluids follow step 10's change rules exactly. A layer with no faces has no entity.
- **Liquid mesh.**
  - Faces are per Cell, not greedy-merged, because surface heights differ from Cell to Cell.
  - The surface sits at Mass / Capacity, clamped to 0.05–1, and at 1 under the same liquid. Capacity is Density × 1 m³, the same formula as the palette's.
  - **Step faces.** Against the same liquid, the part of a side that rises above the neighbour's surface still shows. Without it, a spreading Cell standing above its neighbours shows a hole down its side. Test 3 holds for neighbours at the same level.
  - A bottom face shows where the Cell below is neither the same liquid nor solid, as under falling Water.
  - A full Cell's top is left out under a solid. It would lie flush with the solid's underside and flicker against it when seen from below.
  - A Cell the neighbourhood doesn't hold counts as Vacuum: one in a missing Chunk, or diagonally past an edge, such as the Cell above a neighbour across the top edge.
- **Gas mesh.** Faces show toward every different Element except solids. A face against a solid points into it, so it could only be seen from inside the solid, and it would flicker against the solid's face.
- **Materials.** Both are `StandardMaterial` with a white base and `AlphaMode::Blend`, so the vertex alpha sets the opacity. Both have `cull_mode: None` and `double_sided: true`, so a surface shows from beneath and a cloud of Steam shows from inside.
  - Liquid: lit, with `perceptual_roughness` 0.2 and `reflectance` 0.3.
  - Gas: `unlit`, so a haze looks the same from every side instead of having shaded sides.
  - Fluid entities get `NotShadowCaster`: in 0.20, Blend meshes cast shadows unless told not to.
  - OIT wasn't needed. Faces that overlap within one mesh share a colour, so their blend order doesn't show.
- **Performance.**
  - `ElementId::index` is now `#[inline]`. Both meshers call it for every Cell from another crate, and the game crate is built at opt-level 1, so without the hint every call was a real one.
  - `ChunkNeighbourhood::cell` takes a shortcut for Cells within the Chunk.
  - **Meshing cost.** Fluid meshing averages 0.08 ms per generated Chunk, against 0.26 ms for solids. In the running game, where the meshing threads share the CPU, it is 0.3 ms against 0.76 ms. Each remesh costs about 40% more.
  - **Frame time.** Measured by alternating step-12 and step-13 builds, because a busy Steam client and laptop heat made runs drift. During steady play, the mean frame went from 8.1–8.6 ms to 9.5–10.7 ms. Frames over 16.7 ms went from about 5 to about 30 per 10 s.
    - Hiding the fluid entities changed nothing. Skipping fluid meshing brought the baseline back. So the cost is meshing CPU competing with the main and render threads, not drawing.
  - **Ideas that didn't help.** A bitmask in place of `Vec<bool>`, `#[inline]` on the mesher's helpers, and a shortcut for Cells buried inside a pocket all measured no faster, so they were dropped.

## Seams and tests

Pure mesher functions, with unit tests:

1. A half-full liquid Cell has its top face at y + 0.5.
2. A liquid Cell under the same liquid has no top face.
3. Two neighbouring Cells of the same liquid share no side face.
4. A liquid Cell next to a solid has no side face on that side.
5. Gas with tint 0 produces no geometry.
6. `elements.rs`: a tint outside 0 to 1 is rejected.

Added at the same seams:

- In `src/render/fluids.rs`:
  - a surface is drawn between 5% and full height (1 g, and an over-Capacity 1,200 kg);
  - a step down to the same liquid shows the side above the lower surface;
  - a full Cell under a solid has no top, but a half-full one does;
  - a liquid Cell over gas shows its bottom;
  - a tinted gas shows faces toward a different Element that isn't solid, in its colour at its tint;
  - faces at the Chunk border follow the neighbour Chunk, which counts as Vacuum when missing.
- In `crates/sim/tests/elements.rs`:
  - test 6 also checks NaN, the builder method, and that 0 and 1 are accepted;
  - Steam is the only tinted Element in the shipped file, at 0.15.

Every quad test also checks each triangle's winding.

## Done when

- [x] The tests pass: 11 in `src/render/fluids.rs`, and 2 more in `crates/sim/tests/elements.rs`.
- [x] **Human check:**
  - A Water pocket looks like a pool, and a placed Water Cell visibly spreads into a thin, level layer.
  - Boiling Water shows faint white Steam drifting away.
  - The agent checked these with a temporary harness, since removed. It placed Water and heated it through `SimBridge`, held the camera still, and took screenshots.
  - **Spreading.** 1,000 kg of Water placed on the ground at (5, −2, 5) was a cross of uneven columns after 0.3 s. By 9 s it was a thin, level sheet across the flat ground. 10 s after placing, it covered 68–76 Cells, with all 1,000,000 g within 8 m.
  - **Boiling.** A 3 × 3 patch of Water at 365 K, heated at 100 K/s, boiled within about 0.5 s. Faint white Steam rose and spread out. Higher up, some Steam cooled and condensed back into thin Water Cells that fell like rain: that is the simulation working as designed. Boiling 9 t of Water made about 21,000 Cells of Steam.
  - **Pool.** The nearest pool, Water two or more Cells deep with open sky above it, was in a large cave at (38, −21, −15). The Water shows as translucent blue volumes, but the cave around it is a flat, featureless grey. Faces in shade are all lit by the same ambient light, and step 10 deferred ambient occlusion. Whether it "looks like a pool" is for the human to judge.
  - The human hasn't checked it yet.

### Deferred

- **Underwater.** Only a liquid's faces are tinted. With the eyes in Water, the surface above is tinted, but rock walls seen through the Water aren't. Tinting the view while the eyes are in a liquid would fix this.
- **Remeshing what didn't change.** Solid geometry depends only on which Cells are solid, which rarely changes. Yet every flow of liquid remeshes the Chunk's solids too, at 0.76 ms in the running game. Skipping a layer whose inputs haven't changed would likely more than pay back the fluids' cost, but it hasn't been measured.
  - A whole-Chunk check would help less: in the running game, 79% of Chunk changes alter an Element or a liquid's Mass.
- **Steam opacity.** Every Steam Cell shows at the same tint, whatever its Mass, so 1 g of Steam looks as dense as a full Cell.

## Scope boundary

This step ends with fluids on screen. Overlays and the crosshair readout come in step 14.
