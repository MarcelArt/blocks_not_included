use std::collections::HashMap;

use crate::cell::{Cell, CellPos};

pub(crate) const CHUNK_SIZE: usize = 32;
pub(crate) const CHUNK_VOLUME: usize = CHUNK_SIZE * CHUNK_SIZE * CHUNK_SIZE;

/// A Chunk's position, in units of whole Chunks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ChunkPos {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl ChunkPos {
    /// The world position of this Chunk's Cell at `index`.
    pub fn cell_pos(self, index: usize) -> CellPos {
        let size = CHUNK_SIZE as i32;
        let (x, y, z) = local(index);
        CellPos::new(
            self.x * size + x as i32,
            self.y * size + y as i32,
            self.z * size + z as i32,
        )
    }
}

/// The Cell at `pos`. Any Cell in a Chunk that doesn't exist yet is Vacuum.
pub(crate) fn cell_at(chunks: &HashMap<ChunkPos, Chunk>, pos: CellPos) -> Cell {
    let (chunk, index) = locate(pos);
    chunks
        .get(&chunk)
        .map_or(Cell::VACUUM, |chunk| chunk.cells[index])
}

/// Overwrites the Cell at `pos`, creating its Chunk if needed.
pub(crate) fn set_cell(chunks: &mut HashMap<ChunkPos, Chunk>, pos: CellPos, cell: Cell) {
    let (chunk, index) = locate(pos);
    chunks.entry(chunk).or_insert_with(Chunk::new).cells[index] = cell;
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
}

impl Chunk {
    pub fn new() -> Self {
        Self {
            cells: vec![Cell::VACUUM; CHUNK_VOLUME].into_boxed_slice(),
        }
    }
}
