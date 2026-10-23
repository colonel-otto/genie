//! INTERACT's order (`0x140e68b90`) to its units without a group, each at once or through its
//! unit AI's queue, several of them at their offsets around the order's point, and the work
//! command it gives (object slot 37, `FUN_14106ff70`, through slot 0x128 `FUN_141211b80` and slot
//! 0x7e8 `FUN_141227190`): the unit takes the master and the task the task finder picks for the
//! target ([`crate::tasks`]), and the task's action replaces its action list. The slice builds
//! the Hunt action ([`crate::hunt`]); other actions, orders to points and unit groups are not
//! modelled yet.

use crate::attack::{Arms, Engagement, gaps};
use crate::hunt::{HUNT, HuntAction, HuntTask, Quarry};
use crate::masters::Master;
use crate::rng::Stream;
use crate::tasks::{Actor, Diplomacy, TaskTarget, enabled};
use crate::turn::{Releaser, Turns, Unit, World};
use crate::unit_ai::{HUNT_ACTION, HUNT_ORDER, MOVE_ORDER, Unsupported};

/// INTERACT's fields past its units (`+0x4`..`+0x16`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Interact {
    /// `t_id` (`+0x4`): the target object, -1 for a point.
    pub target: i32,
    /// `x`, `y` (`+0x8`, `+0xc`).
    pub point: [f32; 2],
    /// `+0x12`: the work command's carried-resource test.
    pub carry_test: bool,
    /// `+0x13`: the order is queued.
    pub queued: bool,
    /// `+0x14`: a unit with a unit AI takes the order at once.
    pub direct: bool,
    /// `+0x15`: each unit's last human order time is set (`FUN_140e5e780`).
    pub human: bool,
    /// `+0x16`: the work command's task filter, -1 for any.
    pub filter: i16,
}

/// MOVE's fields past its units (`+0x4`..`+0x15`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoveOrder {
    /// `t_id` (`+0x4`): an object to follow, -1 for the point.
    pub target: i32,
    /// `x`, `y` (`+0x8`, `+0xc`).
    pub point: [f32; 2],
    /// `+0x13`: queued; `+0x14`: a unit with a unit AI takes it at once; `+0x15`: the last human
    /// order time is set.
    pub queued: bool,
    /// `+0x14`: a unit with a unit AI takes it at once.
    pub direct: bool,
    /// `+0x15`: the last human order time is set.
    pub human: bool,
    /// `+0x12` (`cade_api`'s `controlHeld`): the splitter makes each unit it may group a group of
    /// its own ([`Turns::split`]).
    pub control: bool,
}

/// What a unit's slot 0x7e8 (`FUN_141227190`) is given through slot 0x128: the target, the point
/// (x, y, z), the work command's carried-resource test and task filter, and the unit's `+0xa14`
/// while it runs (`notified`: INTERACT passes 1, the unit AI's queued order on a target 0): only
/// with it does the new action's notify clear the unit AI's notifies (`FUN_140c64e50`, for a unit
/// of a type above 69).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Work {
    pub target: i32,
    pub point: [f32; 3],
    pub carry_test: bool,
    pub filter: i16,
    pub notified: bool,
}

/// Masters and classes the order handler and slot 0x7e8 treat apart (decoded; the game stores a
/// class as its value − 0xeac, a master id as its id + 0x609c).
const VILLAGER: i16 = 4;
const CLASS_51: i16 = 51;
const CLASS_54: i16 = 54;
const LIVESTOCK: i16 = 58;
/// The town centre (master 109, `0x6109` raw), to which livestock's `MoveTo` goes as to a point
/// (`FUN_140aa0df0`).
const TOWN_CENTRE: i16 = 109;
/// A hunt sends at most this many animals of a herd to its drop site (`FUN_140ac4c60`'s list,
/// `DAT_143e55e20`).
const HERD_MOST: usize = 16;
const MASTER_84: i16 = 84;
const MASTER_1808: i16 = 1808;
const MASTER_1893: i16 = 1893;
const MASTER_1897: i16 = 1897;
/// A building's master type (`'P'`); types below 30 do not move.
pub(crate) const BUILDING: u8 = 80;
const MOVING: u8 = 30;
/// The task filter of garrisoning (3).
pub(crate) const GARRISON: i16 = 3;
/// The action type of a building packing or unpacking (0x7d), whose target a gather drops
/// (`FUN_140a903a0`).
const PACK_ACTION: i16 = 0x7d;
/// A master's object flag 0x20: a building a gather takes as its target as any other.
const GATHERED_BUILDING: u32 = 0x20;
/// A task's action type of attacking (7), whose factory makes an Attack (type 9).
const ATTACK_TASK: i16 = 7;

/// Where an object is: its player and its index in the live or the sleeping list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Place {
    Live(usize, usize),
    Sleeping(usize, usize),
}

/// What the order splitter reads of a unit: its master's type, formation category (not 0),
/// object flags' bit 1, its top speed (slot 0x208), its place and where it last moved to (slot
/// 0x520's ground part), its state, and for the path between two units its master's radius on
/// x (`+0x5d8`) and its terrain restriction.
#[derive(Clone, Copy, Debug)]
struct Splittable {
    id: i32,
    group: i32,
    kind: u8,
    grouped: bool,
    flag_1: bool,
    speed: f32,
    position: [f32; 3],
    moved_at: [f32; 2],
    state: u8,
    radius: f32,
    restriction: Option<usize>,
}

/// What the order splitter makes of an order's units (`FUN_140fd76d0`'s result at
/// `DAT_143f11960`): the units that move alone, in the order the splitter leaves them, its
/// groups, each its first unit first, and the units a path did not join.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Split {
    /// The owner of the list's first unit (`DAT_143f11960`), whose groups a new group of the
    /// split joins; None for a list whose first unit is gone (the game's −1).
    pub(crate) owner: Option<usize>,
    pub(crate) loose: Vec<i32>,
    pub(crate) groups: Vec<Vec<i32>>,
    /// The units a path from them did not join (`+0x3a48`, a `std::unordered_set` the splitter
    /// fills through `FUN_140a3e380` and the kept-group order `FUN_140fd9f40` reads), as a set:
    /// its bucket order is not held.
    pub(crate) far: std::collections::BTreeSet<i32>,
}

impl Split {
    /// The splitter keeps a group while it holds fewer than 60 and the group has at most 60
    /// units (`+0x20` below `0x3c`, the count below `0x3d`); else the group and its units are
    /// dropped.
    fn keep(&mut self, units: Vec<i32>) {
        if self.groups.len() < 60 && units.len() <= 60 {
            self.groups.push(units);
        }
    }
}

/// What FOLLOW reads of its target (`FUN_14161c920`'s `param_3`): its id, its place on the
/// ground and, for a master with a footprint (`+0x5d8` or `+0x5dc` above 0), its class.
#[derive(Clone, Copy, Debug)]
struct Followed {
    id: i32,
    place: [f32; 2],
    class: Option<i32>,
}

/// The splitter's path from one unit to another (`FUN_140f7cb30` -> `FUN_140f7c530`): the
/// planner's result code and the path's length; None without a planner for the restriction.
type SplitPath<'a> =
    dyn FnMut(&Splittable, &Splittable) -> Result<Option<(u8, f32)>, Unsupported> + 'a;

/// The splitter groups units of a type above this (`0x45`).
const MAX_UNGROUPED: u8 = 69;
/// Units whose speeds differ by more than this do not join where the player breaks formations by
/// speed (`FUN_140fd76d0`, `DAT_143cc2918`).
const SPLIT_SPEED_GAP: f32 = crate::image::order::SPLIT_SPEED_GAP;
/// Units nearer than this are measured by a path (`FUN_140fd76d0`, `DAT_142e7f38c`) while
/// `DAT_1439e60e3` (1 at run time) holds.
const SPLIT_PATH_REACH: f32 = crate::image::order::SPLIT_PATH_REACH;
const SPLIT_BY_PATH: bool = true;

/// `FUN_140fd76d0`'s test whether `b` joins `a`: both above type 69, both of formation category
/// 0 or neither, the same object flags' bit 1, speeds within [`SPLIT_SPEED_GAP`] where the player
/// breaks formations by speed (`COMISS`, `JA`: a NaN gap is within), then (`member`: for the
/// chosen unit's group, also `b` in a state of 2 or more) within the player's influence distance
/// on the ground (between where each last moved to, slot 0x520: `FUN_14087e560` with the height
/// zeroed; `COMISS`, `JA`: a NaN distance is within). Below [`SPLIT_PATH_REACH`] (`COMISS`, `JBE`:
/// a NaN distance is not) the game measures a path instead, when the planner of `b`'s
/// restriction exists: from `b`'s place to `a`'s (`FUN_140f7c530` with `a`'s radius on x as the
/// radii and `b`'s as the range, no tile search), which joins `b` when it reaches the goal
/// (result 1) no longer than the influence distance (a NaN length is not longer); else `b` joins
/// the splitter's far units (`FUN_140a3e380`).
fn splits_together(
    a: &Splittable,
    b: &Splittable,
    f: crate::turn::Formations,
    member: bool,
    path: &mut SplitPath<'_>,
    far: &mut std::collections::BTreeSet<i32>,
) -> Result<bool, Unsupported> {
    if a.kind <= MAX_UNGROUPED || b.kind <= MAX_UNGROUPED {
        return Ok(false);
    }
    if a.grouped != b.grouped || a.flag_1 != b.flag_1 {
        return Ok(false);
    }
    if f.by_speed && SPLIT_SPEED_GAP < (a.speed - b.speed).abs() {
        return Ok(false);
    }
    if member && b.state < 2 {
        return Ok(false);
    }
    let (dx, dy) = (a.moved_at[0] - b.moved_at[0], a.moved_at[1] - b.moved_at[1]);
    let d = crate::fmath::sqrt(dx * dx + 0.0 + dy * dy);
    if SPLIT_BY_PATH
        && d < SPLIT_PATH_REACH
        && let Some((code, length)) = path(a, b)?
    {
        let joins = code == 1 && crate::fmath::at_most_or_unordered(length, f.influence);
        if !joins {
            let _ = far.insert(b.id);
        }
        return Ok(joins);
    }
    Ok(crate::fmath::at_most_or_unordered(d, f.influence))
}

/// The class `FUN_140c89760` lets scout without the master's flag when a game setting allows it
/// (raw `0x3a`).
const AUTO_SCOUT_CLASS: i16 = 58;
/// The trophy master `TRANSFORM_OBJECT` changes ("Trophy None") and the trophies it may take.
const TROPHY: i16 = 1649;
const TROPHIES: std::ops::RangeInclusive<i16> = 1650..=1653;
/// A patrol's waypoint counts when it is this far from the last one on x or y
/// (`FUN_14161b970`, `DAT_14330eb8c`), or this far the other way (`DAT_14330ec20`).
const WAYPOINT_APART: f32 = crate::image::order::WAYPOINT_APART;
const WAYPOINT_APART_LOW: f32 = crate::image::order::WAYPOINT_APART_LOW;

impl Turns {
    pub(crate) fn place(&self, id: i32) -> Option<Place> {
        self.players.iter().enumerate().find_map(|(p, player)| {
            if let Some(i) = player.objects.iter().position(|u| u.id == id) {
                return Some(Place::Live(p, i));
            }
            let i = player.sleeping.iter().position(|u| u.id == id)?;
            Some(Place::Sleeping(p, i))
        })
    }

    pub(crate) fn object(&self, id: i32) -> Option<(usize, &Unit)> {
        match self.place(id)? {
            Place::Live(p, i) => Some((p, &self.players[p].objects[i])),
            Place::Sleeping(p, i) => Some((p, &self.players[p].sleeping[i])),
        }
    }

    /// The object `id`, live or sleeping, to change (`FUN_1415417a0`'s object).
    pub(crate) fn object_mut(&mut self, id: i32) -> Option<(usize, &mut Unit)> {
        match self.place(id)? {
            Place::Live(p, i) => Some((p, &mut self.players[p].objects[i])),
            Place::Sleeping(p, i) => Some((p, &mut self.players[p].sleeping[i])),
        }
    }

    pub(crate) fn master_of(&self, player: usize, master: i16) -> Result<&Master, Unsupported> {
        self.masters
            .get(player)
            .and_then(|m| m.get(master))
            .ok_or(Unsupported("an object of a master the slice does not have"))
    }

