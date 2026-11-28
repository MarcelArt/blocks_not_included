//! The game around its Worlds (`docs/plans/19-menus-and-saving.md`): the main menu's New Game and
//! Load Game, renaming and deleting saved Worlds, saving in play, and leaving a World.
//!
//! The menus (`crate::menu`) only draw. What the player asks of them arrives here as
//! [`MenuAction`] messages, and what became of it is in [`MainMenu`], [`CurrentWorld`],
//! [`Saving`] and [`Leaving`]. Tests drive the same seam without a window.
//!
//! # The main menu
//!
//! While no World is open, the main menu shows. Everything it does to the saved Worlds runs as a
//! job on another thread, one job at a time ([`MainMenu::job`]), so the window keeps drawing:
//!
//! - **New Game** reads the rules from the simulation's files, generates the World from its World
//!   Seed, or a random one, and creates it in the [`WorldStore`] with its first save: the
//!   simulation before its first Tick, and the player standing on the ground. The World opens
//!   once that save commits.
//! - **Load Game** lists the saved Worlds in the catalog's order, and opens the one chosen, by its
//!   [`WorldId`]. Renaming keeps the World's row where it is. Deleting asks first.
//!
//! A job that fails leaves the menu where it was, with the reason in [`MainMenu::problem`], and
//! every save as it was.
//!
//! # Saving in play
//!
//! A World opened from the main menu ([`CurrentWorld`]) saves when the player asks
//! ([`MenuAction::Save`]), after every [`Autosave::interval`] of unpaused play, and as the player
//! leaves it. [`Saving`] runs one save at a time, in the order asked: a save asked for while
//! another is under way waits for it, so a newer save never commits behind an older one.
//!
//! # Leaving a World
//!
//! Returning to the main menu, or closing the window, saves the World first. Meanwhile the World
//! stays open and paused ([`Play::Leaving`]). Once the save commits, the World closes, or the game
//! exits. If it fails, the player chooses ([`Leaving`]): try again, go back to the World, or leave
//! without saving.

mod saving;

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use bevy::prelude::*;
use bevy::tasks::futures::check_ready;
use bevy::tasks::{AsyncComputeTaskPool, Task};
use bevy::window::WindowCloseRequested;

use crate::input_policy::InputPolicySystems;
use crate::saves::{StoreError, WorldEntry, WorldId, WorldStore, WorldSummary, random_u64};
use crate::session::{CloseWorld, OpenWorld, PauseControl, Play, Session, WorldStart};
use crate::settings::Settings;
use crate::sim_bridge::SendFocus;
use crate::sim_files::read_rules;
pub use saving::{Autosave, SaveReason, SaveStatus, Saving};

/// Where Worlds are saved, and where a new World's rules come from.
#[derive(Resource, Clone)]
pub struct Library {
    store: Result<WorldStore, String>,
    /// The folder the simulation's files are read from.
    assets: PathBuf,
}

impl Library {
    /// Worlds saved in `store`, and new Worlds simulated under the files in the `assets` folder at
    /// `assets`, as those files are when each World is created.
    pub fn new(store: WorldStore, assets: impl Into<PathBuf>) -> Self {
        Self {
            store: Ok(store),
            assets: assets.into(),
        }
    }

    /// Nowhere to save Worlds, for this reason. The main menu says why, and creates and loads
    /// nothing.
    pub fn unavailable(why: impl Into<String>, assets: impl Into<PathBuf>) -> Self {
        Self {
            store: Err(why.into()),
            assets: assets.into(),
        }
    }

    /// Where Worlds are saved, or why there is nowhere.
    pub fn store(&self) -> Result<&WorldStore, &str> {
        self.store.as_ref().map_err(String::as_str)
    }
}

/// The time on the wall, for the last-played times of saves. Tests set their own.
#[derive(Resource, Clone)]
pub struct WallClock(Arc<dyn Fn() -> SystemTime + Send + Sync>);

impl WallClock {
    pub fn new(now: impl Fn() -> SystemTime + Send + Sync + 'static) -> Self {
        Self(Arc::new(now))
    }

    pub fn now(&self) -> SystemTime {
        (self.0)()
    }
}

impl Default for WallClock {
    fn default() -> Self {
        Self::new(SystemTime::now)
    }
}

