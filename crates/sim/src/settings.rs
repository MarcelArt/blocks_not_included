//! Tuning values for the simulation, loaded from a settings file by the game.

use std::fmt;

use serde::Deserialize;

/// Tuning values for the simulation, passed to
/// [`Simulation::set_settings`](crate::Simulation::set_settings).
///
/// A settings file may leave out any value, which then keeps its default. Every value must lie in
/// the range its documentation gives; [`validate`](Self::validate) checks them.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SimSettings {
    /// The global speed-up for heat conduction. With real values, heat takes days to cross one
    /// Cell.
    pub conduction_multiplier: f32,
    /// After a Tick, a Chunk is Settled if no Cell in it changed Element or Mass and no
    /// temperature in it changed by more than this many kelvin.
    pub settle_threshold_k: f32,
    /// Whether Settled Chunks skip work. Turning this off simulates every Chunk every Tick.
    pub settled_chunks_enabled: bool,
    /// The fraction of the Mass difference that a lone pair of touching gas Cells evens out per
    /// Tick. Above 0 and at most 0.5, which evens out a lone pair in one Tick; any more would
    /// overshoot.
    pub gas_evening_out_fraction: f64,
    /// The fraction of the Mass difference that a lone pair of sideways liquid neighbours evens
    /// out per Tick. Above 0 and at most 0.5, for the same reason.
    pub liquid_spreading_fraction: f64,
    /// How much more a liquid Cell must hold than a Vacuum or gas neighbour before any of it
    /// spreads there, as a fraction of the liquid's Capacity, from 0 to 1. A pool's edge stops
    /// advancing once its Cells hold less than this, which is what lets pools come to rest
    /// instead of thinning out forever.
    pub liquid_spreading_threshold: f64,
}

impl Default for SimSettings {
    fn default() -> Self {
        Self {
            conduction_multiplier: 1.0,
            settle_threshold_k: 0.01,
            settled_chunks_enabled: true,
            gas_evening_out_fraction: 0.25,
            liquid_spreading_fraction: 0.5,
            liquid_spreading_threshold: 0.01,
        }
    }
}

impl SimSettings {
    /// Checks that every value lies in its range, reporting the first one that doesn't. Values out
    /// of range would break the simulation's invariants: a flow fraction above 0.5 overshoots,
    /// and one above 1 would move Mass a Cell doesn't hold.
    pub fn validate(&self) -> Result<(), SettingsError> {
        let fraction = "above 0 and at most 0.5";
        // Each check holds only for an in-range number, so NaN fails all of them.
        let checks = [
            (
                "conduction_multiplier",
                f64::from(self.conduction_multiplier),
                "finite and at least 0",
                self.conduction_multiplier.is_finite() && self.conduction_multiplier >= 0.0,
            ),
            (
                "settle_threshold_k",
                f64::from(self.settle_threshold_k),
                "finite and at least 0",
                self.settle_threshold_k.is_finite() && self.settle_threshold_k >= 0.0,
            ),
            (
                "gas_evening_out_fraction",
                self.gas_evening_out_fraction,
                fraction,
                self.gas_evening_out_fraction > 0.0 && self.gas_evening_out_fraction <= 0.5,
            ),
            (
                "liquid_spreading_fraction",
                self.liquid_spreading_fraction,
                fraction,
                self.liquid_spreading_fraction > 0.0 && self.liquid_spreading_fraction <= 0.5,
            ),
            (
                "liquid_spreading_threshold",
                self.liquid_spreading_threshold,
                "from 0 to 1",
                (0.0..=1.0).contains(&self.liquid_spreading_threshold),
            ),
        ];
        for (setting, value, allowed, in_range) in checks {
            if !in_range {
                return Err(SettingsError {
                    setting,
                    value,
                    allowed,
                });
            }
        }
        Ok(())
    }
}

/// Why a [`SimSettings`] was rejected: one value lies outside its range.
#[derive(Clone, Debug, PartialEq)]
pub struct SettingsError {
    /// The setting's name, as written in a settings file.
    pub setting: &'static str,
    pub value: f64,
    /// The range the value must lie in, in words.
    pub allowed: &'static str,
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} is {}, but it must be {}",
            self.setting, self.value, self.allowed
        )
    }
}

impl std::error::Error for SettingsError {}
