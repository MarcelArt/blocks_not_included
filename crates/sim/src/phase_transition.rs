//! Phase Transitions: rules 1–5 of `docs/plans/04-phase-transitions.md`.
//!
//! This is the last phenomenon of each Tick, after heat, gas and liquid (rule 1). Every non-Vacuum
//! Cell whose temperature lies past one of its Element's thresholds, above `high` or below `low`,
//! becomes that threshold's target Element in place. Its Mass and temperature stay exactly as they
//! were (rule 2, ADR 0002). An Element with no threshold on a side never transitions on that side
//! (rule 4).
//!
//! Latent heat is not modelled (rule 3). The hysteresis gap that [`ElementTable::validate`]
//! demands between opposite thresholds keeps a Cell from flipping back and forth.
//!
//! A Cell crosses at most one threshold per Tick, so Ice far above Water's boiling point becomes
//! Water this Tick and Steam the next. Every Cell transitions on its own, so processing order
//! doesn't matter.
//!
//! A Cell that becomes a liquid may now hold more than the liquid's capacity. Liquid flow sheds
//! the excess on later Ticks (rule 5, and rule 1 of `docs/plans/03-liquid-flow.md`).

use std::collections::HashMap;

use crate::cell::Cell;
use crate::chunk::{Chunk, ChunkPos};
use crate::element::{ElementId, ElementTable, Side};

/// Turns every Cell past one of its Element's thresholds into that threshold's target Element.
///
/// Settled Chunks are skipped, unless heat or Mass arrived in them across a border earlier in
/// this Tick, which wakes them.
pub(crate) fn transform(chunks: &mut HashMap<ChunkPos, Chunk>, elements: &ElementTable) {
    let thresholds = Thresholds::resolve(elements);
    for chunk in chunks.values_mut().filter(|chunk| chunk.is_awake()) {
        for index in 0..chunk.cells.len() {
            let cell = chunk.cells[index];
            if let Some(element) = thresholds.crossed(cell.element, cell.temperature_k) {
                chunk.write(index, Cell { element, ..cell });
            }
        }
    }
}

/// One side of an Element's Phase Transitions, with its target looked up by name.
#[derive(Clone, Copy)]
struct Threshold {
    side: Side,
    temperature_k: f32,
    into: ElementId,
}

/// Every Element's thresholds, indexed by [`ElementId::index`].
struct Thresholds(Vec<[Option<Threshold>; 2]>);

impl Thresholds {
    /// Looks up each threshold's target once, rather than by name for every Cell.
    ///
    /// [`Simulation::new`](crate::Simulation::new) accepts tables that haven't been validated, so
    /// a threshold whose target the table doesn't hold, or whose target is Vacuum, is ignored:
    /// turning matter into Vacuum would break the rule that Vacuum has no Mass.
    fn resolve(elements: &ElementTable) -> Self {
        let threshold = |element, side: Side| {
            let transition = side.of(element)?;
            let into = elements
                .id_of(&transition.into)
                .filter(|&into| into != ElementId::VACUUM)?;
            Some(Threshold {
                side,
                temperature_k: transition.temperature_k,
                into,
            })
        };
        Self(
            elements
                .iter()
                .map(|(_, element)| [Side::High, Side::Low].map(|side| threshold(element, side)))
                .collect(),
        )
    }

    /// The Element a Cell of `element` at `temperature_k` becomes, if it is past a threshold.
    ///
    /// A validated table puts `low` below `high`, so a Cell is past at most one of them. NaN is
    /// past neither.
    fn crossed(&self, element: ElementId, temperature_k: f32) -> Option<ElementId> {
        self.0[usize::from(element.index())]
            .into_iter()
            .flatten()
            .find(|threshold| {
                threshold
                    .side
                    .is_past(temperature_k, threshold.temperature_k)
            })
            .map(|threshold| threshold.into)
    }
}
