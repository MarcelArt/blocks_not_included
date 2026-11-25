//! The bridge between the game and the simulation, which runs on a thread of its own (ADR 0003).
//!
//! The game sends the simulation thread [`SimCommand`]s. The thread sends back the Cells of every
//! Chunk within render distance that loads or changes, and where the Active Region is whenever it
//! moves, which the game keeps in [`WorldView`], and a [`SimReport`] after every Tick. It answers
//! a replaced Element table or settings with a [`ReplaceOutcome`], and a request for a capture
//! with a [`SimCapture`], on a channel of its own.
//!
//! # Starting
//!
//! The simulation starts from a fresh world ([`SimBridgePlugin::new`],
//! [`SimBridgePlugin::generated`]) or from a capture ([`SimBridgePlugin::restore`]).
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
use std::path::Path;
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
    ActiveRegion, CHUNK_SIZE, Cell, CellPos, Checkpoint, CheckpointError, ChunkPos, ChunkSource,
    Edit, ElementError, ElementTable, Phase, SettingsError, SimSettings, SimStats, Simulation,
    WORLD_BOTTOM_Y, WORLD_TOP_Y,
};
use worldgen::{Generator, MissingElements};

use crate::sim_files::{ElementsFile, SettingsFile, SimFile};
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

/// How much real time each measurement of [`TickRate`] covers.
const TICK_RATE_INTERVAL: Duration = Duration::from_secs(1);

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
    /// Replaces the Element table, as [`Simulation::replace_elements`] does. The answer arrives
    /// as a [`ReplaceOutcome`], and [`Elements`] holds the table once it is accepted.
    ReplaceElements(ElementTable),
    /// Replaces the tuning values, as [`Simulation::set_settings`] does. The answer arrives as a
    /// [`ReplaceOutcome`].
    ReplaceSettings(SimSettings),
    /// Captures the simulation, once every command sent before has been handled, and sends the
    /// [`SimCapture`] on this channel. [`SimBridge::capture`] sends this for you.
    Capture(Sender<SimCapture>),
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

/// The simulation's answer to a [`SimCommand::ReplaceElements`] or
/// [`SimCommand::ReplaceSettings`], sent as a message on the frame it arrives. A rejected table or
/// settings leaves the simulation as it was.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum ReplaceOutcome {
    /// By the time this arrives, [`Elements`] holds an accepted table.
    Elements(Result<(), ElementError>),
    Settings(Result<(), SettingsError>),
}

/// The Ticks per second the simulation achieved over the latest [`TICK_RATE_INTERVAL`] of real
/// time. It is 0 until the first measurement, and while the clock is stopped.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct TickRate(pub f64);

/// The focus (`CONTEXT.md`): the Cell the Active Region is centred on. The bridge sends the
/// simulation every change, and keeps render distance around the focus's Chunk.
///
/// It starts just above the surface at the origin, where it stays until the player moves it.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct SimFocus(pub CellPos);

/// The Element table the simulation runs with, for Element names and colours.
#[derive(Resource, Clone, Debug, Deref)]
pub struct Elements(pub ElementTable);

/// What a generated world's terrain comes from: its World Seed, and the Element table the world
/// generator was built with.
///
/// Hot reloads of the simulation's Elements leave it as it is, so every Chunk the generator makes
/// matches the ones it made before, which a [`Checkpoint`] relies on (ADR 0005).
#[derive(Clone, Debug, PartialEq)]
pub struct Generation {
    pub seed: u64,
    pub elements: ElementTable,
}

impl Generation {
    /// The world generator these inputs make.
    pub fn generator(&self) -> Result<Generator, MissingElements> {
        Generator::new(self.seed, &self.elements)
    }
}

/// The simulation captured between two Ticks, as [`SimBridge::capture`] answers: its
/// [`Checkpoint`], with the rules it ran under and, for a generated world, what its terrain comes
/// from. [`SimBridgePlugin::restore`] starts the simulation from it again.
///
/// It is owned: nothing the simulation does afterwards changes it, so it can be encoded and
/// written anywhere, at any pace. [`WorldStore`](crate::saves::WorldStore) saves it to disk.
#[derive(Clone, Debug, PartialEq)]
pub struct SimCapture {
    pub checkpoint: Checkpoint,
    /// The Element table the simulation ran with: the last one it accepted, or the one it
    /// started with.
    pub elements: ElementTable,
    /// The tuning values the simulation ran with: the last ones it accepted, or the ones it
    /// started with.
    pub settings: SimSettings,
    /// What the world generator was built from, which reloads don't change. `None` for a world
    /// from another [`ChunkSource`].
    pub generation: Option<Generation>,
}

