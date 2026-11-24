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

### Decisions made during implementation

- **Modules.**
  - `src/player/raycast.rs`: `Hit`, `raycast` and `REACH`.
  - `src/tools/mod.rs`: `GodToolsPlugin`, the `Aim` resource (the Cell in view), the systems, and the Edit log.
  - `src/tools/palette.rs`: `Tool`, `Palette`, `cell_of` and the egui window.
  - `src/tools/target.rs`: `stops_sight` (solids and liquids) and `placement_target`.
  - `src/tools/brush.rs`: `brush_cells` and `BrushStroke`, which batches the brush's Edits.
- **Raycast.**
  - Where a ray crosses an edge or a corner exactly, it steps along x first, then y, then z.
  - A ray that starts inside a target reports a zero normal, so placement refuses it: the Cell "in front" is the hit Cell itself.
  - A Cell whose Chunk isn't in `WorldView` doesn't stop sight, and placement refuses it.
- **Aim.** The `Aim` resource holds the Cell in view, found after the player has moved each frame. Step 14's readout target is the same Cell, so it can read `Aim`.
- **Input.**
  - Tools act only while the cursor is captured and egui doesn't want the pointer.
  - While the cursor is captured, the palette window is not interactable. On X11 the hidden cursor still moves (`Locked` falls back to `Confined`), so without this it could hover and press the palette. With it, egui never wants the pointer while the cursor is captured.
  - A click on the palette doesn't capture the cursor. The click that does capture is used up with `ButtonInput::clear_just_pressed`.
  - The tools run before `PlayerSystems`, on the previous frame's cursor state and `Aim`: what the player saw when they clicked. So a click on a frame that starts with the cursor free never uses a tool. The agent check found this: run after the player's systems, a left and right click in the same frame captured the cursor and spawned a Cell.
  - Number keys don't switch tools while egui wants the keyboard, such as while typing into a value field.
- **Place and Spawn fluid.** Each keeps its own Element, Mass and temperature in the palette. Place offers every Element but Vacuum; Spawn fluid offers only gases and liquids. They start on the first solid and the first liquid in `assets/elements.ron`: Ice and Water.
- **Palette values.**
  - Mass is a logarithmic slider from 1 g up to a limit: Capacity for a liquid, Density × 1 m³ for a solid, and ten times that for a gas, whose Mass is its pressure. Choosing an Element resets Mass to Density × 1 m³.
  - Temperature runs from −273.15 °C to 3,000 °C. Choosing an Element keeps the temperature set, unless the Element would cross one of its thresholds as soon as it is placed. It then comes at 20 °C, or 10 K inside a threshold that 20 °C would cross: Ice at −10 °C, Steam at 109 °C, Magma at 1,136.85 °C. The agent check found why the 20 °C reset is needed: moving the old temperature just inside the new Element's thresholds left Water at 9 °C after Ice.
  - The palette also shows the chosen Element's thresholds, e.g. "Becomes Water above 0.00 °C".
- **Heat brush.**
  - It reaches the Cells whose centres lie within the radius of the hit Cell's centre. The radius runs from 0 to 5 m, with a default of 1.5 m (19 Cells). Only Cells in `WorldView` that aren't Vacuum get Edits.
  - Strength runs from −1,000 to 1,000 K/s, with a default of 100 K/s.
  - "At most 10 Edits per second" is read per Cell. Each 0.1 s held makes one batch, with one `ShiftTemperature` per Cell carrying strength × the time held since the last batch. Less than 0.1 s left over at release is dropped.
  - A stroke starts only with a press made while the cursor is captured.
- **Feedback.**
  - A white gizmo outline marks the Cell the tool in hand would change. The heat brush also shows its radius as a sphere.
  - Every Edit is logged at `info`. After each Tick, the changed Cell is logged if it has changed, for 25 Ticks (5 s). The brush's batches are logged at `debug`.
