mod common;

use common::{Invariants, cell, run_ticks};
use sim::{CellPos, Element, ElementError, ElementTable, Phase, Simulation};

const SHIPPED_ELEMENTS: &str = include_str!("../../../assets/elements.ron");

const STARTER_NAMES: [&str; 10] = [
    "Oxygen",
    "Carbon Dioxide",
    "Hydrogen",
    "Steam",
    "Water",
    "Magma",
    "Ice",
    "Granite",
    "Dirt",
    "Igneous Rock",
];

/// One valid Element entry with the given Phase, plus `extra` fields such as Transitions.
fn element(name: &str, phase: &str, extra: &str) -> String {
    format!(
        "(name: {name:?}, phase: {phase}, color: (1.0, 1.0, 1.0), density: 1000.0, \
         specific_heat: 1.0, conductivity: 1.0, {extra})"
    )
}

/// An Elements file listing `entries`.
fn file(entries: &[String]) -> String {
    format!("(elements: [{}])", entries.join(", "))
}

/// Loads `text`, expecting a rejection whose message names `offender`.
fn rejection_naming(offender: &str, text: &str) -> ElementError {
    let error = ElementTable::from_ron(text).expect_err("the file should be rejected");
    let message = error.to_string();
    assert!(
        message.contains(offender),
        "{message:?} doesn't name {offender}"
    );
    error
}

#[test]
fn the_shipped_elements_file_loads_with_all_ten_starter_elements() {
    let elements = ElementTable::from_ron(SHIPPED_ELEMENTS).expect("the shipped file is valid");

    for name in STARTER_NAMES {
        assert!(elements.id_of(name).is_some(), "{name} is missing");
    }
    // The ten starter Elements plus the built-in Vacuum.
    assert_eq!(elements.iter().count(), 11);
}

#[test]
fn water_is_a_liquid_with_a_density_of_1000() {
    let elements = ElementTable::from_ron(SHIPPED_ELEMENTS).expect("the shipped file is valid");

    let water = elements.get(elements.id_of("Water").expect("Water is listed"));

    assert_eq!(water.density, 1000.0);
    assert_eq!(water.phase, Phase::Liquid);
}

#[test]
fn a_transition_into_an_unknown_element_is_rejected() {
    // Steam is not listed.
    let text = file(&[element(
        "Water",
        "Liquid",
        r#"high: Some((temperature_k: 373.15, into: "Steam"))"#,
    )]);

    let error = rejection_naming("Water", &text);

    assert!(
        matches!(error, ElementError::UnknownTarget { .. }),
        "{error:?}"
    );
}

#[test]
fn a_file_that_lists_vacuum_is_rejected() {
    // Vacuum is built in, so the file must not list it, even as a gas.
    let text = file(&[element("Vacuum", "Gas", "")]);

    let error = rejection_naming("Vacuum", &text);

    assert!(matches!(error, ElementError::ReservedName), "{error:?}");
}

#[test]
fn duplicate_names_are_rejected() {
    let text = file(&[
        element("Water", "Liquid", ""),
        element("Water", "Solid", ""),
    ]);

    let error = rejection_naming("Water", &text);

    assert!(
        matches!(error, ElementError::DuplicateName { .. }),
        "{error:?}"
    );
}

#[test]
fn a_missing_hysteresis_gap_is_rejected() {
    // Water at 373.5 K would boil into Steam, which at 373.5 K (< 374 K) condenses straight back.
    let text = file(&[
        element(
            "Water",
            "Liquid",
            r#"high: Some((temperature_k: 373.15, into: "Steam"))"#,
        ),
        element(
            "Steam",
            "Gas",
            r#"low: Some((temperature_k: 374.0, into: "Water"))"#,
        ),
    ]);

    let error = rejection_naming("Water", &text);

    assert!(
        matches!(error, ElementError::NoHysteresisGap { .. }),
        "{error:?}"
    );
}

#[test]
fn a_wrong_direction_transition_is_rejected() {
    // Cooling a liquid must condense it, but this turns Water into a gas.
    let text = file(&[
        element(
            "Water",
            "Liquid",
            r#"low: Some((temperature_k: 272.15, into: "Steam"))"#,
        ),
        element("Steam", "Gas", ""),
    ]);

    let error = rejection_naming("Water", &text);

    assert!(
        matches!(error, ElementError::WrongDirection { .. }),
        "{error:?}"
    );
}

