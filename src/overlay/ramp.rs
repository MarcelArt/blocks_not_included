//! The colour ramps the Temperature and Mass Overlays colour Cells by, and the buckets that sort
//! Cells by colour.
//!
//! An Overlay draws a face only where the bucket changes from one Cell to the next, so a run of
//! Cells in one bucket shows as a single layer. Each Overlay has [`BUCKETS`] buckets.

/// A linear RGB colour.
pub type Rgb = [f32; 3];

/// How many colour buckets the Temperature and Mass Overlays sort Cells into.
pub const BUCKETS: u16 = 16;

pub const DARK_BLUE: Rgb = [0.0, 0.0, 0.25];
pub const BLUE: Rgb = [0.0, 0.25, 1.0];
pub const GREEN: Rgb = [0.0, 0.8, 0.1];
pub const YELLOW: Rgb = [1.0, 0.85, 0.0];
pub const RED: Rgb = [1.0, 0.05, 0.0];
pub const WHITE: Rgb = [1.0, 1.0, 1.0];
pub const BLACK: Rgb = [0.0, 0.0, 0.0];
pub const MAGENTA: Rgb = [1.0, 0.0, 1.0];

/// The Temperature Overlay's fixed scale: temperatures in kelvin, each with its colour. Between
/// two stops the colour is interpolated linearly; past either end it is that end's colour.
pub const TEMPERATURE_STOPS: [(f32, Rgb); 6] = [
    (250.0, DARK_BLUE),
    (273.0, BLUE),
    (293.0, GREEN),
    (373.0, YELLOW),
    (800.0, RED),
    (1_500.0, WHITE),
];

/// The Temperature Overlay's colour for `temperature_k`.
pub fn temperature_color(temperature_k: f32) -> Rgb {
    let (stop, along) = temperature_segment(temperature_k);
    lerp(
        TEMPERATURE_STOPS[stop].1,
        TEMPERATURE_STOPS[stop + 1].1,
        along,
    )
}

/// The Temperature Overlay's bucket for `temperature_k`, from 0 to [`BUCKETS`] − 1.
///
/// The buckets split the scale into equal shares of its colours rather than of its kelvin, so
/// each pair of neighbouring stops gets the same number of buckets: 16 / 5 = 3.2. Near room
/// temperature, where the stops lie close together, a bucket spans 6 to 25 K; in the 700 K from
/// red to white, it spans about 220 K.
pub fn temperature_bucket(temperature_k: f32) -> u16 {
    let (stop, along) = temperature_segment(temperature_k);
    let position = (stop as f32 + along) / SEGMENTS as f32;
    bucket_at(position, BUCKETS)
}

/// The colour a Cell in Temperature `bucket` is drawn in: the colour of a temperature within it,
/// spread so that bucket 0 shows the first stop's colour and the last bucket the last stop's.
pub fn temperature_bucket_color(bucket: u16) -> Rgb {
    let position = f32::from(bucket.min(BUCKETS - 1)) / f32::from(BUCKETS - 1);
    let scaled = position * SEGMENTS as f32;
    let stop = (scaled as usize).min(SEGMENTS - 1);
    let along = scaled - stop as f32;
    let (low, high) = (TEMPERATURE_STOPS[stop].0, TEMPERATURE_STOPS[stop + 1].0);
    temperature_color(low * (1.0 - along) + high * along)
}

/// The Mass Overlay's colour for a Cell holding `mass_grams` of an Element whose full Cell holds
/// `full_grams`, its Density × 1 m³: black when empty, white when full, and magenta above full.
pub fn mass_color(mass_grams: u32, full_grams: u32) -> Rgb {
    if mass_grams > full_grams {
        return MAGENTA;
    }
    lerp(BLACK, WHITE, fill(mass_grams, full_grams))
}

/// The Mass Overlay's bucket for a Cell holding `mass_grams` of an Element whose full Cell holds
/// `full_grams`, from 0 to [`BUCKETS`] − 1. The last bucket holds every Cell above full; the
/// others split empty to full evenly.
pub fn mass_bucket(mass_grams: u32, full_grams: u32) -> u16 {
    if mass_grams > full_grams {
        return OVER_FULL;
    }
    bucket_at(fill(mass_grams, full_grams), OVER_FULL)
}

/// The colour a Cell in Mass `bucket` is drawn in: from black for the emptiest to white for the
/// fullest, and magenta for the bucket above full.
pub fn mass_bucket_color(bucket: u16) -> Rgb {
    // The colour of a Cell holding `bucket` fourteenths of a full one.
    mass_color(u32::from(bucket), u32::from(OVER_FULL - 1))
}