/// What the player asks of the menus. Each is handled early in the next frame; one that doesn't
/// fit what the game is doing is ignored.
#[derive(Message, Clone, Debug, PartialEq, Eq)]
pub enum MenuAction {
    /// Lists the saved Worlds again.
    ListWorlds,
    /// Creates a World called `name`, generated from `seed`, or from a random World Seed if
    /// `None`, and opens it once its first save commits.
    NewGame { name: String, seed: Option<u64> },
    /// Selects a World in the list.
    Select(WorldId),
    /// Loads a saved World and opens it.
    LoadGame(WorldId),
    /// Renames a saved World. Its seed, progress and last-played time stay as they are.
    Rename { id: WorldId, name: String },
    /// Asks to delete a saved World: nothing is deleted until [`MenuAction::ConfirmDelete`].
    AskToDelete(WorldId),
    /// Deletes the World asked about, for good.
    ConfirmDelete,
    /// Keeps the World asked about.
    CancelDelete,
    /// Runs the paused World again.
    Resume,
    /// Saves the open World.
    Save,
    /// Saves the open World, closes it, and shows the main menu.
    ReturnToMainMenu,
    /// Quits the game: at once from the main menu, or, with a World open, once it has saved, as
    /// closing the window does.
    Exit,
    /// After the save for leaving failed: tries it again.
    RetryLeaving,
    /// After the save for leaving failed: goes back to the World, running or paused as it was.
    CancelLeaving,
    /// After the save for leaving failed: leaves anyway. The World's last save stays as it was.
    LeaveWithoutSaving,
}

/// The main menu: the saved Worlds, the one selected, and the job under way.
#[derive(Resource, Default)]
pub struct MainMenu {
    /// The saved Worlds in the catalog's order (the most recently played first), as last listed.
    pub worlds: Vec<CatalogRow>,
    /// The World selected in the list. It stays selected across a failed load or a rename.
    pub selected: Option<WorldId>,
    /// The World the player asked to delete, until they confirm or cancel.
    pub confirming_delete: Option<WorldId>,
    /// The job under way. Until it ends, actions that would start another are ignored.
    pub job: Option<MenuJob>,
    /// The latest job that failed, and why. Cleared as the next job starts.
    pub problem: Option<MenuProblem>,
    task: Option<Task<JobOutcome>>,
}

impl MainMenu {
    /// The World `id`'s row in the list.
    pub fn row(&self, id: WorldId) -> Option<&CatalogRow> {
        self.worlds.iter().find(|row| row.id == id)
    }
}

/// A job of the main menu's, run on another thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuJob {
    Listing,
    Creating,
    Loading(WorldId),
    Renaming(WorldId),
    Deleting(WorldId),
}

/// A job that failed, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuProblem {
    pub job: MenuJob,
    pub message: String,
}

/// One saved World in the list.
#[derive(Clone, Debug, PartialEq)]
pub struct CatalogRow {
    pub id: WorldId,
    /// What the catalog says of it, or why its save can't be read. Either way it can be deleted.
    pub world: Result<WorldSummary, String>,
}

impl From<WorldEntry> for CatalogRow {
    fn from(entry: WorldEntry) -> Self {
        Self {
            id: entry.id,
            world: entry.summary.map_err(|error| error.to_string()),
        }
    }
}

/// The saved World open now, as of its latest save. Only a World opened from the main menu has
/// one, and only such a World saves.
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct CurrentWorld(pub WorldSummary);

/// Whether the player is leaving the open World.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct Leaving(pub Option<Departure>);

/// Leaving the open World: where to, and how far it has got.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Departure {
    pub to: Destination,
    /// What the World did before: [`Play::Running`] or [`Play::Paused`]. Cancelling returns to it.
    pub back_to: Play,
    /// Why the save for leaving failed, once it has: the player then chooses what to do. `None`
    /// while it saves.
    pub failed: Option<String>,
}

/// Where a player leaving a World goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Destination {
    MainMenu,
    /// Out of the game.
    Desktop,
}

/// A World Seed as the New Game form reads it: blank for a random one, or a whole number from 0 to
/// `u64::MAX`, in decimal.
pub fn parse_seed(text: &str) -> Result<Option<u64>, SeedError> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    if !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(SeedError::NotANumber);
    }
    text.parse().map(Some).map_err(|_| SeedError::TooLarge)
}

/// Why the New Game form can't read a World Seed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeedError {
    NotANumber,
    TooLarge,
}

impl fmt::Display for SeedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotANumber => f.write_str("a World Seed is a whole number, such as 2026"),
            Self::TooLarge => write!(f, "the largest World Seed is {}", u64::MAX),
        }
    }
}

