//! The player: a body that walks, jumps, flies and swims through the world in first person, and
//! that the Active Region follows. It exists while a World is open.
//!
//! Controls, while input is gameplay input (`crate::input_policy`): the mouse looks around, as
//! fast as the player's mouse sensitivity says. The movement actions move, Jump jumps and Sneak
//! sneaks ([`movement`]). A tap of Run while going forward runs, until Forward is let go, Sneak is
//! held or the body hits a wall ([`TapRun`]). Pressing Jump twice quickly switches flying on and
//! off ([`flight`]). Flying goes through solids; Jump rises and Fly down sinks. In a liquid,
//! holding Jump swims upward, and climbs out onto a bank. The view widens while running
//! ([`view`]). The actions are bound in Settings (`crate::settings`), by default to WASD, Space,
//! Ctrl and Shift: Shift sneaks on foot and flies down in flight. While Alt is held, the body
//! takes no input but still falls, swims and slides; while the World is paused, it doesn't move at
//! all.
//!
//! A World opened without a player spawns them on the ground below the focus ([`ground_below`]).
//! A saved World's player carries on exactly as saved: where they were, looking the same way,
//! moving as they were. A new World from the main menu is saved before it opens, with its player
//! already standing on that ground, so it opens as saved. Either way, the player is held in place
//! until the Chunks around them have loaded.

pub mod collision;
pub mod flight;
pub mod movement;
pub mod raycast;
pub mod view;

use std::f32::consts::FRAC_PI_2;

use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use bevy_egui::PrimaryEguiContext;
use sim::{CHUNK_SIZE, CellPos, ChunkPos, Phase, WORLD_BOTTOM_Y, WORLD_TOP_Y};

use crate::input_policy::{InputPolicy, gameplay_input};
use crate::session::{Play, PlayerStart, Session, SessionSystems};
use crate::settings::{Action, ActionInput, Bindings, Settings};
use crate::sim_bridge::{Elements, RENDER_DISTANCE, SimFocus};
use crate::world::{WorldView, capacity_grams, liquid_fill};
use flight::{FlightToggle, JumpFrame};
use movement::{Body, Gait, Matter, MoveInput, MoveMode, gait, move_body, swimming};
use view::{FIELD_OF_VIEW, RunView};

/// How far the view turns per pixel the mouse moves at a mouse sensitivity of 1, in radians: about
/// a tenth of a degree.
pub const LOOK_SPEED: f32 = 0.002;

/// How far the view may tilt up or down: just short of straight up and down, where yaw would
/// become ambiguous.
const PITCH_LIMIT: f32 = FRAC_PI_2 - 0.01;

/// The longest step of time a single frame moves the body through, in seconds. After a stall,
/// the body moves less than the time that passed, rather than one long, sudden step.
const MAX_FRAME_STEP: f32 = 0.1;

/// The player: a first-person camera at the eyes of a body that moves through the world, and the
/// Active Region's focus. Needs the session, the input policy and the sim bridge.
pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Settings>().add_systems(
            OnEnter(Session::Open),
            spawn_player.in_set(SessionSystems::Build),
        )
        .add_systems(
            Update,
            (
                place_on_ground,
                release_saved_player,
                look_around.run_if(gameplay_input),
                move_player,
                focus_on_player,
            )
                .chain()
                .in_set(PlayerSystems)
                .run_if(in_state(Play::Running)),
        )
        // Pausing, leaving the World or closing it forgets a first press of Jump, and stops
        // running.
        .add_systems(OnExit(Play::Running), (forget_jump_presses, stop_running));
    }
}

/// The player's systems in `Update`, which run while the World runs. Once they have run, the
/// player has turned and moved for the frame.
#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub struct PlayerSystems;

/// The player's body. Its entity is the camera, at the body's eyes.
#[derive(Component, Clone, Copy, Debug, Deref, DerefMut)]
pub struct Player(pub Body);

