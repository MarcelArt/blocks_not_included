//! Splitting a pass's work across threads by Chunk, and finding Cells without hashing.
//!
//! Every phenomenon works in passes: it plans its moves from the state at the start of the pass,
//! then carries them out together. Planning only reads, so it runs on every stored Chunk at once,
//! one task per Chunk ([`Grid::plan`]). Carrying the moves out changes each Cell from within its
//! own Chunk, so that runs one task per Chunk as well ([`chunks_mut`]).
//!
//! Tasks may finish in any order, but their results are always gathered in the order of the
//! Chunks' positions, and no task reads anything another task writes. So a Tick gives exactly the
//! same result whatever the number of threads.

use rayon::prelude::*;

use crate::cell::{Cell, CellPos};
use crate::chunk::{ActiveChunks, CHUNK_SIZE, CHUNK_VOLUME, Chunk, ChunkPos, index, locate};
use crate::region::ActiveRegion;
use crate::transfer::FACES;

/// The stored Chunks of the Active Region, in the order of their positions, with a quick way to
/// find each one's place in that order.
pub(crate) struct Layout {
    /// The position of every stored Chunk, sorted.
    keys: Vec<ChunkPos>,
    region: ActiveRegion,
    /// The smallest box of Chunks holding every stored Chunk: its first Chunk and its size.
    min: ChunkPos,
    dims: [usize; 3],
    /// For each Chunk of that box, x slowest and z fastest, its place in `keys`, or [`NO_SLOT`].
    slots: Vec<u32>,
}

const NO_SLOT: u32 = u32::MAX;

impl Layout {
    pub fn new(chunks: &ActiveChunks) -> Self {
        let mut keys: Vec<ChunkPos> = chunks.keys().copied().collect();
        keys.sort_unstable();
        let min = ChunkPos::new(
            keys.iter().map(|pos| pos.x).min().unwrap_or(0),
            keys.iter().map(|pos| pos.y).min().unwrap_or(0),
            keys.iter().map(|pos| pos.z).min().unwrap_or(0),
        );
        let span = |axis: fn(&ChunkPos) -> i32, min: i32| {
            keys.iter()
                .map(|pos| (axis(pos) - min) as usize + 1)
                .max()
                .unwrap_or(0)
        };
        let dims = [
            span(|pos| pos.x, min.x),
            span(|pos| pos.y, min.y),
            span(|pos| pos.z, min.z),
        ];
        let mut layout = Layout {
            keys,
            region: chunks.region,
            min,
            dims,
            slots: vec![NO_SLOT; dims[0] * dims[1] * dims[2]],
        };
        for slot in 0..layout.keys.len() {
            let place = layout
                .place(layout.keys[slot])
                .expect("every stored Chunk lies in the box");
            layout.slots[place] = slot as u32;
        }
        layout
    }

    /// How many Chunks are stored.
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// The position of the stored Chunk at `slot`.
    pub fn key(&self, slot: usize) -> ChunkPos {
        self.keys[slot]
    }

    /// The place in the order of the stored Chunk at `pos`, if it is stored.
    pub fn slot(&self, pos: ChunkPos) -> Option<usize> {
        let slot = self.slots[self.place(pos)?];
        (slot != NO_SLOT).then_some(slot as usize)
    }

    /// Where `pos` lies in `slots`, if it lies in the box.
    fn place(&self, pos: ChunkPos) -> Option<usize> {
        // A position before the box wraps around to a huge offset, so one comparison per axis
        // tells whether it lies in the box.
        let offset = |at: i32, min: i32| at.wrapping_sub(min) as u32 as usize;
        let (x, y, z) = (
            offset(pos.x, self.min.x),
            offset(pos.y, self.min.y),
            offset(pos.z, self.min.z),
        );
        let [size_x, size_y, size_z] = self.dims;
        (x < size_x && y < size_y && z < size_z).then(|| (x * size_y + y) * size_z + z)
    }
}

/// Something Cells can be read from by their world position.
pub(crate) trait Cells {
    /// The Cell at `pos`: Vacuum in a Chunk of the Active Region that isn't stored, and
    /// [`Cell::SEALED_EDGE`] outside the Active Region.
    fn cell(&self, pos: CellPos) -> Cell;
}

/// A read-only view of the stored Chunks for one pass.
pub(crate) struct Grid<'a> {
    layout: Layout,
    /// The stored Chunks, in the order of `layout`.
    chunks: Vec<&'a Chunk>,
    /// The Cells of each Chunk of the layout's box, or `None` where no Chunk is stored, so a
    /// lookup takes a single step.
    cells_by_place: Vec<Option<&'a [Cell]>>,
}

