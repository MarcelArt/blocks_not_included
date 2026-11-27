//! The simulation's data files, `assets/elements.ron` and `assets/sim_settings.ron`, loaded as
//! assets so that editing one while the game runs changes the simulation. Development builds
//! watch `assets/` for changes: this crate's default `dev` feature turns on Bevy's `file_watcher`.
//!
//! The assets load, and are watched, for as long as the game runs. A new World's simulation starts
//! from the files as [`SimStart::sandbox`] reads them, and a saved World's from the rules it was
//! saved with. Either way, only a version of a file that loads while the World is open counts:
//! it goes to that World's simulation as a [`SimCommand::ReplaceElements`] or
//! [`SimCommand::ReplaceSettings`]. Versions that load while no World is open, or as one opens,
//! are dropped, so a saved World keeps its own rules until a file changes during play. While the
//! World is paused, the latest version of each file is held back, and sent once it runs again.
//!
//! A version that fails to parse or validate, or that the simulation rejects, is reported in
//! [`SimFileStatus`] and the log, and the simulation keeps the version before. Once the
//! simulation takes up an Element table, the sim bridge puts it in
//! [`Elements`](crate::sim_bridge::Elements), and every Chunk is remeshed in its colours.
//!
//! [`SimStart::sandbox`]: crate::sim_bridge::SimStart::sandbox

use std::fmt;
use std::marker::PhantomData;
use std::str::Utf8Error;

use bevy::asset::io::Reader;
use bevy::asset::{AssetLoadError, AssetLoadFailedEvent, AssetLoader, LoadContext};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use sim::{ElementError, ElementTable, SettingsError, SimSettings};

use crate::session::{Play, Session, SessionSystems};
use crate::sim_bridge::{ReplaceOutcome, SimBridge, SimCommand, SimReport};

/// Loads the simulation's data files as assets, sends the open World's simulation every new
/// version, and reports what became of it in [`SimFileStatus`]. Needs the session, the sim bridge
/// and Bevy's `AssetPlugin`.
pub struct SimFilesPlugin;

impl Plugin for SimFilesPlugin {
    fn build(&self, app: &mut App) {
        register::<ElementsFile>(app);
        register::<SettingsFile>(app);
        app.init_resource::<SimFileStatus>()
            .add_systems(Startup, (load::<ElementsFile>, load::<SettingsFile>))
            // Each World starts as its files did.
            .add_systems(
                OnEnter(Session::Open),
                (|mut status: ResMut<SimFileStatus>| *status = SimFileStatus::default())
                    .in_set(SessionSystems::Build),
            )
            .add_systems(
                Update,
                (
                    send_new_versions::<ElementsFile>,
                    send_new_versions::<SettingsFile>,
                    record_failures::<ElementsFile>,
                    record_failures::<SettingsFile>,
                    record_outcomes.run_if(in_state(Session::Open)),
                ),
            );
    }
}

/// One of the simulation's data files, as an asset.
pub trait SimFile: Asset + Clone {
    /// Where it is, within `assets/`.
    const PATH: &'static str;

    /// Parses and validates the file's contents.
    fn parse(bytes: &[u8]) -> Result<Self, SimFileError>;

    /// The command that has the simulation take up this version.
    fn replace_command(&self) -> SimCommand;

    /// What became of the file's latest version.
    fn status(status: &mut SimFileStatus) -> &mut FileStatus;
}

/// `assets/elements.ron`: every Element the simulation knows about.
#[derive(Asset, TypePath, Clone, Debug)]
pub struct ElementsFile(pub ElementTable);

impl SimFile for ElementsFile {
    const PATH: &'static str = "elements.ron";

    fn parse(bytes: &[u8]) -> Result<Self, SimFileError> {
        let text = std::str::from_utf8(bytes).map_err(SimFileError::NotText)?;
        ElementTable::from_ron(text)
            .map(Self)
            .map_err(SimFileError::Elements)
    }

    fn replace_command(&self) -> SimCommand {
        SimCommand::ReplaceElements(self.0.clone())
    }

    fn status(status: &mut SimFileStatus) -> &mut FileStatus {
        &mut status.elements
    }
}

