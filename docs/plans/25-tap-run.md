# 25 · Tap Run while moving forward

[Plan index](README.md) · Requires [21 Movement controls](21-movement-controls.md). Independent of step 24.

## Goal and starting point

On foot, tap the bound Run action while holding Forward to start running. In `src/player/mod.rs`, `move_input` currently reads Run as a held action. The controller updates `RunView` before moving. In `src/player/movement.rs`, `gait` accepts any nonzero direction for running and `advance` returns only the updated Body. The walk path already has the per-axis collision result from `move_and_collide` in `src/player/collision.rs`.

## Design

1. Add transient per-player running state alongside FlightToggle in `src/player/mod.rs`. Sample actions through the configured bindings. A sideways component is allowed. Opposing Forward and Back together produce zero forward input. Apply this transition before building the frame's MoveInput (proposed locals):

   ```rust
   let forward = actions.pressed(Action::MoveForward) && !actions.pressed(Action::MoveBack);
   if !can_run || !forward || actions.pressed(Action::Sneak) {
       active = false;
   } else if actions.just_pressed(Action::Run) {
       active = true;
   }
   ```

   Supply the resulting state as `MoveInput.run`. Scope eligibility to on-foot gameplay, including jumps. Clear on pause, loss of gameplay input, or entry into swimming or flight. Start each player inactive.
2. In `movement.rs`, expose the walk step's horizontal obstruction alongside its resulting Body through a new movement-result entry point. Keep `advance` as a Body-returning wrapper for existing callers. Set the hit flag from `moved.blocked.x || moved.blocked.z` on the actual movement sweep. This catches diagonal wall slides while allowing ordinary ground contact and landings.
3. Consume the result in the controller. A horizontal hit clears the state on that frame even if Forward and Run are still held. Update the existing eased RunView from the resolved gait after collision. Moving clear of the obstacle requires a fresh Run tap to regain running speed. Update held-Run descriptions in the player module and movement documentation.

## Tests and completion

- Extend `tests/movement.rs` to move a running Body into a wall head-on and diagonally. Both report a horizontal hit, including when the unobstructed axis still moves. A jump and landing on clear ground report none.
- Add controller transition tests in `src/player/mod.rs` for a Run tap before Forward, Forward plus a Run tap, releasing Run, tapping Run again, forward diagonals, and sideways-only input. Assert that Forward release, Sneak, and horizontal hits each clear running, and continuing or resuming Forward alone leaves the player walking. Include simultaneous Run and Sneak, held Run across a collision, and pause/resume. Run `cargo test --workspace`.
- Run `cargo run`. Hold Forward, tap and release Run, jump, then run diagonally into a wall. Speed and widened view persist through the unobstructed jump. At the wall the view starts narrowing, and walking speed persists after steering clear. Tap Run to restart. Repeat with Sneak and with Forward release, including with rebound controls.
- Done when these transitions work through normal gameplay, including collision cancellation while sliding, and existing flight, swim, sneak and camera tests pass.

## Implementation and verification

- **Controller** (`src/player/mod.rs`). `TapRun` is a component spawned with the player next to `FlightToggle`, and each player starts walking. `Controls` samples one frame of the movement actions through the bindings, with Run read as `just_pressed`. `step` is one frame apart from Bevy: it runs the transition from the Design section, builds `MoveInput` with `run` taken from `TapRun`, moves the body, and stops running on a hit. "On foot" means walking and not swimming. Losing gameplay input reads as no controls held, so Forward isn't held and running stops. `stop_running` runs in `OnExit(Play::Running)` alongside `forget_jump_presses`. `move_input` became `Controls::move_input`.
- **Movement** (`src/player/movement.rs`). `move_body` returns `Moved { body, hit_wall }`, where `hit_wall` is `blocked.x || blocked.z` from the walk's `move_and_collide`. Flight never hits. `advance` wraps it and returns only the Body. `gait` is unchanged: running still needs a nonzero direction.
- **View.** `RunView` now updates after the move, from `gait` of the pre-move body with `run` re-read from `TapRun` after any hit. It therefore starts narrowing on the hit frame itself. That frame's move already ran at running speed.
- **Docs.** The held-Run wording is updated in the player module docs, the movement module docs, `MoveInput::run`, `Gait::Run`, `gait`, and `Action::Run`.

Tests:

- `tests/movement.rs` (12, up from 10): a running body hits a wall head-on, at an angle across x and across z (sliding on along the other axis), and while flush and sliding on. Moving along a wall, and flying through one, report no hit. A running jump and its landing on clear ground, then running on and standing still, report none.
- `src/player/mod.rs` (7 new, through `step` with real collision): a Run tap before Forward; Forward plus a tap; Run released and tapped again; a jump and landing keep running; forward diagonals; sideways-only, Back-only, and Forward + Back; Sneak stops running and Run + Sneak never starts it; an angled wall hit with Forward and Right still held stops running on the hit frame, sliding on walks, and running again needs a fresh tap; gameplay loss, flight, and swimming stop it; and `stop_running` (pause) needs a fresh tap after resuming.
- `tests/sessions.rs` (+1, headless app with real key messages and the default bindings): Ctrl before W walks; with W held, a tap of Ctrl runs at 76° after Ctrl is released; Shift stops running; releasing and pressing W again walks even with Ctrl held; a pause and resume with W and Ctrl held walks.
- Mutation checks, each reverted, turned at least one test red: no Sneak check, Forward without the Back check, held Run starting a run, no stop on a hit, ignoring on-foot or swimming, the view reading the pre-hit gait, a hit counting `blocked.y`, a hit ignoring z, never hitting, `stop_running` doing nothing or not scheduled, and reading Run as `pressed`.
- `cargo test --workspace` and `cargo clippy --workspace --all-targets` pass with no warnings.
- **Pending:** the in-game `cargo run` check from the list above: diagonal wall slide, jump, Sneak, Forward release, and rebound controls. It needs someone at the keyboard.