/// The main menu's jobs, saving in play, and leaving a World. Needs the session, the input
/// policy, the sim bridge, the player, and a [`Library`] resource. The window's close button is
/// left to it: give `WindowPlugin` `close_when_requested: false`.
pub struct LifecyclePlugin;

impl Plugin for LifecyclePlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<MenuAction>()
            .add_message::<WindowCloseRequested>()
            .init_resource::<MainMenu>()
            .init_resource::<WallClock>()
            .init_resource::<Saving>()
            .init_resource::<Autosave>()
            .init_resource::<Leaving>()
            .init_resource::<Settings>()
            .add_systems(Startup, report_library)
            .add_systems(OnEnter(Session::Closed), list_worlds)
            .add_systems(OnExit(Session::Open), forget_world)
            .add_systems(
                PreUpdate,
                (handle_actions, finish_menu_job)
                    .chain()
                    .after(InputPolicySystems),
            )
            .add_systems(
                Update,
                (
                    saving::follow_autosave_setting.run_if(resource_changed::<Settings>),
                    saving::count_play_time
                        .run_if(in_state(Play::Running).and_then(resource_exists::<CurrentWorld>)),
                )
                    .chain(),
            )
            .add_systems(
                PostUpdate,
                saving::run_saves
                    .after(SendFocus)
                    .run_if(in_state(Session::Open)),
            );
    }
}

/// Logs where Worlds are saved, or why they can't be.
fn report_library(library: Res<Library>) {
    match library.store() {
        Ok(store) => info!("Worlds are saved in {}", store.dir().display()),
        Err(why) => error!("Worlds can't be saved: {why}"),
    }
}

/// Lists the saved Worlds as the main menu shows.
fn list_worlds(mut menu: ResMut<MainMenu>, library: Res<Library>) {
    menu.confirming_delete = None;
    if menu.job.is_none() {
        list_worlds_now(&mut menu, &library);
    }
}

/// Forgets the World as it closes.
fn forget_world(
    mut commands: Commands,
    mut saving: ResMut<Saving>,
    mut autosave: ResMut<Autosave>,
    mut leaving: ResMut<Leaving>,
) {
    commands.remove_resource::<CurrentWorld>();
    saving.reset();
    autosave.since_save = default();
    *leaving = Leaving::default();
}

/// What a finished job of the main menu's brought back.
enum JobOutcome {
    Listed(Result<Vec<CatalogRow>, String>),
    /// A World to open, created or loaded.
    Opened(Result<Box<(WorldSummary, WorldStart)>, String>),
    Renamed(Result<WorldSummary, String>),
    Deleted(Result<(), String>),
}

/// Starts `job`, which does `work` with the store on another thread. Fails it at once if there is
/// nowhere to save Worlds.
fn start_job(
    menu: &mut MainMenu,
    library: &Library,
    job: MenuJob,
    work: impl FnOnce(&WorldStore) -> JobOutcome + Send + 'static,
) {
    menu.problem = None;
    let store = match library.store() {
        Ok(store) => store.clone(),
        Err(why) => {
            menu.problem = Some(MenuProblem {
                job,
                message: format!("there is nowhere to save Worlds: {why}"),
            });
            return;
        }
    };
    menu.job = Some(job);
    menu.task = Some(AsyncComputeTaskPool::get().spawn(async move { work(&store) }));
}

/// Handles the frame's [`MenuAction`]s, and the window's close button.
#[expect(clippy::too_many_arguments, reason = "a system's parameters")]
fn handle_actions(
    mut actions: MessageReader<MenuAction>,
    mut close_requests: MessageReader<WindowCloseRequested>,
    mut menu: ResMut<MainMenu>,
    library: Res<Library>,
    clock: Res<WallClock>,
    mut play: PauseControl,
    current: Option<Res<CurrentWorld>>,
    mut leaving: ResMut<Leaving>,
    mut saving: ResMut<Saving>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    let mut world = InWorld {
        play: &mut play,
        saved: current.is_some(),
        leaving: &mut leaving,
        saving: &mut saving,
    };
    let in_menu = world.play.state().is_none();
    let mut go_to = None;
    for action in actions.read().cloned() {
        match action {
            MenuAction::Resume => world.play.resume(),
            MenuAction::Save => world.save(),
            MenuAction::ReturnToMainMenu if !in_menu => {
                go_to = world.leave(Destination::MainMenu).or(go_to);
            }
            MenuAction::Exit if !in_menu => go_to = world.leave(Destination::Desktop).or(go_to),
            MenuAction::Exit => go_to = Some(Destination::Desktop),
            MenuAction::RetryLeaving => world.retry(),
            MenuAction::CancelLeaving => world.cancel(),
            MenuAction::LeaveWithoutSaving => go_to = world.leave_without_saving().or(go_to),
            menu_action if in_menu => handle_menu_action(&mut menu, &library, &clock, menu_action),
            ignored => debug!("ignored {ignored:?} with a World open"),
        }
    }
    if close_requests.read().count() > 0 {
        go_to = if in_menu {
            Some(Destination::Desktop)
        } else {
            world.leave(Destination::Desktop).or(go_to)
        };
    }
    if let Some(to) = go_to {
        go(to, &mut commands, &mut exit);
    }
}