    /// What the task finder reads of object `id`; an annex stands for its main building (`+0xbb8`),
    /// `steps` times.
    fn task_target(&self, id: i32, steps: usize) -> Result<TaskTarget<'_>, Unsupported> {
        let mut id = id;
        for _ in 0..steps {
            match self.object(id).and_then(|(_, u)| u.annex_of) {
                Some(main) => id = main,
                None => break,
            }
        }
        let (player, u) = self
            .object(id)
            .ok_or(Unsupported("a target the slice does not hold"))?;
        let master = self.master_of(player, u.life.master_id)?;
        let [powerup, max_powerup] = u.hp_powerups;
        Ok(TaskTarget {
            id,
            master,
            player,
            state: u.life.state,
            held: (u.life.attribute_type, u.life.amount),
            hp: u.life.hp + (max_powerup - powerup),
            max_hp: f32::from(master.hit_points) + max_powerup,
            attackable: u.attackable,
            pasture: u.pasture,
            top_action: u.top_action_kind(),
        })
    }

    /// INTERACT (`0x140e68b90`) to its units, after their unit AIs reset. The splitter
    /// ([`Turns::loose_units`]) leaves the units out of any group; several of a human player
    /// whose box ([`Turns::order_spread`]) is narrow enough take places around the order's point
    /// at their offsets from their centroid (the loop of `LAB_140e6920a`), any other each the
    /// point itself (the loop of `LAB_140e69a3c`), in the splitter's order
    /// ([`Turns::interact_unit`]). A single unit the world does not hold takes nothing.
    pub(crate) fn interact(
        &mut self,
        player: usize,
        units: &[i32],
        order: &Interact,
    ) -> Result<(), Unsupported> {
        match units {
            [] => return Ok(()),
            &[id] if self.place(id).is_none() => return Ok(()),
            _ => {}
        }
        let loose = self.loose_units(units);
        self.groups.count_command();
        let loose = loose?;
        self.leave_groups(player, &loose, order.target)?;
        // FUN_140d61870: a player whose lobby entry is of type 4, which a human's is not
        if 1 < loose.len() && self.players[player].computer {
            return Err(Unsupported(
                "an order to several units of a computer player (FUN_140d61870)",
            ));
        }
        let centroid = if 1 < loose.len() {
            self.order_spread(&loose, order.queued)?
        } else {
            None
        };
        for (i, &id) in loose.iter().enumerate() {
            let others: Vec<i32> = loose
                .iter()
                .enumerate()
                .filter(|&(j, _)| j != i)
                .map(|(_, &u)| u)
                .collect();
            self.interact_unit(player, id, &others, centroid, order)?;
        }
        Ok(())
    }

    /// The units of an order that move alone ([`Turns::split`] without Ctrl held); a group it
    /// makes is not modelled.
    pub(crate) fn loose_units(&mut self, units: &[i32]) -> Result<Vec<i32>, Unsupported> {
        self.loose_units_kept(units, &|_| true)
    }

    /// [`Turns::loose_units`] where, after the sort, `kept` drops units (the garrison command's
    /// test, `FUN_140fd9ae0` for command `0x2713`).
    pub(crate) fn loose_units_kept(
        &mut self,
        units: &[i32],
        kept: &dyn Fn(i32) -> bool,
    ) -> Result<Vec<i32>, Unsupported> {
        let split = self.split(units, kept, false)?;
        if !split.groups.is_empty() {
            return Err(Unsupported("a unit group the order splitter forms"));
        }
        Ok(split.loose)
    }

    /// The order splitter (`FUN_140fd9dc0` -> `FUN_140fd9ae0` -> `FUN_140fd76d0`): it sorts the
    /// units by id (`FUN_140fd7280` with `LAB_140fd89b0`), drops those `kept` refuses (the
    /// garrison command's test, `FUN_140fd9ae0` for command `0x2713`), takes at most 60 and drops
    /// an id the next one repeats. An order whose first unit is gone splits into nothing
    /// (`FUN_140fd9ae0` reads the owner of the first).
    ///
    /// Then, in this order (each pass in the list's order, the loose units in the order the
    /// passes leave them):
    /// - a unit in a group the world holds (`+0x6d0`, `FUN_14152ff40`) takes the order's other
    ///   members of that group (`FUN_1410112b0`): two or more are a group of the split, one alone
    ///   moves loose;
    /// - without the owner's auto formations (`+0x6c0`) every other unit moves loose;
    /// - a unit above type 69 whose master's formation category is 0 (villagers) moves loose (the
    ///   dock test `FUN_141158210` is 1 only on an `E` tile in game type 12);
    /// - of the rest, the unit with the most others that join it ([`splits_together`]), the first
    ///   of equals, takes those that join it and are in a state of 2 or more: two or more are a
    ///   group, one alone moves loose; with Ctrl held (`control`) it is a group of its own.
    ///
    /// The split keeps at most 60 groups of at most 60 units: a group past them is dropped with
    /// its units. An order to more than 60 units splits its sorted list, the gone units in it, in
    /// runs of 60, each on its own into the one split (`FUN_140fd9ae0`'s `FixedMemoryResource`
    /// of 60): an id the next run repeats is in both.
    pub(crate) fn split(
        &mut self,
        units: &[i32],
        kept: &dyn Fn(i32) -> bool,
        control: bool,
    ) -> Result<Split, Unsupported> {
        // FUN_140fd9ae0 splits only when the list's first unit exists (its owner): else no unit
        // takes the order
        let Some((owner, _)) = units.first().and_then(|&id| self.object(id)) else {
            return Ok(Split::default());
        };
        // FUN_140fd9dc0's objects, null for a gone id; LAB_140fd89b0 sorts a null as -1, and
        // equal ids are one object, so any sort gives the game's order
        let mut sorted: Vec<Option<i32>> = units
            .iter()
            .map(|&id| self.object(id).map(|_| id))
            .collect();
        sorted.sort_unstable_by_key(|u| u.unwrap_or(-1));
        for u in &mut sorted {
            if u.is_some_and(|id| !kept(id)) {
                *u = None;
            }
        }
        let mut split = Split {
            owner: Some(owner),
            ..Split::default()
        };
        for run in sorted.chunks(60) {
            self.split_run(owner, run, control, &mut split)?;
        }
        Ok(split)
    }

    /// One run of at most 60 of the order splitter (`FUN_140fd76d0`) into `split`: a null entry
    /// and one the run's next entry repeats are left out.
    fn split_run(
        &mut self,
        owner: usize,
        run: &[Option<i32>],
        control: bool,
        split: &mut Split,
    ) -> Result<(), Unsupported> {
        let mut left: Vec<Splittable> = Vec::with_capacity(run.len());
        for (i, &u) in run.iter().enumerate() {
            let Some(id) = u else {
                continue;
            };
            if run.get(i + 1).copied().flatten() == Some(id) {
                continue;
            }
            let Some((p, u)) = self.object(id) else {
                continue;
            };
            let master = self.master_of(p, u.life.master_id)?;
            left.push(Splittable {
                id,
                group: u.group.id,
                kind: master.kind,
                grouped: master.formation_category != 0,
                flag_1: master.object_flags & 2 != 0,
                speed: u.mover.as_ref().map_or(0.0, |m| m.stats.speed),
                position: u.position,
                moved_at: u
                    .mover
                    .as_ref()
                    .map_or([u.position[0], u.position[1]], |m| {
                        [m.moved_at[0], m.moved_at[1]]
                    }),
                state: u.life.state,
                radius: master.radius[0],
                restriction: u.mover.as_ref().map(|m| m.stats.restriction),
            });
        }
        // the units in a group the world holds, with the order's other members of their group
        let mut taken = vec![false; left.len()];
        for i in 0..left.len() {
            if taken[i] {
                continue;
            }
            let Some(group) = self.groups.get(left[i].group) else {
                continue;
            };
            taken[i] = true;
            let mut members = vec![left[i].id];
            for (j, u) in left.iter().enumerate() {
                if !taken[j] && group.has(u.id) {
                    members.push(u.id);
                    taken[j] = true;
                }
            }
            match members[..] {
                [alone] => split.loose.push(alone),
                _ => split.keep(members),
            }
        }
        let mut left: Vec<Splittable> = left
            .into_iter()
            .zip(taken)
            .filter_map(|(u, taken)| (!taken).then_some(u))
            .collect();
        let formations = self.players[owner].formations;
        if !formations.auto {
            split.loose.extend(left.iter().map(|u| u.id));
            return Ok(());
        }
        // units above type 69 of formation category 0 stay loose (the dock test FUN_141158210 is
        // 1 only on an `E` tile in game type 12)
        let (loose, rest): (Vec<Splittable>, Vec<Splittable>) = left
            .into_iter()
            .partition(|u| MAX_UNGROUPED < u.kind && !u.grouped);
        split.loose.extend(loose.into_iter().map(|u| u.id));
        left = rest;
        // the path between two near units: the tile planner of the second's restriction, from
        // it to the first (FUN_140f7c530, its result and the measured path's length)
        let tiles = self.spatial.tiles();
        let pathing = &mut self.pathing;
        let mut path = |a: &Splittable, b: &Splittable| -> Result<Option<(u8, f32)>, Unsupported> {
            let Some(restriction) = b.restriction else {
                return Ok(None);
            };
            let Some(planner) = pathing.planner(restriction, tiles) else {
                return Ok(None);
            };
            let req = crate::tile_path::Request {
                from: [b.position[0], b.position[1]],
                to: [a.position[0], a.position[1]],
                radii: a.radius,
                range: b.radius,
                closed: Vec::new(),
                full: false,
            };
            let mut p = crate::tile_path::Path::default();
            let found = planner.plan::<Unsupported>(&req, false, &mut p, None)?;
            p.measure();
            Ok(Some((found.code(), p.length)))
        };
        // then the unit with the most others that join it, the first of equals, with them
        while !left.is_empty() {
            let mut best = (0, 0);
            for (i, a) in left.iter().enumerate() {
                let mut n = 1;
                for b in left.iter().filter(|b| b.id != a.id) {
                    let joins =
                        splits_together(a, b, formations, false, &mut path, &mut split.far)?;
                    n += usize::from(joins);
                }
                if best.1 < n {
                    best = (i, n);
                }
            }
            let chosen = left.remove(best.0);
            let mut group = vec![chosen.id];
            if !control {
                let mut i = 0;
                while i < left.len() {
                    if splits_together(
                        &chosen,
                        &left[i],
                        formations,
                        true,
                        &mut path,
                        &mut split.far,
                    )? {
                        group.push(left.remove(i).id);
                    } else {
                        i += 1;
                    }
                }
                if group.len() < 2 {
                    split.loose.push(chosen.id);
                    continue;
                }
            }
            split.keep(group);
        }
        Ok(())
    }

    /// The loose units of a MOVE or an INTERACT of `player` leave their groups (`FUN_14102c160`
    /// with 0 on the group of each one's `+0x6d0` the world holds, [`Turns::leave_group_of`]),
    /// in order. With more than one loose unit of a player whose lobby entry is not a computer's
    /// (`FUN_140d61870`) the routine's target is -1 (MOVE `FUN_140e67800` at `0x140e67e5d`,
    /// INTERACT `FUN_140e68b90` at `0x140e6913c`); else it is the order's target object's id
    /// when the world holds it (`FUN_140e6b820`), -1 without (`0x140e6873b`, `0x140e6991e`).
    fn leave_groups(
        &mut self,
        player: usize,
        loose: &[i32],
        target: i32,
    ) -> Result<(), Unsupported> {
        let target = if 1 < loose.len() && !self.players[player].computer {
            -1
        } else {
            self.world_object(target).map_or(-1, |t| t.id())
        };
        for &id in loose {
            self.leave_group_of(id, target)?;
        }
        Ok(())
    }

    /// The centroid of an order's loose units ([`centroid`]) over those that move (above type 29
    /// with speed), or all of a queued order.
    fn order_spread(&self, loose: &[i32], queued: bool) -> Result<Option<[f32; 2]>, Unsupported> {
        let mut points = Vec::with_capacity(loose.len());
        for &id in loose {
            let Some((p, u)) = self.object(id) else {
                continue;
            };
            let master = self.master_of(p, u.life.master_id)?;
            let speed = master.moving.as_ref().map_or(0.0, |m| m.speed);
            if (MOVING <= master.kind && 0.0 < speed) || queued {
                points.push([u.position[0], u.position[1]]);
            }
        }
        Ok(centroid(&points))
    }

    /// One unit of INTERACT: a unit that does not move is not modelled. Its ignore list empties
    /// (`FUN_141197da0`), it waits to move (slot 0x2a0 with 1) and lists the order's other units
    /// (`FUN_14118c790`). With a centroid its point is its offset from it added to the order's,
    /// kept `EDGE` inside the map. A unit without a unit AI, or of an order that is not queued and
    /// either direct (`+0x14`) or to a unit AI with every queued order ready (`FUN_140c4dfa0`),
    /// takes the order itself (slot 0x128); any other unit AI queues order `0x2db` on the target at
    /// the unit's point (slot 0x28, or without a centroid slot 0x30, which shows the point it
    /// queues: priority 100, the order's point shown, ready when direct). The readiness pass after
    /// an order neither queued nor direct (`FUN_140c42b60`) is not modelled.
    fn interact_unit(
        &mut self,
        player: usize,
        id: i32,
        others: &[i32],
        centroid: Option<[f32; 2]>,
        order: &Interact,
    ) -> Result<(), Unsupported> {
        let Some(place) = self.place(id) else {
            return Ok(());
        };
        let Place::Live(p, idx) = place else {
            return Err(Unsupported(
                "an order to a unit the slice does not hold live",
            ));
        };
        // the command's check (`FUN_140e55250`) drops another player's unit (its id -1) unless
        // the game setting `+0x1074` (`FUN_1412ffe00`, not held) lets players order each
        // other's units; the lookup itself has no owner test (`FUN_140e6b750`)
        if p != player {
            return Err(Unsupported(
                "an order to another player's unit (the setting +0x1074)",
            ));
        }
        let issuer = player;
        let master = self.master_of(player, self.players[player].objects[idx].life.master_id)?;
        let speed = master.moving.as_ref().map_or(0.0, |m| m.speed);
        if master.kind < MOVING || (master.class != CLASS_54 && speed <= 0.0) {
            // an object that does not move takes slot 0x128 at once with 1 (no carried-resource
            // test, any task) at the order's point, z 0; a queued order then goes on as a mover's
            self.order_at_once(player, idx, order.target, order.point)?;
            if !order.queued {
                return Ok(());
            }
            return Err(Unsupported(
                "a queued order to an object that does not move",
            ));
        }
        let target = self
            .object(order.target)
            .map(|(tp, t)| (t.life.master_id, tp));
        let map = self.spatial.map;
        let unit = &mut self.players[player].objects[idx];
        let mover = unit
            .mover
            .as_mut()
            .ok_or(Unsupported("an ordered unit without movement state"))?;
        mover.ignore_units.clear();
        mover.waiting = 1;
        for &other in others {
            ignore_unit(&mut mover.ignore_units, other);
        }
        let point = match centroid {
            None => order.point,
            Some(c) => offset_point([unit.position[0], unit.position[1]], c, order.point, map),
        };
        let takes_it = unit
            .ai
            .as_ref()
            .is_none_or(|ai| !order.queued && (order.direct || !ai.has_unready_order()));
        if takes_it {
            // slot 0x128 with 1 (FUN_141211b80): `+0xa14` is 1 and a unit AI's order queue
            // empties (FUN_140c5e010); FUN_14106c590 passes the order on unless the target is the
            // unit itself (and counts `+0x888`, not held)
            if let Some(ai) = unit.ai.as_mut() {
                ai.orders.clear();
            }
            if order.target == id {
                return Ok(());
            }
            let at = [point[0], point[1], 0.0];
            let work = Work {
                target: order.target,
                point: at,
                carry_test: order.carry_test,
                filter: order.filter,
                notified: true,
            };
            return self.order_work(player, idx, &work);
        }
        let (target_type, target_owner) =
            target.map_or((-1, -1), |(m, o)| (i32::from(m), o as i32));
        let queued = crate::unit_ai::QueuedOrder {
            issuer: issuer as i32,
            order: crate::unit_ai::ORDER_ON_TARGET,
            priority: 100,
            target: if target.is_some() { order.target } else { -1 },
            target_owner,
            point: [point[0], point[1], 0.0],
            range: 0.0,
            shown: order.point,
            ready: order.direct,
            target_type,
        };
        let state = unit.life.state;
        let ai = unit
            .ai
            .as_mut()
            .ok_or(Unsupported("unreachable: no unit AI"))?;
        let _ = ai.queue_order(state, queued, false, false)?;
        if !order.queued && !order.direct {
            return Err(Unsupported(
                "the readiness pass after an order neither queued nor direct (FUN_140c42b60)",
            ));
        }
        Ok(())
    }

    /// Slot 0x128 with 1 (`FUN_141211b80`) on `target` at `point` (z 0), no carried-resource test
    /// and any task: `+0xa14` is 1 and a unit AI's order queue empties (`FUN_140c5e010`);
    /// `FUN_14106c590` passes the order on to slot 0x7e8 unless the target is the unit itself
    /// (its `+0x888` count is not held).
    fn order_at_once(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        point: [f32; 2],
    ) -> Result<(), Unsupported> {
        let unit = &mut self.players[player].objects[idx];
        if let Some(ai) = unit.ai.as_mut() {
            ai.orders.clear();
        }
        if target == unit.id {
            return Ok(());
        }
        let work = Work {
            target,
            point: [point[0], point[1], 0.0],
            carry_test: false,
            filter: -1,
            notified: true,
        };
        self.order_work(player, idx, &work)
    }

    /// The unit AI's order on a target (`FUN_140c5c170` case `0x2db`, from its queue): the unit's
    /// slot 0x128 on the target at its place with 0 — its `+0xa14` clear while the work command
    /// runs, so the new action's notify leaves the AI's notifies, and its order queue stays —
    /// with no carried-resource test and no filter; `FUN_14106c590` passes it on unless the target
    /// is the unit itself (its `+0x888` count is not held). Then the order's priority becomes the
    /// AI's (the tail at `LAB_140c5d432`, which keeps the order the work gave it). A target that is
    /// gone, whose type the AI would look for again (slots 0x1b0, 0x88), is not modelled.
    pub(crate) fn order_on_target(
        &mut self,
        player: usize,
        idx: usize,
        order: crate::unit_ai::QueuedOrder,
    ) -> Result<(), Unsupported> {
        if order.order == crate::unit_ai::BUILD_ORDER {
            return self.build_order(player, idx, order);
        }
        let (_, t) = self
            .object(order.target)
            .ok_or(Unsupported("a queued order on a target that is gone"))?;
        let point = t.position;
        if order.target != self.players[player].objects[idx].id {
            let work = Work {
                target: order.target,
                point,
                carry_test: false,
                filter: -1,
                notified: false,
            };
            self.order_work(player, idx, &work)?;
        }
        let ai = self.players[player].objects[idx]
            .ai
            .as_mut()
            .ok_or(Unsupported("an order on a target without a unit AI"))?;
        ai.order_priority = order.priority;
        Ok(())
    }

    /// The unit AI's full update after the order on a target it took
    /// ([`crate::unit_ai::UnitAi::after_order`]), with the unit's master (its class and weapon
    /// range) and the AI's target as the world holds it now (its owner and master's class).
    pub(crate) fn after_order(
        &mut self,
        player: usize,
        idx: usize,
        after: crate::unit_ai::AfterOrder,
    ) -> Result<(), Unsupported> {
        let unit = &self.players[player].objects[idx];
        let master = self.master_of(player, unit.life.master_id)?;
        let (class, weapon_range) = (master.class, master.weapon_range);
        let target = match unit.ai.as_ref().map_or(-1, |ai| ai.current_target) {
            -1 => None,
            id => match self.object(id) {
                Some((owner, t)) => Some((owner, self.master_of(owner, t.life.master_id)?.class)),
                None => None,
            },
        };
        let computer = self.players[player].computer;
        self.players[player].objects[idx]
            .ai
            .as_mut()
            .ok_or(Unsupported("an order on a target without a unit AI"))?
            .after_order(after, player, class, weapon_range, target, computer)
    }

    /// MOVE (`FUN_140e67800`) to its units, after their unit AIs reset. One unit takes the order
    /// at the point (the loop of `LAB_140e6838c`). Several go through the splitter
    /// ([`Turns::loose_units`]); those of a human player whose box ([`centroid`]'s) is narrow
    /// enough each take a cell of a grid around the point ([`grid_point`], the loop after the
    /// box), any others the point itself, in the splitter's order, each listing the others in its
    /// ignore list ([`Turns::move_unit`]).
    pub(crate) fn move_order(
        &mut self,
        player: usize,
        units: &[i32],
        order: &MoveOrder,
    ) -> Result<(), Unsupported> {
        // the groups take the command count, which then counts up whatever the units do
        let loose = match units {
            [] => {
                self.groups.count_command();
                return Ok(());
            }
            &[id] if !order.control => {
                self.groups.count_command();
                self.leave_groups(player, &[id], order.target)?;
                return self.move_unit(player, id, &[], None, order);
            }
            _ => {
                let loose = self
                    .split(units, &|_| true, order.control)
                    .and_then(|mut s| self.move_groups(&mut s, order).map(|()| s.loose));
                self.groups.count_command();
                let loose = loose?;
                self.leave_groups(player, &loose, order.target)?;
                loose
            }
        };
        // FUN_140d61870: a player whose lobby entry is of type 4, which a human's is not
        if 1 < loose.len() && self.players[player].computer {
            return Err(Unsupported(
                "a move of several units of a computer player (FUN_140d61870)",
            ));
        }
        let narrow = 1 < loose.len() && self.order_spread(&loose, order.queued)?.is_some();
        let map = self.spatial.map;
        for (i, &id) in loose.iter().enumerate() {
            let others: Vec<i32> = loose
                .iter()
                .enumerate()
                .filter(|&(j, _)| j != i)
                .map(|(_, &u)| u)
                .collect();
            let cell = narrow.then(|| grid_point(i, loose.len(), order.point, map));
            self.move_unit(player, id, &others, cell, order)?;
        }
        Ok(())
    }

    /// MOVE's groups (`FUN_140e67800`'s loop over the splitter's groups): the group all of a
    /// split group's units are in (`FUN_14152ece0`, [`crate::group::common_group`]) takes the
    /// order when it holds just them, the splitter keeping it ([`Turns::keep_group`]; with a
    /// target the world holds, `FUN_140fd9f40`'s range test, not modelled); one the world does not
    /// hold takes nothing; else a new group of the split's owner (`FUN_14152efc0`) takes them
    /// ([`Turns::add_to_group`]), its statistics ([`Turns::group_statistics`]) and its place
    /// ([`Turns::set_initial_position`]). Each listed group (60 at most) takes the command count
    /// as its differentiator, with Ctrl held locks its formation to a line
    /// ([`Turns::lock_formation`] with 2) and takes the command ([`Turns::group_command`]); a new
    /// group of a queued MOVE first takes a command on its first unit's target
    /// ([`Turns::first_unit_target`]), not queued and direct. Then every
    /// listed group lists them all as its differentiator groups (`setDifferentiatorGroups`,
    /// `FUN_1410395a0`).
    fn move_groups(&mut self, split: &mut Split, order: &MoveOrder) -> Result<(), Unsupported> {
        /// The groups one command lists (`DAT_143f152b8`, below `0x3c`).
        const LISTED: usize = 60;
        let mut listed: Vec<i32> = Vec::new();
        for units in split.groups.clone() {
            let units = &units[..];
            let common =
                crate::group::common_group(units, |id| self.object(id).map(|(_, u)| u.group.id));
            let new = if common == -1 {
                true
            } else {
                match self.groups.get(common) {
                    None => continue,
                    Some(g) => g.members.len() != units.len(),
                }
            };
            let id = if new {
                let owner = split.owner.map_or(u32::MAX, |p| p as u32);
                let id = self.groups.create(owner).id;
                for &unit in units {
                    let _ = self.add_to_group(id, unit)?;
                }
                self.group_statistics(id, false)?;
                self.set_initial_position(id)?;
                id
            } else {
                // FUN_140e6b820's target: the order's object while the world holds it
                if self.world_object(order.target).is_some() {
                    return Err(Unsupported(
                        "a MOVE of a group the splitter keeps (FUN_140fd9f40)",
                    ));
                }
                self.keep_group(split, common)?;
                common
            };
            if listed.len() < LISTED {
                listed.push(id);
            }
            let commands = self.groups.commands;
            if let Some(g) = self.groups.get_mut(id) {
                g.differentiator_id = commands;
            }
            if order.control {
                self.lock_formation(id, crate::group::formation::LINE)?;
            }
            // a queued MOVE's new group first takes its first unit's target, not queued and
            // direct (0x140e67b5d..0x140e67cb1)
            if new
                && order.queued
                && let Some((target, at)) = self.first_unit_target(units)
            {
                let _ = self.group_command(id, target, at, (false, true))?;
            }
            let _ =
                self.group_command(id, order.target, order.point, (order.queued, order.direct))?;
        }
        let list: Vec<u32> = listed.iter().map(|&g| g as u32).collect();
        for &id in &listed {
            if let Some(g) = self.groups.get_mut(id) {
                set_differentiator_groups(g, &list);
            }
        }
        Ok(())
    }

    /// The target a queued MOVE's new group takes first (`FUN_140e67800` at
    /// `0x140e67b5d`..`0x140e67cb1`): the first of `units` the world holds, when it has a unit
    /// AI, names its target (`+0x48`, `FUN_140c4cc40`); the object the world holds for it
    /// (`FUN_1415417a0`) gives its id and its place (x, y). The unit AI's retarget flag
    /// (`FUN_140c4ee90`, `+0x1d6` 0), the command's seventh argument, is read only by a command
    /// on a target ([`Turns::group_command`] does not take one).
    fn first_unit_target(&self, units: &[i32]) -> Option<(i32, [f32; 2])> {
        let (_, first) = units.iter().find_map(|&id| self.object(id))?;
        let ai = first.ai.as_ref()?;
        let t = self.world_object(ai.current_target)?;
        let [x, y, _] = t.position();
        Some((t.id(), [x, y]))
    }

    /// PATROL (0x15): the command's check (`FUN_140e55590`, [`Turns::command_check`]: of the
    /// first 60 ids, 500 for a computer player, one the world lacks or another player's becomes
    /// −1, and a human player's command left with none is refused), then `FUN_140e69cc0`: the
    /// waypoints (1 to 10, the decoder's), those first ids, the gone ones null (`FUN_140e6b750`),
    /// the analytics record (`FUN_1409648c0`, no simulation state) and the order
    /// ([`Turns::patrol_order`]).
    pub(crate) fn patrol(
        &mut self,
        player: usize,
        ids: &[i32],
        waypoints: &[[f32; 2]],
        reset: bool,
    ) -> Result<(), Unsupported> {
        let Some(ids) = self.command_check(player, ids) else {
            return Ok(());
        };
        let cap = if self.players[player].computer {
            500
        } else {
            60
        };
        self.patrol_order(&ids[..ids.len().min(cap)], waypoints, reset)
    }

    /// `FUN_14161b970` for PATROL (its patrol flag 1, no target, `param_7` 0, `param_8` 1):
    /// first each listed unit's AI (the gone entries count in the list) takes an idle timer of
    /// 0 and an adjusted idle timeout and secondary timer of [`patrol_timeout`]; with `reset`
    /// (`+0x8`) its saved patrol goes (`zeroPatrolInformation`). The splitter (`FUN_140fd9ae0`, no
    /// Ctrl, [`Turns::split`]) parts the units. Each of its groups: a new group of the split's
    /// owner (`FUN_14152efc0`) takes them ([`Turns::add_to_group`]), its statistics and its
    /// place ([`Turns::group_statistics`], [`Turns::set_initial_position`]), unless they are a
    /// group the world holds whole, which the splitter keeps ([`Turns::keep_group`]); one the
    /// world does not hold takes nothing. Each listed group (60 at most, `DAT_143f152b8`) takes
    /// the command count as its differentiator, the waypoints its place passes ([`waypoints_from`])
    /// but the last as its own (`FUN_14100f6b0`, the target forgotten) and a patrol's leg to the
    /// last ([`Turns::command_move_to`]: range 0, the target forgotten, direct). Then every
    /// listed group lists them all (`FUN_1410395a0`) and the command count goes on. Each loose
    /// unit last ([`Turns::patrol_unit`]).
    pub(crate) fn patrol_order(
        &mut self,
        ids: &[i32],
        waypoints: &[[f32; 2]],
        reset: bool,
    ) -> Result<(), Unsupported> {
        /// The groups one command lists (`DAT_143f152b8`, below `0x3c`).
        const LISTED: usize = 60;
        let dt = self.dt;
        for (i, &id) in ids.iter().enumerate() {
            if let Some(ai) = self.object_mut(id).and_then(|(_, u)| u.ai.as_mut()) {
                ai.idle_timer = 0;
                let timeout = patrol_timeout(i, dt);
                ai.adjusted_idle_timeout = timeout;
                ai.secondary_timer = timeout;
                if reset {
                    ai.zero_patrol();
                }
            }
        }
        let mut split = self.split(ids, &|_| true, false)?;
        let mut listed: Vec<i32> = Vec::new();
        for units in split.groups.clone() {
            let common =
                crate::group::common_group(&units, |id| self.object(id).map(|(_, u)| u.group.id));
            let kept = match common {
                -1 => None,
                id => match self.groups.get(id) {
                    None => continue,
                    Some(g) => (g.members.len() == units.len()).then_some(id),
                },
            };
            let id = if let Some(id) = kept {
                self.keep_group(&mut split, id)?;
                id
            } else {
                let owner = split.owner.map_or(u32::MAX, |p| p as u32);
                let id = self.groups.create(owner).id;
                for &unit in &units {
                    let _ = self.add_to_group(id, unit)?;
                }
                self.group_statistics(id, false)?;
                self.set_initial_position(id)?;
                id
            };
            if listed.len() < LISTED {
                listed.push(id);
            }
            let commands = self.groups.commands;
            let g = self
                .groups
                .get_mut(id)
                .expect("the group found or made above");
            g.differentiator_id = commands;
            let [x, _, z] = g.motion.position[0];
            let kept = waypoints_from([x, z], waypoints);
            let Some((&last, before)) = kept.split_last() else {
                continue;
            };
            for &[x, z] in before {
                g.add_waypoint(x, z, true);
            }
            let leg = crate::group::CommandMoveTo {
                range: 0.0,
                leg: true,
                forget_target: true,
                hold: false,
                queued: false,
                direct: true,
                keep_flag: false,
            };
            let _ = self.command_move_to(id, -1, last, leg)?;
        }
        let list: Vec<u32> = listed.iter().map(|&g| g as u32).collect();
        for &id in &listed {
            if let Some(g) = self.groups.get_mut(id) {
                set_differentiator_groups(g, &list);
            }
        }
        self.groups.count_command();
        for i in 0..split.loose.len() {
            self.patrol_unit(&split.loose, i, waypoints)?;
        }
        Ok(())
    }

    /// `FUN_140fd9f40` without a target (the patrol's): the splitter kept group `group`. Each
    /// loose unit of `split` the world holds, of type 70 or more whose master has a formation
    /// category (`+0x8bc`) and not among the split's far units, joins it (`FUN_1410083c0`; flags
    /// bit 17 set) when the tile planner of its terrain restriction, where one exists, finds a
    /// path from its place to the group's (`FUN_140f7c530`, no radii, no range: result 1) and
    /// where it last moved to (slot 0x520) is within its owner's influence distance of the
    /// group's place in three dimensions (`COMISS`/`JC`: a NaN is not); it leaves the loose units,
    /// and the loop's index goes on past the unit that takes its place, which is passed over.
    pub(crate) fn keep_group(&mut self, split: &mut Split, group: i32) -> Result<(), Unsupported> {
        let mut i = 0;
        while i < split.loose.len() {
            let id = split.loose[i];
            let Some((p, u)) = self.object(id) else {
                i += 1;
                continue;
            };
            let master = self.master_of(p, u.life.master_id)?;
            if master.kind < 70 || master.formation_category == 0 || split.far.contains(&id) {
                i += 1;
                continue;
            }
            let g = self.groups.get(group).expect("the caller holds the group");
            let at = g.motion.position[0];
            let from = [u.position[0], u.position[1]];
            let moved = u.mover.as_ref().map_or(u.position, |m| m.moved_at);
            let restriction = u.mover.as_ref().map(|m| m.stats.restriction);
            let tiles = self.spatial.tiles();
            if let Some(planner) = restriction.and_then(|r| self.pathing.planner(r, tiles)) {
                let req = crate::tile_path::Request {
                    from,
                    to: [at[0], at[2]],
                    radii: 0.0,
                    range: 0.0,
                    closed: Vec::new(),
                    full: false,
                };
                let mut path = crate::tile_path::Path::default();
                if planner
                    .plan::<Unsupported>(&req, false, &mut path, None)?
                    .code()
                    != 1
                {
                    i += 1;
                    continue;
                }
            }
            // slot 0x520: +0x824 as the game holds it (x, height, y)
            let d = crate::fmath::sub([moved[0], moved[2], moved[1]], at);
            let influence = self.players[p].formations.influence;
            if d[0] * d[0] + d[1] * d[1] + d[2] * d[2] <= influence * influence {
                let _ = split.loose.remove(i);
                let _ = self.add_to_group(group, id)?;
                if let Some(g) = self.groups.get_mut(group) {
                    g.flags |= 0x2_0000;
                }
            }
            i += 1;
        }
        Ok(())
    }

    /// PATROL's loose unit `loose[i]` (`FUN_14161b970`'s loop over the splitter's loose units):
    /// one the world does not hold takes nothing. It leaves its unit group (`FUN_14102c160` with
    /// −1); a unit that moves (with movement state: type 30 or more) loses its own waypoints
    /// (slot 0x450) and its ignore list (`FUN_141197da0`), waits to move (slot 0x2a0 with 1) and
    /// lists the other loose units (`FUN_14118c790`); an object below type 30 has those slots
    /// empty, and its ignore list (the static part's, which only a walker's short pather reads)
    /// is not held. Of the waypoints its place passes ([`waypoints_from`]) none leaves it there;
    /// else each but the last becomes its own (slot 0x448 at its height) and it moves to the last
    /// (slot 0x340: range 0, forced, walking): of type 70 or 80 through `FUN_141216c50`
    /// ([`Turns::move_aside`]; type 50 and 60 run `FUN_1410866e0` without its class test, which
    /// only masters of type 70 meet), types 30 and 40 not at all (`FUN_14106d330`). Its unit AI
    /// takes the patrol order and the move action (`FUN_140c64b60` with 1, `FUN_140c64b50`) and
    /// no target at the command's last waypoint at the unit's height (`FUN_140c64be0`), even
    /// where the filter left that waypoint out.
    fn patrol_unit(
        &mut self,
        loose: &[i32],
        i: usize,
        waypoints: &[[f32; 2]],
    ) -> Result<(), Unsupported> {
        use crate::unit_ai::{MOVE_ACTION, PATROL};
        let id = loose[i];
        let Some((p, u)) = self.object(id) else {
            return Ok(());
        };
        let kind = self.master_of(p, u.life.master_id)?.kind;
        self.leave_group_of(id, -1)?;
        let (_, u) = self.object_mut(id).expect("the unit found above");
        if let Some(m) = u.mover.as_mut() {
            m.clear_user_waypoints();
            m.ignore_units.clear();
            m.waiting = 1;
            for (k, &other) in loose.iter().enumerate() {
                if k != i {
                    ignore_unit(&mut m.ignore_units, other);
                }
            }
        }
        let [x, y, z] = u.position;
        let kept = waypoints_from([x, y], waypoints);
        let Some((&[lx, ly], before)) = kept.split_last() else {
            return Ok(());
        };
        if let Some(m) = u.mover.as_mut() {
            for &[wx, wy] in before {
                m.add_user_waypoint(wx, wy, z);
            }
        }
        // a sleeping object is of a type below 30 (only slot 0x180 puts one to sleep)
        if 50 <= kind
            && let Some(Place::Live(p, idx)) = self.place(id)
        {
            let walk = crate::unit_ai::AiMove {
                point: [lx, ly, z],
                range: 0.0,
                force: true,
                walk: true,
            };
            self.move_aside(p, idx, walk)?;
        }
        let unit = match self.place(id) {
            Some(Place::Live(p, i)) => &mut self.players[p].objects[i],
            Some(Place::Sleeping(p, i)) => &mut self.players[p].sleeping[i],
            None => return Ok(()),
        };
        let rng = self.streams.get(Stream::Ai);
        if let Some(ai) = unit.ai.as_mut() {
            let shown = waypoints.last().copied().unwrap_or_default();
            ai.set_order(PATROL, rng);
            ai.set_action(MOVE_ACTION);
            ai.set_target(-1, -1, [shown[0], shown[1], z]);
        }
        Ok(())
    }

    /// FOLLOW (0x14): the command's check (`FUN_140e54c50`, [`Turns::command_check`]), then
    /// `FUN_140e66a60`: a target the world does not hold does nothing; the first 60 ids (500 for
    /// a computer player) go, gone ones null (`FUN_140e6b750`), to `FUN_14161c920`. Its unit AI
    /// resets (`FUN_141631040`) are the caller's ([`crate::turn`]). The splitter
    /// (`FUN_140fd9ae0`, no Ctrl) parts the units; a group it forms is not modelled (a new
    /// group, `FUN_14152efc0`, follows through `FUN_141007320`; a kept one through
    /// `FUN_140fd9f40`). The command count goes on. Several loose units leave their unit groups
    /// (`FUN_14102c160` with −1) and, when the box of those that move is narrow enough
    /// ([`follow_spread`]), each moves to its offset from their mean added to the target's place
    /// ([`Turns::follow_offset`]); else each follows the target ([`Turns::follow_unit`]).
    pub(crate) fn follow(
        &mut self,
        player: usize,
        units: &[i32],
        target: i32,
    ) -> Result<(), Unsupported> {
        let Some(ids) = self.command_check(player, units) else {
            return Ok(());
        };
        let Some((tp, t)) = self.object(target) else {
            return Ok(());
        };
        let tm = self.master_of(tp, t.life.master_id)?;
        let followed = Followed {
            id: target,
            place: [t.position[0], t.position[1]],
            class: (0.0 < tm.radius[0] || 0.0 < tm.radius[1]).then_some(i32::from(tm.class)),
        };
        let cap = if self.players[player].computer {
            500
        } else {
            60
        };
        let ids = &ids[..ids.len().min(cap)];
        let split = self.split(ids, &|_| true, false)?;
        if !split.groups.is_empty() {
            return Err(Unsupported(
                "a FOLLOW whose units the splitter groups (FUN_141007320, FUN_140fd9f40)",
            ));
        }
        self.groups.count_command();
        let loose = split.loose;
        if 1 < loose.len() {
            let mut points = Vec::with_capacity(loose.len());
            for &id in &loose {
                self.leave_group_of(id, -1)?;
                if let Some(at) = self.follower(id)? {
                    points.push(at);
                }
            }
            if let Some(mean) = follow_spread(&points) {
                for i in 0..loose.len() {
                    self.follow_offset(&loose, i, followed, mean)?;
                }
                return Ok(());
            }
        }
        for i in 0..loose.len() {
            self.follow_unit(&loose, i, followed)?;
        }
        Ok(())
    }

    /// Unit `id` leaves the unit group the world holds of its link (`FUN_14102c160` with `target`
    /// and 0, [`Turns::remove_unit`]); a unit the world does not hold or in no group, nothing.
    fn leave_group_of(&mut self, id: i32, target: i32) -> Result<(), Unsupported> {
        let link = self.object(id).map_or(-1, |(_, u)| u.group.id);
        if link != -1 {
            let _ = self.remove_unit(link, id, target, false)?;
        }
        Ok(())
    }

    /// FOLLOW's test of a unit (`FUN_14161c920`): one the world holds, of a type of 30 or more
    /// whose master moves (slot 0xe8 above 0; `COMISS`, `JNC`: a NaN speed moves), gives its
    /// place on the ground.
    fn follower(&self, id: i32) -> Result<Option<[f32; 2]>, Unsupported> {
        let Some((p, u)) = self.object(id) else {
            return Ok(None);
        };
        let master = self.master_of(p, u.life.master_id)?;
        let speed = master.moving.as_ref().map_or(0.0, |m| m.speed);
        let moves = MOVING <= master.kind && crate::fmath::below_or_unordered(0.0, speed);
        Ok(moves.then_some([u.position[0], u.position[1]]))
    }

    /// A follower's start (`FUN_14161c920`, both loops): its ignore list empties
    /// (`FUN_141197da0`), it waits to move (slot 0x2a0 with 1) and lists the other loose units
    /// ([`ignore_unit`]); its place in the live list. A sleeping object is of a type below 30
    /// (only slot 0x180 puts one to sleep), which [`Turns::follower`] passed over.
    fn follower_start(&mut self, loose: &[i32], i: usize) -> Result<(usize, usize), Unsupported> {
        let Some(Place::Live(p, idx)) = self.place(loose[i]) else {
            return Err(Unsupported(
                "a FOLLOW of a sleeping object of a type of 30 or more",
            ));
        };
        let mover = self.players[p].objects[idx]
            .mover
            .as_mut()
            .ok_or(Unsupported("a followed unit without movement state"))?;
        mover.ignore_units.clear();
        mover.waiting = 1;
        for (k, &other) in loose.iter().enumerate() {
            if k != i {
                ignore_unit(&mut mover.ignore_units, other);
            }
        }
        Ok((p, idx))
    }

    /// FOLLOW's loose unit `loose[i]` (`FUN_14161c920`'s loop for units that do not move at
    /// their offsets): it leaves its unit group (`FUN_14102c160` with the target's id); one that
    /// does not move takes nothing ([`Turns::follower`]); else it starts
    /// ([`Turns::follower_start`]) and moves to the target at its place (slot 0x138,
    /// [`Turns::move_to_object_with`], z 0). Its unit AI takes the follow order and action
    /// (`FUN_140c64b60` with 1, `FUN_140c64b50`) only when no action of its took the move; then
    /// the target with its class (none without a footprint) at its place (`FUN_140c64be0`).
    /// Slot 0x2d8's walk and show targets and `+0x888` are not held.
    fn follow_unit(
        &mut self,
        loose: &[i32],
        i: usize,
        followed: Followed,
    ) -> Result<(), Unsupported> {
        use crate::unit_ai::{FOLLOW_ACTION, FOLLOW_ORDER};
        self.leave_group_of(loose[i], followed.id)?;
        if self.follower(loose[i])?.is_none() {
            return Ok(());
        }
        let (p, idx) = self.follower_start(loose, i)?;
        let [x, y] = followed.place;
        let taken = self.move_to_object_with(p, idx, followed.id, [x, y, 0.0], true)?;
        let rng = self.streams.get(Stream::Ai);
        if let Some(ai) = self.players[p].objects[idx].ai.as_mut() {
            if !taken {
                ai.set_order(FOLLOW_ORDER, rng);
                ai.set_action(FOLLOW_ACTION);
            }
            let (t, class) = followed.class.map_or((-1, -1), |c| (followed.id, c));
            ai.set_target(t, class, [x, y, 0.0]);
        }
        Ok(())
    }

    /// FOLLOW's loose unit `loose[i]` at its offset (`FUN_14161c920`'s loop after the box): one
    /// that moves ([`Turns::follower`]) starts ([`Turns::follower_start`]) and goes to its place
    /// less `mean` plus the target's, held on the map ([`follow_axis`]): slot 0x138 (z 0) to the
    /// target there when its master has a footprint, else to the point
    /// ([`Turns::move_point_order_with`]). Its unit AI takes the move order and action
    /// (`FUN_140c64b60` with 1, `FUN_140c64b50`) and the target (none without a footprint) at
    /// that place (`FUN_140c64be0`). Slot 0x2d8's walk and show targets are not held.
    fn follow_offset(
        &mut self,
        loose: &[i32],
        i: usize,
        followed: Followed,
        mean: [f32; 2],
    ) -> Result<(), Unsupported> {
        use crate::unit_ai::MOVE_ACTION;
        let Some([x, y]) = self.follower(loose[i])? else {
            return Ok(());
        };
        let (p, idx) = self.follower_start(loose, i)?;
        let map = self.spatial.map;
        let at = [
            follow_axis(x - mean[0] + followed.place[0], map[0]),
            follow_axis(y - mean[1] + followed.place[1], map[1]),
            0.0,
        ];
        match followed.class {
            Some(_) => {
                let _ = self.move_to_object_with(p, idx, followed.id, at, true)?;
            }
            None => self.move_point_order_with(p, idx, at, true)?,
        }
        let rng = self.streams.get(Stream::Ai);
        if let Some(ai) = self.players[p].objects[idx].ai.as_mut() {
            ai.set_order(MOVE_ORDER, rng);
            ai.set_action(MOVE_ACTION);
            let (t, class) = followed.class.map_or((-1, -1), |c| (followed.id, c));
            ai.set_target(t, class, at);
        }
        Ok(())
    }

    /// Slot 0x138 to `target` outside slot 0x128's work command (the unit's `+0xa14` 1):
    /// [`Turns::move_to_object_with`].
    pub(crate) fn move_to_object(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        point: [f32; 3],
    ) -> Result<(), Unsupported> {
        self.move_to_object_with(player, idx, target, point, true)
            .map(|_| ())
    }

    /// Slot 0x138 to `target` (`FUN_141216f10`): a villager is no longer under the town bell
    /// (`+0x9dc`); with `+0xa14` (`a14`: 1 but inside slot 0x128's work command with 0) its unit
    /// AI's order queue empties (`FUN_140c5e010`); then `FUN_14106d340`: unless the unit is its own
    /// target, its unit AI's notifies go with the world time (`FUN_140c5dc10`) and, but after a
    /// kill it stops on (`FUN_140c41c10`), its current action's slot 0x40 may take the target and
    /// `point` (a Hunt's, [`Turns::hunt_take_move`]; an Artifact's, `FUN_140aaca30`, would: not
    /// modelled; a `MoveTo`'s or an Explore's never); else a `MoveTo` to the target
    /// (`FUN_140aa0df0`: range 0 from slot 0x268, the walking sprite) takes the target it holds
    /// (slot 0x178, [`crate::move_to::holds_object`]) and replaces its actions (slot 0x808, the
    /// old ones' teardowns let their targets go, [`Turns::clear_actions`]; then its start,
    /// [`MoveToAction::to_object`]). Its `command_flag` (`+0x888`) is not held.
    ///
    /// [`MoveToAction::to_object`]: crate::move_to::MoveToAction::to_object
    pub(crate) fn move_to_object_with(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        point: [f32; 3],
        a14: bool,
    ) -> Result<bool, Unsupported> {
        let world_time = self.world_time;
        let villager = self
            .master_of(player, self.players[player].objects[idx].life.master_id)?
            .class
            == VILLAGER;
        let unit = &mut self.players[player].objects[idx];
        if villager {
            unit.town_bell.flag = false;
        }
        if a14 && let Some(ai) = unit.ai.as_mut() {
            ai.orders.clear();
        }
        if target == unit.id {
            return Ok(false);
        }
        if let Some(ai) = unit.ai.as_mut() {
            ai.clear_notifies(world_time)?;
        }
        let stop_after_kill = unit.ai.as_ref().is_some_and(|ai| ai.stop_after_kill);
        if unit.ai.is_some()
            && !stop_after_kill
            && self.head_takes_move(player, idx, Some(target), point)?
        {
            return Ok(true);
        }
        let unit = &self.players[player].objects[idx];
        if unit.unmodelled_action.is_some() {
            return Err(Unsupported("a move to an object of a unit with an action"));
        }
        // FUN_140aa0df0 makes the MoveTo first, which takes its target (slot 0x178, quiet 0);
        // slot 0x808 then clears the list (the old actions' teardowns) and starts it
        let livestock = self.master_of(player, unit.life.master_id)?.class == LIVESTOCK;
        let (tp, t) = self.object(target).ok_or(crate::move_to::TARGET_LACKING)?;
        let t_state = t.life.state;
        let t_master = self.master_of(tp, t.life.master_id)?.id;
        if crate::move_to::holds_object(livestock, t_master, t_state) {
            self.take(target);
        }
        self.clear_actions(player, idx)?;
        let unit = &mut self.players[player].objects[idx];
        let mover = unit
            .mover
            .as_mut()
            .ok_or(Unsupported("a moved unit without movement state"))?;
        let move_to = crate::move_to::MoveToAction::to_object(
            &self.spatial,
            unit.position,
            mover,
            (target, t_state),
            0.0,
            (false, livestock),
        )?;
        if move_to.arrived() {
            unit.shows_standing_sprite(
                self.masters.get(player),
                &self.graphics,
                self.streams.get(Stream::Sprite),
            )?;
        }
        unit.move_to = Some(move_to);
        Ok(false)
    }

    /// The head action's slot 0x40 (`FUN_140a97730`) on a move of the unit at `idx` to `point`,
    /// toward the object `target` if any: true when the action takes the move. An empty list
    /// takes nothing, nor does the base slot (`FUN_140a78490`) of a `MoveTo`, an Explore, a Build
    /// (0x140aade60), an Attack, an Enter, a Fly, a missile or a Make. The Artifact's
    /// (`FUN_140aaca30`) always takes it: toward an object ([`Turns::artifact_take_object`]), or
    /// to the point, where a `MoveTo` it walked goes and stops the unit; the Hunt's
    /// (`0x140ac52c0`, [`Turns::hunt_take_move`]) and the Gather's (`0x140a8b740`,
    /// [`Turns::gather_take_move`]) decide themselves.
    fn head_takes_move(
        &mut self,
        player: usize,
        idx: usize,
        target: Option<i32>,
        point: [f32; 3],
    ) -> Result<bool, Unsupported> {
        let unit = &mut self.players[player].objects[idx];
        if unit.unmodelled_action.is_some() {
            return Err(Unsupported(
                "slot 0x40 of an action the slice does not hold",
            ));
        }
        match unit.top_action_kind() {
            None
            | Some(
                crate::move_to::MOVE_TO
                | crate::explore::EXPLORE
                | crate::build::BUILD
                | crate::attack::ATTACK
                | crate::garrison::ENTER
                | crate::fly::FLY
                | crate::missile::MISSILE
                | crate::make::MAKE,
            ) => Ok(false),
            Some(crate::artifact::ARTIFACT_TYPE) => {
                if let Some(t) = target {
                    self.artifact_take_object(player, idx, t)?;
                    return Ok(true);
                }
                let walked = unit.artifact.as_mut().is_some_and(|a| a.take_point(point));
                if walked {
                    unit.mover
                        .as_mut()
                        .ok_or(Unsupported("a walker without movement state"))?
                        .stop()?;
                }
                Ok(true)
            }
            Some(HUNT) => self.hunt_take_move(player, idx, target, point),
            Some(crate::gather::GATHER) => self.gather_take_move(player, idx, target, point),
            Some(other) => {
                unreachable!("a top action of type {other}, which top_action never holds")
            }
        }
    }

    /// An animal's Artifact takes a move to the object `target` (its slot 8, `FUN_140aaca30`,
    /// [`crate::artifact::Artifact::take_object`]): its old target is let go (slot 0x180,
    /// [`Turns::release`]), then the target, in a state below 7, taken (slot 0x178,
    /// [`Turns::take`]); livestock (class 58) goes to a building within 1, to the town centre
    /// (master 109) at its point; a `MoveTo` it walked goes, whose teardown stops the unit (slot
    /// 0x5d0).
    pub(crate) fn artifact_take_object(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
    ) -> Result<(), Unsupported> {
        let by = self.releaser(player, idx);
        let unit = &self.players[player].objects[idx];
        let livestock = self.master_of(player, unit.life.master_id)?.class == LIVESTOCK;
        let (tp, t) = self.object(target).ok_or(Unsupported(
            "an Artifact's move to an object the slice does not hold",
        ))?;
        let alive = t.life.state < 7;
        let t_master = self.master_of(tp, t.life.master_id)?;
        let to_building = livestock && t_master.kind == BUILDING;
        let to_town_centre = livestock && t_master.id == TOWN_CENTRE;
        let at = self
            .spatial
            .get(target)
            .map(|p| p.position)
            .ok_or(Unsupported(
                "an Artifact's move to an object without a place",
            ))?;
        let unit = &mut self.players[player].objects[idx];
        let (walked, targets) = unit.artifact.as_mut().map_or_else(
            || (false, crate::artifact::Targets::default()),
            |a| a.take_object((target, alive), at, to_building, to_town_centre),
        );
        if let Some(t) = targets.released {
            self.release(t, by)?;
        }
        if let Some(t) = targets.taken {
            self.take(t);
        }
        if walked {
            self.players[player].objects[idx]
                .mover
                .as_mut()
                .ok_or(Unsupported("a walker without movement state"))?
                .stop()?;
        }
        Ok(())
    }

    /// A hunt's own livestock `sheep` sent to the drop site `site` (`0x140ac60e0` case 6): the
    /// animal's slot 0x138 to the site ([`Turns::move_to_object`]), then `FUN_140ac4c60` from it
    /// ([`Turns::herd_from`]).
    ///
    /// # Errors
    ///
    /// What the moves do not model.
    pub(crate) fn lead_herd(&mut self, p: usize, sheep: i32, site: i32) -> Result<(), Unsupported> {
        self.send_to_site(p, sheep, site)?;
        let mut listed = Vec::new();
        self.herd_from(p, sheep, site, &mut listed)
    }

    /// `FUN_140ac4c60` from the animal `sheep` (`listed` the animals sent so far, a static list of
    /// 16): it joins the list; while it holds fewer than 16, the tiles within 3 of the animal's
    /// (x outer, y inner, held to the map) that player `p` sees (`FUN_141663500` 0xf), each in its
    /// list's order, give the next: livestock (class 58) of the player in state 2 whose top action
    /// has neither a target nor a point ([`Unit::top_action_idle`]) and not listed yet is sent to
    /// the site (slot 0x138) and herds from it in turn; once 16 are listed it ends.
    fn herd_from(
        &mut self,
        p: usize,
        sheep: i32,
        site: i32,
        listed: &mut Vec<i32>,
    ) -> Result<(), Unsupported> {
        listed.push(sheep);
        if HERD_MOST <= listed.len() {
            return Ok(());
        }
        // the animal's own place (+0x3c8, +0x4c8), truncated; send_to_site found it live
        let at = self.object(sheep).expect("sent to the site").1.position;
        let (x, y) = (
            crate::checkpoint::cvttss2si(at[0]),
            crate::checkpoint::cvttss2si(at[1]),
        );
        let [w, h] = self.spatial.map;
        let (x0, x1) = ((x - 3).max(0), if x + 3 < w { x + 3 } else { w - 1 });
        let (y0, y1) = ((y - 3).max(0), if y + 3 < h { y + 3 } else { h - 1 });
        for tx in x0..=x1 {
            for ty in y0..=y1 {
                if !self.spatial.sees_tile(p, tx, ty) {
                    continue;
                }
                let ids = self.spatial.tiles().list(tx, ty).to_vec();
                for id in ids {
                    let Some(Place::Live(q, i)) = self.place(id) else {
                        continue;
                    };
                    let u = &self.players[q].objects[i];
                    if q != p
                        || u.life.state != 2
                        || self.master_of(q, u.life.master_id)?.class != LIVESTOCK
                        || !u.top_action_idle()?
                        || listed.contains(&id)
                    {
                        continue;
                    }
                    self.send_to_site(p, id, site)?;
                    self.herd_from(p, id, site, listed)?;
                    if HERD_MOST <= listed.len() {
                        return Ok(());
                    }
                }
            }
        }
        Ok(())
    }

    /// Slot 0x138 of player `p`'s animal `sheep` to the drop site `site` at its place.
    fn send_to_site(&mut self, p: usize, sheep: i32, site: i32) -> Result<(), Unsupported> {
        let Some(Place::Live(q, idx)) = self.place(sheep) else {
            return Err(Unsupported("a led animal the slice does not hold live"));
        };
        if q != p {
            return Err(Unsupported("a led animal of another player"));
        }
        let point = self
            .spatial
            .get(site)
            .map(|s| s.position)
            .ok_or(Unsupported("a drop site without a place"))?;
        self.move_to_object(p, idx, site, point)
    }

    /// `UNIT_ORDER`'s action 4 (`FUN_14161e370` case 4) to the objects `ids`: each the world
    /// holds (any player's) that is a building in state 2 cancels the `index`-th unit of its
    /// queue ([`Turns::cancel_queued`], `all` the command's `+0x1d`).
    pub(crate) fn cancel_order(
        &mut self,
        ids: &[i32],
        index: u32,
        all: bool,
    ) -> Result<(), Unsupported> {
        for &id in ids {
            let Some(place) = self.place(id) else {
                continue;
            };
            let Place::Live(p, idx) = place else {
                return Err(Unsupported("a cancel at a sleeping object"));
            };
            let unit = &self.players[p].objects[idx];
            if unit.life.state != 2 || self.master_of(p, unit.life.master_id)?.kind < 80 {
                continue;
            }
            self.cancel_queued(p, idx, index as usize, all)?;
        }
        Ok(())
    }

    /// `UNIT_ORDER`'s order `action` to the units `ids` (`FUN_14161e370`). Action 8, the drop-off
    /// (as 6 and 7): each unit the world holds in state 2 of a type above 39 that carries
    /// something (`+0x310` above 0) and whose top action is a Hunt (110) or a Gather (5) with a
    /// target or a point gives it its drop-off (slot 0x78, [`HuntAction::drop_off_asked`], then
    /// `set_state(8)`: [`Unit::drop_off_in`], [`Unit::gather_drop_off_in`]). A Build's (101) and
    /// a top action without a target (`FUN_141206a20`, `FUN_141205ac0` around it) are not
    /// modelled, nor are the other actions and another player's unit; a unit without one of
    /// these actions: [`Turns::drop_off_unasked`].
    pub(crate) fn unit_order(
        &mut self,
        player: usize,
        ids: &[i32],
        action: u8,
    ) -> Result<(), Unsupported> {
        if action != 8 {
            return Err(Unsupported(
                "a UNIT_ORDER other than the garrison (5) and the drop-off (8)",
            ));
        }
        for &id in ids {
            let Some(place) = self.place(id) else {
                continue;
            };
            let Place::Live(p, idx) = place else {
                return Err(Unsupported("a UNIT_ORDER to a sleeping object"));
            };
            if p != player {
                return Err(Unsupported("a UNIT_ORDER to another player's unit"));
            }
            let unit = &self.players[p].objects[idx];
            let kind = self.master_of(p, unit.life.master_id)?.kind;
            if unit.life.state != 2 || kind < 40 || unit.life.amount <= 0.0 {
                continue;
            }
            if unit.unmodelled_action.is_some() {
                return Err(Unsupported("a UNIT_ORDER drop-off of an action not held"));
            }
            if let Some(gather) = unit
                .gather
                .as_ref()
                .filter(|_| unit.top_action_kind() == Some(crate::gather::GATHER))
            {
                if gather.target == -1 && gather.point[0] == -1.0 && gather.point[1] == -1.0 {
                    return Err(Unsupported(
                        "a UNIT_ORDER drop-off of a Gather without a target (FUN_141206a20)",
                    ));
                }
                let attributes = self.players[p].attributes.len();
                if !gather.drop_off_asked(unit.life.attribute_type, attributes)? {
                    continue;
                }
                let mut unit = std::mem::take(&mut self.players[p].objects[idx]);
                let player = &self.players[p];
                let world = crate::turn::DropWorld {
                    masters: self.masters.get(p),
                    spatial: &self.spatial,
                    zones: &mut self.zones,
                    pathing: &mut self.pathing,
                    object: self.streams.get(Stream::Object),
                    world_time: self.world_time,
                    computer: player.computer,
                    construction_rate: player.construction_rate,
                    groups: &self.groups,
                };
                let result = unit.gather_drop_off_in(world, &player.objects);
                self.players[p].objects[idx] = unit;
                if let Some(site) = result? {
                    self.take(site);
                }
                continue;
            }
            if unit.top_action_kind() == Some(crate::build::BUILD) {
                return Err(Unsupported("a UNIT_ORDER drop-off of a Build (slot 0x78)"));
            }
            let Some(hunt) = unit
                .hunt
                .as_ref()
                .filter(|_| unit.top_action_kind() == Some(HUNT))
            else {
                self.drop_off_unasked(p, idx)?;
                continue;
            };
            if hunt.target == -1 && hunt.point[0] == -1.0 && hunt.point[1] == -1.0 {
                return Err(Unsupported(
                    "a UNIT_ORDER drop-off of a Hunt without a target (FUN_141206a20)",
                ));
            }
            let attributes = self.players[p].attributes.len();
            if !hunt.drop_off_asked(unit.life.attribute_type, attributes)? {
                continue;
            }
            let mut unit = std::mem::take(&mut self.players[p].objects[idx]);
            let player = &self.players[p];
            let world = crate::turn::DropWorld {
                masters: self.masters.get(p),
                spatial: &self.spatial,
                zones: &mut self.zones,
                pathing: &mut self.pathing,
                object: self.streams.get(Stream::Object),
                world_time: self.world_time,
                computer: player.computer,
                construction_rate: player.construction_rate,
                groups: &self.groups,
            };
            let result = unit.drop_off_in(world, &player.objects);
            self.players[p].objects[idx] = unit;
            result?;
        }
        Ok(())
    }

    /// `UNIT_ORDER`'s drop-off of a carrying unit without a gather action on top
    /// (`FUN_14161e370`'s case 8, past the Gather, Hunt and Build): a master with drop sites
    /// (`FUN_14109e210`) searches from the unit (`FUN_1411671e0`), which is not modelled; any
    /// other asks the drop-site table (`FUN_141658e20` -> `FUN_140fdd950`) for buildings taking
    /// its held resource from its class: entries of its class with resource types, for its civ.
    /// With none the unit does nothing (a livestock with its food); a building found (the
    /// player's closest in state 2, then the work command on it, slot 0x7e8) is not modelled.
    fn drop_off_unasked(&self, p: usize, idx: usize) -> Result<(), Unsupported> {
        let unit = &self.players[p].objects[idx];
        let master = self.master_of(p, unit.life.master_id)?;
        if master.drop_sites.iter().any(|&d| 0 <= d) {
            return Err(Unsupported(
                "a UNIT_ORDER drop-off of a unit with drop sites and no gather action (FUN_1411671e0)",
            ));
        }
        let civ = self.players[p].civ;
        let class = master.class;
        if self
            .drop_sites
            .iter()
            .any(|d| d.worker_class == class && (d.civs.is_empty() || d.civs.contains(&civ)))
        {
            return Err(Unsupported(
                "a UNIT_ORDER drop-off at a building of the drop-site table (slot 0x7e8)",
            ));
        }
        Ok(())
    }

    /// `UNIT_AI_STATE` (`do_command` case 0x12): the check ([`Turns::command_check`]), then
    /// `FUN_140e6a7c0` over its first 60 ids (500 for a computer player): each object the world
    /// holds (any player's the check let through) with a unit AI takes the stance `state` (slot
    /// 0x1c0, [`set_stance`]), but not stances 1 and 2 for classes 54 and 51 (and, the siege AI's
    /// slot, masters 42 and 331). The UI's flag is not held.
    pub(crate) fn unit_ai_state(
        &mut self,
        player: usize,
        ids: &[i32],
        state: u8,
    ) -> Result<(), Unsupported> {
        let Some(ids) = self.command_check(player, ids) else {
            return Ok(());
        };
        let cap = if self.players[player].computer {
            500
        } else {
            60
        };
        for &id in ids.iter().take(cap) {
            let Some(place) = self.place(id) else {
                continue;
            };
            let (p, idx) = match place {
                Place::Live(p, i) => (p, i),
                // only slot 0x180 puts an object to sleep, one of a type below 30, which has no
                // unit AI: the handler passes it over
                Place::Sleeping(p, i) if self.players[p].sleeping[i].ai.is_none() => continue,
                Place::Sleeping(..) => {
                    return Err(Unsupported(
                        "a UNIT_AI_STATE of a sleeping object with a unit AI",
                    ));
                }
            };
            let unit = &self.players[p].objects[idx];
            if unit.ai.is_none() {
                continue;
            }
            let master = self.master_of(p, unit.life.master_id)?;
            if matches!(state, 1 | 2)
                && (matches!(master.class, 51 | 54) || matches!(master.id, 42 | 331))
            {
                continue;
            }
            let class = master.class;
            let unit = &mut self.players[p].objects[idx];
            let place = ([unit.position[0], unit.position[1]], class);
            let ai = unit.ai.as_mut().expect("checked above");
            let mut fx = Vec::new();
            set_stance(ai, state, place, &mut fx)?;
            for e in fx {
                match e {
                    crate::unit_ai::Effect::Stop => self.command_stop(p, idx)?,
                    _ => return Err(Unsupported("a stance's stop effect not modelled")),
                }
            }
        }
        Ok(())
    }

    /// GATE (`do_command` case 0x72): the command's check refuses an object `id` the world does
    /// not hold, or another player's while players may not order each other's units (the
    /// setting `+0x1074`, [`crate::settings::Settings::cheats_enabled`]; the world's `+0xb12` cheat is not held).
    /// The object, live or sleeping (`FUN_1415417a0`), of a master of type 80 toggles its gate
    /// lock: slot 0x670 ([`lock_gate`]) with slot 0x668's value negated. Its sound (slot 0x4c0)
    /// and the UI's flag are not held.
    pub(crate) fn gate(&mut self, player: usize, id: i32) -> Result<(), Unsupported> {
        let Some((p, unit)) = self.object(id) else {
            return Ok(());
        };
        if p != player && !self.settings.cheats_enabled {
            return Ok(());
        }
        if self.master_of(p, unit.life.master_id)?.kind != BUILDING {
            return Ok(());
        }
        let now = self.world_time;
        let (_, unit) = self.object_mut(id).expect("found above");
        // slot 0x668: the lock's value (a building without its gate state is lock_gate's)
        let locked = unit.gate.is_some_and(|g| g.locked);
        lock_gate(unit, !locked, now)
    }

    /// `AUTO_SCOUT` (0x26, `FUN_140e666f0`): of the first 60 ids (500 for a computer player,
    /// `FUN_141659cf0`) each object the world holds (any player's: the handler has no owner
    /// test) with a unit AI that may scout (`FUN_140c89760`: its master's object flags `+0x654`
    /// have bit 4; class 58's rule behind a game setting, `FUN_140eca780`, is not modelled)
    /// takes an order at once (slot 0x30 -> slot 0x28 with Immediate,
    /// [`crate::unit_ai::UnitAi::take_order_now`]): the stop order when its action is the auto
    /// scout's already, else the auto scout order, from the object's owner (`+0x718`), without
    /// a target, at the point (−1, −1) at the unit's height, range 1, priority 100, shown at
    /// (−1, −1) and ready. Its sound and the UI's flags are not held.
    pub(crate) fn auto_scout(&mut self, player: usize, ids: &[i32]) -> Result<(), Unsupported> {
        use crate::unit_ai::{
            AUTO_SCOUT_ACTION, AUTO_SCOUT_ORDER, Effect, QueuedOrder, STOP_ORDER,
        };
        let cap = if self.players[player].computer {
            500
        } else {
            60
        };
        for &id in ids.iter().take(cap) {
            let Some(place) = self.place(id) else {
                continue;
            };
            let (p, idx) = match place {
                Place::Live(p, i) => (p, i),
                // only slot 0x180 puts an object to sleep, one of a type below 30, which has no
                // unit AI: the handler passes it over
                Place::Sleeping(p, i) if self.players[p].sleeping[i].ai.is_none() => continue,
                Place::Sleeping(..) => {
                    return Err(Unsupported(
                        "an AUTO_SCOUT of a sleeping object with a unit AI",
                    ));
                }
            };
            let unit = &self.players[p].objects[idx];
            if unit.ai.is_none() {
                continue;
            }
            let master = self.master_of(p, unit.life.master_id)?;
            if master.object_flags & 0x10 == 0 {
                if master.class == AUTO_SCOUT_CLASS {
                    return Err(Unsupported("AUTO_SCOUT's class 58 rule (FUN_140eca780)"));
                }
                continue;
            }
            let class = master.class;
            let unit = &mut self.players[p].objects[idx];
            let (state, position) = (unit.life.state, unit.position);
            let ai = unit.ai.as_mut().expect("checked above");
            let order = QueuedOrder {
                issuer: p as i32,
                order: if ai.current_action == AUTO_SCOUT_ACTION {
                    STOP_ORDER
                } else {
                    AUTO_SCOUT_ORDER
                },
                priority: 100,
                target: -1,
                target_owner: -1,
                point: [-1.0, -1.0, position[2]],
                range: 1.0,
                shown: [-1.0, -1.0],
                ready: true,
                target_type: -1,
            };
            let mut fx = Vec::new();
            ai.take_order_now(state, order, ([position[0], position[1]], class), &mut fx)?;
            for e in fx {
                match e {
                    Effect::Stop => self.command_stop(p, idx)?,
                    Effect::Explore(point) => self.command_explore(p, idx, point)?,
                    _ => return Err(Unsupported("an order effect of AUTO_SCOUT not modelled")),
                }
            }
        }
        Ok(())
    }

    /// STOP (0x01, `RGE_Command::do_command` case 1) of the player's units `ids`: the command's
    /// check ([`Turns::command_check`]), the lookup ([`Turns::lookup`]), then `FUN_14161e200`. First
    /// each unit AI resets as for an order (`FUN_141631040`, the loop of `FUN_140e6cfc0`:
    /// [`crate::unit_ai::UnitAi::reset_for_order`]). Several entries (a gone unit's null among
    /// them) that are exactly one unit group's members (`FUN_14152ec80`, the group's size
    /// `+0xe4`) stop as the group: each unit AI's order queue empties and its idle timer takes its
    /// adjusted timeout, then the group's stop (`FUN_1410110c0`), which is not modelled. Else each
    /// unit leaves the unit group the world holds of its link (`FUN_14102c160` with -1 and 0,
    /// [`Turns::remove_unit`]), its unit AI's order queue empties (`FUN_140c5e010`) and its idle
    /// timer takes its adjusted timeout (`+0x160` = `+0x164`), its ignore list empties
    /// (`FUN_141197da0`), the object stops (slot 0x148, [`Turns::object_stop`]) and its movement
    /// stands (slot 0x560, `FUN_1411616f0`: slot 0x5d0, then `FUN_140f98e80` on its walked
    /// level).
    pub(crate) fn stop_command(&mut self, player: usize, ids: &[i32]) -> Result<(), Unsupported> {
        let Some(ids) = self.command_check(player, ids) else {
            return Ok(());
        };
        let list: Vec<Option<i32>> = self
            .lookup(&ids)
            .into_iter()
            .zip(&ids)
            .map(|(place, &id)| place.map(|_| id))
            .collect();
        let units: Vec<i32> = list.iter().flatten().copied().collect();
        for &id in &units {
            let unit = match self.place(id) {
                Some(Place::Live(p, i)) => &mut self.players[p].objects[i],
                Some(Place::Sleeping(p, i)) => &mut self.players[p].sleeping[i],
                None => continue,
            };
            if let Some(ai) = unit.ai.as_mut() {
                ai.reset_for_order(self.streams.get(Stream::Ai));
            }
        }
        if 1 < list.len() {
            let entries: Vec<i32> = list.iter().map(|u| u.unwrap_or(-1)).collect();
            let group =
                crate::group::common_group(&entries, |id| self.object(id).map(|(_, u)| u.group.id));
            if self
                .groups
                .get(group)
                .is_some_and(|g| g.members.len() == list.len())
            {
                return Err(Unsupported("a STOP of a whole unit group (FUN_1410110c0)"));
            }
        }
        for &id in &units {
            let link = self.object(id).map_or(-1, |(_, u)| u.group.id);
            if link != -1 {
                let _ = self.remove_unit(link, id, -1, false)?;
            }
            let Some((_, u)) = self.object_mut(id) else {
                continue;
            };
            if let Some(ai) = u.ai.as_mut() {
                ai.orders.clear();
                ai.idle_timer = ai.adjusted_idle_timeout;
            }
            if let Some(m) = u.mover.as_mut() {
                m.ignore_units.clear();
            }
            // a sleeping object is of a type below 30 (only slot 0x180 puts one to sleep), whose
            // slots 0x148 and 0x560 are empty (0x141072730, 0x141072740)
            let Some(Place::Live(p, idx)) = self.place(id) else {
                continue;
            };
            self.object_stop(p, idx)?;
            if let Some(m) = self.players[p].objects[idx].mover.as_mut() {
                m.stand()?;
            }
        }
        Ok(())
    }

    /// The check of a command on units (STOP's `FUN_140e55a30` and `UNIT_AI_STATE`'s `FUN_140e55d30`,
    /// twins of `FUN_140e558b0`) of the ids of `player`: of the first 60 (500 for a computer,
    /// `FUN_141659cf0`) an id the world does not hold, or another player's without the setting
    /// `+0x1074` ([`crate::settings::Settings::cheats_enabled`]), becomes -1; the ids past them stay unchecked. A
    /// human's command whose checked ids all went is refused (None).
    /// The packet's own tests (its size, a player the game lacks) are the decoder's, and the
    /// world's `+0xb12` (the cheat that would let either through) is not held.
    pub(crate) fn command_check(&self, player: usize, ids: &[i32]) -> Option<Vec<i32>> {
        let computer = self.players[player].computer;
        let checked = ids.len().min(if computer { 500 } else { 60 });
        let mut ids = ids.to_vec();
        let mut dropped = 0;
        for id in &mut ids[..checked] {
            let keep = self
                .object(*id)
                .is_some_and(|(p, _)| p == player || self.settings.cheats_enabled);
            if *id == -1 || !keep {
                *id = -1;
                dropped += 1;
            }
        }
        if checked <= dropped && !computer {
            return None;
        }
        Some(ids)
    }

    /// `FUN_140e6b750`: each id's object (`FUN_1415417a0`, live or sleeping), None where the
    /// world holds none; the list keeps the ids' count.
    pub(crate) fn lookup(&self, ids: &[i32]) -> Vec<Option<Place>> {
        ids.iter().map(|&id| self.place(id)).collect()
    }

    /// stopAction (slot 0x3c0, `FUN_14108c160`) in the command phase: the action list empties
    /// (`FUN_140a98620`, [`Turns::clear_actions`]) and a unit in state 2 takes its standing
    /// sprite (slot 0x798, then slot 0x68) where the slice holds its sprite.
    pub(crate) fn command_stop(&mut self, p: usize, idx: usize) -> Result<(), Unsupported> {
        if self.players[p].objects[idx].unmodelled_action.is_some() {
            return Err(Unsupported(
                "a command's stop of an action in the command phase",
            ));
        }
        self.clear_actions(p, idx)?;
        if self.players[p].objects[idx].life.state == 2 {
            self.standing_sprite(p, idx)?;
        }
        Ok(())
    }

    /// The unit's explore (slot 0x398) in the command phase: a unit without an Explore takes a
    /// new one ([`crate::explore::ExploreAction::new`]) at its height; its `set_state`'s notify and
    /// standing sprite follow.
    fn command_explore(
        &mut self,
        p: usize,
        idx: usize,
        point: [f32; 2],
    ) -> Result<(), Unsupported> {
        use crate::explore::{EXPLORE_ACTION, ExploreAction, Owner, WANTS_POINT};
        let unit = &mut self.players[p].objects[idx];
        if unit.explore.is_some() || unit.top_action().is_some() {
            return Err(Unsupported("a command's explore of a unit with an action"));
        }
        let (e, owner) = ExploreAction::new([point[0], point[1], unit.position[2]]);
        unit.explore = Some(e);
        match owner {
            Owner::Nothing => return Ok(()),
            Owner::WantsPoint => {
                let (id, state) = (unit.id, unit.life.state);
                if let Some(ai) = unit.ai.as_mut() {
                    let n = crate::unit_ai::Notify::own(id, WANTS_POINT, [EXPLORE_ACTION, 0, 0]);
                    ai.post(n, state)?;
                }
            }
            Owner::Standing => {}
            Owner::Stopped => return Err(Unsupported("an Explore's set_state 2 (slots 0x68)")),
        }
        {
            self.standing_sprite(p, idx)?;
        }
        Ok(())
    }

    /// Slot 0x68 with the master's standing sprite (`+0x470`): the unit's top speed is the
    /// sprite's ([`crate::movement::Mover::shows_standing`]), and the sprite itself where the
    /// slice holds it.
    pub(crate) fn standing_sprite(&mut self, p: usize, idx: usize) -> Result<(), Unsupported> {
        let unit = &mut self.players[p].objects[idx];
        if let Some(m) = unit.mover.as_mut() {
            m.shows_standing();
        }
        if unit.sprite.is_none() {
            return Ok(());
        }
        let master = self.players[p].objects[idx].life.master_id;
        let standing = self.master_of(p, master)?.standing_sprite;
        let g = self
            .graphics
            .get(&standing)
            .copied()
            .ok_or(Unsupported("a unit without its standing sprite"))?;
        let rng = self.streams.get(Stream::Sprite);
        crate::attack::set_sprite(&mut self.players[p].objects[idx].sprite, g, rng);
        Ok(())
    }

    /// One unit of MOVE: an object that does not move ignores an order not queued. The unit's
    /// ignore list empties (`FUN_141197da0`), it waits to move (slot 0x2a0 with 1) and lists the
    /// order's `others` (`FUN_14118c790`). An order that is queued, or neither direct nor to a
    /// unit AI with every queued order ready (`FUN_140c4dfa0`), goes to its unit AI's queue as
    /// order `0x2c6` at its point (slot 0x30, or in a grid slot 0x28: range 0, priority 100, the
    /// order's point shown, ready when direct); a unit without a unit AI then does nothing.
    /// Otherwise the unit takes the move itself (slot 0x138) to the point, or to its grid `cell`.
    fn move_unit(
        &mut self,
        player: usize,
        id: i32,
        others: &[i32],
        cell: Option<[f32; 2]>,
        order: &MoveOrder,
    ) -> Result<(), Unsupported> {
        let point2 = cell.unwrap_or(order.point);
        let Some(place) = self.place(id) else {
            return Ok(());
        };
        let Place::Live(p, idx) = place else {
            return Err(Unsupported("a move of a unit the slice does not hold live"));
        };
        // the command's check (`FUN_140e553f0`) drops another player's unit (its id -1) unless
        // the game setting `+0x1074` (`FUN_1412ffe00`, not held) lets players order each
        // other's units
        if p != player {
            return Err(Unsupported(
                "a move of another player's unit (the setting +0x1074)",
            ));
        }
        let issuer = player;
        let master = self.master_of(player, self.players[player].objects[idx].life.master_id)?;
        let speed = master.moving.as_ref().map_or(0.0, |m| m.speed);
        if master.kind < MOVING || speed <= 0.0 {
            if order.queued {
                return Err(Unsupported("a queued move of an object that does not move"));
            }
            return Ok(());
        }
        let target = self
            .object(order.target)
            .map(|(tp, t)| (t.life.master_id, tp));
        let unit = &mut self.players[player].objects[idx];
        // FUN_141197da0, then slot 0x2a0 with 1: it waits to move
        let mover = unit
            .mover
            .as_mut()
            .ok_or(Unsupported("a moved unit without movement state"))?;
        mover.ignore_units.clear();
        mover.waiting = 1;
        for &other in others {
            ignore_unit(&mut mover.ignore_units, other);
        }
        let takes_it = !order.queued
            && unit
                .ai
                .as_ref()
                .is_none_or(|ai| order.direct || !ai.has_unready_order());
        if !takes_it {
            let state = unit.life.state;
            let Some(ai) = unit.ai.as_mut() else {
                return Ok(());
            };
            let (target_type, target_owner) =
                target.map_or((-1, -1), |(m, o)| (i32::from(m), o as i32));
            let queued = crate::unit_ai::QueuedOrder {
                issuer: issuer as i32,
                order: MOVE_ORDER,
                priority: 100,
                target: if target.is_some() { order.target } else { -1 },
                target_owner,
                point: [point2[0], point2[1], 0.0],
                range: 0.0,
                shown: order.point,
                ready: order.direct,
                target_type,
            };
            let _ = ai.queue_order(state, queued, false, false)?;
            if !order.queued && !order.direct {
                return Err(Unsupported(
                    "the readiness pass after a move neither queued nor direct (FUN_140c42b60)",
                ));
            }
            return Ok(());
        }
        // slot 0x138 (FUN_141216f10): a villager's town-bell flag (not held) and, `+0xa14` being
        // 1, the unit AI's order queue empties (FUN_140c5e010); then FUN_14106d340
        if let Some(ai) = unit.ai.as_mut() {
            ai.orders.clear();
        }
        if order.target == id {
            return Ok(());
        }
        if order.target != -1 {
            return Err(Unsupported("a move that follows an object"));
        }
        let order = &MoveOrder {
            point: point2,
            ..*order
        };
        let taken = self.move_to_point(player, idx, [order.point[0], order.point[1], 0.0])?;
        // the order handler's notify: a unit that took the move as a MoveTo gets the move order
        // and action, its target forgotten (FUN_140c4dcb0); every unit then the point
        // (FUN_140c64be0); in a grid every unit gets the order, the action and its cell, its
        // target kept; slot 0x2d8's walk and show targets are not held
        let unit = &mut self.players[player].objects[idx];
        if let Some(ai) = unit.ai.as_mut() {
            let ai_stream = self.streams.get(Stream::Ai);
            match cell {
                None => ai.notify_move(!taken, None, order.point, ai_stream),
                Some(c) => ai.notify_grid_move(c, ai_stream),
            }
        }
        Ok(())
    }

    /// `FUN_14106d340` on a move of the unit at `idx` to `point`: a unit with a unit AI
    /// (`FUN_140c5dc10` clears its notifies with the world time) lets its head action take the
    /// point ([`Turns::head_takes_move`]) unless it stops after a kill (`FUN_140c41c10`); else a
    /// `MoveTo` to the point replaces its actions. True when the action took it.
    pub(crate) fn move_to_point(
        &mut self,
        player: usize,
        idx: usize,
        point: [f32; 3],
    ) -> Result<bool, Unsupported> {
        // a unit with a unit AI (FUN_140c5dc10 clears its notifies with the world time) lets its
        // head action's slot 0x40 take the point unless it stops after a kill (FUN_140c41c10)
        let world_time = self.world_time;
        let unit = &mut self.players[player].objects[idx];
        if let Some(ai) = unit.ai.as_mut() {
            ai.clear_notifies(world_time)?;
        }
        let stop_after_kill = unit.ai.as_ref().is_some_and(|ai| ai.stop_after_kill);
        let taken = unit.ai.is_some()
            && !stop_after_kill
            && self.head_takes_move(player, idx, None, point)?;
        if !taken {
            // the MoveTo to the point (FUN_140aa0d30, range 0) and its install (slot 0x808)
            self.clear_actions(player, idx)?;
            let unit = &mut self.players[player].objects[idx];
            unit.move_to = Some(crate::move_to::MoveToAction {
                run: false,
                // FUN_140aa2100: no target, so far from it: state 3 for a unit with speed
                state: crate::move_to::MoveTo::Start,
                goal: Some(point),
                target: -1,
                range: 0.0,
                wait: crate::move_to::MoveWait::default(),
            });
        }
        Ok(taken)
    }

    /// The unit's slot 0x340 (`FUN_141216c50`) forced, from its unit AI's move off a footprint
    /// (slot 0x148): a unit of class 51 or 54 (packing) is not modelled; any other runs
    /// `FUN_1410866e0`: its head action's slot 0x40 may take the point
    /// ([`Turns::head_takes_move`]), else a `MoveTo` to the point
    /// within the move's range, walking or running, replaces its actions.
    pub(crate) fn move_aside(
        &mut self,
        player: usize,
        idx: usize,
        m: crate::unit_ai::AiMove,
    ) -> Result<(), Unsupported> {
        let unit = &self.players[player].objects[idx];
        let class = self.master_of(player, unit.life.master_id)?.class;
        if matches!(class, CLASS_51 | CLASS_54) {
            return Err(Unsupported(
                "a move off a footprint of a unit of class 51 or 54",
            ));
        }
        let point = m.point;
        let taken = self.head_takes_move(player, idx, None, point)?;
        if !taken {
            self.clear_actions(player, idx)?;
            let unit = &mut self.players[player].objects[idx];
            unit.move_to = Some(crate::move_to::MoveToAction {
                run: !m.walk,
                state: crate::move_to::MoveTo::Start,
                goal: Some(point),
                target: -1,
                range: m.range,
                wait: crate::move_to::MoveWait::default(),
            });
        }
        Ok(())
    }

    /// Slot 0x7e8 (`FUN_141227190`) of a unit on a target object: a unit in state 3 or later, or
    /// of class 54 under its own Pack or Unpack (`0x7d`, `0x7c`), ignores it. Against a target
    /// the unit is not allied with (`FUN_141663d90` with allies) [`work_reach`]
    /// (`FUN_1410862c0`) decides: class 54 out of reach packs (`FUN_140ad4ff0`), class 51 in
    /// reach unpacks (`FUN_140ae58a0`), each through slot 0x808; else a target below the least
    /// range is the unit's notify [`TOO_CLOSE`] to its own unit AI (slot 0x328) and nothing more.
    /// Without a target the world holds (none, or one gone) the work command goes to the point,
    /// and a computer's class 54 packs instead. Neither action is held. A villager drops its
    /// town-bell flag (`FUN_1412067f0`); then the work command; then it leaves the pasture it is
    /// in (`FUN_141189ca0`).
    pub(crate) fn order_work(
        &mut self,
        player: usize,
        idx: usize,
        order: &Work,
    ) -> Result<(), Unsupported> {
        let unit = &self.players[player].objects[idx];
        let master = self.master_of(player, unit.life.master_id)?;
        // the class-54 test of its top action comes first in the game, but either way the order
        // is ignored
        if 2 < unit.life.state {
            return Ok(());
        }
        let class = master.class;
        let target = self.object(order.target).map(|(tp, _)| tp);
        let world = World {
            players: &self.players,
            drop_sites: &self.drop_sites,
        };
        let reach = match target {
            Some(tp) if !world.allied(player, tp, true) => {
                let (me, them) = (
                    self.spatial
                        .get(unit.id)
                        .ok_or(Unsupported("an ordered unit without a place"))?,
                    self.spatial
                        .get_known(order.target)?
                        .ok_or(Unsupported("a target without a place"))?,
                );
                let (_, gap) = gaps(me.position, me.radius, them.position, them.radius);
                Some(work_reach(gap, master.weapon_range, master.min_range))
            }
            _ => None,
        };
        let packs = match reach {
            None => target.is_none() && class == CLASS_54 && self.players[player].computer,
            Some((within, _)) => (class == CLASS_54 && !within) || (class == CLASS_51 && within),
        };
        // a class 54 under an action the slice does not hold may be under its Pack or Unpack
        if packs || (class == CLASS_54 && unit.unmodelled_action.is_some()) {
            return Err(Unsupported(
                "a Pack or Unpack of class 54 or 51 (FUN_140ad4ff0, FUN_140ae58a0)",
            ));
        }
        if reach.is_some_and(|(_, too_close)| too_close) {
            let unit = &mut self.players[player].objects[idx];
            unit.post_own(TOO_CLOSE, [order.target, -1, -1])?;
            return Ok(());
        }
        if target.is_none() {
            // no target, or one the world does not hold (slot 0x7e8 with none): the work command
            // at the point
            let order = Work {
                target: -1,
                ..*order
            };
            self.town_bell_order(player, idx, -1, order.filter)?;
            self.work_command(player, idx, &order)?;
            if self.players[player].objects[idx].my_pasture != -1 {
                return Err(Unsupported("a unit that leaves its pasture"));
            }
            return Ok(());
        }
        let master = self.master_of(player, self.players[player].objects[idx].life.master_id)?;
        // livestock (class 58) takes the enter command (slot 0x3a0); where it enters nothing, the
        // work command goes to the target's place without a target
        let mut order = *order;
        if master.class == LIVESTOCK {
            if self.livestock_enters(player, idx, order.target)? {
                return Err(Unsupported(
                    "livestock entering its target (slot 0x3a0's Enter)",
                ));
            }
            let at = self
                .spatial
                .get_known(order.target)?
                .ok_or(Unsupported("a target without a place"))?
                .position;
            (order.point[0], order.point[1], order.target) = (at[0], at[1], -1);
        }
        let order = &order;
        self.town_bell_order(player, idx, order.target, order.filter)?;
        self.work_command(player, idx, order)?;
        let unit = &self.players[player].objects[idx];
        let master = self.master_of(player, unit.life.master_id)?;
        if order.target != -1 {
            let target = self.task_target(order.target, 0)?;
            if master.class == VILLAGER
                && target.master.kind == BUILDING
                && matches!(target.master.id, MASTER_1893 | MASTER_1897)
            {
                return Err(Unsupported("a villager's order on master 1893 or 1897"));
            }
        }
        let unit = &mut self.players[player].objects[idx];
        if unit.my_pasture != -1 {
            return Err(Unsupported("a unit that leaves its pasture"));
        }
        Ok(())
    }

    /// Slot 0x3a0 (`FUN_1410847e0`, the enter command, with 0) of the unit at `idx` on `target`:
    /// true when it enters, false when nothing comes of it (a target the world does not hold, or
    /// no task). It is entering the target already (its top action an Enter on it); else slot
    /// 0x820 ([`Turns::find_work_task`]) looks for a task on the target with the garrison filter
    /// (3) and the master's change, whose Enter it would take.
    fn livestock_enters(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
    ) -> Result<bool, Unsupported> {
        if self.object(target).is_none() {
            return Ok(false);
        }
        let unit = &self.players[player].objects[idx];
        if unit.top_action_kind() == Some(crate::garrison::ENTER)
            && unit.top_target() == Some(target)
        {
            return Ok(true);
        }
        Ok(self
            .find_work_task(player, idx, target, GARRISON, true)?
            .is_some())
    }

    /// Slot 0x138 (`FUN_141216f10`) without a target, outside slot 0x128's work command (the
    /// unit's `+0xa14` 1): [`Turns::move_point_order_with`].
    pub(crate) fn move_point_order(
        &mut self,
        player: usize,
        idx: usize,
        point: [f32; 3],
    ) -> Result<(), Unsupported> {
        self.move_point_order_with(player, idx, point, true)
    }

    /// Slot 0x138 (`FUN_141216f10`) without a target, the no-task end of a work command on a
    /// point: a villager drops its town-bell flag (not held otherwise), the unit AI's order queue
    /// empties (`FUN_140c5e010`) while the unit's `+0xa14` (`a14`) is set, then the move to the
    /// point ([`Turns::move_to_point`], `FUN_14106d340`). `+0xa14` is 1 but inside slot 0x128
    /// (`FUN_141211b80`), which holds it at its argument while its work command runs.
    pub(crate) fn move_point_order_with(
        &mut self,
        player: usize,
        idx: usize,
        point: [f32; 3],
        a14: bool,
    ) -> Result<(), Unsupported> {
        let villager = self
            .master_of(player, self.players[player].objects[idx].life.master_id)?
            .class
            == VILLAGER;
        let unit = &mut self.players[player].objects[idx];
        if villager {
            unit.town_bell.flag = false;
        }
        if a14 && let Some(ai) = unit.ai.as_mut() {
            ai.orders.clear();
        }
        let _ = self.move_to_point(player, idx, point)?;
        Ok(())
    }

    /// `FUN_1412067f0` before slot 0x7e8's work command, for a villager or master 1808 and a
    /// target the world holds: the town-bell record ([`Turns::record_town_bell`]); then, with the
    /// garrison filter (3), the flag goes on for a building in state 2 of the player or an ally
    /// whose master takes villagers (`garrison_type` bit 0), with any other filter it goes off.
    /// The UI's flag for the viewing player's building (`+0x1874`) is not held.
    pub(crate) fn town_bell_order(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        filter: i16,
    ) -> Result<(), Unsupported> {
        let unit = &self.players[player].objects[idx];
        let master = self.master_of(player, unit.life.master_id)?;
        if !(master.class == VILLAGER || master.id == MASTER_1808) {
            return Ok(());
        }
        let Some((tp, t)) = self.object(target) else {
            return Ok(());
        };
        let takes = if filter == GARRISON {
            let tm = self.master_of(tp, t.life.master_id)?;
            let world = World {
                players: &self.players,
                drop_sites: &self.drop_sites,
            };
            (tm.kind == BUILDING
                && t.life.state == 2
                && world.allied(player, tp, true)
                && tm.building.is_some_and(|b| b.garrison_type & 1 != 0))
            .then_some(true)
        } else {
            Some(false)
        };
        self.record_town_bell(player, idx)?;
        if let Some(flag) = takes {
            self.players[player].objects[idx].town_bell.flag = flag;
        }
        Ok(())
    }

    /// `FUN_141206a20`: the unit's present work as its town-bell record
    /// ([`crate::town_bell::TownBell::record`]): its top action's target (slot 400), or, for one
    /// in a pasture the world holds, the pasture; without one the target of its unit AI's first
    /// own queued order (`FUN_140c4da50`: issued by the unit, with a target), else none at its own
    /// place. Then a Gather's or Hunt's target class (`+0x68`), its unit AI's current action
    /// (`+0x44`) and the top action's second target (`FUN_140a78430`). The targets count only
    /// where the world holds them.
    pub(crate) fn record_town_bell(
        &mut self,
        player: usize,
        idx: usize,
    ) -> Result<(), Unsupported> {
        use crate::town_bell::BackToWork;
        let unit = &self.players[player].objects[idx];
        if unit.town_bell.flag {
            return Ok(());
        }
        if unit.unmodelled_action.is_some() {
            return Err(Unsupported("a town-bell record of an action not held"));
        }
        let held = |id: i32| (0 <= id).then(|| self.object(id)).flatten();
        let mut found = unit.top_target().and_then(held).map(|(_, t)| t);
        if let Some(t) = found
            && t.pasture != -1
        {
            found = held(t.pasture).map(|(_, p)| p);
        }
        if found.is_none() {
            let own = unit
                .ai
                .as_ref()
                .map_or(-1, |ai| own_order_target(ai, unit.id));
            found = held(own).map(|(_, t)| t);
        }
        let (target, point) = found.map_or((-1, [unit.position[0], unit.position[1]]), |t| {
            (t.id, [t.position[0], t.position[1]])
        });
        let target_type = match unit.top_action_kind() {
            Some(HUNT) => unit.hunt.map_or(-1, |h| i32::from(h.target_class)),
            Some(crate::gather::GATHER) => unit.gather.map_or(-1, |g| g.target_type),
            _ => -1,
        };
        let ai = unit.ai.as_ref().ok_or(Unsupported(
            "a town-bell record of a unit without a unit AI",
        ))?;
        let target2 = unit
            .top_second_target()
            .and_then(held)
            .map_or(-1, |(_, t)| t.id);
        let now = BackToWork {
            target,
            target_type,
            action: ai.current_action,
            target2,
            point,
        };
        self.players[player].objects[idx].town_bell.record(now);
        Ok(())
    }

    /// The work command's check of the unit's top Attack (`FUN_14106ff70`): one on this very
    /// target, the filter none or 9, keeps it when its slot 9 (`FUN_140a841d0`, [`Attack::keeps`])
    /// does, and the unit AI hears of it as of a new one (slot 0x828: the attack order and
    /// action, the target, the retarget time). True when kept.
    ///
    /// [`Attack::keeps`]: crate::attack::Attack::keeps
    fn keep_attack(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        filter: i16,
        point: [f32; 3],
        clears: bool,
    ) -> Result<bool, Unsupported> {
        let unit = &self.players[player].objects[idx];
        let top = unit.top_action_kind();
        let attacking = unit
            .attack
            .filter(|a| top == Some(crate::attack::ATTACK) && a.target == target);
        let Some(a) = attacking.filter(|_| matches!(filter, -1 | 9)) else {
            return Ok(false);
        };
        if !a.keeps(target)? {
            return Ok(false);
        }
        let (tp, t) = self
            .object(target)
            .ok_or(Unsupported("an attack's target the slice does not hold"))?;
        let class = i32::from(self.master_of(tp, t.life.master_id)?.class);
        let world_time = self.world_time;
        let unit = &mut self.players[player].objects[idx];
        if let Some(ai) = unit.ai.as_mut() {
            ai.last_retarget_time = world_time;
            ai.notify_work(
                700,
                600,
                Some((target, class)),
                point,
                clears,
                self.streams.get(Stream::Ai),
            )?;
        }
        Ok(true)
    }

    /// The unit's own task list's finder with filter 3 (slot 2 of its master's `+0x788`,
    /// `FUN_1412784d0`: the garrison's, [`crate::tasks::find_task`]) on `target`: whether it
    /// finds a task, as a gather point on a building asks (`FUN_140ace9c0`).
    pub(crate) fn garrison_task_found(
        &self,
        player: usize,
        idx: usize,
        target: i32,
    ) -> Result<bool, Unsupported> {
        let unit = &self.players[player].objects[idx];
        let owner = &self.players[player];
        let master = self.master_of(player, unit.life.master_id)?;
        let actor = Actor {
            id: unit.id,
            master,
            player,
            held: (unit.life.attribute_type, unit.life.amount),
            attribute_95: owner.attributes.get(95).copied().unwrap_or(0.0),
            civ: owner.civ,
        };
        let world = World {
            players: &self.players,
            drop_sites: &self.drop_sites,
        };
        let t = self.task_target(target, 1)?;
        let general = self.task_target(target, 2)?;
        Ok(
            crate::tasks::find_task(master, &actor, Some(&t), Some(&general), None, 3, &world)?
                .is_some(),
        )
    }

    /// The work command (slot 37, `FUN_14106ff70`) of a unit on a target object: the master and
    /// task of [`crate::masters::Masters::work_master`] (the master changes, slot 0x90), the task's
    /// requirement, then the task's action from the action list's factory (slot 0x10), the unit
    /// AI's notify (slot 0x828) and the install (slot 0x808).
    fn work_command(&mut self, player: usize, idx: usize, order: &Work) -> Result<(), Unsupported> {
        let Work {
            target,
            point,
            carry_test,
            filter,
            notified,
        } = *order;
        let unit = &self.players[player].objects[idx];
        if unit.id == target {
            return Ok(());
        }
        // FUN_140c64e50: a unit of a type below 70 always clears its AI's notifies
        let clears = notified || self.master_of(player, unit.life.master_id)?.kind < 70;
        // a work command on a point (no target) finds no task
        let found = if target == -1 {
            None
        } else {
            let masters = self
                .masters
                .get(player)
                .ok_or(Unsupported("a work command: no master table"))?;
            let owner = &self.players[player];
            let actor = Actor {
                id: unit.id,
                master: self.master_of(player, unit.life.master_id)?,
                player,
                held: (unit.life.attribute_type, unit.life.amount),
                attribute_95: owner.attributes.get(95).copied().unwrap_or(0.0),
                civ: owner.civ,
            };
            let world = World {
                players: &self.players,
                drop_sites: &self.drop_sites,
            };
            let t = self.task_target(target, 1)?;
            let general = self.task_target(target, 2)?;
            masters
                .work_master(&actor, &t, &general, filter, &world)?
                .map(|(with, task)| (with, *task))
        };
        let Some((with, task)) = found else {
            if filter != -1 {
                return Ok(());
            }
            // LAB_1410706a8 without a task: the top Attack on the target keeps it; else a unit
            // whose master moves (slot 0xe8, its speed `+0x740`) is told of a move (slot 0x828)
            // and moves there (slot 0x138), which is not modelled; one that does not, nothing
            if self.keep_attack(player, idx, target, filter, point, clears)? {
                return Ok(());
            }
            return self.work_move(player, idx, target, point, (clears, notified));
        };
        // a new master's task is another than the one the unit's action holds
        let same_master = with == self.players[player].objects[idx].life.master_id;
        if !same_master {
            self.change_master(player, idx, with)?;
        }
        if !enabled(&self.players[player].attributes, task.enablement) {
            return Ok(());
        }
        // LAB_1410706a8: an action on the task found takes the new target itself (its slot 9)
        let unit = &self.players[player].objects[idx];
        let top = unit.top_action_kind();
        let kept = match unit.hunt {
            Some(h) if task.action_type == HUNT && h.task.id == task.id => {
                let t = self.task_target(target, 0)?;
                if t.master.id == MASTER_84 && t.player == player {
                    return Err(Unsupported("a hunt sent to its player's master 84"));
                }
                let class = i32::from(t.master.class);
                self.retarget_hunt(player, idx, target)?;
                let unit = &mut self.players[player].objects[idx];
                if let Some(ai) = unit.ai.as_mut() {
                    ai.notify_work(
                        HUNT_ORDER,
                        HUNT_ACTION,
                        Some((target, class)),
                        point,
                        clears,
                        self.streams.get(Stream::Ai),
                    )?;
                }
                true
            }
            _ if top == Some(task.action_type) && task.action_type == crate::build::BUILD => {
                if !same_master {
                    return Err(Unsupported(
                        "a work command on the Build task of the unit's former master",
                    ));
                }
                self.retake_build(player, idx, target, point, clears)?;
                true
            }
            _ if top == Some(crate::gather::GATHER)
                && task.action_type == crate::gather::GATHER
                && unit.gather.is_some_and(|g| g.task.id == task.id) =>
            {
                let t = self.task_target(target, 0)?;
                if t.master.id == MASTER_84 && t.player == player {
                    return Err(Unsupported("a gather sent to its player's master 84"));
                }
                let class = i32::from(t.master.class);
                let took = self.retarget_gather(player, idx, target)?;
                if took {
                    let unit = &mut self.players[player].objects[idx];
                    if let Some(ai) = unit.ai.as_mut() {
                        ai.notify_work(
                            crate::ai_kinds::ORDER_GATHER,
                            crate::unit_ai::GATHER_ACTION,
                            Some((target, class)),
                            point,
                            clears,
                            self.streams.get(Stream::Ai),
                        )?;
                    }
                }
                took
            }
            _ if top == Some(task.action_type) && top != Some(HUNT) => {
                return Err(Unsupported("a work command on the unit's current task"));
            }
            _ => false,
        };
        if carry_test {
            return Err(Unsupported(
                "a work command with the carried-resource test (+0x12)",
            ));
        }
        if self.keep_attack(player, idx, target, filter, point, clears)? {
            return Ok(());
        }
        if matches!(top, Some(9 | 0x7e)) || (top == Some(0x6b) && task.action_type == 3) {
            return Err(Unsupported(
                "a work command on a unit attacking, gathering or 0x7e",
            ));
        }
        if kept {
            return Ok(());
        }
        match task.action_type {
            crate::garrison::ENTER => self.start_enter(player, idx, target, &task, point, clears),
            ATTACK_TASK => self.start_attack(player, idx, target, &task, point, clears),
            HUNT => self.start_hunt(
                player,
                idx,
                target,
                HuntTask {
                    id: task.id,
                    range: task.work_range,
                    resource: task.attribute_type,
                    sprites: task.sprites,
                    work_value: task.work_val1,
                    productivity: task.attribute_type2,
                    kind4: task.attribute_type4,
                },
                point,
                clears,
                true,
            ),
            crate::build::BUILD => self.start_build(player, idx, target, &task, point, clears),
            // the factory makes no action of a mule cart's task: the move without one
            crate::build::CART_TASK => {
                self.work_move(player, idx, target, point, (clears, notified))
            }
            crate::gather::GATHER => {
                self.start_gather(player, idx, target, &task, Some((point, clears)))
            }
            _ => Err(Unsupported(
                "the action factory for a task the slice does not model",
            )),
        }
    }

    /// The work command without an action (`FUN_14106ff70`: no task, or one the action list's
    /// factory makes no action of, the mule cart's 0x99): a unit whose master moves (slot 0xe8,
    /// its speed `+0x740`) is told of a move (slot 0x828, its notifies cleared with `clears`) and,
    /// in state 2 or of class 51, moves to the target or the point (slot 0x138 with the unit's
    /// `+0xa14`, the work's `notified`); one that does not move does nothing.
    fn work_move(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        point: [f32; 3],
        (clears, notified): (bool, bool),
    ) -> Result<(), Unsupported> {
        let unit = &self.players[player].objects[idx];
        let master = self.master_of(player, unit.life.master_id)?;
        if !master.moving.is_some_and(|m| 0.0 < m.speed) {
            return Ok(());
        }
        // slot 0x828 with 1 (the move's order and action) for a unit in state 2, -1 for
        // another; then the move to the target (slot 0x138) for a unit in state 2 or of
        // class 51
        let (alive, class) = (unit.life.state == 2, master.class);
        let named = if target == -1 {
            None
        } else {
            let (tp, t) = self.object(target).ok_or(Unsupported(
                "a work command's target the slice does not hold",
            ))?;
            Some((
                target,
                i32::from(self.master_of(tp, t.life.master_id)?.class),
            ))
        };
        let (order, action) = if alive {
            (MOVE_ORDER, crate::unit_ai::MOVE_ACTION)
        } else {
            (-1, -1)
        };
        let unit = &mut self.players[player].objects[idx];
        if let Some(ai) = unit.ai.as_mut() {
            ai.notify_work(
                order,
                action,
                named,
                point,
                clears,
                self.streams.get(Stream::Ai),
            )?;
        }
        if alive || class == 51 {
            if target == -1 {
                self.move_point_order_with(player, idx, point, notified)?;
            } else {
                let _ = self.move_to_object_with(player, idx, target, point, notified)?;
            }
        }
        Ok(())
    }

    /// The unit's slot 0x820 (`FUN_14106c6a0`) on `target` (its place the point): the task its
    /// master's list gives for the target with `filter` (the list's slot 0x10, [`find_task`] in
    /// mode 0); without one, a master with a task swap group (`+0x7b8` not 0) tries each of its
    /// player's masters by id of the same class, type and group, its own too, with any filter;
    /// the first with a task gives it, and with `change` the unit takes that master (slot 0x90,
    /// [`Turns::change_master`], or [`Turns::transform_building`] for a building) though it be its
    /// own. None: no task. The terrain under the point, which only a task for a terrain reads, is
    /// not passed.
    ///
    /// [`find_task`]: crate::tasks::find_task
    fn find_work_task(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        filter: i16,
        change: bool,
    ) -> Result<Option<crate::masters::Task>, Unsupported> {
        let unit = &self.players[player].objects[idx];
        let own = self.master_of(player, unit.life.master_id)?;
        let owner = &self.players[player];
        let actor = Actor {
            id: unit.id,
            master: own,
            player,
            held: (unit.life.attribute_type, unit.life.amount),
            attribute_95: owner.attributes.get(95).copied().unwrap_or(0.0),
            civ: owner.civ,
        };
        let world = World {
            players: &self.players,
            drop_sites: &self.drop_sites,
        };
        let t = self.task_target(target, 1)?;
        let general = self.task_target(target, 2)?;
        let find = |list: &Master, filter: i16| {
            crate::tasks::find_task(list, &actor, Some(&t), Some(&general), None, filter, &world)
                .map(Option::<&crate::masters::Task>::copied)
        };
        if let Some(task) = find(own, filter)? {
            return Ok(Some(task));
        }
        if own.task_by_group == 0 {
            return Ok(None);
        }
        let mut found = None;
        // the master table exists: master_of found the unit's own in it
        for candidate in self.masters[player].iter() {
            if candidate.class != own.class
                || candidate.kind != own.kind
                || candidate.task_by_group != own.task_by_group
            {
                continue;
            }
            if let Some(task) = find(candidate, -1)? {
                found = Some((candidate.id, task));
                break;
            }
        }
        let Some((with, task)) = found else {
            return Ok(None);
        };
        if change {
            if self.master_of(player, with)?.kind == BUILDING {
                self.transform_building(player, idx, with)?;
            } else {
                self.change_master(player, idx, with)?;
            }
        }
        Ok(Some(task))
    }

    /// `HuntObject`'s unit part (`FUN_140c4e220`, [`crate::unit_ai::Effect::HuntObject`], after the
    /// unit's update): its stopAction (slot 0x3c0: the action list cleared,
    /// [`Turns::clear_actions`], and in state 2 the standing sprite), then its hunt command (object
    /// slot 0x360, `FUN_1412151f0`, forced): with no action left slot 0x820 finds the task
    /// ([`Turns::find_work_task`] with any filter and the master switch) and a new Hunt on it,
    /// whatever its action type or requirement (`FUN_140ac2dd0`), replaces the unit's actions
    /// ([`Turns::start_hunt`] without the unit AI's notify: `HuntObject` set the AI itself). A
    /// target the world does not hold or no task would fail the command, which the unit AI took
    /// as done: not modelled.
    pub(crate) fn hunt_command(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
    ) -> Result<(), Unsupported> {
        self.clear_actions(player, idx)?;
        if self.players[player].objects[idx].life.state == 2 {
            self.standing_sprite(player, idx)?;
        }
        let (_, t) = self.object(target).ok_or(Unsupported(
            "a hunt command on an object the world does not hold",
        ))?;
        let point = t.position;
        let task = self
            .find_work_task(player, idx, target, -1, true)?
            .ok_or(Unsupported("a hunt command without a task"))?;
        self.start_hunt(
            player,
            idx,
            target,
            HuntTask {
                id: task.id,
                range: task.work_range,
                resource: task.attribute_type,
                sprites: task.sprites,
                work_value: task.work_val1,
                productivity: task.attribute_type2,
                kind4: task.attribute_type4,
            },
            point,
            false,
            false,
        )
    }

    /// The unit's gather command (object slot 0x368, `FUN_141085030`) forced on `target`, as the
    /// unit AI's `GatherObject` runs it ([`crate::unit_ai::Effect::GatherObject`], after the
    /// unit's update): slot 0x820 finds the task ([`Turns::find_work_task`] with any filter and
    /// the master switch);
    /// the unit's top Gather on the task found takes the target (its slot 0x48 with the target's
    /// place, [`Turns::retarget_gather`]; what it answers the command passes over); else a new
    /// Gather on the task replaces the unit's actions (`FUN_140a88310`, `FUN_140a98620`, slot
    /// 0x808: [`Turns::start_gather`] without the unit AI's notify), whatever the task's action
    /// type (a farm's foundation: the builder's Build task). A target the world does not hold or
    /// no task would fail the command, which the unit AI took as done: not modelled.
    pub(crate) fn gather_command(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
    ) -> Result<(), Unsupported> {
        if self.object(target).is_none() {
            return Err(Unsupported(
                "a gather command on an object the world does not hold",
            ));
        }
        let task = self
            .find_work_task(player, idx, target, -1, true)?
            .ok_or(Unsupported("a gather command without a task"))?;
        let unit = &self.players[player].objects[idx];
        if unit.top_action_kind() == Some(crate::gather::GATHER)
            && unit.gather.is_some_and(|g| g.task.id == task.id)
        {
            let _ = self.retarget_gather(player, idx, target)?;
            return Ok(());
        }
        // FUN_140a88310 makes a Gather on whatever task it found (a farm's foundation gives the
        // builder's Build task)
        self.start_gather(player, idx, target, &task, None)
    }

    /// The work command's Build (its task of action type 101, the factory `FUN_140aca4b0` case
    /// 0x65): the action takes its foundation (slot 0x178); the unit AI's notify
    /// (`FUN_14106e4e0` kind 0x65: the build order and action, the foundation with its class, the
    /// work's point); the install (slot 0x808) clears the action list and holds the Build
    /// ([`crate::build::BuildAction::start`] with the footprints' gap, the task's range and work
    /// value, the master's work rate with the unit's powerup), as BUILD's builders take it.
    fn start_build(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        task: &crate::masters::Task,
        point: [f32; 3],
        clears: bool,
    ) -> Result<(), Unsupported> {
        let (tp, t) = self
            .object(target)
            .ok_or(Unsupported("a Build's foundation the slice does not hold"))?;
        let tm = self.master_of(tp, t.life.master_id)?;
        let (t_at, t_radius, class) = (t.position, tm.radius, i32::from(tm.class));
        let unit = &self.players[player].objects[idx];
        let builder = self.master_of(player, unit.life.master_id)?;
        let gap = crate::movement::footprint_gap(unit.position, builder.radius, t_at, t_radius);
        let build_task = crate::build::BuildTask {
            range: task.work_range,
            work_value: task.work_val1,
            work_rate: builder.work_rate + unit.work_rate_powerup,
        };
        self.take(target);
        let unit = &mut self.players[player].objects[idx];
        if let Some(ai) = unit.ai.as_mut() {
            ai.notify_work(
                crate::unit_ai::BUILD_ORDER,
                crate::unit_ai::BUILD_ACTION,
                Some((target, class)),
                point,
                clears,
                self.streams.get(Stream::Ai),
            )?;
        }
        self.clear_actions(player, idx)?;
        self.players[player].objects[idx].build =
            Some(crate::build::BuildAction::start(target, gap, build_task));
        Ok(())
    }

    /// The work command's target for the Build the unit runs (`LAB_1410706a8`: its task is the
    /// one found, slot 0x48 takes the target, `FUN_140ab1b30`), then the notify of the Build
    /// (slot 0x828: order 702, action 602, the target with its class). Its own foundation again
    /// is [`crate::build::BuildAction::retake`]; another foundation of the player's (slot 0x68,
    /// `FUN_140ab0300`: a builder counted among the old one's builders leaves them, through
    /// `FUN_140aaddb0`, an annex's main building's; `FUN_140a7a230` lets the old one go, slot
    /// 0x180, and takes the new, slot 0x178) starts over in `set_state(4)`
    /// ([`crate::build::BuildAction::retarget`]). Another player's object (`set_state(11)`) is
    /// not modelled.
    fn retake_build(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        point: [f32; 3],
        clears: bool,
    ) -> Result<(), Unsupported> {
        let class = self.build_take(player, idx, target)?;
        let unit = &mut self.players[player].objects[idx];
        if let Some(ai) = unit.ai.as_mut() {
            ai.notify_work(
                crate::unit_ai::BUILD_ORDER,
                crate::unit_ai::BUILD_ACTION,
                Some((target, class)),
                point,
                clears,
                self.streams.get(Stream::Ai),
            )?;
        }
        Ok(())
    }

    /// The Build's slot 0x48 (`FUN_140ab1b30`) on `target` ([`Turns::retake_build`]'s part
    /// before the notify): returns the target's master class.
    pub(crate) fn build_take(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
    ) -> Result<i32, Unsupported> {
        let (tp, t) = self
            .object(target)
            .ok_or(Unsupported("a Build's foundation the slice does not hold"))?;
        if tp != player {
            return Err(Unsupported(
                "a Build retargeted to another player's object (set_state 11)",
            ));
        }
        let tm = self.master_of(tp, t.life.master_id)?;
        let (t_at, t_radius, class) = (t.position, tm.radius, i32::from(tm.class));
        let unit = &self.players[player].objects[idx];
        let old = unit
            .build
            .ok_or(Unsupported("a Build's retarget without its Build"))?;
        let builder = self.master_of(player, unit.life.master_id)?;
        let gap = crate::movement::footprint_gap(unit.position, builder.radius, t_at, t_radius);
        if old.target != target {
            // the task finder took the new target below state 3, which slot 0x68 keeps
            let _ = self.build_set_target(player, idx, (old.target, old.added), target)?;
            let unit = &mut self.players[player].objects[idx];
            let (Some(build), mover) = (unit.build.as_mut(), unit.mover.as_mut()) else {
                return Err(Unsupported("a Build's retarget without its Build"));
            };
            build.retarget(target, gap, mover)?;
            return Ok(class);
        }
        let computer = self.players[player].computer;
        let Some(Place::Live(fp, fi)) = self.place(target) else {
            return Err(Unsupported("a Build's foundation that sleeps"));
        };
        let (mut build, mut mover) = {
            let unit = &mut self.players[player].objects[idx];
            (unit.build.take(), unit.mover.take())
        };
        // FUN_140aaddb0's building: an annex's main building
        let counted = match self.counted_building(target)? {
            Some(id) if id != target => self.object_mut(id).map(|(_, b)| b),
            _ => Some(&mut self.players[fp].objects[fi]),
        };
        let builders = counted
            .and_then(|b| b.construction.as_mut())
            .map(|k| &mut k.builders);
        let r =
            build
                .as_mut()
                .expect("found above")
                .retake(gap, mover.as_mut(), builders, computer);
        let unit = &mut self.players[player].objects[idx];
        (unit.build, unit.mover) = (build, mover);
        r?;
        Ok(class)
    }

    /// The Build's slot 0x68 (`FUN_140ab0300`) of the builder at `idx`, its target `old` and
    /// whether it is counted among the builders (`added`, `+0x68`), on `new` (−1 for none): a
    /// counted Build of another target leaves the builders ([`Turns::leave_builders`]) and is no
    /// longer counted; then the base ([`Turns::retarget_action`]) takes the target. Returns the
    /// Build's target after it.
    pub(crate) fn build_set_target(
        &mut self,
        player: usize,
        idx: usize,
        (old, added): (i32, bool),
        new: i32,
    ) -> Result<i32, Unsupported> {
        let leaves = old != new && added;
        if leaves {
            self.leave_builders(old)?;
        }
        let target = self.retarget_action(player, idx, old, new, false)?;
        if let Some(build) = self.players[player].objects[idx].build.as_mut() {
            build.target = target;
            build.added &= !leaves;
        }
        Ok(target)
    }

    /// `FUN_140aaddb0`: the building a Build on `id` counts its builder in, when the world holds
    /// `id` (live or sleeping, `FUN_1415417a0`): the object itself, or for a building (type 80)
    /// that is an annex (`+0xbb8`) its main building.
    pub(crate) fn counted_building(&self, id: i32) -> Result<Option<i32>, Unsupported> {
        let Some((p, f)) = self.object(id) else {
            return Ok(None);
        };
        let kind = self.master_of(p, f.life.master_id)?.kind;
        Ok(Some(if kind == BUILDING {
            f.annex_of.unwrap_or(id)
        } else {
            id
        }))
    }

    /// The builder count: the building `FUN_140aaddb0` gives for the foundation `id` a Build
    /// counted its builder among ([`Turns::counted_building`]) counts one builder less
    /// (`+0x9e8`, held in its construction record); none once the world no longer holds `id`.
    pub(crate) fn leave_builders(&mut self, id: i32) -> Result<(), Unsupported> {
        let Some(site) = self.counted_building(id)? else {
            return Ok(());
        };
        let c = self
            .object_mut(site)
            .and_then(|(_, b)| b.construction.as_mut())
            .ok_or(Unsupported("a Build's foundation without its construction"))?;
        c.builders = c.builders.wrapping_sub(1);
        Ok(())
    }

    /// The work command's Attack (its task of action type 7, the factory `FUN_140a97e00` case 7):
    /// the unit AI takes action 600 (`FUN_140c64b50`) and the target (an annex its main building,
    /// `FUN_141189b20`) at its place (`FUN_140c64c10`); the Attack (`FUN_140a7c210`, sub-action 0:
    /// the task's sprites, the task's range or slot 0x238's for 0) takes its target (slot 0x178);
    /// the notify (`FUN_14106e4e0` kind 9: the last retarget time `+0xbc` takes the world's time,
    /// then order 700, action 600, the target with its class and the work's point); the install
    /// clears the action list and starts the Attack (slot 5, `0x140a7f8c0`: `+0x900` cleared, the
    /// standing sprite, the range test of [`Attack::command`]; a unit that cannot move in reach
    /// goes ready, `set_state(6)`, [`Attack::enter_ready`]); then the Attack takes the target as
    /// its second target too (`FUN_140a7a190`, slot 0x178). The unit AI's `+0x1d5`, cleared, is
    /// not held; a task range other than 0 is not modelled.
    fn start_attack(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        task: &crate::masters::Task,
        point: [f32; 3],
        clears: bool,
    ) -> Result<(), Unsupported> {
        use crate::attack::Strike;
        use crate::move_to::Walk;
        if task.work_range != 0.0 {
            return Err(Unsupported("an attack task with a range of its own"));
        }
        let (tp, t) = self.object(target).ok_or(Unsupported(
            "an attack task's target the slice does not hold",
        ))?;
        let tm = self.master_of(tp, t.life.master_id)?;
        let aimed = if tm.kind == BUILDING {
            t.annex_of.unwrap_or(target)
        } else {
            target
        };
        let (class, fog_visibility) = (i32::from(tm.class), tm.fog_visibility);
        let (ap, a) = self
            .object(aimed)
            .ok_or(Unsupported("an attack task's main building is gone"))?;
        let aimed_at = a.position;
        let aimed_class = i32::from(self.master_of(ap, a.life.master_id)?.class);
        let world_time = self.world_time;
        let unit = &mut self.players[player].objects[idx];
        if let Some(ai) = unit.ai.as_mut() {
            ai.set_action(600);
            ai.set_target(aimed, aimed_class, aimed_at);
        }
        self.take(target);
        let unit = &mut self.players[player].objects[idx];
        if let Some(ai) = unit.ai.as_mut() {
            ai.last_retarget_time = world_time;
            ai.notify_work(
                700,
                600,
                Some((target, class)),
                point,
                clears,
                self.streams.get(Stream::Ai),
            )?;
        }
        self.clear_actions(player, idx)?;
        let master = self.master_of(player, self.players[player].objects[idx].life.master_id)?;
        // the task factory (`FUN_140a97e00` case 7) gives the Attack the task's moving sprite
        let arms = crate::attack::arms(master, &self.graphics, task.sprites, task.sprites[0]);
        let standing = arms
            .idle_sprite
            .ok_or(Unsupported("an attacker without its standing sprite"))?;
        let me_at = *self
            .spatial
            .get(self.players[player].objects[idx].id)
            .ok_or(Unsupported("an attacker without a place"))?;
        let t_at = *self
            .spatial
            .get_known(target)?
            .ok_or(Unsupported("an attack's target without a place"))?;
        let (tx, ty) = (t_at.position[0] as i32, t_at.position[1] as i32);
        let visible = self.spatial.sees_tile(player, tx, ty)
            || (fog_visibility != 0 && self.spatial.explored(player, tx, ty)?);
        let reload = {
            let u = &self.players[player].objects[idx];
            u.reload_stinger + (master.reload - u.reload_powerup)
        };
        let hit_chance = master.hit_chance;
        let computer = self.players[player].computer;
        let volleys = self
            .masters
            .get(player)
            .is_some_and(|m| crate::attack::volleys(master, m));
        let mut victim = self.victim(target, player).ok_or(Unsupported(
            "an attack's target the slice does not hold live",
        ))?;
        let own = {
            let u = &self.players[player].objects[idx];
            let terrain = self.spatial.terrain.as_ref();
            crate::turn::victim_of(
                u,
                master,
                &self.players[player].attributes,
                player,
                terrain,
                self.mode_680,
            )
        };
        let speed = self.players[player].objects[idx]
            .mover
            .as_ref()
            .map_or(0.0, |m| m.stats.speed);
        let e = crate::attack::engagement(
            self.players[player].objects[idx].ai.as_ref(),
            speed,
            master,
            &me_at,
            &t_at,
            world_time,
        )?;
        let unit = &mut self.players[player].objects[idx];
        if unit.sprite.is_none() {
            return Err(Unsupported(
                "an attack task of a unit whose sprite the slice does not hold",
            ));
        }
        unit.attack_count = 0;
        crate::attack::set_sprite(&mut unit.sprite, standing, self.streams.get(Stream::Sprite));
        let mover = unit
            .mover
            .as_mut()
            .ok_or(Unsupported("an attacker without movement state"))?;
        let crate::attack::Commanded {
            mut attack,
            arrived,
            posted,
            ready,
            failed,
        } = crate::attack::Attack::command(target, &e, arms, visible, tp, mover)?;
        if arrived {
            return Err(Unsupported("an attack task whose MoveTo starts arrived"));
        }
        let (id, state) = (unit.id, unit.life.state);
        for (kind, p) in posted {
            if let Some(ai) = unit.ai.as_mut() {
                ai.post(crate::unit_ai::Notify::own(id, kind, p), state)?;
            }
        }
        if failed {
            attack.fail_at_start(
                &mut unit.sprite,
                &mut unit.anim_speed,
                self.streams.get(Stream::Sprite),
            )?;
        }
        if ready {
            let me = crate::movement::Me {
                id: unit.id,
                position: unit.position,
                group: unit.group.id,
                action_state: crate::attack::READY,
            };
            let can_move = unit
                .ai
                .as_ref()
                .is_some_and(|ai| ai.current_order != -1 || ai.stance <= 1);
            let stance = unit.ai.as_ref().map_or(0, |ai| ai.stance);
            let queue_empty = unit.ai.as_ref().is_some_and(|ai| ai.orders.is_empty());
            let walk = Walk {
                mover: unit.mover.as_mut(),
                me,
                dt: 0.0,
                world: &mut self.spatial,
                plan: None,
            };
            let mut blasted = Vec::new();
            let mut strike = Strike {
                sprite: &mut unit.sprite,
                anim_speed: &mut unit.anim_speed,
                standing: Some(standing),
                volleys,
                attack_timer: &mut unit.attack_timer,
                reload_worldtime: &mut unit.reload_worldtime,
                reload,
                hit_chance,
                victim: &mut victim,
                blasted: &mut blasted,
                sprite_rng: self.streams.get(Stream::Sprite),
                own,
                executed: None,
                fire: None,
                can_move,
                stance,
                posted: Vec::new(),
                queue_empty,
                world_time,
                ai_order: unit.ai.as_ref().map(|ai| ai.current_order),
                ai_type: unit.ai.as_ref().map(|ai| ai.ai_type),
                computer,
                armed: true,
                volley_under_way: unit.next_volley != 0,
                let_go: None,
                forgets: false,
                sends_home: false,
            };
            attack.enter_ready(walk, &mut strike)?;
            if strike.executed.is_some() || strike.fire.is_some() || !strike.posted.is_empty() {
                return Err(Unsupported("an attack task's swing at its start"));
            }
        }
        attack.target2 = target;
        unit.attack = Some(attack);
        self.take(target);
        Ok(())
    }

    /// The hunt's slot 9 (`0x140ac97c0`) on the work command's new target: a new target is
    /// released for the old one (slot 0x68, `FUN_140ac7980`, through `FUN_140a7a230`) and the hunt
    /// starts over.
    fn retarget_hunt(&mut self, player: usize, idx: usize, target: i32) -> Result<(), Unsupported> {
        let quarry = self.quarry(player, target)?;
        let engagement = self.engagement(player, idx, target)?;
        let task = self.players[player].objects[idx]
            .hunt
            .map(|h| h.task)
            .ok_or(Unsupported("a hunt's retarget without its hunt"))?;
        let arms = self.arms(player, idx, &task)?;
        // FUN_14109d4f0: the target's master is one of the unit's master's drop sites
        let target_master = self.object(target).map(|(_, t)| t.life.master_id);
        let drops_here = self
            .master_of(player, self.players[player].objects[idx].life.master_id)?
            .drop_sites
            .iter()
            .any(|&d| Some(d) == target_master);
        let unit = &mut self.players[player].objects[idx];
        let old = unit.hunt.map(|h| h.target).filter(|&t| t != -1);
        let held = unit.life.amount;
        let (Some(hunt), Some(mover)) = (unit.hunt.as_mut(), unit.mover.as_mut()) else {
            return Err(Unsupported("a hunter without its hunt or movement state"));
        };
        let unit_player = player;
        if let Some(arrived) = hunt.retarget(
            &quarry,
            unit_player,
            held,
            drops_here,
            &engagement,
            arms,
            mover,
        )? {
            if arrived {
                crate::attack::show_arrived(
                    &arms,
                    &mut unit.sprite,
                    &mut unit.anim_speed,
                    self.streams.get(Stream::Sprite),
                )?;
            }
            // the hunt's slot 0x68 (FUN_140ac7980): FUN_140a7a230 on the new target, which the
            // work command's task finder took below state 3
            let _ = self.retarget_action(player, idx, old.unwrap_or(-1), target, false)?;
        }
        Ok(())
    }

    /// An action's slot 0x68 base (`FUN_140a7a230`) for the unit at `idx`: its target `old` (−1
    /// for none) becomes `new`. The same target changes nothing. A different old target the
    /// world still holds is let go (slot 0x180, [`Turns::release`]) unless the action is a
    /// sub-action (`quiet`, `+0x60`); a new target in state 7 or more is not taken, and the
    /// action is left without one; else it is taken (slot 0x178 with the action's type,
    /// [`Turns::take`]) unless `quiet`. Returns the action's target id (`+0x30`) after it.
    pub(crate) fn retarget_action(
        &mut self,
        player: usize,
        idx: usize,
        old: i32,
        new: i32,
        quiet: bool,
    ) -> Result<i32, Unsupported> {
        if old == new {
            return Ok(old);
        }
        if old != -1 && !quiet && self.object(old).is_some() {
            let by = self.releaser(player, idx);
            self.release(old, by)?;
        }
        let Some((_, t)) = self.object(new) else {
            return Ok(-1);
        };
        if 7 <= t.life.state {
            return Ok(-1);
        }
        if !quiet {
            self.take(new);
        }
        Ok(new)
    }

    /// The Gather's slot 0x48 (`0x140a903a0`) on the work command's new target
    /// ([`crate::gather::GatherAction::retarget`]): false when it does not take it, for a building
    /// (type 80 without object flag 0x20) other than a farm, so the work command makes a new
    /// action. The targets that changed are let go (slot 0x180) and taken (slot 0x178). Not
    /// modelled: a building packing or unpacking (its top action 0x7d), a foundation or master
    /// 1893 or 1897 of the unit's player (slots 0x808 and 0x7e8: its build).
    fn retarget_gather(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
    ) -> Result<bool, Unsupported> {
        let (tp, t) = self
            .object(target)
            .ok_or(Unsupported("a gather's target the slice does not hold"))?;
        let tm = self.master_of(tp, t.life.master_id)?;
        if tm.kind == BUILDING {
            if t.top_action_kind() == Some(PACK_ACTION) || t.unmodelled_action.is_some() {
                return Err(Unsupported(
                    "a gather retargeted to a building with an action (packing)",
                ));
            }
            if tm.object_flags & GATHERED_BUILDING == 0 {
                let (mp, main) = match t.annex_of {
                    Some(m) => self
                        .object(m)
                        .ok_or(Unsupported("an annex whose main building is gone"))?,
                    None => (tp, t),
                };
                let main_master = self.master_of(mp, main.life.master_id)?.id;
                if (main.life.state < 2 || matches!(main_master, MASTER_1893 | MASTER_1897))
                    && mp == player
                {
                    return Err(Unsupported(
                        "a gather retargeted to its player's foundation (its build)",
                    ));
                }
                if tm.class != crate::gather::FARM_CLASS {
                    return Ok(false);
                }
            }
        }
        let (t_at, t_radius) = (t.position, tm.radius);
        let unit = &self.players[player].objects[idx];
        let me = self.master_of(player, unit.life.master_id)?;
        let resource = crate::gather::Resource {
            id: target,
            position: t_at,
            class: tm.class,
            master: tm.id,
            player: tp,
            workers: t.workers,
            drop_site: self.drops_at_main(player, idx, target)?,
            gap: crate::movement::footprint_gap(unit.position, me.radius, t_at, t_radius),
            radius: t_radius,
        };
        let computer = self.players[player].computer;
        let farm_stops = self.settings.walkable_farms;
        let by = self.releaser(player, idx);
        // a stop farm's time per stop: the gatherer's work rate (its master's plus its powerup),
        // capacity and the task's productivity attribute (none: 1.0)
        let productivity = unit.gather.map_or(-1, |g| g.task.productivity);
        let rate = crate::gather::StopRate {
            work_rate: unit.work_rate_powerup + me.work_rate,
            capacity: f32::from(me.capacity),
            productivity: crate::gather::productivity_of(
                productivity,
                &self.players[player].attributes,
            ),
        };
        let unit = &mut self.players[player].objects[idx];
        let held = unit.life.amount;
        let (Some(gather), Some(mover)) = (unit.gather.as_mut(), unit.mover.as_mut()) else {
            return Err(Unsupported(
                "a gatherer without its gather or movement state",
            ));
        };
        let (old_target, old_second) = (gather.target, gather.carry_to);
        let rng = self.streams.get(Stream::Action);
        let took = gather.retarget(
            &resource,
            player,
            computer,
            held,
            (farm_stops, rate),
            mover,
            rng,
        )?;
        let (new_target, new_second) = (gather.target, gather.carry_to);
        // a farm another farmer works: taken (slot 0x68) and let go again (set_state 11)
        let waits = gather.state == crate::gather::WALK_BACK && new_target == -1;
        for (old, new) in [(old_target, new_target), (old_second, new_second)] {
            if old != new {
                if old != -1 {
                    self.release(old, by)?;
                }
                if new != -1 {
                    self.take(new);
                }
            }
        }
        if waits && old_target != target {
            self.take(target);
            self.release(target, by)?;
        }
        Ok(took)
    }

    /// The Gather's slot 0x40 (`0x140a8b740`, [`crate::gather::GatherAction::take_move`]) on a
    /// move of the gatherer at `idx` to `point`, toward the object `target` if any: always taken;
    /// the targets that changed are let go and taken.
    pub(crate) fn gather_take_move(
        &mut self,
        player: usize,
        idx: usize,
        target: Option<i32>,
        point: [f32; 3],
    ) -> Result<bool, Unsupported> {
        let moved_to = match target {
            Some(t) => {
                let (tp, o) = self
                    .object(t)
                    .ok_or(Unsupported("a move to an object the slice does not hold"))?;
                let tm = self.master_of(tp, o.life.master_id)?;
                let (at, radius) = (o.position, tm.radius);
                let unit = &self.players[player].objects[idx];
                let me = self.master_of(player, unit.life.master_id)?;
                Some(crate::gather::MoveTarget {
                    id: t,
                    player: tp,
                    drop_site: self.drops_at_main(player, idx, t)?,
                    gap: crate::movement::footprint_gap(unit.position, me.radius, at, radius),
                })
            }
            None => None,
        };
        let by = self.releaser(player, idx);
        let unit = &mut self.players[player].objects[idx];
        let held = unit.life.amount;
        let (Some(gather), Some(mover)) = (unit.gather.as_mut(), unit.mover.as_mut()) else {
            return Err(Unsupported(
                "a gatherer without its gather or movement state",
            ));
        };
        let (old_target, old_second) = (gather.target, gather.carry_to);
        gather.take_move(moved_to, point, player, held, mover)?;
        let (new_target, new_second) = (gather.target, gather.carry_to);
        for (old, new) in [(old_target, new_target), (old_second, new_second)] {
            if old != new {
                if old != -1 {
                    self.release(old, by)?;
                }
                if new != -1 {
                    self.take(new);
                }
            }
        }
        Ok(true)
    }

    /// `FUN_14109d4f0` of the unit at `idx` on `target`, or on a building annex's main building
    /// (`+0xbb8`): the unit's master has that master among its drop sites.
    fn drops_at_main(&self, player: usize, idx: usize, target: i32) -> Result<bool, Unsupported> {
        let (tp, t) = self
            .object(target)
            .ok_or(Unsupported("a hunt's target the slice does not hold"))?;
        let tm = self.master_of(tp, t.life.master_id)?;
        let main = if tm.kind == BUILDING {
            t.annex_of.unwrap_or(target)
        } else {
            target
        };
        let main_master = match self.object(main) {
            Some((mp, m)) => self.master_of(mp, m.life.master_id)?.id,
            None => tm.id,
        };
        let unit = &self.players[player].objects[idx];
        Ok(self
            .master_of(player, unit.life.master_id)?
            .drop_sites
            .contains(&main_master))
    }

    /// The Hunt's slot 0x40 from a move ([`HuntAction::take_move`]) with `target` (None for a
    /// point) and `point`: true when it takes it. The target it had goes (slot 0x68 with none:
    /// [`Turns::release`]), and a drop site it carries its load to becomes its second target
    /// (`FUN_140a7a190`: [`Turns::take`]).
    ///
    /// [`HuntAction::take_move`]: crate::hunt::HuntAction::take_move
    pub(crate) fn hunt_take_move(
        &mut self,
        player: usize,
        idx: usize,
        target: Option<i32>,
        point: [f32; 3],
    ) -> Result<bool, Unsupported> {
        let quarry = match target {
            Some(t) => {
                let mut q = self.quarry(player, t)?;
                q.drop_site = self.drops_at_main(player, idx, t)?;
                Some(q)
            }
            None => None,
        };
        let gap = match target {
            Some(t) => self.engagement(player, idx, t)?.gap,
            None => 0.0,
        };
        let by = self.releaser(player, idx);
        let unit = &mut self.players[player].objects[idx];
        let (held, my_pasture) = (unit.life.amount, unit.my_pasture);
        let (Some(hunt), Some(mover)) = (unit.hunt.as_mut(), unit.mover.as_mut()) else {
            return Err(Unsupported("a hunter without its hunt or movement state"));
        };
        let (old_target, old_second) = (hunt.target, hunt.carry_to);
        if !hunt.take_move(
            quarry.as_ref(),
            point,
            player,
            held,
            quarry.as_ref().is_some_and(|q| q.drop_site),
            my_pasture,
            gap,
            mover,
        )? {
            return Ok(false);
        }
        let new_second = hunt.carry_to;
        if old_target != -1 && hunt.target == -1 {
            self.release(old_target, by)?;
        }
        if new_second != old_second && new_second != -1 {
            if old_second != -1 {
                self.release(old_second, by)?;
            }
            self.take(new_second);
        }
        Ok(true)
    }

    /// What a hunt reads of its target.
    fn quarry(&self, player: usize, target: i32) -> Result<Quarry, Unsupported> {
        let (tp, t) = self
            .object(target)
            .ok_or(Unsupported("a hunt's target the slice does not hold"))?;
        let t_master = self.master_of(tp, t.life.master_id)?;
        let world = World {
            players: &self.players,
            drop_sites: &self.drop_sites,
        };
        Ok(Quarry {
            id: target,
            position: t.position,
            class: t_master.class,
            kind: t_master.kind,
            state: t.life.state,
            player: tp,
            amount: t.life.amount,
            allied: world.allied(player, tp, true),
            top_action: t.top_action_kind(),
            drop_site: false,
        })
    }

    /// The work command's Gather (its task of action type 5, the base factory `FUN_140a97e00`
    /// case 5 with a target): the constructor ([`crate::gather::GatherAction::new`]) takes its
    /// target (slot 0x178); the unit AI's notify (slot 0x828: the gather order and action, the
    /// target with its class, the work's point; `notify`'s point and clearing, None for the gather
    /// command's, which has none); the install (slot 0x808) clears the action list and starts the
    /// Gather ([`crate::gather::GatherAction::start`]). A load to its own drop site takes the
    /// target as the second target, then lets it go as the first.
    fn start_gather(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        task: &crate::masters::Task,
        notify: Option<([f32; 3], bool)>,
    ) -> Result<(), Unsupported> {
        let (tp, t) = self
            .object(target)
            .ok_or(Unsupported("a gather's target the slice does not hold"))?;
        let tm = self.master_of(tp, t.life.master_id)?;
        let (t_at, t_radius) = (t.position, tm.radius);
        let unit = &self.players[player].objects[idx];
        let me = self.master_of(player, unit.life.master_id)?;
        let resource = crate::gather::Resource {
            id: target,
            position: t_at,
            class: tm.class,
            master: tm.id,
            player: tp,
            workers: t.workers,
            drop_site: self.drops_at_main(player, idx, target)?,
            gap: crate::movement::footprint_gap(unit.position, me.radius, t_at, t_radius),
            radius: t_radius,
        };
        let task = HuntTask {
            id: task.id,
            range: task.work_range,
            resource: task.attribute_type,
            sprites: task.sprites,
            work_value: task.work_val1,
            productivity: task.attribute_type2,
            kind4: task.attribute_type4,
        };
        // the constructor's time per stop: the gatherer's work rate (its master's plus its
        // powerup, slot 0x6e8), capacity and productivity attribute (none: 1.0)
        let rate = crate::gather::StopRate {
            work_rate: unit.work_rate_powerup + me.work_rate,
            capacity: f32::from(me.capacity),
            productivity: crate::gather::productivity_of(
                task.productivity,
                &self.players[player].attributes,
            ),
        };
        let mut gather =
            crate::gather::GatherAction::new(task, &resource, self.settings.walkable_farms, rate)?;
        self.take(target);
        let unit = &mut self.players[player].objects[idx];
        if let (Some(ai), Some((point, clears))) = (unit.ai.as_mut(), notify) {
            ai.notify_work(
                crate::ai_kinds::ORDER_GATHER,
                crate::unit_ai::GATHER_ACTION,
                Some((target, i32::from(resource.class))),
                point,
                clears,
                self.streams.get(Stream::Ai),
            )?;
        }
        self.clear_actions(player, idx)?;
        let computer = self.players[player].computer;
        let carry_rule = self.settings.carry_rule();
        let unit = &mut self.players[player].objects[idx];
        let mover = unit
            .mover
            .as_mut()
            .ok_or(Unsupported("a gatherer without movement state"))?;
        let mut held = (unit.life.attribute_type, unit.life.amount);
        let rng = self.streams.get(Stream::Action);
        gather.start(
            &resource, player, computer, carry_rule, &mut held, mover, rng,
        )?;
        (unit.life.attribute_type, unit.life.amount) = held;
        let drops = gather.target == -1 && gather.carry_to == target;
        // a farm another farmer works: the start let it go (set_state 11, then slot 0x68)
        let waits = gather.target == -1 && gather.carry_to != target;
        unit.gather = Some(gather);
        if drops {
            self.take(target);
        }
        if drops || waits {
            let by = self.releaser(player, idx);
            self.release(target, by)?;
        }
        Ok(())
    }

    /// [`crate::attack::arms`] of the unit at `idx` for its hunt's Attack (Hunt's `set_state` 12:
    /// the `MoveTo` shows the hunt's walking sprite, [`HuntTask::walk_sprite_id`]).
    fn arms(&self, player: usize, idx: usize, task: &HuntTask) -> Result<Arms, Unsupported> {
        let unit = &self.players[player].objects[idx];
        let master = self.master_of(player, unit.life.master_id)?;
        let moving = task.walk_sprite_id(unit.life.attribute_type, unit.life.amount);
        Ok(crate::attack::arms(
            master,
            &self.graphics,
            task.sprites,
            moving,
        ))
    }

    /// [`crate::attack::engagement`] of the unit at `idx` with `target`.
    fn engagement(
        &self,
        player: usize,
        idx: usize,
        target: i32,
    ) -> Result<Engagement, Unsupported> {
        let unit = &self.players[player].objects[idx];
        let master = self.master_of(player, unit.life.master_id)?;
        let me = self
            .spatial
            .get(unit.id)
            .ok_or(Unsupported("an attacker without a place"))?;
        let them = self
            .spatial
            .get_known(target)?
            .ok_or(Unsupported("an attack's target without a place"))?;
        let speed = unit.mover.as_ref().map_or(0.0, |m| m.stats.speed);
        crate::attack::engagement(unit.ai.as_ref(), speed, master, me, them, self.world_time)
    }

    /// The Hunt from the factory (`FUN_140aca4b0` case 0x6e): the action takes its target (slot
    /// 0x178); the work command's unit AI notify (`FUN_14106e4e0`, `notify`) sets the hunt order;
    /// the install (`FUN_14106e990`) clears the action list and starts the hunt.
    #[allow(clippy::too_many_arguments)]
    fn start_hunt(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        task: HuntTask,
        point: [f32; 3],
        clears: bool,
        notify: bool,
    ) -> Result<(), Unsupported> {
        let mut quarry = self.quarry(player, target)?;
        quarry.drop_site = self.drops_at_main(player, idx, target)?;
        self.take(target);
        let unit = &mut self.players[player].objects[idx];
        if let Some(ai) = unit.ai.as_mut().filter(|_| notify) {
            ai.notify_work(
                HUNT_ORDER,
                HUNT_ACTION,
                Some((target, i32::from(quarry.class))),
                point,
                clears,
                self.streams.get(Stream::Ai),
            )?;
        }
        self.clear_actions(player, idx)?;
        let engagement = self.engagement(player, idx, target)?;
        let arms = self.arms(player, idx, &task)?;
        let computer = self.players[player].computer;
        let carry_rule = self.settings.carry_rule();
        let unit = &mut self.players[player].objects[idx];
        let mover = unit
            .mover
            .as_mut()
            .ok_or(Unsupported("a hunter without movement state"))?;
        let mut held = (unit.life.attribute_type, unit.life.amount);
        let (hunt, arrived) = HuntAction::start(
            task,
            &quarry,
            player,
            computer,
            carry_rule,
            &mut held,
            &engagement,
            arms,
            mover,
        )?;
        (unit.life.attribute_type, unit.life.amount) = held;
        // an Attack whose MoveTo started arrived: the unit shows the idle sprite (slot 0x68) and
        // its `+0x7b8` clears (`Attack::arrived_sprite`)
        if arrived {
            let g = arms
                .idle_sprite
                .ok_or(Unsupported("an attacker without its standing sprite"))?;
            crate::attack::set_sprite(&mut unit.sprite, g, self.streams.get(Stream::Sprite));
            if let Some(a) = unit.anim_speed.as_mut() {
                *a = 0.0;
            }
        }
        let drops = hunt.target == -1 && hunt.carry_to == target;
        unit.hunt = Some(hunt);
        // a load to its own drop site: the target taken as the second (FUN_140a7a190), then let
        // go as the first (slot 0x68 with none)
        if drops {
            self.take(target);
            let by = self.releaser(player, idx);
            self.release(target, by)?;
        }
        Ok(())
    }

    /// Slot 0x178 on the object an action takes as its target (`FUN_14119d3e0`): its worker
    /// count (`+0x644`) rises, to at most 550; a sleeping object wakes: it leaves its player's
    /// sleeping list (slot 0x2d8, the last object taking its place) for the end of the live list
    /// (slot 0x2d0). A live object flagged to sleep only drops the flags.
    pub(crate) fn take(&mut self, id: i32) {
        match self.place(id) {
            Some(Place::Live(p, i)) => {
                let u = &mut self.players[p].objects[i];
                if u.workers < 550 {
                    u.workers += 1;
                }
                u.life.goto_sleep = false;
            }
            Some(Place::Sleeping(p, i)) => {
                let owner = &mut self.players[p];
                let u = &mut owner.sleeping[i];
                if u.workers < 550 {
                    u.workers += 1;
                }
                let woken = owner.sleeping.swap_remove(i);
                let (id, master) = (woken.id, woken.life.master_id);
                owner.objects.push(woken);
                self.track_relist(p, id, master);
            }
            None => {}
        }
    }

    /// Slot 0x180 on the object an action lets go: its worker count falls. A static object (type
    /// below 30, `FUN_141197d00`) that no one works on any more and is alive (state 2 with hit
    /// points at least 0) or in state 6 is flagged to sleep (`+0x641`, `+0x643`). A building's
    /// repairer list (`FUN_1411d6020`) and a unit's notify (`FUN_141086e60`, 699) are not held.
    pub(crate) fn release(&mut self, id: i32, by: Releaser) -> Result<(), Unsupported> {
        let Some(Place::Live(p, i)) = self.place(id) else {
            return Ok(());
        };
        let kind = {
            let u = &self.players[p].objects[i];
            self.master_of(p, u.life.master_id)?.kind
        };
        let world = World {
            players: &self.players,
            drop_sites: &self.drop_sites,
        };
        let allied = world.allied(by.player, p, true);
        crate::turn::released(&mut self.players[p].objects[i], kind, by, allied)
    }

    /// Object `idx` of player `player` as the releaser of its actions' targets.
    pub(crate) fn releaser(&self, player: usize, idx: usize) -> Releaser {
        let u = &self.players[player].objects[idx];
        Releaser {
            id: u.id,
            class: self
                .master_of(player, u.life.master_id)
                .map_or(-1, |m| m.class),
            player,
        }
    }

    /// The action list's clearing (`FUN_140a98620`) of object `idx` of player `player`: each
    /// action's teardown ([`Unit::delete_actions`]), then what they leave to the world: a
    /// counted builder leaves its building (`FUN_140aadbf0` before its base teardown,
    /// [`Turns::leave_builders`]), then the targets go (slot 0x180, [`Turns::release`]). A
    /// building's production is a list of its own, which stays.
    pub(crate) fn clear_actions(&mut self, player: usize, idx: usize) -> Result<(), Unsupported> {
        let by = self.releaser(player, idx);
        let (mut releases, mut builder_leaves) = (Vec::new(), Vec::new());
        self.players[player].objects[idx].delete_actions(&mut releases, &mut builder_leaves, by)?;
        for id in builder_leaves {
            self.leave_builders(id)?;
        }
        for (id, by) in releases {
            self.release(id, by)?;
        }
        Ok(())
    }

    /// `TRANSFORM_OBJECT` (`RGE_Command::do_command` case 0x29): the object `id`, the player's
    /// (another player's is refused, the cheat flag `+0xb12` aside), of the trophy master
    /// (1649) takes one of the trophies 1650 to 1653 the player has (`FUN_141659d20`): slot 0x90
    /// with 0, the building's ([`Turns::transform_building`]).
    pub(crate) fn transform_object(
        &mut self,
        player: usize,
        id: i32,
        master: i16,
    ) -> Result<(), Unsupported> {
        let Some(Place::Live(p, idx)) = self.place(id) else {
            return Ok(());
        };
        if p != player || self.master_of(player, master).is_err() {
            return Ok(());
        }
        let unit = &self.players[p].objects[idx];
        if unit.life.master_id != TROPHY || !TROPHIES.contains(&master) {
            return Ok(());
        }
        self.transform_building(p, idx, master)
    }

    /// A building's slot 0x90 (`FUN_1411e6d90`, with 0 from each caller): [`Turns::change_master`],
    /// then in state 2 its build points (`+0xc50`) take the new master's build time (its first
    /// train location's time, 0 without one). The slice holds them in the building's
    /// construction record.
    fn transform_building(
        &mut self,
        player: usize,
        idx: usize,
        new: i16,
    ) -> Result<(), Unsupported> {
        self.change_master(player, idx, new)?;
        let build_time = self.master_of(player, new)?.build_time;
        let unit = &mut self.players[player].objects[idx];
        if unit.life.state == 2
            && let Some(c) = unit.construction.as_mut()
        {
            c.progress = f32::from(build_time);
        }
        Ok(())
    }

    /// The player's gate toggle (`FUN_14162f8c0`, from its slots 0x340 and 0x338): a gate of its
    /// player's masters, without masters of its own ([`crate::turn::Gate::other_masters`]),
    /// changes to its master's `transform_unit` (slot 0x90, [`Turns::transform_building`]). A
    /// gate that shut (the new master's height above 0) kills the units in it (slot 0xd8 on
    /// those its count finds, [`Turns::shut_in_gate`]) and takes the world time plus 1 as its
    /// last move time (`+0x870`).
    pub(crate) fn toggle_gate(&mut self, player: usize, idx: usize) -> Result<(), Unsupported> {
        let unit = &self.players[player].objects[idx];
        if unit.gate.is_some_and(|g| g.other_masters) {
            return Err(Unsupported(
                "a gate's toggle to a master of its own or its original owner's",
            ));
        }
        let new = self
            .master_of(player, unit.life.master_id)?
            .building
            .as_ref()
            .map_or(-1, |b| b.transform_unit);
        if new == -1 {
            return Ok(());
        }
        self.transform_building(player, idx, new)?;
        let height = self.master_of(player, new)?.height;
        let now = self.world_time;
        if 0.0 < height {
            let id = self.players[player].objects[idx].id;
            let annexes = crate::turn::annexes_of(id, &self.players[player].objects);
            let (units, _) = crate::turn::gate_units(&self.spatial, id, &annexes)?;
            for u in units {
                self.shut_in_gate(u)?;
            }
            self.players[player].objects[idx]
                .mover
                .as_mut()
                .ok_or(Unsupported("a shut gate without its moving part"))?
                .last_move = now.wrapping_add(1);
        }
        Ok(())
    }

    /// Slot 0xd8 of object `id` in a gate that shut (`FUN_14162f8c0`): from type 70 on
    /// (`FUN_141211340`) class 42 is spared, and livestock (class 58) while it holds some
    /// (`COMISS`, `JA`: a NaN load holds none); else, and below type 70, `FUN_14118f580`: an
    /// object asleep wakes to the end of its player's live list (slots 0x2d8, 0x2d0), one flagged
    /// to sleep drops the flag, and its hit points go to 0. Its place in its player's
    /// special-object list (`+0x30c` bit 0, `FUN_141677380`) is not held.
    fn shut_in_gate(&mut self, id: i32) -> Result<(), Unsupported> {
        /// The first master type whose slot 0xd8 is `FUN_141211340` (`TRIBE_Combat_Object`).
        const TRIBE_COMBAT: u8 = 70;
        /// The class `FUN_141211340` always spares.
        const SPARED: i16 = 42;
        let Some((p, u)) = self.object(id) else {
            return Ok(());
        };
        let master = self.master_of(p, u.life.master_id)?;
        let spared = TRIBE_COMBAT <= master.kind
            && (master.class == SPARED || (master.class == LIVESTOCK && 0.0 < u.life.amount));
        if spared {
            return Ok(());
        }
        let unit = match self.place(id).expect("found above") {
            Place::Live(p, i) => &mut self.players[p].objects[i],
            Place::Sleeping(p, i) => {
                let woken = self.players[p].sleeping.swap_remove(i);
                let (id, master) = (woken.id, woken.life.master_id);
                self.players[p].objects.push(woken);
                // the wake's remove and add (slots 0x2d8, 0x2d0) file it last in the tracker
                self.track_relist(p, id, master);
                self.players[p].objects.last_mut().expect("pushed")
            }
        };
        unit.life.goto_sleep = false;
        unit.life.hp = 0.0;
        Ok(())
    }

    /// Slot 0x90 (`FUN_141225b70` with 0): the unit changes its master. Its action list is
    /// cleared; in a state below 3 (`FUN_1411a6a70`) the old master's storage leaves its player's
    /// attributes and the new one's comes in (`FUN_1411a56c0`, flag 2), its hit points keep their
    /// share of the new master's (whole, at least 1), its sight goes and the new master's comes
    /// (slots 0x1d8, 0x1d0), its place and obstruction record take the new master's
    /// (`FUN_141055b40`, `FUN_14118c540`) and it moves with the new master's values; its powerups
    /// go ([`clear_powerups`]) and in state 2 its volleys (`+0xa00`) are the new master's volley
    /// fire; it takes the new master's standing sprite (slot 0x68). Its
    /// per-master counts, facet, class registry (`DAT_143f69170`) and particles are not held.
    pub(crate) fn change_master(
        &mut self,
        player: usize,
        idx: usize,
        new: i16,
    ) -> Result<(), Unsupported> {
        self.clear_actions(player, idx)?;
        let unit = &self.players[player].objects[idx];
        let old = self.master_of(player, unit.life.master_id)?.clone();
        let next = self.master_of(player, new)?.clone();
        if unit.life.state < 3 {
            if unit.life.inside.is_some() {
                return Err(Unsupported("a master change of an object inside another"));
            }
            let owner = &mut self.players[player];
            for &(kind, amount, flag) in &old.storage {
                if kind >= 0 && matches!(flag, 2 | 16 | 32) {
                    owner.add_attribute(kind, -amount)?;
                }
            }
            let unit = &mut owner.objects[idx];
            let new_hp = f32::from(next.hit_points);
            let share = unit.life.hp / f32::from(old.hit_points) * new_hp;
            let hp = if share > new_hp {
                new_hp
            } else if 1.0 <= share {
                share
            } else {
                1.0
            };
            let old_hp = unit.life.hp;
            unit.life.hp = f32::from(hp as i32 as i16);
            // FUN_1411859a0: the damage share picks the damage sprites; one that stays keeps them
            // (the object's `+0xd`, not held, taken to be the old master's share)
            if (old.damage_sprites || next.damage_sprites)
                && (damage_share(old_hp, old.hit_points, old.class)?
                    != damage_share(unit.life.hp, next.hit_points, next.class)?)
            {
                return Err(Unsupported(
                    "a master change that changes the damage sprites (FUN_1411859a0)",
                ));
            }
            unit.life.master_id = new;
            let moving = unit
                .mover
                .as_ref()
                .is_some_and(crate::movement::Mover::is_moving);
            if let Some(m) = unit.mover.as_mut() {
                m.stats = next
                    .moving
                    .ok_or(Unsupported("a moving unit's new master does not move"))?;
            }
            let id = unit.id;
            if let Some(mut placed) = self.spatial.get(id).copied() {
                placed.radius = next.radius;
                placed.class = next.class;
                placed.master = new;
                placed.obstruction_type = next.obstruction_type;
                placed.buildable_on = next.buildable_on;
                placed.tall = next.tall;
                placed.speed = next.moving.as_ref().map_or(0.0, |m| m.speed);
                if let Some(o) = placed.obstruction.as_mut() {
                    o.kind = next.obstruction_type;
                    o.class = next.obstruction_class;
                }
                self.pathing.removed(id);
                self.spatial
                    .sight_master(id, next.radius, next.class, new, next.line_of_sight)?;
                self.spatial.change_master(id, placed);
                self.pathing.moved(id, &placed, moving);
            }
            // slot 0x68 with the new master's standing sprite (`+0x470`, FUN_141217000): a new
            // instance unless the unit shows it already. The slice holds sprites from an attack
            // on; a unit whose sprite it does not hold shows its old master's. Without the
            // graphics (no data file) the Sprite stream is not modelled.
            if !self.graphics.is_empty() && old.standing_sprite != next.standing_sprite {
                let g = (0 <= next.standing_sprite)
                    .then(|| self.graphics.get(&next.standing_sprite).copied())
                    .flatten()
                    .ok_or(Unsupported(
                        "a new master's standing sprite the data file does not have",
                    ))?;
                let rng = self.streams.get(Stream::Sprite);
                let unit = &mut self.players[player].objects[idx];
                if unit.sprite.is_some() {
                    crate::attack::set_sprite(&mut unit.sprite, g, rng);
                } else {
                    let _ = crate::sprite::ActiveSprite::start(g, rng);
                }
            }
            // the tracker's promotion (FUN_14104fb20): last in its new key master's list and
            // class's
            let id = self.players[player].objects[idx].id;
            self.track_promote(player, id, old.id, new);
            let owner = &mut self.players[player];
            for &(kind, amount, flag) in &next.storage {
                if kind >= 0 && flag == 2 {
                    owner.add_attribute(kind, amount)?;
                }
            }
        }
        let unit = &mut self.players[player].objects[idx];
        clear_powerups(unit);
        // in state 2 the volleys (`+0xa00`) are the new master's volley fire, the shares of the
        // units inside dropped
        if unit.life.state == 2 {
            unit.firepower = next.volley_fire;
        }
        Ok(())
    }
}

