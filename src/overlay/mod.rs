//! The Overlays (`CONTEXT.md`): views that colour every Cell in the Active Region by its Element,
//! temperature or Mass, in place of its normal look. O cycles through them: Element, Temperature,
//! Mass, then off again. A legend in the bottom-right corner names the Overlay shown and its
//! colours.
//!
//! While an Overlay is shown, each loaded Chunk in the Active Region is drawn as translucent
//! faces wherever the Overlay's value changes from one Cell to the next ([`mesh`]), meshed
//! off the main thread. The normal look of every Chunk is hidden, Dormant ones included, so the
//! Overlay is all there is to see, against a dark, neutral background: the sky's blue showing
//! through every layer would make the Temperature Overlay look colder than it is.
//!
//! Each World starts with no Overlay. Closing it despawns the Overlay's meshes and drops the
//! meshing under way.

mod mesh;
mod ramp;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use bevy::camera::visibility::VisibilitySystems;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy::tasks::futures::check_ready;
use bevy::tasks::{AsyncComputeTaskPool, Task};
use bevy_egui::input::egui_wants_any_keyboard_input;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use sim::{ActiveRegion, ChunkPos, Phase};

use crate::render::{ChunkMesh, ChunkNeighbourhood, face_neighbours, take_nearest};
use crate::session::{Session, SessionSystems};
use crate::sim_bridge::{Elements, SimFocus};
use crate::world::WorldView;
use mesh::{OverlayLooks, mesh_overlay};
use ramp::{BLACK, MAGENTA, Rgb, TEMPERATURE_STOPS, WHITE};

/// The key that cycles through the Overlays.
const CYCLE_KEY: KeyCode = KeyCode::KeyO;

/// The background while an Overlay is shown.
const OVERLAY_BACKGROUND: Color = Color::srgb(0.06, 0.06, 0.07);

/// How many Chunks may start meshing each frame. The Active Region holds 32 Chunks, so a new
/// Overlay has started meshing all of them within two frames.
const MESHING_STARTS_PER_FRAME: usize = 16;

/// The Overlays, and the key that cycles through them, while a World is open. Needs the session,
/// the sim bridge, the Chunk meshes, and `EguiPlugin`.
pub struct OverlayPlugin;

impl Plugin for OverlayPlugin {
    fn build(&self, app: &mut App) {
        let open = in_state(Session::Open);
        app.init_resource::<ShownOverlay>()
            .add_systems(
                OnEnter(Session::Open),
                set_up_overlay_meshes.in_set(SessionSystems::Build),
            )
            // Meshing under way is dropped, which cancels it. The entities go by `DespawnOnExit`,
            // and the background returns to normal as the Overlay goes.
            .add_systems(
                OnExit(Session::Open),
                |mut commands: Commands, mut shown: ResMut<ShownOverlay>| {
                    commands.remove_resource::<OverlayMeshes>();
                    shown.set_if_neq(ShownOverlay::default());
                },
            )
            .add_systems(
                Update,
                (
                    cycle_overlay
                        .run_if(open.clone())
                        .run_if(not(egui_wants_any_keyboard_input)),
                    use_overlay_looks.run_if(open.clone()).run_if(
                        resource_changed::<ShownOverlay>
                            .or_else(resource_exists_and_changed::<Elements>),
                    ),
                    set_background.run_if(resource_changed::<ShownOverlay>),
                    (follow_world_view, start_meshing, finish_meshing).run_if(open.clone()),
                )
                    .chain(),
            )
            // Chunk meshes spawned during `Update` are hidden before the frame is drawn.
            .add_systems(
                PostUpdate,
                hide_normal_look.before(VisibilitySystems::VisibilityPropagate),
            )
            .add_systems(EguiPrimaryContextPass, draw_legend.run_if(open));
    }
}

