//! The god tools. They change the world through Edits, which create and destroy matter by
//! design.
//!
//! Controls: 1–4 choose Dig, Place, Heat brush or Spawn fluid, and the palette in the top-left
//! corner sets what they put down and how the brush heats. While input is gameplay input
//! (`crate::input_policy`), with the cursor captured:
//!
//! - Dig: left click turns the Cell in view into Vacuum.
//! - Place: right click puts down the palette's Cell against the face in view.
//! - Heat brush: holding the left button heats, or cools, the Cells around the one in view.
//! - Spawn fluid: right click puts down the palette's gas or liquid against the face in view.
//!
//! Holding Alt frees the cursor to use the palette. A click made while it is free never uses a
//! tool, even once Alt is released with the button still down. While the World is paused, the
//! tools do nothing.
//!
//! Every Edit is logged, and the Cell it changed is logged as it changes over the next Ticks.

mod brush;
mod palette;
mod target;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;
use bevy_egui::input::{EguiWantsInput, egui_wants_any_keyboard_input};
use sim::{Cell, CellPos, Edit, ElementId, Phase};

use crate::input_policy::{InputPolicy, gameplay_input};
use crate::player::raycast::{Hit, REACH, raycast};
use crate::player::{AwaitingSpawn, Player, PlayerSystems};
use crate::session::{Play, Session, SessionSystems};
use crate::sim_bridge::{Elements, SimBridge, SimCommand, SimReport};
use crate::world::WorldView;
use brush::{BrushStroke, brush_cells};
pub use palette::{Palette, Tool, cell_of, full_mass_grams};
pub use target::{placement_target, stops_sight};

/// How many Ticks the log follows the Cell an Edit changed: 5 s.
const WATCH_TICKS: u64 = 25;

/// The colour the Cell a tool would change is outlined in.
const OUTLINE_COLOR: Color = Color::WHITE;

/// The god tools and their palette, while a World is open. Needs the session, the input policy,
/// the sim bridge and the player, and `EguiPlugin`.
pub struct GodToolsPlugin;

impl Plugin for GodToolsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Aim>()
            .init_resource::<WatchedCell>()
            .add_systems(
                OnEnter(Session::Open),
                set_up_tools.in_set(SessionSystems::Build),
            )
            // The tools act before the player's systems, on the Cell in view as it was at the end
            // of the last frame: what the player saw when they clicked.
            .add_systems(
                Update,
                (
                    choose_tool
                        .run_if(gameplay_input)
                        .run_if(not(egui_wants_any_keyboard_input)),
                    use_tool,
                )
                    .chain()
                    .before(PlayerSystems)
                    .run_if(in_state(Play::Running)),
            )
            .add_systems(
                Update,
                (aim, outline_target, log_watched_cell)
                    .chain()
                    .after(PlayerSystems)
                    .run_if(in_state(Session::Open)),
            )
            .add_systems(
                EguiPrimaryContextPass,
                palette::draw_palette.run_if(in_state(Session::Open)),
            );
    }
}

/// The Cell in view: the first Cell within reach of the player's eyes, along their line of sight,
/// that is neither Vacuum nor gas. Dig and the heat brush act on it, and Place and Spawn fluid put
/// down against it. `None` while nothing is in reach, or the player is waiting to spawn.
///
/// It is found once the player has moved for the frame, so during `Update` before
/// [`PlayerSystems`] it is the Cell in view on the last frame drawn.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Deref)]
pub struct Aim(pub Option<Hit>);

/// The Cell the latest Edit changed, which the log follows for [`WATCH_TICKS`].
#[derive(Resource, Clone, Copy, Debug, Default)]
struct WatchedCell {
    pos: Option<CellPos>,
    /// The last Tick to log it after.
    until_tick: u64,
    /// The Cell as last logged.
    logged: Option<Cell>,
}

/// The Cells as the game last heard them from the simulation.
#[derive(SystemParam)]
struct Cells<'w> {
    view: Res<'w, WorldView>,
    elements: Res<'w, Elements>,
}

