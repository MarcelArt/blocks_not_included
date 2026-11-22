mod common;

use common::*;
use sim::{
    CHUNK_SIZE, Cell, CellPos, ChunkPos, Edit, Element, ElementId, ElementTable, SimSettings,
    SimStats, Simulation,
};

const SHIPPED_ELEMENTS: &str = include_str!("../../../assets/elements.ron");

/// The shipped Elements, plus a `TestWall` that conducts no heat.
fn shipped_elements() -> ElementTable {
    let mut elements = ElementTable::from_ron(SHIPPED_ELEMENTS).expect("the shipped file is valid");
    elements.add(test_wall());
    elements
}

/// A solid that conducts heat well: 1000 g of it holds 1000 J/K.
fn test_rock() -> Element {
    Element::new("TestRock")
        .density(1_000.0)
        .specific_heat(1.0)
        .conductivity(100.0)
}

fn id(elements: &ElementTable, name: &str) -> ElementId {
    elements
        .id_of(name)
        .unwrap_or_else(|| panic!("there is no Element called {name:?}"))
}

#[test]
fn edit_dig_turns_a_cell_into_vacuum_and_neighbouring_gas_flows_in() {
    let mut elements = ElementTable::new();
    let wall = elements.add(test_wall());
    let gas = elements.add(test_gas("TestGas", 1.0));
    let mut sim = Simulation::new(elements);
    let source = CellPos::new(0, 0, 0);
    let dug = CellPos::new(1, 0, 0);
    let sealed = SealedBox::build(&mut sim, wall, source, dug);
    sim.set(source, cell(gas, 1_000, 300.0));
    sim.set(dug, cell(wall, 1_000, 300.0));

    sim.apply(Edit::Dig { pos: dug });

    assert_eq!(sim.get(dug), Cell::VACUUM);
    let invariants = Invariants::record(&sim, sealed.all());
    run_ticks(&mut sim, 3, &invariants);
    let arrived = sim.get(dug);
    assert_eq!(
        arrived.element, gas,
        "the gas should have flowed into the dug Cell"
    );
    assert!(arrived.mass_grams > 0);
}

#[test]
fn edit_place_overwrites_a_cell_and_total_mass_changes_by_exactly_the_difference() {
    let mut elements = ElementTable::new();
    let wall = elements.add(test_wall());
    let gas = elements.add(test_gas("TestGas", 1.0));
    let mut sim = Simulation::new(elements);
    let pos = CellPos::new(0, 0, 0);
    let sealed = SealedBox::build(&mut sim, wall, pos, CellPos::new(1, 0, 0));
    sim.set(pos, cell(gas, 1_000, 300.0));
    sim.set(CellPos::new(1, 0, 0), cell(gas, 1_000, 300.0));
    let before = sim.total_mass_grams();

    let placed = cell(wall, 2_700_000, 450.0);
    sim.apply(Edit::Place { pos, cell: placed });

    assert_eq!(sim.get(pos), placed);
    // 1,000 g of gas replaced by 2,700,000 g of wall: a gain of 2,699,000 g.
    assert_eq!(sim.total_mass_grams(), before + 2_699_000);
    // From here on, Ticks conserve Mass again.
    let invariants = Invariants::record(&sim, sealed.all());
    run_ticks(&mut sim, 3, &invariants);
}

#[test]
fn edit_shift_temperature_warms_a_cell_by_the_given_kelvin_and_never_below_0_k() {
    let mut elements = ElementTable::new();
    let wall = elements.add(test_wall());
    let mut sim = Simulation::new(elements);
    let pos = CellPos::new(0, 0, 0);
    let vacuum = CellPos::new(5, 0, 0);
    sim.set(pos, cell(wall, 1_000, 300.0));

    sim.apply(Edit::ShiftTemperature { pos, kelvin: 25.0 });
    // 300 K + 25 K.
    assert_eq!(sim.get(pos), cell(wall, 1_000, 325.0));

    sim.apply(Edit::ShiftTemperature {
        pos,
        kelvin: -1_000.0,
    });
    // 325 K − 1000 K would be −675 K; temperature stops at absolute zero.
    assert_eq!(sim.get(pos), cell(wall, 1_000, 0.0));

    // Vacuum holds no matter, so it has no temperature to shift.
    sim.apply(Edit::ShiftTemperature {
        pos: vacuum,
        kelvin: 50.0,
    });
    assert_eq!(sim.get(vacuum), Cell::VACUUM);
}

