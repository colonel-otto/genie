//! The world's clock in the game's milliseconds: a moment of it ([`WorldTime`]) and a span
//! ([`Millis`]). The game keeps both as `u32` and its arithmetic on them wraps.

/// A moment of the world's clock in milliseconds since the game's start (`world_time`): the
/// clock itself and the moments the objects keep (a unit AI's last retarget, a reload's end,
/// a building's last garrison count).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorldTime(u32);

impl WorldTime {
    /// The game's start.
    pub const ZERO: Self = Self(0);

    /// The moment `ms` milliseconds after the game's start.
    #[must_use]
    pub const fn from_millis(ms: u32) -> Self {
        Self(ms)
    }

    /// The milliseconds since the game's start, as the save and the game's log hold them.
    #[must_use]
    pub const fn millis(self) -> u32 {
        self.0
    }

    /// The moment `d` after it (the game's wrapping add).
    #[must_use]
    pub const fn after(self, d: Millis) -> Self {
        Self(self.0.wrapping_add(d.0))
    }

    /// The moment `d` before it, not before the game's start (the game's saturating
    /// subtraction: a unit's last human order time that the world's dt counts down).
    #[must_use]
    pub const fn saturating_before(self, d: Millis) -> Self {
        Self(self.0.saturating_sub(d.0))
    }

    /// The span from `earlier` to it (the game's wrapping subtraction).
    #[must_use]
    pub const fn since(self, earlier: Self) -> Millis {
        Millis(self.0.wrapping_sub(earlier.0))
    }

    /// The span from the game's start to it: where the game passes the world time as a span
    /// (a unit AI's notifies cleared on a MOVE or an ownership change, whose full update passes
    /// the owner's time since its last turn).
    #[must_use]
    pub const fn since_start(self) -> Millis {
        Millis(self.0)
    }

    /// The moment in seconds (`cvtsi2ss`, then `divss` by the image's 1000.0, `DAT_14330ebe0`:
    /// the facing's clock, `FUN_141162db0`).
    #[must_use]
    pub fn seconds(self) -> f32 {
        self.0 as f32 / crate::math::image::facing::MS_PER_SECOND
    }
}

impl std::fmt::Display for WorldTime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl genie_diff::Diff for WorldTime {
    fn diff(&self, other: &Self, d: &mut genie_diff::Differ) {
        self.0.diff(&other.0, d);
    }
}

/// A span of the world's clock in milliseconds: an update's `dt`, a timer, an interval.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Millis(u32);

impl Millis {
    /// No time.
    pub const ZERO: Self = Self(0);

    /// A span of `ms` milliseconds.
    #[must_use]
    pub const fn new(ms: u32) -> Self {
        Self(ms)
    }

    /// The milliseconds, as the save and the commands hold them.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// The span and `d` (the game's wrapping add: a timer that counts the updates' time up).
    #[must_use]
    pub const fn wrapping_add(self, d: Self) -> Self {
        Self(self.0.wrapping_add(d.0))
    }

    /// The span less `d`, at least none (a timer that counts down to 0).
    #[must_use]
    pub const fn saturating_sub(self, d: Self) -> Self {
        Self(self.0.saturating_sub(d.0))
    }

    /// The span in seconds (`cvtsi2ss`, then `divss` by 1000 in f32: `world+0x164` of an
    /// update's `dt`).
    #[must_use]
    pub fn seconds(self) -> f32 {
        self.0 as f32 / 1000.0
    }
}

impl std::fmt::Display for Millis {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl genie_diff::Diff for Millis {
    fn diff(&self, other: &Self, d: &mut genie_diff::Differ) {
        self.0.diff(&other.0, d);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_wraps_as_the_game_s_u32() {
        let late = WorldTime::from_millis(u32::MAX - 9);
        let later = late.after(Millis::new(20));
        assert_eq!(later.millis(), 10);
        assert_eq!(later.since(late), Millis::new(20));
        assert_eq!(WorldTime::ZERO.since(late).get(), 10);
        assert!(WorldTime::from_millis(3) < WorldTime::from_millis(4));
        assert_eq!(WorldTime::from_millis(1500).to_string(), "1500");
        assert_eq!(
            WorldTime::from_millis(1500).since_start(),
            Millis::new(1500)
        );
        let order = WorldTime::from_millis(30);
        assert_eq!(order.saturating_before(Millis::new(50)), WorldTime::ZERO);
        assert_eq!(order.saturating_before(Millis::new(20)).millis(), 10);
    }

    #[test]
    fn a_span_counts_up_wrapping_and_down_to_none() {
        let t = Millis::new(u32::MAX).wrapping_add(Millis::new(2));
        assert_eq!(t.get(), 1);
        assert_eq!(
            Millis::new(50).saturating_sub(Millis::new(80)),
            Millis::ZERO
        );
        assert_eq!(Millis::new(80).saturating_sub(Millis::new(50)).get(), 30);
        assert_eq!(Millis::new(7).to_string(), "7");
    }

    #[test]
    fn seconds_are_the_milliseconds_over_a_thousand_in_f32() {
        assert_eq!(
            Millis::new(50).seconds().to_bits(),
            (50.0_f32 / 1000.0).to_bits()
        );
        assert_eq!(WorldTime::from_millis(16_777_217).seconds(), 16_777.216);
    }
}
