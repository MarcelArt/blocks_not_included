//! Turns a Chunk's solid Cells into a mesh, with `binary-greedy-meshing`.
//!
//! The mesher reads a padded cube of `(CHUNK_SIZE + 2)³` values: the Chunk's own Cells, plus the
//! layer of Cells that touches it from each face neighbour, so that a face against a solid
//! neighbour is left out. For solids, a value of 0 means "not solid"; any other value is the
//! Cell's [`ElementId::index`] + 1. [`mesh_greedy`] lets the Overlays choose values of their own.
//!
//! Solid faces are darkened at each corner by the solid Cells around it in front of the face
//! (ambient occlusion), so creases, corners and the foot of every wall show even where the sun
//! doesn't reach, between faces of one colour. A greedy quad is split only where that shading
//! changes across it.

use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, Mesh, PrimitiveTopology};
use binary_greedy_meshing::{Face, Mesher, Quad, Vertex};
use sim::{CHUNK_SIZE, Cell, ChunkPos, ElementId, ElementTable, Phase};

/// Cells along each side of the padded cube: the Chunk plus one layer on either side.
const PADDED: usize = CHUNK_SIZE + 2;

/// The six face neighbours of a Chunk, as offsets: up, down, +x, −x, +z, −z.
const FACE_NEIGHBOURS: [(i32, i32, i32); 6] = [
    (0, 1, 0),
    (0, -1, 0),
    (1, 0, 0),
    (-1, 0, 0),
    (0, 0, 1),
    (0, 0, -1),
];

/// The 26 Chunks that touch a Chunk by a face, an edge or a corner, as offsets: x, then y, then
/// z, each from −1 to 1, leaving out the Chunk itself.
const NEIGHBOURS: [(i32, i32, i32); 26] = {
    let mut offsets = [(0, 0, 0); 26];
    let (mut i, mut slot) = (0, 0);
    while i < 27 {
        if i != 13 {
            offsets[slot] = (i / 9 - 1, i / 3 % 3 - 1, i % 3 - 1);
            slot += 1;
        }
        i += 1;
    }
    offsets
};

/// Where the neighbour at `offset` lies in [`NEIGHBOURS`].
fn neighbour_slot((dx, dy, dz): (i32, i32, i32)) -> usize {
    let i = (dx + 1) * 9 + (dy + 1) * 3 + (dz + 1);
    // The Chunk itself, at 13, is left out.
    (if i > 13 { i - 1 } else { i }) as usize
}

/// How much of its colour a corner of a solid face shows at each occlusion level: from 0, closed
/// in on both sides by solid Cells in front of the face, to 3, with none around it.
const OCCLUSION_SHADES: [f32; 4] = [0.35, 0.55, 0.76, 1.0];

/// The occlusion level of a corner with no solid Cell around it.
const UNOCCLUDED: u8 = 3;

/// The Chunks that share a face with the Chunk at `pos`.
pub(crate) fn face_neighbours(pos: ChunkPos) -> [ChunkPos; 6] {
    FACE_NEIGHBOURS.map(|(dx, dy, dz)| ChunkPos::new(pos.x + dx, pos.y + dy, pos.z + dz))
}

/// The Chunks that touch the Chunk at `pos`, by a face, an edge or a corner. Its solid mesh
/// depends on all of them: the faces along its border on its face neighbours, and the shading of
/// those faces' corners on the others too.
pub(crate) fn neighbours(pos: ChunkPos) -> [ChunkPos; 26] {
    NEIGHBOURS.map(|(dx, dy, dz)| ChunkPos::new(pos.x + dx, pos.y + dy, pos.z + dz))
}

/// The colour each solid Element is drawn in, by [`ElementId::index`]. Every other Element is
/// `None`, so this also says which Cells are meshed here.
#[derive(Clone, Debug)]
pub struct SolidColors(Vec<Option<FaceLook>>);

#[derive(Clone, Copy, Debug)]
struct FaceLook {
    color: [f32; 4],
    band: Option<usize>,
}

// The atlas repeats horizontally. Each vertical band holds a Chunk's worth
// of repeats plus one wrap tile at each end for samples right on an edge.
// A final white tile preserves the Element colour for untextured solids.
const BAND_HEIGHT: usize = CHUNK_SIZE + 2;
const ATLAS_HEIGHT: usize = 4 * BAND_HEIGHT + 1;

impl SolidColors {
    pub fn new(elements: &ElementTable) -> Self {
        let colors = elements
            .iter()
            .map(|(_, element)| {
                let [r, g, b] = element.color;
                (element.phase == Phase::Solid).then(|| FaceLook {
                    color: [r, g, b, 1.0],
                    band: ["Dirt", "Granite", "Ice", "Igneous Rock"]
                        .iter()
                        .position(|&name| name == element.name),
                })
            })
            .collect();
        Self(colors)
    }

    /// The mesher's value for a Cell of `element`: 0 unless it is solid, otherwise its index + 1.
    fn voxel(&self, element: ElementId) -> u16 {
        let index = element.index();
        match self.0.get(usize::from(index)) {
            // The table holds fewer than `u16::MAX` Elements, so this doesn't overflow.
            Some(Some(_)) => index + 1,
            _ => 0,
        }
    }

    /// The colour and texture of the solid Element whose mesher value is `voxel`.
    fn of_voxel(&self, voxel: u16) -> FaceLook {
        self.0
            .get(usize::from(voxel) - 1)
            .copied()
            .flatten()
            .expect("the mesher emits quads only for the values of solid Elements")
    }
}

/// What meshing a Chunk reads: its own Cells, and those of each neighbour that is loaded.
pub struct ChunkNeighbourhood {
    cells: Arc<[Cell]>,
    /// In [`NEIGHBOURS`] order. `None` where the neighbour isn't loaded.
    neighbours: [Option<Arc<[Cell]>>; 26],
}

