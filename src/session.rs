//! World sessions: opening a World, pausing it, and closing it again
//! (`docs/plans/18-world-sessions.md`).
//!
//! Nothing of a World exists until it opens. [`OpenWorld`] opens one from a [`WorldStart`]:
//! a new World ([`WorldStart::new_game`]), saved before its first Tick with its player standing
//! on the ground, or a saved one ([`WorldStart::load_game`]). Either way, the player carries on
//! exactly as saved. A [`WorldStart`] without a player spawns them on the ground. While it is
//! open, the state is [`Session::Open`]: the simulation thread runs, and the World's resources,
//! entities and background work exist. [`CloseWorld`] closes it, and all of that goes with it:
//! the simulation thread shuts down, its resources are removed, entities marked
//! `DespawnOnExit(Session::Open)` are despawned (the player and its camera, the sun, every
//! Chunk's and Overlay's meshes), and meshing under way is dropped. The next World starts from
//! nothing of the last.
//!
//! # Pause
//!
//! While a World is open, [`Play`] says whether it runs. Pausing ([`Play::Pausing`]) freezes the
//! simulation between two Ticks, after every command sent before, and holds the player, the
//! focus, the god tools, the debug clock and hot reloads still. The pause is complete
//! ([`Play::Paused`]) once the simulation has acknowledged it. Resuming unfreezes the simulation,
//! with the debug clock as it was, and sends the latest hot reload held back meanwhile.
//! [`PauseControl`] asks for these transitions. Esc does both (`crate::input_policy`).
//!
//! Leaving a World ([`Play::Leaving`]) pauses it the same way, but neither Esc nor
//! [`PauseControl::resume`] runs it again: only [`PauseControl::stay`] does, for a player who
//! changes their mind. It holds the World still while its last save is written
//! (`crate::lifecycle`).
//!
//! Opening a World runs, in `OnEnter(Session::Open)`, the [`SessionSystems::Start`] systems,
//! which start the simulation and insert the World's resources, then the
//! [`SessionSystems::Build`] systems, which build on them.

use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::state::state::FreelyMutableState;
use sim::{CheckpointError, ElementTable, Phase, SimSettings};
use worldgen::MissingElements;

use crate::player::movement::Body;
use crate::player::{feet_in, ground_below, ground_column, spawn_facing};
use crate::saves::{PlayerState, WorldSave};
use crate::sim_bridge::{PendingSim, SimBridge, SimStart};
use crate::world::locate;

/// Whether a World is open.
#[derive(States, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Session {
    /// No World: no simulation runs, and nothing of a World is drawn.
    #[default]
    Closed,
    Open,
}

/// Whether the open World runs. It exists only while a World is open, and starts running.
#[derive(SubStates, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[source(Session = Session::Open)]
pub enum Play {
    #[default]
    Running,
    /// Paused, and waiting for the simulation to acknowledge it. Nothing the player does reaches
    /// the World any more.
    Pausing,
    /// Paused, and acknowledged: the simulation has stopped between two Ticks, after every
    /// command sent before the pause.
    Paused,
    /// Paused while the player leaves the World, for as long as its last save takes. Nothing the
    /// player does reaches the World, and only [`PauseControl::stay`] runs it again.
    Leaving,
}

/// The order of the systems that open a World, in `OnEnter(Session::Open)`.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SessionSystems {
    /// Starts the simulation, and inserts the World's resources: `SimBridge`, `Elements`,
    /// `SimFocus`, `WorldView`, `SimReport`, `TickRate`, and [`PlayerStart`].
    Start,
    /// Everything that builds on them: the player, the meshes, the tools.
    Build,
}

/// What a World opens from: its simulation, and the player as saved, if it was.
pub struct WorldStart {
    pub sim: SimStart,
    /// The player as saved, held where they were until the Chunks around them load. `None` for a
    /// new World, whose player spawns on the ground below the simulation's starting focus.
    pub player: Option<PlayerState>,
}

