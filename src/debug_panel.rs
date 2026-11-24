//! The debug panel in the bottom-left corner, which F3 shows and hides: how the simulation is
//! doing, its clock, and what became of the latest version of each of its data files. A new
//! problem with a file shows the panel.
//!
//! P stops the clock and starts it again, and N runs one Tick while it is stopped. The panel's
//! buttons do the same, once Esc has freed the cursor.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use bevy_egui::input::egui_wants_any_keyboard_input;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};

use crate::readout::group_thousands;
use crate::sim_bridge::{SimBridge, SimCommand, SimReport, TickRate};
use crate::sim_files::{FileStatus, SimFileStatus};

const TOGGLE_KEY: KeyCode = KeyCode::F3;
const PAUSE_KEY: KeyCode = KeyCode::KeyP;
const STEP_KEY: KeyCode = KeyCode::KeyN;

/// How wide the panel's text runs before it wraps, in points, so a long error message doesn't
/// stretch the panel across the screen.
const PANEL_WIDTH: f32 = 320.0;

/// The debug panel, and the keys for the simulation's clock. Needs the sim bridge, the
/// simulation files, and `EguiPlugin`.
pub struct DebugPanelPlugin;

impl Plugin for DebugPanelPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DebugPanel>()
            .init_resource::<SimClock>()
            .add_systems(
                Update,
                (
                    (toggle_panel, use_clock_keys).run_if(not(egui_wants_any_keyboard_input)),
                    show_new_problems.run_if(resource_changed::<SimFileStatus>),
                ),
            )
            .add_systems(EguiPrimaryContextPass, draw_panel);
    }
}

/// Whether the debug panel is shown.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DebugPanel {
    pub shown: bool,
}

/// Whether the simulation's clock is stopped, as the debug panel last set it. It starts running.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SimClock {
    pub paused: bool,
}

/// What the player asks of the simulation's clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClockAction {
    Pause,
    Resume,
    Step,
}

/// The simulation's clock, as the panel and the keys control it.
#[derive(SystemParam)]
struct Clock<'w> {
    state: ResMut<'w, SimClock>,
    bridge: Res<'w, SimBridge>,
}

impl Clock<'_> {
    fn paused(&self) -> bool {
        self.state.paused
    }

    fn apply(&mut self, action: ClockAction) {
        let command = match action {
            ClockAction::Pause => {
                self.state.paused = true;
                SimCommand::Pause
            }
            ClockAction::Resume => {
                self.state.paused = false;
                SimCommand::Resume
            }
            ClockAction::Step => SimCommand::Step,
        };
        self.bridge.send(command);
    }
}

fn toggle_panel(keys: Res<ButtonInput<KeyCode>>, mut panel: ResMut<DebugPanel>) {
    if keys.just_pressed(TOGGLE_KEY) {
        panel.shown = !panel.shown;
    }
}

/// P stops or starts the clock; N runs one Tick while it is stopped.
fn use_clock_keys(keys: Res<ButtonInput<KeyCode>>, mut clock: Clock) {
    if keys.just_pressed(PAUSE_KEY) {
        clock.apply(if clock.paused() {
            ClockAction::Resume
        } else {
            ClockAction::Pause
        });
    }
    if keys.just_pressed(STEP_KEY) && clock.paused() {
        clock.apply(ClockAction::Step);
    }
}

/// Shows the panel when a file's latest version is rejected for a new reason.
fn show_new_problems(
    status: Res<SimFileStatus>,
    mut before: Local<SimFileStatus>,
    mut panel: ResMut<DebugPanel>,
) {
    let new_problem = status
        .files()
        .into_iter()
        .zip(before.files())
        .any(|((_, now), (_, then))| matches!(now, FileStatus::Rejected(_)) && now != then);
    if new_problem {
        panel.shown = true;
    }
    *before = status.clone();
}

