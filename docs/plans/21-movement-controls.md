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

### Decisions made during implementation

- **Modules.**
  - `src/player/flight.rs` (new): `FlightToggle`, the double press, read one frame at a time from a `JumpFrame`, with no Bevy input in it. `DOUBLE_PRESS_WINDOW` is 300 ms.
  - `src/player/view.rs` (new): `RunView`, the eased field of view, and `FIELD_OF_VIEW` (70°), `RUN_FIELD_OF_VIEW` (76°), `RUN_VIEW_EASE` (0.25 s).
  - `src/player/movement.rs`: `Matter` (what a Cell holds, as the body meets it: open, solid, or liquid with its fill), `Gait`, `gait`, `swimming`, the Sneak ledge guard, and the climb out of a liquid. `advance` now takes `impl Fn(IVec3) -> Matter` rather than a Phase. `From<Phase>` gives a full liquid Cell.
  - `src/world/liquid.rs` (new): `liquid_fill`, `capacity_grams` and `MIN_SURFACE`, moved out of the fluid mesher, which now calls them. Movement calls them too, so the surface the player swims in is the one drawn.
  - `collision::is_clear`, whether a box reaches into no solid Cell.
- **The double press.**
  - A press is a bound Jump input going down while none was held at the end of the last frame. Holding Jump, or pressing a second key bound to Jump while one is held, is one press. A press and release within one frame still counts.
  - Two presses switch flying when the second comes at most 300 ms after the first, 300 ms included. The switch uses up both presses, so a third starts again. Switching zeroes the velocity, as F did.
  - Times come from `Time<Real>`, so a long frame doesn't shorten the window.
  - A first press is forgotten: while swimming, where presses only swim; on any frame without gameplay input (Alt, no focus); when Jump's bindings change; and in `OnExit(Play::Running)`, which covers pausing, leaving and closing the World. The toggle is a component on the player, so each World starts without one.
- **F is gone.** `player::FLY_KEY` is removed, and `FIXED_SHORTCUTS` holds only the debug panel's three keys. F can be bound to anything.
- **Run and Sneak.** `run` and `sneak` in the settings file, both in `Context::walking()`, bound by default to both Ctrl keys and both Shift keys. Fly down keeps Shift too: it is flight-only.
  - Speeds: walking 4.5, running 7.0, sneaking 1.5 m/s.
  - The gait, in order: flying; swimming; Sneak held sneaks on the ground and walks in mid-air; Run held while moving runs, on the ground or in the air, so a running jump keeps its speed; otherwise walking. Sneak takes precedence, so Ctrl and Shift together never run.
  - "Moving" means a movement action is held. Running into a wall still counts as running.
- **The ledge guard.** While sneaking on the ground, the move across the ground is cut short, along x and then z, so the body keeps at least 0.01 m (`LEDGE_HOLD`) of its footprint over a solid Cell in the layer under its feet. Diagonally against an edge, it slides along it. Liquid holds nothing up, so a sneaking body stops at the edge of a pool.
  - The guard doesn't apply on the frame Sneak jumps, and in mid-air Sneak doesn't sneak, so a jump leaves the ledge.
  - Gravity is unchanged.
- **The field of view** moves linearly over 0.25 s each way and is eased with smoothstep, so halfway is 73°. Only `PerspectiveProjection::fov` changes. The camera stays at the body's eyes. The projection is marked changed only when the value changes. While paused, the view holds still. Without gameplay input it narrows, as nothing runs.
- **Immersion.** The body swims while the liquid surface over its centre is above its centre. The surface is found from the Cell holding the centre, up through the run of liquid Cells (any liquid) above it, as high as the top one's fill. A partly filled Cell therefore holds the body only as deep as its surface. Shallows 0.6 full are waded on foot at walking speed, and jumped out of. With full Cells this is the old rule.
  - The fill is `liquid_fill`, the mesher's rule: Mass / Capacity, at least 0.05, or 1 under more of the same liquid. It looks at most three Cells up.
