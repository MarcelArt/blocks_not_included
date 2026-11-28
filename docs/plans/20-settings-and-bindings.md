# Step 20 — Settings and bindings

Read [CONVENTIONS.md](CONVENTIONS.md), [BEVY-0.20-NOTES.md](BEVY-0.20-NOTES.md), and [the phase scope](../phase-2a-plans.md).

**Prerequisites** — Steps 17–19.

## Goal

Expose persistent, per-user display, mouse, autosave, and gameplay input settings from both menus.

## Design

- Store user settings under the config root from step 17, separate from World saves and developer simulation tuning. Apply validated settings on startup. Missing settings use defaults. Report unreadable or invalid preferences and start with usable defaults. Write settings changes atomically and surface failures.
- Offer windowed resolution and Windowed or Borderless Fullscreen. Retain the selected windowed dimensions while fullscreen. Default mouse sensitivity to the current look speed. Offer autosave intervals of 1, 3, 5, and 10 minutes, with 1 minute as the default, using the save coordinator from step 19.
- Make **Settings** reachable from the main and pause menus. Returning goes to the originating menu. In-session Settings keeps gameplay paused.
- Use named actions for movement, Jump, flight descent, Overlay cycling, god-tool selection, and tool actions. Route the existing controls and their displayed hints through the same binding data. Step 21 adds Run and Sneak and changes the flight toggle. Keep debug clock shortcuts fixed and account for them when checking conflicts.
- Reserve Esc for pause and Alt for the cursor. When a new binding overlaps another action in the same context, show the conflicting action and let the player resolve it before applying. Allow a shared key for actions in mutually exclusive movement contexts. Consume the input used to set a binding. Provide **Reset to Defaults**.

### Decisions made during implementation

- **Modules.**
  - `src/settings/` (new): `mod.rs` holds `Settings`, `Display`, `DisplayMode`, `WindowSize`, `AutosaveInterval`, the resources `SettingsStorage`, `SettingsStatus` and `SettingsPage`, the `SettingsAction` message, and `SettingsPlugin`. `bindings.rs` holds `Action`, `Input`, `Context`, `Bindings`, `Refusal`, and the `ActionInput` system parameter. `editor.rs` holds `SettingsEditor`, the page's draft and rebinding logic, with no Bevy in it. `store.rs` holds `SettingsStore`, the file.
  - `src/menu.rs` became `src/menu/mod.rs`, and the page is in `src/menu/settings_page.rs`.
  - `src/atomic_write.rs` (new): the staging and folder flushing that World saves used, now shared with settings.
- **The file.** `settings.ron` in `UserDirs::settings`, with `version: 1`. Each setting is read on its own, as a RON `RawValue`:
  - a missing setting takes its default without a report, as for a file from before the setting existed;
  - an invalid value takes its default, is reported (`SettingsProblem::Invalid`), and the other settings are kept;
  - a file that can't be read or parsed, or has another version, is reported (`Unreadable`), and every setting takes its default;
  - controls are read per action. If the result has a conflict, every control takes its default.
  - Reading never changes the file. It is rewritten, whole and valid, only when the player applies settings. `SettingsStore::save` refuses settings that fail `Settings::check`.
- **Startup.** `main.rs` reads the settings before building the app, so the primary window opens with the saved mode and size. Problems show on the main menu's first page and on the Settings page until settings are next kept.
- **Applying.** The page edits a draft. **Apply** sets the `Settings` resource and writes the file on the main thread: it is a few hundred bytes. If the write fails, the settings still apply until the game quits, and the page says why they weren't kept. **Back**, or **Cancel** while there are changes, discards the draft. **Reset to Defaults** resets the whole draft, and still needs Apply.
- **Display.** Window sizes are physical pixels, from 640 × 360 to 7680 × 4320. The page offers seven common sizes that fit the primary monitor, plus the current size. Borderless fullscreen uses `MonitorSelection::Current`. Exclusive fullscreen isn't offered. The window size is kept while fullscreen and applied when the game becomes a window again. Leaving fullscreen, KDE on Wayland restored the size from before after the game had asked for its own, so for 1 s after becoming a window, the game asks again whenever the window takes another size.
- **Mouse sensitivity** multiplies `player::LOOK_SPEED`, 0.002 rad per pixel, from 0.1× to 5×. It defaults to 1×, the old speed.
- **Autosave.** `Autosave::interval` follows `Settings::autosave` whenever the settings change. Play already counted still counts, so a shorter interval that has already passed autosaves at once.
- **Actions.** Movement (4), Jump, Fly down (`Descend`), Cycle the Overlays, one Choose action per tool, and one use action per tool: Dig, Place, Heat brush, Spawn fluid. With one use action per tool, the existing controls keep their meaning: left click digs and heats, right click places and spawns. Each action can be bound to any number of keys and mouse buttons, or to none. Rebinding sets one input. Either Shift, Ctrl or system key binds both, and the pair shows as "Shift".
- **Contexts and conflicts.** An action applies in a `Context`: a set of movement modes (walk, fly) and a set of tools in hand. Two actions conflict if they share an input and their contexts overlap in both dimensions.
  - Fly down is flight-only, so step 21's Sneak (`Context::walking()`) may share Shift with it.
  - The tool dimension gives the rule a real case now: Dig and Heat brush share the left button by default, and Place may take it too.
