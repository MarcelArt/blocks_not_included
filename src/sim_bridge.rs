//! The bridge between the game and the simulation, which runs on a thread of its own (ADR 0003).
//!
//! The game sends the simulation thread [`SimCommand`]s. The thread sends back the Cells of every
//! Chunk within render distance that loads or changes, which the game keeps in [`WorldView`], and
//! a [`SimReport`] after every Tick.
//!
//! # The simulation thread
//!
//! It runs a Tick every [`TICK_INTERVAL`], against a deadline, and handles every pending command
//! before each one. A Tick that overruns is followed by the next one at once, but missed Ticks
//! are never made up.
//!
//! Render distance holds about a thousand Chunks, which take seconds to generate. So the thread
//! loads them between Ticks, one at a time and nearest the focus first, rather than holding up
//! the clock.

use std::collections::{HashSet, VecDeque};
use std::fs;
use std::ops::ControlFlow;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use bevy::asset::io::file::FileAssetReader;
use bevy::prelude::*;
// Not `std::sync::mpsc`: crossbeam's `Receiver` is `Sync`, so the game's end of a channel can sit
// in a `Resource` without a `Mutex`, and its `recv_deadline`, which the simulation's clock waits
// with, is stable. Bevy depends on it already, so it costs no extra build time.
use crossbeam_channel::{Receiver, Sender, TryRecvError};
use sim::{
    CHUNK_SIZE, Cell, CellPos, ChunkPos, ChunkSource, Edit, ElementTable, Phase, SimSettings,
    SimStats, Simulation, WORLD_BOTTOM_Y, WORLD_TOP_Y,
};
use worldgen::Generator;

use crate::world::{WorldView, locate};

/// The seed the Sim Sandbox world is generated from.
pub const WORLD_SEED: u64 = 2026;

/// Real time between Ticks: 5 Ticks per second, each covering 0.2 s of simulated time.
pub const TICK_INTERVAL: Duration = Duration::from_millis(200);

/// How far around the focus's Chunk the game keeps a copy of the world, so Dormant Chunks are
/// there to draw.
pub const RENDER_DISTANCE: RenderDistance = RenderDistance {
    horizontal: 6,
    vertical: 3,
};

/// How often the game logs how the simulation is doing.
const LOG_INTERVAL: Duration = Duration::from_secs(5);

/// What the game asks of the simulation thread. The thread handles commands in the order they
/// were sent, and every pending one before each Tick.
#[derive(Clone, Debug)]
pub enum SimCommand {
    /// Applies an Edit.
    Edit(Edit),
    /// Moves the focus, which the Active Region follows.
    Focus(CellPos),
    /// Moves render distance to `radius` around the Chunk at `center`. Chunks within it are
    /// loaded and sent to the game, nearest `center` first; Chunks beyond it leave [`WorldView`].
    LoadAround {
        center: ChunkPos,
        radius: RenderDistance,
    },
    /// Stops the clock. Edits still apply, and Chunks still load.
    Pause,
    /// Starts the clock again. The next Tick follows one [`TICK_INTERVAL`] later.
    Resume,
    /// While the clock is stopped, runs exactly one Tick. Ignored while it runs.
    Step,
    /// Ends the simulation thread. [`SimBridge`] sends this itself when the app exits.
    Shutdown,
}

/// How many Chunks around a centre Chunk are within render distance, along each side: a box
/// `2 × horizontal + 1` Chunks across and `2 × vertical + 1` Chunks tall.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderDistance {
    pub horizontal: i32,
    pub vertical: i32,
}

/// How the simulation is doing, as of its latest Tick. Until the first Tick, it is all zeroes.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct SimReport {
    /// How many Ticks have run: the same as `stats.tick_count`.
    pub tick: u64,
    /// How long the latest Tick took to compute.
    pub tick_duration: Duration,
    pub stats: SimStats,
}

/// The focus (`CONTEXT.md`): the Cell the Active Region is centred on. The bridge sends the
/// simulation every change, and keeps render distance around the focus's Chunk.
///
/// It starts just above the surface at the origin, where it stays until the player moves it.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct SimFocus(pub CellPos);

/// The Element table the simulation runs with, for Element names and colours.
#[derive(Resource, Clone, Debug, Deref)]
pub struct Elements(pub ElementTable);

