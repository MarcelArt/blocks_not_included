# Step 16 — Simulation checkpoint

Read [CONVENTIONS.md](CONVENTIONS.md), [ADR 0005](../adr/0005-world-saves-store-chunk-overrides.md), and [the phase scope](../phase-2a-plans.md).

**Prerequisites** — Steps 01–15.

## Goal

Capture the authoritative simulation between Ticks and restore it so that the next Tick under the same rules and inputs produces the same result.

## Design

- Extend the public seam in `crates/sim/src/simulation.rs` for capture and restore using a supplied `ChunkSource`. Keep filesystem, Bevy, and save format decisions outside `sim`. Use the World Seed and the original validated generator inputs when rebuilding `worldgen::Generator`. Allow startup with a supplied seed while preserving current sandbox startup until the menu step.
- Capture the Focus, actual Active Region placement used for hysteresis, Tick count, Settled status, and cumulative Mass lost to space. Restore these directly. The normal constructor loads the default origin region, and runtime setters can recenter or wake the Active Region.
- Capture complete Cells for Chunks whose contents differ from the generation baseline. Include differences from Ticks and Edits, whether active, Dormant, or beyond render distance. Track candidates separately from the change queue drained by `src/sim_bridge.rs`. Once a candidate exactly matches its original generated Cells, its saved override can be removed. Preserve Mass as whole grams and temperature at its original precision.
- Distinguish an explicit all-Vacuum override from a baseline Chunk that can be regenerated. Preserve the information needed to restore active membership and Settled state even for pristine Chunks. Rebuild render caches from the restored simulation and keep render-cache history out of the continuation contract.
- Expose capture through an ordered worker request. Capture after earlier Focus and Edit commands have been handled. Return owned state for encoding and disk writes outside the game update schedule and outside the Tick. Restore only after validating the checkpoint against the supplied rules and source, including Chunk coordinates, Cell counts, and Element identities.
- Keep the original generator Element definitions and the last accepted live Element table and simulation settings available to the game-side save layer. Read the accepted state from the worker at the same boundary as the Cells. The current source retains startup generation inputs after a live rules reload.

### Decisions made during implementation

- **`sim` API.**
  - `Simulation::checkpoint(&self) -> Checkpoint` and `Simulation::restore(elements, settings, source, checkpoint) -> Result<Simulation, CheckpointError>`.
  - Also `Simulation::elements()` and `Simulation::focus()`.
  - Types are in `crates/sim/src/checkpoint.rs`.
- **What a `Checkpoint` holds.**
  - Plain data with public fields, so the game chooses the save format. It derives no serde traits.
  - `element_names` lists the capture's Element table in id order, starting with Vacuum. Restore checks every name against the supplied table, so Element identities are checked by name as well as by index. The supplied table may append Elements, as `replace_elements` allows.
  - `focus`, plus `placement_min` and `placement_chunks` for the Active Region's box before the world limits cut it off.
  - `tick_count` and `mass_lost_to_space_grams`.
  - `chunks: BTreeMap<ChunkPos, ChunkCheckpoint>`. Each entry has `contents` and `settled`. `contents` is one of:
    - `Generated`: no payload; the source regenerates it.
    - `Vacuum`: an explicit all-Vacuum override.
    - `Cells(Vec<Cell>)`.
- **Which Chunks are listed.**
  - Every Chunk stored in the Active Region, even a pristine one, so active membership and Settled state survive.
  - A Chunk of the Active Region that isn't listed holds nothing but Vacuum, as generated. Restore asks the source for it, and rejects the checkpoint with `MissingChunk` if the source puts matter there.
  - Dormant Chunks only when they differ from generation.
  - Pristine Dormant Chunks are left out, including those loaded only for render distance. They reload from the source on demand, as never-loaded Chunks do. Until then, a restored simulation's `dormant_chunks` and `total_mass_grams` are lower, and their Cells read as Vacuum. Restoring them eagerly would cost about 8 ms each, for roughly a thousand Chunks in render distance.
- **Candidates.**
  - Each Chunk has a sticky `modified` flag, set in `Chunk::write`, through which every Cell change goes. Nothing outside the simulation clears it, so draining `take_changed_chunks` leaves it alone.
  - Each generated Chunk also keeps a 64-bit fingerprint of its generated Cells.
  - At capture, a modified Chunk counts as `Generated` again only if its fingerprint matches and regenerating it confirms an exact match, bit for bit, temperatures included.
  - Regenerating every candidate would cost about 8 ms per Chunk: roughly 250 ms for a busy region. With fingerprints, a capture of 32 modified Chunks takes about 15 ms.
  - Chunks restored from `Cells` or `Vacuum` have no known baseline, so they stay overrides.