impl ChunkNeighbourhood {
    /// The Chunk at `pos` and its 26 neighbours, as `chunk` finds them. `None` if `chunk` lacks
    /// the Chunk at `pos` itself.
    pub fn gather(
        pos: ChunkPos,
        mut chunk: impl FnMut(ChunkPos) -> Option<Arc<[Cell]>>,
    ) -> Option<Self> {
        let cells = chunk(pos)?;
        let neighbours = neighbours(pos).map(chunk);
        Some(Self { cells, neighbours })
    }

    /// The Chunk's own Cells, in the order `sim` lists them.
    pub fn cells(&self) -> &[Cell] {
        &self.cells
    }

    /// The Cells of the neighbour at `offset`, if it is loaded.
    fn neighbour(&self, offset: (i32, i32, i32)) -> Option<&Arc<[Cell]>> {
        self.neighbours[neighbour_slot(offset)].as_ref()
    }

    /// The Cell at `[x, y, z]` relative to the Chunk's origin, which may lie one Cell outside the
    /// Chunk along any of the axes, in a neighbour. `None` if that neighbour isn't loaded.
    pub fn cell(&self, [x, y, z]: [i32; 3]) -> Option<Cell> {
        let size = CHUNK_SIZE as i32;
        let inside = |c: i32| (0..size).contains(&c);
        if inside(x) && inside(y) && inside(z) {
            // In the Chunk itself, as nearly every lookup is.
            return Some(self.cells[chunk_index([x, y, z].map(|c| c as usize))]);
        }
        let step = |c: i32| {
            debug_assert!(
                (-1..=size).contains(&c),
                "{c} is more than one Cell outside"
            );
            if c < 0 {
                -1
            } else if c >= size {
                1
            } else {
                0
            }
        };
        let cells = self.neighbour((step(x), step(y), step(z)))?;
        let local = [x, y, z].map(|c| c.rem_euclid(size) as usize);
        Some(cells[chunk_index(local)])
    }
}

/// A Chunk's solid Cells as triangles, with positions relative to the Chunk's origin. Every four
/// vertices make one quad, drawn as two triangles.
#[derive(Debug, Default)]
pub struct ChunkGeometry {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    /// Linear RGBA.
    pub colors: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
    pub(super) uvs: Vec<[f32; 2]>,
}

/// Meshes the solid Cells of a Chunk. Faces between two solid Cells are left out, and so are
/// faces against a solid Cell of a neighbour Chunk; a missing neighbour counts as not solid. Each
/// corner of a face is darkened by the solid Cells around it in front of the face.
pub fn mesh_solids(neighbourhood: &ChunkNeighbourhood, colors: &SolidColors) -> ChunkGeometry {
    let voxel = |cell: Cell| colors.voxel(cell.element);
    let Some((padded, quads)) = greedy_quads(neighbourhood, voxel, Faces::Opaque) else {
        return ChunkGeometry::default();
    };
    let occluders = Occluders::new(&padded, neighbourhood, voxel);
    ChunkGeometry::from_quads(&quads, |voxel| colors.of_voxel(voxel), Some(&occluders))
}

/// Which faces between two drawn Cells [`mesh_greedy`] leaves out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Faces {
    /// Every one: neither Cell can be seen through.
    Opaque,
    /// Only those between two Cells of the same value. Where the value changes, both Cells show a
    /// face, each facing the other.
    Translucent,
}

/// Meshes the Cells of a Chunk that `voxel` gives a value other than 0, merging neighbouring
/// faces of the same value, and colouring each face by its Cell's value with `color`. A face
/// shows against a Cell of value 0, and against a missing neighbour Chunk.
pub fn mesh_greedy(
    neighbourhood: &ChunkNeighbourhood,
    voxel: impl Fn(Cell) -> u16,
    faces: Faces,
    color: impl Fn(u16) -> [f32; 4],
) -> ChunkGeometry {
    match greedy_quads(neighbourhood, voxel, faces) {
        Some((_, quads)) => ChunkGeometry::from_quads(
            &quads,
            |voxel| FaceLook {
                color: color(voxel),
                band: None,
            },
            None,
        ),
        None => ChunkGeometry::default(),
    }
}

/// The greedy mesher's quads for the Cells of a Chunk that `voxel` gives a value other than 0,
/// for each face direction in [`Face`] order, with the padded cube they were meshed from. `None`
/// if there are no such Cells.
fn greedy_quads(
    neighbourhood: &ChunkNeighbourhood,
    voxel: impl Fn(Cell) -> u16,
    faces: Faces,
) -> Option<(PaddedChunk, [Vec<Quad>; 6])> {
    if neighbourhood.cells.iter().all(|&cell| voxel(cell) == 0) {
        // Such as a Chunk of solids high in the Atmosphere: about half of render distance.
        return None;
    }
    let padded = PaddedChunk::new(neighbourhood, voxel);
    let mut mesher = Mesher::<CHUNK_SIZE>::new();
    let none = vec![0; PADDED.pow(2)];
    let (opaque, translucent): (&[u64], &[u64]) = match faces {
        Faces::Opaque => (&padded.columns, &none),
        Faces::Translucent => (&none, &padded.columns),
    };
    mesher.fast_mesh(&padded.voxels, opaque, translucent);
    Some((padded, mesher.quads))
}