- **Dependency.** `bevy_egui` 0.43.1, with default features off except `render` and `default_fonts`.

## Seams and tests

Pure functions, with unit tests:

1. Raycast hits a Cell straight ahead along each axis and reports the face it entered through.
2. Raycast follows a diagonal ray through the right sequence of Cells.
3. Raycast misses when the target is beyond `max_distance`.
4. A ray starting inside a target Cell reports that Cell.
5. Placement target: the hit Cell plus its normal, and refused when that Cell is solid or liquid.

Added at the same seams:

- In `src/tools/target.rs`, test 5 is three tests: the Cell in front is filled when it holds Vacuum or gas; refused when it is solid, liquid or not loaded; refused when the ray started inside the hit Cell.
- In `src/tools/brush.rs`:
  - a radius of 0 reaches only the centre, 1 reaches the centre and its 6 face neighbours, and 1.5 reaches 19 Cells;
  - a held stroke sends a batch once 0.1 s has built up;
  - it sends at most 10 batches a second, at 30 to 1,000 frames per second;
  - nothing is sent outside a stroke.
- In `src/tools/palette.rs`:
  - a chosen Element comes as Density × 1 m³;
  - it keeps a temperature that crosses no threshold;
  - past a threshold, it comes at 20 °C or 10 K inside;
  - a new palette places the first solid and spawns the first liquid;
  - the Mass limits of a liquid and a gas.

## Done when

- [x] The tests pass: 4 in `src/player/raycast.rs`, 3 in `src/tools/target.rs`, 5 in `src/tools/brush.rs` and 5 in `src/tools/palette.rs`.
- [x] **Human check:**
  - Digging a Cell next to a cave makes the Oxygen visibly fill the hole. Use the debug log, or wait for the step 14 readout.
  - Placing Water on the ground makes it spread.
  - Heating Ice past 0 °C melts it.
  - The agent checked this with a temporary harness, since removed. Like step 11's, it pressed keys and mouse buttons on `ButtonInput`, set the body and view, followed the Cells around each Edit after every Tick, and took screenshots.
  - **Dig.** With the eyes in a cave Cell at (0, −4, 1), it dug the Dirt wall at (1, −4, 1). On the next Tick, the hole held 0.35 kg of Hydrogen: that cave's top layer is Hydrogen, so which gas fills a hole depends on the cave.
  - **Place.** 1,000 kg of Water at 20 °C, placed on the ground at (4, −2, 4), spread from 1 Cell to 41 in 20 Ticks. Its own Cell dropped to 67 kg, and the Mass was conserved until Water left the 9 × 7 × 9 box being watched.
  - **Heat brush.** Ice placed at −10 °C and brushed at 100 K/s was Water at 0.25 °C on the next Tick. Held for 2 s, the brush boiled that Water to Steam, then heated the Dirt under it to 211 °C.
  - **Spawn fluid.** 1.98 kg of Carbon Dioxide spawned on the ground stayed in its Cell: a lone Cell of heavy gas under Oxygen has nowhere to go.
  - **Input.** The click that captured the cursor didn't dig. A left and right click in the same frame, with the cursor free, captured it without spawning, and the next right click spawned.
  - **Not checked by the agent:** using the palette with a real mouse, which means picking an Element, dragging the sliders, and a click on the palette leaving the cursor free. Gas and liquid can't be seen yet, so their outlines float over nothing until step 13.

### Deferred

- **Crosshair.** Step 14 adds it. Until then, the outline of the target Cell shows where the player is aiming.
- **Brush strength.** 100 K/s boils a Cell of Water in about a second. A lower default may suit better once the Temperature Overlay shows what the brush does.
- **Elements changing at runtime.** The palette keeps its `ElementId`s when `Elements` changes. Step 15's hot reload should reset the palette if the table shrinks.

## Scope boundary

This step ends with changing the world. Seeing liquids properly and seeing gases come in step 13.
