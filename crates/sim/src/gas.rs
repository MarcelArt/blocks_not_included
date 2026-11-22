//! Gas flow: rules 1–6 of `docs/plans/02-gas-flow.md`, and gas escaping through the world's
//! ceiling ([`escape_to_space`]).
//!
//! Each Tick runs three passes, in this order: layering (rule 4), Displacement (rule 5), then
//! spreading into Vacuum and evening out (rules 2 and 3). Every pass plans all of its moves from
//! the state at its own start and then applies them together, the way heat does. Moved Mass
//! mixes its heat into whatever it joins (rule 1). Only gas and Vacuum Cells take part, so gas
//! never enters a solid or a liquid (rule 6). Liquids push gas aside in liquid flow instead
//! (`liquid.rs`).
//!
//! # Ordering and bias
//!
//! Of the two approaches the plan offers, this module plans from the start state rather than
//! shuffling direction order. With shuffled directions, one direction still goes first within any
//! one Tick, so a symmetric setup is only symmetric on average. Planning from a snapshot keeps a
//! mirror-symmetric setup exactly symmetric, Tick by Tick.
//!
//! Conflicts are settled without looking at processing order:
//! - A Vacuum Cell reached by two different gases goes to the one offering the most grams.
//! - Displacement candidates are taken in a pseudo-random order, each Cell keeping the first
//!   role it gets (see [`displace`]).
//! - Exact ties use a pseudo-random tie-break seeded by the Tick count and Cell position.
//!
//! Only these tie-breaks and the Displacement order can make mirror images differ. Nothing
//! depends on `HashMap` iteration order, so the same state and Tick count always give the same
//! result.
//!
//! # How much moves
//!
//! Across each face, a gas Cell moves [`SimSettings::gas_evening_out_fraction`] of the Mass
//! difference, divided by the larger number of open faces of the two Cells (a Vacuum neighbour
//! counts as 0 g). So a lone pair moves exactly that fraction. The fraction is at most 0.5, so
//! however many faces flow at once, each Cell's new Mass stays a blend of its neighbours' old
//! Mass, so nothing overshoots, and no pass takes more than that fraction of a gas Cell's Mass, so
//! it never empties. Flows are truncated to whole grams, which
//! is how gas comes to rest (ADR 0004).

use crate::cell::{Cell, CellPos};
use crate::chunk::{ActiveChunks, CHUNK_SIZE, cell_at, index};
use crate::displacement::displace;
use crate::element::{ElementId, ElementTable, Phase};
use crate::region::top_chunk_y;
use crate::settings::SimSettings;
use crate::transfer::{
    FACES, Transfer, UP, apply, cells_where, settle_vacuum_claims, share, swap_layers,
};

/// Moves gas Mass for one Tick.
pub(crate) fn flow(
    chunks: &mut ActiveChunks,
    elements: &ElementTable,
    tick: u64,
    settings: &SimSettings,
) {
    let fraction = settings.gas_evening_out_fraction;
    // Rule 4: a gas Cell directly above a different, less dense gas swaps places with it.
    swap_layers(chunks, |above, below| sinks_through(above, below, elements));
    // Rule 5: Displacement.
    let pushes = gas_cells(chunks, elements)
        .into_iter()
        .flat_map(|target| pushes_into(chunks, elements, fraction, target))
        .collect();
    displace(chunks, tick, pushes);
    spread(chunks, elements, fraction, tick);
}

/// Gas escaping through the world's ceiling: every gas Cell in the top layer of the world loses
/// `fraction` of its Mass to space, rounded down to whole grams (ADR 0004). Returns how many grams
/// escaped.
///
/// This is the second named exception to Mass conservation, after Edits. Settled Chunks are
/// skipped: a gas Cell that lost nothing last Tick, and gained nothing since, loses nothing now.
pub(crate) fn escape_to_space(
    chunks: &mut ActiveChunks,
    elements: &ElementTable,
    fraction: f64,
) -> u64 {
    let top = CHUNK_SIZE - 1;
    let mut escaped = 0;
    for (_, chunk) in chunks
        .iter_mut()
        .filter(|(pos, chunk)| pos.y == top_chunk_y() && chunk.is_awake())
    {
        for (x, z) in (0..CHUNK_SIZE).flat_map(|x| (0..CHUNK_SIZE).map(move |z| (x, z))) {
            let index = index((x, top, z));
            let cell = chunk.cells[index];
            // Truncation rounds any loss below 1 g to zero (ADR 0004).
            let grams = (f64::from(cell.mass_grams) * fraction) as u32;
            if grams == 0 || !is_gas(cell, elements) {
                continue;
            }
            escaped += u64::from(grams);
            let left = cell.mass_grams - grams;
            let cell = if left == 0 {
                Cell::VACUUM
            } else {
                Cell {
                    mass_grams: left,
                    ..cell
                }
            };
            chunk.write(index, cell);
        }
    }
    escaped
}

