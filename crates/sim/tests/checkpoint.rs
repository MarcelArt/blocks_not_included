//! Checkpoints (`docs/plans/16-simulation-checkpoint.md`): capturing the simulation between Ticks
//! and restoring it, so it carries on exactly as it would have.

mod common;

use common::*;
use sim::{
    CHUNK_SIZE, Cell, CellPos, ChunkContents, ChunkPos, ChunkSource, Edit, Element, ElementId,
    ElementTable, SimSettings, Simulation,
};

const CHUNK_VOLUME: usize = CHUNK_SIZE * CHUNK_SIZE * CHUNK_SIZE;

/// Horizontal layers of one Cell each: a Cell is that of the highest layer starting at or below
/// its y, or Vacuum below them all. The layers are listed from the bottom up.
#[derive(Clone)]
struct LayeredSource(Vec<(i32, Cell)>);

impl ChunkSource for LayeredSource {
    fn generate(&self, chunk: ChunkPos) -> Vec<Cell> {
        (0..CHUNK_VOLUME)
            .map(|i| {
                // Cells are listed x fastest, then z, then y.
                let y = chunk.origin().y + (i / (CHUNK_SIZE * CHUNK_SIZE)) as i32;
                self.0
                    .iter()
                    .rev()
                    .find(|&&(bottom_y, _)| bottom_y <= y)
                    .map_or(Cell::VACUUM, |&(_, cell)| cell)
            })
            .collect()
    }
}

/// A solid that conducts heat well: 1000 g of it holds 1000 J/K.
fn test_rock() -> Element {
    Element::new("TestRock")
        .density(1_000.0)
        .specific_heat(1.0)
        .conductivity(100.0)
}

/// TestRock, then TestGas.
fn rock_and_gas() -> (ElementTable, ElementId, ElementId) {
    let mut elements = ElementTable::new();
    let rock = elements.add(test_rock());
    let gas = elements.add(test_gas("TestGas", 1.0));
    (elements, rock, gas)
}

/// Rock at 300 K below y = 0, under Vacuum.
fn rock_below_zero(rock: ElementId) -> LayeredSource {
    LayeredSource(vec![
        (i32::MIN, cell(rock, 1_000, 300.0)),
        (0, Cell::VACUUM),
    ])
}

/// Restores `checkpoint` with the defaults' settings, failing the test if it is rejected.
fn restore(
    elements: &ElementTable,
    source: &LayeredSource,
    checkpoint: sim::Checkpoint,
) -> Simulation {
    Simulation::restore(
        elements.clone(),
        SimSettings::default(),
        Box::new(source.clone()),
        checkpoint,
    )
    .expect("the checkpoint is restored")
}

/// Whether the Cells of the Chunk at `chunk` are the same in `a` and `b`. Compared without
/// `assert_eq!`, which would print all 32,768 Cells.
fn same_chunk(a: &Simulation, b: &Simulation, chunk: ChunkPos) -> bool {
    a.chunk_cells(chunk) == b.chunk_cells(chunk)
}

// The default Active Region is 4 × 2 × 4 Chunks, best centred on the default focus at the
// origin: Chunks −2 to 1 along x and z, and −1 to 0 along y.

#[test]
fn cells_changed_by_an_edit_and_by_a_tick_round_trip_including_a_dormant_chunk() {
    let (elements, rock, _) = rock_and_gas();
    let source = rock_below_zero(rock);
    let mut sim = Simulation::with_source(elements.clone(), Box::new(source.clone()));
    // A hot spot in the ground, which the Ticks spread to its neighbours.
    let (spot, neighbour) = (CellPos::new(5, -5, 5), CellPos::new(6, -5, 5));
    sim.apply(Edit::ShiftTemperature {
        pos: spot,
        kelvin: 700.0,
    });
    // A dug Cell at x = 1000, in Chunk 31 along x: far beyond the Active Region, so Dormant.
    let far = CellPos::new(1_000, -5, 0);
    sim.apply(Edit::Dig { pos: far });
    let invariants = Invariants::record(&sim, vec![spot, neighbour, far]);
    run_ticks(&mut sim, 3, &invariants);
    let (spot_chunk, far_chunk) = (ChunkPos::of(spot), ChunkPos::of(far));
    assert!(!sim.active_region().contains(far_chunk));
    // 100 W/(m·K) × 0.2 s × 700 K = 14,000 J into 1000 J/K: the first Tick alone warms the
    // neighbour by 14 K.
    assert!(sim.get(neighbour).temperature_k > 310.0);

    let before_draining = sim.checkpoint();
    sim.take_changed_chunks();
    let checkpoint = sim.checkpoint();

    // Draining the changes the game draws from leaves the checkpoint as it was.
    assert!(checkpoint == before_draining);
    for chunk in [spot_chunk, far_chunk] {
        let contents = &checkpoint.chunks[&chunk].contents;
        assert!(
            matches!(contents, ChunkContents::Cells(cells) if cells.len() == CHUNK_VOLUME),
            "{chunk:?} has no Cells"
        );
    }
    let restored = restore(&elements, &source, checkpoint.clone());
    for chunk in [spot_chunk, far_chunk] {
        assert!(same_chunk(&sim, &restored, chunk), "{chunk:?} differs");
    }
    for pos in [spot, neighbour, far] {
        assert_eq!(restored.get(pos), sim.get(pos));
    }
    assert_eq!(restored.get(far), Cell::VACUUM);
    assert!(!restored.active_region().contains(far_chunk));
    // A restored simulation captures the same checkpoint again.
    assert!(restored.checkpoint() == checkpoint);
}

