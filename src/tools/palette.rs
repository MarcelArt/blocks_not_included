//! The palette: which god tool is in hand, what Place and Spawn fluid put down, and how the heat
//! brush heats. It is drawn with egui in the top-left corner.

use std::ops::RangeInclusive;

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};
use sim::{Cell, Element, ElementId, ElementTable, Phase};

use crate::input_policy::InputPolicy;
use crate::session::Play;
use crate::sim_bridge::Elements;

/// The temperature the palette starts at, in kelvin: 20 °C.
const ROOM_TEMPERATURE_K: f32 = 293.15;

/// How far inside a Phase Transition threshold the temperature moves to, in kelvin, when the
/// Element chosen would otherwise cross that threshold as soon as it is placed.
const THRESHOLD_MARGIN_K: f32 = 10.0;

/// How many times its Density × 1 m³ a gas Cell may be given. For a gas, Mass is pressure, so
/// this allows ten times the pressure of a Cell filled at the gas's Density.
const GAS_MASS_LIMIT: u32 = 10;

/// 0 °C in kelvin. An f32, as temperatures are, so a threshold at 273.15 K shows as 0 °C rather
/// than −0.00 °C.
const ZERO_CELSIUS_K: f32 = 273.15;

/// The temperature slider's range, in °C: from 0 K to well past Granite's melting point.
const TEMPERATURE_RANGE_C: RangeInclusive<f64> = -273.15..=3_000.0;

/// The lightest Mass the palette puts down, in kg: the smallest amount of matter, 1 g.
const LIGHTEST_KG: f64 = 0.001;

/// The heat brush's radius range, in metres.
const BRUSH_RADIUS_RANGE: RangeInclusive<f32> = 0.0..=5.0;

/// The heat brush's strength range, in kelvin per second. Below 0 it cools.
const BRUSH_STRENGTH_RANGE: RangeInclusive<f32> = -1_000.0..=1_000.0;

/// A god tool. Number keys 1–4 choose one, in this order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Tool {
    /// Left click turns the Cell in view into Vacuum.
    #[default]
    Dig,
    /// Right click puts down a Cell of any Element against the face in view.
    Place,
    /// Holding the left button shifts the temperature of every Cell near the Cell in view.
    HeatBrush,
    /// Right click puts down a Cell of gas or liquid against the face in view.
    SpawnFluid,
}

impl Tool {
    pub const ALL: [Tool; 4] = [Tool::Dig, Tool::Place, Tool::HeatBrush, Tool::SpawnFluid];

    /// The number key that chooses this tool.
    pub fn key(self) -> KeyCode {
        match self {
            Tool::Dig => KeyCode::Digit1,
            Tool::Place => KeyCode::Digit2,
            Tool::HeatBrush => KeyCode::Digit3,
            Tool::SpawnFluid => KeyCode::Digit4,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Tool::Dig => "Dig",
            Tool::Place => "Place",
            Tool::HeatBrush => "Heat brush",
            Tool::SpawnFluid => "Spawn fluid",
        }
    }

    /// Whether this tool can put down a Cell of an Element in `phase`. Dig and the heat brush put
    /// nothing down.
    pub fn puts_down(self, phase: Phase) -> bool {
        match self {
            Tool::Place => phase != Phase::Vacuum,
            Tool::SpawnFluid => matches!(phase, Phase::Gas | Phase::Liquid),
            Tool::Dig | Tool::HeatBrush => false,
        }
    }
}

/// The god tools' settings, as the palette shows them.
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct Palette {
    pub tool: Tool,
    /// The Cell Place puts down: any Element but Vacuum.
    pub place: Cell,
    /// The Cell Spawn fluid puts down: a gas or a liquid.
    pub spawn: Cell,
    /// How far the heat brush reaches from the centre of the Cell in view, in metres.
    pub brush_radius: f32,
    /// How fast the heat brush warms the Cells it reaches, in kelvin per second. Below 0 it cools
    /// them.
    pub brush_strength: f32,
}

