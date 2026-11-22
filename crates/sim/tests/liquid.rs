mod common;

use common::*;
use sim::{CellPos, ElementId, ElementTable, Simulation};

/// A full Cell of a Density-1000 liquid holds 1000 kg/m³ × 1 m³ = 1,000,000 g.
const FULL: u32 = 1_000_000;

/// The spreading threshold for a Density-1000 liquid: 1% of 1,000,000 g.
const SPREADING_THRESHOLD: u32 = 10_000;

/// A sealed column one Cell wide, `height` Cells tall, standing on y = 0.
fn column(height: i32) -> Vec<CellPos> {
    (0..height).map(|y| CellPos::new(0, y, 0)).collect()
}

#[test]
fn a_liquid_in_a_sealed_column_of_vacuum_ends_up_at_the_bottom() {
    let mut elements = ElementTable::new();
    let wall = elements.add(test_wall());
    let water = elements.add(test_liquid("TestWater", 1_000.0));
    let mut sim = Simulation::new(elements);
    let column = column(8);
    let sealed = SealedBox::build(&mut sim, wall, column[0], column[7]);
    sim.set(column[7], cell(water, 500_000, 300.0));
    let invariants = Invariants::record(&sim, sealed.all()).with_capacity(water, FULL);

    // It falls at most one Cell per Tick, so 7 Ticks reach the bottom; the rest show it stays.
    run_ticks(&mut sim, 20, &invariants);

    assert_eq!(sim.get(column[0]), cell(water, 500_000, 300.0));
    for &pos in &column[1..] {
        assert_eq!(
            sim.get(pos).element,
            ElementId::VACUUM,
            "{pos:?} still holds liquid"
        );
    }
}

#[test]
fn a_liquid_cell_above_a_gas_cell_swaps_below_it_in_one_tick() {
    let mut elements = ElementTable::new();
    let wall = elements.add(test_wall());
    let water = elements.add(test_liquid("TestWater", 1_000.0));
    let gas = elements.add(test_gas("TestGas", 1.0));
    let mut sim = Simulation::new(elements);
    let [below, above] = [CellPos::new(0, 0, 0), CellPos::new(0, 1, 0)];
    let sealed = SealedBox::build(&mut sim, wall, below, above);
    sim.set(above, cell(water, 400_000, 280.0));
    sim.set(below, cell(gas, 5_000, 350.0));
    let invariants = Invariants::record(&sim, sealed.all()).with_capacity(water, FULL);

    run_ticks(&mut sim, 1, &invariants);

    // The Cells swap entirely: Element, Mass and temperature.
    assert_eq!(sim.get(below), cell(water, 400_000, 280.0));
    assert_eq!(sim.get(above), cell(gas, 5_000, 350.0));
}

#[test]
fn three_full_cells_of_water_released_at_the_top_of_a_column_fill_exactly_the_bottom_three() {
    let mut elements = ElementTable::new();
    let wall = elements.add(test_wall());
    let water = elements.add(test_liquid("TestWater", 1_000.0));
    let mut sim = Simulation::new(elements);
    let column = column(10);
    let sealed = SealedBox::build(&mut sim, wall, column[0], column[9]);
    for &pos in &column[7..] {
        sim.set(pos, cell(water, FULL, 300.0));
    }
    let invariants = Invariants::record(&sim, sealed.all()).with_capacity(water, FULL);

    run_ticks(&mut sim, 30, &invariants);

    // Liquids do not compress: 3 × 1,000,000 g fills exactly three Cells.
    for &pos in &column[..3] {
        assert_eq!(sim.get(pos), cell(water, FULL, 300.0));
    }
    for &pos in &column[3..] {
        assert_eq!(
            sim.get(pos).element,
            ElementId::VACUUM,
            "{pos:?} still holds liquid"
        );
    }
    assert_at_rest(&mut sim, 20, &sealed.all(), &invariants);
}

