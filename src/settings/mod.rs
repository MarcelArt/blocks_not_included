//! The player's own settings (`docs/plans/20-settings-and-bindings.md`): the window, the mouse,
//! how often the open World autosaves, and the gameplay controls. They are the same in every
//! World, and kept apart from World saves and from the simulation's own settings
//! (`assets/sim_settings.ron`), in [`UserDirs::settings`].
//!
//! - **On startup** the game reads them before it opens its window ([`SettingsStorage::load`]),
//!   so the window opens as they say. Whatever can't be read takes its default, and the menus
//!   say what was wrong ([`SettingsStatus::load_problems`]).
//! - **The Settings page**, from the main menu or the pause menu, edits a draft of them
//!   ([`SettingsPage`], [`SettingsEditor`]). Applying the draft applies it at once and writes it
//!   ([`SettingsAction::Apply`]). If the write fails, the settings still apply until the game
//!   quits, and the page says why they weren't kept. Going back discards what wasn't applied, and
//!   returns to the menu the page was opened from. While the page is open from the pause menu,
//!   the World stays paused.
//! - **Esc** on the page stops listening for a control, or cancels a conflict, or goes back.
//!   Whatever key or button is pressed to bind a control is used up: nothing else in the game
//!   sees it ([`SettingsInputSystems`]).
//!
//! [`UserDirs::settings`]: crate::user_dirs::UserDirs::settings

mod bindings;
mod editor;
mod store;

use std::fmt;
use std::ops::RangeInclusive;
use std::time::Duration;

use bevy::input::InputSystems;
use bevy::prelude::*;
use bevy::window::{MonitorSelection, PrimaryWindow, WindowMode, WindowResolution};
use serde::{Deserialize, Serialize};

pub use bindings::{
    Action, ActionInput, Bindings, BindingsError, Conflict, Context, FIXED_SHORTCUTS, Input,
    RESERVED_KEYS, Refusal, inputs_label,
};
pub use editor::{Offered, Rebinding, SettingsEditor};
pub use store::{LoadedSettings, SETTINGS_FILE, SETTINGS_VERSION, SettingsProblem, SettingsStore};

use crate::input_policy::{InputPolicySystems, PAUSE_KEY};
use crate::session::{PauseControl, Play, Session};
use crate::user_dirs::UserDirs;

/// The player's settings, the same in every World.
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct Settings {
    pub display: Display,
    /// How fast the mouse turns the view, as a multiple of `crate::player::LOOK_SPEED`. Within
    /// [`SENSITIVITY_RANGE`].
    pub mouse_sensitivity: f32,
    /// How much unpaused play passes between autosaves.
    pub autosave: AutosaveInterval,
    pub bindings: Bindings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            display: Display::default(),
            mouse_sensitivity: 1.0,
            autosave: AutosaveInterval::default(),
            bindings: Bindings::default(),
        }
    }
}

impl Settings {
    /// Why these settings can't be used, if they can't: the first that is invalid.
    pub fn check(&self) -> Result<(), SettingsProblem> {
        let invalid = |setting: &str, why: String| SettingsProblem::Invalid {
            setting: setting.to_owned(),
            why,
        };
        self.display
            .window_size
            .check()
            .map_err(|why| invalid("display", why))?;
        check_sensitivity(self.mouse_sensitivity)
            .map_err(|why| invalid("mouse_sensitivity", why))?;
        self.bindings
            .check()
            .map_err(|error| invalid("controls", error.to_string()))
    }
}

/// The range mouse sensitivity may be set in.
pub const SENSITIVITY_RANGE: RangeInclusive<f32> = 0.1..=5.0;

/// Why `sensitivity` can't be used, if it can't.
pub(crate) fn check_sensitivity(sensitivity: f32) -> Result<(), String> {
    if SENSITIVITY_RANGE.contains(&sensitivity) {
        Ok(())
    } else {
        Err(format!(
            "{sensitivity} is outside {} to {}",
            SENSITIVITY_RANGE.start(),
            SENSITIVITY_RANGE.end()
        ))
    }
}

/// How the game's window shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Display {
    pub mode: DisplayMode,
    /// The window's size while it is a window, kept while it is fullscreen.
    pub window_size: WindowSize,
}