/// Holds the player in place, neither moving nor moving the focus, until the Chunks around them
/// have loaded.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub enum AwaitingSpawn {
    /// A new World's player: once the focus's column has loaded, they stand on its highest solid
    /// Cell.
    OnGround,
    /// A saved World's player: once the Chunks around them have loaded, they carry on exactly as
    /// saved.
    AsSaved,
}

/// Which way a new player faces: along +x and +z, away from the sun.
pub fn spawn_facing() -> Quat {
    Transform::default()
        .looking_to(Vec3::new(1.0, 0.0, 1.0), Vec3::Y)
        .rotation
}

/// Spawns the player with the World, held in place until the Chunks around them load: as saved,
/// or at the focus, facing as [`spawn_facing`] says. Their camera holds egui's primary context,
/// which a new camera doesn't otherwise get once the first World's camera is gone.
fn spawn_player(mut commands: Commands, start: Res<PlayerStart>, focus: Res<SimFocus>) {
    let (body, rotation, awaiting) = match start.0 {
        Some(saved) => (saved.body, saved.rotation(), AwaitingSpawn::AsSaved),
        None => {
            let body = Body::standing_at(feet_in(focus.0));
            (body, spawn_facing(), AwaitingSpawn::OnGround)
        }
    };
    commands.spawn((
        Player(body),
        awaiting,
        FlightToggle::default(),
        TapRun::default(),
        RunView::default(),
        Camera3d::default(),
        Projection::from(PerspectiveProjection {
            fov: FIELD_OF_VIEW.to_radians(),
            ..default()
        }),
        Transform::from_translation(body.eyes()).with_rotation(rotation),
        PrimaryEguiContext,
        DespawnOnExit(Session::Open),
    ));
}

/// The Chunk layers of the world, from the lowest to the highest.
fn world_chunk_layers() -> std::ops::RangeInclusive<i32> {
    let size = CHUNK_SIZE as i32;
    WORLD_BOTTOM_Y / size..=WORLD_TOP_Y / size - 1
}

/// The Chunks of `focus`'s column that [`ground_below`] looks through: render distance above and
/// below the focus's Chunk, within the world's limits. Lowest first.
pub fn ground_column(focus: CellPos) -> impl Iterator<Item = ChunkPos> {
    let centre = ChunkPos::of(focus);
    let layers = world_chunk_layers();
    let lowest = (centre.y - RENDER_DISTANCE.vertical).max(*layers.start());
    let highest = (centre.y + RENDER_DISTANCE.vertical).min(*layers.end());
    (lowest..=highest).map(move |y| ChunkPos::new(centre.x, y, centre.z))
}

/// Where a player standing on the ground below `focus` stands: in the Cell just above the highest
/// solid Cell of `focus`'s column within [`ground_column`], or at `focus` if none of it is solid.
/// `is_solid` says which Cells are.
pub fn ground_below(focus: CellPos, is_solid: impl Fn(CellPos) -> bool) -> CellPos {
    let size = CHUNK_SIZE as i32;
    let column: Vec<ChunkPos> = ground_column(focus).collect();
    let (Some(lowest), Some(highest)) = (column.first(), column.last()) else {
        return focus;
    };
    let CellPos { x, z, .. } = focus;
    (lowest.y * size..(highest.y + 1) * size)
        .rev()
        .map(|y| CellPos::new(x, y, z))
        .find(|&pos| is_solid(pos))
        .map_or(focus, |pos| CellPos::new(x, pos.y + 1, z))
}

