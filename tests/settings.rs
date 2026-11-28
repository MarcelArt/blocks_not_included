//! The player's settings on disk (`docs/plans/20-settings-and-bindings.md`, seam 1): every setting
//! round-trips through the file; missing settings take their defaults quietly, and invalid ones
//! are reported and take their defaults while the rest are kept; a failed write keeps the last
//! one; and development and release builds keep settings of their own.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::{fs, io, process};

use bevy::prelude::{KeyCode, MouseButton};
use blocks_not_included::settings::{
    Action, AutosaveInterval, Bindings, Display, DisplayMode, Input, LoadedSettings, SETTINGS_FILE,
    Settings, SettingsProblem, SettingsStore, WindowSize,
};
use blocks_not_included::user_dirs::{Build, UserDirs};

/// A folder of its own for one test, removed when the test ends.
struct TempDir(PathBuf);

impl TempDir {
    fn new(test: &str) -> Self {
        let path = std::env::temp_dir().join(format!("bni-settings-{}-{test}", process::id()));
        let _ = fs::remove_dir_all(&path);
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Settings with nothing left at its default.
fn custom() -> Settings {
    let mut bindings = Bindings::default();
    bindings.set(Action::Jump, vec![Input::Mouse(MouseButton::Back)]);
    bindings.set(Action::Dig, vec![Input::Key(KeyCode::KeyK)]);
    bindings.set(Action::Place, vec![Input::Mouse(MouseButton::Other(9))]);
    bindings.set(
        Action::MoveForward,
        vec![Input::Key(KeyCode::KeyW), Input::Key(KeyCode::ArrowUp)],
    );
    bindings.set(Action::CycleOverlay, Vec::new());
    Settings {
        display: Display {
            mode: DisplayMode::BorderlessFullscreen,
            window_size: WindowSize::new(2560, 1440),
        },
        mouse_sensitivity: 1.75,
        autosave: AutosaveInterval::TenMinutes,
        bindings,
    }
}

fn loaded_cleanly(settings: Settings) -> LoadedSettings {
    LoadedSettings {
        settings,
        problems: Vec::new(),
    }
}

fn write_file(store: &SettingsStore, text: &str) {
    fs::create_dir_all(store.dir()).expect("the folder");
    fs::write(store.path(), text).expect("the file");
}

/// The names of the settings `problems` reports as invalid, or "unreadable".
fn problem_settings(problems: &[SettingsProblem]) -> Vec<String> {
    problems
        .iter()
        .map(|problem| match problem {
            SettingsProblem::Unreadable(_) => "unreadable".to_owned(),
            SettingsProblem::Invalid { setting, .. } => setting.clone(),
        })
        .collect()
}

/// Staged files left in `dir`.
fn staged_files(dir: &Path) -> Vec<String> {
    fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| name.ends_with(".staged"))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn every_setting_round_trips_through_the_file() {
    let dir = TempDir::new("round-trip");
    let store = SettingsStore::new(&dir.0);

    store.save(&custom()).expect("the settings are kept");
    assert_eq!(store.load(), loaded_cleanly(custom()));
    assert_eq!(store.path(), dir.0.join(SETTINGS_FILE));

    // Every choice of each setting, and the edges of the sensitivity's range. 1/3 has no exact
    // decimal form, so it shows that the float is written in full.
    for (index, autosave) in AutosaveInterval::ALL.into_iter().enumerate() {
        for mode in DisplayMode::ALL {
            for sensitivity in [0.1, 1.0 / 3.0, 5.0] {
                let settings = Settings {
                    display: Display {
                        mode,
                        window_size: WindowSize::CHOICES[index],
                    },
                    mouse_sensitivity: sensitivity,
                    autosave,
                    ..custom()
                };
                store.save(&settings).expect("the settings are kept");
                assert_eq!(store.load(), loaded_cleanly(settings));
            }
        }
    }
    assert_eq!(staged_files(&dir.0), Vec::<String>::new());
}

#[test]
fn missing_settings_take_their_defaults_without_complaint() {
    let dir = TempDir::new("missing");
    let store = SettingsStore::new(dir.0.join("not yet made"));

    // No file at all: the first run.
    assert_eq!(store.load(), loaded_cleanly(Settings::default()));
    assert!(!store.dir().exists(), "reading creates nothing");

    // A file from before most settings existed.
    write_file(
        &store,
        "(version: 1, mouse_sensitivity: 2.5, controls: { \"jump\": [Mouse(Middle)] })",
    );
    let mut expected = Settings {
        mouse_sensitivity: 2.5,
        ..Settings::default()
    };
    expected
        .bindings
        .set(Action::Jump, vec![Input::Mouse(MouseButton::Middle)]);
    assert_eq!(store.load(), loaded_cleanly(expected));
}

#[test]
fn an_unreadable_file_is_reported_and_every_setting_takes_its_default() {
    let dir = TempDir::new("unreadable");
    let store = SettingsStore::new(&dir.0);
    for text in [
        "this isn't RON",
        // A newer layout, or none at all.
        "(version: 2, mouse_sensitivity: 2.0)",
        "(mouse_sensitivity: 2.0)",
        "",
    ] {
        write_file(&store, text);
        let loaded = store.load();
        assert_eq!(loaded.settings, Settings::default(), "{text:?}");
        assert_eq!(
            problem_settings(&loaded.problems),
            ["unreadable"],
            "{text:?}"
        );
        assert_eq!(
            fs::read_to_string(store.path()).expect("the file"),
            text,
            "reading never changes the file"
        );
    }
}

#[test]
fn each_invalid_setting_is_reported_and_takes_its_default_while_the_rest_are_kept() {
    let dir = TempDir::new("invalid");
    let store = SettingsStore::new(&dir.0);

    // The mode is kept, but not the size; Jump can't be bound to Esc; no action is called
    // "fly_backwards"; Dig's binding is fine.
    let text = r#"(
        version: 1,
        display: (mode: BorderlessFullscreen, window_size: (width: 100, height: 100)),
        mouse_sensitivity: 0.0,
        autosave_minutes: 7,
        controls: {
            "jump": [Key(Escape)],
            "fly_backwards": [Key(KeyB)],
            "dig": [Key(KeyK)],
        },
    )"#;
    write_file(&store, text);
    let loaded = store.load();
    let mut expected = Settings::default();
    expected.display.mode = DisplayMode::BorderlessFullscreen;
    expected
        .bindings
        .set(Action::Dig, vec![Input::Key(KeyCode::KeyK)]);
    assert_eq!(loaded.settings, expected);
    assert_eq!(
        problem_settings(&loaded.problems),
        [
            "display",
            "mouse_sensitivity",
            "autosave_minutes",
            // The file's controls, by name.
            "controls: fly_backwards",
            "controls: jump",
        ]
    );
    assert_eq!(fs::read_to_string(store.path()).expect("the file"), text);

    // Values of the wrong kind, and a sensitivity that isn't a number.
    write_file(
        &store,
        r#"(version: 1, display: 3, mouse_sensitivity: "fast", autosave_minutes: 3, controls: 4)"#,
    );
    let loaded = store.load();
    let expected = Settings {
        autosave: AutosaveInterval::ThreeMinutes,
        ..Settings::default()
    };
    assert_eq!(loaded.settings, expected);
    assert_eq!(
        problem_settings(&loaded.problems),
        ["display", "mouse_sensitivity", "controls"]
    );
    write_file(&store, "(version: 1, mouse_sensitivity: NaN)");
    assert_eq!(
        problem_settings(&store.load().problems),
        ["mouse_sensitivity"]
    );

    // Two actions that apply at once, bound to one key: neither binding can be trusted, so every
    // control takes its default. The other settings are kept.
    write_file(
        &store,
        r#"(version: 1, autosave_minutes: 5, controls: { "jump": [Key(KeyO)], "dig": [Key(KeyK)] })"#,
    );
    let loaded = store.load();
    let expected = Settings {
        autosave: AutosaveInterval::FiveMinutes,
        ..Settings::default()
    };
    assert_eq!(loaded.settings, expected);
    assert_eq!(problem_settings(&loaded.problems), ["controls"]);
    let message = loaded.problems[0].to_string();
    assert!(
        message.contains("Jump, swim up, fly up") && message.contains("Cycle the Overlays"),
        "{message}"
    );
}

#[test]
fn a_failed_write_keeps_the_last_settings_kept_and_says_why() {
    let dir = TempDir::new("failed-write");
    let fail = Arc::new(AtomicBool::new(false));
    let store = {
        let fail = fail.clone();
        SettingsStore::new(&dir.0).before_publish(move || {
            if fail.load(Ordering::SeqCst) {
                Err(io::Error::other("the disk is full"))
            } else {
                Ok(())
            }
        })
    };

    // A failed first write leaves no file, so the defaults load.
    fail.store(true, Ordering::SeqCst);
    let error = store.save(&custom()).expect_err("the write fails");
    assert!(error.to_string().contains("the disk is full"), "{error}");
    assert!(!store.path().exists());
    assert_eq!(store.load(), loaded_cleanly(Settings::default()));
    assert_eq!(staged_files(&dir.0), Vec::<String>::new());

    // A failed write after a good one leaves the good one.
    fail.store(false, Ordering::SeqCst);
    store.save(&custom()).expect("the settings are kept");
    let kept = fs::read(store.path()).expect("the file");
    fail.store(true, Ordering::SeqCst);
    store
        .save(&Settings::default())
        .expect_err("the write fails");
    assert_eq!(fs::read(store.path()).expect("the file"), kept);
    assert_eq!(store.load(), loaded_cleanly(custom()));
    assert_eq!(staged_files(&dir.0), Vec::<String>::new());

    // Settings that aren't valid are never written.
    fail.store(false, Ordering::SeqCst);
    let invalid = Settings {
        mouse_sensitivity: 99.0,
        ..custom()
    };
    let error = store.save(&invalid).expect_err("refused");
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(fs::read(store.path()).expect("the file"), kept);

    // A folder that can't be made: a file is in its way.
    let blocked = SettingsStore::new(store.path().join("settings"));
    assert!(blocked.save(&custom()).is_err());
}

#[test]
fn development_and_release_builds_keep_settings_of_their_own() {
    let home = TempDir::new("builds");
    let home_path: OsString = home.0.clone().into();
    let env = move |name: &str| (name == "HOME").then(|| home_path.clone());
    let development = UserDirs::xdg(&env, Build::Development).expect("resolved");
    let release = UserDirs::xdg(&env, Build::Release).expect("resolved");
    assert_ne!(development.settings, release.settings);
    let (development, release) = (
        SettingsStore::new(development.settings),
        SettingsStore::new(release.settings),
    );

    development.save(&custom()).expect("kept");
    assert_eq!(release.load(), loaded_cleanly(Settings::default()));

    let release_settings = Settings {
        mouse_sensitivity: 0.5,
        ..Settings::default()
    };
    release.save(&release_settings).expect("kept");
    assert_eq!(development.load(), loaded_cleanly(custom()));
    assert_eq!(release.load(), loaded_cleanly(release_settings));
}