impl ChunkGeometry {
    /// Whether there is nothing to draw.
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    /// A mesh for the GPU alone: the main world drops its copy once uploaded.
    pub fn into_mesh(self) -> Mesh {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, self.colors)
        .with_inserted_indices(Indices::U32(self.indices));
        if !self.uvs.is_empty() {
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, self.uvs);
        }
        mesh
    }

    /// `quads` lists the mesher's quads for each face direction, in [`Face`] order. `look` gives
    /// the colour and texture of a quad's value. With `occluders`, each corner is darkened by the solid Cells
    /// around it in front of its face, and a quad is split where that changes across it.
    fn from_quads(
        quads: &[Vec<Quad>; 6],
        look: impl Fn(u16) -> FaceLook,
        occluders: Option<&Occluders>,
    ) -> Self {
        let count = quads.iter().map(Vec::len).sum::<usize>();
        let mut geometry = Self {
            positions: Vec::with_capacity(count * 4),
            normals: Vec::with_capacity(count * 4),
            colors: Vec::with_capacity(count * 4),
            indices: Vec::with_capacity(count * 6),
            uvs: Vec::new(),
        };
        let mut scratch = Scratch::default();
        for (face, quads) in (0..6).map(Face::from).zip(quads) {
            let side = Side::of(face);
            for &quad in quads {
                let voxel = u16::try_from(quad.voxel_id()).expect("values are u16");
                let FaceLook { color, band } = look(voxel);
                let rect = side.rect(face.vertices_packed(quad));
                match occluders {
                    Some(occluders) => {
                        let start = geometry.positions.len();
                        geometry.push_occluded(rect, color, occluders, &mut scratch);
                        let [u, v] = side.along();
                        geometry
                            .uvs
                            .extend(geometry.positions[start..].iter().map(|p| {
                                let row = match band {
                                    Some(band) => (band * BAND_HEIGHT + 1) as f32 + p[v],
                                    None => ATLAS_HEIGHT as f32 - 0.5,
                                };
                                [p[u], row / ATLAS_HEIGHT as f32]
                            }));
                    }
                    None => geometry.push_rect(rect, color, [UNOCCLUDED; 4]),
                }
            }
        }
        geometry
    }

    /// Adds `rect`, split into as few quads as keep each one's shading exact: within each, the
    /// occlusion at every Cell corner is what interpolating between the quad's own corners gives.
    fn push_occluded(
        &mut self,
        rect: Rect,
        color: [f32; 4],
        occluders: &Occluders,
        scratch: &mut Scratch,
    ) {
        let Rect {
            u: [u0, u1],
            v: [v0, v1],
            ..
        } = rect;
        let (width, height) = ((u1 - u0) as usize, (v1 - v0) as usize);
        let Scratch {
            solid,
            levels,
            done,
        } = scratch;

        // Which Cells are solid in the layer the rectangle looks out into, from one before it to
        // one past it along u and v, u fastest. Those right in front of it are all open, or it
        // would show no face there, so only the ring of Cells around them is looked up.
        let span = width + 2;
        solid.clear();
        solid.resize(span * (height + 2), false);
        let mut any_solid = false;
        let mut look_up = |i: usize, j: usize| {
            let (u, v) = (u0 - 1 + i as i32, v0 - 1 + j as i32);
            let is_solid = occluders.in_front(rect.side, rect.plane, u, v);
            solid[j * span + i] = is_solid;
            any_solid |= is_solid;
        };
        for i in 0..span {
            look_up(i, 0);
            look_up(i, height + 1);
        }
        for j in 1..=height {
            look_up(0, j);
            look_up(width + 1, j);
        }
        if !any_solid {
            // Nearly every quad in the open.
            self.push_rect(rect, color, [UNOCCLUDED; 4]);
            return;
        }

        // The occlusion at every Cell corner of the rectangle, u fastest. The four Cells around
        // the corner `i` along u and `j` along v lie at `i` and `i + 1`, and `j` and `j + 1`, in
        // `solid`.
        levels.clear();
        for j in 0..=height {
            for i in 0..=width {
                let at = |i: usize, j: usize| solid[j * span + i];
                levels.push(occlusion([
                    at(i, j),
                    at(i + 1, j),
                    at(i + 1, j + 1),
                    at(i, j + 1),
                ]));
            }
        }

        // The occlusion at the corners of the Cell-sized face `i` along u and `j` along v from
        // the rectangle's corner, in the order `push_rect` takes them.
        let corners = |i: usize, j: usize| {
            let at = |i: usize, j: usize| levels[j * (width + 1) + i];
            [at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1)]
        };
        // Greedy again, over the Cell-sized faces: merge runs whose corners match. Neighbours
        // share corners, so matching faces have the same occlusion all along the run, and
        // interpolating between its ends gives every corner within it exactly.
        done.clear();
        done.resize(width * height, false);
        for j in 0..height {
            for i in 0..width {
                if done[j * width + i] {
                    continue;
                }
                let these = corners(i, j);
                let mut w = 1;
                while i + w < width && !done[j * width + i + w] && corners(i + w, j) == these {
                    w += 1;
                }
                let mut h = 1;
                while j + h < height
                    && (i..i + w).all(|k| !done[(j + h) * width + k] && corners(k, j + h) == these)
                {
                    h += 1;
                }
                for row in j..j + h {
                    done[row * width + i..row * width + i + w].fill(true);
                }
                let (i, j, w, h) = (i as i32, j as i32, w as i32, h as i32);
                let part = Rect {
                    u: [u0 + i, u0 + i + w],
                    v: [v0 + j, v0 + j + h],
                    ..rect
                };
                self.push_rect(part, color, these);
            }
        }
    }

    /// Adds one quad covering `rect`, in `color` darkened at each corner by its occlusion level
    /// in `levels`: at (u0, v0), (u1, v0), (u1, v1) and (u0, v1).
    fn push_rect(&mut self, rect: Rect, color: [f32; 4], levels: [u8; 4]) {
        let Rect {
            side,
            plane,
            u: [u0, u1],
            v: [v0, v1],
        } = rect;
        let [u_axis, v_axis] = side.along();
        let corner = |u: i32, v: i32| {
            let mut position = [plane as f32; 3];
            position[u_axis] = u as f32;
            position[v_axis] = v as f32;
            position
        };
        // Counter-clockwise seen from the positive end of the axis, since u × v points that way.
        let mut corners = [
            (corner(u0, v0), levels[0]),
            (corner(u1, v0), levels[1]),
            (corner(u1, v1), levels[2]),
            (corner(u0, v1), levels[3]),
        ];
        if !side.positive {
            // Counter-clockwise seen from the negative end.
            corners.reverse();
        }

        let first = u32::try_from(self.positions.len()).expect("fewer than 2³² vertices");
        let normal = side.normal();
        let [r, g, b, alpha] = color;
        for (position, level) in corners {
            let shade = OCCLUSION_SHADES[usize::from(level)];
            self.positions.push(position);
            self.normals.push(normal);
            self.colors.push([r * shade, g * shade, b * shade, alpha]);
        }
        // Two triangles, split along the diagonal between the less occluded pair of corners: a
        // dark corner then darkens only its own triangle, rather than a band across the quad.
        let [a, b, c, d] = corners.map(|(_, level)| u32::from(level));
        let triangles = if a + c >= b + d {
            [0, 1, 2, 0, 2, 3]
        } else {
            [1, 2, 3, 1, 3, 0]
        };
        self.indices.extend(triangles.map(|i| first + i));
    }
}

