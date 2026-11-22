mod common;

use std::sync::{Arc, Mutex};

use common::*;
use sim::{
    CHUNK_SIZE, Cell, CellPos, ChunkPos, ChunkSource, Edit, Element, ElementId, ElementTable,
    SimSettings, Simulation, WORLD_BOTTOM_Y, WORLD_TOP_Y,
};

const CHUNK_VOLUME: usize = CHUNK_SIZE * CHUNK_SIZE * CHUNK_SIZE;

/// A source that fills every Chunk with a marker solid, and records which Chunks it generated.
struct MarkerSource {
    marker: ElementId,
    generated: Arc<Mutex<Vec<ChunkPos>>>,
}

impl MarkerSource {
    /// The source, and the list of Chunks it generates, in order.
    fn new(marker: ElementId) -> (Box<Self>, Arc<Mutex<Vec<ChunkPos>>>) {
        let generated = Arc::new(Mutex::new(Vec::new()));
        let source = MarkerSource {
            marker,
            generated: Arc::clone(&generated),
        };
        (Box::new(source), generated)
    }
}

impl ChunkSource for MarkerSource {
    fn generate(&self, chunk: ChunkPos) -> Vec<Cell> {
        self.generated.lock().unwrap().push(chunk);
        vec![marker_cell(self.marker, chunk); CHUNK_VOLUME]
    }
}

/// What a [`MarkerSource`] fills the Chunk at `chunk` with. Its Mass tells nearby Chunks apart.
fn marker_cell(marker: ElementId, chunk: ChunkPos) -> Cell {
    let digits =
        chunk.x.rem_euclid(10) * 100 + chunk.y.rem_euclid(10) * 10 + chunk.z.rem_euclid(10);
    cell(marker, 1_000 + digits as u32, 300.0)
}

/// A solid that conducts no heat, so a Chunk full of it never changes.
fn test_marker() -> Element {
    Element::new("TestMarker")
        .density(1_000.0)
        .specific_heat(1.0)
}

fn generated(log: &Arc<Mutex<Vec<ChunkPos>>>) -> Vec<ChunkPos> {
    log.lock().unwrap().clone()
}

/// A solid that conducts heat well: 1000 g of it holds 1000 J/K.
fn test_rock() -> Element {
    Element::new("TestRock")
        .density(1_000.0)
        .specific_heat(1.0)
        .conductivity(100.0)
}

// The default Active Region is 4 × 2 × 4 Chunks of 32 Cells, best centred on the default focus at
// the origin: Cells x and z from −64 to 63, and y from −32 to 31.

#[test]
fn a_hot_spot_in_a_dormant_chunk_keeps_its_exact_temperature_for_100_ticks() {
    let mut elements = ElementTable::new();
    let rock = elements.add(test_rock());
    let mut sim = Simulation::new(elements);
    // The same hot spot twice: a 1000 K Cell inside a 3 × 3 × 3 block of 300 K rock. One block
    // is in the Active Region; the other is at x = 200, which is Chunk 6, far outside it.
    let block_at = |x| cuboid(CellPos::new(x - 1, 4, 4), CellPos::new(x + 1, 6, 6));
    let (active_spot, dormant_spot) = (CellPos::new(5, 5, 5), CellPos::new(200, 5, 5));
    for pos in [block_at(5), block_at(200)].concat() {
        sim.set(pos, cell(rock, 1_000, 300.0));
    }
    sim.set(active_spot, cell(rock, 1_000, 1_000.0));
    sim.set(dormant_spot, cell(rock, 1_000, 1_000.0));
    let dormant_block: Vec<_> = block_at(200).iter().map(|&pos| sim.get(pos)).collect();
    let invariants = Invariants::record(&sim, [block_at(5), block_at(200)].concat());

    run_ticks(&mut sim, 100, &invariants);

    // Time is stopped in the Dormant Chunk: every Cell is exactly as it was set.
    let now: Vec<_> = block_at(200).iter().map(|&pos| sim.get(pos)).collect();
    assert_eq!(now, dormant_block);
    assert_eq!(sim.get(dormant_spot).temperature_k, 1_000.0);
    // The same hot spot in the Active Region cooled. Each Tick it loses at least
    // 100 W/(m·K) × 0.2 s × (its excess) per face, so after 100 Ticks it is far below 1000 K.
    let active_k = sim.get(active_spot).temperature_k;
    assert!(
        active_k < 900.0,
        "the active hot spot is still at {active_k} K"
    );
}

