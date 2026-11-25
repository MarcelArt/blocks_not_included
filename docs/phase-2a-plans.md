# Phase 2A plan

Phase 2A turns the Sim Sandbox into a game that opens at a main menu and lets players create, save, and resume named Worlds. It also adds persistent player settings, textures, and first-person control improvements.

## World creation and saves

- **New Game** asks for a World name and offers an optional World Seed. Leaving the seed blank generates a fresh one. Show the resulting seed so the player can reuse it. Starting from the same seed under the same generation rules produces the same initial terrain.
- Keep the existing terrain rules for hills, caves, and pockets. Two Worlds created with the same seed have separate saves and evolve independently.
- **Load Game** restores the saved player position and view and the saved Elements, Mass, and temperature of Cells in both Active and Dormant Chunks, including Chunks beyond render distance. Time stops between sessions. The saved seed can reconstruct untouched terrain.
- Restore the Tick count, Active Region, and Settled Chunk state so the next Tick matches uninterrupted play with the same inputs and rules. Restore player velocity and flight mode.
- Store complete Cell data for Chunks that differ from generated terrain, including Chunks emptied to Vacuum. Regenerate unchanged terrain from the World Seed. A failed or interrupted write leaves the previous successful save loadable.
- Load saves with compatible format, generation rules, and simulation rules. Explain incompatibility without changing the save. Development saves preserve the generator's startup Element definitions and the effective simulation rules separately, and allow further hot reloads during play. Save migration is deferred.
- Store `sandbox` as the game mode. **New Game** will offer mode selection once a second mode exists.
- The World list shows each World's name, seed, and last-played time, with the most recently played first. Each World has its own identity and display names may repeat. Rename keeps the seed and progress. Delete is permanent after a confirmation that shows the name, seed, and last-played time.

## Menus and pause

- The main menu offers **New Game**, **Load Game**, **Settings**, and **Exit**.
- Esc opens the pause menu with the cursor visible and freezes simulation Ticks, movement, and tool input. The menu lets the player resume, save manually, open Settings, or return to the main menu. Resume restores the debug clock's previous state.
- During gameplay, holding Alt temporarily reveals the cursor so the player can interact with the debug panel. While Alt is held, suspend movement input, mouse look, and gameplay tool actions. Player physics still runs, and the debug clock continues to govern Ticks.
- Save on creation, when returning to the main menu, and on normal window close. If saving on exit fails, keep the World open and offer **Retry**, **Cancel**, and **Leave Without Saving**.

## Settings and storage

- Settings apply across Worlds. Provide windowed resolution, windowed and borderless fullscreen modes, mouse sensitivity, and rebindable gameplay controls including tool shortcuts. Reserve Esc and Alt, report binding conflicts, allow shared keys for mutually exclusive actions, and offer **Reset to Defaults**.
- Offer autosave intervals of 1, 3, 5, and 10 minutes, defaulting to 1 minute. Count time while the World is unpaused and restart the timer after a successful save.
- On Linux, store saves under `$XDG_DATA_HOME/blocks_not_included` and settings under `$XDG_CONFIG_HOME/blocks_not_included`. The base directories default to `~/.local/share` and `~/.config` respectively. On Windows, use the per-user Local App Data known folder with separate saves and settings locations. Use the `blocks_not_included.dev` directory name when the repository's `dev` feature is enabled. [XDG Base Directory Specification](https://specifications.freedesktop.org/basedir/) · [Windows known folders](https://learn.microsoft.com/en-us/windows/win32/shell/knownfolderid)

## Textures

- Use the `imagegen` skill to create and integrate textures for Dirt, Granite, Ice, and Igneous Rock, plus Water and Magma.
- Use seamless, low-resolution pixel textures. Repeat solid textures once per Cell face, including on greedy-meshed faces that span multiple Cells. Tile static textures across liquid surfaces and exposed sides. Keep Water translucent, preserve visible liquid fill height, and preserve diagnostic Overlays.

## Movement

- Toggle the existing noclip flight with two distinct Jump presses within 300 ms, with a release between them. Replace the `F` toggle. Treat Jump presses while swimming as swim input and clear pending taps when entering water, pausing, or losing gameplay input.
- Hold Ctrl to Run and Shift to Sneak on land. Shift descends in flight. Sneak takes precedence over Run, slows grounded movement, and keeps the player from walking off unsupported ledges. Jump can leave a ledge deliberately. Ease the vertical field of view from 70° to 76° while running and back when running ends.
- Holding Jump and moving toward the bank should let the player exit a pool onto a ledge up to one Cell above the waterline. Verify this in partly filled shallows and a deeper pool, with solid walls and low ceilings still blocking movement.

## Implementation order

Implement [steps 16–23 in order](plans/README.md#phase-2a). Each step has prerequisites, test seams, and acceptance checks.
