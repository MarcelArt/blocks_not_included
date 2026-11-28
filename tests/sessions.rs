//! World sessions in a headless `App` (`docs/plans/18-world-sessions.md`): opening a World,
//! closing it and opening another, loading a saved player, pausing, and who the keyboard and mouse
//! are for.
//!
//! The simulation runs on a thread of its own, at 5 Ticks per second, so these tests wait in real
//! time, with generous timeouts. Keys and mouse buttons go in as the messages a window sends, so
//! the input policy reads them as it does in the game. There is no real window: a stand-in
//! primary window entity holds the cursor options the policy sets.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use bevy::asset::AssetPlugin;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input::mouse::MouseButtonInput;
use bevy::input::{ButtonState, InputPlugin};
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use bevy::time::TimeUpdateStrategy;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use bevy_egui::input::EguiWantsInput;
use blocks_not_included::debug_panel::{DebugPanelPlugin, SimClock};
use blocks_not_included::input_policy::{InputPolicy, InputPolicyPlugin};
use blocks_not_included::player::movement::{Body, MoveMode};
use blocks_not_included::player::{AwaitingSpawn, Player, PlayerPlugin};
use blocks_not_included::render::{ChunkLayer, ChunkMesh, ChunkMeshPlugin};
use blocks_not_included::saves::{PlayerState, WorldSave};
use blocks_not_included::session::{
    CloseWorld, OpenWorld, Play, Session, SessionPlugin, WorldStart,
};
use blocks_not_included::sim_bridge::{
    Elements, Generation, RENDER_DISTANCE, ReplaceOutcome, SimBridge, SimBridgePlugin, SimCapture,
    SimCommand, SimFocus, SimReport, SimStart, TickRate,
};
use blocks_not_included::sim_files::{SettingsFile, SimFilesPlugin};
use blocks_not_included::tools::{Aim, GodToolsPlugin, cell_of};
use blocks_not_included::world::WorldView;
use sim::{
    CHUNK_SIZE, Cell, CellPos, ChunkPos, ChunkSource, Edit, Element, ElementId, ElementTable,
    Phase, SimSettings, Simulation,
};

/// Far longer than anything here should take, so a slow machine doesn't fail a test.
const TIMEOUT: Duration = Duration::from_secs(20);

const SHIPPED_ELEMENTS: &str = include_str!("../assets/elements.ron");
const SHIPPED_SETTINGS: &str = include_str!("../assets/sim_settings.ron");

// --- Worlds ---

/// TestRock up to `y = 4`, and TestGas above it, so every Chunk holds something and loads into
/// the world view. Records when it is dropped, if asked to: the simulation owns its source, so
/// once it is dropped, the simulation is gone too.
struct GroundSource {
    rock: ElementId,
    gas: ElementId,
    dropped: Option<Arc<AtomicBool>>,
}

impl GroundSource {
    fn new(rock: ElementId, gas: ElementId) -> Self {
        Self {
            rock,
            gas,
            dropped: None,
        }
    }
}

impl ChunkSource for GroundSource {
    fn generate(&self, chunk: ChunkPos) -> Vec<Cell> {
        (0..CHUNK_SIZE.pow(3))
            .map(|i| {
                // Cells are listed x fastest, then z, then y.
                let y = chunk.origin().y + (i / (CHUNK_SIZE * CHUNK_SIZE)) as i32;
                if y <= 4 {
                    rock_cell(self.rock)
                } else {
                    gas_cell(self.gas)
                }
            })
            .collect()
    }
}

impl Drop for GroundSource {
    fn drop(&mut self) {
        if let Some(dropped) = &self.dropped {
            dropped.store(true, Ordering::SeqCst);
        }
    }
}

fn rock_cell(rock: ElementId) -> Cell {
    Cell {
        element: rock,
        mass_grams: 1_000,
        temperature_k: 300.0,
    }
}

fn gas_cell(gas: ElementId) -> Cell {
    Cell {
        element: gas,
        mass_grams: 1_000,
        temperature_k: 300.0,
    }
}

/// Vacuum, TestRock and TestGas. Neither conducts heat, and the gas is spread evenly, so the
/// world never changes by itself.
fn elements() -> (ElementTable, ElementId, ElementId) {
    let mut elements = ElementTable::new();
    let rock = elements.add(Element::new("TestRock").density(1_000.0).specific_heat(1.0));
    let gas = elements.add(
        Element::new("TestGas")
            .phase(Phase::Gas)
            .density(1.0)
            .specific_heat(1.0),
    );
    (elements, rock, gas)
}