/// Whether gas `above` sinks through gas `below`: they are different gases, and `above` is
/// denser. The decision uses each Element's Density only, never how much Mass the Cells hold.
fn sinks_through(above: Cell, below: Cell, elements: &ElementTable) -> bool {
    is_gas(below, elements)
        && is_gas(above, elements)
        && below.element != above.element
        && elements.get(above.element).density > elements.get(below.element).density
}

/// Every push a different gas could make into `target`: from a neighbour holding more Mass,
/// moving what it would move if `target` were Vacuum. Pairs that layering (rule 4) decides are
/// left out.
fn pushes_into(
    chunks: &ActiveChunks,
    elements: &ElementTable,
    fraction: f64,
    target: CellPos,
) -> Vec<Transfer> {
    let held = cell_at(chunks, target);
    let mut pushes = Vec::new();
    for face in FACES {
        let from = target.offset(face);
        let source = cell_at(chunks, from);
        if !is_gas(source, elements)
            || source.element == held.element
            || source.mass_grams <= held.mass_grams
            || layers(chunks, elements, from, target)
        {
            continue;
        }
        let faces = face_count(chunks, elements, from, target);
        let grams = share(source.mass_grams, 0, faces, fraction);
        if grams > 0 {
            pushes.push(Transfer::new(from, source, target, grams));
        }
    }
    pushes
}

/// Whether rule 4 governs the touching Cells `a` and `b`.
fn layers(chunks: &ActiveChunks, elements: &ElementTable, a: CellPos, b: CellPos) -> bool {
    let (lower, upper) = if b == a.offset(UP) {
        (a, b)
    } else if a == b.offset(UP) {
        (b, a)
    } else {
        return false;
    };
    sinks_through(cell_at(chunks, upper), cell_at(chunks, lower), elements)
}

/// Rules 2 and 3: every gas Cell moves part of its Mass into each Vacuum face neighbour, and
/// toward each emptier face neighbour holding the same gas.
///
/// A Vacuum Cell becomes only one Element per Tick: when different gases reach it at once, the
/// one offering the most grams wins, and the others' offers are withdrawn.
fn spread(chunks: &mut ActiveChunks, elements: &ElementTable, fraction: f64, tick: u64) {
    let mut transfers = Vec::new();
    let mut into_vacuum = Vec::new();
    for from in gas_cells(chunks, elements) {
        let source = cell_at(chunks, from);
        for face in FACES {
            let to = from.offset(face);
            let target = cell_at(chunks, to);
            if target.element != ElementId::VACUUM && target.element != source.element {
                continue;
            }
            let faces = face_count(chunks, elements, from, to);
            let grams = share(source.mass_grams, target.mass_grams, faces, fraction);
            if grams == 0 {
                continue;
            }
            let transfer = Transfer::new(from, source, to, grams);
            if target.element == ElementId::VACUUM {
                into_vacuum.push(transfer);
            } else {
                transfers.push(transfer);
            }
        }
    }
    transfers.extend(settle_vacuum_claims(into_vacuum, tick));
    apply(chunks, transfers);
}

/// The larger open-face count of two touching Cells.
fn face_count(chunks: &ActiveChunks, elements: &ElementTable, a: CellPos, b: CellPos) -> u32 {
    open_faces(chunks, elements, a).max(open_faces(chunks, elements, b))
}

/// How many of a Cell's faces touch gas or Vacuum, and so could carry gas.
fn open_faces(chunks: &ActiveChunks, elements: &ElementTable, pos: CellPos) -> u32 {
    FACES
        .into_iter()
        .filter(|&face| is_open(cell_at(chunks, pos.offset(face)), elements))
        .count() as u32
}

fn is_open(cell: Cell, elements: &ElementTable) -> bool {
    matches!(elements.get(cell.element).phase, Phase::Gas | Phase::Vacuum)
}

pub(crate) fn is_gas(cell: Cell, elements: &ElementTable) -> bool {
    elements.get(cell.element).phase == Phase::Gas
}

/// Every gas Cell in the world.
fn gas_cells(chunks: &ActiveChunks, elements: &ElementTable) -> Vec<CellPos> {
    cells_where(chunks, |cell| is_gas(cell, elements))
}
