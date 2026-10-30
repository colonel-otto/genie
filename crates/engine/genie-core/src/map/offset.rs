//! A displacement on the map as one type ([`Offset`]): x, y and z, apart from a place
//! ([`crate::map::Position`]) and the game's vectors (x, height, y,
//! [`crate::math::fmath::BVector`]).

use crate::map::Position;

/// A displacement on the map in the map's order: x and y in tiles, z the height. A missile's
/// direction (`+0x68`..`+0x70`) and its step of an update (`+0x7dc`..`+0x7e4`), a master's
/// weapon offset from its object's place (`+0x844`..`+0x84c`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Offset(
    /// Its x, y and z.
    [f32; 3],
);

impl Offset {
    /// No displacement.
    pub const ZERO: Self = Self([0.0; 3]);

    /// The displacement by `x`, `y` and height `z`.
    #[must_use]
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self([x, y, z])
    }

    /// Its x, y and z, in that order (a save's three floats).
    #[must_use]
    pub const fn get(self) -> [f32; 3] {
        self.0
    }

    /// Its x.
    #[must_use]
    pub const fn x(self) -> f32 {
        self.0[0]
    }

    /// Its y.
    #[must_use]
    pub const fn y(self) -> f32 {
        self.0[1]
    }

    /// Its height.
    #[must_use]
    pub const fn z(self) -> f32 {
        self.0[2]
    }

    /// The displacement with x `x`.
    #[must_use]
    pub const fn with_x(self, x: f32) -> Self {
        Self([x, self.0[1], self.0[2]])
    }

    /// The displacement with y `y`.
    #[must_use]
    pub const fn with_y(self, y: f32) -> Self {
        Self([self.0[0], y, self.0[2]])
    }

    /// The displacement with height `z`.
    #[must_use]
    pub const fn with_z(self, z: f32) -> Self {
        Self([self.0[0], self.0[1], z])
    }

    /// `s` times each component, `s` first (`s * x`, as the game multiplies a step).
    #[must_use]
    pub fn scaled(self, s: f32) -> Self {
        let [x, y, z] = self.0;
        Self([s * x, s * y, s * z])
    }
}

impl From<[f32; 3]> for Offset {
    /// The displacement of three floats in the map's order (x, y, z), as a save holds them.
    fn from(c: [f32; 3]) -> Self {
        Self(c)
    }
}

impl genie_diff::Diff for Offset {
    fn diff(&self, other: &Self, d: &mut genie_diff::Differ) {
        self.0.diff(&other.0, d);
    }
}

impl Position {
    /// The place `by` moves this one to: each of its components plus the displacement's, the
    /// place's first (`x + dx`, as the game adds a step).
    #[must_use]
    pub fn moved(self, by: Offset) -> Self {
        Self::new(self.x() + by.x(), self.y() + by.y(), self.z() + by.z())
    }
}

#[cfg(test)]
mod tests {
    use crate::map::{Offset, Position};

    /// An offset keeps its components' bits in the map's order, scales each by the factor
    /// first and moves a place by its components.
    #[test]
    fn an_offset_keeps_its_bits_scales_and_moves_a_place() {
        let o = Offset::new(-0.0, 2.0, f32::from_bits(0x7fc0_0001));
        assert_eq!(
            o.get().map(f32::to_bits),
            [0x8000_0000, 2.0f32.to_bits(), 0x7fc0_0001]
        );
        assert_eq!(
            (o.x().to_bits(), o.y(), o.z().to_bits()),
            (0x8000_0000, 2.0, 0x7fc0_0001)
        );
        assert_eq!(Offset::from([1.0, 2.0, 3.0]), Offset::new(1.0, 2.0, 3.0));
        assert_eq!(
            Offset::ZERO.with_x(1.0).with_y(2.0).with_z(3.0),
            Offset::new(1.0, 2.0, 3.0)
        );
        assert_eq!(
            Offset::new(1.0, -2.0, 0.5).scaled(2.0),
            Offset::new(2.0, -4.0, 1.0)
        );
        // -0 + 0 is +0: the place's component comes first
        let moved = Position::new(-0.0, 10.0, 1.0).moved(Offset::new(0.0, 0.5, -1.0));
        assert_eq!((moved.x().to_bits(), moved.y(), moved.z()), (0, 10.5, 0.0));
    }
}
