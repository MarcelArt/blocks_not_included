//! The menus: the main menu while no World is open, the pause menu over a paused World, the
//! Settings page over either, and a note of each save during play. They draw what
//! `crate::lifecycle` and `crate::settings` hold, and ask them for everything through
//! [`MenuAction`]s and [`SettingsAction`]s.

mod settings_page;

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, PrimaryEguiContext, egui};

use crate::lifecycle::{
    CatalogRow, CurrentWorld, Departure, Destination, Leaving, Library, MainMenu, MenuAction,
    MenuJob, SaveReason, SaveStatus, Saving, WallClock, parse_seed,
};
use crate::saves::{WorldId, WorldSummary};
use crate::session::{Play, Session};
use crate::settings::{SettingsAction, SettingsPage, SettingsStatus};

/// The main menu's background.
const BACKDROP: Color = Color::srgb(0.06, 0.08, 0.11);

/// How wide the menus are, in points.
const MENU_WIDTH: f32 = 440.0;

/// How tall a menu button is, in points.
const BUTTON_HEIGHT: f32 = 34.0;

/// How long the note of a save that committed stays up.
const SAVED_NOTE: Duration = Duration::from_secs(3);

/// How long the note of a save that failed stays up.
const FAILED_NOTE: Duration = Duration::from_secs(10);

/// The main menu, the pause menu, the Settings page and the note of each save. Needs the
/// lifecycle, the settings, the session and `EguiPlugin`.
pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MenuScreen>()
            .add_systems(OnEnter(Session::Closed), (spawn_menu_camera, show_home))
            .add_systems(
                EguiPrimaryContextPass,
                (
                    draw_main_menu.run_if(in_state(Session::Closed)),
                    (draw_pause_menu, draw_save_note).run_if(in_state(Session::Open)),
                    settings_page::draw_settings_page,
                ),
            );
    }
}

/// Which page of the main menu shows, with what the player has typed into it.
#[derive(Resource, Default)]
enum MenuScreen {
    #[default]
    Home,
    NewGame {
        name: String,
        seed: String,
        /// Whether the page has just opened, so the name field takes the keyboard.
        opened: bool,
    },
    LoadGame {
        /// The World being renamed, and its new name so far.
        renaming: Option<(WorldId, String)>,
    },
}

/// The main menu has no World to look at, so it gets a camera of its own, which egui draws with.
fn spawn_menu_camera(mut commands: Commands) {
    commands.spawn((
        Camera2d,
        Camera {
            clear_color: ClearColorConfig::Custom(BACKDROP),
            ..default()
        },
        PrimaryEguiContext,
        DespawnOnExit(Session::Closed),
    ));
}

fn show_home(mut screen: ResMut<MenuScreen>) {
    *screen = MenuScreen::Home;
}

