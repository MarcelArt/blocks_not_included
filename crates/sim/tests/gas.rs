mod common;

use common::*;
use sim::{Cell, CellPos, ElementId, ElementTable, Simulation};

#[test]
fn a_gas_cell_moves_some_mass_into_a_neighbouring_vacuum_cell_at_its_own_temperature() {
    let mut elements = ElementTable::new();
    let wall = elements.add(test_wall());
    let gas = elements.add(test_gas("TestGas", 1.0));
    let mut sim = Simulation::new(elements);
    let source = CellPos::new(0, 0, 0);
    let vacuum = CellPos::new(1, 0, 0);
    let sealed = SealedBox::build(&mut sim, wall, source, vacuum);
    sim.set(source, cell(gas, 1_000, 350.0));
    let invariants = Invariants::record(&sim, sealed.all());

    run_ticks(&mut sim, 1, &invariants);

    let arrived = sim.get(vacuum);
    assert_eq!(arrived.element, gas);
    assert!(arrived.mass_grams > 0);
    // One source at 350 K is the only Mass in the Cell, so it is exactly 350 K.
    assert_eq!(arrived.temperature_k, 350.0);
}

#[test]
fn gas_released_into_a_sealed_box_of_vacuum_spreads_until_every_inside_cell_holds_gas() {
    let mut elements = ElementTable::new();
    let wall = elements.add(test_wall());
    let gas = elements.add(test_gas("TestGas", 1.0));
    let mut sim = Simulation::new(elements);
    let sealed = SealedBox::build(&mut sim, wall, CellPos::new(0, 0, 0), CellPos::new(4, 4, 4));
    sim.set(CellPos::new(0, 0, 0), cell(gas, 1_000_000, 300.0));
    let invariants = Invariants::record(&sim, sealed.all());

    let mut ticks = 0;
    while !sealed
        .inside()
        .iter()
        .all(|&pos| sim.get(pos).element == gas)
    {
        assert!(
            ticks < 200,
            "after {ticks} Ticks some inside Cells still hold no gas"
        );
        run_ticks(&mut sim, 1, &invariants);
        ticks += 1;
    }
}

#[test]
fn two_touching_cells_of_one_gas_even_out_and_then_come_to_rest() {
    let mut elements = ElementTable::new();
    let wall = elements.add(test_wall());
    let gas = elements.add(test_gas("TestGas", 1.0));
    let mut sim = Simulation::new(elements);
    let full = CellPos::new(0, 0, 0);
    let empty = CellPos::new(1, 0, 0);
    let sealed = SealedBox::build(&mut sim, wall, full, empty);
    sim.set(full, cell(gas, 2_000, 300.0));
    sim.set(empty, cell(gas, 1_000, 300.0));
    let invariants = Invariants::record(&sim, sealed.all());

    run_ticks(&mut sim, 100, &invariants);

    // (2000 g + 1000 g) / 2 = 1500 g. Flows below 1 g round to zero (ADR 0004), so the pair may
    // stop a gram either side of it.
    for pos in [full, empty] {
        let grams = sim.get(pos).mass_grams;
        assert!(
            grams.abs_diff(1_500) <= 1,
            "{pos:?} holds {grams} g, not 1500 g"
        );
    }
    assert_at_rest(&mut sim, 100, &sealed.all(), &invariants);
}

#[test]
fn moved_mass_carries_its_heat_into_the_cell_it_joins() {
    let mut elements = ElementTable::new();
    let wall = elements.add(test_wall());
    let gas = elements.add(test_gas("TestGas", 1.0)); // Conducts no heat.
    let mut sim = Simulation::new(elements);
    let cool = CellPos::new(0, 0, 0);
    let hot = CellPos::new(1, 0, 0);
    let sealed = SealedBox::build(&mut sim, wall, cool, hot);
    sim.set(cool, cell(gas, 2_000, 300.0));
    sim.set(hot, cell(gas, 1_000, 400.0));
    let invariants = Invariants::record(&sim, sealed.all());

    run_ticks(&mut sim, 100, &invariants);

    // Only the hot Cell receives Mass, so the cool Cell stays exactly 300 K.
    assert_eq!(sim.get(cool).temperature_k, 300.0);
    // The hot Cell receives 500 g, all at 300 K: (1000 g × 400 K + 500 g × 300 K) / 1500 g
    // = 366.67 K. The pair may stop 1 g short (ADR 0004), and each gram shifts that by 0.044 K.
    let hot_k = sim.get(hot).temperature_k;
    assert!(
        (hot_k - 366.67).abs() < 0.05,
        "the hot Cell ended at {hot_k} K"
    );
}

