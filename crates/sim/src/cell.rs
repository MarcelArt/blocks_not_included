use crate::element::ElementId;

/// One 1 m cube of the world and the matter in it: exactly one Element, with a Mass and a temperature.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cell {
    pub element: ElementId,
    pub mass_grams: u32,
    pub temperature_k: f32,
}

impl Cell {
    pub const VACUUM: Cell = Cell {
        element: ElementId::VACUUM,
        mass_grams: 0,
        temperature_k: 0.0,
    };
}

/// A Cell's position in the world grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CellPos {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl CellPos {
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    pub(crate) const fn offset(self, (dx, dy, dz): (i32, i32, i32)) -> Self {
        Self::new(self.x + dx, self.y + dy, self.z + dz)
    }
}
