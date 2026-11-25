# Step 22 — Solid Element textures

Read [CONVENTIONS.md](CONVENTIONS.md), [BEVY-0.20-NOTES.md](BEVY-0.20-NOTES.md), and [the phase scope](../phase-2a-plans.md).

**Prerequisites** — Steps 10, 14, and 19.

## Goal

Give Dirt, Granite, Ice, and Igneous Rock distinct, seamless pixel textures while preserving solid meshing, lighting, and Overlays.

## Design

- Use the `imagegen` skill to make a coherent low-resolution set for the four shipped solid Elements. Inspect each tile repeated in a grid, including across both wrap edges, before adding it to `assets/`. Keep the Elements recognizable in sun and shade.
- Add texture coordinates and a texture selection to solid geometry in `src/render/mesher.rs`. Keep greedy geometry and the shared material path. A face spanning several Cells repeats the selected texture once per metre in both directions. Preserve the tile phase across neighbouring quads and Chunk boundaries.
- Resolve textures by Element identity, preserve current lighting and corner shading, and give an Element without a texture its existing flat colour. Keep normal Element hot reload and remeshing working. Debug Overlays keep their existing ramps and visibility rules.

## Seams and tests

1. A single Cell and a greedy quad spanning multiple Cells produce the expected tile count on each face axis. Adjacent quads and Chunk borders have matching repeat alignment.
2. Two solid Elements choose distinct textures. A solid Element without a texture uses its fallback and still renders after a remesh.
3. Check that greedy face counts and winding remain intact and that toggling an Overlay hides and restores the textured geometry.

## Done when

- **In game** — Inspect all four Elements, a large merged wall, and a Chunk seam. Verify textures repeat without stretching or obvious seams. Confirm current lighting and diagnostic Overlays still read correctly.