/// Draws the main menu's page, unless the Settings page shows over it.
#[expect(clippy::too_many_arguments, reason = "a system's parameters")]
fn draw_main_menu(
    mut contexts: EguiContexts,
    mut screen: ResMut<MenuScreen>,
    menu: Res<MainMenu>,
    library: Res<Library>,
    clock: Res<WallClock>,
    (page, settings): (Res<SettingsPage>, Res<SettingsStatus>),
    mut actions: MessageWriter<MenuAction>,
    mut settings_actions: MessageWriter<SettingsAction>,
) -> Result {
    if page.is_open() {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?;
    let mut act = |action| {
        actions.write(action);
    };
    centred_window("main menu", ctx, |ui| match &mut *screen {
        MenuScreen::Home => match home(ui, &library, &settings, &mut act) {
            Some(HomeChoice::Page(next)) => {
                if matches!(next, MenuScreen::LoadGame { .. }) {
                    act(MenuAction::ListWorlds);
                }
                *screen = next;
            }
            Some(HomeChoice::Settings) => {
                settings_actions.write(SettingsAction::Open);
            }
            None => {}
        },
        MenuScreen::NewGame { name, seed, opened } => {
            if new_game(ui, name, seed, opened, &menu, &mut act) {
                *screen = MenuScreen::Home;
            }
        }
        MenuScreen::LoadGame { renaming } => {
            if load_game(ui, renaming, &menu, clock.now(), &mut act) {
                *screen = MenuScreen::Home;
            }
        }
    });
    if let Some(row) = menu.confirming_delete.and_then(|id| menu.row(id)) {
        confirm_delete(ctx, row, clock.now(), menu.job.is_some(), &mut act);
    }
    Ok(())
}

/// A window without a title bar in the middle of the screen.
fn centred_window(id: &str, ctx: &egui::Context, contents: impl FnOnce(&mut egui::Ui)) {
    egui::Window::new(id)
        .title_bar(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .resizable(false)
        .collapsible(false)
        .fixed_size([MENU_WIDTH, 0.0])
        .show(ctx, |ui| {
            ui.set_width(MENU_WIDTH);
            ui.spacing_mut().item_spacing.y = 8.0;
            contents(ui);
        });
}

/// A button as wide as the menu, with its label in the middle.
fn wide_button(ui: &mut egui::Ui, enabled: bool, label: &str) -> bool {
    ui.vertical_centered(|ui| {
        let button = egui::Button::new(egui::RichText::new(label).size(16.0))
            .min_size(egui::vec2(ui.available_width(), BUTTON_HEIGHT));
        ui.add_enabled(enabled, button).clicked()
    })
    .inner
}

/// Buttons side by side, as wide as the menu between them. Returns the one clicked.
fn button_row<const N: usize>(ui: &mut egui::Ui, buttons: [(&str, bool); N]) -> Option<usize> {
    let mut clicked = None;
    ui.columns(N, |columns| {
        for (index, (column, (label, enabled))) in columns.iter_mut().zip(buttons).enumerate() {
            if wide_button(column, enabled, label) {
                clicked = Some(index);
            }
        }
    });
    clicked
}

/// `text` in the error colour, starting with a capital: error messages start in lower case.
fn error_text(ui: &mut egui::Ui, text: impl Into<String>) {
    let text = text.into();
    let mut chars = text.chars();
    let text = match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => text,
    };
    let color = ui.visuals().error_fg_color;
    ui.label(egui::RichText::new(text).color(color));
}

fn busy_text(ui: &mut egui::Ui, text: &str) {
    ui.horizontal(|ui| {
        ui.spinner();
        ui.label(text);
    });
}

/// Where the first page leads.
enum HomeChoice {
    Page(MenuScreen),
    Settings,
}

/// The first page: New Game, Load Game, Settings, Exit. Returns where to go next, if the player
/// chose.
fn home(
    ui: &mut egui::Ui,
    library: &Library,
    settings: &SettingsStatus,
    act: &mut impl FnMut(MenuAction),
) -> Option<HomeChoice> {
    ui.vertical_centered(|ui| {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("Blocks Not Included")
                .size(34.0)
                .strong(),
        );
        ui.label(egui::RichText::new("Every Cell simulated").weak());
        ui.add_space(12.0);
    });
    if let Err(why) = library.store() {
        error_text(ui, format!("Worlds can't be saved: {why}"));
    }
    if !settings.load_problems.is_empty() {
        error_text(
            ui,
            "Some settings couldn't be read, so they use their defaults. Settings says which.",
        );
    }
    let mut next = None;
    if wide_button(ui, true, "New Game") {
        next = Some(HomeChoice::Page(MenuScreen::NewGame {
            name: String::new(),
            seed: String::new(),
            opened: true,
        }));
    }
    if wide_button(ui, true, "Load Game") {
        next = Some(HomeChoice::Page(MenuScreen::LoadGame { renaming: None }));
    }
    if wide_button(ui, true, "Settings") {
        next = Some(HomeChoice::Settings);
    }
    if wide_button(ui, true, "Exit") {
        act(MenuAction::Exit);
    }
    ui.add_space(4.0);
    next
}

