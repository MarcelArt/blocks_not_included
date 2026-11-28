//! Where the game keeps each player's own files: World saves, and their settings.
//!
//! - **Linux**, and any other system but Windows: saves under `$XDG_DATA_HOME`, by default
//!   `~/.local/share`, and settings under `$XDG_CONFIG_HOME`, by default `~/.config`, as the
//!   [XDG Base Directory Specification](https://specifications.freedesktop.org/basedir/) says.
//! - **Windows**: both under the per-user
//!   [Local App Data known folder](https://learn.microsoft.com/en-us/windows/win32/shell/knownfolderid),
//!   in separate folders.
//!
//! A development build, with the `dev` feature, keeps its files in `blocks_not_included.dev`
//! rather than `blocks_not_included`, so it never touches a release build's saves.

use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};

/// Which kind of build the game is, which decides the name of its folders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Build {
    /// Built with the `dev` feature, which watches `assets/` for changes.
    Development,
    Release,
}

impl Build {
    /// This build.
    pub const CURRENT: Build = if cfg!(feature = "dev") {
        Build::Development
    } else {
        Build::Release
    };

    /// The name of the folder this build's files go in, inside each base directory.
    pub fn folder_name(self) -> &'static str {
        match self {
            Build::Development => "blocks_not_included.dev",
            Build::Release => "blocks_not_included",
        }
    }
}

/// Where one player's files go, for one [`Build`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserDirs {
    /// The folder of World saves, for a [`WorldStore`](crate::saves::WorldStore).
    pub saves: PathBuf,
    /// The folder of the player's settings, kept apart from saves.
    pub settings: PathBuf,
}

impl UserDirs {
    /// This player's folders for this build, on this system. Nothing is created.
    pub fn for_this_build() -> Result<UserDirs, UserDirsError> {
        Self::for_build(Build::CURRENT)
    }

    /// This player's folders for `build`, on this system. Nothing is created.
    pub fn for_build(build: Build) -> Result<UserDirs, UserDirsError> {
        #[cfg(windows)]
        {
            use known_folders::{KnownFolder, get_known_folder_path};
            get_known_folder_path(KnownFolder::LocalAppData)
                .map(|local_app_data| Self::windows(&local_app_data, build))
                .ok_or(UserDirsError::NoLocalAppData)
        }
        #[cfg(not(windows))]
        {
            Self::xdg(|name| std::env::var_os(name), build)
        }
    }

    /// The folders under the XDG base directories, reading environment variables with `env`:
    /// saves in `$XDG_DATA_HOME/<folder>/saves`, and settings in `$XDG_CONFIG_HOME/<folder>`.
    ///
    /// A variable that is unset, empty or relative is ignored, as the spec says, and its default
    /// under `$HOME` is used instead: `~/.local/share` or `~/.config`. Fails if a default is
    /// needed and `$HOME` isn't an absolute path.
    pub fn xdg(
        env: impl Fn(&str) -> Option<OsString>,
        build: Build,
    ) -> Result<UserDirs, UserDirsError> {
        let absolute = |name: &str| {
            env(name)
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
        };
        let base = |name: &str, default: &str| match absolute(name) {
            Some(path) => Ok(path),
            None => absolute("HOME")
                .map(|home| home.join(default))
                .ok_or(UserDirsError::NoHome),
        };
        let folder = build.folder_name();
        Ok(UserDirs {
            saves: base("XDG_DATA_HOME", ".local/share")?
                .join(folder)
                .join("saves"),
            settings: base("XDG_CONFIG_HOME", ".config")?.join(folder),
        })
    }

    /// The folders on Windows, under the Local App Data known folder at `local_app_data`: saves in
    /// `<folder>\saves`, and settings in `<folder>\settings`.
    pub fn windows(local_app_data: &Path, build: Build) -> UserDirs {
        let app = local_app_data.join(build.folder_name());
        UserDirs {
            saves: app.join("saves"),
            settings: app.join("settings"),
        }
    }
}

/// Why the player's folders couldn't be found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserDirsError {
    /// A default XDG base directory was needed, but `$HOME` isn't set to an absolute path.
    NoHome,
    /// Windows didn't say where the Local App Data known folder is.
    NoLocalAppData,
}

impl fmt::Display for UserDirsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NoHome => "can't find the home folder: $HOME isn't set to an absolute path",
            Self::NoLocalAppData => "Windows didn't say where the Local App Data folder is",
        })
    }
}

impl std::error::Error for UserDirsError {}