/// Runs the simulation on a thread of its own, steers it with [`SimFocus`] and render distance,
/// and keeps [`WorldView`] and [`SimReport`] up to date.
pub struct SimBridgePlugin {
    /// What the simulation starts from, taken when the plugin is built.
    setup: Mutex<Option<Setup>>,
}

struct Setup {
    elements: ElementTable,
    settings: SimSettings,
    source: Box<dyn ChunkSource>,
}

impl SimBridgePlugin {
    /// A simulation of `elements` under `settings`, whose Chunks come from `source`.
    pub fn new(
        elements: ElementTable,
        settings: SimSettings,
        source: Box<dyn ChunkSource>,
    ) -> Self {
        let setup = Setup {
            elements,
            settings,
            source,
        };
        Self {
            setup: Mutex::new(Some(setup)),
        }
    }

    /// The Sim Sandbox: Elements from `assets/elements.ron`, settings from
    /// `assets/sim_settings.ron`, and a world generated from [`WORLD_SEED`].
    ///
    /// # Panics
    ///
    /// If either file can't be read or parsed, or the Elements lack one the world is made of.
    pub fn sandbox() -> Self {
        let assets = FileAssetReader::new("assets").root_path().clone();
        let read = |name: &str| {
            let path = assets.join(name);
            fs::read_to_string(&path)
                .unwrap_or_else(|err| panic!("can't read {}: {err}", path.display()))
        };
        let elements = ElementTable::from_ron(&read("elements.ron"))
            .unwrap_or_else(|err| panic!("assets/elements.ron: {err}"));
        let settings = ron::from_str(&read("sim_settings.ron"))
            .unwrap_or_else(|err| panic!("assets/sim_settings.ron: {err}"));
        let generator = Generator::new(WORLD_SEED, &elements).unwrap_or_else(|err| panic!("{err}"));
        Self::new(elements, settings, Box::new(generator))
    }
}

impl Plugin for SimBridgePlugin {
    fn build(&self, app: &mut App) {
        let Setup {
            elements,
            settings,
            source,
        } = self
            .setup
            .lock()
            .ok()
            .and_then(|mut setup| setup.take())
            .expect("SimBridgePlugin is built only once");
        if let Err(err) = settings.validate() {
            panic!("invalid simulation settings: {err}");
        }
        let focus = just_above_surface(&*source, &elements, 0, 0);
        app.insert_resource(Elements(elements.clone()))
            .insert_resource(SimFocus(focus))
            .insert_resource(SimBridge::spawn(elements, settings, source))
            .init_resource::<WorldView>()
            .init_resource::<SimReport>()
            .add_systems(First, receive_updates)
            .add_systems(Update, log_progress)
            .add_systems(PostUpdate, follow_focus)
            .add_systems(Last, shut_down_on_exit);
    }
}

/// The game's end of the simulation thread. When the app exits, or this is dropped, it shuts the
/// thread down and waits for it to end.
#[derive(Resource)]
pub struct SimBridge {
    commands: Sender<SimCommand>,
    updates: Receiver<SimUpdate>,
    /// `None` once the thread has ended.
    thread: Option<JoinHandle<()>>,
}

impl SimBridge {
    /// Starts the simulation thread.
    fn spawn(elements: ElementTable, settings: SimSettings, source: Box<dyn ChunkSource>) -> Self {
        let (commands, command_receiver) = crossbeam_channel::unbounded();
        let (update_sender, updates) = crossbeam_channel::unbounded();
        let thread = thread::Builder::new()
            .name("sim".into())
            .spawn(move || {
                let mut sim = Simulation::with_source(elements, source);
                sim.set_settings(settings)
                    .expect("SimBridgePlugin validated the settings");
                SimThread::new(sim, command_receiver, update_sender).run();
            })
            .expect("the simulation thread starts");
        Self {
            commands,
            updates,
            thread: Some(thread),
        }
    }

    /// Sends `command` to the simulation thread. Does nothing once the thread has ended.
    pub fn send(&self, command: SimCommand) {
        // Sending fails only once the thread has ended, when there is nothing left to do.
        let _ = self.commands.send(command);
    }

    /// Shuts the simulation thread down, and waits for it to end.
    fn shut_down(&mut self) {
        if let Some(thread) = self.thread.take() {
            self.send(SimCommand::Shutdown);
            match thread.join() {
                Ok(()) => info!("the simulation thread has shut down"),
                Err(_) => error!("the simulation thread panicked"),
            }
        }
    }
}

