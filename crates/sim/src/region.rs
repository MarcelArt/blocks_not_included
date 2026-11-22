//! The Active Region (`CONTEXT.md`, ADR 0001) and the world's vertical limits.
//!
//! # Where the Active Region sits
//!
//! The Active Region is a box of Chunks, as many along each axis as
//! [`SimSettings::active_region_chunks`](crate::SimSettings::active_region_chunks) says. Along each
//! axis it sits where its centre is nearest the focus, so the focus lies within half a Chunk of
//! the centre. With an even number of Chunks the centre is a Chunk border: at the default size of
//! 4 × 2 × 4 and the default focus at the origin, the box holds Chunks −2 to 1 along x and z, and
//! −1 to 0 along y.
//!
//! # Hysteresis
//!
//! Along each axis, the box moves only once the focus is more than [`HYSTERESIS_CELLS`] past the
//! point where the next box over would be better centred, and then it moves to the box best
//! centred on the focus. Without that margin, a player walking back and forth across that point
//! would shift the box every time. With an even number of Chunks the point runs through the middle
//! of a Chunk, so crossing a Chunk border never shifts the box at all; with an odd number it is a
//! Chunk border.
//!
//! # World limits
//!
//! Chunks below [`WORLD_BOTTOM_Y`], or at or above [`WORLD_TOP_Y`], are never loaded. Near a
//! limit the box is cut off there, so the Active Region holds fewer Chunks, and phenomena see the
//! world's floor and ceiling as the Active Region's sealed edge. Only the ceiling lets anything
//! through: gas in the top layer escapes to space (`gas.rs`).

use crate::cell::CellPos;
use crate::chunk::{CHUNK_SIZE, ChunkPos};

/// The y of the world's bottom layer of Cells. The floor beneath it is sealed.
pub const WORLD_BOTTOM_Y: i32 = -1024;

/// The y just above the world's top layer of Cells. Gas in the top layer escapes to space.
pub const WORLD_TOP_Y: i32 = 256;

const SIZE: i32 = CHUNK_SIZE as i32;

/// The world's Chunk layers, from the bottom one to just above the top one.
const BOTTOM_CHUNK_Y: i32 = WORLD_BOTTOM_Y / SIZE;
const TOP_CHUNK_Y: i32 = WORLD_TOP_Y / SIZE;

const _: () = assert!(WORLD_BOTTOM_Y % SIZE == 0 && WORLD_TOP_Y % SIZE == 0);

/// How many Cells past the point where the next box over would be better centred the focus must
/// go before the Active Region moves.
pub(crate) const HYSTERESIS_CELLS: i32 = 4;

/// Whether the Chunk at `chunk` lies within the world's vertical limits, so it can be loaded.
pub(crate) fn is_in_world(chunk: ChunkPos) -> bool {
    (BOTTOM_CHUNK_Y..TOP_CHUNK_Y).contains(&chunk.y)
}

/// The Chunk layer that holds the world's top layer of Cells.
pub(crate) const fn top_chunk_y() -> i32 {
    TOP_CHUNK_Y - 1
}

/// The Active Region (`CONTEXT.md`): the box of Chunks that is simulated, from [`min`](Self::min)
/// to [`max`](Self::max) inclusive.
///
/// It never reaches past the world's vertical limits. If the focus is so far above or below the
/// world that the box misses it entirely, the Active Region is empty.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActiveRegion {
    min: ChunkPos,
    max: ChunkPos,
}

impl ActiveRegion {
    /// A region holding no Chunks.
    pub(crate) const EMPTY: ActiveRegion = ActiveRegion {
        min: ChunkPos::new(0, 0, 0),
        max: ChunkPos::new(-1, -1, -1),
    };

    /// The Chunk with the smallest x, y and z.
    pub fn min(self) -> ChunkPos {
        self.min
    }

    /// The Chunk with the largest x, y and z.
    pub fn max(self) -> ChunkPos {
        self.max
    }

    pub fn is_empty(self) -> bool {
        self.min.x > self.max.x || self.min.y > self.max.y || self.min.z > self.max.z
    }

    pub fn contains(self, chunk: ChunkPos) -> bool {
        (self.min.x..=self.max.x).contains(&chunk.x)
            && (self.min.y..=self.max.y).contains(&chunk.y)
            && (self.min.z..=self.max.z).contains(&chunk.z)
    }

    /// Every Chunk in the region, sorted by x, then y, then z.
    pub fn chunks(self) -> impl Iterator<Item = ChunkPos> {
        let (min, max) = (self.min, self.max);
        (min.x..=max.x).flat_map(move |x| {
            (min.y..=max.y).flat_map(move |y| (min.z..=max.z).map(move |z| ChunkPos::new(x, y, z)))
        })
    }
}

/// Where the Active Region's box sits before the world's limits cut it off. It remembers the
/// box, so the box can follow the focus with hysteresis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Placement {
    min: ChunkPos,
    /// Chunks along x, y and z.
    size: [i32; 3],
}

impl Placement {
    /// The box of `size` Chunks best centred on `focus`.
    pub fn around(focus: CellPos, size: [u32; 3]) -> Self {
        let size = size.map(|chunks| {
            i32::try_from(chunks).expect("SimSettings::validate keeps the size in range")
        });
        let [x, y, z] = size;
        Self {
            min: ChunkPos::new(
                best_centred(focus.x, x),
                best_centred(focus.y, y),
                best_centred(focus.z, z),
            ),
            size,
        }
    }

    /// The box that follows the focus to `focus`: this one, along each axis where the focus is no
    /// more than [`HYSTERESIS_CELLS`] past the point where the next box over would be better
    /// centred, and the box best centred on `focus` along the others.
    pub fn follow(self, focus: CellPos) -> Self {
        let follow = |min: i32, focus: i32, chunks: i32| {
            let (lowest, highest) = best_centred_focus(min, chunks);
            let kept = (lowest - HYSTERESIS_CELLS)..=(highest + HYSTERESIS_CELLS);
            if kept.contains(&focus) {
                min
            } else {
                best_centred(focus, chunks)
            }
        };
        let [x, y, z] = self.size;
        Self {
            min: ChunkPos::new(
                follow(self.min.x, focus.x, x),
                follow(self.min.y, focus.y, y),
                follow(self.min.z, focus.z, z),
            ),
            size: self.size,
        }
    }

    /// The Active Region: this box, cut off at the world's vertical limits.
    pub fn region(self) -> ActiveRegion {
        let [x, y, z] = self.size;
        let min = self.min;
        let max = min.offset((x - 1, y - 1, z - 1));
        ActiveRegion {
            min: ChunkPos::new(min.x, min.y.max(BOTTOM_CHUNK_Y), min.z),
            max: ChunkPos::new(max.x, max.y.min(TOP_CHUNK_Y - 1), max.z),
        }
    }
}

/// Along one axis, the first of `chunks` Chunks in the box whose centre is nearest `focus`.
fn best_centred(focus: i32, chunks: i32) -> i32 {
    // The inverse of `best_centred_focus`: the box starting at Chunk `min` is best centred for
    // focus positions from `min × SIZE + (chunks − 1) × SIZE / 2` onward, for one Chunk's width.
    (focus - (chunks - 1) * SIZE / 2).div_euclid(SIZE)
}

/// Along one axis, the lowest and highest focus positions for which the box of `chunks` Chunks
/// starting at Chunk `min` is the best centred: those within half a Chunk of its centre.
fn best_centred_focus(min: i32, chunks: i32) -> (i32, i32) {
    let lowest = min * SIZE + (chunks - 1) * SIZE / 2;
    (lowest, lowest + SIZE - 1)
}
