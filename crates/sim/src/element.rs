use std::collections::HashSet;
use std::fmt;

use serde::Deserialize;

/// Identifies an [`Element`] within the [`ElementTable`] that issued it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ElementId(u16);

impl ElementId {
    /// Vacuum is built into every [`ElementTable`].
    pub const VACUUM: ElementId = ElementId(0);

    /// This Element's position in its table, as a compact number for renderers.
    ///
    /// Vacuum is 0. In a table loaded with [`ElementTable::from_ron`], every other index is the
    /// Element's 1-based position in the file.
    pub fn index(self) -> u16 {
        self.0
    }
}

/// Whether an Element is a solid, a liquid, a gas or vacuum.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize)]
pub enum Phase {
    Solid,
    Liquid,
    Gas,
    /// Only the built-in Vacuum is in this Phase.
    Vacuum,
}

impl Phase {
    /// How condensed this Phase is: solids most, gases least. Vacuum holds no matter, so it has
    /// no place on the scale.
    fn condensation(self) -> Option<u8> {
        match self {
            Phase::Gas => Some(0),
            Phase::Liquid => Some(1),
            Phase::Solid => Some(2),
            Phase::Vacuum => None,
        }
    }
}

/// A Phase Transition threshold: crossing `temperature_k` turns an Element into `into`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transition {
    pub temperature_k: f32,
    /// The name of the Element this one becomes.
    pub into: String,
}

/// Which of an Element's two Transitions: `low` (crossed by cooling) or `high` (by heating).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    Low,
    High,
}

impl Side {
    fn opposite(self) -> Side {
        match self {
            Side::Low => Side::High,
            Side::High => Side::Low,
        }
    }

    pub(crate) fn of(self, element: &Element) -> Option<&Transition> {
        match self {
            Side::Low => element.low.as_ref(),
            Side::High => element.high.as_ref(),
        }
    }

    /// Whether a Transition on this side may turn a `from` Element into a `to` Element: `low`
    /// must lead to a more condensed Phase and `high` to a less condensed one. Vacuum is neither,
    /// so no Transition leads into or out of it.
    fn allows(self, from: Phase, to: Phase) -> bool {
        match (from.condensation(), to.condensation()) {
            (Some(from), Some(to)) => match self {
                Side::Low => to > from,
                Side::High => to < from,
            },
            _ => false,
        }
    }

    /// Whether a temperature lies past a threshold on this side: below it for `low`, above it
    /// for `high`. NaN is past nothing.
    pub(crate) fn is_past(self, temperature_k: f32, threshold_k: f32) -> bool {
        match self {
            Side::Low => temperature_k < threshold_k,
            Side::High => temperature_k > threshold_k,
        }
    }
}

impl fmt::Display for Side {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Side::Low => "low",
            Side::High => "high",
        })
    }
}

/// A kind of matter with fixed physical properties and exactly one Phase.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Element {
    pub name: String,
    pub phase: Phase,
    /// Linear RGB, the flat render colour.
    pub color: [f32; 3],
    /// kg/m³. Decides which Element sinks beneath another; a full liquid Cell holds
    /// `density × 1 m³`, and it is the default Mass for a placed solid.
    pub density: f32,
    /// Joules needed to warm one gram by one kelvin, J/(g·K).
    pub specific_heat: f32,
    /// How readily heat passes through, W/(m·K).
    pub conductivity: f32,
    /// Below `low.temperature_k` this Element becomes `low.into`.
    #[serde(default)]
    pub low: Option<Transition>,
    /// Above `high.temperature_k` this Element becomes `high.into`.
    #[serde(default)]
    pub high: Option<Transition>,
}

impl Element {
    /// A new grey solid with no Density, heat capacity, conductivity or Phase Transitions; set
    /// those with the builder methods.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            phase: Phase::Solid,
            color: [0.5, 0.5, 0.5],
            density: 0.0,
            specific_heat: 0.0,
            conductivity: 0.0,
            low: None,
            high: None,
        }
    }

    pub fn phase(mut self, phase: Phase) -> Self {
        self.phase = phase;
        self
    }

    pub fn color(mut self, linear_rgb: [f32; 3]) -> Self {
        self.color = linear_rgb;
        self
    }

    pub fn density(mut self, kilograms_per_cubic_metre: f32) -> Self {
        self.density = kilograms_per_cubic_metre;
        self
    }

    pub fn specific_heat(mut self, joules_per_gram_kelvin: f32) -> Self {
        self.specific_heat = joules_per_gram_kelvin;
        self
    }

    pub fn conductivity(mut self, watts_per_metre_kelvin: f32) -> Self {
        self.conductivity = watts_per_metre_kelvin;
        self
    }

    /// Below `temperature_k`, this Element becomes the Element named `into`.
    pub fn low(mut self, temperature_k: f32, into: impl Into<String>) -> Self {
        self.low = Some(Transition {
            temperature_k,
            into: into.into(),
        });
        self
    }

    /// Above `temperature_k`, this Element becomes the Element named `into`.
    pub fn high(mut self, temperature_k: f32, into: impl Into<String>) -> Self {
        self.high = Some(Transition {
            temperature_k,
            into: into.into(),
        });
        self
    }
}

