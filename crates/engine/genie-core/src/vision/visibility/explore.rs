//! `explore_terrain` and `unexplore_terrain` over a circle and a rectangle, and the reveal of
//! a rectangle.

use crate::GenieError;
use crate::vision::los_tables::{CENTRED, OFFSET};
use crate::vision::visibility::{ANCHOR, LINKED, MOST, Visibility};
use crate::world::slots::CheckedIndex;

impl Visibility {
    // The visible map's routines (`RGE_Visible_Map`).

    /// `explore_terrain` (`FUN_141284a40`): the tiles of a circle of `los` (at most 20) around
    /// tile (`x`, `y`), from the table [`OFFSET`] for a building whose centre is on a tile corner,
    /// count one more seer. A tile seen for the first time counts as explored, and its first sight
    /// of a building's tile whose anchor it never saw explores the building's tiles. Returns the
    /// tiles explored.
    pub fn explore(
        &mut self,
        player: usize,
        x: i32,
        y: i32,
        los: i32,
        offset: bool,
    ) -> Result<i32, GenieError> {
        let table = *(if offset { &OFFSET } else { &CENTRED }).checked(los.clamp(0, 20) as usize)?;
        let (visible, seen) = (1u64 << player, 1u64 << (32 + player));
        let mut newly = 0;
        for &(dy, dx0, dx1) in table {
            let ty = y + i32::from(dy);
            if ty < 0 || self.height <= ty {
                continue;
            }
            let x0 = (x + i32::from(dx0)).max(0);
            let x1 = (x + i32::from(dx1)).min(self.width - 1);
            for tx in x0..=x1 {
                let i = self.index(tx, ty);
                let mut c = self
                    .maps
                    .checked(player)?
                    .counts
                    .checked(i)?
                    .wrapping_add(1);
                if c == 0 {
                    newly += 1;
                    self.maps.checked_mut(player)?.explored += 1;
                    self.new_tile(player, tx, ty);
                    if self.computer.get(player).copied().unwrap_or(false) {
                        self.ai_tiles.push((player, [tx, ty]));
                    }
                    if self.links.checked(i)?.0 & (LINKED | ANCHOR) != 0 {
                        self.reveal_building(player, tx, ty)?;
                    }
                    c = 1;
                    *self.unified.checked_mut(i)? |= seen;
                }
                if c == 1 {
                    *self.unified.checked_mut(i)? |= visible;
                }
                *self.maps.checked_mut(player)?.counts.checked_mut(i)? = c.min(MOST);
            }
        }
        Ok(newly)
    }

    /// `explore_terrain`'s case of a building's tile (`FUN_140f68f50`, `FUN_140f68e30`): when the
    /// building's anchor was never explored, the tile counts one seer and the building's tiles
    /// count as explored.
    pub(super) fn reveal_building(
        &mut self,
        player: usize,
        x: i32,
        y: i32,
    ) -> Result<(), GenieError> {
        let Some([ax, ay]) = self.anchor_of(x, y) else {
            return Ok(());
        };
        if *self
            .maps
            .checked(player)?
            .counts
            .checked(self.index(ax, ay))?
            != -1
        {
            return Ok(());
        }
        let Some([x0, y0, x1, y1]) = self.extent_of(x, y) else {
            return Ok(());
        };
        let i = self.index(x, y);
        *self.maps.checked_mut(player)?.counts.checked_mut(i)? = 1;
        let _ = self.reveal_rect(player, x0, y0, x1, y1)?;
        Ok(())
    }

