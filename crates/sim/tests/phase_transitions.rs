mod common;

use common::*;
use sim::{CellPos, ElementId, ElementTable, Simulation};

const SHIPPED_ELEMENTS: &str = include_str!("../../../assets/elements.ron");

/// The shipped Elements, plus a `TestWall` that conducts no heat, so a sealed box never warms or
/// cools what it holds.
fn shipped_elements() -> ElementTable {
    let mut elements = ElementTable::from_ron(SHIPPED_ELEMENTS).expect("the shipped file is valid");
    elements.add(test_wall());
    elements
}

fn ids<const N: usize>(elements: &ElementTable, names: [&str; N]) -> [ElementId; N] {
    names.map(|name| {
        elements
            .id_of(name)
            .unwrap_or_else(|| panic!("there is no Element called {name:?}"))
    })
}

#[test]
fn water_above_373_15_k_becomes_steam_with_the_same_mass_and_temperature() {
    let elements = shipped_elements();
    let [wall, water, steam] = ids(&elements, ["TestWall", "Water", "Steam"]);
    let mut sim = Simulation::new(elements);
    let pos = CellPos::new(0, 0, 0);
    let sealed = SealedBox::build(&mut sim, wall, pos, pos);
    sim.set(pos, cell(water, 1_000_000, 373.5));
    let invariants = Invariants::record(&sim, sealed.all()).allowing_phase_transitions();

    run_ticks(&mut sim, 1, &invariants);

    // Water boils above 373.15 K, in place, keeping its Mass and temperature exactly (ADR 0002).
    assert_eq!(sim.get(pos), cell(steam, 1_000_000, 373.5));
}

#[test]
fn steam_below_372_15_k_becomes_water() {
    let elements = shipped_elements();
    let [wall, water, steam] = ids(&elements, ["TestWall", "Water", "Steam"]);
    let mut sim = Simulation::new(elements);
    let pos = CellPos::new(0, 0, 0);
    let sealed = SealedBox::build(&mut sim, wall, pos, pos);
    sim.set(pos, cell(steam, 600, 372.0));
    let invariants = Invariants::record(&sim, sealed.all()).allowing_phase_transitions();

    run_ticks(&mut sim, 1, &invariants);

    // Steam condenses below 372.15 K, keeping its Mass and temperature exactly.
    assert_eq!(sim.get(pos), cell(water, 600, 372.0));
}

#[test]
fn inside_the_gap_at_372_5_k_water_stays_water_and_steam_stays_steam() {
    let elements = shipped_elements();
    let [wall, water, steam] = ids(&elements, ["TestWall", "Water", "Steam"]);
    let mut sim = Simulation::new(elements);
    // Each in its own sealed box, so neither warms or cools the other.
    let [water_pos, steam_pos] = [CellPos::new(0, 0, 0), CellPos::new(5, 0, 0)];
    let water_box = SealedBox::build(&mut sim, wall, water_pos, water_pos);
    let steam_box = SealedBox::build(&mut sim, wall, steam_pos, steam_pos);
    sim.set(water_pos, cell(water, 1_000_000, 372.5));
    sim.set(steam_pos, cell(steam, 600, 372.5));
    let invariants = Invariants::record(&sim, [water_box.all(), steam_box.all()].concat());

    // 372.5 K is below Water's boiling point (373.15 K) and above Steam's condensation point
    // (372.15 K), so neither transitions. The per-Element Mass check would catch it if either did.
    assert_at_rest(&mut sim, 20, &[water_pos, steam_pos], &invariants);
    assert_eq!(sim.get(water_pos), cell(water, 1_000_000, 372.5));
    assert_eq!(sim.get(steam_pos), cell(steam, 600, 372.5));
}

#[test]
fn an_element_with_no_threshold_never_transitions() {
    let elements = shipped_elements();
    let [wall, oxygen, steam] = ids(&elements, ["TestWall", "Oxygen", "Steam"]);
    let mut sim = Simulation::new(elements);
    let [oxygen_pos, steam_pos] = [CellPos::new(0, 0, 0), CellPos::new(5, 0, 0)];
    let oxygen_box = SealedBox::build(&mut sim, wall, oxygen_pos, oxygen_pos);
    let steam_box = SealedBox::build(&mut sim, wall, steam_pos, steam_pos);
    // Oxygen has no thresholds at all, however cold it gets.
    sim.set(oxygen_pos, cell(oxygen, 1_000, 10.0));
    // Steam has a low threshold but no high one, however hot it gets.
    sim.set(steam_pos, cell(steam, 600, 5_000.0));
    let invariants = Invariants::record(&sim, [oxygen_box.all(), steam_box.all()].concat());

    assert_at_rest(&mut sim, 20, &[oxygen_pos, steam_pos], &invariants);
    assert_eq!(sim.get(oxygen_pos), cell(oxygen, 1_000, 10.0));
    assert_eq!(sim.get(steam_pos), cell(steam, 600, 5_000.0));
}