/// Dropping the bridge, as dropping the app does, shuts the simulation thread down.
impl Drop for SimBridge {
    fn drop(&mut self) {
        self.shut_down();
    }
}

/// Moves everything the simulation thread has sent into [`WorldView`] and [`SimReport`], and
/// starts the frame's record of changed Chunks.
fn receive_updates(
    mut bridge: ResMut<SimBridge>,
    mut view: ResMut<WorldView>,
    mut report: ResMut<SimReport>,
) {
    if view.has_changes() {
        view.clear_changes();
    }
    loop {
        let received = bridge.updates.try_recv();
        match received {
            Ok(SimUpdate::Chunk(ChunkUpdate { pos, cells })) => view.insert(pos, cells),
            Ok(SimUpdate::ChunkLeftView(pos)) => view.remove(pos),
            Ok(SimUpdate::Report(latest)) => *report = latest,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                // The thread has ended unasked, which only a panic or a `SimCommand::Shutdown`
                // from elsewhere does. Pass a panic on, so the game doesn't carry on frozen.
                if let Some(thread) = bridge.thread.take()
                    && let Err(panic) = thread.join()
                {
                    std::panic::resume_unwind(panic);
                }
                return;
            }
        }
    }
}

/// Sends the simulation every change of focus, and moves render distance whenever the focus
/// enters another Chunk.
fn follow_focus(focus: Res<SimFocus>, bridge: Res<SimBridge>, mut centre: Local<Option<ChunkPos>>) {
    if !focus.is_changed() {
        return;
    }
    bridge.send(SimCommand::Focus(focus.0));
    let chunk = ChunkPos::of(focus.0);
    if *centre != Some(chunk) {
        *centre = Some(chunk);
        bridge.send(SimCommand::LoadAround {
            center: chunk,
            radius: RENDER_DISTANCE,
        });
    }
}

/// Every [`LOG_INTERVAL`], logs the Ticks per second achieved and the simulation's stats.
fn log_progress(
    time: Res<Time<Real>>,
    report: Res<SimReport>,
    view: Res<WorldView>,
    mut logged: Local<Option<(Duration, u64)>>,
) {
    let now = time.elapsed();
    let (logged_at, logged_tick) = *logged.get_or_insert((now, report.tick));
    let elapsed = now - logged_at;
    if elapsed < LOG_INTERVAL {
        return;
    }
    *logged = Some((now, report.tick));
    let ticks_per_second = report.tick.saturating_sub(logged_tick) as f64 / elapsed.as_secs_f64();
    let stats = report.stats;
    info!(
        "sim: {ticks_per_second:.1} Ticks/s; Tick {} took {:.1} ms; \
         Chunks: {} active ({} Settled), {} Dormant, {} in view; \
         total Mass {:.3} kg, {} g lost to space",
        report.tick,
        report.tick_duration.as_secs_f64() * 1_000.0,
        stats.active_chunks,
        stats.settled_chunks,
        stats.dormant_chunks,
        view.len(),
        stats.total_mass_grams as f64 / 1_000.0,
        stats.mass_lost_to_space_grams,
    );
}

/// Shuts the simulation thread down when the app exits.
fn shut_down_on_exit(mut exits: MessageReader<AppExit>, mut bridge: ResMut<SimBridge>) {
    if !exits.is_empty() {
        exits.clear();
        bridge.shut_down();
    }
}

/// The Cell just above the highest solid or liquid Cell at `(x, z)`, or at `y = 0` if that whole
/// column of the world is gas and Vacuum. Generates the Chunks the column runs through from the
/// top down, until it finds one.
fn just_above_surface(
    source: &dyn ChunkSource,
    elements: &ElementTable,
    x: i32,
    z: i32,
) -> CellPos {
    let size = CHUNK_SIZE as i32;
    for chunk_y in (WORLD_BOTTOM_Y / size..WORLD_TOP_Y / size).rev() {
        let chunk_origin = CellPos::new(x, chunk_y * size, z);
        let cells = source.generate(ChunkPos::of(chunk_origin));
        for y in (chunk_origin.y..chunk_origin.y + size).rev() {
            let pos = CellPos::new(x, y, z);
            let phase = elements.get(cells[locate(pos).1].element).phase;
            if matches!(phase, Phase::Solid | Phase::Liquid) {
                return CellPos::new(x, y + 1, z);
            }
        }
    }
    CellPos::new(x, 0, z)
}

