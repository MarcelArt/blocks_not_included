//! Switching flying on and off with two quick presses of Jump, decided apart from Bevy's input
//! events: the game reads each frame's Jump into a [`JumpFrame`], and [`FlightToggle`] says when
//! flying switches.
//!
//! A press is Jump going down while none of its inputs is held, so holding Jump, or pressing
//! another of its keys while one is held, counts once. Two presses at most
//! [`DOUBLE_PRESS_WINDOW`] apart switch flying, in either direction, and are used up: a third
//! press starts again. Holding Jump still jumps, swims or flies up as ever.
//!
//! A first press is forgotten while the body swims, where Jump only swims; without gameplay input
//! (Alt held, or the window without focus); when Jump is rebound; and on [`reset`], which the
//! game calls as the World pauses or closes.
//!
//! [`reset`]: FlightToggle::reset

use std::time::Duration;

use bevy::prelude::Component;

use crate::settings::Input;

/// The longest time from one press of Jump to the next that switches flying.
pub const DOUBLE_PRESS_WINDOW: Duration = Duration::from_millis(300);

/// What Jump did during one frame.
#[derive(Clone, Copy, Debug)]
pub struct JumpFrame<'a> {
    /// When the frame ran, on a clock that never goes back.
    pub now: Duration,
    /// The inputs Jump is bound to.
    pub bound: &'a [Input],
    /// Whether one of them went down during the frame.
    pub pressed: bool,
    /// Whether one of them is down at the frame's end.
    pub held: bool,
    /// Whether input is gameplay input (`crate::input_policy`).
    pub gameplay: bool,
    /// Whether the body is swimming, so that Jump swims.
    pub swimming: bool,
}

/// Counts presses of Jump to switch flying. It lives on the player, so it goes with the World.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct FlightToggle {
    /// When the press that may start a double press went down, if one may.
    first: Option<Duration>,
    /// Whether Jump was held at the end of the last frame read.
    held: bool,
    /// What Jump was bound to then.
    bound: Vec<Input>,
}

impl FlightToggle {
    /// Reads one frame of Jump. Returns whether flying switches.
    pub fn read(&mut self, frame: JumpFrame) -> bool {
        if frame.bound != self.bound.as_slice() {
            self.bound = frame.bound.to_vec();
            self.first = None;
        }
        let was_held = std::mem::replace(&mut self.held, frame.held);
        if !frame.gameplay || frame.swimming {
            self.first = None;
            return false;
        }
        if !frame.pressed || was_held {
            return false;
        }
        match self.first.take() {
            Some(first) if frame.now.saturating_sub(first) <= DOUBLE_PRESS_WINDOW => true,
            _ => {
                self.first = Some(frame.now);
                false
            }
        }
    }

    /// Forgets a first press, as the World pauses or closes.
    pub fn reset(&mut self) {
        self.first = None;
        self.held = false;
    }
}