#[test]
fn a_denser_gas_above_a_lighter_one_swaps_below_it_whatever_their_mass() {
    let mut elements = ElementTable::new();
    let wall = elements.add(test_wall());
    let heavy = elements.add(test_gas("Heavy", 2.0));
    let light = elements.add(test_gas("Light", 0.09));
    let mut sim = Simulation::new(elements);
    let below = CellPos::new(0, 0, 0);
    let above = CellPos::new(0, 1, 0);
    let sealed = SealedBox::build(&mut sim, wall, below, above);
    sim.set(above, cell(heavy, 10, 250.0));
    sim.set(below, cell(light, 5_000, 350.0));
    let invariants = Invariants::record(&sim, sealed.all());

    run_ticks(&mut sim, 1, &invariants);

    // The Cells swap entirely: Element, Mass and temperature.
    assert_eq!(sim.get(below), cell(heavy, 10, 250.0));
    assert_eq!(sim.get(above), cell(light, 5_000, 350.0));
}

#[test]
fn a_sealed_column_of_alternating_heavy_and_light_gas_ends_with_all_the_heavy_gas_below() {
    let mut elements = ElementTable::new();
    let wall = elements.add(test_wall());
    let heavy = elements.add(test_gas("Heavy", 2.0));
    let light = elements.add(test_gas("Light", 0.09));
    let mut sim = Simulation::new(elements);
    let sealed = SealedBox::build(&mut sim, wall, CellPos::new(0, 0, 0), CellPos::new(0, 7, 0));
    // Light at the bottom, then alternating upward: the worst case for sorting.
    for y in 0..8 {
        let element = if y % 2 == 0 { light } else { heavy };
        sim.set(CellPos::new(0, y, 0), cell(element, 1_000, 300.0));
    }
    let invariants = Invariants::record(&sim, sealed.all());

    // A column of 8 needs at most 8 rounds of swaps; the rest show that it stays sorted.
    run_ticks(&mut sim, 20, &invariants);

    let column: Vec<ElementId> = (0..8)
        .map(|y| sim.get(CellPos::new(0, y, 0)).element)
        .collect();
    assert_eq!(
        column,
        [[heavy; 4], [light; 4]].concat(),
        "column from the bottom up"
    );
}

#[test]
fn a_high_pressure_gas_displaces_a_different_gas_that_can_merge_into_its_own_neighbour() {
    let mut elements = ElementTable::new();
    let wall = elements.add(test_wall());
    let pusher = elements.add(test_gas("Pusher", 1.0));
    let pushed = elements.add(test_gas("Pushed", 1.0));
    let mut sim = Simulation::new(elements);
    let row = [0, 1, 2].map(|x| CellPos::new(x, 0, 0));
    let sealed = SealedBox::build(&mut sim, wall, row[0], row[2]);
    sim.set(row[0], cell(pusher, 5_000, 300.0));
    sim.set(row[1], cell(pushed, 100, 300.0));
    sim.set(row[2], cell(pushed, 100, 300.0));
    let invariants = Invariants::record(&sim, sealed.all());

    run_ticks(&mut sim, 1, &invariants);

    assert_eq!(
        sim.get(row[1]).element,
        pusher,
        "the pusher advanced into the vacated Cell"
    );
    // All of the pushed gas merged into its neighbour: 100 g + 100 g.
    assert_eq!(sim.get(row[2]), cell(pushed, 200, 300.0));
    run_ticks(&mut sim, 50, &invariants);
}

#[test]
fn gas_with_nowhere_to_go_is_never_displaced_and_nothing_moves() {
    let mut elements = ElementTable::new();
    let wall = elements.add(test_wall());
    let pusher = elements.add(test_gas("Pusher", 1.0));
    let boxed_in = elements.add(test_gas("BoxedIn", 1.0));
    let other = elements.add(test_gas("Other", 1.0));
    let mut sim = Simulation::new(elements);
    let row = [0, 1, 2].map(|x| CellPos::new(x, 0, 0));
    let sealed = SealedBox::build(&mut sim, wall, row[0], row[2]);
    // The middle gas touches only walls and two different gases, each holding less than the
    // Cell next to it, so every possible push is blocked.
    sim.set(row[0], cell(pusher, 5_000, 300.0));
    sim.set(row[1], cell(boxed_in, 100, 300.0));
    sim.set(row[2], cell(other, 50, 300.0));
    let invariants = Invariants::record(&sim, sealed.all());

    assert_at_rest(&mut sim, 100, &sealed.all(), &invariants);
}

