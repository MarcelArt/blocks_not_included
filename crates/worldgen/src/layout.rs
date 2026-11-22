//! The Sim Sandbox world's layout: where the surface lies, and what fills the world above and
//! below it. The values come from the design session (`docs/plans/07-world-generation.md`).

use fastnoise_lite::{FastNoiseLite, FractalType, NoiseType};
use sim::{CHUNK_SIZE, CellPos, WORLD_BOTTOM_Y};

/// The y just above the floor: the world's bottom Chunk layer, which is all Magma.
pub(crate) const FLOOR_TOP_Y: i32 = WORLD_BOTTOM_Y + CHUNK_SIZE as i32;

/// The temperature of the Atmosphere.
pub(crate) const ATMOSPHERE_TEMPERATURE_K: f32 = 293.0;

/// The temperature of the ground at the surface, from which it rises with depth.
const SURFACE_TEMPERATURE_K: f32 = 288.0;

/// How much warmer the ground gets per Cell of depth below the surface: about 288 K at the
/// surface, and about 1500 K at the world's floor, 1024 Cells below y = 0.
///
/// Even under the lowest surface, 40 Cells below y = 0, the floor is at least 953 Cells deep, so
/// at least 288 + 1.2 × 953 ≈ 1432 K: too hot for its Magma to freeze, which happens below 1400 K.
const TEMPERATURE_GRADIENT_K_PER_CELL: f32 = 1.2;

/// The surface rolls around y = 0 by at most this many Cells up or down.
const SURFACE_AMPLITUDE_CELLS: f32 = 40.0;
/// Hills are a few hundred Cells across.
const SURFACE_FREQUENCY: f32 = 1.0 / 256.0;
const SURFACE_OCTAVES: i32 = 3;

/// The Dirt layer on top of the ground is this many Cells thick, at least and at most.
const DIRT_DEPTH_CELLS: (i32, i32) = (3, 5);
const DIRT_DEPTH_FREQUENCY: f32 = 1.0 / 32.0;

/// Caves are carved out where this 3D noise lies above [`CAVE_THRESHOLD`].
const CAVE_FREQUENCY: f32 = 1.0 / 48.0;
const CAVE_OCTAVES: i32 = 2;
const CAVE_THRESHOLD: f32 = 0.4;
/// Caves are this many times wider than they are tall, so they have floors to walk on.
const CAVE_FLATTENING: f64 = 2.0;
/// Within this many Cells of the surface, the threshold rises towards
/// [`CAVE_SURFACE_THRESHOLD`], so few caves break through to the Atmosphere.
const CAVE_FADE_DEPTH_CELLS: i32 = 16;
const CAVE_SURFACE_THRESHOLD: f32 = 0.8;

/// Pockets replace Granite where this 3D noise lies above [`POCKET_THRESHOLD`]: small blobs, a
/// few Cells across.
const POCKET_FREQUENCY: f32 = 1.0 / 16.0;
const POCKET_THRESHOLD: f32 = 0.7;
/// Which Element fills a pocket follows a slower noise, so each blob is mostly one Element:
/// Water below `-POCKET_KIND_SPLIT`, Hydrogen above `POCKET_KIND_SPLIT`, Carbon Dioxide between.
const POCKET_KIND_FREQUENCY: f32 = 1.0 / 64.0;
const POCKET_KIND_SPLIT: f32 = 0.2;

/// Seeds the noise fields; each field mixes in its own salt, so they don't line up.
mod salt {
    pub const SURFACE: u64 = 1;
    pub const DIRT_DEPTH: u64 = 2;
    pub const CAVES: u64 = 3;
    pub const POCKETS: u64 = 4;
    pub const POCKET_KINDS: u64 = 5;
}

/// What a pocket is filled with, before any Phase Transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pocket {
    Water,
    CarbonDioxide,
    Hydrogen,
}

/// What lies along one vertical column of Cells.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Column {
    /// The y of the column's top ground Cell. The Atmosphere starts just above it.
    pub surface_y: i32,
    /// How many Cells of Dirt lie on top of the Granite, from the surface down.
    pub dirt_depth: i32,
}

/// The noise fields that shape the world, all drawn from one seed.
pub(crate) struct Terrain {
    surface: FastNoiseLite,
    dirt_depth: FastNoiseLite,
    caves: FastNoiseLite,
    pockets: FastNoiseLite,
    pocket_kinds: FastNoiseLite,
}

