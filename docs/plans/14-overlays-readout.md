# Step 14: Overlays and crosshair readout

> Read `docs/plans/CONVENTIONS.md` and `docs/plans/BEVY-0.20-NOTES.md` first. `CONTEXT.md` defines **Overlay**.

**Prerequisites:** step 13 is done.

## Goal

The player can switch the view to an Overlay that colours Cells by Element, temperature or Mass, and can always read the exact values of the Cell they are aiming at and the gas at their head.

## Design

- **Overlay modes.** Off, Element, Temperature and Mass. The O key cycles through them.
- **What an Overlay draws.** In an Overlay, every non-Vacuum Cell in the loaded Chunks within the Active Region is drawn as translucent geometry, coloured by the chosen property and replacing the normal look.
  - To keep the vertex count low, sort each Cell into one of 16 colour buckets and mesh the buckets with `binary-greedy-meshing`. You get faces only where the bucket changes, so layers stand out.
  - Rebuild a Chunk's Overlay mesh when the Chunk changes or the mode changes.
- **Temperature ramp.** A fixed scale; interpolate linearly between these stops:

  | Temperature | Colour |
  |---|---|
  | 250 K | dark blue |
  | 273 K | blue |
  | 293 K | green |
  | 373 K | yellow |
  | 800 K | red |
  | 1500 K | white |

- **Mass ramp.** Mass divided by Density × 1 m³: black at 0, white at 1, magenta above 1.
- **Element Overlay.** Each Element's colour, including the gases.
- **Crosshair.** A small cross at the centre of the screen.
- **Readout panel** (egui, top-right):
  - **Target:** the first Cell on the ray (reach 8 m) that is neither Vacuum nor gas. Show its Element, Phase, Mass and temperature in °C.
  - **At head:** the Cell at eye height. Show its Element, Mass (which is pressure, for a gas) and temperature.
  - Values come from `WorldView`, so they can be up to one Tick old (ADR 0003).
- **Formatting Mass.** Choose the unit by size: `12 g`, `1.43 kg`, `1,000 kg`, `2.7 t`.

### Decisions made during implementation

- **Modules.**
  - `src/overlay/ramp.rs` holds the pure ramps and buckets.
  - `src/overlay/mesh.rs` holds `OverlayLooks` and `mesh_overlay`.
  - `src/overlay/mod.rs` holds `Overlay`, the `ShownOverlay` resource, `OverlayPlugin` and the legend.
  - `src/readout.rs` holds `ReadoutPlugin` (the crosshair and the panel), `format_mass` and `format_temperature`.
- **Shared with step 10's mesher.**
  - `mesh_greedy` in `src/render/mesher.rs` takes a value function, a colour function and `Faces::Opaque` or `Faces::Translucent`. `mesh_solids` now calls it.
  - With `Faces::Translucent`, every value goes to the crate's transparent mask, so a face shows wherever the value changes, on both sides.
  - `render::take_nearest` picks the stale Chunks nearest the focus for both meshing pipelines.
- **The Active Region on the game side.** The simulation thread sends `SimUpdate::ActiveRegion` whenever the region moves, and once at start. `WorldView::active_region()` holds it, and is `None` until the first update arrives.
- **What an Overlay draws.**
  - Every loaded Chunk in the Active Region is drawn: 32 Chunks with the shipped settings.
  - Its border reads the real Cells of loaded neighbours, inside the Active Region or not. A missing neighbour counts as Vacuum, as for solids.
  - A Chunk is remeshed when it or a face neighbour changes or leaves, when the Active Region moves, when the Overlay changes, and when `Elements` changes. Until a new mesh is ready, the old one stays.
  - The Element Overlay's value is the Element itself, not a bucket. Each Element keeps its own colour, so a table of any size works.
- **The normal look.**
  - While an Overlay is shown, every Chunk's normal meshes are hidden, Dormant Chunks included. Hiding only the Active Region's left holes where the Dormant world's solid faces had been culled against Cells now drawn translucent.
  - The background turns a dark neutral grey (sRGB 0.06, 0.06, 0.07). Through every layer, the sky's blue tinted the Temperature Overlay cyan, which on that scale reads as cold.
- **How it is drawn.**
  - One shared `StandardMaterial`, white, with `AlphaMode::Blend` and the vertex alpha at 0.55.
  - Back faces stay culled, and where the value changes both Cells show a face. So from any side, the player sees only the face pointing towards them: each layer in its own colour, as if seen from outside it. A layer they stand inside shows no faces.
  - At alpha 0.35, the nearest layers were lost in the pale haze of the ones behind.
  - **Lit, not unlit.** Unlit, the faces of one layer looked the same from every side, so no shape could be read. Lit, a sunlit top face shows at its colour, which matches the legend, and the other sides are shaded.
  - Overlay entities get `NotShadowCaster`.