#[test]
fn gas_released_at_the_centre_of_a_sealed_cube_spreads_identically_in_all_six_directions() {
    let mut elements = ElementTable::new();
    let wall = elements.add(test_wall());
    let gas = elements.add(test_gas("TestGas", 1.0));
    let mut sim = Simulation::new(elements);
    // Centred on the origin, so the cube straddles eight Chunks.
    let sealed = SealedBox::build(
        &mut sim,
        wall,
        CellPos::new(-3, -3, -3),
        CellPos::new(3, 3, 3),
    );
    sim.set(CellPos::new(0, 0, 0), cell(gas, 1_000_000, 300.0));
    let invariants = Invariants::record(&sim, sealed.all());

    for tick in 0..60 {
        run_ticks(&mut sim, 1, &invariants);
        for pos in sealed.inside() {
            // Mirroring any axis or swapping axes maps this Cell onto its image here.
            let mut image = [pos.x.abs(), pos.y.abs(), pos.z.abs()];
            image.sort();
            let image = CellPos::new(image[0], image[1], image[2]);
            let (grams, image_grams) = (sim.get(pos).mass_grams, sim.get(image).mass_grams);
            assert_eq!(
                grams, image_grams,
                "tick {tick}: {pos:?} differs from {image:?}"
            );
        }
    }
    assert!(
        sealed
            .inside()
            .iter()
            .all(|&pos| sim.get(pos).element == gas)
    );
}

#[test]
fn solid_walls_never_change_and_no_gas_appears_outside_the_box() {
    let mut elements = ElementTable::new();
    let wall = elements.add(test_wall());
    let heavy = elements.add(test_gas("Heavy", 2.0));
    let light = elements.add(test_gas("Light", 0.09));
    let mut sim = Simulation::new(elements);
    let (min, max) = (CellPos::new(0, 0, 0), CellPos::new(3, 3, 3));
    let sealed = SealedBox::build(&mut sim, wall, min, max);
    // High-pressure gases in opposite corners, the heavy one on top, so layering, Displacement
    // and spreading all press against the walls.
    sim.set(CellPos::new(0, 3, 0), cell(heavy, 200_000, 400.0));
    sim.set(CellPos::new(3, 0, 3), cell(light, 500_000, 250.0));
    let invariants = Invariants::record(&sim, sealed.all());
    let walls: Vec<Cell> = sealed.walls().iter().map(|&pos| sim.get(pos)).collect();
    // The shell of Cells just outside the walls.
    let outside = SealedBox {
        min: CellPos::new(min.x - 1, min.y - 1, min.z - 1),
        max: CellPos::new(max.x + 1, max.y + 1, max.z + 1),
    }
    .walls();

    for tick in 0..200 {
        run_ticks(&mut sim, 1, &invariants);
        let now: Vec<Cell> = sealed.walls().iter().map(|&pos| sim.get(pos)).collect();
        assert_eq!(now, walls, "tick {tick}: a wall changed");
        for &pos in &outside {
            assert_eq!(
                sim.get(pos),
                Cell::VACUUM,
                "tick {tick}: matter appeared at {pos:?}"
            );
        }
    }
}

// Cases the implementation revealed, beyond the ten tests in the plan.

#[test]
fn in_a_stack_of_three_gases_each_cell_swaps_at_most_once_per_tick() {
    let mut elements = ElementTable::new();
    let wall = elements.add(test_wall());
    let heavy = elements.add(test_gas("Heavy", 2.0));
    let middle = elements.add(test_gas("Middle", 1.0));
    let light = elements.add(test_gas("Light", 0.09));
    let mut sim = Simulation::new(elements);
    // Chunks are 32 Cells tall, so this column straddles a Chunk border.
    let column = [31, 32, 33].map(|y| CellPos::new(0, y, 0));
    let sealed = SealedBox::build(&mut sim, wall, column[0], column[2]);
    // Lightest at the bottom, so the middle Cell is in two pairs that both want to swap.
    for (pos, element) in column.into_iter().zip([light, middle, heavy]) {
        sim.set(pos, cell(element, 1_000, 300.0));
    }
    let invariants = Invariants::record(&sim, sealed.all());

    run_ticks(&mut sim, 1, &invariants);

    let height_of = |element| {
        column
            .iter()
            .position(|&pos| sim.get(pos).element == element)
    };
    for (element, start) in [(light, 0), (middle, 1), (heavy, 2)] {
        let now = height_of(element).expect("no gas leaves the column");
        assert!(
            now.abs_diff(start) <= 1,
            "{element:?} moved from {start} to {now} in one Tick"
        );
    }
    run_ticks(&mut sim, 10, &invariants);
    assert_eq!(
        column.map(|pos| sim.get(pos).element),
        [heavy, middle, light]
    );
}

