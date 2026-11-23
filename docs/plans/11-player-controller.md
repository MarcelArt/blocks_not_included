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

### Decisions made during implementation

- **Modules.**
  - `src/player/collision.rs`: `Aabb`, `MoveResult` and `move_and_collide`.
  - `src/player/movement.rs`: the body and how it moves each frame. It holds `Body`, `MoveMode` (`Walk` or `Fly`), `MoveInput`, the tuning constants, and `advance`, the pure function that picks between the two kinds of movement.
  - `src/player/mod.rs`: `PlayerPlugin`, plus the `Player(Body)` and `AwaitingSpawn` components. The player's entity is the camera itself, at eye height.
- **Collision.**
  - Along each axis, a solid Cell stops the box only if the Cell's near face lies at or ahead of the box's leading face. The box then stops flush with that face, so standing still never drifts.
  - A 1 mm skin counts as touching rather than overlapping, which absorbs f32 rounding. f32 positions are finer than that only up to about 8 km from the origin; beyond that, bodies may snag on floors.
  - Solid Cells the box already overlaps don't stop it. A body left inside rock when fly mode switches off isn't trapped by those Cells, though the Cells around them still box it in.
- **Solidity.** `phase_at` treats a Cell missing from `WorldView` as solid.
  - `WorldView` also omits Chunks that have held nothing but Vacuum since they loaded, so those count as solid too.
  - The generated world has no such Chunks below the world's top, since the Atmosphere fills everything above ground. Above `WORLD_TOP_Y` nothing loads, so the world's ceiling is solid.
- **Movement.**
  - Walking is 4.5 m/s. Flying is 12 m/s, with no gravity.
  - Gravity is 25 m/s², a game value rather than Earth's 9.8, so falls don't feel floaty. A jump starts at 8 m/s and rises 1.28 m, enough to step up one Cell.
  - Falling tops out at 50 m/s.
  - Holding Space jumps again on every landing. There is no automatic step-up, so each one-Cell step takes a jump.
  - A frame moves the body through at most 0.1 s, so a stall doesn't become one long, sudden step.
- **Liquids.** These apply while the Cell at the body's centre (0.9 m above the feet) is liquid:
  - gravity is 0.2 of normal, and speed is halved;
  - holding Space swims upward at 3 m/s;
  - sinking is capped at 2 m/s. The cap was added because reduced gravity alone keeps the body accelerating downward through deep liquid.
- **Look.**
  - The mouse turns the view by 0.002 rad per pixel, and pitch stops 0.01 rad short of straight up or down.
  - The vertical field of view is 70°. The player starts facing +x and +z, with the sun behind them.
- **Spawning.**
  - The player waits until every Chunk of the focus's column (the origin column) within vertical render distance is in `WorldView`. The range is ±3 Chunks, cut off at the world's limits.
  - The feet then go on the highest solid Cell of Cell column (0, 0), at its centre. In the generated world that's (0, 1, 0), about 0.25 s after the window opens. The nearby meshes aren't drawn yet at that point, so the first second shows only sky.
- **Focus.** `SimFocus` is the Cell the feet are in. It is written only when it changes, so the bridge sends a `Focus` only when the player enters a new Cell.
- **Active Region log.** The simulation thread logs `the Active Region moved to Chunks … following the focus to …` whenever a Focus moves the Active Region.
- **Temporary camera removed.** `FreeCamera`, `FreeCameraPlugin` and Bevy's `free_camera` feature are gone.

## Seams and tests

**Seam:** `move_and_collide`, as a pure function with unit tests.

1. A falling body stops on a floor and is grounded.
2. Moving diagonally into a wall slides along it.
3. The body can't fit through a gap one Cell high, because it is 1.8 m tall.
4. A body standing exactly on a Cell boundary doesn't fall through, and isn't pushed out sideways.
5. In noclip mode the body moves through solids, which is the toggle's purpose. Test the mode switch through whatever function picks between the two kinds of movement.

Added at the same seams:

- In `src/player/collision.rs`:
  - a body sunk 0.4 mm into the floor by rounding stands on it and still walks;
  - a rising body stops under a ceiling without being grounded.
- In `src/player/movement.rs`, through `advance`:
  - in a liquid the body moves at half speed and sinks slowly, capped at 2 m/s, while gas has no effect;
  - holding Space in a liquid swims upward;
  - Space jumps from the ground but not in mid-air.

## Done when

- [x] The tests pass: 6 in `src/player/collision.rs` and 4 in `src/player/movement.rs`.
- [x] **Human check:** you spawn on the ground, walk over hills, can't walk through walls, fly with F, and swim in a water pocket. The debug log shows the Active Region moving as you travel.
  - The agent checked this with a temporary harness, since removed. The harness held keys by pressing them on `ButtonInput<KeyCode>`, logged the body four times a second, and took screenshots.
  - The player spawned at (0, 1, 0) on Dirt.
  - Holding W and Space carried them over hills, down to y = −5.
  - With F, they rose at 12 m/s and flew straight through two hills. The log showed the Active Region moving twice.
  - With F off, they fell and landed on Dirt.
  - Placed in a Water pocket 4 Cells tall, they sank at up to 2 m/s, stood on its Granite floor, swam up at 3 m/s until their head met its ceiling, then sank again.
  - Walls were only crossed on open terrain, so blocking is covered by the unit tests. The human hasn't checked it yet.

### Deferred

- **Climbing out of a liquid.** Swimming stops once the body's centre leaves the liquid, so at the surface the feet are 0.9 m under it. A ledge level with the surface can't be climbed onto; fly mode gets out. A "jump out of the liquid" rule would fix it.
- **Precision far from the origin.** Beyond about 8 km, f32 positions are coarser than the 1 mm skin. A floating origin, or f64 positions, would fix it.
- **Step-up.** Walking up a one-Cell step takes a jump.

## Scope boundary

This step ends at moving around. Changing the world comes in step 12.