/// Once every Chunk of the focus's column within render distance is loaded (the origin column,
/// where a new World's focus starts), stands a new World's player on that column's highest solid
/// Cell, as [`ground_below`] finds it.
fn place_on_ground(
    mut commands: Commands,
    view: Res<WorldView>,
    elements: Res<Elements>,
    focus: Res<SimFocus>,
    player: Single<(Entity, &mut Player, &mut Transform, &AwaitingSpawn)>,
) {
    let (entity, mut player, mut transform, awaiting) = player.into_inner();
    if *awaiting != AwaitingSpawn::OnGround {
        return;
    }
    if !ground_column(focus.0).all(|chunk| view.chunk(chunk).is_some()) {
        return;
    }
    let feet = ground_below(focus.0, |pos| {
        view.cell(pos)
            .is_some_and(|cell| elements.get(cell.element).phase == Phase::Solid)
    });

    **player = Body::standing_at(feet_in(feet));
    transform.translation = player.eyes();
    commands.entity(entity).remove::<AwaitingSpawn>();
    info!("the player spawned at ({}, {}, {})", feet.x, feet.y, feet.z);
}

/// Once the Chunk a saved World's player stands in and every Chunk around it are loaded, or lie
/// beyond the world's limits, lets the player carry on as saved.
///
/// A generated world's Atmosphere fills every Chunk within its limits, so each of these loads
/// into the world view.
fn release_saved_player(
    mut commands: Commands,
    view: Res<WorldView>,
    player: Single<(Entity, &Player, &AwaitingSpawn)>,
) {
    let (entity, player, awaiting) = player.into_inner();
    if *awaiting != AwaitingSpawn::AsSaved {
        return;
    }
    let feet = player.feet.floor().as_ivec3();
    let centre = ChunkPos::of(CellPos::new(feet.x, feet.y, feet.z));
    let layers = world_chunk_layers();
    let mut around = (-1..=1).flat_map(|x| {
        (-1..=1).flat_map(move |y| {
            (-1..=1).map(move |z| ChunkPos::new(centre.x + x, centre.y + y, centre.z + z))
        })
    });
    if around.all(|pos| !layers.contains(&pos.y) || view.chunk(pos).is_some()) {
        commands.entity(entity).remove::<AwaitingSpawn>();
        info!(
            "the player carries on at ({:.2}, {:.2}, {:.2}), as saved",
            player.feet.x, player.feet.y, player.feet.z
        );
    }
}

/// Turns the view with the mouse, as fast as the player's mouse sensitivity says. A player held
/// while Chunks load keeps looking the way they spawned.
fn look_around(
    motion: Res<AccumulatedMouseMotion>,
    settings: Res<Settings>,
    mut player: Single<&mut Transform, (With<Player>, Without<AwaitingSpawn>)>,
) {
    if motion.delta == Vec2::ZERO {
        return;
    }
    // The mouse reports the whole movement since the last frame, so it isn't scaled by time.
    let turn = motion.delta * LOOK_SPEED * settings.mouse_sensitivity;
    let (yaw, pitch, _) = player.rotation.to_euler(EulerRot::YXZ);
    let yaw = yaw - turn.x;
    let pitch = (pitch - turn.y).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    player.rotation = Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0);
}

/// What [`move_player`] changes on the player.
type MovingPlayer = (
    &'static mut Player,
    &'static mut FlightToggle,
    &'static mut TapRun,
    &'static mut RunView,
    &'static mut Transform,
    &'static mut Projection,
);

