//! Moving Mass between Cells: the machinery that gas and liquid flow share.
//!
//! A flow rule plans its moves as [`Transfer`]s from the state at the start of a pass, then hands
//! the whole batch to [`apply`], which carries it out at once. Moved Mass mixes its heat into
//! whatever it joins (rule 1 of `docs/plans/02-gas-flow.md`). Whole-Cell swaps between layers go
//! through [`swap_layers`], and pushes into a different gas through
//! [`displace`](crate::displacement::displace).
//!
//! Wherever a rule has to choose between equal options, it uses a deterministic pseudo-random
//! tie-break seeded by the Tick count and Cell position, so nothing depends on `HashMap`
//! iteration order, or on how the work is split across threads (`grid.rs`), and the same state
//! and Tick count always give the same result.

use std::collections::BTreeSet;

use rayon::prelude::*;

use crate::cell::{Cell, CellPos};
use crate::chunk::{ActiveChunks, CHUNK_VOLUME, Chunk, ChunkPos, locate};
use crate::element::ElementId;
use crate::grid::{Cells, Grid, Layout, change_chunks, write_cells};

pub(crate) const UP: (i32, i32, i32) = (0, 1, 0);
pub(crate) const DOWN: (i32, i32, i32) = (0, -1, 0);

/// The six face neighbours of a Cell.
pub(crate) const FACES: [(i32, i32, i32); 6] = [
    (1, 0, 0),
    (-1, 0, 0),
    (0, 1, 0),
    (0, -1, 0),
    (0, 0, 1),
    (0, 0, -1),
];

/// The four face neighbours at a Cell's own height.
pub(crate) const SIDEWAYS: [(i32, i32, i32); 4] = [(1, 0, 0), (-1, 0, 0), (0, 0, 1), (0, 0, -1)];

/// Mass on its way from one Cell to a face neighbour, carrying the source's temperature.
#[derive(Clone, Copy)]
pub(crate) struct Transfer {
    pub from: CellPos,
    pub to: CellPos,
    pub element: ElementId,
    pub grams: u32,
    pub temperature_k: f32,
}

impl Transfer {
    /// `grams` of `source`, the Cell at `from`, moving to `to`.
    pub fn new(from: CellPos, source: Cell, to: CellPos, grams: u32) -> Self {
        Self {
            from,
            to,
            element: source.element,
            grams,
            temperature_k: source.temperature_k,
        }
    }
}

/// Grams that move across one face from a Cell holding `from_grams` toward one holding
/// `to_grams`: `fraction` of the difference, divided by `faces`.
pub(crate) fn share(from_grams: u32, to_grams: u32, faces: u32, fraction: f64) -> u32 {
    let difference = f64::from(from_grams.saturating_sub(to_grams));
    // Truncation rounds any flow below 1 g to zero (ADR 0004).
    (difference * fraction / f64::from(faces)) as u32
}

/// Swaps every Cell that `sinks` through the Cell directly below it with that Cell, entirely:
/// Element, Mass and temperature.
///
/// `sinks(above, below)` decides whether `above` sinks through `below`. It never holds for a
/// `below` that `can_be_sunk_through` rules out, which saves looking above every Cell.
pub(crate) fn swap_layers(
    chunks: &mut ActiveChunks,
    can_be_sunk_through: impl Fn(Cell) -> bool + Sync,
    sinks: impl Fn(Cell, Cell) -> bool + Sync,
) {
    let writes = {
        let grid = Grid::new(chunks);
        grid.plan(can_be_sunk_through, |cells, lower, below, writes| {
            let upper = lower.offset(UP);
            let above = cells.cell(upper);
            if sinks(above, below) && pairs_sinking_below(cells, &sinks, lower) % 2 == 0 {
                writes.push((lower, above));
                writes.push((upper, below));
            }
        })
    };
    // No Cell swaps twice, so every swap reads the state at the start.
    write_cells(chunks, writes);
}

