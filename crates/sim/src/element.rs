/// Identifies an [`Element`] within the [`ElementTable`] that issued it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ElementId(u16);

impl ElementId {
    /// Vacuum is built into every [`ElementTable`].
    pub const VACUUM: ElementId = ElementId(0);
}

/// A kind of matter with fixed physical properties.
#[derive(Clone, Debug)]
pub struct Element {
    pub name: String,
    /// Joules needed to warm one gram by one kelvin, J/(g·K).
    pub specific_heat: f32,
    /// How readily heat passes through, W/(m·K).
    pub conductivity: f32,
}

impl Element {
    /// A new Element with no heat capacity and no conductivity; set those with the builder methods.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            specific_heat: 0.0,
            conductivity: 0.0,
        }
    }

    pub fn specific_heat(mut self, joules_per_gram_kelvin: f32) -> Self {
        self.specific_heat = joules_per_gram_kelvin;
        self
    }

    pub fn conductivity(mut self, watts_per_metre_kelvin: f32) -> Self {
        self.conductivity = watts_per_metre_kelvin;
        self
    }
}

/// Every Element the simulation knows about. Vacuum is always present.
#[derive(Clone, Debug)]
pub struct ElementTable {
    elements: Vec<Element>,
}

impl ElementTable {
    pub fn new() -> Self {
        Self {
            elements: vec![Element::new("Vacuum")],
        }
    }

    pub fn add(&mut self, element: Element) -> ElementId {
        let id = ElementId(u16::try_from(self.elements.len()).expect("too many Elements"));
        self.elements.push(element);
        id
    }

    pub(crate) fn get(&self, id: ElementId) -> &Element {
        &self.elements[usize::from(id.0)]
    }
}

impl Default for ElementTable {
    fn default() -> Self {
        Self::new()
    }
}