#[test]
fn heat_runs_before_phase_transitions_within_a_tick() {
    let elements = shipped_elements();
    let [wall, granite, water, steam] =
        ["TestWall", "Granite", "Water", "Steam"].map(|name| id(&elements, name));
    let mut sim = Simulation::new(elements);
    let water_pos = CellPos::new(0, 0, 0);
    let granite_pos = CellPos::new(1, 0, 0);
    let sealed = SealedBox::build(&mut sim, wall, water_pos, granite_pos);
    sim.set(water_pos, cell(water, 10, 373.0));
    sim.set(granite_pos, cell(granite, 2_700_000, 500.0));
    let invariants = Invariants::record(&sim, sealed.all()).allowing_phase_transitions();

    run_ticks(&mut sim, 1, &invariants);

    // Water's conductivity (0.609 W/(m·K)) is the poorer, so 0.609 × 0.2 s × (500 − 373) K
    // = 15.5 J flow in, well under the per-face cap of about 127 K × 41.79 J/K / 6 = 885 J.
    // 10 g of Water holds 41.79 J/K, so it warms by 0.37 K to about 373.37 K: past Water's
    // boiling point of 373.15 K, so it is Steam by the end of the same Tick.
    let boiled = sim.get(water_pos);
    assert_eq!(boiled.element, steam, "got {boiled:?}");
    assert!(
        (boiled.temperature_k - 373.37).abs() < 0.01,
        "expected about 373.37 K, got {boiled:?}"
    );
}

#[test]
fn doubling_the_conduction_multiplier_doubles_a_single_ticks_heat_flow() {
    let warming_with = |conduction_multiplier: f32| {
        let mut elements = ElementTable::new();
        let rock = elements.add(test_rock());
        let mut sim = Simulation::new(elements);
        sim.set_settings(SimSettings {
            conduction_multiplier,
            ..SimSettings::default()
        })
        .expect("valid settings");
        let (hot, cold) = (CellPos::new(0, 0, 0), CellPos::new(1, 0, 0));
        sim.set(hot, cell(rock, 1_000, 400.0));
        sim.set(cold, cell(rock, 1_000, 300.0));
        let invariants = Invariants::record(&sim, vec![hot, cold]);
        run_ticks(&mut sim, 1, &invariants);
        sim.get(cold).temperature_k - 300.0
    };

    // 100 W/(m·K) × 0.2 s × 100 K = 2000 J into 1000 J/K warms the cold Cell by 2 K. The per-face
    // cap is 100 K × (1000 × 1000 / 2000) J/K / 6 = 8333 J, so even the doubled 4000 J is under
    // it. Every value here is exact in f32.
    assert_eq!(warming_with(1.0), 2.0);
    assert_eq!(warming_with(2.0), 4.0);
}

#[test]
fn a_chunk_where_nothing_happens_is_settled_after_one_tick() {
    let mut elements = ElementTable::new();
    let rock = elements.add(test_rock());
    let mut sim = Simulation::new(elements);
    // A block of rock at one temperature, inside one Chunk: no heat flows and nothing moves.
    let block = cuboid(CellPos::new(4, 4, 4), CellPos::new(6, 6, 6));
    for &pos in &block {
        sim.set(pos, cell(rock, 1_000, 300.0));
    }
    let invariants = Invariants::record(&sim, block);
    // 27 Cells of 1000 g each.
    let total_mass_grams = 27_000;
    assert_eq!(
        sim.stats(),
        SimStats {
            tick_count: 0,
            active_chunks: 1,
            awake_chunks: 1,
            settled_chunks: 0,
            dormant_chunks: 0,
            total_mass_grams,
            mass_lost_to_space_grams: 0,
        },
        "setting Cells wakes their Chunk"
    );

    run_ticks(&mut sim, 1, &invariants);

    assert_eq!(
        sim.stats(),
        SimStats {
            tick_count: 1,
            active_chunks: 1,
            awake_chunks: 0,
            settled_chunks: 1,
            dormant_chunks: 0,
            total_mass_grams,
            mass_lost_to_space_grams: 0,
        }
    );
}