#[test]
fn an_element_with_zero_density_or_zero_specific_heat_is_rejected() {
    let zero_density = r#"(elements: [(name: "Ghost", phase: Gas, color: (1.0, 1.0, 1.0),
        density: 0.0, specific_heat: 1.0, conductivity: 1.0)])"#;
    let zero_specific_heat = r#"(elements: [(name: "Ghost", phase: Gas, color: (1.0, 1.0, 1.0),
        density: 1.0, specific_heat: 0.0, conductivity: 1.0)])"#;

    for text in [zero_density, zero_specific_heat] {
        let error = rejection_naming("Ghost", text);

        assert!(
            matches!(error, ElementError::InvalidProperty { .. }),
            "{error:?}"
        );
    }
}

#[test]
fn an_empty_name_is_rejected() {
    let text = file(&[element("Water", "Liquid", ""), element("", "Solid", "")]);

    // The nameless Element is the second one listed.
    let error = rejection_naming("#2", &text);

    assert!(matches!(error, ElementError::EmptyName { .. }), "{error:?}");
}

#[test]
fn a_low_threshold_that_is_not_below_the_high_threshold_is_rejected() {
    // At 375 K this Water would be both above its boiling point and below its freezing point.
    let text = file(&[
        element(
            "Water",
            "Liquid",
            r#"low: Some((temperature_k: 380.0, into: "Ice")),
               high: Some((temperature_k: 373.15, into: "Steam"))"#,
        ),
        element("Ice", "Solid", ""),
        element("Steam", "Gas", ""),
    ]);

    let error = rejection_naming("Water", &text);

    assert!(
        matches!(error, ElementError::LowNotBelowHigh { .. }),
        "{error:?}"
    );
}

#[test]
fn a_listed_element_in_the_vacuum_phase_is_rejected() {
    // The built-in Vacuum is the only Element in the vacuum Phase.
    let text = file(&[element("Void", "Vacuum", "")]);

    let error = rejection_naming("Void", &text);

    assert!(
        matches!(error, ElementError::VacuumPhase { .. }),
        "{error:?}"
    );
}

#[test]
fn a_transition_into_vacuum_is_rejected() {
    // A Transition keeps the Cell's Mass, but a Vacuum Cell must hold none.
    let text = file(&[element(
        "Hydrogen",
        "Gas",
        r#"high: Some((temperature_k: 5000.0, into: "Vacuum"))"#,
    )]);

    let error = rejection_naming("Hydrogen", &text);

    assert!(
        matches!(error, ElementError::WrongDirection { .. }),
        "{error:?}"
    );
}

#[test]
fn a_tint_outside_0_to_1_is_rejected() {
    for tint in ["-0.1", "1.5", "NaN"] {
        let text = file(&[element("Fog", "Gas", &format!("tint: {tint}"))]);

        let error = rejection_naming("Fog", &text);

        assert!(
            matches!(
                error,
                ElementError::InvalidProperty {
                    property: "tint",
                    ..
                }
            ),
            "tint {tint}: {error:?}"
        );
    }

    // The builder sets the same field, and both ends of the range are allowed.
    let fog = |tint| {
        let mut elements = ElementTable::new();
        elements.add(
            Element::new("Fog")
                .phase(Phase::Gas)
                .density(1.0)
                .specific_heat(1.0)
                .tint(tint),
        );
        elements.validate()
    };
    assert_eq!(fog(0.0), Ok(()));
    assert_eq!(fog(1.0), Ok(()));
    assert!(matches!(
        fog(1.01),
        Err(ElementError::InvalidProperty {
            property: "tint",
            ..
        })
    ));
}

#[test]
fn steam_is_the_only_tinted_element_in_the_shipped_file() {
    let elements = ElementTable::from_ron(SHIPPED_ELEMENTS).expect("the shipped file is valid");

    for (_, element) in elements.iter() {
        let expected = if element.name == "Steam" { 0.15 } else { 0.0 };
        assert_eq!(element.tint, expected, "{}", element.name);
    }
}

/// A solid that conducts heat at `conductivity`, with 1 J/(g·K) so a Cell's heat capacity is its
/// Mass in grams.
fn rock(name: &str, conductivity: f32) -> Element {
    Element::new(name)
        .density(2_700.0)
        .specific_heat(1.0)
        .conductivity(conductivity)
}

/// Rock, Sand and Clay, in that order. Rock conducts at `rock_conductivity`, and the other two
/// conduct no heat.
fn three_rocks(rock_conductivity: f32) -> ElementTable {
    let mut elements = ElementTable::new();
    elements.add(rock("Rock", rock_conductivity));
    elements.add(rock("Sand", 0.0));
    elements.add(rock("Clay", 0.0));
    elements
}

