//! The game: everything that uses Bevy. The simulation itself is the Bevy-free `sim` crate, run on
//! a thread of its own (ADR 0003).
//!
//! Vocabulary is defined in `CONTEXT.md` at the repository root.

mod atomic_write;
pub mod debug_panel;
pub mod input_policy;
pub mod lifecycle;
pub mod menu;
pub mod overlay;
pub mod player;
pub mod readout;
pub mod render;
pub mod saves;
pub mod session;
pub mod settings;
pub mod sim_bridge;
pub mod sim_files;
pub mod tools;
pub mod user_dirs;
pub mod world;