#[test]
fn gas_and_heat_against_the_active_regions_edge_never_cross_into_the_dormant_chunk_beyond() {
    let mut elements = ElementTable::new();
    let wall = elements.add(test_wall());
    let gas = elements.add(test_gas("TestGas", 1.0));
    let rock = elements.add(test_rock());
    let mut sim = Simulation::new(elements);
    // A box of Cells x = 60..=63 against the Active Region's +x edge, which runs between x = 63
    // and x = 64. It is walled in on every side but that one: beyond it is only the Dormant Chunk
    // at x = 64..=95, whose Cells facing the box are Vacuum.
    let (min, max) = (CellPos::new(60, 0, 0), CellPos::new(63, 2, 2));
    let walls: Vec<CellPos> = SealedBox { min, max }
        .walls()
        .into_iter()
        .filter(|pos| pos.x <= max.x)
        .collect();
    for &pos in &walls {
        sim.set(pos, cell(wall, 1_000, 300.0));
    }
    // High-pressure gas right against the edge, and hot rock touching cold rock across it.
    // Gas and walls conduct no heat, so the rocks could only swap heat with each other.
    for pos in cuboid(min, max) {
        sim.set(pos, cell(gas, 100_000, 300.0));
    }
    let (hot, cold) = (CellPos::new(63, 1, 1), CellPos::new(64, 1, 1));
    sim.set(hot, cell(rock, 1_000, 1_000.0));
    sim.set(cold, cell(rock, 1_000, 300.0));
    let beyond = ChunkPos::of(cold);
    assert!(!sim.active_region().contains(beyond));
    let dormant: Vec<Cell> = sim.chunk_cells(beyond).expect("set loads it").to_vec();
    let inside = [cuboid(min, max), walls].concat();
    let invariants = Invariants::record(&sim, inside.clone());
    let inside_grams = grams_by_element(&sim, &inside);

    for tick in 0..100 {
        run_ticks(&mut sim, 1, &invariants);
        assert_eq!(
            sim.chunk_cells(beyond).expect("still loaded"),
            dormant,
            "tick {tick}: the Dormant Chunk changed"
        );
        // The edge is insulated both ways: 100 W/(m·K) × 0.2 s × 700 K = 14,000 J would have
        // cooled the hot rock by 14 K in the first Tick alone.
        assert_eq!(sim.get(hot), cell(rock, 1_000, 1_000.0), "tick {tick}");
    }
    // Every gram stayed inside (the invariants check this after every Tick, too).
    assert_eq!(grams_by_element(&sim, &inside), inside_grams);
}

