//! Turns a Chunk's liquid Cells, and its gas Cells whose Element has a tint, into translucent
//! meshes.
//!
//! Each liquid Cell is drawn as a box whose top shows how full it is: it reaches Mass / Capacity
//! of the way up the Cell, kept between [`MIN_SURFACE`] and 1, or all the way under more of the
//! same liquid. A face is left out where the box meets more of the same liquid or a solid, except
//! for the part of a side that rises above a lower neighbour's surface. A tinted gas Cell shows a
//! face wherever it borders a different Element that isn't solid.
//!
//! Heights differ from Cell to Cell, so the faces aren't greedy-merged: each is one Cell wide.

use sim::{Cell, ElementId, ElementTable, Phase};

use super::mesher::{ChunkGeometry, ChunkNeighbourhood, chunk_local};

/// How opaque a liquid is drawn.
pub const LIQUID_ALPHA: f32 = 0.6;

/// The lowest a liquid's surface is drawn within its Cell, so that a thin film stays visible.
pub const MIN_SURFACE: f32 = 0.05;

/// How a Cell of each Element takes part in drawing fluids.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Look {
    /// Vacuum, and every gas without a tint: nothing is drawn, and a fluid beside it shows its
    /// face.
    Clear,
    /// A fluid shows no face against it, which would be hidden anyway.
    Solid,
    Liquid {
        /// Linear RGBA, at [`LIQUID_ALPHA`].
        color: [f32; 4],
        /// Mass of a full Cell: Density × 1 m³.
        capacity_grams: f32,
    },
    TintedGas {
        /// Linear RGBA, with the tint as alpha.
        color: [f32; 4],
    },
}

/// How each Element is drawn as a fluid, by [`ElementId::index`].
#[derive(Clone, Debug)]
pub struct FluidLooks {
    looks: Vec<Look>,
    /// Whether a Cell of each Element is drawn here: a cheap first look at every Cell of a Chunk,
    /// most of which aren't.
    drawn: Vec<bool>,
}

impl FluidLooks {
    pub fn new(elements: &ElementTable) -> Self {
        let looks = elements
            .iter()
            .map(|(_, element)| {
                let [r, g, b] = element.color;
                match element.phase {
                    Phase::Solid => Look::Solid,
                    Phase::Liquid => Look::Liquid {
                        color: [r, g, b, LIQUID_ALPHA],
                        // kg/m³ × 1 m³ × 1000 g/kg. Density is above 0 in a valid table.
                        capacity_grams: (element.density * 1_000.0).max(1.0),
                    },
                    Phase::Gas if element.tint > 0.0 => Look::TintedGas {
                        color: [r, g, b, element.tint],
                    },
                    Phase::Gas | Phase::Vacuum => Look::Clear,
                }
            })
            .collect::<Vec<_>>();
        let drawn = looks
            .iter()
            .map(|look| matches!(look, Look::Liquid { .. } | Look::TintedGas { .. }))
            .collect();
        Self { looks, drawn }
    }

    fn of(&self, element: ElementId) -> Look {
        self.looks
            .get(usize::from(element.index()))
            .copied()
            .unwrap_or(Look::Clear)
    }

    fn is_drawn(&self, element: ElementId) -> bool {
        self.drawn
            .get(usize::from(element.index()))
            .is_some_and(|&drawn| drawn)
    }
}

/// A Chunk's translucent geometry: its liquids, and its tinted gases. They are drawn with
/// different materials.
#[derive(Debug, Default)]
pub struct FluidGeometry {
    pub liquid: ChunkGeometry,
    pub gas: ChunkGeometry,
}