- **Settled state.** Only active Chunks carry it. A Dormant Chunk is woken when it enters the Active Region, so capture writes `false` for it, and restore rejects `true` outside the Active Region.
- **Validation.** Restore checks the checkpoint before it builds anything:
  - the settings, and the Element names;
  - the box size against `active_region_chunks` (`RegionSize`). A different region size is a rule mismatch, not something to adapt to;
  - that following the focus would keep the box where it is (`RegionAwayFromFocus`);
  - that every Chunk lies within the world, with `CHUNK_SIZE³` Cells;
  - that every Cell's Element index is named, and that Vacuum has zero Mass and every other Cell at least 1 g.
  Temperatures aren't range-checked; they are kept bit for bit.
- **Render caches.** A restored simulation lists every stored Chunk as changed, so the bridge redraws from it.
- **Game (`src/sim_bridge.rs`).**
  - `SimCommand::Capture(Sender<SimCapture>)`, sent by `SimBridge::capture() -> Receiver<SimCapture>`. The worker answers in command order, between Ticks, on the caller's own channel. A save task can wait for it off the Bevy schedule.
  - `SimCapture` holds:
    - `checkpoint`;
    - `elements` and `settings`: the last accepted rules, read from the simulation at the same boundary as the Cells;
    - `generation: Option<Generation>`.
  - `Generation { seed, elements }` holds the generator's startup inputs; `Generation::generator()` rebuilds the generator. The worker keeps a copy, and reloads never touch it.
  - `SimBridgePlugin::generated(seed, elements, settings)` starts a fresh generated world from any seed. `sandbox()` now calls it with `WORLD_SEED`, so startup is unchanged.
  - `SimBridgePlugin::restore(capture, source) -> Result<_, CheckpointError>` builds and validates the simulation on the caller's thread. A rejected capture never starts a worker. The focus starts at the checkpoint's focus. Step 18 will move this out of plugin construction.
- **worldgen.** No code change was needed: `Generator::new(seed, &elements)` already rebuilds from the inputs, and the game keeps them. A test pins restoring a generated world with the startup Elements while the simulation runs under a reloaded table.

## Seams and tests

1. Round-trip Cells changed by an Edit and by a Tick, including a Dormant Chunk beyond the current view. Draining view changes before capture leaves the checkpoint intact.
2. Dig a generated Chunk entirely to Vacuum, capture and restore it, then reactivate it. It stays Vacuum. An untouched generated Chunk has no Cell payload.
3. Restore a checkpoint taken after moving Focus within the hysteresis band. Use the same subsequent commands and compare the next Tick's Cells, Active Region, and counters with uninterrupted play. Include a nonzero Tick count and a Settled Chunk.
4. Apply a valid Element or settings reload and reject an invalid follow-up. Capture includes the last accepted rules and the same generator baseline. Restore preserves both.
5. Reject malformed checkpoint metadata or Cells before publishing the restored simulation. Verify an earlier queued Edit is included and a later command cannot mutate a checkpoint already returned.

Where they are:

- Tests 1–5 at the simulation seam: `crates/sim/tests/checkpoint.rs`. That file also pins that a Chunk changed and changed back is no longer saved.
- On generated terrain: `crates/worldgen/tests/checkpoint.rs`.
- At the worker seam (tests 3–5 and capture ordering): `tests/sim_bridge.rs`.

## Done when

- [x] Same-rules continuation is verified through the public simulation and worker seams.
- [x] Capture includes simulated changes and explicit Vacuum overrides and omits unchanged terrain payloads.
- Tests: 6 in `crates/sim/tests/checkpoint.rs`, 1 in `crates/worldgen/tests/checkpoint.rs`, and 4 more in `tests/sim_bridge.rs`.

### Deferred

- **Decoding Cells.** `ElementId` has no public constructor, so a save layer can't yet rebuild a checkpoint's Cells from stored indices. Step 17 needs one, such as `ElementId::from_index`, or a lookup through the table. `Simulation::restore` already rejects unknown indices. *Resolved in step 17: `ElementTable::id_at`.*
- **Serialising the rules.** `ElementTable`, `Element` and `SimSettings` deserialise but don't serialise, so step 17 needs `Serialize` or its own encoding to store development rules. *Resolved in step 17: `Serialize` and `ElementTable::to_ron`.*

The next step encodes and commits saves to disk.
