//! World saves: each World's progress on disk, and the catalog that lists them
//! (`docs/plans/17-world-save-storage.md`, ADR 0005).
//!
//! A [`WorldStore`] keeps every World in a file of its own, `<id>.world`, in one folder, such as
//! [`UserDirs::saves`](crate::user_dirs::UserDirs::saves). The store names each World with a
//! [`WorldId`] it generates. A World's name is only what the catalog shows: names may repeat, and
//! no name decides where anything is written.
//!
//! A save holds the World's [`SimCapture`] and the player's [`PlayerState`]. The capture holds the
//! simulation's checkpoint, the rules it ran under, and the World Seed and Elements its terrain is
//! generated from. Terrain the generator can make again isn't stored (ADR 0005). The layout is in
//! `format.rs`.
//!
//! # Committing
//!
//! Every write stages a whole new file beside the World's committed one and flushes it to disk.
//! Then it renames the staged file over the committed one, which Linux and Windows both do in one
//! step. A write that fails or is cut short at any point leaves the last committed save as it was.
//! Writes to one World take turns, and a [`SaveTicket`] stops a save that finishes late from
//! overwriting a newer one.
//!
//! # Compatibility
//!
//! A save records the revision of its file format, of the generation rules
//! ([`worldgen::RULES_REVISION`]), and of the simulation rules ([`sim::RULES_REVISION`]). A build
//! loads only its own revisions, and reports which one differs. A save also holds the Elements
//! and settings themselves, which the store's [`RulesPolicy`] checks. Reading a save never changes
//! it, whether it loads or not.

mod format;

use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;
use std::fs::{self, File};
use std::hash::{BuildHasher, RandomState};
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bevy::math::{EulerRot, Quat};
use serde::{Deserialize, Serialize};
use sim::{ElementTable, SimSettings};

use crate::atomic_write::{stage, sync_dir};
use crate::player::movement::Body;
use crate::sim_bridge::{Generation, SimCapture};
use format::Header;

/// Names one World in a [`WorldStore`], for as long as it exists: 16 lowercase hexadecimal
/// digits, which the store generates. It is also the name of the World's file, so it is safe in
/// a path on every system.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorldId(u64);

impl WorldId {
    /// The id `text` writes, if it is one: exactly 16 lowercase hexadecimal digits.
    pub fn parse(text: &str) -> Option<WorldId> {
        let digits = text.len() == 16
            && text
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'));
        digits
            .then(|| u64::from_str_radix(text, 16).ok().map(WorldId))
            .flatten()
    }

    /// A new id, picked at random.
    fn generate() -> WorldId {
        WorldId(random_u64())
    }
}

impl fmt::Display for WorldId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

/// How a World is played. Only the sandbox exists so far.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameMode {
    #[serde(rename = "sandbox")]
    Sandbox,
}

/// The player as a save keeps them: enough to carry on moving exactly as they were.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerState {
    /// Where the body is, how fast it is moving, whether it flies, and whether it stands on the
    /// ground.
    pub body: Body,
    /// How far the view has turned about the vertical, in radians, as `EulerRot::YXZ` gives it.
    pub yaw: f32,
    /// How far the view tilts up or down, in radians.
    pub pitch: f32,
}

impl PlayerState {
    /// The player with `body`, whose camera is turned by `rotation`: about the vertical, then up
    /// or down, as mouse look turns it.
    pub fn new(body: Body, rotation: Quat) -> Self {
        let (yaw, pitch, _) = rotation.to_euler(EulerRot::YXZ);
        Self { body, yaw, pitch }
    }

    /// The camera's rotation.
    pub fn rotation(&self) -> Quat {
        Quat::from_euler(EulerRot::YXZ, self.yaw, self.pitch, 0.0)
    }
}

/// Everything a World's save holds besides what the catalog lists.
#[derive(Clone, Debug, PartialEq)]
pub struct WorldSave {
    /// The simulation, its rules, and what its terrain comes from. Its `generation` must be set:
    /// a World's terrain comes from its World Seed.
    pub capture: SimCapture,
    pub player: PlayerState,
}

/// What the catalog lists for a World: enough to show and choose it, without loading it.
#[derive(Clone, Debug, PartialEq)]
pub struct WorldSummary {
    pub id: WorldId,
    pub name: String,
    pub seed: u64,
    /// When the World was last saved in play, to the millisecond.
    pub last_played: SystemTime,
    pub mode: GameMode,
    /// Whether this build can load the World, as far as its header tells: a release build may
    /// still refuse its rules when it loads.
    pub compatibility: Result<(), Incompatibility>,
}

