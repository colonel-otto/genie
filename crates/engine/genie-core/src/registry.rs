//! The objects-in-sight registry (`Visible_Unit_Manager`, `DAT_143f69170`): per player and per
//! master class, a record of each combat object the player sees, kept by the objects' own updates
//! from the unified map's visible bits, and the query the unit AIs search it with
//! (`GetVisibleUnits`, `FUN_141288c00`).
//!
//! The save holds neither the lists nor the objects' masks: the slice starts from the game's own
//! (`load_oracle/probes/registry.py`). A list's order is its history's: records join at its end and
//! leave by a swap with its last.

use crate::unit_ai::Unsupported;
use std::collections::BTreeMap;

/// The unified map bits an object's mask keeps (`0x1ffffff01ffffff`): 25 visible and 25 explored.
pub const MASK: u64 = 0x1ff_ffff_01ff_ffff;

/// The players a mask has room for in `Update_Unit_Info`'s loop: the object's record pointers
/// (`+0x8b0`) are nine.
pub const SLOTS: usize = 9;

/// The query's cap on the radius (`DAT_142f64e88`).
const MOST_RADIUS: f32 = crate::image::registry::MOST_RADIUS;

/// The query box's margin (`DAT_142ec9eb8`).
const MARGIN: f32 = crate::image::registry::MARGIN;

/// The records a query keeps per relation (`0x1c00` bytes per output list).
pub const MOST_FOUND: usize = 256;

/// One record (0x1c bytes): the object, a distance (`FLT_MAX` in the lists; a query's copy holds
/// its own), the owner's id (`player+0x718`), the place and the master's radii.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Record {
    pub id: i32,
    pub distance: f32,
    pub owner: u8,
    pub x: f32,
    pub y: f32,
    pub rx: f32,
    pub ry: f32,
}

/// An object's part in the registry: its mask (`+0x8f8`) and, per player, its record's index in
/// that player's list of its class (`+0x8b0`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Entry {
    pub mask: u64,
    pub slots: [Option<usize>; SLOTS],
}

/// The registry: `lists[player][class]`, and each object's [`Entry`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Registry {
    pub players: usize,
    pub classes: usize,
    pub lists: Vec<Vec<Vec<Record>>>,
    pub entries: BTreeMap<i32, Entry>,
}

/// What owner `owner` is to `viewer` in its queries (`player+0x278`): 1 itself, 0 Gaia, then by
/// the diplomacy `relation` (`player+0x260`): an ally (0) 2, a neutral (1) 3, an enemy (3) 4. The
/// mapping reproduces the game's lists of this recording's players (Gaia `[1, 4, 4]`, player 1
/// `[0, 1, 4]`, player 2 `[0, 4, 1]`); its writer is not ported.
pub fn relation_kind(viewer: usize, owner: usize, relation: u8) -> Result<u32, Unsupported> {
    if viewer == owner {
        return Ok(1);
    }
    if owner == 0 {
        return Ok(0);
    }
    match relation {
        0 => Ok(2),
        1 => Ok(3),
        3 => Ok(4),
        _ => Err(Unsupported("a diplomacy relation other than 0, 1 and 3")),
    }
}

impl Registry {
    pub fn new(players: usize, classes: usize) -> Self {
        Registry {
            players,
            classes,
            lists: vec![vec![Vec::new(); classes]; players],
            entries: BTreeMap::new(),
        }
    }

    pub fn entry(&self, id: i32) -> Option<&Entry> {
        self.entries.get(&id)
    }

    /// `Update_Unit_Info` (`FUN_14128a1d0`): for each player whose bit (of the low 16) differs
    /// between `old` and `new`, the object joins the end of that player's list of its class, or
    /// leaves it (the list's last record takes its place).
    #[allow(clippy::too_many_arguments)]
    pub fn update_unit(
        &mut self,
        id: i32,
        owner: u8,
        [x, y]: [f32; 2],
        [rx, ry]: [f32; 2],
        class: i16,
        old: u64,
        new: u64,
    ) -> Result<(), Unsupported> {
        let mut changed = (old ^ new) & 0xffff;
        let mut seen = new;
        if class < 0 {
            return Err(Unsupported(
                "a registry update for a master without a class",
            ));
        }
        let class = class as usize;
        if self.classes <= class {
            return Ok(());
        }
        for p in 0..self.players {
            if changed == 0 {
                return Ok(());
            }
            if changed & 1 != 0 {
                let slot = self
                    .entries
                    .entry(id)
                    .or_default()
                    .slots
                    .get(p)
                    .copied()
                    .flatten();
                let list = &mut self.lists[p][class];
                if seen & 1 == 0 {
                    if !list.is_empty() {
                        let i = slot.ok_or(Unsupported(
                            "a registry removal of an object without its record",
                        ))?;
                        let last = list.len() - 1;
                        if i != last {
                            list.swap(i, last);
                            let moved = list[i].id;
                            if let Some(s) = self
                                .entries
                                .get_mut(&moved)
                                .and_then(|e| e.slots.get_mut(p))
                            {
                                *s = Some(i);
                            }
                        }
                        self.lists[p][class].pop();
                    }
                    if let Some(s) = self.entries.get_mut(&id).and_then(|e| e.slots.get_mut(p)) {
                        *s = None;
                    }
                } else {
                    list.push(Record {
                        id,
                        distance: f32::MAX,
                        owner,
                        x,
                        y,
                        rx,
                        ry,
                    });
                    let i = list.len() - 1;
                    if let Some(s) = self.entries.get_mut(&id).and_then(|e| e.slots.get_mut(p)) {
                        *s = Some(i);
                    }
                }
            }
            changed >>= 1;
            seen >>= 1;
        }
        Ok(())
    }

