//! Rebinding the controls (`docs/plans/20-settings-and-bindings.md`, seam 2), through the Settings
//! page's editor as the page drives it: reserved keys and fixed shortcuts are refused; a key
//! another action uses at the same time asks before it is taken, and actions that never apply at
//! once may share one; cancelling and resetting; and the hints that name the controls.

use bevy::prelude::{KeyCode, MouseButton};
use blocks_not_included::overlay::cycle_hint;
use blocks_not_included::settings::{
    Action, AutosaveInterval, Bindings, Context, DisplayMode, FIXED_SHORTCUTS, Input, Offered,
    RESERVED_KEYS, Rebinding, Refusal, Settings, SettingsEditor,
};
use blocks_not_included::tools::{Tool, tool_hint};

fn key(code: KeyCode) -> Input {
    Input::Key(code)
}

fn mouse(button: MouseButton) -> Input {
    Input::Mouse(button)
}

fn fresh_editor() -> SettingsEditor {
    SettingsEditor::new(&Settings::default())
}

#[test]
fn the_defaults_bind_every_action_and_conflict_nowhere() {
    let bindings = Bindings::default();
    assert_eq!(bindings.check(), Ok(()));
    for action in Action::ALL {
        assert!(!bindings.inputs(action).is_empty(), "{action:?} is bound");
    }
    let labels: Vec<(Action, String)> = [
        Action::MoveForward,
        Action::Jump,
        Action::Descend,
        Action::ChooseDig,
        Action::Dig,
        Action::Place,
        Action::CycleOverlay,
    ]
    .into_iter()
    .map(|action| (action, bindings.label(action)))
    .collect();
    assert_eq!(
        labels,
        [
            (Action::MoveForward, "W".to_owned()),
            (Action::Jump, "Space".to_owned()),
            // Both Shift keys, named as one.
            (Action::Descend, "Shift".to_owned()),
            (Action::ChooseDig, "1".to_owned()),
            (Action::Dig, "Left Mouse".to_owned()),
            (Action::Place, "Right Mouse".to_owned()),
            (Action::CycleOverlay, "O".to_owned()),
        ]
    );
    // Dig and the heat brush share the left button: only one tool is ever in hand.
    assert_eq!(
        bindings.inputs(Action::Dig),
        bindings.inputs(Action::HeatBrush)
    );
}

#[test]
fn esc_alt_and_the_fixed_shortcuts_are_refused_and_the_editor_keeps_listening() {
    let mut editor = fresh_editor();
    editor.listen(Action::Jump);
    for (code, purpose) in RESERVED_KEYS {
        let refusal = Refusal::Reserved {
            input: key(code),
            purpose,
        };
        assert_eq!(editor.offer(key(code)), Offered::Refused(refusal));
    }
    for (code, purpose) in FIXED_SHORTCUTS {
        let refusal = Refusal::Fixed {
            input: key(code),
            purpose,
        };
        assert_eq!(editor.offer(key(code)), Offered::Refused(refusal));
        assert_eq!(
            *editor.rebinding(),
            Rebinding::Listening {
                action: Action::Jump,
                refused: Some(refusal),
            }
        );
    }
    let fixed: Vec<KeyCode> = FIXED_SHORTCUTS.iter().map(|(code, _)| *code).collect();
    assert_eq!(
        fixed,
        [KeyCode::F3, KeyCode::KeyP, KeyCode::KeyN, KeyCode::KeyF],
        "the debug panel, its clock and step, and flying"
    );
    assert_eq!(editor.draft, Settings::default(), "nothing was bound");
    assert_eq!(
        Refusal::of(key(KeyCode::Escape))
            .expect("refused")
            .to_string(),
        "Esc is reserved for pausing"
    );

    // Still listening, it takes the next input that can be bound.
    assert_eq!(editor.offer(key(KeyCode::KeyJ)), Offered::Bound);
    assert_eq!(editor.draft.bindings.inputs(Action::Jump), [key(KeyCode::KeyJ)]);
    assert_eq!(*editor.rebinding(), Rebinding::Idle);
}