#[test]
fn an_edit_in_a_settled_chunk_wakes_it_and_gas_flows_into_the_dug_cell() {
    let mut elements = ElementTable::new();
    let wall = elements.add(test_wall());
    let gas = elements.add(test_gas("TestGas", 1.0));
    let mut sim = Simulation::new(elements);
    // Chunks are 32 Cells wide: the gas at x = 29..=31 is in one Chunk, and the wall Cell at
    // x = 32 that gets dug is in the next. The whole box, walls included, sits in those two.
    let dug = CellPos::new(32, 5, 5);
    let sealed = SealedBox::build(&mut sim, wall, CellPos::new(29, 5, 5), dug);
    for x in 29..=31 {
        sim.set(CellPos::new(x, 5, 5), cell(gas, 1_000, 300.0));
    }
    sim.set(dug, cell(wall, 1_000, 300.0));
    let invariants = Invariants::record(&sim, sealed.all());
    // Evenly spread gas and walls that conduct no heat: nothing changes, so both Chunks settle.
    run_ticks(&mut sim, 1, &invariants);
    assert_eq!(
        (sim.stats().awake_chunks, sim.stats().settled_chunks),
        (0, 2)
    );

    sim.apply(Edit::Dig { pos: dug });

    assert_eq!(
        (sim.stats().awake_chunks, sim.stats().settled_chunks),
        (1, 1),
        "the Edit wakes only the Chunk it is in"
    );
    let invariants = Invariants::record(&sim, sealed.all());
    run_ticks(&mut sim, 1, &invariants);
    // The gas Cell beside the dug one has two open faces (gas and the new Vacuum), so it moves
    // 1000 g × 0.25 / 2 = 125 g across the border, waking the gas's Chunk too.
    assert_eq!(sim.get(dug), cell(gas, 125, 300.0));
    assert_eq!(
        (sim.stats().awake_chunks, sim.stats().settled_chunks),
        (2, 0)
    );
}

#[test]
fn heat_flowing_out_of_an_awake_chunk_wakes_a_settled_neighbour_and_crosses_the_border() {
    let mut elements = ElementTable::new();
    let rock = elements.add(test_rock());
    let mut sim = Simulation::new(elements);
    // A bar of rock at one temperature, crossing from one Chunk (x ≤ 31) into the next (x ≥ 32).
    let bar = cuboid(CellPos::new(28, 5, 5), CellPos::new(35, 5, 5));
    for &pos in &bar {
        sim.set(pos, cell(rock, 1_000, 300.0));
    }
    let invariants = Invariants::record(&sim, bar.clone());
    run_ticks(&mut sim, 1, &invariants);
    assert_eq!(
        (sim.stats().awake_chunks, sim.stats().settled_chunks),
        (0, 2)
    );

    let (last_in_awake, first_in_settled, second_in_settled) = (
        CellPos::new(31, 5, 5),
        CellPos::new(32, 5, 5),
        CellPos::new(33, 5, 5),
    );
    sim.apply(Edit::ShiftTemperature {
        pos: last_in_awake,
        kelvin: 100.0,
    });
    assert_eq!(
        (sim.stats().awake_chunks, sim.stats().settled_chunks),
        (1, 1)
    );

    run_ticks(&mut sim, 1, &invariants);
    // 100 W/(m·K) × 0.2 s × (400 − 300) K = 2000 J crosses the border into 1000 J/K: +2 K,
    // exact in f32. Receiving it wakes the Settled Chunk.
    assert_eq!(sim.get(first_in_settled).temperature_k, 302.0);
    assert_eq!(
        (sim.stats().awake_chunks, sim.stats().settled_chunks),
        (2, 0)
    );

    run_ticks(&mut sim, 1, &invariants);
    // Now awake, the Chunk passes the heat on inside itself: at least 100 W/(m·K) × 0.2 s ×
    // (302 − 300) K = 40 J into 1000 J/K, so +0.04 K.
    let inside_k = sim.get(second_in_settled).temperature_k;
    assert!(
        inside_k >= 300.039,
        "the heat stopped at the border: {inside_k} K"
    );
}