/// An Overlay (`CONTEXT.md`): which property it colours Cells by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Overlay {
    /// Each Cell in its Element's colour, gases included.
    Element,
    /// From dark blue at 250 K, through green at 20 °C, to white at 1500 K.
    Temperature,
    /// From black when empty to white when full, and magenta above full.
    Mass,
}

impl Overlay {
    /// In the order O cycles through them.
    pub const ALL: [Overlay; 3] = [Overlay::Element, Overlay::Temperature, Overlay::Mass];

    pub fn name(self) -> &'static str {
        match self {
            Overlay::Element => "Element",
            Overlay::Temperature => "Temperature",
            Overlay::Mass => "Mass",
        }
    }
}

/// Which Overlay is shown, if any.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq, Deref)]
pub struct ShownOverlay(pub Option<Overlay>);

impl ShownOverlay {
    /// The next one in O's cycle: none, then each of [`Overlay::ALL`] in turn, then none again.
    pub fn next(self) -> Self {
        let next = match self.0 {
            None => Overlay::ALL.first(),
            Some(shown) => Overlay::ALL
                .iter()
                .skip_while(|&&overlay| overlay != shown)
                .nth(1),
        };
        Self(next.copied())
    }
}

/// The entity drawing the Overlay of the Chunk at this position.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct OverlayMesh(pub ChunkPos);

/// What the Overlay shown draws, and what is being meshed.
#[derive(Resource)]
struct OverlayMeshes {
    /// Shared by every Chunk: white, so the vertex colours show.
    material: Handle<StandardMaterial>,
    /// How the Overlay shown draws Cells. `None` while none is shown.
    looks: Option<Arc<OverlayLooks>>,
    /// The Active Region the Chunks drawn were chosen by. `None` while no Overlay is shown, and
    /// when a new one is, so that every Chunk in the Active Region is meshed afresh.
    region: Option<ActiveRegion>,
    /// Chunks in the Active Region whose meshes are out of date and not yet being remade.
    stale: HashSet<ChunkPos>,
    /// Meshing under way. Dropping a task cancels it.
    tasks: HashMap<ChunkPos, Task<Option<Mesh>>>,
    /// The entity drawing each Chunk. A Chunk with nothing to draw has none.
    entities: HashMap<ChunkPos, Entity>,
}

impl OverlayMeshes {
    fn in_region(&self, pos: ChunkPos) -> bool {
        self.region.is_some_and(|region| region.contains(pos))
    }

    /// Marks the Chunk at `pos` to be remeshed, if it is in the Active Region and `view`.
    fn mark_stale(&mut self, pos: ChunkPos, view: &WorldView) {
        if self.in_region(pos) && view.chunk(pos).is_some() {
            self.stale.insert(pos);
        }
    }

    /// Stops drawing and meshing the Chunk at `pos`.
    fn stop_drawing(&mut self, pos: ChunkPos, commands: &mut Commands) {
        self.stale.remove(&pos);
        self.tasks.remove(&pos);
        if let Some(entity) = self.entities.remove(&pos) {
            commands.entity(entity).despawn();
        }
    }
}

fn set_up_overlay_meshes(mut commands: Commands, mut materials: ResMut<Assets<StandardMaterial>>) {
    let material = StandardMaterial {
        base_color: Color::WHITE,
        alpha_mode: AlphaMode::Blend,
        // Lit like the solids, so a sunlit top face shows at its colour and the sides are shaded,
        // which shows each layer's shape. Matte, with no highlights.
        perceptual_roughness: 1.0,
        reflectance: 0.1,
        // Back faces stay culled. Where the value changes, a face shows each way, so the player
        // sees only the one facing them: the far side's colour, as if looking at that layer from
        // outside it.
        ..default()
    };
    commands.insert_resource(OverlayMeshes {
        material: materials.add(material),
        looks: None,
        region: None,
        stale: HashSet::new(),
        tasks: HashMap::new(),
        entities: HashMap::new(),
    });
}

