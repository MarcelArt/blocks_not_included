//! Liquid flow: rules 1–5 of `docs/plans/03-liquid-flow.md`.
//!
//! A liquid Cell holds at most its capacity: its Element's Density × 1 m³ (rule 1). Each Tick
//! runs five passes, in this order:
//!
//! 1. **Shedding** (rule 1). A Cell over capacity, which only a Phase Transition can cause, moves
//!    the excess into its face neighbours in all six directions.
//! 2. **Layering** (rules 2 and 3). A liquid Cell directly above a gas, or above a different, less
//!    dense liquid, swaps places with it entirely.
//! 3. **Falling** (rule 2). A liquid Cell moves as much Mass as fits into Vacuum, or into a partly
//!    full Cell of the same liquid, directly below it.
//! 4. **Displacement** (rule 4). A liquid Cell that can't fall pushes part of its Mass sideways
//!    into gas, through Displacement (`displacement.rs`).
//! 5. **Spreading** (rule 4). A liquid Cell that can't fall moves part of its Mass sideways into
//!    Vacuum, and toward emptier Cells of the same liquid.
//!
//! Each pass plans its moves from the state at its own start, so a Cell that can fall when
//! spreading starts doesn't spread (rule 2), and a Cell over capacity sheds rather than spreads
//! (rule 1). Moved Mass mixes its heat into whatever it joins (`transfer.rs`). Only liquid, gas
//! and Vacuum Cells take part, so liquids never enter solids and solids never move (rule 5).
//!
//! # Ordering and bias
//!
//! As in gas flow (`gas.rs`), every pass plans all of its moves from a snapshot and applies them
//! together, rather than visiting directions in a shuffled order, so a mirror-symmetric setup
//! stays exactly symmetric, Tick by Tick. Conflicts are settled the same way:
//! - A Vacuum Cell reached by two different liquids goes to the one offering the most grams.
//! - Displacement takes candidates in a pseudo-random order, each Cell keeping the first role it
//!   gets.
//! - Stacks that want to swap swap every other pair from the bottom up, so no Cell swaps twice.
//! - Excess too small to split evenly when shedding goes one gram each to neighbours chosen by a
//!   pseudo-random tie-break.
//!
//! Tie-breaks are seeded by the Tick count and Cell position, so the same state and Tick count
//! always give the same result.
//!
//! # How much moves sideways
//!
//! Across each sideways face, a Cell moves [`SimSettings::liquid_spreading_fraction`] of the
//! Mass difference, divided by the larger number of non-solid sideways neighbours of the two
//! Cells. At the default of 0.5, a lone pair evens out in one Tick, and however many faces flow at
//! once, each Cell's new Mass stays a blend of its neighbours' old Mass: nothing overshoots or
//! passes capacity. A Vacuum or gas neighbour counts as 0 g.
//!
//! Within a pool, Cells of the same liquid even out with no threshold, like gas, until flows round
//! to zero (ADR 0004). This keeps pools level. A pool's edge advances into Vacuum or gas only
//! from a Cell holding at least [`SimSettings::liquid_spreading_threshold`] of its Capacity more
//! than the neighbour, so a pool stops spreading before it thins out to nothing. With that
//! threshold on every sideways move, each pair of neighbours stopped just under it and the steps
//! added up, leaving sloped mounds (an amendment to rule 4, approved by the human).

use std::collections::HashMap;

use crate::cell::{Cell, CellPos};
use crate::chunk::{Chunk, ChunkPos, cell_at};
use crate::displacement::displace;
use crate::element::{ElementId, ElementTable, Phase};
use crate::gas::is_gas;
use crate::settings::SimSettings;
use crate::transfer::{
    DOWN, FACES, SIDEWAYS, TieBreak, Transfer, apply, cells_where, pick, settle_vacuum_claims,
    share, swap_layers,
};