#[test]
fn a_chunk_dug_out_entirely_stays_vacuum_after_a_restore_and_untouched_chunks_hold_no_cells() {
    let (elements, rock, _) = rock_and_gas();
    let source = rock_below_zero(rock);
    let mut sim = Simulation::with_source(elements.clone(), Box::new(source.clone()));
    // Dig out every Cell of Chunk (0, −1, 0), which the source fills with rock: x and z from 0
    // to 31, and y from −32 to −1.
    let dug = ChunkPos::new(0, -1, 0);
    for pos in cuboid(dug.origin(), CellPos::new(31, -1, 31)) {
        sim.apply(Edit::Dig { pos });
    }
    sim.tick();
    // The box best centred on x = 1000 is Chunks 29 to 32 along x, so the dug Chunk is Dormant.
    let away = CellPos::new(1_000, 0, 0);
    sim.focus_on(away);
    assert!(!sim.active_region().contains(dug));

    let checkpoint = sim.checkpoint();

    assert_eq!(checkpoint.chunks[&dug].contents, ChunkContents::Vacuum);
    // Every other Chunk listed is in the Active Region, untouched, so it holds no Cells. The
    // untouched rock Dormant beside the dug Chunk isn't listed at all.
    for (&chunk, listed) in &checkpoint.chunks {
        if chunk != dug {
            assert!(sim.active_region().contains(chunk), "{chunk:?}");
            assert_eq!(listed.contents, ChunkContents::Generated, "{chunk:?}");
        }
    }
    // Along x, the 4 Chunks 29 to 32, along z, the 4 Chunks −2 to 1, and along y, the rock
    // layer −1: the Vacuum layer 0 isn't stored.
    assert_eq!(checkpoint.chunks.len(), 4 * 4 + 1);
    let beside = ChunkPos::new(1, -1, 0);
    assert!(!checkpoint.chunks.contains_key(&beside));

    // Back at the origin, the dug Chunk is active again, and still Vacuum, in both.
    let mut restored = restore(&elements, &source, checkpoint);
    for sim in [&mut sim, &mut restored] {
        sim.focus_on(CellPos::new(0, 0, 0));
        sim.tick();
        let cells = sim.chunk_cells(dug).expect("stored");
        assert!(cells.iter().all(|&cell| cell == Cell::VACUUM));
    }
    // The rock beside it came back from the source.
    assert_eq!(restored.get(beside.origin()), cell(rock, 1_000, 300.0));
    assert!(same_chunk(&sim, &restored, beside));
}

/// Checks after a Tick that Mass has been lost only through the world's ceiling.
fn check_mass(sim: &Simulation, start_grams: u64, tick: u64) {
    let stats = sim.stats();
    assert_eq!(
        stats.total_mass_grams + stats.mass_lost_to_space_grams,
        start_grams,
        "tick {tick}: Mass changed"
    );
}

