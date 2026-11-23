//! Moving a box through the grid of Cells without entering a solid one.

use bevy::math::{BVec3, IVec3, Vec3};

/// An axis-aligned box in world space, in metres. The Cell at `(x, y, z)` fills the box from
/// `(x, y, z)` to `(x + 1, y + 1, z + 1)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    pub fn new(min: Vec3, max: Vec3) -> Self {
        Self { min, max }
    }
}

/// Where a box ended up after [`move_and_collide`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoveResult {
    pub aabb: Aabb,
    /// Whether a solid Cell stopped the box moving down: it is standing on something.
    pub grounded: bool,
    /// Along which axes a solid Cell cut the move short.
    pub blocked: BVec3,
}

/// How far a box may reach into a Cell and still count as only touching it. It absorbs rounding:
/// f32 positions are finer than this up to about 8 km from the origin.
const SKIN: f32 = 1e-3;

/// The order [`move_and_collide`] moves along the axes: y, then x, then z.
const AXES: [usize; 3] = [1, 0, 2];

/// Moves `aabb` by `delta`, one axis at a time (y, then x, then z), stopping it against any Cell
/// for which `is_solid` holds. Along an axis where it meets one, it stops flush with that Cell's
/// face, and still moves along the other axes, so it slides along walls and floors.
///
/// A solid Cell the box already overlaps doesn't stop it, so a box caught inside solids can move
/// out of them.
pub fn move_and_collide(aabb: Aabb, delta: Vec3, is_solid: impl Fn(IVec3) -> bool) -> MoveResult {
    let mut aabb = aabb;
    let mut blocked = [false; 3];
    for axis in AXES {
        let distance = delta[axis];
        if distance == 0.0 {
            continue;
        }
        let size = aabb.max[axis] - aabb.min[axis];
        match first_solid_face(aabb, axis, distance, &is_solid) {
            Some(face) if distance > 0.0 => {
                aabb.max[axis] = face;
                aabb.min[axis] = face - size;
                blocked[axis] = true;
            }
            Some(face) => {
                aabb.min[axis] = face;
                aabb.max[axis] = face + size;
                blocked[axis] = true;
            }
            None => {
                aabb.min[axis] += distance;
                aabb.max[axis] += distance;
            }
        }
    }
    MoveResult {
        aabb,
        grounded: blocked[1] && delta.y < 0.0,
        blocked: BVec3::from(blocked),
    }
}

