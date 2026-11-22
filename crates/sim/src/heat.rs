use rayon::prelude::*;

use crate::cell::Cell;
use crate::chunk::{ActiveChunks, CHUNK_SIZE, CHUNK_VOLUME, Chunk, index};
use crate::element::{ElementId, ElementTable};
use crate::grid::{Grid, change_chunks};
use crate::simulation::TICK_SECONDS;

/// Positive-axis face neighbours; visiting only these covers each shared face exactly once.
const FORWARD_FACES: [(i32, i32, i32); 3] = [(1, 0, 0), (0, 1, 0), (0, 0, 1)];

/// Moves heat between Cells that share a face.
///
/// Every flow is computed from the temperatures at the start of the Tick and applied together
/// afterwards, so no direction or processing order is favoured. Each Cell adds up its flows in
/// the same order every time, so f32 rounding comes out the same however the work is split
/// across threads. `multiplier` speeds conduction up: with real values, heat takes days to cross
/// one Cell.
///
/// Faces inside a Settled Chunk are skipped. A face between two Chunks is processed if either
/// Chunk is awake, so heat flows into and out of a Settled Chunk at its borders.
pub(crate) fn conduct(chunks: &mut ActiveChunks, elements: &ElementTable, multiplier: f32) {
    let conduction = Conduction {
        elements,
        multiplier,
    };
    let (layout, gained) = {
        let grid = Grid::new(chunks);
        // Every Chunk that is awake or touches an awake Chunk may gain or lose heat.
        let gained: Vec<Vec<f32>> = (0..grid.len())
            .into_par_iter()
            .with_max_len(1)
            .map(|slot| gather(&grid, slot, &conduction).unwrap_or_default())
            .collect();
        (grid.into_layout(), gained)
    };
    change_chunks(chunks, &layout, gained, |chunk, gained| {
        for (index, joules) in gained.into_iter().enumerate() {
            let cell = chunk.cells[index];
            if joules != 0.0 && cell.element != ElementId::VACUUM {
                let temperature_k = cell.temperature_k + joules / heat_capacity(cell, elements);
                chunk.write(
                    index,
                    Cell {
                        temperature_k,
                        ..cell
                    },
                );
            }
        }
    });
}

/// The joules each Cell of the Chunk at `slot` gains this Tick, or `None` if neither it nor any
/// Chunk it touches is awake.
///
/// A Cell adds up its flows in a fixed order: across its Chunk's borders with the neighbours
/// below it on x, then y, then z; then within its Chunk, in the order of the Cells' indices;
/// then across the borders with the neighbours above it on x, then y, then z. This is the order
/// in which visiting the Chunks one at a time, sorted by position, and each one's faces toward
/// its positive neighbours, would reach them.
fn gather(grid: &Grid, slot: usize, conduction: &Conduction) -> Option<Vec<f32>> {
    let (pos, chunk) = grid.at(slot);
    let neighbour = |face: (i32, i32, i32)| grid.chunk(pos.offset(face));
    let awake = chunk.is_awake();
    let borders_awake = [-1, 1].into_iter().any(|sign| {
        FORWARD_FACES
            .into_iter()
            .any(|(x, y, z)| neighbour((x * sign, y * sign, z * sign)).is_some_and(Chunk::is_awake))
    });
    if !awake && !borders_awake {
        return None;
    }
    let mut gained = vec![0.0; CHUNK_VOLUME];
    // A border is processed if either Chunk is awake.
    let processed = |face| neighbour(face).filter(|there: &&Chunk| awake || there.is_awake());
    for (axis, (x, y, z)) in FORWARD_FACES.into_iter().enumerate() {
        if let Some(below) = processed((-x, -y, -z)) {
            gather_across_border(below, chunk, None, Some(&mut gained), axis, conduction);
        }
    }
    if awake {
        gather_within_chunk(chunk, &mut gained, conduction);
    }
    for (axis, face) in FORWARD_FACES.into_iter().enumerate() {
        if let Some(above) = processed(face) {
            gather_across_border(chunk, above, Some(&mut gained), None, axis, conduction);
        }
    }
    Some(gained)
}