/// An action of the main menu's, while no World is open.
fn handle_menu_action(
    menu: &mut MainMenu,
    library: &Library,
    clock: &WallClock,
    action: MenuAction,
) {
    match action {
        MenuAction::Select(id) => menu.selected = Some(id),
        MenuAction::CancelDelete => menu.confirming_delete = None,
        _ if menu.job.is_some() => debug!("ignored {action:?}: the menu is busy"),
        MenuAction::ListWorlds => list_worlds_now(menu, library),
        MenuAction::NewGame { name, seed } => {
            if name.trim().is_empty() {
                menu.problem = Some(MenuProblem {
                    job: MenuJob::Creating,
                    message: StoreError::EmptyName.to_string(),
                });
                return;
            }
            let assets = library.assets.clone();
            let now = clock.now();
            start_job(menu, library, MenuJob::Creating, move |store| {
                JobOutcome::Opened(create_world(store, &assets, &name, seed, now).map(Box::new))
            });
        }
        MenuAction::LoadGame(id) => {
            menu.selected = Some(id);
            start_job(menu, library, MenuJob::Loading(id), move |store| {
                JobOutcome::Opened(load_world(store, id).map(Box::new))
            });
        }
        MenuAction::Rename { id, name } => {
            menu.selected = Some(id);
            if name.trim().is_empty() {
                menu.problem = Some(MenuProblem {
                    job: MenuJob::Renaming(id),
                    message: StoreError::EmptyName.to_string(),
                });
                return;
            }
            start_job(menu, library, MenuJob::Renaming(id), move |store| {
                JobOutcome::Renamed(store.rename(&id, &name).map_err(|error| error.to_string()))
            });
        }
        MenuAction::AskToDelete(id) => {
            menu.selected = Some(id);
            menu.confirming_delete = Some(id);
        }
        MenuAction::ConfirmDelete => {
            if let Some(id) = menu.confirming_delete {
                start_job(menu, library, MenuJob::Deleting(id), move |store| {
                    JobOutcome::Deleted(store.delete(&id).map_err(|error| error.to_string()))
                });
            }
        }
        MenuAction::Resume
        | MenuAction::Save
        | MenuAction::ReturnToMainMenu
        | MenuAction::Exit
        | MenuAction::RetryLeaving
        | MenuAction::CancelLeaving
        | MenuAction::LeaveWithoutSaving => {}
    }
}

fn list_worlds_now(menu: &mut MainMenu, library: &Library) {
    start_job(menu, library, MenuJob::Listing, |store| {
        JobOutcome::Listed(
            store
                .list()
                .map(|entries| entries.into_iter().map(CatalogRow::from).collect())
                .map_err(|error| format!("can't list the saved Worlds: {error}")),
        )
    });
}

/// Creates a World with its first save, and what it opens from.
fn create_world(
    store: &WorldStore,
    assets: &std::path::Path,
    name: &str,
    seed: Option<u64>,
    now: SystemTime,
) -> Result<(WorldSummary, WorldStart), String> {
    let (elements, settings) = read_rules(assets).map_err(|error| error.to_string())?;
    let seed = seed.unwrap_or_else(random_u64);
    let (start, save) =
        WorldStart::new_game(seed, elements, settings).map_err(|error| error.to_string())?;
    let summary = store
        .create(name, &save, now)
        .map_err(|error| error.to_string())?;
    info!(
        "created World {} \"{}\" from seed {seed}",
        summary.id, summary.name
    );
    Ok((summary, start))
}

/// Loads a saved World, and what it opens from.
fn load_world(store: &WorldStore, id: WorldId) -> Result<(WorldSummary, WorldStart), String> {
    let loaded = store.load(&id).map_err(|error| error.to_string())?;
    let start = WorldStart::load_game(loaded.save).map_err(|error| error.to_string())?;
    Ok((loaded.summary, start))
}

