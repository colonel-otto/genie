//! A point on the map as one type: where a command sends its units, builds, unloads or
//! flares.

use getset::CopyGetters;

/// A point on the map, x and y in tiles: a command's two floats, which the game takes as the
/// packet holds them (any bits, so every pair is a point).
#[derive(Clone, Copy, Debug, Default, PartialEq, CopyGetters)]
pub struct Point {
    /// Its x.
    #[getset(get_copy = "pub")]
    x: f32,
    /// Its y.
    #[getset(get_copy = "pub")]
    y: f32,
}

impl Point {
    /// The point at `x`, `y`.
    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    /// Its x and y.
    #[must_use]
    pub const fn xy(self) -> [f32; 2] {
        [self.x, self.y]
    }
}

#[cfg(test)]
mod tests {
    use crate::map::Point;

    /// A point's x and y keep their bits.
    #[test]
    fn a_point_s_x_and_y_keep_their_bits() {
        let p = Point::new(-0.0, f32::from_bits(0x7fc0_0001));
        let [x, y] = p.xy();
        assert_eq!((x.to_bits(), y.to_bits()), (0x8000_0000, 0x7fc0_0001));
        assert_eq!(
            (p.x().to_bits(), p.y().to_bits()),
            (0x8000_0000, 0x7fc0_0001)
        );
    }
}
