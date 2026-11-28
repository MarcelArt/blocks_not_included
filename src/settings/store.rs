//! The player's settings on disk: `settings.ron`, in [`UserDirs::settings`].
//!
//! Each setting is read on its own. A missing setting takes its default without complaint, as in
//! a file written before the setting existed. A setting whose value is invalid takes its default
//! too, and is reported, while the rest are kept. A file that can't be read or parsed at all is
//! reported, and every setting takes its default. Reading never changes the file: it is only
//! written when the player applies their settings, and then whole and valid.
//!
//! A write stages the new file beside the old one and renames it into place
//! (`crate::atomic_write`), so a write that fails leaves the last one whole.
//!
//! [`UserDirs::settings`]: crate::user_dirs::UserDirs::settings

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ron::extensions::Extensions;
use ron::value::RawValue;
use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize, Serializer};

use super::bindings::{Action, Bindings, Input, Refusal};
use super::{AutosaveInterval, Display, Settings, check_sensitivity};
use crate::atomic_write::{stage, sync_dir};
use crate::saves::random_u64;

/// The revision of the settings file's layout.
pub const SETTINGS_VERSION: u32 = 1;

/// The settings file's name in its folder.
pub const SETTINGS_FILE: &str = "settings.ron";

/// A hook run after the new file is staged and before it is published.
type PublishHook = Arc<dyn Fn() -> io::Result<()> + Send + Sync>;

/// Where one player's settings are kept, for one build: the settings file in a folder of its own.
#[derive(Clone)]
pub struct SettingsStore {
    dir: PathBuf,
    before_publish: Option<PublishHook>,
}

/// The settings read from the file, and what was wrong with it.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadedSettings {
    /// Every setting that could be read, and the default for every other.
    pub settings: Settings,
    pub problems: Vec<SettingsProblem>,
}

/// Something wrong with the settings file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingsProblem {
    /// The file can't be read or parsed. Every setting takes its default.
    Unreadable(String),
    /// One setting's value is invalid, so it takes its default.
    Invalid { setting: String, why: String },
}

impl fmt::Display for SettingsProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable(why) => write!(f, "the settings file can't be read: {why}"),
            Self::Invalid { setting, why } => {
                write!(f, "the setting “{setting}” is invalid: {why}")
            }
        }
    }
}

impl SettingsStore {
    /// The settings in the folder `dir`, which needn't exist yet.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            before_publish: None,
        }
    }

    /// Runs `hook` in every write after the new file is staged and before it is published. An
    /// error from it fails the write there. Tests use it to fail a write part-way.
    pub fn before_publish(
        mut self,
        hook: impl Fn() -> io::Result<()> + Send + Sync + 'static,
    ) -> Self {
        self.before_publish = Some(Arc::new(hook));
        self
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The settings file.
    pub fn path(&self) -> PathBuf {
        self.dir.join(SETTINGS_FILE)
    }

    /// Reads the settings. Never fails: whatever can't be read takes its default, and is
    /// reported unless it is simply missing.
    pub fn load(&self) -> LoadedSettings {
        let text = match fs::read_to_string(self.path()) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return LoadedSettings {
                    settings: Settings::default(),
                    problems: Vec::new(),
                };
            }
            Err(error) => return unreadable(error.to_string()),
        };
        let options = ron::Options::default().with_default_extension(Extensions::IMPLICIT_SOME);
        match options.from_str::<StoredSettings>(&text) {
            Ok(stored) if stored.version == SETTINGS_VERSION => stored.read(),
            Ok(stored) => unreadable(format!(
                "it is version {} of the file, and this build reads version {SETTINGS_VERSION}",
                stored.version
            )),
            Err(error) => unreadable(error.to_string()),
        }
    }

    /// Writes `settings` whole, replacing the file. A write that fails leaves the file as it was.
    /// Settings that aren't valid aren't written.
    pub fn save(&self, settings: &Settings) -> io::Result<()> {
        settings
            .check()
            .map_err(|problem| io::Error::new(io::ErrorKind::InvalidInput, problem.to_string()))?;
        let text =
            ron::ser::to_string_pretty(&ToStore(settings), pretty()).map_err(io::Error::other)?;
        fs::create_dir_all(&self.dir)?;
        self.remove_staged();
        let staged = self
            .dir
            .join(format!(".{SETTINGS_FILE}.{:016x}.staged", random_u64()));
        let published = stage(&staged, text.as_bytes())
            .and_then(|()| self.before_publish.as_ref().map_or(Ok(()), |hook| hook()))
            .and_then(|()| fs::rename(&staged, self.path()));
        if let Err(error) = published {
            let _ = fs::remove_file(&staged);
            return Err(error);
        }
        sync_dir(&self.dir);
        Ok(())
    }

    /// Removes staged files that crashed writes left behind. They were never published, so
    /// nothing depends on them.
    fn remove_staged(&self) {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return;
        };
        let prefix = format!(".{SETTINGS_FILE}.");
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if name.starts_with(&prefix) && name.ends_with(".staged") {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
}

