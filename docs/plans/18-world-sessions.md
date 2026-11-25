# Step 18 — World sessions and pause

Read [CONVENTIONS.md](CONVENTIONS.md), [BEVY-0.20-NOTES.md](BEVY-0.20-NOTES.md), and [the phase scope](../phase-2a-plans.md).

**Prerequisites** — Steps 16 and 17.

## Goal

Open a fresh or restored World on demand, run it only during its session, and give the game a pause distinct from the debug clock.

## Design

- Move simulation-worker startup out of plugin construction in `src/sim_bridge.rs`. Enter a World through two paths: seed a fresh simulation and find its normal spawn, or install the saved simulation and player pose before the first Tick. Keep the player held during streaming until the spawn or restored location is ready. Preserve saved orientation, movement state, and Focus through that transition.
- Tie the worker, view, chunk entities, pending mesh jobs, world camera, lights, overlays, readout, and tools to the session's lifetime. Clear session-scoped state and stop the worker on departure. Old worker updates or completed jobs cannot publish into a new session. Asset watching can remain application-wide, but route rule updates only to the current worker.
- Split application pause from the debug clock. A completed pause acknowledges the worker has reached a Tick boundary after earlier commands. Block new gameplay Edits, Focus changes, debug stepping, and hot-reload mutations while paused. Defer eligible reloads until resume. A developer load starts with saved assets and waits for subsequent changes before applying hot reload. Ignore startup watcher messages that would overwrite restored definitions.
- Establish an input policy before movement, tools, and UI dispatch. Esc freezes the World and reveals the cursor. Resume restores the prior debug clock state. Alt during gameplay frees the pointer for the debug panel, suspends movement input, look, and tool actions, and leaves physics and the debug clock running. Releasing Alt recaptures the cursor during focused gameplay. Gate pointer input on the same frame it is released and clear stale clicks and look motion on recapture. Keep the cursor free when the window loses focus.

## Seams and tests

1. Open, exit, and open another World in a headless app. Verify worker teardown and isolation of view, pending work, and simulation state.
2. Load a flying player far from the origin. The initial Focus and pose come from the save and the fresh-spawn placement never runs. Rebuilt meshes correspond to the loaded World.
3. Pause with a running clock and with the debug clock stopped. After acknowledgement, confirm Cells, Tick count, and player pose remain fixed. Debug Step and queued hot reload cannot change the paused World. Resume restores the prior debug state.
4. Keep Alt held while clicking the debug panel, then release it. Check that pointer input never becomes a tool action and that physics can still move a player who was already falling.

## Done when

- New Game and Load Game have distinct initialization paths that the next step can drive.
- Pause and cursor transitions are available to the menu, with session isolation verified headlessly.
