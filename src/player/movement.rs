//! How the player's body moves each frame: on foot under gravity, swimming, or flying.
//!
//! - **On foot**, Run runs while the body moves, and Sneak sneaks while it stands on the ground:
//!   slower, and never off the edge of what it stands on. Sneak takes precedence over Run. Jump
//!   jumps from the ground, sneaking or not, so a jump can leave a ledge.
//! - **Swimming**, while the surface of a liquid stands above the body's centre. A partly filled
//!   Cell's surface stands as high as the fluid meshes draw it (`crate::world::liquid_fill`), so
//!   shallows are waded through on foot.
//! - **Climbing out.** Holding Jump while moving into a bank whose top is at most one Cell above
//!   the waterline, with room to stand on it, lifts the body just high enough to step onto it.
//!   Without such a bank, Jump only swims up to the surface.
//! - **Flying** goes straight where the controls say, through anything.

use bevy::math::{BVec3, IVec3, Vec3};
use sim::Phase;

use super::collision::{Aabb, is_clear, move_and_collide, overlapped_cells};

/// The body's width, height and depth, in metres.
pub const BODY_SIZE: Vec3 = Vec3::new(0.6, 1.8, 0.6);

/// How far above the soles of its feet the body's eyes are, in metres.
pub const EYE_HEIGHT: f32 = 1.62;

/// Walking speed, in metres per second.
pub const WALK_SPEED: f32 = 4.5;

/// Running speed, in metres per second.
pub const RUN_SPEED: f32 = 7.0;

/// Sneaking speed, in metres per second.
pub const SNEAK_SPEED: f32 = 1.5;

/// Flying speed, in metres per second.
pub const FLY_SPEED: f32 = 12.0;

/// Gravity's pull, in metres per second squared: stronger than Earth's 9.8, so falls and jumps
/// feel quick rather than floaty, as games usually make them.
pub const GRAVITY: f32 = 25.0;

/// How fast a jump starts upward, in metres per second. Under [`GRAVITY`] it rises
/// 8² / (2 × 25) = 1.28 m: enough to step up onto the next Cell.
pub const JUMP_SPEED: f32 = 8.0;

/// The fastest the body falls, in metres per second.
pub const TERMINAL_SPEED: f32 = 50.0;

// In a liquid: while the liquid's surface stands above the body's centre. Gas has no effect on
// movement.

/// The fraction of [`GRAVITY`] that pulls the body down in a liquid.
pub const LIQUID_GRAVITY: f32 = 0.2;

/// The fraction of [`WALK_SPEED`] the body moves at in a liquid.
pub const LIQUID_SPEED: f32 = 0.5;

/// The fastest the body sinks in a liquid, in metres per second.
pub const SINK_SPEED: f32 = 2.0;

/// How fast the body swims upward in a liquid while Jump is held, in metres per second.
pub const SWIM_SPEED: f32 = 3.0;

/// How far above the waterline the top of a bank may stand for the body to climb out onto it:
/// one Cell.
pub const EXIT_REACH: f32 = 1.0;

/// How far above the top of the bank a body climbing out aims its feet, in metres. It covers the
/// height lost to frames of up to 1/30 s, about the speed × the frame / 2.
pub const EXIT_CLEARANCE: f32 = 0.25;

/// How far below the surface a swimmer's centre may be and still climb out, in metres. Treading
/// water, it bobs with its centre at the surface, dipping about one frame's swim beneath it.
const AFLOAT_DEPTH: f32 = 0.1;

/// How much of its width a sneaking body keeps over the Cells it stands on, in metres: more than
/// the collision's skin, so that it still stands on them.
const LEDGE_HOLD: f32 = 0.01;

/// How far above its soles the body looks for the liquid its feet are in, in metres, so feet
/// resting on the floor of a pool are in it.
const SOLE: f32 = 0.01;

/// How many Cells above a point [`liquid_surface`] looks for the surface: enough to reach past
/// the top of the body from its feet.
const SURFACE_SEARCH: i32 = 3;

/// How the body moves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MoveMode {
    /// Under gravity, stopped by solid Cells.
    #[default]
    Walk,
    /// Without gravity, and straight through solid Cells.
    Fly,
}

impl MoveMode {
    pub fn toggled(self) -> Self {
        match self {
            Self::Walk => Self::Fly,
            Self::Fly => Self::Walk,
        }
    }
}