/// Why an [`ElementTable`] was rejected. Rule numbers refer to `docs/plans/01-element-model.md`.
#[derive(Clone, Debug, PartialEq)]
pub enum ElementError {
    /// The text is not a well-formed Elements file.
    Parse(String),
    /// Rule 1: an Element has an empty name, so it is identified by its [`ElementId::index`],
    /// which is also its 1-based position in the Elements file.
    EmptyName { index: u16 },
    /// Rule 1: the file lists `"Vacuum"`, which every table already has built in.
    ReservedName,
    /// Rule 1: two Elements share a name.
    DuplicateName { element: String },
    /// Rule 2: a Transition names an Element the table doesn't hold.
    UnknownTarget {
        element: String,
        side: Side,
        into: String,
    },
    /// Rule 3: a `low` Transition doesn't lead to a more condensed Phase, or a `high` one doesn't
    /// lead to a less condensed Phase. Vacuum is never a valid target.
    WrongDirection {
        element: String,
        phase: Phase,
        side: Side,
        into: String,
        into_phase: Phase,
    },
    /// Rule 4: `element`'s `side` Transition turns it into `into` at `temperature_k`, but
    /// `into`'s opposite threshold, at `opposite_k`, would turn it again straight away.
    NoHysteresisGap {
        element: String,
        side: Side,
        temperature_k: f32,
        into: String,
        opposite_k: f32,
    },
    /// Rule 4: an Element's own `low` threshold is not below its `high` threshold.
    LowNotBelowHigh {
        element: String,
        low_k: f32,
        high_k: f32,
    },
    /// Rule 5: a physical property is out of range. Density and specific heat must be above
    /// zero, and conductivity must not be negative.
    InvalidProperty {
        element: String,
        /// `"density"`, `"specific_heat"` or `"conductivity"`.
        property: &'static str,
        value: f32,
    },
    /// A listed Element is in the vacuum Phase, which belongs to the built-in Vacuum alone
    /// (`CONTEXT.md`).
    VacuumPhase { element: String },
}

impl fmt::Display for ElementError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(message) => write!(f, "the Elements file is not valid RON: {message}"),
            Self::EmptyName { index } => write!(f, "Element #{index} has an empty name"),
            Self::ReservedName => write!(f, "\"Vacuum\" is built in, so it must not be listed"),
            Self::DuplicateName { element } => write!(f, "{element:?} is listed more than once"),
            Self::UnknownTarget {
                element,
                side,
                into,
            } => write!(f, "{element:?}'s {side} Transition names {into:?}, which is not an Element"),
            Self::WrongDirection {
                element,
                phase,
                side,
                into,
                into_phase,
            } => {
                let needed = match side {
                    Side::Low => "more",
                    Side::High => "less",
                };
                write!(
                    f,
                    "{element:?} ({phase:?}) has a {side} Transition into {into:?} ({into_phase:?}), \
                     but a {side} Transition must lead to a {needed} condensed Phase"
                )
            }
            Self::NoHysteresisGap {
                element,
                side,
                temperature_k,
                into,
                opposite_k,
            } => {
                let (crossing, needed) = match side {
                    Side::Low => ("below", "above"),
                    Side::High => ("above", "below"),
                };
                let opposite = side.opposite();
                write!(
                    f,
                    "{element:?} becomes {into:?} {crossing} {temperature_k} K, so {into:?}'s \
                     {opposite} threshold must be {needed} {temperature_k} K, not {opposite_k} K"
                )
            }
            Self::LowNotBelowHigh {
                element,
                low_k,
                high_k,
            } => write!(
                f,
                "{element:?}'s low threshold ({low_k} K) must be below its high threshold ({high_k} K)"
            ),
            Self::InvalidProperty {
                element,
                property,
                value,
            } => {
                let needed = if *property == "conductivity" { "at least 0" } else { "above 0" };
                write!(f, "{element:?} has {property} {value}, but it must be {needed}")
            }
            Self::VacuumPhase { element } => write!(
                f,
                "{element:?} is in the vacuum Phase, which only the built-in Vacuum may be"
            ),
        }
    }
}

impl std::error::Error for ElementError {}

/// The layout of `assets/elements.ron`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ElementsFile {
    elements: Vec<Element>,
}

/// Every Element the simulation knows about. Vacuum is always present.
#[derive(Clone, Debug)]
pub struct ElementTable {
    elements: Vec<Element>,
}

impl ElementTable {
    pub fn new() -> Self {
        Self {
            elements: vec![Element::new("Vacuum").phase(Phase::Vacuum).color([0.0, 0.0, 0.0])],
        }
    }

