//! The player's object tracker (`player+0x18`, the "Object Tracker"): lists of the player's
//! object ids by key, in the order the ids came in. A key is a master (the one its key master
//! stands for, [`key`]) or a class + 900; a building waits under its key + 90000 (its class +
//! 90900) until its completion moves it. The tracker is not saved: the load files every object
//! of the player's lists in order (live, sleeping, then the ghosts), each building completed
//! (`build_pts` at least its build time) at the end of its own load.
//!
//! What files and moves ids, and where genie-core runs it:
//! - the player's add (slot 0x2d0, `FUN_14165cc80` -> `FUN_14104e7f0`, [`Tracker::add`]): a new
//!   object, a woken one, one going to sleep (after its removal), the new owner of one changing
//!   hands, and the load;
//! - the player's remove (slot 0x2d8, `FUN_141668040` -> `FUN_14104fff0`, [`Tracker::remove`]):
//!   an object leaving the world, sleep and wake, the old owner;
//! - a building's completion's first part (`FUN_1411c6bd0` -> `FUN_14104eac0`,
//!   [`Tracker::complete`]), a building's load and its owner change (`FUN_1411df160`,
//!   `FUN_1411cd860`);
//! - a master change (`FUN_1411a6a70` -> `FUN_14104fb20`, [`Tracker::promote`]);
//! - an upgrade to master 0x67f or 0x6a1 (`FUN_141657d10` -> `FUN_14104fd10`,
//!   [`Tracker::move_all`]).
//!
//! - the production queue's places: a queued unit files a −1 at the end of its master's pending
//!   list and its class's (`FUN_141050230`, [`Tracker::place`]), its spawn or cancel drops the
//!   last of each (`FUN_14104f330`, [`Tracker::unplace`]); the counter `+0x60`
//!   (`FUN_14104fb00`, [`Tracker::count_queued`]) follows the same writers.
//!
//! Not held: the ghosts (they file under the ghosts' master 243) and the counters at `+0x64` to
//! `+0x6c`. A reader of the done lists ([`Tracker::list`], [`Tracker::class_list`]) needs none of
//! them.
//!
//! 158041: the key `FUN_141658430`, a master's list `FUN_14104f960` (an empty one, `+0x40`, for
//! a key it lacks: `FUN_14104dd80`, `FUN_141050ae0`), a class's `FUN_14104f8f0`, the lists'
//! erase `FUN_140a63bd0` (order kept).

use crate::objects::lines::UnitLines;
use crate::objects::masters::Masters;

/// A pending key is its done key + this (a master's + 90000, a class's + 90900).
pub const PENDING: i32 = 90_000;
/// A class's key is the class + this.
pub const CLASS_KEY: i32 = 900;
/// The keys run below this master id (`FUN_141658430`: 9000 or more is none).
const TRACKED_MASTERS: i32 = 9000;
/// The buildings' type, which wait in the pending lists.
const BUILDING: u8 = 80;
/// The classes a key holds below this (`FUN_141658430`).
const CLASSES: i16 = 100;
/// The classes that count as done from the start (packed and unpacked siege, 51 and 54).
const NEVER_PENDING: [i16; 2] = [51, 54];
/// The classes whose key carries the military byte (`+9`, `FUN_141658430`'s switch).
const MILITARY: [i16; 25] = [
    0, 6, 12, 17, 18, 20, 22, 23, 24, 25, 26, 28, 35, 36, 43, 44, 45, 46, 47, 50, 52, 53, 56, 57,
    59,
];

/// The key `FUN_141658430` gives a master (12 bytes: the key master, its class, two bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    /// `+0`: the key master (−1 for none).
    pub master: i32,
    /// `+4`: the key master's class (−1 for none).
    pub class: i32,
    /// `+8`: the object waits in the pending lists until its completion.
    pub pending: bool,
    /// `+9`: the key master's class is a military one.
    pub military: bool,
}