/// Meshes the liquid and tinted gas Cells of a Chunk. A Cell the neighbourhood doesn't hold counts
/// as Vacuum: one in a missing neighbour Chunk, or diagonally past an edge of the Chunk, such as
/// the Cell above a liquid neighbour across the Chunk's top edge.
pub fn mesh_fluids(neighbourhood: &ChunkNeighbourhood, looks: &FluidLooks) -> FluidGeometry {
    let mut geometry = FluidGeometry::default();
    let cells = neighbourhood.cells();
    // Most Chunks hold no fluid that is drawn.
    let Some(first) = cells.iter().position(|cell| looks.is_drawn(cell.element)) else {
        return geometry;
    };
    for (index, &cell) in cells.iter().enumerate().skip(first) {
        if !looks.is_drawn(cell.element) {
            continue;
        }
        let look = looks.of(cell.element);
        let around = Around {
            neighbourhood,
            looks,
            pos: chunk_local(index).map(|c| c as i32),
            cell,
        };
        match look {
            Look::Liquid {
                color,
                capacity_grams,
            } => around.mesh_liquid(color, capacity_grams, &mut geometry.liquid),
            Look::TintedGas { color } => around.mesh_gas(color, &mut geometry.gas),
            Look::Clear | Look::Solid => {}
        }
    }
    geometry
}

/// One fluid Cell of the Chunk being meshed, and what surrounds it.
struct Around<'a> {
    neighbourhood: &'a ChunkNeighbourhood,
    looks: &'a FluidLooks,
    /// Relative to the Chunk's origin.
    pos: [i32; 3],
    cell: Cell,
}

impl Around<'_> {
    /// The box of liquid in this Cell: its faces, except where it meets the same liquid or a
    /// solid.
    fn mesh_liquid(&self, color: [f32; 4], capacity_grams: f32, geometry: &mut ChunkGeometry) {
        let height = self.surface(self.pos, capacity_grams);
        let above = self.next(Facing::UP);
        // A full Cell's top lies flush against a solid above it.
        if !self.is_same(above) && !(self.is_solid(above) && height >= 1.0) {
            self.push_face(geometry, Facing::UP, 0.0, height, color);
        }
        let below = self.next(Facing::DOWN);
        if !self.is_same(below) && !self.is_solid(below) {
            self.push_face(geometry, Facing::DOWN, 0.0, height, color);
        }
        for facing in Facing::AROUND {
            let next = self.next(facing);
            let low = if self.is_solid(next) {
                continue;
            } else if self.is_same(next) {
                // Only the part of the side above the neighbour's surface shows.
                self.surface(facing.step(self.pos), capacity_grams)
            } else {
                0.0
            };
            if low < height {
                self.push_face(geometry, facing, low, height, color);
            }
        }
    }

    /// A face on each side of this Cell that borders a different Element that isn't solid.
    fn mesh_gas(&self, color: [f32; 4], geometry: &mut ChunkGeometry) {
        for facing in Facing::ALL {
            let next = self.next(facing);
            if !self.is_same(next) && !self.is_solid(next) {
                self.push_face(geometry, facing, 0.0, 1.0, color);
            }
        }
    }

    /// How high the liquid at `pos`, this Cell or a neighbour holding the same liquid, is drawn
    /// within its Cell, from 0 to 1.
    fn surface(&self, pos: [i32; 3], capacity_grams: f32) -> f32 {
        if self.is_same(self.neighbourhood.cell(Facing::UP.step(pos))) {
            return 1.0;
        }
        let mass_grams = self
            .neighbourhood
            .cell(pos)
            .map_or(0, |cell| cell.mass_grams);
        (mass_grams as f32 / capacity_grams).clamp(MIN_SURFACE, 1.0)
    }

    /// The neighbouring Cell `facing` points to, if it is known.
    fn next(&self, facing: Facing) -> Option<Cell> {
        self.neighbourhood.cell(facing.step(self.pos))
    }

    /// Whether `other` holds this Cell's Element. An unknown Cell counts as Vacuum.
    fn is_same(&self, other: Option<Cell>) -> bool {
        other.is_some_and(|other| other.element == self.cell.element)
    }

    /// Whether `other` is solid. An unknown Cell counts as Vacuum.
    fn is_solid(&self, other: Option<Cell>) -> bool {
        other.is_some_and(|other| self.looks.of(other.element) == Look::Solid)
    }

    /// Adds the face `facing` of the box that fills this Cell from `low` to `high` up its height,
    /// wound counter-clockwise seen from outside.
    fn push_face(
        &self,
        geometry: &mut ChunkGeometry,
        facing: Facing,
        low: f32,
        high: f32,
        color: [f32; 4],
    ) {
        let [x, y, z] = self.pos.map(|c| c as f32);
        let min = [x, y + low, z];
        let max = [x + 1.0, y + high, z + 1.0];
        // The face lies across `axis`; `u` and `v` follow it in turn, so u × v points along
        // +axis, and corners ordered (0, 0), (1, 0), (1, 1), (0, 1) in (u, v) wind
        // counter-clockwise seen from +axis. The other way round suits a face towards −axis.
        let Facing { axis, positive } = facing;
        let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
        let corner = |along_u: bool, along_v: bool| {
            let mut corner = [0.0; 3];
            corner[axis] = if positive { max[axis] } else { min[axis] };
            corner[u] = if along_u { max[u] } else { min[u] };
            corner[v] = if along_v { max[v] } else { min[v] };
            corner
        };
        let corners = if positive {
            [(false, false), (true, false), (true, true), (false, true)]
        } else {
            [(false, false), (false, true), (true, true), (true, false)]
        };
        let first = u32::try_from(geometry.positions.len()).expect("fewer than 2³² vertices");
        let mut normal = [0.0; 3];
        normal[axis] = if positive { 1.0 } else { -1.0 };
        for (along_u, along_v) in corners {
            geometry.positions.push(corner(along_u, along_v));
            geometry.normals.push(normal);
            geometry.colors.push(color);
        }
        geometry
            .indices
            .extend([0, 1, 2, 0, 2, 3].map(|corner| first + corner));
    }
}

