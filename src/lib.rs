//! The game: everything that uses Bevy. The simulation itself is the Bevy-free `sim` crate, run on
//! a thread of its own (ADR 0003).
//!
//! Vocabulary is defined in `CONTEXT.md` at the repository root.

pub mod debug_panel;
pub mod overlay;
pub mod player;
pub mod readout;
pub mod render;
pub mod sim_bridge;
pub mod sim_files;
pub mod tools;
pub mod world;