/// Two hot spots, released after everything has settled, right at the corner where four Chunks
/// meet. Both cool quickly:
/// - A hot pebble set into a massive rock floor that conducts heat well. The pebble's flows hit
///   the per-face cap, so each Tick it hands two thirds of its excess heat to the four floor
///   Cells beside it (walls and gas conduct none). The floor is so massive that it warms by only
///   about 0.0002 K.
/// - A small puff of hot light gas at the bottom of a room of cold gas, layered dense below and
///   light above. It rises through the dense layer, pushing gas across the Chunk borders as it
///   goes, and cools as cold gas mixes into it, until the gas comes to rest. Gas here conducts no
///   heat, so its temperatures change only as Mass moves.
///
/// Returns the Simulation and the Cells of the box and its walls.
fn run_two_cooling_hot_spots(settings: SimSettings, ticks: usize) -> (Simulation, Vec<CellPos>) {
    let mut elements = ElementTable::new();
    let wall = elements.add(test_wall());
    let conductor = elements.add(
        Element::new("TestConductor")
            .density(1_000.0)
            .specific_heat(1.0)
            .conductivity(10_000.0),
    );
    let heavy = elements.add(test_gas("Heavy", 2.0));
    let light = elements.add(test_gas("Light", 0.5));
    let mut sim = Simulation::new(elements);
    let settling = settings.settled_chunks_enabled;
    sim.set_settings(settings).expect("valid settings");
    let sealed = SealedBox::build(
        &mut sim,
        wall,
        CellPos::new(26, 26, 26),
        CellPos::new(37, 30, 37),
    );
    for pos in sealed.inside() {
        let filling = match pos.y {
            26 => cell(conductor, 1_000_000_000, 250.0),
            27 | 28 => cell(heavy, 1_000, 250.0),
            _ => cell(light, 1_000, 250.0),
        };
        sim.set(pos, filling);
    }
    let invariants = Invariants::record(&sim, sealed.all());
    run_ticks(&mut sim, 1, &invariants);
    if settling {
        assert_eq!(
            sim.stats().settled_chunks,
            4,
            "everything is at rest before the hot spots"
        );
    }

    // The corner Cell at (31, _, 31) touches the Chunks at x = 32 and z = 32.
    sim.apply(Edit::Place {
        pos: CellPos::new(31, 26, 31),
        cell: cell(conductor, 1_000, 1_000.0),
    });
    sim.apply(Edit::Place {
        pos: CellPos::new(31, 27, 31),
        cell: cell(light, 500, 600.0),
    });
    let invariants = Invariants::record(&sim, sealed.all());
    run_ticks(&mut sim, ticks, &invariants);
    (sim, sealed.all())
}

#[test]
fn settled_chunks_change_nothing_beyond_the_settle_threshold() {
    // Both hot spots cool quickly, and gas comes to rest in whole grams, so a Chunk that settles
    // here has nothing left to change. A region that cools slowly is different: it settles once
    // its per-Tick change drops below the threshold, and stops short of where it would otherwise
    // end up, by up to the threshold divided by the fraction of the gap it closes per Tick.
    // `settle_threshold_k` is the knob for that (approved by the human in step 05).
    let ticks = 200;
    let on = SimSettings::default();
    let off = SimSettings {
        settled_chunks_enabled: false,
        ..SimSettings::default()
    };
    let (settled, cells) = run_two_cooling_hot_spots(on.clone(), ticks);
    let (simulated, _) = run_two_cooling_hot_spots(off, ticks);

    // The pebble keeps a third of its excess each Tick: 750 K × (1/3)^200 is nothing.
    let pebble_k = simulated.get(CellPos::new(31, 26, 31)).temperature_k;
    assert!(
        (pebble_k - 250.0).abs() < 0.01,
        "the pebble is still at {pebble_k} K"
    );
    assert_eq!(
        settled.stats().settled_chunks,
        4,
        "everything came to rest, so every Chunk should be Settled and skipped"
    );
    for pos in cells {
        let (a, b) = (settled.get(pos), simulated.get(pos));
        assert_eq!(
            (a.element, a.mass_grams),
            (b.element, b.mass_grams),
            "{pos:?} differs in Element or Mass"
        );
        assert!(
            (a.temperature_k - b.temperature_k).abs() <= on.settle_threshold_k,
            "{pos:?} is {} K with Settled Chunks and {} K without",
            a.temperature_k,
            b.temperature_k
        );
    }
}

