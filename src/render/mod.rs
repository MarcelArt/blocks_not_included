//! Drawing the world: every Chunk in [`WorldView`] has its solid Cells drawn as flat-coloured
//! cubes, remeshed off the main thread whenever they change. [`ScenePlugin`] adds the sky, the
//! sun and a camera.

mod mesher;
mod scene;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use bevy::prelude::*;
use bevy::tasks::futures::check_ready;
use bevy::tasks::{AsyncComputeTaskPool, Task};
use sim::ChunkPos;

use crate::sim_bridge::{Elements, SimFocus, distance_squared};
use crate::world::WorldView;
use mesher::{ChunkNeighbourhood, SolidColors, face_neighbours, mesh_solids};
pub use scene::ScenePlugin;

/// How many Chunks may start meshing each frame. Render distance fills in about 6 s, at some 200
/// Chunks a second, and each arrival also sends up to six loaded neighbours back to be remeshed:
/// on this laptop meshing kept up with loading at 144 frames per second. At 60 it would need
/// about 13 a frame. Below that, meshing lags behind loading until loading ends.
const MESHING_STARTS_PER_FRAME: usize = 16;

/// Draws the solid Cells of every Chunk in [`WorldView`], and keeps them up to date. Needs the
/// sim bridge.
pub struct ChunkMeshPlugin;

impl Plugin for ChunkMeshPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, set_up_chunk_meshes).add_systems(
            Update,
            (
                use_element_colors.run_if(resource_changed::<Elements>),
                follow_world_view,
                start_meshing,
                finish_meshing,
            )
                .chain(),
        );
    }
}

/// The entity drawing the solid Cells of the Chunk at this position.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct SolidMesh(pub ChunkPos);

/// What is drawn of each Chunk, and what is being meshed.
#[derive(Resource)]
struct ChunkMeshes {
    /// The one material every Chunk shares: white, so the vertex colours show.
    material: Handle<StandardMaterial>,
    colors: Arc<SolidColors>,
    /// Chunks in the view whose mesh is out of date and not yet being remade.
    stale: HashSet<ChunkPos>,
    /// Meshing under way. Dropping a task cancels it.
    tasks: HashMap<ChunkPos, Task<Option<Mesh>>>,
    /// The entity drawing each Chunk that has any solid faces.
    entities: HashMap<ChunkPos, Entity>,
}

fn set_up_chunk_meshes(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
    elements: Res<Elements>,
) {
    let material = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        // Rock and soil: matte, with no highlights.
        perceptual_roughness: 1.0,
        reflectance: 0.1,
        ..default()
    });
    commands.insert_resource(ChunkMeshes {
        material,
        colors: Arc::new(SolidColors::new(&elements)),
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
    meshes.stale.extend(view.chunks().map(|(pos, _)| pos));
}

/// Stops drawing Chunks that left the view this frame. Marks stale every Chunk in the view that
/// changed, and every one with a face neighbour that changed or left, since the faces along its
/// border depend on that neighbour.
fn follow_world_view(
    mut commands: Commands,
    view: Res<WorldView>,
    mut meshes: ResMut<ChunkMeshes>,
) {
    let meshes = &mut *meshes;
    for pos in view.removed() {
        meshes.stale.remove(pos);
        meshes.tasks.remove(pos);
        if let Some(entity) = meshes.entities.remove(pos) {
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
        face_neighbours(pos).into_iter().for_each(&mut mark_stale);
    }
    for &pos in view.removed() {
        face_neighbours(pos).into_iter().for_each(&mut mark_stale);
    }
}

/// Starts meshing the stale Chunks nearest the focus on the async compute pool, up to
/// [`MESHING_STARTS_PER_FRAME`].
fn start_meshing(view: Res<WorldView>, focus: Res<SimFocus>, mut meshes: ResMut<ChunkMeshes>) {
    if meshes.stale.is_empty() {
        return;
    }
    let centre = ChunkPos::of(focus.0);
    let mut nearest: Vec<ChunkPos> = meshes.stale.iter().copied().collect();
    if nearest.len() > MESHING_STARTS_PER_FRAME {
        nearest.select_nth_unstable_by_key(MESHING_STARTS_PER_FRAME, |&pos| {
            distance_squared(pos, centre)
        });
        nearest.truncate(MESHING_STARTS_PER_FRAME);
    }
    let pool = AsyncComputeTaskPool::get();
    for pos in nearest {
        meshes.stale.remove(&pos);
        let Some(neighbourhood) = ChunkNeighbourhood::gather(pos, |pos| view.chunk(pos).cloned())
        else {
            continue;
        };
        let colors = Arc::clone(&meshes.colors);
        let task = pool.spawn(async move {
            let geometry = mesh_solids(&neighbourhood, &colors);
            (!geometry.is_empty()).then(|| geometry.into_mesh())
        });
        // This cancels any meshing of the Chunk already under way, which is out of date.
        meshes.tasks.insert(pos, task);
    }
}

/// Draws each Chunk whose meshing has finished, or stops drawing it if it has no solid faces.
fn finish_meshing(
    mut commands: Commands,
    mut meshes: ResMut<ChunkMeshes>,
    mut mesh_assets: ResMut<Assets<Mesh>>,
) {
    let ChunkMeshes {
        material,
        tasks,
        entities,
        ..
    } = &mut *meshes;
    tasks.retain(|&pos, task| {
        let Some(mesh) = check_ready(task) else {
            return true;
        };
        match (mesh, entities.get(&pos).copied()) {
            (Some(mesh), Some(entity)) => {
                commands
                    .entity(entity)
                    .insert(Mesh3d(mesh_assets.add(mesh)));
            }
            (Some(mesh), None) => {
                let origin = pos.origin();
                let entity = commands
                    .spawn((
                        SolidMesh(pos),
                        Mesh3d(mesh_assets.add(mesh)),
                        MeshMaterial3d(material.clone()),
                        Transform::from_translation(
                            IVec3::new(origin.x, origin.y, origin.z).as_vec3(),
                        ),
                    ))
                    .id();
                entities.insert(pos, entity);
            }
            (None, Some(entity)) => {
                commands.entity(entity).despawn();
                entities.remove(&pos);
            }
            (None, None) => {}
        }
        false
    });
}