/// Moves the body as the controls say, through the world as the game last heard it from the
/// simulation, and keeps the camera at its eyes. Two quick presses of Jump switch flying first.
/// Without gameplay input, the body moves as if no control were held. The view widens while the
/// body runs, and narrows again after.
fn move_player(
    time: Res<Time>,
    real_time: Res<Time<Real>>,
    actions: ActionInput,
    policy: Res<InputPolicy>,
    view: Res<WorldView>,
    elements: Res<Elements>,
    player: Single<MovingPlayer, Without<AwaitingSpawn>>,
) {
    let (mut player, mut toggle, mut run, mut run_view, mut transform, mut projection) =
        player.into_inner();
    let matter_at = |cell: IVec3| matter_in(&view, &elements, cell);
    let jump = JumpFrame {
        now: real_time.elapsed(),
        bound: actions.bindings().inputs(Action::Jump),
        pressed: actions.just_pressed(Action::Jump),
        held: actions.pressed(Action::Jump),
        gameplay: policy.gameplay,
        swimming: swimming(&player, matter_at),
    };
    if toggle.read(jump) {
        player.mode = player.mode.toggled();
        player.velocity = Vec3::ZERO;
        info!("flying: {}", player.mode == MoveMode::Fly);
    }
    let controls = if policy.gameplay {
        Controls::read(&actions)
    } else {
        Controls::default()
    };

    let dt = time.delta_secs().min(MAX_FRAME_STEP);
    let (body, gait) = step(**player, &controls, &transform, &mut run, dt, matter_at);
    **player = body;
    run_view.update(gait == Gait::Run, dt);
    show_field_of_view(&mut projection, run_view.field_of_view());
    transform.translation = player.eyes();
}

/// One frame of `controls` for `body`, which faces as `facing` does: starts or stops running as
/// `run` says, then moves the body for `dt` seconds through the Cells `matter_at` describes. A
/// wall the body hits across the ground stops running on the same frame. Returns the moved body,
/// and how it went once a hit is taken into account, which the view follows.
fn step(
    body: Body,
    controls: &Controls,
    facing: &Transform,
    run: &mut TapRun,
    dt: f32,
    matter_at: impl Fn(IVec3) -> Matter,
) -> (Body, Gait) {
    let on_foot = body.mode == MoveMode::Walk && !swimming(&body, &matter_at);
    let input = controls.move_input(facing, run.read(controls, on_foot));
    let moved = move_body(body, input, dt, &matter_at);
    if moved.hit_wall {
        run.stop();
    }
    let resolved = MoveInput {
        run: run.running(),
        ..input
    };
    (moved.body, gait(&body, &resolved, matter_at))
}

/// What the body meets in the Cell at `cell`. A liquid's surface stands as high as the fluid
/// meshes draw it. A Cell whose Chunk isn't loaded is solid, so the body can't fall into unloaded
/// space.
fn matter_in(view: &WorldView, elements: &Elements, cell: IVec3) -> Matter {
    let pos = CellPos::new(cell.x, cell.y, cell.z);
    let Some(here) = view.cell(pos) else {
        return Matter::Solid;
    };
    let element = elements.get(here.element);
    match element.phase {
        Phase::Solid => Matter::Solid,
        Phase::Liquid => {
            let above = view.cell(CellPos::new(cell.x, cell.y + 1, cell.z));
            let same_above = above.is_some_and(|above| above.element == here.element);
            Matter::Liquid {
                fill: liquid_fill(here.mass_grams, capacity_grams(element), same_above),
            }
        }
        Phase::Gas | Phase::Vacuum => Matter::Open,
    }
}

/// Sets the camera's vertical field of view to `degrees`, marking the projection changed only if
/// it does change.
fn show_field_of_view(projection: &mut Mut<Projection>, degrees: f32) {
    let fov = degrees.to_radians();
    if let Projection::Perspective(perspective) = projection.bypass_change_detection()
        && perspective.fov != fov
    {
        perspective.fov = fov;
        projection.set_changed();
    }
}

/// Forgets a first press of Jump, as the World stops running.
fn forget_jump_presses(mut toggles: Query<&mut FlightToggle>) {
    for mut toggle in &mut toggles {
        toggle.reset();
    }
}

/// Stops the player running, as the World stops running.
fn stop_running(mut runs: Query<&mut TapRun>) {
    for mut run in &mut runs {
        run.stop();
    }
}

/// The movement actions during one frame, read through the player's bindings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Controls {
    forward: bool,
    back: bool,
    left: bool,
    right: bool,
    jump: bool,
    descend: bool,
    sneak: bool,
    /// Whether an input bound to Run went down during the frame. Holding Run doesn't matter.
    run_tapped: bool,
}