/// Runs the simulation on a thread of its own, steers it with [`SimFocus`] and render distance,
/// and keeps [`WorldView`] and [`SimReport`] up to date.
pub struct SimBridgePlugin {
    /// What the simulation starts from, taken when the plugin is built.
    setup: Mutex<Option<Setup>>,
}

struct Setup {
    /// The Element table the simulation starts with, for [`Elements`].
    elements: ElementTable,
    generation: Option<Generation>,
    start: Start,
}

/// How the simulation thread gets its simulation.
enum Start {
    /// It makes a new one, whose Chunks come from `source`.
    Fresh {
        elements: ElementTable,
        settings: SimSettings,
        source: Box<dyn ChunkSource>,
    },
    /// It takes this one, restored from a checkpoint.
    Restored(Box<Simulation>),
}

impl SimBridgePlugin {
    /// A simulation of `elements` under `settings`, whose Chunks come from `source`.
    pub fn new(
        elements: ElementTable,
        settings: SimSettings,
        source: Box<dyn ChunkSource>,
    ) -> Self {
        Self::starting(Setup {
            elements: elements.clone(),
            generation: None,
            start: Start::Fresh {
                elements,
                settings,
                source,
            },
        })
    }

    /// A simulation of `elements` under `settings`, in a world generated from `seed` and
    /// `elements`. The generator keeps these Elements whatever the simulation reloads later, and
    /// every [`SimCapture`] holds them in its [`Generation`].
    ///
    /// Fails if `elements` lacks one the world is made of.
    pub fn generated(
        seed: u64,
        elements: ElementTable,
        settings: SimSettings,
    ) -> Result<Self, MissingElements> {
        let generation = Generation { seed, elements };
        let generator = generation.generator()?;
        Ok(Self::starting(Setup {
            elements: generation.elements.clone(),
            start: Start::Fresh {
                elements: generation.elements.clone(),
                settings,
                source: Box::new(generator),
            },
            generation: Some(generation),
        }))
    }

    /// The simulation `capture` holds, restored with its own rules, and with `source` for the
    /// Chunks it leaves to the world generator. For a generated world, that is the generator its
    /// [`Generation`] makes. The focus starts where the capture's was.
    ///
    /// The capture is checked against its rules and `source` first, as [`Simulation::restore`]
    /// does, and a capture that fails is rejected before any simulation starts.
    pub fn restore(
        capture: SimCapture,
        source: Box<dyn ChunkSource>,
    ) -> Result<Self, CheckpointError> {
        let SimCapture {
            checkpoint,
            elements,
            settings,
            generation,
        } = capture;
        let sim = Simulation::restore(elements.clone(), settings, source, checkpoint)?;
        Ok(Self::starting(Setup {
            elements,
            generation,
            start: Start::Restored(Box::new(sim)),
        }))
    }

    /// The Sim Sandbox: Elements from `assets/elements.ron`, settings from
    /// `assets/sim_settings.ron`, and a world generated from [`WORLD_SEED`].
    ///
    /// # Panics
    ///
    /// If either file can't be read or parsed, or the Elements lack one the world is made of.
    pub fn sandbox() -> Self {
        let assets = FileAssetReader::new("assets").root_path().clone();
        let ElementsFile(elements) = read_sim_file(&assets);
        let SettingsFile(settings) = read_sim_file(&assets);
        Self::generated(WORLD_SEED, elements, settings).unwrap_or_else(|err| panic!("{err}"))
    }

    fn starting(setup: Setup) -> Self {
        Self {
            setup: Mutex::new(Some(setup)),
        }
    }
}

/// Reads and parses `F` from the `assets` folder at `assets`.
///
/// # Panics
///
/// If the file can't be read, or can't be used.
fn read_sim_file<F: SimFile>(assets: &Path) -> F {
    let path = assets.join(F::PATH);
    let bytes =
        fs::read(&path).unwrap_or_else(|err| panic!("can't read {}: {err}", path.display()));
    F::parse(&bytes).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}

