mod common;

use sim::{ElementId, Simulation};

/// Checks the simulation invariants over every loaded Cell after a Tick: Vacuum holds no Mass,
/// every other Cell holds at least 1 g, and Mass is only ever lost through the world's ceiling.
fn check_invariants(sim: &Simulation, start_grams: u64, tick: usize) {
    for chunk in sim.active_region().chunks() {
        for cell in sim.chunk_cells(chunk).unwrap_or_default() {
            if cell.element == ElementId::VACUUM {
                assert_eq!(
                    cell.mass_grams, 0,
                    "tick {tick}: Vacuum in {chunk:?} holds Mass"
                );
            } else {
                assert!(
                    cell.mass_grams >= 1,
                    "tick {tick}: {cell:?} in {chunk:?} holds no Mass"
                );
            }
        }
    }
    let stats = sim.stats();
    assert_eq!(
        stats.total_mass_grams + stats.mass_lost_to_space_grams,
        start_grams,
        "tick {tick}: Mass changed"
    );
}

#[test]
fn the_busy_scenario_gives_identical_cells_with_one_thread_and_with_four() {
    let elements = common::shipped_elements();
    let mut one = common::busy(&elements);
    one.set_threads(1);
    let mut four = common::busy(&elements);
    four.set_threads(4);
    let start_grams = one.total_mass_grams();

    for tick in 0..20 {
        one.tick();
        four.tick();
        check_invariants(&one, start_grams, tick);
        check_invariants(&four, start_grams, tick);
    }

    // Most Chunks are still awake, so the threads really did split the work.
    assert!(one.stats().awake_chunks >= 24, "{:?}", one.stats());
    assert_eq!(one.stats(), four.stats());
    for chunk in one.active_region().chunks() {
        // Compared without `assert_eq!`, which would print all 32,768 Cells.
        assert!(
            one.chunk_cells(chunk) == four.chunk_cells(chunk),
            "{chunk:?} differs between one thread and four"
        );
    }
}