impl Controls {
    fn read(actions: &ActionInput) -> Self {
        Self {
            forward: actions.pressed(Action::MoveForward),
            back: actions.pressed(Action::MoveBack),
            left: actions.pressed(Action::MoveLeft),
            right: actions.pressed(Action::MoveRight),
            jump: actions.pressed(Action::Jump),
            descend: actions.pressed(Action::Descend),
            sneak: actions.pressed(Action::Sneak),
            run_tapped: actions.just_pressed(Action::Run),
        }
    }

    /// Whether they go forward: Forward held and Back not, with or without going sideways too.
    fn going_forward(&self) -> bool {
        self.forward && !self.back
    }

    /// What they ask of the body, which faces as `facing` does and runs as `running` says.
    fn move_input(&self, facing: &Transform, running: bool) -> MoveInput {
        let axis =
            |positive: bool, negative: bool| f32::from(i8::from(positive) - i8::from(negative));
        // Level, whichever way the view tilts.
        let forward = facing.forward().with_y(0.0).normalize_or_zero();
        let right = facing.right().with_y(0.0).normalize_or_zero();
        let direction =
            forward * axis(self.forward, self.back) + right * axis(self.right, self.left);
        MoveInput {
            direction: direction.normalize_or_zero(),
            up: self.jump,
            down: self.descend,
            run: running,
            sneak: self.sneak,
        }
    }
}

/// Whether the player runs. A tap of Run starts running while the player goes forward
/// ([`Controls::going_forward`]) on foot: walking, jumping or falling, but not swimming or flying,
/// and not holding Sneak. Running then lasts, whether Run is held or not, until any of that stops
/// holding, the body hits a wall across the ground ([`movement::Moved::hit_wall`]), input stops
/// being gameplay input, or the World stops running. Running again takes a fresh tap of Run.
///
/// It lives on the player, so each World's player starts walking.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TapRun {
    running: bool,
}

impl TapRun {
    /// Reads one frame of `controls`, before the body moves, and returns whether it runs.
    /// `on_foot` says whether the body is walking, and not swimming.
    fn read(&mut self, controls: &Controls, on_foot: bool) -> bool {
        if !on_foot || !controls.going_forward() || controls.sneak {
            self.running = false;
        } else if controls.run_tapped {
            self.running = true;
        }
        self.running
    }

    /// Stops running: as the body hits a wall, or as the World stops running.
    pub fn stop(&mut self) {
        self.running = false;
    }

    pub fn running(&self) -> bool {
        self.running
    }
}

/// How to switch flying on and off, as the Settings page says it: "Press Space twice quickly to
/// switch flying on and off."
pub fn flight_hint(bindings: &Bindings) -> String {
    let action = Action::Jump;
    if bindings.inputs(action).is_empty() {
        format!(
            "“{}” is unbound, so flying can't be switched on. Bind it in Settings.",
            action.label()
        )
    } else {
        format!(
            "Press {} twice quickly to switch flying on and off.",
            bindings.label(action)
        )
    }
}

/// Makes the Cell the player's feet are in the focus, which the Active Region follows.
fn focus_on_player(mut focus: ResMut<SimFocus>, player: Single<&Player, Without<AwaitingSpawn>>) {
    let cell = player.feet.floor().as_ivec3();
    focus.set_if_neq(SimFocus(CellPos::new(cell.x, cell.y, cell.z)));
}

/// Where the feet of a body standing in the Cell at `pos` go: the middle of its floor.
pub fn feet_in(pos: CellPos) -> Vec3 {
    IVec3::new(pos.x, pos.y, pos.z).as_vec3() + Vec3::new(0.5, 0.0, 0.5)
}

/// The controller's side of running (`docs/plans/25-tap-run.md`): which frames of controls start
/// and stop it, as [`step`] reads them and moves the body.
#[cfg(test)]
mod tests {
    use bevy::ecs::system::RunSystemOnce;

