//! The Settings page, over the main menu or the pause menu: the window, the mouse, autosaves and
//! the controls. It edits the draft in [`SettingsPage`], and asks `crate::settings` to apply it or
//! go back through [`SettingsAction`]s.

use bevy::prelude::*;
use bevy::window::{Monitor, PrimaryMonitor};
use bevy_egui::{EguiContexts, egui};

use super::{busy_text, button_row, error_text};
use crate::debug_panel::{CLOCK_KEY, PANEL_KEY, STEP_KEY};
use crate::input_policy::PAUSE_KEY;
use crate::player::flight_hint;
use crate::session::Session;
use crate::settings::{
    Action, AutosaveInterval, DisplayMode, Input, Rebinding, SENSITIVITY_RANGE, Settings,
    SettingsAction, SettingsEditor, SettingsPage, SettingsStatus, SettingsStorage, WindowSize,
    inputs_label,
};

/// How wide the page is, in points.
const PAGE_WIDTH: f32 = 560.0;

/// How wide each control's button is, in points.
const BINDING_WIDTH: f32 = 190.0;

/// The controls, in groups, as the page lists them.
const CONTROL_GROUPS: [(&str, &[Action]); 3] = [
    (
        "Movement",
        &[
            Action::MoveForward,
            Action::MoveBack,
            Action::MoveLeft,
            Action::MoveRight,
            Action::Jump,
            Action::Run,
            Action::Sneak,
            Action::Descend,
        ],
    ),
    (
        "God tools",
        &[
            Action::ChooseDig,
            Action::ChoosePlace,
            Action::ChooseHeatBrush,
            Action::ChooseSpawnFluid,
            Action::Dig,
            Action::Place,
            Action::HeatBrush,
            Action::SpawnFluid,
        ],
    ),
    ("View", &[Action::CycleOverlay]),
];

/// Draws the Settings page while it is open, and the prompt of a control being rebound.
#[expect(clippy::too_many_arguments, reason = "a system's parameters")]
pub(super) fn draw_settings_page(
    mut contexts: EguiContexts,
    mut page: ResMut<SettingsPage>,
    settings: Res<Settings>,
    status: Res<SettingsStatus>,
    storage: Res<SettingsStorage>,
    session: Res<State<Session>>,
    monitors: Query<&Monitor, With<PrimaryMonitor>>,
    mut actions: MessageWriter<SettingsAction>,
) -> Result {
    let Some(editor) = page.editor_mut() else {
        return Ok(());
    };
    let ctx = contexts.ctx_mut()?;
    let screen = monitors
        .iter()
        .next()
        .map(|monitor| WindowSize::new(monitor.physical_width, monitor.physical_height));
    // Over a World, the page dims it as the pause menu does. Over the main menu, there is
    // nothing to dim.
    let backdrop = match session.get() {
        Session::Open => egui::Color32::from_black_alpha(140),
        Session::Closed => egui::Color32::TRANSPARENT,
    };
    let max_height = (ctx.content_rect().height() - 210.0).max(120.0);
    let mut chosen = None;
    egui::Modal::new(egui::Id::new("settings page"))
        .backdrop_color(backdrop)
        .show(ctx, |ui| {
            ui.set_width(PAGE_WIDTH);
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.vertical_centered(|ui| ui.heading("Settings"));
            egui::ScrollArea::vertical()
                .max_height(max_height)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    display_section(ui, editor, screen);
                    mouse_and_saving_sections(ui, editor);
                    controls_section(ui, editor);
                });
            ui.separator();
            let dirty = editor.draft != *settings;
            status_lines(ui, &status, &storage, dirty);
            let back = if dirty { "Cancel" } else { "Back" };
            chosen = button_row(
                ui,
                [("Reset to Defaults", true), ("Apply", dirty), (back, true)],
            );
        });
    match chosen {
        Some(0) => editor.reset_to_defaults(),
        Some(1) => {
            actions.write(SettingsAction::Apply);
        }
        Some(_) => {
            actions.write(SettingsAction::Back);
        }
        None => {}
    }
    rebinding_prompt(ctx, editor);
    Ok(())
}

/// A section's heading.
fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new(title).size(17.0).strong());
}

/// The window's mode and size. The size applies while the game is a window, and is kept while it
/// is fullscreen.
fn display_section(ui: &mut egui::Ui, editor: &mut SettingsEditor, screen: Option<WindowSize>) {
    section(ui, "Display");
    let display = &mut editor.draft.display;
    egui::Grid::new("display settings")
        .num_columns(2)
        .spacing([16.0, 8.0])
        .show(ui, |ui| {
            ui.label("Mode");
            egui::ComboBox::from_id_salt("display mode")
                .selected_text(display.mode.label())
                .width(220.0)
                .show_ui(ui, |ui| {
                    for mode in DisplayMode::ALL {
                        ui.selectable_value(&mut display.mode, mode, mode.label());
                    }
                });
            ui.end_row();

            ui.label("Window size");
            let mut sizes: Vec<WindowSize> = WindowSize::CHOICES
                .into_iter()
                .filter(|size| screen.is_none_or(|screen| size.fits(screen)))
                .collect();
            if !sizes.contains(&display.window_size) {
                sizes.push(display.window_size);
                sizes.sort_by_key(|size| (size.width, size.height));
            }
            egui::ComboBox::from_id_salt("window size")
                .selected_text(display.window_size.to_string())
                .width(220.0)
                .show_ui(ui, |ui| {
                    for size in sizes {
                        ui.selectable_value(&mut display.window_size, size, size.to_string());
                    }
                });
            ui.end_row();
        });
    if display.mode == DisplayMode::BorderlessFullscreen {
        ui.weak("Fullscreen fills the screen. The window size applies once it is a window again.");
    }
}

