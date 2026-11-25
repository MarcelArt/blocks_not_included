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

## Seams and tests

1. Round-trip Cells changed by an Edit and by a Tick, including a Dormant Chunk beyond the current view. Draining view changes before capture leaves the checkpoint intact.
2. Dig a generated Chunk entirely to Vacuum, capture and restore it, then reactivate it. It stays Vacuum. An untouched generated Chunk has no Cell payload.
3. Restore a checkpoint taken after moving Focus within the hysteresis band. Use the same subsequent commands and compare the next Tick's Cells, Active Region, and counters with uninterrupted play. Include a nonzero Tick count and a Settled Chunk.
4. Apply a valid Element or settings reload and reject an invalid follow-up. Capture includes the last accepted rules and the same generator baseline. Restore preserves both.
5. Reject malformed checkpoint metadata or Cells before publishing the restored simulation. Verify an earlier queued Edit is included and a later command cannot mutate a checkpoint already returned.

## Done when

- Same-rules continuation is verified through the public simulation and worker seams.
- Capture includes simulated changes and explicit Vacuum overrides and omits unchanged terrain payloads.

The next step encodes and commits saves to disk.