/// How many pairs of Cells directly beneath `lower` each sink through the Cell below them, one
/// after another.
///
/// A stack whose Cells each sink through the one below forms a run of pairs that each want to
/// swap, and neighbouring pairs share a Cell. Starting from the bottom of the run, every other
/// pair swaps, so no Cell swaps twice: `lower` swaps with the Cell above it only if this count is
/// even. Every rule passed in as `sinks` ranks Elements strictly (gas by Density; liquid above
/// any gas, then by Density), so no Element appears twice in a run, and the run is never longer
/// than the number of Elements.
fn pairs_sinking_below(
    cells: &impl Cells,
    sinks: &impl Fn(Cell, Cell) -> bool,
    lower: CellPos,
) -> u32 {
    let sinks_into = |lower: CellPos| sinks(cells.cell(lower.offset(UP)), cells.cell(lower));
    let mut pairs_below = 0;
    let mut below = lower.offset(DOWN);
    while sinks_into(below) {
        pairs_below += 1;
        below = below.offset(DOWN);
    }
    pairs_below
}

/// Splits moves planned Chunk by Chunk, each marked with whether it moves Mass into a Vacuum
/// Cell, into the moves that go ahead as planned, still Chunk by Chunk, and the offers for
/// Vacuum Cells, which race for them (see [`settle_vacuum_claims`]).
pub(crate) fn split_off_vacuum_claims(
    planned: Vec<Vec<(Transfer, bool)>>,
) -> (Vec<Vec<Transfer>>, Vec<Transfer>) {
    let split: Vec<(Vec<Transfer>, Vec<Transfer>)> = planned
        .into_par_iter()
        .map(|planned| {
            let (mut transfers, mut into_vacuum) = (Vec::with_capacity(planned.len()), Vec::new());
            for (transfer, claims_vacuum) in planned {
                if claims_vacuum {
                    into_vacuum.push(transfer);
                } else {
                    transfers.push(transfer);
                }
            }
            (transfers, into_vacuum)
        })
        .collect();
    let mut all_into_vacuum = Vec::new();
    let transfers = split
        .into_iter()
        .map(|(transfers, into_vacuum)| {
            all_into_vacuum.extend(into_vacuum);
            transfers
        })
        .collect();
    (transfers, all_into_vacuum)
}

/// Withdraws the offers that lose the race for a Vacuum Cell. A Vacuum Cell becomes only one
/// Element per Tick: of the Elements offered to it, the one offering the most grams wins.
///
/// Every offer in `offers` must move Mass into a Vacuum Cell.
pub(crate) fn settle_vacuum_claims(mut offers: Vec<Transfer>, tick: u64) -> Vec<Transfer> {
    offers.sort_by_key(|offer| (key(offer.to), key(offer.from)));
    let mut winners = Vec::new();
    for offers in offers.chunk_by(|a, b| a.to == b.to) {
        let winner = strongest_element(offers, tick, offers[0].to);
        winners.extend(
            offers
                .iter()
                .filter(|offer| Some(offer.element) == winner)
                .cloned(),
        );
    }
    winners
}

/// The Element offering the most grams in total. Equal offers are settled by a tie-break seeded
/// by the Tick count and `pos`.
pub(crate) fn strongest_element(offers: &[Transfer], tick: u64, pos: CellPos) -> Option<ElementId> {
    // Offers into one Cell come from its face neighbours, so there are rarely more than six;
    // their totals are kept on the stack.
    const FEW: usize = 6;
    let unused = (0, ElementId::VACUUM, 0);
    let (mut few, mut many) = ([unused; FEW], Vec::new());
    let totals: &mut [(u16, ElementId, u64)] = if offers.len() <= FEW {
        &mut few
    } else {
        many.resize(offers.len(), unused);
        &mut many
    };
    let mut len = 0;
    for offer in offers {
        let grams = u64::from(offer.grams);
        match totals[..len]
            .iter_mut()
            .find(|(_, element, _)| *element == offer.element)
        {
            Some((_, _, total)) => *total += grams,
            None => {
                totals[len] = (offer.element.index(), offer.element, grams);
                len += 1;
            }
        }
    }
    // Sorted by index so tied Elements are listed in a fixed order.
    let totals = &mut totals[..len];
    totals.sort_unstable_by_key(|&(index, _, _)| index);
    let most = totals.iter().map(|&(_, _, total)| total).max()?;
    let tied = totals
        .iter()
        .filter(|&&(_, _, total)| total == most)
        .map(|&(_, element, _)| element);
    let count = tied.clone().count();
    tied.clone().nth(pick(count, tick, pos, TieBreak::Element))
}