/// Moves liquid Mass for one Tick.
pub(crate) fn flow(
    chunks: &mut HashMap<ChunkPos, Chunk>,
    elements: &ElementTable,
    tick: u64,
    settings: &SimSettings,
) {
    shed(chunks, elements, tick);
    swap_layers(chunks, |above, below| sinks_through(above, below, elements));
    fall(chunks, elements);
    displace_gas(chunks, elements, settings, tick);
    spread(chunks, elements, settings, tick);
}

/// Rule 1: every liquid Cell holding more than its capacity sheds the excess into its face
/// neighbours, in all six directions: first into gas, through Displacement, then into Vacuum and
/// into Cells of the same liquid with room. A Cell with no such neighbour stays over capacity,
/// because compression is not modelled.
fn shed(chunks: &mut HashMap<ChunkPos, Chunk>, elements: &ElementTable, tick: u64) {
    let over_capacity = |chunks: &HashMap<ChunkPos, Chunk>| {
        cells_where(chunks, |cell| {
            is_liquid(cell, elements) && cell.mass_grams > capacity(cell.element, elements)
        })
    };
    let pushes = over_capacity(chunks)
        .into_iter()
        .flat_map(|from| shedding_offers(chunks, elements, tick, from, Shedding::IntoGasToo))
        .filter(|push| {
            let target = cell_at(chunks, push.to);
            // Displacement only ever pushes from the Cell holding more Mass.
            is_gas(target, elements) && cell_at(chunks, push.from).mass_grams > target.mass_grams
        })
        .collect();
    displace(chunks, tick, pushes);

    let mut transfers = Vec::new();
    let mut into_vacuum = Vec::new();
    for offer in over_capacity(chunks)
        .into_iter()
        .flat_map(|from| shedding_offers(chunks, elements, tick, from, Shedding::NotIntoGas))
    {
        if cell_at(chunks, offer.to).element == ElementId::VACUUM {
            into_vacuum.push(offer);
        } else {
            transfers.push(offer);
        }
    }
    transfers.extend(settle_vacuum_claims(into_vacuum, tick));
    apply(chunks, transfers);
}

/// Which neighbours an over-capacity Cell is shedding into.
#[derive(Clone, Copy, PartialEq)]
enum Shedding {
    /// Gas neighbours as well as Vacuum and same-liquid Cells with room. Every one of them
    /// counts when the excess is split, so gas gets only its fair share.
    IntoGasToo,
    /// Only Vacuum and same-liquid Cells with room.
    NotIntoGas,
}

/// What the over-capacity liquid Cell at `from` sheds into each neighbour that can take some.
///
/// The excess is split evenly between those neighbours. Grams that don't split evenly go one each
/// to the next few neighbours after a pseudo-random one, so even a few grams of excess always
/// find a way out. Every over-capacity Cell of the same liquid that touches a neighbour gets an
/// equal part of that neighbour's room, so together they never fill it past capacity. A gas
/// neighbour has the room of Vacuum, since once its gas has gone (Displacement) the liquid fills
/// it as if it were Vacuum.
fn shedding_offers(
    chunks: &HashMap<ChunkPos, Chunk>,
    elements: &ElementTable,
    tick: u64,
    from: CellPos,
    shedding: Shedding,
) -> Vec<Transfer> {
    let source = cell_at(chunks, from);
    let capacity = capacity(source.element, elements);
    let excess = source.mass_grams - capacity;
    let open: Vec<(CellPos, u32)> = FACES
        .into_iter()
        .map(|face| from.offset(face))
        .filter_map(|to| {
            let target = cell_at(chunks, to);
            let room = if shedding == Shedding::IntoGasToo && is_gas(target, elements) {
                capacity
            } else {
                room_for(source, target, elements)
            };
            (room > 0).then_some((to, room))
        })
        .collect();
    if open.is_empty() {
        return Vec::new();
    }
    let count = open.len();
    let (each, remainder) = (excess / count as u32, excess as usize % count);
    let first = pick(count, tick, from, TieBreak::Shed);
    let mut offers = Vec::new();
    for (i, (to, room)) in open.into_iter().enumerate() {
        let extra = u32::from((i + count - first) % count < remainder);
        let sharers = over_capacity_neighbours(chunks, elements, to, source.element);
        let grams = (each + extra).min(room / sharers);
        if grams > 0 {
            offers.push(Transfer::new(from, source, to, grams));
        }
    }
    offers
}