/// Working space for [`ChunkGeometry::push_occluded`], reused from one quad to the next.
#[derive(Default)]
struct Scratch {
    solid: Vec<bool>,
    levels: Vec<u8>,
    done: Vec<bool>,
}

/// How closed in a corner of a face is, by which of the four Cells that meet there, in the layer
/// the face looks out into, are solid: from 3, with none of them, to 0. They are listed in order
/// around the corner, so the first and third, and the second and fourth, lie across it from each
/// other. At least one is open: the Cell in front of the face.
fn occlusion(solid: [bool; 4]) -> u8 {
    let [a, b, c, d] = solid;
    match solid.into_iter().filter(|&solid| solid).count() {
        0 => UNOCCLUDED,
        1 => 2,
        2 if !(a && c || b && d) => 1,
        // Two across the corner from each other close it in as much as three do: they stand on
        // both sides of the open Cell in front of the face.
        _ => 0,
    }
}

/// Which way a face points: along `axis` (0 for x, 1 for y, 2 for z), to its positive or
/// negative end.
#[derive(Clone, Copy, Debug)]
struct Side {
    axis: usize,
    positive: bool,
}

impl Side {
    fn of(face: Face) -> Self {
        let n = face.n();
        let axis = n.iter().position(|&c| c != 0).expect("a face has a normal");
        Self {
            axis,
            positive: n[axis] > 0,
        }
    }

    fn normal(self) -> [f32; 3] {
        let mut normal = [0.0; 3];
        normal[self.axis] = if self.positive { 1.0 } else { -1.0 };
        normal
    }

    /// The two axes a face on this side spans, u then v, in the order that makes u × v point to
    /// the positive end of `axis`.
    fn along(self) -> [usize; 2] {
        [(self.axis + 1) % 3, (self.axis + 2) % 3]
    }

    /// The rectangle a quad's vertices, as the mesher gives them, span.
    fn rect(self, vertices: [Vertex; 4]) -> Rect {
        let positions = vertices.map(|vertex| vertex.xyz().map(|c| c as i32));
        let span = |axis: usize| {
            let along = positions.map(|position| position[axis]);
            [along.into_iter().min(), along.into_iter().max()].map(Option::unwrap)
        };
        let [u_axis, v_axis] = self.along();
        Rect {
            side: self,
            plane: span(self.axis)[0],
            u: span(u_axis),
            v: span(v_axis),
        }
    }
}

/// A face rectangle, relative to the Chunk's origin: which way it points, where it lies along
/// that axis, and the Cell corners it runs between along the two others ([`Side::along`]).
#[derive(Clone, Copy, Debug)]
struct Rect {
    side: Side,
    plane: i32,
    u: [i32; 2],
    v: [i32; 2],
}

/// Which Cells of a Chunk's padded cube are solid, including those along its edges and at its
/// corners, which the mesher's own padding leaves out: what darkens the corners of solid faces.
struct Occluders {
    /// One `u64` per (x, y) column of the padded cube, with bit z set where the Cell is solid, as
    /// in [`PaddedChunk::columns`].
    columns: Box<[u64]>,
}

impl Occluders {
    /// The Cells `voxel` gives a value other than 0. `padded` holds those of the Chunk and its
    /// face neighbours already.
    fn new(
        padded: &PaddedChunk,
        neighbourhood: &ChunkNeighbourhood,
        voxel: impl Fn(Cell) -> u16,
    ) -> Self {
        let mut columns = padded.columns.clone();
        let size = CHUNK_SIZE as i32;
        let span = |offset: i32| match offset {
            -1 => -1..0,
            1 => size..size + 1,
            _ => 0..size,
        };
        let edge_or_corner = |&(dx, dy, dz): &(i32, i32, i32)| {
            [dx, dy, dz].into_iter().filter(|&d| d != 0).count() >= 2
        };
        for (dx, dy, dz) in NEIGHBOURS.into_iter().filter(edge_or_corner) {
            for x in span(dx) {
                for y in span(dy) {
                    for z in span(dz) {
                        if neighbourhood
                            .cell([x, y, z])
                            .is_some_and(|cell| voxel(cell) != 0)
                        {
                            let [x, y, z] = [x, y, z].map(|c| (c + 1) as usize);
                            columns[x + y * PADDED] |= 1 << z;
                        }
                    }
                }
            }
        }
        Self { columns }
    }

    /// Whether the Cell at `[x, y, z]` relative to the Chunk's origin, each from −1 to
    /// `CHUNK_SIZE`, is solid.
    fn is_solid(&self, position: [i32; 3]) -> bool {
        let [x, y, z] = position.map(|c| (c + 1) as usize);
        self.columns[x + y * PADDED] >> z & 1 == 1
    }

    /// Whether the Cell at `(u, v)` is solid, in the layer that faces on `side` lying at `plane`
    /// along its axis look out into.
    fn in_front(&self, side: Side, plane: i32, u: i32, v: i32) -> bool {
        let [u_axis, v_axis] = side.along();
        let mut position = [if side.positive { plane } else { plane - 1 }; 3];
        position[u_axis] = u;
        position[v_axis] = v;
        self.is_solid(position)
    }
}

/// The mesher's input for one Chunk.
struct PaddedChunk {
    /// One value per padded Cell, in the mesher's order: z fastest, then x, then y
    /// (`binary_greedy_meshing::pad_linearize`). That is not the order `sim` lists Cells in.
    voxels: Box<[u16]>,
    /// One `u64` per (x, y) column of `voxels`, with bit z set where that value isn't 0.
    columns: Box<[u64]>,
}