#[test]
fn take_changed_chunks_reports_exactly_the_chunks_that_changed_then_returns_an_empty_list() {
    let mut elements = ElementTable::new();
    let rock = elements.add(test_rock());
    let mut sim = Simulation::new(elements);
    // A bar of rock crossing from one Chunk (x ≤ 31) into the next, and a lone rock Cell in a
    // third Chunk two Chunks away along z.
    let bar = cuboid(CellPos::new(28, 5, 5), CellPos::new(35, 5, 5));
    let lone = CellPos::new(5, 5, 70);
    for &pos in &bar {
        sim.set(pos, cell(rock, 1_000, 300.0));
    }
    sim.set(lone, cell(rock, 1_000, 300.0));
    let invariants = Invariants::record(&sim, [bar, vec![lone]].concat());
    let [first, second, third] =
        [CellPos::new(31, 5, 5), CellPos::new(32, 5, 5), lone].map(ChunkPos::of);

    // Setting Cells changes them. The list is sorted by x, then y, then z.
    assert_eq!(sim.take_changed_chunks(), vec![first, third, second]);
    assert_eq!(sim.take_changed_chunks(), vec![]);

    // Everything is at one temperature, so a Tick changes nothing.
    run_ticks(&mut sim, 1, &invariants);
    assert_eq!(sim.take_changed_chunks(), vec![]);

    // Heat the bar's end Cell in the first Chunk; the next Tick carries heat across the border
    // into the second. The lone Cell is untouched.
    sim.apply(Edit::ShiftTemperature {
        pos: CellPos::new(31, 5, 5),
        kelvin: 100.0,
    });
    run_ticks(&mut sim, 1, &invariants);
    assert_eq!(sim.take_changed_chunks(), vec![first, second]);
    assert_eq!(sim.take_changed_chunks(), vec![]);
}

#[test]
fn gas_pushed_into_a_settled_chunk_spreads_on_inside_it_within_the_same_tick() {
    let mut elements = ElementTable::new();
    let wall = elements.add(test_wall());
    let first = elements.add(test_gas("First", 1.0));
    let second = elements.add(test_gas("Second", 1.0));
    let mut sim = Simulation::new(elements);
    // A sealed corridor one Cell across: one gas at x = 28..=31 in one Chunk, another at
    // x = 32..=35 in the next, 1000 g in every Cell. Equal Mass everywhere, so nothing moves.
    let row = |x| CellPos::new(x, 5, 5);
    let sealed = SealedBox::build(&mut sim, wall, row(28), row(35));
    for x in 28..=35 {
        let element = if x <= 31 { first } else { second };
        sim.set(row(x), cell(element, 1_000, 300.0));
    }
    let invariants = Invariants::record(&sim, sealed.all());
    run_ticks(&mut sim, 1, &invariants);
    assert_eq!(
        (sim.stats().awake_chunks, sim.stats().settled_chunks),
        (0, 2)
    );

    sim.apply(Edit::Place {
        pos: row(31),
        cell: cell(first, 5_000, 300.0),
    });
    let invariants = Invariants::record(&sim, sealed.all());
    run_ticks(&mut sim, 1, &invariants);

    // Displacement first: x = 31 pushes 5000 g × 0.25 / 2 open faces = 625 g into x = 32, in the
    // Settled Chunk, and the 1000 g there squeezes on into x = 33, which now holds 2000 g.
    // Spreading next, in the same Tick: x = 33 sends (2000 − 1000) × 0.25 / 2 = 125 g to x = 34,
    // although neither Cell touches the awake Chunk. On the other side, x = 31 sends
    // (4375 − 1000) × 0.125 = 421 g to x = 30 and (4375 − 625) × 0.125 = 468 g to x = 32.
    let after: Vec<(ElementId, u32)> = (28..=35)
        .map(|x| (sim.get(row(x)).element, sim.get(row(x)).mass_grams))
        .collect();
    assert_eq!(
        after,
        [
            (first, 1_000),
            (first, 1_000),
            (first, 1_421),
            (first, 3_486),
            (first, 1_093),
            (second, 1_875),
            (second, 1_125),
            (second, 1_000),
        ]
    );
}

