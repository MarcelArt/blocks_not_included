use std::collections::HashMap;
use std::ops::{Deref, DerefMut};

use crate::cell::{Cell, CellPos};
use crate::region::ActiveRegion;

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

/// The Chunks that every phenomenon works on: the stored Chunks of the Active Region. It
/// dereferences to the map of those Chunks.
///
/// A Chunk of the Active Region that isn't stored holds nothing but Vacuum: a Chunk generated
/// that way needs no storage until something enters it. Every Cell outside the Active Region lies
/// beyond its sealed, insulated edge (ADR 0001), and reads as [`Cell::SEALED_EDGE`], a solid that
/// conducts no heat. So no Mass or heat crosses the edge, and Dormant Chunks are never touched.
pub(crate) struct ActiveChunks {
    chunks: HashMap<ChunkPos, Chunk>,
    pub region: ActiveRegion,
}

impl ActiveChunks {
    pub fn new(region: ActiveRegion) -> Self {
        Self {
            chunks: HashMap::new(),
            region,
        }
    }
}

impl Deref for ActiveChunks {
    type Target = HashMap<ChunkPos, Chunk>;

    fn deref(&self) -> &Self::Target {
        &self.chunks
    }
}

impl DerefMut for ActiveChunks {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.chunks
    }
}

/// Splits a world position into the Chunk containing it and the Cell's index within that Chunk.
pub(crate) fn locate(pos: CellPos) -> (ChunkPos, usize) {
    // CHUNK_SIZE is a power of two, so an arithmetic shift is the Euclidean division, and the
    // mask is the remainder, for negative positions too.
    const SHIFT: u32 = CHUNK_SIZE.trailing_zeros();
    const MASK: i32 = CHUNK_SIZE as i32 - 1;
    let chunk = ChunkPos {
        x: pos.x >> SHIFT,
        y: pos.y >> SHIFT,
        z: pos.z >> SHIFT,
    };
    let local = (
        (pos.x & MASK) as usize,
        (pos.y & MASK) as usize,
        (pos.z & MASK) as usize,
    );
    (chunk, index(local))
}

const _: () = assert!(CHUNK_SIZE.is_power_of_two());

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
        Self::from_cells(vec![Cell::VACUUM; CHUNK_VOLUME])
    }

    /// An awake Chunk of `cells`, which must number `CHUNK_VOLUME`.
    pub fn from_cells(cells: Vec<Cell>) -> Self {
        debug_assert_eq!(cells.len(), CHUNK_VOLUME);
        Self {
            cells: cells.into_boxed_slice(),
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
