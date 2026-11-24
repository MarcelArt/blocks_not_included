//! Turns a Chunk's solid Cells into a mesh, with `binary-greedy-meshing`.
//!
//! The mesher reads a padded cube of `(CHUNK_SIZE + 2)³` values: the Chunk's own Cells, plus the
//! layer of Cells that touches it from each face neighbour, so that a face against a solid
//! neighbour is left out. A value of 0 means "not solid"; any other value is the Cell's
//! [`ElementId::index`] + 1.

use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, Mesh, PrimitiveTopology};
use binary_greedy_meshing::{Face, Mesher, Quad};
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

/// The Chunks that share a face with the Chunk at `pos`.
pub(crate) fn face_neighbours(pos: ChunkPos) -> [ChunkPos; 6] {
    FACE_NEIGHBOURS.map(|(dx, dy, dz)| ChunkPos::new(pos.x + dx, pos.y + dy, pos.z + dz))
}

/// The colour each solid Element is drawn in, by [`ElementId::index`]. Every other Element is
/// `None`, so this also says which Cells are meshed here.
#[derive(Clone, Debug)]
pub struct SolidColors(Vec<Option<[f32; 4]>>);

impl SolidColors {
    pub fn new(elements: &ElementTable) -> Self {
        let colors = elements
            .iter()
            .map(|(_, element)| {
                let [r, g, b] = element.color;
                (element.phase == Phase::Solid).then_some([r, g, b, 1.0])
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

    /// The colour of the solid Element whose mesher value is `voxel`.
    fn of_voxel(&self, voxel: u64) -> [f32; 4] {
        usize::try_from(voxel - 1)
            .ok()
            .and_then(|index| self.0.get(index).copied().flatten())
            .expect("the mesher emits quads only for the values of solid Elements")
    }
}

/// What meshing a Chunk reads: its own Cells, and those of each face neighbour that is loaded.
pub struct ChunkNeighbourhood {
    cells: Arc<[Cell]>,
    /// In [`FACE_NEIGHBOURS`] order. `None` where the neighbour isn't loaded.
    neighbours: [Option<Arc<[Cell]>>; 6],
}

impl ChunkNeighbourhood {
    /// The Chunk at `pos` and its face neighbours, as `chunk` finds them. `None` if `chunk` lacks
    /// the Chunk at `pos` itself.
    pub fn gather(
        pos: ChunkPos,
        mut chunk: impl FnMut(ChunkPos) -> Option<Arc<[Cell]>>,
    ) -> Option<Self> {
        let cells = chunk(pos)?;
        let neighbours = face_neighbours(pos).map(chunk);
        Some(Self { cells, neighbours })
    }

    /// The Chunk's own Cells, in the order `sim` lists them.
    pub fn cells(&self) -> &[Cell] {
        &self.cells
    }

    /// The Cell at `[x, y, z]` relative to the Chunk's origin, which may lie one Cell outside the
    /// Chunk along one axis, in a face neighbour. `None` if that neighbour isn't loaded, and for a
    /// position in an edge or corner neighbour, which isn't gathered.
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
        let offset = (step(x), step(y), step(z));
        let neighbour = FACE_NEIGHBOURS.iter().position(|&o| o == offset)?;
        let cells = self.neighbours[neighbour].as_ref()?;
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
}

/// Meshes the solid Cells of a Chunk. Faces between two solid Cells are left out, and so are
/// faces against a solid Cell of a neighbour Chunk; a missing neighbour counts as not solid.
pub fn mesh_solids(neighbourhood: &ChunkNeighbourhood, colors: &SolidColors) -> ChunkGeometry {
    let solid = |cell: &Cell| colors.voxel(cell.element) != 0;
    if !neighbourhood.cells.iter().any(solid) {
        // Such as a Chunk high in the Atmosphere: about half of render distance.
        return ChunkGeometry::default();
    }
    let padded = PaddedChunk::new(neighbourhood, colors);
    let mut mesher = Mesher::<CHUNK_SIZE>::new();
    // Every solid is opaque, so no value is transparent.
    let transparent_columns = vec![0; PADDED.pow(2)];
    mesher.fast_mesh(&padded.voxels, &padded.solid_columns, &transparent_columns);
    ChunkGeometry::from_quads(&mesher.quads, colors)
}

impl ChunkGeometry {
    /// Whether there is nothing to draw.
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    /// A mesh for the GPU alone: the main world drops its copy once uploaded.
    pub fn into_mesh(self) -> Mesh {
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, self.colors)
        .with_inserted_indices(Indices::U32(self.indices))
    }

    /// `quads` lists the mesher's quads for each face direction, in [`Face`] order.
    fn from_quads(quads: &[Vec<Quad>; 6], colors: &SolidColors) -> Self {
        let count = quads.iter().map(Vec::len).sum();
        let mut geometry = Self {
            positions: Vec::with_capacity(count * 4),
            normals: Vec::with_capacity(count * 4),
            colors: Vec::with_capacity(count * 4),
            indices: binary_greedy_meshing::indices(count),
        };
        for (face, quads) in (0..6).map(Face::from).zip(quads) {
            let normal = face.n().map(|n| n as f32);
            for &quad in quads {
                let color = colors.of_voxel(quad.voxel_id());
                for vertex in face.vertices_packed(quad) {
                    geometry.positions.push(vertex.xyz().map(|c| c as f32));
                    geometry.normals.push(normal);
                    geometry.colors.push(color);
                }
            }
        }
        geometry
    }
}

/// The mesher's input for one Chunk.
struct PaddedChunk {
    /// One value per padded Cell, in the mesher's order: z fastest, then x, then y
    /// (`binary_greedy_meshing::pad_linearize`). That is not the order `sim` lists Cells in.
    voxels: Box<[u16]>,
    /// One `u64` per (x, y) column of `voxels`, with bit z set where that value isn't 0.
    solid_columns: Box<[u64]>,
}

impl PaddedChunk {
    fn new(neighbourhood: &ChunkNeighbourhood, colors: &SolidColors) -> Self {
        let mut padded = Self {
            voxels: vec![0; PADDED.pow(3)].into(),
            solid_columns: vec![0; PADDED.pow(2)].into(),
        };
        for (index, cell) in neighbourhood.cells.iter().enumerate() {
            let [x, y, z] = chunk_local(index);
            padded.set([x + 1, y + 1, z + 1], colors.voxel(cell.element));
        }
        for (&offset, neighbour) in FACE_NEIGHBOURS.iter().zip(&neighbourhood.neighbours) {
            let Some(cells) = neighbour else {
                // A missing neighbour counts as not solid: its layer stays 0.
                continue;
            };
            let face = BorderLayer::facing(offset);
            for a in 0..CHUNK_SIZE {
                for b in 0..CHUNK_SIZE {
                    let (local, padded_pos) = face.cell(a, b);
                    let cell = cells[chunk_index(local)];
                    padded.set(padded_pos, colors.voxel(cell.element));
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
            self.solid_columns[column] |= 1 << z;
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
        World, elements, expected_quads, origin_neighbourhood, quads_of, world,
    };

    fn padded(world: &World) -> PaddedChunk {
        PaddedChunk::new(&origin_neighbourhood(world), &SolidColors::new(&elements()))
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
        assert!(padded.solid_columns.iter().all(|&column| column == 0));
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