#[test]
fn magma_below_1400_k_becomes_igneous_rock_and_stops_moving() {
    let elements = shipped_elements();
    let [wall, magma, igneous_rock] = ids(&elements, ["TestWall", "Magma", "Igneous Rock"]);
    let mut sim = Simulation::new(elements);
    // A sealed column of Vacuum one Cell wide, with a full Cell of Magma at the top.
    let column: Vec<CellPos> = (0..6).map(|y| CellPos::new(0, y, 0)).collect();
    let sealed = SealedBox::build(&mut sim, wall, column[0], column[5]);
    sim.set(column[5], cell(magma, 2_600_000, 1_300.0));
    let invariants = Invariants::record(&sim, sealed.all()).allowing_phase_transitions();

    run_ticks(&mut sim, 1, &invariants);

    // Phase Transitions run last, so the Magma first falls one Cell as a liquid, then freezes
    // there with the same Mass and temperature.
    assert_eq!(sim.get(column[5]).element, ElementId::VACUUM);
    assert_eq!(sim.get(column[4]), cell(igneous_rock, 2_600_000, 1_300.0));
    // A solid never moves, even with Vacuum beneath it.
    assert_at_rest(&mut sim, 20, &sealed.all(), &invariants);
}

#[test]
fn a_boiling_water_cell_becomes_steam_of_the_same_mass_which_then_spreads_into_the_vacuum() {
    let elements = shipped_elements();
    let [wall, water, steam] = ids(&elements, ["TestWall", "Water", "Steam"]);
    let mut sim = Simulation::new(elements);
    // A sealed 3 × 4 × 3 box of Vacuum whose floor layer is walled in except for one Cell in the
    // middle, which holds the Water. Liquids never flow upward, so the Water can't move, but
    // Steam can.
    let sealed = SealedBox::build(
        &mut sim,
        wall,
        CellPos::new(-1, 0, -1),
        CellPos::new(1, 3, 1),
    );
    let pocket = CellPos::new(0, 0, 0);
    for pos in sealed
        .inside()
        .into_iter()
        .filter(|pos| pos.y == 0 && *pos != pocket)
    {
        sim.set(pos, cell(wall, 1_000, 300.0));
    }
    sim.set(pocket, cell(water, 1_000_000, 400.0));
    let invariants = Invariants::record(&sim, sealed.all()).allowing_phase_transitions();

    run_ticks(&mut sim, 1, &invariants);

    assert_eq!(sim.get(pocket), cell(steam, 1_000_000, 400.0));
    let mut grams = sim.get(pocket).mass_grams;
    for tick in 0..10 {
        run_ticks(&mut sim, 1, &invariants);
        let now = sim.get(pocket);
        // At 400 K it stays above Steam's condensation point (372.15 K).
        assert_eq!(now.element, steam, "tick {tick}: the pocket condensed");
        assert!(
            now.mass_grams < grams,
            "tick {tick}: the pocket still holds {} g",
            now.mass_grams
        );
        grams = now.mass_grams;
    }
}

#[test]
fn granite_above_1500_k_becomes_magma_over_capacity_and_sheds_the_excess_into_a_vacuum_cell() {
    let elements = shipped_elements();
    let [wall, granite, magma] = ids(&elements, ["TestWall", "Granite", "Magma"]);
    let mut sim = Simulation::new(elements);
    // A sealed column two Cells tall. Liquids never flow upward except to shed excess, so all
    // that reaches the Vacuum Cell is what the Magma sheds.
    let [below, above] = [CellPos::new(0, 0, 0), CellPos::new(0, 1, 0)];
    let sealed = SealedBox::build(&mut sim, wall, below, above);
    // A full Cell of Granite: 2700 kg/m³ × 1 m³ = 2,700,000 g.
    sim.set(below, cell(granite, 2_700_000, 1_600.0));
    let invariants = Invariants::record(&sim, sealed.all()).allowing_phase_transitions();

    run_ticks(&mut sim, 1, &invariants);

    // Magma's capacity is 2600 kg/m³ × 1 m³ = 2,600,000 g, so the new Magma is 100,000 g over.
    assert_eq!(sim.get(below), cell(magma, 2_700_000, 1_600.0));
    assert_eq!(sim.get(above).element, ElementId::VACUUM);

    run_ticks(&mut sim, 1, &invariants);

    // It sheds the 100,000 g of excess into the Vacuum above it, which now holds Magma.
    assert_eq!(sim.get(below), cell(magma, 2_600_000, 1_600.0));
    assert_eq!(sim.get(above), cell(magma, 100_000, 1_600.0));
    assert_at_rest(&mut sim, 20, &sealed.all(), &invariants);
}