/// The New Game form. Returns whether to go back to the first page.
fn new_game(
    ui: &mut egui::Ui,
    name: &mut String,
    seed: &mut String,
    opened: &mut bool,
    menu: &MainMenu,
    act: &mut impl FnMut(MenuAction),
) -> bool {
    let creating = menu.job == Some(MenuJob::Creating);
    let busy = menu.job.is_some();
    ui.heading("New Game");
    ui.label("Name");
    let name_field = ui.add_enabled(
        !creating,
        egui::TextEdit::singleline(name)
            .hint_text("My World")
            .desired_width(f32::INFINITY),
    );
    if std::mem::take(opened) {
        name_field.request_focus();
    }
    ui.label("World Seed");
    let seed_field = ui.add_enabled(
        !creating,
        egui::TextEdit::singleline(seed)
            .hint_text("Blank for a random seed")
            .desired_width(f32::INFINITY),
    );
    let parsed = parse_seed(seed);
    if let Err(error) = parsed {
        error_text(ui, format!("{error}."));
    } else {
        ui.label(egui::RichText::new("The same seed always starts with the same terrain.").weak());
    }
    if creating {
        busy_text(ui, "Creating the World…");
    }
    if let Some(problem) = menu
        .problem
        .as_ref()
        .filter(|problem| problem.job == MenuJob::Creating)
    {
        error_text(
            ui,
            format!("The World wasn't created: {}.", problem.message),
        );
    }
    let ready = !name.trim().is_empty() && !busy;
    let submitted = (name_field.lost_focus() || seed_field.lost_focus())
        && ui.input(|input| input.key_pressed(egui::Key::Enter));
    ui.add_space(4.0);
    let clicked = button_row(
        ui,
        [("Create", ready && parsed.is_ok()), ("Back", !creating)],
    );
    if let Ok(seed) = parsed
        && ready
        && (clicked == Some(0) || submitted)
    {
        act(MenuAction::NewGame {
            name: name.trim().to_owned(),
            seed,
        });
    }
    clicked == Some(1)
}

/// The list of saved Worlds, and what can be done to the one selected. Returns whether to go back
/// to the first page.
fn load_game(
    ui: &mut egui::Ui,
    renaming: &mut Option<(WorldId, String)>,
    menu: &MainMenu,
    now: SystemTime,
    act: &mut impl FnMut(MenuAction),
) -> bool {
    let busy = menu.job.is_some();
    ui.heading("Load Game");
    if menu.worlds.is_empty() && menu.job != Some(MenuJob::Listing) {
        ui.label("No saved Worlds yet. Start one with New Game.");
    }
    egui::ScrollArea::vertical()
        .max_height(320.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for row in &menu.worlds {
                let response = world_row(ui, row, menu.selected == Some(row.id), now);
                if response.clicked() {
                    act(MenuAction::Select(row.id));
                }
                if response.double_clicked() && loadable(row) && !busy {
                    act(MenuAction::LoadGame(row.id));
                }
            }
        });

    match &menu.job {
        Some(MenuJob::Listing) => busy_text(ui, "Reading the saved Worlds…"),
        Some(MenuJob::Loading(_)) => busy_text(ui, "Loading the World…"),
        Some(MenuJob::Renaming(_)) => busy_text(ui, "Renaming…"),
        Some(MenuJob::Deleting(_)) => busy_text(ui, "Deleting…"),
        Some(MenuJob::Creating) | None => {}
    }
    if let Some(problem) = &menu.problem {
        let what = match problem.job {
            MenuJob::Listing => "The saved Worlds couldn't be read",
            MenuJob::Creating => "The World wasn't created",
            MenuJob::Loading(_) => "The World didn't load",
            MenuJob::Renaming(_) => "The World wasn't renamed",
            MenuJob::Deleting(_) => "The World wasn't deleted",
        };
        error_text(ui, format!("{what}: {}.", problem.message));
    }

    ui.add_space(4.0);
    if let Some((id, name)) = renaming {
        ui.label("New name");
        let field = ui.add(
            egui::TextEdit::singleline(name)
                .id(rename_field_id())
                .desired_width(f32::INFINITY),
        );
        let submitted = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        let (id, name) = (*id, name.trim().to_owned());
        let ready = !busy && !name.is_empty();
        match button_row(ui, [("Save Name", ready), ("Cancel", true)]) {
            Some(0) => {}
            Some(_) => {
                *renaming = None;
                return false;
            }
            None if submitted && ready => {}
            None => return false,
        }
        act(MenuAction::Rename { id, name });
        *renaming = None;
        return false;
    }
    let selected = menu.selected.and_then(|id| menu.row(id));
    let chosen = button_row(
        ui,
        [
            ("Load", !busy && selected.is_some_and(loadable)),
            (
                "Rename",
                !busy && selected.is_some_and(|row| row.world.is_ok()),
            ),
            ("Delete", !busy && selected.is_some()),
            ("Back", !matches!(menu.job, Some(MenuJob::Loading(_)))),
        ],
    );
    match (chosen, selected) {
        (Some(0), Some(row)) => act(MenuAction::LoadGame(row.id)),
        (Some(1), Some(row)) => {
            if let Ok(world) = &row.world {
                *renaming = Some((row.id, world.name.clone()));
                ui.memory_mut(|memory| memory.request_focus(rename_field_id()));
            }
        }
        (Some(2), Some(row)) => act(MenuAction::AskToDelete(row.id)),
        (Some(3), _) => return true,
        _ => {}
    }
    false
}