- **Climbing out.** Holding Jump while moving into a bank lifts the body, if all of these hold:
  - its feet are in a liquid and it is afloat: its centre is at most 0.1 m (`AFLOAT_DEPTH`) below the surface, or above it, wading;
  - along an axis where the move was blocked, the top of a solid Cell is at most one Cell (`EXIT_REACH`) above the waterline;
  - the body has room to rise where it is up to that height, and to stand on that top half a body-width in.
  - The lift is `√(2 g (rise + 0.25 m))`, aiming the feet 0.25 m (`EXIT_CLEARANCE`) above the top. That covers the height lost to frames of up to about 1/30 s.
  - The lift is checked each frame while the feet are in the liquid. It never lowers the speed, and gives no more than the target height needs. Treading water with no such bank never rises faster than swimming's 3 m/s.
  - Swimming up now keeps a faster rise: `max(3 m/s, v − liquid gravity × dt)`. Swimming up from below still starts at 3 m/s, as before.
  - Solid collision is unchanged throughout.
- **"One Cell above the waterline"** is read as up to 1 m above the liquid's surface. On Cell-aligned terrain, a partly filled pool's highest bank is the Cell top at or below its waterline + 1 m. Shallows 0.6 full in a pool one Cell deep take a bank 0.4 m above the water, which an ordinary jump clears now that the shallows aren't swum in. A pool with a full layer and a half-full one, waterline 1.5 m, takes a bank 0.5 m above and needs the climb. A deep pool's bank stands a whole Cell above.
- **Settings page.** Run and Sneak are listed between Jump and Fly down. Under the controls, `player::flight_hint` names the bound Jump, for example "Press Space twice quickly to switch flying on and off.", and says Sneak and Fly down may share a key. The fixed-keys line no longer names F.
- **Settings files from step 20** have no `run` or `sneak`, so those take their defaults. If such a file bound Ctrl or Shift to an action that applies on foot, step 20's rule applies: every control takes its default, and the menus say why. Step 20's tests pin that rule, so it is unchanged.
- **Step 20's tests.** `tests/bindings.rs` now expects three fixed shortcuts rather than four. Its Ctrl-pair test first unbinds Run, which now has Ctrl by default.

## Seams and tests

1. Verify the double-press boundary, held input, separated presses, rebindings, and reset after swimming or loss of input. Verify toggling on and off preserves the intended flight controls.
2. Verify normal travel, running, grounded sneaking, Ctrl plus Shift precedence, walking and sneaking off the same edge, and a deliberate jump from the edge.
3. Use a partly filled shallow pool and a deep pool with a solid ledge one Cell above each waterline. Verify forward and Jump reaches the bank. Verify cases with no bank, a blocking wall, and a low ceiling.

Where they are:

- Test 1: `tests/flight_toggle.rs`, at the `FlightToggle` seam, frame by frame. Eight tests:
  - the boundary, at 300 ms and 301 ms;
  - a third press after a switch;
  - held Jump, and a second bound key pressed while one is held;
  - a press and release within one frame;
  - rebinding;
  - swimming;
  - no gameplay input and `reset`;
  - Shift sneaking on foot but flying down in flight, and Jump flying up.
- Test 2: `tests/movement.rs`, at `advance` and `gait`, plus `RunView`. Five tests:
  - the three speeds;
  - Sneak over Run, on the ground and in mid-air;
  - walking off and sneaking to the same edge, and sliding along it;
  - a deliberate jump from it;
  - the view widening and narrowing as the gait says.
