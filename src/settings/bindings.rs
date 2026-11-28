//! The gameplay controls: named [`Action`]s, and the keys and mouse buttons bound to each.
//!
//! Every control the game reads during play goes through [`Bindings`], and so does every hint that
//! names one, so a control rebound in Settings changes both.
//!
//! # Conflicts
//!
//! Each action applies in a [`Context`]: in which movement modes, and with which god tool in hand.
//! Two actions conflict when they share an input and their contexts overlap, so that one press
//! would do both. Actions that never apply at once may share an input. By default, Dig and the
//! heat brush share the left mouse button, as only one tool is ever in hand, and Fly down, used
//! only in flight, shares Shift with Sneak, used only on foot.
//!
//! Some keys can't be bound at all. Esc pauses and Alt frees the cursor ([`RESERVED_KEYS`]). The
//! debug panel's keys keep their keys too ([`FIXED_SHORTCUTS`]): they work in every context, so
//! any action bound to one would conflict with it.

use std::fmt;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use super::Settings;
use crate::debug_panel;
use crate::input_policy::{FREE_CURSOR_KEYS, PAUSE_KEY};
use crate::player::movement::MoveMode;
use crate::tools::Tool;

/// A key or a mouse button.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Input {
    Key(KeyCode),
    Mouse(MouseButton),
}

impl Input {
    /// How the menus and hints name it: "W", "Space", "Left Shift", "Left Mouse".
    pub fn label(self) -> String {
        match self {
            Input::Key(key) => key_label(key),
            Input::Mouse(button) => match button {
                MouseButton::Left => "Left Mouse".to_owned(),
                MouseButton::Right => "Right Mouse".to_owned(),
                MouseButton::Middle => "Middle Mouse".to_owned(),
                MouseButton::Back => "Mouse 4".to_owned(),
                MouseButton::Forward => "Mouse 5".to_owned(),
                MouseButton::Other(n) => format!("Mouse {n}"),
            },
        }
    }

    /// The inputs a press of this one binds: both Shift keys for either, and the same for Ctrl
    /// and the system keys, as the defaults bind them.
    pub fn with_twin(self) -> Vec<Input> {
        match self {
            Input::Key(key) => match twin(key) {
                Some(other) => {
                    let mut pair = [key, other];
                    pair.sort_by_key(|key| twin_order(*key));
                    pair.map(Input::Key).to_vec()
                }
                None => vec![self],
            },
            Input::Mouse(_) => vec![self],
        }
    }
}

impl fmt::Display for Input {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label())
    }
}

/// The left or right key that a binding treats as the same key as `key`.
fn twin(key: KeyCode) -> Option<KeyCode> {
    Some(match key {
        KeyCode::ShiftLeft => KeyCode::ShiftRight,
        KeyCode::ShiftRight => KeyCode::ShiftLeft,
        KeyCode::ControlLeft => KeyCode::ControlRight,
        KeyCode::ControlRight => KeyCode::ControlLeft,
        KeyCode::SuperLeft => KeyCode::SuperRight,
        KeyCode::SuperRight => KeyCode::SuperLeft,
        _ => return None,
    })
}

/// Left before right.
fn twin_order(key: KeyCode) -> u8 {
    u8::from(matches!(
        key,
        KeyCode::ShiftRight | KeyCode::ControlRight | KeyCode::SuperRight
    ))
}

