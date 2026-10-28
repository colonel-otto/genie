//! One of a player's object lists (FD-045-D11, D20): its objects in update order and the place
//! of each by id, which the game's table by id (`FUN_1415417a0`, `world+0x168` and `+0x178`)
//! answers.
//!
//! The list's own mutators keep the index; a write through a slot (its stand-in while the
//! object is out for its update, [`TAKEN_OUT`]) keeps it too, as long as the object put back is
//! the one taken. A lookup checks the slot's id, so a stand-in answers none. A list that ever
//! held one id twice (a test's default units) looks up by scanning, the first slot first.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, DefaultHasher};

use crate::objects::unit::{TAKEN_OUT, Unit};

/// The index's hasher: `SipHash` with fixed keys, so a run hashes the same everywhere.
type Fixed = BuildHasherDefault<DefaultHasher>;

/// A player's live or sleeping objects in their list's order, with each one's slot by id.
#[derive(Clone, Debug, Default)]
pub struct ObjectList {
    units: Vec<Unit>,
    slots: HashMap<i32, usize, Fixed>,
    /// The list held one id in two slots: the index does not answer, a scan does.
    twice: bool,
}

impl ObjectList {
    /// The slot of object `id`, if the list holds it (not while it is out for its update).
    #[must_use]
    pub fn position_of(&self, id: i32) -> Option<usize> {
        if id == TAKEN_OUT {
            return None;
        }
        if self.twice {
            return self.units.iter().position(|u| u.id == id);
        }
        let &i = self.slots.get(&id)?;
        self.units.get(i).is_some_and(|u| u.id == id).then_some(i)
    }

    /// Object `id`, if the list holds it.
    #[must_use]
    pub fn by_id(&self, id: i32) -> Option<&Unit> {
        self.position_of(id).map(|i| &self.units[i])
    }

    /// Object `id` to change, if the list holds it.
    pub fn by_id_mut(&mut self, id: i32) -> Option<&mut Unit> {
        self.position_of(id).map(|i| &mut self.units[i])
    }

    /// `unit` goes to the end of the list.
    pub fn push(&mut self, unit: Unit) {
        self.file(unit.id, self.units.len());
        self.units.push(unit);
    }

    /// The objects of `units` go to the end of the list, in their order.
    pub fn extend(&mut self, units: impl IntoIterator<Item = Unit>) {
        for unit in units {
            self.push(unit);
        }
    }

    /// The object in slot `i` leaves the list; the last one takes its slot.
    ///
    /// # Panics
    ///
    /// When the list holds no slot `i`.
    pub fn swap_remove(&mut self, i: usize) -> Unit {
        let gone = self.units.swap_remove(i);
        self.forget(gone.id, i);
        if let Some(moved) = self.units.get(i) {
            let _ = self.slots.insert(moved.id, i);
        }
        gone
    }

    /// The object in slot `i` leaves the list; the ones after it move up.
    ///
    /// # Panics
    ///
    /// When the list holds no slot `i`.
    pub fn remove(&mut self, i: usize) -> Unit {
        let gone = self.units.remove(i);
        self.forget(gone.id, i);
        for (k, u) in self.units.iter().enumerate().skip(i) {
            let _ = self.slots.insert(u.id, k);
        }
        gone
    }

    /// The last object leaves the list, if it holds one.
    pub fn pop(&mut self) -> Option<Unit> {
        let gone = self.units.pop()?;
        self.forget(gone.id, self.units.len());
        Some(gone)
    }

    /// The objects from slot `len` on leave the list.
    pub fn truncate(&mut self, len: usize) {
        while len < self.units.len() {
            let _ = self.pop();
        }
    }

    /// The objects `keep` answers false for leave the list; the others keep their order.
    pub fn retain(&mut self, keep: impl FnMut(&Unit) -> bool) {
        self.units.retain(keep);
        self.reindex();
    }

    /// Every object leaves the list.
    pub fn clear(&mut self) {
        self.units.clear();
        self.slots.clear();
        self.twice = false;
    }

    /// The index again from the list's slots (after a test wrote an id in place).
    pub fn reindex(&mut self) {
        self.slots.clear();
        self.twice = false;
        for i in 0..self.units.len() {
            self.file(self.units[i].id, i);
        }
    }

    /// The index files object `id` in slot `i`, unless it holds the id already.
    fn file(&mut self, id: i32, i: usize) {
        match self.slots.entry(id) {
            std::collections::hash_map::Entry::Occupied(_) => self.twice = true,
            std::collections::hash_map::Entry::Vacant(v) => {
                let _ = v.insert(i);
            }
        }
    }

    /// The index forgets object `id`'s slot `i` (a stand-in's id has none).
    fn forget(&mut self, id: i32, i: usize) {
        if self.slots.get(&id) == Some(&i) {
            let _ = self.slots.remove(&id);
        }
    }
}

impl From<Vec<Unit>> for ObjectList {
    fn from(units: Vec<Unit>) -> Self {
        let mut list = Self {
            units,
            slots: HashMap::default(),
            twice: false,
        };
        list.reindex();
        list
    }
}

impl FromIterator<Unit> for ObjectList {
    fn from_iter<I: IntoIterator<Item = Unit>>(units: I) -> Self {
        Self::from(units.into_iter().collect::<Vec<_>>())
    }
}

impl std::ops::Deref for ObjectList {
    type Target = [Unit];

    fn deref(&self) -> &[Unit] {
        &self.units
    }
}