#[test]
fn a_denser_liquid_above_a_lighter_one_ends_up_below_it() {
    let mut elements = ElementTable::new();
    let wall = elements.add(test_wall());
    let heavy = elements.add(test_liquid("Heavy", 2_000.0));
    let light = elements.add(test_liquid("Light", 1_000.0));
    let mut sim = Simulation::new(elements);
    let [below, above] = [CellPos::new(0, 0, 0), CellPos::new(0, 1, 0)];
    let sealed = SealedBox::build(&mut sim, wall, below, above);
    // A partly full heavy Cell above a full light one: Density decides, not Mass.
    sim.set(above, cell(heavy, 300_000, 320.0));
    sim.set(below, cell(light, FULL, 290.0));
    let invariants = Invariants::record(&sim, sealed.all())
        .with_capacity(heavy, 2 * FULL)
        .with_capacity(light, FULL);

    run_ticks(&mut sim, 1, &invariants);

    // The Cells swap entirely: Element, Mass and temperature.
    assert_eq!(sim.get(below), cell(heavy, 300_000, 320.0));
    assert_eq!(sim.get(above), cell(light, FULL, 290.0));
    assert_at_rest(&mut sim, 20, &sealed.all(), &invariants);
}

#[test]
fn liquid_in_a_flat_bottomed_sealed_box_settles_into_a_level_layer() {
    let mut elements = ElementTable::new();
    let wall = elements.add(test_wall());
    let water = elements.add(test_liquid("TestWater", 1_000.0));
    let mut sim = Simulation::new(elements);
    let sealed = SealedBox::build(&mut sim, wall, CellPos::new(0, 0, 0), CellPos::new(4, 3, 4));
    // Two full Cells stacked in a top corner: 2,000,000 g over a 5 × 5 floor is 80,000 g a Cell.
    sim.set(CellPos::new(0, 3, 0), cell(water, FULL, 300.0));
    sim.set(CellPos::new(0, 2, 0), cell(water, FULL, 300.0));
    let invariants = Invariants::record(&sim, sealed.all()).with_capacity(water, FULL);

    // The last gram-sized flows stop after about 200 Ticks; this leaves a wide margin.
    run_ticks(&mut sim, 400, &invariants);

    let (floor, above): (Vec<CellPos>, Vec<CellPos>) =
        sealed.inside().into_iter().partition(|pos| pos.y == 0);
    let grams: Vec<u32> = floor.iter().map(|&pos| sim.get(pos).mass_grams).collect();
    let (least, most) = (grams.iter().min().unwrap(), grams.iter().max().unwrap());
    assert!(
        most - least <= SPREADING_THRESHOLD,
        "the floor Cells hold from {least} g to {most} g"
    );
    for pos in above {
        assert_eq!(
            sim.get(pos).element,
            ElementId::VACUUM,
            "{pos:?} still holds liquid"
        );
    }
    assert_at_rest(&mut sim, 100, &sealed.all(), &invariants);
}

#[test]
fn liquid_spreads_into_gas_through_displacement_and_boxed_in_gas_blocks_it() {
    let mut elements = ElementTable::new();
    let wall = elements.add(test_wall());
    let water = elements.add(test_liquid("TestWater", 1_000.0));
    let gas = elements.add(test_gas("TestGas", 1.0));
    let other = elements.add(test_gas("OtherGas", 1.0));
    let mut sim = Simulation::new(elements);
    // Two separate rows on the floor of their own sealed boxes.
    let open = [0, 1, 2].map(|x| CellPos::new(x, 0, 0));
    let boxed = [0, 1, 2].map(|x| CellPos::new(x, 0, 10));
    let open_box = SealedBox::build(&mut sim, wall, open[0], open[2]);
    let boxed_box = SealedBox::build(&mut sim, wall, boxed[0], boxed[2]);
    // In the open row the gas next to the water can merge into its own neighbour.
    sim.set(open[0], cell(water, FULL, 300.0));
    sim.set(open[1], cell(gas, 1_000, 300.0));
    sim.set(open[2], cell(gas, 1_000, 300.0));
    // In the boxed row it touches only walls, the water and a different gas.
    sim.set(boxed[0], cell(water, FULL, 300.0));
    sim.set(boxed[1], cell(gas, 1_000, 300.0));
    sim.set(boxed[2], cell(other, 500, 300.0));
    let invariants = Invariants::record(&sim, [open_box.all(), boxed_box.all()].concat())
        .with_capacity(water, FULL);
    let boxed_before: Vec<_> = boxed_box.all().iter().map(|&pos| sim.get(pos)).collect();

    run_ticks(&mut sim, 1, &invariants);

    // The water advanced into the vacated Cell, and all of the gas merged into its neighbour:
    // 1000 g + 1000 g.
    assert_eq!(sim.get(open[1]).element, water);
    assert_eq!(sim.get(open[2]), cell(gas, 2_000, 300.0));
    let boxed_now: Vec<_> = boxed_box.all().iter().map(|&pos| sim.get(pos)).collect();
    assert_eq!(boxed_now, boxed_before, "boxed-in gas was displaced");
    assert_at_rest(&mut sim, 50, &boxed_box.all(), &invariants);
}