/// O shows the next Overlay in the cycle.
fn cycle_overlay(keys: Res<ButtonInput<KeyCode>>, mut shown: ResMut<ShownOverlay>) {
    if keys.just_pressed(CYCLE_KEY) {
        *shown = shown.next();
        info!("Overlay: {}", shown.map_or("off", |overlay| overlay.name()));
    }
}

/// Takes up a new Overlay, or a changed Element table. Every Chunk in the Active Region is
/// remeshed in the new looks; until each is done, it keeps its old mesh. Without an Overlay,
/// nothing is drawn.
fn use_overlay_looks(
    mut commands: Commands,
    shown: Res<ShownOverlay>,
    elements: Res<Elements>,
    mut meshes: ResMut<OverlayMeshes>,
) {
    let meshes = &mut *meshes;
    // Meshing under way is in the old looks.
    meshes.tasks.clear();
    meshes.stale.clear();
    // `follow_world_view` meshes the whole Active Region afresh.
    meshes.region = None;
    meshes.looks = shown
        .0
        .map(|overlay| Arc::new(OverlayLooks::new(overlay, &elements)));
    if meshes.looks.is_none() {
        for (_, entity) in meshes.entities.drain() {
            commands.entity(entity).despawn();
        }
    }
}

/// Darkens the background while an Overlay is shown, and brings the normal one back after.
fn set_background(
    shown: Res<ShownOverlay>,
    mut background: ResMut<ClearColor>,
    mut normal: Local<Option<Color>>,
) {
    let normal = *normal.get_or_insert(background.0);
    background.0 = if shown.is_some() {
        OVERLAY_BACKGROUND
    } else {
        normal
    };
}

/// Follows the Active Region, and the Chunks that changed or left the view this frame. Chunks that
/// left the Active Region stop being drawn, and those that entered it are meshed. A changed Chunk
/// is remeshed, and so is each face neighbour of a Chunk that changed or left, since the faces
/// along its border depend on that neighbour.
fn follow_world_view(
    mut commands: Commands,
    view: Res<WorldView>,
    mut meshes: ResMut<OverlayMeshes>,
) {
    if meshes.looks.is_none() {
        return;
    }
    let meshes = &mut *meshes;
    let region = view.active_region();
    if region != meshes.region {
        let old = std::mem::replace(&mut meshes.region, region);
        let left: Vec<ChunkPos> = (meshes.entities.keys())
            .chain(meshes.tasks.keys())
            .chain(&meshes.stale)
            .copied()
            .filter(|&pos| !meshes.in_region(pos))
            .collect();
        for pos in left {
            meshes.stop_drawing(pos, &mut commands);
        }
        let entered = region
            .into_iter()
            .flat_map(ActiveRegion::chunks)
            .filter(|&pos| old.is_none_or(|old| !old.contains(pos)));
        for pos in entered {
            meshes.mark_stale(pos, &view);
        }
    }
    for &pos in view.removed() {
        meshes.stop_drawing(pos, &mut commands);
        for neighbour in face_neighbours(pos) {
            meshes.mark_stale(neighbour, &view);
        }
    }
    for &pos in view.changed() {
        meshes.mark_stale(pos, &view);
        for neighbour in face_neighbours(pos) {
            meshes.mark_stale(neighbour, &view);
        }
    }
}

/// Starts meshing the stale Chunks nearest the focus on the async compute pool, up to
/// [`MESHING_STARTS_PER_FRAME`].
fn start_meshing(view: Res<WorldView>, focus: Res<SimFocus>, mut meshes: ResMut<OverlayMeshes>) {
    let Some(looks) = meshes.looks.clone() else {
        return;
    };
    let pool = AsyncComputeTaskPool::get();
    for pos in take_nearest(&mut meshes.stale, focus.0, MESHING_STARTS_PER_FRAME) {
        let Some(neighbourhood) = ChunkNeighbourhood::gather(pos, |pos| view.chunk(pos).cloned())
        else {
            continue;
        };
        let looks = Arc::clone(&looks);
        let task = pool.spawn(async move {
            let geometry = mesh_overlay(&neighbourhood, &looks);
            (!geometry.is_empty()).then(|| geometry.into_mesh())
        });
        // This cancels any meshing of the Chunk already under way, which is out of date.
        meshes.tasks.insert(pos, task);
    }
}