#[test]
fn a_restored_simulation_carries_on_exactly_as_uninterrupted_play_after_the_focus_wandered() {
    let (elements, rock, gas) = rock_and_gas();
    // Rock in Chunk layer 6 (y = 192 to 223) under gas in the world's top Chunk layer, 7 (y =
    // 224 to 255), where it escapes to space. Below y = 192, Vacuum.
    let source = LayeredSource(vec![
        (192, cell(rock, 1_000, 300.0)),
        (224, cell(gas, 1_000, 300.0)),
    ]);
    let mut sim = Simulation::with_source(elements.clone(), Box::new(source.clone()));
    // The box best centred on y = 224 is Chunk layers 6 to 7.
    sim.focus_on(CellPos::new(0, 224, 0));
    // A hot spot in the rock, and a release of gas, which spreads in ways the Tick count breaks
    // ties for.
    sim.apply(Edit::ShiftTemperature {
        pos: CellPos::new(5, 200, 5),
        kelvin: 700.0,
    });
    sim.apply(Edit::Place {
        pos: CellPos::new(-20, 240, -20),
        cell: cell(gas, 50_000, 350.0),
    });
    let start_grams = sim.total_mass_grams();
    for tick in 0..5 {
        sim.tick();
        check_mass(&sim, start_grams, tick);
    }
    // Along x, Chunks −2 to 1 are the best-centred box for x = −16 to 15. From x = 16 on,
    // Chunks −1 to 2 would be better centred, but the box stays put up to x = 19 (see
    // `region.rs`). So the box isn't where the focus alone would put it.
    let region = sim.active_region();
    sim.focus_on(CellPos::new(17, 224, 0));
    assert_eq!(sim.active_region(), region);
    for tick in 5..7 {
        sim.tick();
        check_mass(&sim, start_grams, tick);
    }
    let stats = sim.stats();
    assert_eq!(stats.tick_count, 7);
    assert!(stats.settled_chunks > 0, "{stats:?}");
    assert!(stats.mass_lost_to_space_grams > 0, "{stats:?}");

    let mut restored = restore(&elements, &source, sim.checkpoint());

    assert_eq!(restored.active_region(), region);
    assert_eq!(restored.focus(), CellPos::new(17, 224, 0));
    assert_eq!(restored.stats(), stats);
    // The same commands for both: the focus wanders back within the box's margin, gas is
    // released, and Ticks run.
    for sim in [&mut sim, &mut restored] {
        sim.focus_on(CellPos::new(12, 224, 0));
        sim.apply(Edit::Place {
            pos: CellPos::new(30, 250, 30),
            cell: cell(gas, 20_000, 400.0),
        });
    }
    // Edits are the named exception to Mass conservation, so count again from here.
    let start_grams = sim.stats().total_mass_grams + sim.stats().mass_lost_to_space_grams;
    for tick in 7..10 {
        sim.tick();
        restored.tick();
        check_mass(&sim, start_grams, tick);
        assert_eq!(restored.active_region(), region, "tick {tick}");
        assert_eq!(restored.stats(), sim.stats(), "tick {tick}");
        for chunk in region.chunks() {
            assert!(
                same_chunk(&sim, &restored, chunk),
                "tick {tick}: {chunk:?} differs"
            );
        }
    }
}

/// `rock_and_gas` with TestRock's conductivity changed and a TestSand appended.
fn reloaded_elements(rock_conductivity: f32) -> (ElementTable, ElementId) {
    let (mut elements, _, _) = rock_and_gas();
    let mut changed = ElementTable::new();
    for (id, element) in elements.iter().skip(1) {
        let element = element.clone();
        let element = if id.index() == 1 {
            element.conductivity(rock_conductivity)
        } else {
            element
        };
        changed.add(element);
    }
    elements = changed;
    let sand = elements.add(Element::new("TestSand").density(1_500.0).specific_heat(1.0));
    (elements, sand)
}

