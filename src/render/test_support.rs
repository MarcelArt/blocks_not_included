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
    let cells: Vec<_> = cells
        .iter()
        .map(|&(pos, name, mass_grams)| (pos, name, mass_grams, 293.15))
        .collect();
    world_of(&cells)
}

/// A world holding nothing but the Cells listed, each of the named Element, 1 kg and the
/// temperature in kelvin. Every Chunk with a listed Cell is loaded; all others are missing.
pub fn world_with_temperature(cells: &[(CellPos, &str, f32)]) -> World {
    let cells: Vec<_> = cells
        .iter()
        .map(|&(pos, name, temperature_k)| (pos, name, 1_000, temperature_k))
        .collect();
    world_of(&cells)
}

/// A world holding nothing but the Cells listed, as `(position, Element, Mass in grams,
/// temperature in kelvin)`. Every Chunk with a listed Cell is loaded; all others are missing.
fn world_of(cells: &[(CellPos, &str, u32, f32)]) -> World {
    let elements = elements();
    let mut chunks: HashMap<ChunkPos, Vec<Cell>> = HashMap::new();
    for &(pos, name, mass_grams, temperature_k) in cells {
        let (chunk, index) = locate(pos);
        let element = elements.id_of(name).expect("a test Element");
        chunks
            .entry(chunk)
            .or_insert_with(|| vec![Cell::VACUUM; CHUNK_SIZE.pow(3)])[index] = Cell {
            element,
            mass_grams,
            temperature_k,
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

/// One quad of a [`ChunkGeometry`] whose corners may differ in colour: the box its corners span,
/// and each corner's position and colour, sorted by position.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct ShadedQuad {
    pub normal: [f32; 3],
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub corners: [([f32; 3], [f32; 4]); 4],
}

/// The quads of `geometry`, sorted, each in a single colour. Checks everything
/// [`shaded_quads`] does, and that every quad's four corners share one colour.
pub fn quads_of(geometry: &ChunkGeometry) -> Vec<Quad> {
    let mut quads: Vec<Quad> = shaded_quads(geometry)
        .into_iter()
        .map(|quad| {
            let color = quad.corners[0].1;
            assert!(
                quad.corners.iter().all(|&(_, c)| c == color),
                "{quad:?} is shaded"
            );
            Quad {
                normal: quad.normal,
                min: quad.min,
                max: quad.max,
                color,
            }
        })
        .collect();
    quads.sort_by(|a, b| a.partial_cmp(b).unwrap());
    quads
}

/// The quads of `geometry`, sorted. Checks, for every quad, that its four corners share one
/// normal, that it is flat along that normal, and that both its triangles wind
/// counter-clockwise seen from where the normal points, as Bevy needs to show them.
pub fn shaded_quads(geometry: &ChunkGeometry) -> Vec<ShadedQuad> {
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

    let mut quads: Vec<ShadedQuad> = (0..vertices / 4)
        .map(|quad| {
            let first = quad * 4;
            let normal = geometry.normals[first];
            assert!(
                geometry.normals[first..first + 4]
                    .iter()
                    .all(|&n| n == normal)
            );
            let mut corners: [([f32; 3], [f32; 4]); 4] = std::array::from_fn(|i| {
                (geometry.positions[first + i], geometry.colors[first + i])
            });
            corners.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let min = [0, 1, 2].map(|axis| {
                corners
                    .iter()
                    .map(|(p, _)| p[axis])
                    .fold(f32::INFINITY, f32::min)
            });
            let max = [0, 1, 2].map(|axis| {
                corners
                    .iter()
                    .map(|(p, _)| p[axis])
                    .fold(f32::NEG_INFINITY, f32::max)
            });
            let flat_axis = normal.iter().position(|&n| n != 0.0).unwrap();
            assert_eq!(
                min[flat_axis], max[flat_axis],
                "quad is flat along its normal"
            );
            ShadedQuad {
                normal,
                min,
                max,
                corners,
            }
        })
        .collect();
    quads.sort_by(|a, b| a.partial_cmp(b).unwrap());
    quads
}

/// The quad with `normal` spanning `min` to `max`, which is flat along the normal, with each
/// corner in the colour `color` gives its position.
pub fn expected_shaded_quad(
    normal: [f32; 3],
    min: [f32; 3],
    max: [f32; 3],
    color: impl Fn([f32; 3]) -> [f32; 4],
) -> ShadedQuad {
    let flat_axis = normal.iter().position(|&n| n != 0.0).unwrap();
    let [a, b] = [(flat_axis + 1) % 3, (flat_axis + 2) % 3];
    let mut corners =
        [(false, false), (true, false), (false, true), (true, true)].map(|(a_max, b_max)| {
            let mut corner = min;
            corner[a] = if a_max { max[a] } else { min[a] };
            corner[b] = if b_max { max[b] } else { min[b] };
            (corner, color(corner))
        });
    corners.sort_by(|a, b| a.partial_cmp(b).unwrap());
    ShadedQuad {
        normal,
        min,
        max,
        corners,
    }
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
