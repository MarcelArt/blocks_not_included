# Step 04: Phase Transitions

> Read `docs/plans/CONVENTIONS.md` first.

**Prerequisites:** step 03 is done. Step 01 already stores and validates the `low` and `high` thresholds.

## Goal

At the end of each Tick, Cells whose temperature has crossed their Element's threshold become the target Element in place. Mass is kept, and the new matter then flows by the rules of its new Phase.

## Rules

1. Phase Transitions run last in each Tick, after heat, gas and liquid.
2. A non-Vacuum Cell above its Element's `high` threshold becomes the `high` target Element. A Cell below its `low` threshold becomes the `low` target. Mass and temperature stay exactly as they were (ADR 0002).
3. Latent heat is not modelled in this milestone. The hysteresis gap that step 01 validates keeps Cells from flipping back and forth.
4. An Element with no threshold on a side never transitions on that side (`CONTEXT.md`).
5. A Cell that becomes a liquid may now exceed the liquid's capacity. For example, a 2,700,000 g Granite Cell melts into Magma, whose capacity is 2,600,000 g. Step 03's rule 1 makes it shed the excess on later Ticks. If solids box it in, it stays over capacity, because compression is not modelled.

## Seams and tests

**Seam:** `Simulation` (`set`, `get`, `tick`, `total_mass_grams`). Build the tables with `ElementTable::from_ron`, using either the shipped `assets/elements.ron` or small RON strings written in the tests. Put the tests in `crates/sim/tests/phase_transitions.rs`, in this order:

1. Water above 373.15 K becomes Steam, with the same Mass and temperature.
2. Steam below 372.15 K becomes Water.
3. Inside the gap, at 372.5 K, Water stays Water and Steam stays Steam.
4. An Element with no threshold (Oxygen at 10 K) never transitions.
5. Magma below 1400 K becomes Igneous Rock and stops moving.
6. A boiling 1,000,000 g Water Cell becomes a 1,000,000 g Steam Cell, which then spreads into the surrounding Vacuum: its Mass falls over the following Ticks.
7. Granite above 1500 K becomes Magma over capacity, and sheds the excess into a neighbouring Vacuum Cell.
8. Total Mass is unchanged across every transition scenario.

## Done when

- [x] All eight tests exist and pass, and all earlier tests still pass.

## Scope boundary

This step ends with a complete set of phenomena inside one Tick. Step 05 makes the Tick order explicit and adds Edits and Settled Chunks.