/// Which way a face of a Cell looks: across `axis` (0 for x, 1 for y, 2 for z), towards its
/// positive or negative end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Facing {
    axis: usize,
    positive: bool,
}

impl Facing {
    const UP: Self = Self::new(1, true);
    const DOWN: Self = Self::new(1, false);
    /// The four sides.
    const AROUND: [Self; 4] = [
        Self::new(0, true),
        Self::new(0, false),
        Self::new(2, true),
        Self::new(2, false),
    ];
    const ALL: [Self; 6] = [
        Self::UP,
        Self::DOWN,
        Self::AROUND[0],
        Self::AROUND[1],
        Self::AROUND[2],
        Self::AROUND[3],
    ];

    const fn new(axis: usize, positive: bool) -> Self {
        Self { axis, positive }
    }

    /// The position one Cell from `pos` this way.
    fn step(self, mut pos: [i32; 3]) -> [i32; 3] {
        pos[self.axis] += if self.positive { 1 } else { -1 };
        pos
    }
}

#[cfg(test)]
mod tests {
    use sim::CellPos;

    use super::*;
    use crate::render::test_support::{
        Quad, World, elements, expected_quads, origin_neighbourhood, quads_of, world,
        world_with_mass,
    };

    /// Water's colour at [`LIQUID_ALPHA`].
    const WATER: [f32; 4] = [0.15, 0.35, 0.8, 0.6];
    /// Steam's colour at its tint, 0.15.
    const STEAM: [f32; 4] = [0.95, 0.95, 0.95, 0.15];

    const UP: [f32; 3] = [0.0, 1.0, 0.0];
    const DOWN: [f32; 3] = [0.0, -1.0, 0.0];
    const EAST: [f32; 3] = [1.0, 0.0, 0.0];
    const WEST: [f32; 3] = [-1.0, 0.0, 0.0];
    const SOUTH: [f32; 3] = [0.0, 0.0, 1.0];
    const NORTH: [f32; 3] = [0.0, 0.0, -1.0];

    fn geometry(world: &World) -> FluidGeometry {
        mesh_fluids(&origin_neighbourhood(world), &FluidLooks::new(&elements()))
    }

    fn liquid_quads(world: &World) -> Vec<Quad> {
        quads_of(&geometry(world).liquid)
    }