#[test]
fn a_column_dropped_at_the_centre_of_a_symmetric_box_spreads_equally_in_all_four_directions() {
    let mut elements = ElementTable::new();
    let wall = elements.add(test_wall());
    let water = elements.add(test_liquid("TestWater", 1_000.0));
    let mut sim = Simulation::new(elements);
    // Centred on x = z = 0, so the floor straddles four Chunks.
    let sealed = SealedBox::build(
        &mut sim,
        wall,
        CellPos::new(-3, 0, -3),
        CellPos::new(3, 4, 3),
    );
    // Four full Cells hanging one Cell above the floor.
    for y in 1..=4 {
        sim.set(CellPos::new(0, y, 0), cell(water, FULL, 300.0));
    }
    let invariants = Invariants::record(&sim, sealed.all()).with_capacity(water, FULL);

    for tick in 0..300 {
        run_ticks(&mut sim, 1, &invariants);
        for pos in sealed.inside() {
            // Mirroring x or z, or swapping them, maps this Cell onto its image here.
            let (a, b) = (pos.x.abs(), pos.z.abs());
            let image = CellPos::new(a.min(b), pos.y, a.max(b));
            assert_eq!(
                sim.get(pos),
                sim.get(image),
                "tick {tick}: {pos:?} differs from {image:?}"
            );
        }
    }
    // It reached every wall: 4,000,000 g over a 7 × 7 floor is about 81,600 g a Cell.
    for pos in sealed.inside().into_iter().filter(|pos| pos.y == 0) {
        assert_eq!(sim.get(pos).element, water, "{pos:?} stayed dry");
    }
}

/// A sealed box straddling eight Chunks, scattered with two liquids and two gases of varied Mass
/// and temperature and pockets of Vacuum, so every liquid and gas rule fires and competes for
/// Cells.
fn mixing_box() -> MixingBox {
    let mut elements = ElementTable::new();
    let wall = elements.add(test_wall());
    let heavy_liquid = elements.add(test_liquid("HeavyLiquid", 2_000.0));
    let light_liquid = elements.add(test_liquid("LightLiquid", 1_000.0));
    let heavy_gas = elements.add(test_gas("HeavyGas", 2.0));
    let light_gas = elements.add(test_gas("LightGas", 0.09));
    let mut sim = Simulation::new(elements);
    let sealed = SealedBox::build(
        &mut sim,
        wall,
        CellPos::new(-3, -3, -3),
        CellPos::new(2, 2, 2),
    );
    for pos in sealed.inside() {
        let pattern = (pos.x * 7 + pos.y * 13 + pos.z * 5).rem_euclid(6);
        let variety = (pos.x * 31 + pos.y * 17 + pos.z * 11).rem_euclid(50) as u32;
        let kelvin = 250.0 + (pos.x * 3 + pos.y * 5 + pos.z * 7).rem_euclid(150) as f32;
        let filling = match pattern {
            0 => cell(heavy_liquid, 1_000 + variety * 40_000, kelvin),
            1 => cell(light_liquid, 1_000 + variety * 20_000, kelvin),
            2 => cell(heavy_gas, 100 + variety * 1_000, kelvin),
            3 => cell(light_gas, 100 + variety * 1_000, kelvin),
            _ => continue, // Patterns 4 and 5 stay Vacuum.
        };
        sim.set(pos, filling);
    }
    MixingBox {
        sim,
        sealed,
        liquids: [(heavy_liquid, 2 * FULL), (light_liquid, FULL)],
        gases: [heavy_gas, light_gas],
    }
}

struct MixingBox {
    sim: Simulation,
    sealed: SealedBox,
    /// Each liquid, with its capacity.
    liquids: [(ElementId, u32); 2],
    gases: [ElementId; 2],
}