#[test]
fn a_checkpoint_after_reloads_restores_with_the_last_accepted_rules_and_the_same_source() {
    let (startup, rock, gas) = rock_and_gas();
    let source = rock_below_zero(rock);
    let mut sim = Simulation::with_source(startup.clone(), Box::new(source.clone()));
    let (reloaded, sand) = reloaded_elements(10.0);
    assert_eq!(sim.replace_elements(reloaded), Ok(()));
    // Renaming TestGas is rejected: Cells may hold it.
    let (mut renamed, _) = reloaded_elements(10.0);
    renamed = {
        let mut table = ElementTable::new();
        for (id, element) in renamed.iter().skip(1) {
            let mut element = element.clone();
            if id == gas {
                element.name = "TestSteam".into();
            }
            table.add(element);
        }
        table
    };
    assert!(sim.replace_elements(renamed).is_err());
    let accepted = SimSettings {
        conduction_multiplier: 10.0,
        ..SimSettings::default()
    };
    assert_eq!(sim.set_settings(accepted.clone()), Ok(()));
    let overshooting = SimSettings {
        gas_evening_out_fraction: 0.9,
        ..SimSettings::default()
    };
    assert!(sim.set_settings(overshooting).is_err());
    // A Cell of the appended Element, and a hot spot in the rock.
    let (sand_pos, spot) = (CellPos::new(3, 0, 3), CellPos::new(5, -5, 5));
    sim.apply(Edit::Place {
        pos: sand_pos,
        cell: cell(sand, 1_500_000, 300.0),
    });
    sim.apply(Edit::ShiftTemperature {
        pos: spot,
        kelvin: 700.0,
    });
    sim.tick();

    let checkpoint = sim.checkpoint();

    assert_eq!(
        checkpoint.element_names,
        ["Vacuum", "TestRock", "TestGas", "TestSand"]
    );
    // The table the simulation started with lacks TestSand, which a Cell holds.
    let with_startup = Simulation::restore(
        startup,
        accepted.clone(),
        Box::new(source.clone()),
        checkpoint.clone(),
    );
    assert_eq!(
        with_startup.err(),
        Some(sim::CheckpointError::Elements(
            sim::ElementError::MissingElement {
                element: "TestSand".into()
            }
        ))
    );
    let mut restored = Simulation::restore(
        sim.elements().clone(),
        sim.settings().clone(),
        Box::new(source.clone()),
        checkpoint,
    )
    .expect("restored with the accepted rules");
    assert_eq!(restored.settings(), &accepted);
    assert_eq!(restored.elements().get(rock).conductivity, 10.0);
    assert_eq!(restored.elements().id_of("TestSand"), Some(sand));
    assert_eq!(restored.get(sand_pos), sim.get(sand_pos));
    // The hot spot cools under the accepted conductivity and multiplier, in both.
    for tick in 0..3 {
        sim.tick();
        restored.tick();
        for chunk in sim.active_region().chunks() {
            assert!(
                same_chunk(&sim, &restored, chunk),
                "tick {tick}: {chunk:?} differs"
            );
        }
    }
}