/// The box of an order's units (`0x140e68b90`, the loop after the splitter): the least and the
/// greatest x and y, where a value below the least so far moves only the least (the greatest
/// starts at 0, the least at `BOX_START`), and the sums. A box at most `SPREAD` each way gives
/// the centroid (the sums alone without a point); a wider one None.
fn centroid(points: &[[f32; 2]]) -> Option<[f32; 2]> {
    /// `DAT_142eea788`.
    const BOX_START: f32 = crate::image::order::BOX_START;
    /// `DAT_142eb9430`.
    const SPREAD: f32 = crate::image::order::SPREAD;
    let (mut least, mut most, mut sum) = ([BOX_START; 2], [0.0f32; 2], [0.0f32; 2]);
    for p in points {
        for k in 0..2 {
            let v = p[k];
            if least[k] <= v {
                if most[k] < v {
                    most[k] = v;
                }
            } else {
                least[k] = v;
            }
            sum[k] += v;
        }
    }
    if SPREAD < most[0] - least[0] || SPREAD < most[1] - least[1] {
        return None;
    }
    if !points.is_empty() {
        let n = points.len() as f32;
        sum = [sum[0] / n, sum[1] / n];
    }
    Some(sum)
}

/// A unit's point in an order to several (`0x140e68b90`): its offset from their centroid added to
/// the order's point, `EDGE` inside the map where it falls off it.
fn offset_point(
    position: [f32; 2],
    centroid: [f32; 2],
    point: [f32; 2],
    map: [i32; 2],
) -> [f32; 2] {
    [
        inside_map(position[0] - centroid[0] + point[0], map[0]),
        inside_map(position[1] - centroid[1] + point[1], map[1]),
    ]
}