    /// `unexplore_terrain` (`FUN_141287170`): the circle's tiles count one seer less (a tile at 0
    /// stays 0); a tile left without seers is no longer visible.
    pub fn unexplore(
        &mut self,
        player: usize,
        x: i32,
        y: i32,
        los: i32,
        offset: bool,
    ) -> Result<(), GenieError> {
        let table = *(if offset { &OFFSET } else { &CENTRED }).checked(los.clamp(0, 20) as usize)?;
        let hidden = !(1u64 << player);
        for &(dy, dx0, dx1) in table {
            let ty = y + i32::from(dy);
            if ty < 0 || self.height <= ty {
                continue;
            }
            let x0 = (x + i32::from(dx0)).max(0);
            let x1 = (x + i32::from(dx1)).min(self.width - 1);
            for tx in x0..=x1 {
                let i = self.index(tx, ty);
                let c = *self.maps.checked(player)?.counts.checked(i)?;
                let c = if c != 0 { c.wrapping_sub(1) } else { 0 };
                if c == 0 {
                    *self.unified.checked_mut(i)? &= hidden;
                }
                *self.maps.checked_mut(player)?.counts.checked_mut(i)? = c;
            }
        }
        Ok(())
    }

    /// The tile range of a rectangle held on the map, None when it misses the map.
    pub(super) fn held(&self, x0: i32, y0: i32, x1: i32, y1: i32) -> Option<(i32, i32, i32, i32)> {
        (x0 < self.width && 0 <= x1 && y0 < self.height && 0 <= y1).then(|| {
            (
                x0.max(0),
                y0.max(0),
                x1.min(self.width - 1),
                y1.min(self.height - 1),
            )
        })
    }

    /// `explore_terrain_sq` (`FUN_141284f10`): `explore_terrain` over a rectangle, without the
    /// buildings' case.
    pub fn explore_rect(
        &mut self,
        player: usize,
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
    ) -> Result<i32, GenieError> {
        let Some((x0, y0, x1, y1)) = self.held(x0, y0, x1, y1) else {
            return Ok(0);
        };
        let (visible, seen) = (1u64 << player, 1u64 << (32 + player));
        let mut newly = 0;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let i = self.index(x, y);
                let mut c = self
                    .maps
                    .checked(player)?
                    .counts
                    .checked(i)?
                    .wrapping_add(1);
                if c == 0 {
                    newly += 1;
                    self.maps.checked_mut(player)?.explored += 1;
                    self.new_tile(player, x, y);
                    c = 1;
                    *self.unified.checked_mut(i)? |= seen;
                }
                if c == 1 {
                    *self.unified.checked_mut(i)? |= visible;
                }
                *self.maps.checked_mut(player)?.counts.checked_mut(i)? = c.min(MOST);
            }
        }
        Ok(newly)
    }

    /// `unexplore_terrain_sq` (`FUN_141287480`): `unexplore_terrain` over a rectangle.
    pub fn unexplore_rect(
        &mut self,
        player: usize,
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
    ) -> Result<(), GenieError> {
        let Some((x0, y0, x1, y1)) = self.held(x0, y0, x1, y1) else {
            return Ok(());
        };
        let hidden = !(1u64 << player);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let i = self.index(x, y);
                let c = *self.maps.checked(player)?.counts.checked(i)?;
                let c = if c != 0 { c.wrapping_sub(1) } else { 0 };
                if c == 0 {
                    *self.unified.checked_mut(i)? &= hidden;
                }
                *self.maps.checked_mut(player)?.counts.checked_mut(i)? = c;
            }
        }
        Ok(())
    }

    /// `reveal_terrain_sq` (`FUN_141285da0`): the rectangle's never explored tiles count as
    /// explored.
    pub fn reveal_rect(
        &mut self,
        player: usize,
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
    ) -> Result<i32, GenieError> {
        let Some((x0, y0, x1, y1)) = self.held(x0, y0, x1, y1) else {
            return Ok(0);
        };
        let seen = 1u64 << (32 + player);
        let mut newly = 0;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let i = self.index(x, y);
                if *self.maps.checked(player)?.counts.checked(i)? == -1 {
                    self.maps.checked_mut(player)?.explored += 1;
                    newly += 1;
                    self.new_tile(player, x, y);
                    *self.unified.checked_mut(i)? |= seen;
                    *self.maps.checked_mut(player)?.counts.checked_mut(i)? = 0;
                }
            }
        }
        Ok(newly)
    }
}