/// What the simulation thread sends the game.
enum SimUpdate {
    Chunk(ChunkUpdate),
    /// The Chunk at this position has left render distance.
    ChunkLeftView(ChunkPos),
    Report(SimReport),
}

/// All the Cells of a Chunk within render distance that has loaded or changed.
struct ChunkUpdate {
    pos: ChunkPos,
    cells: Arc<[Cell]>,
}

/// The simulation thread: the simulation, its clock, and what the game has been sent.
struct SimThread {
    sim: Simulation,
    commands: Receiver<SimCommand>,
    updates: Sender<SimUpdate>,
    paused: bool,
    /// When the next Tick is due.
    next_tick: Instant,
    /// The Chunks within render distance, once the game has said where that is. Until then,
    /// every Chunk counts as within it.
    view: Option<ChunkBox>,
    /// The Chunks in the game's [`WorldView`]: every Chunk sent to it, unless it has left render
    /// distance since.
    shown: HashSet<ChunkPos>,
    /// Chunks within render distance that aren't shown yet, nearest its centre first.
    to_show: VecDeque<ChunkPos>,
}

/// How long the simulation thread waits for a command.
enum Wait {
    No,
    Until(Instant),
    Indefinitely,
}

impl SimThread {
    fn new(sim: Simulation, commands: Receiver<SimCommand>, updates: Sender<SimUpdate>) -> Self {
        Self {
            sim,
            commands,
            updates,
            paused: false,
            next_tick: Instant::now() + TICK_INTERVAL,
            view: None,
            shown: HashSet::new(),
            to_show: VecDeque::new(),
        }
    }

    fn run(mut self) {
        // The Active Region, loaded when the simulation was made.
        self.send_changed_chunks(None);
        while self.handle_commands().is_continue() {
            // Edits, and Chunks that entered the Active Region as the focus moved.
            self.send_changed_chunks(None);
            if !self.paused && Instant::now() >= self.next_tick {
                self.tick();
                self.next_tick = (self.next_tick + TICK_INTERVAL).max(Instant::now());
            }
            // Even when every Tick overruns, at least one Chunk loads between them.
            self.show_next_chunk();
        }
    }

    /// Waits for a command, unless there is other work to do, then handles every pending one.
    /// Breaks once the thread should end.
    fn handle_commands(&mut self) -> ControlFlow<()> {
        let mut wait = if !self.to_show.is_empty() {
            Wait::No
        } else if self.paused {
            Wait::Indefinitely
        } else {
            Wait::Until(self.next_tick)
        };
        while let Some(command) = self.receive(wait)? {
            self.handle(command)?;
            wait = Wait::No;
        }
        ControlFlow::Continue(())
    }

    /// The next command, if one comes while waiting as `wait` says. Breaks if the game's end of
    /// the channel is gone.
    fn receive(&self, wait: Wait) -> ControlFlow<(), Option<SimCommand>> {
        let received = match wait {
            Wait::No => self
                .commands
                .try_recv()
                .map_err(|err| err.is_disconnected()),
            Wait::Until(deadline) => self
                .commands
                .recv_deadline(deadline)
                .map_err(|err| err.is_disconnected()),
            Wait::Indefinitely => self.commands.recv().map_err(|_| true),
        };
        match received {
            Ok(command) => ControlFlow::Continue(Some(command)),
            Err(true) => ControlFlow::Break(()),
            Err(false) => ControlFlow::Continue(None),
        }
    }

    fn handle(&mut self, command: SimCommand) -> ControlFlow<()> {
        match command {
            SimCommand::Edit(edit) => self.sim.apply(edit),
            SimCommand::Focus(pos) => self.sim.focus_on(pos),
            SimCommand::LoadAround { center, radius } => self.show_around(center, radius),
            SimCommand::Pause => self.paused = true,
            SimCommand::Resume if self.paused => {
                self.paused = false;
                self.next_tick = Instant::now() + TICK_INTERVAL;
            }
            SimCommand::Step if self.paused => self.tick(),
            SimCommand::Resume | SimCommand::Step => {}
            SimCommand::Shutdown => return ControlFlow::Break(()),
        }
        ControlFlow::Continue(())
    }