/// One World in the catalog.
#[derive(Debug)]
pub struct WorldEntry {
    pub id: WorldId,
    /// What it holds, or why it can't be read. A World that can't be read can still be deleted.
    pub summary: Result<WorldSummary, StoreError>,
}

/// A World read back from its save.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadedWorld {
    pub summary: WorldSummary,
    pub save: WorldSave,
}

/// A place in line for saving one World, taken when the state to save is sampled. A save whose
/// ticket is older than one already committed is dropped, so a slow save can't overwrite a
/// newer one. Tickets belong to the store that issued them.
#[derive(Debug, PartialEq, Eq)]
pub struct SaveTicket {
    id: WorldId,
    order: u64,
}

impl SaveTicket {
    /// The World this ticket saves.
    pub fn id(&self) -> WorldId {
        self.id
    }
}

/// Which rules a store loads Worlds under.
#[derive(Clone, Debug, PartialEq)]
pub enum RulesPolicy {
    /// For development builds: each World comes back with the rules it was saved with, whatever
    /// the asset files now say. Its generator keeps the Elements it started with, and its
    /// simulation the last Elements and settings it accepted.
    AsSaved,
    /// For release builds, which ship one set of rules: a World loads only if it was generated
    /// and simulated under exactly these Elements and settings.
    Shipped {
        elements: ElementTable,
        settings: SimSettings,
    },
}

/// Why a build can't load a World. Loading it leaves it untouched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Incompatibility {
    /// The save's file is laid out in a format revision this build can't read.
    Format { saved: u32, supported: u32 },
    /// The World's terrain was generated under another revision of the generation rules.
    GenerationRules { saved: u32, supported: u32 },
    /// The World was simulated under another revision of the simulation rules.
    SimulationRules { saved: u32, supported: u32 },
    /// The World's terrain was generated from Elements other than the ones this build ships.
    GeneratorElements,
    /// The World was simulated with Elements other than the ones this build ships.
    SimulationElements,
    /// The World was simulated with settings other than the ones this build ships.
    SimulationSettings,
}

impl fmt::Display for Incompatibility {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Format { saved, supported } => write!(
                f,
                "it was saved in file format {saved}, and this version reads format {supported}"
            ),
            Self::GenerationRules { saved, supported } => write!(
                f,
                "its terrain comes from revision {saved} of the generation rules, and this \
                 version has revision {supported}"
            ),
            Self::SimulationRules { saved, supported } => write!(
                f,
                "it was simulated under revision {saved} of the simulation rules, and this \
                 version has revision {supported}"
            ),
            Self::GeneratorElements => {
                f.write_str("its terrain was generated from other Elements than this version's")
            }
            Self::SimulationElements => {
                f.write_str("it was simulated with other Elements than this version's")
            }
            Self::SimulationSettings => {
                f.write_str("it was simulated with other settings than this version's")
            }
        }
    }
}

