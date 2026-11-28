//! How the player's body moves (`docs/plans/21-movement-controls.md`, seams 2 and 3), apart from
//! Bevy's input: walking, running and sneaking, a sneaking body kept on its ledge, the view
//! widening while running, and swimming out of a pool onto its bank.
//!
//! Frames are 1/60 s unless a test says otherwise.

use bevy::math::{IVec3, Vec3};
use blocks_not_included::player::movement::{
    BODY_SIZE, Body, Gait, JUMP_SPEED, Matter, MoveInput, RUN_SPEED, SNEAK_SPEED, SWIM_SPEED,
    WALK_SPEED, advance, gait, swimming,
};
use blocks_not_included::player::view::{
    FIELD_OF_VIEW, RUN_FIELD_OF_VIEW, RUN_VIEW_EASE, RunView,
};
use blocks_not_included::world::{MIN_SURFACE, liquid_fill};

const FRAME: f32 = 1.0 / 60.0;

/// Asserts that `actual` is `expected` to within 0.01 mm, the rounding of a few f32 steps.
#[track_caller]
fn assert_near(actual: Vec3, expected: Vec3) {
    assert!(
        actual.abs_diff_eq(expected, 1e-5),
        "expected {expected}, got {actual}"
    );
}

/// Solid ground at y ≤ 0, whose top is at y = 1, and gas above.
fn ground(cell: IVec3) -> Matter {
    if cell.y <= 0 {
        Matter::Solid
    } else {
        Matter::Open
    }
}

/// A body standing on [`ground`] at x = 0.5, z = 0.5.
fn standing() -> Body {
    Body {
        grounded: true,
        ..Body::standing_at(Vec3::new(0.5, 1.0, 0.5))
    }
}

fn forward() -> MoveInput {
    MoveInput {
        direction: Vec3::X,
        ..MoveInput::default()
    }
}

// --- Seam 2: on land ---

#[test]
fn walking_running_and_sneaking_each_go_at_their_own_speed() {
    assert_eq!((WALK_SPEED, RUN_SPEED, SNEAK_SPEED), (4.5, 7.0, 1.5));
    let body = standing();
    let run = MoveInput {
        run: true,
        ..forward()
    };
    let sneak = MoveInput {
        sneak: true,
        ..forward()
    };

    // 0.1 s at 4.5, 7 and 1.5 m/s: 0.45, 0.7 and 0.15 m. Gravity holds each on the ground.
    for (input, expected, distance) in [
        (forward(), Gait::Walk, 0.45),
        (run, Gait::Run, 0.7),
        (sneak, Gait::Sneak, 0.15),
    ] {
        assert_eq!(gait(&body, &input, ground), expected);
        let moved = advance(body, input, 0.1, ground);
        assert_near(moved.feet, Vec3::new(0.5 + distance, 1.0, 0.5));
        assert!(moved.grounded, "{expected:?}");
    }

    // Run held without moving is standing still, not running.
    let still = MoveInput {
        run: true,
        ..MoveInput::default()
    };
    assert_eq!(gait(&body, &still, ground), Gait::Walk);

    // In mid-air, running carries on through a jump.
    let airborne = Body {
        grounded: false,
        ..Body::standing_at(Vec3::new(0.5, 3.0, 0.5))
    };
    assert_eq!(gait(&airborne, &run, ground), Gait::Run);
}

#[test]
fn sneak_takes_precedence_over_run() {
    let both = MoveInput {
        run: true,
        sneak: true,
        ..forward()
    };
    assert_eq!(gait(&standing(), &both, ground), Gait::Sneak);
    let moved = advance(standing(), both, 0.1, ground);
    // 1.5 m/s for 0.1 s.
    assert_near(moved.feet, Vec3::new(0.65, 1.0, 0.5));

    // Sneaking is on the ground. In mid-air, Sneak still keeps Run from running, and gravity
    // pulls as ever: 25 m/s² for 0.1 s.
    let airborne = Body {
        grounded: false,
        ..Body::standing_at(Vec3::new(0.5, 5.0, 0.5))
    };
    assert_eq!(gait(&airborne, &both, ground), Gait::Walk);
    let fallen = advance(airborne, both, 0.1, ground);
    assert_near(fallen.velocity, Vec3::new(WALK_SPEED, -2.5, 0.0));
}

/// A ledge: solid ground whose top is at y = 1 where x < 3, and a drop to ground whose top is at
/// y = −4 where x ≥ 3.
fn ledge(cell: IVec3) -> Matter {
    if cell.y <= -5 || (cell.y <= 0 && cell.x < 3) {
        Matter::Solid
    } else {
        Matter::Open
    }
}

