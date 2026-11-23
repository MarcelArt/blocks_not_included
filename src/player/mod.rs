//! The player: a body that walks, jumps, flies and swims through the world in first person, and
//! that the Active Region follows.
//!
//! Controls: click to capture the cursor, and Esc to release it. While it is captured, the mouse
//! looks around. WASD moves, Space jumps, and F switches flying on and off. Flying goes through
//! solids; Space rises and Shift sinks. In a liquid, holding Space swims upward.

pub mod collision;
pub mod movement;

use std::f32::consts::FRAC_PI_2;

use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use sim::{CHUNK_SIZE, CellPos, ChunkPos, Phase, WORLD_BOTTOM_Y, WORLD_TOP_Y};

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
/// Active Region's focus. Needs the sim bridge.
pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_player).add_systems(
            Update,
            (
                place_on_ground,
                capture_cursor,
                look_around,
                move_player,
                focus_on_player,
            )
                .chain(),
        );
    }
}

/// The player's body. Its entity is the camera, at the body's eyes.
#[derive(Component, Clone, Copy, Debug, Deref, DerefMut)]
pub struct Player(pub Body);

/// Holds the player in place until the Chunks of the column it spawns in are loaded.
#[derive(Component, Clone, Copy, Debug)]
pub struct AwaitingSpawn;

/// Spawns the player at the focus, held in place until the ground under it has loaded. It faces
/// along +x and +z, away from the sun.
fn spawn_player(mut commands: Commands, focus: Res<SimFocus>) {
    let body = Body::standing_at(feet_in(focus.0));
    commands.spawn((
        Player(body),
        AwaitingSpawn,
        Camera3d::default(),
        Projection::from(PerspectiveProjection {
            fov: FIELD_OF_VIEW.to_radians(),
            ..default()
        }),
        Transform::from_translation(body.eyes()).looking_to(Vec3::new(1.0, 0.0, 1.0), Vec3::Y),
    ));
}

/// Once every Chunk of the focus's column within render distance is loaded (the origin column,
/// where the focus starts), stands the player on that column's highest solid Cell.
fn place_on_ground(
    mut commands: Commands,
    view: Res<WorldView>,
    elements: Res<Elements>,
    focus: Res<SimFocus>,
    player: Single<(Entity, &mut Player, &mut Transform), With<AwaitingSpawn>>,
) {
    let CellPos { x, z, .. } = focus.0;
    let centre = ChunkPos::of(focus.0);
    let size = CHUNK_SIZE as i32;
    let lowest = (centre.y - RENDER_DISTANCE.vertical).max(WORLD_BOTTOM_Y / size);
    let highest = (centre.y + RENDER_DISTANCE.vertical).min(WORLD_TOP_Y / size - 1);
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

    let (entity, mut player, mut transform) = player.into_inner();
    **player = Body::standing_at(feet_in(feet));
    transform.translation = player.eyes();
    commands.entity(entity).remove::<AwaitingSpawn>();
    info!("the player spawned at ({}, {}, {})", feet.x, feet.y, feet.z);
}

/// Captures the cursor on a click, and releases it on Esc.
fn capture_cursor(
    mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
) {
    if mouse.just_pressed(MouseButton::Left) {
        cursor.visible = false;
        cursor.grab_mode = CursorGrabMode::Locked;
    }
    if keys.just_pressed(KeyCode::Escape) {
        cursor.visible = true;
        cursor.grab_mode = CursorGrabMode::None;
    }
}

/// While the cursor is captured, turns the view with the mouse.
fn look_around(
    cursor: Single<&CursorOptions, With<PrimaryWindow>>,
    motion: Res<AccumulatedMouseMotion>,
    mut player: Single<&mut Transform, With<Player>>,
) {
    if cursor.grab_mode == CursorGrabMode::None || motion.delta == Vec2::ZERO {
        return;
    }
    // The mouse reports the whole movement since the last frame, so it isn't scaled by time.
    let (yaw, pitch, _) = player.rotation.to_euler(EulerRot::YXZ);
    let yaw = yaw - motion.delta.x * MOUSE_SENSITIVITY;
    let pitch = (pitch - motion.delta.y * MOUSE_SENSITIVITY).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    player.rotation = Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0);
}

/// Moves the body as the keys say, through the world as the game last heard it from the
/// simulation, and keeps the camera at its eyes.
fn move_player(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    view: Res<WorldView>,
    elements: Res<Elements>,
    player: Single<(&mut Player, &mut Transform), Without<AwaitingSpawn>>,
) {
    let (mut player, mut transform) = player.into_inner();
    if keys.just_pressed(KeyCode::KeyF) {
        player.mode = player.mode.toggled();
        player.velocity = Vec3::ZERO;
        info!("flying: {}", player.mode == MoveMode::Fly);
    }

    let axis = |positive: KeyCode, negative: KeyCode| {
        f32::from(i8::from(keys.pressed(positive)) - i8::from(keys.pressed(negative)))
    };
    // Level, whichever way the view tilts.
    let forward = transform.forward().with_y(0.0).normalize_or_zero();
    let right = transform.right().with_y(0.0).normalize_or_zero();
    let direction =
        forward * axis(KeyCode::KeyW, KeyCode::KeyS) + right * axis(KeyCode::KeyD, KeyCode::KeyA);
    let input = MoveInput {
        direction: direction.normalize_or_zero(),
        up: keys.pressed(KeyCode::Space),
        down: keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]),
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

/// Makes the Cell the player's feet are in the focus, which the Active Region follows.
fn focus_on_player(mut focus: ResMut<SimFocus>, player: Single<&Player, Without<AwaitingSpawn>>) {
    let cell = player.feet.floor().as_ivec3();
    focus.set_if_neq(SimFocus(CellPos::new(cell.x, cell.y, cell.z)));
}

/// Where the feet of a body standing in the Cell at `pos` go: the middle of its floor.
fn feet_in(pos: CellPos) -> Vec3 {
    IVec3::new(pos.x, pos.y, pos.z).as_vec3() + Vec3::new(0.5, 0.0, 0.5)
}
