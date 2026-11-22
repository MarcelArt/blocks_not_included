# Step 03: Liquid flow

> Read `docs/plans/CONVENTIONS.md` first.

**Prerequisites:** step 02 is done (gas flow and Displacement).

## Goal

Liquids fall, spread out into level pools, settle into layers by Density and push gases aside. Mass is conserved exactly.

## Starting point

Each Tick runs heat, then gas. Add the liquid rules in a new module (`crates/sim/src/liquid.rs`) that runs after gas. Reuse step 02's code for heat carried by Mass and for Displacement; don't write a second copy.

## Rules

1. **Capacity.** A liquid Cell holds at most its Element's Density × 1 m³, so 1,000,000 g for Water. Liquids do not compress. The one exception is a Cell that has just become a liquid through a Phase Transition (step 04). The liquid rules treat such a Cell as one that must shed its excess, into any non-solid neighbour, before normal spreading.
2. **Falling.** A liquid that can fall does, before it spreads sideways. Depending on what is below it:
   - **Vacuum:** it moves as much Mass down as fits.
   - **A gas:** the two Cells swap entirely. Liquids always sit below gases.
   - **A partly full Cell of the same liquid:** it tops that Cell up toward capacity.
3. **Liquid layering.** A denser liquid directly above a less dense one swaps with it entirely. The decision uses per-Element Density.
4. **Spreading.** A liquid that can't fall moves part of its Mass sideways, in four directions:
   - toward same-liquid neighbours that hold less;
   - into Vacuum neighbours;
   - into gas neighbours, through Displacement (step 02).
   
   A sideways move needs the source to hold more than the target by at least a **spreading threshold**, a named constant around 1% of capacity. That threshold is what lets pools come to rest. Flows below 1 g round to zero.
5. Moving Mass carries heat (step 02, rule 1). Liquids never enter solids, and solids never move.

Apply the same ordering and bias rules as step 02: independent of direction and processing order, and deterministic.

## Seams and tests

**Seam:** `Simulation` (`set`, `get`, `tick`, `total_mass_grams`). Put the tests in `crates/sim/tests/liquid.rs`, in this order:

1. A liquid in a sealed column of Vacuum ends up at the bottom.
2. A liquid Cell above a gas Cell swaps below it in one Tick.
3. Three full Cells' worth of Water released at the top of a sealed column one Cell wide fills exactly the bottom three Cells to 1,000,000 g each.
4. A denser liquid above a lighter one ends up below it.
5. Liquid in a flat-bottomed sealed box settles into a level layer. Bottom-layer Cells differ by no more than the spreading threshold, and further Ticks change nothing.
6. Spreading into a gas uses Displacement: gas that has somewhere to go is pushed aside, and boxed-in gas blocks the liquid.
7. A column of liquid dropped at the centre of a symmetric flat box spreads equally in all four horizontal directions.
8. Total Mass per Element is unchanged after every Tick, across scenarios that mix liquids and gases.

## Done when

- [ ] All eight tests exist and pass, and the heat and gas tests still pass.

## Scope boundary

This step ends with liquids moving. Liquids changing Element (freezing, boiling) come in step 04.