fn rename_field_id() -> egui::Id {
    egui::Id::new("rename field")
}

/// Whether this build can load the World, as far as the list can tell.
fn loadable(row: &CatalogRow) -> bool {
    row.world
        .as_ref()
        .is_ok_and(|world| world.compatibility.is_ok())
}

/// One World's row, which selects it: its name and when it was last played, and below them its
/// seed, or why it can't load.
fn world_row(
    ui: &mut egui::Ui,
    row: &CatalogRow,
    selected: bool,
    now: SystemTime,
) -> egui::Response {
    const HEIGHT: f32 = 44.0;
    const MARGIN: f32 = 8.0;
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), HEIGHT),
        egui::Sense::click(),
    );
    let visuals = ui.style().interact_selectable(&response, selected);
    if selected || response.hovered() {
        let fill = if selected {
            ui.visuals().selection.bg_fill
        } else {
            visuals.weak_bg_fill
        };
        ui.painter().rect_filled(rect, visuals.corner_radius, fill);
    }
    let strong = if selected {
        ui.visuals().selection.stroke.color
    } else {
        ui.visuals().strong_text_color()
    };
    let weak = ui.visuals().weak_text_color();
    let error = ui.visuals().error_fg_color;
    let (name, last_played, detail, detail_color) = match &row.world {
        Ok(world) => {
            let detail = match &world.compatibility {
                Ok(()) => (format!("Seed {}", world.seed), weak),
                Err(why) => (format!("Can't load: {why}"), error),
            };
            let ago = time_ago(world.last_played, now);
            (
                world.name.clone(),
                Some((ago, world.last_played)),
                detail.0,
                detail.1,
            )
        }
        Err(why) => (
            format!("Unreadable save {}", row.id),
            None,
            why.clone(),
            error,
        ),
    };
    let inner = rect.shrink2(egui::vec2(MARGIN, 5.0));
    let mut right_edge = inner.right();
    if let Some((ago, _)) = &last_played {
        let galley = text_galley(ui, ago.clone(), 13.0, weak, inner.width());
        right_edge -= galley.size().x + MARGIN;
        ui.painter().galley(
            egui::pos2(inner.right() - galley.size().x, inner.top() + 2.0),
            galley,
            weak,
        );
    }
    let name = text_galley(ui, name, 16.0, strong, right_edge - inner.left());
    ui.painter().galley(inner.left_top(), name, strong);
    let detail = text_galley(ui, detail, 13.0, detail_color, inner.width());
    ui.painter().galley(
        egui::pos2(inner.left(), inner.bottom() - detail.size().y),
        detail,
        detail_color,
    );
    match last_played {
        Some((_, at)) => response.on_hover_text(format!("Last played {}", utc_minute(at))),
        None => response,
    }
}