/// How many over-capacity Cells of `liquid` touch `pos`, counting at least one.
fn over_capacity_neighbours(
    chunks: &HashMap<ChunkPos, Chunk>,
    elements: &ElementTable,
    pos: CellPos,
    liquid: ElementId,
) -> u32 {
    let capacity = capacity(liquid, elements);
    let count = FACES
        .into_iter()
        .map(|face| cell_at(chunks, pos.offset(face)))
        .filter(|cell| cell.element == liquid && cell.mass_grams > capacity)
        .count();
    count.max(1) as u32
}

/// Whether liquid `above` sinks through the Cell `below`, swapping with it entirely: through any
/// gas, because liquids always sit below gases (rule 2), and through a different, less dense
/// liquid (rule 3). The decision uses each Element's Density only, never how much Mass the Cells
/// hold.
fn sinks_through(above: Cell, below: Cell, elements: &ElementTable) -> bool {
    if !is_liquid(above, elements) {
        return false;
    }
    is_gas(below, elements)
        || (is_liquid(below, elements)
            && below.element != above.element
            && elements.get(above.element).density > elements.get(below.element).density)
}

/// Rule 2: every liquid Cell moves as much Mass as fits into Vacuum, or into a partly full Cell
/// of the same liquid, directly below it.
///
/// Each Cell receives from the one Cell above it at most, and never more than its room, so no
/// Cell ends up over capacity, even one that is also falling.
fn fall(chunks: &mut HashMap<ChunkPos, Chunk>, elements: &ElementTable) {
    let mut transfers = Vec::new();
    for from in cells_where(chunks, |cell| is_liquid(cell, elements)) {
        let source = cell_at(chunks, from);
        let to = from.offset(DOWN);
        let grams = source
            .mass_grams
            .min(room_for(source, cell_at(chunks, to), elements));
        if grams > 0 {
            transfers.push(Transfer::new(from, source, to, grams));
        }
    }
    apply(chunks, transfers);
}

/// Rule 4: every liquid Cell that can't fall pushes part of its Mass sideways into neighbouring
/// gas, through Displacement.
fn displace_gas(
    chunks: &mut HashMap<ChunkPos, Chunk>,
    elements: &ElementTable,
    settings: &SimSettings,
    tick: u64,
) {
    let pushes = cells_where(chunks, |cell| is_liquid(cell, elements))
        .into_iter()
        .flat_map(|from| spreading_offers(chunks, elements, settings, from))
        .filter(|offer| is_gas(cell_at(chunks, offer.to), elements))
        .collect();
    displace(chunks, tick, pushes);
}

/// Rule 4: every liquid Cell that can't fall moves part of its Mass sideways, into Vacuum and
/// toward emptier Cells of the same liquid.
///
/// A Vacuum Cell becomes only one Element per Tick: when different liquids reach it at once, the
/// one offering the most grams wins, and the others' offers are withdrawn.
fn spread(
    chunks: &mut HashMap<ChunkPos, Chunk>,
    elements: &ElementTable,
    settings: &SimSettings,
    tick: u64,
) {
    let mut transfers = Vec::new();
    let mut into_vacuum = Vec::new();
    for from in cells_where(chunks, |cell| is_liquid(cell, elements)) {
        for offer in spreading_offers(chunks, elements, settings, from) {
            let target = cell_at(chunks, offer.to);
            if target.element == ElementId::VACUUM {
                into_vacuum.push(offer);
            } else if target.element == offer.element {
                transfers.push(offer);
            }
        }
    }
    transfers.extend(settle_vacuum_claims(into_vacuum, tick));
    apply(chunks, transfers);
}

