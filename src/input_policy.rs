//! Who the keyboard and mouse are for. Each frame, before movement, the god tools or any key the
//! game handles reads them, [`InputPolicy`] says whether they are gameplay input, and the cursor
//! follows.
//!
//! - **Gameplay**: while the World runs, its window has focus, and Alt isn't held. The cursor is
//!   captured: hidden, and locked to the window. The mouse looks around, the keys move, and the
//!   buttons use the god tools. egui's windows let clicks pass through.
//! - **Esc** pauses the World, and resumes it again (`crate::session::Play`). While paused, the
//!   cursor is free and nothing reaches the World.
//! - **Holding Alt** frees the cursor, for the debug panel and the palette. Movement, mouse look
//!   and the god tools take no input, but physics and the debug clock carry on.
//! - **Without focus** the cursor stays free, whatever else holds.
//!
//! When gameplay starts again, as Alt is released, the World resumes, or the window regains focus,
//! the cursor is captured again, and the clicks and mouse motion from while it was free are
//! dropped: a mouse button still held from then counts as up until it is pressed again. When
//! gameplay stops, it stops on the same frame, though the window applies the cursor change only
//! at the end of it.

use bevy::input::InputSystems;
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use bevy_egui::input::EguiWantsInput;

use crate::session::{PauseControl, Play};

/// The key that pauses the World, and resumes it.
pub const PAUSE_KEY: KeyCode = KeyCode::Escape;

/// The keys that free the cursor while held.
pub const FREE_CURSOR_KEYS: [KeyCode; 2] = [KeyCode::AltLeft, KeyCode::AltRight];

/// Whether this frame's keyboard and mouse input is gameplay input. Set in `PreUpdate`, before
/// anything else reads input.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputPolicy {
    /// Movement, mouse look and the god tools take input, and the cursor is captured: the World
    /// runs, its window has focus, and Alt isn't held.
    pub gameplay: bool,
}

/// A run condition: whether this frame's input is gameplay input.
pub fn gameplay_input(policy: Res<InputPolicy>) -> bool {
    policy.gameplay
}

/// The system that sets [`InputPolicy`], in `PreUpdate`, once Bevy has read the frame's input.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct InputPolicySystems;

/// Esc, Alt, window focus and the cursor. Needs Bevy's input, and the session plugin.
pub struct InputPolicyPlugin;

impl Plugin for InputPolicyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<InputPolicy>().add_systems(
            PreUpdate,
            apply_input_policy
                .in_set(InputPolicySystems)
                .after(InputSystems),
        );
    }
}

/// Pauses or resumes on Esc, decides whether input is gameplay input, captures or frees the
/// cursor to match, and drops stale clicks and mouse motion as gameplay starts again.
fn apply_input_policy(
    keys: Res<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut motion: ResMut<AccumulatedMouseMotion>,
    ui: Option<Res<EguiWantsInput>>,
    mut pause: PauseControl,
    window: Option<Single<(&Window, &mut CursorOptions), With<PrimaryWindow>>>,
    mut policy: ResMut<InputPolicy>,
) {
    // Esc in a text field is egui's, to leave it.
    let typing = ui.is_some_and(|ui| ui.wants_any_keyboard_input());
    if keys.just_pressed(PAUSE_KEY) && !typing {
        pause.toggle();
    }
    let focused = window.as_ref().is_none_or(|window| window.0.focused);
    let gameplay =
        pause.state() == Some(Play::Running) && focused && !keys.any_pressed(FREE_CURSOR_KEYS);
    if gameplay && !policy.gameplay {
        mouse.reset_all();
        motion.delta = Vec2::ZERO;
    }
    policy.set_if_neq(InputPolicy { gameplay });
    if let Some(window) = window {
        let (_, mut cursor) = window.into_inner();
        let (grab_mode, visible) = if gameplay {
            (CursorGrabMode::Locked, false)
        } else {
            (CursorGrabMode::None, true)
        };
        if cursor.grab_mode != grab_mode || cursor.visible != visible {
            cursor.grab_mode = grab_mode;
            cursor.visible = visible;
        }
    }
}