fn unreadable(why: String) -> LoadedSettings {
    LoadedSettings {
        settings: Settings::default(),
        problems: vec![SettingsProblem::Unreadable(why)],
    }
}

fn pretty() -> ron::ser::PrettyConfig {
    ron::ser::PrettyConfig::new().depth_limit(2)
}

/// The file as read: each setting is parsed on its own, so one that is invalid doesn't stop the
/// others being read.
#[derive(Deserialize)]
struct StoredSettings {
    version: u32,
    #[serde(default)]
    display: Option<Box<RawValue>>,
    #[serde(default)]
    mouse_sensitivity: Option<Box<RawValue>>,
    #[serde(default)]
    autosave_minutes: Option<Box<RawValue>>,
    #[serde(default)]
    controls: Option<Box<RawValue>>,
}

impl StoredSettings {
    fn read(self) -> LoadedSettings {
        let mut settings = Settings::default();
        let mut problems = Vec::new();
        let mut invalid = |setting: &str, why: String| {
            problems.push(SettingsProblem::Invalid {
                setting: setting.to_owned(),
                why,
            });
        };

        if let Some(raw) = self.display {
            match raw.into_rust::<Display>() {
                Ok(display) => {
                    settings.display.mode = display.mode;
                    match display.window_size.check() {
                        Ok(()) => settings.display.window_size = display.window_size,
                        Err(why) => invalid("display", why),
                    }
                }
                Err(error) => invalid("display", error.to_string()),
            }
        }
        if let Some(raw) = self.mouse_sensitivity {
            match raw.into_rust::<f32>() {
                Ok(sensitivity) => match check_sensitivity(sensitivity) {
                    Ok(()) => settings.mouse_sensitivity = sensitivity,
                    Err(why) => invalid("mouse_sensitivity", why),
                },
                Err(error) => invalid("mouse_sensitivity", error.to_string()),
            }
        }
        if let Some(raw) = self.autosave_minutes {
            match raw.into_rust::<u32>() {
                Ok(minutes) => match AutosaveInterval::from_minutes(minutes) {
                    Some(interval) => settings.autosave = interval,
                    None => invalid(
                        "autosave_minutes",
                        format!("{minutes} isn't one of the intervals: 1, 3, 5 or 10 minutes"),
                    ),
                },
                Err(error) => invalid("autosave_minutes", error.to_string()),
            }
        }
        if let Some(raw) = self.controls {
            match raw.into_rust::<BTreeMap<String, Box<RawValue>>>() {
                Ok(controls) => {
                    settings.bindings = read_controls(controls, &mut invalid);
                }
                Err(error) => invalid("controls", error.to_string()),
            }
        }
        LoadedSettings { settings, problems }
    }
}

/// The bindings the file lists, with the default for each action it leaves out or binds
/// invalidly. If the result binds two actions that apply at once to one input, every action
/// takes its default binding.
fn read_controls(
    controls: BTreeMap<String, Box<RawValue>>,
    invalid: &mut impl FnMut(&str, String),
) -> Bindings {
    let mut bindings = Bindings::default();
    for (name, raw) in controls {
        let setting = format!("controls: {name}");
        let Some(action) = Action::from_name(&name) else {
            invalid(&setting, "no action has that name".to_owned());
            continue;
        };
        match raw.into_rust::<Vec<Input>>() {
            Ok(inputs) => match inputs.iter().find_map(|input| Refusal::of(*input)) {
                Some(refusal) => invalid(&setting, refusal.to_string()),
                None => bindings.set(action, inputs),
            },
            Err(error) => invalid(&setting, error.to_string()),
        }
    }
    if let Err(error) = bindings.check() {
        invalid(
            "controls",
            format!("{error}, so every control takes its default"),
        );
        bindings = Bindings::default();
    }
    bindings
}

/// The file as written.
struct ToStore<'a>(&'a Settings);

impl Serialize for ToStore<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Stored<'a> {
            version: u32,
            display: &'a Display,
            mouse_sensitivity: f32,
            autosave_minutes: u32,
            controls: Controls<'a>,
        }
        let settings = self.0;
        Stored {
            version: SETTINGS_VERSION,
            display: &settings.display,
            mouse_sensitivity: settings.mouse_sensitivity,
            autosave_minutes: settings.autosave.minutes(),
            controls: Controls(&settings.bindings),
        }
        .serialize(serializer)
    }
}

/// Every action's binding, by name, in the order Settings lists them.
struct Controls<'a>(&'a Bindings);

impl Serialize for Controls<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(Action::ALL.len()))?;
        for action in Action::ALL {
            map.serialize_entry(action.name(), self.0.inputs(action))?;
        }
        map.end()
    }
}
