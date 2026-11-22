use std::collections::HashMap;

use crate::cell::{Cell, CellPos};
use crate::chunk::{Chunk, ChunkPos, cell_at, set_cell};
use crate::element::ElementTable;
use crate::{gas, heat, liquid};

/// Simulated time covered by one Tick.
pub(crate) const TICK_SECONDS: f32 = 0.2;

/// The whole simulated world. Any Cell that has never been set is Vacuum.
pub struct Simulation {
    elements: ElementTable,
    chunks: HashMap<ChunkPos, Chunk>,
    /// How many Ticks have run; seeds the flow tie-breaks.
    ticks: u64,
}

impl Simulation {
    pub fn new(elements: ElementTable) -> Self {
        Self {
            elements,
            chunks: HashMap::new(),
            ticks: 0,
        }
    }

    pub fn set(&mut self, pos: CellPos, cell: Cell) {
        set_cell(&mut self.chunks, pos, cell);
    }

    pub fn get(&self, pos: CellPos) -> Cell {
        cell_at(&self.chunks, pos)
    }

    /// The Mass of every Cell in the world added together.
    pub fn total_mass_grams(&self) -> u64 {
        self.chunks
            .values()
            .flat_map(|chunk| chunk.cells.iter())
            .map(|cell| u64::from(cell.mass_grams))
            .sum()
    }

    /// Advances the simulation by one Tick.
    pub fn tick(&mut self) {
        heat::conduct(&mut self.chunks, &self.elements);
        gas::flow(&mut self.chunks, &self.elements, self.ticks);
        liquid::flow(&mut self.chunks, &self.elements, self.ticks);
        self.ticks += 1;
    }
}