/// What a key is called on a US keyboard, where [`KeyCode`] names keys by their place.
fn key_label(key: KeyCode) -> String {
    use KeyCode as K;
    let name = match key {
        K::Space => "Space",
        K::Enter => "Enter",
        K::Tab => "Tab",
        K::Backspace => "Backspace",
        K::Escape => "Esc",
        K::CapsLock => "Caps Lock",
        K::ShiftLeft => "Left Shift",
        K::ShiftRight => "Right Shift",
        K::ControlLeft => "Left Ctrl",
        K::ControlRight => "Right Ctrl",
        K::AltLeft => "Left Alt",
        K::AltRight => "Right Alt",
        K::SuperLeft => "Left Super",
        K::SuperRight => "Right Super",
        K::ContextMenu => "Menu",
        K::ArrowUp => "Up Arrow",
        K::ArrowDown => "Down Arrow",
        K::ArrowLeft => "Left Arrow",
        K::ArrowRight => "Right Arrow",
        K::Insert => "Insert",
        K::Delete => "Delete",
        K::Home => "Home",
        K::End => "End",
        K::PageUp => "Page Up",
        K::PageDown => "Page Down",
        K::Backquote => "`",
        K::Minus => "-",
        K::Equal => "=",
        K::BracketLeft => "[",
        K::BracketRight => "]",
        K::Backslash => "\\",
        K::Semicolon => ";",
        K::Quote => "'",
        K::Comma => ",",
        K::Period => ".",
        K::Slash => "/",
        K::NumpadAdd => "Numpad +",
        K::NumpadSubtract => "Numpad -",
        K::NumpadMultiply => "Numpad *",
        K::NumpadDivide => "Numpad /",
        K::NumpadDecimal => "Numpad .",
        K::NumpadEnter => "Numpad Enter",
        _ => {
            let debug = format!("{key:?}");
            return if let Some(letter) = debug.strip_prefix("Key") {
                letter.to_owned()
            } else if let Some(digit) = debug.strip_prefix("Digit") {
                digit.to_owned()
            } else if let Some(digit) = debug.strip_prefix("Numpad") {
                format!("Numpad {digit}")
            } else {
                // F1–F35 name themselves, and so does anything rarer.
                debug
            };
        }
    };
    name.to_owned()
}

/// What a press of a bound input does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    MoveForward,
    MoveBack,
    MoveLeft,
    MoveRight,
    /// Jumps, swims upward, or flies upward. Pressed twice quickly, it switches flying on and off.
    Jump,
    /// Tapped while going forward on foot, runs until Forward is let go, Sneak is held or the body
    /// hits a wall (`crate::player::TapRun`).
    Run,
    /// Sneaks, on the ground, while held: slower, and never off a ledge.
    Sneak,
    /// Flies downward.
    Descend,
    CycleOverlay,
    ChooseDig,
    ChoosePlace,
    ChooseHeatBrush,
    ChooseSpawnFluid,
    /// Uses Dig, while it is in hand.
    Dig,
    /// Uses Place, while it is in hand.
    Place,
    /// Uses the heat brush, while it is in hand, for as long as the input is held.
    HeatBrush,
    /// Uses Spawn fluid, while it is in hand.
    SpawnFluid,
}

impl Action {
    /// Every action, in the order Settings lists them.
    pub const ALL: [Action; 17] = [
        Action::MoveForward,
        Action::MoveBack,
        Action::MoveLeft,
        Action::MoveRight,
        Action::Jump,
        Action::Run,
        Action::Sneak,
        Action::Descend,
        Action::ChooseDig,
        Action::ChoosePlace,
        Action::ChooseHeatBrush,
        Action::ChooseSpawnFluid,
        Action::Dig,
        Action::Place,
        Action::HeatBrush,
        Action::SpawnFluid,
        Action::CycleOverlay,
    ];

    /// The action that chooses `tool`.
    pub fn choose(tool: Tool) -> Action {
        match tool {
            Tool::Dig => Action::ChooseDig,
            Tool::Place => Action::ChoosePlace,
            Tool::HeatBrush => Action::ChooseHeatBrush,
            Tool::SpawnFluid => Action::ChooseSpawnFluid,
        }
    }

    /// The action that uses `tool` while it is in hand.
    pub fn use_tool(tool: Tool) -> Action {
        match tool {
            Tool::Dig => Action::Dig,
            Tool::Place => Action::Place,
            Tool::HeatBrush => Action::HeatBrush,
            Tool::SpawnFluid => Action::SpawnFluid,
        }
    }

