//! The sky and the sun, and a temporary camera to look at the world with.

use bevy::camera_controller::free_camera::{FreeCamera, FreeCameraPlugin};
use bevy::light::CascadeShadowConfigBuilder;
use bevy::prelude::*;
use sim::CHUNK_SIZE;

use crate::sim_bridge::{RENDER_DISTANCE, SimFocus};

/// A clear daytime blue.
const SKY: Color = Color::srgb(0.55, 0.75, 0.95);

// Under the camera's default exposure (EV100 9.7), a face lit by `E` lux at angle θ shows at
// about its colour × E × cos θ / π × 0.001, and ambient light of brightness `B` adds its colour ×
// B × 0.001. These values put a sunlit top face at about its Element's colour, sides the sun
// reaches at about 0.7 and 0.55 of it, and shade at about 0.35, so every Element keeps its colour
// and each side of a cube has its own shade. Brighter light washes the colours out.

/// The sun's illuminance, in lux: about 0.65 of a top face's colour.
const SUN_ILLUMINANCE: f32 = 2_500.0;

/// Which way sunlight travels: high, and from the side the camera starts on.
const SUN_DIRECTION: Vec3 = Vec3::new(0.5, -1.0, 0.3);

/// Light from the sky, which also reaches faces the sun doesn't: slightly blue.
const SKY_LIGHT: Color = Color::srgb(0.8, 0.87, 1.0);

/// The sky light's brightness: about 0.35 of a face's colour.
const SKY_LIGHT_BRIGHTNESS: f32 = 450.0;

/// The sky colour, the sun, and a free-flying camera that starts above the focus.
pub struct ScenePlugin;

impl Plugin for ScenePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ClearColor(SKY))
            .insert_resource(GlobalAmbientLight {
                color: SKY_LIGHT,
                brightness: SKY_LIGHT_BRIGHTNESS,
                ..default()
            })
            .add_plugins(FreeCameraPlugin)
            .add_systems(Startup, (spawn_sun, spawn_camera));
    }
}

fn spawn_sun(mut commands: Commands) {
    let render_distance = (RENDER_DISTANCE.horizontal as usize * CHUNK_SIZE) as f32;
    commands.spawn((
        DirectionalLight {
            illuminance: SUN_ILLUMINANCE,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::default().looking_to(SUN_DIRECTION, Vec3::Y),
        CascadeShadowConfigBuilder {
            first_cascade_far_bound: 24.0,
            maximum_distance: render_distance,
            ..default()
        }
        .build(),
    ));
}

/// Temporary, until the first-person controller (step 11) replaces it: a camera to fly around
/// with. Hold the right mouse button (or press M) to look around; WASD moves, Q and E go down and
/// up, Shift runs, and the scroll wheel changes speed.
fn spawn_camera(mut commands: Commands, focus: Res<SimFocus>) {
    let focus = IVec3::new(focus.0.x, focus.0.y, focus.0.z).as_vec3();
    commands.spawn((
        Camera3d::default(),
        Transform::from_translation(focus + Vec3::new(-40.0, 30.0, -40.0))
            .looking_at(focus, Vec3::Y),
        FreeCamera {
            walk_speed: 10.0,
            run_speed: 40.0,
            ..default()
        },
    ));
}
