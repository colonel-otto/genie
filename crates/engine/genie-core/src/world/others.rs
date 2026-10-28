//! The world's objects as one object's update reaches them (FD-045-D19): [`Others`] to change
//! the owner's other objects, [`OthersRef`] to read them, both by id.
//!
//! The game reaches another object by its id (`FUN_1415417a0`, the table at `world+0x168`); a
//! caller asks for the owner's own object ([`OthersRef::own`]), another player's
//! ([`OthersRef::foreign`]) or anyone's ([`OthersRef::find`]), never for a list position.

use crate::objects::object_list::ObjectList;
use crate::objects::unit::{TAKEN_OUT, Unit};
use crate::unit_ai::world_view::Foreign;
use crate::world::player::Player;

/// The objects besides the one that updates, to read: its owner's other live objects in their
/// list's order and the other players' live and sleeping ones.
#[derive(Clone, Copy)]
pub(crate) struct OthersRef<'a> {
    /// The owner, as the update reads it (a captured unit's new one).
    player: usize,
    /// The owner's live list (the updating object's slot holds its stand-in, [`TAKEN_OUT`]).
    own: &'a ObjectList,
    /// The other players' lists.
    foreign: Foreign<'a>,
}

impl<'a> OthersRef<'a> {
    /// Player `player`'s view: its live list `own` and the other players' `foreign`.
    pub(crate) const fn new(player: usize, own: &'a ObjectList, foreign: Foreign<'a>) -> Self {
        Self {
            player,
            own,
            foreign,
        }
    }

    /// Player `player`'s view of its live list `own` alone, no other player's.
    #[cfg(test)]
    pub(crate) const fn alone(player: usize, own: &'a ObjectList) -> Self {
        let foreign = Foreign {
            before: &[],
            after: &[],
            left: None,
        };
        Self::new(player, own, foreign)
    }

    /// Player `p`'s view of the world's `players`.
    ///
    /// # Panics
    ///
    /// When `players` holds no player `p`.
    pub(crate) const fn of(players: &'a [Player], p: usize) -> Self {
        let (before, rest) = players.split_at(p);
        let (player, after) = rest.split_first().expect("the view's player");
        let foreign = Foreign {
            before,
            after,
            left: None,
        };
        Self::new(p, &player.objects, foreign)
    }

    /// The view of player `new`, which took the updating object over in its update, with its
    /// live list `list`: this view's owner's list counts among the other players' (`left`).
    pub(crate) const fn changed_hands(self, new: usize, list: &'a ObjectList) -> Self {
        Self {
            player: new,
            own: list,
            foreign: Foreign {
                left: Some((self.player, self.own)),
                ..self.foreign
            },
        }
    }

    /// The owner's live object `id`.
    pub(crate) fn own(&self, id: i32) -> Option<&'a Unit> {
        self.own.by_id(id)
    }

    /// The owner's live objects in their list's order, the updating one aside.
    pub(crate) fn own_iter(&self) -> impl Iterator<Item = &'a Unit> + use<'a> {
        self.own.iter().filter(|u| u.id != TAKEN_OUT)
    }

    /// The other players' live and sleeping objects with their players: the players in their
    /// order, each one's live list before its sleeping one, then the list `left` names.
    pub(crate) fn foreign_iter(&self) -> impl Iterator<Item = (usize, &'a Unit)> + use<'a> {
        let Foreign {
            before,
            after,
            left,
        } = self.foreign;
        let skip = before.len() + 1;
        let before = before.iter().enumerate();
        let after = after.iter().enumerate().map(move |(q, p)| (q + skip, p));
        let lists = before
            .chain(after)
            .flat_map(|(q, p)| p.objects.iter().chain(&p.sleeping).map(move |u| (q, u)));
        let left = left.into_iter().flat_map(|(q, list)| {
            let list = list.iter().filter(|u| u.id != TAKEN_OUT);
            list.map(move |u| (q, u))
        });
        lists.chain(left)
    }

    /// Another player's live or sleeping object `id`, with that player
    /// ([`OthersRef::foreign_iter`]'s first).
    pub(crate) fn foreign(&self, id: i32) -> Option<(usize, &'a Unit)> {
        let Foreign {
            before,
            after,
            left,
        } = self.foreign;
        let skip = before.len() + 1;
        let before = before.iter().enumerate();
        let after = after.iter().enumerate().map(move |(q, p)| (q + skip, p));
        let mut players = before.chain(after);
        players
            .find_map(|(q, p)| {
                p.objects
                    .by_id(id)
                    .or_else(|| p.sleeping.by_id(id))
                    .map(|u| (q, u))
            })
            .or_else(|| left.and_then(|(q, list)| list.by_id(id).map(|u| (q, u))))
    }

    /// Every object the view holds: the owner's, then the other players'.
    pub(crate) fn every(&self) -> impl Iterator<Item = &'a Unit> + use<'a> {
        self.own_iter().chain(self.foreign_iter().map(|(_, u)| u))
    }

    /// Object `id` with its owner: the owner's own, else another player's.
    pub(crate) fn find(&self, id: i32) -> Option<(usize, &'a Unit)> {
        self.own(id)
            .map(|u| (self.player, u))
            .or_else(|| self.foreign(id))
    }

    /// Object `id`, the owner's own or another player's.
    pub(crate) fn get(&self, id: i32) -> Option<&'a Unit> {
        self.find(id).map(|(_, u)| u)
    }

    /// The other players, whose data (diplomacy, attributes) the view's readers read.
    pub(crate) const fn players(&self) -> Foreign<'a> {
        self.foreign
    }
}

