# Step 23 — Liquid Element textures

Read [CONVENTIONS.md](CONVENTIONS.md), [BEVY-0.20-NOTES.md](BEVY-0.20-NOTES.md), and [the phase scope](../phase-2a-plans.md).

**Prerequisites** — Steps 13, 21, and 22.

## Goal

Make Water and Magma visually distinct with static pixel textures while keeping liquid Mass visible through fill height.

## Design

- Use the `imagegen` skill to produce seamless, low-resolution Water and Magma tiles. Match the scale of the solid tiles. Use static textures and keep Water translucent.
- Extend `src/render/fluids.rs` to texture liquid surfaces and exposed sides. Tile according to world scale. A shallow side samples a shallow portion of the tile so an exposed face does not stretch as the liquid changes height. Align neighbouring faces and Chunk seams.
- Preserve existing fill heights, exposed step faces, visible undersides, and layer-specific blending and shadow behavior. Keep the current gas tint path and diagnostic Overlays. A liquid Element without a texture retains its flat-colour rendering.

## Seams and tests

1. Check positions and UV extents for full and partly filled liquid surfaces, sides, and exposed step faces. Check adjacent Chunks produce continuous repeat alignment.
2. Verify Water and Magma choose their own appearances, and an untextured liquid and tinted gas still use the expected fallback paths.

## Done when

- **In game** — Compare full and shallow Water beside a bank, watch a flowing boundary change height, and inspect Magma. Verify transparent Water, visible fill heights, no stretched side tiles, and unchanged gas tint and Overlay behavior. Check that a textured shallow pool still works for the pool exit from step 21.
