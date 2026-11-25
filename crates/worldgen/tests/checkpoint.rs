//! A checkpoint of a generated world (`docs/plans/16-simulation-checkpoint.md`), restored with a
//! generator rebuilt from the World Seed and the Elements it was first built with.

mod common;

use sim::{Cell, CellPos, ChunkContents, ChunkPos, Edit, ElementTable, SimStats, Simulation};
use worldgen::Generator;

/// The shipped Elements with Oxygen's Density doubled, as a hot reload might change it. A
/// generator built from these would fill the sky with twice the Mass.
fn reloaded_with_denser_oxygen(elements: &ElementTable) -> ElementTable {
    let mut reloaded = ElementTable::new();
    for (_, element) in elements.iter().skip(1) {
        let mut element = element.clone();
        if element.name == "Oxygen" {
            element.density *= 2.0;
        }
        reloaded.add(element);
    }
    reloaded
}

/// The parts of `stats` that hold for the Active Region alone. A restored simulation hasn't
/// loaded the untouched Dormant Chunks yet, so it counts fewer of them, and less total Mass.
fn active_stats(stats: SimStats) -> (u64, usize, usize, usize, u64) {
    (
        stats.tick_count,
        stats.active_chunks,
        stats.awake_chunks,
        stats.settled_chunks,
        stats.mass_lost_to_space_grams,
    )
}

#[test]
fn a_generated_world_restored_with_its_seed_and_startup_elements_carries_on_exactly() {
    let startup = common::shipped_elements();
    // High in the sky, every Chunk is Settled and untouched (see `at_rest`).
    let mut sim = common::at_rest(&startup);
    let reloaded = reloaded_with_denser_oxygen(&startup);
    sim.replace_elements(reloaded.clone())
        .expect("only a property changed");
    let carbon_dioxide = startup.id_of("Carbon Dioxide").unwrap();
    // A release of gas in Chunk (0, 4, 0), which spans y = 128 to 159.
    let release = CellPos::new(5, 150, 5);
    sim.apply(Edit::Place {
        pos: release,
        cell: Cell {
            element: carbon_dioxide,
            mass_grams: 20_000,
            temperature_k: 350.0,
        },
    });
    // At x = 17 the box stays put, though another would be better centred (see `region.rs`).
    let region = sim.active_region();
    sim.focus_on(CellPos::new(17, 160, 0));
    assert_eq!(sim.active_region(), region);
    for _ in 0..3 {
        sim.tick();
    }
    // A Chunk loaded for render distance and never changed: Dormant.
    let far = ChunkPos::new(5, 0, 5);
    sim.load(far);
    assert!(sim.chunk_cells(far).is_some());

    let checkpoint = sim.checkpoint();

    // Within 3 Ticks, the gas and its heat stay within a few Cells of the release, all inside
    // its Chunk. Every other Chunk of the Active Region is untouched sky.
    let released_in = ChunkPos::of(release);
    for (&chunk, listed) in &checkpoint.chunks {
        let has_cells = matches!(listed.contents, ChunkContents::Cells(_));
        assert_eq!(has_cells, chunk == released_in, "{chunk:?}");
        assert!(region.contains(chunk), "{chunk:?} isn't active");
    }
    assert_eq!(checkpoint.chunks.len(), 32);
    assert!(sim.stats().settled_chunks > 0, "{:?}", sim.stats());

    let generator = Generator::new(common::SEED, &startup).expect("the startup Elements");
    let mut restored = Simulation::restore(
        sim.elements().clone(),
        sim.settings().clone(),
        Box::new(generator),
        checkpoint,
    )
    .expect("restored");

    assert_eq!(active_stats(restored.stats()), active_stats(sim.stats()));
    // The untouched Chunk comes back from the seed when it loads.
    restored.load(far);
    assert!(restored.chunk_cells(far) == sim.chunk_cells(far));
    for tick in 0..3 {
        sim.tick();
        restored.tick();
        assert_eq!(
            active_stats(restored.stats()),
            active_stats(sim.stats()),
            "tick {tick}"
        );
        for chunk in region.chunks() {
            // Compared without `assert_eq!`, which would print all 32,768 Cells.
            assert!(
                restored.chunk_cells(chunk) == sim.chunk_cells(chunk),
                "tick {tick}: {chunk:?} differs"
            );
        }
    }
    // The sky the startup Elements generate, not the reloaded ones: 1.43 kg/m³ × 1 m³.
    let sky = restored.get(CellPos::new(-40, 180, -40));
    assert_eq!(sky.element, startup.id_of("Oxygen").unwrap());
    assert_eq!(sky.mass_grams, 1_430);
}
