//! The player: a body that walks, jumps, flies and swims through the world in first person, and
//! that the Active Region follows. It exists while a World is open.
//!
//! Controls, while input is gameplay input (`crate::input_policy`): the mouse looks around, as
//! fast as the player's mouse sensitivity says. The movement actions move, Jump jumps, and F
//! switches flying on and off. Flying goes through solids; Jump rises and Fly down sinks. In a
//! liquid, holding Jump swims upward. The actions are bound in Settings (`crate::settings`), by
//! default to WASD, Space and Shift. While Alt is held, the body takes no input but still falls,
//! swims and slides; while the World is paused, it doesn't move at all.
//!
//! A World opened without a player spawns them on the ground below the focus ([`ground_below`]).
//! A saved World's player carries on exactly as saved: where they were, looking the same way,
//! moving as they were. A new World from the main menu is saved before it opens, with its player
//! already standing on that ground, so it opens as saved. Either way, the player is held in place
//! until the Chunks around them have loaded.

pub mod collision;
pub mod movement;
pub mod raycast;

use std::f32::consts::FRAC_PI_2;

use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use bevy_egui::PrimaryEguiContext;
use sim::{CHUNK_SIZE, CellPos, ChunkPos, Phase, WORLD_BOTTOM_Y, WORLD_TOP_Y};

use crate::input_policy::{InputPolicy, gameplay_input};
use crate::session::{Play, PlayerStart, Session, SessionSystems};
use crate::settings::{Action, ActionInput, Settings};
use crate::sim_bridge::{Elements, RENDER_DISTANCE, SimFocus};
use crate::world::WorldView;
use movement::{Body, MoveInput, MoveMode, advance};

/// How far the view turns per pixel the mouse moves at a mouse sensitivity of 1, in radians: about
/// a tenth of a degree.
pub const LOOK_SPEED: f32 = 0.002;

/// Switches flying on and off. Fixed until step 21 moves flying to a double press of Jump: Settings
/// can't rebind it, or bind anything else to it (`crate::settings::FIXED_SHORTCUTS`).
pub const FLY_KEY: KeyCode = KeyCode::KeyF;

/// How far the view may tilt up or down: just short of straight up and down, where yaw would
/// become ambiguous.
const PITCH_LIMIT: f32 = FRAC_PI_2 - 0.01;

/// The longest step of time a single frame moves the body through, in seconds. After a stall,
/// the body moves less than the time that passed, rather than one long, sudden step.
const MAX_FRAME_STEP: f32 = 0.1;

/// The vertical field of view, in degrees.
const FIELD_OF_VIEW: f32 = 70.0;

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
        );
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

/// Moves the body as the controls say, through the world as the game last heard it from the
/// simulation, and keeps the camera at its eyes. Without gameplay input, the body moves as if no
/// control were held.
fn move_player(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    actions: ActionInput,
    policy: Res<InputPolicy>,
    view: Res<WorldView>,
    elements: Res<Elements>,
    player: Single<(&mut Player, &mut Transform), Without<AwaitingSpawn>>,
) {
    let (mut player, mut transform) = player.into_inner();
    let input = if policy.gameplay {
        if keys.just_pressed(FLY_KEY) {
            player.mode = player.mode.toggled();
            player.velocity = Vec3::ZERO;
            info!("flying: {}", player.mode == MoveMode::Fly);
        }
        move_input(&actions, &transform)
    } else {
        MoveInput::default()
    };

    let phase_at = |cell: IVec3| match view.cell(CellPos::new(cell.x, cell.y, cell.z)) {
        Some(cell) => elements.get(cell.element).phase,
        // A Chunk that isn't loaded is solid, so the body can't fall into unloaded space.
        None => Phase::Solid,
    };
    let dt = time.delta_secs().min(MAX_FRAME_STEP);
    **player = advance(**player, input, dt, phase_at);
    transform.translation = player.eyes();
}

/// What the controls held ask of the body, which faces as `transform` does.
fn move_input(actions: &ActionInput, transform: &Transform) -> MoveInput {
    let axis = |positive: Action, negative: Action| {
        f32::from(i8::from(actions.pressed(positive)) - i8::from(actions.pressed(negative)))
    };
    // Level, whichever way the view tilts.
    let forward = transform.forward().with_y(0.0).normalize_or_zero();
    let right = transform.right().with_y(0.0).normalize_or_zero();
    let direction = forward * axis(Action::MoveForward, Action::MoveBack)
        + right * axis(Action::MoveRight, Action::MoveLeft);
    MoveInput {
        direction: direction.normalize_or_zero(),
        up: actions.pressed(Action::Jump),
        down: actions.pressed(Action::Descend),
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
