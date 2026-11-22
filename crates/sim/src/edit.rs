//! Edits: the player's god tools for changing Cells.

use crate::cell::{Cell, CellPos};
use crate::element::ElementId;

/// One change the player makes to one Cell, applied between Ticks with
/// [`Simulation::apply`](crate::Simulation::apply).
///
/// Edits are god tools, so they create and destroy matter: they are the named exception to the
/// rule that total Mass never changes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Edit {
    /// Overwrites whatever was at `pos` with `cell`. A `cell` of Vacuum, or of no Mass, places a
    /// Vacuum Cell, and a temperature below 0 K becomes 0 K, so the Cell is always a valid one.
    Place { pos: CellPos, cell: Cell },
    /// Turns the Cell at `pos` into Vacuum.
    Dig { pos: CellPos },
    /// Warms the Cell at `pos` by `kelvin`, or cools it if `kelvin` is negative, but never below
    /// 0 K. Vacuum has no temperature to shift, so a Vacuum Cell stays as it is.
    ShiftTemperature { pos: CellPos, kelvin: f32 },
}

impl Edit {
    /// The Cell this Edit changes.
    pub fn pos(self) -> CellPos {
        match self {
            Edit::Place { pos, .. } | Edit::Dig { pos } | Edit::ShiftTemperature { pos, .. } => pos,
        }
    }

    /// What the Cell at [`pos`](Self::pos), currently `old`, becomes.
    pub(crate) fn result(self, old: Cell) -> Cell {
        match self {
            Edit::Place { cell, .. }
                if cell.element == ElementId::VACUUM || cell.mass_grams == 0 =>
            {
                Cell::VACUUM
            }
            Edit::Place { cell, .. } => Cell {
                temperature_k: cell.temperature_k.max(0.0),
                ..cell
            },
            Edit::Dig { .. } => Cell::VACUUM,
            Edit::ShiftTemperature { .. } if old.element == ElementId::VACUUM => old,
            Edit::ShiftTemperature { kelvin, .. } => Cell {
                temperature_k: (old.temperature_k + kelvin).max(0.0),
                ..old
            },
        }
    }
}
