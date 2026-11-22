use sim::{Cell, CellPos, Element, ElementId, ElementTable, SimSettings, Simulation};

fn cell(element: ElementId, mass_grams: u32, temperature_k: f32) -> Cell {
    Cell {
        element,
        mass_grams,
        temperature_k,
    }
}

fn test_rock() -> Element {
    Element::new("TestRock")
        .specific_heat(1.0)
        .conductivity(100.0)
}

fn run_ticks(sim: &mut Simulation, ticks: usize) {
    for _ in 0..ticks {
        sim.tick();
    }
}

#[test]
fn heat_flows_from_a_hotter_cell_into_a_colder_face_neighbour() {
    let mut elements = ElementTable::new();
    let rock = elements.add(test_rock());
    let mut sim = Simulation::new(elements);
    let hot = CellPos::new(0, 0, 0);
    let cold = CellPos::new(1, 0, 0);
    sim.set(hot, cell(rock, 1_000, 400.0));
    sim.set(cold, cell(rock, 1_000, 300.0));

    sim.tick();

    assert!(sim.get(hot).temperature_k < 400.0 && sim.get(cold).temperature_k > 300.0);
}

#[test]
fn touching_cells_settle_at_their_heat_capacity_weighted_average_temperature() {
    let mut elements = ElementTable::new();
    let rock = elements.add(test_rock());
    let mut sim = Simulation::new(elements);
    // This checks where conduction ends up, so every Chunk keeps simulating. With Settled Chunks
    // on, the pair would freeze once its per-Tick change dropped below the settle threshold,
    // about 0.36 K short of the average (an amendment approved by the human in step 05).
    sim.set_settings(SimSettings {
        settled_chunks_enabled: false,
        ..SimSettings::default()
    })
    .expect("valid settings");
    let big = CellPos::new(0, 0, 0);
    let small = CellPos::new(1, 0, 0);
    sim.set(big, cell(rock, 3_000, 300.0));
    sim.set(small, cell(rock, 1_000, 400.0));

    run_ticks(&mut sim, 2_000);

    // Same Element, so (3000 g × 300 K + 1000 g × 400 K) / 4000 g = 325 K.
    let temperatures = [sim.get(big).temperature_k, sim.get(small).temperature_k];
    assert!(
        temperatures.iter().all(|t| (t - 325.0).abs() < 0.01),
        "expected both Cells at 325 K, got {temperatures:?}"
    );
}

#[test]
fn a_vacuum_gap_blocks_heat_completely() {
    let mut elements = ElementTable::new();
    let rock = elements.add(test_rock());
    let mut sim = Simulation::new(elements);
    let hot = CellPos::new(0, 0, 0);
    let cold = CellPos::new(2, 0, 0); // (1, 0, 0) is never set, so it is Vacuum.
    sim.set(hot, cell(rock, 1_000, 400.0));
    sim.set(cold, cell(rock, 1_000, 300.0));

    run_ticks(&mut sim, 100);

    assert_eq!(sim.get(cold).temperature_k, 300.0);
}

#[test]
fn cells_touching_only_diagonally_exchange_no_heat() {
    let mut elements = ElementTable::new();
    let rock = elements.add(test_rock());
    let mut sim = Simulation::new(elements);
    let hot = CellPos::new(0, 0, 0);
    let diagonal = CellPos::new(1, 1, 0); // Shares only an edge with `hot`.
    sim.set(hot, cell(rock, 1_000, 400.0));
    sim.set(diagonal, cell(rock, 1_000, 300.0));

    run_ticks(&mut sim, 100);

    assert_eq!(sim.get(diagonal).temperature_k, 300.0);
}

#[test]
fn heat_crosses_chunk_borders() {
    let mut elements = ElementTable::new();
    let rock = elements.add(test_rock());
    let mut sim = Simulation::new(elements);
    // Chunks are 32 Cells wide, so these two face neighbours sit in different Chunks.
    let hot = CellPos::new(31, 0, 0);
    let cold = CellPos::new(32, 0, 0);
    sim.set(hot, cell(rock, 1_000, 400.0));
    sim.set(cold, cell(rock, 1_000, 300.0));

    sim.tick();

    assert!(sim.get(hot).temperature_k < 400.0 && sim.get(cold).temperature_k > 300.0);
}

