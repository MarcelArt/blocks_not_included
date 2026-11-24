//! What the meshers' tests share: a small Element table, worlds built Cell by Cell, and a way to
//! read a [`ChunkGeometry`] back as quads.

use std::collections::HashMap;
use std::sync::Arc;

use sim::{CHUNK_SIZE, Cell, CellPos, ChunkPos, Element, ElementTable, Phase};

use super::mesher::{ChunkGeometry, ChunkNeighbourhood};
use crate::world::locate;

/// Vacuum 0, Granite 1, Dirt 2, Water 3, Oxygen 4, Steam 5, Magma 6.
pub fn elements() -> ElementTable {
    let mut elements = ElementTable::new();
    elements.add(Element::new("Granite").color([0.55, 0.5, 0.5]));
    elements.add(Element::new("Dirt").color([0.45, 0.3, 0.18]));
    elements.add(
        Element::new("Water")
            .phase(Phase::Liquid)
            .color([0.15, 0.35, 0.8])
            .density(1_000.0),
    );
    elements.add(
        Element::new("Oxygen")
            .phase(Phase::Gas)
            .color([0.75, 0.85, 1.0])
            .density(1.43),
    );
    elements.add(
        Element::new("Steam")
            .phase(Phase::Gas)
            .color([0.95, 0.95, 0.95])
            .density(0.6)
            .tint(0.15),
    );
    elements.add(
        Element::new("Magma")
            .phase(Phase::Liquid)
            .color([1.0, 0.35, 0.05])
            .density(2_600.0),
    );
    elements
}

pub type World = HashMap<ChunkPos, Arc<[Cell]>>;

/// A world holding nothing but the Cells listed, each of the named Element and 1 kg. Every Chunk
/// with a listed Cell is loaded; all others are missing.
pub fn world(cells: &[(CellPos, &str)]) -> World {
    let cells: Vec<_> = cells
        .iter()
        .map(|&(pos, name)| (pos, name, 1_000))
        .collect();
    world_with_mass(&cells)
}

/// A world holding nothing but the Cells listed, each of the named Element and Mass in grams.
/// Every Chunk with a listed Cell is loaded; all others are missing.
pub fn world_with_mass(cells: &[(CellPos, &str, u32)]) -> World {
    let elements = elements();
    let mut chunks: HashMap<ChunkPos, Vec<Cell>> = HashMap::new();
    for &(pos, name, mass_grams) in cells {
        let (chunk, index) = locate(pos);
        let element = elements.id_of(name).expect("a test Element");
        chunks
            .entry(chunk)
            .or_insert_with(|| vec![Cell::VACUUM; CHUNK_SIZE.pow(3)])[index] = Cell {
            element,
            mass_grams,
            temperature_k: 293.15,
        };
    }
    chunks
        .into_iter()
        .map(|(pos, cells)| (pos, Arc::from(cells)))
        .collect()
}

/// The Chunk at the origin, with its neighbours as they are in `world`. The origin Chunk is all
/// Vacuum if `world` lacks it.
pub fn origin_neighbourhood(world: &World) -> ChunkNeighbourhood {
    let origin = ChunkPos::new(0, 0, 0);
    ChunkNeighbourhood::gather(origin, |pos| {
        world
            .get(&pos)
            .cloned()
            .or_else(|| (pos == origin).then(|| Arc::from(vec![Cell::VACUUM; CHUNK_SIZE.pow(3)])))
    })
    .expect("the origin Chunk")
}

/// One quad of a [`ChunkGeometry`]: the box its corners span, which is flat along its normal.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct Quad {
    pub normal: [f32; 3],
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub color: [f32; 4],
}

/// The quads of `geometry`, sorted. Checks, for every quad, that its four corners share one
/// normal and colour, that it is flat along that normal, and that both its triangles wind
/// counter-clockwise seen from where the normal points, as Bevy needs to show them.
pub fn quads_of(geometry: &ChunkGeometry) -> Vec<Quad> {
    let vertices = geometry.positions.len();
    assert_eq!(vertices % 4, 0);
    assert_eq!(geometry.normals.len(), vertices);
    assert_eq!(geometry.colors.len(), vertices);
    assert_eq!(geometry.indices.len(), vertices / 4 * 6);

    let vec = |p: [f32; 3]| bevy::math::Vec3::from_array(p);
    for triangle in geometry.indices.chunks(3) {
        let [a, b, c] = [0, 1, 2].map(|i| triangle[i] as usize);
        assert_eq!(
            [a / 4, b / 4],
            [c / 4, c / 4],
            "a triangle stays within its quad"
        );
        let [pa, pb, pc] = [a, b, c].map(|i| vec(geometry.positions[i]));
        let facing = (pb - pa).cross(pc - pa).normalize();
        assert_eq!(
            facing,
            vec(geometry.normals[a]),
            "triangle {triangle:?} winds wrong"
        );
    }

    let mut quads: Vec<Quad> = (0..vertices / 4)
        .map(|quad| {
            let corners = quad * 4..quad * 4 + 4;
            let normal = geometry.normals[quad * 4];
            let color = geometry.colors[quad * 4];
            assert!(
                geometry.normals[corners.clone()]
                    .iter()
                    .all(|&n| n == normal)
            );
            assert!(geometry.colors[corners.clone()].iter().all(|&c| c == color));
            let positions = &geometry.positions[corners];
            let min = [0, 1, 2].map(|axis| {
                positions
                    .iter()
                    .map(|p| p[axis])
                    .fold(f32::INFINITY, f32::min)
            });
            let max = [0, 1, 2].map(|axis| {
                positions
                    .iter()
                    .map(|p| p[axis])
                    .fold(f32::NEG_INFINITY, f32::max)
            });
            let flat_axis = normal.iter().position(|&n| n != 0.0).unwrap();
            assert_eq!(
                min[flat_axis], max[flat_axis],
                "quad is flat along its normal"
            );
            Quad {
                normal,
                min,
                max,
                color,
            }
        })
        .collect();
    quads.sort_by(|a, b| a.partial_cmp(b).unwrap());
    quads
}

/// The quads `expected` lists as `(normal, min, max)`, all in `color`, sorted as [`quads_of`]
/// sorts them.
pub fn expected_quads(color: [f32; 4], expected: &[([f32; 3], [f32; 3], [f32; 3])]) -> Vec<Quad> {
    let mut quads: Vec<Quad> = expected
        .iter()
        .map(|&(normal, min, max)| Quad {
            normal,
            min,
            max,
            color,
        })
        .collect();
    quads.sort_by(|a, b| a.partial_cmp(b).unwrap());
    quads
}