/// An order's point on one axis kept on a map of `size` tiles: below 0 it becomes `EDGE`, at
/// the size or beyond the size less `EDGE`.
fn inside_map(v: f32, size: i32) -> f32 {
    /// `DAT_14330eb8c`.
    const EDGE: f32 = crate::image::order::EDGE;
    if v < 0.0 {
        EDGE
    } else if size as f32 <= v {
        size as f32 - EDGE
    } else {
        v
    }
}

/// FOLLOW's box (`FUN_14161c920`, the loop at `0x14161cc72`) over the places (x, y) of the loose
/// units that move, as its code has it: each unit's lesser and greater coordinate
/// (`CMOVBE` on `COMISS x, y` and `COMISS y, x`: a tie or NaN picks x) replace the last unit's,
/// which first fold into a running least (from [`BOX_START`]) and greatest (from 0), each by
/// `COMISS`/`CMOVBE` (a tie or NaN takes the last unit's); the sums run on.
/// The box passes when the last unit's spread and the running one's are each at most
/// [`SPREAD`] (`COMISS`, `JC`: NaN fails); then it gives the mean place (the sums over the
/// count, none without units).
fn follow_spread(points: &[[f32; 2]]) -> Option<[f32; 2]> {
    use crate::fmath::{at_most_or_unordered, below_or_unordered};
    /// `DAT_142eea788`.
    const BOX_START: f32 = crate::image::order::BOX_START;
    /// `DAT_142eb9430`.
    const SPREAD: f32 = crate::image::order::SPREAD;
    let (mut lesser, mut least, mut greater, mut greatest) = (BOX_START, BOX_START, 0.0f32, 0.0f32);
    let (mut sx, mut sy) = (0.0f32, 0.0f32);
    for &[x, y] in points {
        least = if at_most_or_unordered(lesser, least) {
            lesser
        } else {
            least
        };
        greatest = if at_most_or_unordered(greatest, greater) {
            greater
        } else {
            greatest
        };
        lesser = if at_most_or_unordered(x, y) { x } else { y };
        greater = if at_most_or_unordered(y, x) { x } else { y };
        sx += x;
        sy += y;
    }
    if below_or_unordered(SPREAD, greater - lesser) || below_or_unordered(SPREAD, greatest - least)
    {
        return None;
    }
    if !points.is_empty() {
        let n = points.len() as i32 as f32;
        (sx, sy) = (sx / n, sy / n);
    }
    Some([sx, sy])
}

