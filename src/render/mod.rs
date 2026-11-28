//! Drawing the world: every Chunk in [`WorldView`] has its solid Cells drawn as textured
//! cubes, its liquids as translucent volumes, and its tinted gases as a faint haze, remeshed off
//! the main thread whenever they change. [`ScenePlugin`] adds the sky and the sun.
//!
//! All of it lasts as long as the World: closing it despawns every Chunk's meshes and drops the
//! meshing under way, so none of it is drawn in the next World.

mod fluids;
mod mesher;
mod scene;
#[cfg(test)]
pub(crate) mod test_support;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy::tasks::futures::check_ready;
use bevy::tasks::{AsyncComputeTaskPool, Task};
use sim::{CellPos, ChunkPos};

use crate::session::{Session, SessionSystems};
use crate::sim_bridge::{Elements, SimFocus, distance_squared};
use crate::world::WorldView;
use fluids::{FluidLooks, mesh_fluids};
use mesher::neighbours;
pub(crate) use mesher::{ChunkGeometry, ChunkNeighbourhood, Faces, face_neighbours, mesh_greedy};
use mesher::{SolidColors, mesh_solids};
pub use scene::ScenePlugin;

/// How many Chunks may start meshing each frame. Render distance fills in about 6 s, at some 200
/// Chunks a second, and each arrival also sends its loaded neighbours back to be remeshed: on
/// this laptop meshing kept up with loading at 144 frames per second, when that was up to six of
/// them. At 60 it would need about 13 a frame. Below that, meshing lags behind loading until
/// loading ends.
const MESHING_STARTS_PER_FRAME: usize = 16;

/// Draws the solid, liquid and tinted gas Cells of every Chunk in [`WorldView`], and keeps them up
/// to date, while a World is open. Needs the session and the sim bridge.
pub struct ChunkMeshPlugin;

impl Plugin for ChunkMeshPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            OnEnter(Session::Open),
            set_up_chunk_meshes.in_set(SessionSystems::Build),
        )
        // Meshing under way is dropped, which cancels it. The entities go by `DespawnOnExit`.
        .add_systems(OnExit(Session::Open), |mut commands: Commands| {
            commands.remove_resource::<ChunkMeshes>();
        })
        .add_systems(
            Update,
            (
                use_element_colors.run_if(resource_exists_and_changed::<Elements>),
                follow_world_view,
                start_meshing,
                finish_meshing,
            )
                .chain()
                .run_if(in_state(Session::Open)),
        );
    }
}

/// Which of a Chunk's meshes an entity draws. Each has a material of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChunkLayer {
    /// Opaque, textured cubes.
    Solid,
    /// Translucent volumes, whose tops show how full each Cell is.
    Liquid,
    /// A faint, unlit haze around each gas whose Element has a tint.
    Gas,
}

impl ChunkLayer {
    const ALL: [ChunkLayer; 3] = [ChunkLayer::Solid, ChunkLayer::Liquid, ChunkLayer::Gas];

    fn index(self) -> usize {
        self as usize
    }
}

/// The entity drawing one layer of the Chunk at `pos`.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChunkMesh {
    pub pos: ChunkPos,
    pub layer: ChunkLayer,
}

/// A Chunk's new meshes, by [`ChunkLayer`]: `None` for a layer with nothing to draw.
type LayerMeshes = [Option<Mesh>; 3];

/// What is drawn of each Chunk, and what is being meshed.
#[derive(Resource)]
struct ChunkMeshes {
    /// One material per [`ChunkLayer`], shared by every Chunk: white, so the vertex colours show.
    materials: [Handle<StandardMaterial>; 3],
    colors: Arc<SolidColors>,
    looks: Arc<FluidLooks>,
    /// Chunks in the view whose meshes are out of date and not yet being remade.
    stale: HashSet<ChunkPos>,
    /// Meshing under way. Dropping a task cancels it.
    tasks: HashMap<ChunkPos, Task<LayerMeshes>>,
    /// The entities drawing each Chunk, by [`ChunkLayer`]. A layer with nothing to draw has none.
    entities: HashMap<ChunkPos, [Option<Entity>; 3]>,
}

fn set_up_chunk_meshes(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
    assets: Res<AssetServer>,
    elements: Res<Elements>,
) {
    let solid = StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(
            assets
                .load_builder()
                .with_settings(|settings: &mut bevy::image::ImageLoaderSettings| {
                    use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
                    settings.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                        address_mode_u: ImageAddressMode::Repeat,
                        ..ImageSamplerDescriptor::nearest()
                    });
                })
                .load("textures/solids.png"),
        ),
        // Rock and soil: matte, with no highlights.
        perceptual_roughness: 1.0,
        reflectance: 0.1,
        ..default()
    };
    // Fluid faces show from both sides, so a surface stays visible from beneath it, and from
    // inside a cloud of gas. Translucent faces don't write depth, so overlapping ones all show.
    let liquid = StandardMaterial {
        base_color: Color::WHITE,
        alpha_mode: AlphaMode::Blend,
        // A smooth surface that catches the sun.
        perceptual_roughness: 0.2,
        reflectance: 0.3,
        double_sided: true,
        cull_mode: None,
        ..default()
    };
    let gas = StandardMaterial {
        base_color: Color::WHITE,
        alpha_mode: AlphaMode::Blend,
        // A haze rather than a surface: the same colour from every side.
        unlit: true,
        double_sided: true,
        cull_mode: None,
        ..default()
    };
    commands.insert_resource(ChunkMeshes {
        materials: [solid, liquid, gas].map(|material| materials.add(material)),
        colors: Arc::new(SolidColors::new(&elements)),
        looks: Arc::new(FluidLooks::new(&elements)),
        stale: HashSet::new(),
        tasks: HashMap::new(),
        entities: HashMap::new(),
    });
}