    use super::movement::{RUN_SPEED, SNEAK_SPEED, WALK_SPEED};
    use super::*;

    const FRAME: f32 = 1.0 / 60.0;

    const NONE: Controls = Controls {
        forward: false,
        back: false,
        left: false,
        right: false,
        jump: false,
        descend: false,
        sneak: false,
        run_tapped: false,
    };
    const FORWARD: Controls = Controls {
        forward: true,
        ..NONE
    };
    /// Forward held, and Run tapped.
    const FORWARD_TAP: Controls = Controls {
        run_tapped: true,
        ..FORWARD
    };
    /// Forward and Right held: at an angle, facing +x, towards +x and +z.
    const FORWARD_RIGHT: Controls = Controls {
        right: true,
        ..FORWARD
    };

    /// Solid ground whose top is at y = 1, and a wall filling x ≥ 12. Gas elsewhere.
    fn walled(cell: IVec3) -> Matter {
        if cell.y <= 0 || cell.x >= 12 {
            Matter::Solid
        } else {
            Matter::Open
        }
    }

    /// Solid ground whose top is at y = 1, and two full layers of liquid on it where x ≥ 3.
    fn pool(cell: IVec3) -> Matter {
        match cell.y {
            ..=0 => Matter::Solid,
            1..=2 if cell.x >= 3 => Matter::Liquid { fill: 1.0 },
            _ => Matter::Open,
        }
    }

    /// The player, one frame at a time, as `move_player` moves them.
    struct Rig {
        body: Body,
        facing: Transform,
        run: TapRun,
    }

    impl Rig {
        /// Standing on the ground at x = 0.5, z = 0.5, facing +x, towards the wall of [`walled`].
        fn new() -> Self {
            Self {
                body: Body {
                    grounded: true,
                    ..Body::standing_at(Vec3::new(0.5, 1.0, 0.5))
                },
                facing: Transform::default().looking_to(Vec3::X, Vec3::Y),
                run: TapRun::default(),
            }
        }

        /// One frame of `controls` in `world`. Returns whether the body ran, as the view sees it.
        fn frame_in(&mut self, world: impl Fn(IVec3) -> Matter, controls: Controls) -> bool {
            let (body, gait) = step(
                self.body,
                &controls,
                &self.facing,
                &mut self.run,
                FRAME,
                world,
            );
            self.body = body;
            gait == Gait::Run
        }

        fn frame(&mut self, controls: Controls) -> bool {
            self.frame_in(walled, controls)
        }

        /// `frames` frames of `controls`. Returns whether the body ran, which every frame agrees
        /// on.
        fn hold(&mut self, frames: usize, controls: Controls) -> bool {
            let ran: Vec<bool> = (0..frames).map(|_| self.frame(controls)).collect();
            assert!(ran.iter().all(|&r| r == ran[0]), "{controls:?}: {ran:?}");
            ran[0]
        }

        /// How fast the body goes across the ground, in metres per second.
        #[track_caller]
        fn assert_speed(&self, expected: f32) {
            let speed = self.body.velocity.with_y(0.0).length();
            assert!(
                (speed - expected).abs() < 1e-4,
                "{speed} m/s, not {expected}"
            );
        }
    }

    #[test]
    fn a_tap_of_run_while_going_forward_runs_until_forward_is_let_go() {
        let mut rig = Rig::new();
        assert!(!rig.run.running(), "a player starts walking");

        // A tap of Run before Forward, and Forward after it, walk.
        assert!(!rig.frame(Controls {
            run_tapped: true,
            ..NONE
        }));
        assert!(!rig.hold(5, FORWARD));
        rig.assert_speed(WALK_SPEED);

        // Forward with a tap of Run runs. Running carries on once Run is let go, or held: either
        // way the frames after the tap have none.
        assert!(rig.frame(FORWARD_TAP));
        rig.assert_speed(RUN_SPEED);
        assert!(rig.hold(5, FORWARD));
        rig.assert_speed(RUN_SPEED);
        // Tapping Run again runs on.
        assert!(rig.frame(FORWARD_TAP));
        assert!(rig.hold(5, FORWARD));

        // Letting go of Forward stops running, and going forward again walks.
        assert!(!rig.frame(NONE));
        assert!(!rig.hold(5, FORWARD));
        rig.assert_speed(WALK_SPEED);

        // A fresh tap runs again.
        assert!(rig.frame(FORWARD_TAP));
        rig.assert_speed(RUN_SPEED);
    }

