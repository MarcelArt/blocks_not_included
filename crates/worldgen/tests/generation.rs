use std::collections::HashMap;

use sim::{
    CHUNK_SIZE, Cell, CellPos, ChunkPos, ChunkSource, Element, ElementId, ElementTable, Phase,
    Simulation, WORLD_BOTTOM_Y, WORLD_TOP_Y,
};
use worldgen::Generator;

const SHIPPED_ELEMENTS: &str = include_str!("../../../assets/elements.ron");

/// The world's Chunk layers, from the bottom one to just above the top one.
const BOTTOM_CHUNK_Y: i32 = WORLD_BOTTOM_Y / CHUNK_SIZE as i32;
const TOP_CHUNK_Y: i32 = WORLD_TOP_Y / CHUNK_SIZE as i32;

fn shipped_elements() -> ElementTable {
    ElementTable::from_ron(SHIPPED_ELEMENTS).expect("the shipped Elements file is valid")
}

fn generator(seed: u64) -> Generator {
    Generator::new(seed, &shipped_elements()).expect("the shipped Elements are enough")
}

/// The Mass of each Element over the Active Region.
fn grams_by_element(sim: &Simulation) -> HashMap<ElementId, u64> {
    let mut grams = HashMap::new();
    for chunk in sim.active_region().chunks() {
        for cell in sim.chunk_cells(chunk).unwrap_or_default() {
            *grams.entry(cell.element).or_default() += u64::from(cell.mass_grams);
        }
    }
    grams
}

/// Many Chunks to check every Cell of: every Chunk layer of the world, in a few columns spread
/// around the origin.
fn sample_chunks() -> Vec<ChunkPos> {
    let columns = [(0, 0), (-7, 11), (123, -45)];
    (BOTTOM_CHUNK_Y..TOP_CHUNK_Y)
        .flat_map(|y| columns.map(|(x, z)| ChunkPos::new(x, y, z)))
        .collect()
}

/// Chunks around the surface, where the terrain depends on the seed.
const TERRAIN_CHUNKS: [ChunkPos; 3] = [
    ChunkPos::new(0, -1, 0),
    ChunkPos::new(3, 0, -2),
    ChunkPos::new(-5, -3, 7),
];

#[test]
fn the_same_seed_and_chunk_give_identical_cells_and_another_seed_gives_different_ones() {
    for chunk in TERRAIN_CHUNKS {
        let first = generator(1).generate(chunk);
        let again = generator(1).generate(chunk);
        let other_seed = generator(2).generate(chunk);

        // Compared without `assert_eq!`, which would print all 32,768 Cells.
        assert!(first == again, "seed 1 gave different Cells for {chunk:?}");
        assert!(
            first != other_seed,
            "seeds 1 and 2 gave the same Cells for {chunk:?}"
        );
    }
}

#[test]
fn generating_chunks_in_a_different_order_gives_identical_cells() {
    // A 3 × 3 × 3 block of Chunks around the surface, plus one far away.
    let mut chunks: Vec<ChunkPos> = (-1..=1)
        .flat_map(|x| (-2..=0).flat_map(move |y| (-1..=1).map(move |z| ChunkPos::new(x, y, z))))
        .collect();
    chunks.push(ChunkPos::new(40, -20, -13));

    let forwards = generator(7);
    let in_order: HashMap<ChunkPos, Vec<Cell>> = chunks
        .iter()
        .map(|&chunk| (chunk, forwards.generate(chunk)))
        .collect();
    let backwards = generator(7);
    let reversed: HashMap<ChunkPos, Vec<Cell>> = chunks
        .iter()
        .rev()
        .map(|&chunk| (chunk, backwards.generate(chunk)))
        .collect();

    for chunk in &chunks {
        assert!(
            in_order[chunk] == reversed[chunk],
            "{chunk:?} came out differently when generated in reverse order"
        );
    }
    // The same generator again, after all the others: still the same Cells.
    assert!(forwards.generate(chunks[0]) == in_order[&chunks[0]]);
}

