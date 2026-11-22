# Step 12: God tools and palette

> Read `docs/plans/CONVENTIONS.md` and `docs/plans/BEVY-0.20-NOTES.md` first.

**Prerequisites:** step 11 is done.

## Goal

The player can dig, place any Element at a chosen Mass and temperature, heat or cool an area with a brush, and spawn gas or liquid, all chosen from a palette.

## Design

- **Raycast** (`src/player/raycast.rs`). A pure grid traversal using Amanatides–Woo DDA: `raycast(origin, direction, max_distance, is_target) -> Option<Hit { cell: IVec3, normal: IVec3 }>`. Reach is 8 m.
- **Targets.**
  - Dig and the heat brush aim at the first Cell on the ray that is neither Vacuum nor gas.
  - Place and spawn fill the Cell in front of the face that was hit (`hit.cell + hit.normal`), but only if that Cell is Vacuum or gas.
- **Tools.** Number keys 1–4 select a tool.
  - **Dig** (left click) sends `Edit::Dig`.
  - **Place** (right click) sends `Edit::Place` with the palette's Element, Mass and temperature.
  - **Heat brush** (hold the left button) sends `Edit::ShiftTemperature` to every non-Vacuum Cell within the brush radius of the hit. The rate is ±K per second; batch the changes into at most 10 Edits per second.
  - **Spawn fluid** (right click) places a gas or liquid Cell with the palette's Mass.
- **Palette** (`bevy_egui` 0.43): a tool selector; an Element picker built from the `ElementTable` resource; a Mass slider shown in kg, defaulting to Density × 1 m³; a temperature slider shown in °C and stored in K; and brush radius and strength.
- **Input.** While egui wants the pointer, tools don't fire.
- These tools create and destroy matter by design (Q10 of the design session).

## Seams and tests

Pure functions, with unit tests:

1. Raycast hits a Cell straight ahead along each axis and reports the face it entered through.
2. Raycast follows a diagonal ray through the right sequence of Cells.
3. Raycast misses when the target is beyond `max_distance`.
4. A ray starting inside a target Cell reports that Cell.
5. Placement target: the hit Cell plus its normal, and refused when that Cell is solid or liquid.

## Done when

- [ ] The tests pass.
- [ ] **Human check:**
  - Digging a Cell next to a cave makes the Oxygen visibly fill the hole. Use the debug log, or wait for the step 14 readout.
  - Placing Water on the ground makes it spread.
  - Heating Ice past 0 °C melts it.

## Scope boundary

This step ends with changing the world. Seeing liquids properly and seeing gases come in step 13.