/// A FOLLOW offset's coordinate on a map of `size` tiles (`FUN_14161c920`): above the size less
/// [`EDGE_GAP`] (at least 0, `MAXSS`) it is that (`COMISS`, `JA`), below 0 it is 0, else itself
/// (a NaN too).
fn follow_axis(v: f32, size: i32) -> f32 {
    /// `DAT_142eb99cc`.
    const EDGE_GAP: f32 = crate::image::group::SHORT_LENGTH;
    let most = crate::fmath::maxss(size as f32 - EDGE_GAP, 0.0);
    if v > most {
        most
    } else if 0.0 > v {
        0.0
    } else {
        v
    }
}

/// The cell of the `i`th of `n` units in MOVE's grid (`FUN_140e67800`): `side` cells a row, the
/// root of `n` times `GRID_AREA` truncated; the unit's column and row, less half the side, a
/// `GRID_STEP` each from the order's point, kept on the map ([`inside_map`]).
fn grid_point(i: usize, n: usize, point: [f32; 2], map: [i32; 2]) -> [f32; 2] {
    /// `DAT_142eb4838`.
    const GRID_AREA: f32 = crate::image::order::GRID_AREA;
    /// `DAT_142e7f384`.
    const GRID_STEP: f32 = crate::image::order::GRID_STEP;
    let side = crate::fmath::sqrt(n as i32 as f32 * GRID_AREA) as i32;
    let (i, half) = (i as i32, side / 2);
    let x = (i % side - half) as f32 * GRID_STEP + point[0];
    let y = (i / side - half) as f32 * GRID_STEP + point[1];
    [inside_map(x, map[0]), inside_map(y, map[1])]
}

