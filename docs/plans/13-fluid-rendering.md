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

## Seams and tests

Pure mesher functions, with unit tests:

1. A half-full liquid Cell has its top face at y + 0.5.
2. A liquid Cell under the same liquid has no top face.
3. Two neighbouring Cells of the same liquid share no side face.
4. A liquid Cell next to a solid has no side face on that side.
5. Gas with tint 0 produces no geometry.
6. `elements.rs`: a tint outside 0 to 1 is rejected.

## Done when

- [ ] The tests pass.
- [ ] **Human check:**
  - A Water pocket looks like a pool, and a placed Water Cell visibly spreads into a thin, level layer.
  - Boiling Water shows faint white Steam drifting away.

## Scope boundary

This step ends with fluids on screen. Overlays and the crosshair readout come in step 14.