/// A new World of [`GroundSource`]: the player spawns on the ground at the origin.
fn new_world(dropped: Option<Arc<AtomicBool>>) -> WorldStart {
    let (elements, rock, gas) = elements();
    let source = GroundSource {
        dropped,
        ..GroundSource::new(rock, gas)
    };
    WorldStart {
        sim: SimStart::fresh(elements, SimSettings::default(), Box::new(source)),
        player: None,
    }
}

/// A saved World of [`GroundSource`] after `ticks` Ticks, with the focus at `player`'s feet and
/// `player` as saved.
fn saved_world(player: PlayerState, ticks: u64) -> WorldStart {
    let (elements, rock, gas) = elements();
    let mut sim = Simulation::with_source(elements.clone(), Box::new(GroundSource::new(rock, gas)));
    sim.focus_on(cell_of_feet(player.body.feet));
    for _ in 0..ticks {
        sim.tick();
    }
    let capture = SimCapture {
        checkpoint: sim.checkpoint(),
        elements,
        settings: SimSettings::default(),
        generation: None,
    };
    let sim = SimStart::restored(capture, Box::new(GroundSource::new(rock, gas)))
        .expect("the capture restores");
    WorldStart {
        sim,
        player: Some(player),
    }
}

fn cell_of_feet(feet: Vec3) -> CellPos {
    let cell = feet.floor().as_ivec3();
    CellPos::new(cell.x, cell.y, cell.z)
}

/// A walking player in mid-air at `feet`, falling at 1 m/s, looking `pitch` up or down.
fn falling_player(feet: Vec3, pitch: f32) -> PlayerState {
    PlayerState {
        body: Body {
            feet,
            velocity: Vec3::new(0.0, -1.0, 0.0),
            mode: MoveMode::Walk,
            grounded: false,
        },
        yaw: 0.5,
        pitch,
    }
}

// --- The app ---

/// The primary window's stand-in.
#[derive(Resource)]
struct TestWindow(Entity);

/// Every [`ReplaceOutcome`] the app has received, oldest first. Messages are cleared after two
/// frames, so a system copies them here.
#[derive(Resource, Default)]
struct Outcomes(Vec<ReplaceOutcome>);

/// A headless app with everything a World needs but drawing: the session, the input policy, the
/// sim bridge and its files, the player, the Chunk meshes, the god tools and the debug panel.
fn session_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        StatesPlugin,
        InputPlugin,
        AssetPlugin {
            file_path: concat!(env!("CARGO_MANIFEST_DIR"), "/assets").into(),
            watch_for_changes_override: Some(false),
            ..default()
        },
    ))
    // For the god tools' outline. Gizmos look for skinned meshes too.
    .add_plugins(bevy::gizmos::GizmoPlugin)
    .init_asset::<bevy::mesh::skinning::SkinnedMeshInverseBindposes>()
    .init_asset::<Mesh>()
    .init_asset::<StandardMaterial>()
    // egui itself needs a renderer. Nothing here is drawn, so egui never wants input.
    .init_resource::<EguiWantsInput>()
    .add_plugins((
        SessionPlugin,
        InputPolicyPlugin,
        SimBridgePlugin,
        SimFilesPlugin,
        PlayerPlugin,
        ChunkMeshPlugin,
        GodToolsPlugin,
        DebugPanelPlugin,
    ))
    .init_resource::<Outcomes>()
    .add_systems(
        Update,
        |mut outcomes: MessageReader<ReplaceOutcome>, mut seen: ResMut<Outcomes>| {
            seen.0.extend(outcomes.read().cloned());
        },
    );
    let window = app
        .world_mut()
        .spawn((Window::default(), CursorOptions::default(), PrimaryWindow))
        .id();
    app.insert_resource(TestWindow(window));
    // Startup.
    app.update();
    app
}

fn open(app: &mut App, start: WorldStart) {
    app.world_mut().commands().queue(OpenWorld(start));
    app.world_mut().flush();
    app.update();
}

fn close(app: &mut App) {
    app.world_mut().commands().queue(CloseWorld);
    app.world_mut().flush();
    app.update();
}