- Test 3: `tests/movement.rs`. Five tests:
  - the shared fill;
  - swimming only below the surface, in partly filled Cells;
  - climbing out of shallows 0.6 full, a pool 1.5 m deep with a half-full top layer, and a pool three full Cells deep, each onto the highest bank it allows. The swimmer rises at swimming speed until it is afloat. No frame finds the body inside a solid.
  - With no bank, a wall two Cells high, a ceiling one Cell above the bank, or a ceiling over the pool, it only treads water: never faster than 3 m/s up, never higher than the surface allows.
  - Dry jumps are unchanged.
- In the game's systems: `tests/sessions.rs`, `jump_pressed_twice_switches_flying_and_running_widens_the_view`, in a headless app with its clocks moved by hand. It checks that:
  - F no longer flies, and two taps of Space do;
  - in flight, Space rises 2.4 m and Shift sinks 1.2 m;
  - a pause or Alt between two taps forgets the first;
  - two taps land the player;
  - Ctrl and W widen the view to 76° and it returns to 70° 0.3 s after letting go, with the camera at the eyes throughout.
- `tests/bindings.rs` also checks that F can be bound, Run's and Sneak's defaults and contexts, Sneak sharing Shift with Fly down but not Ctrl with Run, and the flight hint following Jump's binding.
- Mutation checks, each reverted, turned at least one test red:
  - the old centre-Cell immersion;
  - no climbing out;
  - an unlimited bank height;
  - no room needed on the bank, or on the way up;
  - climbing out from any depth;
  - no ledge guard;
  - sneaking in mid-air;
  - Run before Sneak;
  - a held Jump counting again;
  - presses counting while swimming, or without gameplay input;
  - rebinding keeping a press;
  - an exclusive 300 ms;
  - a pause keeping a press;
  - the view never widening.

Checked in the real game with a temporary harness, since removed. It ran a new World in scratch XDG folders and built a ledge and three pools of Granite and Water 15 m above the spawn. It then pressed keys, logged the body, and took screenshots.

- **Sneak.** With Shift and W, the feet stopped at 0.29 m past the edge, still on it. Adding Ctrl changed nothing, and the view stayed at 70°. A tap of Space while still holding Shift and W left the ledge. W alone walked off the same edge.
- **Run.** Ctrl and W ran at 7 m/s, and the view was 72.7° after 0.12 s and 76° after 0.6 s. It was back to 70° 0.4 s after letting go of W, with Ctrl still held. It widened again when W was pressed, and narrowed to 70° at 1.5 m/s once Shift took over. In the screenshots, the pools ahead look smaller while running.
- **Flight.** Two taps of Space flew. Holding Space rose, and Shift flew down. Two more taps landed the player on the platform.
- **Pools.** Holding Space and W, the player climbed out of all three pools without flying: shallows 0.6 full, a pool 1.5 m deep, and a pool three Cells deep. The feet were over the bank and above its top 0.75 s, 1.0 s and 1.25 s after starting from each pool's floor: in the deep pool, for example, at 4.21 m over a bank whose top is at 4. With W still held, the player then walked off the far side of the bank.
- **Settings.** Run (Ctrl), Sneak (Shift) and Fly down (Shift) were listed together.

## Done when

- [x] **In game** — Run and Sneak near a ledge, toggle flight twice with the bound Jump, descend with Shift, and swim out of both test pools without flight. Verify the run field of view returns to normal when movement stops or Sneak takes over. (With the harness above.)
- Tests: 8 in `tests/flight_toggle.rs`, 10 in `tests/movement.rs`, and 1 more in `tests/sessions.rs`. `tests/bindings.rs` keeps its 8, extended.

### Deferred

- **Low frame rates.** The climb's 0.25 m clearance covers frames up to about 1/30 s. At the 0.1 s frame limit, a climb falls short, and so does an ordinary jump onto the next Cell.
- **Wading** isn't slowed; only swimming is.
- **Sneaking doesn't lower the camera.** The step didn't ask for it.
- **Flying faster with Run** isn't offered: Run applies only on foot.
- **The flight hint** is shown only on the Settings page, below the controls. No in-game hint names movement keys yet.
