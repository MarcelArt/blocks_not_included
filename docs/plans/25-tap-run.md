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