impl<'a> Grid<'a> {
    pub fn new(chunks: &'a ActiveChunks) -> Self {
        let layout = Layout::new(chunks);
        let refs: Vec<&Chunk> = layout.keys.iter().map(|pos| &chunks[pos]).collect();
        let cells_by_place = layout
            .slots
            .iter()
            .map(|&slot| (slot != NO_SLOT).then(|| &*refs[slot as usize].cells))
            .collect();
        Self {
            layout,
            chunks: refs,
            cells_by_place,
        }
    }

    pub fn len(&self) -> usize {
        self.chunks.len()
    }

    /// The stored Chunk at `slot`, and its position.
    pub fn at(&self, slot: usize) -> (ChunkPos, &'a Chunk) {
        (self.layout.key(slot), self.chunks[slot])
    }

    /// The stored Chunk at `pos`, if it is stored.
    pub fn chunk(&self, pos: ChunkPos) -> Option<&'a Chunk> {
        self.layout.slot(pos).map(|slot| self.chunks[slot])
    }

    /// Which Cells of each stored Chunk phenomena visit in this pass, in the order of the layout.
    pub fn visits(&self) -> Vec<Visit> {
        (0..self.len())
            .map(|slot| {
                let (pos, chunk) = self.at(slot);
                if chunk.is_awake() {
                    return Visit::All;
                }
                let facing_awake = FACES
                    .into_iter()
                    .enumerate()
                    .filter(|&(_, face)| self.chunk(pos.offset(face)).is_some_and(Chunk::is_awake));
                Visit::Faces(facing_awake.fold(0, |mask, (i, _)| mask | 1 << i))
            })
            .collect()
    }

    /// Runs `plan` on every Cell phenomena visit in this pass for which `keep` holds, one task
    /// per Chunk, and gathers everything planned, Chunk by Chunk in the order of their positions.
    ///
    /// Phenomena visit every Cell of an awake Chunk, and the Cells of a Settled Chunk that share a
    /// face with an awake Chunk (see [`Visit`]). Every flow rule finds the Cells it moves Mass
    /// from, or into, this way, so a Settled Chunk takes part only at its borders with awake
    /// Chunks.
    ///
    /// `plan` reads Cells through a [`Local`] view, which finds Cells of the Chunk being planned
    /// for without any lookup.
    pub fn plan<T: Send>(
        &self,
        keep: impl Fn(Cell) -> bool + Sync,
        plan: impl Fn(&Local, CellPos, Cell, &mut Vec<T>) + Sync,
    ) -> Vec<T> {
        self.plan_by_chunk(keep, plan)
            .into_iter()
            .flatten()
            .collect()
    }

    /// Like [`plan`](Self::plan), but leaves what each Chunk planned in a list of its own,
    /// which saves gathering them into one list when there are many.
    pub fn plan_by_chunk<T: Send>(
        &self,
        keep: impl Fn(Cell) -> bool + Sync,
        plan: impl Fn(&Local, CellPos, Cell, &mut Vec<T>) + Sync,
    ) -> Vec<Vec<T>> {
        let visits = self.visits();
        (0..self.len())
            .into_par_iter()
            .with_max_len(1)
            .map(|slot| {
                let (pos, chunk) = self.at(slot);
                let local = Local {
                    grid: self,
                    home: pos,
                    cells: &chunk.cells,
                };
                let mut planned = Vec::new();
                visits[slot].for_each(|index| {
                    let cell = chunk.cells[index];
                    if keep(cell) {
                        plan(&local, pos.cell_pos(index), cell, &mut planned);
                    }
                });
                planned
            })
            .collect()
    }

    pub fn into_layout(self) -> Layout {
        self.layout
    }
}

impl Cells for Grid<'_> {
    fn cell(&self, pos: CellPos) -> Cell {
        let (chunk, index) = locate(pos);
        match self
            .layout
            .place(chunk)
            .and_then(|place| self.cells_by_place[place])
        {
            Some(cells) => cells[index],
            None if self.layout.region.contains(chunk) => Cell::VACUUM,
            None => Cell::SEALED_EDGE,
        }
    }
}

/// The view of the stored Chunks from one Chunk's planning task: its own Cells are read
/// directly, and any other through the [`Grid`].
pub(crate) struct Local<'g, 'a> {
    grid: &'g Grid<'a>,
    home: ChunkPos,
    cells: &'a [Cell],
}

impl Cells for Local<'_, '_> {
    #[inline]
    fn cell(&self, pos: CellPos) -> Cell {
        let (chunk, index) = locate(pos);
        if chunk == self.home {
            self.cells[index]
        } else {
            self.grid.cell(pos)
        }
    }
}

