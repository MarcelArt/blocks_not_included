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
