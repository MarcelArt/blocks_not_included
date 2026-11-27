//! `SimBridgePlugin` in a headless `App`, with a tiny Element table and a world of Vacuum. Each
//! test opens a World through the session, without a player.
//!
//! The simulation runs on a thread of its own, at 5 Ticks per second, so these tests wait in real
//! time, with generous timeouts.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use blocks_not_included::session::{OpenWorld, SessionPlugin, WorldStart};
use blocks_not_included::sim_bridge::{
    Elements, ReplaceOutcome, SimBridge, SimBridgePlugin, SimCapture, SimCommand, SimFocus,
    SimReport, SimStart,
};
use blocks_not_included::world::WorldView;
use sim::{
    CHUNK_SIZE, Cell, CellPos, CheckpointError, ChunkContents, ChunkPos, ChunkSource, Edit,
    Element, ElementError, ElementId, ElementTable, Phase, SimSettings,
};

/// Far longer than anything here should take, so a slow machine doesn't fail a test.
const TIMEOUT: Duration = Duration::from_secs(10);

/// Every Chunk is all Vacuum.
struct VacuumSource;

impl ChunkSource for VacuumSource {
    fn generate(&self, _: ChunkPos) -> Vec<Cell> {
        vec![Cell::VACUUM; CHUNK_SIZE.pow(3)]
    }
}

/// A world of Vacuum that records when it is dropped. The simulation owns its source, so once
/// it is dropped, the simulation is gone too.
struct DropRecordingSource {
    dropped: Arc<AtomicBool>,
}

impl ChunkSource for DropRecordingSource {
    fn generate(&self, chunk: ChunkPos) -> Vec<Cell> {
        VacuumSource.generate(chunk)
    }
}

impl Drop for DropRecordingSource {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

/// Ground of TestRock up to `y = 4`, under Vacuum. TestRock is the first Element added, so it
/// has the first id after Vacuum's.
struct FlatGroundSource {
    rock: ElementId,
}

impl ChunkSource for FlatGroundSource {
    fn generate(&self, chunk: ChunkPos) -> Vec<Cell> {
        let mut cells = VacuumSource.generate(chunk);
        for (i, cell) in cells.iter_mut().enumerate() {
            // Cells are listed x fastest, then z, then y.
            let y = chunk.origin().y + (i / (CHUNK_SIZE * CHUNK_SIZE)) as i32;
            if y <= 4 {
                *cell = rock_cell(self.rock);
            }
        }
        cells
    }
}

/// Vacuum, except one TestRock Cell at `(1, 1, 1)` from the origin of each Chunk listed.
struct MarkedChunksSource {
    rock: ElementId,
    marked: Vec<ChunkPos>,
}

impl ChunkSource for MarkedChunksSource {
    fn generate(&self, chunk: ChunkPos) -> Vec<Cell> {
        let mut cells = VacuumSource.generate(chunk);
        if self.marked.contains(&chunk) {
            // (1, 1, 1) is at index (1 × CHUNK_SIZE + 1) × CHUNK_SIZE + 1.
            cells[(CHUNK_SIZE + 1) * CHUNK_SIZE + 1] = rock_cell(self.rock);
        }
        cells
    }
}

fn marked_cell(chunk: ChunkPos) -> CellPos {
    let origin = chunk.origin();
    CellPos::new(origin.x + 1, origin.y + 1, origin.z + 1)
}

fn rock_cell(rock: ElementId) -> Cell {
    Cell {
        element: rock,
        mass_grams: 1_000,
        temperature_k: 300.0,
    }
}

/// A solid that conducts no heat, so a Cell of it never changes.
fn test_rock() -> Element {
    Element::new("TestRock").density(1_000.0).specific_heat(1.0)
}

fn tiny_elements() -> (ElementTable, ElementId) {
    let mut elements = ElementTable::new();
    let rock = elements.add(test_rock());
    (elements, rock)
}

/// An app with a World open from `start`, before its first frame.
fn app_starting(start: SimStart) -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin, SessionPlugin, SimBridgePlugin));
    app.world_mut().commands().queue(OpenWorld(WorldStart {
        sim: start,
        player: None,
    }));
    app.world_mut().flush();
    // Startup, during which the World opens.
    app.update();
    app
}

fn app_with(source: Box<dyn ChunkSource>) -> App {
    let (elements, _) = tiny_elements();
    app_starting(SimStart::fresh(elements, SimSettings::default(), source))
}

