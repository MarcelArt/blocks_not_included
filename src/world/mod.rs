//! The game-side copy of the simulated world, which every game system reads from.

mod components;
mod view;

pub use view::WorldView;
pub(crate) use view::locate;
