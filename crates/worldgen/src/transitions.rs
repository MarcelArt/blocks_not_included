//! Following Phase Transitions at generation time.
//!
//! The generator picks the Element it intends for each Cell, then turns it into whatever that
//! Element becomes at the Cell's temperature, step by step, until it is past none of its
//! thresholds. A "Water" pocket deep underground comes out as Steam, and Granite near the floor
//! as Magma. So the simulation's first Tick finds no Phase Transition to make.
//!
//! The thresholds are read exactly as the simulation reads them: a Cell is past `low` when it is
//! below it and past `high` when it is above it, and a threshold whose target the table lacks, or
//! whose target is Vacuum, is ignored.

use sim::{ElementId, ElementTable, Side};

/// One side of an Element's Phase Transitions, with its target looked up by name.
#[derive(Clone, Copy)]
struct Threshold {
    side: Side,
    temperature_k: f32,
    into: ElementId,
}

impl Threshold {
    fn is_crossed_at(self, temperature_k: f32) -> bool {
        match self.side {
            Side::Low => temperature_k < self.temperature_k,
            Side::High => temperature_k > self.temperature_k,
        }
    }
}

/// Every Element's Phase Transitions, indexed by [`ElementId::index`].
pub(crate) struct Transitions(Vec<[Option<Threshold>; 2]>);

impl Transitions {
    pub fn new(elements: &ElementTable) -> Self {
        Self(
            elements
                .iter()
                .map(|(_, element)| {
                    [(Side::Low, &element.low), (Side::High, &element.high)].map(
                        |(side, transition)| {
                            let transition = transition.as_ref()?;
                            let into = elements
                                .id_of(&transition.into)
                                .filter(|&into| into != ElementId::VACUUM)?;
                            Some(Threshold {
                                side,
                                temperature_k: transition.temperature_k,
                                into,
                            })
                        },
                    )
                })
                .collect(),
        )
    }

    /// What a Cell of `element` at `temperature_k` ends up as once it has made every Phase
    /// Transition its temperature calls for.
    ///
    /// A validated table leaves a gap between opposite thresholds, so no chain of Transitions
    /// loops. The number of steps is bounded anyway, for tables that weren't validated.
    pub fn follow(&self, mut element: ElementId, temperature_k: f32) -> ElementId {
        for _ in 0..self.0.len() {
            let crossed = self.0[usize::from(element.index())]
                .into_iter()
                .flatten()
                .find(|threshold| threshold.is_crossed_at(temperature_k));
            match crossed {
                Some(threshold) => element = threshold.into,
                None => break,
            }
        }
        element
    }
}