/// A hot and a cold Rock Cell side by side, with Rock conducting no heat, after one Tick: nothing
/// has changed, so their Chunk is Settled.
struct SettledPair {
    sim: Simulation,
    hot: CellPos,
    cold: CellPos,
    invariants: Invariants,
}

impl SettledPair {
    fn new() -> Self {
        let elements = three_rocks(0.0);
        let rock = elements.id_of("Rock").expect("Rock is listed");
        let mut sim = Simulation::new(elements);
        let (hot, cold) = (CellPos::new(0, 0, 0), CellPos::new(1, 0, 0));
        sim.set(hot, cell(rock, 1_000, 400.0));
        sim.set(cold, cell(rock, 1_000, 300.0));
        let invariants = Invariants::record(&sim, vec![hot, cold]);

        run_ticks(&mut sim, 1, &invariants);

        assert_eq!(
            sim.stats().settled_chunks,
            1,
            "the pair's Chunk should be Settled"
        );
        Self {
            sim,
            hot,
            cold,
            invariants,
        }
    }

    fn temperatures(&self) -> [f32; 2] {
        [
            self.sim.get(self.hot).temperature_k,
            self.sim.get(self.cold).temperature_k,
        ]
    }
}

#[test]
fn replaced_elements_with_changed_properties_take_effect_on_the_next_tick() {
    let mut pair = SettledPair::new();

    let replaced = pair.sim.replace_elements(three_rocks(100.0));
    run_ticks(&mut pair.sim, 1, &pair.invariants);

    assert_eq!(replaced, Ok(()));
    // Rock now conducts, and replacing the table woke the Settled Chunk, so heat has flowed.
    let [hot, cold] = pair.temperatures();
    assert!(hot < 400.0 && cold > 300.0, "{hot} K and {cold} K");
}

#[test]
fn an_element_appended_at_the_end_is_accepted_and_cells_can_hold_it() {
    let mut pair = SettledPair::new();
    let mut elements = three_rocks(0.0);
    let silt = elements.add(rock("Silt", 100.0));

    let replaced = pair.sim.replace_elements(elements);
    let (warm, cool) = (CellPos::new(0, 2, 0), CellPos::new(1, 2, 0));
    pair.sim.set(warm, cell(silt, 1_000, 350.0));
    pair.sim.set(cool, cell(silt, 1_000, 250.0));
    let invariants = Invariants::record(&pair.sim, vec![pair.hot, pair.cold, warm, cool]);
    run_ticks(&mut pair.sim, 1, &invariants);

    assert_eq!(replaced, Ok(()));
    let [warm, cool] = [warm, cool].map(|pos| pair.sim.get(pos).temperature_k);
    assert!(
        warm < 350.0 && cool > 250.0,
        "Silt conducts: {warm} K and {cool} K"
    );
    // Rock still conducts no heat.
    assert_eq!(pair.temperatures(), [400.0, 300.0]);
}

#[test]
fn a_reordered_renamed_or_removed_element_is_rejected_and_the_old_table_stays() {
    // Each candidate also makes Rock conduct, which would show if it were taken up.
    let table = |names: &[&str]| {
        let mut elements = ElementTable::new();
        for &name in names {
            elements.add(rock(name, if name == "Rock" { 100.0 } else { 0.0 }));
        }
        elements
    };
    let missing_sand = || ElementError::MissingElement {
        element: "Sand".into(),
    };
    let candidates = [
        (
            "reordered",
            table(&["Sand", "Rock", "Clay"]),
            // Rock was #1 and is now #2.
            ElementError::ElementMoved {
                element: "Rock".into(),
                from: 1,
                to: 2,
            },
            "Rock",
        ),
        (
            "renamed",
            table(&["Rock", "Silt", "Clay"]),
            missing_sand(),
            "Sand",
        ),
        ("removed", table(&["Rock", "Clay"]), missing_sand(), "Sand"),
    ];

    for (change, elements, expected, offender) in candidates {
        let mut pair = SettledPair::new();

        let replaced = pair.sim.replace_elements(elements);
        run_ticks(&mut pair.sim, 1, &pair.invariants);

        let error = replaced.expect_err(change);
        assert_eq!(error, expected, "{change}");
        let message = error.to_string();
        assert!(
            message.contains(offender),
            "{change}: {message:?} doesn't name {offender}"
        );
        // Rock still conducts no heat.
        assert_eq!(pair.temperatures(), [400.0, 300.0], "{change}");
    }
}
