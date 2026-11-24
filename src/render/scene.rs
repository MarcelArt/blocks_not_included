//! The sky, its light, the sun, and a lamp by the camera under a roof. The camera is the
//! player's (`crate::player`).

use bevy::asset::RenderAssetUsages;
use bevy::light::CascadeShadowConfigBuilder;
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
};
use sim::{CHUNK_SIZE, CellPos, Phase};

use crate::sim_bridge::{Elements, RENDER_DISTANCE};
use crate::world::WorldView;

/// A clear daytime blue.
const SKY: Color = Color::srgb(0.55, 0.75, 0.95);

// Under the camera's default exposure (EV100 9.7), a face lit by `E` lux at angle θ shows at
// about its colour × E × cos θ / π × 0.001, and sky light of intensity `B` from the way it faces
// adds its colour × B × 0.001. These values put a sunlit top face at about its Element's colour,
// sides the sun reaches at about 0.7 and 0.5 of it, and shade at 0.2 to 0.55 by which way a face
// points, so every Element keeps its colour and each side of a cube has its own shade. Brighter
// light washes the colours out.

/// The sun's illuminance, in lux: about 0.65 of a top face's colour.
const SUN_ILLUMINANCE: f32 = 2_500.0;

/// Which way sunlight travels: high, and from behind the player as they start out.
const SUN_DIRECTION: Vec3 = Vec3::new(0.5, -1.0, 0.3);

/// Light from the sky, which also reaches faces the sun doesn't: slightly blue.
const SKY_LIGHT: Color = Color::srgb(0.8, 0.87, 1.0);

/// How bright the sky light is on a face that points up: about 0.55 of its colour.
const SKY_LIGHT_INTENSITY: f32 = 550.0;

/// The share of [`SKY_LIGHT_INTENSITY`] a face gets by which way it points. Flat ambient light
/// leaves faces of one colour alike wherever the sun doesn't reach, as in a cave, so that a floor
/// can't be told from the wall or the drop beyond it. The sky is brightest overhead and darkest
/// underfoot, and the x and z sides differ so that two walls meeting at a corner do too.
const SKY_LIGHT_UP: f32 = 1.0;
const SKY_LIGHT_X: f32 = 0.72;
const SKY_LIGHT_Z: f32 = 0.56;
const SKY_LIGHT_DOWN: f32 = 0.4;

/// The lamp's luminous power, in lumens. From a metre behind the eyes, it adds about 0.2 of a
/// face's colour 4 m ahead (200,000 / (4π × 5²) ≈ 640 lux), and a seventh of that 12 m ahead.
const LAMP_INTENSITY: f32 = 0.0;

/// How far the lamp reaches, in metres.
const LAMP_RANGE: f32 = 40.0;

/// Where the lamp hangs from the camera: above and behind the eyes, so the faces right in front
/// of the player aren't washed out.
const LAMP_OFFSET: Vec3 = Vec3::new(0.0, 0.4, 1.0);

/// Warm, against the sky light's blue.
const LAMP_COLOR: Color = Color::srgb(1.0, 0.95, 0.86);

/// How far above the lamp a solid Cell counts as a roof, in Cells.
const ROOF_CELLS: i32 = 24;

/// How quickly the lamp comes on and goes off: the share of the way it goes in a second is
/// 1 − e^(−rate).
const LAMP_FADE_RATE: f32 = 4.0;

/// The sky colour and its light, the sun, and the lamp that lights the player's way under a
/// roof. Needs the sim bridge.
pub struct ScenePlugin;

impl Plugin for ScenePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ClearColor(SKY))
            // The sky light takes its place.
            .insert_resource(GlobalAmbientLight::NONE)
            .add_systems(Startup, (spawn_sun, make_sky_light))
            .add_systems(Update, (light_cameras_with_the_sky, light_the_lamp));
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

/// The light the sky casts on everything a camera sees, from every direction.
#[derive(Resource)]
struct SkyLight(EnvironmentMapLight);

fn make_sky_light(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let cubemap = images.add(sky_light_cubemap());
    commands.insert_resource(SkyLight(EnvironmentMapLight {
        diffuse_map: cubemap.clone(),
        specular_map: cubemap,
        intensity: SKY_LIGHT_INTENSITY,
        ..default()
    }));
}

/// The sky light seen from each way a face can point: one texel on each side of a cube, in
/// [`SKY_LIGHT`] at the share of its brightness that side gets.
fn sky_light_cubemap() -> Image {
    // wgpu's order for the sides of a cube.
    let sides = [
        SKY_LIGHT_X,    // +x
        SKY_LIGHT_X,    // −x
        SKY_LIGHT_UP,   // +y
        SKY_LIGHT_DOWN, // −y
        SKY_LIGHT_Z,    // +z
        SKY_LIGHT_Z,    // −z
    ];
    let sky = SKY_LIGHT.to_linear();
    let texels = sides
        .into_iter()
        .flat_map(|share| {
            [sky.red * share, sky.green * share, sky.blue * share, 1.0]
                .map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8)
        })
        .collect();
    Image {
        texture_view_descriptor: Some(TextureViewDescriptor {
            dimension: Some(TextureViewDimension::Cube),
            ..default()
        }),
        ..Image::new(
            Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 6,
            },
            TextureDimension::D2,
            texels,
            TextureFormat::Rgba8Unorm,
            RenderAssetUsages::RENDER_WORLD,
        )
    }
}

/// Gives every 3D camera the sky light, and a lamp, once it is spawned.
fn light_cameras_with_the_sky(
    mut commands: Commands,
    sky: Res<SkyLight>,
    cameras: Query<Entity, (With<Camera3d>, Without<EnvironmentMapLight>)>,
) {
    for camera in &cameras {
        commands.entity(camera).insert(sky.0.clone()).with_child((
            Lamp,
            PointLight {
                color: LAMP_COLOR,
                // Off until it finds a roof.
                intensity: 0.0,
                range: LAMP_RANGE,
                // It hangs by the eyes, so the shadows it would cast hide behind what casts them.
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::from_translation(LAMP_OFFSET),
        ));
    }
}

/// A light by a camera's eyes, which lights the way under a roof. The sky light alone shows a
/// cave's near and far faces alike; the lamp shows the near ones brighter, so a drop stands out
/// from the floor before it.
#[derive(Component)]
struct Lamp;

/// Fades each lamp on under a roof and off in the open.
fn light_the_lamp(
    time: Res<Time>,
    view: Res<WorldView>,
    elements: Res<Elements>,
    mut lamps: Query<(&mut PointLight, &GlobalTransform), With<Lamp>>,
) {
    let fade = 1.0 - (-LAMP_FADE_RATE * time.delta_secs()).exp();
    for (mut light, transform) in &mut lamps {
        let cell = transform.translation().floor().as_ivec3();
        let roofed = (1..=ROOF_CELLS).any(|dy| {
            view.cell(CellPos::new(cell.x, cell.y + dy, cell.z))
                .is_some_and(|cell| elements.0.get(cell.element).phase == Phase::Solid)
        });
        let target = if roofed { LAMP_INTENSITY } else { 0.0 };
        light.intensity += (target - light.intensity) * fade;
    }
}