impl PaddedChunk {
    /// The values `voxel` gives the Chunk's Cells and the layer around it.
    fn new(neighbourhood: &ChunkNeighbourhood, voxel: impl Fn(Cell) -> u16) -> Self {
        let mut padded = Self {
            voxels: vec![0; PADDED.pow(3)].into(),
            columns: vec![0; PADDED.pow(2)].into(),
        };
        for (index, &cell) in neighbourhood.cells.iter().enumerate() {
            let [x, y, z] = chunk_local(index);
            padded.set([x + 1, y + 1, z + 1], voxel(cell));
        }
        for offset in FACE_NEIGHBOURS {
            let Some(cells) = neighbourhood.neighbour(offset) else {
                // A missing neighbour's layer stays 0.
                continue;
            };
            let face = BorderLayer::facing(offset);
            for a in 0..CHUNK_SIZE {
                for b in 0..CHUNK_SIZE {
                    let (local, padded_pos) = face.cell(a, b);
                    padded.set(padded_pos, voxel(cells[chunk_index(local)]));
                }
            }
        }
        padded
    }

    /// Sets the value at padded position `[x, y, z]`, each from 0 to `CHUNK_SIZE + 1`.
    fn set(&mut self, [x, y, z]: [usize; 3], voxel: u16) {
        if voxel != 0 {
            let column = x + y * PADDED;
            self.voxels[z + column * PADDED] = voxel;
            self.columns[column] |= 1 << z;
        }
    }
}

/// The layer of a neighbour Chunk's Cells that touches the Chunk being meshed, and where it goes
/// in the padding.
struct BorderLayer {
    /// The axis the neighbour lies along: 0 for x, 1 for y, 2 for z.
    axis: usize,
    /// The layer's coordinate along `axis`, in the neighbour.
    local: usize,
    /// The same, in the padded cube.
    padded: usize,
}

impl BorderLayer {
    /// The layer of the neighbour at `offset` that faces back towards the Chunk.
    fn facing(offset: (i32, i32, i32)) -> Self {
        let (axis, sign) = match offset {
            (sign, 0, 0) => (0, sign),
            (0, sign, 0) => (1, sign),
            (0, 0, sign) => (2, sign),
            _ => unreachable!("a face neighbour is offset along one axis"),
        };
        if sign > 0 {
            // The neighbour's first layer lies just past the Chunk's last.
            Self {
                axis,
                local: 0,
                padded: CHUNK_SIZE + 1,
            }
        } else {
            Self {
                axis,
                local: CHUNK_SIZE - 1,
                padded: 0,
            }
        }
    }

    /// The layer's Cell at `(a, b)` along the other two axes: its position in the neighbour, and
    /// in the padded cube.
    fn cell(&self, a: usize, b: usize) -> ([usize; 3], [usize; 3]) {
        let mut local = [0; 3];
        let mut padded = [0; 3];
        let others = [(self.axis + 1) % 3, (self.axis + 2) % 3];
        for (axis, along) in others.into_iter().zip([a, b]) {
            local[axis] = along;
            padded[axis] = along + 1;
        }
        local[self.axis] = self.local;
        padded[self.axis] = self.padded;
        (local, padded)
    }
}

/// The position `[x, y, z]` within its Chunk of the Cell at `index`, in the order `sim` lists a
/// Chunk's Cells: x fastest, then z, then y.
pub(super) fn chunk_local(index: usize) -> [usize; 3] {
    [
        index % CHUNK_SIZE,
        index / CHUNK_SIZE.pow(2),
        index / CHUNK_SIZE % CHUNK_SIZE,
    ]
}