#[test]
fn mixing_liquids_and_gases_conserve_mass_per_element_and_play_out_the_same_every_time() {
    let MixingBox {
        sim: mut first,
        sealed,
        liquids,
        gases,
    } = mixing_box();
    let mut second = mixing_box().sim;
    let invariants = liquids.into_iter().fold(
        Invariants::record(&first, sealed.all()),
        |invariants, (liquid, capacity)| invariants.with_capacity(liquid, capacity),
    );

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
    // Liquids always sit below gases: no liquid Cell is left above a gas Cell.
    for pos in sealed.inside() {
        let below = CellPos::new(pos.x, pos.y - 1, pos.z);
        let is_liquid = liquids
            .iter()
            .any(|&(liquid, _)| first.get(pos).element == liquid);
        let gas_below = gases.contains(&first.get(below).element);
        assert!(!(is_liquid && gas_below), "liquid at {pos:?} rests on gas");
    }
}

// Cases the implementation revealed, beyond the eight tests in the plan.

#[test]
fn a_liquid_tops_up_a_partly_full_cell_of_the_same_liquid_below_it_and_mixes_its_heat() {
    let mut elements = ElementTable::new();
    let wall = elements.add(test_wall());
    let water = elements.add(test_liquid("TestWater", 1_000.0));
    let mut sim = Simulation::new(elements);
    let [below, above] = [CellPos::new(0, 0, 0), CellPos::new(0, 1, 0)];
    let sealed = SealedBox::build(&mut sim, wall, below, above);
    sim.set(below, cell(water, 600_000, 300.0));
    sim.set(above, cell(water, 700_000, 400.0));
    let invariants = Invariants::record(&sim, sealed.all()).with_capacity(water, FULL);

    run_ticks(&mut sim, 1, &invariants);

    // 400,000 g fall, filling the lower Cell: (600,000 g × 300 K + 400,000 g × 400 K)
    // / 1,000,000 g = 340 K. The rest stays where it was.
    let topped_up = sim.get(below);
    assert_eq!(topped_up.mass_grams, FULL);
    assert!(
        (topped_up.temperature_k - 340.0).abs() < 0.01,
        "{topped_up:?}"
    );
    assert_eq!(sim.get(above), cell(water, 300_000, 400.0));
    assert_at_rest(&mut sim, 20, &sealed.all(), &invariants);
}

#[test]
fn a_cell_over_capacity_sheds_its_excess_until_the_liquid_fills_whole_cells() {
    let mut elements = ElementTable::new();
    let wall = elements.add(test_wall());
    let water = elements.add(test_liquid("TestWater", 1_000.0));
    let mut sim = Simulation::new(elements);
    let column = column(6);
    let sealed = SealedBox::build(&mut sim, wall, column[0], column[5]);
    // Only a Phase Transition can do this in play (step 04); here it is set directly.
    sim.set(column[5], cell(water, 3 * FULL, 300.0));
    let invariants = Invariants::record(&sim, sealed.all());

    run_ticks(&mut sim, 30, &invariants);

    // Liquids do not compress: 3,000,000 g ends up as three full Cells.
    for &pos in &column[..3] {
        assert_eq!(sim.get(pos), cell(water, FULL, 300.0));
    }
    for &pos in &column[3..] {
        assert_eq!(
            sim.get(pos).element,
            ElementId::VACUUM,
            "{pos:?} still holds liquid"
        );
    }
}

#[test]
fn a_cell_over_capacity_sheds_its_excess_into_gas_through_displacement() {
    let mut elements = ElementTable::new();
    let wall = elements.add(test_wall());
    let water = elements.add(test_liquid("TestWater", 1_000.0));
    let gas = elements.add(test_gas("TestGas", 1.0));
    let mut sim = Simulation::new(elements);
    let column = column(3);
    let sealed = SealedBox::build(&mut sim, wall, column[0], column[2]);
    sim.set(column[0], cell(water, 1_200_000, 300.0));
    sim.set(column[1], cell(gas, 1_000, 300.0));
    sim.set(column[2], cell(gas, 1_000, 300.0));
    let invariants = Invariants::record(&sim, sealed.all());

    run_ticks(&mut sim, 1, &invariants);

    // The only way out is up, so all 200,000 g of excess go into the gas Cell, whose gas merges
    // into its own neighbour: 1000 g + 1000 g.
    assert_eq!(sim.get(column[0]), cell(water, FULL, 300.0));
    assert_eq!(sim.get(column[1]), cell(water, 200_000, 300.0));
    assert_eq!(sim.get(column[2]), cell(gas, 2_000, 300.0));
    assert_at_rest(&mut sim, 20, &sealed.all(), &invariants);
}