#[test]
fn two_gases_reaching_one_vacuum_cell_in_the_same_tick_turn_it_into_only_one_of_them() {
    let mut elements = ElementTable::new();
    let wall = elements.add(test_wall());
    let first = elements.add(test_gas("First", 1.0));
    let second = elements.add(test_gas("Second", 1.0));
    let mut sim = Simulation::new(elements);
    // Two separate rows, each with Vacuum between two different gases: one uneven, one tied.
    let uneven = [0, 1, 2].map(|x| CellPos::new(x, 0, 0));
    let tied = [0, 1, 2].map(|x| CellPos::new(x, 0, 10));
    let uneven_box = SealedBox::build(&mut sim, wall, uneven[0], uneven[2]);
    let tied_box = SealedBox::build(&mut sim, wall, tied[0], tied[2]);
    sim.set(uneven[0], cell(first, 1_000, 300.0));
    sim.set(uneven[2], cell(second, 5_000, 300.0));
    sim.set(tied[0], cell(first, 1_000, 300.0));
    sim.set(tied[2], cell(second, 1_000, 300.0));
    let invariants = Invariants::record(&sim, [uneven_box.all(), tied_box.all()].concat());

    // The invariants check that Mass per Element is unchanged, so neither gas leaks into a Cell
    // labelled as the other.
    run_ticks(&mut sim, 1, &invariants);

    // The gas pushing more Mass wins the Cell; the other keeps all of its Mass.
    assert_eq!(sim.get(uneven[1]).element, second);
    assert_eq!(sim.get(uneven[0]), cell(first, 1_000, 300.0));
    assert!([first, second].contains(&sim.get(tied[1]).element));
}

#[test]
fn two_gases_sharing_a_sealed_space_come_to_rest() {
    let mut elements = ElementTable::new();
    let wall = elements.add(test_wall());
    let first = elements.add(test_gas("First", 1.0));
    let second = elements.add(test_gas("Second", 1.0));
    let mut sim = Simulation::new(elements);
    let row = [0, 1, 2, 3, 4, 5].map(|x| CellPos::new(x, 0, 0));
    let sealed = SealedBox::build(&mut sim, wall, row[0], row[5]);
    // 3300 g against 3000 g: no whole number of Cells gives the two gases equal pressure, so a
    // push whenever one side holds more would trade a Cell back and forth forever.
    for pos in &row[..3] {
        sim.set(*pos, cell(first, 1_100, 300.0));
    }
    for pos in &row[3..] {
        sim.set(*pos, cell(second, 1_000, 300.0));
    }
    let invariants = Invariants::record(&sim, sealed.all());

    run_ticks(&mut sim, 300, &invariants);

    assert_at_rest(&mut sim, 100, &sealed.all(), &invariants);
}

/// A sealed box straddling eight Chunks, scattered with three gases of varied Mass and
/// temperature and pockets of Vacuum, so every gas rule fires and competes for Cells.
fn mixing_box() -> (Simulation, SealedBox) {
    let mut elements = ElementTable::new();
    let wall = elements.add(test_wall());
    let gases = [
        elements.add(test_gas("Heavy", 2.0)),
        elements.add(test_gas("Middle", 1.0)),
        elements.add(test_gas("Light", 0.09)),
    ];
    let mut sim = Simulation::new(elements);
    let sealed = SealedBox::build(
        &mut sim,
        wall,
        CellPos::new(-3, -3, -3),
        CellPos::new(2, 2, 2),
    );
    for pos in sealed.inside() {
        let pattern = (pos.x * 7 + pos.y * 13 + pos.z * 5).rem_euclid(5) as usize;
        let Some(&gas) = gases.get(pattern) else {
            continue; // Patterns 3 and 4 stay Vacuum.
        };
        let grams = 100 + (pos.x * 31 + pos.y * 17 + pos.z * 11).rem_euclid(50) as u32 * 1_000;
        let kelvin = 250.0 + (pos.x * 3 + pos.y * 5 + pos.z * 7).rem_euclid(150) as f32;
        sim.set(pos, cell(gas, grams, kelvin));
    }
    (sim, sealed)
}

#[test]
fn mixing_gases_conserve_mass_per_element_and_play_out_the_same_every_time() {
    let (mut first, sealed) = mixing_box();
    let (mut second, _) = mixing_box();
    let invariants = Invariants::record(&first, sealed.all());

    for tick in 0..300 {
        run_ticks(&mut first, 1, &invariants);
        run_ticks(&mut second, 1, &invariants);
        for pos in sealed.all() {
            assert_eq!(
                first.get(pos),
                second.get(pos),
                "tick {tick}: runs differ at {pos:?}"
            );
        }
    }
}