#[test]
fn total_mass_is_unchanged_across_every_phase_transition_in_the_shipped_elements() {
    let elements = shipped_elements();
    let [wall] = ids(&elements, ["TestWall"]);
    // For every threshold: a full Cell of its Element 50 K past it, and the target Element.
    let mut scenarios = Vec::new();
    for (element, properties) in elements.iter() {
        // Density × 1 m³.
        let full = (properties.density * 1_000.0).round() as u32;
        for (transition, past_k) in [(&properties.low, -50.0), (&properties.high, 50.0)] {
            if let Some(transition) = transition {
                let [into] = ids(&elements, [transition.into.as_str()]);
                let start = cell(element, full, transition.temperature_k + past_k);
                scenarios.push((start, into));
            }
        }
    }
    // Water's low and high, Steam's low, Magma's low, and the high of Ice, Granite and Igneous
    // Rock.
    assert_eq!(scenarios.len(), 7);
    let mut sim = Simulation::new(elements);
    // Each stands at the bottom of its own sealed column three Cells tall, so gas has room to
    // spread and liquid over capacity has room to shed.
    let mut columns = Vec::new();
    for (i, &(start, into)) in scenarios.iter().enumerate() {
        let column: Vec<CellPos> = (0..3).map(|y| CellPos::new(3 * i as i32, y, 0)).collect();
        let sealed = SealedBox::build(&mut sim, wall, column[0], column[2]);
        sim.set(column[0], start);
        columns.push((sealed, column, start, into));
    }
    let all = columns
        .iter()
        .flat_map(|(sealed, ..)| sealed.all())
        .collect();
    let invariants = Invariants::record(&sim, all).allowing_phase_transitions();

    run_ticks(&mut sim, 1, &invariants);

    for (_, column, start, into) in &columns {
        for &pos in column {
            let now = sim.get(pos);
            assert!(
                now.element == ElementId::VACUUM || now.element == *into,
                "{pos:?} holds {now:?}, but {start:?} should have become {into:?}"
            );
        }
    }
    // The new matter goes on flowing by the rules of its new Phase, and `run_ticks` checks after
    // every Tick that total Mass hasn't changed.
    run_ticks(&mut sim, 50, &invariants);
}

// Cases the implementation revealed, beyond the eight tests in the plan.

#[test]
fn a_cell_crosses_at_most_one_threshold_per_tick() {
    let elements = shipped_elements();
    let [wall, ice, water, steam] = ids(&elements, ["TestWall", "Ice", "Water", "Steam"]);
    let mut sim = Simulation::new(elements);
    let pos = CellPos::new(0, 0, 0);
    let sealed = SealedBox::build(&mut sim, wall, pos, pos);
    // Above Ice's melting point (273.15 K) and Water's boiling point (373.15 K).
    sim.set(pos, cell(ice, 917_000, 400.0));
    let invariants = Invariants::record(&sim, sealed.all()).allowing_phase_transitions();

    run_ticks(&mut sim, 1, &invariants);
    assert_eq!(sim.get(pos), cell(water, 917_000, 400.0));

    run_ticks(&mut sim, 1, &invariants);
    assert_eq!(sim.get(pos), cell(steam, 917_000, 400.0));
}

#[test]
fn a_cell_that_melts_while_boxed_in_by_solids_stays_over_capacity() {
    let elements = shipped_elements();
    let [wall, granite, magma] = ids(&elements, ["TestWall", "Granite", "Magma"]);
    let mut sim = Simulation::new(elements);
    let pos = CellPos::new(0, 0, 0);
    let sealed = SealedBox::build(&mut sim, wall, pos, pos);
    sim.set(pos, cell(granite, 2_700_000, 1_600.0));
    let invariants = Invariants::record(&sim, sealed.all()).allowing_phase_transitions();

    run_ticks(&mut sim, 1, &invariants);

    // 100,000 g over Magma's capacity of 2,600,000 g, with nowhere to shed it. Compression is not
    // modelled, so it stays that way.
    assert_eq!(sim.get(pos), cell(magma, 2_700_000, 1_600.0));
    assert_at_rest(&mut sim, 20, &sealed.all(), &invariants);
}
