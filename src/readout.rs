//! The crosshair, and the readout panel in the top-right corner. The panel shows the Cell in view
//! (the Target) and the Cell at the player's eyes (At head), as the game last heard them from the
//! simulation: up to one Tick old (ADR 0003).

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use sim::{Cell, ElementTable, Phase};

use crate::player::raycast::REACH;
use crate::player::{AwaitingSpawn, Player};
use crate::sim_bridge::Elements;
use crate::tools::Aim;
use crate::world::WorldView;

/// How far each arm of the crosshair reaches from the centre of the screen, in points.
const CROSSHAIR_ARM: f32 = 6.0;

/// The crosshair and the readout panel. Needs the sim bridge, the player, the god tools (for the
/// Cell in view) and `EguiPlugin`.
pub struct ReadoutPlugin;

impl Plugin for ReadoutPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(EguiPrimaryContextPass, (draw_crosshair, draw_readout));
    }
}

/// Draws a small white cross, outlined in black so it shows against the sky, at the centre of the
/// screen: where the player's line of sight points.
fn draw_crosshair(mut contexts: EguiContexts) -> Result {
    let ctx = contexts.ctx_mut()?;
    let centre = ctx.viewport_rect().center();
    // Behind egui's windows, in front of the world.
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Background,
        egui::Id::new("crosshair"),
    ));
    let outline = egui::Stroke::new(3.0, egui::Color32::from_black_alpha(160));
    let cross = egui::Stroke::new(1.0, egui::Color32::WHITE);
    for (arm, stroke) in [(CROSSHAIR_ARM + 1.0, outline), (CROSSHAIR_ARM, cross)] {
        let (across, down) = (egui::vec2(arm, 0.0), egui::vec2(0.0, arm));
        painter.line_segment([centre - across, centre + across], stroke);
        painter.line_segment([centre - down, centre + down], stroke);
    }
    Ok(())
}

/// Draws the readout panel in the top-right corner.
fn draw_readout(
    mut contexts: EguiContexts,
    aim: Res<Aim>,
    view: Res<WorldView>,
    elements: Res<Elements>,
    eyes: Query<&Transform, (With<Player>, Without<AwaitingSpawn>)>,
) -> Result {
    egui::Window::new("Readout")
        .anchor(egui::Align2::RIGHT_TOP, [-8.0, 8.0])
        .title_bar(false)
        .resizable(false)
        // It only shows things, so it never takes the pointer.
        .interactable(false)
        .show(contexts.ctx_mut()?, |ui| {
            // Wide enough for most values, so the panel doesn't jitter as they change.
            ui.set_min_width(190.0);
            ui.strong("Target");
            match **aim {
                Some(hit) => {
                    let cell = view.cell(cell_pos(hit.cell));
                    cell_rows(ui, "target", hit.cell, cell, &elements);
                }
                None => {
                    ui.weak(format!("Nothing solid or liquid within {REACH} m"));
                }
            }
            ui.separator();
            ui.strong("At head");
            match eyes.single() {
                Ok(eyes) => {
                    let pos = eyes.translation.floor().as_ivec3();
                    cell_rows(ui, "at head", pos, view.cell(cell_pos(pos)), &elements);
                }
                Err(_) => {
                    ui.weak("Waiting to spawn");
                }
            }
        });
    Ok(())
}

/// Rows naming the Cell at `pos`, `cell` as the world view holds it, and its Element, Phase,
/// Mass and temperature. A gas's Mass is its pressure. Vacuum has neither Mass nor temperature.
fn cell_rows(ui: &mut egui::Ui, id: &str, pos: IVec3, cell: Option<Cell>, elements: &ElementTable) {
    egui::Grid::new(id).num_columns(2).show(ui, |ui| {
        ui.weak("Cell");
        ui.weak(format!("({}, {}, {})", pos.x, pos.y, pos.z));
        ui.end_row();
        let Some(cell) = cell else {
            ui.label("Not loaded");
            ui.end_row();
            return;
        };
        let element = elements.get(cell.element);
        ui.label("Element");
        ui.label(&element.name);
        ui.end_row();
        ui.label("Phase");
        ui.label(phase_name(element.phase));
        ui.end_row();
        if element.phase == Phase::Vacuum {
            return;
        }
        ui.label(if element.phase == Phase::Gas {
            "Mass (pressure)"
        } else {
            "Mass"
        });
        ui.label(format_mass(cell.mass_grams));
        ui.end_row();
        ui.label("Temperature");
        ui.label(format_temperature(cell.temperature_k));
        ui.end_row();
    });
}

