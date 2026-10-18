//! The map's zones (`RGE_Zone_Map`, made by `FUN_141410680`, labelled by `FUN_141413190`): every
//! tile gets the number of the patch of tiles it belongs to, a patch being the tiles a 4-way
//! flood reaches over tiles that are, like its first, passable or not for a terrain restriction
//! (its value for the tile's terrain, map tile `+0x9`, above 0). Patches are numbered in the order
//! of their first tile, row by row; past 511 the numbers start again at 0 (`FUN_141413190`: the
//! 513th patch is 0, the next 1, ...).
//!
//! The map keeps one zone map per passability pattern (`map+0x5e28`, `FUN_1414138d0`: a map is
//! shared by the restrictions whose values have the same signs), made the first time a unit of
//! the pattern asks for its zone (`FUN_1411931c0`, cached on the unit at `+0x698`). A map is not
//! made again when the terrain changes later; the slice makes it from the terrain it holds when
//! it first needs it, which differs only if a terrain change since the game made it changed a
//! tile's passability.

/// The number after the last a patch takes; the next patch takes 0 again.
const PATCHES: u16 = 0x200;

/// A tile's zone off the map (`FUN_141413890`).
const OFF_MAP: u16 = 0;

/// The zones of one passability pattern.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZoneMap {
    width: i32,
    height: i32,
    /// The pattern: for each terrain, the restriction's value is above 0.
    pub passable: Vec<bool>,
    zones: Vec<u16>,
}

impl ZoneMap {
    /// The zones of the map's terrain (`kinds`, row by row) for a restriction whose value per
    /// terrain is `values`: `FUN_141413190`'s scan row by row, each tile not yet reached
    /// starting a patch (`FUN_141413340`'s flood over the four neighbours of each tile it takes).
    /// A terrain the restriction has no value for counts as impassable.
    #[must_use]
    pub fn new(width: i32, height: i32, kinds: &[u8], values: &[f32]) -> Self {
        let passable: Vec<bool> = values.iter().map(|&v| 0.0 < v).collect();
        let open = |i: usize| {
            passable
                .get(usize::from(kinds[i]))
                .copied()
                .unwrap_or(false)
        };
        let n = (width.max(0) * height.max(0)) as usize;
        let mut zones = vec![PATCHES; n];
        let mut next: u16 = 0;
        let mut stack = Vec::new();
        for start in 0..n {
            if zones[start] != PATCHES {
                continue;
            }
            let zone = if next < PATCHES { next } else { 0 };
            next = zone + 1;
            let kind = open(start);
            zones[start] = zone;
            stack.push(start);
            while let Some(i) = stack.pop() {
                let (x, y) = ((i as i32) % width, (i as i32) / width);
                for (nx, ny) in [(x - 1, y), (x, y - 1), (x + 1, y), (x, y + 1)] {
                    if nx < 0 || ny < 0 || width <= nx || height <= ny {
                        continue;
                    }
                    let j = (ny * width + nx) as usize;
                    if zones[j] == PATCHES && open(j) == kind {
                        zones[j] = zone;
                        stack.push(j);
                    }
                }
            }
        }
        Self {
            width,
            height,
            passable,
            zones,
        }
    }

    /// A tile's zone (`FUN_141413890`): 0 off the map.
    #[must_use]
    pub fn zone(&self, x: i32, y: i32) -> u16 {
        if x < 0 || y < 0 || self.width <= x || self.height <= y {
            return OFF_MAP;
        }
        self.zones[(y * self.width + x) as usize]
    }
}

/// The map's zone maps, one per passability pattern, made when first asked for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Zones {
    maps: Vec<ZoneMap>,
}

impl Zones {
    /// The zone map of a restriction whose value per terrain is `values` (`FUN_1414138d0`): the
    /// map of the same pattern, else a new one of the terrain `kinds`.
    pub fn of(&mut self, width: i32, height: i32, kinds: &[u8], values: &[f32]) -> &ZoneMap {
        let pattern: Vec<bool> = values.iter().map(|&v| 0.0 < v).collect();
        let at = if let Some(i) = self.maps.iter().position(|m| m.passable == pattern) {
            i
        } else {
            self.maps.push(ZoneMap::new(width, height, kinds, values));
            self.maps.len() - 1
        };
        &self.maps[at]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patches_are_numbered_by_their_first_tile_row_by_row() {
        // terrain 1 is impassable: a wall splits the passable tiles in two
        #[rustfmt::skip]
        let kinds = [
            0, 1, 0,
            0, 1, 0,
            0, 1, 1,
        ];
        let z = ZoneMap::new(3, 3, &kinds, &[1.0, 0.0]);
        assert_eq!([z.zone(0, 0), z.zone(1, 0), z.zone(2, 0)], [0, 1, 2]);
        assert_eq!([z.zone(0, 2), z.zone(1, 2), z.zone(2, 2)], [0, 1, 1]);
        assert_eq!(z.zone(2, 1), 2);
        assert_eq!(z.zone(-1, 0), OFF_MAP);
    }

    #[test]
    fn the_flood_takes_four_neighbours_only() {
        // tiles touching at a corner only are patches of their own, passable or not
        let kinds = [0, 1, 1, 0];
        let z = ZoneMap::new(2, 2, &kinds, &[1.0, 0.0]);
        assert_eq!([z.zone(0, 0), z.zone(1, 0)], [0, 1]);
        assert_eq!([z.zone(0, 1), z.zone(1, 1)], [2, 3]);
    }

    #[test]
    fn numbers_start_again_after_511() {
        // a row of 514 tiles alternating passable and not: 514 patches
        let kinds: Vec<u8> = (0..514).map(|i| (i % 2) as u8).collect();
        let z = ZoneMap::new(514, 1, &kinds, &[1.0, 0.0]);
        assert_eq!(
            [z.zone(511, 0), z.zone(512, 0), z.zone(513, 0)],
            [511, 0, 1]
        );
    }

    #[test]
    fn restrictions_of_one_pattern_share_their_map() {
        let mut zones = Zones::default();
        let kinds = [0, 1];
        let _ = zones.of(2, 1, &kinds, &[1.0, 0.0]);
        let _ = zones.of(2, 1, &kinds, &[0.5, -1.0]);
        let _ = zones.of(2, 1, &kinds, &[0.0, 1.0]);
        assert_eq!(zones.maps.len(), 2);
    }
}
