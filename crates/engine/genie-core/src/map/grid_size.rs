//! The size of a grid of tiles as one type ([`GridSize`]): its width and height, apart from a
//! tile on it.

/// The size of a grid of tiles, its width (along x) and its height (along y), in tiles: the map's
/// (the obstruction manager's `+0xa8`, the pathing's), or an influence map's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GridSize(
    /// Its width and height.
    [i32; 2],
);

impl GridSize {
    /// The grid `width` tiles wide and `height` tiles high.
    #[must_use]
    pub const fn new(width: i32, height: i32) -> Self {
        Self([width, height])
    }

    /// Its width and height, in that order (a save's two integers).
    #[must_use]
    pub const fn get(self) -> [i32; 2] {
        self.0
    }

    /// Its width, along x.
    #[must_use]
    pub const fn width(self) -> i32 {
        self.0[0]
    }

    /// Its height, along y.
    #[must_use]
    pub const fn height(self) -> i32 {
        self.0[1]
    }
}

impl From<[i32; 2]> for GridSize {
    /// The size of two integers in their order (width, height), as a save holds them.
    fn from(c: [i32; 2]) -> Self {
        Self(c)
    }
}

impl genie_diff::Diff for GridSize {
    fn diff(&self, other: &Self, d: &mut genie_diff::Differ) {
        self.0.diff(&other.0, d);
    }
}

#[cfg(test)]
mod tests {
    use crate::map::GridSize;

    /// A grid size keeps its width and height in their order.
    #[test]
    fn a_grid_size_keeps_its_width_and_height() {
        let s = GridSize::new(120, 80);
        assert_eq!((s.width(), s.height(), s.get()), (120, 80, [120, 80]));
        assert_eq!(GridSize::from([120, 80]), s);
        assert_eq!(GridSize::default().get(), [0, 0]);
    }
}