impl Cells<'_> {
    /// The Cell at `cell`, or `None` if its Chunk isn't in the view.
    fn get(&self, cell: IVec3) -> Option<Cell> {
        self.view.cell(cell_pos(cell))
    }

    /// The Phase of the Cell at `cell`, or `None` if its Chunk isn't in the view.
    fn phase_at(&self, cell: IVec3) -> Option<Phase> {
        self.get(cell)
            .map(|cell| self.elements.get(cell.element).phase)
    }

    /// The Cell the tool puts down against `hit`, if it can: see [`placement_target`].
    fn placement_target(&self, hit: Hit) -> Option<IVec3> {
        placement_target(hit, |cell| self.phase_at(cell))
    }

    /// `cell`'s Element, Mass and temperature, for the log.
    fn describe(&self, cell: Cell) -> String {
        let name = &self.elements.get(cell.element).name;
        if cell.element == ElementId::VACUUM {
            return name.clone();
        }
        let kg = f64::from(cell.mass_grams) / 1_000.0;
        let celsius = cell.temperature_k - 273.15;
        format!("{name}, {kg:.3} kg at {celsius:.2} °C")
    }
}

/// The mouse buttons, as far as the tools may use them: only while input is gameplay input, and
/// egui doesn't want the pointer.
#[derive(SystemParam)]
struct ToolButtons<'w> {
    mouse: Res<'w, ButtonInput<MouseButton>>,
    policy: Res<'w, InputPolicy>,
    ui: Res<'w, EguiWantsInput>,
}

impl ToolButtons<'_> {
    fn usable(&self) -> bool {
        self.policy.gameplay && !self.ui.wants_any_pointer_input()
    }

    fn just_pressed(&self, button: MouseButton) -> bool {
        self.usable() && self.mouse.just_pressed(button)
    }

    fn pressed(&self, button: MouseButton) -> bool {
        self.usable() && self.mouse.pressed(button)
    }
}

/// Sends Edits to the simulation, and has the log follow the Cells they change.
#[derive(SystemParam)]
struct Editor<'w> {
    bridge: Res<'w, SimBridge>,
    report: Res<'w, SimReport>,
    watched: ResMut<'w, WatchedCell>,
}

impl Editor<'_> {
    fn send(&self, edit: Edit) {
        self.bridge.send(SimCommand::Edit(edit));
    }

    /// Has the log follow `pos`, which held `old` before the Edit, for the next [`WATCH_TICKS`].
    /// Until the Edit's result reaches the game, the Cell still reads as `old`, which isn't
    /// logged. While the log is following `pos` already, it carries on from where it was.
    fn watch(&mut self, pos: CellPos, old: Option<Cell>) {
        let until_tick = self.report.tick + WATCH_TICKS;
        if self.watched.pos == Some(pos) {
            self.watched.until_tick = until_tick;
            return;
        }
        *self.watched = WatchedCell {
            pos: Some(pos),
            until_tick,
            logged: old,
        };
    }
}

/// Starts a World's tools afresh: Dig in hand, the palette set for its Elements, nothing in view
/// and nothing followed in the log.
fn set_up_tools(
    mut commands: Commands,
    elements: Res<Elements>,
    mut aim: ResMut<Aim>,
    mut watched: ResMut<WatchedCell>,
) {
    commands.insert_resource(Palette::new(&elements));
    *aim = Aim::default();
    *watched = WatchedCell::default();
}

/// Number keys 1–4 choose a tool.
fn choose_tool(keys: Res<ButtonInput<KeyCode>>, mut palette: ResMut<Palette>) {
    if let Some(tool) = Tool::ALL
        .into_iter()
        .find(|tool| keys.just_pressed(tool.key()))
    {
        palette.tool = tool;
    }
}

/// Finds the Cell in view.
fn aim(
    eyes: Single<&Transform, (With<Player>, Without<AwaitingSpawn>)>,
    cells: Cells,
    mut aim: ResMut<Aim>,
) {
    let hit = raycast(eyes.translation, *eyes.forward(), REACH, |cell| {
        cells.phase_at(cell).is_some_and(stops_sight)
    });
    aim.set_if_neq(Aim(hit));
}

