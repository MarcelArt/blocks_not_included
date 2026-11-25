//! Checkpoints: the simulation captured between two Ticks, so that it can be restored and carry
//! on exactly as it would have (`docs/plans/16-simulation-checkpoint.md`, ADR 0005).
//!
//! A checkpoint holds no rules and no terrain the [`ChunkSource`](crate::ChunkSource) can
//! regenerate. Restoring one takes the Element table, the settings and the source, and checks the
//! checkpoint against them before anything is built. How a checkpoint is stored is up to the
//! game.

use std::collections::BTreeMap;
use std::fmt;

use crate::cell::{Cell, CellPos};
use crate::chunk::{CHUNK_VOLUME, ChunkPos};
use crate::element::{ElementError, ElementId};
use crate::region::{ActiveRegion, is_in_world};
use crate::settings::SettingsError;

/// The simulation between two Ticks, as [`Simulation::checkpoint`](crate::Simulation::checkpoint)
/// captures it: everything [`Simulation::restore`](crate::Simulation::restore) needs to carry on
/// exactly as the captured simulation would, given the same Element table, settings and
/// [`ChunkSource`](crate::ChunkSource).
///
/// It holds the Cells of every Chunk that differs from what the source generates, whether it is
/// active or Dormant. Of a Chunk the source can regenerate, it holds only what the source can't
/// tell: whether it is stored in the Active Region, and whether it is Settled.
#[derive(Clone, Debug, PartialEq)]
pub struct Checkpoint {
    /// The name of every Element of the table the simulation ran with, in [`ElementId`] order
    /// from Vacuum, so the Elements the Cells hold can be checked against another table by name.
    pub element_names: Vec<String>,
    /// The focus (`CONTEXT.md`).
    pub focus: CellPos,
    /// The Chunk with the smallest x, y and z of the Active Region's box, before the world's
    /// vertical limits cut it off.
    ///
    /// The box follows the focus with hysteresis (`region.rs`), so it can't be worked out from
    /// the focus alone: a focus that wandered back and forth may have left it anywhere within the
    /// hysteresis margin.
    pub placement_min: ChunkPos,
    /// How many Chunks the Active Region's box spans along x, y and z: the settings'
    /// [`active_region_chunks`](crate::SimSettings::active_region_chunks).
    pub placement_chunks: [u32; 3],
    /// How many Ticks have run.
    pub tick_count: u64,
    /// The Mass that gas has lost through the world's ceiling.
    pub mass_lost_to_space_grams: u64,
    /// Every Chunk stored in the Active Region, and every Dormant Chunk whose Cells differ from
    /// what the source generates. A Chunk of the Active Region that isn't listed holds nothing
    /// but Vacuum, as the source generates it. A Dormant Chunk that isn't listed loads from the
    /// source when it is needed.
    pub chunks: BTreeMap<ChunkPos, ChunkCheckpoint>,
}

/// One Chunk of a [`Checkpoint`].
#[derive(Clone, Debug, PartialEq)]
pub struct ChunkCheckpoint {
    pub contents: ChunkContents,
    /// Whether it is a Settled Chunk (`CONTEXT.md`). Only a Chunk of the Active Region can be.
    pub settled: bool,
}

/// What a Chunk of a [`Checkpoint`] holds.
#[derive(Clone, Debug, PartialEq)]
pub enum ChunkContents {
    /// Exactly the Cells the [`ChunkSource`](crate::ChunkSource) generates for it.
    Generated,
    /// Nothing but Vacuum, whatever the source generates: say, a Chunk dug out entirely.
    Vacuum,
    /// These Cells: `CHUNK_SIZE³` of them, in the order
    /// [`Simulation::chunk_cells`](crate::Simulation::chunk_cells) lists them.
    Cells(Vec<Cell>),
}

/// Why [`Simulation::restore`](crate::Simulation::restore) rejected a [`Checkpoint`]. Nothing is
/// restored.
#[derive(Clone, Debug, PartialEq)]
pub enum CheckpointError {
    /// The settings fail [`SimSettings::validate`](crate::SimSettings::validate).
    Settings(SettingsError),
    /// The Element table lacks an Element the checkpoint names, or holds it at another position:
    /// [`ElementError::MissingElement`] or [`ElementError::ElementMoved`].
    Elements(ElementError),
    /// The Active Region's box spans a different number of Chunks than the settings'
    /// `active_region_chunks`.
    RegionSize {
        checkpoint: [u32; 3],
        settings: [u32; 3],
    },
    /// The Active Region's box lies where the focus could never have left it: following the
    /// focus would move it at once.
    RegionAwayFromFocus {
        focus: CellPos,
        placement_min: ChunkPos,
    },
    /// A Chunk lies outside the world's vertical limits, where none is ever loaded.
    ChunkOutsideWorld { chunk: ChunkPos },
    /// A Chunk's Cells don't number `CHUNK_SIZE³`.
    CellCount { chunk: ChunkPos, cells: usize },
    /// A Cell holds an Element the checkpoint doesn't name, given by its [`ElementId::index`].
    UnknownElement { pos: CellPos, element: u16 },
    /// A Vacuum Cell holds Mass, or a Cell of any other Element holds none.
    InvalidMass { pos: CellPos, cell: Cell },
    /// A Chunk outside the Active Region is marked Settled. Only an active Chunk can be.
    SettledOutsideActiveRegion { chunk: ChunkPos },
    /// The checkpoint leaves out a Chunk of the Active Region, so the Chunk held nothing but
    /// Vacuum, as the source generated it. But the source generates matter there: it isn't the
    /// source the checkpoint was captured with.
    MissingChunk { chunk: ChunkPos },
}