impl WorldStart {
    /// A new World generated from `seed`, simulated with `elements` under `settings`, and its
    /// first save. Its player stands on the ground at the origin, where [`ground_below`] puts
    /// them, facing as [`spawn_facing`] says, and the focus is at their feet. The save holds
    /// exactly the World this opens: the player, and the simulation before its first Tick.
    ///
    /// This finds the ground at the origin, generating the Chunks above it from the top of the
    /// world down, and generates the Active Region around it, so a caller may prefer to build it
    /// on another thread.
    ///
    /// # Panics
    ///
    /// If `settings` are invalid.
    pub fn new_game(
        seed: u64,
        elements: ElementTable,
        settings: SimSettings,
    ) -> Result<(Self, WorldSave), MissingElements> {
        let mut sim = SimStart::generated(seed, elements, settings)?;
        let focus = sim.focus();
        let column: HashMap<_, _> = ground_column(focus)
            .filter_map(|chunk| Some((chunk, sim.generate(chunk)?)))
            .collect();
        let feet = ground_below(focus, |pos| {
            let (chunk, index) = locate(pos);
            column
                .get(&chunk)
                .is_some_and(|cells| sim.elements().get(cells[index].element).phase == Phase::Solid)
        });
        sim.set_focus(feet);
        let player = PlayerState::new(Body::standing_at(feet_in(feet)), spawn_facing());
        let (sim, capture) = sim.capture_before_start();
        let start = Self {
            sim,
            player: Some(player),
        };
        Ok((start, WorldSave { capture, player }))
    }

    /// The World `save` holds, with its terrain generated from its World Seed and the
    /// generator's Elements, its simulation as captured, and its player as saved.
    ///
    /// This checks and restores the capture, regenerating the Active Region's untouched Chunks at
    /// about 8 ms each (step 16), so a caller may prefer to build it on another thread.
    pub fn load_game(save: WorldSave) -> Result<Self, LoadError> {
        let WorldSave { capture, player } = save;
        let generator = capture
            .generation
            .as_ref()
            .ok_or(LoadError::NotGenerated)?
            .generator()
            .map_err(LoadError::Generator)?;
        let sim =
            SimStart::restored(capture, Box::new(generator)).map_err(LoadError::Checkpoint)?;
        Ok(Self {
            sim,
            player: Some(player),
        })
    }
}

/// Why a saved World can't open.
#[derive(Clone, Debug, PartialEq)]
pub enum LoadError {
    /// The save doesn't say what its terrain is generated from.
    NotGenerated,
    /// The generator's Elements lack one the terrain is made of.
    Generator(MissingElements),
    /// The simulation's checkpoint doesn't fit its rules or its terrain.
    Checkpoint(CheckpointError),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotGenerated => write!(f, "the save doesn't say how its terrain is generated"),
            Self::Generator(error) => error.fmt(f),
            Self::Checkpoint(error) => write!(f, "the saved simulation can't be restored: {error}"),
        }
    }
}

impl std::error::Error for LoadError {}

/// The player a World opens with, for the player's systems: as saved, or `None` to spawn on the
/// ground. Inserted as the World opens.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct PlayerStart(pub Option<PlayerState>);

/// Opens a World from its [`WorldStart`] at the next state transition: during this frame's, if
/// applied before `PreUpdate` ends. Open one only while none is.
pub struct OpenWorld(pub WorldStart);

impl Command for OpenWorld {
    type Out = ();

    fn apply(self, world: &mut World) {
        let WorldStart { sim, player } = self.0;
        if world
            .get_resource::<State<Session>>()
            .is_some_and(|session| *session.get() == Session::Open)
        {
            warn!("a World opens while another is open: the open one closes first");
        }
        world.insert_resource(PendingSim(Mutex::new(sim)));
        world.insert_resource(PlayerStart(player));
        world
            .resource_mut::<NextState<Session>>()
            .set(Session::Open);
    }
}

/// Closes the open World at the next state transition.
pub struct CloseWorld;

impl Command for CloseWorld {
    type Out = ();

    fn apply(self, world: &mut World) {
        world
            .resource_mut::<NextState<Session>>()
            .set_if_different(Session::Closed);
    }
}

/// The World's sessions and its pause. Needs Bevy's `StatesPlugin`, which `DefaultPlugins` has,
/// and the sim bridge.
pub struct SessionPlugin;