/// The index of the Cell at `[x, y, z]` within its Chunk, in the order `sim` lists them.
pub(super) fn chunk_index([x, y, z]: [usize; 3]) -> usize {
    (y * CHUNK_SIZE + z) * CHUNK_SIZE + x
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use sim::CellPos;

    use super::*;
    use crate::render::test_support::{
        ShadedQuad, World, elements, expected_quads, expected_shaded_quad, origin_neighbourhood,
        quads_of, shaded_quads, world,
    };

    #[test]
    fn solid_faces_repeat_once_per_metre_across_chunk_borders() {
        let cells: Vec<_> = (30..35)
            .map(|x| (CellPos::new(x, 4, 7), "Granite"))
            .collect();
        let world = world(&cells);
        for x in [0, 1] {
            let chunk =
                ChunkNeighbourhood::gather(ChunkPos::new(x, 0, 0), |pos| world.get(&pos).cloned())
                    .unwrap();
            let mesh = mesh_solids(&chunk, &SolidColors::new(&elements())).into_mesh();
            let bevy::mesh::VertexAttributeValues::Float32x2(uvs) =
                mesh.attribute(Mesh::ATTRIBUTE_UV_0).unwrap()
            else {
                panic!("expected UV coordinates")
            };
            let positions = mesh
                .attribute(Mesh::ATTRIBUTE_POSITION)
                .unwrap()
                .as_float3()
                .unwrap();
            let normals = mesh
                .attribute(Mesh::ATTRIBUTE_NORMAL)
                .unwrap()
                .as_float3()
                .unwrap();
            // Each Chunk has four merged sides and one end cap.
            assert_eq!(uvs.len(), 20);
            for ((uv, &[x, y, z]), normal) in uvs.iter().zip(positions).zip(normals) {
                let expected = if normal[0] != 0.0 {
                    [y, z]
                } else if normal[1] != 0.0 {
                    [z, x]
                } else {
                    [x, y]
                };
                // Granite begins after 34 Dirt tiles and its first wrap tile.
                assert_eq!(uv[0], expected[0]);
                assert!((uv[1] * 137.0 - 35.0 - expected[1]).abs() < 0.00001);
            }
        }
    }

    #[test]
    fn a_remesh_uses_element_identity_and_preserves_untextured_colours() {
        let world = world(&[(CellPos::new(5, 6, 7), "Granite")]);
        let chunk = origin_neighbourhood(&world);
        // Reuse the same Cell (Element index 1) with each replacement table.
        for (name, first_row) in [
            ("Dirt", 1),
            ("Granite", 35),
            ("Ice", 69),
            ("Igneous Rock", 103),
            ("Quartz", 0),
        ] {
            let mut table = ElementTable::new();
            table.add(sim::Element::new(name).color([0.4, 0.3, 0.7]));
            let mesh = mesh_solids(&chunk, &SolidColors::new(&table));
            assert_eq!(mesh.uvs.len(), 24);
            for ((uv, position), normal) in mesh.uvs.iter().zip(&mesh.positions).zip(&mesh.normals)
            {
                let v = if normal[0] != 0.0 {
                    position[2]
                } else if normal[1] != 0.0 {
                    position[0]
                } else {
                    position[1]
                };
                // The white fallback samples the centre of the last tile.
                let row = if name == "Quartz" {
                    136.5
                } else {
                    first_row as f32 + v
                };
                assert!((uv[1] * 137.0 - row).abs() < 0.00002);
            }
            assert_eq!(mesh.colors, vec![[0.4, 0.3, 0.7, 1.0]; 24]);
        }
    }

    fn padded(world: &World) -> PaddedChunk {
        let colors = SolidColors::new(&elements());
        PaddedChunk::new(&origin_neighbourhood(world), |cell| {
            colors.voxel(cell.element)
        })
    }

    #[test]
    fn a_cell_on_a_neighbours_border_lands_in_the_padding_beside_it() {
        // One Granite Cell on the border of each of the origin Chunk's six neighbours, and one
        // a layer further in, which the padding mustn't reach.
        let world = world(&[
            (CellPos::new(3, 32, 4), "Granite"),
            (CellPos::new(3, 33, 4), "Granite"),
            (CellPos::new(3, -1, 4), "Granite"),
            (CellPos::new(3, -2, 4), "Granite"),
            (CellPos::new(32, 5, 7), "Granite"),
            (CellPos::new(33, 5, 7), "Granite"),
            (CellPos::new(-1, 5, 7), "Granite"),
            (CellPos::new(-2, 5, 7), "Granite"),
            (CellPos::new(9, 10, 32), "Granite"),
            (CellPos::new(9, 10, 33), "Granite"),
            (CellPos::new(9, 10, -1), "Granite"),
            (CellPos::new(9, 10, -2), "Granite"),
        ]);
        let padded = padded(&world);

        // Granite is index 1, so the mesher's value is 1 + 1 = 2. A world Cell (x, y, z) sits at
        // padded (x + 1, y + 1, z + 1), and the padded index is z + 34x + 34²y = z + 34x + 1156y.
        let granite = 2;
        let expected = [
            38_289, // (3, 32, 4) → (4, 33, 5): 5 + 136 + 38148
            141,    // (3, -1, 4) → (4, 0, 5): 5 + 136 + 0
            8_066,  // (32, 5, 7) → (33, 6, 8): 8 + 1122 + 6936
            6_944,  // (-1, 5, 7) → (0, 6, 8): 8 + 0 + 6936
            13_089, // (9, 10, 32) → (10, 11, 33): 33 + 340 + 12716
            13_056, // (9, 10, -1) → (10, 11, 0): 0 + 340 + 12716
        ];
        for index in expected {
            assert_eq!(padded.voxels[index], granite, "padded index {index}");
        }
        let solid = padded.voxels.iter().filter(|&&voxel| voxel != 0).count();
        assert_eq!(solid, 6, "only the six border Cells are in the padding");
    }

    #[test]
    fn a_missing_neighbour_leaves_its_padding_layer_empty() {
        // The origin Chunk is solid Granite, and none of its neighbours is loaded.
        let granite = Cell {
            element: elements().id_of("Granite").unwrap(),
            mass_grams: 2_700_000,
            temperature_k: 293.15,
        };
        let world = HashMap::from([(
            ChunkPos::new(0, 0, 0),
            Arc::from(vec![granite; CHUNK_SIZE.pow(3)]),
        )]);
        let padded = padded(&world);

        for (index, &voxel) in padded.voxels.iter().enumerate() {
            // Undo the padded index z + 34x + 1156y.
            let (z, x, y) = (index % 34, index / 34 % 34, index / 1156);
            let in_padding = [x, y, z].iter().any(|&c| c == 0 || c == 33);
            let expected = if in_padding { 0 } else { 2 };
            assert_eq!(voxel, expected, "padded ({x}, {y}, {z})");
        }
    }

    #[test]
    fn liquid_and_gas_cells_are_not_solid() {
        let world = world(&[
            (CellPos::new(5, 5, 5), "Water"),
            (CellPos::new(6, 5, 5), "Oxygen"),
            (CellPos::new(32, 5, 5), "Water"),
            (CellPos::new(32, 6, 5), "Oxygen"),
        ]);
        let padded = padded(&world);

        assert!(padded.voxels.iter().all(|&voxel| voxel == 0));
        assert!(padded.columns.iter().all(|&column| column == 0));
    }

    /// The quads of the origin Chunk's solid geometry, checked and sorted by [`quads_of`].
    fn quads(world: &World) -> Vec<crate::render::test_support::Quad> {
        quads_of(&mesh_solids(
            &origin_neighbourhood(world),
            &SolidColors::new(&elements()),
        ))
    }

    const GRANITE: [f32; 4] = [0.55, 0.5, 0.5, 1.0];
    const DIRT: [f32; 4] = [0.45, 0.3, 0.18, 1.0];

    #[test]
    fn a_single_solid_cell_has_six_outward_faces() {
        let world = world(&[(CellPos::new(5, 6, 7), "Granite")]);

        // The Cell spans (5, 6, 7) to (6, 7, 8). Each face lies on the side its normal points to.
        let expected = expected_quads(
            GRANITE,
            &[
                ([1.0, 0.0, 0.0], [6.0, 6.0, 7.0], [6.0, 7.0, 8.0]),
                ([-1.0, 0.0, 0.0], [5.0, 6.0, 7.0], [5.0, 7.0, 8.0]),
                ([0.0, 1.0, 0.0], [5.0, 7.0, 7.0], [6.0, 7.0, 8.0]),
                ([0.0, -1.0, 0.0], [5.0, 6.0, 7.0], [6.0, 6.0, 8.0]),
                ([0.0, 0.0, 1.0], [5.0, 6.0, 8.0], [6.0, 7.0, 8.0]),
                ([0.0, 0.0, -1.0], [5.0, 6.0, 7.0], [6.0, 7.0, 7.0]),
            ],
        );
        assert_eq!(quads(&world), expected);
    }

    #[test]
    fn two_adjacent_cells_of_one_element_share_no_face() {
        let world = world(&[
            (CellPos::new(5, 6, 7), "Granite"),
            (CellPos::new(6, 6, 7), "Granite"),
        ]);

        // Together they span (5, 6, 7) to (7, 7, 8). Nothing lies on x = 6 between them, and
        // greedy meshing merges each side into a single quad.
        let expected = expected_quads(
            GRANITE,
            &[
                ([1.0, 0.0, 0.0], [7.0, 6.0, 7.0], [7.0, 7.0, 8.0]),
                ([-1.0, 0.0, 0.0], [5.0, 6.0, 7.0], [5.0, 7.0, 8.0]),
                ([0.0, 1.0, 0.0], [5.0, 7.0, 7.0], [7.0, 7.0, 8.0]),
                ([0.0, -1.0, 0.0], [5.0, 6.0, 7.0], [7.0, 6.0, 8.0]),
                ([0.0, 0.0, 1.0], [5.0, 6.0, 8.0], [7.0, 7.0, 8.0]),
                ([0.0, 0.0, -1.0], [5.0, 6.0, 7.0], [7.0, 7.0, 7.0]),
            ],
        );
        assert_eq!(quads(&world), expected);
    }

    #[test]
    fn a_face_against_a_neighbour_chunk_shows_only_if_that_neighbour_is_missing() {
        // A Granite Cell in the origin Chunk's +x, +z corner, spanning (31, 6, 31) to
        // (32, 7, 32). Its +x and +z faces lie on the borders with two neighbour Chunks.
        let corner = (CellPos::new(31, 6, 31), "Granite");
        let other_faces = [
            ([-1.0, 0.0, 0.0], [31.0, 6.0, 31.0], [31.0, 7.0, 32.0]),
            ([0.0, 1.0, 0.0], [31.0, 7.0, 31.0], [32.0, 7.0, 32.0]),
            ([0.0, -1.0, 0.0], [31.0, 6.0, 31.0], [32.0, 6.0, 32.0]),
            ([0.0, 0.0, -1.0], [31.0, 6.0, 31.0], [32.0, 7.0, 31.0]),
        ];

        // Both neighbours are loaded, with Granite against those faces.
        let covered = world(&[
            corner,
            (CellPos::new(32, 6, 31), "Granite"),
            (CellPos::new(31, 6, 32), "Granite"),
        ]);
        assert_eq!(quads(&covered), expected_quads(GRANITE, &other_faces));

        // Neither neighbour is loaded.
        let alone = world(&[corner]);
        let mut all_faces = other_faces.to_vec();
        all_faces.push(([1.0, 0.0, 0.0], [32.0, 6.0, 31.0], [32.0, 7.0, 32.0]));
        all_faces.push(([0.0, 0.0, 1.0], [31.0, 6.0, 32.0], [32.0, 7.0, 32.0]));
        assert_eq!(quads(&alone), expected_quads(GRANITE, &all_faces));
    }

    #[test]
    fn each_quad_has_its_elements_colour() {
        // Granite spans (5, 6, 7) to (6, 7, 8), and Dirt (6, 6, 7) to (7, 7, 8). Both are solid,
        // so nothing lies on x = 6 between them, and different Elements don't merge.
        let world = world(&[
            (CellPos::new(5, 6, 7), "Granite"),
            (CellPos::new(6, 6, 7), "Dirt"),
        ]);

        let mut expected = expected_quads(
            GRANITE,
            &[
                ([-1.0, 0.0, 0.0], [5.0, 6.0, 7.0], [5.0, 7.0, 8.0]),
                ([0.0, 1.0, 0.0], [5.0, 7.0, 7.0], [6.0, 7.0, 8.0]),
                ([0.0, -1.0, 0.0], [5.0, 6.0, 7.0], [6.0, 6.0, 8.0]),
                ([0.0, 0.0, 1.0], [5.0, 6.0, 8.0], [6.0, 7.0, 8.0]),
                ([0.0, 0.0, -1.0], [5.0, 6.0, 7.0], [6.0, 7.0, 7.0]),
            ],
        );
        expected.extend(expected_quads(
            DIRT,
            &[
                ([1.0, 0.0, 0.0], [7.0, 6.0, 7.0], [7.0, 7.0, 8.0]),
                ([0.0, 1.0, 0.0], [6.0, 7.0, 7.0], [7.0, 7.0, 8.0]),
                ([0.0, -1.0, 0.0], [6.0, 6.0, 7.0], [7.0, 6.0, 8.0]),
                ([0.0, 0.0, 1.0], [6.0, 6.0, 8.0], [7.0, 7.0, 8.0]),
                ([0.0, 0.0, -1.0], [6.0, 6.0, 7.0], [7.0, 7.0, 7.0]),
            ],
        ));
        expected.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(quads(&world), expected);
    }

    /// `color` darkened as a corner is at occlusion `level`: 3 for none, down to 0 for the most.
    fn occluded(color: [f32; 4], level: usize) -> [f32; 4] {
        let [r, g, b, a] = color;
        let shade = OCCLUSION_SHADES[level];
        [r * shade, g * shade, b * shade, a]
    }

    /// The quads of the origin Chunk's solid geometry with `normal`, lying in the plane where the
    /// coordinate along that normal is `plane`.
    fn shaded_quads_in(world: &World, normal: [f32; 3], plane: f32) -> Vec<ShadedQuad> {
        let axis = normal.iter().position(|&n| n != 0.0).unwrap();
        shaded_quads(&mesh_solids(
            &origin_neighbourhood(world),
            &SolidColors::new(&elements()),
        ))
        .into_iter()
        .filter(|quad| quad.normal == normal && quad.min[axis] == plane)
        .collect()
    }

    const UP: [f32; 3] = [0.0, 1.0, 0.0];

    #[test]
    fn a_floor_along_a_wall_darkens_towards_it_in_one_strip() {
        // A 4 × 4 Granite floor at y = 5, x and z 5 to 8, and a wall along its z = 5 edge, one
        // Cell high, running x = 4 to 9: past both ends of the floor.
        let mut cells: Vec<(CellPos, &str)> = Vec::new();
        for x in 5..=8 {
            for z in 5..=8 {
                cells.push((CellPos::new(x, 5, z), "Granite"));
            }
        }
        for x in 4..=9 {
            cells.push((CellPos::new(x, 6, 5), "Granite"));
        }
        let world = world(&cells);

        // The wall covers the floor's z = 5 row, so the top shows from z = 6 to 9, on y = 6.
        // Every corner on z = 6 has two wall Cells beside it, at z = 5 and x either side of it:
        // occlusion 1. Every other corner has none: 3. So the row along the wall is one strip,
        // darker on its wall side, and the rest of the floor one quad.
        let strip = expected_shaded_quad(UP, [5.0, 6.0, 6.0], [9.0, 6.0, 7.0], |[_, _, z]| {
            occluded(GRANITE, if z == 6.0 { 1 } else { 3 })
        });
        let rest = expected_shaded_quad(UP, [5.0, 6.0, 7.0], [9.0, 6.0, 9.0], |_| GRANITE);
        let mut expected = vec![strip, rest];
        expected.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(shaded_quads_in(&world, UP, 6.0), expected);
    }

    #[test]
    fn two_walls_meeting_over_a_corner_make_it_the_darkest() {
        // A Granite floor Cell at (5, 5, 5), and Dirt walls on two sides of the Cell above it:
        // at (4, 6, 5) on −x and (5, 6, 4) on −z. The Cell between them, (4, 6, 4), is open.
        let world = world(&[
            (CellPos::new(5, 5, 5), "Granite"),
            (CellPos::new(4, 6, 5), "Dirt"),
            (CellPos::new(5, 6, 4), "Dirt"),
        ]);

        // The floor's top spans (5, 6, 5) to (6, 6, 6). Its corner at (5, 6, 5) has a wall on
        // either side: occlusion 0, though the Cell between the walls is open. The corners at
        // (6, 6, 5) and (5, 6, 6) each have one wall beside them: 2. The far corner has none: 3.
        let expected = expected_shaded_quad(UP, [5.0, 6.0, 5.0], [6.0, 6.0, 6.0], |[x, _, z]| {
            let level = match (x == 5.0, z == 5.0) {
                (true, true) => 0,
                (true, false) | (false, true) => 2,
                (false, false) => 3,
            };
            occluded(GRANITE, level)
        });
        assert_eq!(shaded_quads_in(&world, UP, 6.0), vec![expected]);
    }

    #[test]
    fn the_darkest_corner_of_a_face_lies_in_only_one_of_its_triangles() {
        // The floor and walls of the test above. The floor's top is one quad, whose corner at
        // (5, 6, 5) is the darkest.
        let world = world(&[
            (CellPos::new(5, 5, 5), "Granite"),
            (CellPos::new(4, 6, 5), "Dirt"),
            (CellPos::new(5, 6, 4), "Dirt"),
        ]);
        let geometry = mesh_solids(
            &origin_neighbourhood(&world),
            &SolidColors::new(&elements()),
        );
        let corner = (0..geometry.positions.len())
            .find(|&i| geometry.positions[i] == [5.0, 6.0, 5.0] && geometry.normals[i] == UP)
            .expect("the floor's corner") as u32;

        // Split along the other diagonal, the darkness stays in that corner's triangle, rather
        // than reaching across the face to the far corner.
        let triangles = geometry
            .indices
            .chunks(3)
            .filter(|triangle| triangle.contains(&corner))
            .count();
        assert_eq!(triangles, 1);
    }

    #[test]
    fn a_face_at_the_chunk_edge_is_darkened_by_a_cell_in_a_diagonal_neighbour() {
        // A Granite Cell at (31, 31, 10), in the origin Chunk's top +x edge, and another at
        // (32, 32, 10), which lies in the Chunk at (1, 1, 0): beside the first Cell's top face,
        // in neither the +x nor the +y neighbour.
        let world = world(&[
            (CellPos::new(31, 31, 10), "Granite"),
            (CellPos::new(32, 32, 10), "Granite"),
        ]);

        // The top spans (31, 32, 10) to (32, 32, 11). Both corners on x = 32 have the other
        // Cell beside them: occlusion 2.
        let expected =
            expected_shaded_quad(UP, [31.0, 32.0, 10.0], [32.0, 32.0, 11.0], |[x, ..]| {
                occluded(GRANITE, if x == 32.0 { 2 } else { 3 })
            });
        assert_eq!(shaded_quads_in(&world, UP, 32.0), vec![expected]);
    }

    #[test]
    fn liquids_and_gases_darken_nothing() {
        // A Granite floor Cell at (5, 5, 5), with Water and Oxygen beside the Cell above it.
        let world = world(&[
            (CellPos::new(5, 5, 5), "Granite"),
            (CellPos::new(4, 6, 5), "Water"),
            (CellPos::new(5, 6, 4), "Oxygen"),
        ]);

        let expected = expected_quads(
            GRANITE,
            &[([0.0, 1.0, 0.0], [5.0, 6.0, 5.0], [6.0, 6.0, 6.0])],
        );
        let top: Vec<_> = quads(&world)
            .into_iter()
            .filter(|quad| quad.normal == UP)
            .collect();
        assert_eq!(top, expected);
    }

    #[test]
    fn a_chunk_without_solids_has_no_geometry() {
        // Water and Oxygen in the Chunk, and Granite next door.
        let world = world(&[
            (CellPos::new(5, 6, 7), "Water"),
            (CellPos::new(6, 6, 7), "Oxygen"),
            (CellPos::new(32, 6, 7), "Granite"),
        ]);
        let colors = SolidColors::new(&elements());

        assert!(mesh_solids(&origin_neighbourhood(&world), &colors).is_empty());
    }
}