#[test]
fn a_tiny_cell_never_ends_up_hotter_than_the_huge_cells_heating_it() {
    let mut elements = ElementTable::new();
    let rock = elements.add(test_rock());
    let mut sim = Simulation::new(elements);
    let tiny = CellPos::new(0, 0, 0);
    sim.set(tiny, cell(rock, 1, 300.0));
    // Huge hot Cells on all six faces push heat in at once.
    let heaters = [
        CellPos::new(1, 0, 0),
        CellPos::new(-1, 0, 0),
        CellPos::new(0, 1, 0),
        CellPos::new(0, -1, 0),
        CellPos::new(0, 0, 1),
        CellPos::new(0, 0, -1),
    ];
    for heater in heaters {
        sim.set(heater, cell(rock, 1_000_000_000, 1_000.0));
    }

    for tick in 0..100 {
        sim.tick();
        let tiny_k = sim.get(tiny).temperature_k;
        let coolest_heater_k = heaters
            .iter()
            .map(|&heater| sim.get(heater).temperature_k)
            .fold(f32::INFINITY, f32::min);
        // A thousandth of a kelvin allows for f32 rounding; overshoot would be hundreds of kelvin.
        assert!(
            tiny_k <= coolest_heater_k + 0.001,
            "tick {tick}: tiny Cell at {tiny_k} K overshot the heaters at {coolest_heater_k} K"
        );
    }
}

#[test]
fn the_poorer_conductor_limits_heat_flow_between_unlike_elements() {
    let mut elements = ElementTable::new();
    let good = elements.add(Element::new("GoodConductor").specific_heat(1.0).conductivity(1_000.0));
    let poor = elements.add(Element::new("PoorConductor").specific_heat(1.0).conductivity(1.0));
    let mut sim = Simulation::new(elements);
    // Two pairs, far apart with Vacuum between them, so they can't affect each other.
    let (mixed_hot, mixed_cold) = (CellPos::new(0, 0, 0), CellPos::new(1, 0, 0));
    let (poor_hot, poor_cold) = (CellPos::new(10, 0, 0), CellPos::new(11, 0, 0));
    sim.set(mixed_hot, cell(good, 1_000, 400.0));
    sim.set(mixed_cold, cell(poor, 1_000, 300.0));
    sim.set(poor_hot, cell(poor, 1_000, 400.0));
    sim.set(poor_cold, cell(poor, 1_000, 300.0));

    sim.tick();

    // Every Cell has the same heat capacity, so equal warming means equal heat flow.
    let mixed_warming = sim.get(mixed_cold).temperature_k - 300.0;
    let poor_warming = sim.get(poor_cold).temperature_k - 300.0;
    assert!(
        mixed_warming <= poor_warming * 1.000_1,
        "good→poor warmed by {mixed_warming} K, faster than poor→poor at {poor_warming} K"
    );
}

#[test]
fn heat_spreads_equally_in_every_direction() {
    let mut elements = ElementTable::new();
    let rock = elements.add(test_rock());
    let mut sim = Simulation::new(elements);
    // A 3×3×3 cube centred on the origin, so it straddles eight Chunks.
    for x in -1..=1 {
        for y in -1..=1 {
            for z in -1..=1 {
                sim.set(CellPos::new(x, y, z), cell(rock, 1_000, 300.0));
            }
        }
    }
    sim.set(CellPos::new(0, 0, 0), cell(rock, 1_000, 400.0));

    sim.tick();

    let face_neighbours = [
        CellPos::new(1, 0, 0),
        CellPos::new(-1, 0, 0),
        CellPos::new(0, 1, 0),
        CellPos::new(0, -1, 0),
        CellPos::new(0, 0, 1),
        CellPos::new(0, 0, -1),
    ]
    .map(|pos| sim.get(pos).temperature_k);
    let spread = face_neighbours.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b))
        - face_neighbours.iter().fold(f32::INFINITY, |a, &b| a.min(b));
    assert!(spread < 1e-4, "face neighbours warmed unequally: {face_neighbours:?}");
}