#[test]
fn a_malformed_checkpoint_is_rejected_before_anything_is_restored() {
    use sim::CheckpointError as E;

    let (elements, rock, _) = rock_and_gas();
    let source = rock_below_zero(rock);
    let mut sim = Simulation::with_source(elements.clone(), Box::new(source.clone()));
    // A Cell changed in Chunk (0, −1, 0), and one in the Dormant Chunk (31, −1, 0).
    let (near, far) = (CellPos::new(5, -5, 5), CellPos::new(1_000, -5, 0));
    let (near_chunk, far_chunk) = (ChunkPos::of(near), ChunkPos::of(far));
    sim.apply(Edit::ShiftTemperature {
        pos: near,
        kelvin: 100.0,
    });
    sim.apply(Edit::Dig { pos: far });
    sim.tick();
    let valid = sim.checkpoint();
    let mut bigger = elements.clone();
    let unknown = bigger.add(Element::new("TestUnknown"));
    // The first Cell of a Chunk's Cells is at its origin.
    let first_cell_becomes = |checkpoint: &mut sim::Checkpoint, chunk: ChunkPos, cell: Cell| {
        let ChunkContents::Cells(cells) = &mut checkpoint.chunks.get_mut(&chunk).unwrap().contents
        else {
            panic!("{chunk:?} holds no Cells");
        };
        cells[0] = cell;
    };
    type Change<'a> = Box<dyn Fn(&mut sim::Checkpoint, &mut SimSettings) + 'a>;
    let cases: Vec<(&str, Change<'_>, E)> = vec![
        (
            "a renamed Element",
            Box::new(|c, _| c.element_names[1] = "Basalt".into()),
            E::Elements(sim::ElementError::MissingElement {
                element: "Basalt".into(),
            }),
        ),
        (
            "Elements in another order",
            Box::new(|c, _| c.element_names.swap(1, 2)),
            E::Elements(sim::ElementError::ElementMoved {
                element: "TestGas".into(),
                from: 1,
                to: 2,
            }),
        ),
        (
            "invalid settings",
            Box::new(|_, s| s.gas_evening_out_fraction = 0.9),
            E::Settings(
                SimSettings {
                    gas_evening_out_fraction: 0.9,
                    ..SimSettings::default()
                }
                .validate()
                .unwrap_err(),
            ),
        ),
        (
            "a box of another size",
            Box::new(|c, _| c.placement_chunks = [4, 2, 5]),
            E::RegionSize {
                checkpoint: [4, 2, 5],
                settings: [4, 2, 4],
            },
        ),
        (
            "a box the focus is far from",
            Box::new(|c, _| c.focus = CellPos::new(1_000, 0, 0)),
            E::RegionAwayFromFocus {
                focus: CellPos::new(1_000, 0, 0),
                placement_min: ChunkPos::new(-2, -1, -2),
            },
        ),
        (
            "a Chunk above the world",
            Box::new(|c, _| {
                let listed = c.chunks[&near_chunk].clone();
                c.chunks.insert(ChunkPos::new(0, 8, 0), listed);
            }),
            E::ChunkOutsideWorld {
                chunk: ChunkPos::new(0, 8, 0),
            },
        ),
        (
            "too few Cells",
            Box::new(|c, _| {
                c.chunks.get_mut(&far_chunk).unwrap().contents =
                    ChunkContents::Cells(vec![Cell::VACUUM; 10]);
            }),
            E::CellCount {
                chunk: far_chunk,
                cells: 10,
            },
        ),
        (
            "a Cell of an unnamed Element",
            Box::new(move |c, _| first_cell_becomes(c, far_chunk, cell(unknown, 1_000, 300.0))),
            E::UnknownElement {
                pos: far_chunk.origin(),
                element: 3,
            },
        ),
        (
            "Vacuum with Mass",
            Box::new(move |c, _| {
                let vacuum = Cell {
                    mass_grams: 5,
                    ..Cell::VACUUM
                };
                first_cell_becomes(c, near_chunk, vacuum);
            }),
            E::InvalidMass {
                pos: near_chunk.origin(),
                cell: Cell {
                    mass_grams: 5,
                    ..Cell::VACUUM
                },
            },
        ),
        (
            "rock without Mass",
            Box::new(move |c, _| first_cell_becomes(c, near_chunk, cell(rock, 0, 300.0))),
            E::InvalidMass {
                pos: near_chunk.origin(),
                cell: cell(rock, 0, 300.0),
            },
        ),
        (
            "a Settled Dormant Chunk",
            Box::new(|c, _| c.chunks.get_mut(&far_chunk).unwrap().settled = true),
            E::SettledOutsideActiveRegion { chunk: far_chunk },
        ),
        (
            "an active Chunk of rock left out",
            Box::new(|c, _| {
                c.chunks.remove(&ChunkPos::new(1, -1, 1));
            }),
            E::MissingChunk {
                chunk: ChunkPos::new(1, -1, 1),
            },
        ),
    ];

    for (case, change, expected) in cases {
        let (mut checkpoint, mut settings) = (valid.clone(), SimSettings::default());
        change(&mut checkpoint, &mut settings);
        let restored = Simulation::restore(
            elements.clone(),
            settings,
            Box::new(source.clone()),
            checkpoint,
        );
        assert_eq!(restored.err(), Some(expected), "{case}");
    }
    // Unchanged, it is restored.
    let restored = restore(&elements, &source, valid);
    assert_eq!(restored.get(far), Cell::VACUUM);
}

#[test]
fn a_chunk_whose_cells_come_back_to_exactly_what_was_generated_holds_no_cells() {
    let (elements, rock, gas) = rock_and_gas();
    let source = rock_below_zero(rock);
    let mut sim = Simulation::with_source(elements.clone(), Box::new(source.clone()));
    // One Cell of the active Chunk (0, −1, 0) and one of the Dormant Chunk (31, −1, 0) change,
    // and change back.
    let generated = cell(rock, 1_000, 300.0);
    let (near, far) = (CellPos::new(5, -5, 5), CellPos::new(1_000, -5, 0));
    for pos in [near, far] {
        sim.set(pos, cell(gas, 1_000, 300.0));
    }
    let changed = sim.checkpoint();
    for pos in [near, far] {
        sim.set(pos, generated);
    }

    let checkpoint = sim.checkpoint();

    let (near_chunk, far_chunk) = (ChunkPos::of(near), ChunkPos::of(far));
    assert!(matches!(
        changed.chunks[&near_chunk].contents,
        ChunkContents::Cells(_)
    ));
    assert!(changed.chunks.contains_key(&far_chunk));
    assert_eq!(
        checkpoint.chunks[&near_chunk].contents,
        ChunkContents::Generated
    );
    assert!(!checkpoint.chunks.contains_key(&far_chunk));
    // Within 0.0001 K is not exactly.
    sim.set(near, cell(rock, 1_000, 300.0001));
    assert!(matches!(
        sim.checkpoint().chunks[&near_chunk].contents,
        ChunkContents::Cells(_)
    ));
}
