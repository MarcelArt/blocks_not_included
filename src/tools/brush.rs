//! The heat brush: which Cells it reaches, and how holding it becomes batches of Edits.

use bevy::math::IVec3;

/// The shortest time between two batches of the heat brush's Edits, in seconds, so it sends at
/// most 10 Edits per Cell per second however fast frames come.
pub const BATCH_INTERVAL: f32 = 0.1;

/// The Cells within `radius` metres of the Cell at `centre`, measured between Cell centres:
/// `centre` alone at a radius of 0.
pub fn brush_cells(centre: IVec3, radius: f32) -> impl Iterator<Item = IVec3> {
    // A negative or NaN radius reaches only the centre.
    let radius = radius.max(0.0);
    let reach = radius.floor() as i32;
    let span = -reach..=reach;
    span.clone()
        .flat_map(move |y| span.clone().map(move |z| (y, z)))
        .flat_map(move |(y, z)| (-reach..=reach).map(move |x| IVec3::new(x, y, z)))
        .filter(move |offset| offset.length_squared() as f32 <= radius * radius)
        .map(move |offset| centre + offset)
}

/// Holding the heat brush down, turned into batches. Each batch covers the time held since the
/// previous one, and batches come at most every [`BATCH_INTERVAL`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BrushStroke {
    /// Seconds held since the last batch, or `None` outside a stroke.
    held: Option<f32>,
}

impl BrushStroke {
    /// Starts a stroke, as the button goes down.
    pub fn start(&mut self) {
        self.held = Some(0.0);
    }

    /// Ends the stroke. The time held since the last batch, less than [`BATCH_INTERVAL`], is
    /// dropped.
    pub fn end(&mut self) {
        self.held = None;
    }

    /// Holds the stroke for another `seconds`. Once at least [`BATCH_INTERVAL`] has built up
    /// since the last batch, returns all of it, the time the next batch covers. Outside a stroke,
    /// does nothing.
    pub fn hold(&mut self, seconds: f32) -> Option<f32> {
        let held = self.held.as_mut()?;
        *held += seconds;
        (*held >= BATCH_INTERVAL).then(|| std::mem::take(held))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cells(centre: IVec3, radius: f32) -> Vec<IVec3> {
        let mut cells: Vec<_> = brush_cells(centre, radius).collect();
        cells.sort_by_key(|cell| cell.to_array());
        cells
    }

    #[test]
    fn a_brush_of_radius_zero_reaches_only_the_centre() {
        let centre = IVec3::new(4, -2, 7);
        assert_eq!(cells(centre, 0.0), [centre]);
    }

    #[test]
    fn a_brush_reaches_the_cells_whose_centres_lie_within_its_radius() {
        let centre = IVec3::new(4, -2, 7);
        // Radius 1: the centre and its six face neighbours, 1 m away. Edge neighbours are
        // √2 ≈ 1.41 m away, so they are left out.
        let mut expected: Vec<_> = [
            IVec3::ZERO,
            IVec3::X,
            IVec3::NEG_X,
            IVec3::Y,
            IVec3::NEG_Y,
            IVec3::Z,
            IVec3::NEG_Z,
        ]
        .map(|offset| centre + offset)
        .into();
        expected.sort_by_key(|cell| cell.to_array());
        assert_eq!(cells(centre, 1.0), expected);

        // Radius 1.5 adds the 12 edge neighbours at √2 m, but not the 8 corners at √3 ≈ 1.73 m:
        // 1 + 6 + 12 = 19.
        assert_eq!(cells(centre, 1.5).len(), 19);
    }

    #[test]
    fn a_held_stroke_sends_a_batch_once_a_tenth_of_a_second_has_built_up() {
        let mut stroke = BrushStroke::default();
        stroke.start();
        // Frames of 0.04 s: 0.08 s isn't enough, and 0.12 s is, all of it in one batch.
        assert_eq!(stroke.hold(0.04), None);
        assert_eq!(stroke.hold(0.04), None);
        let batch = stroke.hold(0.04).expect("0.12 s have built up");
        assert!((batch - 0.12).abs() < 1e-6, "the batch covers {batch} s");
        // The next batch starts from nothing.
        assert_eq!(stroke.hold(0.04), None);
    }

    #[test]
    fn a_held_stroke_sends_at_most_ten_batches_a_second() {
        for frames_per_second in [30, 60, 144, 1000] {
            let mut stroke = BrushStroke::default();
            stroke.start();
            let frame = 1.0 / frames_per_second as f32;
            let batches = (0..frames_per_second)
                .filter_map(|_| stroke.hold(frame))
                .count();
            assert!(
                batches <= 10,
                "{batches} batches in 1 s at {frames_per_second} frames per second"
            );
        }
    }

    #[test]
    fn nothing_is_sent_outside_a_stroke() {
        let mut stroke = BrushStroke::default();
        assert_eq!(stroke.hold(1.0), None);
        stroke.start();
        assert_eq!(stroke.hold(0.05), None);
        // Ending the stroke drops the 0.05 s held so far.
        stroke.end();
        assert_eq!(stroke.hold(1.0), None);
    }
}
