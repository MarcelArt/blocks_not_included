use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use bevy::prelude::*;
use sim::{CHUNK_SIZE, Cell, CellPos, ChunkPos};

/// The game's copy of every Chunk within render distance of the focus, as of the latest Tick the
/// simulation has sent (ADR 0003: up to one Tick old). The sim bridge fills it early in each
/// frame, so every system after that sees the same state.
///
/// A loaded Chunk that has held nothing but Vacuum since it was generated isn't listed, and
/// neither is a Chunk that hasn't finished loading. Both read as `None`.
///
/// It also records which Chunks changed during the current frame, for renderers to read: see
/// [`changed`](Self::changed) and [`removed`](Self::removed).
#[derive(Resource, Default)]
pub struct WorldView {
    chunks: HashMap<ChunkPos, Arc<[Cell]>>,
    changed: HashSet<ChunkPos>,
    removed: HashSet<ChunkPos>,
}

impl WorldView {
    /// The Cells of the Chunk at `pos`, in the order `sim::Simulation::chunk_cells` lists them.
    pub fn chunk(&self, pos: ChunkPos) -> Option<&Arc<[Cell]>> {
        self.chunks.get(&pos)
    }

    /// The Cell at `pos`, or `None` if its Chunk isn't in the view.
    pub fn cell(&self, pos: CellPos) -> Option<Cell> {
        let (chunk, index) = locate(pos);
        self.chunks.get(&chunk).map(|cells| cells[index])
    }

    /// Every Chunk in the view, in no particular order.
    pub fn chunks(&self) -> impl Iterator<Item = (ChunkPos, &Arc<[Cell]>)> {
        self.chunks.iter().map(|(&pos, cells)| (pos, cells))
    }

    /// How many Chunks are in the view.
    pub fn len(&self) -> usize {
        self.chunks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    /// Chunks that entered the view, or whose Cells changed, this frame. Each is in the view.
    pub fn changed(&self) -> &HashSet<ChunkPos> {
        &self.changed
    }

    /// Chunks that left the view this frame. None of them is in the view.
    pub fn removed(&self) -> &HashSet<ChunkPos> {
        &self.removed
    }

    /// Whether any Chunk changed or left the view this frame.
    pub(crate) fn has_changes(&self) -> bool {
        !self.changed.is_empty() || !self.removed.is_empty()
    }

    /// Starts a new frame's record of changes.
    pub(crate) fn clear_changes(&mut self) {
        self.changed.clear();
        self.removed.clear();
    }

    /// Adds the Chunk at `pos`, or replaces its Cells.
    pub(crate) fn insert(&mut self, pos: ChunkPos, cells: Arc<[Cell]>) {
        debug_assert_eq!(cells.len(), CHUNK_SIZE.pow(3));
        self.chunks.insert(pos, cells);
        self.removed.remove(&pos);
        self.changed.insert(pos);
    }

    /// Takes the Chunk at `pos` out of the view.
    pub(crate) fn remove(&mut self, pos: ChunkPos) {
        if self.chunks.remove(&pos).is_some() {
            self.changed.remove(&pos);
            self.removed.insert(pos);
        }
    }
}

/// Splits a world position into the Chunk containing it and the Cell's index within that Chunk:
/// x fastest, then z, then y, as `sim::Simulation::chunk_cells` lists them.
pub(crate) fn locate(pos: CellPos) -> (ChunkPos, usize) {
    let chunk = ChunkPos::of(pos);
    let origin = chunk.origin();
    let local = |cell: i32, origin: i32| (cell - origin) as usize;
    let (x, y, z) = (
        local(pos.x, origin.x),
        local(pos.y, origin.y),
        local(pos.z, origin.z),
    );
    (chunk, (y * CHUNK_SIZE + z) * CHUNK_SIZE + x)
}