    /// Its name in the settings file.
    pub fn name(self) -> &'static str {
        match self {
            Action::MoveForward => "move_forward",
            Action::MoveBack => "move_back",
            Action::MoveLeft => "move_left",
            Action::MoveRight => "move_right",
            Action::Jump => "jump",
            Action::Run => "run",
            Action::Sneak => "sneak",
            Action::Descend => "descend",
            Action::CycleOverlay => "cycle_overlay",
            Action::ChooseDig => "choose_dig",
            Action::ChoosePlace => "choose_place",
            Action::ChooseHeatBrush => "choose_heat_brush",
            Action::ChooseSpawnFluid => "choose_spawn_fluid",
            Action::Dig => "dig",
            Action::Place => "place",
            Action::HeatBrush => "heat_brush",
            Action::SpawnFluid => "spawn_fluid",
        }
    }

    /// The action named `name` in the settings file.
    pub fn from_name(name: &str) -> Option<Action> {
        Action::ALL.into_iter().find(|action| action.name() == name)
    }

    /// How Settings names it.
    pub fn label(self) -> &'static str {
        match self {
            Action::MoveForward => "Move forward",
            Action::MoveBack => "Move back",
            Action::MoveLeft => "Move left",
            Action::MoveRight => "Move right",
            Action::Jump => "Jump, swim up, fly up",
            Action::Run => "Run",
            Action::Sneak => "Sneak",
            Action::Descend => "Fly down",
            Action::CycleOverlay => "Cycle the Overlays",
            Action::ChooseDig => "Choose Dig",
            Action::ChoosePlace => "Choose Place",
            Action::ChooseHeatBrush => "Choose Heat brush",
            Action::ChooseSpawnFluid => "Choose Spawn fluid",
            Action::Dig => "Dig",
            Action::Place => "Place",
            Action::HeatBrush => "Heat brush",
            Action::SpawnFluid => "Spawn fluid",
        }
    }

    /// When it applies.
    pub fn context(self) -> Context {
        match self {
            Action::Descend => Context::flying(),
            Action::Run | Action::Sneak => Context::walking(),
            Action::Dig => Context::holding(Tool::Dig),
            Action::Place => Context::holding(Tool::Place),
            Action::HeatBrush => Context::holding(Tool::HeatBrush),
            Action::SpawnFluid => Context::holding(Tool::SpawnFluid),
            Action::MoveForward
            | Action::MoveBack
            | Action::MoveLeft
            | Action::MoveRight
            | Action::Jump
            | Action::CycleOverlay
            | Action::ChooseDig
            | Action::ChoosePlace
            | Action::ChooseHeatBrush
            | Action::ChooseSpawnFluid => Context::ALWAYS,
        }
    }

    /// What it is bound to by default.
    pub fn default_inputs(self) -> Vec<Input> {
        let key = |key| vec![Input::Key(key)];
        match self {
            Action::MoveForward => key(KeyCode::KeyW),
            Action::MoveBack => key(KeyCode::KeyS),
            Action::MoveLeft => key(KeyCode::KeyA),
            Action::MoveRight => key(KeyCode::KeyD),
            Action::Jump => key(KeyCode::Space),
            Action::Run => Input::Key(KeyCode::ControlLeft).with_twin(),
            Action::Sneak | Action::Descend => Input::Key(KeyCode::ShiftLeft).with_twin(),
            Action::CycleOverlay => key(KeyCode::KeyO),
            Action::ChooseDig => key(KeyCode::Digit1),
            Action::ChoosePlace => key(KeyCode::Digit2),
            Action::ChooseHeatBrush => key(KeyCode::Digit3),
            Action::ChooseSpawnFluid => key(KeyCode::Digit4),
            Action::Dig | Action::HeatBrush => vec![Input::Mouse(MouseButton::Left)],
            Action::Place | Action::SpawnFluid => vec![Input::Mouse(MouseButton::Right)],
        }
    }

    fn index(self) -> usize {
        Action::ALL
            .iter()
            .position(|action| *action == self)
            .expect("every action is in ALL")
    }
}

/// When an action applies: in which movement modes, and with which god tools in hand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Context {
    /// One bit for each [`MoveMode`].
    modes: u8,
    /// One bit for each [`Tool`].
    tools: u8,
}

impl Context {
    /// In every movement mode, with any tool in hand.
    pub const ALWAYS: Context = Context {
        modes: u8::MAX,
        tools: u8::MAX,
    };

    /// Only while walking: on foot or swimming, rather than flying.
    pub fn walking() -> Context {
        Self::in_mode(MoveMode::Walk)
    }