/// `text` laid out on one line, cut short with "…" if it is wider than `max_width`.
fn text_galley(
    ui: &egui::Ui,
    text: String,
    size: f32,
    color: egui::Color32,
    max_width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::single_section(
        text,
        egui::TextFormat::simple(egui::FontId::proportional(size), color),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_width.max(0.0));
    ui.painter().layout_job(job)
}

/// Asks the player to confirm deleting a World, showing what it is.
fn confirm_delete(
    ctx: &egui::Context,
    row: &CatalogRow,
    now: SystemTime,
    busy: bool,
    act: &mut impl FnMut(MenuAction),
) {
    egui::Modal::new(egui::Id::new("confirm delete")).show(ctx, |ui| {
        ui.set_width(360.0);
        ui.spacing_mut().item_spacing.y = 8.0;
        match &row.world {
            Ok(world) => {
                ui.heading(format!("Delete “{}”?", world.name));
                summary_grid(ui, world, now);
            }
            Err(why) => {
                ui.heading("Delete this unreadable save?");
                error_text(ui, why.clone());
            }
        }
        ui.label("This can't be undone.");
        if busy {
            busy_text(ui, "Deleting…");
        }
        match button_row(ui, [("Delete", !busy), ("Cancel", !busy)]) {
            Some(0) => act(MenuAction::ConfirmDelete),
            Some(_) => act(MenuAction::CancelDelete),
            None => {}
        }
    });
}

/// A World's seed and last-played time.
fn summary_grid(ui: &mut egui::Ui, world: &WorldSummary, now: SystemTime) {
    egui::Grid::new(("summary", world.id))
        .num_columns(2)
        .show(ui, |ui| {
            ui.label("Seed");
            ui.label(world.seed.to_string());
            ui.end_row();
            ui.label("Last played");
            ui.label(format!(
                "{} ({})",
                time_ago(world.last_played, now),
                utc_minute(world.last_played)
            ));
            ui.end_row();
        });
}