- **Temperature buckets.**
  - The 16 buckets split the scale into equal shares of its colours, not of its kelvin: 3.2 buckets for each pair of neighbouring stops. A bucket spans 6 to 25 K near room temperature, and about 220 K from red to white.
  - Equal kelvin buckets would be 78 K wide, which would put the whole Active Region (288 to about 330 K) in one or two buckets.
  - A bucket's colour is the ramp's colour at its share of the scale. Bucket 0 is dark blue and bucket 15 white; every bucket's colour is one that some temperature in it has.
  - With the shipped world:
    - the surface Cells (288 K) sit in one bucket and the Atmosphere (293 K) in the next, so the ground's surface shows;
    - the next layer boundaries fall about 3 m, 17 m and 38 m down.
- **Mass buckets.**
  - 15 buckets split empty to full evenly, and the 16th holds every Cell above full.
  - A bucket's colour is grey at bucket / 14: black for the emptiest, white for the fullest.
  - "Full" is Density × 1 m³, rounded the way the simulation rounds Capacity (`tools::full_mass_grams`). Without the rounding, 1.43 × 1000 in `f32` falls just short of 1,430 g, and every full Cell of Oxygen would show magenta.
- **Stop colours,** in linear RGB:

  | Stop | Colour |
  |---|---|
  | dark blue | (0, 0, 0.25) |
  | blue | (0, 0.25, 1) |
  | green | (0, 0.8, 0.1) |
  | yellow | (1, 0.85, 0) |
  | red | (1, 0.05, 0) |
  | white | (1, 1, 1) |
  | black | (0, 0, 0) |
  | magenta | (1, 0, 1) |

  Interpolation is linear, in linear RGB. A NaN temperature counts as the coldest.
- **Carbon Dioxide's colour** in `assets/elements.ron` changed from (0.45, 0.45, 0.45) to (0.1, 0.1, 0.1).
  - In the Element Overlay, the old grey was almost Granite's colour, so a pool of Carbon Dioxide on a Granite floor didn't show.
  - It is an untinted gas, so the normal look doesn't change.
- **Controls and panels.**
  - O cycles: off, Element, Temperature, Mass, off. It is ignored while egui wants the keyboard, as the tool keys are.
  - The readout sits top-right and the Overlay legend bottom-right. Both are always `.interactable(false)`, so they never take the pointer.
  - The legend lists each Element's colour, the six temperature stops in whole °C, or the Mass ramp's three colours.
- **Crosshair.** Drawn with egui on the `Background` layer: over the world, under the windows. It has 6-point arms, white over a black outline, so it shows against the sky.
- **Readout.**
  - **Target** is the `Aim` resource from step 12, which is the same Cell. **At head** is the Cell containing the eyes.
  - Both show the Cell's position, Element, Phase, Mass and temperature. A gas's Mass is labelled "Mass (pressure)". Vacuum shows no Mass or temperature, and a Cell whose Chunk isn't loaded reads "Not loaded".
  - Temperature shows in °C to two decimals, never as "−0.00 °C".
- **Formatting Mass.**
  - Below 1 kg, whole grams. Up to and including 1,000 kg (a full Cell of Water), kilograms. Above that, tonnes.
  - Kilograms and tonnes show three significant digits, without trailing zeros, and with commas from 1,000 on. For example, 999,999 g shows as "1,000 kg" and 1,000,001 g as "1 t".
- **Performance.**
  - Measured in a 1280 × 720 window at the spawn point, after loading had finished, alternating 8 s windows.
  - With no Overlay, the mean frame was 8.2–9.0 ms, with 11–36 frames over 16.7 ms per window.
  - With the Temperature Overlay, the mean frame was 8.8–9.7 ms, with 52–85 frames over 16.7 ms per window.
  - As in step 13, the cost is meshing CPU competing with the main thread. While an Overlay is shown, each Chunk that changes is meshed twice: once for its normal look and once for the Overlay.

## Seams and tests

Pure functions, with unit tests:

1. The temperature ramp returns exactly the stop colours at each stop, and the midpoint between two stops halfway between.
2. The Mass ramp gives black at 0 and white at full, and anything over capacity is magenta.
3. Bucket mapping is stable: equal values always land in the same bucket, and the 16 buckets cover the whole range.
4. Mass formatting covers 12 g, 1,430 g, 1,000,000 g and 2,700,000 g.

Added at the same seams:

