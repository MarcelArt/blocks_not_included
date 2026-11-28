//! The Settings page's draft, and the rebinding of a control in it, apart from any window, so that
//! tests drive it as the page does.
//!
//! The page edits a copy of the applied [`Settings`]: nothing changes until the player applies
//! it. To rebind an action, the page [`listen`](SettingsEditor::listen)s for the next key or mouse
//! button, which the game [`offer`](SettingsEditor::offer)s it. An input that can't be bound is
//! refused, and the page keeps listening. One that another action already uses in an overlapping
//! context asks first: the player [`replace`](SettingsEditor::replace)s that binding, or
//! [`cancel`](SettingsEditor::cancel)s and keeps both as they were.

use super::Settings;
use super::bindings::{Action, Input, Refusal};

/// The Settings page's copy of the settings, and the rebinding under way.
#[derive(Clone, Debug, PartialEq)]
pub struct SettingsEditor {
    /// The settings as the page shows them, applied or not.
    pub draft: Settings,
    rebinding: Rebinding,
}

/// Where the rebinding of a control has got to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rebinding {
    /// None under way.
    Idle,
    /// Waiting for the input to bind `action` to. `refused` says why the last one offered can't
    /// be bound, if it can't.
    Listening {
        action: Action,
        refused: Option<Refusal>,
    },
    /// `inputs` are bound to the actions `taken_from` already, in contexts that overlap
    /// `action`'s. The player decides whether to take them.
    Conflict {
        action: Action,
        inputs: Vec<Input>,
        taken_from: Vec<Action>,
    },
}

/// What became of an input offered to the editor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Offered {
    /// Nothing was listening for it.
    Ignored,
    /// The action was bound to it in the draft.
    Bound,
    /// It can't be bound. The editor keeps listening.
    Refused(Refusal),
    /// Other actions use it at the same time. The editor waits for the player to decide.
    Conflict,
}

impl SettingsEditor {
    /// A draft of `applied`, with no rebinding under way.
    pub fn new(applied: &Settings) -> Self {
        Self {
            draft: applied.clone(),
            rebinding: Rebinding::Idle,
        }
    }

    pub fn rebinding(&self) -> &Rebinding {
        &self.rebinding
    }

    /// Whether the editor is waiting for an input to bind.
    pub fn is_listening(&self) -> bool {
        matches!(self.rebinding, Rebinding::Listening { .. })
    }

    /// Waits for the input to bind `action` to, dropping any other rebinding under way.
    pub fn listen(&mut self, action: Action) {
        self.rebinding = Rebinding::Listening {
            action,
            refused: None,
        };
    }

    /// Offers `input`, pressed while the editor listens, to bind the action it listens for. A left
    /// or right Shift, Ctrl or system key binds both.
    pub fn offer(&mut self, input: Input) -> Offered {
        let Rebinding::Listening { action, .. } = self.rebinding else {
            return Offered::Ignored;
        };
        if let Some(refusal) = Refusal::of(input) {
            self.rebinding = Rebinding::Listening {
                action,
                refused: Some(refusal),
            };
            return Offered::Refused(refusal);
        }
        let inputs = input.with_twin();
        let bindings = &mut self.draft.bindings;
        let taken_from = bindings.conflicts(action, &inputs);
        if taken_from.is_empty() {
            bindings.set(action, inputs);
            self.rebinding = Rebinding::Idle;
            Offered::Bound
        } else {
            self.rebinding = Rebinding::Conflict {
                action,
                inputs,
                taken_from,
            };
            Offered::Conflict
        }
    }

    /// Resolves a conflict by binding the action anyway: the actions that had its inputs lose
    /// them, and keep any others.
    pub fn replace(&mut self) {
        let Rebinding::Conflict {
            action,
            inputs,
            taken_from,
        } = std::mem::replace(&mut self.rebinding, Rebinding::Idle)
        else {
            return;
        };
        let bindings = &mut self.draft.bindings;
        for other in taken_from {
            for input in &inputs {
                bindings.remove(other, *input);
            }
        }
        bindings.set(action, inputs);
    }

    /// Stops listening, or resolves a conflict by keeping every binding as it was.
    pub fn cancel(&mut self) {
        self.rebinding = Rebinding::Idle;
    }

    /// Sets every setting in the draft to its default, and drops any rebinding under way.
    pub fn reset_to_defaults(&mut self) {
        self.draft = Settings::default();
        self.rebinding = Rebinding::Idle;
    }
}