- **Keys that can't be bound.** `RESERVED_KEYS`: Esc and both Alts. `FIXED_SHORTCUTS`: F3, P, N (`debug_panel::{PANEL_KEY, CLOCK_KEY, STEP_KEY}`) and F (`player::FLY_KEY`). These work in every context, so they are refused with their purpose rather than offered as conflicts. **Step 21** removes `FLY_KEY` from `FIXED_SHORTCUTS` when it moves flying to Jump.
- **Rebinding.** Clicking a control's button listens. The next key or button pressed is offered to the editor: refused (keep listening), bound, or a conflict that names the actions affected and what they keep, with **Replace** or **Cancel**.
  - `listen_for_input` runs in `PreUpdate` after Bevy's input and before the input policy (`SettingsInputSystems`). While listening, it clears every just-pressed key and button, so nothing else sees them.
  - While the page is open it also takes Esc: Esc stops listening, cancels a conflict, or closes the page. So Esc never resumes the World from Settings.
  - A modal covers the page while listening, so a mouse button pressed to bind presses nothing under it. egui hit-tests against the previous frame's layout, where the modal still is.
- **The page and the World.** `SettingsAction::Open` works only from the main menu or while the World is paused, and the page closes when a session starts or ends, when the World runs again, and when the player starts leaving. The pause menu and the main menu hide while it shows. Over a World it dims the scene as the pause menu does.
- **Hints.** The palette's tool buttons ("1 Dig"), its use line (`tools::tool_hint`), and the Overlay legend (`overlay::cycle_hint`) read the applied bindings each frame. An unbound action says so and points to Settings. The debug panel's "(P)" and "(N)" labels stay, since those keys are fixed.
- **Dependencies.** Bevy's `serialize` feature, so `KeyCode` and `MouseButton` serialize by name (`Key(KeyW)`, `Mouse(Left)`). No new crates.

## Seams and tests

1. Round-trip all preferences and test missing, invalid, and failed-write cases. Verify development and release settings are independent.
2. Test reserved keys, simultaneous action conflicts, mutually exclusive bindings, cancellation, and reset. Verify displayed hints change with the applied settings.
3. Apply each autosave interval to the controlled clock tests in step 19.

Where they are:

- Test 1: `tests/settings.rs`, at the `SettingsStore` seam with real files in a temporary folder. Six tests:
  - every setting and choice round-trips;
  - missing settings, and a missing file, take their defaults quietly;
  - unreadable files;
  - each invalid setting alone, including wrong types and a controls conflict;
  - failed first and later writes, invalid settings refused, and a blocked folder;
  - separate development and release folders, through `UserDirs::xdg`.
- Test 2: `tests/bindings.rs`, at the `SettingsEditor` and `Bindings` seam. Eight tests: the defaults, reserved and fixed keys, conflicts that ask before taking a key, inputs shared by actions that never apply at once, Shift and Ctrl pairs, cancelling, reset, and the hint text.
- Test 3: `tests/lifecycle.rs`, `each_autosave_interval_chosen_in_settings_counts_unpaused_play`. It starts from a kept 3-minute interval, then chooses each interval on the Settings page over the pause menu, and checks that the autosave comes exactly one interval later.
- Also in `tests/lifecycle.rs`, `settings_over_the_pause_menu_keep_the_world_paused_and_apply_at_once`:
  - Settings won't open while the World runs.
  - Rebinding by key press uses the key up, and Esc while listening only stops listening.
  - Applied settings reach the window: borderless, then 1600 × 900, asked for again when the window manager restores another size, then left alone once settled.
  - What applies is kept. Esc goes back to a still-paused World, and no Tick runs.
  - After resuming, J jumps and Space doesn't, and mouse motion turns twice as far.
- Mutation checks, each reverted, turned at least one test red:
  - not using up keys;
  - letting Esc through on the page;
  - not resizing on return to a window;
  - ignoring the autosave setting;
  - contexts that ignore tools, or movement modes;
  - opening Settings while the World runs;
  - an invalid setting resetting every setting;
  - writing the file in place;
  - letting fixed keys be bound;
  - ignoring mouse sensitivity.

Checked in the real game with a temporary harness, since removed. It used scratch XDG folders, sent key and pointer events, and took screenshots:

- The Settings page showed over the main menu and over the pause menu. The listening prompt and the conflict prompt showed, and the conflict named Cycle the Overlays.
- J rebound Jump. O, taken from Cycle the Overlays, chose Place in the World while 2 no longer did. The palette showed "O Place", and the legend said Cycle the Overlays is unbound.
- Borderless fullscreen filled the 1920 × 1080 screen. Back to windowed at 1600 × 900, the window was 1600 × 900, after the fix above; before it, KDE left it at 1280 × 720.
- Over the pause menu, the Tick count stayed at 27 for 2 s with Settings open. Esc went back to the pause menu, still paused.
- After a restart: Jump on J, Choose Place on O, Cycle the Overlays unbound, sensitivity 2×, and the window opened at 1600 × 900. The reloaded World's hints matched.
- Clicking the left button over **Back** while listening for Jump bound it (a conflict with Dig and Heat brush) and left the page open. The same click, not listening, closed the page.

## Done when

- [x] **In game** — Change resolution, switch to borderless fullscreen and back, change sensitivity, and rebind Jump and a tool shortcut. Restart and verify the settings and hints. Open Settings from pause and verify the World remains paused. (With the harness above; mouse sensitivity's effect is checked headlessly.)
- Tests: 6 in `tests/settings.rs`, 8 in `tests/bindings.rs`, and 2 more in `tests/lifecycle.rs`.

### Deferred

- **Windows run.** `tests/settings.rs` uses the same `fs::rename` replacement as World saves, and has run only on Linux.
- **Key names** follow a US layout: a `KeyCode` names a key by its place, so on AZERTY the key labelled Z shows as "W".
- **HiDPI.** Bevy opens the window at the saved size scaled by the screen's factor, and the game then sets the saved physical size, so a scaled screen sees one resize at startup. This laptop's scale is 1, so it hasn't been seen.
- **A window resized by hand** isn't kept: the next start uses the size chosen in Settings.
- **Gamepads** aren't bindable.