/// How many spans lie between the Temperature Overlay's stops.
const SEGMENTS: usize = TEMPERATURE_STOPS.len() - 1;

/// The Mass Overlay's bucket for Cells holding more than a full Cell.
const OVER_FULL: u16 = BUCKETS - 1;

/// Which pair of neighbouring stops `temperature_k` lies between, as the index of the colder one,
/// and how far it lies from that one to the other, from 0 to 1. Past either end of the scale, it
/// is that end; NaN counts as the coldest.
fn temperature_segment(temperature_k: f32) -> (usize, f32) {
    if temperature_k.is_nan() {
        return (0, 0.0);
    }
    let stop = TEMPERATURE_STOPS[1..]
        .iter()
        .position(|&(upper, _)| temperature_k <= upper)
        .unwrap_or(SEGMENTS - 1);
    let (low, high) = (TEMPERATURE_STOPS[stop].0, TEMPERATURE_STOPS[stop + 1].0);
    let along = ((temperature_k - low) / (high - low)).clamp(0.0, 1.0);
    (stop, along)
}

/// Which of `count` equal buckets `position`, from 0 to 1, falls in. 1 falls in the last.
fn bucket_at(position: f32, count: u16) -> u16 {
    // `as` saturates, and turns NaN into 0.
    ((position * f32::from(count)) as u16).min(count - 1)
}

/// What fraction of a full Cell, `full_grams`, `mass_grams` is.
fn fill(mass_grams: u32, full_grams: u32) -> f32 {
    (f64::from(mass_grams) / f64::from(full_grams.max(1))) as f32
}