impl std::ops::DerefMut for ObjectList {
    fn deref_mut(&mut self) -> &mut [Unit] {
        &mut self.units
    }
}

impl IntoIterator for ObjectList {
    type Item = Unit;
    type IntoIter = std::vec::IntoIter<Unit>;

    fn into_iter(self) -> Self::IntoIter {
        self.units.into_iter()
    }
}

impl<'a> IntoIterator for &'a ObjectList {
    type Item = &'a Unit;
    type IntoIter = std::slice::Iter<'a, Unit>;

    fn into_iter(self) -> Self::IntoIter {
        self.units.iter()
    }
}

impl<'a> IntoIterator for &'a mut ObjectList {
    type Item = &'a mut Unit;
    type IntoIter = std::slice::IterMut<'a, Unit>;

    fn into_iter(self) -> Self::IntoIter {
        self.units.iter_mut()
    }
}

impl PartialEq for ObjectList {
    fn eq(&self, other: &Self) -> bool {
        self.units == other.units
    }
}

impl PartialEq<Vec<Unit>> for ObjectList {
    fn eq(&self, other: &Vec<Unit>) -> bool {
        &self.units == other
    }
}

impl<const N: usize> PartialEq<[Unit; N]> for ObjectList {
    fn eq(&self, other: &[Unit; N]) -> bool {
        self.units == other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(id: i32) -> Unit {
        Unit {
            id,
            ..Unit::default()
        }
    }

    fn list(ids: &[i32]) -> ObjectList {
        ids.iter().map(|&id| unit(id)).collect()
    }

    /// Every id's slot as a scan of the list finds it.
    fn indexed_as_scanned(l: &ObjectList) -> bool {
        l.iter()
            .all(|u| l.position_of(u.id) == l.iter().position(|v| v.id == u.id))
    }

    #[test]
    fn a_lookup_by_id_finds_the_object_s_slot() {
        let mut l = list(&[5, 7, 9]);
        assert_eq!(l.position_of(7), Some(1));
        assert_eq!(l.by_id(9).map(|u| u.id), Some(9));
        assert!(l.by_id(8).is_none());
        if let Some(u) = l.by_id_mut(5) {
            u.life.hp = 3.0;
        }
        assert_eq!(l[0].life.hp, 3.0);
        l.push(unit(11));
        assert_eq!(l.position_of(11), Some(3));
        l.extend([unit(12), unit(13)]);
        assert_eq!(l.position_of(13), Some(5));
        assert!(indexed_as_scanned(&l));
    }

    #[test]
    fn the_list_s_removals_keep_every_slot() {
        let mut l = list(&[1, 2, 3, 4, 5, 6]);
        assert_eq!(l.swap_remove(1).id, 2, "the last takes its slot");
        assert_eq!((l.position_of(6), l.position_of(2)), (Some(1), None));
        assert_eq!(l.remove(0).id, 1, "the ones after it move up");
        assert_eq!(l.position_of(6), Some(0));
        assert_eq!(l.pop().map(|u| u.id), Some(5));
        assert!(l.by_id(5).is_none());
        l.truncate(2);
        assert_eq!(l.iter().map(|u| u.id).collect::<Vec<_>>(), [6, 3]);
        assert!(l.by_id(4).is_none());
        l.retain(|u| u.id != 6);
        assert_eq!(l.position_of(3), Some(0));
        let last = l.swap_remove(0);
        assert_eq!((last.id, l.is_empty()), (3, true));
        l.clear();
        assert!(l.by_id(3).is_none());
        assert!(indexed_as_scanned(&list(&[8, 9])));
    }

    #[test]
    fn an_object_out_for_its_update_is_not_in_its_list() {
        let mut l = list(&[1, 2]);
        let out = std::mem::replace(&mut l[0], Unit::taken_out());
        assert!(l.by_id(1).is_none(), "its slot holds the stand-in");
        assert!(l.by_id(TAKEN_OUT).is_none(), "the stand-in is no object");
        l[0] = out;
        assert_eq!(l.position_of(1), Some(0), "put back, it is found again");
    }

    #[test]
    fn a_list_that_held_an_id_twice_finds_its_first_slot() {
        let mut l = list(&[0, 4, 0]);
        assert_eq!(l.position_of(0), Some(0));
        let _ = l.swap_remove(0);
        assert_eq!(
            l.position_of(0),
            Some(0),
            "the other one, which took the slot"
        );
        let _ = l.remove(0);
        assert_eq!(l.position_of(0), None);
        assert_eq!(l.position_of(4), Some(0));
        l[0].id = 6;
        l.reindex();
        assert_eq!(
            l.position_of(6),
            Some(0),
            "an id written in place counts once reindexed"
        );
    }

    #[test]
    fn lists_compare_by_their_objects_in_order() {
        let mut a = list(&[1, 2]);
        let b = list(&[1, 2]);
        assert_eq!(a, b);
        assert_eq!(a, vec![unit(1), unit(2)]);
        assert_eq!(a, [unit(1), unit(2)]);
        let _ = a.swap_remove(0);
        assert_ne!(a, b);
        let ids: Vec<i32> = b.into_iter().map(|u| u.id).collect();
        assert_eq!(ids, [1, 2]);
        let mut c = list(&[3]);
        for u in &mut c {
            u.life.hp = 2.0;
        }
        assert_eq!((&c).into_iter().map(|u| u.life.hp).sum::<f32>(), 2.0);
    }
}
