# Step 17 — World save storage and catalog

Read [CONVENTIONS.md](CONVENTIONS.md), [ADR 0005](../adr/0005-world-saves-store-chunk-overrides.md), and [the phase scope](../phase-2a-plans.md).

**Prerequisite** — Step 16.

## Goal

Create, list, read, update, rename, and delete independent World saves through a headless game-side storage API.

## Design

- Resolve per-user paths in one game-side module shared later by Settings. On Linux use `$XDG_DATA_HOME` for saves and `$XDG_CONFIG_HOME` for settings. Respect their default locations. On Windows use Local App Data, with distinct saves and settings locations. Use `blocks_not_included.dev` when the existing `dev` feature is enabled and `blocks_not_included` otherwise. Pass temporary roots to tests. [XDG Base Directory Specification](https://specifications.freedesktop.org/basedir/) · [Windows known folders](https://learn.microsoft.com/en-us/windows/win32/shell/knownfolderid)
- Give each World a stable, generated path-safe ID. Use the display name only as metadata. Trim the name and require a nonempty result. List Worlds by last-played time, most recent first, with deterministic ordering for ties. List entries without decoding their Cell payloads. A damaged entry can report its own error while healthy Worlds remain usable.
- Store the World Seed, display name, last-played time, and game mode `sandbox`. Store a versioned checkpoint and the player's feet position, camera yaw and pitch, velocity, movement mode, and state such as grounded status needed to resume movement. Track the save format, generation rules, and simulation rules explicitly so each incompatibility can be reported. Preserve exact Cell values, including Element identity and temperature precision.
- Release saves require compatible generation and simulation rules. Development saves also carry the validated generator Element definitions and accepted live rules captured in step 16. Restore those definitions even if the asset files have since changed. Reject unsupported format or rule revisions. Treat compatibility as a property of the actual rules and format so an unrelated code change can remain compatible.
- Publish each save atomically. Stage data before publishing it and prove that a failed or interrupted replacement preserves the last committed version on Linux and Windows. Serialize writes for a World and avoid letting a stale completion overwrite a newer save. Keep incompatible and corrupt saves untouched when reading them.
- Rename updates display metadata on the same ID without replacing terrain or bumping last-played time. Delete targets that ID and is permanent. The menu owns confirmation and error presentation.

### Decisions made during implementation

- **Where files go: `src/user_dirs.rs`.**
  - `UserDirs { saves, settings }`. `UserDirs::for_this_build()` reads the real system. `UserDirs::xdg(env, build)` and `UserDirs::windows(local_app_data, build)` are pure, so tests pass their own environment.
  - Linux: saves in `$XDG_DATA_HOME/<folder>/saves`, settings in `$XDG_CONFIG_HOME/<folder>`. An unset, empty or relative variable falls back to `~/.local/share` or `~/.config`. `$HOME` must be absolute when a default is needed.
  - Windows: `<Local App Data>/<folder>/saves` and `<Local App Data>/<folder>/settings`, from `SHGetKnownFolderPath` through the `known-folders` crate (a Windows-only dependency).
  - `<folder>` is `blocks_not_included.dev` for `Build::Development`, the `dev` feature, and `blocks_not_included` for `Build::Release`.
- **The store: `src/saves/`.**
  - `WorldStore::open(dir, RulesPolicy)`, then `list`, `create(name, &WorldSave, now)`, `load(id)`, `ticket(id)`, `save(ticket, &WorldSave, now)`, `rename(id, name)`, `delete(id)` and `path_of(id)`. It is `Clone`, and every method can be called from any thread.
  - `WorldSave { capture: SimCapture, player: PlayerState }`, where `PlayerState { body: Body, yaw, pitch }`. The capture's `generation` must be set (`StoreError::NotGenerated`), and the seed comes from it.
  - Times are passed in, so tests control them. Last-played times are kept to the millisecond.
  - One `StoreError` enum covers every operation, with a `Display` the menu can show.
- **One file per World**, `<id>.world`. A `WorldId` is 16 lowercase hex digits from a random `u64`. Names are trimmed, must not be blank, and never reach a path.
- **File layout** (`src/saves/format.rs`, `FORMAT = 1`):
  - magic `BNIWORLD`, the format revision, and the header length;
  - a RON header: name, seed, last-played time, mode `sandbox`, both rules revisions, and the body's length and CRC-32;
  - a zlib body: the generator Elements, live Elements and settings as RON text, then the player, then the checkpoint. Cells are stored a field at a time: indices, then Masses, then temperature bits. Every float is stored by its bits.
- **Catalog.** `list` reads headers only. It checks that each file's length matches its header, so a truncated save shows as damaged in the list. Damage to the Cells shows on load, through the CRC. Order: last played first, then name, then id. Unreadable entries come last, by id. Files that aren't `<id>.world` are ignored.
- **Committing.**
  - Each write stages `.<id>.<random>.staged` beside the save, flushes it, renames it over `<id>.world`, then flushes the folder. Folders can be flushed only on Unix.
  - A failure removes the staged file. Staged files left by a crash are removed at that World's next write or delete.
  - `create` publishes the first save, so a World exists only once that save commits.
  - `WorldStore::before_publish(hook)` runs between staging and publishing. Tests use it to fail or hold a write there.
- **Ordering.**
  - Within a store, writes to one World take turns behind a per-World lock.
  - `ticket` hands out increasing numbers. `save` commits only a ticket newer than the last one committed, otherwise it returns `Superseded`. It checks before encoding and again inside its turn.
  - A save keeps the committed name, so a save sampled before a rename doesn't undo it. It refuses another seed (`SeedMismatch`) and a deleted World (`NotFound`).
  - Turns don't extend across stores, so open one store per folder.
- **Compatibility.**
  - New constants, both 1: `sim::RULES_REVISION` (what a checkpoint means) and `worldgen::RULES_REVISION` (which Cells a seed generates). `the_generated_terrain_is_pinned_to_the_rules_revision` pins worldgen's to a fingerprint of five generated Chunks.
  - A format or revision mismatch is its own `Incompatibility` variant. Revision mismatches show in the list as well as on load.
  - Every save, development or release, stores the generator Elements, the live Elements and the settings. `RulesPolicy::AsSaved`, for development, restores them as saved. `RulesPolicy::Shipped { elements, settings }`, for release, requires all three to equal what the build ships. Otherwise it returns `GeneratorElements`, `SimulationElements` or `SimulationSettings`.
  - Storing the rules in release saves too lets a release compare the actual rules rather than rely on a revision someone remembered to bump. Code changes that keep both revisions keep saves compatible.
- **`sim` additions.** `Serialize` on `Element`, `Phase`, `Transition` and `SimSettings`. `PartialEq` on `Element` and `ElementTable`. `ElementTable::to_ron()`, which `from_ron` reads back equal. `ElementTable::id_at(index)`, which decodes stored Cells. These resolve step 16's deferred items. In the game, `SimCapture` and `Generation` derive `PartialEq`.
- **Cost.** A busy World with 32 changed Chunks (10 MB of Cells) is a 1.5 MB file. In the dev profile it writes in about 0.3 s and loads in about 0.2 s. Listing takes well under 1 ms. Step 19 should encode and write off the main thread.

## Seams and tests

1. Create two Worlds with the same name and seed. Change and rename one, then delete it. The other World still has its original metadata and Cells. Creation validates names, and display text cannot select a path.
2. Load fresh and changed saves into the same in-memory values, including a flying player and a Vacuum override. Verify developer rule round-trips and rejection of unsupported versions without a write.
3. Force a write failure after staging but before publication. Reopen storage and load the earlier complete save. Also test a failed first write, stale write ordering, and a corrupt entry beside a healthy one.
4. Verify sort order and root resolution with overrides and defaults, including independent development and release directories. Exercise the actual replacement path on Linux and Windows.

Where they are:

- Tests 1–4 at the store seam: `tests/world_store.rs`. It also covers seed and generator guards, saving after delete, release rule checks, and listing only save files.
- Root resolution: `tests/user_dirs.rs`.
- Rules round trips and `id_at`: `crates/sim/tests/elements.rs`.
- The generation revision pin: `crates/worldgen/tests/generation.rs`.

## Done when

- [x] The World store survives reopening and returns enough metadata for the list without loading the simulation.
- [x] Failed or incompatible loads leave committed data intact. No menu or in-game saving is required in this step.
- Tests: 17 in `tests/world_store.rs`, 6 in `tests/user_dirs.rs`, 3 in `crates/sim/tests/elements.rs`, and 1 in `crates/worldgen/tests/generation.rs`.

### Deferred

- **Windows run.** The store tests use the real file system and no platform-specific code, so on Windows they exercise its `fs::rename` replacement path. So far they have run only on Linux. Run `cargo test --test world_store --test user_dirs` on Windows to cover it.
- **Transient Windows locks.** A rename that fails because another program, such as an antivirus scanner, briefly holds the save is reported as an error, not retried. The committed save stays intact.
