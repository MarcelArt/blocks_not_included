//! The Bevy-free simulation core: Cells, Elements, Chunks and the Tick.
//!
//! Vocabulary is defined in `CONTEXT.md` at the repository root.

mod cell;
mod checkpoint;
mod chunk;
mod displacement;
mod edit;
mod element;
mod gas;
mod grid;
mod heat;
mod liquid;
mod phase_transition;
mod region;
mod settings;
mod settled;
mod simulation;
mod transfer;

/// The revision of the simulation's rules, as far as a World save depends on them: what a
/// [`Checkpoint`] means, and the units and invariants of the Cells it holds. A save made under
/// another revision is refused rather than misread (`docs/plans/17-world-save-storage.md`).
///
/// Bump this when a checkpoint captured before a change would be restored wrongly after it: say,
/// Mass changes units, a Chunk changes size, or a field of [`Checkpoint`] changes meaning. The
/// Element table and settings a World runs with are saved alongside it, and a save checks them
/// separately.
pub const RULES_REVISION: u32 = 1;

pub use cell::{Cell, CellPos};
pub use checkpoint::{Checkpoint, CheckpointError, ChunkCheckpoint, ChunkContents};
pub use chunk::{CHUNK_SIZE, ChunkPos};
pub use edit::Edit;
pub use element::{Element, ElementError, ElementId, ElementTable, Phase, Side, Transition};
pub use region::{ActiveRegion, WORLD_BOTTOM_Y, WORLD_TOP_Y};
pub use settings::{SettingsError, SimSettings};
pub use simulation::{ChunkSource, SimStats, Simulation};