/// Takes up a changed Element table: every Chunk is remeshed in its colours.
fn use_element_colors(
    elements: Res<Elements>,
    view: Res<WorldView>,
    mut meshes: ResMut<ChunkMeshes>,
) {
    meshes.colors = Arc::new(SolidColors::new(&elements));
    meshes.looks = Arc::new(FluidLooks::new(&elements));
    meshes.stale.extend(view.chunks().map(|(pos, _)| pos));
}

/// Stops drawing Chunks that left the view this frame. Marks stale every Chunk in the view that
/// changed, and every one with a neighbour that changed or left, since the faces along its border
/// depend on its face neighbours, and the shading of their corners on its edge and corner
/// neighbours too.
fn follow_world_view(
    mut commands: Commands,
    view: Res<WorldView>,
    mut meshes: ResMut<ChunkMeshes>,
) {
    let meshes = &mut *meshes;
    for pos in view.removed() {
        meshes.stale.remove(pos);
        meshes.tasks.remove(pos);
        for entity in meshes.entities.remove(pos).into_iter().flatten().flatten() {
            commands.entity(entity).despawn();
        }
    }
    let mut mark_stale = |pos: ChunkPos| {
        if view.chunk(pos).is_some() {
            meshes.stale.insert(pos);
        }
    };
    for &pos in view.changed() {
        mark_stale(pos);
        neighbours(pos).into_iter().for_each(&mut mark_stale);
    }
    for &pos in view.removed() {
        neighbours(pos).into_iter().for_each(&mut mark_stale);
    }
}

/// Starts meshing the stale Chunks nearest the focus on the async compute pool, up to
/// [`MESHING_STARTS_PER_FRAME`].
fn start_meshing(view: Res<WorldView>, focus: Res<SimFocus>, mut meshes: ResMut<ChunkMeshes>) {
    let pool = AsyncComputeTaskPool::get();
    for pos in take_nearest(&mut meshes.stale, focus.0, MESHING_STARTS_PER_FRAME) {
        let Some(neighbourhood) = ChunkNeighbourhood::gather(pos, |pos| view.chunk(pos).cloned())
        else {
            continue;
        };
        let colors = Arc::clone(&meshes.colors);
        let looks = Arc::clone(&meshes.looks);
        let task = pool.spawn(async move {
            let solid = mesh_solids(&neighbourhood, &colors);
            let fluids = mesh_fluids(&neighbourhood, &looks);
            [solid, fluids.liquid, fluids.gas]
                .map(|geometry| (!geometry.is_empty()).then(|| geometry.into_mesh()))
        });
        // This cancels any meshing of the Chunk already under way, which is out of date.
        meshes.tasks.insert(pos, task);
    }
}

/// Takes up to `count` Chunks out of `stale`: those nearest the Chunk of `focus`.
pub(crate) fn take_nearest(
    stale: &mut HashSet<ChunkPos>,
    focus: CellPos,
    count: usize,
) -> Vec<ChunkPos> {
    if stale.len() <= count {
        return stale.drain().collect();
    }
    let centre = ChunkPos::of(focus);
    let mut nearest: Vec<ChunkPos> = stale.iter().copied().collect();
    nearest.select_nth_unstable_by_key(count, |&pos| distance_squared(pos, centre));
    nearest.truncate(count);
    for pos in &nearest {
        stale.remove(pos);
    }
    nearest
}

/// Draws each Chunk whose meshing has finished: each layer with faces gets a new mesh, and each
/// without stops being drawn.
fn finish_meshing(
    mut commands: Commands,
    mut meshes: ResMut<ChunkMeshes>,
    mut mesh_assets: ResMut<Assets<Mesh>>,
) {
    let ChunkMeshes {
        materials,
        tasks,
        entities,
        ..
    } = &mut *meshes;
    tasks.retain(|&pos, task| {
        let Some(layer_meshes) = check_ready(task) else {
            return true;
        };
        let drawn = entities.entry(pos).or_default();
        for (layer, mesh) in ChunkLayer::ALL.into_iter().zip(layer_meshes) {
            let entity = &mut drawn[layer.index()];
            match (mesh, *entity) {
                (Some(mesh), Some(entity)) => {
                    commands
                        .entity(entity)
                        .insert(Mesh3d(mesh_assets.add(mesh)));
                }
                (Some(mesh), None) => {
                    let origin = pos.origin();
                    let mut spawned = commands.spawn((
                        ChunkMesh { pos, layer },
                        DespawnOnExit(Session::Open),
                        Mesh3d(mesh_assets.add(mesh)),
                        MeshMaterial3d(materials[layer.index()].clone()),
                        Transform::from_translation(
                            IVec3::new(origin.x, origin.y, origin.z).as_vec3(),
                        ),
                    ));
                    if layer != ChunkLayer::Solid {
                        // Light passes through fluids.
                        spawned.insert(NotShadowCaster);
                    }
                    *entity = Some(spawned.id());
                }
                (None, Some(old)) => {
                    commands.entity(old).despawn();
                    *entity = None;
                }
                (None, None) => {}
            }
        }
        if drawn.iter().all(Option::is_none) {
            entities.remove(&pos);
        }
        false
    });
}
