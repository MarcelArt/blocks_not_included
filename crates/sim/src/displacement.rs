//! Displacement (`CONTEXT.md`): gas or liquid moving part of its Mass into a neighbouring Cell of
//! a different gas, once that gas has left entirely for a face neighbour holding the same gas, or
//! for Vacuum.
//!
//! Gas flow (rule 5 of `docs/plans/02-gas-flow.md`) and liquid flow (rules 1 and 4 of
//! `docs/plans/03-liquid-flow.md`) each decide which of their Cells push into which gas, and how
//! much, then hand those pushes to [`displace`]. Only a Cell holding more Mass than the gas Cell
//! it pushes into may push. `displace` decides which pushes can go ahead and carries them out.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

use rayon::prelude::*;

use crate::cell::{Cell, CellPos};
use crate::chunk::ActiveChunks;
use crate::element::ElementId;
use crate::grid::{Cells, Grid};
use crate::transfer::{FACES, TieBreak, Transfer, apply, key, pick, strongest_element, tie_break};

/// A map keyed by Cell position, with a quick hash. The standard hash guards against keys chosen
/// to collide, which positions in the simulation never are. Nothing iterates these maps in their
/// own order, so the hash never affects the result.
type PosMap<V> = HashMap<CellPos, V, BuildHasherDefault<PosHasher>>;

/// The hasher of a [`PosMap`]: multiply and rotate, after FxHash.
#[derive(Default)]
struct PosHasher(u64);

impl PosHasher {
    fn add(&mut self, word: u64) {
        self.0 = (self.0.rotate_left(5) ^ word).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
}

impl Hasher for PosHasher {
    fn write(&mut self, bytes: &[u8]) {
        bytes.iter().for_each(|&byte| self.add(u64::from(byte)));
    }

    fn write_i32(&mut self, n: i32) {
        self.add(u64::from(n as u32));
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

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
pub(crate) fn displace(chunks: &mut ActiveChunks, tick: u64, pushes: Vec<Transfer>) {
    if pushes.is_empty() {
        return;
    }
    let transfers = plan(&Grid::new(chunks), tick, pushes);
    apply(chunks, transfers);
}

/// A gas Cell that something pushes into, with what deciding its fate needs from the state at
/// the start of the round.
struct Candidate<'p> {
    vacated: CellPos,
    leaving: Cell,
    /// Each face neighbour holding the same gas or Vacuum, where the displaced gas could go, and
    /// the grams it holds.
    exits: PerFace<(CellPos, u64)>,
    /// Every push into this Cell, and the grams its source holds. Each comes from a different
    /// face neighbour, so there are at most six.
    pushes: &'p [Transfer],
    source_grams: PerFace<u32>,
    /// Where it comes in the pseudo-random order.
    order: u64,
}

/// The transfers [`displace`] carries out.
fn plan(grid: &Grid, tick: u64, mut pushes: Vec<Transfer>) -> Vec<Transfer> {
    pushes.par_sort_unstable_by_key(|push| (key(push.to), key(push.from)));
    let pushes_into: Vec<&[Transfer]> = pushes.chunk_by(|a, b| a.to == b.to).collect();
    let mut candidates: Vec<Candidate> = pushes_into
        .into_par_iter()
        .filter_map(|pushes| {
            let vacated = pushes[0].to;
            let leaving = grid.cell(vacated);
            let mut exits = PerFace::new((vacated, 0));
            for face in FACES {
                let exit = vacated.offset(face);
                let held = grid.cell(exit);
                if held.element == leaving.element || held.element == ElementId::VACUUM {
                    exits.push((exit, u64::from(held.mass_grams)));
                }
            }
            if exits.is_empty() {
                return None;
            }
            let mut source_grams = PerFace::new(0);
            for push in pushes {
                source_grams.push(grid.cell(push.from).mass_grams);
            }
            Some(Candidate {
                vacated,
                leaving,
                exits,
                pushes,
                source_grams,
                order: tie_break(tick, vacated, TieBreak::Order),
            })
        })
        .collect();
    candidates.par_sort_unstable_by_key(|candidate| (candidate.order, key(candidate.vacated)));

    let mut roles: PosMap<Role> =
        PosMap::with_capacity_and_hasher(candidates.len() * 4, Default::default());
    let mut transfers = Vec::new();
    for candidate in &candidates {
        let (vacated, leaving) = (candidate.vacated, candidate.leaving);
        if roles.contains_key(&vacated) {
            continue;
        }
        // Each open exit, with the grams it will hold once this gas has squeezed in.
        let mut exits = PerFace::new((vacated, 0));
        for &(exit, held) in candidate.exits.as_slice() {
            let role = roles.get(&exit);
            if role.is_some_and(|role| role.vacated) {
                continue;
            }
            let already = match role.and_then(|role| role.squeezed_in) {
                Some((element, _)) if element != leaving.element => continue,
                Some((_, grams)) => grams,
                None => 0,
            };
            exits.push((exit, held + already + u64::from(leaving.mass_grams)));
        }
        let Some((exit, squeezed)) = emptiest(exits.as_slice(), tick, vacated) else {
            continue;
        };
        let mut pushes = PerFace::new(candidate.pushes[0]);
        for (push, &source_grams) in candidate
            .pushes
            .iter()
            .zip(candidate.source_grams.as_slice())
        {
            let role = roles.get(&push.from);
            let pushed = role.map_or(0, |role| role.pushed_grams);
            let keeps =
                u64::from(source_grams).saturating_sub(u64::from(pushed) + u64::from(push.grams));
            if !role.is_some_and(|role| role.vacated) && keeps > squeezed {
                pushes.push(*push);
            }
        }
        let Some(pusher) = strongest_element(pushes.as_slice(), tick, vacated) else {
            continue;
        };

        roles.entry(vacated).or_default().vacated = true;
        let exit_role = roles.entry(exit).or_default();
        let already = exit_role.squeezed_in.map_or(0, |(_, grams)| grams);
        exit_role.squeezed_in = Some((leaving.element, already + u64::from(leaving.mass_grams)));
        transfers.push(Transfer::new(vacated, leaving, exit, leaving.mass_grams));
        for push in pushes
            .as_slice()
            .iter()
            .filter(|push| push.element == pusher)
        {
            roles.entry(push.from).or_default().pushed_grams += push.grams;
            transfers.push(*push);
        }
    }
    transfers
}

/// The exit that will hold the fewest grams. Equal exits are settled by a tie-break seeded by
/// the Tick count and `pos`.
fn emptiest(exits: &[(CellPos, u64)], tick: u64, pos: CellPos) -> Option<(CellPos, u64)> {
    let least = exits.iter().map(|&(_, grams)| grams).min()?;
    let tied = exits.iter().copied().filter(|&(_, grams)| grams == least);
    let count = tied.clone().count();
    tied.clone().nth(pick(count, tick, pos, TieBreak::Exit))
}

/// Up to one thing per face of a Cell, kept without allocating.
#[derive(Clone, Copy)]
struct PerFace<T: Copy> {
    items: [T; FACES.len()],
    len: usize,
}

impl<T: Copy> PerFace<T> {
    /// An empty list; `filler` only fills the unused places.
    fn new(filler: T) -> Self {
        Self {
            items: [filler; FACES.len()],
            len: 0,
        }
    }

    fn push(&mut self, item: T) {
        self.items[self.len] = item;
        self.len += 1;
    }

    fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn as_slice(&self) -> &[T] {
        &self.items[..self.len]
    }
}