fn mouse_and_saving_sections(ui: &mut egui::Ui, editor: &mut SettingsEditor) {
    let draft = &mut editor.draft;
    section(ui, "Mouse");
    ui.add(
        egui::Slider::new(&mut draft.mouse_sensitivity, SENSITIVITY_RANGE)
            .logarithmic(true)
            .max_decimals(2)
            .suffix("×")
            .text("Sensitivity"),
    );
    section(ui, "Saving");
    ui.horizontal(|ui| {
        ui.label("Autosave every");
        for interval in AutosaveInterval::ALL {
            ui.selectable_value(&mut draft.autosave, interval, interval.label());
        }
    });
    ui.weak("Only time while the World runs counts. Each save starts the count again.");
}

/// Every control and what it is bound to. Clicking one listens for its new input.
fn controls_section(ui: &mut egui::Ui, editor: &mut SettingsEditor) {
    section(ui, "Controls");
    let mut listen = None;
    let listening_for = match editor.rebinding() {
        Rebinding::Listening { action, .. } | Rebinding::Conflict { action, .. } => Some(*action),
        Rebinding::Idle => None,
    };
    egui::Grid::new("controls")
        .num_columns(2)
        .spacing([16.0, 4.0])
        .min_col_width(240.0)
        .show(ui, |ui| {
            for (group, controls) in CONTROL_GROUPS {
                ui.label(egui::RichText::new(group).strong());
                ui.end_row();
                for &action in controls {
                    ui.label(action.label());
                    let bindings = &editor.draft.bindings;
                    let unbound = bindings.inputs(action).is_empty();
                    let mut text = egui::RichText::new(bindings.label(action));
                    if unbound {
                        text = text.color(ui.visuals().warn_fg_color);
                    }
                    let button = egui::Button::new(text)
                        .selected(listening_for == Some(action))
                        .min_size(egui::vec2(BINDING_WIDTH, 0.0));
                    if ui.add(button).clicked() {
                        listen = Some(action);
                    }
                    ui.end_row();
                }
            }
        });
    if let Some(action) = listen {
        editor.listen(action);
    }
    ui.weak(format!(
        "{} Sneak and Fly down may share a key: one is on foot, the other in flight.",
        flight_hint(&editor.draft.bindings)
    ));
    let key = |key: KeyCode| Input::Key(key).label();
    ui.weak(format!(
        "{} pauses and Alt frees the cursor. {} shows the debug panel, and {} and {} run its \
         clock: these keys can't be changed.",
        key(PAUSE_KEY),
        key(PANEL_KEY),
        key(CLOCK_KEY),
        key(STEP_KEY),
    ));
}

/// What became of the settings: whether they can be kept at all, what was wrong with the file
/// they were read from, and how the latest apply went.
fn status_lines(
    ui: &mut egui::Ui,
    status: &SettingsStatus,
    storage: &SettingsStorage,
    dirty: bool,
) {
    if let Err(why) = storage.store() {
        error_text(
            ui,
            format!("Settings can't be kept: {why}. They apply until you quit."),
        );
    }
    if !status.load_problems.is_empty() {
        error_text(
            ui,
            "Some settings couldn't be read, so they use their defaults:",
        );
        for problem in &status.load_problems {
            error_text(ui, format!("• {problem}."));
        }
    }
    if dirty {
        ui.weak("Changes apply once you choose Apply.");
        return;
    }
    match &status.applied {
        Some(Ok(())) => {
            ui.weak("Settings applied and kept.");
        }
        Some(Err(why)) => error_text(
            ui,
            format!("Settings applied, but not kept: {why}. They last until you quit."),
        ),
        None => {}
    }
}

/// While a control is being rebound: asks for its new input, or what to do about the actions
/// that already use it. The prompt covers the page, so a mouse button pressed to bind a control
/// presses nothing on the page.
fn rebinding_prompt(ctx: &egui::Context, editor: &mut SettingsEditor) {
    let mut choice = None;
    match editor.rebinding().clone() {
        Rebinding::Idle => return,
        Rebinding::Listening { action, refused } => {
            egui::Modal::new(egui::Id::new("rebinding")).show(ctx, |ui| {
                ui.set_width(380.0);
                ui.spacing_mut().item_spacing.y = 8.0;
                ui.heading(format!("Rebind “{}”", action.label()));
                busy_text(ui, "Press a key or mouse button. Esc cancels.");
                if let Some(refused) = refused {
                    error_text(ui, format!("{refused}. Press another."));
                }
            });
        }
        Rebinding::Conflict {
            action,
            inputs,
            taken_from,
        } => {
            egui::Modal::new(egui::Id::new("rebinding")).show(ctx, |ui| {
                ui.set_width(380.0);
                ui.spacing_mut().item_spacing.y = 8.0;
                ui.heading(format!("Rebind “{}”", action.label()));
                let input = inputs_label(&inputs).unwrap_or_default();
                for other in &taken_from {
                    let remaining: Vec<Input> = editor
                        .draft
                        .bindings
                        .inputs(*other)
                        .iter()
                        .copied()
                        .filter(|bound| !inputs.contains(bound))
                        .collect();
                    let after = match inputs_label(&remaining) {
                        Some(kept) => format!("It would keep {kept}."),
                        None => "It would be left unbound.".to_owned(),
                    };
                    ui.label(format!(
                        "{input} is already bound to “{}”, which applies at the same time. {after}",
                        other.label()
                    ));
                }
                choice = button_row(ui, [("Replace", true), ("Cancel", true)]);
            });
        }
    }
    match choice {
        Some(0) => editor.replace(),
        Some(_) => editor.cancel(),
        None => {}
    }
}