/// Draws the panel while it is shown. While the cursor is captured, clicks pass through it, as
/// they do through the god tools' palette.
fn draw_panel(
    mut contexts: EguiContexts,
    panel: Res<DebugPanel>,
    report: Res<SimReport>,
    rate: Res<TickRate>,
    files: Res<SimFileStatus>,
    mut clock: Clock,
    cursor: Single<&CursorOptions, With<PrimaryWindow>>,
) -> Result {
    if !panel.shown {
        return Ok(());
    }
    let captured = cursor.grab_mode != CursorGrabMode::None;
    let mut action = None;
    egui::Window::new("Simulation")
        .anchor(egui::Align2::LEFT_BOTTOM, [8.0, -8.0])
        .resizable(false)
        .collapsible(false)
        .interactable(!captured)
        .show(contexts.ctx_mut()?, |ui| {
            ui.set_max_width(PANEL_WIDTH);
            stats_grid(ui, &report, rate.0, clock.paused());
            ui.horizontal(|ui| {
                let running = !clock.paused();
                for (label, enabled, chosen) in [
                    ("Pause (P)", running, ClockAction::Pause),
                    ("Resume (P)", !running, ClockAction::Resume),
                    ("Step (N)", !running, ClockAction::Step),
                ] {
                    if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                        action = Some(chosen);
                    }
                }
            });
            ui.separator();
            for (path, status) in files.files() {
                match status {
                    FileStatus::AsStarted => ui.weak(format!("{path}: as the game started")),
                    FileStatus::Applied { tick } => ui.label(format!(
                        "{path}: reloaded at Tick {}",
                        group_thousands(&tick.to_string())
                    )),
                    FileStatus::Rejected(message) => ui.colored_label(
                        ui.visuals().error_fg_color,
                        format!("{path}: {message}. The simulation keeps the version before."),
                    ),
                };
            }
        });
    if let Some(action) = action {
        clock.apply(action);
    }
    Ok(())
}

/// Rows for the clock, the Ticks, Mass and the Chunks.
fn stats_grid(ui: &mut egui::Ui, report: &SimReport, ticks_per_second: f64, paused: bool) {
    let stats = &report.stats;
    let rows = [
        (
            "Clock",
            if paused { "Stopped" } else { "Running" }.to_owned(),
        ),
        ("Tick", group_thousands(&report.tick.to_string())),
        ("Ticks per second", format!("{ticks_per_second:.1}")),
        (
            "Last Tick took",
            format!("{:.1} ms", report.tick_duration.as_secs_f64() * 1_000.0),
        ),
        ("Total Mass", format_kilograms(stats.total_mass_grams)),
        (
            "Lost to space",
            format_kilograms(stats.mass_lost_to_space_grams),
        ),
        ("Active Chunks", stats.active_chunks.to_string()),
        ("Settled Chunks", stats.settled_chunks.to_string()),
        ("Dormant Chunks", stats.dormant_chunks.to_string()),
    ];
    egui::Grid::new("simulation stats")
        .num_columns(2)
        .show(ui, |ui| {
            for (name, value) in rows {
                ui.label(name);
                ui.label(value);
                ui.end_row();
            }
        });
}

/// `grams` in kilograms, to the gram, so a change of a single gram shows: "1,234.567 kg".
fn format_kilograms(grams: u64) -> String {
    let whole = group_thousands(&(grams / 1_000).to_string());
    format!("{whole}.{:03} kg", grams % 1_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mass_is_shown_in_kilograms_to_the_gram() {
        assert_eq!(format_kilograms(0), "0.000 kg");
        // 1,430 g = 1.43 kg: a full Cell of Oxygen.
        assert_eq!(format_kilograms(1_430), "1.430 kg");
        // 2,831,155,200,123 g ≈ 1,000,000 full Cells of Granite, plus 123 g.
        assert_eq!(format_kilograms(2_831_155_200_123), "2,831,155,200.123 kg");
    }
}
