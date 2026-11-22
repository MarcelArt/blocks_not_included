use std::collections::{BTreeSet, HashMap};

use crate::cell::{Cell, CellPos};
use crate::chunk::{Chunk, ChunkPos, cell_at, set_cell};
use crate::edit::Edit;
use crate::element::ElementTable;
use crate::settings::{SettingsError, SimSettings};
use crate::settled::TickStart;
use crate::{gas, heat, liquid, phase_transition};

/// Simulated time covered by one Tick.
pub(crate) const TICK_SECONDS: f32 = 0.2;

/// A summary of the simulation's state, for the debug panel and the log.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SimStats {
    /// How many Ticks have run.
    pub tick_count: u64,
    /// Chunks that the next Tick simulates.
    pub awake_chunks: usize,
    /// Settled Chunks (`CONTEXT.md`), which the next Tick skips.
    pub settled_chunks: usize,
    /// The Mass of every Cell in the world added together.
    pub total_mass_grams: u64,
}

/// The whole simulated world. Any Cell that has never been set is Vacuum.
pub struct Simulation {
    elements: ElementTable,
    settings: SimSettings,
    chunks: HashMap<ChunkPos, Chunk>,
    /// How many Ticks have run; seeds the flow tie-breaks.
    ticks: u64,
    /// Chunks in which a Cell has changed since the last call to `take_changed_chunks`.
    changed: BTreeSet<ChunkPos>,
}

impl Simulation {
    pub fn new(elements: ElementTable) -> Self {
        Self {
            elements,
            settings: SimSettings::default(),
            chunks: HashMap::new(),
            ticks: 0,
            changed: BTreeSet::new(),
        }
    }

    /// Replaces the tuning values, which take effect from the next Tick, and wakes every Chunk,
    /// since a Chunk at rest under the old values may not be under the new ones.
    ///
    /// Rejects settings that fail [`SimSettings::validate`], keeping the previous ones.
    pub fn set_settings(&mut self, settings: SimSettings) -> Result<(), SettingsError> {
        settings.validate()?;
        self.settings = settings;
        for chunk in self.chunks.values_mut() {
            chunk.settled = false;
        }
        Ok(())
    }

    pub fn settings(&self) -> &SimSettings {
        &self.settings
    }

    /// Overwrites the Cell at `pos`, for tests and world generation, and wakes its Chunk.
    pub fn set(&mut self, pos: CellPos, cell: Cell) {
        let changed = self.get(pos) != cell;
        set_cell(&mut self.chunks, pos, cell);
        let chunk_pos = ChunkPos::of(pos);
        let chunk = self
            .chunks
            .get_mut(&chunk_pos)
            .expect("set_cell creates the Chunk");
        chunk.settled = false;
        if changed {
            self.changed.insert(chunk_pos);
        }
    }

    /// Applies `edit` straight away. The game calls this between Ticks.
    pub fn apply(&mut self, edit: Edit) {
        let pos = edit.pos();
        let cell = edit.result(self.get(pos));
        self.set(pos, cell);
    }

    pub fn get(&self, pos: CellPos) -> Cell {
        cell_at(&self.chunks, pos)
    }

    /// The Cells of the Chunk at `chunk`, or `None` if it has never held anything.
    ///
    /// There are `CHUNK_SIZE³` of them, x fastest, then z, then y: the Cell at
    /// `chunk.origin()` offset by `(x, y, z)` is at index `(y × CHUNK_SIZE + z) × CHUNK_SIZE + x`.
    pub fn chunk_cells(&self, chunk: ChunkPos) -> Option<&[Cell]> {
        self.chunks.get(&chunk).map(|chunk| &*chunk.cells)
    }

    /// The Mass of every Cell in the world added together.
    pub fn total_mass_grams(&self) -> u64 {
        self.chunks
            .values()
            .flat_map(|chunk| chunk.cells.iter())
            .map(|cell| u64::from(cell.mass_grams))
            .sum()
    }

    /// Every Chunk in which any Cell has changed since the last call, sorted by x, then y, then
    /// z. The list starts empty again afterwards.
    pub fn take_changed_chunks(&mut self) -> Vec<ChunkPos> {
        std::mem::take(&mut self.changed).into_iter().collect()
    }

    pub fn stats(&self) -> SimStats {
        let settled_chunks = self.chunks.values().filter(|chunk| chunk.settled).count();
        SimStats {
            tick_count: self.ticks,
            awake_chunks: self.chunks.len() - settled_chunks,
            settled_chunks,
            total_mass_grams: self.total_mass_grams(),
        }
    }

    /// Advances the simulation by one Tick: heat, then gas, then liquid, then Phase Transitions.
    pub fn tick(&mut self) {
        let start = TickStart::record(&mut self.chunks, &self.settings);
        heat::conduct(
            &mut self.chunks,
            &self.elements,
            self.settings.conduction_multiplier,
        );
        gas::flow(&mut self.chunks, &self.elements, self.ticks, &self.settings);
        liquid::flow(&mut self.chunks, &self.elements, self.ticks, &self.settings);
        phase_transition::transform(&mut self.chunks, &self.elements);
        start.settle(&mut self.chunks, &self.settings);
        self.changed.extend(
            self.chunks
                .iter()
                .filter(|(_, chunk)| chunk.touched)
                .map(|(&pos, _)| pos),
        );
        self.ticks += 1;
    }
}