/// The face of the nearest solid Cell that `aabb` would meet moving `distance` along `axis`, if
/// any: the position, along `axis`, where its leading face must stop.
fn first_solid_face(
    aabb: Aabb,
    axis: usize,
    distance: f32,
    is_solid: &impl Fn(IVec3) -> bool,
) -> Option<f32> {
    let [across, along] = [(axis + 1) % 3, (axis + 2) % 3];
    // The Cells the box overlaps along the other two axes, by more than the skin.
    let overlapped = |other: usize| {
        let first = (aabb.min[other] + SKIN).floor() as i32;
        let last = (aabb.max[other] - SKIN).ceil() as i32 - 1;
        first..=last
    };
    let layer_is_solid = |layer: i32| {
        overlapped(across).any(|a| {
            overlapped(along).any(|b| {
                let mut cell = IVec3::ZERO;
                cell[axis] = layer;
                cell[across] = a;
                cell[along] = b;
                is_solid(cell)
            })
        })
    };
    if distance > 0.0 {
        // Layers whose near face, at `layer`, lies ahead of the leading face and within reach.
        let leading = aabb.max[axis];
        let first = (leading - SKIN).ceil() as i32;
        let last = (leading + distance).floor() as i32;
        (first..=last)
            .find(|&layer| layer_is_solid(layer))
            .map(|layer| layer as f32)
    } else {
        // Layers whose near face, at `layer + 1`, lies below the leading face and within reach.
        let leading = aabb.min[axis];
        let first = (leading + SKIN).floor() as i32 - 1;
        let last = (leading + distance).ceil() as i32 - 1;
        (last..=first)
            .rev()
            .find(|&layer| layer_is_solid(layer))
            .map(|layer| (layer + 1) as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A box the size of the player's body, 0.6 × 1.8 × 0.6 m, with its feet at `feet`.
    fn body(feet: Vec3) -> Aabb {
        Aabb::new(
            feet - Vec3::new(0.3, 0.0, 0.3),
            feet + Vec3::new(0.3, 1.8, 0.3),
        )
    }

    /// Solid ground: every Cell at y = 0 or below. Its top is at y = 1.
    fn ground(cell: IVec3) -> bool {
        cell.y <= 0
    }

    /// Asserts that `actual` is `expected` to within 0.01 mm: moving a box adds and subtracts its
    /// size, which rounds in the last bits of an f32.
    #[track_caller]
    fn assert_box_near(actual: Aabb, expected: Aabb) {
        let close = |a: Vec3, b: Vec3| a.abs_diff_eq(b, 1e-5);
        assert!(
            close(actual.min, expected.min) && close(actual.max, expected.max),
            "expected {expected:?}, got {actual:?}"
        );
    }

    #[test]
    fn a_falling_body_stops_on_the_floor_and_is_grounded() {
        let start = body(Vec3::new(0.5, 3.0, 0.5));
        let moved = move_and_collide(start, Vec3::new(0.0, -5.0, 0.0), ground);
        // The feet fall from y = 3 to the floor's top at y = 1: 2 m of the 5 m asked for.
        assert_box_near(moved.aabb, body(Vec3::new(0.5, 1.0, 0.5)));
        assert!(moved.grounded);
        assert_eq!(moved.blocked, BVec3::new(false, true, false));
    }

    #[test]
    fn moving_diagonally_into_a_wall_slides_along_it() {
        // A wall filling x ≥ 3, standing on the ground.
        let world = |cell: IVec3| ground(cell) || cell.x >= 3;
        let start = body(Vec3::new(2.0, 1.0, 0.5));
        let moved = move_and_collide(start, Vec3::new(1.0, 0.0, 1.0), world);
        // The body's right side, at x = 2.3, stops at the wall's face at x = 3: 0.7 m of the 1 m
        // along x. Along z nothing is in the way, so it goes the whole 1 m, from 0.5 to 1.5.
        assert_box_near(moved.aabb, body(Vec3::new(2.7, 1.0, 1.5)));
        assert_eq!(moved.blocked, BVec3::new(true, false, false));
        assert!(!moved.grounded);
    }

    #[test]
    fn a_body_cant_fit_through_a_gap_one_cell_high() {
        // A wall one Cell thick at x = 3, with an opening at ground level `height` Cells high.
        let wall_with_gap =
            |height: i32| move |cell: IVec3| ground(cell) || (cell.x == 3 && cell.y > height);
        let start = body(Vec3::new(2.0, 1.0, 0.5));
        let delta = Vec3::new(3.0, 0.0, 0.0);

        // The body spans y = 1 to 2.8, so it reaches into the wall's Cell at y = 2 above the gap.
        // Its right side, at x = 2.3, stops at the wall's face at x = 3.
        let through_one = move_and_collide(start, delta, wall_with_gap(1));
        assert_box_near(through_one.aabb, body(Vec3::new(2.7, 1.0, 0.5)));
        assert!(through_one.blocked.x);

        // A gap two Cells high, from y = 1 to 3, is tall enough: it goes the whole 3 m.
        let through_two = move_and_collide(start, delta, wall_with_gap(2));
        assert_box_near(through_two.aabb, body(Vec3::new(5.0, 1.0, 0.5)));
        assert!(!through_two.blocked.x);
    }

    #[test]
    fn a_body_on_cell_boundaries_neither_falls_through_nor_is_pushed_sideways() {
        // A wall filling x ≤ 1, whose face at x = 2 the body's left side touches. The body's
        // sides lie on Cell boundaries, at x = 2 and z = 2, and its feet on the ground's top.
        let world = |cell: IVec3| ground(cell) || cell.x <= 1;
        let start = body(Vec3::new(2.3, 1.0, 2.3));
        assert_eq!(start.min, Vec3::new(2.0, 1.0, 2.0));

        // Gravity pulls it down a little, as every frame it stands there.
        let standing = move_and_collide(start, Vec3::new(0.0, -0.1, 0.0), world);
        assert_eq!(standing.aabb, start);
        assert!(standing.grounded);

        // Walking along the wall, it slides the whole 1 m along z, snagging neither on the ground
        // under it nor on the wall beside it.
        let walking = move_and_collide(start, Vec3::new(0.0, -0.1, 1.0), world);
        assert_box_near(walking.aabb, body(Vec3::new(2.3, 1.0, 3.3)));
        assert!(walking.grounded);
        assert_eq!(walking.blocked, BVec3::new(false, true, false));
    }

    #[test]
    fn a_body_sunk_into_the_floor_by_rounding_stands_on_it_and_walks() {
        // Feet 0.4 mm below the ground's top, as f32 rounding can leave them: within the 1 mm
        // that counts as touching.
        let start = body(Vec3::new(0.5, 1.0 - 0.0004, 0.5));
        let moved = move_and_collide(start, Vec3::new(1.0, -0.1, 0.0), ground);
        // Back flush with the ground's top at y = 1, and the whole 1 m along x.
        assert_box_near(moved.aabb, body(Vec3::new(1.5, 1.0, 0.5)));
        assert!(moved.grounded);
        assert!(!moved.blocked.x);
    }

    #[test]
    fn a_rising_body_stops_under_a_ceiling_without_being_grounded() {
        // A ceiling filling y ≥ 4, whose underside is at y = 4.
        let world = |cell: IVec3| ground(cell) || cell.y >= 4;
        let start = body(Vec3::new(0.5, 1.0, 0.5));
        let moved = move_and_collide(start, Vec3::new(0.0, 2.0, 0.0), world);
        // The head rises from y = 2.8 to the underside at y = 4: 1.2 m of the 2 m.
        assert_box_near(moved.aabb, body(Vec3::new(0.5, 2.2, 0.5)));
        assert!(moved.blocked.y);
        assert!(!moved.grounded);
    }
}
