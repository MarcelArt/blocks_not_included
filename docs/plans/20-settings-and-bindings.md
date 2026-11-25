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

## Seams and tests

1. Round-trip all preferences and test missing, invalid, and failed-write cases. Verify development and release settings are independent.
2. Test reserved keys, simultaneous action conflicts, mutually exclusive bindings, cancellation, and reset. Verify displayed hints change with the applied settings.
3. Apply each autosave interval to the controlled clock tests in step 19.

## Done when

- **In game** — Change resolution, switch to borderless fullscreen and back, change sensitivity, and rebind Jump and a tool shortcut. Restart and verify the settings and hints. Open Settings from pause and verify the World remains paused.