fn send(app: &App, command: SimCommand) {
    app.world().resource::<SimBridge>().send(command);
}

/// The Tick count of the latest report from the simulation.
fn tick(app: &App) -> u64 {
    app.world().resource::<SimReport>().tick
}

/// Runs frames for `duration`.
fn update_for(app: &mut App, duration: Duration) {
    let start = Instant::now();
    while start.elapsed() < duration {
        app.update();
        thread::sleep(Duration::from_millis(5));
    }
}

/// Runs frames until `done` holds, failing the test after [`TIMEOUT`].
fn update_until(app: &mut App, what: &str, done: impl Fn(&App) -> bool) {
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

#[test]
fn a_placed_cell_shows_up_in_the_world_view_within_a_few_ticks() {
    let (_, rock) = tiny_elements();
    let mut app = app_with(Box::new(VacuumSource));
    update_until(&mut app, "the clock runs", |app| tick(app) >= 1);

    let pos = CellPos::new(3, 5, -7);
    let placed = Cell {
        element: rock,
        mass_grams: 1_000,
        temperature_k: 300.0,
    };
    let sent_at = tick(&app);
    send(&app, SimCommand::Edit(Edit::Place { pos, cell: placed }));
    update_until(&mut app, "the placed Cell shows up", |app| {
        app.world().resource::<WorldView>().cell(pos) == Some(placed)
    });

    // Edits are applied between Ticks, and the Chunk is sent straight after; the margin is for a
    // slow machine.
    assert!(
        tick(&app) <= sent_at + 3,
        "took from Tick {sent_at} to Tick {}",
        tick(&app)
    );
    // The rest of the Chunk is still Vacuum.
    let view = app.world().resource::<WorldView>();
    assert_eq!(view.cell(CellPos::new(4, 5, -7)), Some(Cell::VACUUM));
}

#[test]
fn pause_stops_the_tick_count_and_step_advances_it_by_exactly_one() {
    let mut app = app_with(Box::new(VacuumSource));
    update_until(&mut app, "the clock runs", |app| tick(app) >= 2);

    send(&app, SimCommand::Pause);
    // A Tick may be under way as Pause arrives; 0.6 s is three Tick intervals, ample for its
    // report to come through.
    update_for(&mut app, Duration::from_millis(600));
    let paused_at = tick(&app);
    // Five Tick intervals.
    update_for(&mut app, Duration::from_secs(1));
    assert_eq!(tick(&app), paused_at, "the clock ran while paused");

    send(&app, SimCommand::Step);
    update_until(&mut app, "the Step's Tick", |app| tick(app) > paused_at);
    update_for(&mut app, Duration::from_secs(1));
    assert_eq!(tick(&app), paused_at + 1, "Step ran more than one Tick");

    send(&app, SimCommand::Resume);
    update_until(&mut app, "the clock runs again", |app| {
        tick(app) >= paused_at + 3
    });
}

/// Every [`ReplaceOutcome`] the app has received, oldest first.
#[derive(Resource, Default)]
struct Outcomes(Vec<ReplaceOutcome>);

/// `app_with`, recording every [`ReplaceOutcome`] in [`Outcomes`].
fn app_recording_outcomes(source: Box<dyn ChunkSource>) -> App {
    let mut app = app_with(source);
    app.init_resource::<Outcomes>().add_systems(
        Update,
        |mut outcomes: MessageReader<ReplaceOutcome>, mut seen: ResMut<Outcomes>| {
            seen.0.extend(outcomes.read().cloned());
        },
    );
    app
}

/// Sends `command`, then runs frames until the simulation's answer arrives, and returns it.
fn answer_to(app: &mut App, command: SimCommand) -> ReplaceOutcome {
    let before = app.world().resource::<Outcomes>().0.len();
    send(app, command);
    update_until(app, "the simulation answers", |app| {
        app.world().resource::<Outcomes>().0.len() > before
    });
    app.world().resource::<Outcomes>().0[before].clone()
}

#[test]
fn an_accepted_element_table_replaces_elements_and_a_rejected_one_leaves_it() {
    let mut app = app_recording_outcomes(Box::new(VacuumSource));
    let elements = |app: &App| app.world().resource::<Elements>().0.clone();

    // Renaming TestRock is rejected: Cells may hold it.
    let mut renamed = ElementTable::new();
    renamed.add(Element::new("Basalt").density(1_000.0).specific_heat(1.0));
    let rejected = answer_to(&mut app, SimCommand::ReplaceElements(renamed));

    assert_eq!(
        rejected,
        ReplaceOutcome::Elements(Err(ElementError::MissingElement {
            element: "TestRock".into()
        }))
    );
    assert!(elements(&app).id_of("TestRock").is_some());
    assert!(elements(&app).id_of("Basalt").is_none());

    // Appending an Element is accepted.
    let (mut appended, rock) = tiny_elements();
    let sand = appended.add(Element::new("TestSand").density(1_500.0).specific_heat(1.0));
    let accepted = answer_to(&mut app, SimCommand::ReplaceElements(appended));

    assert_eq!(accepted, ReplaceOutcome::Elements(Ok(())));
    assert_eq!(elements(&app).id_of("TestRock"), Some(rock));
    assert_eq!(elements(&app).id_of("TestSand"), Some(sand));
}

#[test]
fn replaced_settings_are_answered_and_invalid_ones_are_rejected() {
    let mut app = app_recording_outcomes(Box::new(VacuumSource));

    // Above 0.5, a pair of gas Cells would overshoot.
    let overshooting = SimSettings {
        gas_evening_out_fraction: 0.9,
        ..SimSettings::default()
    };
    let rejected = answer_to(&mut app, SimCommand::ReplaceSettings(overshooting));
    let faster_heat = SimSettings {
        conduction_multiplier: 10.0,
        ..SimSettings::default()
    };
    let accepted = answer_to(&mut app, SimCommand::ReplaceSettings(faster_heat));

    let ReplaceOutcome::Settings(Err(error)) = rejected else {
        panic!("expected a rejection, got {rejected:?}");
    };
    assert_eq!(error.setting, "gas_evening_out_fraction");
    assert_eq!(accepted, ReplaceOutcome::Settings(Ok(())));
}

#[test]
fn dropping_the_app_shuts_the_simulation_thread_down() {
    let dropped = Arc::new(AtomicBool::new(false));
    let (finished, finishing) = mpsc::channel();
    // The App lives on a thread of its own, so the test fails rather than hangs if dropping it
    // never returns.
    let source = DropRecordingSource {
        dropped: Arc::clone(&dropped),
    };
    let dropped_by_app = Arc::clone(&dropped);
    thread::spawn(move || {
        let mut app = app_with(Box::new(source));
        update_until(&mut app, "the clock runs", |app| tick(app) >= 1);
        drop(app);
        finished
            .send(dropped_by_app.load(Ordering::SeqCst))
            .unwrap();
    });
    let simulation_gone = finishing
        .recv_timeout(TIMEOUT)
        .expect("dropping the App never returned");
    assert!(
        simulation_gone,
        "the simulation thread was still running after the App was dropped"
    );
}

#[test]
fn the_focus_starts_just_above_the_surface_at_the_origin() {
    let (_, rock) = tiny_elements();
    let app = app_with(Box::new(FlatGroundSource { rock }));
    // The ground's top Cell is at y = 4.
    assert_eq!(app.world().resource::<SimFocus>().0, CellPos::new(0, 5, 0));
}

#[test]
fn chunks_leave_the_world_view_beyond_render_distance_and_arrive_within_it() {
    let (_, rock) = tiny_elements();
    // Render distance reaches 6 Chunks either way along x, so with the focus at the origin, the
    // Chunk at x = 10 is beyond it; with the focus in the Chunk at x = 10, the origin's is.
    let (near, far) = (ChunkPos::new(0, 0, 0), ChunkPos::new(10, 0, 0));
    let mut app = app_with(Box::new(MarkedChunksSource {
        rock,
        marked: vec![near, far],
    }));
    update_until(&mut app, "the Chunk at the origin arrives", |app| {
        let view = app.world().resource::<WorldView>();
        view.changed().contains(&near) && view.cell(marked_cell(near)) == Some(rock_cell(rock))
    });
    assert!(app.world().resource::<WorldView>().chunk(far).is_none());

    app.world_mut().resource_mut::<SimFocus>().0 = marked_cell(far);
    update_until(&mut app, "the Chunk at the origin leaves", |app| {
        app.world()
            .resource::<WorldView>()
            .removed()
            .contains(&near)
    });
    assert!(app.world().resource::<WorldView>().chunk(near).is_none());
    update_until(&mut app, "the Chunk at x = 10 arrives", |app| {
        app.world().resource::<WorldView>().cell(marked_cell(far)) == Some(rock_cell(rock))
    });
}

#[test]
fn exiting_the_app_shuts_the_simulation_thread_down() {
    let dropped = Arc::new(AtomicBool::new(false));
    let mut app = app_with(Box::new(DropRecordingSource {
        dropped: Arc::clone(&dropped),
    }));
    update_until(&mut app, "the clock runs", |app| tick(app) >= 1);

    app.world_mut().write_message(AppExit::Success);
    // The frame that sees the AppExit waits for the thread to end, so one is enough.
    app.update();
    assert!(
        dropped.load(Ordering::SeqCst),
        "the simulation thread was still running after the app exited"
    );
}

/// The Active Region as the world view last heard it from the simulation, as its first and last
/// Chunks.
fn active_region(app: &App) -> Option<(ChunkPos, ChunkPos)> {
    app.world()
        .resource::<WorldView>()
        .active_region()
        .map(|region| (region.min(), region.max()))
}

#[test]
fn the_world_view_holds_the_active_region_and_follows_it_as_the_focus_moves() {
    let mut app = app_with(Box::new(VacuumSource));

    // The default Active Region is 4 × 2 × 4 Chunks, best centred on the focus at the origin.
    // Along x and z, Chunks −2 to 1 span Cells −64 to 63, centred on 0; along y, Chunks −1 to 0
    // span Cells −32 to 31.
    update_until(&mut app, "the Active Region arrives", |app| {
        active_region(app) == Some((ChunkPos::new(-2, -1, -2), ChunkPos::new(1, 0, 1)))
    });

    // Along x, Chunks 29 to 32 span Cells 928 to 1055, centred on 991.5: nearer 1000 than the
    // 1023.5 of Chunks 30 to 33.
    app.world_mut().resource_mut::<SimFocus>().0 = CellPos::new(1_000, 0, 0);
    update_until(&mut app, "the Active Region follows the focus", |app| {
        active_region(app) == Some((ChunkPos::new(29, -1, -2), ChunkPos::new(32, 0, 1)))
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

/// The Cell at `pos` as `capture` holds it: `None` unless its Chunk holds Cells.
fn captured_cell(capture: &SimCapture, pos: CellPos) -> Option<Cell> {
    let chunk = ChunkPos::of(pos);
    let ChunkContents::Cells(cells) = &capture.checkpoint.chunks.get(&chunk)?.contents else {
        return None;
    };
    let origin = chunk.origin();
    let (x, y, z) = (
        (pos.x - origin.x) as usize,
        (pos.y - origin.y) as usize,
        (pos.z - origin.z) as usize,
    );
    // Cells are listed x fastest, then z, then y.
    Some(cells[(y * CHUNK_SIZE + z) * CHUNK_SIZE + x])
}

#[test]
fn a_capture_includes_every_edit_sent_before_it_and_no_later_one() {
    let (_, rock) = tiny_elements();
    let mut app = app_with(Box::new(VacuumSource));
    // Two Cells of one Chunk.
    let (earlier, later) = (CellPos::new(3, 5, -7), CellPos::new(4, 5, -7));
    let placed = rock_cell(rock);

    send(
        &app,
        SimCommand::Edit(Edit::Place {
            pos: earlier,
            cell: placed,
        }),
    );
    let answer = app.world().resource::<SimBridge>().capture();
    send(
        &app,
        SimCommand::Edit(Edit::Place {
            pos: later,
            cell: placed,
        }),
    );
    let captured = answer.recv_timeout(TIMEOUT).expect("the capture arrives");
    update_until(&mut app, "the later Edit shows up", |app| {
        app.world().resource::<WorldView>().cell(later) == Some(placed)
    });

    // The capture is the game's own: the later Edit, applied since, doesn't reach it.
    assert_eq!(captured_cell(&captured, earlier), Some(placed));
    assert_eq!(captured_cell(&captured, later), Some(Cell::VACUUM));
}

/// TestRock, then a TestGas that conducts no heat.
fn rock_and_gas() -> (ElementTable, ElementId) {
    let (mut elements, _) = tiny_elements();
    let gas = elements.add(
        Element::new("TestGas")
            .phase(Phase::Gas)
            .density(1.0)
            .specific_heat(1.0),
    );
    (elements, gas)
}

/// `app`, with the clock stopped before any Tick.
fn paused_app(app: App) -> App {
    // The first Tick is due a whole Tick interval after the thread starts.
    send(&app, SimCommand::Pause);
    app
}

/// Runs one Tick with the clock stopped, and waits for its report.
fn step(app: &mut App) {
    let before = tick(app);
    send(app, SimCommand::Step);
    update_until(app, "the Step's Tick", |app| tick(app) > before);
}

#[test]
fn a_restored_simulation_carries_on_through_the_bridge_as_the_captured_one_does() {
    let (elements, gas) = rock_and_gas();
    let mut played = paused_app(app_starting(SimStart::fresh(
        elements,
        SimSettings::default(),
        Box::new(VacuumSource),
    )));
    // A release of gas, spreading into Vacuum in ways the Tick count breaks ties for.
    let release = CellPos::new(3, 3, 3);
    let gas_cell = Cell {
        element: gas,
        mass_grams: 100_000,
        temperature_k: 300.0,
    };
    send(
        &played,
        SimCommand::Edit(Edit::Place {
            pos: release,
            cell: gas_cell,
        }),
    );
    for _ in 0..3 {
        step(&mut played);
    }
    let captured = capture(&played);
    assert_eq!(captured.checkpoint.tick_count, 3);

    let restored = SimStart::restored(captured, Box::new(VacuumSource)).expect("restored");
    let mut resumed = paused_app(app_starting(restored));
    step(&mut played);
    step(&mut resumed);

    let report = |app: &App| *app.world().resource::<SimReport>();
    assert_eq!(report(&resumed).stats, report(&played).stats);
    assert_eq!(report(&resumed).tick, 4);
    let chunk = ChunkPos::of(release);
    update_until(&mut resumed, "the Tick's Cells arrive", |app| {
        let view = app.world().resource::<WorldView>();
        view.chunk(chunk).is_some() && view.cell(release) != Some(gas_cell)
    });
    let cells = |app: &App| app.world().resource::<WorldView>().chunk(chunk).cloned();
    assert!(cells(&resumed) == cells(&played), "the Cells differ");
}

#[test]
fn a_capture_holds_the_last_accepted_rules_and_the_generators_startup_elements() {
    let startup = ElementTable::from_ron(include_str!("../assets/elements.ron"))
        .expect("the shipped Elements are valid");
    let granite = startup.id_of("Granite").unwrap();
    let start = SimStart::generated(7, startup.clone(), SimSettings::default())
        .expect("the shipped Elements are enough");
    let mut app = app_starting(start);
    app.init_resource::<Outcomes>().add_systems(
        Update,
        |mut outcomes: MessageReader<ReplaceOutcome>, mut seen: ResMut<Outcomes>| {
            seen.0.extend(outcomes.read().cloned());
        },
    );
    // Granite conducts 100 times better, then a rename is rejected.
    let mut conductive = ElementTable::new();
    let mut renamed = ElementTable::new();
    for (id, element) in startup.iter().skip(1) {
        let mut element = element.clone();
        if id == granite {
            element.conductivity *= 100.0;
        }
        conductive.add(element.clone());
        if id == granite {
            element.name = "Basalt".into();
        }
        renamed.add(element);
    }
    let conductive_k = conductive.get(granite).conductivity;
    let accepted = SimSettings {
        conduction_multiplier: 10.0,
        ..SimSettings::default()
    };
    let overshooting = SimSettings {
        gas_evening_out_fraction: 0.9,
        ..SimSettings::default()
    };
    for (command, accept) in [
        (SimCommand::ReplaceElements(conductive), true),
        (SimCommand::ReplaceElements(renamed), false),
        (SimCommand::ReplaceSettings(accepted.clone()), true),
        (SimCommand::ReplaceSettings(overshooting), false),
    ] {
        let outcome = answer_to(&mut app, command);
        let accepted = matches!(
            outcome,
            ReplaceOutcome::Elements(Ok(())) | ReplaceOutcome::Settings(Ok(()))
        );
        assert_eq!(accepted, accept, "{outcome:?}");
    }

    let captured = capture(&app);

    let check = |captured: &SimCapture| {
        assert_eq!(captured.elements.get(granite).conductivity, conductive_k);
        assert_eq!(captured.settings, accepted);
        let generation = captured.generation.as_ref().expect("a generated world");
        assert_eq!(generation.seed, 7);
        assert_eq!(
            generation.elements.get(granite).conductivity,
            startup.get(granite).conductivity
        );
    };
    check(&captured);
    // Restored with the generator rebuilt from its own inputs, the rules and the generator's
    // inputs carry over.
    let generator = captured
        .generation
        .as_ref()
        .unwrap()
        .generator()
        .expect("the startup Elements are enough");
    let restored = SimStart::restored(captured, Box::new(generator)).expect("restored");
    let resumed = app_starting(restored);
    check(&capture(&resumed));
    assert_eq!(
        resumed
            .world()
            .resource::<Elements>()
            .get(granite)
            .conductivity,
        conductive_k
    );
}

#[test]
fn a_malformed_capture_is_rejected_and_no_simulation_starts() {
    let (_, rock) = tiny_elements();
    let app = app_with(Box::new(VacuumSource));
    send(&app, SimCommand::Pause);
    let pos = CellPos::new(3, 5, -7);
    send(
        &app,
        SimCommand::Edit(Edit::Place {
            pos,
            cell: rock_cell(rock),
        }),
    );
    let mut captured = capture(&app);
    // A Cell of rock with no Mass.
    let ChunkContents::Cells(cells) = &mut captured
        .checkpoint
        .chunks
        .get_mut(&ChunkPos::of(pos))
        .expect("listed")
        .contents
    else {
        panic!("no Cells");
    };
    let placed = cells.iter_mut().find(|cell| cell.element == rock).unwrap();
    placed.mass_grams = 0;
    let dropped = Arc::new(AtomicBool::new(false));
    let source = DropRecordingSource {
        dropped: Arc::clone(&dropped),
    };

    let restored = SimStart::restored(captured, Box::new(source));

    assert!(
        matches!(restored, Err(CheckpointError::InvalidMass { .. })),
        "{:?}",
        restored.err()
    );
    // The source went with the rejected checkpoint: no simulation holds it.
    assert!(dropped.load(Ordering::SeqCst));
}

#[test]
fn a_freeze_is_answered_after_earlier_edits_and_holds_off_ticks_and_steps_until_unfrozen() {
    let (_, rock) = tiny_elements();
    let mut app = app_with(Box::new(VacuumSource));
    update_until(&mut app, "the clock runs", |app| tick(app) >= 1);

    let pos = CellPos::new(3, 5, -7);
    send(
        &app,
        SimCommand::Edit(Edit::Place {
            pos,
            cell: rock_cell(rock),
        }),
    );
    app.world_mut().resource_mut::<SimBridge>().freeze();
    let frozen_at = |app: &App| app.world().resource::<SimBridge>().frozen_at();
    update_until(&mut app, "the freeze is answered", |app| {
        frozen_at(app).is_some()
    });

    // The Edit sent before the freeze has reached the world view by the time the answer does,
    // which came between two Ticks: the report of the last Tick before it arrived first.
    assert_eq!(
        app.world().resource::<WorldView>().cell(pos),
        Some(rock_cell(rock))
    );
    let frozen_tick = tick(&app);
    assert_eq!(frozen_at(&app), Some(frozen_tick));
    // Frozen with the clock running, no Tick runs: five Tick intervals pass.
    update_for(&mut app, Duration::from_secs(1));
    assert_eq!(tick(&app), frozen_tick);
    // Nor with the clock stopped does a Step run one.
    send(&app, SimCommand::Pause);
    send(&app, SimCommand::Step);
    update_for(&mut app, Duration::from_millis(600));
    assert_eq!(tick(&app), frozen_tick);

    // Unfrozen, the clock is as it was left: stopped, until a Step or a Resume.
    app.world_mut().resource_mut::<SimBridge>().unfreeze();
    assert_eq!(frozen_at(&app), None);
    update_for(&mut app, Duration::from_millis(600));
    assert_eq!(tick(&app), frozen_tick);
    step(&mut app);
    assert_eq!(tick(&app), frozen_tick + 1);
    send(&app, SimCommand::Resume);
    update_until(&mut app, "the clock runs again", |app| {
        tick(app) >= frozen_tick + 3
    });
}

#[test]
fn a_restored_simulation_reports_its_tick_count_from_the_start() {
    let mut played = paused_app(app_with(Box::new(VacuumSource)));
    for _ in 0..3 {
        step(&mut played);
    }
    let captured = capture(&played);

    let restored = SimStart::restored(captured, Box::new(VacuumSource)).expect("restored");
    let resumed = paused_app(app_starting(restored));

    // Before the simulation's own first report.
    assert_eq!(tick(&resumed), 3);
}
