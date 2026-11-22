//! The Bevy-free simulation core: Cells, Elements, Chunks and the Tick.
//!
//! Vocabulary is defined in `CONTEXT.md` at the repository root.

mod cell;
mod chunk;
mod displacement;
mod edit;
mod element;
mod gas;
mod heat;
mod liquid;
mod phase_transition;
mod settings;
mod settled;
mod simulation;
mod transfer;

pub use cell::{Cell, CellPos};
pub use chunk::{CHUNK_SIZE, ChunkPos};
pub use edit::Edit;
pub use element::{Element, ElementError, ElementId, ElementTable, Phase, Side, Transition};
pub use settings::{SettingsError, SimSettings};
pub use simulation::{SimStats, Simulation};