impl Display {
    /// The game's window as it opens.
    pub fn primary_window(&self) -> Window {
        Window {
            mode: self.mode.window_mode(),
            resolution: WindowResolution::new(self.window_size.width, self.window_size.height),
            ..default()
        }
    }
}

/// Whether the game is a window or fills its screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DisplayMode {
    #[default]
    Windowed,
    /// A window without borders as large as the screen it is on.
    BorderlessFullscreen,
}

impl DisplayMode {
    pub const ALL: [DisplayMode; 2] = [DisplayMode::Windowed, DisplayMode::BorderlessFullscreen];

    pub fn label(self) -> &'static str {
        match self {
            DisplayMode::Windowed => "Windowed",
            DisplayMode::BorderlessFullscreen => "Borderless fullscreen",
        }
    }

    pub fn window_mode(self) -> WindowMode {
        match self {
            DisplayMode::Windowed => WindowMode::Windowed,
            DisplayMode::BorderlessFullscreen => {
                WindowMode::BorderlessFullscreen(MonitorSelection::Current)
            }
        }
    }
}

/// A window's size, in physical pixels: the screen's own pixels, whatever the system's scaling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowSize {
    pub width: u32,
    pub height: u32,
}

impl Default for WindowSize {
    /// Bevy's own default.
    fn default() -> Self {
        Self::new(1280, 720)
    }
}

impl WindowSize {
    /// The sizes Settings offers, where the screen fits them.
    pub const CHOICES: [WindowSize; 7] = [
        WindowSize::new(1280, 720),
        WindowSize::new(1366, 768),
        WindowSize::new(1600, 900),
        WindowSize::new(1920, 1080),
        WindowSize::new(2560, 1440),
        WindowSize::new(3200, 1800),
        WindowSize::new(3840, 2160),
    ];

    /// The smallest window allowed.
    pub const MIN: WindowSize = WindowSize::new(640, 360);

    /// The largest window allowed: 8K.
    pub const MAX: WindowSize = WindowSize::new(7680, 4320);

    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    /// Why a window can't be this size, if it can't.
    pub fn check(self) -> Result<(), String> {
        let (min, max) = (Self::MIN, Self::MAX);
        if (min.width..=max.width).contains(&self.width)
            && (min.height..=max.height).contains(&self.height)
        {
            Ok(())
        } else {
            Err(format!("a window of {self} is outside {min} to {max}"))
        }
    }

    /// Whether a window this size fits on a screen of `screen`.
    pub fn fits(self, screen: WindowSize) -> bool {
        self.width <= screen.width && self.height <= screen.height
    }
}

impl fmt::Display for WindowSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} × {}", self.width, self.height)
    }
}

/// How much unpaused play passes between autosaves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AutosaveInterval {
    #[default]
    OneMinute,
    ThreeMinutes,
    FiveMinutes,
    TenMinutes,
}

impl AutosaveInterval {
    pub const ALL: [AutosaveInterval; 4] = [
        AutosaveInterval::OneMinute,
        AutosaveInterval::ThreeMinutes,
        AutosaveInterval::FiveMinutes,
        AutosaveInterval::TenMinutes,
    ];

    pub fn minutes(self) -> u32 {
        match self {
            AutosaveInterval::OneMinute => 1,
            AutosaveInterval::ThreeMinutes => 3,
            AutosaveInterval::FiveMinutes => 5,
            AutosaveInterval::TenMinutes => 10,
        }
    }

    pub fn from_minutes(minutes: u32) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|interval| interval.minutes() == minutes)
    }

    pub fn duration(self) -> Duration {
        Duration::from_secs(u64::from(self.minutes()) * 60)
    }

    pub fn label(self) -> String {
        match self.minutes() {
            1 => "1 minute".to_owned(),
            minutes => format!("{minutes} minutes"),
        }
    }
}

/// Where this build keeps the player's settings, or why there is nowhere.
#[derive(Resource, Clone)]
pub struct SettingsStorage(Result<SettingsStore, String>);

impl Default for SettingsStorage {
    fn default() -> Self {
        Self::unavailable("no settings folder was given")
    }
}

impl SettingsStorage {
    pub fn new(store: SettingsStore) -> Self {
        Self(Ok(store))
    }

    /// Nowhere to keep settings, for this reason. They still apply until the game quits.
    pub fn unavailable(why: impl Into<String>) -> Self {
        Self(Err(why.into()))
    }

