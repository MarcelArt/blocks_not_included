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

### Decisions made during implementation

- **Modules.**
  - `src/session.rs` (new): the `Session` state (`Closed`, `Open`), its `Play` sub-state (`Running`, `Pausing`, `Paused`), `WorldStart`, `LoadError`, the `OpenWorld` and `CloseWorld` commands, `PauseControl`, and `SessionPlugin`.
  - `src/input_policy.rs` (new): the `InputPolicy` resource, the `gameplay_input` run condition, and `InputPolicyPlugin`.
  - `src/sim_bridge.rs`: `SimStart` replaces the constructors of `SimBridgePlugin`, which is now a unit struct.
- **Opening a World.**
  - `SimStart::fresh`, `SimStart::generated` and `SimStart::restored` build what the simulation starts from. A restored capture is checked when its `SimStart` is built, so a bad capture never starts a worker. `SimStart` is `Send`, so step 19 can build it off the main thread.
  - `WorldStart { sim, player: Option<PlayerState> }` names the two paths:
    - `WorldStart::new_game(seed, elements, settings)`: `player: None`, so the player spawns on the ground.
    - `WorldStart::load_game(WorldSave)`: rebuilds the generator from the save's `Generation`, restores the capture, and keeps the saved player.
  - `commands.queue(OpenWorld(start))` sets the next `Session` to `Open`. Applied before `PreUpdate` ends, it takes effect that frame. Opening a World while one is open isn't supported; it logs a warning.
  - In `OnEnter(Session::Open)`, the `SessionSystems::Start` set runs first. It starts the worker and inserts `SimBridge`, `Elements`, `SimFocus`, `WorldView`, `SimReport`, `TickRate` and `PlayerStart`. Then `SessionSystems::Build` spawns the player, the sun and the meshes, and resets the palette, the debug clock and the file status.
- **The player.**
  - `AwaitingSpawn` is now an enum:
    - `OnGround`: the existing placement on the ground.
    - `AsSaved`: the player is held, exactly as saved, until the Chunk at the feet and its 26 neighbours are in `WorldView`. Neighbours beyond the world's vertical limits don't count. A generated world's Atmosphere fills every Chunk inside those limits, so all of them load.
  - While held, the player neither moves, nor looks around, nor moves the focus. The saved Focus is the checkpoint's, and releasing the player doesn't change it.
  - `PlayerState::new(body, rotation)` and `PlayerState::rotation()` convert between a camera rotation and the saved yaw and pitch, for step 19's saving.
- **Closing a World.**
  - The player and camera, the sun, and every Chunk and Overlay mesh carry `DespawnOnExit(Session::Open)`. The lamp is a child of the camera, so it goes with it.
  - `OnExit` removes the bridge and every World resource. Dropping the bridge shuts the worker down and joins it, so its channel and any updates still queued go with it.
  - `OnExit` also removes `ChunkMeshes` and `OverlayMeshes`, which drops and so cancels meshing under way. It resets `ShownOverlay`, so the background returns to the sky colour.
  - State that used to live in `Local`s and would have outlived a World now lives in World resources: the render-distance centre in `follow_focus` and the tick-rate baseline. The heat brush's stroke now ends before it holds, so a stroke left from earlier can't fire.
- **Run conditions.** Every system that reads a World resource runs only `in_state(Session::Open)`, or `in_state(Play::Running)` if it changes the World. Bevy evaluates every run condition on a system without short-circuiting, so conditions on World resources use `resource_exists_and_changed`.
- **Pause.**
  - The worker has a `frozen` flag, separate from the debug clock's `paused`. While frozen, no Tick runs and `Step` is ignored.
  - `SimCommand::Freeze(n)` first sends everything earlier commands changed, then answers `SimUpdate::Frozen { n, tick }` on the updates channel. By the time the answer arrives, the game has received those changes. `Unfreeze` leaves the debug clock as it was.
  - `SimBridge::freeze()`, `unfreeze()` and `frozen_at()` hold the protocol. An answer to a freeze that has been ended since is ignored.
  - `OnExit(Play::Running)` freezes the worker, `OnEnter(Play::Running)` unfreezes it, and `Pausing` becomes `Paused` once the answer arrives.
  - `PauseControl::pause`, `resume` and `toggle` ask for these transitions. `PauseControl::state()` accounts for transitions requested this frame, including a World about to open or close.
  - While not `Running`:
    - the player's systems, the tools, the debug clock keys and `follow_focus` don't run;
    - the debug panel greys out its clock buttons and says the World is paused;
    - the latest version of each changed file is held back.
- **Hot reload.**
  - The assets load and are watched for the whole app.
  - `send_new_versions` sends only to the World's bridge. It drops every version read in a World's first frame: asset events are written in `PostUpdate`, so those are from before the World opened.
  - A saved World therefore keeps its saved rules until a file changes during play, in development as well as release.
  - Several reloads in one frame now collapse into the latest version.
