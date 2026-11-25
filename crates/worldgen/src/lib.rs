//! World generation for the Sim Sandbox: one Chunk at a time, deterministic per seed.
//!
//! Rolling hills around y = 0 under an Oxygen Atmosphere; a few Cells of Dirt over Granite that
//! warms with depth, riddled with Oxygen-filled caves and small pockets of Water, Carbon Dioxide
//! and Hydrogen; and a floor of Magma. The values are in `layout.rs`.
//!
//! Every Cell comes out already past any Phase Transition its temperature calls for
//! (`transitions.rs`), and full: Density × 1 m³, which for a gas is roughly one atmosphere.
//!
//! Vocabulary is defined in `CONTEXT.md` at the repository root.

mod layout;
mod transitions;

use std::fmt;

use sim::{CHUNK_SIZE, Cell, CellPos, ChunkPos, ChunkSource, ElementId, ElementTable};

use layout::{ATMOSPHERE_TEMPERATURE_K, Column, FLOOR_TOP_Y, Pocket, Terrain, ground_temperature};
use transitions::Transitions;

/// The revision of the generation rules: which Cells a seed and a table of Elements generate.
///
/// A World save stores only the Chunks that differ from generated terrain and regenerates the
/// rest from its World Seed (ADR 0005), so a save can be loaded only under the rules it was made
/// with. Bump this whenever the same seed and Elements would generate different Cells; the test
/// `the_generated_terrain_is_pinned_to_the_rules_revision` fails until you do. Changes that
/// generate the same Cells, such as speeding the generator up, keep saves compatible.
pub const RULES_REVISION: u32 = 1;

/// The Elements the layout uses, by name.
const OXYGEN: &str = "Oxygen";
const DIRT: &str = "Dirt";
const GRANITE: &str = "Granite";
const MAGMA: &str = "Magma";
const WATER: &str = "Water";
const CARBON_DIOXIDE: &str = "Carbon Dioxide";
const HYDROGEN: &str = "Hydrogen";

/// Generates the Sim Sandbox world one Chunk at a time.
///
/// A Chunk's Cells depend only on the seed and the Chunk's position: the generator holds no
/// mutable state, so the order Chunks are generated in makes no difference.
pub struct Generator {
    terrain: Terrain,
    oxygen: ElementId,
    dirt: ElementId,
    granite: ElementId,
    magma: ElementId,
    water: ElementId,
    carbon_dioxide: ElementId,
    hydrogen: ElementId,
    transitions: Transitions,
    /// The Mass of a full Cell of each Element, indexed by [`ElementId::index`].
    full_mass_grams: Vec<u32>,
}

impl Generator {
    /// A generator for the world with `seed`, made of Elements from `elements`.
    ///
    /// Fails if `elements` lacks any Element the layout uses, naming every one it lacks.
    pub fn new(seed: u64, elements: &ElementTable) -> Result<Generator, MissingElements> {
        let mut missing = Vec::new();
        let mut find = |name: &'static str| {
            elements.id_of(name).unwrap_or_else(|| {
                missing.push(name);
                ElementId::VACUUM
            })
        };
        let generator = Generator {
            terrain: Terrain::new(seed),
            oxygen: find(OXYGEN),
            dirt: find(DIRT),
            granite: find(GRANITE),
            magma: find(MAGMA),
            water: find(WATER),
            carbon_dioxide: find(CARBON_DIOXIDE),
            hydrogen: find(HYDROGEN),
            transitions: Transitions::new(elements),
            full_mass_grams: elements
                .iter()
                .map(|(_, element)| full_mass_grams(element.density))
                .collect(),
        };
        if missing.is_empty() {
            Ok(generator)
        } else {
            Err(MissingElements { names: missing })
        }
    }

    /// The Cell at `pos`, in `column`: the Element the layout intends for it, after any Phase
    /// Transitions its temperature calls for, and full.
    fn cell(&self, pos: CellPos, column: Column) -> Cell {
        let (intended, temperature_k) = self.intended(pos, column);
        let element = self.transitions.follow(intended, temperature_k);
        Cell {
            element,
            mass_grams: self.full_mass_grams[usize::from(element.index())],
            temperature_k,
        }
    }

    /// The Element the layout intends for the Cell at `pos`, in `column`, and its temperature.
    ///
    /// Above the surface is the Atmosphere. Below it, the floor comes first, then caves, then
    /// the Dirt layer, and pockets only replace the Granite under that.
    fn intended(&self, pos: CellPos, column: Column) -> (ElementId, f32) {
        let depth = column.surface_y - pos.y;
        if depth < 0 {
            return (self.oxygen, ATMOSPHERE_TEMPERATURE_K);
        }
        let element = if pos.y < FLOOR_TOP_Y {
            self.magma
        } else if self.terrain.is_cave(pos, depth) {
            self.oxygen
        } else if depth < column.dirt_depth {
            self.dirt
        } else {
            match self.terrain.pocket(pos) {
                Some(Pocket::Water) => self.water,
                Some(Pocket::CarbonDioxide) => self.carbon_dioxide,
                Some(Pocket::Hydrogen) => self.hydrogen,
                None => self.granite,
            }
        };
        (element, ground_temperature(depth))
    }
}

impl ChunkSource for Generator {
    fn generate(&self, chunk: ChunkPos) -> Vec<Cell> {
        let size = CHUNK_SIZE as i32;
        let origin = chunk.origin();
        // In the order Cells are listed within a layer: x fastest, then z.
        let columns: Vec<(i32, i32, Column)> = (origin.z..origin.z + size)
            .flat_map(|z| (origin.x..origin.x + size).map(move |x| (x, z)))
            .map(|(x, z)| (x, z, self.terrain.column(x, z)))
            .collect();
        let mut cells = Vec::with_capacity(columns.len() * CHUNK_SIZE);
        for y in origin.y..origin.y + size {
            for &(x, z, column) in &columns {
                cells.push(self.cell(CellPos::new(x, y, z), column));
            }
        }
        cells
    }
}

/// The Mass of a full Cell of an Element with `density`: Density × 1 m³, and at least 1 g.
fn full_mass_grams(density: f32) -> u32 {
    // kg/m³ × 1 m³ × 1000 g/kg, rounded the way the simulation rounds a liquid's Capacity.
    ((f64::from(density) * 1_000.0).round() as u32).max(1)
}

/// Why a [`Generator`] could not be built: the Element table lacks Elements the layout uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissingElements {
    /// The name of every Element the table lacks.
    pub names: Vec<&'static str>,
}

impl fmt::Display for MissingElements {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "world generation needs Elements the table lacks: ")?;
        for (i, name) in self.names.iter().enumerate() {
            let separator = if i == 0 { "" } else { ", " };
            write!(f, "{separator}{name:?}")?;
        }
        Ok(())
    }
}

impl std::error::Error for MissingElements {}