impl Palette {
    /// Digging, with Place set to the first solid in `elements` and Spawn fluid to the first
    /// liquid, or gas if there is none, each as [`cell_of`] makes them at 20 °C. Without such an
    /// Element, a tool puts down Vacuum.
    pub fn new(elements: &ElementTable) -> Self {
        let first = |phase: Phase| {
            elements
                .iter()
                .find(|(_, element)| element.phase == phase)
                .map(|(id, _)| id)
        };
        let place = first(Phase::Solid).unwrap_or(ElementId::VACUUM);
        let spawn = first(Phase::Liquid)
            .or_else(|| first(Phase::Gas))
            .unwrap_or(ElementId::VACUUM);
        Self {
            tool: Tool::Dig,
            place: cell_of(place, ROOM_TEMPERATURE_K, elements),
            spawn: cell_of(spawn, ROOM_TEMPERATURE_K, elements),
            brush_radius: 1.5,
            brush_strength: 100.0,
        }
    }
}

/// A Cell of the Element `id`, as the palette offers it when that Element is chosen: its
/// Density × 1 m³, at `temperature_k`, the temperature set before.
///
/// If the Element would cross one of its Phase Transition thresholds at `temperature_k` as soon
/// as it is placed, it comes at 20 °C instead, or 10 K short of a threshold that 20 °C would
/// cross. It is never below 0 K.
pub fn cell_of(id: ElementId, temperature_k: f32, elements: &ElementTable) -> Cell {
    if id == ElementId::VACUUM {
        return Cell::VACUUM;
    }
    let element = elements.get(id);
    let low = element.low.as_ref().map(|low| low.temperature_k);
    let high = element.high.as_ref().map(|high| high.temperature_k);
    // As the simulation decides: a Cell crosses a threshold once strictly past it.
    let crosses = low.is_some_and(|low| temperature_k < low)
        || high.is_some_and(|high| temperature_k > high)
        || temperature_k.is_nan()
        || temperature_k < 0.0;
    let temperature_k = if crosses {
        let lowest = low.map_or(0.0, |low| low + THRESHOLD_MARGIN_K);
        let highest = high.map_or(f32::INFINITY, |high| high - THRESHOLD_MARGIN_K);
        if lowest <= highest {
            ROOM_TEMPERATURE_K.clamp(lowest, highest)
        } else {
            // Thresholds closer than twice the margin: halfway between them.
            (lowest + highest) / 2.0
        }
    } else {
        temperature_k
    };
    Cell {
        element: id,
        mass_grams: full_mass_grams(element),
        temperature_k,
    }
}

/// The Mass of a Cell full of `element`: its Density × 1 m³, and at least 1 g.
pub fn full_mass_grams(element: &Element) -> u32 {
    // kg/m³ × 1 m³ × 1000 g/kg, rounded as the simulation rounds a liquid's Capacity.
    ((f64::from(element.density) * 1_000.0).round() as u32).max(1)
}

/// The most Mass the palette puts down in one Cell of `element`. A liquid holds at most its
/// Capacity, and a solid is no denser than its Density; a gas may be pressed up to ten times its
/// Density.
pub fn mass_limit_grams(element: &Element) -> u32 {
    let full = full_mass_grams(element);
    match element.phase {
        Phase::Gas => full.saturating_mul(GAS_MASS_LIMIT),
        Phase::Solid | Phase::Liquid | Phase::Vacuum => full,
    }
}

