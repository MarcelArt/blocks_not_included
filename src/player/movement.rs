//! How the player's body moves each frame: walking under gravity, swimming, or flying.

use bevy::math::{IVec3, Vec3};
use sim::Phase;

use super::collision::{Aabb, move_and_collide};

/// The body's width, height and depth, in metres.
pub const BODY_SIZE: Vec3 = Vec3::new(0.6, 1.8, 0.6);

/// How far above the soles of its feet the body's eyes are, in metres.
pub const EYE_HEIGHT: f32 = 1.62;

/// Walking speed, in metres per second.
pub const WALK_SPEED: f32 = 4.5;

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

// In a liquid: while the Cell at the body's centre holds one. Gas has no effect on movement.

/// The fraction of [`GRAVITY`] that pulls the body down in a liquid.
pub const LIQUID_GRAVITY: f32 = 0.2;

/// The fraction of [`WALK_SPEED`] the body moves at in a liquid.
pub const LIQUID_SPEED: f32 = 0.5;

/// The fastest the body sinks in a liquid, in metres per second.
pub const SINK_SPEED: f32 = 2.0;

/// How fast the body swims upward in a liquid while Jump is held, in metres per second.
pub const SWIM_SPEED: f32 = 3.0;

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

/// What the player asks of the body during one frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MoveInput {
    /// Which way to go across the ground, in world space: level, and at most 1 long.
    pub direction: Vec3,
    /// Jump: jump, swim up, or fly up.
    pub up: bool,
    /// Fly down.
    pub down: bool,
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
}

/// Moves `body` through `dt` seconds of `input`, flying or walking as its mode says.
///
/// `phase_at` gives the Phase of the Element in each Cell. It should treat a Cell whose Chunk
/// isn't loaded as solid, so the body can't fall into unloaded space.
pub fn advance(body: Body, input: MoveInput, dt: f32, phase_at: impl Fn(IVec3) -> Phase) -> Body {
    match body.mode {
        MoveMode::Fly => fly(body, input, dt),
        MoveMode::Walk => walk(body, input, dt, phase_at),
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

/// Moves under gravity, stopped by solid Cells.
fn walk(body: Body, input: MoveInput, dt: f32, phase_at: impl Fn(IVec3) -> Phase) -> Body {
    let centre = body.feet + Vec3::Y * (BODY_SIZE.y / 2.0);
    let in_liquid = phase_at(centre.floor().as_ivec3()) == Phase::Liquid;
    let mut fall_speed = body.velocity.y;
    let speed = if in_liquid {
        fall_speed = if input.up {
            SWIM_SPEED
        } else {
            (fall_speed - GRAVITY * LIQUID_GRAVITY * dt).max(-SINK_SPEED)
        };
        WALK_SPEED * LIQUID_SPEED
    } else {
        if input.up && body.grounded {
            fall_speed = JUMP_SPEED;
        }
        fall_speed = (fall_speed - GRAVITY * dt).max(-TERMINAL_SPEED);
        WALK_SPEED
    };
    let velocity = input.direction * speed + Vec3::Y * fall_speed;
    let moved = move_and_collide(body.aabb(), velocity * dt, |cell| {
        phase_at(cell) == Phase::Solid
    });
    let aabb = moved.aabb;
    Body {
        feet: Vec3::new(
            (aabb.min.x + aabb.max.x) / 2.0,
            aabb.min.y,
            (aabb.min.z + aabb.max.z) / 2.0,
        ),
        // A solid Cell stops the body along each axis it blocks.
        velocity: Vec3::select(moved.blocked, Vec3::ZERO, velocity),
        grounded: moved.grounded,
        ..body
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Solid ground at y ≤ 0, whose top is at y = 1, and a solid wall filling x ≥ 3. Everything
    /// else is gas.
    fn walled_ground(cell: IVec3) -> Phase {
        if cell.y <= 0 || cell.x >= 3 {
            Phase::Solid
        } else {
            Phase::Gas
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

    /// Solid ground at y ≤ 0, `fluid` from y = 1 to 3, and gas above.
    fn pool_of(fluid: Phase) -> impl Fn(IVec3) -> Phase {
        move |cell: IVec3| match cell.y {
            ..=0 => Phase::Solid,
            1..=3 => fluid,
            _ => Phase::Gas,
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
