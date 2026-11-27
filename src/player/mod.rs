//! The player: a body that walks, jumps, flies and swims through the world in first person, and
//! that the Active Region follows. It exists while a World is open.
//!
//! Controls, while input is gameplay input (`crate::input_policy`): the mouse looks around. WASD
//! moves, Space jumps, and F switches flying on and off. Flying goes through solids; Space rises
//! and Shift sinks. In a liquid, holding Space swims upward. While Alt is held, the body takes no
//! input but still falls, swims and slides; while the World is paused, it doesn't move at all.
//!
//! A new World's player spawns on the ground below the focus. A saved World's player carries on
//! exactly as saved: where they were, looking the same way, moving as they were. Either is held
//! in place until the Chunks around them have loaded.

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
use crate::sim_bridge::{Elements, RENDER_DISTANCE, SimFocus};
use crate::world::WorldView;
use movement::{Body, MoveInput, MoveMode, advance};

/// How far the view turns per pixel the mouse moves, in radians: about a tenth of a degree.
const MOUSE_SENSITIVITY: f32 = 0.002;

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
        app.add_systems(
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

/// Spawns the player with the World, held in place until the Chunks around them load: as saved,
/// or at the focus, facing along +x and +z, away from the sun. Their camera holds egui's primary
/// context, which a new camera doesn't otherwise get once the first World's camera is gone.
fn spawn_player(mut commands: Commands, start: Res<PlayerStart>, focus: Res<SimFocus>) {
    let (body, rotation, awaiting) = match start.0 {
        Some(saved) => (saved.body, saved.rotation(), AwaitingSpawn::AsSaved),
        None => {
            let facing = Transform::default().looking_to(Vec3::new(1.0, 0.0, 1.0), Vec3::Y);
            let body = Body::standing_at(feet_in(focus.0));
            (body, facing.rotation, AwaitingSpawn::OnGround)
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

/// Once every Chunk of the focus's column within render distance is loaded (the origin column,
/// where a new World's focus starts), stands a new World's player on that column's highest solid
/// Cell.
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
    let CellPos { x, z, .. } = focus.0;
    let centre = ChunkPos::of(focus.0);
    let size = CHUNK_SIZE as i32;
    let layers = world_chunk_layers();
    let lowest = (centre.y - RENDER_DISTANCE.vertical).max(*layers.start());
    let highest = (centre.y + RENDER_DISTANCE.vertical).min(*layers.end());
    let loaded = |y: i32| view.chunk(ChunkPos::new(centre.x, y, centre.z)).is_some();
    if !(lowest..=highest).all(loaded) {
        return;
    }
    let top = (highest + 1) * size;
    let bottom = lowest * size;
    let is_solid = |y: i32| {
        view.cell(CellPos::new(x, y, z))
            .is_some_and(|cell| elements.get(cell.element).phase == Phase::Solid)
    };
    // With no solid Cell in the column, it stays at the focus.
    let feet = (bottom..top)
        .rev()
        .find(|&y| is_solid(y))
        .map_or(focus.0, |y| CellPos::new(x, y + 1, z));

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

/// Turns the view with the mouse. A player held while Chunks load keeps looking the way they
/// spawned.
fn look_around(
    motion: Res<AccumulatedMouseMotion>,
    mut player: Single<&mut Transform, (With<Player>, Without<AwaitingSpawn>)>,
) {
    if motion.delta == Vec2::ZERO {
        return;
    }
    // The mouse reports the whole movement since the last frame, so it isn't scaled by time.
    let (yaw, pitch, _) = player.rotation.to_euler(EulerRot::YXZ);
    let yaw = yaw - motion.delta.x * MOUSE_SENSITIVITY;
    let pitch = (pitch - motion.delta.y * MOUSE_SENSITIVITY).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    player.rotation = Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0);
}

/// Moves the body as the keys say, through the world as the game last heard it from the
/// simulation, and keeps the camera at its eyes. Without gameplay input, the body moves as if no
/// key were held.
fn move_player(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    policy: Res<InputPolicy>,
    view: Res<WorldView>,
    elements: Res<Elements>,
    player: Single<(&mut Player, &mut Transform), Without<AwaitingSpawn>>,
) {
    let (mut player, mut transform) = player.into_inner();
    let input = if policy.gameplay {
        if keys.just_pressed(KeyCode::KeyF) {
            player.mode = player.mode.toggled();
            player.velocity = Vec3::ZERO;
            info!("flying: {}", player.mode == MoveMode::Fly);
        }
        move_input(&keys, &transform)
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

/// What the keys held ask of the body, which faces as `transform` does.
fn move_input(keys: &ButtonInput<KeyCode>, transform: &Transform) -> MoveInput {
    let axis = |positive: KeyCode, negative: KeyCode| {
        f32::from(i8::from(keys.pressed(positive)) - i8::from(keys.pressed(negative)))
    };
    // Level, whichever way the view tilts.
    let forward = transform.forward().with_y(0.0).normalize_or_zero();
    let right = transform.right().with_y(0.0).normalize_or_zero();
    let direction =
        forward * axis(KeyCode::KeyW, KeyCode::KeyS) + right * axis(KeyCode::KeyD, KeyCode::KeyA);
    MoveInput {
        direction: direction.normalize_or_zero(),
        up: keys.pressed(KeyCode::Space),
        down: keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]),
    }
}

/// Makes the Cell the player's feet are in the focus, which the Active Region follows.
fn focus_on_player(mut focus: ResMut<SimFocus>, player: Single<&Player, Without<AwaitingSpawn>>) {
    let cell = player.feet.floor().as_ivec3();
    focus.set_if_neq(SimFocus(CellPos::new(cell.x, cell.y, cell.z)));
}

/// Where the feet of a body standing in the Cell at `pos` go: the middle of its floor.
fn feet_in(pos: CellPos) -> Vec3 {
    IVec3::new(pos.x, pos.y, pos.z).as_vec3() + Vec3::new(0.5, 0.0, 0.5)
}
