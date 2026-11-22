//! Gas flow: rules 1–6 of `docs/plans/02-gas-flow.md`.
//!
//! Each Tick runs three passes, in this order: layering (rule 4), Displacement (rule 5), then
//! spreading into Vacuum and evening out (rules 2 and 3). Every pass plans all of its moves from
//! the state at its own start and then applies them together, the way heat does. Moved Mass
//! mixes its heat into whatever it joins (rule 1). Only gas and Vacuum Cells take part, so gas
//! never enters a solid, and liquids count as solids until step 03 (rule 6).
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
//! Across each face, a gas Cell moves [`EVENING_OUT_FRACTION`] of the Mass difference, divided by
//! the larger number of open faces of the two Cells (a Vacuum neighbour counts as 0 g). So a lone
//! pair moves exactly that fraction. However many faces flow at once, each Cell's new Mass stays
//! a blend of its neighbours' old Mass, so nothing overshoots, and no pass takes more than that
//! fraction of a gas Cell's Mass, so it never empties. Flows are truncated to whole grams, which
//! is how gas comes to rest (ADR 0004).

use std::collections::{BTreeMap, HashMap};

use crate::cell::{Cell, CellPos};
use crate::chunk::{CHUNK_VOLUME, Chunk, ChunkPos, cell_at, set_cell};
use crate::element::{ElementId, ElementTable, Phase};

/// The fraction of the Mass difference that a lone pair of touching Cells evens out per Tick.
///
/// Up to 0.5, which would even out a lone pair in one Tick; any more would overshoot.
pub(crate) const EVENING_OUT_FRACTION: f64 = 0.25;
const _: () = assert!(EVENING_OUT_FRACTION > 0.0 && EVENING_OUT_FRACTION <= 0.5);

const UP: (i32, i32, i32) = (0, 1, 0);
const DOWN: (i32, i32, i32) = (0, -1, 0);

/// The six face neighbours of a Cell.
const FACES: [(i32, i32, i32); 6] = [
    (1, 0, 0),
    (-1, 0, 0),
    (0, 1, 0),
    (0, -1, 0),
    (0, 0, 1),
    (0, 0, -1),
];

/// Moves gas Mass for one Tick.
pub(crate) fn flow(chunks: &mut HashMap<ChunkPos, Chunk>, elements: &ElementTable, tick: u64) {
    layer(chunks, elements);
    displace(chunks, elements, tick);
    spread(chunks, elements, tick);
}