- **Input policy.**
  - `InputPolicy { gameplay }` is set in `PreUpdate`, after `InputSystems`. Gameplay input means the World will be `Running` this frame, the primary window has focus, and Alt isn't held.
  - The cursor is captured (`Locked`, hidden) exactly while `gameplay` holds, and free otherwise. That includes with no World open.
  - When `gameplay` turns on, the policy calls `ButtonInput<MouseButton>::reset_all()`, so a button still held reads as up until it is pressed again, and zeroes `AccumulatedMouseMotion`.
  - Esc toggles the pause unless egui wants the keyboard.
  - The old click-to-capture is gone.
  - Mouse look and the F toggle need gameplay input. Physics runs whenever the World runs, with empty input without it.
  - The tools use `InputPolicy`, not `CursorOptions`, for their buttons. egui windows are interactable when `gameplay` is false.
- **egui.** bevy_egui gives its primary context only to the first camera ever spawned, through a `Local`. So `main.rs` turns `auto_create_primary_context` off, and each World's camera carries `PrimaryEguiContext`.
- **Reports.** The worker sends a `SimReport` as it starts, and `SimReport` is inserted with the starting Tick count. A restored World therefore shows its Tick at once, and the tick rate doesn't spike from 0 to the restored count.
- **Startup.** Until step 19's menu, `main.rs` queues `OpenWorld` for `SimStart::sandbox()` at startup.

## Seams and tests

1. Open, exit, and open another World in a headless app. Verify worker teardown and isolation of view, pending work, and simulation state.
2. Load a flying player far from the origin. The initial Focus and pose come from the save and the fresh-spawn placement never runs. Rebuilt meshes correspond to the loaded World.
3. Pause with a running clock and with the debug clock stopped. After acknowledgement, confirm Cells, Tick count, and player pose remain fixed. Debug Step and queued hot reload cannot change the paused World. Resume restores the prior debug state.
4. Keep Alt held while clicking the debug panel, then release it. Check that pointer input never becomes a tool action and that physics can still move a player who was already falling.

Where they are:

- Tests 1–4: `tests/sessions.rs`, a headless app with the real plugins and no renderer. A stand-in primary window entity takes the cursor options. Keys and buttons go in as `KeyboardInput` and `MouseButtonInput` messages.
  - Test 1 also reopens the same saved World, to show that render distance reloads around a centre the last World used.
  - Test 2 also checks that the player is released only once the 27 Chunks around them have loaded.
  - Test 3 is two tests, one for each debug clock state. They send `Step` straight to the worker as well as pressing N.
  - Test 4 ends with a fresh click that does dig, so the test can tell a blocked tool from a broken one.
- Also in `tests/sessions.rs`: a loaded World keeps its saved settings when the file changes just before it opens, and takes up a change made during play.
- At the worker seam, in `tests/sim_bridge.rs`:
  - a freeze is answered after earlier Edits reach `WorldView`, holds off Ticks and Steps, and unfreezes to the debug clock as it was;
  - a restored World reports its Tick count from the start.
- Mutation checks, each reverted, turned the matching test red:
  - letting the worker Step while frozen;
  - letting versions from before opening through;
  - skipping the mouse reset;
  - keeping the render-distance centre across Worlds;
  - releasing a saved player at once.

Checked in the real game with a temporary harness, since removed:

- Esc paused on the same frame, with the cursor freed, and was acknowledged within one Tick. The Tick count then held for a second.
- The panel's clock buttons were greyed out while paused.
- Resume and releasing Alt each recaptured the cursor.
- Closing joined the worker and left no meshes or player.
- A capture of the World, loaded as a save, put the player back where they were, looking the same way, at the saved Tick.
- egui drew on the second World's camera.

## Done when

- [x] New Game and Load Game have distinct initialization paths that the next step can drive.
- [x] Pause and cursor transitions are available to the menu, with session isolation verified headlessly.
- Tests: 6 in `tests/sessions.rs`, and 2 more in `tests/sim_bridge.rs`.

### Deferred

- **A camera for the main menu.** With no World open there is no camera, so neither the 3D view nor egui draws anything. Step 19's menu needs a camera with `PrimaryEguiContext` while `Session::Closed`.
- **Saving.** Step 19 samples the player with `PlayerState::new(body, transform.rotation)` and the Focus before calling `SimBridge::capture`. A pause followed by a capture is ordered after the freeze, so the capture holds the paused World.
- **Building a `SimStart` off the main thread.** `load_game` restores the capture on the caller's thread, and `new_game` generates the Chunks above the origin's ground there. Step 19 can move either to a task.
- **Focus loss doesn't pause.** Losing focus frees the cursor and stops gameplay input, but the World keeps running.