impl Key {
    /// The key of a master the player lacks (`FUN_141658430`'s −1, −1, 0).
    pub const NONE: Self = Self {
        master: -1,
        class: -1,
        pending: false,
        military: false,
    };
}

/// `FUN_141658430`: the key of player's master `master` (`masters`, the player's table; None
/// gives [`Key::NONE`]). A building (type 80) keys by its stack unit (`+0x986`) when it names
/// one (a stack unit the player lacks, or out of 0 to 8999, gives none) and waits in the pending
/// lists; an annex (a head unit, `+0x9b8`, and no annex of its own) files under no class. Any
/// other master keys by its line's first master ([`UnitLines::base`]). The key master must be one
/// the player has, below 9000; its class below 100 is the key's class, and classes 51 and 54 do
/// not wait.
#[must_use]
pub fn key(masters: Option<&Masters>, lines: &UnitLines, master: i16) -> Key {
    let get = |id: i32| {
        masters?
            .get(i16::try_from(id).ok()?)
            .filter(|_| (0..TRACKED_MASTERS).contains(&id))
    };
    let Some(m) = get(i32::from(master)) else {
        return Key::NONE;
    };
    let mut out = Key::NONE;
    let mut id = i32::from(master);
    let mut classed = true;
    if m.kind == BUILDING {
        let (stack, head, first_annex) = m
            .building
            .as_ref()
            .map_or((-1, -1, -1), |b| (b.stack_unit, b.head_unit, b.annexes[0]));
        if stack != -1 {
            id = i32::from(stack);
            if get(id).is_none() {
                return Key::NONE;
            }
        }
        if head != -1 && first_annex == -1 {
            classed = false;
        }
        out.pending = true;
    } else {
        id = lines.base(id);
    }
    let Some(k) = get(id) else {
        return Key::NONE;
    };
    out.master = id;
    if classed && (0..CLASSES).contains(&k.class) {
        out.class = i32::from(k.class);
        if NEVER_PENDING.contains(&k.class) {
            out.pending = false;
            out.military = false;
        }
        if MILITARY.contains(&k.class) {
            out.military = true;
        }
    }
    out
}

/// A player's tracker: the lists by key ([`key`]'s master, a class + 900, each + 90000 while
/// pending), each in the order its ids came in.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tracker {
    lists: std::collections::BTreeMap<i32, Vec<i32>>,
    /// `+0x60` (`FUN_14104fb00`): what the production queues' writers count, units queued and
    /// spawned or cancelled (a cancel of several counts them twice, as the game does).
    pub queued: i32,
}

impl Tracker {
    /// The list of key `key` (a key the tracker lacks: the empty one, `+0x40`).
    #[must_use]
    pub fn list(&self, key: i32) -> &[i32] {
        self.lists.get(&key).map_or(&[], Vec::as_slice)
    }

    /// `FUN_14104f8f0`: the done objects filed under class `class` (key class + 900).
    #[must_use]
    pub fn class_list(&self, class: i32) -> &[i32] {
        self.list(class + CLASS_KEY)
    }

    fn push(&mut self, key: i32, id: i32) {
        self.lists.entry(key).or_default().push(id);
    }

    /// `FUN_140a63bd0` on the first `id` of list `key`: the ids after it move up.
    fn erase(&mut self, key: i32, id: i32) {
        if let Some(list) = self.lists.get_mut(&key)
            && let Some(k) = list.iter().position(|&x| x == id)
        {
            let _ = list.remove(k);
        }
    }

    fn holds(&self, key: i32, id: i32) -> bool {
        self.list(key).contains(&id)
    }

    /// `FUN_14104e7f0`: `id` joins the end of its key master's list and of its class's (when it
    /// has one), each + 90000 while pending. (The game logs a key master of 900 to 999, which a
    /// class's list uses.)
    pub fn add(&mut self, id: i32, key: Key) {
        let wait = if key.pending { PENDING } else { 0 };
        self.push(key.master + wait, id);
        if key.class != -1 {
            self.push(key.class + CLASS_KEY + wait, id);
        }
    }