fn send(app: &App, command: SimCommand) {
    app.world().resource::<SimBridge>().send(command);
}

fn tick(app: &App) -> u64 {
    app.world().resource::<SimReport>().tick
}

fn view(app: &App) -> &WorldView {
    app.world().resource::<WorldView>()
}

fn play(app: &App) -> Option<Play> {
    app.world()
        .get_resource::<State<Play>>()
        .map(|state| *state.get())
}

/// The player's body and camera rotation, and whether they are held while Chunks load.
fn player(app: &mut App) -> Option<(Body, Quat, Option<AwaitingSpawn>)> {
    app.world_mut()
        .query::<(&Player, &Transform, Option<&AwaitingSpawn>)>()
        .iter(app.world())
        .next()
        .map(|(player, transform, awaiting)| (**player, transform.rotation, awaiting.copied()))
}

fn player_released(app: &mut App) -> bool {
    player(app).is_some_and(|(_, _, awaiting)| awaiting.is_none())
}

fn chunk_meshes(app: &mut App) -> Vec<ChunkMesh> {
    app.world_mut()
        .query::<&ChunkMesh>()
        .iter(app.world())
        .copied()
        .collect()
}

fn cursor(app: &App) -> CursorOptions {
    let window = app.world().resource::<TestWindow>().0;
    app.world()
        .get::<CursorOptions>(window)
        .expect("the window's cursor options")
        .clone()
}

fn captured(cursor: &CursorOptions) -> bool {
    cursor.grab_mode == CursorGrabMode::Locked && !cursor.visible
}

fn key(app: &mut App, key_code: KeyCode, state: ButtonState) {
    let window = app.world().resource::<TestWindow>().0;
    app.world_mut().write_message(KeyboardInput {
        key_code,
        logical_key: Key::Unidentified(bevy::input::keyboard::NativeKey::Unidentified),
        state,
        text: None,
        repeat: false,
        window,
    });
}

/// Presses and releases `key_code` over two frames.
fn tap(app: &mut App, key_code: KeyCode) {
    key(app, key_code, ButtonState::Pressed);
    app.update();
    key(app, key_code, ButtonState::Released);
    app.update();
}

fn mouse(app: &mut App, button: MouseButton, state: ButtonState) {
    let window = app.world().resource::<TestWindow>().0;
    app.world_mut().write_message(MouseButtonInput {
        button,
        state,
        window,
    });
}

/// Asks the simulation for a capture and waits for it.
fn capture(app: &App) -> SimCapture {
    app.world()
        .resource::<SimBridge>()
        .capture()
        .recv_timeout(TIMEOUT)
        .expect("the capture arrives")
}

/// Runs frames for `duration`, checking `each` after every one.
fn update_for(app: &mut App, duration: Duration, mut each: impl FnMut(&mut App)) {
    let start = Instant::now();
    while start.elapsed() < duration {
        app.update();
        each(app);
        thread::sleep(Duration::from_millis(5));
    }
}

/// Runs frames until `done` holds, failing the test after [`TIMEOUT`].
fn update_until(app: &mut App, what: &str, mut done: impl FnMut(&mut App) -> bool) {
    let start = Instant::now();
    loop {
        app.update();
        if done(app) {
            return;
        }
        assert!(start.elapsed() < TIMEOUT, "timed out waiting until {what}");
        thread::sleep(Duration::from_millis(5));
    }
}

/// Whether `pos` lies within render distance of the Chunk at `centre`.
fn within_render_distance(pos: ChunkPos, centre: ChunkPos) -> bool {
    let (h, v) = (RENDER_DISTANCE.horizontal, RENDER_DISTANCE.vertical);
    (pos.x - centre.x).abs() <= h && (pos.y - centre.y).abs() <= v && (pos.z - centre.z).abs() <= h
}

// --- Tests ---

