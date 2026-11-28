//! Switching flying on and off with two presses of Jump (`docs/plans/21-movement-controls.md`,
//! seam 1), apart from Bevy's input: the 300 ms boundary, a held Jump, presses too far apart, a
//! rebound Jump, and the presses forgotten while swimming, without gameplay input, or over a
//! pause. Then the flight controls on either side of a switch.

use bevy::math::{IVec3, Vec3};
use bevy::prelude::KeyCode;
use blocks_not_included::player::flight::{DOUBLE_PRESS_WINDOW, FlightToggle, JumpFrame};
use blocks_not_included::player::movement::{
    Body, FLY_SPEED, Gait, Matter, MoveInput, MoveMode, SNEAK_SPEED, advance, gait,
};
use blocks_not_included::settings::Input;
use std::time::Duration;

const SPACE: Input = Input::Key(KeyCode::Space);
const J: Input = Input::Key(KeyCode::KeyJ);

/// The player's hands on the bound Jump, one frame at a time, as the game feeds them to a
/// [`FlightToggle`].
struct Hands {
    toggle: FlightToggle,
    bound: Vec<Input>,
    swimming: bool,
    gameplay: bool,
}

impl Hands {
    fn new() -> Self {
        Self {
            toggle: FlightToggle::default(),
            bound: vec![SPACE],
            swimming: false,
            gameplay: true,
        }
    }

    /// One frame at `ms` on the clock: whether a bound input went down during it, and whether one
    /// is down at its end. Returns whether flying switches.
    fn frame(&mut self, ms: u64, pressed: bool, held: bool) -> bool {
        self.toggle.read(JumpFrame {
            now: Duration::from_millis(ms),
            bound: &self.bound,
            pressed,
            held,
            gameplay: self.gameplay,
            swimming: self.swimming,
        })
    }

    fn press(&mut self, ms: u64) -> bool {
        self.frame(ms, true, true)
    }

    fn hold(&mut self, ms: u64) -> bool {
        self.frame(ms, false, true)
    }

    fn release(&mut self, ms: u64) -> bool {
        self.frame(ms, false, false)
    }
}

#[test]
fn two_presses_up_to_300_ms_apart_switch_flying() {
    assert_eq!(DOUBLE_PRESS_WINDOW, Duration::from_millis(300));

    // Exactly 300 ms from the first press to the second switches.
    let mut hands = Hands::new();
    assert!(!hands.press(1_000));
    assert!(!hands.release(1_100));
    assert!(hands.press(1_300));

    // 301 ms doesn't: the second press is a first press of its own, and a third press 299 ms
    // after it switches.
    let mut hands = Hands::new();
    assert!(!hands.press(1_000));
    assert!(!hands.release(1_100));
    assert!(!hands.press(1_301));
    assert!(!hands.release(1_400));
    assert!(hands.press(1_600));
}

#[test]
fn a_switch_uses_up_both_presses_so_a_third_starts_again() {
    let mut hands = Hands::new();
    assert!(!hands.press(0));
    assert!(!hands.release(50));
    assert!(hands.press(100), "on");
    assert!(!hands.release(150));
    // The third press, 100 ms after the second, is a first press.
    assert!(!hands.press(200));
    assert!(!hands.release(250));
    // And the fourth switches back.
    assert!(hands.press(300), "off");
}

#[test]
fn held_jump_counts_once_however_long_and_however_many_of_its_keys_go_down() {
    // Holding the one key: never a second press.
    let mut hands = Hands::new();
    assert!(!hands.press(0));
    for ms in (20..=1_000).step_by(20) {
        assert!(!hands.hold(ms), "held at {ms} ms");
    }

    // Jump bound to Space and J: J going down while Space is held isn't a press of Jump.
    let mut hands = Hands::new();
    hands.bound = vec![SPACE, J];
    assert!(!hands.press(0));
    assert!(!hands.frame(100, true, true), "J while Space is held");
    assert!(!hands.release(150));
    // A press after a release is: 200 ms after the first.
    assert!(hands.press(200));

    // A long hold doesn't count towards the window: the next press, 450 ms after the first,
    // is a first press.
    let mut hands = Hands::new();
    assert!(!hands.press(0));
    assert!(!hands.hold(300));
    assert!(!hands.release(400));
    assert!(!hands.press(450));
}

#[test]
fn a_press_and_release_within_one_frame_counts() {
    let mut hands = Hands::new();
    // Down and up again before the frame ended: pressed, but not held.
    assert!(!hands.frame(0, true, false));
    assert!(hands.frame(100, true, false));
}