/// Draws each Chunk whose meshing has finished, or stops drawing it if it has no faces.
fn finish_meshing(
    mut commands: Commands,
    mut meshes: ResMut<OverlayMeshes>,
    mut mesh_assets: ResMut<Assets<Mesh>>,
) {
    let OverlayMeshes {
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
                        OverlayMesh(pos),
                        DespawnOnExit(Session::Open),
                        Mesh3d(mesh_assets.add(mesh)),
                        MeshMaterial3d(material.clone()),
                        Transform::from_translation(
                            IVec3::new(origin.x, origin.y, origin.z).as_vec3(),
                        ),
                        NotShadowCaster,
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

/// While an Overlay is shown, hides the normal look of every Chunk.
fn hide_normal_look(shown: Res<ShownOverlay>, mut chunks: Query<&mut Visibility, With<ChunkMesh>>) {
    let visibility = if shown.is_some() {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    };
    for mut chunk in &mut chunks {
        chunk.set_if_neq(visibility);
    }
}

/// Names the Overlay shown, and what its colours mean, in the bottom-right corner.
fn draw_legend(
    mut contexts: EguiContexts,
    shown: Res<ShownOverlay>,
    elements: Res<Elements>,
) -> Result {
    egui::Window::new("Overlay")
        .anchor(egui::Align2::RIGHT_BOTTOM, [-8.0, -8.0])
        .resizable(false)
        .collapsible(false)
        // It only shows things, so it never takes the pointer.
        .interactable(false)
        .show(contexts.ctx_mut()?, |ui| {
            match shown.0 {
                None => {
                    ui.label("Off");
                }
                Some(overlay) => {
                    ui.strong(overlay.name());
                    match overlay {
                        Overlay::Element => {
                            for (_, element) in elements.iter() {
                                if element.phase != Phase::Vacuum {
                                    swatch(ui, element.color, &element.name);
                                }
                            }
                        }
                        Overlay::Temperature => {
                            for (temperature_k, color) in TEMPERATURE_STOPS.into_iter().rev() {
                                swatch(ui, color, &whole_celsius(temperature_k));
                            }
                        }
                        Overlay::Mass => {
                            swatch(ui, MAGENTA, "Over full");
                            swatch(ui, WHITE, "Full: Density × 1 m³");
                            swatch(ui, BLACK, "Empty");
                        }
                    }
                }
            }
            ui.weak("O cycles the Overlays.");
        });
    Ok(())
}

/// A square of `color`, followed by `label`.
fn swatch(ui: &mut egui::Ui, [r, g, b]: Rgb, label: &str) {
    ui.horizontal(|ui| {
        let (square, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
        let color = egui::Color32::from(egui::Rgba::from_rgb(r, g, b));
        ui.painter().rect_filled(square, 2.0, color);
        ui.label(label);
    });
}

/// `temperature_k` in whole degrees Celsius, such as "−23 °C" for 250 K.
fn whole_celsius(temperature_k: f32) -> String {
    let celsius = (temperature_k - 273.15).round();
    // 273 K is −0.15 °C, which rounds to −0.
    let celsius = if celsius == 0.0 { 0.0 } else { celsius };
    format!("{celsius} °C")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn o_cycles_from_off_through_each_overlay_and_back_to_off() {
        let mut shown = ShownOverlay::default();
        let mut seen = Vec::new();
        for _ in 0..4 {
            shown = shown.next();
            seen.push(shown.0);
        }
        assert_eq!(
            seen,
            [
                Some(Overlay::Element),
                Some(Overlay::Temperature),
                Some(Overlay::Mass),
                None
            ]
        );
    }
}