/// The pause menu, over the World while it is paused or being left, unless the Settings page
/// shows over it.
#[expect(clippy::too_many_arguments, reason = "a system's parameters")]
fn draw_pause_menu(
    mut contexts: EguiContexts,
    play: Res<State<Play>>,
    current: Option<Res<CurrentWorld>>,
    saving: Res<Saving>,
    leaving: Res<Leaving>,
    (time, clock): (Res<Time<Real>>, Res<WallClock>),
    page: Res<SettingsPage>,
    mut actions: MessageWriter<MenuAction>,
    mut settings_actions: MessageWriter<SettingsAction>,
) -> Result {
    if *play.get() == Play::Running || page.is_open() {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?;
    let mut act = |action| {
        actions.write(action);
    };
    egui::Modal::new(egui::Id::new("pause menu"))
        .backdrop_color(egui::Color32::from_black_alpha(140))
        .show(ctx, |ui| {
            ui.set_width(MENU_WIDTH - 80.0);
            ui.spacing_mut().item_spacing.y = 8.0;
            if let Some(departure) = &leaving.0 {
                leaving_dialog(ui, departure, &mut act);
                return;
            }
            ui.vertical_centered(|ui| ui.heading("Paused"));
            match &current {
                Some(current) => {
                    ui.label(egui::RichText::new(&current.0.name).size(18.0).strong());
                    summary_grid(ui, &current.0, clock.now());
                }
                None => {
                    ui.label("This World isn't saved anywhere.");
                }
            }
            save_status(ui, &saving, time.elapsed());
            if wide_button(ui, true, "Resume") {
                act(MenuAction::Resume);
            }
            let can_save = current.is_some() && saving.in_progress().is_none();
            if wide_button(ui, can_save, "Save") {
                act(MenuAction::Save);
            }
            if wide_button(ui, true, "Settings") {
                settings_actions.write(SettingsAction::Open);
            }
            if wide_button(ui, true, "Return to Main Menu") {
                act(MenuAction::ReturnToMainMenu);
            }
        });
    Ok(())
}

/// What the pause menu says while the player leaves: that the World is saving, or that it didn't,
/// with the choices that leaves.
fn leaving_dialog(ui: &mut egui::Ui, departure: &Departure, act: &mut impl FnMut(MenuAction)) {
    let leaving_for = match departure.to {
        Destination::MainMenu => "returning to the main menu",
        Destination::Desktop => "quitting",
    };
    let Some(why) = &departure.failed else {
        ui.heading("Saving…");
        busy_text(ui, &format!("Saving the World before {leaving_for}."));
        return;
    };
    ui.heading("The World didn't save");
    error_text(ui, format!("{why}."));
    ui.label("Its last save is kept as it was.");
    if wide_button(ui, true, "Retry") {
        act(MenuAction::RetryLeaving);
    }
    let back = if departure.back_to == Play::Running {
        "Cancel and Keep Playing"
    } else {
        "Cancel"
    };
    if wide_button(ui, true, back) {
        act(MenuAction::CancelLeaving);
    }
    if wide_button(ui, true, "Leave Without Saving") {
        act(MenuAction::LeaveWithoutSaving);
    }
}

/// A line on the latest save.
fn save_status(ui: &mut egui::Ui, saving: &Saving, now: Duration) {
    if saving.in_progress().is_some() {
        busy_text(ui, "Saving…");
        return;
    }
    match &saving.last {
        SaveStatus::None => {}
        SaveStatus::Saved { reason, at } => {
            let ago = now.saturating_sub(*at).as_secs();
            let what = match reason {
                SaveReason::Autosave => "Autosaved",
                SaveReason::Manual | SaveReason::Leaving => "Saved",
            };
            ui.label(egui::RichText::new(format!("{what} {}", seconds_ago(ago))).weak());
        }
        SaveStatus::Failed {
            reason, message, ..
        } => {
            let what = match reason {
                SaveReason::Autosave => "The autosave failed",
                SaveReason::Manual | SaveReason::Leaving => "The World didn't save",
            };
            error_text(ui, format!("{what}: {message}. Its last save is kept."));
        }
    }
}

fn seconds_ago(seconds: u64) -> String {
    match seconds {
        0..5 => "just now".to_owned(),
        5..60 => format!("{seconds} seconds ago"),
        _ => match seconds / 60 {
            1 => "a minute ago".to_owned(),
            minutes => format!("{minutes} minutes ago"),
        },
    }
}

/// A note at the top of the screen during play while the World saves, and for a while after.
fn draw_save_note(
    mut contexts: EguiContexts,
    play: Res<State<Play>>,
    saving: Res<Saving>,
    time: Res<Time<Real>>,
) -> Result {
    if *play.get() != Play::Running {
        return Ok(());
    }
    let now = time.elapsed();
    let note = match &saving.last {
        _ if saving.in_progress().is_some() => Some((false, "Saving…".to_owned())),
        SaveStatus::Saved { reason, at } if now.saturating_sub(*at) < SAVED_NOTE => {
            let what = match reason {
                SaveReason::Autosave => "Autosaved",
                SaveReason::Manual | SaveReason::Leaving => "Saved",
            };
            Some((false, what.to_owned()))
        }
        SaveStatus::Failed {
            reason,
            message,
            at,
        } if now.saturating_sub(*at) < FAILED_NOTE => {
            let what = match reason {
                SaveReason::Autosave => "The autosave failed",
                SaveReason::Manual | SaveReason::Leaving => "The World didn't save",
            };
            Some((true, format!("{what}: {message}. Its last save is kept.")))
        }
        _ => None,
    };
    let Some((failed, text)) = note else {
        return Ok(());
    };
    let ctx = contexts.ctx_mut()?;
    egui::Area::new(egui::Id::new("save note"))
        .anchor(egui::Align2::CENTER_TOP, [0.0, 12.0])
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_max_width(MENU_WIDTH);
                if failed {
                    error_text(ui, text);
                } else {
                    ui.label(text);
                }
            });
        });
    Ok(())
}