    #[test]
    fn running_carries_on_through_a_jump_and_its_landing() {
        let mut rig = Rig::new();
        assert!(rig.frame(FORWARD_TAP));
        assert!(rig.frame(Controls {
            jump: true,
            ..FORWARD
        }));
        assert!(!rig.body.grounded, "it jumped");
        // A jump lasts 2 × 8 / 25 = 0.64 s: under 60 frames.
        for frame in 0.. {
            assert!(frame < 60, "{:?}", rig.body);
            assert!(rig.frame(FORWARD), "frame {frame}: {:?}", rig.body);
            rig.assert_speed(RUN_SPEED);
            if rig.body.grounded {
                break;
            }
        }
        assert!(rig.hold(5, FORWARD));
    }

    #[test]
    fn running_goes_forward_with_or_without_sideways_but_not_sideways_alone() {
        let right = Controls {
            right: true,
            ..NONE
        };
        let mut rig = Rig::new();
        // Forward and Right with a tap of Run run at an angle, towards +x and +z.
        assert!(rig.frame(Controls {
            run_tapped: true,
            ..FORWARD_RIGHT
        }));
        rig.assert_speed(RUN_SPEED);
        assert!(
            rig.body.velocity.x > 1.0 && rig.body.velocity.z > 1.0,
            "{:?}",
            rig.body
        );
        // Letting go of Right, or holding Left instead, runs on.
        assert!(rig.hold(3, FORWARD));
        assert!(rig.hold(
            3,
            Controls {
                left: true,
                ..FORWARD
            }
        ));

        // Right alone stops running. With Right alone, a tap of Run walks, and so does adding
        // Forward later.
        assert!(!rig.frame(right));
        assert!(!rig.frame(Controls {
            run_tapped: true,
            ..right
        }));
        assert!(!rig.hold(3, FORWARD_RIGHT));
        rig.assert_speed(WALK_SPEED);

        // Forward and Back together go nowhere: a tap of Run doesn't run, and adding Back to a
        // run stops it.
        let both_ways = Controls {
            back: true,
            ..FORWARD
        };
        assert!(!rig.frame(Controls {
            run_tapped: true,
            ..both_ways
        }));
        assert!(rig.frame(FORWARD_TAP));
        assert!(!rig.frame(both_ways));
        assert!(!rig.run.running());

        // Nor does a tap of Run run backwards.
        assert!(!rig.frame(Controls {
            back: true,
            run_tapped: true,
            ..NONE
        }));
        rig.assert_speed(WALK_SPEED);
    }

    #[test]
    fn sneak_stops_running_and_keeps_a_tap_from_starting_it() {
        let sneaking = Controls {
            sneak: true,
            ..FORWARD
        };
        let mut rig = Rig::new();
        assert!(rig.frame(FORWARD_TAP));
        // Holding Sneak sneaks, and letting it go walks.
        assert!(!rig.hold(3, sneaking));
        rig.assert_speed(SNEAK_SPEED);
        assert!(!rig.hold(3, FORWARD));
        rig.assert_speed(WALK_SPEED);

        // Run and Sneak pressed together don't run, while Sneak is held or after it is let go.
        assert!(!rig.frame(Controls {
            run_tapped: true,
            ..sneaking
        }));
        assert!(!rig.hold(3, sneaking));
        assert!(!rig.hold(3, FORWARD));
        rig.assert_speed(WALK_SPEED);
    }