    /// Only while flying.
    pub fn flying() -> Context {
        Self::in_mode(MoveMode::Fly)
    }

    fn in_mode(mode: MoveMode) -> Context {
        Context {
            modes: 1 << mode as u8,
            ..Context::ALWAYS
        }
    }

    /// Only with `tool` in hand.
    pub fn holding(tool: Tool) -> Context {
        let bit = Tool::ALL
            .iter()
            .position(|t| *t == tool)
            .expect("every tool is in ALL");
        Context {
            tools: 1 << bit,
            ..Context::ALWAYS
        }
    }

    /// Whether some moment falls in both: a movement mode both apply in, with a tool in hand that
    /// both apply with.
    pub fn overlaps(self, other: Context) -> bool {
        self.modes & other.modes != 0 && self.tools & other.tools != 0
    }
}

/// The keys only the game itself uses: Esc pauses, and Alt frees the cursor.
pub const RESERVED_KEYS: [(KeyCode, &str); 3] = [
    (PAUSE_KEY, "pausing"),
    (FREE_CURSOR_KEYS[0], "freeing the cursor"),
    (FREE_CURSOR_KEYS[1], "freeing the cursor"),
];

/// Shortcuts whose keys can't be rebound or bound to anything else: the debug panel's.
pub const FIXED_SHORTCUTS: [(KeyCode, &str); 3] = [
    (debug_panel::PANEL_KEY, "showing the debug panel"),
    (debug_panel::CLOCK_KEY, "stopping and starting the debug clock"),
    (debug_panel::STEP_KEY, "running one Tick of the debug clock"),
];

/// Why an input can't be bound to anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The game keeps it for this.
    Reserved { input: Input, purpose: &'static str },
    /// A fixed shortcut uses it for this.
    Fixed { input: Input, purpose: &'static str },
}

impl Refusal {
    /// Why `input` can't be bound, if it can't.
    pub fn of(input: Input) -> Option<Refusal> {
        let Input::Key(key) = input else {
            return None;
        };
        let find = |list: &[(KeyCode, &'static str)]| {
            list.iter()
                .find(|(fixed, _)| *fixed == key)
                .map(|(_, purpose)| *purpose)
        };
        if let Some(purpose) = find(&RESERVED_KEYS) {
            Some(Refusal::Reserved { input, purpose })
        } else {
            find(&FIXED_SHORTCUTS).map(|purpose| Refusal::Fixed { input, purpose })
        }
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::Reserved { input, purpose } => {
                write!(f, "{input} is reserved for {purpose}")
            }
            Refusal::Fixed { input, purpose } => {
                write!(f, "{input} is the fixed shortcut for {purpose}")
            }
        }
    }
}

/// Two actions that one press would both do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Conflict {
    pub first: Action,
    pub second: Action,
    pub input: Input,
}

impl fmt::Display for Conflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "“{}” and “{}” are both bound to {}",
            self.first.label(),
            self.second.label(),
            self.input
        )
    }
}

/// What each action is bound to. An action may be unbound.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bindings([Vec<Input>; Action::ALL.len()]);

impl Default for Bindings {
    fn default() -> Self {
        Self(Action::ALL.map(Action::default_inputs))
    }
}

impl Bindings {
    /// What `action` is bound to.
    pub fn inputs(&self, action: Action) -> &[Input] {
        &self.0[action.index()]
    }

    /// Binds `action` to `inputs` alone, whatever else they are bound to: see
    /// [`conflicts`](Self::conflicts) and [`Refusal::of`] first.
    pub fn set(&mut self, action: Action, mut inputs: Vec<Input>) {
        let mut seen = Vec::with_capacity(inputs.len());
        inputs.retain(|input| {
            let new = !seen.contains(input);
            seen.push(*input);
            new
        });
        self.0[action.index()] = inputs;
    }

    /// Unbinds `input` from `action`, which keeps any other inputs.
    pub fn remove(&mut self, action: Action, input: Input) {
        self.0[action.index()].retain(|bound| *bound != input);
    }