#[test]
fn a_world_opened_after_another_closes_starts_from_nothing_of_the_first() {
    let mut app = session_app();
    // Nothing runs until a World opens.
    assert!(app.world().get_resource::<SimBridge>().is_none());

    let (_, rock, _) = elements();
    let a_dropped = Arc::new(AtomicBool::new(false));
    open(&mut app, new_world(Some(Arc::clone(&a_dropped))));
    update_until(&mut app, "World A's player spawns", player_released);
    update_until(&mut app, "World A's Chunks are drawn", |app| {
        !chunk_meshes(app).is_empty()
    });
    // An Edit only World A has: a Cell of TestRock in the gas above its ground.
    let edited = CellPos::new(3, 10, 3);
    send(
        &app,
        SimCommand::Edit(Edit::Place {
            pos: edited,
            cell: rock_cell(rock),
        }),
    );
    update_until(&mut app, "World A's Edit shows up", |app| {
        view(app).cell(edited) == Some(rock_cell(rock))
    });

    // Render distance is still loading and meshing as World A closes.
    close(&mut app);

    assert_eq!(
        app.world().resource::<State<Session>>().get(),
        &Session::Closed
    );
    assert!(
        a_dropped.load(Ordering::SeqCst),
        "World A's simulation thread was still running"
    );
    let world = app.world();
    assert!(world.get_resource::<SimBridge>().is_none());
    assert!(world.get_resource::<WorldView>().is_none());
    assert!(world.get_resource::<Elements>().is_none());
    assert!(world.get_resource::<SimFocus>().is_none());
    assert!(world.get_resource::<SimReport>().is_none());
    assert!(world.get_resource::<TickRate>().is_none());
    assert!(player(&mut app).is_none(), "the player and camera remain");
    assert!(
        chunk_meshes(&mut app).is_empty(),
        "World A's Chunks are drawn"
    );
    assert!(!captured(&cursor(&app)), "the cursor stays captured");

    // World B: saved after 3 Ticks, its player standing 64 Chunks along x from the origin, so far
    // beyond render distance of anything World A had.
    let far = PlayerState {
        body: Body {
            grounded: true,
            ..Body::standing_at(Vec3::new(2_048.5, 5.0, 0.5))
        },
        yaw: 0.0,
        pitch: 0.0,
    };
    let far_chunk = ChunkPos::of(cell_of_feet(far.body.feet));
    open(&mut app, saved_world(far, 3));
    update_until(&mut app, "World B's player is released", player_released);

    // For a second of meshing, every Chunk in view and every Chunk drawn is World B's.
    update_for(&mut app, Duration::from_secs(1), |app| {
        for (pos, _) in view(app).chunks() {
            assert!(within_render_distance(pos, far_chunk), "{pos:?} is in view");
        }
        for mesh in chunk_meshes(app) {
            assert!(
                within_render_distance(mesh.pos, far_chunk),
                "{:?} is drawn",
                mesh.pos
            );
        }
    });
    assert!(!chunk_meshes(&mut app).is_empty(), "World B isn't drawn");
    assert_eq!(view(&app).cell(edited), None);
    // World B's simulation carries on from its own 3 Ticks, and knows nothing of World A's Edit.
    assert!(tick(&app) >= 3, "Tick {}", tick(&app));
    let captured = capture(&app);
    assert!(captured.checkpoint.tick_count >= 3);
    assert!(
        !captured
            .checkpoint
            .chunks
            .contains_key(&ChunkPos::of(edited))
    );

    // World B again, from its save: render distance loads around the same Chunk as last time,
    // which the Chunks above the Active Region need before the player is released.
    close(&mut app);
    open(&mut app, saved_world(far, 3));
    update_until(
        &mut app,
        "World B's player is released again",
        player_released,
    );
}

/// Every focus the app has had, at the end of each frame of a World.
#[derive(Resource, Default)]
struct FocusSeen(Vec<CellPos>);