    /// This player's settings for this build, in [`UserDirs::settings`].
    pub fn for_this_build() -> Self {
        match UserDirs::for_this_build() {
            Ok(dirs) => Self::new(SettingsStore::new(dirs.settings)),
            Err(error) => Self::unavailable(error.to_string()),
        }
    }

    pub fn store(&self) -> Result<&SettingsStore, &str> {
        self.0.as_ref().map_err(String::as_str)
    }

    /// The settings kept, or the defaults if there is nowhere to keep them.
    pub fn load(&self) -> LoadedSettings {
        match &self.0 {
            Ok(store) => store.load(),
            Err(_) => LoadedSettings {
                settings: Settings::default(),
                problems: Vec::new(),
            },
        }
    }
}

/// What the menus say about the settings.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct SettingsStatus {
    /// What was wrong with the settings file at startup, until settings are next kept.
    pub load_problems: Vec<SettingsProblem>,
    /// How the latest apply on the Settings page went: kept, or why not. `None` until the page
    /// next applies.
    pub applied: Option<Result<(), String>>,
}

impl SettingsStatus {
    pub fn after_loading(problems: Vec<SettingsProblem>) -> Self {
        Self {
            load_problems: problems,
            applied: None,
        }
    }
}

/// The Settings page, while it is open: its draft and the rebinding under way.
#[derive(Resource, Default)]
pub struct SettingsPage(Option<SettingsEditor>);

impl SettingsPage {
    pub fn is_open(&self) -> bool {
        self.0.is_some()
    }

    pub fn editor(&self) -> Option<&SettingsEditor> {
        self.0.as_ref()
    }

    pub fn editor_mut(&mut self) -> Option<&mut SettingsEditor> {
        self.0.as_mut()
    }
}

/// What the player asks of the Settings page. Each is handled early in the next frame.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsAction {
    /// Opens the page over the main menu, or over the pause menu while the World is paused.
    Open,
    /// Closes the page, discarding what wasn't applied, and returns to the menu it was opened from.
    Back,
    /// Applies the page's draft, and keeps it.
    Apply,
}

/// The system that feeds the keyboard and mouse to the Settings page, in `PreUpdate`, before the
/// input policy. While the page listens for a control, it uses up every key and button pressed,
/// and while the page is open, it uses up Esc.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SettingsInputSystems;

/// The player's settings, and the Settings page. Needs the session and Bevy's input. Insert
/// the [`Settings`] read at startup, the [`SettingsStorage`] to keep them in, and the
/// [`SettingsStatus`] after loading them; each is otherwise its default.
pub struct SettingsPlugin;

impl Plugin for SettingsPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<SettingsAction>()
            .init_resource::<Settings>()
            .init_resource::<SettingsStorage>()
            .init_resource::<SettingsStatus>()
            .init_resource::<SettingsPage>()
            .add_systems(Startup, report_settings)
            .add_systems(
                PreUpdate,
                (
                    listen_for_input
                        .in_set(SettingsInputSystems)
                        .after(InputSystems)
                        .before(InputPolicySystems),
                    handle_settings_actions.after(InputPolicySystems),
                ),
            )
            .add_systems(Update, apply_display)
            // The page belongs to the menu it was opened from, and goes with it.
            .add_systems(OnExit(Session::Closed), close_page)
            .add_systems(OnExit(Session::Open), close_page)
            .add_systems(OnEnter(Play::Running), close_page)
            .add_systems(OnEnter(Play::Leaving), close_page);
    }
}

/// Logs where settings are kept, and what was wrong with them.
fn report_settings(storage: Res<SettingsStorage>, status: Res<SettingsStatus>) {
    match storage.store() {
        Ok(store) => info!("settings are kept in {}", store.path().display()),
        Err(why) => warn!("settings can't be kept: {why}"),
    }
    for problem in &status.load_problems {
        warn!("{problem}; using the default");
    }
}

fn close_page(mut page: ResMut<SettingsPage>) {
    page.0 = None;
}