    /// `FUN_14104eac0`: a pending `id` leaves its pending lists for the end of its done ones
    /// (the master's, then the class's), unless already there; a key that does not wait
    /// changes nothing.
    pub fn complete(&mut self, id: i32, key: Key) {
        if !key.pending {
            return;
        }
        self.erase(key.master + PENDING, id);
        if !self.holds(key.master, id) {
            self.push(key.master, id);
        }
        if key.class != -1 {
            let class = key.class + CLASS_KEY;
            self.erase(class + PENDING, id);
            if !self.holds(class, id) {
                self.push(class, id);
            }
        }
    }

    /// `FUN_14104fff0`: `id` leaves its class's lists (the pending one too while its key waits)
    /// and its key master's (the pending one first). (A tracker in teardown, its `+0xb0`, is
    /// not held.)
    pub fn remove(&mut self, id: i32, key: Key) {
        if key.class != -1 {
            let class = key.class + CLASS_KEY;
            self.erase(class, id);
            if key.pending {
                self.erase(class + PENDING, id);
            }
        }
        if key.pending {
            self.erase(key.master + PENDING, id);
        }
        self.erase(key.master, id);
    }

    /// `FUN_14104fb00`: the counter of queued units (`+0x60`) changes by `n`.
    pub fn count_queued(&mut self, n: i16) {
        self.queued += i32::from(n);
    }

    /// `FUN_141050230`: a queued unit of master `master` (its [`key`]'s class `class`, −1 for
    /// none) files a −1 at the end of the master's pending list (+ 90000, the master itself, not
    /// its key master) and of the class's (+ 90900).
    pub fn place(&mut self, master: i16, class: i32) {
        self.push(i32::from(master) + PENDING, -1);
        if class != -1 {
            self.push(class + CLASS_KEY + PENDING, -1);
        }
    }

    /// `FUN_14104f330`: a queued unit's spawn or cancel drops the last id of the same lists
    /// (`FUN_140a63bd0` at the end; an empty list stays empty).
    pub fn unplace(&mut self, master: i16, class: i32) {
        let mut drop_last = |key: i32| {
            if let Some(list) = self.lists.get_mut(&key) {
                let _ = list.pop();
            }
        };
        drop_last(i32::from(master) + PENDING);
        if class != -1 {
            drop_last(class + CLASS_KEY + PENDING);
        }
    }

    /// `FUN_14104fb20`: an object changes masters. Out of its old key master's done list it goes
    /// to the end of the new one's, and from the old class's done list to the end of the new
    /// one's (a key without a class has none); one the old done list does not hold stays where
    /// it is (the game logs "Skipping promotion on untracked object.").
    pub fn promote(&mut self, id: i32, old: Key, new: Key) {
        if !self.holds(old.master, id) {
            return;
        }
        self.erase(old.master, id);
        self.push(new.master, id);
        if old.class != -1 {
            self.erase(old.class + CLASS_KEY, id);
        }
        if new.class != -1 {
            self.push(new.class + CLASS_KEY, id);
        }
    }

    /// `FUN_14104fd10`: every id of list `old` the world holds (`class_of` gives its master's
    /// class) goes, in order, to the end of list `new`, out of the done list of `old_class` (the
    /// class of [`key`] of master `old`, −1 for none: key 899) and to the end of its own class's;
    /// list `old` then empties. Below the pending keys the same runs for the pending keys of
    /// both (each + 90000), whose class [`key`] gives none (a pending id goes to its class's done
    /// list). Its one caller (`FUN_141657d10`, the upgrades to masters 0x67f and 0x6a1) passes two masters;
    /// the same key twice would loop for ever in the game.
    pub fn move_all(
        &mut self,
        old: i32,
        new: i32,
        old_class: i32,
        class_of: &dyn Fn(i32) -> Option<i16>,
    ) {
        if let Some(ids) = self.lists.get(&old).cloned() {
            for id in ids {
                let Some(class) = (0 <= id).then(|| class_of(id)).flatten() else {
                    continue;
                };
                self.push(new, id);
                self.erase(old_class + CLASS_KEY, id);
                self.push(i32::from(class) + CLASS_KEY, id);
            }
            if let Some(list) = self.lists.get_mut(&old) {
                list.clear();
            }
        }
        if old < PENDING {
            self.move_all(old + PENDING, new + PENDING, Key::NONE.class, class_of);
        }
    }
}