#[test]
fn a_chunk_well_above_the_surface_is_all_oxygen_at_atmosphere_mass_and_temperature() {
    let elements = shipped_elements();
    let oxygen = elements.id_of("Oxygen").unwrap();
    let generator = generator(3);
    // The surface stays within 40 Cells of y = 0, so these Chunks, from y = 96 up to the top
    // layer at y = 255, are all sky.
    for chunk in [
        ChunkPos::new(0, 3, 0),
        ChunkPos::new(-9, 5, 4),
        ChunkPos::new(2, 7, -6),
    ] {
        for cell in generator.generate(chunk) {
            assert_eq!(cell.element, oxygen, "{chunk:?} holds {cell:?}");
            // Oxygen's Density is 1.43 kg/m³, so a 1 m³ Cell holds 1.43 kg = 1,430 g.
            assert_eq!(cell.mass_grams, 1_430, "{chunk:?} holds {cell:?}");
            // The Atmosphere is generated at 293 K; the tolerance only allows for rounding.
            assert!(
                (cell.temperature_k - 293.0).abs() < 0.01,
                "{chunk:?} holds {cell:?}"
            );
        }
    }
}

#[test]
fn going_down_the_average_temperature_of_solid_cells_rises_with_every_chunk_layer() {
    let elements = shipped_elements();
    let generator = generator(4);
    let is_solid = |cell: &Cell| elements.get(cell.element).phase == Phase::Solid;

    // A 2 × 2 column of Chunks, from the top of the world to the bottom.
    let mut layers = Vec::new();
    for y in (BOTTOM_CHUNK_Y..TOP_CHUNK_Y).rev() {
        let (mut sum_k, mut count) = (0.0_f64, 0_u32);
        for (x, z) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            for cell in generator.generate(ChunkPos::new(x, y, z)) {
                if is_solid(&cell) {
                    sum_k += f64::from(cell.temperature_k);
                    count += 1;
                }
            }
        }
        // Layers of sky, and the Magma floor, hold no solid Cells.
        if count > 0 {
            layers.push((y, sum_k / f64::from(count)));
        }
    }

    // The ground spans 1024 Cells below y = 0 (32 layers), less the floor layer, plus whatever
    // the hills above y = 0 add.
    assert!(
        layers.len() >= 31,
        "only {} layers hold solids",
        layers.len()
    );
    for pair in layers.windows(2) {
        let [(upper_y, upper_k), (lower_y, lower_k)] = pair else {
            unreachable!()
        };
        assert!(
            lower_k > upper_k,
            "solids average {lower_k} K in layer {lower_y}, but {upper_k} K in layer {upper_y} above it"
        );
    }
}

#[test]
fn the_bottom_chunk_layer_is_magma() {
    let elements = shipped_elements();
    let magma = elements.id_of("Magma").unwrap();
    let generator = generator(5);
    for (x, z) in [(0, 0), (-1, 3), (17, -40), (-200, -9)] {
        let chunk = ChunkPos::new(x, BOTTOM_CHUNK_Y, z);
        for cell in generator.generate(chunk) {
            assert_eq!(cell.element, magma, "{chunk:?} holds {cell:?}");
        }
    }
}

#[test]
fn every_generated_cell_lies_within_its_elements_phase_transition_range() {
    let elements = shipped_elements();
    let generator = generator(6);
    for chunk in sample_chunks() {
        for cell in generator.generate(chunk) {
            let element = elements.get(cell.element);
            if let Some(low) = &element.low {
                assert!(
                    cell.temperature_k >= low.temperature_k,
                    "{chunk:?} holds {} at {} K, below its low threshold of {} K",
                    element.name,
                    cell.temperature_k,
                    low.temperature_k
                );
            }
            if let Some(high) = &element.high {
                assert!(
                    cell.temperature_k <= high.temperature_k,
                    "{chunk:?} holds {} at {} K, above its high threshold of {} K",
                    element.name,
                    cell.temperature_k,
                    high.temperature_k
                );
            }
        }
    }
}

