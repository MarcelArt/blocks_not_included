//! Turns a Chunk's Cells into an Overlay's translucent mesh.
//!
//! Every Cell but Vacuum gets a value: its Element in the Element Overlay, and its bucket in the
//! Temperature and Mass Overlays. Faces are greedy-merged, and show only where the value changes,
//! so each run of Cells sharing a value shows as one layer. Where it changes, both Cells show a
//! face, each facing the other; drawn with back faces culled, the player sees only the one facing
//! them, so each layer shows from outside in its own colour.

use sim::{Cell, ElementTable, Phase};

use super::Overlay;
use super::ramp::{
    Rgb, mass_bucket, mass_bucket_color, temperature_bucket, temperature_bucket_color,
};
use crate::render::{ChunkGeometry, ChunkNeighbourhood, Faces, mesh_greedy};
use crate::tools::full_mass_grams;

/// How opaque an Overlay's faces are drawn.
pub const OVERLAY_ALPHA: f32 = 0.55;

/// How one Overlay draws Cells of each Element.
#[derive(Clone, Debug)]
pub struct OverlayLooks {
    overlay: Overlay,
    /// By [`ElementId::index`](sim::ElementId::index). `None` for Vacuum, which isn't drawn.
    elements: Vec<Option<ElementLook>>,
}

#[derive(Clone, Copy, Debug)]
struct ElementLook {
    color: Rgb,
    /// Mass of a full Cell: Density × 1 m³.
    full_grams: u32,
}

impl OverlayLooks {
    pub fn new(overlay: Overlay, elements: &ElementTable) -> Self {
        let elements = elements
            .iter()
            .map(|(_, element)| {
                (element.phase != Phase::Vacuum).then(|| ElementLook {
                    color: element.color,
                    full_grams: full_mass_grams(element),
                })
            })
            .collect();
        Self { overlay, elements }
    }

    /// The mesher's value for `cell`: 0 for Vacuum, which isn't drawn, and otherwise 1 + its
    /// Element's index or its bucket.
    fn voxel(&self, cell: Cell) -> u16 {
        let index = cell.element.index();
        let Some(Some(look)) = self.elements.get(usize::from(index)) else {
            return 0;
        };
        // The table holds fewer than `u16::MAX` Elements, so this doesn't overflow.
        1 + match self.overlay {
            Overlay::Element => index,
            Overlay::Temperature => temperature_bucket(cell.temperature_k),
            Overlay::Mass => mass_bucket(cell.mass_grams, look.full_grams),
        }
    }

    /// The colour faces of the mesher's value `voxel` are drawn in, at [`OVERLAY_ALPHA`].
    fn color(&self, voxel: u16) -> [f32; 4] {
        let value = voxel - 1;
        let [r, g, b] = match self.overlay {
            Overlay::Element => {
                self.elements[usize::from(value)]
                    .expect("only Elements other than Vacuum have values")
                    .color
            }
            Overlay::Temperature => temperature_bucket_color(value),
            Overlay::Mass => mass_bucket_color(value),
        };
        [r, g, b, OVERLAY_ALPHA]
    }
}

/// Meshes the Cells of a Chunk that aren't Vacuum, as `looks` draws them. A face shows against
/// Vacuum, and against a missing neighbour Chunk.
pub fn mesh_overlay(neighbourhood: &ChunkNeighbourhood, looks: &OverlayLooks) -> ChunkGeometry {
    mesh_greedy(
        neighbourhood,
        |cell| looks.voxel(cell),
        Faces::Translucent,
        |voxel| looks.color(voxel),
    )
}

#[cfg(test)]
mod tests {
    use sim::CellPos;

    use super::*;
    use crate::render::test_support::{
        Quad, World, elements, expected_quads, origin_neighbourhood, quads_of, world,
        world_with_mass, world_with_temperature,
    };

    fn quads(world: &World, overlay: Overlay) -> Vec<Quad> {
        let looks = OverlayLooks::new(overlay, &elements());
        quads_of(&mesh_overlay(&origin_neighbourhood(world), &looks))
    }

    fn with_alpha([r, g, b]: Rgb) -> [f32; 4] {
        [r, g, b, OVERLAY_ALPHA]
    }

    /// The six faces of the box from `min` to `max`, as `(normal, min, max)`.
    fn box_faces(min: [f32; 3], max: [f32; 3]) -> [([f32; 3], [f32; 3], [f32; 3]); 6] {
        let [x0, y0, z0] = min;
        let [x1, y1, z1] = max;
        [
            ([1.0, 0.0, 0.0], [x1, y0, z0], [x1, y1, z1]),
            ([-1.0, 0.0, 0.0], [x0, y0, z0], [x0, y1, z1]),
            ([0.0, 1.0, 0.0], [x0, y1, z0], [x1, y1, z1]),
            ([0.0, -1.0, 0.0], [x0, y0, z0], [x1, y0, z1]),
            ([0.0, 0.0, 1.0], [x0, y0, z1], [x1, y1, z1]),
            ([0.0, 0.0, -1.0], [x0, y0, z0], [x1, y1, z0]),
        ]
    }

