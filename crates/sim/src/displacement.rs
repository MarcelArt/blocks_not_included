//! Displacement (`CONTEXT.md`): gas or liquid moving part of its Mass into a neighbouring Cell of
//! a different gas, once that gas has left entirely for a face neighbour holding the same gas, or
//! for Vacuum.
//!
//! Gas flow (rule 5 of `docs/plans/02-gas-flow.md`) and liquid flow (rules 1 and 4 of
//! `docs/plans/03-liquid-flow.md`) each decide which of their Cells push into which gas, and how
//! much, then hand those pushes to [`displace`]. Only a Cell holding more Mass than the gas Cell
//! it pushes into may push. `displace` decides which pushes can go ahead and carries them out.

use std::collections::HashMap;

use crate::cell::CellPos;
use crate::chunk::{Chunk, ChunkPos, cell_at};
use crate::element::ElementId;
use crate::transfer::{FACES, TieBreak, Transfer, apply, key, pick, strongest_element, tie_break};

/// What a Cell is already doing in this round of Displacement, which rules out conflicting moves.
#[derive(Default)]
struct Role {
    /// Its gas has left entirely, making way for another Element.
    vacated: bool,
    /// Grams it is already pushing into vacated neighbours.
    pushed_grams: u32,
    /// The displaced gas already squeezing into it: its Element, and how many grams.
    squeezed_in: Option<(ElementId, u64)>,
}

/// Carries out every push in `pushes` that Displacement allows, and drops the rest.
///
/// Each push moves Mass into a face neighbour holding a different gas, planned as if that Cell
/// were Vacuum. It goes ahead only if:
/// - the displaced gas can leave entirely, for a face neighbour holding the same gas or Vacuum;
/// - the pushing Cell still holds more after the move than the exit the displaced gas squeezes
///   into. Without this, two gases whose pressures can't balance in whole Cells would trade a
///   Cell back and forth forever, and gas would never come to rest (ADR 0004).
///
/// When several Elements push into one gas Cell, the one pushing the most grams wins it.
///
/// Gas Cells are considered in a pseudo-random order seeded by the Tick count and position. Each
/// Cell takes the first role offered: a vacated Cell can't push or receive, a pushing Cell can't
/// be vacated, and an exit can't be vacated or turn Vacuum into two different Elements.
pub(crate) fn displace(chunks: &mut HashMap<ChunkPos, Chunk>, tick: u64, pushes: Vec<Transfer>) {
    let mut pushes_into: HashMap<CellPos, Vec<Transfer>> = HashMap::new();
    for push in pushes {
        pushes_into.entry(push.to).or_default().push(push);
    }
    let mut candidates: Vec<CellPos> = pushes_into
        .keys()
        .copied()
        .filter(|&pos| !exits(chunks, pos).is_empty())
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
        let pushes: Vec<Transfer> = pushes_into[&vacated]
            .iter()
            .filter(|push| {
                let role = roles.get(&push.from);
                let pushed = role.map_or(0, |role| role.pushed_grams);
                let keeps = u64::from(cell_at(chunks, push.from).mass_grams)
                    .saturating_sub(u64::from(pushed) + u64::from(push.grams));
                !role.is_some_and(|role| role.vacated) && keeps > squeezed
            })
            .cloned()
            .collect();
        let Some(pusher) = strongest_element(&pushes, tick, vacated) else {
            continue;
        };

        roles.entry(vacated).or_default().vacated = true;
        let exit_role = roles.entry(exit).or_default();
        let already = exit_role.squeezed_in.map_or(0, |(_, grams)| grams);
        exit_role.squeezed_in = Some((leaving.element, already + u64::from(leaving.mass_grams)));
        transfers.push(Transfer::new(vacated, leaving, exit, leaving.mass_grams));
        for push in pushes.into_iter().filter(|push| push.element == pusher) {
            roles.entry(push.from).or_default().pushed_grams += push.grams;
            transfers.push(push);
        }
    }
    apply(chunks, transfers);
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