/// Takes in the main menu's job once it ends: opens the World it made or loaded, or updates the
/// list, or reports why it failed.
fn finish_menu_job(mut menu: ResMut<MainMenu>, mut commands: Commands) {
    let Some(task) = menu.task.as_mut() else {
        return;
    };
    let Some(outcome) = check_ready(task) else {
        return;
    };
    menu.task = None;
    let job = menu
        .job
        .take()
        .expect("every job's task has the job beside it");
    let failure = match outcome {
        JobOutcome::Listed(listed) => listed.map(|worlds| {
            let listed = |id: &WorldId| worlds.iter().any(|row| row.id == *id);
            if !menu.selected.as_ref().is_some_and(listed) {
                menu.selected = None;
            }
            if !menu.confirming_delete.as_ref().is_some_and(listed) {
                menu.confirming_delete = None;
            }
            menu.worlds = worlds;
        }),
        JobOutcome::Opened(opened) => opened.map(|opened| {
            let (summary, start) = *opened;
            menu.selected = Some(summary.id);
            commands.insert_resource(CurrentWorld(summary));
            commands.queue(OpenWorld(start));
        }),
        JobOutcome::Renamed(renamed) => renamed.map(|summary| {
            // The row stays where it is, as the player last saw the list.
            if let Some(row) = menu.worlds.iter_mut().find(|row| row.id == summary.id) {
                row.world = Ok(summary);
            }
        }),
        JobOutcome::Deleted(deleted) => deleted.map(|()| {
            if let MenuJob::Deleting(id) = job {
                menu.worlds.retain(|row| row.id != id);
                if menu.selected == Some(id) {
                    menu.selected = None;
                }
            }
            menu.confirming_delete = None;
        }),
    };
    if let Err(message) = failure {
        warn!("{job:?} failed: {message}");
        if matches!(job, MenuJob::Deleting(_)) {
            menu.confirming_delete = None;
        }
        menu.problem = Some(MenuProblem { job, message });
    }
}

/// What the actions for the open World act on.
struct InWorld<'a, 'w> {
    play: &'a mut PauseControl<'w>,
    /// Whether the World is saved anywhere: whether it has a [`CurrentWorld`].
    saved: bool,
    leaving: &'a mut Leaving,
    saving: &'a mut Saving,
}

impl InWorld<'_, '_> {
    /// Saves the World, unless it is being left.
    fn save(&mut self) {
        if self.saved && self.leaving.0.is_none() && self.play.state().is_some() {
            self.saving.request(SaveReason::Manual);
        }
    }

    /// Leaves the World for `to` once it has saved, holding it paused until then. Returns `to`
    /// if the World is to be left now: a World saved nowhere is left at once.
    fn leave(&mut self, to: Destination) -> Option<Destination> {
        if let Some(departure) = &mut self.leaving.0 {
            // Quitting outranks returning to the menu.
            if to == Destination::Desktop {
                departure.to = to;
            }
            return None;
        }
        if !self.saved {
            return Some(to);
        }
        let before = self.play.leave()?;
        let back_to = if before == Play::Running {
            Play::Running
        } else {
            Play::Paused
        };
        self.leaving.0 = Some(Departure {
            to,
            back_to,
            failed: None,
        });
        self.saving.request(SaveReason::Leaving);
        None
    }

    fn retry(&mut self) {
        if let Some(departure) = &mut self.leaving.0
            && departure.failed.take().is_some()
        {
            self.saving.request(SaveReason::Leaving);
        }
    }

    fn cancel(&mut self) {
        if let Some(departure) = self
            .leaving
            .0
            .take_if(|departure| departure.failed.is_some())
        {
            self.play.stay(departure.back_to);
        }
    }

    /// Where to go now, once the save for leaving has failed.
    fn leave_without_saving(&mut self) -> Option<Destination> {
        let departure = self.leaving.0.as_ref()?;
        departure.failed.as_ref()?;
        warn!("leaving the World without saving: its last save stays as it was");
        Some(departure.to)
    }
}

/// Closes the World and shows the main menu, or quits the game.
fn go(to: Destination, commands: &mut Commands, exit: &mut MessageWriter<AppExit>) {
    match to {
        Destination::MainMenu => commands.queue(CloseWorld),
        Destination::Desktop => {
            exit.write(AppExit::Success);
        }
    }
}