    fn gas_quads(world: &World) -> Vec<Quad> {
        quads_of(&geometry(world).gas)
    }

    /// The four side faces of the Cell whose lowest corner is `(x, y, z)`, from `y + low` to
    /// `y + high`, as `(normal, min, max)`.
    fn sides([x, y, z]: [f32; 3], low: f32, high: f32) -> [([f32; 3], [f32; 3], [f32; 3]); 4] {
        let (bottom, top) = (y + low, y + high);
        [
            (EAST, [x + 1.0, bottom, z], [x + 1.0, top, z + 1.0]),
            (WEST, [x, bottom, z], [x, top, z + 1.0]),
            (SOUTH, [x, bottom, z + 1.0], [x + 1.0, top, z + 1.0]),
            (NORTH, [x, bottom, z], [x + 1.0, top, z]),
        ]
    }

    /// The top face of the Cell whose lowest corner is `(x, y, z)`, at `y + height`.
    fn top([x, y, z]: [f32; 3], height: f32) -> ([f32; 3], [f32; 3], [f32; 3]) {
        (UP, [x, y + height, z], [x + 1.0, y + height, z + 1.0])
    }

    #[test]
    fn a_half_full_liquid_cell_has_its_top_face_at_half_height() {
        // 500 kg of Water is half its 1,000 kg Capacity, standing on Granite.
        let world = world_with_mass(&[
            (CellPos::new(5, 6, 7), "Water", 500_000),
            (CellPos::new(5, 5, 7), "Granite", 2_700_000),
        ]);

        // The top lies at y + 0.5 = 6.5, and each side reaches up to it. Nothing faces the
        // Granite below.
        let cell = [5.0, 6.0, 7.0];
        let mut expected = vec![top(cell, 0.5)];
        expected.extend(sides(cell, 0.0, 0.5));
        assert_eq!(liquid_quads(&world), expected_quads(WATER, &expected));
    }

    #[test]
    fn a_liquid_cell_under_the_same_liquid_has_no_top_face() {
        // 900 kg of Water under 1,000 kg more, on Granite.
        let world = world_with_mass(&[
            (CellPos::new(5, 6, 7), "Water", 900_000),
            (CellPos::new(5, 7, 7), "Water", 1_000_000),
            (CellPos::new(5, 5, 7), "Granite", 2_700_000),
        ]);

        // The lower Cell is drawn to its full height, though it holds 0.9 of its Capacity, so
        // its sides meet the upper Cell's at y = 7. Only the upper Cell has a top, at y = 8.
        let mut expected = vec![top([5.0, 7.0, 7.0], 1.0)];
        expected.extend(sides([5.0, 6.0, 7.0], 0.0, 1.0));
        expected.extend(sides([5.0, 7.0, 7.0], 0.0, 1.0));
        assert_eq!(liquid_quads(&world), expected_quads(WATER, &expected));
    }

    #[test]
    fn two_neighbouring_cells_of_the_same_liquid_share_no_side_face() {
        // Two half-full Cells of Water side by side along x, on Granite.
        let world = world_with_mass(&[
            (CellPos::new(5, 6, 7), "Water", 500_000),
            (CellPos::new(6, 6, 7), "Water", 500_000),
            (CellPos::new(5, 5, 7), "Granite", 2_700_000),
            (CellPos::new(6, 5, 7), "Granite", 2_700_000),
        ]);

        // Nothing lies on x = 6 between them. Each Cell has its own top, at y = 6.5.
        let (west, east) = ([5.0, 6.0, 7.0], [6.0, 6.0, 7.0]);
        let [_, west_side, west_south, west_north] = sides(west, 0.0, 0.5);
        let [east_side, _, east_south, east_north] = sides(east, 0.0, 0.5);
        let expected = [
            top(west, 0.5),
            top(east, 0.5),
            west_side,
            west_south,
            west_north,
            east_side,
            east_south,
            east_north,
        ];
        assert_eq!(liquid_quads(&world), expected_quads(WATER, &expected));
    }