/// How long before `now` the time `then` was, as the World list says it: "just now",
/// "5 minutes ago", "3 hours ago", "yesterday", "4 days ago", or, a month or more back, the date.
pub fn time_ago(then: SystemTime, now: SystemTime) -> String {
    let Ok(ago) = now.duration_since(then) else {
        return utc_minute(then);
    };
    let minutes = ago.as_secs() / 60;
    let hours = minutes / 60;
    let days = hours / 24;
    match (minutes, hours, days) {
        (0, _, _) => "just now".to_owned(),
        (1, _, _) => "a minute ago".to_owned(),
        (minutes, 0, _) => format!("{minutes} minutes ago"),
        (_, 1, _) => "an hour ago".to_owned(),
        (_, hours, 0) => format!("{hours} hours ago"),
        (_, _, 1) => "yesterday".to_owned(),
        (_, _, days @ 2..30) => format!("{days} days ago"),
        _ => utc_minute(then)[..10].to_owned(),
    }
}

/// `time` in UTC, to the minute: "2026-11-27 16:38 UTC".
pub fn utc_minute(time: SystemTime) -> String {
    let seconds = match time.duration_since(UNIX_EPOCH) {
        Ok(since) => since.as_secs() as i64,
        Err(before) => -(before.duration().as_secs() as i64),
    };
    let (days, of_day) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        of_day / 3_600,
        of_day % 3_600 / 60
    )
}

/// The proleptic Gregorian date `days` after 1970-01-01, after Howard Hinnant's
/// `civil_from_days`.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_from_march = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_from_march + 2) / 5 + 1) as u32;
    let month = if month_from_march < 10 {
        month_from_march + 3
    } else {
        month_from_march - 9
    } as u32;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(seconds: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(seconds)
    }

    #[test]
    fn utc_times_read_as_calendar_dates() {
        assert_eq!(utc_minute(at(0)), "1970-01-01 00:00 UTC");
        // The billionth second, and the 1.7 billionth: well-known dates.
        assert_eq!(utc_minute(at(1_000_000_000)), "2001-09-09 01:46 UTC");
        assert_eq!(utc_minute(at(1_700_000_000)), "2023-11-14 22:13 UTC");
        // 2024-02-29 12:00 UTC = 19,782 days × 86,400 s + 12 h: a leap day.
        assert_eq!(
            utc_minute(at(19_782 * 86_400 + 43_200)),
            "2024-02-29 12:00 UTC"
        );
    }

    #[test]
    fn last_played_reads_as_how_long_ago() {
        let now = at(1_700_000_000);
        let ago = |seconds: u64| time_ago(at(1_700_000_000 - seconds), now);
        assert_eq!(ago(30), "just now");
        assert_eq!(ago(90), "a minute ago");
        assert_eq!(ago(5 * 60), "5 minutes ago");
        assert_eq!(ago(3_600 + 60), "an hour ago");
        assert_eq!(ago(5 * 3_600), "5 hours ago");
        assert_eq!(ago(30 * 3_600), "yesterday");
        assert_eq!(ago(4 * 86_400), "4 days ago");
        // 40 days before 2023-11-14 is 2023-10-05.
        assert_eq!(ago(40 * 86_400), "2023-10-05");
        // A time after now, as a clock set back could give, shows as it is.
        assert_eq!(time_ago(at(1_700_000_060), now), "2023-11-14 22:14 UTC");
    }
}
