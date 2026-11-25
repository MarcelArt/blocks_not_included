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

## Seams and tests

1. Create two Worlds with the same name and seed. Change and rename one, then delete it. The other World still has its original metadata and Cells. Creation validates names, and display text cannot select a path.
2. Load fresh and changed saves into the same in-memory values, including a flying player and a Vacuum override. Verify developer rule round-trips and rejection of unsupported versions without a write.
3. Force a write failure after staging but before publication. Reopen storage and load the earlier complete save. Also test a failed first write, stale write ordering, and a corrupt entry beside a healthy one.
4. Verify sort order and root resolution with overrides and defaults, including independent development and release directories. Exercise the actual replacement path on Linux and Windows.

## Done when

- The World store survives reopening and returns enough metadata for the list without loading the simulation.
- Failed or incompatible loads leave committed data intact. No menu or in-game saving is required in this step.