#[test]
fn moving_the_focus_activates_a_dormant_chunk_which_resumes_exactly_where_it_was_left() {
    let mut elements = ElementTable::new();
    let rock = elements.add(test_rock());
    // Two copies of one world: a hot spot cooling inside a block of rock, all in Chunk (0, 0, 0).
    // In `paused` the Chunk spends 50 Ticks Dormant; in `reference` it stays active. Heat flow
    // doesn't depend on the Tick count, so the two can be compared Cell for Cell.
    let chunk = ChunkPos::new(0, 0, 0);
    let block = cuboid(CellPos::new(4, 4, 4), CellPos::new(6, 6, 6));
    let build = || {
        let mut sim = Simulation::new(elements.clone());
        for &pos in &block {
            sim.set(pos, cell(rock, 1_000, 300.0));
        }
        sim.set(CellPos::new(5, 5, 5), cell(rock, 1_000, 1_000.0));
        sim
    };
    let (mut paused, mut reference) = (build(), build());
    let invariants = Invariants::record(&paused, block.clone());
    run_ticks(&mut paused, 10, &invariants);
    run_ticks(&mut reference, 10, &invariants);

    // The box best centred on x = 1000 is Chunks 29..=32 along x, far from Chunk 0.
    paused.focus_on(CellPos::new(1_000, 0, 0));
    assert!(!paused.active_region().contains(chunk));
    let left: Vec<Cell> = paused.chunk_cells(chunk).expect("still loaded").to_vec();
    run_ticks(&mut paused, 50, &invariants);
    assert_eq!(paused.chunk_cells(chunk).expect("still loaded"), left);

    paused.focus_on(CellPos::new(0, 0, 0));
    assert!(paused.active_region().contains(chunk));
    assert_eq!(paused.chunk_cells(chunk).expect("still loaded"), left);
    run_ticks(&mut paused, 10, &invariants);
    run_ticks(&mut reference, 10, &invariants);
    assert_eq!(paused.chunk_cells(chunk), reference.chunk_cells(chunk));
    // It really was still cooling: the hot spot is well above the block's final 326 K, which is
    // (1000 + 26 × 300) / 27.
    let spot_k = paused.get(CellPos::new(5, 5, 5)).temperature_k;
    assert!(spot_k > 330.0, "the hot spot is already at {spot_k} K");
}

#[test]
fn a_never_seen_chunk_entering_the_active_region_comes_from_the_chunk_source() {
    let mut elements = ElementTable::new();
    let marker = elements.add(test_marker());
    let (source, log) = MarkerSource::new(marker);
    let mut sim = Simulation::with_source(elements, source);
    // The default Active Region is generated straight away: 4 × 2 × 4 = 32 Chunks.
    let region: Vec<ChunkPos> = sim.active_region().chunks().collect();
    assert_eq!(region.len(), 32);
    assert_eq!(generated(&log), region);
    let new = ChunkPos::new(2, 0, 0);
    assert_eq!(sim.chunk_cells(new), None, "never seen yet");

    // The box best centred on x = 20 is Chunks −1..=2 along x (see the hysteresis test), so the
    // 2 × 4 Chunks at x = 2 enter and the 2 × 4 at x = −2 become Dormant.
    sim.focus_on(CellPos::new(20, 0, 0));

    let entered: Vec<ChunkPos> = generated(&log)[32..].to_vec();
    assert_eq!(entered.len(), 8);
    assert!(entered.iter().all(|chunk| chunk.x == 2));
    assert!(
        entered
            .iter()
            .all(|&chunk| sim.active_region().contains(chunk))
    );
    assert_eq!(
        sim.chunk_cells(new).expect("generated"),
        vec![marker_cell(marker, new); CHUNK_VOLUME]
    );
    let stats = sim.stats();
    assert_eq!((stats.active_chunks, stats.dormant_chunks), (32, 8));

    // Back again: the Chunks at x = −2 come from memory, not from the source.
    sim.focus_on(CellPos::new(0, 0, 0));
    assert_eq!(generated(&log).len(), 40);
    let stats = sim.stats();
    assert_eq!((stats.active_chunks, stats.dormant_chunks), (32, 8));
}