/// Everything the liquid Cell at `from` would move sideways this Tick, if it can't fall and isn't
/// over capacity:
///
/// - toward each emptier neighbour holding the same liquid. Like gas evening out, this has no
///   threshold, so a pool levels out until its flows round to zero (ADR 0004);
/// - into each neighbour holding Vacuum or gas, but only if `from` holds at least the spreading
///   threshold more than it. This is the edge of a pool advancing, and the threshold stops a pool
///   thinning out forever. A gas neighbour is offered what Vacuum would be, since once its gas has
///   gone (Displacement) the liquid fills it as if it were Vacuum.
fn spreading_offers(
    chunks: &HashMap<ChunkPos, Chunk>,
    elements: &ElementTable,
    settings: &SimSettings,
    from: CellPos,
) -> Vec<Transfer> {
    let source = cell_at(chunks, from);
    let capacity = capacity(source.element, elements);
    if source.mass_grams > capacity || can_fall(chunks, elements, from) {
        return Vec::new();
    }
    let threshold = (f64::from(capacity) * settings.liquid_spreading_threshold) as u64;
    let mut offers = Vec::new();
    for face in SIDEWAYS {
        let to = from.offset(face);
        let target = cell_at(chunks, to);
        let advances = matches!(
            elements.get(target.element).phase,
            Phase::Vacuum | Phase::Gas
        );
        let fills_from = if target.element == source.element {
            target.mass_grams
        } else if advances
            && u64::from(source.mass_grams) >= u64::from(target.mass_grams) + threshold
        {
            0
        } else {
            continue;
        };
        let faces = sideways_faces(chunks, elements, from, to);
        let grams = share(
            source.mass_grams,
            fills_from,
            faces,
            settings.liquid_spreading_fraction,
        );
        if grams > 0 {
            offers.push(Transfer::new(from, source, to, grams));
        }
    }
    offers
}

/// Whether the liquid Cell at `pos` can fall: into the Cell below it, or through it.
fn can_fall(chunks: &HashMap<ChunkPos, Chunk>, elements: &ElementTable, pos: CellPos) -> bool {
    let (cell, below) = (cell_at(chunks, pos), cell_at(chunks, pos.offset(DOWN)));
    room_for(cell, below, elements) > 0 || sinks_through(cell, below, elements)
}

/// The larger count of non-solid sideways neighbours of two sideways neighbours.
///
/// Dividing each face's flow by this keeps every Cell's new Mass a blend of its neighbours' old
/// Mass, however many faces flow at once, so nothing overshoots.
fn sideways_faces(
    chunks: &HashMap<ChunkPos, Chunk>,
    elements: &ElementTable,
    a: CellPos,
    b: CellPos,
) -> u32 {
    let open = |pos: CellPos| {
        SIDEWAYS
            .into_iter()
            .filter(|&face| {
                elements
                    .get(cell_at(chunks, pos.offset(face)).element)
                    .phase
                    != Phase::Solid
            })
            .count() as u32
    };
    open(a).max(open(b))
}

/// How many grams of `liquid` the Cell `target` can take: a full Cell's worth if it is Vacuum, the
/// rest of its capacity if it holds the same liquid, and nothing otherwise.
fn room_for(liquid: Cell, target: Cell, elements: &ElementTable) -> u32 {
    let capacity = capacity(liquid.element, elements);
    if target.element == ElementId::VACUUM {
        capacity
    } else if target.element == liquid.element {
        capacity.saturating_sub(target.mass_grams)
    } else {
        0
    }
}

/// The most grams one Cell of `liquid` holds: its Density × 1 m³ (rule 1).
pub(crate) fn capacity(liquid: ElementId, elements: &ElementTable) -> u32 {
    // kg/m³ × 1 m³ × 1000 g/kg.
    (f64::from(elements.get(liquid).density) * 1_000.0).round() as u32
}

fn is_liquid(cell: Cell, elements: &ElementTable) -> bool {
    elements.get(cell.element).phase == Phase::Liquid
}