    #[test]
    fn a_liquid_cell_next_to_a_solid_has_no_side_face_on_that_side() {
        // A full Cell of Water on Granite, with Granite to its +x side.
        let world = world_with_mass(&[
            (CellPos::new(5, 6, 7), "Water", 1_000_000),
            (CellPos::new(5, 5, 7), "Granite", 2_700_000),
            (CellPos::new(6, 6, 7), "Granite", 2_700_000),
        ]);

        let cell = [5.0, 6.0, 7.0];
        let [_, west, south, north] = sides(cell, 0.0, 1.0);
        let expected = [top(cell, 1.0), west, south, north];
        assert_eq!(liquid_quads(&world), expected_quads(WATER, &expected));
    }

    #[test]
    fn gas_without_a_tint_produces_no_geometry() {
        // Oxygen has no tint. It borders Vacuum, Granite and Steam here.
        let world = world(&[
            (CellPos::new(5, 6, 7), "Oxygen"),
            (CellPos::new(6, 6, 7), "Oxygen"),
            (CellPos::new(5, 5, 7), "Granite"),
            (CellPos::new(31, 6, 7), "Oxygen"),
        ]);

        let geometry = geometry(&world);

        assert!(geometry.liquid.is_empty());
        assert!(geometry.gas.is_empty());
    }

    #[test]
    fn a_liquid_surface_is_drawn_between_5_percent_and_full_height() {
        // 1 g of Water, a thin film, and 1,200 kg, which a Phase Transition can leave over
        // Capacity. Both stand on Granite, apart.
        let world = world_with_mass(&[
            (CellPos::new(5, 6, 7), "Water", 1),
            (CellPos::new(5, 5, 7), "Granite", 2_700_000),
            (CellPos::new(9, 6, 7), "Water", 1_200_000),
            (CellPos::new(9, 5, 7), "Granite", 2_700_000),
        ]);

        // 1 g / 1,000,000 g is far below 0.05, so the film shows at y + 0.05 = 6.05;
        // 1,200,000 g / 1,000,000 g = 1.2 is clamped to 1, at y = 7.
        let (film, full) = ([5.0, 6.0, 7.0], [9.0, 6.0, 7.0]);
        let mut expected = vec![top(film, 0.05), top(full, 1.0)];
        expected.extend(sides(film, 0.0, 0.05));
        expected.extend(sides(full, 0.0, 1.0));
        assert_eq!(liquid_quads(&world), expected_quads(WATER, &expected));
    }

    #[test]
    fn a_step_down_to_the_same_liquid_shows_a_face_above_the_lower_surface() {
        // Spreading Water: a full Cell beside one holding a quarter of its Capacity, on Granite.
        let world = world_with_mass(&[
            (CellPos::new(5, 6, 7), "Water", 1_000_000),
            (CellPos::new(6, 6, 7), "Water", 250_000),
            (CellPos::new(5, 5, 7), "Granite", 2_700_000),
            (CellPos::new(6, 5, 7), "Granite", 2_700_000),
        ]);

        // Between them, at x = 6, the full Cell shows the part of its side above the lower
        // surface: from y + 0.25 = 6.25 up to 7. The lower Cell shows nothing on that side.
        let (high, low) = ([5.0, 6.0, 7.0], [6.0, 6.0, 7.0]);
        let [_, high_west, high_south, high_north] = sides(high, 0.0, 1.0);
        let [step, ..] = sides(high, 0.25, 1.0);
        let [low_east, _, low_south, low_north] = sides(low, 0.0, 0.25);
        let expected = [
            top(high, 1.0),
            top(low, 0.25),
            step,
            high_west,
            high_south,
            high_north,
            low_east,
            low_south,
            low_north,
        ];
        assert_eq!(liquid_quads(&world), expected_quads(WATER, &expected));
    }