#[test]
fn an_input_another_action_uses_at_the_same_time_asks_before_it_is_taken() {
    let mut editor = fresh_editor();

    // O cycles the Overlays whatever the player is doing, as Jump applies.
    editor.listen(Action::Jump);
    assert_eq!(editor.offer(key(KeyCode::KeyO)), Offered::Conflict);
    assert_eq!(
        *editor.rebinding(),
        Rebinding::Conflict {
            action: Action::Jump,
            inputs: vec![key(KeyCode::KeyO)],
            taken_from: vec![Action::CycleOverlay],
        }
    );
    assert_eq!(editor.draft, Settings::default(), "nothing changes yet");
    editor.replace();
    let bindings = &editor.draft.bindings;
    assert_eq!(bindings.inputs(Action::Jump), [key(KeyCode::KeyO)]);
    assert_eq!(bindings.label(Action::CycleOverlay), "Unbound");
    assert_eq!(bindings.check(), Ok(()));
    assert_eq!(*editor.rebinding(), Rebinding::Idle);

    // Dig applies while walking forward. Taking W leaves Move forward its other key.
    editor
        .draft
        .bindings
        .set(Action::MoveForward, vec![key(KeyCode::KeyW), key(KeyCode::ArrowUp)]);
    editor.listen(Action::Dig);
    assert_eq!(editor.offer(key(KeyCode::KeyW)), Offered::Conflict);
    editor.replace();
    let bindings = &editor.draft.bindings;
    assert_eq!(bindings.inputs(Action::Dig), [key(KeyCode::KeyW)]);
    assert_eq!(bindings.label(Action::MoveForward), "Up Arrow");

    // Fly down applies in flight with any tool in hand, so the right button is Place's and Spawn
    // fluid's at the same time.
    let mut editor = fresh_editor();
    editor.listen(Action::Descend);
    assert_eq!(editor.offer(mouse(MouseButton::Right)), Offered::Conflict);
    let Rebinding::Conflict { taken_from, .. } = editor.rebinding() else {
        panic!("a conflict");
    };
    assert_eq!(*taken_from, [Action::Place, Action::SpawnFluid]);
}

#[test]
fn actions_that_never_apply_at_once_may_share_an_input() {
    let mut editor = fresh_editor();

    // Place takes the left button that Dig and the heat brush use, without asking: no two of
    // them are ever in hand at once.
    editor.listen(Action::Place);
    assert_eq!(editor.offer(mouse(MouseButton::Left)), Offered::Bound);
    let bindings = &editor.draft.bindings;
    for action in [Action::Dig, Action::HeatBrush, Action::Place] {
        assert_eq!(bindings.inputs(action), [mouse(MouseButton::Left)], "{action:?}");
    }
    assert_eq!(bindings.check(), Ok(()));

    // Movement modes work the same way: an action only used in flight never applies with one
    // only used on foot (step 21's Sneak shares Shift with Fly down), but either applies with an
    // action used in both.
    assert!(!Context::flying().overlaps(Context::walking()));
    assert!(Context::flying().overlaps(Context::ALWAYS));
    assert!(Context::walking().overlaps(Context::holding(Tool::Dig)));
    assert!(!Context::holding(Tool::Dig).overlaps(Context::holding(Tool::Place)));
    // In flight with Dig in hand, both apply.
    assert!(Context::flying().overlaps(Context::holding(Tool::Dig)));
    assert_eq!(Action::Descend.context(), Context::flying());
}