    /// Parses an Elements file such as `assets/elements.ron`, then [validates](Self::validate) it.
    ///
    /// Elements receive ids in file order, after the built-in Vacuum.
    pub fn from_ron(text: &str) -> Result<ElementTable, ElementError> {
        let file: ElementsFile =
            ron::from_str(text).map_err(|error| ElementError::Parse(error.to_string()))?;
        let mut table = ElementTable::new();
        for element in file.elements {
            table.add(element);
        }
        table.validate()?;
        Ok(table)
    }

    pub fn add(&mut self, element: Element) -> ElementId {
        let id = ElementId(u16::try_from(self.elements.len()).expect("too many Elements"));
        self.elements.push(element);
        id
    }

    /// Checks the rules every playable table follows, reporting the first one broken.
    ///
    /// [`Simulation::new`](crate::Simulation::new) doesn't call this, so tests can use
    /// deliberately partial Elements.
    pub fn validate(&self) -> Result<(), ElementError> {
        let mut names = HashSet::new();
        for (id, element) in self.listed() {
            self.validate_properties(id, element, &mut names)?;
        }
        for (_, element) in self.listed() {
            for side in [Side::Low, Side::High] {
                if let Some(transition) = side.of(element) {
                    self.validate_transition(element, side, transition)?;
                }
            }
            if let (Some(low), Some(high)) = (&element.low, &element.high)
                && !Side::Low.is_past(low.temperature_k, high.temperature_k)
            {
                return Err(ElementError::LowNotBelowHigh {
                    element: element.name.clone(),
                    low_k: low.temperature_k,
                    high_k: high.temperature_k,
                });
            }
        }
        Ok(())
    }

    /// Rules 1 and 5, and the vacuum Phase rule, for one listed Element.
    fn validate_properties<'a>(
        &self,
        id: ElementId,
        element: &'a Element,
        names: &mut HashSet<&'a str>,
    ) -> Result<(), ElementError> {
        let name = || element.name.clone();
        if element.name.is_empty() {
            return Err(ElementError::EmptyName { index: id.index() });
        }
        if element.name == self.get(ElementId::VACUUM).name {
            return Err(ElementError::ReservedName);
        }
        if !names.insert(&element.name) {
            return Err(ElementError::DuplicateName { element: name() });
        }
        if element.phase == Phase::Vacuum {
            return Err(ElementError::VacuumPhase { element: name() });
        }
        // Each check holds only for an in-range number, so NaN fails all of them.
        for (property, value, in_range) in [
            ("density", element.density, element.density > 0.0),
            ("specific_heat", element.specific_heat, element.specific_heat > 0.0),
            ("conductivity", element.conductivity, element.conductivity >= 0.0),
        ] {
            if !in_range {
                return Err(ElementError::InvalidProperty {
                    element: name(),
                    property,
                    value,
                });
            }
        }
        Ok(())
    }

    /// Rules 2, 3 and the cross-Element half of rule 4, for one Transition.
    fn validate_transition(
        &self,
        element: &Element,
        side: Side,
        transition: &Transition,
    ) -> Result<(), ElementError> {
        let Some(into) = self.id_of(&transition.into).map(|id| self.get(id)) else {
            return Err(ElementError::UnknownTarget {
                element: element.name.clone(),
                side,
                into: transition.into.clone(),
            });
        };
        if !side.allows(element.phase, into.phase) {
            return Err(ElementError::WrongDirection {
                element: element.name.clone(),
                phase: element.phase,
                side,
                into: into.name.clone(),
                into_phase: into.phase,
            });
        }
        // A Cell that has just crossed this threshold is `into`. `into`'s opposite threshold must
        // lie strictly beyond this one, on its own side, so the Cell has a gap to cross back.
        let opposite_side = side.opposite();
        if let Some(opposite) = opposite_side.of(into)
            && !opposite_side.is_past(opposite.temperature_k, transition.temperature_k)
        {
            return Err(ElementError::NoHysteresisGap {
                element: element.name.clone(),
                side,
                temperature_k: transition.temperature_k,
                into: into.name.clone(),
                opposite_k: opposite.temperature_k,
            });
        }
        Ok(())
    }

    /// Every Element except the built-in Vacuum.
    fn listed(&self) -> impl Iterator<Item = (ElementId, &Element)> {
        self.iter().skip(1)
    }

    /// The Element `id` refers to.
    ///
    /// # Panics
    ///
    /// If `id` was issued by a different table that holds more Elements.
    pub fn get(&self, id: ElementId) -> &Element {
        &self.elements[usize::from(id.0)]
    }

    /// The id of the Element called `name`, if there is one.
    pub fn id_of(&self, name: &str) -> Option<ElementId> {
        self.iter()
            .find(|(_, element)| element.name == name)
            .map(|(id, _)| id)
    }

    /// Every Element in id order, starting with Vacuum.
    pub fn iter(&self) -> impl Iterator<Item = (ElementId, &Element)> {
        // `add` keeps the length within u16, so every index fits.
        (0..).map(ElementId).zip(&self.elements)
    }
}

impl Default for ElementTable {
    fn default() -> Self {
        Self::new()
    }
}