/// Why a [`WorldStore`] operation failed. A failed write leaves the World's last committed save
/// as it was.
#[derive(Debug)]
pub enum StoreError {
    /// A World's name is blank once trimmed.
    EmptyName,
    /// There is no World with this id: it was deleted, or never created.
    NotFound(WorldId),
    /// This build can't load the World.
    Incompatible(Incompatibility),
    /// The World's save is damaged, for this reason.
    Corrupt(String),
    /// The save to write has no World Seed: its terrain doesn't come from the world generator.
    NotGenerated,
    /// The save to write comes from another World Seed than the World's.
    SeedMismatch { world: u64, save: u64 },
    /// A newer save of the World was committed first, so this one was dropped.
    Superseded,
    /// Reading or writing the World's file failed.
    Io(io::Error),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyName => f.write_str("a World's name can't be blank"),
            Self::NotFound(id) => write!(f, "World {id} doesn't exist: it may have been deleted"),
            Self::Incompatible(why) => write!(f, "this version of the game can't load it: {why}"),
            Self::Corrupt(why) => write!(f, "the save is damaged: {why}"),
            Self::NotGenerated => {
                f.write_str("only a World whose terrain comes from a World Seed can be saved")
            }
            Self::SeedMismatch { world, save } => write!(
                f,
                "the World's seed is {world}, but the state to save comes from seed {save}"
            ),
            Self::Superseded => f.write_str(
                "a newer save of this World was committed first, so this one was dropped",
            ),
            Self::Io(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

/// A hook run on the store's own thread after a write is staged and before it is published.
type PublishHook = Box<dyn Fn(&WorldId) -> io::Result<()> + Send + Sync>;

/// The Worlds in one folder: creates, lists, loads, saves, renames and deletes them. Cloning it
/// shares the same store, and every method can be called from any thread.
///
/// Open one store per folder: writes to a World take turns within a store, not across stores.
#[derive(Clone)]
pub struct WorldStore {
    inner: Arc<Inner>,
}

struct Inner {
    dir: PathBuf,
    rules: RulesPolicy,
    /// Each World's turn at writing, once it has been written or ticketed through this store.
    worlds: Mutex<HashMap<WorldId, Arc<Mutex<Turns>>>>,
    before_publish: Option<PublishHook>,
}

/// One World's writes through a store. Holding its lock is a turn at writing.
#[derive(Default)]
struct Turns {
    /// The order of the latest ticket issued.
    issued: u64,
    /// The order of the latest ticket whose save committed.
    committed: u64,
}

impl WorldStore {
    /// The store of the Worlds in `dir`, which is created if it doesn't exist. Worlds load under
    /// `rules`.
    pub fn open(dir: impl Into<PathBuf>, rules: RulesPolicy) -> io::Result<WorldStore> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        Ok(WorldStore {
            inner: Arc::new(Inner {
                dir,
                rules,
                worlds: Mutex::default(),
                before_publish: None,
            }),
        })
    }

    /// Runs `hook` in every write after the new file is staged and before it is published. An
    /// error from it fails the write there, as a full disk or a crash would. For tests.
    ///
    /// # Panics
    ///
    /// If the store has been cloned.
    pub fn before_publish(
        mut self,
        hook: impl Fn(&WorldId) -> io::Result<()> + Send + Sync + 'static,
    ) -> Self {
        Arc::get_mut(&mut self.inner)
            .expect("the hook is set before the store is shared")
            .before_publish = Some(Box::new(hook));
        self
    }

    /// The folder the Worlds are in.
    pub fn dir(&self) -> &Path {
        &self.inner.dir
    }

    /// Where the World `id` is saved.
    pub fn path_of(&self, id: &WorldId) -> PathBuf {
        self.inner.dir.join(format!("{id}.world"))
    }

    /// Every World in the folder: the most recently played first, then by name, then by id. Worlds
    /// that can't be read come last, by id. Reads only each save's header, never its Cells.
    ///
    /// Fails only if the folder itself can't be read.
    pub fn list(&self) -> io::Result<Vec<WorldEntry>> {
        let mut entries = Vec::new();
        for entry in fs::read_dir(&self.inner.dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(id) = name
                .to_str()
                .and_then(|name| name.strip_suffix(".world"))
                .and_then(WorldId::parse)
            else {
                continue;
            };
            if !entry.path().is_file() {
                continue;
            }
            match self.read_header(&id) {
                // Deleted since the folder was read.
                Err(StoreError::NotFound(_)) => {}
                read => entries.push(WorldEntry {
                    id,
                    summary: read.map(|header| summary(id, &header)),
                }),
            }
        }
        entries.sort_by(catalog_order);
        Ok(entries)
    }

    /// Creates a World called `name`, trimmed, whose first save is `save`, last played at `now`.
    /// The World exists once that save commits: if it fails, there is no World.
    pub fn create(
        &self,
        name: &str,
        save: &WorldSave,
        now: SystemTime,
    ) -> Result<WorldSummary, StoreError> {
        let name = valid_name(name)?;
        let generation = generation_of(save)?;
        let body = format::encode_body(save, generation);
        let header = Header::new(name.to_owned(), generation.seed, millis(now), &body);
        let file = format::assemble(&header, &body);
        loop {
            let id = WorldId::generate();
            let world = self.world(&id);
            let _turn = lock(&world);
            // A clash among 2⁶⁴ ids is all but impossible, but never overwrite a World.
            if self.path_of(&id).try_exists().map_err(StoreError::Io)? {
                continue;
            }
            self.publish(&id, &file)?;
            return Ok(summary(id, &header));
        }
    }

    /// Reads the World `id` back. Leaves the save as it is, whether it loads or not.
    pub fn load(&self, id: &WorldId) -> Result<LoadedWorld, StoreError> {
        let bytes = fs::read(self.path_of(id)).map_err(|error| io_error(id, error))?;
        let (header, body) = format::split(&bytes)?;
        compatibility(&header).map_err(StoreError::Incompatible)?;
        let save = format::decode_body(&header, body, |saved| self.check_rules(saved))?;
        Ok(LoadedWorld {
            summary: summary(*id, &header),
            save,
        })
    }

    /// A place in line for saving the World `id`. Take it when the state to save is sampled,
    /// before it is captured and encoded.
    pub fn ticket(&self, id: &WorldId) -> SaveTicket {
        let world = self.world(id);
        let mut turns = lock(&world);
        turns.issued += 1;
        SaveTicket {
            id: *id,
            order: turns.issued,
        }
    }

    /// Saves `save` as the World the ticket names, last played at `now`, keeping the World's name.
    ///
    /// Drops the save with [`StoreError::Superseded`] if a save with a newer ticket has already
    /// committed. Fails if the World has been deleted, or if `save` comes from another World
    /// Seed.
    pub fn save(
        &self,
        ticket: SaveTicket,
        save: &WorldSave,
        now: SystemTime,
    ) -> Result<WorldSummary, StoreError> {
        let generation = generation_of(save)?;
        let world = self.world(&ticket.id);
        let superseded = |turns: &Turns| ticket.order <= turns.committed;
        // Encoding takes a while, so it is skipped if it would be wasted, and done outside the
        // World's turn.
        if superseded(&lock(&world)) {
            return Err(StoreError::Superseded);
        }
        let body = format::encode_body(save, generation);

        let mut turns = lock(&world);
        if superseded(&turns) {
            return Err(StoreError::Superseded);
        }
        let committed = self.read_header(&ticket.id)?;
        if committed.seed != generation.seed {
            return Err(StoreError::SeedMismatch {
                world: committed.seed,
                save: generation.seed,
            });
        }
        let header = Header::new(committed.name, generation.seed, millis(now), &body);
        self.publish(&ticket.id, &format::assemble(&header, &body))?;
        turns.committed = ticket.order;
        Ok(summary(ticket.id, &header))
    }

    /// Renames the World `id` to `name`, trimmed. Its terrain, progress and last-played time stay
    /// as they are.
    pub fn rename(&self, id: &WorldId, name: &str) -> Result<WorldSummary, StoreError> {
        let name = valid_name(name)?;
        let world = self.world(id);
        let _turn = lock(&world);
        let bytes = fs::read(self.path_of(id)).map_err(|error| io_error(id, error))?;
        let (mut header, body) = format::split(&bytes)?;
        header.name = name.to_owned();
        self.publish(id, &format::assemble(&header, body))?;
        Ok(summary(*id, &header))
    }

    /// Deletes the World `id`, for good, whether or not its save can be read.
    pub fn delete(&self, id: &WorldId) -> Result<(), StoreError> {
        let world = self.world(id);
        let _turn = lock(&world);
        fs::remove_file(self.path_of(id)).map_err(|error| io_error(id, error))?;
        self.remove_staged(id);
        sync_dir(&self.inner.dir);
        Ok(())
    }

    /// The World `id`'s turns at writing.
    fn world(&self, id: &WorldId) -> Arc<Mutex<Turns>> {
        Arc::clone(lock(&self.inner.worlds).entry(*id).or_default())
    }

    /// Reads the header of the World `id`'s save.
    fn read_header(&self, id: &WorldId) -> Result<Header, StoreError> {
        let file = File::open(self.path_of(id)).map_err(|error| io_error(id, error))?;
        let len = file.metadata().map_err(StoreError::Io)?.len();
        format::read_header(&mut BufReader::new(file), len)
    }

    /// Checks a save's rules against the store's policy.
    fn check_rules(&self, saved: &format::SavedRules) -> Result<(), StoreError> {
        let RulesPolicy::Shipped { elements, settings } = &self.inner.rules else {
            return Ok(());
        };
        let refusal = if saved.generator_elements != *elements {
            Some(Incompatibility::GeneratorElements)
        } else if saved.elements != *elements {
            Some(Incompatibility::SimulationElements)
        } else if saved.settings != *settings {
            Some(Incompatibility::SimulationSettings)
        } else {
            None
        };
        refusal.map_or(Ok(()), |why| Err(StoreError::Incompatible(why)))
    }

    /// Commits `bytes` as the World `id`'s save: stages them in a file of their own beside it,
    /// flushes that to disk, and renames it over the save. Until the rename, the save is as it
    /// was, so a failure or a crash before then loses only the new save.
    fn publish(&self, id: &WorldId, bytes: &[u8]) -> Result<(), StoreError> {
        // Left behind by writes a crash cut short.
        self.remove_staged(id);
        let staged = self
            .inner
            .dir
            .join(format!(".{id}.{:016x}.staged", random_u64()));
        let published = stage(&staged, bytes)
            .and_then(|()| match &self.inner.before_publish {
                Some(hook) => hook(id),
                None => Ok(()),
            })
            .and_then(|()| fs::rename(&staged, self.path_of(id)));
        if let Err(error) = published {
            let _ = fs::remove_file(&staged);
            return Err(StoreError::Io(error));
        }
        sync_dir(&self.inner.dir);
        Ok(())
    }

    /// Removes the World `id`'s staged files, if any. They were never published, so nothing
    /// depends on them.
    fn remove_staged(&self, id: &WorldId) {
        let Ok(entries) = fs::read_dir(&self.inner.dir) else {
            return;
        };
        let prefix = format!(".{id}.");
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if name.starts_with(&prefix) && name.ends_with(".staged") {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
}

/// `name`, trimmed, if anything is left.
fn valid_name(name: &str) -> Result<&str, StoreError> {
    let name = name.trim();
    if name.is_empty() {
        Err(StoreError::EmptyName)
    } else {
        Ok(name)
    }
}

/// What a save's terrain comes from.
fn generation_of(save: &WorldSave) -> Result<&Generation, StoreError> {
    save.capture
        .generation
        .as_ref()
        .ok_or(StoreError::NotGenerated)
}

/// Whether this build has the rules revisions `header` names.
fn compatibility(header: &Header) -> Result<(), Incompatibility> {
    if header.generation_rules != worldgen::RULES_REVISION {
        return Err(Incompatibility::GenerationRules {
            saved: header.generation_rules,
            supported: worldgen::RULES_REVISION,
        });
    }
    if header.simulation_rules != sim::RULES_REVISION {
        return Err(Incompatibility::SimulationRules {
            saved: header.simulation_rules,
            supported: sim::RULES_REVISION,
        });
    }
    Ok(())
}

fn summary(id: WorldId, header: &Header) -> WorldSummary {
    WorldSummary {
        id,
        name: header.name.clone(),
        seed: header.seed,
        last_played: UNIX_EPOCH + Duration::from_millis(header.last_played_ms),
        mode: header.mode,
        compatibility: compatibility(header),
    }
}

/// The catalog's order: the most recently played first, then by name, then by id, and Worlds
/// that can't be read last, by id.
fn catalog_order(a: &WorldEntry, b: &WorldEntry) -> Ordering {
    match (&a.summary, &b.summary) {
        (Ok(a), Ok(b)) => b
            .last_played
            .cmp(&a.last_played)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.id.cmp(&b.id)),
        (Ok(_), Err(_)) => Ordering::Less,
        (Err(_), Ok(_)) => Ordering::Greater,
        (Err(_), Err(_)) => a.id.cmp(&b.id),
    }
}

/// `time` in milliseconds since the Unix epoch, or 0 before it.
fn millis(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH).map_or(0, |since| {
        u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
    })
}

/// `error` from reading or writing the World `id`'s save, as a [`StoreError`].
fn io_error(id: &WorldId, error: io::Error) -> StoreError {
    match error.kind() {
        io::ErrorKind::NotFound => StoreError::NotFound(*id),
        _ => StoreError::Io(error),
    }
}

/// Locks `mutex`, whether or not a thread panicked holding it: the counts it guards stay valid.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// 64 random bits: from the system's randomness, which seeds `RandomState`, mixed with the time
/// and a count so no two calls repeat.
pub(crate) fn random_u64() -> u64 {
    static CALLS: AtomicU64 = AtomicU64::new(0);
    let call = CALLS.fetch_add(1, AtomicOrdering::Relaxed);
    RandomState::new().hash_one((SystemTime::now(), call, std::process::id()))
}
