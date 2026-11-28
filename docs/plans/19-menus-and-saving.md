# Step 19 — Main menu and saving in play

Read [CONVENTIONS.md](CONVENTIONS.md), [BEVY-0.20-NOTES.md](BEVY-0.20-NOTES.md), and [the phase scope](../phase-2a-plans.md).

**Prerequisites** — Steps 17 and 18.

## Goal

Boot to the main menu, create or manage a World, and save progress through the normal play and quit flows.

## Design

- The main menu has **New Game**, **Load Game**, and **Exit**. The pause screen has **Resume**, **Save**, and **Return to Main Menu**. Step 20 adds **Settings** to both menus. Render the menu without starting a World or showing its gameplay HUD.
- **New Game** asks for a name and an optional seed. Accept a decimal seed in the existing `u64` range and show validation errors inline. Blank generates a random seed. Display the chosen seed after creation in the pause menu and in the World list. A World is ready to play once its initial save commits. The initial save uses the resolved spawn and initial simulation state.
- **Load Game** displays name, seed, and last-played time in catalog order. Select rows by World ID. Report load errors and leave the player at the list. Rename refreshes the same row. Delete requires confirmation showing the selected World's name, seed, and last-played time. Keep a player in the list if a management operation fails.
- Feed manual Save, autosave, and exit saving through one coordinator. Sample player pose together with the Focus that precedes the worker's checkpoint request. Serialize requests so a newer save cannot commit behind an older one. Show pending writes, success, and failures without blocking the render thread.
- Count unpaused play time using the selected autosave interval. Use one minute until step 20 wires in preferences. Reset the timer on successful save and avoid retrying a failed autosave every frame. Preserve the previous save on failure and keep play available.
- Save when returning to the main menu or closing the window normally. Keep the World open and paused until a successful commit, then take the requested transition. On write failure offer **Retry**, **Cancel**, and **Leave Without Saving**. If an earlier in-play save is still pending, the final checkpoint must cover the state at exit. A canceled exit returns to the World or pause screen it came from.

### Decisions made during implementation

- **Modules.**
  - `src/lifecycle/` (new): the seam. `mod.rs` has `Library`, `WallClock`, the `MenuAction` message, `MainMenu`, `CurrentWorld`, `Leaving` and `LifecyclePlugin`. `saving.rs` has `Saving`, `SaveStatus`, `SaveReason` and `Autosave`.
  - `src/menu.rs` (new): `MenuPlugin`, the egui pages, the main menu's camera, and the note of each save during play. It only draws what the seam holds and sends `MenuAction`s.
  - `main.rs` no longer opens a World. It inserts the `Library`: the store in `UserDirs::saves`, with `RulesPolicy::AsSaved` for the `dev` build and `RulesPolicy::Shipped` (from `assets/`) for release. If there is nowhere to save, the menu says why, and New Game and Load Game report it.
- **The seam.** A menu sends `MenuAction`s, which `LifecyclePlugin` handles in `PreUpdate`, after the input policy. A World opened or closed there changes state that same frame. What became of each action is in resources:
  - `MainMenu { worlds, selected, confirming_delete, job, problem }`. `worlds` holds `CatalogRow { id, world: Result<WorldSummary, String> }` in the catalog's order.
  - `CurrentWorld(WorldSummary)`: the World open from the store, updated by each save. Only such a World saves; one opened with `OpenWorld` directly, as step 18's tests do, closes and quits without saving.
  - `Saving { last: SaveStatus, … }`, `Autosave { interval, since_save }`, and `Leaving(Option<Departure { to, back_to, failed }>)`.
- **Main menu jobs.** Listing, creating, loading, renaming and deleting each run as one task on the async compute pool, one at a time. While one runs, actions that would start another are ignored. A job that fails sets `MainMenu::problem` and changes nothing else: the list, the selection and every save stay as they were. Rows are selected by `WorldId`. A rename replaces its row in place, and a delete removes it. The list is read again whenever the main menu shows.
- **New Game.**
  - The form's seed goes through `parse_seed`: blank is a random seed, and otherwise a decimal `u64`. The errors (`SeedError`) show inline, and Create waits for a valid seed and a name.
  - The rules come from `assets/` as they are when the World is created (`sim_files::read_rules`). `SimStart::sandbox` and `WORLD_SEED` are gone.
  - `WorldStart::new_game(seed, elements, settings)` now returns the start and its first save. It finds the player's ground with the same rule `place_on_ground` uses (`player::ground_below` over `player::ground_column`), from generated Chunks, and moves the focus to the player's feet. `SimStart::capture_before_start` then builds the simulation on that thread, focuses it, and captures it before its first Tick. The simulation thread starts with exactly that simulation (`Start::Built`). So the first save and the World that opens are the same, and a new World opens like a loaded one, held until the 27 Chunks around the player load.
  - `WorldStore::create` commits that save before the World opens. If it fails, no World opens and none exists.
- **Saving in play** (`saving.rs`).
  - Each save request becomes `Saving::waiting`. Requests made while one waits merge into one, keeping the weightier reason: leaving, then manual, then autosave.
  - In `PostUpdate`, after the new `SendFocus` set (`follow_focus`), a waiting save starts if none is under way. It samples `PlayerState::new(body, rotation)`, takes the store's ticket, records the wall clock as the last-played time, and sends `SimBridge::capture`. The capture is handled after the focus the frame ended with.
  - Later frames poll the capture with `try_recv`, then encode and write it on the async compute pool, polled with `check_ready`. Nothing waits on the main thread. One save runs at a time, so tickets are taken and committed in order.
  - A save for leaving starts only once the World is no longer `Running`. Its capture is therefore taken after the freeze.
