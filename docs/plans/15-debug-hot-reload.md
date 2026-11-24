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

### Decisions made during implementation

- **Modules.**
  - `src/sim_files.rs` holds the `SimFile` trait, the `ElementsFile` and `SettingsFile` assets, one generic `SimFileLoader<F>` used for both, the `SimFileStatus` resource and `SimFilesPlugin`.
  - `src/debug_panel.rs` holds `DebugPanelPlugin`, the `DebugPanel` resource (whether it is shown) and `SimClock` (whether the game has stopped the clock).
- **Development builds.** The game crate has a default `dev` feature that turns on `bevy/file_watcher`. Build a release with `--no-default-features`.
- **Startup.** `SimBridgePlugin::sandbox()` still reads both files synchronously, since the simulation and the world generator need them before the first frame. It now parses them with the same `SimFile::parse` the loaders use. Only later versions go through the assets: an `AssetEvent::Modified` is sent to the simulation, and the first load is not.
- **Loaders.**
  - Both files are `.ron`, so the loaders claim no extension, and the asset server chooses them by asset type.
  - The settings loader also runs `SimSettings::validate`, so an out-of-range value is reported when the file loads.
- **Answers from the simulation.**
  - The simulation thread answers each `ReplaceElements` and `ReplaceSettings`. The bridge passes the answer on as a `ReplaceOutcome` message.
  - `Elements` changes only once the simulation has accepted the table. Remeshing was already in place from steps 10–14: Chunk colours and Overlay looks both follow `resource_changed::<Elements>`.
- **`Simulation::replace_elements`.**
  - It rejects a table with `ElementError::MissingElement` (an Element removed or renamed) or `ElementError::ElementMoved` (an Element at another position).
  - Like `Simulation::new`, it doesn't validate the table: `ElementTable::from_ron` already has.
  - It wakes every active Chunk, as `set_settings` does.
  - Changing an existing Element's Phase counts as a changed property, so it is allowed.
- **Ticks per second.** A `TickRate` resource in `sim_bridge.rs` is measured over each second of real time. The five-second log line now reports it too, where it used to average over its own five seconds.
- **The panel.**
  - It sits in the bottom-left corner and starts hidden. When a file is rejected for a new reason, the panel opens.
  - The button that doesn't apply is greyed out: Pause while stopped, and Resume and Step while running.
  - The keys work while the cursor is captured. The buttons, like the palette's, need Esc first.
  - Mass shows in kilograms to the gram, so a change of one gram shows.

## Seams and tests

1. `crates/sim/tests/elements.rs`: `replace_elements` accepts changed properties and Elements appended at the end. It rejects a reordered, renamed or removed Element, and leaves the old table in place.
2. Game unit test: each `AssetLoader` parses the shipped file. A malformed file produces an error, not a panic.

Also pinned:

- In `src/sim_files.rs`: the asset server loads each shipped file with its own loader, and a file that fails to load shows the loader's own message in `SimFileStatus`.
- In `tests/sim_bridge.rs`: an accepted table reaches `Elements`, and a rejected one leaves it unchanged. Replaced settings are answered, and invalid ones are rejected.
- In `src/debug_panel.rs`: Mass shows in kilograms to the gram.

## Done when

- [x] The tests pass: 3 more in `crates/sim/tests/elements.rs`, 6 in `src/sim_files.rs`, 1 in `src/debug_panel.rs` and 2 more in `tests/sim_bridge.rs`.
- [x] **Human check:**
  - Raising Granite's conductivity in `elements.ron` while the game runs visibly speeds up heat spreading through rock within a second.
  - A typo in the file shows an error in the panel and the game keeps running.
  - Pause and Step work from the panel and the keys.
  - The agent checked these in the running game by editing the files and reading the log. The panel was checked with a temporary harness, since removed, that pressed P on `ButtonInput` and took a screenshot.
  - **Granite.** Conductivity went from 2.8 to 280. The simulation took up the new table 0.37 s after the file was written; 300 ms of that is the file watcher's wait.
  - **Typo.** `conductivty` was reported with its line and column, and the game kept running. The simulation kept the previous table.
  - **Rejected by the simulation.** Renaming Dirt to Soil was reported as `"Dirt" is gone, but Cells may hold it`. Restoring the file was taken up.
  - **Settings.** `conduction_multiplier: 2000.0` was taken up. `gas_evening_out_fraction: 0.9` was rejected with its allowed range. Restoring the file was taken up.
  - **Panel.** A rejected file opened the panel, which showed the error in red, wrapped within the panel. After P, the clock read Stopped, Ticks per second fell to 0.0, Pause was greyed out, and Resume and Step were enabled.
  - **Not checked by the agent:** seeing heat spread faster, F3, N, and clicking the buttons with the real mouse.
  - The human hasn't checked it yet.

### Deferred

- **A second error line.** When a reload fails, Bevy retries it without the asset type. The loaders claim no extension, so the retry logs `Could not find an asset loader matching: Asset Type: None`. It is harmless, and the panel shows only the loader's message. Claiming `.ron` would be worse: the settings loader would then retry `elements.ron`.
- **The world generator keeps its table.** It was made with the Elements of startup. A Chunk generated after a reload starts with the old Phase Transitions applied. The simulation corrects that Chunk on its first Tick in the Active Region.
- **The palette's Mass.** The Place and Spawn fluid Cells keep the Mass from the Density at the time they were chosen, until chosen again.
- **The first version.** An edit made between startup's own read and the first asset load is not sent to the simulation, since only reloads are.

## Scope boundary

The Sim Sandbox milestone ends here. Survival, saving to disk, falling solids and textures belong to the next milestone.