impl fmt::Display for CheckpointError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Settings(error) => write!(f, "the settings are invalid: {error}"),
            Self::Elements(error) => write!(f, "the Elements don't match: {error}"),
            Self::RegionSize {
                checkpoint,
                settings,
            } => write!(
                f,
                "the Active Region spans {checkpoint:?} Chunks, but the settings say {settings:?}"
            ),
            Self::RegionAwayFromFocus {
                focus,
                placement_min,
            } => write!(
                f,
                "the Active Region's box starting at Chunk ({}, {}, {}) can't have followed the \
                 focus to ({}, {}, {})",
                placement_min.x, placement_min.y, placement_min.z, focus.x, focus.y, focus.z
            ),
            Self::ChunkOutsideWorld { chunk } => write!(
                f,
                "Chunk ({}, {}, {}) lies outside the world's vertical limits",
                chunk.x, chunk.y, chunk.z
            ),
            Self::CellCount { chunk, cells } => write!(
                f,
                "Chunk ({}, {}, {}) holds {cells} Cells, not {CHUNK_VOLUME}",
                chunk.x, chunk.y, chunk.z
            ),
            Self::UnknownElement { pos, element } => write!(
                f,
                "the Cell at ({}, {}, {}) holds Element #{element}, which the checkpoint doesn't \
                 name",
                pos.x, pos.y, pos.z
            ),
            Self::InvalidMass { pos, cell } => {
                let problem = if cell.element == ElementId::VACUUM {
                    "Vacuum with Mass"
                } else {
                    "no Mass"
                };
                write!(
                    f,
                    "the Cell at ({}, {}, {}) holds {problem}: {cell:?}",
                    pos.x, pos.y, pos.z
                )
            }
            Self::SettledOutsideActiveRegion { chunk } => write!(
                f,
                "Chunk ({}, {}, {}) is Settled, but it lies outside the Active Region",
                chunk.x, chunk.y, chunk.z
            ),
            Self::MissingChunk { chunk } => write!(
                f,
                "Chunk ({}, {}, {}) of the Active Region should hold nothing but Vacuum, but the \
                 world generator puts matter there",
                chunk.x, chunk.y, chunk.z
            ),
        }
    }
}

impl std::error::Error for CheckpointError {}

impl Checkpoint {
    /// The names in [`element_names`](Self::element_names) with the position each must have in a
    /// table.
    pub(crate) fn element_positions(&self) -> impl Iterator<Item = (usize, &str)> {
        self.element_names.iter().map(String::as_str).enumerate()
    }

    /// Checks every listed Chunk against the Active Region its box makes: that it lies within
    /// the world, is Settled only if it is active, and holds valid Cells of Elements the
    /// checkpoint names.
    pub(crate) fn validate_chunks(&self, region: ActiveRegion) -> Result<(), CheckpointError> {
        for (&chunk, listed) in &self.chunks {
            if !is_in_world(chunk) {
                return Err(CheckpointError::ChunkOutsideWorld { chunk });
            }
            if listed.settled && !region.contains(chunk) {
                return Err(CheckpointError::SettledOutsideActiveRegion { chunk });
            }
            if let ChunkContents::Cells(cells) = &listed.contents {
                self.validate_cells(chunk, cells)?;
            }
        }
        Ok(())
    }

    /// Checks that `cells`, the Cells of the Chunk at `chunk`, number `CHUNK_SIZE³`, hold only
    /// Elements the checkpoint names, and keep the Mass invariants.
    fn validate_cells(&self, chunk: ChunkPos, cells: &[Cell]) -> Result<(), CheckpointError> {
        if cells.len() != CHUNK_VOLUME {
            return Err(CheckpointError::CellCount {
                chunk,
                cells: cells.len(),
            });
        }
        for (index, &cell) in cells.iter().enumerate() {
            let pos = || chunk.cell_pos(index);
            if usize::from(cell.element.index()) >= self.element_names.len() {
                return Err(CheckpointError::UnknownElement {
                    pos: pos(),
                    element: cell.element.index(),
                });
            }
            if (cell.element == ElementId::VACUUM) != (cell.mass_grams == 0) {
                return Err(CheckpointError::InvalidMass { pos: pos(), cell });
            }
        }
        Ok(())
    }
}