    fn tick(&mut self) {
        let started = Instant::now();
        self.sim.tick();
        let tick_duration = started.elapsed();
        self.send_changed_chunks(None);
        let stats = self.sim.stats();
        self.send(SimUpdate::Report(SimReport {
            tick: stats.tick_count,
            tick_duration,
            stats,
        }));
    }

    /// Moves render distance to `radius` around `center`: Chunks beyond it leave the game's view,
    /// and Chunks within it that the game lacks are queued to be shown.
    fn show_around(&mut self, center: ChunkPos, radius: RenderDistance) {
        let view = ChunkBox::around(center, radius);
        let mut leaving: Vec<ChunkPos> = self
            .shown
            .iter()
            .copied()
            .filter(|&pos| !view.contains(pos))
            .collect();
        leaving.sort();
        for pos in leaving {
            self.shown.remove(&pos);
            self.send(SimUpdate::ChunkLeftView(pos));
        }
        let mut entering: Vec<ChunkPos> = view
            .chunks()
            .filter(|pos| !self.shown.contains(pos))
            .collect();
        entering.sort_by_key(|&pos| distance_squared(pos, center));
        self.to_show = entering.into();
        self.view = Some(view);
    }

    /// Loads the nearest queued Chunk the game lacks, if any, and sends it.
    fn show_next_chunk(&mut self) {
        while let Some(pos) = self.to_show.pop_front() {
            if !self.shown.contains(&pos) {
                self.sim.load(pos);
                self.send_changed_chunks(Some(pos));
                return;
            }
        }
    }

    /// Sends the game every Chunk within render distance that has loaded or changed since the
    /// last call, and `also`. A Chunk that holds nothing but Vacuum since it loaded isn't stored,
    /// so it isn't sent. A changed Chunk beyond render distance isn't sent either, but it will be
    /// once it enters, since the game lacks it.
    fn send_changed_chunks(&mut self, also: Option<ChunkPos>) {
        let mut chunks = self.sim.take_changed_chunks();
        if let Some(pos) = also
            && !chunks.contains(&pos)
        {
            chunks.push(pos);
        }
        for pos in chunks {
            if self.view.is_some_and(|view| !view.contains(pos)) {
                continue;
            }
            if let Some(cells) = self.sim.chunk_cells(pos) {
                let cells = Arc::from(cells);
                self.shown.insert(pos);
                self.send(SimUpdate::Chunk(ChunkUpdate { pos, cells }));
            }
        }
    }

    fn send(&self, update: SimUpdate) {
        // Sending fails only once the game's end is gone, and then a Shutdown is on its way, or
        // the commands channel is gone too.
        let _ = self.updates.send(update);
    }
}

/// A box of Chunks, from `min` to `max` inclusive.
#[derive(Clone, Copy, Debug)]
struct ChunkBox {
    min: ChunkPos,
    max: ChunkPos,
}

impl ChunkBox {
    fn around(center: ChunkPos, radius: RenderDistance) -> Self {
        let RenderDistance {
            horizontal: h,
            vertical: v,
        } = radius;
        Self {
            min: ChunkPos::new(center.x - h, center.y - v, center.z - h),
            max: ChunkPos::new(center.x + h, center.y + v, center.z + h),
        }
    }

    fn contains(self, pos: ChunkPos) -> bool {
        (self.min.x..=self.max.x).contains(&pos.x)
            && (self.min.y..=self.max.y).contains(&pos.y)
            && (self.min.z..=self.max.z).contains(&pos.z)
    }

    fn chunks(self) -> impl Iterator<Item = ChunkPos> {
        let (min, max) = (self.min, self.max);
        (min.x..=max.x).flat_map(move |x| {
            (min.y..=max.y).flat_map(move |y| (min.z..=max.z).map(move |z| ChunkPos::new(x, y, z)))
        })
    }
}

fn distance_squared(a: ChunkPos, b: ChunkPos) -> i64 {
    let d = |a: i32, b: i32| i64::from(a - b).pow(2);
    d(a.x, b.x) + d(a.y, b.y) + d(a.z, b.z)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_settings_are_the_defaults_with_heat_conduction_sped_up() {
        let settings: SimSettings = ron::from_str(include_str!("../assets/sim_settings.ron"))
            .expect("the shipped settings file parses");
        let expected = SimSettings {
            conduction_multiplier: 1_000.0,
            ..SimSettings::default()
        };
        assert_eq!(settings, expected);
        assert_eq!(settings.validate(), Ok(()));
    }
}