/// The objects besides the one that updates, to change: what [`OthersRef`] reads, with the
/// owner's other live objects and the other players to write.
pub(crate) struct Others<'a> {
    player: usize,
    own: &'a mut ObjectList,
    /// The owner's sleeping list, which only a write by id reaches.
    own_sleeping: &'a mut ObjectList,
    /// The players before the owner and after it.
    before: &'a mut [Player],
    after: &'a mut [Player],
}

impl<'a> Others<'a> {
    /// Player `player`'s view: its live list `own`, its sleeping list `own_sleeping` and the
    /// players `before` and `after` it.
    pub(crate) const fn new(
        player: usize,
        (own, own_sleeping): (&'a mut ObjectList, &'a mut ObjectList),
        (before, after): (&'a mut [Player], &'a mut [Player]),
    ) -> Self {
        Self {
            player,
            own,
            own_sleeping,
            before,
            after,
        }
    }

    /// The view to read.
    pub(crate) const fn view(&self) -> OthersRef<'_> {
        let foreign = Foreign {
            before: self.before,
            after: self.after,
            left: None,
        };
        OthersRef::new(self.player, self.own, foreign)
    }

    /// [`OthersRef::own`].
    pub(crate) fn own(&self, id: i32) -> Option<&Unit> {
        self.view().own(id)
    }

    /// [`OthersRef::own_iter`].
    pub(crate) fn own_iter(&self) -> impl Iterator<Item = &Unit> {
        self.view().own_iter()
    }

    /// [`OthersRef::find`].
    pub(crate) fn find(&self, id: i32) -> Option<(usize, &Unit)> {
        self.view().find(id)
    }

    /// Object `id` of any list, live or sleeping, with its owner, to change: what the world's
    /// table by id (`FUN_1415417a0`) gives the game's call on it.
    pub(crate) fn get_mut(&mut self, id: i32) -> Option<(usize, &mut Unit)> {
        let p = self.player;
        if let Some(i) = self.own.position_of(id) {
            return Some((p, &mut self.own[i]));
        }
        if let Some(i) = self.own_sleeping.position_of(id) {
            return Some((p, &mut self.own_sleeping[i]));
        }
        let skip = self.before.len() + 1;
        let before = self.before.iter_mut().enumerate();
        let after = self
            .after
            .iter_mut()
            .enumerate()
            .map(|(q, pl)| (q + skip, pl));
        before.chain(after).find_map(|(q, pl)| {
            let Player {
                objects, sleeping, ..
            } = pl;
            objects
                .by_id_mut(id)
                .or_else(|| sleeping.by_id_mut(id))
                .map(|u| (q, u))
        })
    }

    /// The owner's live object `id`, to change.
    pub(crate) fn own_mut(&mut self, id: i32) -> Option<&mut Unit> {
        self.own.by_id_mut(id)
    }

    /// The owner's live objects in their list's order, the updating one aside, to change.
    pub(crate) fn own_iter_mut(&mut self) -> impl Iterator<Item = &mut Unit> {
        self.own.iter_mut().filter(|u| u.id != TAKEN_OUT)
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

    fn player(live: &[i32], sleeping: &[i32]) -> Player {
        Player {
            objects: live.iter().map(|&id| unit(id)).collect(),
            sleeping: sleeping.iter().map(|&id| unit(id)).collect(),
            ..Player::default()
        }
    }

    /// Player 1 updates object 11 (taken out); player 0 holds 1 live and 2 asleep, player 2
    /// holds 20 and, again, 12.
    fn world() -> (ObjectList, [Player; 1], [Player; 1]) {
        let mut own = ObjectList::from(vec![unit(10), Unit::taken_out(), unit(12)]);
        own[2].life.hp = 5.0;
        let mut theirs = player(&[20, 12], &[]);
        theirs.objects[1].life.hp = 7.0;
        (own, [player(&[1], &[2])], [theirs])
    }

    #[test]
    fn a_lookup_finds_the_owner_s_object_before_another_player_s() {
        let (own, before, after) = world();
        let foreign = Foreign {
            before: &before,
            after: &after,
            left: None,
        };
        let v = OthersRef::new(1, &own, foreign);
        assert_eq!(v.own(10).map(|u| u.id), Some(10));
        assert!(v.own(1).is_none(), "another player's is not the owner's");
        assert_eq!(v.find(12).map(|(q, u)| (q, u.life.hp)), Some((1, 5.0)));
        assert_eq!(v.foreign(12).map(|(q, u)| (q, u.life.hp)), Some((2, 7.0)));
        assert_eq!(v.find(2).map(|(q, u)| (q, u.id)), Some((0, 2)));
        assert_eq!(v.find(20).map(|(q, _)| q), Some(2));
        assert_eq!(v.get(1).map(|u| u.id), Some(1));
        assert_eq!(v.players().player(2).map(|p| p.objects.len()), Some(2));
        assert!(
            v.players().player(1).is_none(),
            "the owner's data is the update's own"
        );
        let foreign: Vec<(usize, i32)> = v.foreign_iter().map(|(q, u)| (q, u.id)).collect();
        assert_eq!(foreign, [(0, 1), (0, 2), (2, 20), (2, 12)]);
        let every: Vec<i32> = v.every().map(|u| u.id).collect();
        assert_eq!(every, [10, 12, 1, 2, 20, 12]);
        assert!(
            v.get(11).is_none(),
            "the updating object is out of its list"
        );
        assert!(v.find(TAKEN_OUT).is_none(), "nor is its stand-in");
    }

    #[test]
    fn the_owner_s_objects_iterate_in_list_order_without_the_one_that_updates() {
        let (mut own, mut before, mut after) = world();
        let mut asleep = ObjectList::from(vec![unit(13)]);
        let mut others = Others::new(1, (&mut own, &mut asleep), (&mut before, &mut after));
        let ids: Vec<i32> = others.own_iter().map(|u| u.id).collect();
        assert_eq!(ids, [10, 12]);
        for u in others.own_iter_mut() {
            u.life.hp += 1.0;
        }
        if let Some(u) = others.own_mut(10) {
            u.life.hp = 9.0;
        }
        assert!(others.own_mut(20).is_none());
        assert_eq!(others.own(10).map(|u| u.life.hp), Some(9.0));
        for (id, owner) in [(12, 1), (13, 1), (2, 0), (20, 2)] {
            let (q, u) = others.get_mut(id).expect("any list's object");
            u.life.hp = 4.0;
            assert_eq!(q, owner);
        }
        assert!(others.get_mut(11).is_none(), "the updating object is out");
        assert!(
            others.view().get(13).is_none(),
            "a read leaves the owner's sleepers out"
        );
        assert_eq!(others.view().find(2).map(|(_, u)| u.life.hp), Some(4.0));
        assert_eq!(others.find(12).map(|(_, u)| u.life.hp), Some(4.0));
        assert_eq!(others.find(20).map(|(q, _)| q), Some(2));
        assert_eq!(others.view().foreign(10).map(|(q, _)| q), None);
        assert_eq!(own[1].id, TAKEN_OUT, "the stand-in stays as it was");
    }

    #[test]
    fn a_captured_object_reads_its_new_owner_s_list_as_its_own() {
        let (own, before, after) = world();
        let foreign = Foreign {
            before: &before,
            after: &after,
            left: None,
        };
        let v = OthersRef::new(1, &own, foreign).changed_hands(2, &after[0].objects);
        assert_eq!(v.own(20).map(|u| u.id), Some(20));
        assert_eq!(v.find(12).map(|(q, u)| (q, u.life.hp)), Some((2, 7.0)));
        assert_eq!(
            v.find(10).map(|(q, _)| q),
            Some(1),
            "the old owner's list is left"
        );
        assert_eq!(v.own_iter().count(), 2);
        assert!(
            v.find(TAKEN_OUT).is_none(),
            "the left list's stand-in is no object"
        );
    }
}
