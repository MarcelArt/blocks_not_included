//! The game: everything that uses Bevy. The simulation itself is the Bevy-free `sim` crate, run on
//! a thread of its own (ADR 0003).
//!
//! Vocabulary is defined in `CONTEXT.md` at the repository root.

pub mod player;
pub mod render;
pub mod sim_bridge;
pub mod tools;
pub mod world;
