//! The game-side copy of the simulated world, which every game system reads from.

mod components;
mod liquid;
mod view;

pub use liquid::{MIN_SURFACE, capacity_grams, liquid_fill};
pub use view::WorldView;
pub(crate) use view::locate;
