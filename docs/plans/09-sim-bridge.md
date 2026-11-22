# Step 09: Sim thread bridge

> Read `docs/plans/CONVENTIONS.md` and `docs/plans/BEVY-0.20-NOTES.md` first, then `docs/adr/0003-bevy-free-background-sim.md`.

**Prerequisites:** steps 01–08 are done.

## Goal

The game runs the `Simulation` on a background thread at 5 Ticks per second. It sends the simulation commands, receives changed Chunks and stats, and keeps a game-side copy of the world that every later game step reads from.

## Starting point

- `src/main.rs` builds an `App` with `DefaultPlugins` and declares an empty `src/world/` module (`mod.rs`, `components.rs`). Put the game-side world view in `src/world/`.
- Put the bridge in `src/sim_bridge.rs` as `SimBridgePlugin`.

## Design

- **Startup.**
  1. Parse `assets/elements.ron` with `ElementTable::from_ron`.
  2. Parse a new `assets/sim_settings.ron`, holding `SimSettings` with its defaults written out, except `conduction_multiplier: 1000.0` as a first guess to tune later.
  3. Build a `worldgen::Generator` with a `WORLD_SEED` constant.
  4. Spawn a `std::thread` that owns the `Simulation`.
  5. Store the `ElementTable` as a resource too, for names and colours.
- **Commands to the sim thread:**
  ```rust
  enum SimCommand { Edit(Edit), Focus(CellPos), LoadAround { center: ChunkPos, radius: i32 }, Pause, Resume, Step, Shutdown }
  ```
- **Sim thread loop.**
  - Tick every 200 ms against a deadline. If a Tick overruns, run the next one immediately, but never let missed Ticks pile up.
  - Drain all pending commands before each Tick.
  - After each Tick, and after loads, send a `ChunkUpdate { pos: ChunkPos, cells: Arc<[Cell]> }` for every Chunk from `take_changed_chunks`, and for every newly loaded Chunk.
  - After each Tick, also send a `SimReport { tick, tick_duration, stats }`.
- **Channels.** Use `std::sync::mpsc` or `crossbeam-channel`; choose one and justify it in a comment.
- **Game side.**
  - A system early in the frame drains updates into a `WorldView` resource (`HashMap<ChunkPos, Arc<[Cell]>>`), plus a per-frame record of which Chunks changed, for renderers to read.
  - Keep the latest `SimReport` as a resource too.
- **Focus.** A `SimFocus(CellPos)` resource. The bridge sends `Focus` whenever it changes. Until step 11, it stays fixed just above the surface at the origin.
- **Render distance.** A constant, for example 6 Chunks horizontally and 3 vertically. The bridge sends `LoadAround` when the focus Chunk changes, so Dormant Chunks are available to draw.
- **Shutdown.** Send `Shutdown` on `AppExit` and join the thread.
- **Logging.** Log achieved Ticks per second and stats with `info!` every 5 seconds.

## Seams and tests

**Seam:** `SimBridgePlugin` in a headless `App` (`MinimalPlugins`), configured with a tiny `ElementTable` and an all-Vacuum source. Make the plugin constructible from those parts so tests don't need the asset files. Use generous timeouts, since threads are involved.

1. An `Edit::Place` sent through the bridge shows up in `WorldView` within a few Ticks.
2. `Pause` stops the Tick count from advancing, and `Step` advances it by exactly one.
3. Dropping the `App` shuts the sim thread down; the test finishes without hanging.

## Done when

- [ ] All three tests pass.
- [ ] `cargo run` starts, logs about 5 Ticks/s with stats, and exits cleanly when the window is closed.

## Scope boundary

This step ends with data flowing. Nothing is drawn yet; drawing starts in step 10.