/// What a tie-break is deciding, so separate decisions at one Cell don't share a coin toss.
#[derive(Clone, Copy)]
pub(crate) enum TieBreak {
    Order,
    Element,
    Exit,
    Shed,
}

/// A pseudo-random index below `len`, the same for the same Tick, Cell and decision.
pub(crate) fn pick(len: usize, tick: u64, pos: CellPos, decision: TieBreak) -> usize {
    (tie_break(tick, pos, decision) % len as u64) as usize
}

/// A deterministic pseudo-random number for one decision at one Cell in one Tick.
pub(crate) fn tie_break(tick: u64, pos: CellPos, decision: TieBreak) -> u64 {
    [pos.x, pos.y, pos.z, decision as i32]
        .into_iter()
        .fold(mix(tick), |hash, part| mix(hash ^ u64::from(part as u32)))
}

/// The SplitMix64 finaliser: spreads every input bit across the whole output.
fn mix(value: u64) -> u64 {
    let mut z = value.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A sort key for positions, so planned moves can be put in a fixed order.
pub(crate) fn key(pos: CellPos) -> (i32, i32, i32) {
    (pos.x, pos.y, pos.z)
}

/// One end of a transfer, as seen by the Chunk holding that Cell.
#[derive(Clone, Copy)]
struct End {
    /// The Cell's index within its Chunk.
    index: usize,
    /// Where the Mass comes from, as a sort key: arrivals at one Cell mix their heat in this order.
    from: (i32, i32, i32),
    grams: u32,
    /// Whether the Mass arrives here, or leaves.
    arrives: bool,
    temperature_k: f32,
    element: ElementId,
}

/// Applies every transfer at once. Moved Mass mixes its heat into what it joins: `m` grams at
/// `Ts` joining `M` grams at `Td` give `(m·Ts + M·Td) / (m + M)`, and a Vacuum Cell that receives
/// Mass takes on the arriving Element and temperature.
///
/// A Cell may only receive an Element it already holds, or one that replaces everything it held.
///
/// Each Chunk works out its own Cells, one task per Chunk. Every Cell mixes its arrivals in the
/// order of where they come from, whatever order they were planned in, so the result never
/// depends on planning order or thread count.
pub(crate) fn apply(chunks: &mut ActiveChunks, transfers: Vec<Transfer>) {
    apply_batches(chunks, vec![transfers]);
}

/// Like [`apply`], for transfers planned in batches, such as one per Chunk.
pub(crate) fn apply_batches(chunks: &mut ActiveChunks, batches: Vec<Vec<Transfer>>) {
    if batches.iter().all(Vec::is_empty) {
        return;
    }
    // Transfers are sorted into Chunks a part at a time, in parallel. However the parts fall,
    // each Cell sorts its own ends below, so the result never depends on them.
    const PART: usize = 4096;
    let parts = || batches.par_iter().flat_map(|batch| batch.par_chunks(PART));
    // A Chunk holding nothing but Vacuum isn't stored until Mass enters it. Mass only leaves
    // Cells that hold some, so every source's Chunk is stored already.
    let entered: BTreeSet<ChunkPos> = {
        let stored = &**chunks;
        parts()
            .map(|part| {
                part.iter()
                    .map(|transfer| ChunkPos::of(transfer.to))
                    .zip(part.iter().map(|transfer| ChunkPos::of(transfer.from)))
                    .filter(|&(to, from)| to != from && !stored.contains_key(&to))
                    .map(|(to, _)| to)
                    .collect::<BTreeSet<_>>()
            })
            .reduce(BTreeSet::new, |mut all, part| {
                all.extend(part);
                all
            })
    };
    for chunk in entered {
        debug_assert!(
            chunks.region.contains(chunk),
            "{chunk:?} lies beyond the Active Region's sealed edge"
        );
        chunks.insert(chunk, Chunk::new());
    }
    let layout = Layout::new(chunks);
    let parts: Vec<Vec<Vec<End>>> = parts()
        .map(|part| {
            let mut sorted: Vec<Vec<End>> = (0..layout.len()).map(|_| Vec::new()).collect();
            for transfer in part {
                let from = key(transfer.from);
                for (pos, arrives) in [(transfer.from, false), (transfer.to, true)] {
                    let (chunk, index) = locate(pos);
                    let slot = layout.slot(chunk).expect("stored above");
                    sorted[slot].push(End {
                        index,
                        from,
                        grams: transfer.grams,
                        arrives,
                        temperature_k: transfer.temperature_k,
                        element: transfer.element,
                    });
                }
            }
            sorted
        })
        .collect();
    let sorted: Vec<Vec<End>> = (0..layout.len())
        .into_par_iter()
        .map(|slot| parts.iter().flat_map(|part| &part[slot]).copied().collect())
        .collect();
    change_chunks(chunks, &layout, sorted, |chunk, ends| {
        for ends in by_cell(ends).chunk_by_mut(|a, b| a.index == b.index) {
            // Each Cell has only a few ends. Only departures from one Cell share a key, and the
            // order they are added up in doesn't matter.
            ends.sort_unstable_by_key(|end| end.from);
            let index = ends[0].index;
            let cell = change(chunk.cells[index], ends);
            chunk.write(index, cell);
        }
    });
}

/// `ends`, all in one Chunk, grouped by Cell in the order of the Cells' indices: a counting sort,
/// since a Chunk may hold many ends but each Cell only a few.
fn by_cell(ends: Vec<End>) -> Vec<End> {
    let mut starts = vec![0_u32; CHUNK_VOLUME + 1];
    for end in &ends {
        starts[end.index + 1] += 1;
    }
    for index in 0..CHUNK_VOLUME {
        starts[index + 1] += starts[index];
    }
    let mut grouped = ends.clone();
    for end in ends {
        let at = &mut starts[end.index];
        grouped[*at as usize] = end;
        *at += 1;
    }
    grouped
}

/// What `cell` becomes once everything in `ends`, all of which leave it or arrive at it, has
/// moved. Arrivals must be listed in the order of where they come from.
fn change(cell: Cell, ends: &[End]) -> Cell {
    let mut departed = 0;
    let mut arrived = 0;
    // The sum of grams × kelvin over everything that arrived.
    let mut arrived_heat = 0.0;
    let mut arrived_element = None;
    for end in ends {
        if end.arrives {
            arrived += u64::from(end.grams);
            arrived_heat += f64::from(end.grams) * f64::from(end.temperature_k);
            arrived_element = Some(end.element);
        } else {
            departed += end.grams;
        }
    }
    let stayed = cell.mass_grams - departed;
    let mass_grams =
        u32::try_from(u64::from(stayed) + arrived).expect("a Cell holds at most u32::MAX grams");
    if mass_grams == 0 {
        return Cell::VACUUM;
    }
    debug_assert!(
        stayed == 0 || arrived_element.is_none_or(|element| element == cell.element),
        "Mass arrived at a Cell that still holds a different Element"
    );
    let element = match arrived_element {
        Some(element) if stayed == 0 => element,
        _ => cell.element,
    };
    let heat = f64::from(stayed) * f64::from(cell.temperature_k) + arrived_heat;
    Cell {
        element,
        mass_grams,
        temperature_k: (heat / f64::from(mass_grams)) as f32,
    }
}