/// Rule 4: a gas Cell directly above a different, less dense gas swaps places with it entirely.
fn layer(chunks: &mut HashMap<ChunkPos, Chunk>, elements: &ElementTable) {
    let swaps: Vec<CellPos> = gas_cells(chunks, elements)
        .into_iter()
        .filter(|&lower| swaps_upward(chunks, elements, lower))
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
/// Stacked gases that get denser going up form a run of pairs that each want to swap, and
/// neighbouring pairs share a Cell. Starting from the bottom of the run, every other pair swaps,
/// so no Cell swaps twice. The run is never longer than the number of gas Elements, because
/// Density rises strictly along it.
fn swaps_upward(
    chunks: &HashMap<ChunkPos, Chunk>,
    elements: &ElementTable,
    lower: CellPos,
) -> bool {
    if !sinks_through(chunks, elements, lower) {
        return false;
    }
    let mut pairs_below = 0;
    let mut below = lower.offset(DOWN);
    while sinks_through(chunks, elements, below) {
        pairs_below += 1;
        below = below.offset(DOWN);
    }
    pairs_below % 2 == 0
}

/// Whether the Cell above `lower` holds a gas that is denser than a different gas in `lower`.
fn sinks_through(
    chunks: &HashMap<ChunkPos, Chunk>,
    elements: &ElementTable,
    lower: CellPos,
) -> bool {
    let (below, above) = (cell_at(chunks, lower), cell_at(chunks, lower.offset(UP)));
    is_gas(below, elements)
        && is_gas(above, elements)
        && below.element != above.element
        && elements.get(above.element).density > elements.get(below.element).density
}

/// What a Cell is already doing in this Tick's Displacement, which rules out conflicting moves.
#[derive(Default)]
struct Role {
    /// Its gas has left entirely, making way for another gas.
    vacated: bool,
    /// Grams it is already pushing into vacated neighbours.
    pushed_grams: u32,
    /// The displaced gas already squeezing into it: its Element, and how many grams.
    squeezed_in: Option<(ElementId, u64)>,
}

/// Rule 5: Displacement. Gas pushes part of its Mass into a Cell of a different gas, whose
/// entire contents leave for a face neighbour holding the same gas or Vacuum.
///
/// A push needs the pushing Cell to hold more than the Cell it displaces, and to still hold more
/// after the move than the exit the displaced gas squeezes into. Without that second condition
/// two gases whose pressures can't balance in whole Cells would trade a Cell back and forth
/// forever, and gas would never come to rest (ADR 0004).
///
/// Cells are considered in a pseudo-random order seeded by the Tick count and position. Each
/// takes the first role offered: a vacated Cell can't push or receive, a pushing Cell can't be
/// vacated, and an exit can't be vacated or turn Vacuum into two different Elements.
fn displace(chunks: &mut HashMap<ChunkPos, Chunk>, elements: &ElementTable, tick: u64) {
    let mut candidates: Vec<CellPos> = gas_cells(chunks, elements)
        .into_iter()
        .filter(|&pos| {
            !pushes_into(chunks, elements, pos).is_empty() && !exits(chunks, pos).is_empty()
        })
        .collect();
    candidates.sort_by_key(|&pos| (tie_break(tick, pos, TieBreak::Order), key(pos)));

    let mut roles: HashMap<CellPos, Role> = HashMap::new();
    let mut transfers = Vec::new();
    for vacated in candidates {
        if roles.contains_key(&vacated) {
            continue;
        }
        let leaving = cell_at(chunks, vacated);
        // Each open exit, with the grams it will hold once this gas has squeezed in.
        let exits: Vec<(CellPos, u64)> = exits(chunks, vacated)
            .into_iter()
            .filter_map(|exit| {
                let role = roles.get(&exit);
                if role.is_some_and(|role| role.vacated) {
                    return None;
                }
                let already = match role.and_then(|role| role.squeezed_in) {
                    Some((element, _)) if element != leaving.element => return None,
                    Some((_, grams)) => grams,
                    None => 0,
                };
                let held = u64::from(cell_at(chunks, exit).mass_grams);
                Some((exit, held + already + u64::from(leaving.mass_grams)))
            })
            .collect();
        let Some((exit, squeezed)) = emptiest(&exits, tick, vacated) else {
            continue;
        };
        let pushes: Vec<Transfer> = pushes_into(chunks, elements, vacated)
            .into_iter()
            .filter(|push| {
                let role = roles.get(&push.from);
                let pushed = role.map_or(0, |role| role.pushed_grams);
                let keeps = cell_at(chunks, push.from).mass_grams - pushed - push.grams;
                !role.is_some_and(|role| role.vacated) && u64::from(keeps) > squeezed
            })
            .collect();
        let Some(pusher) = strongest_element(&pushes, tick, vacated) else {
            continue;
        };

        roles.entry(vacated).or_default().vacated = true;
        let exit_role = roles.entry(exit).or_default();
        let already = exit_role.squeezed_in.map_or(0, |(_, grams)| grams);
        exit_role.squeezed_in = Some((leaving.element, already + u64::from(leaving.mass_grams)));
        transfers.push(Transfer {
            from: vacated,
            to: exit,
            element: leaving.element,
            grams: leaving.mass_grams,
            temperature_k: leaving.temperature_k,
        });
        for push in pushes.into_iter().filter(|push| push.element == pusher) {
            roles.entry(push.from).or_default().pushed_grams += push.grams;
            transfers.push(push);
        }
    }
    apply(chunks, transfers);
}

/// Every push a different gas could make into `target`: from a neighbour holding more Mass,
/// moving what it would move if `target` were Vacuum. Pairs that layering (rule 4) decides are
/// left out.
fn pushes_into(
    chunks: &HashMap<ChunkPos, Chunk>,
    elements: &ElementTable,
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
        let grams = share(
            source.mass_grams,
            0,
            face_count(chunks, elements, from, target),
        );
        if grams > 0 {
            pushes.push(Transfer {
                from,
                to: target,
                element: source.element,
                grams,
                temperature_k: source.temperature_k,
            });
        }
    }
    pushes
}

/// Where the gas in `pos` could go if it were displaced: face neighbours holding the same gas,
/// or Vacuum.
fn exits(chunks: &HashMap<ChunkPos, Chunk>, pos: CellPos) -> Vec<CellPos> {
    let element = cell_at(chunks, pos).element;
    FACES
        .into_iter()
        .map(|face| pos.offset(face))
        .filter(|&exit| {
            let held = cell_at(chunks, exit).element;
            held == element || held == ElementId::VACUUM
        })
        .collect()
}

/// Whether rule 4 governs the touching Cells `a` and `b`.
fn layers(
    chunks: &HashMap<ChunkPos, Chunk>,
    elements: &ElementTable,
    a: CellPos,
    b: CellPos,
) -> bool {
    if b == a.offset(UP) {
        sinks_through(chunks, elements, a)
    } else if a == b.offset(UP) {
        sinks_through(chunks, elements, b)
    } else {
        false
    }
}

