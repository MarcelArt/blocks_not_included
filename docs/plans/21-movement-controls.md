# Step 21 — Flight, Run, Sneak, and pool exits

Read [CONVENTIONS.md](CONVENTIONS.md), [BEVY-0.20-NOTES.md](BEVY-0.20-NOTES.md), and [the phase scope](../phase-2a-plans.md).

**Prerequisites** — Steps 18 and 20.

## Goal

Make common first-person actions predictable on land, in flight, and near the surface of a pool.

## Design

- Count distinct presses of the bound Jump action. Two presses within 300 ms with a release between them toggle the existing noclip mode in either direction. Holding Jump keeps its ordinary movement behavior and cannot count twice. Replace the `F` toggle. While swimming, treat Jump as swim input and clear any pending tap. Also reset the gesture when pausing, releasing gameplay input, rebinding Jump, or leaving the World.
- Add rebindable Run and Sneak actions with Ctrl and Shift as defaults. Shift doubles as the default Descend binding in flight. On land, Ctrl runs while held and moving, and Shift sneaks while grounded. Sneak takes precedence, reduces speed, and prevents unsupported horizontal movement off ledges. Keep ordinary gravity and permit deliberate jumps from ledges. Ease vertical field of view from 70° to 76° while running and back afterward without moving the camera's eye position.
- Determine immersion from the liquid surface height, including partial fill. Use a shared fill calculation where movement and the fluid mesher need the same surface. Fix the transition from swimming to leaving the pool so holding Jump while moving toward a bank can clear a ledge up to one Cell above the waterline. Keep solid collision during the exit, preserve normal dry jumping, and avoid repeated upward boosts from treading Water without a bank.
- Keep the movement and gesture decision code testable independently of Bevy input events. Update controls and hints in the same step.

## Seams and tests

1. Verify the double-press boundary, held input, separated presses, rebindings, and reset after swimming or loss of input. Verify toggling on and off preserves the intended flight controls.
2. Verify normal travel, running, grounded sneaking, Ctrl plus Shift precedence, walking and sneaking off the same edge, and a deliberate jump from the edge.
3. Use a partly filled shallow pool and a deep pool with a solid ledge one Cell above each waterline. Verify forward and Jump reaches the bank. Verify cases with no bank, a blocking wall, and a low ceiling.

## Done when

- **In game** — Run and Sneak near a ledge, toggle flight twice with the bound Jump, descend with Shift, and swim out of both test pools without flight. Verify the run field of view returns to normal when movement stops or Sneak takes over.
