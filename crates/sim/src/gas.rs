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

use rayon::prelude::*;

use crate::cell::{Cell, CellPos};
use crate::chunk::{ActiveChunks, CHUNK_SIZE, index};
use crate::displacement::displace;
use crate::element::{ElementId, ElementTable, Phase};
use crate::grid::{Cells, Grid, Layout, chunks_mut};
use crate::region::top_chunk_y;
use crate::settings::SimSettings;
use crate::transfer::{
    FACES, Transfer, UP, apply_batches, settle_vacuum_claims, share, split_off_vacuum_claims,
    swap_layers,
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
    swap_layers(
        chunks,
        |below| is_gas(below, elements),
        |above, below| sinks_through(above, below, elements),
    );
    // Rule 5: Displacement.
    let pushes = {
        let grid = Grid::new(chunks);
        grid.plan(
            |cell| is_gas(cell, elements),
            |cells, target, held, pushes| {
                pushes_into(cells, elements, fraction, target, held, pushes);
            },
        )
    };
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
    let layout = Layout::new(chunks);
    let in_top_layer = |slot: &usize| layout.key(*slot).y == top_chunk_y();
    chunks_mut(chunks, &layout)
        .into_par_iter()
        .enumerate()
        .filter(|(slot, chunk)| in_top_layer(slot) && chunk.is_awake())
        .map(|(_, chunk)| {
            let mut escaped = 0;
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
            escaped
        })
        .sum()
}

/// Whether gas `above` sinks through gas `below`: they are different gases, and `above` is
/// denser. The decision uses each Element's Density only, never how much Mass the Cells hold.
fn sinks_through(above: Cell, below: Cell, elements: &ElementTable) -> bool {
    is_gas(below, elements)
        && is_gas(above, elements)
        && below.element != above.element
        && elements.get(above.element).density > elements.get(below.element).density
}

/// Every push a different gas could make into `target`, which holds `held`: from a neighbour
/// holding more Mass, moving what it would move if `target` were Vacuum. Pairs that layering
/// (rule 4) decides are left out.
fn pushes_into(
    cells: &impl Cells,
    elements: &ElementTable,
    fraction: f64,
    target: CellPos,
    held: Cell,
    pushes: &mut Vec<Transfer>,
) {
    for face in FACES {
        let from = target.offset(face);
        let source = cells.cell(from);
        if source.element == held.element
            || source.mass_grams <= held.mass_grams
            || !is_gas(source, elements)
            || layers(cells, elements, from, target)
        {
            continue;
        }
        let faces = open_faces(cells, elements, from).max(open_faces(cells, elements, target));
        let grams = share(source.mass_grams, 0, faces, fraction);
        if grams > 0 {
            pushes.push(Transfer::new(from, source, target, grams));
        }
    }
}

/// Whether rule 4 governs the touching Cells `a` and `b`.
fn layers(cells: &impl Cells, elements: &ElementTable, a: CellPos, b: CellPos) -> bool {
    let (lower, upper) = if b == a.offset(UP) {
        (a, b)
    } else if a == b.offset(UP) {
        (b, a)
    } else {
        return false;
    };
    sinks_through(cells.cell(upper), cells.cell(lower), elements)
}

/// Rules 2 and 3: every gas Cell moves part of its Mass into each Vacuum face neighbour, and
/// toward each emptier face neighbour holding the same gas.
///
/// A Vacuum Cell becomes only one Element per Tick: when different gases reach it at once, the
/// one offering the most grams wins, and the others' offers are withdrawn.
fn spread(chunks: &mut ActiveChunks, elements: &ElementTable, fraction: f64, tick: u64) {
    let planned = {
        let grid = Grid::new(chunks);
        grid.plan_by_chunk(
            |cell| is_gas(cell, elements),
            |cells, from, source, planned| {
                // The larger open-face count of two touching Cells divides each face's flow. The
                // source's own count is the same for every face, so it is counted once, and
                // only if something might move.
                let mut open_here = None;
                for face in FACES {
                    let to = from.offset(face);
                    let target = cells.cell(to);
                    if target.element != ElementId::VACUUM && target.element != source.element {
                        continue;
                    }
                    // Nothing moves toward a Cell holding as much or more, whatever the face
                    // count.
                    if source.mass_grams <= target.mass_grams {
                        continue;
                    }
                    let open_here =
                        *open_here.get_or_insert_with(|| open_faces(cells, elements, from));
                    let faces = open_here.max(open_faces(cells, elements, to));
                    let grams = share(source.mass_grams, target.mass_grams, faces, fraction);
                    if grams > 0 {
                        let into_vacuum = target.element == ElementId::VACUUM;
                        planned.push((Transfer::new(from, source, to, grams), into_vacuum));
                    }
                }
            },
        )
    };
    let (mut transfers, into_vacuum) = split_off_vacuum_claims(planned);
    transfers.push(settle_vacuum_claims(into_vacuum, tick));
    apply_batches(chunks, transfers);
}

/// How many of a Cell's faces touch gas or Vacuum, and so could carry gas.
fn open_faces(cells: &impl Cells, elements: &ElementTable, pos: CellPos) -> u32 {
    FACES
        .into_iter()
        .filter(|&face| is_open(cells.cell(pos.offset(face)), elements))
        .count() as u32
}

fn is_open(cell: Cell, elements: &ElementTable) -> bool {
    matches!(elements.get(cell.element).phase, Phase::Gas | Phase::Vacuum)
}

pub(crate) fn is_gas(cell: Cell, elements: &ElementTable) -> bool {
    elements.get(cell.element).phase == Phase::Gas
}