/// What decides how much heat crosses a face.
struct Conduction<'a> {
    elements: &'a ElementTable,
    multiplier: f32,
}

fn gather_within_chunk(chunk: &Chunk, gained: &mut [f32], conduction: &Conduction) {
    let cells = &chunk.cells[..CHUNK_VOLUME];
    let gained = &mut gained[..CHUNK_VOLUME];
    let last = CHUNK_SIZE - 1;
    // In the order of the Cells' indices: x fastest, then z, then y.
    for y in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            for x in 0..CHUNK_SIZE {
                let a = index((x, y, z));
                let neighbours = [
                    (x < last).then(|| a + 1),
                    (y < last).then(|| a + CHUNK_SIZE * CHUNK_SIZE),
                    (z < last).then(|| a + CHUNK_SIZE),
                ];
                for b in neighbours.into_iter().flatten() {
                    if let Some(joules) = conduction.flow(cells[a], cells[b]) {
                        gained[a] += joules;
                        gained[b] -= joules;
                    }
                }
            }
        }
    }
}

/// Pairs the Cells on `here`'s positive face along `axis` with the touching Cells of `there`,
/// adding the flows to whichever of the two Chunks' gains are given.
fn gather_across_border(
    here: &Chunk,
    there: &Chunk,
    mut gained_here: Option<&mut [f32]>,
    mut gained_there: Option<&mut [f32]>,
    axis: usize,
    conduction: &Conduction,
) {
    let last = CHUNK_SIZE - 1;
    for u in 0..CHUNK_SIZE {
        for v in 0..CHUNK_SIZE {
            let (a, b) = match axis {
                0 => (index((last, u, v)), index((0, u, v))),
                1 => (index((u, last, v)), index((u, 0, v))),
                _ => (index((u, v, last)), index((u, v, 0))),
            };
            if let Some(joules) = conduction.flow(here.cells[a], there.cells[b]) {
                if let Some(gained) = gained_here.as_deref_mut() {
                    gained[a] += joules;
                }
                if let Some(gained) = gained_there.as_deref_mut() {
                    gained[b] -= joules;
                }
            }
        }
    }
}

impl Conduction<'_> {
    /// Joules flowing from `b` into `a` across their shared face during one Tick, or `None`
    /// where nothing can flow because a Cell is Vacuum or both are at the same finite temperature.
    /// Adding or taking away a zero flow would change no sum, so those faces are skipped.
    fn flow(&self, a: Cell, b: Cell) -> Option<f32> {
        if a.element == ElementId::VACUUM
            || b.element == ElementId::VACUUM
            || (a.temperature_k == b.temperature_k && a.temperature_k.is_finite())
        {
            return None;
        }
        let elements = self.elements;
        // The poorer conductor limits the flow.
        let conductivity = elements
            .get(a.element)
            .conductivity
            .min(elements.get(b.element).conductivity);
        let (capacity_a, capacity_b) = (heat_capacity(a, elements), heat_capacity(b, elements));
        let difference = b.temperature_k - a.temperature_k;
        // Heat that would bring just this pair to the same temperature. All six faces of a Cell
        // apply their flows at once, so capping each face at a sixth of this keeps every Cell's
        // new temperature a blend of its neighbours' old ones: nothing can overshoot, however
        // large the multiplier.
        let to_equal = (difference * capacity_a * capacity_b / (capacity_a + capacity_b)).abs();
        let cap = to_equal / 6.0;
        // Across a 1 m² face between Cells 1 m apart.
        Some((conductivity * self.multiplier * TICK_SECONDS * difference).clamp(-cap, cap))
    }
}

/// Joules needed to warm the whole Cell by one kelvin.
fn heat_capacity(cell: Cell, elements: &ElementTable) -> f32 {
    cell.mass_grams as f32 * elements.get(cell.element).specific_heat
}