/// What a Cell holds, as the body meets it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Matter {
    /// A gas, or Vacuum: the body passes through it.
    Open,
    /// The body can't enter it.
    Solid,
    /// A liquid, whose surface stands `fill` of the way up the Cell, from 0 to 1, as
    /// `crate::world::liquid_fill` says.
    Liquid { fill: f32 },
}

impl From<Phase> for Matter {
    /// A Cell of an Element of `phase`, full if it is a liquid.
    fn from(phase: Phase) -> Self {
        match phase {
            Phase::Solid => Matter::Solid,
            Phase::Liquid => Matter::Liquid { fill: 1.0 },
            Phase::Gas | Phase::Vacuum => Matter::Open,
        }
    }
}

/// What the player asks of the body during one frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MoveInput {
    /// Which way to go across the ground, in world space: level, and at most 1 long.
    pub direction: Vec3,
    /// Jump: jump, swim up, or fly up.
    pub up: bool,
    /// Fly down.
    pub down: bool,
    /// Run, on foot.
    pub run: bool,
    /// Sneak, on foot.
    pub sneak: bool,
}

/// How the body goes during a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gait {
    Walk,
    /// On foot, moving, with Run held and Sneak not.
    Run,
    /// On the ground, with Sneak held.
    Sneak,
    /// In a liquid deeper than the body's centre.
    Swim,
    Fly,
}

impl Gait {
    /// How fast the body goes across the ground, in metres per second.
    pub fn speed(self) -> f32 {
        match self {
            Gait::Walk => WALK_SPEED,
            Gait::Run => RUN_SPEED,
            Gait::Sneak => SNEAK_SPEED,
            Gait::Swim => WALK_SPEED * LIQUID_SPEED,
            Gait::Fly => FLY_SPEED,
        }
    }
}

/// The player's body, from one frame to the next.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Body {
    /// The centre of the soles of its feet.
    pub feet: Vec3,
    /// In metres per second.
    pub velocity: Vec3,
    pub mode: MoveMode,
    /// Whether it ended its last move standing on a solid Cell.
    pub grounded: bool,
}

impl Body {
    /// A body at rest, walking, with its feet at `feet`.
    pub fn standing_at(feet: Vec3) -> Self {
        Self {
            feet,
            velocity: Vec3::ZERO,
            mode: MoveMode::Walk,
            grounded: false,
        }
    }

    /// The box the body fills.
    pub fn aabb(&self) -> Aabb {
        let half = BODY_SIZE / 2.0;
        Aabb::new(
            self.feet - Vec3::new(half.x, 0.0, half.z),
            self.feet + Vec3::new(half.x, BODY_SIZE.y, half.z),
        )
    }

    pub fn eyes(&self) -> Vec3 {
        self.feet + Vec3::Y * EYE_HEIGHT
    }

    /// The middle of the box the body fills.
    pub fn centre(&self) -> Vec3 {
        self.feet + Vec3::Y * (BODY_SIZE.y / 2.0)
    }
}

/// Whether `body` is swimming: walking, with the surface of a liquid above its centre.
///
/// `matter_at` says what each Cell holds, as for [`advance`].
pub fn swimming(body: &Body, matter_at: impl Fn(IVec3) -> Matter) -> bool {
    let centre = body.centre();
    body.mode == MoveMode::Walk
        && liquid_surface(centre, matter_at).is_some_and(|surface| surface > centre.y)
}

/// How `body` goes during a frame of `input`. On foot, Sneak takes precedence over Run: holding
/// both sneaks on the ground, and walks in the air.
pub fn gait(body: &Body, input: &MoveInput, matter_at: impl Fn(IVec3) -> Matter) -> Gait {
    if body.mode == MoveMode::Fly {
        Gait::Fly
    } else if swimming(body, matter_at) {
        Gait::Swim
    } else if input.sneak {
        if body.grounded {
            Gait::Sneak
        } else {
            Gait::Walk
        }
    } else if input.run && input.direction != Vec3::ZERO {
        Gait::Run
    } else {
        Gait::Walk
    }
}

/// Moves `body` through `dt` seconds of `input`, flying or walking as its mode says.
///
/// `matter_at` says what each Cell holds. It should treat a Cell whose Chunk isn't loaded as
/// solid, so the body can't fall into unloaded space.
pub fn advance(body: Body, input: MoveInput, dt: f32, matter_at: impl Fn(IVec3) -> Matter) -> Body {
    match body.mode {
        MoveMode::Fly => fly(body, input, dt),
        MoveMode::Walk => walk(body, input, dt, matter_at),
    }
}