impl Plugin for SessionPlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<Session>()
            .add_sub_state::<Play>()
            .configure_sets(
                OnEnter(Session::Open),
                (SessionSystems::Start, SessionSystems::Build).chain(),
            )
            .add_systems(OnExit(Session::Open), |mut commands: Commands| {
                commands.remove_resource::<PlayerStart>();
            })
            .add_systems(
                OnExit(Play::Running),
                freeze_simulation.run_if(in_state(Session::Open)),
            )
            .add_systems(OnEnter(Play::Running), unfreeze_simulation)
            .add_systems(PreUpdate, complete_pause.run_if(in_state(Play::Pausing)));
    }
}

/// Freezes the simulation as the World pauses.
fn freeze_simulation(mut bridge: ResMut<SimBridge>) {
    bridge.freeze();
}

/// Unfreezes the simulation as the World runs again. As a World opens, its simulation isn't
/// frozen, so this does nothing.
fn unfreeze_simulation(bridge: Option<ResMut<SimBridge>>) {
    if let Some(mut bridge) = bridge {
        bridge.unfreeze();
    }
}

/// Completes the pause once the simulation acknowledges it.
fn complete_pause(bridge: Res<SimBridge>, mut pause: PauseControl) {
    if bridge.frozen_at().is_some() {
        pause.complete();
    }
}

/// Asks for the open World to pause or run. Each request takes effect at the next state
/// transition: this frame's, if made before `PreUpdate` ends.
#[derive(SystemParam)]
pub struct PauseControl<'w> {
    session: Res<'w, State<Session>>,
    next_session: Res<'w, NextState<Session>>,
    play: Option<Res<'w, State<Play>>>,
    next: Option<ResMut<'w, NextState<Play>>>,
}

impl PauseControl<'_> {
    /// Whether the World will run, pause or be paused once this frame's requests take effect:
    /// `None` if no World will be open. A World about to open will run.
    pub fn state(&self) -> Option<Play> {
        let session = *self.session.get();
        if pending(&self.next_session).unwrap_or(session) != Session::Open {
            return None;
        }
        let Some(play) = self.play.as_deref().filter(|_| session == Session::Open) else {
            return Some(Play::default());
        };
        let next = self.next.as_deref().and_then(pending);
        Some(next.unwrap_or(*play.get()))
    }

    /// Pauses the World, if it runs.
    pub fn pause(&mut self) {
        if self.state() == Some(Play::Running) {
            self.set(Play::Pausing);
        }
    }

    /// Runs the World again, if it is paused.
    pub fn resume(&mut self) {
        if matches!(self.state(), Some(Play::Pausing | Play::Paused)) {
            self.set(Play::Running);
        }
    }

    /// Pauses the World if it runs, or runs it if it is paused. Does nothing while the player
    /// leaves it.
    pub fn toggle(&mut self) {
        match self.state() {
            Some(Play::Running) => self.pause(),
            Some(Play::Pausing | Play::Paused) => self.resume(),
            Some(Play::Leaving) | None => {}
        }
    }

    /// Holds the World paused while the player leaves it ([`Play::Leaving`]). Returns what it was
    /// doing before, for [`stay`](Self::stay), or `None` if no World will be open.
    pub fn leave(&mut self) -> Option<Play> {
        let before = self.state()?;
        self.set(Play::Leaving);
        Some(before)
    }

    /// Stops leaving the World: runs it again if it ran before (`back_to`), or pauses it.
    pub fn stay(&mut self, back_to: Play) {
        if self.state() == Some(Play::Leaving) {
            self.set(if back_to == Play::Running {
                Play::Running
            } else {
                // The simulation is frozen already, so the pause completes at once.
                Play::Pausing
            });
        }
    }

    fn complete(&mut self) {
        if self.state() == Some(Play::Pausing) {
            self.set(Play::Paused);
        }
    }

    fn set(&mut self, play: Play) {
        if let Some(next) = self.next.as_mut() {
            next.set_if_different(play);
        }
    }
}

/// The state `next` will make current, if any.
fn pending<S: FreelyMutableState + Copy>(next: &NextState<S>) -> Option<S> {
    match next {
        NextState::Pending(state) | NextState::PendingIfDifferent(state) => Some(*state),
        NextState::Unchanged => None,
    }
}
