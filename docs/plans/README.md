# Sim Sandbox implementation plans

Give these to an agent one at a time, in order, for example: "implement docs/plans/02-gas-flow.md". Each step file names its prerequisites and points the agent to [CONVENTIONS.md](CONVENTIONS.md). When an agent finishes a step, it marks that step **done** in the table below.

| Step | Crate | Status |
|---|---|---|
| 00 Heat conduction: built before these plans, so it has no plan file. Code: [heat.rs](../../crates/sim/src/heat.rs); tests: [tests/heat.rs](../../crates/sim/tests/heat.rs) | sim | done |
| [01 Element model and data file](01-element-model.md) | sim | done |
| [02 Gas flow](02-gas-flow.md) | sim | done |
| [03 Liquid flow](03-liquid-flow.md) | sim | done |
| [04 Phase Transitions](04-phase-transitions.md) | sim | todo |
| [05 Tick pipeline, Edits, Settled Chunks](05-tick-pipeline.md) | sim | todo |
| [06 Active Region and Dormant Chunks](06-active-region.md) | sim | todo |
| [07 World generation](07-world-generation.md) | worldgen | todo |
| [08 Performance](08-performance.md) | sim, worldgen | todo |
| [09 Sim thread bridge](09-sim-bridge.md) | game | todo |
| [10 Rendering solid Cells](10-solid-rendering.md) | game | todo |
| [11 First-person controller](11-player-controller.md) | game | todo |
| [12 God tools and palette](12-god-tools.md) | game | todo |
| [13 Liquids and gases on screen](13-fluid-rendering.md) | game | todo |
| [14 Overlays and crosshair readout](14-overlays-readout.md) | game | todo |
| [15 Debug panel and hot reload](15-debug-hot-reload.md) | game, sim | todo |

Steps 01–08 can be tested without opening a window. From step 10 on, each step ends with a list of things for you to check in the running game.