    #[test]
    fn hitting_a_wall_stops_running_though_forward_and_run_are_still_held() {
        let mut rig = Rig::new();
        assert!(rig.frame(Controls {
            run_tapped: true,
            ..FORWARD_RIGHT
        }));
        // At an angle into the wall, whose face is at x = 12: 11.2 m along x, at 7 × √½ ≈ 4.95
        // m/s, takes about 2.3 s. It runs on every frame until the one on which it reaches the
        // wall, its feet 0.3 m from the face, and from that very frame the view narrows.
        for frame in 1.. {
            assert!(frame < 300, "{:?}", rig.body);
            let ran = rig.frame(FORWARD_RIGHT);
            let at_the_wall = (rig.body.feet.x - 11.7).abs() < 1e-3;
            assert_eq!(ran, !at_the_wall, "frame {frame}: {:?}", rig.body);
            if at_the_wall {
                break;
            }
        }
        assert!(!rig.run.running());

        // Sliding on along the wall with Forward and Right, and Run still held, walks: along z at
        // walking speed × √½.
        assert!(!rig.hold(5, FORWARD_RIGHT));
        let along = WALK_SPEED * std::f32::consts::FRAC_1_SQRT_2;
        assert!((rig.body.velocity.z - along).abs() < 1e-4, "{:?}", rig.body);

        // Turned along the wall, clear of it, Forward walks until a fresh tap of Run.
        rig.facing = Transform::default().looking_to(Vec3::Z, Vec3::Y);
        assert!(!rig.hold(5, FORWARD));
        rig.assert_speed(WALK_SPEED);
        assert!(rig.frame(FORWARD_TAP));
        assert!(rig.hold(5, FORWARD));
        rig.assert_speed(RUN_SPEED);
    }

    #[test]
    fn swimming_flying_or_losing_gameplay_input_stops_running() {
        // Without gameplay input, the controls read as nothing held.
        let mut rig = Rig::new();
        assert!(rig.frame(FORWARD_TAP));
        assert!(!rig.frame(Controls::default()));
        assert!(!rig.hold(3, FORWARD));

        // Switching to flight stops it, and walking again walks. A tap in flight starts nothing.
        assert!(rig.frame(FORWARD_TAP));
        rig.body.mode = MoveMode::Fly;
        assert!(!rig.frame(FORWARD));
        assert!(!rig.run.running());
        assert!(!rig.frame(FORWARD_TAP));
        assert!(!rig.run.running());
        rig.body.mode = MoveMode::Walk;
        assert!(!rig.frame(FORWARD));

        // Running into a pool stops once the body swims, and a tap in it starts nothing.
        let mut rig = Rig::new();
        assert!(rig.frame_in(pool, FORWARD_TAP));
        let mut frames = 1;
        while rig.frame_in(pool, FORWARD) {
            frames += 1;
            assert!(frames < 120, "{:?}", rig.body);
        }
        assert!(swimming(&rig.body, pool), "{:?}", rig.body);
        assert!(!rig.run.running());
        assert!(!rig.frame_in(pool, FORWARD_TAP));
        assert!(!rig.run.running());
    }

    #[test]
    fn pausing_stops_running_until_a_fresh_tap() {
        let mut rig = Rig::new();
        assert!(rig.frame(FORWARD_TAP));

        // The World pauses: `stop_running` runs as it stops running.
        let mut world = World::new();
        let player = world.spawn(rig.run).id();
        world.run_system_once(stop_running).expect("stop_running");
        rig.run = *world.get::<TapRun>(player).expect("the player's TapRun");
        assert!(!rig.run.running());

        // Resumed with Forward and Run still held, it walks until a fresh tap of Run.
        assert!(!rig.hold(3, FORWARD));
        assert!(rig.frame(FORWARD_TAP));
    }
}
