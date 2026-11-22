# Step 02: Gas flow

> Read `docs/plans/CONVENTIONS.md` first.

**Prerequisites:** step 01 is done, so Elements have Phase and Density.

## Goal

Gases spread into Vacuum, even out with neighbouring Cells of the same gas, settle into layers by Density, and push other gases aside through **Displacement**. Mass is conserved exactly, and moving Mass carries its heat with it.

## Starting point

- `Simulation::tick` (in `crates/sim/src/simulation.rs`) currently runs heat conduction only (`crates/sim/src/heat.rs`). Each Tick it collects every heat flow from the temperatures at the start of the Tick, then applies them all together.
- Put the gas rules in a new module (`crates/sim/src/gas.rs`) and run them after heat within each Tick.

## Rules

1. **Moved Mass carries heat.** When `m` grams at temperature `Ts` join `M` grams of the same Element at `Td`, the result is `(m·Ts + M·Td) / (m + M)`. A Vacuum Cell that receives Mass takes on the source's Element and temperature.
2. **Into Vacuum.** A gas Cell moves part of its Mass into each face-neighbouring Vacuum Cell, in all six directions. Gases have no gravity of their own.
3. **Evening out.** Two touching Cells of the same gas move Mass from the fuller Cell toward the emptier one, by a fraction of the difference each Tick. Make that fraction a named constant. Any flow below 1 g rounds to zero, which is how gas comes to rest (ADR 0004).
4. **Layering.** When a gas Cell sits directly above a Cell of a different gas with greater Density, the two Cells swap entirely: Element, Mass and temperature. The decision uses each Element's Density only, never how much Mass the Cells hold.
5. **Displacement.** This rule covers every partial move into a Cell that holds a *different* gas, whenever rule 4 does not apply.
   - Gas X may move part of its Mass into a neighbouring Cell holding gas Y only if Y's entire contents can leave that Cell. Y leaves by merging into one of its own face neighbours that holds Y, or by moving into a neighbouring Vacuum Cell.
   - X can do this only from a Cell holding more Mass than the Y Cell it displaces, so pressure drives it.
   - Once Y has left, X fills the vacated Cell as if it were Vacuum.
   - If Y has nowhere to go, nothing moves.
   
   Ask the human to add **Displacement** to `CONTEXT.md` when this step is done.
6. Gas never moves into a solid Cell. Liquids arrive in step 03; in this step, treat liquid Cells like solids.

### Ordering and bias

All moves must be independent of direction and processing order, and deterministic. Two workable approaches; choose one and explain the choice in a module doc comment:

- Plan every transfer from the state at the start of the Tick, the way heat does. Then resolve conflicts, such as two different gases targeting the same Vacuum Cell, with a deterministic pseudo-random tie-break seeded by the Tick count and Cell position.
- Process directions in an order that is shuffled each Tick, seeded the same way.

A Vacuum Cell can only become one Element per Tick.

## Seams and tests

**Seam:** `Simulation` (`set`, `get`, `tick`), plus a new `Simulation::total_mass_grams() -> u64`.

- Build sealed boxes from a test solid with zero conductivity.
- Check the invariants over every Cell the test set, using `get`.
- Put the tests in `crates/sim/tests/gas.rs`, in this order:

1. A gas Cell next to Vacuum moves some Mass into it, and the Vacuum Cell becomes that gas at the source's temperature.
2. Gas released into a sealed box of Vacuum spreads until every inside Cell holds gas. Total Mass is unchanged after every Tick.
3. Two touching Cells of one gas holding 2000 g and 1000 g settle at 1500 g each, within 1 g. After that, another 100 Ticks change nothing.
4. Moved Mass carries heat. Use a gas with zero conductivity: 2000 g at 300 K next to 1000 g at 400 K. They settle at 300 K and about 366.67 K. Only the hot Cell receives Mass, and it receives it all at 300 K: (1000·400 + 500·300) / 1500 = 366.67 K.
5. A heavy gas (Density 2.0, 10 g) above a light gas (Density 0.09, 5000 g) ends up below it after one Tick. Density decides, not Mass.
6. A sealed column of alternating heavy and light gas Cells ends with every heavy Cell below every light Cell.
7. Displacement: a high-pressure gas Cell next to a different gas that has a same-Element neighbour to merge into advances into the vacated Cell. Total Mass per Element is unchanged.
8. Gas that is boxed in, with no same-Element or Vacuum neighbour, is never displaced, and nothing moves.
9. Gas released at the centre of a sealed cube of Vacuum spreads identically in all six directions. Mirror-image Cells hold equal Mass.
10. Solid walls never change, and no gas appears outside the box.

## Done when

- [ ] All ten tests exist and pass, and the heat tests still pass.
- [ ] The gas tests check the invariants after every Tick.

## Scope boundary

This step ends with gases. Liquids, and gas interacting with liquids, come in step 03. Speed is not a goal yet; Settled Chunks arrive in step 05 and parallelism in step 08.