/// Moves straight where the input says, through anything.
fn fly(body: Body, input: MoveInput, dt: f32) -> Body {
    let vertical = f32::from(i8::from(input.up) - i8::from(input.down));
    let velocity = (input.direction + Vec3::Y * vertical) * FLY_SPEED;
    Body {
        feet: body.feet + velocity * dt,
        velocity,
        grounded: false,
        ..body
    }
}

/// Moves under gravity, or swims, stopped by solid Cells.
fn walk(body: Body, input: MoveInput, dt: f32, matter_at: impl Fn(IVec3) -> Matter) -> Body {
    let solid = |cell: IVec3| matter_at(cell) == Matter::Solid;
    let gait = gait(&body, &input, &matter_at);
    let mut fall_speed = body.velocity.y;
    let mut jumping = false;
    if gait == Gait::Swim {
        let sunk = fall_speed - GRAVITY * LIQUID_GRAVITY * dt;
        // Holding Jump swims up, unless the body already rises faster, as it climbs out.
        fall_speed = if input.up {
            sunk.max(SWIM_SPEED)
        } else {
            sunk.max(-SINK_SPEED)
        };
    } else {
        jumping = input.up && body.grounded;
        if jumping {
            fall_speed = JUMP_SPEED;
        }
        fall_speed = (fall_speed - GRAVITY * dt).max(-TERMINAL_SPEED);
    }
    let mut across = input.direction * gait.speed();
    if gait == Gait::Sneak && !jumping {
        across = keep_on_ledge(body.aabb(), across, dt, solid);
    }
    let velocity = across + Vec3::Y * fall_speed;
    let moved = move_and_collide(body.aabb(), velocity * dt, solid);
    let aabb = moved.aabb;
    let mut moved_body = Body {
        feet: Vec3::new(
            (aabb.min.x + aabb.max.x) / 2.0,
            aabb.min.y,
            (aabb.min.z + aabb.max.z) / 2.0,
        ),
        // A solid Cell stops the body along each axis it blocks.
        velocity: Vec3::select(moved.blocked, Vec3::ZERO, velocity),
        grounded: moved.grounded,
        ..body
    };
    if input.up
        && let Some(lift) = climb_out(&moved_body, input.direction, moved.blocked, &matter_at)
    {
        moved_body.velocity.y = moved_body.velocity.y.max(lift);
    }
    moved_body
}

/// The part of `velocity` across the ground that a sneaking body filling `aabb` may move with for
/// `dt` without stepping off the solid Cells it stands on. Along x, then z, it goes as far as it
/// still keeps [`LEDGE_HOLD`] of its width over one of them, so it slides along an edge.
fn keep_on_ledge(aabb: Aabb, velocity: Vec3, dt: f32, solid: impl Fn(IVec3) -> bool) -> Vec3 {
    if dt <= 0.0 {
        return velocity;
    }
    // The layer of Cells the feet rest on.
    let under = overlapped_cells(aabb.min.y, aabb.max.y).start() - 1;
    let mut aabb = aabb;
    let mut kept = velocity;
    for axis in [0, 2] {
        let step = velocity[axis] * dt;
        if step == 0.0 {
            continue;
        }
        let other = 2 - axis;
        // Whether the Cell at `along` on this axis holds the body up, wherever the footprint
        // reaches across the other.
        let holds = |along: i32| {
            overlapped_cells(aabb.min[other], aabb.max[other]).any(|across| {
                let mut cell = IVec3::new(0, under, 0);
                cell[axis] = along;
                cell[other] = across;
                solid(cell)
            })
        };
        let stood_on = overlapped_cells(aabb.min[axis] + step, aabb.max[axis] + step);
        let allowed = if stood_on.into_iter().any(holds) {
            step
        } else {
            // As far as the farthest Cell it stands on now still holds it.
            let now = overlapped_cells(aabb.min[axis], aabb.max[axis]);
            let limit = if step > 0.0 {
                now.filter(|&along| holds(along))
                    .map(|along| (along + 1) as f32 - LEDGE_HOLD - aabb.min[axis])
                    .reduce(f32::max)
            } else {
                now.filter(|&along| holds(along))
                    .map(|along| along as f32 + LEDGE_HOLD - aabb.max[axis])
                    .reduce(f32::min)
            };
            match limit {
                Some(limit) if step > 0.0 => limit.clamp(0.0, step),
                Some(limit) => limit.clamp(step, 0.0),
                // Standing on nothing, there is no edge to keep to.
                None => step,
            }
        };
        kept[axis] = allowed / dt;
        aabb.min[axis] += allowed;
        aabb.max[axis] += allowed;
    }
    kept
}