/// Draws the palette in the top-left corner. During gameplay, with the cursor captured, clicks
/// pass through it, so the hidden cursor can't press its buttons and egui never wants the
/// pointer.
pub(super) fn draw_palette(
    mut contexts: EguiContexts,
    mut palette: ResMut<Palette>,
    elements: Res<Elements>,
    policy: Res<InputPolicy>,
    play: Res<State<Play>>,
) -> Result {
    let captured = policy.gameplay;
    // Only mark the palette changed when a widget changes it.
    let mut edited = palette.clone();
    egui::Window::new("God tools")
        .anchor(egui::Align2::LEFT_TOP, [8.0, 8.0])
        .resizable(false)
        .collapsible(false)
        .interactable(!captured)
        .show(contexts.ctx_mut()?, |ui| {
            ui.horizontal(|ui| {
                for (number, tool) in (1..).zip(Tool::ALL) {
                    ui.selectable_value(
                        &mut edited.tool,
                        tool,
                        format!("{number} {}", tool.name()),
                    );
                }
            });
            ui.separator();
            match edited.tool {
                Tool::Dig => {
                    ui.label("Left click digs out the Cell in view.");
                }
                Tool::Place => {
                    cell_picker(ui, &mut edited.place, Tool::Place, &elements);
                    ui.label("Right click places it against the face in view.");
                }
                Tool::HeatBrush => {
                    ui.add(
                        egui::Slider::new(&mut edited.brush_radius, BRUSH_RADIUS_RANGE)
                            .step_by(0.5)
                            .suffix(" m")
                            .text("Radius"),
                    );
                    ui.add(
                        egui::Slider::new(&mut edited.brush_strength, BRUSH_STRENGTH_RANGE)
                            .suffix(" K/s")
                            .text("Strength"),
                    );
                    ui.label(
                        "Hold the left button to heat the Cells around the one in view. Below \
                         0 K/s it cools them.",
                    );
                }
                Tool::SpawnFluid => {
                    cell_picker(ui, &mut edited.spawn, Tool::SpawnFluid, &elements);
                    ui.label("Right click spawns it against the face in view.");
                }
            }
            ui.separator();
            ui.weak(if captured {
                "Hold Alt to use this panel."
            } else if *play.get() == Play::Running {
                "Release Alt to aim again."
            } else {
                "Esc resumes the World."
            });
        });
    palette.set_if_neq(edited);
    Ok(())
}