#[test]
fn rebinding_jump_forgets_a_first_press() {
    let mut hands = Hands::new();
    assert!(!hands.press(0));
    assert!(!hands.release(50));
    hands.bound = vec![J];
    assert!(!hands.press(100), "the press of Space is forgotten");
    assert!(!hands.release(150));
    assert!(hands.press(200), "two presses of J switch");
}

#[test]
fn swimming_takes_jump_as_swim_input_and_forgets_a_first_press() {
    // A press on land, then into the water.
    let mut hands = Hands::new();
    assert!(!hands.press(0));
    assert!(!hands.release(50));
    hands.swimming = true;
    assert!(!hands.release(60));
    hands.swimming = false;
    assert!(!hands.press(100), "the press on land was forgotten");
    assert!(!hands.release(150));

    // Presses while swimming swim, and never switch.
    let mut hands = Hands::new();
    hands.swimming = true;
    for ms in (0..1_000).step_by(100) {
        assert!(!hands.press(ms), "swimming at {ms} ms");
        assert!(!hands.release(ms + 50));
    }
    // Out of the water, a press after one made while swimming is a first press.
    let mut hands = Hands::new();
    hands.swimming = true;
    assert!(!hands.press(0));
    assert!(!hands.release(50));
    hands.swimming = false;
    assert!(!hands.press(100));
    assert!(!hands.release(150));
    assert!(hands.press(200));
}

#[test]
fn losing_gameplay_input_or_a_reset_forgets_a_first_press() {
    // Alt held, or the window losing focus, between the presses.
    let mut hands = Hands::new();
    assert!(!hands.press(0));
    assert!(!hands.release(50));
    hands.gameplay = false;
    assert!(!hands.release(60));
    hands.gameplay = true;
    assert!(!hands.press(100));

    // Without gameplay input, presses never count.
    let mut hands = Hands::new();
    hands.gameplay = false;
    assert!(!hands.press(0));
    assert!(!hands.release(50));
    assert!(!hands.press(100));

    // Pausing, or leaving the World, resets it.
    let mut hands = Hands::new();
    assert!(!hands.press(0));
    assert!(!hands.release(50));
    hands.toggle.reset();
    assert!(!hands.press(100));
    assert!(!hands.release(150));
    assert!(hands.press(200));
}

/// Solid ground at y ≤ 0, whose top is at y = 1, and gas above.
fn ground(cell: IVec3) -> Matter {
    if cell.y <= 0 {
        Matter::Solid
    } else {
        Matter::Open
    }
}

/// Asserts that `actual` is `expected` to within 0.01 mm, the rounding of a few f32 steps.
#[track_caller]
fn assert_near(actual: Vec3, expected: Vec3) {
    assert!(
        actual.abs_diff_eq(expected, 1e-5),
        "expected {expected}, got {actual}"
    );
}

#[test]
fn either_side_of_a_switch_shift_means_what_the_mode_says() {
    // Shift is bound to both Sneak and Fly down by default, so both are held.
    let shift = MoveInput {
        direction: Vec3::X,
        down: true,
        sneak: true,
        ..MoveInput::default()
    };
    let standing = Body {
        grounded: true,
        ..Body::standing_at(Vec3::new(0.5, 1.0, 0.5))
    };

    // Walking, it sneaks: 1.5 m/s for 0.1 s.
    assert_eq!(gait(&standing, &shift, ground), Gait::Sneak);
    let sneaked = advance(standing, shift, 0.1, ground);
    assert_near(sneaked.feet, Vec3::new(0.5 + SNEAK_SPEED * 0.1, 1.0, 0.5));

    // Flying, it flies down, through the ground, at 12 m/s, and forward as fast.
    let flying = Body {
        mode: MoveMode::Fly,
        velocity: Vec3::ZERO,
        grounded: false,
        ..standing
    };
    assert_eq!(gait(&flying, &shift, ground), Gait::Fly);
    let flown = advance(flying, shift, 0.1, ground);
    assert_near(
        flown.feet,
        Vec3::new(0.5 + FLY_SPEED * 0.1, 1.0 - FLY_SPEED * 0.1, 0.5),
    );

    // Jump flies up.
    let up = MoveInput {
        up: true,
        ..MoveInput::default()
    };
    let risen = advance(flying, up, 0.1, ground);
    assert_near(risen.feet, Vec3::new(0.5, 1.0 + FLY_SPEED * 0.1, 0.5));
}