/// `assets/sim_settings.ron`: the simulation's tuning values.
#[derive(Asset, TypePath, Clone, Debug)]
pub struct SettingsFile(pub SimSettings);

impl SimFile for SettingsFile {
    const PATH: &'static str = "sim_settings.ron";

    fn parse(bytes: &[u8]) -> Result<Self, SimFileError> {
        let text = std::str::from_utf8(bytes).map_err(SimFileError::NotText)?;
        let settings: SimSettings = ron::from_str(text).map_err(SimFileError::NotRon)?;
        settings.validate().map_err(SimFileError::Settings)?;
        Ok(Self(settings))
    }

    fn replace_command(&self) -> SimCommand {
        SimCommand::ReplaceSettings(self.0.clone())
    }

    fn status(status: &mut SimFileStatus) -> &mut FileStatus {
        &mut status.settings
    }
}

/// Why a version of a simulation file can't be used.
#[derive(Debug)]
pub enum SimFileError {
    /// It couldn't be read.
    Io(std::io::Error),
    /// It isn't UTF-8 text.
    NotText(Utf8Error),
    /// The Elements file is malformed, or breaks one of the rules for Elements.
    Elements(ElementError),
    /// The settings file isn't well-formed, or names a setting that doesn't exist.
    NotRon(ron::error::SpannedError),
    /// A setting lies outside its range.
    Settings(SettingsError),
}

impl fmt::Display for SimFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "can't read the file: {error}"),
            Self::NotText(error) => write!(f, "the file isn't UTF-8 text: {error}"),
            Self::Elements(error) => error.fmt(f),
            Self::NotRon(error) => write!(f, "the settings file is not valid RON: {error}"),
            Self::Settings(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for SimFileError {}

/// What became of the latest version of each simulation file, for the debug panel.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct SimFileStatus {
    pub elements: FileStatus,
    pub settings: FileStatus,
}

impl SimFileStatus {
    /// Each file's path within `assets/`, with its status.
    pub fn files(&self) -> [(&'static str, &FileStatus); 2] {
        [
            (ElementsFile::PATH, &self.elements),
            (SettingsFile::PATH, &self.settings),
        ]
    }
}

/// What became of the latest version of one simulation file.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum FileStatus {
    /// No version has loaded since the World opened: the simulation runs with the rules it
    /// started with.
    #[default]
    AsStarted,
    /// The simulation took up the latest version once it had run `tick` Ticks.
    Applied { tick: u64 },
    /// The latest version can't be used, for this reason. The simulation keeps the version
    /// before.
    Rejected(String),
}

/// Loads a [`SimFile`]. It claims no file extension, since both files are `.ron`, so the asset
/// server chooses it by asset type alone.
#[derive(TypePath)]
struct SimFileLoader<F>(PhantomData<fn() -> F>);

impl<F: SimFile> AssetLoader for SimFileLoader<F> {
    type Asset = F;
    type Settings = ();
    type Error = SimFileError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &(),
        _load_context: &mut LoadContext<'_>,
    ) -> Result<F, SimFileError> {
        let mut bytes = Vec::new();
        reader
            .read_to_end(&mut bytes)
            .await
            .map_err(SimFileError::Io)?;
        F::parse(&bytes)
    }
}

/// Makes `F` an asset, with its loader.
fn register<F: SimFile>(app: &mut App) {
    app.init_asset::<F>()
        .register_asset_loader(SimFileLoader::<F>(PhantomData));
}

/// The handle that keeps `F` loaded. The asset server reloads it whenever the file changes.
#[derive(Resource)]
struct Loaded<F: SimFile>(Handle<F>);

fn load<F: SimFile>(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(Loaded(server.load::<F>(F::PATH)));
}

