//! `SimBridgePlugin` in a headless `App`, with a tiny Element table and a world of Vacuum.
//!
//! The simulation runs on a thread of its own, at 5 Ticks per second, so these tests wait in real
//! time, with generous timeouts.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use bevy::prelude::*;
use blocks_not_included::sim_bridge::{
    SimBridge, SimBridgePlugin, SimCommand, SimFocus, SimReport,
};
use blocks_not_included::world::WorldView;
use sim::{
    CHUNK_SIZE, Cell, CellPos, ChunkPos, ChunkSource, Edit, Element, ElementId, ElementTable,
    SimSettings,
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

fn app_with(source: Box<dyn ChunkSource>) -> App {
    let (elements, _) = tiny_elements();
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        SimBridgePlugin::new(elements, SimSettings::default(), source),
    ));
    app
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