#[test]
fn moving_the_focus_back_and_forth_across_a_chunk_border_keeps_the_same_active_region() {
    let mut sim = Simulation::new(ElementTable::new());
    let x_range = |sim: &Simulation| (sim.active_region().min().x, sim.active_region().max().x);
    let at_origin = sim.active_region();
    assert_eq!(
        (at_origin.min(), at_origin.max()),
        (ChunkPos::new(-2, -1, -2), ChunkPos::new(1, 0, 1))
    );

    // Back and forth across the Chunk borders at x = 0 and y = 0, through the middle of the box.
    for (x, y) in [(-1, 0), (0, -1), (-1, -1), (0, 0), (-1, 0), (0, 0)] {
        sim.focus_on(CellPos::new(x, y, 0));
        assert_eq!(sim.active_region(), at_origin, "focus at x = {x}, y = {y}");
    }

    // Along x, the box of Chunks −2..=1 is centred on x = 0, and it is the best-centred box for
    // focus positions within half a Chunk of that: x = −16 to 15. From x = 16 on, Chunks −1..=2
    // are better centred, but the box stays put until the focus is more than 4 Cells past that
    // point, at x = 20.
    for x in [16, 19, 15, 19, 17] {
        sim.focus_on(CellPos::new(x, 0, 0));
        assert_eq!(sim.active_region(), at_origin, "focus at x = {x}");
    }
    sim.focus_on(CellPos::new(20, 0, 0));
    assert_eq!(x_range(&sim), (-1, 2));

    // Coming back, the new box is the best-centred one for x = 16 to 47, so it stays put down to
    // x = 12 and moves back only at x = 11.
    for x in [16, 15, 12, 19, 12] {
        sim.focus_on(CellPos::new(x, 0, 0));
        assert_eq!(x_range(&sim), (-1, 2), "focus at x = {x}");
    }
    sim.focus_on(CellPos::new(11, 0, 0));
    assert_eq!(sim.active_region(), at_origin);
}

#[test]
fn gas_in_the_top_layer_of_the_world_escapes_and_stats_report_exactly_the_mass_lost() {
    let mut elements = ElementTable::new();
    let wall = elements.add(test_wall());
    let gas = elements.add(test_gas("TestGas", 1.0));
    let mut sim = Simulation::new(elements);
    sim.set_settings(SimSettings {
        gas_escape_fraction: 0.01,
        ..SimSettings::default()
    })
    .expect("valid settings");
    // The world's top layer is y = 255, in Chunk layer 7 (y = 224..=255). Focused near it, the
    // Active Region reaches the top of the world, and the layer above is never loaded.
    sim.focus_on(CellPos::new(0, 250, 0));
    assert_eq!(sim.active_region().max().y, 7);
    // A walled-in column of gas up to the top layer. The wall Cells at y = 256 would lie outside
    // the world, so `set` ignores them: above the column there is only the ceiling.
    let column: Vec<CellPos> = (250..=255).map(|y| CellPos::new(0, y, 0)).collect();
    let sealed = SealedBox::build(&mut sim, wall, column[0], column[5]);
    for &pos in &column {
        sim.set(pos, cell(gas, 10_000, 300.0));
    }
    assert_eq!(sim.get(CellPos::new(0, 256, 0)), Cell::VACUUM);
    let walls: Vec<Cell> = sealed.walls().iter().map(|&pos| sim.get(pos)).collect();
    let start_grams = sim.total_mass_grams();

    sim.tick();
    // Every gas Cell holds the same, so no gas flows. Only the top one loses Mass to space:
    // 10,000 g × 0.01 = 100 g.
    let grams = |sim: &Simulation| -> Vec<u32> {
        column.iter().map(|&pos| sim.get(pos).mass_grams).collect()
    };
    assert_eq!(
        grams(&sim),
        vec![10_000, 10_000, 10_000, 10_000, 10_000, 9_900]
    );
    assert_eq!(sim.stats().mass_lost_to_space_grams, 100);

    sim.tick();
    // y = 254 has 2 open faces and y = 255 has 1, since the ceiling is closed to gas flow, so
    // (10,000 − 9,900) × 0.25 / 2 = 12.5 → 12 g flow up. Then 9,912 g × 0.01 = 99.12 → 99 g
    // escape, leaving 9,813 g. Altogether 100 + 99 = 199 g have escaped.
    assert_eq!(
        grams(&sim),
        vec![10_000, 10_000, 10_000, 10_000, 9_988, 9_813]
    );
    assert_eq!(sim.stats().mass_lost_to_space_grams, 199);

    for tick in 2..300 {
        let (lost_before, top_before) = (
            sim.stats().mass_lost_to_space_grams,
            sim.get(column[5]).mass_grams,
        );
        sim.tick();
        let stats = sim.stats();
        // All the Mass that didn't escape is still here.
        assert_eq!(
            stats.total_mass_grams + stats.mass_lost_to_space_grams,
            start_grams,
            "tick {tick}"
        );
        let now: Vec<Cell> = sealed.walls().iter().map(|&pos| sim.get(pos)).collect();
        assert_eq!(now, walls, "tick {tick}: a wall changed");
        for &pos in &column {
            assert!(sim.get(pos).mass_grams >= 1, "tick {tick}: {pos:?} emptied");
        }
        // 1 % of the top Cell rounds down to at least 1 g whenever it holds 100 g or more. Gas
        // flows up into it before it loses Mass, so it holds at least what it held before.
        if top_before >= 100 {
            assert!(
                stats.mass_lost_to_space_grams > lost_before,
                "tick {tick}: nothing escaped"
            );
        }
    }
}