/// `FUN_1412066b0`, a master change's powerups: the attack (`+0xa24`), reload (`+0xa28`), work
/// rate (`+0xa30`), most hit points (`+0xa3c`) and melee and pierce armor (`+0xa48`, `+0xa4c`)
/// powerups go; the hit points powerup (`+0xa38`) stays. The routine also zeroes `+0xa34`,
/// `+0xa40`, `+0xa50` and `+0x9f8`, empties the powerup map (`+0xa98`, count `+0xaa0`), sets
/// `+0xa2c` to slot 0x1b8 × 0.05 and takes a sight powerup (`+0xad4`) off (slots 0x1d8, 0x1d0):
/// none of them is held.
const fn clear_powerups(unit: &mut Unit) {
    unit.attack_powerup = 0.0;
    unit.reload_powerup = 0.0;
    unit.work_rate_powerup = 0.0;
    unit.hp_powerups[1] = 0.0;
    unit.armor_powerups = [0; 2];
}

/// `FUN_140c4da50`: the target of the first order in unit AI `ai`'s queue (`+0x20`, count
/// `+0x18`) that has one (not negative) and that its own unit `id` issued; −1 for none.
/// The adjusted idle timeout and secondary timer `FUN_14161b970` gives the unit AI of the
/// order's `i`th entry: 750 ms and the world's dt (`world+0x7c`) more for each six before it
/// (`(i / 6) as f64 * dt as f64`, `CVTTSD2SI` to 64 bits, the low 32 plus 750; the product
/// stays below 2^42, so the conversion never sees a value it cannot hold).
fn patrol_timeout(i: usize, dt: u32) -> u32 {
    let stagger = ((i / 6) as f64 * f64::from(dt)) as i64;
    (stagger as u32).wrapping_add(750)
}

/// The waypoints a walker from `from` passes on its way (`FUN_14161b970`'s two filters): each
/// one at least [`WAYPOINT_APART`] from the last one kept on x or on y, either way
/// (`COMISS`/`JNC` on each of the four: a NaN difference is not), the first measured from
/// `from`.
fn waypoints_from(from: [f32; 2], waypoints: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let mut last = from;
    let mut kept = Vec::with_capacity(waypoints.len());
    for &w in waypoints {
        let (dx, dy) = (w[0] - last[0], w[1] - last[1]);
        if WAYPOINT_APART <= dx
            || dx <= WAYPOINT_APART_LOW
            || WAYPOINT_APART <= dy
            || dy <= WAYPOINT_APART_LOW
        {
            kept.push(w);
            last = w;
        }
    }
    kept
}

fn own_order_target(ai: &crate::unit_ai::UnitAi, id: i32) -> i32 {
    ai.orders
        .iter()
        .find(|o| 0 <= o.target && o.issuer == id)
        .map_or(-1, |o| o.target)
}

/// A unit AI's slot 0x1c0 (`FUN_140c64d50`): the stance (`+0x1a9`) takes `state` and the stance's
/// place (`+0x1ac`, `+0x1b0`) the unit's (`place`, with its master's class); `NoAttack` (3) on a
/// unit AI that attacks (its order 700 or its action 600) stops the action (slot 0xb0 with 1,
/// [`crate::unit_ai::UnitAi::stop_with`], whose stopAction the caller runs) and forgets the
/// target (`FUN_140c5e1e0`).
fn set_stance(
    ai: &mut crate::unit_ai::UnitAi,
    state: u8,
    place: ([f32; 2], i16),
    fx: &mut Vec<crate::unit_ai::Effect>,
) -> Result<(), Unsupported> {
    ai.stance = state;
    ai.state_position = place.0;
    if state == 3 && (ai.current_order == 700 || ai.current_action == 600) {
        ai.stop_with(true, Some(place), fx)?;
        ai.remove_target();
    }
    Ok(())
}

/// A building's slot 0x670 (`FUN_1411dba30`): its gate lock (`mGateLocked`, `+0xc7c`, an int its
/// callers set to 0 or 1) takes `locked`; a lock also sets its last move time (`+0x870`,
/// [`crate::movement::Mover::last_move`]) to the world time (`FUN_1415300d0`) plus 1.
fn lock_gate(unit: &mut Unit, locked: bool, now: u32) -> Result<(), Unsupported> {
    let gate = unit
        .gate
        .as_mut()
        .ok_or(Unsupported("a GATE of a building without its gate state"))?;
    gate.locked = locked;
    if locked {
        unit.mover
            .as_mut()
            .ok_or(Unsupported("a locked gate without its moving part"))?
            .last_move = now.wrapping_add(1);
    }
    Ok(())
}

/// `setDifferentiatorGroups` (`FUN_1410395a0`): group `g`'s differentiator groups (`+0x108`,
/// count `+0x110`) are `list` instead. The routine's false for an allocation that failed cannot
/// come: `operator new` (`FUN_142a8bd2c`) throws rather than return none.
fn set_differentiator_groups(g: &mut crate::group::UnitGroup, list: &[u32]) {
    g.differentiator_groups.clear();
    g.differentiator_groups.extend_from_slice(list);
}

/// `FUN_14118c790`: unit `id` joins the end of a unit's ignore list (`+0x670`, count `+0x678`);
/// the list's capacity (`+0x67c`: 24 at first, then twice the count, the new slots −1) is the
/// `Vec`'s, which no reader sees.
fn ignore_unit(ignore: &mut Vec<i32>, id: i32) {
    ignore.push(id);
}

/// `FUN_1410862c0`: the footprint gap (`FUN_14118f680`) against the unit's ranges: whether it
/// is within the weapon range (slot 0x230; `COMISS`, `JC`: NaN out) and not too close, and
/// whether it is too close, below the least range (slot 0x240; `COMISS`, `SETA`: NaN not).
fn work_reach(gap: f32, weapon_range: f32, min_range: f32) -> (bool, bool) {
    let too_close = gap < min_range;
    let within = !crate::fmath::below_or_unordered(weapon_range, gap) && !too_close;
    (within, too_close)
}

/// Notify `0x1fe`: slot 0x7e8's target is below the unit's least range (the unit's own id twice,
/// the target, −1, −1).
const TOO_CLOSE: i32 = 0x1fe;

/// A whole share (`DAT_14330ebcc`), which `FUN_1411859a0` takes hit points against.
const DAMAGE_WHOLE: f32 = crate::image::order::DAMAGE_WHOLE;