/// The Element offering the most grams in total. Equal offers are settled by a tie-break seeded
/// by the Tick count and `pos`.
fn strongest_element(offers: &[Transfer], tick: u64, pos: CellPos) -> Option<ElementId> {
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

/// The exit that will hold the fewest grams. Equal exits are settled by a tie-break seeded by
/// the Tick count and `pos`.
fn emptiest(exits: &[(CellPos, u64)], tick: u64, pos: CellPos) -> Option<(CellPos, u64)> {
    let least = exits.iter().map(|&(_, grams)| grams).min()?;
    let tied: Vec<(CellPos, u64)> = exits
        .iter()
        .copied()
        .filter(|&(_, grams)| grams == least)
        .collect();
    Some(tied[pick(tied.len(), tick, pos, TieBreak::Exit)])
}

/// What a tie-break is deciding, so separate decisions at one Cell don't share a coin toss.
#[derive(Clone, Copy)]
enum TieBreak {
    Order,
    Element,
    Exit,
}

/// A pseudo-random index below `len`, the same for the same Tick, Cell and decision.
fn pick(len: usize, tick: u64, pos: CellPos, decision: TieBreak) -> usize {
    (tie_break(tick, pos, decision) % len as u64) as usize
}

/// A deterministic pseudo-random number for one decision at one Cell in one Tick.
fn tie_break(tick: u64, pos: CellPos, decision: TieBreak) -> u64 {
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
fn key(pos: CellPos) -> (i32, i32, i32) {
    (pos.x, pos.y, pos.z)
}

/// Mass on its way from one Cell to a face neighbour, carrying the source's temperature.
#[derive(Clone)]
struct Transfer {
    from: CellPos,
    to: CellPos,
    element: ElementId,
    grams: u32,
    temperature_k: f32,
}

/// Rules 2 and 3: every gas Cell moves part of its Mass into each Vacuum face neighbour, and
/// toward each emptier face neighbour holding the same gas.
///
/// A Vacuum Cell becomes only one Element per Tick: when different gases reach it at once, the
/// one offering the most grams wins, and the others' offers are withdrawn.
fn spread(chunks: &mut HashMap<ChunkPos, Chunk>, elements: &ElementTable, tick: u64) {
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
            let grams = share(source.mass_grams, target.mass_grams, faces);
            if grams == 0 {
                continue;
            }
            let transfer = Transfer {
                from,
                to,
                element: source.element,
                grams,
                temperature_k: source.temperature_k,
            };
            if target.element == ElementId::VACUUM {
                into_vacuum.push(transfer);
            } else {
                transfers.push(transfer);
            }
        }
    }
    into_vacuum.sort_by_key(|offer| (key(offer.to), key(offer.from)));
    for offers in into_vacuum.chunk_by(|a, b| a.to == b.to) {
        let winner = strongest_element(offers, tick, offers[0].to);
        transfers.extend(
            offers
                .iter()
                .filter(|offer| Some(offer.element) == winner)
                .cloned(),
        );
    }
    apply(chunks, transfers);
}

/// Grams that move across one face from a Cell holding `from_grams` to one holding `to_grams`.
fn share(from_grams: u32, to_grams: u32, faces: u32) -> u32 {
    let difference = f64::from(from_grams.saturating_sub(to_grams));
    // Truncation rounds any flow below 1 g to zero (ADR 0004).
    (difference * EVENING_OUT_FRACTION / f64::from(faces)) as u32
}

/// The larger open-face count of two touching Cells.
fn face_count(
    chunks: &HashMap<ChunkPos, Chunk>,
    elements: &ElementTable,
    a: CellPos,
    b: CellPos,
) -> u32 {
    open_faces(chunks, elements, a).max(open_faces(chunks, elements, b))
}

/// How many of a Cell's faces touch gas or Vacuum, and so could carry gas.
fn open_faces(chunks: &HashMap<ChunkPos, Chunk>, elements: &ElementTable, pos: CellPos) -> u32 {
    FACES
        .into_iter()
        .filter(|&face| is_open(cell_at(chunks, pos.offset(face)), elements))
        .count() as u32
}

fn is_open(cell: Cell, elements: &ElementTable) -> bool {
    matches!(elements.get(cell.element).phase, Phase::Gas | Phase::Vacuum)
}

fn is_gas(cell: Cell, elements: &ElementTable) -> bool {
    elements.get(cell.element).phase == Phase::Gas
}

/// Every gas Cell in the world.
fn gas_cells(chunks: &HashMap<ChunkPos, Chunk>, elements: &ElementTable) -> Vec<CellPos> {
    let mut cells = Vec::new();
    for (&key, chunk) in chunks {
        for index in 0..CHUNK_VOLUME {
            if is_gas(chunk.cells[index], elements) {
                cells.push(key.cell_pos(index));
            }
        }
    }
    cells
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

/// Applies every transfer at once. Moved Mass mixes its heat into what it joins (rule 1).
fn apply(chunks: &mut HashMap<ChunkPos, Chunk>, mut transfers: Vec<Transfer>) {
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
            "gas arrived at {pos:?}, which still holds a different Element"
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