/// While the Settings page is open: Esc stops listening, cancels a conflict, or goes back. While
/// it listens, the first key or button pressed is offered to it, and every one pressed is used up.
fn listen_for_input(
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut page: ResMut<SettingsPage>,
) {
    let Some(editor) = page.0.as_mut() else {
        return;
    };
    if keys.just_pressed(PAUSE_KEY) {
        keys.clear_just_pressed(PAUSE_KEY);
        if *editor.rebinding() == Rebinding::Idle {
            page.0 = None;
        } else {
            editor.cancel();
        }
        return;
    }
    if !editor.is_listening() {
        return;
    }
    let pressed_keys: Vec<KeyCode> = keys.get_just_pressed().copied().collect();
    let pressed_buttons: Vec<MouseButton> = mouse.get_just_pressed().copied().collect();
    for key in &pressed_keys {
        keys.clear_just_pressed(*key);
    }
    for button in &pressed_buttons {
        mouse.clear_just_pressed(*button);
    }
    let first = pressed_keys
        .first()
        .map(|key| Input::Key(*key))
        .or_else(|| pressed_buttons.first().map(|button| Input::Mouse(*button)));
    if let Some(input) = first {
        editor.offer(input);
    }
}

/// Opens and closes the Settings page, and applies its draft.
fn handle_settings_actions(
    mut actions: MessageReader<SettingsAction>,
    mut page: ResMut<SettingsPage>,
    mut settings: ResMut<Settings>,
    storage: Res<SettingsStorage>,
    mut status: ResMut<SettingsStatus>,
    play: PauseControl,
) {
    for action in actions.read() {
        match action {
            SettingsAction::Open => {
                // From the main menu, or the pause menu: never while the World runs.
                let in_a_menu = matches!(play.state(), None | Some(Play::Pausing | Play::Paused));
                if in_a_menu && !page.is_open() {
                    page.0 = Some(SettingsEditor::new(&settings));
                    status.applied = None;
                }
            }
            SettingsAction::Back => page.0 = None,
            SettingsAction::Apply => {
                let Some(editor) = page.0.as_ref() else {
                    continue;
                };
                let draft = editor.draft.clone();
                if let Err(problem) = draft.check() {
                    status.applied = Some(Err(problem.to_string()));
                    continue;
                }
                let kept = match storage.store() {
                    Ok(store) => store.save(&draft).map_err(|error| {
                        format!("can't write {}: {error}", store.path().display())
                    }),
                    Err(why) => Err(format!("there is nowhere to keep them: {why}")),
                };
                match &kept {
                    Ok(()) => {
                        info!("settings applied and kept");
                        status.load_problems.clear();
                    }
                    Err(why) => error!("settings applied, but not kept: {why}"),
                }
                status.applied = Some(kept);
                settings.set_if_neq(draft);
            }
        }
    }
}

/// How long after the game becomes a window it keeps asking for the size chosen. Leaving
/// fullscreen, a window manager may restore the size from before after the game has asked for its
/// own, as KDE's does on Wayland.
const WINDOW_SIZE_SETTLES: Duration = Duration::from_secs(1);

/// The display as last shown, and the window size the game is still asking for.
#[derive(Default)]
struct ShownDisplay {
    display: Option<Display>,
    /// The size to ask for again if the window takes another, until this time on the real clock.
    settling: Option<(WindowSize, Duration)>,
}

/// Shows the window as the settings say, whenever they change how. A window is given its size
/// as it becomes one, and given it again for [`WINDOW_SIZE_SETTLES`] if it takes another, so it
/// comes back from fullscreen at the size chosen. After that, the player may resize it freely.
fn apply_display(
    settings: Res<Settings>,
    time: Res<Time<Real>>,
    window: Option<Single<&mut Window, With<PrimaryWindow>>>,
    mut shown: Local<ShownDisplay>,
) {
    let Some(mut window) = window else {
        return;
    };
    let display = settings.display;
    let now = time.elapsed();
    if shown.display != Some(display) {
        window.mode = display.mode.window_mode();
        shown.settling = (display.mode == DisplayMode::Windowed)
            .then_some((display.window_size, now + WINDOW_SIZE_SETTLES));
        shown.display = Some(display);
    }
    let Some((size, until)) = shown.settling else {
        return;
    };
    if now > until {
        shown.settling = None;
        return;
    }
    if window.resolution.physical_size() != UVec2::new(size.width, size.height) {
        window
            .resolution
            .set_physical_resolution(size.width, size.height);
    }
}
