//! Helpers shared by the flow tests: sealed boxes, test Elements and the simulation invariants.

// Each test file uses a different subset of these helpers.
#![allow(dead_code)]

use std::collections::HashMap;

use sim::{Cell, CellPos, Element, ElementId, Phase, Simulation};

pub fn cell(element: ElementId, mass_grams: u32, temperature_k: f32) -> Cell {
    Cell {
        element,
        mass_grams,
        temperature_k,
    }
}

/// A solid for building sealed boxes. It conducts no heat, so it never changes.
pub fn test_wall() -> Element {
    Element::new("TestWall").density(2_000.0).specific_heat(1.0)
}

/// A gas that conducts no heat, so its temperature changes only when Mass moves.
pub fn test_gas(name: &str, density: f32) -> Element {
    Element::new(name)
        .phase(Phase::Gas)
        .density(density)
        .specific_heat(1.0)
}

/// A liquid that conducts no heat, so its temperature changes only when Mass moves. A full Cell
/// holds `density` × 1 m³, so 1,000,000 g at Density 1000.
pub fn test_liquid(name: &str, density: f32) -> Element {
    Element::new(name)
        .phase(Phase::Liquid)
        .density(density)
        .specific_heat(1.0)
}

/// The Cells from `min` to `max` inclusive, surrounded on every side by a one-Cell-thick wall.
pub struct SealedBox {
    pub min: CellPos,
    pub max: CellPos,
}

impl SealedBox {
    pub fn build(sim: &mut Simulation, wall: ElementId, min: CellPos, max: CellPos) -> Self {
        let sealed = SealedBox { min, max };
        for pos in sealed.walls() {
            sim.set(pos, cell(wall, 1_000, 300.0));
        }
        sealed
    }

    pub fn inside(&self) -> Vec<CellPos> {
        cuboid(self.min, self.max)
    }

    pub fn walls(&self) -> Vec<CellPos> {
        let (min, max) = (self.min, self.max);
        cuboid(
            CellPos::new(min.x - 1, min.y - 1, min.z - 1),
            CellPos::new(max.x + 1, max.y + 1, max.z + 1),
        )
        .into_iter()
        .filter(|pos| !contains(min, max, *pos))
        .collect()
    }

    /// The walls and everything inside them.
    pub fn all(&self) -> Vec<CellPos> {
        let mut cells = self.inside();
        cells.extend(self.walls());
        cells
    }
}

pub fn cuboid(min: CellPos, max: CellPos) -> Vec<CellPos> {
    let mut cells = Vec::new();
    for x in min.x..=max.x {
        for y in min.y..=max.y {
            for z in min.z..=max.z {
                cells.push(CellPos::new(x, y, z));
            }
        }
    }
    cells
}

pub fn contains(min: CellPos, max: CellPos, pos: CellPos) -> bool {
    (min.x..=max.x).contains(&pos.x)
        && (min.y..=max.y).contains(&pos.y)
        && (min.z..=max.z).contains(&pos.z)
}

/// The Mass of each Element over `cells`.
pub fn grams_by_element(sim: &Simulation, cells: &[CellPos]) -> HashMap<ElementId, u64> {
    let mut grams = HashMap::new();
    for &pos in cells {
        let cell = sim.get(pos);
        *grams.entry(cell.element).or_default() += u64::from(cell.mass_grams);
    }
    grams.retain(|_, grams| *grams > 0);
    grams
}

/// The simulation invariants over every Cell a test set, recorded before the first Tick.
pub struct Invariants {
    cells: Vec<CellPos>,
    total_grams: u64,
    grams_by_element: HashMap<ElementId, u64>,
    /// The most grams one Cell of each listed liquid may hold.
    capacities: Vec<(ElementId, u32)>,
}

impl Invariants {
    pub fn record(sim: &Simulation, cells: Vec<CellPos>) -> Self {
        Self {
            total_grams: sim.total_mass_grams(),
            grams_by_element: grams_by_element(sim, &cells),
            cells,
            capacities: Vec::new(),
        }
    }

    /// Also checks that no Cell of `liquid` ever holds more than `grams`.
    pub fn with_capacity(mut self, liquid: ElementId, grams: u32) -> Self {
        self.capacities.push((liquid, grams));
        self
    }

    pub fn check(&self, sim: &Simulation, tick: usize) {
        for &pos in &self.cells {
            let cell = sim.get(pos);
            if cell.element == ElementId::VACUUM {
                assert_eq!(
                    cell.mass_grams, 0,
                    "tick {tick}: Vacuum at {pos:?} holds Mass"
                );
            } else {
                assert!(
                    cell.mass_grams >= 1,
                    "tick {tick}: {pos:?} holds {cell:?} with no Mass"
                );
            }
            for &(liquid, capacity) in &self.capacities {
                assert!(
                    cell.element != liquid || cell.mass_grams <= capacity,
                    "tick {tick}: {pos:?} holds {cell:?}, over its capacity of {capacity} g"
                );
            }
        }
        assert_eq!(
            sim.total_mass_grams(),
            self.total_grams,
            "tick {tick}: total Mass changed"
        );
        assert_eq!(
            grams_by_element(sim, &self.cells),
            self.grams_by_element,
            "tick {tick}: Mass per Element changed"
        );
    }
}

/// Runs `ticks` Ticks, checking the invariants after each one.
pub fn run_ticks(sim: &mut Simulation, ticks: usize, invariants: &Invariants) {
    for tick in 0..ticks {
        sim.tick();
        invariants.check(sim, tick);
    }
}

/// Runs `ticks` more Ticks, checking after each one that no Cell in `cells` changes at all.
pub fn assert_at_rest(
    sim: &mut Simulation,
    ticks: usize,
    cells: &[CellPos],
    invariants: &Invariants,
) {
    let rest: Vec<Cell> = cells.iter().map(|&pos| sim.get(pos)).collect();
    for tick in 0..ticks {
        run_ticks(sim, 1, invariants);
        let now: Vec<Cell> = cells.iter().map(|&pos| sim.get(pos)).collect();
        assert_eq!(
            now, rest,
            "tick {tick} after coming to rest: something moved"
        );
    }
}