/// `body` after `seconds` of frames of `input`.
fn run_for(body: Body, input: MoveInput, seconds: f32, world: impl Fn(IVec3) -> Matter) -> Body {
    let frames = (seconds / FRAME).round() as usize;
    (0..frames).fold(body, |body, _| advance(body, input, FRAME, &world))
}

/// A body standing at x = 2.5 on [`ledge`], half a metre from its edge.
fn near_the_edge() -> Body {
    Body {
        grounded: true,
        ..Body::standing_at(Vec3::new(2.5, 1.0, 0.5))
    }
}

#[test]
fn walking_off_an_edge_falls_but_sneaking_stops_at_it() {
    // Walking for 1 s at 4.5 m/s carries the body 4.5 m, to x = 7, and down to the lower ground.
    let walked = run_for(near_the_edge(), forward(), 1.0, ledge);
    assert!((walked.feet.x - 7.0).abs() < 1e-3, "{}", walked.feet);
    assert!((walked.feet.y - -4.0).abs() < 1e-3, "{}", walked.feet);

    // Sneaking, its back edge, 0.3 m behind its feet, stays over the ledge: its feet stop short
    // of x = 3.3, still on the ground at y = 1.
    let sneak = MoveInput {
        sneak: true,
        ..forward()
    };
    let sneaked = run_for(near_the_edge(), sneak, 1.0, ledge);
    assert!(
        (3.25..3.3).contains(&sneaked.feet.x),
        "stopped at the edge: {}",
        sneaked.feet
    );
    assert_eq!(sneaked.feet.y, 1.0);
    assert!(sneaked.grounded);
    assert!(sneaked.velocity.x.abs() < 1e-3, "it no longer moves");

    // Sneaking at an angle to the edge, it slides along it: 1.5 m/s × √½ along z for 1 s.
    let along = MoveInput {
        direction: Vec3::new(1.0, 0.0, 1.0).normalize(),
        sneak: true,
        ..MoveInput::default()
    };
    let slid = run_for(near_the_edge(), along, 1.0, ledge);
    assert!((3.25..3.3).contains(&slid.feet.x), "{}", slid.feet);
    let z = 0.5 + SNEAK_SPEED * std::f32::consts::FRAC_1_SQRT_2;
    assert!((slid.feet.z - z).abs() < 1e-3, "{}", slid.feet);
    assert_eq!(slid.feet.y, 1.0);
}

#[test]
fn a_deliberate_jump_leaves_the_ledge_while_sneaking() {
    let sneak = MoveInput {
        sneak: true,
        ..forward()
    };
    let at_the_edge = run_for(near_the_edge(), sneak, 1.0, ledge);
    let jump = MoveInput { up: true, ..sneak };
    let jumped = advance(at_the_edge, jump, FRAME, ledge);
    assert!(jumped.velocity.y > 0.0, "it jumped");
    // Still holding Sneak and forward, it flies over the edge at walking speed and lands below.
    let landed = run_for(jumped, sneak, 1.5, ledge);
    assert!(landed.feet.x > 3.3, "{}", landed.feet);
    assert!((landed.feet.y - -4.0).abs() < 1e-3, "{}", landed.feet);
}

#[test]
fn the_view_widens_while_running_and_narrows_when_running_stops_or_sneak_takes_over() {
    assert_eq!((FIELD_OF_VIEW, RUN_FIELD_OF_VIEW, RUN_VIEW_EASE), (70.0, 76.0, 0.25));
    let mut view = RunView::default();
    assert_eq!(view.field_of_view(), 70.0);

    // Half the 0.25 s ease, where it eases in and out symmetrically: halfway, 73°.
    view.update(true, 0.125);
    assert!((view.field_of_view() - 73.0).abs() < 1e-4);
    view.update(true, 0.125);
    assert_eq!(view.field_of_view(), 76.0);
    view.update(true, 1.0);
    assert_eq!(view.field_of_view(), 76.0, "and stays there");

    // A long frame doesn't overshoot.
    view.update(false, 10.0);
    assert_eq!(view.field_of_view(), 70.0);

    // Running, as the gait says, ends when movement stops or Sneak takes over.
    let body = standing();
    let run = MoveInput {
        run: true,
        ..forward()
    };
    for ends in [
        MoveInput {
            run: true,
            ..MoveInput::default()
        },
        MoveInput {
            sneak: true,
            ..run
        },
    ] {
        let mut view = RunView::default();
        view.update(gait(&body, &run, ground) == Gait::Run, 0.25);
        assert_eq!(view.field_of_view(), 76.0);
        view.update(gait(&body, &ends, ground) == Gait::Run, 0.25);
        assert_eq!(view.field_of_view(), 70.0, "{ends:?}");
    }
}

// --- Seam 3: liquids ---