#[test]
fn generated_cells_keep_the_invariants() {
    let elements = shipped_elements();
    let generator = generator(7);
    for chunk in sample_chunks() {
        for cell in generator.generate(chunk) {
            let element = elements.get(cell.element);
            match element.phase {
                Phase::Vacuum => assert_eq!(cell.mass_grams, 0, "{chunk:?}: {cell:?}"),
                _ => assert!(cell.mass_grams >= 1, "{chunk:?}: {cell:?} holds no Mass"),
            }
            if element.phase == Phase::Liquid {
                // A liquid's Capacity is its Density × 1 m³: kg/m³ × 1000 g/kg, so 1,000,000 g
                // for Water at 1000 kg/m³ and 2,600,000 g for Magma at 2600 kg/m³.
                let capacity_grams = (f64::from(element.density) * 1_000.0).round() as u32;
                assert!(
                    cell.mass_grams <= capacity_grams,
                    "{chunk:?}: {cell:?} holds more than {}'s Capacity of {capacity_grams} g",
                    element.name
                );
            }
        }
    }
}

#[test]
fn the_first_tick_of_a_freshly_generated_fully_active_region_makes_no_phase_transition() {
    let elements = shipped_elements();
    // Near the surface; around the depth where Water pockets give way to Steam (about 70 Cells
    // down); and on the floor, where Granite gives way to Magma.
    for focus in [
        CellPos::new(0, 0, 0),
        CellPos::new(40, -64, -25),
        CellPos::new(-30, WORLD_BOTTOM_Y + 24, 10),
    ] {
        let mut sim = Simulation::with_source(elements.clone(), Box::new(generator(8)));
        sim.focus_on(focus);
        let stats = sim.stats();
        // The default Active Region is 4 × 2 × 4 Chunks, all freshly generated and awake.
        assert_eq!(stats.active_chunks, 32, "around {focus:?}");
        assert_eq!(stats.awake_chunks, 32, "around {focus:?}");
        let before = grams_by_element(&sim);

        sim.tick();

        // Flows move Mass between Cells but keep each Element's total; only a Phase Transition
        // turns one Element's Mass into another's. No gas reaches the ceiling from here.
        assert_eq!(
            grams_by_element(&sim),
            before,
            "around {focus:?}, the first Tick changed some Element's total Mass"
        );
        assert_eq!(sim.stats().mass_lost_to_space_grams, 0);
    }
}

// Added during implementation, at the same seam.

#[test]
fn underground_there_are_oxygen_caves_and_pockets_of_water_steam_carbon_dioxide_and_hydrogen() {
    let elements = shipped_elements();
    let generator = generator(9);
    let mut grams_by_element: HashMap<&str, u64> = HashMap::new();
    // Below y = -40, the lowest the surface reaches, down to just above the floor.
    for y in (BOTTOM_CHUNK_Y + 1)..-2 {
        for (x, z) in [(0, 0), (5, -3), (-11, 8)] {
            for cell in generator.generate(ChunkPos::new(x, y, z)) {
                *grams_by_element
                    .entry(&elements.get(cell.element).name)
                    .or_default() += u64::from(cell.mass_grams);
            }
        }
    }
    // Water deeper than about 70 Cells is past its boiling point, 373.15 K, so it comes out as
    // Steam; shallower Water stays Water.
    for name in ["Oxygen", "Water", "Steam", "Carbon Dioxide", "Hydrogen"] {
        assert!(
            grams_by_element.get(name).is_some_and(|&grams| grams > 0),
            "no {name} underground: {grams_by_element:?}"
        );
    }
}

#[test]
fn a_generator_names_every_element_the_layout_needs_that_the_table_lacks() {
    let mut elements = ElementTable::new();
    for name in ["Oxygen", "Dirt", "Granite", "Water"] {
        elements.add(Element::new(name).density(1_000.0).specific_heat(1.0));
    }

    let Err(missing) = Generator::new(1, &elements) else {
        panic!("built a generator without Magma, Carbon Dioxide or Hydrogen");
    };

    let mut names = missing.names.clone();
    names.sort_unstable();
    assert_eq!(names, ["Carbon Dioxide", "Hydrogen", "Magma"]);
    let message = missing.to_string();
    for name in names {
        assert!(message.contains(name), "{message:?} doesn't name {name}");
    }
}
