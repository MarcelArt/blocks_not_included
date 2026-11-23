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

### Decisions made during implementation

- **Channels.** `crossbeam-channel` 0.5.17, which Bevy already depends on. Its `Receiver` is `Sync`, so it sits in the `SimBridge` resource without a `Mutex`, and its `recv_deadline` is stable; std's isn't.
- **Render distance.** `RENDER_DISTANCE` reaches 6 Chunks horizontally and 3 vertically either way: 13 × 13 × 7 = 1,183 Chunks. `LoadAround.radius` is a `RenderDistance { horizontal, vertical }`, since a single `i32` can't hold both.
- **Loading between Ticks.** The sim thread loads render distance one Chunk at a time, nearest the focus first, while it waits for the next Tick. It loads at least one Chunk per Tick even when Ticks overrun. On this laptop all 1,183 Chunks took about 15 s in a dev build, and the clock held 5.0 Ticks/s throughout.
- **Chunks leave `WorldView`.** The sim thread tracks which Chunks the game holds. On `LoadAround`, Chunks beyond the new box leave `WorldView` and are listed in `WorldView::removed()` for that frame, which step 10 uses to despawn them. Changes beyond render distance aren't sent; a Chunk is sent whole when it comes back.
- **Game crate API.** There is a new `src/lib.rs`; `main.rs` only builds the app.
  - `sim_bridge`: `SimBridgePlugin::new` (for tests) and `SimBridgePlugin::sandbox` (the asset files and `WORLD_SEED`), `SimBridge::send(SimCommand)`, `SimReport`, `SimFocus`, `Elements` (the `ElementTable` resource, which derefs to it), `RENDER_DISTANCE`, `TICK_INTERVAL`.
  - `world::WorldView`: `chunk`, `cell`, `chunks` and `len`, plus the per-frame record `changed()` and `removed()`.
  - Schedules: updates are drained in `First`. `SimFocus` is sent in `PostUpdate`, so a change made in `Update` goes out the same frame. Shutdown on `AppExit` runs in `Last`.
- **Commands.** `Step` runs a Tick only while paused. `Resume` restarts the clock one interval later. Chunks changed by an Edit are sent straight away, so Edits show while paused.
- **Initial focus.** Just above the highest solid or liquid Cell at x = z = 0, found by generating that column's Chunks from the top down. If there is none, it is at y = 0.
- **Dev profile.** `sim` and `worldgen` are now built at opt-level 3 in dev builds too, because the `"*"` override skips workspace members. A Tick of the generated world went from about 110 ms to 90 ms under `cargo run`.
- **Memory.** About 1.3 GB resident once render distance is full: roughly 465 MB of Chunks in the simulation, and the same again in `WorldView`. A later change could share them by having `sim` store Chunks as `Arc<[Cell]>` with copy-on-write.

## Seams and tests

**Seam:** `SimBridgePlugin` in a headless `App` (`MinimalPlugins`), configured with a tiny `ElementTable` and an all-Vacuum source. Make the plugin constructible from those parts so tests don't need the asset files. Use generous timeouts, since threads are involved.

1. An `Edit::Place` sent through the bridge shows up in `WorldView` within a few Ticks.
2. `Pause` stops the Tick count from advancing, and `Step` advances it by exactly one.
3. Dropping the `App` shuts the sim thread down; the test finishes without hanging.

Added at the same seam, in `tests/sim_bridge.rs`: the initial focus sits just above the surface; Chunks leave `WorldView` beyond render distance and arrive within it; an `AppExit` shuts the thread down. A unit test in `src/sim_bridge.rs` checks that `assets/sim_settings.ron` holds the defaults except `conduction_multiplier`.

## Done when

- [x] All three tests pass.
- [x] `cargo run` starts, logs about 5 Ticks/s with stats, and exits cleanly when the window is closed. Checked under X11 by closing the window with `xdotool windowquit`: 5.0 Ticks/s, then "the simulation thread has shut down" and a clean exit.

## Scope boundary

This step ends with data flowing. Nothing is drawn yet; drawing starts in step 10.