/// How fast `body` must rise to climb out of the liquid its feet are in onto the bank it moved
/// into, if it can. It can when it is afloat at the surface, or wading; when, along an axis
/// `blocked` stopped it moving in `direction`, the top of a solid Cell stands at most
/// [`EXIT_REACH`] above the waterline; and when there is room for the body both to rise where it
/// is and to stand on that top. It then rises to [`EXIT_CLEARANCE`] above that top.
fn climb_out(
    body: &Body,
    direction: Vec3,
    blocked: BVec3,
    matter_at: impl Fn(IVec3) -> Matter,
) -> Option<f32> {
    if body.mode != MoveMode::Walk {
        return None;
    }
    let surface = liquid_surface(body.feet + Vec3::Y * SOLE, &matter_at)?;
    if surface <= body.feet.y || surface - body.centre().y > AFLOAT_DEPTH {
        return None;
    }
    let solid = |cell: IVec3| matter_at(cell) == Matter::Solid;
    let aabb = body.aabb();
    // The tops of Cells above the feet, and within reach of the waterline.
    let lowest = overlapped_cells(aabb.min.y, aabb.max.y).start() + 1;
    let highest = (surface + EXIT_REACH + 1e-3).floor() as i32;
    for axis in [0, 2] {
        if !blocked.test(axis) || direction[axis] == 0.0 {
            continue;
        }
        let toward = direction[axis].signum() * BODY_SIZE[axis];
        for top in lowest..=highest {
            let rise = top as f32 - aabb.min.y;
            let mut way_up = aabb;
            way_up.max.y += rise;
            let mut on_the_bank = way_up;
            on_the_bank.min.y += rise;
            on_the_bank.min[axis] += toward;
            on_the_bank.max[axis] += toward;
            let mut under_it = on_the_bank;
            under_it.max.y = under_it.min.y;
            under_it.min.y -= 1.0;
            if is_clear(way_up, solid) && is_clear(on_the_bank, solid) && !is_clear(under_it, solid)
            {
                return Some((2.0 * GRAVITY * (rise + EXIT_CLEARANCE)).sqrt());
            }
        }
    }
    None
}