#[test]
fn a_cell_over_capacity_boxed_in_by_solids_stays_over_capacity() {
    let mut elements = ElementTable::new();
    let wall = elements.add(test_wall());
    let water = elements.add(test_liquid("TestWater", 1_000.0));
    let mut sim = Simulation::new(elements);
    let pos = CellPos::new(0, 0, 0);
    let sealed = SealedBox::build(&mut sim, wall, pos, pos);
    sim.set(pos, cell(water, 1_200_000, 300.0));
    let invariants = Invariants::record(&sim, sealed.all());

    // Compression is not modelled, so nothing can happen.
    assert_at_rest(&mut sim, 20, &sealed.all(), &invariants);
}

#[test]
fn several_cells_over_capacity_shedding_into_one_cell_never_fill_it_past_capacity() {
    let mut elements = ElementTable::new();
    let wall = elements.add(test_wall());
    let water = elements.add(test_liquid("TestWater", 1_000.0));
    let mut sim = Simulation::new(elements);
    let row = [0, 1, 2].map(|x| CellPos::new(x, 0, 0));
    let sealed = SealedBox::build(&mut sim, wall, row[0], row[2]);
    // Both ends hold far more than the middle Cell has room for.
    sim.set(row[0], cell(water, 1_900_000, 300.0));
    sim.set(row[2], cell(water, 1_900_000, 300.0));
    let invariants = Invariants::record(&sim, sealed.all());

    run_ticks(&mut sim, 1, &invariants);

    // Each end gets an equal half of the middle Cell's room: 2 × 500,000 g.
    assert_eq!(sim.get(row[1]), cell(water, FULL, 300.0));
    assert_eq!(sim.get(row[0]), cell(water, 1_400_000, 300.0));
    assert_eq!(sim.get(row[2]), cell(water, 1_400_000, 300.0));
}

#[test]
fn liquid_poured_into_a_box_of_gas_settles_level_beneath_it_and_comes_to_rest() {
    let mut elements = ElementTable::new();
    let wall = elements.add(test_wall());
    let water = elements.add(test_liquid("TestWater", 1_000.0));
    let gas = elements.add(test_gas("TestGas", 1.0));
    let mut sim = Simulation::new(elements);
    let sealed = SealedBox::build(&mut sim, wall, CellPos::new(0, 0, 0), CellPos::new(4, 5, 4));
    for pos in sealed.inside() {
        sim.set(pos, cell(gas, 1_000, 300.0));
    }
    // Two full Cells stacked in a top corner: 2,000,000 g over a 5 × 5 floor is 80,000 g a Cell.
    sim.set(CellPos::new(0, 5, 0), cell(water, FULL, 300.0));
    sim.set(CellPos::new(0, 4, 0), cell(water, FULL, 300.0));
    let invariants = Invariants::record(&sim, sealed.all()).with_capacity(water, FULL);

    // It comes to rest after about 220 Ticks; this leaves a wide margin.
    run_ticks(&mut sim, 400, &invariants);

    let (floor, above): (Vec<CellPos>, Vec<CellPos>) =
        sealed.inside().into_iter().partition(|pos| pos.y == 0);
    let grams: Vec<u32> = floor
        .iter()
        .map(|&pos| {
            assert_eq!(sim.get(pos).element, water, "{pos:?} holds no water");
            sim.get(pos).mass_grams
        })
        .collect();
    let (least, most) = (grams.iter().min().unwrap(), grams.iter().max().unwrap());
    assert!(
        most - least <= SPREADING_THRESHOLD,
        "the floor Cells hold from {least} g to {most} g"
    );
    for pos in above {
        assert_eq!(sim.get(pos).element, gas, "{pos:?} holds no gas");
    }
    assert_at_rest(&mut sim, 100, &sealed.all(), &invariants);
}