#[test]
fn a_saved_flying_player_far_from_the_origin_carries_on_where_they_were() {
    let elements = ElementTable::from_ron(SHIPPED_ELEMENTS).expect("the shipped Elements");
    let settings: SimSettings = ron::from_str(SHIPPED_SETTINGS).expect("the shipped settings");
    let generation = Generation {
        seed: 7,
        elements: elements.clone(),
    };
    let generator = generation
        .generator()
        .expect("the shipped Elements suffice");
    // High over terrain 2 km from the origin, flying.
    let saved = PlayerState {
        body: Body {
            feet: Vec3::new(2_000.5, 120.25, -1_500.5),
            velocity: Vec3::new(12.0, 0.0, 0.0),
            mode: MoveMode::Fly,
            grounded: false,
        },
        yaw: 2.0,
        pitch: -0.4,
    };
    let focus = cell_of_feet(saved.body.feet);
    // A Cell of Granite in the open sky beside the player, which only the save holds: generated,
    // its Chunk has nothing solid to draw.
    let granite_pos = CellPos::new(2_003, 121, -1_499);
    let granite_chunk = ChunkPos::of(granite_pos);
    let generated = generator.generate(granite_chunk);
    assert!(
        generated
            .iter()
            .all(|cell| elements.get(cell.element).phase != Phase::Solid),
        "the test needs a Chunk of open sky"
    );
    let mut sim = Simulation::with_source(elements.clone(), Box::new(generator));
    sim.set_settings(settings.clone()).expect("valid settings");
    sim.focus_on(focus);
    let granite = elements.id_of("Granite").expect("Granite");
    sim.apply(Edit::Place {
        pos: granite_pos,
        cell: cell_of(granite, 293.15, &elements),
    });
    let save = WorldSave {
        capture: SimCapture {
            checkpoint: sim.checkpoint(),
            elements,
            settings,
            generation: Some(generation),
        },
        player: saved,
    };
    drop(sim);

    let mut app = session_app();
    app.init_resource::<FocusSeen>().add_systems(
        Last,
        |focus: Option<Res<SimFocus>>, mut seen: ResMut<FocusSeen>| {
            seen.0.extend(focus.map(|focus| focus.0));
        },
    );
    open(
        &mut app,
        WorldStart::load_game(save).expect("the save loads"),
    );

    // Held while the Chunks around them load, as saved.
    let (body, rotation, awaiting) = player(&mut app).expect("the player");
    assert_eq!(awaiting, Some(AwaitingSpawn::AsSaved));
    assert_eq!(body, saved.body);
    assert!(rotation.abs_diff_eq(saved.rotation(), 1e-6));
    update_until(&mut app, "the player is released", |app| {
        let (body, _, awaiting) = player(app).expect("the player");
        if awaiting.is_some() {
            assert_eq!(body, saved.body, "the player moved while held");
        }
        awaiting.is_none()
    });
    // Released once the Chunk they are in and all 26 around it had loaded.
    let centre = ChunkPos::of(focus);
    for x in -1..=1 {
        for y in -1..=1 {
            for z in -1..=1 {
                let pos = ChunkPos::new(centre.x + x, centre.y + y, centre.z + z);
                assert!(view(&app).chunk(pos).is_some(), "{pos:?} isn't loaded");
            }
        }
    }
    update_until(&mut app, "the Granite is drawn", |app| {
        chunk_meshes(app).contains(&ChunkMesh {
            pos: granite_chunk,
            layer: ChunkLayer::Solid,
        })
    });

    // Flying with no keys held, the body stays where it was saved, and still flies.
    let (body, rotation, _) = player(&mut app).expect("the player");
    assert_eq!(body.feet, saved.body.feet);
    assert_eq!(body.mode, MoveMode::Fly);
    let (yaw, pitch, _) = rotation.to_euler(EulerRot::YXZ);
    // Rounding through a quaternion and back.
    assert!((yaw - saved.yaw).abs() < 1e-5, "yaw {yaw}");
    assert!((pitch - saved.pitch).abs() < 1e-5, "pitch {pitch}");
    // The focus was the save's from the first frame on.
    let seen = &app.world().resource::<FocusSeen>().0;
    assert!(!seen.is_empty());
    assert!(seen.iter().all(|&seen| seen == focus), "{seen:?}");
    // Every Chunk drawn lies around the saved player, none around the origin.
    for mesh in chunk_meshes(&mut app) {
        assert!(
            within_render_distance(mesh.pos, ChunkPos::of(focus)),
            "{:?} is drawn",
            mesh.pos
        );
    }
}

/// Sets the settings file's `conduction_multiplier`, as a hot reload of the file would, once the
/// asset server has loaded it.
fn reload_settings(app: &mut App, conduction_multiplier: f32) {
    update_until(app, "the settings file loads", |app| {
        !app.world().resource::<Assets<SettingsFile>>().is_empty()
    });
    let mut files = app.world_mut().resource_mut::<Assets<SettingsFile>>();
    let id = files.ids().next().expect("the settings file");
    files.get_mut(id).expect("loaded").0.conduction_multiplier = conduction_multiplier;
}

fn outcomes(app: &App) -> &[ReplaceOutcome] {
    &app.world().resource::<Outcomes>().0
}