- In `src/overlay/ramp.rs`:
  - halfway is also checked between dark blue and blue, and between red and white;
  - past either end of the scale, the colour is that end's;
  - test 2 is two tests, the second with 1 g over full, 1,200 kg of Water, and Oxygen at ten times full;
  - test 3 is three tests: temperature buckets cover 250 to 1500 K in order and clamp past either end; Mass buckets cover empty to full and the 16th holds what is over full; equal values land in the same bucket, including the same fill of two Elements;
  - each bucket's colour is one its own values take, and the end buckets show the end colours.
- In `src/overlay/mesh.rs`:
  - the Element Overlay draws a gas in its Element's colour;
  - Vacuum isn't drawn in any Overlay;
  - Cells in one bucket share no face;
  - where the bucket changes, each Cell shows a face towards the other;
  - the Mass Overlay sorts Cells by how full they are, whatever their Element;
  - a face at the Chunk border shows only where the neighbour Chunk differs.
- In `src/overlay/mod.rs`: O cycles from off through each Overlay and back to off.
- In `src/readout.rs`:
  - Mass shows three significant digits at most, from 0 g up to `u32::MAX`;
  - temperature shows in °C to two decimal places, without "−0.00".
- In `tests/sim_bridge.rs`: the world view holds the Active Region, and follows it as the focus moves.

Every quad test also checks each triangle's winding.

## Done when

- [x] The tests pass: 9 in `src/overlay/ramp.rs`, 6 in `src/overlay/mesh.rs`, 1 in `src/overlay/mod.rs`, 3 in `src/readout.rs` and 1 more in `tests/sim_bridge.rs`.
- [x] **Human check:**
  - The Temperature Overlay shows the ground getting hotter with depth, and a heat-brushed spot glowing as it cools.
  - The Element Overlay shows Carbon Dioxide pooling below Oxygen in a cave.
  - The readout follows the crosshair.
  - The agent checked these with a temporary harness, since removed. It pressed O on `ButtonInput`, set the body and view, sent Edits through `SimBridge`, logged Cells from `WorldView`, and took screenshots.
  - **Cycling.** O went off → Element → Temperature → Mass → off, hiding the normal look and darkening the background while an Overlay was shown.
  - **Hotter with depth.**
    - In a column 20 m from spawn, the ground read 19.7 °C at y = 0, then 25.6, 31.6, 37.6, 43.5, 49.6 and 55.6 °C every 5 m down: 1.2 K/m, as generated.
    - From 14 m underground, the Temperature Overlay showed a yellow-green layer below the eyes, green around them, and the blue-green surface skin above.
  - **Heated spot.**
    - The harness raised 33 Cells within 2 m of the Dirt Cell in view by 800 K. The centre went from 14.9 °C to 814.9 °C, and showed as nested orange, yellow and yellow-green shells.
    - It barely cools: 814.3 °C after 60 s. See Deferred.
  - **Carbon Dioxide.**
    - 9 Cells of 5.94 kg of Carbon Dioxide were placed 3 Cells above a cave floor at (1, −23, −14). After 25 s, the bottom 2–3 Cells of the cave there held Carbon Dioxide, with Oxygen above.
    - In the Element Overlay, it shows as a dark patch on the floor. Before the colour change, it didn't show.
    - The world also holds 492–636 Cells of generated Carbon Dioxide with Oxygen right above them, in the Active Region at spawn.
  - **Readout.** Looking down from spawn, the target was the Dirt Cell at (2, −1, 2), at 14.90 °C and then 814.86 °C once heated. Turned 0.9 rad, it was (3, −1, 0) at 52.73 °C. At head was Oxygen, 1.43 kg, 19.85 °C.
  - **Not checked by the agent:** the real mouse turning the view, and the Overlays while walking far enough for the Active Region to move.
  - The human hasn't checked it yet.

### Deferred

- **Cooling is slow.** A heat-brushed spot "glowing as it cools" glows, but cools by under 1 K a minute at its centre. That is the simulation's heat conduction at `conduction_multiplier` 1000, not the Overlay. Step 15's debug panel is the place to tune it.
- **The Mass Overlay hides the ground.** A full Cell of rock and a full Cell of air share the "full" bucket, so no face lies between them. Only Cells that aren't exactly full stand out: gas pressed above or thinned below full in caves and pockets, and partly filled liquids. Showing where the Element changes as well would put the surface back, but the step asks for faces only where the bucket changes.
- **Pale Elements.** In the Element Overlay, Oxygen, Hydrogen, Steam, Ice and Granite are still all near-white. Seen through translucent rock, a cave reads as a pale haze. Only Carbon Dioxide's colour was changed.
- **Meshing twice.** While an Overlay is shown, the normal meshes are hidden but still remade whenever a Chunk changes. Pausing normal meshing until the Overlay is turned off would likely remove most of the Overlay's extra slow frames.

## Scope boundary

This step ends with inspection tools. The debug panel and hot reload come in step 15.