/// Uses the tool in hand, as the mouse buttons say. A heat brush stroke ends as soon as the
/// button is up, or the tools can't use it, before it heats any more.
fn use_tool(
    buttons: ToolButtons,
    time: Res<Time>,
    aim: Res<Aim>,
    palette: Res<Palette>,
    cells: Cells,
    mut editor: Editor,
    mut stroke: Local<BrushStroke>,
) {
    if palette.tool != Tool::HeatBrush || !buttons.pressed(MouseButton::Left) {
        stroke.end();
    }
    match palette.tool {
        Tool::Dig => {
            if buttons.just_pressed(MouseButton::Left)
                && let Some(hit) = **aim
                && let Some(old) = cells.get(hit.cell)
            {
                let pos = cell_pos(hit.cell);
                info!("dug out {} at {}", cells.describe(old), show(pos));
                editor.send(Edit::Dig { pos });
                editor.watch(pos, Some(old));
            }
        }
        Tool::Place | Tool::SpawnFluid => {
            if buttons.just_pressed(MouseButton::Right)
                && let Some(target) = aim.and_then(|hit| cells.placement_target(hit))
            {
                let (verb, cell) = match palette.tool {
                    Tool::Place => ("placed", palette.place),
                    _ => ("spawned", palette.spawn),
                };
                let pos = cell_pos(target);
                info!("{verb} {} at {}", cells.describe(cell), show(pos));
                editor.send(Edit::Place { pos, cell });
                editor.watch(pos, cells.get(target));
            }
        }
        Tool::HeatBrush => {
            if buttons.just_pressed(MouseButton::Left) {
                stroke.start();
            }
            if let Some(seconds) = stroke.hold(time.delta_secs())
                && let Some(hit) = **aim
            {
                let kelvin = palette.brush_strength * seconds;
                let mut shifted = 0;
                for cell in brush_cells(hit.cell, palette.brush_radius) {
                    if cells
                        .get(cell)
                        .is_some_and(|c| c.element != ElementId::VACUUM)
                    {
                        editor.send(Edit::ShiftTemperature {
                            pos: cell_pos(cell),
                            kelvin,
                        });
                        shifted += 1;
                    }
                }
                let centre = cell_pos(hit.cell);
                debug!(
                    "the heat brush shifted {shifted} Cells around {} by {kelvin:+.2} K",
                    show(centre)
                );
                editor.watch(centre, cells.get(hit.cell));
            }
        }
    }
}

/// Outlines the Cell the tool in hand would change. For the heat brush, it also shows how far the
/// brush reaches.
fn outline_target(aim: Res<Aim>, palette: Res<Palette>, cells: Cells, mut gizmos: Gizmos) {
    let Some(hit) = **aim else {
        return;
    };
    let target = match palette.tool {
        Tool::Dig | Tool::HeatBrush => Some(hit.cell),
        Tool::Place | Tool::SpawnFluid => cells.placement_target(hit),
    };
    let Some(target) = target else {
        return;
    };
    let centre = target.as_vec3() + Vec3::splat(0.5);
    // A little larger than the Cell, so its faces don't hide the outline.
    let outline = Transform::from_translation(centre).with_scale(Vec3::splat(1.02));
    gizmos.cube(outline, OUTLINE_COLOR);
    if palette.tool == Tool::HeatBrush && palette.brush_radius > 0.0 {
        gizmos.sphere(
            Isometry3d::from_translation(centre),
            palette.brush_radius,
            OUTLINE_COLOR,
        );
    }
}

/// After each Tick, logs the Cell the latest Edit changed if it has changed since it was last
/// logged, for [`WATCH_TICKS`] after that Edit.
fn log_watched_cell(report: Res<SimReport>, cells: Cells, mut watched: ResMut<WatchedCell>) {
    if !report.is_changed() {
        return;
    }
    let Some(pos) = watched.pos else {
        return;
    };
    if report.tick > watched.until_tick {
        watched.pos = None;
        return;
    }
    let Some(cell) = cells.view.cell(pos) else {
        return;
    };
    if watched.logged != Some(cell) {
        info!(
            "Tick {}: {} holds {}",
            report.tick,
            show(pos),
            cells.describe(cell)
        );
        watched.logged = Some(cell);
    }
}

fn cell_pos(cell: IVec3) -> CellPos {
    CellPos::new(cell.x, cell.y, cell.z)
}

/// `pos` as `(x, y, z)`, for the log.
fn show(pos: CellPos) -> String {
    format!("({}, {}, {})", pos.x, pos.y, pos.z)
}
