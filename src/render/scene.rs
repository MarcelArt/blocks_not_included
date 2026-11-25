//! The sky, its light, which dims where little of the sky shows, the sun, and a lamp by the
//! camera. The camera is the player's (`crate::player`).

use bevy::asset::RenderAssetUsages;
use bevy::light::CascadeShadowConfigBuilder;
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
};
use sim::{CHUNK_SIZE, CellPos, Phase};

use crate::player::raycast::raycast;
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

/// The sky light's intensity where no sky shows at all, as when sealed in underground: dark, but
/// every face still shows in the shade of the way it points, so the shapes of the Cells do too.
const ENCLOSED_SKY_LIGHT_INTENSITY: f32 = 45.0;

/// The share of the sky the player must see for full daylight. Beside a cliff or under an
/// overhang, at least this much shows. Less dims the sky light towards
/// [`ENCLOSED_SKY_LIGHT_INTENSITY`].
const OPEN_SKY: f32 = 0.4;

/// How far a ray looks for the sky, in metres. One that meets nothing solid within it counts as
/// reaching the sky.
const SKY_REACH: f32 = 48.0;

/// How often the sky the player sees is looked for again, in seconds.
const SKY_CHECK_INTERVAL: f32 = 0.1;

/// How quickly the light follows the sky the player sees, like eyes adjusting: the share of the
/// way it goes in a second is 1 − e^(−rate).
const DAYLIGHT_RATE: f32 = 2.0;

/// The lamp's luminous power, in lumens, where no sky shows; it fades out as daylight comes in.
/// 0 turns it off. At 200,000, from a metre behind the eyes, it adds about 0.2 of a face's colour
/// 4 m ahead (200,000 / (4π × 5²) ≈ 640 lux), and a seventh of that 12 m ahead.
const LAMP_INTENSITY: f32 = 0.0;

/// How far the lamp reaches, in metres.
const LAMP_RANGE: f32 = 40.0;

/// Where the lamp hangs from the camera: above and behind the eyes, so the faces right in front
/// of the player aren't washed out.
const LAMP_OFFSET: Vec3 = Vec3::new(0.0, 0.4, 1.0);

/// Warm, against the sky light's blue.
const LAMP_COLOR: Color = Color::srgb(1.0, 0.95, 0.86);

/// The sky colour and its light, which dims where little of the sky shows, the sun, and the lamp
/// that lights the player's way where the sky doesn't. Needs the sim bridge.
pub struct ScenePlugin;

impl Plugin for ScenePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ClearColor(SKY))
            // The sky light takes its place.
            .insert_resource(GlobalAmbientLight::NONE)
            .add_systems(Startup, (spawn_sun, make_sky_light))
            .add_systems(Update, (light_cameras_with_the_sky, follow_daylight));
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
        commands
            .entity(camera)
            .insert((sky.0.clone(), Daylight::FULL))
            .with_child((
                Lamp,
                PointLight {
                    color: LAMP_COLOR,
                    // Off in full daylight.
                    intensity: 0.0,
                    range: LAMP_RANGE,
                    // It hangs by the eyes, so the shadows it would cast hide behind what casts
                    // them.
                    shadow_maps_enabled: false,
                    ..default()
                },
                Transform::from_translation(LAMP_OFFSET),
            ));
    }
}

/// A light by a camera's eyes, which lights the way where the sky doesn't. The sky light alone
/// shows a cave's near and far faces alike; the lamp shows the near ones brighter, so a drop
/// stands out from the floor before it.
#[derive(Component)]
struct Lamp;

/// How much daylight reaches a camera, from 0 where no sky shows to 1 under open sky.
#[derive(Component, Clone, Copy, Debug)]
struct Daylight {
    /// Eased towards `target`, as eyes adjust.
    level: f32,
    /// What the sky the camera last saw lets in.
    target: f32,
    /// When to look for the sky again, in seconds since startup.
    next_check: f32,
}

impl Daylight {
    const FULL: Self = Self {
        level: 1.0,
        target: 1.0,
        next_check: 0.0,
    };
}

