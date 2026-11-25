//! Finding the first Cell a ray meets, by walking the grid one Cell at a time: the traversal of
//! Amanatides and Woo, "A Fast Voxel Traversal Algorithm for Ray Tracing" (1987).

use bevy::math::{BVec3, IVec3, Vec3};

/// How far the player reaches, in metres.
pub const REACH: f32 = 8.0;

/// The Cell a ray met.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hit {
    pub cell: IVec3,
    /// The face the ray entered `cell` through, as the unit step from `cell` out through it: the
    /// way back towards the ray's origin along one axis. Zero if the ray started inside `cell`.
    pub normal: IVec3,
}

/// The first Cell along the ray from `origin` in `direction` for which `is_target` holds, if the
/// ray enters it within `max_distance`, which must be finite. The Cell at `(x, y, z)` fills the
/// box from `(x, y, z)` to `(x + 1, y + 1, z + 1)`, and the Cell `origin` lies in comes first.
///
/// Where the ray crosses an edge or a corner exactly, it steps along x first, then y, then z.
pub fn raycast(
    origin: Vec3,
    direction: Vec3,
    max_distance: f32,
    is_target: impl Fn(IVec3) -> bool,
) -> Option<Hit> {
    debug_assert!(
        max_distance.is_finite(),
        "a ray of endless reach never ends"
    );
    let mut cell = origin.floor().as_ivec3();
    if is_target(cell) {
        return Some(Hit {
            cell,
            normal: IVec3::ZERO,
        });
    }
    let direction = direction.normalize_or_zero();
    // Which way the ray steps along each axis: 0 along an axis it runs parallel to.
    let step = IVec3::select(
        direction.cmpgt(Vec3::ZERO),
        IVec3::ONE,
        IVec3::select(direction.cmplt(Vec3::ZERO), IVec3::NEG_ONE, IVec3::ZERO),
    );
    let stepping = step.cmpne(IVec3::ZERO);
    if stepping == BVec3::FALSE {
        return None;
    }
    // How far along the ray it takes to cross one whole Cell along each axis.
    let crossing = Vec3::select(stepping, (1.0 / direction).abs(), Vec3::INFINITY);
    // How far along the ray the next Cell boundary lies along each axis.
    let next_boundary = cell.as_vec3() + step.max(IVec3::ZERO).as_vec3();
    let mut to_boundary = Vec3::select(
        stepping,
        (next_boundary - origin) / direction,
        Vec3::INFINITY,
    );
    loop {
        let axis = nearest_axis(to_boundary);
        let distance = to_boundary[axis];
        // A NaN distance, from a NaN origin, ends the walk too.
        if distance.is_nan() || distance > max_distance {
            return None;
        }
        cell[axis] += step[axis];
        if is_target(cell) {
            let mut normal = IVec3::ZERO;
            normal[axis] = -step[axis];
            return Some(Hit { cell, normal });
        }
        to_boundary[axis] += crossing[axis];
    }
}

/// The axis whose distance is smallest, preferring x, then y, then z.
fn nearest_axis(distances: Vec3) -> usize {
    let mut nearest = 0;
    for axis in [1, 2] {
        if distances[axis] < distances[nearest] {
            nearest = axis;
        }
    }
    nearest
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The centre of the Cell at `(x, y, z)`.
    fn centre_of(x: i32, y: i32, z: i32) -> Vec3 {
        IVec3::new(x, y, z).as_vec3() + Vec3::splat(0.5)
    }

    #[test]
    fn a_cell_straight_ahead_is_hit_through_the_face_facing_the_origin() {
        for direction in [
            IVec3::X,
            IVec3::NEG_X,
            IVec3::Y,
            IVec3::NEG_Y,
            IVec3::Z,
            IVec3::NEG_Z,
        ] {
            // Three Cells ahead of the origin's Cell, (0, 0, 0), along this axis.
            let target = direction * 3;
            let hit = raycast(centre_of(0, 0, 0), direction.as_vec3(), REACH, |cell| {
                cell == target
            });
            // The ray enters through the face that looks back at the origin.
            let expected = Hit {
                cell: target,
                normal: -direction,
            };
            assert_eq!(hit, Some(expected), "looking along {direction}");
        }
    }

    #[test]
    fn a_diagonal_ray_passes_through_every_cell_it_crosses_in_order() {
        // From the centre of (0, 0, 0), 2 m along x for every 1 m along y. Measured along x, the
        // ray crosses x = 1, 2, 3, 4 after 0.5, 1.5, 2.5 and 3.5 m, and y = 1 and 2 after 1 and
        // 3 m. Each metre along x is √5 / 2 ≈ 1.118 m along the ray, so the last crossing within
        // 4.5 m is x = 4, at 3.5 × 1.118 ≈ 3.91 m. The next, x = 5, is at 4.5 × 1.118 ≈ 5.03 m.
        let visited = std::cell::RefCell::new(Vec::new());
        let hit = raycast(centre_of(0, 0, 0), Vec3::new(2.0, 1.0, 0.0), 4.5, |cell| {
            visited.borrow_mut().push(cell);
            false
        });
        assert_eq!(hit, None);
        let expected = [(0, 0), (1, 0), (1, 1), (2, 1), (3, 1), (3, 2), (4, 2)]
            .map(|(x, y)| IVec3::new(x, y, 0));
        assert_eq!(visited.into_inner(), expected);
    }

    #[test]
    fn a_ray_misses_a_target_beyond_its_reach() {
        let target_at = |x: i32| move |cell: IVec3| cell == IVec3::new(x, 0, 0);
        // From the centre of (0, 0, 0) along +x, the Cell at x = 9 starts 8.5 m away, past the
        // 8 m reach.
        assert_eq!(
            raycast(centre_of(0, 0, 0), Vec3::X, REACH, target_at(9)),
            None
        );
        // The Cell at x = 8 starts 7.5 m away, within it.
        let hit = raycast(centre_of(0, 0, 0), Vec3::X, REACH, target_at(8));
        assert_eq!(hit.map(|hit| hit.cell), Some(IVec3::new(8, 0, 0)));
    }

    #[test]
    fn a_ray_starting_inside_a_target_cell_reports_that_cell() {
        // (2.3, 1.7, −0.4) lies in the Cell at (2, 1, −1).
        let inside = IVec3::new(2, 1, -1);
        let hit = raycast(Vec3::new(2.3, 1.7, -0.4), Vec3::NEG_Y, REACH, |cell| {
            cell == inside || cell.y < 0
        });
        // It entered through no face.
        let expected = Hit {
            cell: inside,
            normal: IVec3::ZERO,
        };
        assert_eq!(hit, Some(expected));
    }
}