    /// An ownership change (`FUN_14120f800`): the object's records take the new owner's id.
    pub fn owner_changed(&mut self, id: i32, owner: usize) -> Result<(), Unsupported> {
        let Some(e) = self.entries.get(&id).copied() else {
            return Ok(());
        };
        for (p, slot) in e.slots.iter().enumerate().take(self.players) {
            let Some(i) = *slot else { continue };
            let record = self.lists[p]
                .iter_mut()
                .flat_map(|list| list.get_mut(i))
                .find(|r| r.id == id)
                .ok_or(Unsupported("an object's registry slot without its record"))?;
            record.owner = owner as u8;
        }
        Ok(())
    }

    /// The combat object's new place (`FUN_14108c320`): its records in every list follow.
    pub fn moved(&mut self, id: i32, class: i16, [x, y]: [f32; 2]) {
        let Some(e) = self.entries.get(&id).copied() else {
            return;
        };
        let Ok(class) = usize::try_from(class) else {
            return;
        };
        for (p, slot) in e.slots.iter().enumerate().take(self.players) {
            if let Some(r) = slot.and_then(|i| self.lists[p].get_mut(class)?.get_mut(i)) {
                r.x = x;
                r.y = y;
            }
        }
    }

    /// `GetVisibleUnits` (`FUN_141288c00`) for `player`: the records of its lists of `classes`
    /// whose object is within `radius` (at most 44) of (`x`, `y`) for a searcher of radii `rx`,
    /// `ry` (`mode` 0: a rounded box, 1: a box), and whose owner's relation (`relations[owner]`:
    /// 0 Gaia, 1 self, 2 friend, 3 neutral, 4 enemy) is in `wanted`; by relation, each with its
    /// edge distance, at most 256 each. None when the query reads nothing (no classes, a player
    /// out of range), which leaves the game's last answer in place.
    #[allow(clippy::too_many_arguments)]
    pub fn visible_units(
        &self,
        player: usize,
        [x, y]: [f32; 2],
        radius: f32,
        mode: i32,
        classes: &[i32],
        relations: &[u32],
        [rx, ry]: [f32; 2],
        wanted: u8,
    ) -> Result<Option<[Vec<Record>; 5]>, Unsupported> {
        if classes.is_empty() || self.players <= player {
            return Ok(None);
        }
        let radius = if radius <= MOST_RADIUS {
            radius
        } else {
            MOST_RADIUS
        };
        let wide = radius + rx;
        let tall = radius + ry;
        let (x0, x1) = ((x - wide) - MARGIN, x + wide + MARGIN);
        let (y0, y1) = ((y - tall) - MARGIN, y + tall + MARGIN);
        let round = ((wide - rx) + (tall - ry)) * 0.5;
        let mut found: [Vec<Record>; 5] = Default::default();
        for &class in classes {
            let list = usize::try_from(class)
                .ok()
                .and_then(|c| self.lists[player].get(c))
                .ok_or(Unsupported("a query of a class beyond the registry's"))?;
            for r in list {
                let relation = *relations
                    .get(usize::from(r.owner))
                    .ok_or(Unsupported("a record of an owner without a relation"))?;
                if relation > 4 || wanted & (1 << relation) == 0 {
                    continue;
                }
                if !(r.x <= x1 && x0 <= r.x && r.y <= y1 && y0 <= r.y) {
                    continue;
                }
                let dx = (r.x - x).abs() - r.rx;
                let dy = (r.y - y).abs() - r.ry;
                let inside = match mode {
                    0 => {
                        if rx < dx {
                            if ry < dy {
                                (dy - ry) * (dy - ry) + (dx - rx) * (dx - rx) <= round * round
                            } else {
                                dx < wide
                            }
                        } else {
                            dy < tall
                        }
                    }
                    1 => dx <= wide && dy <= tall,
                    _ => false,
                };
                if !inside {
                    continue;
                }
                let out = &mut found[relation as usize];
                if out.len() < MOST_FOUND {
                    out.push(Record {
                        distance: (dy * dy + dx * dx).sqrt(),
                        ..*r
                    });
                }
            }
        }
        Ok(Some(found))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> Registry {
        Registry::new(3, 65)
    }

    #[test]
    fn an_object_joins_a_list_when_seen_and_leaves_by_a_swap_with_the_last() {
        let mut r = registry();
        for id in [1, 2, 3] {
            r.update_unit(id, 1, [id as f32, 5.0], [0.2, 0.2], 4, 0, 0b101)
                .unwrap();
        }
        assert_eq!(
            r.lists[0][4].iter().map(|x| x.id).collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert_eq!(r.lists[2][4].len(), 3);
        assert!(r.lists[1][4].is_empty());
        // 1 leaves player 0's sight: 3 takes its place
        r.update_unit(1, 1, [1.0, 5.0], [0.2, 0.2], 4, 0b101, 0b100)
            .unwrap();
        assert_eq!(
            r.lists[0][4].iter().map(|x| x.id).collect::<Vec<_>>(),
            [3, 2]
        );
        assert_eq!(r.entry(3).unwrap().slots[0], Some(0));
        assert_eq!(r.entry(1).unwrap().slots[0], None);
        assert_eq!(r.entry(1).unwrap().slots[2], Some(0));
        r.moved(3, 4, [7.0, 7.0]);
        assert_eq!((r.lists[0][4][0].x, r.lists[2][4][2].y), (7.0, 7.0));
    }

    #[test]
    fn a_query_finds_the_records_within_its_rounded_box_by_relation() {
        let mut r = registry();
        r.update_unit(1, 1, [12.0, 10.0], [0.2, 0.2], 4, 0, 1)
            .unwrap();
        r.update_unit(2, 2, [10.0, 13.5], [0.2, 0.2], 4, 0, 1)
            .unwrap();
        r.update_unit(3, 2, [13.0, 13.0], [0.2, 0.2], 4, 0, 1)
            .unwrap();
        r.update_unit(4, 0, [10.5, 10.5], [0.2, 0.2], 9, 0, 1)
            .unwrap();
        let relations = [1, 4, 4];
        let found = r
            .visible_units(
                0,
                [10.0, 10.0],
                3.0,
                0,
                &[4, 9],
                &relations,
                [0.2, 0.2],
                0xff,
            )
            .unwrap()
            .unwrap();
        // 1 is 1.8 off along x, 2 is 3.3 off along y (beyond 3.2), 3 is at the corner and beyond
        // the rounding
        assert_eq!(found[4].iter().map(|x| x.id).collect::<Vec<_>>(), [1]);
        assert_eq!(found[1].iter().map(|x| x.id).collect::<Vec<_>>(), [4]);
        // the edge distances, the overlap along y included: (1.8, -0.2)
        assert!((found[4][0].distance - (1.8f32 * 1.8 + 0.04).sqrt()).abs() < 1e-6);
        assert_eq!(
            r.visible_units(0, [10.0, 10.0], 3.0, 0, &[], &relations, [0.2, 0.2], 0xff)
                .unwrap(),
            None
        );
    }

    #[test]
    fn a_query_caps_its_radius_at_44() {
        let mut r = registry();
        r.update_unit(1, 1, [53.5, 10.0], [0.2, 0.2], 4, 0, 1)
            .unwrap();
        r.update_unit(2, 1, [60.0, 10.0], [0.2, 0.2], 4, 0, 1)
            .unwrap();
        let found = r
            .visible_units(
                0,
                [10.0, 10.0],
                100.0,
                1,
                &[4],
                &[1, 4, 4],
                [0.2, 0.2],
                0xff,
            )
            .unwrap()
            .unwrap();
        // the box reaches 44.2 along x: 1's edge is 43.3 off, 2's 49.8
        assert_eq!(found[4].iter().map(|x| x.id).collect::<Vec<_>>(), [1]);
    }

    #[test]
    fn a_query_keeps_a_wide_object_whose_centre_lies_beyond_the_radius() {
        let mut r = registry();
        r.update_unit(1, 1, [15.0, 10.0], [2.0, 2.0], 4, 0, 1)
            .unwrap();
        let found = r
            .visible_units(0, [10.0, 10.0], 3.0, 0, &[4], &[1, 4, 4], [0.2, 0.2], 0xff)
            .unwrap()
            .unwrap();
        // the centre is 5 off, beyond 3.2, but the edge is 3 off: the 7-tile margin keeps it in
        // the box
        assert_eq!(found[4].iter().map(|x| x.id).collect::<Vec<_>>(), [1]);
    }
}
