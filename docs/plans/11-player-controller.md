# Step 11: First-person controller

> Read `docs/plans/CONVENTIONS.md` and `docs/plans/BEVY-0.20-NOTES.md` first.

**Prerequisites:** step 10 is done.

## Goal

The player walks, jumps, flies and swims through the world in first person, colliding with solid Cells. The Active Region follows them.

## Design

- **Collision** (`src/player/collision.rs`). A pure function, `move_and_collide(aabb, delta, is_solid: impl Fn(IVec3) -> bool) -> MoveResult`.
  - It sweeps one axis at a time: Y first, then X, then Z.
  - It reports whether the body ended up grounded.
  - The body is 0.6 × 1.8 × 0.6 m, with eyes at 1.62 m.
- **Solidity.** A Cell counts as solid if its Element is in the solid Phase. Cells in a Chunk that isn't loaded also count as solid, so the player can't fall into unloaded space.
- **Movement.**
  - WASD to move. The mouse looks around; click to capture the cursor, Esc releases it.
  - Gravity and Space to jump.
  - F toggles fly mode: noclip, with Space and Shift for up and down.
- **Liquids.** While the Cell at the body's centre holds a liquid, gravity drops to a fraction of normal, speed halves, and holding Space swims upward. Gas has no effect on movement (Sim Sandbox design).
- **Spawning.** Hold the player in place until the origin column's Chunks are loaded, then place them on the highest solid Cell.
- **Focus.** Update `SimFocus` to the player's Cell, so the bridge sends `Focus` and the Active Region follows. The hysteresis in step 06 stops it from thrashing.
- Remove the temporary camera from step 10.

## Seams and tests

**Seam:** `move_and_collide`, as a pure function with unit tests.

1. A falling body stops on a floor and is grounded.
2. Moving diagonally into a wall slides along it.
3. The body can't fit through a gap one Cell high, because it is 1.8 m tall.
4. A body standing exactly on a Cell boundary doesn't fall through, and isn't pushed out sideways.
5. In noclip mode the body moves through solids, which is the toggle's purpose. Test the mode switch through whatever function picks between the two kinds of movement.

## Done when

- [ ] The tests pass.
- [ ] **Human check:** you spawn on the ground, walk over hills, can't walk through walls, fly with F, and swim in a water pocket. The debug log shows the Active Region moving as you travel.

## Scope boundary

This step ends at moving around. Changing the world comes in step 12.