/// The height of the surface of the liquid in the Cell holding `point`, if it holds one: the top
/// of the run of liquid Cells from there up, as full as the highest of them is. It may lie below
/// `point`, in a partly filled Cell. It looks [`SURFACE_SEARCH`] Cells up at most: deeper, it
/// gives the top of the highest Cell it looked at.
fn liquid_surface(point: Vec3, matter_at: impl Fn(IVec3) -> Matter) -> Option<f32> {
    let mut cell = point.floor().as_ivec3();
    let Matter::Liquid { mut fill } = matter_at(cell) else {
        return None;
    };
    for _ in 0..SURFACE_SEARCH {
        match matter_at(cell + IVec3::Y) {
            Matter::Liquid { fill: above } => {
                cell.y += 1;
                fill = above;
            }
            Matter::Open | Matter::Solid => break,
        }
    }
    Some(cell.y as f32 + fill)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Solid ground at y ≤ 0, whose top is at y = 1, and a solid wall filling x ≥ 3. Everything
    /// else is gas.
    fn walled_ground(cell: IVec3) -> Matter {
        if cell.y <= 0 || cell.x >= 3 {
            Matter::Solid
        } else {
            Matter::Open
        }
    }

    /// Asserts that `actual` is `expected` to within 0.01 mm, the rounding of a few f32 steps.
    #[track_caller]
    fn assert_near(actual: Vec3, expected: Vec3) {
        assert!(
            actual.abs_diff_eq(expected, 1e-5),
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn a_flying_body_passes_through_solids_and_a_walking_one_stops() {
        let walking = Body::standing_at(Vec3::new(2.0, 1.0, 0.5));
        let flying = Body {
            mode: MoveMode::Fly,
            ..walking
        };
        let towards_the_wall = MoveInput {
            direction: Vec3::X,
            ..MoveInput::default()
        };

        // Flying, 0.5 s at 12 m/s carries the feet 6 m along x, from 2 to 8: through the wall.
        let flown = advance(flying, towards_the_wall, 0.5, walled_ground);
        assert_near(flown.feet, Vec3::new(8.0, 1.0, 0.5));

        // Walking, the body's right side, at x = 2.3, stops at the wall's face at x = 3.
        let walked = advance(walking, towards_the_wall, 0.5, walled_ground);
        assert_near(walked.feet, Vec3::new(2.7, 1.0, 0.5));
    }

    /// Solid ground at y ≤ 0, `fluid` from y = 1 to 3, and gas above. A liquid fills its Cells.
    fn pool_of(fluid: Phase) -> impl Fn(IVec3) -> Matter {
        move |cell: IVec3| {
            match cell.y {
                ..=0 => Phase::Solid,
                1..=3 => fluid,
                _ => Phase::Gas,
            }
            .into()
        }
    }

    #[test]
    fn in_a_liquid_the_body_moves_at_half_speed_and_sinks_slowly_but_gas_has_no_effect() {
        // Feet at y = 2, so the body's centre, 0.9 m higher at y = 2.9, is in the Cell at y = 2.
        let body = Body::standing_at(Vec3::new(0.5, 2.0, 0.5));
        let forward = MoveInput {
            direction: Vec3::X,
            ..MoveInput::default()
        };

        // In gas: 0.1 s at 4.5 m/s goes 0.45 m along x. Gravity, 25 m/s², brings the body to
        // 2.5 m/s downward, so it falls 2.5 × 0.1 = 0.25 m.
        let in_gas = advance(body, forward, 0.1, pool_of(Phase::Gas));
        assert_near(in_gas.feet, Vec3::new(0.95, 1.75, 0.5));

        // In a liquid: half the speed, 0.225 m along x. A fifth of gravity, 5 m/s², brings it to
        // 0.5 m/s downward, so it sinks 0.05 m.
        let in_liquid = advance(body, forward, 0.1, pool_of(Phase::Liquid));
        assert_near(in_liquid.feet, Vec3::new(0.725, 1.95, 0.5));

        // However long it sinks, it never sinks faster than 2 m/s.
        let sinking = Body {
            velocity: Vec3::new(0.0, -1.9, 0.0),
            ..body
        };
        let sunk = advance(sinking, MoveInput::default(), 0.1, pool_of(Phase::Liquid));
        // 1.9 + 0.5 would be 2.4 m/s; capped at 2, it sinks 0.2 m.
        assert_near(sunk.velocity, Vec3::new(0.0, -2.0, 0.0));
        assert_near(sunk.feet, Vec3::new(0.5, 1.8, 0.5));
    }

    #[test]
    fn holding_space_in_a_liquid_swims_upward() {
        // Sinking at 2 m/s through the middle of the pool, centre at y = 2.9.
        let body = Body {
            velocity: Vec3::new(0.0, -2.0, 0.0),
            ..Body::standing_at(Vec3::new(0.5, 2.0, 0.5))
        };
        let space = MoveInput {
            up: true,
            ..MoveInput::default()
        };
        let swum = advance(body, space, 0.1, pool_of(Phase::Liquid));
        // It swims up at 3 m/s, rising 0.3 m in 0.1 s.
        assert_near(swum.velocity, Vec3::new(0.0, 3.0, 0.0));
        assert_near(swum.feet, Vec3::new(0.5, 2.3, 0.5));
    }

    #[test]
    fn space_jumps_from_the_ground_but_not_in_mid_air() {
        let space = MoveInput {
            up: true,
            ..MoveInput::default()
        };
        let on_the_ground = Body {
            grounded: true,
            ..Body::standing_at(Vec3::new(0.5, 1.0, 0.5))
        };
        let jumped = advance(on_the_ground, space, 0.1, walled_ground);
        // It leaves at 8 m/s, and gravity takes 25 × 0.1 = 2.5 m/s off that in the first 0.1 s:
        // 5.5 m/s, which lifts it 0.55 m.
        assert_near(jumped.velocity, Vec3::new(0.0, 5.5, 0.0));
        assert_near(jumped.feet, Vec3::new(0.5, 1.55, 0.5));
        assert!(!jumped.grounded);

        // A moment later, still rising, holding Space adds nothing: 5.5 − 2.5 = 3 m/s.
        let still_rising = advance(jumped, space, 0.1, walled_ground);
        assert_near(still_rising.velocity, Vec3::new(0.0, 3.0, 0.0));
    }
}