/// Sends the open World's simulation every version of `F` reloaded while it is open: at once
/// while the World runs, and the latest one once it runs again while it is paused. The first
/// version loaded isn't a reload, and versions from before the World opened don't count.
fn send_new_versions<F: SimFile>(
    mut events: MessageReader<AssetEvent<F>>,
    loaded: Res<Loaded<F>>,
    files: Res<Assets<F>>,
    bridge: Option<Res<SimBridge>>,
    play: Option<Res<State<Play>>>,
    mut held_back: Local<Option<F>>,
) {
    // Assets write their events late in the frame, so those read as a World opens are from
    // before it did.
    let Some(bridge) = bridge.filter(|bridge| !bridge.is_added()) else {
        events.clear();
        *held_back = None;
        return;
    };
    for event in events.read() {
        if let AssetEvent::Modified { id } = event
            && *id == loaded.0.id()
            && let Some(file) = files.get(*id)
        {
            *held_back = Some(file.clone());
        }
    }
    if play.is_some_and(|play| *play.get() == Play::Running)
        && let Some(file) = held_back.take()
    {
        bridge.send(file.replace_command());
    }
}

/// Reports every version of `F` that failed to load or validate.
fn record_failures<F: SimFile>(
    mut failures: MessageReader<AssetLoadFailedEvent<F>>,
    mut status: ResMut<SimFileStatus>,
) {
    for failure in failures.read() {
        reject::<F>(&mut status, loader_message(&failure.error));
    }
}

/// Reports the simulation's answer to every version sent.
fn record_outcomes(
    mut outcomes: MessageReader<ReplaceOutcome>,
    report: Res<SimReport>,
    mut status: ResMut<SimFileStatus>,
) {
    for outcome in outcomes.read() {
        match outcome {
            ReplaceOutcome::Elements(answer) => {
                record_answer::<ElementsFile, _>(&mut status, answer, report.tick);
            }
            ReplaceOutcome::Settings(answer) => {
                record_answer::<SettingsFile, _>(&mut status, answer, report.tick);
            }
        }
    }
}

fn record_answer<F: SimFile, E: fmt::Display>(
    status: &mut SimFileStatus,
    answer: &Result<(), E>,
    tick: u64,
) {
    match answer {
        Ok(()) => {
            info!(
                "{}: the simulation took up the new version after Tick {tick}",
                F::PATH
            );
            *F::status(status) = FileStatus::Applied { tick };
        }
        Err(error) => reject::<F>(status, error.to_string()),
    }
}

fn reject<F: SimFile>(status: &mut SimFileStatus, message: String) {
    error!(
        "{}: {message}. The simulation keeps the version before.",
        F::PATH
    );
    *F::status(status) = FileStatus::Rejected(message);
}