/// The colour `along` of the way from `from` to `to`: exactly `from` at 0 and `to` at 1.
fn lerp(from: Rgb, to: Rgb, along: f32) -> Rgb {
    std::array::from_fn(|i| from[i] * (1.0 - along) + to[i] * along)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Interpolation is in `f32`, so a colour between stops can be off in its last bits.
    fn assert_close(actual: Rgb, expected: Rgb) {
        let close = actual
            .iter()
            .zip(expected)
            .all(|(a, e)| (a - e).abs() < 1e-6);
        assert!(close, "{actual:?} is not {expected:?}");
    }

    #[test]
    fn each_temperature_stop_has_exactly_its_colour() {
        let stops = [
            (250.0, DARK_BLUE),
            (273.0, BLUE),
            (293.0, GREEN),
            (373.0, YELLOW),
            (800.0, RED),
            (1_500.0, WHITE),
        ];
        for (temperature_k, color) in stops {
            assert_eq!(temperature_color(temperature_k), color, "{temperature_k} K");
        }
    }

    #[test]
    fn halfway_between_two_temperature_stops_is_halfway_between_their_colours() {
        // (293 + 373) / 2 = 333 K, halfway from green (0, 0.8, 0.1) to yellow (1, 0.85, 0).
        assert_close(temperature_color(333.0), [0.5, 0.825, 0.05]);
        // (250 + 273) / 2 = 261.5 K, halfway from dark blue (0, 0, 0.25) to blue (0, 0.25, 1).
        assert_close(temperature_color(261.5), [0.0, 0.125, 0.625]);
        // (800 + 1500) / 2 = 1150 K, halfway from red (1, 0.05, 0) to white (1, 1, 1).
        assert_close(temperature_color(1_150.0), [1.0, 0.525, 0.5]);
    }

    #[test]
    fn past_either_end_of_the_temperature_scale_the_colour_is_that_ends() {
        assert_eq!(temperature_color(0.0), DARK_BLUE);
        assert_eq!(temperature_color(249.0), DARK_BLUE);
        assert_eq!(temperature_color(1_501.0), WHITE);
        assert_eq!(temperature_color(6_000.0), WHITE);
    }

    #[test]
    fn the_mass_ramp_runs_from_black_when_empty_to_white_when_full() {
        // Water, whose full Cell holds 1,000 kg.
        let full = 1_000_000;
        assert_eq!(mass_color(0, full), BLACK);
        assert_eq!(mass_color(full, full), WHITE);
        // 250 kg is a quarter of full.
        assert_close(mass_color(250_000, full), [0.25, 0.25, 0.25]);
    }

    #[test]
    fn mass_over_a_full_cell_is_magenta() {
        // 1 g over Water's 1,000 kg, a Phase Transition's 1,200 kg, and Oxygen pressed to ten
        // times its 1.43 kg.
        assert_eq!(mass_color(1_000_001, 1_000_000), MAGENTA);
        assert_eq!(mass_color(1_200_000, 1_000_000), MAGENTA);
        assert_eq!(mass_color(14_300, 1_430), MAGENTA);
    }

    /// The buckets of `values`, in order.
    fn buckets_of<T>(values: impl Iterator<Item = T>, bucket: impl Fn(T) -> u16) -> Vec<u16> {
        values.map(bucket).collect()
    }

    /// Checks that `buckets` never fall as the values rise, and that each of the 16 buckets is
    /// reached in turn, from the first to the last.
    fn assert_covers_every_bucket_in_order(buckets: &[u16]) {
        assert!(
            buckets.windows(2).all(|pair| pair[0] <= pair[1]),
            "the buckets fall somewhere: {buckets:?}"
        );
        let mut reached: Vec<u16> = buckets.to_vec();
        reached.dedup();
        assert_eq!(reached, (0..BUCKETS).collect::<Vec<_>>());
    }

    #[test]
    fn temperature_buckets_rise_with_temperature_and_cover_the_whole_scale() {
        // Every tenth of a kelvin from 250 K to 1500 K: 12,501 temperatures.
        let temperatures = (0..=12_500).map(|tenths| 250.0 + tenths as f32 / 10.0);
        let buckets = buckets_of(temperatures, temperature_bucket);
        assert_covers_every_bucket_in_order(&buckets);

        // Past either end, a temperature falls in the end's bucket.
        assert_eq!(temperature_bucket(0.0), 0);
        assert_eq!(temperature_bucket(6_000.0), BUCKETS - 1);
    }

    #[test]
    fn mass_buckets_rise_with_mass_up_to_full_and_mass_over_full_has_the_last_bucket() {
        // Every gram from empty to Oxygen's full 1,430 g.
        let full = 1_430;
        let buckets = buckets_of(0..=full, |mass| mass_bucket(mass, full));
        // Over full is the last bucket, so up to full reaches all but that one.
        assert!(buckets.iter().all(|&bucket| bucket < BUCKETS - 1));
        let mut with_over = buckets.clone();
        with_over.push(mass_bucket(full + 1, full));
        assert_covers_every_bucket_in_order(&with_over);
        assert_eq!(mass_bucket(full * 10, full), BUCKETS - 1);
    }

    #[test]
    fn equal_values_always_land_in_the_same_bucket() {
        // Each value's bucket, worked out twice over, in a different order the second time.
        let temperatures: Vec<f32> = (0..2_000).map(|k| k as f32 * 0.85).collect();
        let first: Vec<u16> = temperatures
            .iter()
            .map(|&k| temperature_bucket(k))
            .collect();
        let second: Vec<u16> = temperatures
            .iter()
            .rev()
            .map(|&k| temperature_bucket(k))
            .rev()
            .collect();
        assert_eq!(first, second);

        // A whole-gram Mass at the same fill of different Elements: 500 kg of Water and
        // 1.3 t of Magma are both half full, at 1,000 kg and 2.6 t a Cell.
        assert_eq!(
            mass_bucket(500_000, 1_000_000),
            mass_bucket(1_300_000, 2_600_000)
        );
    }

    #[test]
    fn a_buckets_colour_is_one_its_own_values_take() {
        // The coldest and hottest buckets show the scale's end colours, and the empty and full
        // ones black and white.
        assert_eq!(temperature_bucket_color(0), DARK_BLUE);
        assert_eq!(temperature_bucket_color(BUCKETS - 1), WHITE);
        assert_eq!(mass_bucket_color(0), BLACK);
        assert_eq!(mass_bucket_color(BUCKETS - 2), WHITE);
        assert_eq!(mass_bucket_color(BUCKETS - 1), MAGENTA);

        // Every bucket's colour is the colour of some temperature in it.
        for bucket in 0..BUCKETS {
            let color = temperature_bucket_color(bucket);
            let found = (0..=12_500)
                .map(|tenths| 250.0 + tenths as f32 / 10.0)
                .filter(|&k| temperature_bucket(k) == bucket)
                .any(|k| {
                    temperature_color(k)
                        .iter()
                        .zip(color)
                        .all(|(a, b)| (a - b).abs() < 0.01)
                });
            assert!(found, "no temperature in bucket {bucket} is {color:?}");
        }
    }
}