#[test]
fn either_shift_ctrl_or_system_key_binds_both() {
    let mut editor = fresh_editor();
    editor.listen(Action::CycleOverlay);
    assert_eq!(editor.offer(key(KeyCode::ControlRight)), Offered::Bound);
    let bindings = &editor.draft.bindings;
    assert_eq!(
        bindings.inputs(Action::CycleOverlay),
        [key(KeyCode::ControlLeft), key(KeyCode::ControlRight)]
    );
    assert_eq!(bindings.label(Action::CycleOverlay), "Ctrl");

    // Jump applies in flight, where Shift flies down: both Shift keys conflict.
    editor.listen(Action::Jump);
    assert_eq!(editor.offer(key(KeyCode::ShiftLeft)), Offered::Conflict);
    let Rebinding::Conflict { inputs, .. } = editor.rebinding() else {
        panic!("a conflict");
    };
    assert_eq!(*inputs, [key(KeyCode::ShiftLeft), key(KeyCode::ShiftRight)]);
}

#[test]
fn cancelling_keeps_every_binding_as_it_was() {
    let mut editor = fresh_editor();

    // While listening.
    editor.listen(Action::Jump);
    editor.cancel();
    assert_eq!(*editor.rebinding(), Rebinding::Idle);
    assert_eq!(editor.offer(key(KeyCode::KeyJ)), Offered::Ignored);
    assert_eq!(editor.draft, Settings::default());

    // At a conflict.
    editor.listen(Action::Jump);
    assert_eq!(editor.offer(key(KeyCode::Digit1)), Offered::Conflict);
    editor.cancel();
    assert_eq!(*editor.rebinding(), Rebinding::Idle);
    assert_eq!(editor.draft, Settings::default());

    // Listening for another action drops the conflict too.
    editor.listen(Action::Jump);
    assert_eq!(editor.offer(key(KeyCode::Digit1)), Offered::Conflict);
    editor.listen(Action::Dig);
    editor.replace();
    assert_eq!(editor.draft, Settings::default());

    // Offering the input it already has changes nothing.
    editor.listen(Action::Jump);
    assert_eq!(editor.offer(key(KeyCode::Space)), Offered::Bound);
    assert_eq!(editor.draft, Settings::default());
}

#[test]
fn reset_to_defaults_restores_every_setting_in_the_draft() {
    let mut editor = fresh_editor();
    editor.draft.display.mode = DisplayMode::BorderlessFullscreen;
    editor.draft.mouse_sensitivity = 3.0;
    editor.draft.autosave = AutosaveInterval::FiveMinutes;
    editor.listen(Action::Jump);
    editor.offer(key(KeyCode::KeyJ));
    editor.listen(Action::Dig);
    assert_ne!(editor.draft, Settings::default());

    editor.reset_to_defaults();
    assert_eq!(editor.draft, Settings::default());
    assert_eq!(*editor.rebinding(), Rebinding::Idle, "and stops listening");
}

#[test]
fn the_hints_name_the_controls_as_bound() {
    let mut bindings = Bindings::default();
    assert_eq!(
        tool_hint(Tool::Dig, &bindings),
        "Left Mouse digs out the Cell in view."
    );
    assert_eq!(
        tool_hint(Tool::HeatBrush, &bindings),
        "Hold Left Mouse to heat the Cells around the one in view. Below 0 K/s it cools them."
    );
    assert_eq!(cycle_hint(&bindings), "O cycles the Overlays.");

    bindings.set(Action::Dig, vec![key(KeyCode::KeyG)]);
    bindings.set(Action::Place, vec![mouse(MouseButton::Back)]);
    bindings.set(Action::CycleOverlay, vec![key(KeyCode::Tab)]);
    assert_eq!(tool_hint(Tool::Dig, &bindings), "G digs out the Cell in view.");
    assert_eq!(
        tool_hint(Tool::Place, &bindings),
        "Mouse 4 places it against the face in view."
    );
    assert_eq!(cycle_hint(&bindings), "Tab cycles the Overlays.");

    bindings.set(Action::SpawnFluid, Vec::new());
    bindings.set(Action::CycleOverlay, Vec::new());
    assert_eq!(
        tool_hint(Tool::SpawnFluid, &bindings),
        "“Spawn fluid” is unbound. Bind it in Settings."
    );
    assert_eq!(
        cycle_hint(&bindings),
        "“Cycle the Overlays” is unbound. Bind it in Settings."
    );
}
