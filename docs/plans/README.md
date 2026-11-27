# Sim Sandbox implementation plans

Give these to an agent one at a time, in order, for example: "implement docs/plans/02-gas-flow.md". Each step file names its prerequisites and points the agent to [CONVENTIONS.md](CONVENTIONS.md). When an agent finishes a step, it marks that step **done** in the table below.

| Step | Crate | Status |
|---|---|---|
| 00 Heat conduction: built before these plans, so it has no plan file. Code: [heat.rs](../../crates/sim/src/heat.rs); tests: [tests/heat.rs](../../crates/sim/tests/heat.rs) | sim | done |
| [01 Element model and data file](01-element-model.md) | sim | done |
| [02 Gas flow](02-gas-flow.md) | sim | done |
| [03 Liquid flow](03-liquid-flow.md) | sim | done |
| [04 Phase Transitions](04-phase-transitions.md) | sim | done |
| [05 Tick pipeline, Edits, Settled Chunks](05-tick-pipeline.md) | sim | done |
| [06 Active Region and Dormant Chunks](06-active-region.md) | sim | done |
| [07 World generation](07-world-generation.md) | worldgen | done |
| [08 Performance](08-performance.md): busy 76 ms, settled 44 ms (target 10 ms missed: the world never comes to rest; a region at rest takes 0.07 ms), per Tick on 3 threads | sim, worldgen | done |
| [09 Sim thread bridge](09-sim-bridge.md) | game | done |
| [10 Rendering solid Cells](10-solid-rendering.md) | game | done |
| [11 First-person controller](11-player-controller.md) | game | done |
| [12 God tools and palette](12-god-tools.md) | game | done |
| [13 Liquids and gases on screen](13-fluid-rendering.md) | game, sim | done |
| [14 Overlays and crosshair readout](14-overlays-readout.md) | game | done |
| [15 Debug panel and hot reload](15-debug-hot-reload.md) | game, sim | done |

Steps 01–08 can be tested without opening a window. From step 10 on, each step ends with a list of things for you to check in the running game.

## Phase 2A

[Phase scope and agreed behavior](../phase-2a-plans.md). Steps 16–18 can be checked headlessly. Steps 19–23 also have in-game checks.

| Step | Crate | Status |
|---|---|---|
| [16 Simulation checkpoint](16-simulation-checkpoint.md) | sim, worldgen, game | done |
| [17 World save storage and catalog](17-world-save-storage.md) | game, sim, worldgen | done |
| [18 World sessions and pause](18-world-sessions.md) | game | done |
| [19 Main menu and saving in play](19-menus-and-saving.md) | game | planned |
| [20 Settings and bindings](20-settings-and-bindings.md) | game | planned |
| [21 Flight, Run, Sneak, and pool exits](21-movement-controls.md) | game | planned |
| [22 Solid Element textures](22-solid-textures.md) | game | planned |
| [23 Liquid Element textures](23-liquid-textures.md) | game | planned |
