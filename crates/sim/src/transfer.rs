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
//! iteration order and the same state and Tick count always give the same result.

use std::collections::{BTreeMap, HashMap};

use crate::cell::{Cell, CellPos};
use crate::chunk::{CHUNK_VOLUME, Chunk, ChunkPos, cell_at, set_cell};
use crate::element::ElementId;

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
#[derive(Clone)]
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

/// The position of every Cell in the world for which `keep` holds.
pub(crate) fn cells_where(
    chunks: &HashMap<ChunkPos, Chunk>,
    keep: impl Fn(Cell) -> bool,
) -> Vec<CellPos> {
    let mut cells = Vec::new();
    for (&chunk_pos, chunk) in chunks {
        for index in 0..CHUNK_VOLUME {
            if keep(chunk.cells[index]) {
                cells.push(chunk_pos.cell_pos(index));
            }
        }
    }
    cells
}

/// Swaps every Cell that `sinks` through the Cell directly below it with that Cell, entirely:
/// Element, Mass and temperature.
///
/// `sinks(above, below)` decides whether `above` sinks through `below`.
pub(crate) fn swap_layers(
    chunks: &mut HashMap<ChunkPos, Chunk>,
    sinks: impl Fn(Cell, Cell) -> bool,
) {
    let swaps: Vec<CellPos> = cells_where(chunks, |cell| cell.element != ElementId::VACUUM)
        .into_iter()
        .filter(|&lower| swaps_upward(chunks, &sinks, lower))
        .collect();
    for lower in swaps {
        let upper = lower.offset(UP);
        let (below, above) = (cell_at(chunks, lower), cell_at(chunks, upper));
        set_cell(chunks, lower, above);
        set_cell(chunks, upper, below);
    }
}

/// Whether `lower` swaps with the Cell above it this Tick.
///
/// A stack whose Cells each sink through the one below forms a run of pairs that each want to
/// swap, and neighbouring pairs share a Cell. Starting from the bottom of the run, every other
/// pair swaps, so no Cell swaps twice. Every rule passed in as `sinks` ranks Elements strictly
/// (gas by Density; liquid above any gas, then by Density), so no Element appears twice in a run,
/// and the run is never longer than the number of Elements.
fn swaps_upward(
    chunks: &HashMap<ChunkPos, Chunk>,
    sinks: &impl Fn(Cell, Cell) -> bool,
    lower: CellPos,
) -> bool {
    let sinks_into =
        |lower: CellPos| sinks(cell_at(chunks, lower.offset(UP)), cell_at(chunks, lower));
    if !sinks_into(lower) {
        return false;
    }
    let mut pairs_below = 0;
    let mut below = lower.offset(DOWN);
    while sinks_into(below) {
        pairs_below += 1;
        below = below.offset(DOWN);
    }
    pairs_below % 2 == 0
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
    // Keyed by index so tied Elements are listed in a fixed order.
    let mut totals: BTreeMap<u16, (ElementId, u64)> = BTreeMap::new();
    for offer in offers {
        totals
            .entry(offer.element.index())
            .or_insert((offer.element, 0))
            .1 += u64::from(offer.grams);
    }
    let most = totals.values().map(|&(_, total)| total).max()?;
    let tied: Vec<ElementId> = totals
        .into_values()
        .filter(|&(_, total)| total == most)
        .map(|(element, _)| element)
        .collect();
    Some(tied[pick(tied.len(), tick, pos, TieBreak::Element)])
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

/// What happens to one Cell when a batch of transfers is applied.
#[derive(Default)]
struct Change {
    departed: u32,
    arrived: u64,
    /// The sum of grams × kelvin over everything that arrived.
    arrived_heat: f64,
    arrived_element: Option<ElementId>,
}

/// Applies every transfer at once. Moved Mass mixes its heat into what it joins: `m` grams at
/// `Ts` joining `M` grams at `Td` give `(m·Ts + M·Td) / (m + M)`, and a Vacuum Cell that receives
/// Mass takes on the arriving Element and temperature.
///
/// A Cell may only receive an Element it already holds, or one that replaces everything it held.
pub(crate) fn apply(chunks: &mut HashMap<ChunkPos, Chunk>, mut transfers: Vec<Transfer>) {
    // Sorted so every Cell mixes its arrivals in the same order, whatever order they were
    // planned in.
    let mut changes: BTreeMap<(i32, i32, i32), Change> = BTreeMap::new();
    transfers.sort_by_key(|transfer| (key(transfer.to), key(transfer.from)));
    for transfer in transfers {
        changes.entry(key(transfer.from)).or_default().departed += transfer.grams;
        let arrival = changes.entry(key(transfer.to)).or_default();
        arrival.arrived += u64::from(transfer.grams);
        arrival.arrived_heat += f64::from(transfer.grams) * f64::from(transfer.temperature_k);
        arrival.arrived_element = Some(transfer.element);
    }
    for ((x, y, z), change) in changes {
        let pos = CellPos::new(x, y, z);
        let cell = cell_at(chunks, pos);
        let stayed = cell.mass_grams - change.departed;
        let mass_grams = u32::try_from(u64::from(stayed) + change.arrived)
            .expect("a Cell holds at most u32::MAX grams");
        if mass_grams == 0 {
            set_cell(chunks, pos, Cell::VACUUM);
            continue;
        }
        debug_assert!(
            stayed == 0
                || change
                    .arrived_element
                    .is_none_or(|element| element == cell.element),
            "Mass arrived at {pos:?}, which still holds a different Element"
        );
        let element = match change.arrived_element {
            Some(element) if stayed == 0 => element,
            _ => cell.element,
        };
        let heat = f64::from(stayed) * f64::from(cell.temperature_k) + change.arrived_heat;
        set_cell(
            chunks,
            pos,
            Cell {
                element,
                mass_grams,
                temperature_k: (heat / f64::from(mass_grams)) as f32,
            },
        );
    }
}