/// The loader's own message for a failed load, without the asset server's wrapping, which names
/// the loader's type.
fn loader_message(error: &AssetLoadError) -> String {
    if let AssetLoadError::AssetLoaderError(error) = error
        && let Some(error) = error.error().downcast_ref::<SimFileError>()
    {
        return error.to_string();
    }
    error.to_string()
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};
    use std::{fs, thread};

    use bevy::prelude::*;
    use sim::ElementError;

    use super::*;

    const SHIPPED_ELEMENTS: &[u8] = include_bytes!("../assets/elements.ron");
    const SHIPPED_SETTINGS: &[u8] = include_bytes!("../assets/sim_settings.ron");

    #[test]
    fn the_elements_loader_parses_the_shipped_file() {
        let file = ElementsFile::parse(SHIPPED_ELEMENTS).expect("the shipped file is valid");

        // The ten starter Elements plus the built-in Vacuum.
        assert_eq!(file.0.iter().count(), 11);
        assert!(file.0.id_of("Granite").is_some());
    }

    #[test]
    fn the_settings_loader_parses_the_shipped_file() {
        let file = SettingsFile::parse(SHIPPED_SETTINGS).expect("the shipped file is valid");

        assert_eq!(file.0.conduction_multiplier, 1_000.0);
    }

    #[test]
    fn a_malformed_elements_file_is_an_error_not_a_panic() {
        let shipped = std::str::from_utf8(SHIPPED_ELEMENTS).expect("UTF-8");
        // A typo in a field name, a file cut off halfway, and a negative Density.
        let typo = shipped.replacen("conductivity:", "conductivty:", 1);
        let cut_off = &shipped[..shipped.len() / 2];
        let invalid = shipped.replacen("density: 2700.0", "density: -2700.0", 1);

        for text in [typo.as_str(), cut_off, invalid.as_str()] {
            let error = ElementsFile::parse(text.as_bytes()).expect_err("rejected");

            assert!(matches!(error, SimFileError::Elements(_)), "{error:?}");
        }
        let error = ElementsFile::parse(&[0xff, 0xfe]).expect_err("rejected");
        assert!(matches!(error, SimFileError::NotText(_)), "{error:?}");
    }

    #[test]
    fn a_malformed_settings_file_is_an_error_not_a_panic() {
        let shipped = std::str::from_utf8(SHIPPED_SETTINGS).expect("UTF-8");
        let typo = shipped.replacen("conduction_multiplier:", "conduction_multipler:", 1);
        let cut_off = &shipped[..shipped.len() / 2];
        // Above 0.5, a pair of gas Cells would overshoot.
        let out_of_range = shipped.replacen(
            "gas_evening_out_fraction: 0.25",
            "gas_evening_out_fraction: 0.9",
            1,
        );

        for text in [typo.as_str(), cut_off] {
            let error = SettingsFile::parse(text.as_bytes()).expect_err("rejected");

            assert!(matches!(error, SimFileError::NotRon(_)), "{error:?}");
        }
        let error = SettingsFile::parse(out_of_range.as_bytes()).expect_err("rejected");
        assert!(matches!(error, SimFileError::Settings(_)), "{error:?}");
        assert!(
            error.to_string().contains("gas_evening_out_fraction"),
            "{error}"
        );
    }

    /// An app that loads assets from `root`, with both simulation files' loaders, and doesn't
    /// watch for changes.
    fn asset_app(root: &Path) -> App {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin {
                file_path: root.to_string_lossy().into_owned(),
                watch_for_changes_override: Some(false),
                ..default()
            },
        ));
        register::<ElementsFile>(&mut app);
        register::<SettingsFile>(&mut app);
        app
    }

    fn update_until(app: &mut App, what: &str, done: impl Fn(&mut App) -> bool) {
        let start = Instant::now();
        while !done(app) {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "timed out waiting for {what}"
            );
            app.update();
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn the_asset_server_loads_each_shipped_file_with_its_own_loader() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets");
        let mut app = asset_app(&root);
        let server = app.world().resource::<AssetServer>().clone();
        let elements = server.load::<ElementsFile>(ElementsFile::PATH);
        let settings = server.load::<SettingsFile>(SettingsFile::PATH);

        update_until(&mut app, "both files", |_| {
            server.is_loaded(&elements) && server.is_loaded(&settings)
        });

        let world = app.world();
        let elements = &world
            .resource::<Assets<ElementsFile>>()
            .get(&elements)
            .expect("loaded")
            .0;
        let settings = &world
            .resource::<Assets<SettingsFile>>()
            .get(&settings)
            .expect("loaded")
            .0;
        assert!(elements.id_of("Granite").is_some());
        assert_eq!(settings.conduction_multiplier, 1_000.0);
    }

    #[test]
    fn a_file_that_fails_to_load_is_reported_in_the_status() {
        let root = std::env::temp_dir().join(format!("bni-sim-files-{}", std::process::id()));
        fs::create_dir_all(&root).expect("temp dir");
        fs::write(root.join(ElementsFile::PATH), "(elements: [(name: \"Oops\"").expect("written");
        let mut app = asset_app(&root);
        app.init_resource::<SimFileStatus>()
            .add_systems(Update, record_failures::<ElementsFile>);
        let server = app.world().resource::<AssetServer>().clone();
        let handle = server.load::<ElementsFile>(ElementsFile::PATH);

        update_until(&mut app, "the status", |app| {
            app.world().resource::<SimFileStatus>().elements != FileStatus::AsStarted
        });
        fs::remove_dir_all(&root).expect("temp dir removed");

        assert!(server.load_state(&handle).is_failed());
        let FileStatus::Rejected(message) = &app.world().resource::<SimFileStatus>().elements
        else {
            panic!("expected a rejection");
        };
        // The loader's own message, not the asset server's wrapping of it.
        let expected = ElementError::Parse(String::new()).to_string();
        assert!(message.starts_with(&expected), "{message:?}");
    }
}