/// The Element, Mass and temperature of the Cell `tool` puts down.
fn cell_picker(ui: &mut egui::Ui, cell: &mut Cell, tool: Tool, elements: &ElementTable) {
    egui::ComboBox::from_label("Element")
        .selected_text(elements.get(cell.element).name.as_str())
        .show_ui(ui, |ui| {
            let offered = elements
                .iter()
                .filter(|(_, element)| tool.puts_down(element.phase));
            for (id, element) in offered {
                let chosen = id == cell.element;
                if ui.selectable_label(chosen, element.name.as_str()).clicked() && !chosen {
                    *cell = cell_of(id, cell.temperature_k, elements);
                }
            }
        });

    let element = elements.get(cell.element);
    let heaviest_kg = f64::from(mass_limit_grams(element)) / 1_000.0;
    ui.add(
        egui::Slider::from_get_set(LIGHTEST_KG..=heaviest_kg, |kg| {
            if let Some(kg) = kg {
                cell.mass_grams = (kg * 1_000.0).round().max(1.0) as u32;
            }
            f64::from(cell.mass_grams) / 1_000.0
        })
        .logarithmic(true)
        .max_decimals(3)
        .suffix(" kg")
        .text("Mass"),
    );
    ui.add(
        egui::Slider::from_get_set(TEMPERATURE_RANGE_C, |celsius| {
            if let Some(celsius) = celsius {
                cell.temperature_k = celsius as f32 + ZERO_CELSIUS_K;
            }
            f64::from(cell.temperature_k - ZERO_CELSIUS_K)
        })
        .max_decimals(2)
        .suffix(" °C")
        .text("Temperature"),
    );
    for (transition, past) in [(&element.low, "below"), (&element.high, "above")] {
        if let Some(transition) = transition {
            let celsius = transition.temperature_k - ZERO_CELSIUS_K;
            ui.weak(format!(
                "Becomes {} {past} {celsius:.2} °C.",
                transition.into
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Water, Ice, Steam and Oxygen, with the thresholds of `assets/elements.ron`.
    fn elements() -> ElementTable {
        let mut elements = ElementTable::new();
        elements.add(Element::new("Oxygen").phase(Phase::Gas).density(1.43));
        elements.add(
            Element::new("Water")
                .phase(Phase::Liquid)
                .density(1_000.0)
                .low(272.15, "Ice")
                .high(373.15, "Steam"),
        );
        elements.add(
            Element::new("Ice")
                .phase(Phase::Solid)
                .density(917.0)
                .high(273.15, "Water"),
        );
        elements.add(
            Element::new("Steam")
                .phase(Phase::Gas)
                .density(0.6)
                .low(372.15, "Water"),
        );
        elements
    }

    fn id(elements: &ElementTable, name: &str) -> ElementId {
        elements.id_of(name).expect("the test table has it")
    }

    #[test]
    fn a_chosen_element_comes_as_a_full_cell_at_the_temperature_set() {
        let elements = elements();
        let water = id(&elements, "Water");
        let cell = cell_of(water, 293.15, &elements);
        // Density 1000 kg/m³ × 1 m³ = 1000 kg = 1,000,000 g.
        let expected = Cell {
            element: water,
            mass_grams: 1_000_000,
            temperature_k: 293.15,
        };
        assert_eq!(cell, expected);

        // Oxygen: 1.43 kg/m³ × 1 m³ = 1430 g.
        let oxygen = cell_of(id(&elements, "Oxygen"), 293.15, &elements);
        assert_eq!(oxygen.mass_grams, 1_430);
    }

    #[test]
    fn a_chosen_element_keeps_the_temperature_set_while_it_crosses_no_threshold() {
        let elements = elements();
        let temperature_of = |name, kelvin| cell_of(id(&elements, name), kelvin, &elements);
        // Water stays Water from 272.15 K to 373.15 K.
        assert_eq!(temperature_of("Water", 350.0).temperature_k, 350.0);
        // Ice stays Ice up to 273.15 K.
        assert_eq!(temperature_of("Ice", 200.0).temperature_k, 200.0);
        // Oxygen has no thresholds.
        assert_eq!(temperature_of("Oxygen", 1_000.0).temperature_k, 1_000.0);
    }

    #[test]
    fn a_chosen_element_past_a_threshold_comes_at_20_c_or_10_k_inside_it() {
        let elements = elements();
        let temperature_of = |name, kelvin| cell_of(id(&elements, name), kelvin, &elements);
        // Ice at −10 °C would freeze as Water, which comes at 20 °C instead.
        assert_eq!(temperature_of("Water", 263.15).temperature_k, 293.15);
        // Water at 20 °C would melt as Ice, and so would 20 °C, so Ice comes 10 K below its
        // threshold: 273.15 − 10 = 263.15 K.
        assert_eq!(temperature_of("Ice", 293.15).temperature_k, 263.15);
        // Steam condenses below 372.15 K, so it comes at 372.15 + 10 = 382.15 K.
        assert_eq!(temperature_of("Steam", 293.15).temperature_k, 382.15);
    }

    #[test]
    fn a_new_palette_places_the_first_solid_and_spawns_the_first_liquid() {
        let elements = elements();
        let palette = Palette::new(&elements);
        assert_eq!(palette.tool, Tool::Dig);
        assert_eq!(palette.place.element, id(&elements, "Ice"));
        assert_eq!(palette.spawn.element, id(&elements, "Water"));
    }

    #[test]
    fn a_liquid_is_limited_to_its_capacity_and_a_gas_to_ten_times_its_density() {
        let elements = elements();
        let get = |name| elements.get(id(&elements, name));
        // Water's Capacity: 1000 kg/m³ × 1 m³ = 1,000,000 g.
        assert_eq!(mass_limit_grams(get("Water")), 1_000_000);
        // Oxygen: 10 × 1.43 kg/m³ × 1 m³ = 14.3 kg = 14,300 g.
        assert_eq!(mass_limit_grams(get("Oxygen")), 14_300);
    }
}