impl crate::world::World {
    /// [`key`] of player `p`'s master `master`.
    #[must_use]
    pub fn tracker_key(&self, p: usize, master: i16) -> Key {
        key(self.masters.get(p), &self.lines, master)
    }

    /// The load's filing: each player's objects as its lists hold them, the live ones then the
    /// sleeping ones, each added at the end of its own load (`FUN_14119df60`); a building's load
    /// (`FUN_1411df160`) then places each unit of its queue's entries of a master above 0
    /// ([`Tracker::place`], one count each) and, a building whose build points (`+0xc50`) reach
    /// its build time (`COMISS`/`JC`: a NaN does not), completes it at its end.
    pub fn load_trackers(&mut self) {
        for p in 0..self.players.len() {
            let mut tracker = Tracker::default();
            let player = &self.players[p];
            for u in player.objects.iter().chain(&player.sleeping) {
                let k = self.tracker_key(p, u.life.master_id);
                tracker.add(u.id, k);
                for e in u.production.iter().flat_map(|q| q.queue.entries()) {
                    if 0 < e.master && 0 < e.count {
                        let class = self.tracker_key(p, e.master).class;
                        for _ in 0..e.count {
                            tracker.place(e.master, class);
                            tracker.count_queued(1);
                        }
                    }
                }
                let building = self
                    .masters
                    .get(p)
                    .and_then(|m| m.get(u.life.master_id))
                    .is_some_and(|m| m.kind == BUILDING);
                if building
                    && u.construction
                        .is_some_and(|c| f32::from(c.build_time) <= c.progress)
                {
                    tracker.complete(u.id, k);
                }
            }
            self.players[p].tracker = tracker;
        }
    }

    /// The player's add (slot 0x2d0, `FUN_14165cc80`'s tracker part) of its object `id` of
    /// master `master`.
    pub(crate) fn track_add(&mut self, p: usize, id: i32, master: i16) {
        let k = self.tracker_key(p, master);
        self.players[p].tracker.add(id, k);
    }

    /// The player's remove (slot 0x2d8, `FUN_141668040`'s tracker part) of its object `id` of
    /// master `master`.
    pub(crate) fn track_remove(&mut self, p: usize, id: i32, master: i16) {
        let k = self.tracker_key(p, master);
        self.players[p].tracker.remove(id, k);
    }

    /// An object moving between the player's live and sleeping lists (its sleep, the static
    /// object's update; its wake, `FUN_14119deb0`): the player's remove (slot 0x2d8), then its
    /// add (slot 0x2d0), which files it last.
    pub(crate) fn track_relist(&mut self, p: usize, id: i32, master: i16) {
        let k = self.tracker_key(p, master);
        let tracker = &mut self.players[p].tracker;
        tracker.remove(id, k);
        tracker.add(id, k);
    }

    /// A building's completion's first part (`FUN_1411c6bd0`'s tracker part).
    pub(crate) fn track_complete(&mut self, p: usize, id: i32, master: i16) {
        let k = self.tracker_key(p, master);
        self.players[p].tracker.complete(id, k);
    }

    /// A master change (`FUN_1411a6a70`'s tracker part) of player `p`'s object `id` from master
    /// `old` to `new`.
    pub(crate) fn track_promote(&mut self, p: usize, id: i32, old: i16, new: i16) {
        let (old, new) = (self.tracker_key(p, old), self.tracker_key(p, new));
        self.players[p].tracker.promote(id, old, new);
    }

