# Step 15: Debug panel and hot reload

> Read `docs/plans/CONVENTIONS.md` and `docs/plans/BEVY-0.20-NOTES.md` first.

**Prerequisites:** step 14 is done.

## Goal

A debug panel shows how the simulation is doing and controls its clock. Editing `assets/elements.ron` or `assets/sim_settings.ron` while the game runs changes the simulation within a second. A broken edit shows an error and leaves the game running. This step completes the Sim Sandbox milestone.

## Design

- **Debug panel** (egui, toggled with F3). It shows:
  - Ticks per second achieved and the last Tick's duration;
  - total Mass, and Mass lost to space;
  - active, Settled and Dormant Chunk counts;
  - the Tick count.
  
  It has Pause, Resume and Step buttons, also on the keys P (pause/resume) and N (step).
- **Hot reload.**
  - Enable Bevy's `file_watcher` feature for development builds.
  - Load both RON files as assets through custom `AssetLoader`s that call `ElementTable::from_ron` and parse `SimSettings`.
  - When a file changes, send `SimCommand::ReplaceElements(ElementTable)` or `SimCommand::ReplaceSettings(SimSettings)`.
- **Changes in `sim`:**
  - `Simulation::replace_elements(table) -> Result<(), ElementError>`. Cells store `ElementId`s, so the new table must keep every existing Element name at the same index. Appending new Elements is allowed; renaming, removing or reordering is rejected.
  - `Simulation::set_settings` already exists from step 05.
- **Failures.** A file that fails to parse or validate, or a rejected reload, shows the error message in the debug panel and in the log. The simulation keeps using the previous table or settings.
- **Colours.** After a successful Elements reload, update the `ElementTable` resource and remesh every Chunk so colours update.

## Seams and tests

1. `crates/sim/tests/elements.rs`: `replace_elements` accepts changed properties and Elements appended at the end. It rejects a reordered, renamed or removed Element, and leaves the old table in place.
2. Game unit test: each `AssetLoader` parses the shipped file. A malformed file produces an error, not a panic.

## Done when

- [ ] The tests pass.
- [ ] **Human check:**
  - Raising Granite's conductivity in `elements.ron` while the game runs visibly speeds up heat spreading through rock within a second.
  - A typo in the file shows an error in the panel and the game keeps running.
  - Pause and Step work from the panel and the keys.

## Scope boundary

The Sim Sandbox milestone ends here. Survival, saving to disk, falling solids and textures belong to the next milestone.
