//! A vector in the game's own order, x, height, y ([`GameVector`]): what slot 0x520 returns
//! and the unit AI's order history keeps, apart from genie's places (x, y, height).

use genie_diff::Diff;
use getset::CopyGetters;

use crate::math::fmath::Vec3;

/// A vector as the game's vector routines hold it: x, height, y. Slot 0x520 returns a place so
/// (`FUN_14106d530`: a moving object's `+0x824`; `FUN_141196c10`: any other's place), and the
/// order history records it (`FUN_140c42080`). genie's places are x, y, height
/// ([`GameVector::of_place`], [`GameVector::place`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, CopyGetters, Diff)]
pub struct GameVector {
    /// Its x.
    #[getset(get_copy = "pub")]
    x: f32,
    /// Its height.
    #[getset(get_copy = "pub")]
    height: f32,
    /// Its y.
    #[getset(get_copy = "pub")]
    y: f32,
}

impl GameVector {
    /// The vector of `x`, `height`, `y`.
    #[must_use]
    pub const fn new(x: f32, height: f32, y: f32) -> Self {
        Self { x, height, y }
    }

    /// The vector the game holds as `[x, height, y]` (a save's 12 bytes, a routine's array).
    #[must_use]
    pub const fn from_xhy([x, height, y]: Vec3) -> Self {
        Self { x, height, y }
    }

    /// The game's vector of genie's place `[x, y, height]`.
    #[must_use]
    pub const fn of_place([x, y, height]: Vec3) -> Self {
        Self { x, height, y }
    }

    /// genie's place of it: `[x, y, height]`.
    #[must_use]
    pub const fn place(self) -> Vec3 {
        [self.x, self.y, self.height]
    }

    /// Its three values in the game's order, `[x, height, y]`, for the vector math the game
    /// does in that order.
    #[must_use]
    pub const fn xhy(self) -> Vec3 {
        [self.x, self.height, self.y]
    }
}

#[cfg(test)]
mod tests {
    use crate::math::game_vector::GameVector;

    /// A place and the game's vector are the same values in two orders, each bit kept.
    #[test]
    fn a_place_and_the_game_s_vector_swap_the_height_and_y() {
        let v = GameVector::of_place([10.0, 11.0, -0.0]);
        assert_eq!(
            (v.x(), v.height().to_bits(), v.y()),
            (10.0, 0x8000_0000, 11.0)
        );
        assert_eq!(v.xhy(), [10.0, -0.0, 11.0]);
        assert_eq!(v.place(), [10.0, 11.0, -0.0]);
        assert_eq!(
            GameVector::from_xhy([1.0, 2.0, 3.0]),
            GameVector::new(1.0, 2.0, 3.0)
        );
        assert_eq!(GameVector::new(1.0, 2.0, 3.0).place(), [1.0, 3.0, 2.0]);
    }
}
