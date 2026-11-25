//! Where each player's saves and settings go (`docs/plans/17-world-save-storage.md`): the XDG base
//! directories on Linux, Local App Data on Windows, and separate folders for development and
//! release builds.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use blocks_not_included::user_dirs::{Build, UserDirs, UserDirsError};

/// An environment holding only `vars`.
fn env(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
    let vars: Vec<(String, OsString)> = vars
        .iter()
        .map(|&(name, value)| (name.to_owned(), value.into()))
        .collect();
    move |name| {
        vars.iter()
            .find(|(var, _)| var == name)
            .map(|(_, value)| value.clone())
    }
}

#[test]
fn on_linux_saves_and_settings_go_under_the_xdg_base_directories_when_set() {
    let set = env(&[
        ("HOME", "/home/ada"),
        ("XDG_DATA_HOME", "/data/ada"),
        ("XDG_CONFIG_HOME", "/config/ada"),
    ]);

    let dirs = UserDirs::xdg(set, Build::Release).expect("resolved");

    assert_eq!(
        dirs,
        UserDirs {
            saves: PathBuf::from("/data/ada/blocks_not_included/saves"),
            settings: PathBuf::from("/config/ada/blocks_not_included"),
        }
    );
}

#[test]
fn on_linux_unset_empty_or_relative_base_directories_fall_back_to_the_defaults_under_home() {
    // The XDG spec: an unset or empty variable takes its default, and a relative path is invalid
    // and ignored.
    let unset = env(&[("HOME", "/home/ada")]);
    let empty = env(&[
        ("HOME", "/home/ada"),
        ("XDG_DATA_HOME", ""),
        ("XDG_CONFIG_HOME", ""),
    ]);
    let relative = env(&[
        ("HOME", "/home/ada"),
        ("XDG_DATA_HOME", "data"),
        ("XDG_CONFIG_HOME", "./config"),
    ]);
    let defaults = UserDirs {
        saves: PathBuf::from("/home/ada/.local/share/blocks_not_included/saves"),
        settings: PathBuf::from("/home/ada/.config/blocks_not_included"),
    };

    for environment in [
        Box::new(unset) as Box<dyn Fn(&str) -> Option<OsString>>,
        Box::new(empty),
        Box::new(relative),
    ] {
        assert_eq!(
            UserDirs::xdg(environment, Build::Release),
            Ok(defaults.clone())
        );
    }
}

#[test]
fn on_linux_a_default_needs_home_but_set_base_directories_do_not() {
    let nowhere = env(&[]);
    let relative_home = env(&[("HOME", "ada")]);
    let only_data = env(&[("XDG_DATA_HOME", "/data/ada")]);
    let both = env(&[
        ("XDG_DATA_HOME", "/data/ada"),
        ("XDG_CONFIG_HOME", "/config/ada"),
    ]);

    assert_eq!(
        UserDirs::xdg(nowhere, Build::Release),
        Err(UserDirsError::NoHome)
    );
    assert_eq!(
        UserDirs::xdg(relative_home, Build::Release),
        Err(UserDirsError::NoHome)
    );
    // Settings would still need `~/.config`.
    assert_eq!(
        UserDirs::xdg(only_data, Build::Release),
        Err(UserDirsError::NoHome)
    );
    assert!(UserDirs::xdg(both, Build::Release).is_ok());
}

#[test]
fn on_windows_saves_and_settings_go_in_separate_folders_under_local_app_data() {
    let local_app_data = Path::new("C:/Users/Ada/AppData/Local");

    let dirs = UserDirs::windows(local_app_data, Build::Release);

    let app = local_app_data.join("blocks_not_included");
    assert_eq!(dirs.saves, app.join("saves"));
    assert_eq!(dirs.settings, app.join("settings"));
    assert_ne!(dirs.saves, dirs.settings);
}

#[test]
fn development_and_release_builds_never_share_a_folder() {
    let home = || env(&[("HOME", "/home/ada")]);
    let local_app_data = Path::new("C:/Users/Ada/AppData/Local");

    let linux_dev = UserDirs::xdg(home(), Build::Development).expect("resolved");
    let linux_release = UserDirs::xdg(home(), Build::Release).expect("resolved");
    let windows_dev = UserDirs::windows(local_app_data, Build::Development);
    let windows_release = UserDirs::windows(local_app_data, Build::Release);

    assert_eq!(
        linux_dev.saves,
        PathBuf::from("/home/ada/.local/share/blocks_not_included.dev/saves")
    );
    assert_eq!(
        linux_dev.settings,
        PathBuf::from("/home/ada/.config/blocks_not_included.dev")
    );
    assert_eq!(
        windows_dev.saves,
        local_app_data.join("blocks_not_included.dev").join("saves")
    );
    for (dev, release) in [
        (&linux_dev, &linux_release),
        (&windows_dev, &windows_release),
    ] {
        // Neither build's folder lies inside the other's.
        for (a, b) in [
            (&dev.saves, &release.saves),
            (&dev.settings, &release.settings),
        ] {
            assert!(!a.starts_with(b) && !b.starts_with(a), "{a:?} and {b:?}");
        }
    }
}

#[test]
fn this_build_is_development_exactly_when_the_dev_feature_is_on() {
    let expected = if cfg!(feature = "dev") {
        Build::Development
    } else {
        Build::Release
    };
    assert_eq!(Build::CURRENT, expected);
}
