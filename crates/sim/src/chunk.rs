use std::collections::HashMap;

use crate::cell::{Cell, CellPos};
use crate::transfer::FACES;

/// How many Cells wide a Chunk is along each axis.
pub const CHUNK_SIZE: usize = 32;
pub(crate) const CHUNK_VOLUME: usize = CHUNK_SIZE * CHUNK_SIZE * CHUNK_SIZE;

/// A Chunk's position, in units of whole Chunks. The Chunk at `(0, 0, 0)` holds the Cells from
/// `(0, 0, 0)` to `(CHUNK_SIZE - 1, CHUNK_SIZE - 1, CHUNK_SIZE - 1)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChunkPos {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl ChunkPos {
    /// The Chunk containing the Cell at `pos`.
    pub fn of(pos: CellPos) -> Self {
        locate(pos).0
    }

    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    /// The position of this Chunk's Cell with the smallest x, y and z.
    pub fn origin(self) -> CellPos {
        self.cell_pos(0)
    }

    /// The Chunk `offset` Chunks away from this one.
    pub(crate) fn offset(self, (dx, dy, dz): (i32, i32, i32)) -> Self {
        Self {
            x: self.x + dx,
            y: self.y + dy,
            z: self.z + dz,
        }
    }

    /// The world position of this Chunk's Cell at `index`.
    pub(crate) fn cell_pos(self, index: usize) -> CellPos {
        let size = CHUNK_SIZE as i32;
        let (x, y, z) = local(index);
        CellPos::new(
            self.x * size + x as i32,
            self.y * size + y as i32,
            self.z * size + z as i32,
        )
    }
}

/// The index of every Cell of the Settled Chunk at `pos` that shares a face with a Cell of an
/// awake Chunk. Phenomena visit these as well as every Cell of an awake Chunk, because borders
/// between awake and Settled Chunks are still processed.
pub(crate) fn cells_facing_awake(chunks: &HashMap<ChunkPos, Chunk>, pos: ChunkPos) -> Vec<usize> {
    let mut cells: Vec<usize> = FACES
        .into_iter()
        .filter(|&face| is_awake(chunks, pos.offset(face)))
        .flat_map(face_layer)
        .collect();
    // A Cell on an edge or corner can lie on two awake faces.
    cells.sort_unstable();
    cells.dedup();
    cells
}

/// Whether the Chunk at `pos` exists and is awake.
pub(crate) fn is_awake(chunks: &HashMap<ChunkPos, Chunk>, pos: ChunkPos) -> bool {
    chunks.get(&pos).is_some_and(Chunk::is_awake)
}

/// Whether the Chunk at `pos` shares a face with an awake Chunk.
pub(crate) fn faces_awake(chunks: &HashMap<ChunkPos, Chunk>, pos: ChunkPos) -> bool {
    FACES
        .into_iter()
        .any(|face| is_awake(chunks, pos.offset(face)))
}

/// The index of every Cell in the layer of a Chunk that touches the neighbour in direction `face`.
fn face_layer(face: (i32, i32, i32)) -> impl Iterator<Item = usize> {
    let last = CHUNK_SIZE - 1;
    (0..CHUNK_SIZE).flat_map(move |u| {
        (0..CHUNK_SIZE).map(move |v| {
            index(match face {
                (1, 0, 0) => (last, u, v),
                (-1, 0, 0) => (0, u, v),
                (0, 1, 0) => (u, last, v),
                (0, -1, 0) => (u, 0, v),
                (0, 0, 1) => (u, v, last),
                _ => (u, v, 0),
            })
        })
    })
}

/// The Cell at `pos`. Any Cell in a Chunk that doesn't exist yet is Vacuum.
pub(crate) fn cell_at(chunks: &HashMap<ChunkPos, Chunk>, pos: CellPos) -> Cell {
    let (chunk, index) = locate(pos);
    chunks
        .get(&chunk)
        .map_or(Cell::VACUUM, |chunk| chunk.cells[index])
}

/// Overwrites the Cell at `pos`, creating its Chunk if needed, and marks the Chunk touched if the
/// Cell changed.
pub(crate) fn set_cell(chunks: &mut HashMap<ChunkPos, Chunk>, pos: CellPos, cell: Cell) {
    let (chunk, index) = locate(pos);
    chunks
        .entry(chunk)
        .or_insert_with(Chunk::new)
        .write(index, cell);
}

/// Splits a world position into the Chunk containing it and the Cell's index within that Chunk.
pub(crate) fn locate(pos: CellPos) -> (ChunkPos, usize) {
    let size = CHUNK_SIZE as i32;
    let chunk = ChunkPos {
        x: pos.x.div_euclid(size),
        y: pos.y.div_euclid(size),
        z: pos.z.div_euclid(size),
    };
    let local = (
        pos.x.rem_euclid(size) as usize,
        pos.y.rem_euclid(size) as usize,
        pos.z.rem_euclid(size) as usize,
    );
    (chunk, index(local))
}

pub(crate) fn index((x, y, z): (usize, usize, usize)) -> usize {
    (y * CHUNK_SIZE + z) * CHUNK_SIZE + x
}

pub(crate) fn local(index: usize) -> (usize, usize, usize) {
    let x = index % CHUNK_SIZE;
    let z = (index / CHUNK_SIZE) % CHUNK_SIZE;
    let y = index / (CHUNK_SIZE * CHUNK_SIZE);
    (x, y, z)
}

/// A fixed-size cube of Cells, stored as a flat array.
pub(crate) struct Chunk {
    pub cells: Box<[Cell]>,
    /// Whether this Chunk was Settled (`CONTEXT.md`) at the start of the Tick.
    pub settled: bool,
    /// Whether any of its Cells has changed during the current Tick.
    pub touched: bool,
}

impl Chunk {
    /// An awake Chunk of Vacuum.
    pub fn new() -> Self {
        Self {
            cells: vec![Cell::VACUUM; CHUNK_VOLUME].into_boxed_slice(),
            settled: false,
            touched: false,
        }
    }

    /// Whether phenomena process this Chunk: it was awake at the start of the Tick, or one of its
    /// Cells has changed since, which wakes a Settled Chunk at once. The passes later in the same
    /// Tick then see the whole Chunk, as they would if it had never settled.
    pub fn is_awake(&self) -> bool {
        !self.settled || self.touched
    }

    /// Overwrites the Cell at `index`, marking this Chunk touched if the Cell changed.
    pub fn write(&mut self, index: usize, cell: Cell) {
        if self.cells[index] != cell {
            self.cells[index] = cell;
            self.touched = true;
        }
    }
}