    #[test]
    fn a_full_liquid_cell_under_a_solid_has_no_top_face() {
        // Water filling a Cell between a Granite floor and ceiling, and half-filling another.
        let world = world_with_mass(&[
            (CellPos::new(5, 6, 7), "Water", 1_000_000),
            (CellPos::new(5, 5, 7), "Granite", 2_700_000),
            (CellPos::new(5, 7, 7), "Granite", 2_700_000),
            (CellPos::new(9, 6, 7), "Water", 500_000),
            (CellPos::new(9, 5, 7), "Granite", 2_700_000),
            (CellPos::new(9, 7, 7), "Granite", 2_700_000),
        ]);

        // The full Cell's top would lie flush against the ceiling's underside. The half-full
        // Cell's surface, at y + 0.5, is clear of it and shows.
        let (full, half) = ([5.0, 6.0, 7.0], [9.0, 6.0, 7.0]);
        let mut expected = vec![top(half, 0.5)];
        expected.extend(sides(full, 0.0, 1.0));
        expected.extend(sides(half, 0.0, 0.5));
        assert_eq!(liquid_quads(&world), expected_quads(WATER, &expected));
    }

    #[test]
    fn a_liquid_cell_over_gas_shows_its_bottom_face() {
        // A full Cell of Water held above Oxygen, as in a falling stream.
        let world = world_with_mass(&[
            (CellPos::new(5, 6, 7), "Water", 1_000_000),
            (CellPos::new(5, 5, 7), "Oxygen", 1_430),
        ]);

        let cell = [5.0, 6.0, 7.0];
        let mut expected = vec![top(cell, 1.0), (DOWN, [5.0, 6.0, 7.0], [6.0, 6.0, 8.0])];
        expected.extend(sides(cell, 0.0, 1.0));
        assert_eq!(liquid_quads(&world), expected_quads(WATER, &expected));
    }

    #[test]
    fn a_tinted_gas_shows_faces_where_it_borders_a_different_element_that_is_not_solid() {
        // Two Cells of Steam side by side along x, over Granite, with Oxygen to the -z side of
        // the first and Vacuum everywhere else.
        let world = world(&[
            (CellPos::new(5, 6, 7), "Steam"),
            (CellPos::new(6, 6, 7), "Steam"),
            (CellPos::new(5, 5, 7), "Granite"),
            (CellPos::new(6, 5, 7), "Granite"),
            (CellPos::new(5, 6, 6), "Oxygen"),
        ]);

        // No face between the two Cells of Steam, nor against the Granite. Every face is a
        // whole Cell's side.
        let (west, east) = ([5.0, 6.0, 7.0], [6.0, 6.0, 7.0]);
        let [_, west_side, west_south, west_north] = sides(west, 0.0, 1.0);
        let [east_side, _, east_south, east_north] = sides(east, 0.0, 1.0);
        let expected = [
            top(west, 1.0),
            top(east, 1.0),
            west_side,
            west_south,
            west_north,
            east_side,
            east_south,
            east_north,
        ];
        assert_eq!(gas_quads(&world), expected_quads(STEAM, &expected));
        assert!(geometry(&world).liquid.is_empty());
    }

    #[test]
    fn faces_at_the_chunk_border_follow_the_neighbour_chunk() {
        // Full Water in the origin Chunk's +x, +y corner column, at (31, 31, 7). The +x
        // neighbour holds more Water beside it, and the +y neighbour more Water above it.
        let corner = (CellPos::new(31, 31, 7), "Water", 1_000_000);
        let floor = (CellPos::new(31, 30, 7), "Granite", 2_700_000);
        let cell = [31.0, 31.0, 7.0];
        let [east, west, south, north] = sides(cell, 0.0, 1.0);

        let covered = world_with_mass(&[
            corner,
            floor,
            (CellPos::new(32, 31, 7), "Water", 1_000_000),
            (CellPos::new(31, 32, 7), "Water", 1_000_000),
        ]);
        assert_eq!(
            liquid_quads(&covered),
            expected_quads(WATER, &[west, south, north]),
        );

        // Neither neighbour is loaded, so both count as Vacuum.
        let alone = world_with_mass(&[corner, floor]);
        assert_eq!(
            liquid_quads(&alone),
            expected_quads(WATER, &[top(cell, 1.0), east, west, south, north]),
        );
    }
}