impl Plugin for SimBridgePlugin {
    fn build(&self, app: &mut App) {
        let Setup {
            elements,
            generation,
            start,
        } = self
            .setup
            .lock()
            .ok()
            .and_then(|mut setup| setup.take())
            .expect("SimBridgePlugin is built only once");
        let focus = match &start {
            Start::Fresh {
                elements,
                settings,
                source,
            } => {
                if let Err(err) = settings.validate() {
                    panic!("invalid simulation settings: {err}");
                }
                just_above_surface(&**source, elements, 0, 0)
            }
            Start::Restored(sim) => sim.focus(),
        };
        app.insert_resource(Elements(elements))
            .insert_resource(SimFocus(focus))
            .insert_resource(SimBridge::spawn(start, generation))
            .init_resource::<WorldView>()
            .init_resource::<SimReport>()
            .init_resource::<TickRate>()
            .add_message::<ReplaceOutcome>()
            .add_systems(First, receive_updates)
            .add_systems(Update, (measure_tick_rate, log_progress).chain())
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
    fn spawn(start: Start, generation: Option<Generation>) -> Self {
        let (commands, command_receiver) = crossbeam_channel::unbounded();
        let (update_sender, updates) = crossbeam_channel::unbounded();
        let thread = thread::Builder::new()
            .name("sim".into())
            .spawn(move || {
                let sim = match start {
                    Start::Fresh {
                        elements,
                        settings,
                        source,
                    } => {
                        let mut sim = Simulation::with_source(elements, source);
                        sim.set_settings(settings)
                            .expect("SimBridgePlugin validated the settings");
                        sim
                    }
                    Start::Restored(sim) => *sim,
                };
                SimThread::new(sim, generation, command_receiver, update_sender).run();
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

    /// Asks the simulation for a [`SimCapture`], taken once it has handled every command sent
    /// before this one, and returns the channel it arrives on. It arrives between two Ticks.
    ///
    /// Wait for it away from the game's schedule, such as on the thread that encodes and writes
    /// it. If the simulation thread has ended, the channel is disconnected and nothing arrives.
    pub fn capture(&self) -> Receiver<SimCapture> {
        let (reply, answer) = crossbeam_channel::bounded(1);
        self.send(SimCommand::Capture(reply));
        answer
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

/// Moves everything the simulation thread has sent into [`WorldView`], [`SimReport`] and
/// [`Elements`], passes its answers on as [`ReplaceOutcome`]s, and starts the frame's record of
/// changed Chunks.
fn receive_updates(
    mut bridge: ResMut<SimBridge>,
    mut view: ResMut<WorldView>,
    mut report: ResMut<SimReport>,
    mut elements: ResMut<Elements>,
    mut outcomes: MessageWriter<ReplaceOutcome>,
) {
    if view.has_changes() {
        view.clear_changes();
    }
    loop {
        let received = bridge.updates.try_recv();
        match received {
            Ok(SimUpdate::Chunk(ChunkUpdate { pos, cells })) => view.insert(pos, cells),
            Ok(SimUpdate::ChunkLeftView(pos)) => view.remove(pos),
            Ok(SimUpdate::ActiveRegion(region)) => view.set_active_region(region),
            Ok(SimUpdate::Report(latest)) => *report = latest,
            Ok(SimUpdate::ElementsReplaced(answer)) => {
                let answer = match answer {
                    Ok(table) => {
                        elements.0 = table;
                        Ok(())
                    }
                    Err(error) => Err(error),
                };
                outcomes.write(ReplaceOutcome::Elements(answer));
            }
            Ok(SimUpdate::SettingsReplaced(answer)) => {
                outcomes.write(ReplaceOutcome::Settings(answer));
            }
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

/// Every [`TICK_RATE_INTERVAL`], measures the Ticks per second achieved since the last time.
fn measure_tick_rate(
    time: Res<Time<Real>>,
    report: Res<SimReport>,
    mut rate: ResMut<TickRate>,
    mut measured: Local<Option<(Duration, u64)>>,
) {
    let now = time.elapsed();
    let (measured_at, measured_tick) = *measured.get_or_insert((now, report.tick));
    let elapsed = now - measured_at;
    if elapsed >= TICK_RATE_INTERVAL {
        *measured = Some((now, report.tick));
        rate.0 = report.tick.saturating_sub(measured_tick) as f64 / elapsed.as_secs_f64();
    }
}

/// Every [`LOG_INTERVAL`], logs the Ticks per second achieved and the simulation's stats.
fn log_progress(
    time: Res<Time<Real>>,
    report: Res<SimReport>,
    rate: Res<TickRate>,
    view: Res<WorldView>,
    mut logged_at: Local<Option<Duration>>,
) {
    let now = time.elapsed();
    if now - *logged_at.get_or_insert(now) < LOG_INTERVAL {
        return;
    }
    *logged_at = Some(now);
    let stats = report.stats;
    info!(
        "sim: {:.1} Ticks/s; Tick {} took {:.1} ms; \
         Chunks: {} active ({} Settled), {} Dormant, {} in view; \
         total Mass {:.3} kg, {} g lost to space",
        rate.0,
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
    /// The Active Region has moved here.
    ActiveRegion(ActiveRegion),
    Report(SimReport),
    /// The answer to a [`SimCommand::ReplaceElements`]: the table now in use, or why it was
    /// rejected.
    ElementsReplaced(Result<ElementTable, ElementError>),
    /// The answer to a [`SimCommand::ReplaceSettings`].
    SettingsReplaced(Result<(), SettingsError>),
}

/// All the Cells of a Chunk within render distance that has loaded or changed.
struct ChunkUpdate {
    pos: ChunkPos,
    cells: Arc<[Cell]>,
}

/// The simulation thread: the simulation, its clock, and what the game has been sent.
struct SimThread {
    sim: Simulation,
    /// What the world generator was built from, for captures.
    generation: Option<Generation>,
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
    /// The Active Region as the game was last told it.
    region_sent: Option<ActiveRegion>,
}

/// How long the simulation thread waits for a command.
enum Wait {
    No,
    Until(Instant),
    Indefinitely,
}

impl SimThread {
    fn new(
        sim: Simulation,
        generation: Option<Generation>,
        commands: Receiver<SimCommand>,
        updates: Sender<SimUpdate>,
    ) -> Self {
        Self {
            sim,
            generation,
            commands,
            updates,
            paused: false,
            next_tick: Instant::now() + TICK_INTERVAL,
            view: None,
            shown: HashSet::new(),
            to_show: VecDeque::new(),
            region_sent: None,
        }
    }

    fn run(mut self) {
        // The Active Region, loaded when the simulation was made.
        self.send_active_region();
        self.send_changed_chunks(None);
        while self.handle_commands().is_continue() {
            // Edits, and Chunks that entered the Active Region as the focus moved.
            self.send_active_region();
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
            SimCommand::Focus(pos) => self.focus_on(pos),
            SimCommand::LoadAround { center, radius } => self.show_around(center, radius),
            SimCommand::Pause => self.paused = true,
            SimCommand::Resume if self.paused => {
                self.paused = false;
                self.next_tick = Instant::now() + TICK_INTERVAL;
            }
            SimCommand::Step if self.paused => self.tick(),
            SimCommand::Resume | SimCommand::Step => {}
            SimCommand::ReplaceElements(elements) => {
                let answer = self.sim.replace_elements(elements.clone());
                self.send(SimUpdate::ElementsReplaced(answer.map(|()| elements)));
            }
            SimCommand::ReplaceSettings(settings) => {
                let answer = self.sim.set_settings(settings);
                self.send(SimUpdate::SettingsReplaced(answer));
            }
            SimCommand::Capture(reply) => {
                // Sending fails only if the asker has stopped waiting.
                let _ = reply.send(self.capture());
            }
            SimCommand::Shutdown => return ControlFlow::Break(()),
        }
        ControlFlow::Continue(())
    }

    /// The simulation as it stands, with its rules and what its terrain comes from.
    fn capture(&self) -> SimCapture {
        SimCapture {
            checkpoint: self.sim.checkpoint(),
            elements: self.sim.elements().clone(),
            settings: self.sim.settings().clone(),
            generation: self.generation.clone(),
        }
    }

    /// Moves the focus to `pos`, and logs where the Active Region goes if it moves.
    fn focus_on(&mut self, pos: CellPos) {
        let before = self.sim.active_region();
        self.sim.focus_on(pos);
        let region = self.sim.active_region();
        if region != before {
            let (min, max) = (region.min(), region.max());
            info!(
                "the Active Region moved to Chunks ({}, {}, {}) to ({}, {}, {}), following the \
                 focus to ({}, {}, {})",
                min.x, min.y, min.z, max.x, max.y, max.z, pos.x, pos.y, pos.z,
            );
        }
    }

    /// Tells the game where the Active Region is, if it has moved since the game was last told.
    fn send_active_region(&mut self) {
        let region = self.sim.active_region();
        if self.region_sent != Some(region) {
            self.region_sent = Some(region);
            self.send(SimUpdate::ActiveRegion(region));
        }
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

/// The square of the straight-line distance between two Chunks, counted in Chunks.
pub(crate) fn distance_squared(a: ChunkPos, b: ChunkPos) -> i64 {
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
