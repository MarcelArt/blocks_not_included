use std::collections::HashMap;

use crate::cell::Cell;
use crate::chunk::{CHUNK_SIZE, CHUNK_VOLUME, Chunk, ChunkPos, index, local};
use crate::element::{ElementId, ElementTable};
use crate::simulation::TICK_SECONDS;

/// Positive-axis face neighbours; visiting only these covers each shared face exactly once.
const FORWARD_FACES: [(usize, usize, usize); 3] = [(1, 0, 0), (0, 1, 0), (0, 0, 1)];

/// Moves heat between Cells that share a face.
///
/// Every flow is computed from the temperatures at the start of the Tick and applied together
/// afterwards, so no direction or processing order is favoured.
pub(crate) fn conduct(chunks: &mut HashMap<ChunkPos, Chunk>, elements: &ElementTable) {
    let mut gained: HashMap<ChunkPos, Vec<f32>> = chunks
        .keys()
        .map(|&key| (key, vec![0.0; CHUNK_VOLUME]))
        .collect();

    for (&key, chunk) in chunks.iter() {
        gather_within_chunk(chunk, gained.get_mut(&key).expect("one entry per Chunk"), elements);
        for (axis, (dx, dy, dz)) in FORWARD_FACES.into_iter().enumerate() {
            let next = ChunkPos {
                x: key.x + dx as i32,
                y: key.y + dy as i32,
                z: key.z + dz as i32,
            };
            let Some(there) = chunks.get(&next) else {
                continue;
            };
            let [Some(gained_here), Some(gained_there)] = gained.get_disjoint_mut([&key, &next]) else {
                unreachable!("both Chunks exist, so both have entries");
            };
            gather_across_border(chunk, there, gained_here, gained_there, axis, elements);
        }
    }

    for (key, chunk) in chunks.iter_mut() {
        for (cell, joules) in chunk.cells.iter_mut().zip(&gained[key]) {
            if cell.element != ElementId::VACUUM {
                cell.temperature_k += joules / heat_capacity(*cell, elements);
            }
        }
    }
}

fn gather_within_chunk(chunk: &Chunk, gained: &mut [f32], elements: &ElementTable) {
    for a in 0..CHUNK_VOLUME {
        let (x, y, z) = local(a);
        for (dx, dy, dz) in FORWARD_FACES {
            let neighbour = (x + dx, y + dy, z + dz);
            if neighbour.0 >= CHUNK_SIZE || neighbour.1 >= CHUNK_SIZE || neighbour.2 >= CHUNK_SIZE {
                continue;
            }
            let b = index(neighbour);
            let joules = flow(chunk.cells[a], chunk.cells[b], elements);
            gained[a] += joules;
            gained[b] -= joules;
        }
    }
}

/// Pairs the Cells on `here`'s positive face along `axis` with the touching Cells of `there`.
fn gather_across_border(
    here: &Chunk,
    there: &Chunk,
    gained_here: &mut [f32],
    gained_there: &mut [f32],
    axis: usize,
    elements: &ElementTable,
) {
    let last = CHUNK_SIZE - 1;
    for u in 0..CHUNK_SIZE {
        for v in 0..CHUNK_SIZE {
            let (a, b) = match axis {
                0 => (index((last, u, v)), index((0, u, v))),
                1 => (index((u, last, v)), index((u, 0, v))),
                _ => (index((u, v, last)), index((u, v, 0))),
            };
            let joules = flow(here.cells[a], there.cells[b], elements);
            gained_here[a] += joules;
            gained_there[b] -= joules;
        }
    }
}

/// Joules flowing from `b` into `a` across their shared face during one Tick.
fn flow(a: Cell, b: Cell, elements: &ElementTable) -> f32 {
    if a.element == ElementId::VACUUM || b.element == ElementId::VACUUM {
        return 0.0;
    }
    // The poorer conductor limits the flow.
    let conductivity = elements
        .get(a.element)
        .conductivity
        .min(elements.get(b.element).conductivity);
    let (capacity_a, capacity_b) = (heat_capacity(a, elements), heat_capacity(b, elements));
    let difference = b.temperature_k - a.temperature_k;
    // Heat that would bring just this pair to the same temperature. All six faces of a Cell apply
    // their flows at once, so capping each face at a sixth of this keeps every Cell's new
    // temperature a blend of its neighbours' old ones: nothing can overshoot.
    let to_equal = (difference * capacity_a * capacity_b / (capacity_a + capacity_b)).abs();
    let cap = to_equal / 6.0;
    // Across a 1 m² face between Cells 1 m apart.
    (conductivity * TICK_SECONDS * difference).clamp(-cap, cap)
}

/// Joules needed to warm the whole Cell by one kelvin.
fn heat_capacity(cell: Cell, elements: &ElementTable) -> f32 {
    cell.mass_grams as f32 * elements.get(cell.element).specific_heat
}
