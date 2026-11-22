use std::collections::HashMap;

use crate::cell::{Cell, CellPos};
use crate::chunk::{Chunk, ChunkPos, locate};
use crate::element::ElementTable;
use crate::heat;

/// Simulated time covered by one Tick.
pub(crate) const TICK_SECONDS: f32 = 0.2;

/// The whole simulated world. Any Cell that has never been set is Vacuum.
pub struct Simulation {
    elements: ElementTable,
    chunks: HashMap<ChunkPos, Chunk>,
}

impl Simulation {
    pub fn new(elements: ElementTable) -> Self {
        Self {
            elements,
            chunks: HashMap::new(),
        }
    }

    pub fn set(&mut self, pos: CellPos, cell: Cell) {
        let (chunk, index) = locate(pos);
        self.chunks.entry(chunk).or_insert_with(Chunk::new).cells[index] = cell;
    }

    pub fn get(&self, pos: CellPos) -> Cell {
        let (chunk, index) = locate(pos);
        self.chunks
            .get(&chunk)
            .map_or(Cell::VACUUM, |chunk| chunk.cells[index])
    }

    /// Advances the simulation by one Tick.
    pub fn tick(&mut self) {
        heat::conduct(&mut self.chunks, &self.elements);
    }
}
