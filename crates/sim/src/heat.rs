use std::collections::HashMap;

use crate::cell::Cell;
use crate::chunk::{
    ActiveChunks, CHUNK_SIZE, CHUNK_VOLUME, Chunk, ChunkPos, faces_awake, index, is_awake, local,
};
use crate::element::{ElementId, ElementTable};
use crate::simulation::TICK_SECONDS;

/// Positive-axis face neighbours; visiting only these covers each shared face exactly once.
const FORWARD_FACES: [(i32, i32, i32); 3] = [(1, 0, 0), (0, 1, 0), (0, 0, 1)];

/// Moves heat between Cells that share a face.
///
/// Every flow is computed from the temperatures at the start of the Tick and applied together
/// afterwards, so no direction or processing order is favoured. Chunks are visited in a fixed
/// order, so each Cell adds up its flows in the same order every time. `multiplier` speeds
/// conduction up: with real values, heat takes days to cross one Cell.
///
/// Faces inside a Settled Chunk are skipped. A face between two Chunks is processed if either
/// Chunk is awake, so heat flows into and out of a Settled Chunk at its borders.
pub(crate) fn conduct(chunks: &mut ActiveChunks, elements: &ElementTable, multiplier: f32) {
    let conduction = Conduction {
        elements,
        multiplier,
    };
    let mut keys: Vec<ChunkPos> = chunks.keys().copied().collect();
    keys.sort_unstable();
    // Every Chunk that is awake or touches an awake Chunk may gain or lose heat.
    let mut gained: HashMap<ChunkPos, Vec<f32>> = keys
        .iter()
        .copied()
        .filter(|&key| is_awake(chunks, key) || faces_awake(chunks, key))
        .map(|key| (key, vec![0.0; CHUNK_VOLUME]))
        .collect();

    for &key in &keys {
        let chunk = &chunks[&key];
        if chunk.is_awake() {
            let gained_here = gained.get_mut(&key).expect("an awake Chunk has an entry");
            gather_within_chunk(chunk, gained_here, &conduction);
        }
        for (axis, face) in FORWARD_FACES.into_iter().enumerate() {
            let next = key.offset(face);
            let Some(there) = chunks.get(&next) else {
                continue;
            };
            if !chunk.is_awake() && !there.is_awake() {
                continue;
            }
            let [Some(gained_here), Some(gained_there)] = gained.get_disjoint_mut([&key, &next])
            else {
                unreachable!("one of the two Chunks is awake, so both have entries");
            };
            gather_across_border(chunk, there, gained_here, gained_there, axis, &conduction);
        }
    }

    for (key, gained) in gained {
        let chunk = chunks
            .get_mut(&key)
            .expect("only existing Chunks have entries");
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
    }
}

/// What decides how much heat crosses a face.
struct Conduction<'a> {
    elements: &'a ElementTable,
    multiplier: f32,
}

fn gather_within_chunk(chunk: &Chunk, gained: &mut [f32], conduction: &Conduction) {
    for a in 0..CHUNK_VOLUME {
        let (x, y, z) = local(a);
        for (dx, dy, dz) in FORWARD_FACES {
            let neighbour = (x + dx as usize, y + dy as usize, z + dz as usize);
            if neighbour.0 >= CHUNK_SIZE || neighbour.1 >= CHUNK_SIZE || neighbour.2 >= CHUNK_SIZE {
                continue;
            }
            let b = index(neighbour);
            let joules = conduction.flow(chunk.cells[a], chunk.cells[b]);
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
            let joules = conduction.flow(here.cells[a], there.cells[b]);
            gained_here[a] += joules;
            gained_there[b] -= joules;
        }
    }
}

impl Conduction<'_> {
    /// Joules flowing from `b` into `a` across their shared face during one Tick.
    fn flow(&self, a: Cell, b: Cell) -> f32 {
        let elements = self.elements;
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
        // Heat that would bring just this pair to the same temperature. All six faces of a Cell
        // apply their flows at once, so capping each face at a sixth of this keeps every Cell's
        // new temperature a blend of its neighbours' old ones: nothing can overshoot, however
        // large the multiplier.
        let to_equal = (difference * capacity_a * capacity_b / (capacity_a + capacity_b)).abs();
        let cap = to_equal / 6.0;
        // Across a 1 m² face between Cells 1 m apart.
        (conductivity * self.multiplier * TICK_SECONDS * difference).clamp(-cap, cap)
    }
}

/// Joules needed to warm the whole Cell by one kelvin.
fn heat_capacity(cell: Cell, elements: &ElementTable) -> f32 {
    cell.mass_grams as f32 * elements.get(cell.element).specific_heat
}