/// Dims each camera's sky light, and brings up its lamp, as less of the sky shows from it.
fn follow_daylight(
    time: Res<Time>,
    view: Res<WorldView>,
    elements: Res<Elements>,
    mut cameras: Query<(
        &GlobalTransform,
        &mut Daylight,
        &mut EnvironmentMapLight,
        &Children,
    )>,
    mut lamps: Query<&mut PointLight, With<Lamp>>,
) {
    let now = time.elapsed_secs();
    let ease = 1.0 - (-DAYLIGHT_RATE * time.delta_secs()).exp();
    let is_solid = |cell: IVec3| {
        view.cell(CellPos::new(cell.x, cell.y, cell.z))
            .is_some_and(|cell| elements.0.get(cell.element).phase == Phase::Solid)
    };
    for (transform, mut daylight, mut sky, children) in &mut cameras {
        if now >= daylight.next_check {
            daylight.target = daylight_from(sky_seen(transform.translation(), is_solid));
            daylight.next_check = now + SKY_CHECK_INTERVAL;
        }
        daylight.level += (daylight.target - daylight.level) * ease;
        let level = daylight.level;
        sky.intensity = ENCLOSED_SKY_LIGHT_INTENSITY
            + (SKY_LIGHT_INTENSITY - ENCLOSED_SKY_LIGHT_INTENSITY) * level;
        for &child in children {
            if let Ok(mut lamp) = lamps.get_mut(child) {
                lamp.intensity = LAMP_INTENSITY * (1.0 - level);
            }
        }
    }
}

/// The share of the sky seen from `eye`: of the rays cast up from it along [`sky_directions`],
/// those that meet no Cell `is_solid` holds for within [`SKY_REACH`].
fn sky_seen(eye: Vec3, is_solid: impl Fn(IVec3) -> bool) -> f32 {
    let directions = sky_directions();
    let open = directions
        .iter()
        .filter(|&&direction| raycast(eye, direction, SKY_REACH, &is_solid).is_none())
        .count();
    open as f32 / directions.len() as f32
}

/// The ways [`sky_seen`] looks for the sky: straight up, and six each at 60° and 35° above the
/// horizon, at 0°, 60°, … 300° around from +x.
fn sky_directions() -> [Vec3; 13] {
    std::array::from_fn(|i| {
        if i == 0 {
            return Vec3::Y;
        }
        let elevation = [60.0_f32, 35.0][(i - 1) / 6].to_radians();
        let azimuth = ((i - 1) % 6) as f32 * 60.0_f32.to_radians();
        Vec3::new(
            elevation.cos() * azimuth.cos(),
            elevation.sin(),
            elevation.cos() * azimuth.sin(),
        )
    })
}

/// How much daylight seeing `seen` of the sky lets in: all of it from [`OPEN_SKY`] up, easing
/// down to none with no sky at all.
fn daylight_from(seen: f32) -> f32 {
    let t = (seen / OPEN_SKY).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The eyes in the middle of the Cell at the origin.
    const EYE: Vec3 = Vec3::new(0.5, 0.5, 0.5);

    #[test]
    fn in_the_open_the_whole_sky_shows() {
        assert_eq!(sky_seen(EYE, |_| false), 1.0);
    }

    #[test]
    fn sealed_in_no_sky_shows() {
        // Every Cell is solid but the one the eyes are in.
        assert_eq!(sky_seen(EYE, |cell| cell != IVec3::ZERO), 0.0);
        assert_eq!(daylight_from(0.0), 0.0);
    }

    #[test]
    fn a_low_roof_hides_the_whole_sky() {
        // Everything from 4 Cells up is solid.
        assert_eq!(sky_seen(EYE, |cell| cell.y >= 4), 0.0);
    }

    #[test]
    fn a_roof_out_of_reach_counts_as_sky() {
        // Everything from 60 Cells up is solid. Straight up, the rays reach y = 48.5 at most.
        assert_eq!(sky_seen(EYE, |cell| cell.y >= 60), 1.0);
    }

    #[test]
    fn a_wall_on_one_side_hides_the_sky_there_but_leaves_full_daylight() {
        // Everything from x = 3 on is solid. Of the 13 rays, the 3 in each ring that lean
        // towards +x (at 0°, 60° and 300° around) meet it; straight up and the other 6 don't.
        let seen = sky_seen(EYE, |cell| cell.x >= 3);
        assert_eq!(seen, 7.0 / 13.0);
        // Beside a cliff, under an overhang: still daylight.
        assert_eq!(daylight_from(seen), 1.0);
    }
}