    /// `FUN_14104fd10` on player `p`'s tracker from master `from`'s list to `to`'s
    /// ([`Tracker::move_all`]): an object's class is its master's, the world's object
    /// (`FUN_1415417a0`) of any player.
    pub(crate) fn track_move_all(&mut self, p: usize, from: i16, to: i16) {
        let old_class = self.tracker_key(p, from).class;
        let mut tracker = std::mem::take(&mut self.players[p].tracker);
        let class_of = |id: i32| {
            let (q, u) = self.object(id)?;
            Some(self.masters.get(q)?.get(u.life.master_id)?.class)
        };
        tracker.move_all(i32::from(from), i32::from(to), old_class, &class_of);
        self.players[p].tracker = tracker;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objects::lines::UnitLine;
    use crate::objects::masters::{BuildingMaster, Master};

    fn building(id: i16, class: i16, stack: i16, head: i16, annex: i16) -> Master {
        Master {
            id,
            kind: BUILDING,
            class,
            building: Some(BuildingMaster {
                stack_unit: stack,
                head_unit: head,
                annexes: [annex, -1, -1, -1],
                ..BuildingMaster::default()
            }),
            ..Master::default()
        }
    }

    fn unit(id: i16, class: i16) -> Master {
        Master {
            id,
            kind: 70,
            class,
            ..Master::default()
        }
    }

    fn masters() -> Masters {
        Masters::new([
            unit(4, 0),
            unit(24, 0),
            unit(83, 4),
            unit(118, 4),
            building(79, 52, -1, -1, -1),
            building(234, 52, -1, -1, -1),
            building(109, 3, -1, -1, 618),
            building(618, 3, -1, 109, -1),
            building(1189, 3, 79, -1, -1),
            building(1190, 3, 9999, -1, -1),
            building(42, 54, -1, -1, -1),
            building(43, 130, -1, -1, -1),
            unit(331, 54),
        ])
    }

    fn lines() -> UnitLines {
        UnitLines::build(
            2000,
            &[UnitLine {
                line: Some(-299),
                chain: Some(vec![4, 24, 492]),
                ..UnitLine::default()
            }],
        )
    }

    /// `FUN_141050230` files a −1 under the queued master itself (+ 90000, the crossbowman's
    /// own 24, not its key master 4) and its key's class (+ 90900); `FUN_14104f330` drops the
    /// last of each, whatever it is, and leaves an empty list alone.
    #[test]
    fn a_queued_unit_places_a_minus_one_and_its_spawn_drops_the_last() {
        let mut t = Tracker::default();
        t.place(24, 0);
        t.place(24, 0);
        t.place(83, -1);
        assert_eq!(t.list(24 + PENDING), [-1, -1]);
        assert_eq!(t.list(CLASS_KEY + PENDING), [-1, -1]);
        assert_eq!(t.list(83 + PENDING), [-1]);
        t.lists.get_mut(&(CLASS_KEY + PENDING)).unwrap().push(5);
        t.unplace(24, 0);
        assert_eq!(t.list(24 + PENDING), [-1]);
        assert_eq!(t.list(CLASS_KEY + PENDING), [-1, -1]);
        t.unplace(83, -1);
        t.unplace(83, -1);
        assert_eq!(t.list(83 + PENDING), []);
    }

    #[test]
    fn a_unit_keys_by_its_lines_first_master_and_a_building_by_its_stack_unit() {
        let (m, l) = (masters(), lines());
        let k = |id| key(Some(&m), &l, id);
        // the crossbowman (24) files under the archer (4), class 0 a military one
        assert_eq!(
            k(24),
            Key {
                master: 4,
                class: 0,
                pending: false,
                military: true
            }
        );
        assert_eq!(
            k(83),
            Key {
                master: 83,
                class: 4,
                pending: false,
                military: false
            }
        );
        // a building waits; a stack unit is its key master (with its class)
        assert_eq!(
            k(109),
            Key {
                master: 109,
                class: 3,
                pending: true,
                military: false
            }
        );
        assert_eq!(
            k(1189),
            Key {
                master: 79,
                class: 52,
                pending: true,
                military: true
            }
        );
        // an annex files under no class; a stack unit out of range gives none
        assert_eq!(
            k(618),
            Key {
                master: 618,
                class: -1,
                pending: true,
                military: false
            }
        );
        assert_eq!(k(1190), Key::NONE);
        // classes 51 and 54 do not wait; a class of 100 or more is none and the building waits
        assert_eq!((k(42).pending, k(42).class), (false, 54));
        assert_eq!((k(43).pending, k(43).class), (true, -1));
        assert_eq!((k(331).pending, k(331).class), (false, 54));
        // a master the player lacks, a line whose first master it lacks, no table
        assert_eq!(k(7), Key::NONE);
        assert_eq!(k(492), Key::NONE);
        assert_eq!(key(None, &l, 83), Key::NONE);
    }

    #[test]
    fn a_building_waits_until_its_completion_moves_it_to_the_end_of_the_done_lists() {
        let (m, l) = (masters(), lines());
        let tc = key(Some(&m), &l, 109);
        let mut t = Tracker::default();
        t.add(10, tc);
        t.add(11, tc);
        assert_eq!(
            (t.list(109), t.list(109 + PENDING)),
            (&[][..], &[10, 11][..])
        );
        assert_eq!(t.list(3 + CLASS_KEY + PENDING), [10, 11]);
        t.complete(11, tc);
        t.complete(10, tc);
        assert_eq!(
            (t.list(109), t.list(109 + PENDING)),
            (&[11, 10][..], &[][..])
        );
        assert_eq!(t.class_list(3), [11, 10]);
        // a second completion leaves the order as it is
        t.complete(11, tc);
        assert_eq!(t.list(109), [11, 10]);
        // a key that does not wait changes nothing
        let villager = key(Some(&m), &l, 83);
        t.add(20, villager);
        t.complete(20, villager);
        assert_eq!(t.class_list(4), [20]);
    }

    #[test]
    fn a_removal_keeps_the_others_order() {
        let (m, l) = (masters(), lines());
        let v = key(Some(&m), &l, 83);
        let mut t = Tracker::default();
        for id in [1, 2, 3, 4] {
            t.add(id, v);
        }
        t.remove(2, v);
        assert_eq!(
            (t.list(83), t.class_list(4)),
            (&[1, 3, 4][..], &[1, 3, 4][..])
        );
        // a pending building leaves both its lists, done or not
        let tc = key(Some(&m), &l, 109);
        t.add(30, tc);
        t.add(31, tc);
        t.complete(31, tc);
        t.remove(30, tc);
        t.remove(31, tc);
        assert_eq!(
            (t.list(109), t.list(109 + PENDING), t.class_list(3)),
            (&[][..], &[][..], &[][..])
        );
    }

    #[test]
    fn a_promotion_moves_an_object_to_the_end_of_its_new_lists() {
        let (m, l) = (masters(), lines());
        let (villager, builder) = (key(Some(&m), &l, 83), key(Some(&m), &l, 118));
        let mut t = Tracker::default();
        for id in [1, 2, 3] {
            t.add(id, villager);
        }
        // villager 1 becomes a builder: class 4's list takes it last
        t.promote(1, villager, builder);
        assert_eq!((t.list(83), t.list(118)), (&[2, 3][..], &[1][..]));
        assert_eq!(t.class_list(4), [2, 3, 1]);
        // an archer upgraded in its line stays under the archer, last
        let archer = key(Some(&m), &l, 4);
        let crossbow = key(Some(&m), &l, 24);
        t.add(7, archer);
        t.add(8, archer);
        t.promote(7, archer, crossbow);
        assert_eq!((t.list(4), t.class_list(0)), (&[8, 7][..], &[8, 7][..]));
        // an object the old list does not hold is skipped
        t.promote(99, villager, builder);
        assert_eq!(t.list(118), [1]);
    }

    fn world() -> crate::world::World {
        use crate::objects::unit::Unit;
        use crate::world::{Player, World};
        let unit = |id: i32, master: i16, progress: Option<f32>| {
            let mut u = Unit {
                id,
                construction: progress.map(|progress| crate::construction::build::Construction {
                    progress,
                    desolid: false,
                    builders: 0,
                    hit_points: 2400,
                    build_time: 150,
                    record: crate::map::spatial::Obstruction { kind: 0, class: 0 },
                    snow: false,
                    sprite_override: false,
                    started: false,
                }),
                ..Unit::default()
            };
            u.life.master_id = master;
            u
        };
        let player = Player {
            objects: vec![
                unit(10, 109, Some(150.0)),
                unit(11, 83, None),
                unit(12, 109, Some(149.9)),
                unit(13, 118, None),
                unit(14, 109, Some(f32::NAN)),
            ]
            .into(),
            sleeping: vec![unit(15, 83, None)].into(),
            ..Player::default()
        };
        let mut t = World::new(0, 0, 0, vec![0; 2], vec![Player::default(), player]);
        t.masters = vec![masters(), masters()];
        t.lines = lines();
        t
    }

    #[test]
    fn the_load_files_the_live_objects_then_the_sleeping_ones_and_completes_the_built() {
        let mut t = world();
        t.load_trackers();
        let tracker = &t.players[1].tracker;
        // build points at the build time complete; below it, or NaN, the building waits
        assert_eq!(
            (tracker.list(109), tracker.list(109 + PENDING)),
            (&[10][..], &[12, 14][..])
        );
        assert_eq!(tracker.class_list(4), [11, 13, 15]);
        assert_eq!(
            (tracker.list(83), tracker.list(118)),
            (&[11, 15][..], &[13][..])
        );
        assert_eq!(t.players[0].tracker, Tracker::default());
    }

    #[test]
    fn a_relisted_object_goes_last_and_a_completed_one_joins_its_done_lists() {
        let mut t = world();
        t.load_trackers();
        t.track_relist(1, 11, 83);
        assert_eq!(t.players[1].tracker.class_list(4), [13, 15, 11]);
        t.track_complete(1, 12, 109);
        assert_eq!(t.players[1].tracker.list(109), [10, 12]);
        t.track_promote(1, 15, 83, 118);
        assert_eq!(
            (
                t.players[1].tracker.list(118),
                t.players[1].tracker.class_list(4)
            ),
            (&[13, 15][..], &[13, 11, 15][..])
        );
        t.track_remove(1, 13, 118);
        t.track_add(1, 16, 83);
        assert_eq!(t.players[1].tracker.class_list(4), [11, 15, 16]);
    }

    #[test]
    fn a_move_of_all_reads_each_objects_class_from_the_world() {
        let mut t = world();
        t.load_trackers();
        // the villagers' list (83) moves to 118's; their class (4) stays theirs
        t.track_move_all(1, 83, 118);
        let tracker = &t.players[1].tracker;
        assert_eq!(
            (tracker.list(83), tracker.list(118)),
            (&[][..], &[13, 11, 15][..])
        );
        assert_eq!(tracker.class_list(4), [13, 11, 15]);
    }

    #[test]
    fn a_move_of_all_takes_the_list_and_its_pending_one_and_empties_them() {
        let mut t = Tracker::default();
        t.push(500, 1);
        t.push(500, -1);
        t.push(500, 2);
        t.push(500, 3);
        t.push(5 + CLASS_KEY, 1);
        t.push(5 + CLASS_KEY, 3);
        t.push(500 + PENDING, 4);
        t.push(500 + PENDING, -1);
        // 2 is gone from the world; the objects are of class 6 now
        let class_of = |id: i32| (id != 2).then_some(6);
        t.move_all(500, 600, 5, &class_of);
        assert_eq!((t.list(500), t.list(500 + PENDING)), (&[][..], &[][..]));
        assert_eq!(
            (t.list(600), t.list(600 + PENDING)),
            (&[1, 3][..], &[4][..])
        );
        // the pending id goes to its class's done list too
        assert_eq!(
            (t.class_list(5), t.class_list(6)),
            (&[][..], &[1, 3, 4][..])
        );
    }
}