    /// The actions other than `action` that one press would do along with it if it were bound to
    /// `inputs`: those bound to one of them, in a context that overlaps its own.
    pub fn conflicts(&self, action: Action, inputs: &[Input]) -> Vec<Action> {
        Action::ALL
            .into_iter()
            .filter(|other| *other != action && action.context().overlaps(other.context()))
            .filter(|other| self.inputs(*other).iter().any(|i| inputs.contains(i)))
            .collect()
    }

    /// Why these bindings can't be used, if they can't: an input that can't be bound, or two
    /// actions in overlapping contexts that share one.
    pub fn check(&self) -> Result<(), BindingsError> {
        for action in Action::ALL {
            if let Some(refusal) = self.inputs(action).iter().find_map(|i| Refusal::of(*i)) {
                return Err(BindingsError::Refused { action, refusal });
            }
        }
        for (index, first) in Action::ALL.into_iter().enumerate() {
            for second in Action::ALL.into_iter().skip(index + 1) {
                if !first.context().overlaps(second.context()) {
                    continue;
                }
                if let Some(input) = self
                    .inputs(first)
                    .iter()
                    .find(|input| self.inputs(second).contains(input))
                {
                    return Err(BindingsError::Conflict(Conflict {
                        first,
                        second,
                        input: *input,
                    }));
                }
            }
        }
        Ok(())
    }

    /// How menus and hints name what `action` is bound to: "Space", "Shift" for both Shift keys,
    /// "W or Up Arrow", or "Unbound".
    pub fn label(&self, action: Action) -> String {
        inputs_label(self.inputs(action)).unwrap_or_else(|| "Unbound".to_owned())
    }
}

/// `inputs` named as one, as [`Bindings::label`] names them, or `None` if there are none.
pub fn inputs_label(inputs: &[Input]) -> Option<String> {
    let mut labels: Vec<String> = Vec::new();
    let mut skip = Vec::new();
    for input in inputs {
        if skip.contains(input) {
            continue;
        }
        let twins = input.with_twin();
        if twins.len() == 2 && twins.iter().all(|twin| inputs.contains(twin)) {
            skip.extend(twins);
            let Input::Key(key) = input else {
                unreachable!("only keys have twins")
            };
            labels.push(
                match key {
                    KeyCode::ShiftLeft | KeyCode::ShiftRight => "Shift",
                    KeyCode::ControlLeft | KeyCode::ControlRight => "Ctrl",
                    _ => "Super",
                }
                .to_owned(),
            );
        } else {
            labels.push(input.label());
        }
    }
    (!labels.is_empty()).then(|| labels.join(" or "))
}

/// Why a set of [`Bindings`] can't be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingsError {
    /// `action` is bound to an input nothing may be bound to.
    Refused { action: Action, refusal: Refusal },
    Conflict(Conflict),
}

impl fmt::Display for BindingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BindingsError::Refused { action, refusal } => {
                write!(f, "“{}” can't be bound there: {refusal}", action.label())
            }
            BindingsError::Conflict(conflict) => conflict.fmt(f),
        }
    }
}

/// The keyboard and mouse, read as actions through the player's bindings.
#[derive(SystemParam)]
pub struct ActionInput<'w> {
    keys: Res<'w, ButtonInput<KeyCode>>,
    mouse: Res<'w, ButtonInput<MouseButton>>,
    settings: Res<'w, Settings>,
}

impl ActionInput<'_> {
    /// Whether an input bound to `action` is held.
    pub fn pressed(&self, action: Action) -> bool {
        self.inputs(action).any(|input| match input {
            Input::Key(key) => self.keys.pressed(key),
            Input::Mouse(button) => self.mouse.pressed(button),
        })
    }

    /// Whether an input bound to `action` was pressed this frame.
    pub fn just_pressed(&self, action: Action) -> bool {
        self.inputs(action).any(|input| match input {
            Input::Key(key) => self.keys.just_pressed(key),
            Input::Mouse(button) => self.mouse.just_pressed(button),
        })
    }

    pub fn bindings(&self) -> &Bindings {
        &self.settings.bindings
    }

    fn inputs(&self, action: Action) -> impl Iterator<Item = Input> + '_ {
        self.settings.bindings.inputs(action).iter().copied()
    }
}