/// Which Cells of a stored Chunk phenomena visit in a pass.
#[derive(Clone, Copy)]
pub(crate) enum Visit {
    /// Every Cell: the Chunk is awake.
    All,
    /// The Cells of a Settled Chunk that share a face with an awake Chunk: those on each face
    /// whose bit is set, where bit `i` stands for `FACES[i]`. Borders between awake and Settled
    /// Chunks are still processed.
    Faces(u8),
}

impl Visit {
    /// Calls `visit` with the index of every Cell visited, once each.
    pub fn for_each(self, mut visit: impl FnMut(usize)) {
        let mask = match self {
            Visit::All => return (0..CHUNK_VOLUME).for_each(visit),
            Visit::Faces(mask) => mask,
        };
        for (i, face) in FACES.into_iter().enumerate() {
            if mask & 1 << i == 0 {
                continue;
            }
            // A Cell on an edge or corner lies on more than one face; only the first visits it.
            let earlier = mask & ((1 << i) - 1);
            for index in face_layer(face) {
                let on_earlier = FACES
                    .into_iter()
                    .enumerate()
                    .any(|(j, face)| earlier & 1 << j != 0 && lies_on(index, face));
                if !on_earlier {
                    visit(index);
                }
            }
        }
    }
}

/// The index of every Cell in the layer of a Chunk that touches the neighbour in direction `face`.
pub(crate) fn face_layer(face: (i32, i32, i32)) -> impl Iterator<Item = usize> {
    let last = CHUNK_SIZE - 1;
    (0..CHUNK_SIZE).flat_map(move |u| {
        (0..CHUNK_SIZE).map(move |v| {
            index(match face {
                (1, 0, 0) => (last, u, v),
                (-1, 0, 0) => (0, u, v),
                (0, 1, 0) => (u, last, v),
                (0, -1, 0) => (u, 0, v),
                (0, 0, 1) => (u, v, last),
                _ => (u, v, 0),
            })
        })
    })
}

/// Whether the Cell at `index` lies in the layer of its Chunk that touches the neighbour in
/// direction `face`.
fn lies_on(index: usize, face: (i32, i32, i32)) -> bool {
    let (x, y, z) = crate::chunk::local(index);
    let last = CHUNK_SIZE - 1;
    match face {
        (1, 0, 0) => x == last,
        (-1, 0, 0) => x == 0,
        (0, 1, 0) => y == last,
        (0, -1, 0) => y == 0,
        (0, 0, 1) => z == last,
        _ => z == 0,
    }
}

/// The stored Chunks, mutably, in the order of `layout`, which must have been made from these
/// Chunks.
pub(crate) fn chunks_mut<'a>(chunks: &'a mut ActiveChunks, layout: &Layout) -> Vec<&'a mut Chunk> {
    let mut all: Vec<(&ChunkPos, &mut Chunk)> = chunks.iter_mut().collect();
    all.sort_unstable_by_key(|(pos, _)| **pos);
    debug_assert!(
        all.iter()
            .map(|(pos, _)| **pos)
            .eq(layout.keys.iter().copied())
    );
    all.into_iter().map(|(_, chunk)| chunk).collect()
}

/// Calls `change` on every stored Chunk, one task per Chunk, with what `sorted` holds for it.
///
/// `sorted` holds one bucket per stored Chunk, in the order of `layout`. Empty buckets are
/// skipped.
pub(crate) fn change_chunks<T: Send>(
    chunks: &mut ActiveChunks,
    layout: &Layout,
    sorted: Vec<Vec<T>>,
    change: impl Fn(&mut Chunk, Vec<T>) + Sync,
) {
    chunks_mut(chunks, layout)
        .into_par_iter()
        .zip(sorted)
        .with_max_len(1)
        .filter(|(_, bucket)| !bucket.is_empty())
        .for_each(|(chunk, bucket)| change(chunk, bucket));
}

/// Overwrites every Cell listed in `writes`, each of which must lie in a stored Chunk, and marks
/// each Chunk in which a Cell changed touched. No Cell may be listed twice.
pub(crate) fn write_cells(chunks: &mut ActiveChunks, writes: Vec<(CellPos, Cell)>) {
    if writes.is_empty() {
        return;
    }
    let layout = Layout::new(chunks);
    let mut sorted: Vec<Vec<(usize, Cell)>> = (0..layout.len()).map(|_| Vec::new()).collect();
    for (pos, cell) in writes {
        let (chunk, index) = locate(pos);
        let slot = layout.slot(chunk).expect("only stored Chunks are written");
        sorted[slot].push((index, cell));
    }
    change_chunks(chunks, &layout, sorted, |chunk, writes| {
        for (index, cell) in writes {
            chunk.write(index, cell);
        }
    });
}