#[test]
fn the_body_and_the_fluid_mesh_share_how_full_a_liquid_cell_is() {
    // 600 kg of Water in a Cell whose Capacity is 1,000 kg: its surface is 0.6 of the way up.
    assert_eq!(liquid_fill(600_000, 1_000_000.0, false), 0.6);
    // A film is drawn, and swum in, at 5 % of the Cell.
    assert_eq!(MIN_SURFACE, 0.05);
    assert_eq!(liquid_fill(1, 1_000_000.0, false), MIN_SURFACE);
    // Under more of the same liquid, the Cell is full however much it holds.
    assert_eq!(liquid_fill(600_000, 1_000_000.0, true), 1.0);
}

/// A pool on solid ground whose top is at y = 1, with `cells` layers of liquid from y = 1 up.
/// The top layer is `top_fill` full; those under it are full.
fn pool(cells: i32, top_fill: f32) -> impl Fn(IVec3) -> Matter {
    move |cell: IVec3| match cell.y {
        ..=0 => Matter::Solid,
        y if y < 1 + cells => Matter::Liquid {
            fill: if y == cells { top_fill } else { 1.0 },
        },
        _ => Matter::Open,
    }
}

#[test]
fn a_body_is_swimming_only_while_the_liquid_surface_is_above_its_centre() {
    // Standing on the floor, its centre is 0.9 m up, at y = 1.9.
    let body = standing();
    for (cells, top_fill, swims) in [
        // A shallow pool 0.6 full: wading, at walking speed.
        (1, 0.6, false),
        // 0.95 full, its surface at 1.95, above the centre.
        (1, 0.95, true),
        // A full layer, and a film on top: the surface is at 2.05.
        (2, MIN_SURFACE, true),
    ] {
        let world = pool(cells, top_fill);
        assert_eq!(swimming(&body, &world), swims, "{cells} at {top_fill}");
        let moved = advance(body, forward(), 0.1, &world);
        // 0.1 s at 4.5 m/s on foot, or half that swimming.
        let speed = if swims { WALK_SPEED / 2.0 } else { WALK_SPEED };
        assert!(
            (moved.feet.x - (0.5 + speed * 0.1)).abs() < 1e-5,
            "{cells} at {top_fill}: {}",
            moved.feet
        );
        assert_eq!(
            gait(&body, &forward(), &world),
            if swims { Gait::Swim } else { Gait::Walk }
        );
    }
}

/// A pool beside a bank: solid ground whose top is at y = 1, liquid in `cells` layers from y = 1
/// up where x < 3, the top layer `top_fill` full, and the bank where x ≥ 3.
#[derive(Clone, Copy, Debug)]
struct Bank {
    cells: i32,
    top_fill: f32,
    /// The top of the bank, if there is one: solid where x ≥ 3 below it. Without a bank, the
    /// pool goes on where x ≥ 3.
    top: Option<i32>,
    /// A ceiling over the bank: solid from this height up, where x ≥ 3.
    bank_ceiling: Option<i32>,
    /// A ceiling over the pool: solid from this height up, where x < 3.
    pool_ceiling: Option<i32>,
}

impl Bank {
    fn new(cells: i32, top_fill: f32, top: i32) -> Self {
        Self {
            cells,
            top_fill,
            top: Some(top),
            bank_ceiling: None,
            pool_ceiling: None,
        }
    }

    /// Where the liquid's surface is.
    fn waterline(&self) -> f32 {
        self.cells as f32 + self.top_fill
    }

    fn matter(&self, cell: IVec3) -> Matter {
        if cell.y <= 0 {
            return Matter::Solid;
        }
        let (ceiling, bank) = if cell.x >= 3 {
            (self.bank_ceiling, self.top)
        } else {
            (self.pool_ceiling, None)
        };
        if ceiling.is_some_and(|ceiling| cell.y >= ceiling) || bank.is_some_and(|top| cell.y < top)
        {
            Matter::Solid
        } else if cell.y < 1 + self.cells {
            Matter::Liquid {
                fill: if cell.y == self.cells {
                    self.top_fill
                } else {
                    1.0
                },
            }
        } else {
            Matter::Open
        }
    }

    /// Whether `body` overlaps none of the solid Cells by more than 1 mm, the collision's own
    /// tolerance for rounding.
    fn holds_clear(&self, body: &Body) -> bool {
        let aabb = body.aabb();
        let cells = |min: f32, max: f32| (min + 1e-3).floor() as i32..(max - 1e-3).ceil() as i32;
        cells(aabb.min.x, aabb.max.x).all(|x| {
            cells(aabb.min.y, aabb.max.y).all(|y| {
                cells(aabb.min.z, aabb.max.z)
                    .all(|z| self.matter(IVec3::new(x, y, z)) != Matter::Solid)
            })
        })
    }
}

