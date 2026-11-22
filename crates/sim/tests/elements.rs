use sim::{ElementError, ElementTable, Phase};

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
    assert!(message.contains(offender), "{message:?} doesn't name {offender}");
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

    assert!(matches!(error, ElementError::UnknownTarget { .. }), "{error:?}");
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
    let text = file(&[element("Water", "Liquid", ""), element("Water", "Solid", "")]);

    let error = rejection_naming("Water", &text);

    assert!(matches!(error, ElementError::DuplicateName { .. }), "{error:?}");
}

#[test]
fn a_missing_hysteresis_gap_is_rejected() {
    // Water at 373.5 K would boil into Steam, which at 373.5 K (< 374 K) condenses straight back.
    let text = file(&[
        element("Water", "Liquid", r#"high: Some((temperature_k: 373.15, into: "Steam"))"#),
        element("Steam", "Gas", r#"low: Some((temperature_k: 374.0, into: "Water"))"#),
    ]);

    let error = rejection_naming("Water", &text);

    assert!(matches!(error, ElementError::NoHysteresisGap { .. }), "{error:?}");
}

#[test]
fn a_wrong_direction_transition_is_rejected() {
    // Cooling a liquid must condense it, but this turns Water into a gas.
    let text = file(&[
        element("Water", "Liquid", r#"low: Some((temperature_k: 272.15, into: "Steam"))"#),
        element("Steam", "Gas", ""),
    ]);

    let error = rejection_naming("Water", &text);

    assert!(matches!(error, ElementError::WrongDirection { .. }), "{error:?}");
}

#[test]
fn an_element_with_zero_density_or_zero_specific_heat_is_rejected() {
    let zero_density = r#"(elements: [(name: "Ghost", phase: Gas, color: (1.0, 1.0, 1.0),
        density: 0.0, specific_heat: 1.0, conductivity: 1.0)])"#;
    let zero_specific_heat = r#"(elements: [(name: "Ghost", phase: Gas, color: (1.0, 1.0, 1.0),
        density: 1.0, specific_heat: 0.0, conductivity: 1.0)])"#;

    for text in [zero_density, zero_specific_heat] {
        let error = rejection_naming("Ghost", text);

        assert!(matches!(error, ElementError::InvalidProperty { .. }), "{error:?}");
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

    assert!(matches!(error, ElementError::LowNotBelowHigh { .. }), "{error:?}");
}

#[test]
fn a_listed_element_in_the_vacuum_phase_is_rejected() {
    // The built-in Vacuum is the only Element in the vacuum Phase.
    let text = file(&[element("Void", "Vacuum", "")]);

    let error = rejection_naming("Void", &text);

    assert!(matches!(error, ElementError::VacuumPhase { .. }), "{error:?}");
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

    assert!(matches!(error, ElementError::WrongDirection { .. }), "{error:?}");
}
