//! The camera's field of view, which widens while the body runs.

use bevy::prelude::Component;

/// The vertical field of view, in degrees.
pub const FIELD_OF_VIEW: f32 = 70.0;

/// The vertical field of view while running, in degrees.
pub const RUN_FIELD_OF_VIEW: f32 = 76.0;

/// How long the view takes to widen fully once running starts, or to narrow back once it ends,
/// in seconds.
pub const RUN_VIEW_EASE: f32 = 0.25;

/// How far the view has widened for running, eased in and out so it neither starts nor stops
/// with a jolt. Only the field of view changes: the camera stays at the body's eyes.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct RunView {
    /// From 0, [`FIELD_OF_VIEW`], to 1, [`RUN_FIELD_OF_VIEW`], before easing.
    progress: f32,
}

impl RunView {
    /// Widens the view for `dt` seconds while `running`, or narrows it otherwise.
    pub fn update(&mut self, running: bool, dt: f32) {
        let step = dt / RUN_VIEW_EASE;
        self.progress = if running {
            (self.progress + step).min(1.0)
        } else {
            (self.progress - step).max(0.0)
        };
    }

    /// The vertical field of view, in degrees.
    pub fn field_of_view(&self) -> f32 {
        let t = self.progress;
        // Smoothstep: as slow at either end as it is quick in the middle.
        let eased = t * t * (3.0 - 2.0 * t);
        FIELD_OF_VIEW + (RUN_FIELD_OF_VIEW - FIELD_OF_VIEW) * eased
    }
}