#[test]
fn chunks_below_the_world_bottom_or_at_or_above_its_top_never_load_even_with_the_focus_near_the_limit()
 {
    let mut elements = ElementTable::new();
    let marker = elements.add(test_marker());
    let (source, log) = MarkerSource::new(marker);
    let mut sim = Simulation::with_source(elements, source);
    // In Chunk layers: the world runs from −1024 / 32 = −32 up to 256 / 32 = 8, exclusive.
    let (bottom, top) = (WORLD_BOTTOM_Y / 32, WORLD_TOP_Y / 32);
    assert_eq!((bottom, top), (-32, 8));

    // Near the top, the box best centred on y = 250 would be Chunk layers 7..=8, and near the
    // bottom, the one best centred on y = −1020 would be layers −33..=−32. Each is cut off at the
    // world's limit, leaving 4 × 1 × 4 = 16 Chunks.
    for (y, layer) in [(250, 7), (-1_020, -32)] {
        sim.focus_on(CellPos::new(0, y, 0));
        let region = sim.active_region();
        assert_eq!(
            (region.min().y, region.max().y),
            (layer, layer),
            "focus at y = {y}"
        );
        assert_eq!(sim.stats().active_chunks, 16, "focus at y = {y}");
    }
    let outside = [ChunkPos::new(0, top, 0), ChunkPos::new(0, bottom - 1, 0)];
    for chunk in outside {
        sim.load(chunk);
    }
    sim.set(CellPos::new(0, WORLD_TOP_Y, 0), cell(marker, 1_000, 300.0));
    sim.set(
        CellPos::new(0, WORLD_BOTTOM_Y - 1, 0),
        cell(marker, 1_000, 300.0),
    );

    let generated = generated(&log);
    assert!(
        generated
            .iter()
            .all(|chunk| (bottom..top).contains(&chunk.y)),
        "the source was asked for a Chunk outside the world"
    );
    assert!(generated.iter().any(|chunk| chunk.y == top - 1));
    assert!(generated.iter().any(|chunk| chunk.y == bottom));
    for chunk in outside {
        assert_eq!(sim.chunk_cells(chunk), None, "{chunk:?} was loaded");
    }
    assert_eq!(sim.get(CellPos::new(0, WORLD_TOP_Y, 0)), Cell::VACUUM);
    assert_eq!(
        sim.get(CellPos::new(0, WORLD_BOTTOM_Y - 1, 0)),
        Cell::VACUUM
    );
}

// Cases the implementation revealed, beyond the seven tests in the plan.

#[test]
fn gas_on_the_world_floor_never_leaves_through_it() {
    let mut elements = ElementTable::new();
    let wall = elements.add(test_wall());
    let gas = elements.add(test_gas("TestGas", 1.0));
    let mut sim = Simulation::new(elements);
    sim.focus_on(CellPos::new(0, WORLD_BOTTOM_Y + 4, 0));
    // A walled-in column of dense gas on the world's bottom layer, y = −1024. The wall Cells at
    // y = −1025 would lie outside the world, so `set` ignores them: below the column there is
    // only the floor.
    let column: Vec<CellPos> = (0..4)
        .map(|y| CellPos::new(0, WORLD_BOTTOM_Y + y, 0))
        .collect();
    let sealed = SealedBox::build(&mut sim, wall, column[0], column[3]);
    sim.set(column[0], cell(gas, 1_000_000, 300.0));
    let invariants = Invariants::record(&sim, sealed.all());

    run_ticks(&mut sim, 100, &invariants);

    assert_eq!(sim.stats().mass_lost_to_space_grams, 0);
    // The gas spread up the column, so it was free to move, and all 1,000,000 g are still in it.
    assert!(column.iter().all(|&pos| sim.get(pos).element == gas));
}