/// Everything a pause must hold still: the Tick count, the player's pose, and the simulation.
fn frozen_state(app: &mut App) -> (u64, Body, Quat, SimCapture) {
    let (body, rotation, _) = player(app).expect("the player");
    (tick(app), body, rotation, capture(app))
}

#[test]
fn a_pause_with_the_clock_running_holds_the_world_still_until_resumed() {
    let mut app = session_app();
    // Falling from high up, so the pose changes on every frame of play.
    open(
        &mut app,
        saved_world(falling_player(Vec3::new(0.5, 60.0, 0.5), 0.0), 0),
    );
    update_until(&mut app, "the player is released", player_released);
    update_until(&mut app, "the clock runs", |app| tick(app) >= 2);
    assert!(captured(&cursor(&app)));

    key(&mut app, KeyCode::Escape, ButtonState::Pressed);
    app.update();
    // The same frame Esc goes down, input stops being gameplay and the cursor shows.
    assert!(!app.world().resource::<InputPolicy>().gameplay);
    assert!(!captured(&cursor(&app)));
    assert!(matches!(play(&app), Some(Play::Pausing | Play::Paused)));
    key(&mut app, KeyCode::Escape, ButtonState::Released);
    update_until(&mut app, "the simulation acknowledges the pause", |app| {
        play(app) == Some(Play::Paused)
    });
    let paused = frozen_state(&mut app);

    // Neither the debug clock's Step, from its key or sent straight to the simulation, nor a hot
    // reload changes the paused World.
    tap(&mut app, KeyCode::KeyN);
    send(&app, SimCommand::Step);
    reload_settings(&mut app, 3.0);
    update_for(&mut app, Duration::from_secs(1), |_| {});
    assert_eq!(frozen_state(&mut app), paused);
    assert!(outcomes(&app).is_empty(), "{:?}", outcomes(&app));

    tap(&mut app, KeyCode::Escape);
    assert_eq!(play(&app), Some(Play::Running));
    assert!(captured(&cursor(&app)));
    // The clock runs again, the reload held back during the pause is taken up, and the player
    // falls on.
    update_until(&mut app, "the clock runs again", |app| tick(app) > paused.0);
    update_until(&mut app, "the held-back reload is answered", |app| {
        !outcomes(app).is_empty()
    });
    assert_eq!(outcomes(&app), [ReplaceOutcome::Settings(Ok(()))]);
    assert_eq!(capture(&app).settings.conduction_multiplier, 3.0);
    let (body, _, _) = player(&mut app).expect("the player");
    assert!(body.feet.y < paused.1.feet.y, "the player stopped falling");
}

#[test]
fn a_pause_with_the_clock_stopped_resumes_with_it_still_stopped() {
    let mut app = session_app();
    open(
        &mut app,
        saved_world(falling_player(Vec3::new(0.5, 60.0, 0.5), 0.0), 0),
    );
    update_until(&mut app, "the player is released", player_released);
    update_until(&mut app, "the clock runs", |app| tick(app) >= 1);

    // P stops the debug clock. A Tick may be under way; 0.6 s is three Tick intervals.
    tap(&mut app, KeyCode::KeyP);
    assert!(app.world().resource::<SimClock>().paused);
    update_for(&mut app, Duration::from_millis(600), |_| {});
    let stopped_at = tick(&app);

    tap(&mut app, KeyCode::Escape);
    update_until(&mut app, "the simulation acknowledges the pause", |app| {
        play(app) == Some(Play::Paused)
    });
    let paused = frozen_state(&mut app);
    // While paused, the debug clock's keys do nothing, and a Step sent straight to the stopped
    // clock runs no Tick.
    tap(&mut app, KeyCode::KeyN);
    tap(&mut app, KeyCode::KeyP);
    send(&app, SimCommand::Step);
    update_for(&mut app, Duration::from_millis(600), |_| {});
    assert_eq!(frozen_state(&mut app), paused);
    assert!(app.world().resource::<SimClock>().paused);

    tap(&mut app, KeyCode::Escape);
    assert_eq!(play(&app), Some(Play::Running));
    // The debug clock is still stopped: five Tick intervals pass without a Tick, and N runs
    // exactly one.
    assert!(app.world().resource::<SimClock>().paused);
    update_for(&mut app, Duration::from_secs(1), |_| {});
    assert_eq!(tick(&app), stopped_at);
    tap(&mut app, KeyCode::KeyN);
    update_until(&mut app, "the Step's Tick", |app| tick(app) > stopped_at);
    update_for(&mut app, Duration::from_millis(600), |_| {});
    assert_eq!(tick(&app), stopped_at + 1);
    // The player fell on throughout: physics follows the pause, not the debug clock.
    let (body, _, _) = player(&mut app).expect("the player");
    assert!(body.feet.y < paused.1.feet.y, "the player stopped falling");
}