- **Autosave.** `Autosave::since_save` counts `Time<Real>` while `Play::Running` with a `CurrentWorld`. Once it reaches `interval` (`AUTOSAVE_INTERVAL`, 60 s until step 20) and no save is under way, it asks for one and restarts the count. Every save that commits also restarts it. A failed autosave is therefore tried again only an interval later, and a failed manual save doesn't restart the count.
- **Leaving.**
  - `Play` has a fourth state, `Leaving`. Like a pause, it freezes the simulation, but Esc and `PauseControl::resume` don't end it. `PauseControl::leave` enters it, and `PauseControl::stay(back_to)` returns to `Running`, or to `Pausing`, which completes at once since the simulation is still frozen.
  - Return to Main Menu, Exit with a World open, and the window's close button (`WindowPlugin { close_when_requested: false }`) enter `Leaving` and ask for a save. Once it commits, the game queues `CloseWorld` or writes `AppExit`. A later request to quit while leaving for the menu upgrades the destination.
  - On failure, `Departure::failed` holds the reason, and the pause menu offers Retry, Cancel (back to play or the pause menu, as before), and Leave Without Saving.
  - With no World open, the close button and Exit quit at once.
- **Menus.**
  - The main menu has a camera of its own (`Camera2d` with `PrimaryEguiContext`, `DespawnOnExit(Session::Closed)`). No World, gameplay HUD or simulation exists behind it.
  - The Load Game page draws each World as a two-line row: the name and last-played time, and below them the seed, or why it can't load. A double-click loads it. Last-played times show as "5 minutes ago", and in full in UTC ("2026-11-27 17:25 UTC") on hover and in the delete confirmation. No time-zone crate is in the workspace.
  - The pause menu is an `egui::Modal` over the World: name, seed, last-played time, the latest save's status, and Resume, Save, Return to Main Menu. While leaving, it shows the save under way or the three choices.
  - During play, a note at the top of the screen says when the World is saving, has saved (3 s), or failed to save (10 s).

## Seams and tests

1. Drive the menu actions through the game-side lifecycle seam with a temporary World store. Verify no worker at launch, distinct IDs for repeated names and seeds, the initial save, and loading the selected World.
2. Use an injectable clock to verify autosave eligibility, a paused interval, timer reset on manual and successful automatic saves, and failure without a tight retry loop.
3. Simulate load, management, and write failures. Verify the selection and committed save remain intact where appropriate, and check all three exit failure actions. Exercise both return to menu and the normal window-close event.

Where they are:

- Tests 1–3: `tests/lifecycle.rs`, a headless app with the session, input policy, sim bridge, player and lifecycle plugins. It saves real generated Worlds, under the shipped rules, in a temporary folder. The store's `before_publish` hook fails or holds writes on demand, and the test sets `WallClock` and steps `TimeUpdateStrategy::ManualDuration`.
  - Test 1 also closes the window with a World open, and checks that the save holds the Tick count the World froze at.
  - Test 2 is two tests: autosave timing, and a failed autosave.
  - Test 3 is three tests: failed loads and management, the three choices after a failed save for leaving, and a save for leaving queued behind a held save, which must still hold an Edit made after that save was sampled.
- Also in `tests/lifecycle.rs`: `parse_seed`. In `src/menu.rs`: the UTC and "time ago" formatting.
- Mutation checks, each reverted, turned at least one test red, or hung it:
  - not restarting the count when an autosave is tried;
  - not restarting it when a save commits;
  - starting a save while another is under way (hangs: the new ticket waits for the held write);
  - cancelling back to play from the pause menu;
  - quitting from the close button without saving.

Checked in the real game with a temporary harness, since removed. It saved to a scratch folder, clicked egui through `WindowEvent`s, and took screenshots:

- The game opened at the main menu, with no World behind it.
- The New Game form showed a bad seed's error inline, then created the World from the name and seed typed, and it opened.
- The pause menu showed the name, seed and last-played time, and "Saved just now" after Save.
- The World list showed the World. Rename updated its row. The delete confirmation showed its name, seed and last-played time, and cancelling kept it.
- A player put in flight at (20.5, 30, −10.5), turned, was saved by the close button, which quit within about 150 ms. On reopening and loading, they were there, looking the same way, and still flying.
- Water placed in the air and paused mid-fall looked identical in frames 2 s apart. After Resume it carried on falling. Distant terrain still streams in while paused, as it should.

## Done when

- [x] **In game** — Create two named Worlds from the same seed, make an Edit in one, save manually, return to the main menu, and confirm the Worlds load independently. Rename one and cancel a delete before confirming it. (Two Worlds with an Edit: headless, test 1. Rename and cancelled delete: in the real game.)
- [x] **In game** — Pause during visible fluid motion and confirm the World is frozen. Resume and confirm it continues. Close the window normally and reopen to the saved position and view.
- Tests: 7 in `tests/lifecycle.rs` (6 seam tests and `parse_seed`), and 2 in `src/menu.rs`.

### Deferred

- **Local time.** Last-played times show as how long ago, or in UTC. Local time needs a time-zone crate.
- **Settings.** The autosave interval is `AUTOSAVE_INTERVAL` until step 20 wires `Autosave::interval` to the player's choice, and Settings joins both menus. (Done in step 20.)
- **Focus loss still doesn't pause** (from step 18).
- **Quitting other ways.** Only the window's close button and Exit save first. A signal such as SIGTERM, or a crash, loses play since the last save. The last committed save stays loadable.