impl Terrain {
    pub fn new(seed: u64) -> Self {
        let mut surface = noise(seed, salt::SURFACE, SURFACE_FREQUENCY);
        surface.set_fractal_type(Some(FractalType::FBm));
        surface.set_fractal_octaves(Some(SURFACE_OCTAVES));
        let mut caves = noise(seed, salt::CAVES, CAVE_FREQUENCY);
        caves.set_fractal_type(Some(FractalType::FBm));
        caves.set_fractal_octaves(Some(CAVE_OCTAVES));
        Self {
            surface,
            dirt_depth: noise(seed, salt::DIRT_DEPTH, DIRT_DEPTH_FREQUENCY),
            caves,
            pockets: noise(seed, salt::POCKETS, POCKET_FREQUENCY),
            pocket_kinds: noise(seed, salt::POCKET_KINDS, POCKET_KIND_FREQUENCY),
        }
    }

    /// The column of Cells at `x`, `z`.
    pub fn column(&self, x: i32, z: i32) -> Column {
        let (x, z) = (f64::from(x), f64::from(z));
        // The noise lies within -1..=1, so the surface stays within ±SURFACE_AMPLITUDE_CELLS.
        let surface_y = (self.surface.get_noise_2d(x, z) * SURFACE_AMPLITUDE_CELLS).round() as i32;
        let (thinnest, thickest) = DIRT_DEPTH_CELLS;
        let choices = thickest - thinnest + 1;
        // From 0 to 1, then one of `choices` equal steps.
        let share = (self.dirt_depth.get_noise_2d(x, z) + 1.0) / 2.0;
        let dirt_depth = thinnest + ((share * choices as f32) as i32).min(choices - 1);
        Column {
            surface_y,
            dirt_depth,
        }
    }

    /// Whether a cave is carved out at `pos`, `depth` Cells below the surface.
    pub fn is_cave(&self, pos: CellPos, depth: i32) -> bool {
        let threshold = if depth < CAVE_FADE_DEPTH_CELLS {
            // From CAVE_SURFACE_THRESHOLD at the surface down to CAVE_THRESHOLD.
            let nearness = 1.0 - depth as f32 / CAVE_FADE_DEPTH_CELLS as f32;
            CAVE_THRESHOLD + (CAVE_SURFACE_THRESHOLD - CAVE_THRESHOLD) * nearness
        } else {
            CAVE_THRESHOLD
        };
        let (x, y, z) = coordinates(pos);
        self.caves.get_noise_3d(x, y * CAVE_FLATTENING, z) > threshold
    }

    /// The pocket at `pos`, if Granite there gives way to one.
    pub fn pocket(&self, pos: CellPos) -> Option<Pocket> {
        let (x, y, z) = coordinates(pos);
        if self.pockets.get_noise_3d(x, y, z) <= POCKET_THRESHOLD {
            return None;
        }
        let kind = self.pocket_kinds.get_noise_3d(x, y, z);
        Some(if kind < -POCKET_KIND_SPLIT {
            Pocket::Water
        } else if kind > POCKET_KIND_SPLIT {
            Pocket::Hydrogen
        } else {
            Pocket::CarbonDioxide
        })
    }
}

fn coordinates(pos: CellPos) -> (f64, f64, f64) {
    (f64::from(pos.x), f64::from(pos.y), f64::from(pos.z))
}

/// The temperature of the ground `depth` Cells below the surface.
pub(crate) fn ground_temperature(depth: i32) -> f32 {
    SURFACE_TEMPERATURE_K + TEMPERATURE_GRADIENT_K_PER_CELL * depth as f32
}

/// An OpenSimplex2 noise field at `frequency`, seeded from the world seed and `salt`.
fn noise(seed: u64, salt: u64, frequency: f32) -> FastNoiseLite {
    let mut noise = FastNoiseLite::with_seed(field_seed(seed, salt));
    noise.set_noise_type(Some(NoiseType::OpenSimplex2));
    noise.set_frequency(Some(frequency));
    noise
}

/// The seed of one noise field: the world seed and the field's salt, mixed by SplitMix64's
/// finaliser so that nearby world seeds give unrelated fields.
fn field_seed(seed: u64, salt: u64) -> i32 {
    let mut z = seed.wrapping_add(salt.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    // Keep the low 32 bits.
    z as u32 as i32
}
