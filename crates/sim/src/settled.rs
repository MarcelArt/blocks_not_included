//! Settled Chunks (`CONTEXT.md`): a Chunk in which nothing changed during the last Tick skips
//! work until something changes it.
//!
//! After a Tick, a Chunk that was awake is Settled if, during that Tick, none of its Cells
//! changed Element or Mass and no temperature in it changed by more than
//! [`SimSettings::settle_threshold_k`]. A Settled Chunk wakes when any of its Cells changes at
//! all, whether through an Edit, [`Simulation::set`](crate::Simulation::set), or heat or Mass
//! arriving across a border from an awake neighbour.

use std::collections::HashMap;

use crate::cell::Cell;
use crate::chunk::{Chunk, ChunkPos};
use crate::settings::SimSettings;

/// The Cells of every awake Chunk at the start of a Tick, to compare with at its end.
pub(crate) struct TickStart {
    awake: HashMap<ChunkPos, Box<[Cell]>>,
}

impl TickStart {
    /// Records the awake Chunks and forgets which Chunks were touched before this Tick.
    pub fn record(chunks: &mut HashMap<ChunkPos, Chunk>, settings: &SimSettings) -> Self {
        let mut awake = HashMap::new();
        for (&pos, chunk) in chunks.iter_mut() {
            chunk.touched = false;
            if settings.settled_chunks_enabled && !chunk.settled {
                awake.insert(pos, chunk.cells.clone());
            }
        }
        Self { awake }
    }

    /// Decides which Chunks are Settled for the next Tick.
    pub fn settle(self, chunks: &mut HashMap<ChunkPos, Chunk>, settings: &SimSettings) {
        for (pos, chunk) in chunks.iter_mut() {
            chunk.settled = if !settings.settled_chunks_enabled {
                false
            } else if let Some(start) = self.awake.get(pos) {
                !chunk.touched || is_quiet(start, &chunk.cells, settings.settle_threshold_k)
            } else {
                // Settled at the start of the Tick: any change at all wakes it. A Chunk created
                // during the Tick is awake.
                chunk.settled && !chunk.touched
            };
        }
    }
}

/// Whether no Cell changed Element or Mass, and no temperature changed by more than
/// `threshold_k`, between `start` and `end`.
fn is_quiet(start: &[Cell], end: &[Cell], threshold_k: f32) -> bool {
    start.iter().zip(end).all(|(start, end)| {
        start.element == end.element
            && start.mass_grams == end.mass_grams
            && (end.temperature_k - start.temperature_k).abs() <= threshold_k
    })
}