#[test]
fn holding_alt_frees_the_pointer_without_using_a_tool_while_physics_carries_on() {
    let (_, rock, _) = elements();
    let mut app = session_app();
    // 1.5 m above the ground, falling, looking straight down at it, with Dig in hand.
    let player_start = falling_player(Vec3::new(0.5, 6.5, 0.5), -1.55);
    open(&mut app, saved_world(player_start, 0));
    update_until(&mut app, "the player is released", player_released);
    assert!(app.world().resource::<InputPolicy>().gameplay);
    assert!(captured(&cursor(&app)));

    key(&mut app, KeyCode::AltLeft, ButtonState::Pressed);
    app.update();
    assert!(!app.world().resource::<InputPolicy>().gameplay);
    assert!(!captured(&cursor(&app)), "the cursor is still captured");
    let (before, _, _) = player(&mut app).expect("the player");
    // Clicks while Alt is held are for the debug panel.
    mouse(&mut app, MouseButton::Left, ButtonState::Pressed);
    update_until(&mut app, "the player lands", |app| {
        player(app).expect("the player").0.grounded
    });
    let (landed, _, _) = player(&mut app).expect("the player");
    assert!(
        landed.feet.y < before.feet.y,
        "physics stopped while Alt was held"
    );
    let target = app
        .world()
        .resource::<Aim>()
        .expect("the ground below is in reach")
        .cell;
    let target = CellPos::new(target.x, target.y, target.z);
    assert_eq!(view(&app).cell(target), Some(rock_cell(rock)));

    // Releasing Alt with the button still down captures the cursor again, but the click from
    // while it was free isn't a click on the world.
    key(&mut app, KeyCode::AltLeft, ButtonState::Released);
    app.update();
    assert!(app.world().resource::<InputPolicy>().gameplay);
    assert!(captured(&cursor(&app)));
    assert!(
        !app.world()
            .resource::<ButtonInput<MouseButton>>()
            .pressed(MouseButton::Left)
    );
    // Long enough for an Edit to come back: three Tick intervals.
    update_for(&mut app, Duration::from_millis(600), |_| {});
    assert_eq!(
        view(&app).cell(target),
        Some(rock_cell(rock)),
        "a tool was used"
    );

    // A fresh click on the world digs.
    mouse(&mut app, MouseButton::Left, ButtonState::Released);
    app.update();
    mouse(&mut app, MouseButton::Left, ButtonState::Pressed);
    update_until(&mut app, "the click digs", |app| {
        view(app).cell(target) == Some(Cell::VACUUM)
    });
}

#[test]
fn a_loaded_world_keeps_its_saved_rules_until_a_file_changes_during_play() {
    let mut app = session_app();
    // The settings file changes with no World open, just before one is loaded.
    reload_settings(&mut app, 3.0);
    app.update();
    // Saved with the default settings, unlike the shipped file's.
    open(
        &mut app,
        saved_world(falling_player(Vec3::new(0.5, 5.0, 0.5), 0.0), 0),
    );
    update_for(&mut app, Duration::from_millis(600), |_| {});
    assert!(outcomes(&app).is_empty(), "{:?}", outcomes(&app));
    assert_eq!(capture(&app).settings, SimSettings::default());

    // A change during play is taken up.
    reload_settings(&mut app, 4.0);
    update_until(&mut app, "the reload is answered", |app| {
        !outcomes(app).is_empty()
    });
    assert_eq!(outcomes(&app), [ReplaceOutcome::Settings(Ok(()))]);
    assert_eq!(capture(&app).settings.conduction_multiplier, 4.0);
}