/// What became of a body holding Jump and forward, towards the bank, from the pool's floor at
/// x = 1.5, for at most 5 s.
#[derive(Debug)]
struct Swim {
    /// Whether it ended standing on the bank.
    on_the_bank: bool,
    /// The highest its feet rose while still in the pool, where x < 3.3.
    highest_in_the_pool: f32,
    /// The fastest it rose.
    fastest_rise: f32,
    /// The fastest it rose with its centre more than 0.2 m under the surface.
    fastest_rise_submerged: f32,
}

fn swim_to(bank: Bank) -> Swim {
    let world = |cell: IVec3| bank.matter(cell);
    let input = MoveInput {
        up: true,
        ..forward()
    };
    let mut body = Body::standing_at(Vec3::new(1.5, 1.0, 0.5));
    let mut swim = Swim {
        on_the_bank: false,
        highest_in_the_pool: body.feet.y,
        fastest_rise: 0.0,
        fastest_rise_submerged: 0.0,
    };
    for frame in 0..300 {
        body = advance(body, input, FRAME, world);
        assert!(bank.holds_clear(&body), "frame {frame}: {body:?} in {bank:?}");
        swim.fastest_rise = swim.fastest_rise.max(body.velocity.y);
        if bank.cells > 0 && body.centre().y < bank.waterline() - 0.2 {
            swim.fastest_rise_submerged = swim.fastest_rise_submerged.max(body.velocity.y);
        }
        // Its back edge isn't yet past the bank's face, if there is a bank.
        if bank.top.is_none() || body.feet.x - BODY_SIZE.x / 2.0 < 3.0 {
            swim.highest_in_the_pool = swim.highest_in_the_pool.max(body.feet.y);
        } else if let Some(top) = bank.top
            && body.grounded
            && (body.feet.y - top as f32).abs() < 1e-3
        {
            swim.on_the_bank = true;
            break;
        }
    }
    swim
}

#[test]
fn holding_jump_towards_a_bank_up_to_one_cell_above_the_waterline_climbs_out() {
    for bank in [
        // A shallow pool 0.6 full, waterline 1.6: wading, and the bank's top 0.4 m above it.
        Bank::new(1, 0.6, 2),
        // A full layer and a half-full one, waterline 2.5: swimming, the top 0.5 m above.
        Bank::new(2, 0.5, 3),
        // Three full layers, waterline 4: the bank's top a whole Cell above it.
        Bank::new(3, 1.0, 5),
    ] {
        let swim = swim_to(bank);
        assert!(swim.on_the_bank, "{bank:?}, waterline {}: {swim:?}", bank.waterline());
        // It swims up to the surface at 3 m/s, and climbs only once afloat there.
        assert!(
            swim.fastest_rise_submerged <= SWIM_SPEED + 1e-4,
            "{bank:?}: {swim:?}"
        );
    }
}

#[test]
fn without_a_reachable_bank_the_swimmer_only_treads_water() {
    let deep = Bank::new(3, 1.0, 5);
    for bank in [
        // No bank: the pool goes on.
        Bank { top: None, ..deep },
        // A wall two Cells above the waterline.
        Bank::new(3, 1.0, 6),
        // A ceiling over the bank one Cell above its top, too low for the body.
        Bank {
            bank_ceiling: Some(6),
            ..deep
        },
        // A ceiling over the pool, two Cells above the waterline: the body can't rise far
        // enough beneath it to step onto the bank.
        Bank {
            pool_ceiling: Some(6),
            ..deep
        },
    ] {
        let swim = swim_to(bank);
        assert!(!swim.on_the_bank, "{bank:?}: {swim:?}");
        // Treading water at the surface: it swims up at 3 m/s, and its centre, 0.9 m above its
        // feet, rises no more than 3² / (2 × 25) = 0.18 m out of the water before it falls back,
        // plus up to 3 × 1/60 = 0.05 m on the frame it breaks the surface.
        assert!(swim.fastest_rise <= SWIM_SPEED + 1e-4, "{bank:?}: {swim:?}");
        let highest = bank.waterline() - 0.9 + 0.18 + 0.05;
        assert!(
            swim.highest_in_the_pool <= highest + 0.01,
            "{bank:?}: {swim:?}"
        );
    }
}

#[test]
fn dry_jumping_is_as_it_was() {
    // Without water, a bank one Cell up is a step a jump clears, at the jump's own speed.
    let dry = |top: i32| Bank::new(0, 0.0, top);
    let step = swim_to(dry(2));
    assert!(step.on_the_bank, "{step:?}");
    // It leaves at 8 m/s and loses 25 m/s² × 1/60 s in its first frame.
    assert!((step.fastest_rise - (JUMP_SPEED - 25.0 * FRAME)).abs() < 1e-4, "{step:?}");

    // Two Cells up is a wall.
    let wall = swim_to(dry(3));
    assert!(!wall.on_the_bank, "{wall:?}");
    assert!(wall.fastest_rise <= JUMP_SPEED, "{wall:?}");
}
