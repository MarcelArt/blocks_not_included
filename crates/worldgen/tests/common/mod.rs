//! The scenarios that the Tick benchmark (`benches/tick.rs`) measures and the determinism test
//! (`tests/determinism.rs`) checks. The benchmark includes this file by path.

// The benchmark and the test each use a different subset of these helpers.
#![allow(dead_code)]

use sim::{Cell, CellPos, ElementTable, Phase, SimSettings, Simulation};
use worldgen::Generator;

pub const SEED: u64 = 2026;

pub fn shipped_elements() -> ElementTable {
    ElementTable::from_ron(include_str!("../../../../assets/elements.ron"))
        .expect("the shipped Elements file is valid")
}

/// The settings both scenarios run with: the defaults, except heat conduction sped up as the game
/// runs it (`docs/plans/09-sim-bridge.md`), so hot spots stay hot spots rather than creeping.
pub fn scenario_settings() -> SimSettings {
    SimSettings {
        conduction_multiplier: 1_000.0,
        ..SimSettings::default()
    }
}

/// Where in each Chunk the busy scenario warms the ground or releases gas, from the Chunk's
/// origin.
const SPOTS: [(i32, i32, i32); 4] = [(6, 6, 6), (22, 10, 18), (12, 20, 26), (26, 26, 8)];

/// How much a hot spot warms the ground: enough to melt Granite (1500 K) anywhere near the surface.
const HOT_SPOT_KELVIN: f32 = 1_500.0;

/// The Mass of each Cell of a gas release: about 14 atmospheres of Oxygen's 1,430 g.
const RELEASE_GRAMS: u32 = 20_000;

/// A freshly generated, fully active region around the origin, with a hot spot or a gas release
/// at a few spots in every Chunk, so nearly every Chunk stays awake.
///
/// At each spot, a 3 × 3 × 3 block of ground is warmed by [`HOT_SPOT_KELVIN`], and a 3 × 3 × 3
/// block of Atmosphere is replaced by Carbon Dioxide or Hydrogen at [`RELEASE_GRAMS`] a Cell.
/// Liquid spots are left alone.
pub fn busy(elements: &ElementTable) -> Simulation {
    let generator = Generator::new(SEED, elements).expect("the shipped Elements are enough");
    let mut sim = Simulation::with_source(elements.clone(), Box::new(generator));
    sim.set_settings(scenario_settings())
        .expect("valid settings");
    let gases = [
        elements.id_of("Carbon Dioxide").unwrap(),
        elements.id_of("Hydrogen").unwrap(),
    ];
    let chunks: Vec<_> = sim.active_region().chunks().collect();
    for (n, chunk) in chunks.into_iter().enumerate() {
        let origin = chunk.origin();
        for (i, (x, y, z)) in SPOTS.into_iter().enumerate() {
            let spot = CellPos::new(origin.x + x, origin.y + y, origin.z + z);
            let phase = elements.get(sim.get(spot).element).phase;
            for pos in block_around(spot) {
                let cell = sim.get(pos);
                match (phase, elements.get(cell.element).phase) {
                    (Phase::Solid, Phase::Solid) => sim.apply(sim::Edit::ShiftTemperature {
                        pos,
                        kelvin: HOT_SPOT_KELVIN,
                    }),
                    (Phase::Gas, Phase::Gas) => sim.apply(sim::Edit::Place {
                        pos,
                        cell: Cell {
                            element: gases[(n + i) % gases.len()],
                            mass_grams: RELEASE_GRAMS,
                            temperature_k: 350.0,
                        },
                    }),
                    _ => {}
                }
            }
        }
    }
    sim
}

/// The 3 × 3 × 3 block of Cells centred on `centre`.
fn block_around(centre: CellPos) -> impl Iterator<Item = CellPos> {
    (-1..=1).flat_map(move |dx| {
        (-1..=1).flat_map(move |dy| {
            (-1..=1).map(move |dz| CellPos::new(centre.x + dx, centre.y + dy, centre.z + dz))
        })
    })
}

/// How many Ticks the settled scenario runs before it is measured.
pub const SETTLE_TICKS: usize = 300;

/// The busy scenario's region as generated, without hot spots or gas releases, after
/// [`SETTLE_TICKS`] Ticks: as near to rest as it gets in a practical time.
///
/// It never comes entirely to rest. Water draining from pockets levels out across cave floors,
/// and gas evens out through the caves, a few grams at a time, so most Chunks stay awake: about
/// 24 of the 32 after 300 Ticks. In a 4,000-Tick run at default settings, about 20 still were.
pub fn settled(elements: &ElementTable) -> Simulation {
    let generator = Generator::new(SEED, elements).expect("the shipped Elements are enough");
    let mut sim = Simulation::with_source(elements.clone(), Box::new(generator));
    sim.set_settings(scenario_settings())
        .expect("valid settings");
    for _ in 0..SETTLE_TICKS {
        sim.tick();
    }
    sim
}

/// A full Active Region that is entirely at rest: high in the Atmosphere, where every Cell is
/// Oxygen at the same Mass and temperature, after one Tick. Nothing in it changes, so every
/// Chunk is Settled, and a Tick costs only what Settled Chunks cost.
pub fn at_rest(elements: &ElementTable) -> Simulation {
    let generator = Generator::new(SEED, elements).expect("the shipped Elements are enough");
    let mut sim = Simulation::with_source(elements.clone(), Box::new(generator));
    sim.set_settings(scenario_settings())
        .expect("valid settings");
    // The surface stays within 40 Cells of y = 0, so the region around y = 160 is all sky.
    sim.focus_on(CellPos::new(0, 160, 0));
    sim.tick();
    sim
}