    /// Oxygen's colour, in the test Element table.
    const OXYGEN: Rgb = [0.75, 0.85, 1.0];

    #[test]
    fn the_element_overlay_draws_gases_in_their_elements_colour() {
        let world = world(&[(CellPos::new(5, 6, 7), "Oxygen")]);

        let expected = expected_quads(
            with_alpha(OXYGEN),
            &box_faces([5.0, 6.0, 7.0], [6.0, 7.0, 8.0]),
        );
        assert_eq!(quads(&world, Overlay::Element), expected);
    }

    #[test]
    fn vacuum_is_not_drawn_in_any_overlay() {
        // The origin Chunk is all Vacuum, beside a Chunk of nothing but one Oxygen Cell.
        let world = world(&[(CellPos::new(32, 6, 7), "Oxygen")]);
        for overlay in Overlay::ALL {
            assert_eq!(quads(&world, overlay), [], "{overlay:?}");
        }
    }

    #[test]
    fn cells_in_one_bucket_share_no_face() {
        // Granite at 293 K and at 299 K: both in the bucket from 293 K to 299.25 K, the seventh,
        // whose colour is green.
        let world = world_with_temperature(&[
            (CellPos::new(5, 6, 7), "Granite", 293.0),
            (CellPos::new(6, 6, 7), "Granite", 299.0),
        ]);
        assert_eq!(temperature_bucket(293.0), 6);
        assert_eq!(temperature_bucket(299.0), 6);

        // One box spanning both Cells, from (5, 6, 7) to (7, 7, 8).
        let expected = expected_quads(
            with_alpha(temperature_bucket_color(6)),
            &box_faces([5.0, 6.0, 7.0], [7.0, 7.0, 8.0]),
        );
        assert_eq!(quads(&world, Overlay::Temperature), expected);
    }

    #[test]
    fn where_the_bucket_changes_each_cell_shows_a_face_towards_the_other() {
        // Granite at 293 K beside Granite at 373 K, which is in a hotter bucket.
        let world = world_with_temperature(&[
            (CellPos::new(5, 6, 7), "Granite", 293.0),
            (CellPos::new(6, 6, 7), "Granite", 373.0),
        ]);
        let (cool, hot) = (temperature_bucket(293.0), temperature_bucket(373.0));
        assert_ne!(cool, hot);

        // Each Cell is a box of its own, so both show a face on x = 6: the cool one's facing +x,
        // and the hot one's facing −x.
        let mut expected = expected_quads(
            with_alpha(temperature_bucket_color(cool)),
            &box_faces([5.0, 6.0, 7.0], [6.0, 7.0, 8.0]),
        );
        expected.extend(expected_quads(
            with_alpha(temperature_bucket_color(hot)),
            &box_faces([6.0, 6.0, 7.0], [7.0, 7.0, 8.0]),
        ));
        expected.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(quads(&world, Overlay::Temperature), expected);
    }

    #[test]
    fn the_mass_overlay_sorts_cells_by_how_full_they_are_whatever_their_element() {
        // Full Magma beside full Water, and half-full Water beside that. A full Cell holds
        // 2,600 kg of Magma, and 1,000 kg of Water.
        let world = world_with_mass(&[
            (CellPos::new(5, 6, 7), "Magma", 2_600_000),
            (CellPos::new(6, 6, 7), "Water", 1_000_000),
            (CellPos::new(7, 6, 7), "Water", 500_000),
        ]);
        let (full, half) = (
            mass_bucket(1_000_000, 1_000_000),
            mass_bucket(500_000, 1_000_000),
        );
        assert_eq!(mass_bucket(2_600_000, 2_600_000), full);

        // The two full Cells make one box, from (5, 6, 7) to (7, 7, 8).
        let mut expected = expected_quads(
            with_alpha(mass_bucket_color(full)),
            &box_faces([5.0, 6.0, 7.0], [7.0, 7.0, 8.0]),
        );
        expected.extend(expected_quads(
            with_alpha(mass_bucket_color(half)),
            &box_faces([7.0, 6.0, 7.0], [8.0, 7.0, 8.0]),
        ));
        expected.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(quads(&world, Overlay::Mass), expected);
    }

    #[test]
    fn a_face_at_the_chunk_border_shows_only_where_the_neighbour_chunk_differs() {
        // Oxygen in the origin Chunk's +x corner, at (31, 6, 7), against more Oxygen across the
        // border, and against nothing where that neighbour is missing.
        let inside = (CellPos::new(31, 6, 7), "Oxygen");
        let faces = box_faces([31.0, 6.0, 7.0], [32.0, 7.0, 8.0]);
        let [east, rest @ ..] = faces;

        let covered = world(&[inside, (CellPos::new(32, 6, 7), "Oxygen")]);
        assert_eq!(
            quads(&covered, Overlay::Element),
            expected_quads(with_alpha(OXYGEN), &rest)
        );

        let alone = world(&[inside]);
        let mut all = rest.to_vec();
        all.push(east);
        assert_eq!(
            quads(&alone, Overlay::Element),
            expected_quads(with_alpha(OXYGEN), &all)
        );
    }
}
