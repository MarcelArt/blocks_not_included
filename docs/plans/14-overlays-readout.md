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

## Seams and tests

Pure functions, with unit tests:

1. The temperature ramp returns exactly the stop colours at each stop, and the midpoint between two stops halfway between.
2. The Mass ramp gives black at 0 and white at full, and anything over capacity is magenta.
3. Bucket mapping is stable: equal values always land in the same bucket, and the 16 buckets cover the whole range.
4. Mass formatting covers 12 g, 1,430 g, 1,000,000 g and 2,700,000 g.

## Done when

- [ ] The tests pass.
- [ ] **Human check:**
  - The Temperature Overlay shows the ground getting hotter with depth, and a heat-brushed spot glowing as it cools.
  - The Element Overlay shows Carbon Dioxide pooling below Oxygen in a cave.
  - The readout follows the crosshair.

## Scope boundary

This step ends with inspection tools. The debug panel and hot reload come in step 15.