#[test]
fn an_edit_to_a_dormant_chunk_is_stored_and_takes_effect_once_the_chunk_is_active() {
    let mut elements = ElementTable::new();
    let rock = elements.add(test_rock());
    let mut sim = Simulation::new(elements);
    // Two touching rock Cells at x = 100 and 101, in Chunk 3 along x: Dormant.
    let (warmed, neighbour) = (CellPos::new(100, 5, 5), CellPos::new(101, 5, 5));
    sim.set(warmed, cell(rock, 1_000, 300.0));
    sim.set(neighbour, cell(rock, 1_000, 300.0));
    assert!(!sim.active_region().contains(ChunkPos::of(warmed)));
    let invariants = Invariants::record(&sim, vec![warmed, neighbour]);

    sim.apply(Edit::ShiftTemperature {
        pos: warmed,
        kelvin: 100.0,
    });
    run_ticks(&mut sim, 10, &invariants);
    // The Edit is stored, but no heat flows while the Chunk is Dormant.
    assert_eq!(sim.get(warmed), cell(rock, 1_000, 400.0));
    assert_eq!(sim.get(neighbour), cell(rock, 1_000, 300.0));

    sim.focus_on(warmed);
    run_ticks(&mut sim, 1, &invariants);
    // 100 W/(m·K) × 0.2 s × 100 K = 2000 J into 1000 J/K: +2 K, exact in f32.
    assert_eq!(sim.get(neighbour), cell(rock, 1_000, 302.0));
}

#[test]
fn a_settled_chunk_left_behind_by_the_moving_edge_wakes_and_its_gas_flows_on() {
    let mut elements = ElementTable::new();
    let wall = elements.add(test_wall());
    let gas = elements.add(test_gas("TestGas", 1.0));
    let mut sim = Simulation::new(elements);
    // A corridor of evenly spread gas, x = 60..=63, walled in on every side except +x, where it
    // meets the Active Region's edge. Beyond it, Chunk 2 along x is Vacuum.
    let (min, max) = (CellPos::new(60, 5, 5), CellPos::new(63, 5, 5));
    let walls: Vec<CellPos> = SealedBox { min, max }
        .walls()
        .into_iter()
        .filter(|pos| pos.x <= max.x)
        .collect();
    for &pos in &walls {
        sim.set(pos, cell(wall, 1_000, 300.0));
    }
    for pos in cuboid(min, max) {
        sim.set(pos, cell(gas, 1_000, 300.0));
    }
    // The Cells just beyond the edge too, which the gas reaches once Chunk 2 is active.
    let beyond = cuboid(CellPos::new(64, 4, 4), CellPos::new(65, 6, 6));
    let invariants = Invariants::record(&sim, [cuboid(min, max), walls, beyond].concat());
    run_ticks(&mut sim, 1, &invariants);
    assert_eq!(
        sim.stats().settled_chunks,
        1,
        "the gas is at rest against the edge"
    );

    // The box best centred on x = 20 is Chunks −1..=2, so Chunk 2 becomes active.
    sim.focus_on(CellPos::new(20, 5, 5));
    run_ticks(&mut sim, 1, &invariants);

    // The end of the corridor has 2 open faces; the Cell beyond it, in open Vacuum, has 6. So
    // 1000 g × 0.25 / 6 = 41.7 → 41 g crosses where the edge was.
    assert_eq!(sim.get(CellPos::new(64, 5, 5)), cell(gas, 41, 300.0));
}
