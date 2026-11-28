//! How full a liquid Cell is: one rule for the fluid meshes that draw its surface and for the
//! player who swims in it, so the surface the player sees is the one they meet.

use sim::Element;

/// The lowest a liquid's surface stands within its Cell, so that a thin film stays visible.
pub const MIN_SURFACE: f32 = 0.05;

/// The Mass of a full Cell of the liquid `element`, its Capacity, in grams: its Density × 1 m³,
/// and at least 1 g.
pub fn capacity_grams(element: &Element) -> f32 {
    // kg/m³ × 1 m³ × 1000 g/kg. Density is above 0 in a valid table.
    (element.density * 1_000.0).max(1.0)
}

/// How far up its Cell a liquid's surface stands, from [`MIN_SURFACE`] to 1: Mass / Capacity, or
/// all the way up under more of the same liquid.
#[inline]
pub fn liquid_fill(mass_grams: u32, capacity_grams: f32, same_liquid_above: bool) -> f32 {
    if same_liquid_above {
        1.0
    } else {
        (mass_grams as f32 / capacity_grams).clamp(MIN_SURFACE, 1.0)
    }
}