#[test]
fn chunk_cells_lists_a_chunks_cells_x_fastest_then_z_then_y_from_its_origin() {
    let mut elements = ElementTable::new();
    let rock = elements.add(test_rock());
    let mut sim = Simulation::new(elements);
    let pos = CellPos::new(-30, 4, 70);
    let marked = cell(rock, 1_234, 321.0);
    sim.set(pos, marked);

    // Chunks are 32 Cells wide, so x = -30 lies in Chunk -1, which starts at x = -32.
    let chunk = ChunkPos::of(pos);
    assert_eq!(chunk.origin(), CellPos::new(-32, 0, 64));
    let cells = sim
        .chunk_cells(chunk)
        .expect("setting a Cell creates its Chunk");
    assert_eq!(cells.len(), CHUNK_SIZE * CHUNK_SIZE * CHUNK_SIZE);
    // Local (2, 4, 6): (y × 32 + z) × 32 + x = (4 × 32 + 6) × 32 + 2 = 4290.
    assert_eq!(cells[4290], marked);
    assert_eq!(
        cells.iter().filter(|&&cell| cell != Cell::VACUUM).count(),
        1
    );
    assert_eq!(sim.chunk_cells(ChunkPos::of(CellPos::new(0, 0, 0))), None);
}

#[test]
fn set_settings_rejects_a_value_that_would_break_the_invariants_and_keeps_the_previous_settings() {
    let mut sim = Simulation::new(ElementTable::new());
    let tuned = SimSettings {
        conduction_multiplier: 1_000.0,
        ..SimSettings::default()
    };
    sim.set_settings(tuned.clone()).expect("valid settings");

    // A pair moving more than half its difference overshoots; a Cell moving more than all of it
    // would send away Mass it doesn't hold.
    let error = sim
        .set_settings(SimSettings {
            gas_evening_out_fraction: 0.6,
            ..SimSettings::default()
        })
        .expect_err("0.6 overshoots");
    assert_eq!(error.setting, "gas_evening_out_fraction");
    assert_eq!(sim.settings(), &tuned);

    let error = sim
        .set_settings(SimSettings {
            conduction_multiplier: f32::NAN,
            ..SimSettings::default()
        })
        .expect_err("NaN would spread through every temperature");
    assert_eq!(error.setting, "conduction_multiplier");
    assert_eq!(sim.settings(), &tuned);
}

#[test]
fn changing_the_settings_wakes_every_chunk() {
    let mut elements = ElementTable::new();
    let rock = elements.add(test_rock());
    let mut sim = Simulation::new(elements);
    let pos = CellPos::new(5, 5, 5);
    sim.set(pos, cell(rock, 1_000, 300.0));
    let invariants = Invariants::record(&sim, vec![pos]);
    run_ticks(&mut sim, 1, &invariants);
    assert_eq!(sim.stats().settled_chunks, 1);

    // A Chunk settled under the old settings, for example a looser threshold, may not be at rest
    // under the new ones.
    sim.set_settings(SimSettings {
        settle_threshold_k: 0.001,
        ..SimSettings::default()
    })
    .expect("valid settings");

    assert_eq!(
        (sim.stats().awake_chunks, sim.stats().settled_chunks),
        (1, 0)
    );
}

#[test]
fn edit_place_never_breaks_the_cell_invariants() {
    let mut elements = ElementTable::new();
    let rock = elements.add(test_rock());
    let mut sim = Simulation::new(elements);
    let pos = CellPos::new(0, 0, 0);

    // Every non-Vacuum Cell holds at least 1 g, so placing none of an Element leaves Vacuum.
    sim.apply(Edit::Place {
        pos,
        cell: cell(rock, 0, 300.0),
    });
    assert_eq!(sim.get(pos), Cell::VACUUM);

    // Vacuum always has zero Mass.
    sim.apply(Edit::Place {
        pos,
        cell: cell(ElementId::VACUUM, 500, 300.0),
    });
    assert_eq!(sim.get(pos), Cell::VACUUM);

    // Like ShiftTemperature, Place never goes below 0 K.
    sim.apply(Edit::Place {
        pos,
        cell: cell(rock, 1_000, -5.0),
    });
    assert_eq!(sim.get(pos), cell(rock, 1_000, 0.0));
}
