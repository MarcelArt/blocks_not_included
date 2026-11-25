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

## Seams and tests

1. Drive the menu actions through the game-side lifecycle seam with a temporary World store. Verify no worker at launch, distinct IDs for repeated names and seeds, the initial save, and loading the selected World.
2. Use an injectable clock to verify autosave eligibility, a paused interval, timer reset on manual and successful automatic saves, and failure without a tight retry loop.
3. Simulate load, management, and write failures. Verify the selection and committed save remain intact where appropriate, and check all three exit failure actions. Exercise both return to menu and the normal window-close event.

## Done when

- **In game** — Create two named Worlds from the same seed, make an Edit in one, save manually, return to the main menu, and confirm the Worlds load independently. Rename one and cancel a delete before confirming it.
- **In game** — Pause during visible fluid motion and confirm the World is frozen. Resume and confirm it continues. Close the window normally and reopen to the saved position and view.
