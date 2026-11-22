//! The Bevy-free simulation core: Cells, Elements, Chunks and the Tick.
//!
//! Vocabulary is defined in `CONTEXT.md` at the repository root.

mod cell;
mod chunk;
mod element;
mod heat;
mod simulation;

pub use cell::{Cell, CellPos};
pub use element::{Element, ElementError, ElementId, ElementTable, Phase, Side, Transition};
pub use simulation::Simulation;