/// Step 21 (`docs/plans/21-movement-controls.md`): in the game, two presses of Jump switch flying
/// on and off, and F no longer does. A first press is forgotten over a pause, or while Alt is
/// held. In flight Jump rises and Shift flies down. On foot, Ctrl widens the view while running,
/// without moving the camera from the body's eyes, and the view narrows again once running stops.
#[test]
fn jump_pressed_twice_switches_flying_and_running_widens_the_view() {
    let mut app = session_app();
    open(&mut app, new_world(None));
    update_until(&mut app, "the player stands on the ground", |app| {
        player(app).is_some_and(|(body, _, awaiting)| awaiting.is_none() && body.grounded)
    });
    let body = |app: &mut App| player(app).expect("the player").0;
    let clock = |app: &mut App, ms: u64| {
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(ms)));
    };
    // Frames of 20 ms on the app's clocks.
    clock(&mut app, 20);

    tap(&mut app, KeyCode::KeyF);
    tap(&mut app, KeyCode::KeyF);
    assert_eq!(body(&mut app).mode, MoveMode::Walk, "F no longer flies");

    // Two taps of Space, 40 ms apart: the first jumps, and the second flies.
    tap(&mut app, KeyCode::Space);
    tap(&mut app, KeyCode::Space);
    assert_eq!(body(&mut app).mode, MoveMode::Fly);

    // Space rises, and Shift flies down: 12 m/s for 10 frames, then for 5.
    let hovering = body(&mut app).feet.y;
    for (held, frames, rise) in [(KeyCode::Space, 10, 2.4), (KeyCode::ShiftLeft, 5, -1.2)] {
        let from = body(&mut app).feet.y;
        key(&mut app, held, ButtonState::Pressed);
        for _ in 0..frames {
            app.update();
        }
        key(&mut app, held, ButtonState::Released);
        clock(&mut app, 0);
        app.update();
        clock(&mut app, 20);
        let risen = body(&mut app).feet.y - from;
        assert!((risen - rise).abs() < 1e-3, "{held:?}: {risen}");
    }
    assert!(body(&mut app).feet.y > hovering);
    // Long enough that the hold of Shift's companion press of Space is forgotten.
    clock(&mut app, 400);
    app.update();

    // While the clocks stand still, every press is within 300 ms of the last. A pause between
    // two forgets the first, and so does holding Alt.
    clock(&mut app, 0);
    tap(&mut app, KeyCode::Space);
    tap(&mut app, KeyCode::Escape);
    update_until(&mut app, "the World pauses", |app| play(app) == Some(Play::Paused));
    tap(&mut app, KeyCode::Escape);
    assert_eq!(play(&app), Some(Play::Running));
    tap(&mut app, KeyCode::Space);
    assert_eq!(body(&mut app).mode, MoveMode::Fly, "the pause forgot the first press");
    clock(&mut app, 400);
    app.update();
    clock(&mut app, 0);
    tap(&mut app, KeyCode::Space);
    tap(&mut app, KeyCode::AltLeft);
    tap(&mut app, KeyCode::Space);
    assert_eq!(body(&mut app).mode, MoveMode::Fly, "Alt forgot the first press");

    // Two more presses land, and the body falls back to the ground.
    clock(&mut app, 400);
    app.update();
    clock(&mut app, 20);
    tap(&mut app, KeyCode::Space);
    tap(&mut app, KeyCode::Space);
    assert_eq!(body(&mut app).mode, MoveMode::Walk);
    for _ in 0..100 {
        app.update();
    }
    assert!(body(&mut app).grounded);

    // Running: Ctrl and W for 0.5 s widen the view to 76°, then 0.3 s after letting go of W it
    // is back to 70°. The camera stays at the body's eyes throughout.
    let view = |app: &mut App| {
        let (player, transform, projection) = app
            .world_mut()
            .query::<(&Player, &Transform, &Projection)>()
            .single(app.world())
            .expect("the player's camera");
        assert_eq!(transform.translation, player.eyes(), "the camera left the eyes");
        match projection {
            Projection::Perspective(perspective) => perspective.fov.to_degrees(),
            other => panic!("{other:?}"),
        }
    };
    assert!((view(&mut app) - 70.0).abs() < 1e-3);
    key(&mut app, KeyCode::ControlLeft, ButtonState::Pressed);
    key(&mut app, KeyCode::KeyW, ButtonState::Pressed);
    for _ in 0..25 {
        app.update();
        view(&mut app);
    }
    assert!((view(&mut app) - 76.0).abs() < 1e-3);
    key(&mut app, KeyCode::KeyW, ButtonState::Released);
    for _ in 0..15 {
        app.update();
        view(&mut app);
    }
    assert!((view(&mut app) - 70.0).abs() < 1e-3);
}