/// The damage share `FUN_1411859a0` picks damage sprites by: `100 − hp × 100 / hit points`,
/// whole, of hit points at most the master's. An object whose share is its resource's (the
/// classes `FUN_141193590` names; its master's `+0x681` would add any) is not modelled.
fn damage_share(hp: f32, hit_points: i16, class: i16) -> Result<u8, Unsupported> {
    if matches!(
        class,
        5 | 7 | 8 | 9 | 10 | 15 | 29 | 31 | 32 | 33 | 38 | 40 | 41 | 48 | 58 | 61 | 63
    ) {
        return Err(Unsupported(
            "the damage share of a resource object (FUN_141193590)",
        ));
    }
    let most = f32::from(hit_points);
    let hp = hp.min(most);
    Ok((DAMAGE_WHOLE - hp * DAMAGE_WHOLE / most) as i32 as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// records/425906646's gate 27745 (masters 88 and 91, 1650 hit points each, three damage
    /// sprites): unhurt, its share is 0 under either master; a resource's share is not modelled.
    #[test]
    fn a_damage_share_is_the_whole_less_the_hit_points_share() {
        assert_eq!(damage_share(1650.0, 1650, 39), Ok(0));
        assert_eq!(damage_share(825.0, 1650, 39), Ok(50));
        assert_eq!(damage_share(1.0, 1650, 39), Ok(99));
        assert_eq!(damage_share(2000.0, 1650, 39), Ok(0));
        assert!(damage_share(10.0, 100, 58).is_err());
    }

    /// records/425906646's update 1641: player 6's drop-off order reaches its goose 28766 (class
    /// 58, 100 food, no drop sites, its default task on top): no table entry takes class 58, so
    /// it does nothing; an entry of its class would find a building.
    #[test]
    fn a_drop_off_order_leaves_a_livestock_without_a_drop_site_alone() {
        let goose = Unit {
            id: 28766,
            life: crate::lifecycle::Lifecycle {
                master_id: 1243,
                ..crate::lifecycle::Lifecycle::default()
            },
            ..Unit::default()
        };
        let mut t = Turns::new(
            0,
            1,
            0,
            vec![0],
            vec![crate::turn::Player {
                objects: vec![goose],
                ..crate::turn::Player::default()
            }],
        );
        t.masters = vec![crate::masters::Masters::new([Master {
            id: 1243,
            class: 58,
            ..Master::default()
        }])];
        let tc = crate::tasks::DropSite {
            building: 109,
            worker_class: 4,
            ..crate::tasks::DropSite::default()
        };
        t.drop_sites = vec![tc.clone()];
        assert!(t.drop_off_unasked(0, 0).is_ok());
        t.drop_sites = vec![
            tc,
            crate::tasks::DropSite {
                building: 1897,
                worker_class: 58,
                ..crate::tasks::DropSite::default()
            },
        ];
        assert!(t.drop_off_unasked(0, 0).is_err());
    }

    /// A world of player 1's soldiers (master 4, formation category 3) and villagers (master 83,
    /// category 0) at `units`' places, auto formations on with an influence of 3 tiles, and one
    /// group of id 5 holding `members`.
    fn soldiers(units: &[(i32, i16, [f32; 2])], members: &[i32]) -> Turns {
        let objects = units
            .iter()
            .map(|&(id, master, [x, y])| Unit {
                id,
                position: [x, y, 0.0],
                group: crate::group::GroupLink {
                    id: if members.contains(&id) { 5 } else { -1 },
                    flag: -2,
                },
                life: crate::lifecycle::Lifecycle {
                    master_id: master,
                    state: 2,
                    ..crate::lifecycle::Lifecycle::default()
                },
                ..Unit::default()
            })
            .collect();
        let mut t = Turns::new(
            0,
            1,
            0,
            vec![0, 0],
            vec![
                crate::turn::Player::default(),
                crate::turn::Player {
                    objects,
                    formations: crate::turn::Formations {
                        auto: true,
                        influence: 3.0,
                        by_speed: false,
                        ..crate::turn::Formations::default()
                    },
                    ..crate::turn::Player::default()
                },
            ],
        );
        let master = |id, formation_category| Master {
            id,
            kind: 70,
            formation_category,
            ..Master::default()
        };
        t.masters = vec![
            crate::masters::Masters::new([]),
            crate::masters::Masters::new([master(4, 3), master(83, 0)]),
        ];
        if !members.is_empty() {
            t.groups.groups.push(crate::group::UnitGroup {
                id: 5,
                members: members
                    .iter()
                    .map(|&id| crate::group::Member {
                        id,
                        ..crate::group::Member::default()
                    })
                    .collect(),
                ..crate::group::UnitGroup::default()
            });
        }
        t
    }

    /// `FUN_140fd76d0`: villagers move loose, soldiers within the influence distance of each
    /// other form a group (the one with the most joiners first, by id), one too far moves loose;
    /// with Ctrl held each soldier is a group of its own.
    #[test]
    fn the_splitter_groups_soldiers_near_each_other_and_leaves_villagers_loose() {
        let units = [
            (10, 83, [10.0, 10.0]),
            (11, 4, [20.0, 20.0]),
            (12, 4, [21.0, 20.0]),
            (13, 4, [23.5, 20.0]),
            (14, 4, [40.0, 20.0]),
        ];
        let mut t = soldiers(&units, &[]);
        let split = t.split(&[14, 13, 12, 11, 10], &|_| true, false).unwrap();
        // 12 has two to join it (11 and 13), 11 and 13 one each
        assert_eq!(split.groups, [vec![12, 11, 13]]);
        assert_eq!(split.loose, [10, 14]);
        let split = t.split(&[14, 13, 12, 11, 10], &|_| true, true).unwrap();
        assert_eq!(split.loose, [10]);
        assert_eq!(split.groups.len(), 4);
        // without auto formations every unit moves loose
        t.players[1].formations.auto = false;
        let split = t.split(&[14, 13, 12, 11, 10], &|_| true, false).unwrap();
        assert_eq!(split.loose, [10, 11, 12, 13, 14]);
        assert_eq!(split.groups, Vec::<Vec<i32>>::new());
    }

    /// `FUN_140fd76d0`'s first pass: units in a group the world holds keep their group with the
    /// order's other members, auto formations or not; a member ordered alone moves loose.
    #[test]
    fn the_splitter_keeps_the_order_s_members_of_an_existing_group_together() {
        let units = [
            (11, 4, [20.0, 20.0]),
            (12, 4, [60.0, 20.0]),
            (13, 4, [21.0, 20.0]),
        ];
        let mut t = soldiers(&units, &[11, 12]);
        t.players[1].formations.auto = false;
        let split = t.split(&[13, 12, 11], &|_| true, false).unwrap();
        assert_eq!(split.groups, [vec![11, 12]]);
        assert_eq!(split.loose, [13]);
        let split = t.split(&[12, 13], &|_| true, false).unwrap();
        assert_eq!(split.loose, [12, 13]);
    }

    /// `FUN_140e55a30`: of the first 60 ids (500 for a computer) one the world lacks or another
    /// player's (without the setting `+0x1074`) becomes -1; a human's command left without a unit
    /// is refused.
    #[test]
    fn the_command_check_drops_gone_units_and_others_units_of_its_first_60() {
        let units = [(11, 4, [20.0, 20.0]), (12, 4, [21.0, 20.0])];
        let mut t = soldiers(&units, &[]);
        t.players[0].objects.push(Unit {
            id: 30,
            ..Unit::default()
        });
        assert_eq!(
            t.command_check(1, &[11, 30, 99, -1]),
            Some(vec![11, -1, -1, -1])
        );
        t.settings.cheats_enabled = true;
        assert_eq!(t.command_check(1, &[11, 30, 99]), Some(vec![11, 30, -1]));
        t.settings.cheats_enabled = false;
        assert_eq!(t.command_check(1, &[30, 99]), None);
        t.players[1].computer = true;
        assert_eq!(t.command_check(1, &[30, 99]), Some(vec![-1, -1]));
        t.players[1].computer = false;
        // past the first 60 nothing is checked
        let mut ids = vec![99; 60];
        ids.push(30);
        assert_eq!(t.command_check(1, &ids), None);
        ids[0] = 11;
        let checked = t.command_check(1, &ids).unwrap();
        assert_eq!((checked[0], checked[1], checked[60]), (11, -1, 30));
    }

    /// `FUN_140e6b750`: each id's object, live or sleeping, None for a gone one, in the ids' order.
    #[test]
    fn the_lookup_keeps_a_gone_unit_s_entry() {
        let units = [(11, 4, [20.0, 20.0])];
        let mut t = soldiers(&units, &[]);
        t.players[1].sleeping.push(Unit {
            id: 40,
            ..Unit::default()
        });
        assert_eq!(
            t.lookup(&[99, 40, 11, -1]),
            [
                None,
                Some(Place::Sleeping(1, 0)),
                Some(Place::Live(1, 0)),
                None
            ]
        );
    }

    /// `FUN_14161e200`: the units of a STOP leave their unit group unless they are exactly its
    /// members (a gone unit's entry counts in the list); each unit AI's order queue empties and
    /// its idle timer takes its adjusted timeout; another player's unit the check dropped and a
    /// sleeping object take nothing.
    #[test]
    fn a_stop_takes_its_units_out_of_their_group_and_empties_their_orders() {
        let units = [(11, 4, [20.0, 20.0]), (12, 4, [21.0, 20.0])];
        let mut t = soldiers(&units, &[11, 12]);
        let order = crate::unit_ai::QueuedOrder {
            issuer: 1,
            order: MOVE_ORDER,
            priority: 100,
            target: -1,
            target_owner: -1,
            point: [1.0, 1.0, 0.0],
            range: 0.0,
            shown: [1.0, 1.0],
            ready: true,
            target_type: -1,
        };
        t.players[1].objects[0].ai = Some(crate::unit_ai::UnitAi {
            orders: vec![order],
            idle_timer: 5,
            adjusted_idle_timeout: 300,
            ..crate::unit_ai::UnitAi::default()
        });
        let ai = t.players[1].objects[0].ai.clone();
        t.players[0].objects.push(Unit {
            id: 30,
            ai,
            ..Unit::default()
        });
        t.players[1].sleeping.push(Unit {
            id: 40,
            ..Unit::default()
        });
        t.stop_command(1, &[11, 12, 99, 30, 40]).unwrap();
        let ai = t.players[1].objects[0].ai.as_ref().unwrap();
        assert_eq!((ai.orders.len(), ai.idle_timer), (0, 300));
        assert_eq!(t.players[1].objects[0].group.id, -1);
        assert_eq!(t.players[1].objects[1].group.id, -1);
        assert_eq!(t.groups.get(5).unwrap().members.len(), 0);
        assert_eq!(t.players[0].objects[0].ai.as_ref().unwrap().orders.len(), 1);
    }

    /// MOVE and INTERACT (`FUN_140e67800`, `FUN_140e68b90`): their loose units in a group the
    /// world holds leave it (`FUN_14102c160`), one unit or several; a unit in no group, or in a
    /// group the world does not hold, stays as it is.
    #[test]
    fn the_loose_units_of_a_move_or_an_interact_leave_their_groups() {
        let units = [(11, 83, [20.0, 20.0]), (12, 83, [40.0, 20.0])];
        let order = MoveOrder {
            target: -1,
            point: [30.0, 30.0],
            queued: false,
            direct: false,
            human: false,
            control: false,
        };
        let mut t = soldiers(&units, &[11]);
        t.move_order(1, &[11], &order).unwrap();
        assert_eq!(t.players[1].objects[0].group.id, -1);
        assert_eq!(t.groups.get(5).unwrap().members.len(), 0);
        // two groups' members ordered alone: both loose
        let two = || {
            let mut t = soldiers(&units, &[11]);
            t.groups.groups.push(crate::group::UnitGroup {
                id: 6,
                members: vec![crate::group::Member {
                    id: 12,
                    ..crate::group::Member::default()
                }],
                ..crate::group::UnitGroup::default()
            });
            t.players[1].objects[1].group.id = 6;
            t
        };
        let mut t = two();
        t.move_order(1, &[11, 12], &order).unwrap();
        assert_eq!(t.players[1].objects[0].group.id, -1);
        assert_eq!(t.players[1].objects[1].group.id, -1);
        assert_eq!(t.groups.get(5).unwrap().members.len(), 0);
        assert_eq!(t.groups.get(6).unwrap().members.len(), 0);
        // a group link the world does not hold
        let mut t = soldiers(&units, &[]);
        t.players[1].objects[0].group.id = 9;
        t.move_order(1, &[11], &order).unwrap();
        assert_eq!(t.players[1].objects[0].group.id, 9);
        let interact = Interact {
            target: -1,
            point: [30.0, 30.0],
            carry_test: false,
            queued: false,
            direct: false,
            human: false,
            filter: -1,
        };
        let mut t = two();
        for u in &mut t.players[1].objects {
            u.pasture = -1;
            u.my_pasture = -1;
        }
        t.interact(1, &[11, 12], &interact).unwrap();
        assert_eq!(t.players[1].objects[0].group.id, -1);
        assert_eq!(t.players[1].objects[1].group.id, -1);
    }

    /// A queued MOVE's new group (`FUN_140e67800` at `0x140e67b5d`): the first of its units the
    /// world holds (99 is not), with a unit AI, names the target the group's first command takes
    /// when the world holds it, at its place; a unit without a unit AI or a target the world
    /// does not hold names none.
    #[test]
    fn a_queued_move_s_new_group_takes_its_first_unit_s_target() {
        let units = [(11, 4, [20.0, 20.0]), (13, 4, [21.0, 20.0])];
        let ai = |target| crate::unit_ai::UnitAi {
            current_order: -1,
            current_action: -1,
            current_target: target,
            ..crate::unit_ai::UnitAi::default()
        };
        let mut t = soldiers(&units, &[]);
        t.players[1].objects[0].ai = Some(ai(13));
        t.players[1].objects[1].ai = Some(ai(11));
        assert_eq!(t.first_unit_target(&[99, 11, 13]), Some((13, [21.0, 20.0])));
        assert_eq!(t.first_unit_target(&[13, 11]), Some((11, [20.0, 20.0])));
        t.players[1].objects[0].ai = Some(ai(98));
        assert_eq!(t.first_unit_target(&[11, 13]), None);
        t.players[1].objects[0].ai = None;
        assert_eq!(t.first_unit_target(&[11, 13]), None);
        assert_eq!(t.first_unit_target(&[99]), None);
    }

    /// GATE and `FUN_1411dba30`: player 1's building 50 toggles its lock, a lock setting its last
    /// move time to the world time plus 1, an unlock leaving it; player 2's GATE of it does
    /// nothing unless players may order each other's units; a unit (type 70) takes nothing.
    #[test]
    fn a_gate_command_toggles_the_lock_of_a_building_of_its_player() {
        let units = [(11, 4, [20.0, 20.0])];
        let mut t = soldiers(&units, &[]);
        t.world_time = 400;
        t.players.push(crate::turn::Player::default());
        t.masters.push(crate::masters::Masters::new([]));
        t.masters[1] = crate::masters::Masters::new([
            Master {
                id: 4,
                kind: 70,
                ..Master::default()
            },
            Master {
                id: 79,
                kind: BUILDING,
                ..Master::default()
            },
        ]);
        t.players[1].objects.push(Unit {
            id: 50,
            gate: Some(crate::turn::Gate::default()),
            mover: Some(crate::hunt::tests::walker()),
            life: crate::lifecycle::Lifecycle {
                master_id: 79,
                state: 2,
                ..crate::lifecycle::Lifecycle::default()
            },
            ..Unit::default()
        });
        let building = |t: &Turns| {
            let b = &t.players[1].objects[1];
            (b.gate.unwrap().locked, b.mover.as_ref().unwrap().last_move)
        };
        t.gate(1, 50).unwrap();
        assert_eq!(building(&t), (true, 401));
        t.world_time = 500;
        t.gate(2, 50).unwrap();
        assert_eq!(building(&t), (true, 401));
        t.settings.cheats_enabled = true;
        t.gate(2, 50).unwrap();
        assert_eq!(building(&t), (false, 401));
        t.gate(1, 11).unwrap();
        t.gate(1, 99).unwrap();
        assert_eq!(t.players[1].objects[0].gate, None);
    }

    /// `TRANSFORM_OBJECT` and `FUN_1411e6d90`: player 1's trophy 60 (master 1649) takes trophy
    /// 1650, keeping its share of hit points, and in state 2 its build points take the new
    /// master's build time; in state 1 they stay.
    #[test]
    fn a_transformed_building_takes_its_new_master_s_build_time_in_state_2() {
        let units = [(11, 4, [20.0, 20.0])];
        let mut t = soldiers(&units, &[]);
        let master = |id, build_time| Master {
            id,
            kind: BUILDING,
            hit_points: 100,
            build_time,
            volley_fire: f32::from(build_time / 15),
            ..Master::default()
        };
        t.masters[1] = crate::masters::Masters::new([master(TROPHY, 10), master(1650, 30)]);
        let trophy = |state| Unit {
            id: 60,
            firepower: 7.0,
            construction: Some(crate::build::Construction {
                progress: 10.0,
                desolid: false,
                builders: 0,
                hit_points: 100,
                build_time: 10,
                record: crate::spatial::Obstruction { kind: 2, class: 2 },
                snow: false,
                sprite_override: false,
                started: true,
            }),
            life: crate::lifecycle::Lifecycle {
                master_id: TROPHY,
                state,
                hp: 50.0,
                ..crate::lifecycle::Lifecycle::default()
            },
            ..Unit::default()
        };
        t.players[1].objects = vec![trophy(2)];
        t.transform_object(1, 60, 1650).unwrap();
        let unit = &t.players[1].objects[0];
        assert_eq!(unit.life.master_id, 1650);
        assert_eq!(unit.life.hp, 50.0);
        assert_eq!(unit.construction.unwrap().progress, 30.0);
        // FUN_141225b70 in state 2: the volleys are the new master's volley fire
        assert_eq!(unit.firepower, 2.0);
        t.players[1].objects = vec![trophy(1)];
        t.transform_object(1, 60, 1650).unwrap();
        assert_eq!(t.players[1].objects[0].construction.unwrap().progress, 10.0);
        assert_eq!(t.players[1].objects[0].firepower, 7.0);
    }

    /// `FUN_141216f10` to a point: the unit AI's order queue empties only with the unit's
    /// `+0xa14`, which slot 0x128 with 0 clears while its work command runs; the move follows
    /// either way.
    #[test]
    fn a_move_inside_a_work_command_with_0_keeps_the_order_queue() {
        let units = [(11, 4, [20.0, 20.0])];
        let mut t = soldiers(&units, &[]);
        let order = crate::unit_ai::QueuedOrder {
            issuer: 1,
            order: MOVE_ORDER,
            priority: 100,
            target: -1,
            target_owner: -1,
            point: [1.0, 1.0, 0.0],
            range: 0.0,
            shown: [1.0, 1.0],
            ready: true,
            target_type: -1,
        };
        t.players[1].objects[0].ai = Some(crate::unit_ai::UnitAi {
            orders: vec![order],
            ..crate::unit_ai::UnitAi::default()
        });
        t.move_point_order_with(1, 0, [30.0, 30.0, 0.0], false)
            .unwrap();
        let unit = &t.players[1].objects[0];
        assert_eq!(unit.ai.as_ref().unwrap().orders.len(), 1);
        assert_eq!(unit.move_to.unwrap().goal, Some([30.0, 30.0, 0.0]));
        t.move_point_order(1, 0, [31.0, 30.0, 0.0]).unwrap();
        assert_eq!(t.players[1].objects[0].ai.as_ref().unwrap().orders.len(), 0);
    }

    /// `FUN_140e666f0`: another player's scout takes the order as its owner's, a unit without
    /// bit 4 nothing; of a human's ids only the first 60 count.
    #[test]
    fn an_auto_scout_reaches_any_player_s_scout_of_its_first_60_ids() {
        let units = [(11, 4, [20.0, 20.0])];
        let mut t = soldiers(&units, &[]);
        t.players.push(crate::turn::Player::default());
        let scout = Master {
            id: 448,
            kind: 70,
            class: 12,
            object_flags: 0x10,
            ..Master::default()
        };
        t.masters
            .push(crate::masters::Masters::new([scout.clone()]));
        t.masters[1] = crate::masters::Masters::new([
            scout,
            Master {
                id: 4,
                kind: 70,
                ..Master::default()
            },
        ]);
        let ai = crate::unit_ai::UnitAi {
            current_order: -1,
            current_action: -1,
            ..crate::unit_ai::UnitAi::default()
        };
        let unit = |id| Unit {
            id,
            position: [20.0, 20.0, 0.0],
            ai: Some(ai.clone()),
            life: crate::lifecycle::Lifecycle {
                master_id: 448,
                state: 2,
                ..crate::lifecycle::Lifecycle::default()
            },
            ..Unit::default()
        };
        t.players[1].objects[0].ai = Some(ai.clone());
        t.players[1].objects.push(unit(12));
        t.players[2].objects.push(unit(30));
        let mut ids = vec![99; 59];
        ids.extend([30, 11, 12]);
        t.auto_scout(1, &ids).unwrap();
        let order = |t: &Turns, p: usize, i: usize| {
            t.players[p].objects[i].ai.as_ref().unwrap().current_order
        };
        assert_eq!(order(&t, 2, 0), crate::unit_ai::AUTO_SCOUT_ORDER);
        assert_eq!(order(&t, 1, 0), -1);
        assert_eq!(order(&t, 1, 1), -1);
        t.auto_scout(1, &[12]).unwrap();
        assert_eq!(order(&t, 1, 1), crate::unit_ai::AUTO_SCOUT_ORDER);
    }

    /// `FUN_140c64d50`: the stance and its place are set; `NoAttack` on an attacking unit AI
    /// stops its action (keeping an order not of the action), asks for its stopAction and
    /// forgets its target; any other stance leaves them.
    #[test]
    fn no_attack_stops_an_attacking_unit_ai_and_forgets_its_target() {
        let attacking = crate::unit_ai::UnitAi {
            current_order: 700,
            current_action: 600,
            current_target: 40,
            target_type: 6,
            ..crate::unit_ai::UnitAi::default()
        };
        let mut ai = attacking;
        let mut fx = Vec::new();
        set_stance(&mut ai, 1, ([3.0, 4.0], 6), &mut fx).unwrap();
        assert_eq!((ai.stance, ai.state_position), (1, [3.0, 4.0]));
        assert_eq!((ai.current_action, ai.current_target), (600, 40));
        assert_eq!(fx, []);
        set_stance(&mut ai, 3, ([3.0, 5.0], 6), &mut fx).unwrap();
        assert_eq!(ai.stance, 3);
        // order 700 is the action's (600 + 100): the stop ends it, the state position the place
        assert_eq!((ai.current_order, ai.current_action), (-1, -1));
        assert_eq!((ai.current_target, ai.last_target), (-1, 40));
        assert_eq!(fx, [crate::unit_ai::Effect::Stop]);
    }

    /// `FUN_140e6a7c0` after `FUN_140e55d30`: another player's unit takes the stance only while
    /// players may order each other's; a class-54 unit takes no stance 1.
    #[test]
    fn a_stance_reaches_another_player_s_unit_only_under_the_setting() {
        let units = [(11, 4, [20.0, 20.0])];
        let mut t = soldiers(&units, &[]);
        t.players.push(crate::turn::Player::default());
        t.masters.push(crate::masters::Masters::new([Master {
            id: 4,
            kind: 70,
            class: 54,
            ..Master::default()
        }]));
        t.players[1].objects[0].ai = Some(crate::unit_ai::UnitAi::default());
        t.players[2].objects.push(Unit {
            id: 30,
            ai: Some(crate::unit_ai::UnitAi::default()),
            life: crate::lifecycle::Lifecycle {
                master_id: 4,
                state: 2,
                ..crate::lifecycle::Lifecycle::default()
            },
            ..Unit::default()
        });
        let stance = |t: &Turns, p: usize| t.players[p].objects[0].ai.as_ref().unwrap().stance;
        t.unit_ai_state(1, &[11, 30], 2).unwrap();
        assert_eq!((stance(&t, 1), stance(&t, 2)), (2, 0));
        t.settings.cheats_enabled = true;
        t.unit_ai_state(1, &[30], 1).unwrap();
        assert_eq!(stance(&t, 2), 0);
        t.unit_ai_state(1, &[30], 3).unwrap();
        assert_eq!(stance(&t, 2), 3);
    }

    /// `FUN_1412066b0`: the attack, reload, work rate, most hit points and armor powerups go, the
    /// hit points powerup and the reload stinger stay.
    #[test]
    fn a_master_change_clears_the_powerups_but_the_hit_points_one() {
        let mut unit = Unit {
            attack_powerup: 2.0,
            reload_powerup: 0.5,
            reload_stinger: 0.25,
            work_rate_powerup: 0.15,
            hp_powerups: [10.0, 20.0],
            armor_powerups: [1, 2],
            ..Unit::default()
        };
        clear_powerups(&mut unit);
        assert_eq!(
            (
                unit.attack_powerup,
                unit.reload_powerup,
                unit.work_rate_powerup
            ),
            (0.0, 0.0, 0.0)
        );
        assert_eq!(
            (unit.hp_powerups, unit.armor_powerups),
            ([10.0, 0.0], [0, 0])
        );
        assert_eq!(unit.reload_stinger, 0.25);
    }

    /// `FUN_140a97730`: an empty list and a `MoveTo` head take no move; an Artifact head takes
    /// the point (its goal); an action the slice does not hold is refused.
    #[test]
    fn the_head_action_takes_a_move_only_where_its_slot_0x40_does() {
        let units = [(11, 4, [20.0, 20.0])];
        let mut t = soldiers(&units, &[]);
        let point = [25.0, 21.0, 0.0];
        assert_eq!(t.head_takes_move(1, 0, None, point), Ok(false));
        t.players[1].objects[0].move_to = Some(crate::move_to::MoveToAction {
            run: false,
            state: crate::move_to::MoveTo::Start,
            goal: Some([30.0, 30.0, 0.0]),
            target: -1,
            range: 0.0,
            wait: crate::move_to::MoveWait::default(),
        });
        assert_eq!(t.head_takes_move(1, 0, Some(11), point), Ok(false));
        t.players[1].objects[0].move_to = None;
        t.players[1].objects[0].artifact = Some(crate::artifact::Artifact::new(false));
        assert_eq!(t.head_takes_move(1, 0, None, point), Ok(true));
        let artifact = t.players[1].objects[0].artifact.unwrap();
        assert_eq!(artifact.goal, Some(point));
        t.players[1].objects[0].artifact = None;
        t.players[1].objects[0].unmodelled_action = Some(Unsupported("a test's action"));
        assert!(t.head_takes_move(1, 0, None, point).is_err());
    }

    /// `FUN_14161c920`'s box: each unit's x is set against its own y, the units before the last
    /// fold into the running least and greatest; units side by side on a diagonal pass, two near
    /// each other far off it do not.
    #[test]
    fn follow_s_box_sets_each_unit_s_x_against_its_y() {
        assert_eq!(
            follow_spread(&[[20.0, 20.0], [21.0, 20.0]]),
            Some([20.5, 20.0])
        );
        assert_eq!(follow_spread(&[[10.0, 30.0], [10.5, 30.0]]), None);
        // the last unit's spread: 8 passes, more does not
        assert_eq!(follow_spread(&[[10.0, 18.0]]), Some([10.0, 18.0]));
        assert_eq!(follow_spread(&[[10.0, 18.5]]), None);
        // the running spread holds the units before the last
        assert_eq!(
            follow_spread(&[[10.0, 10.0], [30.0, 30.0], [30.0, 30.0]]),
            None
        );
        assert_eq!(follow_spread(&[]), Some([0.0, 0.0]));
    }

    /// A FOLLOW offset stays on the map: below 0 it is 0, past the size less 0.01 that.
    #[test]
    fn a_follow_offset_stays_on_the_map() {
        assert_eq!(follow_axis(-0.5, 120), 0.0);
        assert_eq!(follow_axis(130.0, 120), 119.99);
        assert_eq!(follow_axis(60.25, 120), 60.25);
        assert!(follow_axis(f32::NAN, 120).is_nan());
    }

    /// Player 1's soldiers 11 and 12 (moving, each with a unit AI) and player 2's 30 at (40, 20),
    /// a master with a footprint of class 6; auto formations off, a map of 120 tiles.
    fn followers() -> Turns {
        let units = [(11, 4, [20.0, 20.0]), (12, 4, [21.0, 20.0])];
        let mut t = soldiers(&units, &[]);
        t.players[1].formations.auto = false;
        t.spatial.map = [120, 120];
        let walker = crate::hunt::tests::walker();
        t.masters[1] = crate::masters::Masters::new([Master {
            id: 4,
            kind: 70,
            moving: Some(walker.stats),
            ..Master::default()
        }]);
        for u in &mut t.players[1].objects {
            u.mover = Some(walker.clone());
            u.ai = Some(crate::unit_ai::UnitAi {
                current_order: -1,
                current_action: -1,
                current_target: -1,
                ..crate::unit_ai::UnitAi::default()
            });
        }
        t.players.push(crate::turn::Player::default());
        t.masters.push(crate::masters::Masters::new([Master {
            id: 5,
            kind: 70,
            class: 6,
            radius: [0.5, 0.5],
            ..Master::default()
        }]));
        t.players[2].objects.push(Unit {
            id: 30,
            position: [40.0, 20.0, 0.0],
            life: crate::lifecycle::Lifecycle {
                master_id: 5,
                state: 2,
                ..crate::lifecycle::Lifecycle::default()
            },
            ..Unit::default()
        });
        t.spatial.insert(
            30,
            crate::spatial::Placed {
                position: [40.0, 20.0, 0.0],
                radius: [0.5, 0.5],
                class: 6,
                player: 2,
                master: 5,
                obstruction_type: 0,
                buildable_on: 0,
                tall: false,
                speed: 0.0,
                unknown: None,
                obstruction: None,
            },
        );
        t
    }

    /// `FUN_14161c920` for one unit: it follows the target (a `MoveTo` to it), its unit AI takes
    /// the follow order and action and the target with its class at its place.
    #[test]
    fn a_follower_takes_the_follow_order_and_walks_to_its_target() {
        let mut t = followers();
        t.follow(1, &[11], 30).unwrap();
        let unit = &t.players[1].objects[0];
        let ai = unit.ai.as_ref().unwrap();
        assert_eq!(
            (ai.current_order, ai.current_action),
            (crate::unit_ai::FOLLOW_ORDER, crate::unit_ai::FOLLOW_ACTION)
        );
        assert_eq!((ai.current_target, ai.target_type), (30, 6));
        assert_eq!(ai.target_point, [40.0, 20.0, 0.0]);
        assert_eq!(unit.move_to.unwrap().target, 30);
        assert_eq!(t.players[1].objects[1].move_to, None);
    }

    /// `FUN_14161c920` for two units side by side: each moves to the target at its offset from
    /// the two's mean, takes the move order and action, and ignores the other.
    #[test]
    fn followers_side_by_side_take_their_offsets_from_the_target() {
        let mut t = followers();
        t.follow(1, &[11, 12], 30).unwrap();
        for (i, (other, x)) in [(12, 39.5), (11, 40.5)].into_iter().enumerate() {
            let unit = &t.players[1].objects[i];
            let ai = unit.ai.as_ref().unwrap();
            assert_eq!(
                (ai.current_order, ai.current_action),
                (MOVE_ORDER, crate::unit_ai::MOVE_ACTION)
            );
            assert_eq!((ai.current_target, ai.target_point), (30, [x, 20.0, 0.0]));
            assert_eq!(unit.mover.as_ref().unwrap().ignore_units, [other]);
            assert_eq!(unit.move_to.unwrap().target, 30);
        }
    }

    /// FOLLOW's check: another player's unit takes nothing while players may not order each
    /// other's units.
    #[test]
    fn a_follow_of_another_player_s_unit_does_nothing() {
        let mut t = followers();
        t.follow(2, &[11], 30).unwrap();
        assert_eq!(
            t.players[1].objects[0].ai.as_ref().unwrap().current_order,
            -1
        );
    }

    /// `FUN_140aaddb0`: a Build on an annex counts its main building's builders one less; a
    /// foundation the world no longer holds counts nothing.
    #[test]
    fn a_builder_leaves_the_main_building_of_an_annex() {
        let units = [(11, 4, [20.0, 20.0])];
        let mut t = soldiers(&units, &[]);
        t.masters[1] = crate::masters::Masters::new([Master {
            id: 79,
            kind: BUILDING,
            ..Master::default()
        }]);
        let construction = crate::build::Construction {
            progress: 0.0,
            desolid: true,
            builders: 2,
            hit_points: 100,
            build_time: 10,
            record: crate::spatial::Obstruction { kind: 2, class: 2 },
            snow: false,
            sprite_override: false,
            started: true,
        };
        let building = |id, annex_of| Unit {
            id,
            annex_of,
            construction: Some(construction),
            life: crate::lifecycle::Lifecycle {
                master_id: 79,
                ..crate::lifecycle::Lifecycle::default()
            },
            ..Unit::default()
        };
        t.players[1].objects = vec![building(60, None), building(70, Some(60))];
        t.leave_builders(70).unwrap();
        let builders = |t: &Turns, i: usize| t.players[1].objects[i].construction.unwrap().builders;
        assert_eq!((builders(&t, 0), builders(&t, 1)), (1, 2));
        t.leave_builders(60).unwrap();
        t.leave_builders(99).unwrap();
        assert_eq!(builders(&t, 0), 0);
    }

    /// `FUN_140c4da50`: the first queued order with a target that the unit itself issued; an
    /// order of another issuer or without a target does not count.
    #[test]
    fn a_unit_s_own_queued_order_names_its_target() {
        let order = |issuer, target| crate::unit_ai::QueuedOrder {
            issuer,
            order: MOVE_ORDER,
            priority: 100,
            target,
            target_owner: -1,
            point: [1.0, 1.0, 0.0],
            range: 0.0,
            shown: [1.0, 1.0],
            ready: true,
            target_type: -1,
        };
        let mut ai = crate::unit_ai::UnitAi {
            orders: vec![order(1, 50), order(11, -1), order(11, 60), order(11, 70)],
            ..crate::unit_ai::UnitAi::default()
        };
        assert_eq!(own_order_target(&ai, 11), 60);
        ai.orders.truncate(2);
        assert_eq!(own_order_target(&ai, 11), -1);
    }

    /// `FUN_140a7a230`: a new target is taken (its workers count one more) and the old one let
    /// go; a quiet sub-action does neither; a target in state 7 or more is not taken; the same
    /// target changes nothing.
    #[test]
    fn an_action_s_new_target_is_taken_and_its_old_one_let_go() {
        let units = [(11, 4, [20.0, 20.0])];
        let mut t = soldiers(&units, &[]);
        t.masters[0] = crate::masters::Masters::new([Master {
            id: 66,
            kind: 70,
            ..Master::default()
        }]);
        let deer = |id, state, workers| Unit {
            id,
            workers,
            combat: true,
            life: crate::lifecycle::Lifecycle {
                master_id: 66,
                state,
                ..crate::lifecycle::Lifecycle::default()
            },
            ..Unit::default()
        };
        t.players[0].objects = vec![deer(40, 2, 1), deer(41, 2, 0), deer(42, 7, 0)];
        let workers =
            |t: &Turns| -> Vec<i32> { t.players[0].objects.iter().map(|u| u.workers).collect() };
        assert_eq!(t.retarget_action(1, 0, 40, 40, false), Ok(40));
        assert_eq!(workers(&t), [1, 0, 0]);
        assert_eq!(t.retarget_action(1, 0, 40, 41, true), Ok(41));
        assert_eq!(workers(&t), [1, 0, 0]);
        assert_eq!(t.retarget_action(1, 0, 40, 41, false), Ok(41));
        assert_eq!(workers(&t), [0, 1, 0]);
        assert_eq!(t.retarget_action(1, 0, 41, 42, false), Ok(-1));
        assert_eq!(workers(&t), [0, 0, 0]);
        assert_eq!(t.retarget_action(1, 0, 99, -1, false), Ok(-1));
    }

    /// `FUN_140ab0300`: a Build counted among its foundation's builders leaves them for a new
    /// target, which it takes; the same target keeps the count.
    #[test]
    fn a_build_s_new_target_leaves_the_old_foundation_s_builders() {
        let units = [(11, 4, [20.0, 20.0])];
        let mut t = soldiers(&units, &[]);
        t.masters[1] = crate::masters::Masters::new([
            Master {
                id: 4,
                kind: 70,
                ..Master::default()
            },
            Master {
                id: 79,
                kind: BUILDING,
                ..Master::default()
            },
        ]);
        let foundation = |id, builders| Unit {
            id,
            construction: Some(crate::build::Construction {
                progress: 0.0,
                desolid: true,
                builders,
                hit_points: 100,
                build_time: 10,
                record: crate::spatial::Obstruction { kind: 2, class: 2 },
                snow: false,
                sprite_override: false,
                started: true,
            }),
            workers: 1,
            life: crate::lifecycle::Lifecycle {
                master_id: 79,
                ..crate::lifecycle::Lifecycle::default()
            },
            ..Unit::default()
        };
        t.players[1].objects.push(foundation(60, 1));
        t.players[1].objects.push(foundation(61, 0));
        t.players[1].objects[0].build = Some(crate::build::BuildAction {
            target: 60,
            state: 6,
            move_to: None,
            goal: None,
            task: crate::build::BuildTask {
                range: 0.1,
                work_value: 1.0,
                work_rate: 1.0,
            },
            added: true,
            timer: 0.0,
        });
        assert_eq!(t.build_set_target(1, 0, (60, true), 60), Ok(60));
        assert!(t.players[1].objects[0].build.unwrap().added);
        assert_eq!(t.build_set_target(1, 0, (60, true), 61), Ok(61));
        let build = t.players[1].objects[0].build.unwrap();
        assert_eq!((build.target, build.added), (61, false));
        let site = |t: &Turns, i: usize| {
            let u = &t.players[1].objects[i];
            (u.construction.unwrap().builders, u.workers)
        };
        assert_eq!((site(&t, 1), site(&t, 2)), ((0, 0), (0, 2)));
    }

    /// `FUN_141211340` and `FUN_14118f580` in a gate that shut: a soldier dies (no hit points),
    /// class 42 and livestock with food are spared, livestock with a NaN load is not, and a
    /// sleeping object (below type 70) wakes to the end of the live list and dies.
    #[test]
    fn a_shutting_gate_kills_the_units_in_it_but_the_spared() {
        let units = [(11, 4, [20.0, 20.0])];
        let mut t = soldiers(&units, &[]);
        let master = |id, kind, class| Master {
            id,
            kind,
            class,
            ..Master::default()
        };
        t.masters[1] = crate::masters::Masters::new([
            master(4, 70, 6),
            master(42, 70, 42),
            master(58, 70, LIVESTOCK),
            master(10, 10, 5),
        ]);
        let unit = |id, master, amount| Unit {
            id,
            life: crate::lifecycle::Lifecycle {
                master_id: master,
                hp: 10.0,
                amount,
                state: 2,
                ..crate::lifecycle::Lifecycle::default()
            },
            ..Unit::default()
        };
        t.players[1].objects = vec![
            unit(11, 4, 0.0),
            unit(12, 42, 0.0),
            unit(13, 58, 50.0),
            unit(14, 58, f32::NAN),
        ];
        t.players[1].sleeping = vec![unit(15, 10, 0.0)];
        for id in 11..=16 {
            t.shut_in_gate(id).unwrap();
        }
        let hp: Vec<(i32, f32)> = t.players[1]
            .objects
            .iter()
            .map(|u| (u.id, u.life.hp))
            .collect();
        assert_eq!(
            hp,
            [(11, 0.0), (12, 10.0), (13, 10.0), (14, 0.0), (15, 0.0)]
        );
        assert_eq!(t.players[1].sleeping.len(), 0);
    }

    /// `FUN_14106c6a0`: a villager's own master without a task for its player's foundation
    /// takes the first master of its class, type and swap group with one, searched with any
    /// filter, and becomes it only with the change; one with its own task keeps its master.
    #[test]
    fn slot_0x820_takes_the_first_master_of_the_swap_group_with_a_task() {
        let units = [(11, 83, [20.0, 20.0])];
        let mut t = soldiers(&units, &[]);
        let task = |action_type, object_id| crate::masters::Task {
            action_type,
            object_group: -1,
            object_id,
            work_val1: 1.0,
            ..crate::masters::Task::default()
        };
        let villager = |id, tasks| Master {
            id,
            kind: 70,
            class: 4,
            task_by_group: 1,
            hit_points: 25,
            tasks,
            ..Master::default()
        };
        t.masters[1] = crate::masters::Masters::new([
            villager(83, vec![task(5, 53)]),
            villager(118, vec![task(101, -1)]),
            Master {
                id: 70,
                kind: BUILDING,
                class: 3,
                hit_points: 550,
                ..Master::default()
            },
        ]);
        t.players[1].objects[0].life.hp = 25.0;
        t.players[1].objects.push(Unit {
            id: 60,
            life: crate::lifecycle::Lifecycle {
                master_id: 70,
                state: 0,
                hp: 550.0,
                ..crate::lifecycle::Lifecycle::default()
            },
            ..Unit::default()
        });
        let found = t.find_work_task(1, 0, 60, GARRISON, false).unwrap();
        assert_eq!(found.map(|t| t.action_type), Some(101));
        assert_eq!(t.players[1].objects[0].life.master_id, 83);
        let found = t.find_work_task(1, 0, 60, -1, true).unwrap();
        assert_eq!(found.map(|t| t.action_type), Some(101));
        assert_eq!(t.players[1].objects[0].life.master_id, 118);
        // 118's own task: no change
        assert!(t.find_work_task(1, 0, 60, -1, true).unwrap().is_some());
        assert_eq!(t.players[1].objects[0].life.master_id, 118);
    }

    /// `0x140ac60e0` case 6 and `FUN_140ac4c60`: the hunt's own sheep goes to the drop site, and
    /// so does the player's idle livestock in state 2 on a tile within 3 that the player sees,
    /// then the livestock around that; another player's or one too far stays.
    #[test]
    fn a_led_sheep_takes_the_idle_livestock_about_it_to_the_drop_site() {
        let units = [(11, 4, [30.0, 30.0])];
        let mut t = soldiers(&units, &[]);
        t.spatial = crate::spatial::Spatial::new([40, 40]);
        let sight = crate::visibility::Visibility::new(
            40,
            40,
            vec![],
            vec![u64::MAX; 1600],
            vec![],
            0,
            vec![],
        );
        t.spatial.sight = Some(sight);
        let walker = crate::hunt::tests::walker();
        let sheep_master = Master {
            id: 594,
            kind: 70,
            class: LIVESTOCK,
            radius: [0.2, 0.2],
            moving: Some(walker.stats),
            ..Master::default()
        };
        t.masters[1] = crate::masters::Masters::new([
            sheep_master.clone(),
            Master {
                id: 4,
                kind: 70,
                radius: [1.5, 1.5],
                ..Master::default()
            },
        ]);
        t.players.push(crate::turn::Player::default());
        t.masters.push(crate::masters::Masters::new([sheep_master]));
        let sheep = |id, x: f32, y: f32| Unit {
            id,
            position: [x, y, 0.0],
            mover: Some(walker.clone()),
            life: crate::lifecycle::Lifecycle {
                master_id: 594,
                state: 2,
                ..crate::lifecycle::Lifecycle::default()
            },
            ..Unit::default()
        };
        t.players[1].objects.extend([
            sheep(20, 10.5, 10.5),
            sheep(21, 12.5, 11.5),
            sheep(22, 20.5, 10.5),
        ]);
        t.players[2].objects.push(sheep(23, 11.5, 10.5));
        let placed = |player, master, at: [f32; 2], radius| crate::spatial::Placed {
            position: [at[0], at[1], 0.0],
            radius: [radius, radius],
            class: LIVESTOCK,
            player,
            master,
            obstruction_type: 0,
            buildable_on: 0,
            tall: false,
            speed: 0.8,
            unknown: None,
            obstruction: None,
        };
        t.spatial.insert(11, placed(1, 4, [30.0, 30.0], 1.5));
        for (id, p, at) in [
            (20, 1, [10.5, 10.5]),
            (21, 1, [12.5, 11.5]),
            (22, 1, [20.5, 10.5]),
            (23, 2, [11.5, 10.5]),
        ] {
            t.spatial.insert(id, placed(p, 594, at, 0.2));
        }
        t.lead_herd(1, 20, 11).unwrap();
        let goes = |p: usize, id: i32| {
            t.players[p]
                .objects
                .iter()
                .find(|u| u.id == id)
                .and_then(|u| u.move_to)
                .map(|m| m.target)
        };
        assert_eq!(goes(1, 20), Some(11));
        assert_eq!(goes(1, 21), Some(11));
        assert_eq!(goes(1, 22), None);
        assert_eq!(goes(2, 23), None);
    }

    /// `FUN_1410395a0`: the old list goes, the new one is copied whole, an empty one too.
    #[test]
    fn a_group_s_differentiator_groups_are_the_list_given() {
        let mut g = crate::group::UnitGroup {
            differentiator_groups: vec![1, 2, 3],
            ..crate::group::UnitGroup::default()
        };
        set_differentiator_groups(&mut g, &[7, 8]);
        assert_eq!(g.differentiator_groups, [7, 8]);
        set_differentiator_groups(&mut g, &[]);
        assert_eq!(g.differentiator_groups, Vec::<u32>::new());
    }

    /// `FUN_14118c790`: each id joins the end of the ignore list, a repeated one again.
    #[test]
    fn an_ignored_unit_joins_the_end_of_the_ignore_list() {
        let mut ignore = vec![7];
        for id in [9, 8, 9] {
            ignore_unit(&mut ignore, id);
        }
        assert_eq!(ignore, [7, 9, 8, 9]);
    }

    /// `FUN_1410862c0`: too close below the least range (COMISS, SETA: NaN not), in reach at
    /// most the weapon range (COMISS, JC: NaN out) and not too close.
    #[test]
    fn a_target_is_in_reach_within_the_weapon_range_and_not_below_the_least() {
        assert_eq!(work_reach(3.0, 5.0, 2.0), (true, false));
        assert_eq!(work_reach(5.0, 5.0, 2.0), (true, false));
        assert_eq!(work_reach(5.5, 5.0, 2.0), (false, false));
        assert_eq!(work_reach(1.0, 5.0, 2.0), (false, true));
        assert_eq!(work_reach(2.0, 5.0, 2.0), (true, false));
        assert_eq!(work_reach(f32::NAN, 5.0, 2.0), (false, false));
        assert_eq!(work_reach(0.0, 0.0, 0.0), (true, false));
    }

    /// Player 1's soldier 10 (master 4, ranges 2 to 5) at (10, 10) and player 2's unit 20 at
    /// (`x`, 10), the two enemies, each with a unit AI in state 2.
    fn soldier_and_enemy(x: f32, class: i16) -> Turns {
        let unit = |id, master, x: f32| Unit {
            id,
            position: [x, 10.0, 0.0],
            ai: Some(crate::unit_ai::UnitAi::default()),
            life: crate::lifecycle::Lifecycle {
                master_id: master,
                state: 2,
                ..crate::lifecycle::Lifecycle::default()
            },
            ..Unit::default()
        };
        let player = |objects, relations| crate::turn::Player {
            objects,
            relations,
            ..crate::turn::Player::default()
        };
        let mut t = Turns::new(
            0,
            1,
            0,
            vec![0, 0, 0],
            vec![
                player(vec![], vec![0, 3, 3]),
                player(vec![unit(10, 4, 10.0)], vec![3, 0, 3]),
                player(vec![unit(20, 5, x)], vec![3, 3, 0]),
            ],
        );
        let master = |id, class| Master {
            id,
            kind: 70,
            class,
            weapon_range: 5.0,
            min_range: 2.0,
            radius: [0.2, 0.2],
            ..Master::default()
        };
        t.masters = vec![
            crate::masters::Masters::new([]),
            crate::masters::Masters::new([master(4, class)]),
            crate::masters::Masters::new([master(5, 6)]),
        ];
        for (id, player, x) in [(10, 1, 10.0), (20, 2, x)] {
            t.spatial.insert(
                id,
                crate::spatial::Placed {
                    position: [x, 10.0, 0.0],
                    radius: [0.2, 0.2],
                    class,
                    player,
                    master: 4,
                    obstruction_type: 0,
                    buildable_on: 0,
                    tall: false,
                    speed: 0.0,
                    unknown: None,
                    obstruction: None,
                },
            );
        }
        t
    }

    /// `FUN_141227190` on an enemy nearer than the least range: the unit posts notify 0x1fe
    /// with the target to its own unit AI (slot 0x328) and takes no work.
    #[test]
    fn an_order_on_an_enemy_below_the_least_range_notifies_the_unit_ai() {
        let mut t = soldier_and_enemy(11.0, 6);
        let work = Work {
            target: 20,
            point: [11.0, 10.0, 0.0],
            carry_test: false,
            filter: -1,
            notified: true,
        };
        t.order_work(1, 0, &work).unwrap();
        let unit = &t.players[1].objects[0];
        let ai = unit.ai.as_ref().unwrap();
        assert_eq!(
            ai.notifies,
            [crate::unit_ai::Notify::own(10, 0x1fe, [20, -1, -1])]
        );
        assert_eq!(unit.top_action_kind(), None);
    }

    /// `FUN_140fd9ae0` with 61 or more: the sorted list (a gone unit null, sorted first) splits
    /// in runs of 60, each split on its own (`FUN_140fd76d0`) into the one split: its groups count
    /// on, and an id repeated across two runs is in each.
    #[test]
    fn the_splitter_splits_an_order_to_more_than_60_units_in_runs_of_60() {
        let units: Vec<_> = (100..161).map(|id| (id, 4, [20.0, 20.0])).collect();
        let mut t = soldiers(&units, &[]);
        let ids: Vec<i32> = (100..161).rev().collect();
        let split = t.split(&ids, &|_| true, false).unwrap();
        assert_eq!(split.groups, [(100..160).collect::<Vec<_>>()]);
        assert_eq!(split.loose, [160]);
        // the gone 999 is the first run's first entry
        let gone: Vec<i32> = ids.iter().copied().chain([999]).collect();
        let split = t.split(&gone, &|_| true, false).unwrap();
        assert_eq!(
            split.groups,
            [(100..159).collect::<Vec<_>>(), vec![159, 160]]
        );
        assert_eq!(split.loose, Vec::<i32>::new());
        // 159 twice: the first run's last entry has no next one to repeat it
        let twice: Vec<i32> = (100..160).chain([159]).collect();
        let split = t.split(&twice, &|_| true, false).unwrap();
        assert_eq!(split.groups, [(100..160).collect::<Vec<_>>()]);
        assert_eq!(split.loose, [159]);
        // with Ctrl held the first run fills the 60 groups, the second's group is dropped
        let split = t.split(&ids, &|_| true, true).unwrap();
        assert_eq!(split.groups.len(), 60);
        assert_eq!(split.groups[59], [159]);
        assert_eq!(split.loose, Vec::<i32>::new());
    }

    /// MarvinBlue/426048731's MOVE of 1569: player 2's villagers 9175 and 9177 (by id) take a
    /// grid of one cell a row, half a tile apart, from the order's point (their `MoveTos`' points in
    /// the save after 1568).
    #[test]
    fn units_of_a_move_take_the_cells_of_a_grid() {
        let point = [29.78125, 92.489_586];
        assert_eq!(grid_point(0, 2, point, [120, 120]), [29.78125, 92.489_586]);
        assert_eq!(grid_point(1, 2, point, [120, 120]), [29.78125, 92.989_586]);
        // nine units: three a row around the point; off the map an axis keeps a tile inside
        assert_eq!(grid_point(0, 9, [10.0, 10.0], [120, 120]), [9.5, 9.5]);
        assert_eq!(grid_point(4, 9, [10.0, 10.0], [120, 120]), [10.0, 10.0]);
        assert_eq!(grid_point(8, 9, [119.75, 0.25], [120, 120]), [119.0, 0.75]);
        assert_eq!(grid_point(0, 9, [0.25, 0.25], [120, 120]), [1.0, 1.0]);
    }

    /// MarvinBlue/426048731's INTERACT of 927: player 1's builders 9171 and 9173 stand side by
    /// side; the queued orders of the save after 927 hold their points around the order's.
    #[test]
    fn units_of_an_order_take_their_offsets_from_their_centroid() {
        let (a, b) = ([104.32466, 39.299_885], [103.67509, 39.299_885]);
        let c = centroid(&[a, b]).unwrap();
        let point = [108.37596, 43.91338];
        assert_eq!(
            offset_point(a, c, point, [120, 120]),
            [108.700_745, 43.91338]
        );
        assert_eq!(offset_point(b, c, point, [120, 120]), [108.05117, 43.91338]);
    }

    /// A unit left of the first one counted moves only the least x, so the greatest stays 0 and
    /// the box passes however far apart the units stand; a unit 9 tiles right of the first does
    /// not.
    #[test]
    fn the_box_of_an_order_moves_only_its_least_value_for_a_lower_one() {
        assert!(centroid(&[[50.0, 50.0], [30.0, 50.0]]).is_some());
        assert_eq!(centroid(&[[30.0, 50.0], [39.0, 50.0]]), None);
        assert_eq!(centroid(&[[30.0, 50.0], [38.0, 50.0]]), Some([34.0, 50.0]));
    }

    /// MarvinBlue/426048731's MOVE of 2979 (the save after 2977): player 2's livestock 9528 and
    /// 9529 (formation category 0) stay loose, the scout 9185 has none to join it.
    #[test]
    fn the_splitter_joins_units_of_one_kind_within_the_influence_distance() {
        let f = crate::turn::Formations {
            auto: true,
            influence: 3.0,
            by_speed: false,
            ..crate::turn::Formations::default()
        };
        let unit = |id, grouped, position: [f32; 3]| Splittable {
            id,
            group: -1,
            kind: 70,
            grouped,
            flag_1: false,
            speed: 1.2,
            position,
            moved_at: [position[0], position[1]],
            state: 2,
            radius: 0.2,
            restriction: Some(1),
        };
        // no planner: the distance decides
        let mut none = |_: &Splittable, _: &Splittable| Ok(None);
        let mut away = std::collections::BTreeSet::new();
        let scout = unit(9185, true, [42.047_367, 94.338_486, 0.0]);
        let sheep = unit(9529, false, [45.5, 93.5, 0.0]);
        assert!(!splits_together(&scout, &sheep, f, false, &mut none, &mut away).unwrap());
        // nearer than 20 tiles the planner's path decides: its goal reached (1) within the
        // influence distance
        let other = unit(9186, true, [44.0, 94.0, 0.0]);
        let mut short = |_: &Splittable, _: &Splittable| Ok(Some((1, 2.5)));
        assert!(splits_together(&scout, &other, f, false, &mut short, &mut away).unwrap());
        let mut long = |_: &Splittable, _: &Splittable| Ok(Some((1, 3.5)));
        assert!(!splits_together(&scout, &other, f, false, &mut long, &mut away).unwrap());
        let mut nearest = |_: &Splittable, _: &Splittable| Ok(Some((2, 2.5)));
        assert!(!splits_together(&scout, &other, f, false, &mut nearest, &mut away).unwrap());
        // a path that did not join puts the unit among the far ones; a NaN length joins
        assert_eq!(away, [9186].into());
        let mut lost = |_: &Splittable, _: &Splittable| Ok(Some((1, f32::NAN)));
        assert!(splits_together(&scout, &other, f, false, &mut lost, &mut away).unwrap());
        // a NaN distance takes no path and is within the influence distance
        let nowhere = Splittable {
            moved_at: [f32::NAN, 94.0],
            ..other
        };
        assert!(splits_together(&scout, &nowhere, f, false, &mut short, &mut away).unwrap());
        let far = unit(9187, true, [80.0, 94.0, 0.0]);
        assert!(!splits_together(&scout, &far, f, false, &mut none, &mut away).unwrap());
        let wide = crate::turn::Formations {
            influence: 40.0,
            ..f
        };
        assert!(splits_together(&scout, &far, wide, false, &mut none, &mut away).unwrap());
        // a member must stand in state 2 or more
        let born = Splittable { state: 0, ..far };
        assert!(!splits_together(&scout, &born, wide, true, &mut none, &mut away).unwrap());
    }

    #[test]
    fn a_point_off_the_map_stays_inside_its_edge() {
        let c = [10.0, 10.0];
        assert_eq!(
            offset_point([5.0, 15.0], c, [2.0, 118.0], [120, 120]),
            [1.0, 119.0]
        );
    }

    /// `FUN_14161b970`'s timers: 750 ms for the first six entries of the list (the gone ones
    /// count), the world's dt more for each six after.
    #[test]
    fn a_patrol_staggers_its_units_timeouts_by_sixes() {
        assert_eq!(patrol_timeout(0, 100), 750);
        assert_eq!(patrol_timeout(5, 100), 750);
        assert_eq!(patrol_timeout(6, 100), 850);
        assert_eq!(patrol_timeout(13, 33), 816);
        assert_eq!(patrol_timeout(499, 0), 750);
    }

    /// `FUN_14161b970`'s filter: a waypoint less than a tile from the last one kept on both x
    /// and y is passed over; a NaN difference does not count (`COMISS`/`JNC`).
    #[test]
    fn a_patrol_passes_over_a_waypoint_within_a_tile_of_the_last() {
        let w = [
            [10.5, 10.5],
            [11.0, 10.0],
            [11.5, 10.9],
            [11.5, 9.0],
            [f32::NAN, 9.0],
            [12.5, 9.0],
        ];
        assert_eq!(
            waypoints_from([10.0, 10.0], &w),
            [[11.0, 10.0], [11.5, 9.0], [12.5, 9.0]]
        );
        assert_eq!(
            waypoints_from([10.0, 10.0], &w[..1]),
            Vec::<[f32; 2]>::new()
        );
    }

    /// Player 1's soldiers of `soldiers`, each walking and with a unit AI off any order, at a
    /// world dt of 100 ms.
    fn patrollers(units: &[(i32, i16, [f32; 2])], members: &[i32]) -> Turns {
        let mut t = soldiers(units, members);
        t.dt = 100;
        for u in &mut t.players[1].objects {
            u.mover = Some(crate::hunt::tests::walker());
            u.ai = Some(crate::unit_ai::UnitAi {
                ai_type: 13,
                current_order: -1,
                current_action: -1,
                current_target: -1,
                target_type: -1,
                idle_timer: 400,
                adjusted_idle_timeout: 2000,
                secondary_timer: 3000,
                patrol_path: Some(crate::unit_ai::PatrolPath::default()),
                patrol_waypoint: 2,
                ..crate::unit_ai::UnitAi::default()
            });
        }
        t
    }

    /// MarvinBlue/426049701's first PATROL (update 25): player 1's scout alone, one waypoint,
    /// `+0x8` set. It moves loose (slot 0x340: a forced, walking `MoveTo` to the waypoint at its
    /// height), waits to move with an empty ignore list and no waypoints of its own; its unit AI
    /// takes the patrol order, the move action and no target at the waypoint, its timers 0 and
    /// 750, its saved patrol gone.
    #[test]
    fn a_lone_unit_patrols_to_its_waypoint() {
        let mut t = patrollers(&[(11, 4, [20.0, 20.0])], &[]);
        t.players[1].objects[0].position[2] = 1.5;
        t.patrol(1, &[11], &[[30.0, 20.0]], true).unwrap();
        let u = &t.players[1].objects[0];
        let move_to = u.move_to.unwrap();
        assert_eq!(
            (move_to.goal, move_to.target, move_to.range, move_to.run),
            (Some([30.0, 20.0, 1.5]), -1, 0.0, false)
        );
        let m = u.mover.as_ref().unwrap();
        assert_eq!(
            (m.waiting, m.ignore_units.len(), m.user_waypoints.len()),
            (1, 0, 0)
        );
        let ai = u.ai.as_ref().unwrap();
        assert_eq!(
            (ai.current_order, ai.last_order, ai.current_action),
            (crate::unit_ai::PATROL, -1, crate::unit_ai::MOVE_ACTION)
        );
        assert_eq!(
            (ai.current_target, ai.target_type, ai.target_point),
            (-1, -1, [30.0, 20.0, 1.5])
        );
        assert_eq!(
            (ai.idle_timer, ai.adjusted_idle_timeout, ai.secondary_timer),
            (0, 750, 750)
        );
        assert_eq!((ai.patrol_path.is_none(), ai.patrol_waypoint), (true, 0));
        assert_eq!(t.groups.commands, 1);
    }

    /// A lone unit's waypoints but the last become its own (slot 0x448, a whole coordinate moved
    /// to the tile's middle); the move goes to the last one kept, the unit AI's point is the
    /// command's last even where the filter left it out. Without `+0x8` the saved patrol stays.
    #[test]
    fn a_lone_unit_s_waypoints_but_the_last_become_its_own() {
        let mut t = patrollers(&[(11, 4, [20.0, 20.0])], &[]);
        let w = [[25.0, 20.0], [25.5, 20.5], [30.0, 20.0], [30.5, 20.5]];
        t.patrol(1, &[11], &w, false).unwrap();
        let u = &t.players[1].objects[0];
        assert_eq!(u.move_to.unwrap().goal, Some([30.0, 20.0, 0.0]));
        assert_eq!(
            u.mover.as_ref().unwrap().user_waypoints,
            [[25.5, 0.0, 20.5]]
        );
        let ai = u.ai.as_ref().unwrap();
        assert_eq!(ai.target_point, [30.5, 20.5, 0.0]);
        assert!(ai.patrol_path.is_some());
    }

    /// Loose units of a PATROL (villagers, category 0) list each other in their ignore lists;
    /// one whose waypoints all lie within a tile of it stays where it is, its order unchanged.
    #[test]
    fn loose_patrollers_ignore_each_other_and_one_at_its_waypoint_stays() {
        let mut t = patrollers(&[(11, 83, [20.0, 20.0]), (12, 83, [29.5, 20.0])], &[]);
        t.patrol(1, &[11, 12], &[[30.0, 20.0]], true).unwrap();
        let [a, b] = &t.players[1].objects[..] else {
            panic!("two units")
        };
        assert_eq!(a.mover.as_ref().unwrap().ignore_units, [12]);
        assert_eq!(b.mover.as_ref().unwrap().ignore_units, [11]);
        assert!(a.move_to.is_some() && b.move_to.is_none());
        assert_eq!(b.ai.as_ref().unwrap().current_order, -1);
        assert_eq!(b.mover.as_ref().unwrap().waiting, 1);
    }

    /// `FUN_140fd9f40`: the splitter keeps group 5 (the order's 11 and 12 are all of it); the
    /// loose 13 of type 70 with a formation category within the owner's influence of the
    /// group's place joins it (flags bit 17), the villager does not, and a unit among the far
    /// ones does not either.
    #[test]
    fn a_kept_group_takes_the_loose_soldiers_about_it() {
        let units = [
            (11, 4, [20.0, 20.0]),
            (12, 4, [21.0, 20.0]),
            (13, 4, [22.0, 20.0]),
            (14, 83, [21.0, 21.0]),
            (15, 4, [21.0, 19.0]),
        ];
        let mut t = soldiers(&units, &[11, 12]);
        for u in &mut t.players[1].objects {
            let mut m = crate::hunt::tests::walker();
            m.moved_at = u.position;
            u.mover = Some(m);
        }
        t.groups.get_mut(5).unwrap().motion.position[0] = [21.0, 0.0, 20.0];
        let mut split = Split {
            owner: Some(1),
            loose: vec![13, 14, 15],
            groups: vec![vec![11, 12]],
            far: [15].into(),
        };
        t.keep_group(&mut split, 5).unwrap();
        assert_eq!(split.loose, [14, 15]);
        let g = t.groups.get(5).unwrap();
        assert!(g.has(13));
        assert_eq!(g.flags & 0x2_0000, 0x2_0000);
        // the unit after one that joins is passed over: 13 joins, 15 slides into its place
        let mut t = soldiers(&units, &[11, 12]);
        t.groups.get_mut(5).unwrap().motion.position[0] = [21.0, 0.0, 20.0];
        let mut split = Split {
            owner: Some(1),
            loose: vec![13, 15],
            ..Split::default()
        };
        t.keep_group(&mut split, 5).unwrap();
        assert_eq!(split.loose, [15]);
    }

    /// A PATROL of two soldiers near each other: the splitter's group becomes a new group of
    /// player 1, listed (its differentiators the command count before and itself), with the
    /// waypoints but the last as its own and a looping path to the last, back and forth from 5
    /// tiles; the command count goes on.
    #[test]
    fn a_patrolling_group_walks_a_loop_through_its_waypoints() {
        let mut t = patrollers(&[(11, 4, [20.0, 20.0]), (12, 4, [21.0, 20.0])], &[]);
        // the group's statistics search the members' surroundings
        let mut s = crate::spatial::Spatial::with_tiles(
            [60, 60],
            crate::spatial::Tiles::new(60, 60, vec![1; 3600]),
        );
        let kind = crate::terrain::TerrainKind {
            shown: true,
            flags: 1,
            forest: false,
        };
        s.terrain = Some(crate::terrain::Terrain::new(
            60,
            60,
            vec![(0, 255, -1); 3600],
            vec![kind],
            vec![vec![1.0]],
        ));
        s.relations = vec![vec![0, 3], vec![0, 0]];
        s.unit_diplomacy = vec![vec![1, 4], vec![0, 1]];
        let mut v = crate::visibility::Visibility::new(60, 60, vec![], vec![], vec![], 0, vec![]);
        v.registry = Some(crate::registry::Registry::new(2, 65));
        s.sight = Some(v);
        t.spatial = s;
        let w = [[25.0, 25.0], [30.0, 20.0]];
        t.patrol(1, &[11, 12], &w, true).unwrap();
        assert_eq!(t.groups.groups.len(), 1);
        let g = &t.groups.groups[0];
        assert_eq!((g.owner, g.has(11), g.has(12)), (1, true, true));
        assert_eq!((g.differentiator_id, g.current_command), (0, 1));
        let top = g.motion.paths.last().unwrap();
        assert_eq!(top.path_flags & 3, 3);
        assert_eq!(top.points.len(), 3);
        assert_eq!(top.points[1..], [[25.0, 0.0, 25.0], [30.0, 0.0, 20.0]]);
        assert_eq!(t.groups.commands, 1);
        let ai = t.players[1].objects[0].ai.as_ref().unwrap();
        assert_eq!(
            (ai.adjusted_idle_timeout, ai.patrol_path.is_none()),
            (750, true)
        );
    }

    /// MOVE (`FUN_140e67800`) of the whole of group 5 and a loose soldier beside it: the
    /// splitter keeps the group (`FUN_140fd9f40`), which takes the soldier and the command
    /// (`BUnitGroup::command`); no new group is made. With a target the world holds the kept group's range test is not
    /// modelled.
    #[test]
    fn a_move_of_a_whole_group_keeps_it() {
        let units = [
            (11, 4, [20.0, 20.0]),
            (12, 4, [21.0, 20.0]),
            (13, 4, [22.0, 20.0]),
        ];
        let order = MoveOrder {
            target: -1,
            point: [40.0, 40.0],
            queued: false,
            direct: false,
            human: false,
            control: false,
        };
        let kept = || {
            let mut t = soldiers(&units, &[11, 12]);
            t.groups.get_mut(5).unwrap().motion.position[0] = [21.0, 0.0, 20.0];
            t
        };
        let mut t = kept();
        t.move_order(1, &[11, 12, 13], &order).unwrap();
        assert_eq!(t.groups.groups.len(), 1);
        let g = t.groups.get(5).unwrap();
        assert!(g.has(13));
        // the keep sets flags bit 17, which the group's command then clears
        assert_eq!((g.flags & 0x2_0000, g.differentiator_id), (0, 0));
        assert_eq!(t.groups.commands, 1);
        let mut t = kept();
        let at_12 = MoveOrder {
            target: 12,
            ..order
        };
        assert!(t.move_order(1, &[11, 12, 13], &at_12).is_err());
    }
}
