# Step 10: Rendering solid Cells

> Read `docs/plans/CONVENTIONS.md` and `docs/plans/BEVY-0.20-NOTES.md` first.

**Prerequisites:** step 09 is done (`WorldView`, plus the per-frame record of changed Chunks).

## Goal

Every loaded Chunk's solid Cells are drawn as flat-coloured cubes, built with `binary-greedy-meshing`, and are remeshed off the main thread whenever they change.

## Design

- **Module.** `src/render/`, with a `ChunkMeshPlugin`.
- **Mesher input.**
  - Build the padded `34³` `u16` array that `Mesher::<32>` expects: the Chunk's own Cells plus one layer of border Cells from each face neighbour.
  - Value 0 means "not solid"; any other value is `ElementId::index() + 1`.
  - A missing neighbour Chunk counts as not solid, so its faces are drawn.
  - Only Cells in the solid Phase are meshed here. Liquids and gases come in step 13.
- **Mesh.**
  - Convert the quads into positions, normals and vertex colours, taking each Element's colour from the `ElementTable` resource.
  - All Chunks share one `StandardMaterial` with a white base colour, so the vertex colours show.
  - Each Chunk entity has `Mesh3d`, `MeshMaterial3d` and a `Transform` at `ChunkPos::origin()`.
- **Remeshing.**
  - Remesh a Chunk when it, or any of its face neighbours, changed.
  - Meshing runs on `AsyncComputeTaskPool`, with a budget on how many tasks start each frame.
  - Despawn a Chunk's entity when the Chunk leaves `WorldView`.
- **Scene.** A directional sun light, ambient light and a sky clear colour. Add a temporary free-fly camera so the world can be seen; step 11 replaces it.
- **Ambient occlusion.** Optional. If the quad data makes per-vertex AO easy, add it; otherwise note it as deferred in your report.

## Seams and tests

Test the pure functions inside the game crate, with `#[cfg(test)]` modules next to them:

1. **Padding assembly.** A Cell on a neighbour Chunk's border lands at the right padded index, and a missing neighbour leaves zeros.
2. **Quad conversion.** A single solid Cell produces 6 faces with outward normals. Two adjacent Cells of the same Element produce no faces between them.
3. **Colour mapping.** A quad of Element E gets E's colour.

## Done when

- [ ] The tests pass.
- [ ] **Human check:** `cargo run` shows rolling terrain with Dirt on top of Granite in flat colours, and caves are visible where hills are cut away. The frame rate stays smooth while Chunks load.

## Scope boundary

This step ends with solids on screen. Walking around comes in step 11.