fn phase_name(phase: Phase) -> &'static str {
    match phase {
        Phase::Solid => "Solid",
        Phase::Liquid => "Liquid",
        Phase::Gas => "Gas",
        Phase::Vacuum => "Vacuum",
    }
}

fn cell_pos(cell: IVec3) -> sim::CellPos {
    sim::CellPos::new(cell.x, cell.y, cell.z)
}

/// `temperature_k` for the readout, in degrees Celsius to two decimal places.
pub fn format_temperature(temperature_k: f32) -> String {
    let celsius = temperature_k - 273.15;
    // Just below 0 °C would otherwise show as "-0.00 °C".
    let celsius = if celsius.abs() < 0.005 { 0.0 } else { celsius };
    format!("{celsius:.2} °C")
}

/// `grams` for the readout, in the unit its size calls for: whole grams below 1 kg, kilograms up
/// to 1,000 kg (a full Cell of Water), and tonnes above that.
pub fn format_mass(grams: u32) -> String {
    if grams < 1_000 {
        return format!("{grams} g");
    }
    let (amount, unit) = if grams <= 1_000_000 {
        (f64::from(grams) / 1e3, "kg")
    } else {
        (f64::from(grams) / 1e6, "t")
    };
    format!("{} {unit}", three_significant_digits(amount))
}

/// `amount`, at least 1, to three significant digits without trailing zeros, and with commas
/// between thousands from 1,000 on.
fn three_significant_digits(amount: f64) -> String {
    let decimals = if amount < 10.0 {
        2
    } else if amount < 100.0 {
        1
    } else {
        0
    };
    // Rounding may carry into another digit, as 9.996 does to "10.00"; trimming tidies that up.
    let rounded = format!("{amount:.decimals$}");
    let rounded = if rounded.contains('.') {
        rounded.trim_end_matches('0').trim_end_matches('.')
    } else {
        &rounded
    };
    let (whole, fraction) = rounded.split_at(rounded.find('.').unwrap_or(rounded.len()));
    let mut grouped = String::new();
    for (i, digit) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped + fraction
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mass_is_shown_in_the_unit_its_size_calls_for() {
        assert_eq!(format_mass(12), "12 g");
        // 1,430 g = 1.43 kg: a full Cell of Oxygen.
        assert_eq!(format_mass(1_430), "1.43 kg");
        // 1,000,000 g = 1,000 kg: a full Cell of Water stays in kilograms.
        assert_eq!(format_mass(1_000_000), "1,000 kg");
        // 2,700,000 g = 2.7 t: a full Cell of Granite.
        assert_eq!(format_mass(2_700_000), "2.7 t");
    }

    #[test]
    fn mass_shows_three_significant_digits_at_most_without_trailing_zeros() {
        assert_eq!(format_mass(0), "0 g");
        assert_eq!(format_mass(999), "999 g");
        assert_eq!(format_mass(1_000), "1 kg");
        // 12.345 kg and 123.456 kg, to three significant digits.
        assert_eq!(format_mass(12_345), "12.3 kg");
        assert_eq!(format_mass(123_456), "123 kg");
        // 999.999 kg rounds up to 1,000 kg, which is still kilograms.
        assert_eq!(format_mass(999_999), "1,000 kg");
        // 1,000.001 kg is just over 1 t.
        assert_eq!(format_mass(1_000_001), "1 t");
        // The most a Cell can hold: 4,294,967,295 g ≈ 4,295 t.
        assert_eq!(format_mass(u32::MAX), "4,295 t");
    }

    #[test]
    fn temperature_is_shown_in_celsius_to_two_decimal_places() {
        // 293.15 K = 20 °C, and 0 K = −273.15 °C.
        assert_eq!(format_temperature(293.15), "20.00 °C");
        assert_eq!(format_temperature(0.0), "-273.15 °C");
        // 273.149 K is −0.001 °C, which rounds to 0.00 °C, without a minus sign.
        assert_eq!(format_temperature(273.149), "0.00 °C");
    }
}
