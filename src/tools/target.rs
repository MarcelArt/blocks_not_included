//! Which Cell a god tool changes, given the Cell in view.

use bevy::math::IVec3;
use sim::Phase;

use crate::player::raycast::Hit;

/// Whether the player's line of sight stops at a Cell in `phase`. It stops at solids and liquids,
/// and passes through Vacuum and gases, which fill the space the player looks through.
pub fn stops_sight(phase: Phase) -> bool {
    matches!(phase, Phase::Solid | Phase::Liquid)
}

/// The Cell Place and Spawn fluid fill when `hit` is the Cell in view: the one in front of the
/// face the line of sight entered it through, provided that Cell holds Vacuum or a gas.
/// `phase_at` gives the Phase of a Cell, or `None` where it isn't known, which refuses it.
///
/// A line of sight that starts inside `hit` entered it through no face, so this is `hit` itself,
/// which is refused: sight stops only at solids and liquids.
pub fn placement_target(hit: Hit, phase_at: impl Fn(IVec3) -> Option<Phase>) -> Option<IVec3> {
    let cell = hit.cell + hit.normal;
    matches!(phase_at(cell), Some(Phase::Vacuum | Phase::Gas)).then_some(cell)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A line of sight that entered the Cell at (3, 0, 0) through its −x face.
    const HIT: Hit = Hit {
        cell: IVec3::new(3, 0, 0),
        normal: IVec3::NEG_X,
    };

    /// The Cell in front of [`HIT`]'s face: (3, 0, 0) + (−1, 0, 0).
    const IN_FRONT: IVec3 = IVec3::new(2, 0, 0);

    /// Granite at the hit Cell, `in_front` in front of it, and nothing known anywhere else.
    fn world(in_front: Option<Phase>) -> impl Fn(IVec3) -> Option<Phase> {
        move |cell| match cell {
            cell if cell == HIT.cell => Some(Phase::Solid),
            IN_FRONT => in_front,
            _ => None,
        }
    }

    #[test]
    fn placement_fills_the_cell_in_front_of_the_face_hit_when_it_holds_vacuum_or_gas() {
        for phase in [Phase::Vacuum, Phase::Gas] {
            let target = placement_target(HIT, world(Some(phase)));
            assert_eq!(target, Some(IN_FRONT), "in front: {phase:?}");
        }
    }

    #[test]
    fn placement_is_refused_when_the_cell_in_front_is_solid_liquid_or_unknown() {
        for phase in [Some(Phase::Solid), Some(Phase::Liquid), None] {
            let target = placement_target(HIT, world(phase));
            assert_eq!(target, None, "in front: {phase:?}");
        }
    }

    #[test]
    fn placement_is_refused_when_the_line_of_sight_started_inside_the_hit_cell() {
        let inside = Hit {
            normal: IVec3::ZERO,
            ..HIT
        };
        // The Cell "in front" is the hit Cell itself, which is solid.
        assert_eq!(placement_target(inside, world(Some(Phase::Vacuum))), None);
    }
}
