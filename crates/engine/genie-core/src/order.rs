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
/// `DAT_143f11960`): the units that move alone, in the order the splitter leaves them, and its
/// groups, each its first unit first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Split {
    pub(crate) loose: Vec<i32>,
    pub(crate) groups: Vec<Vec<i32>>,
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
/// breaks formations by speed, then (`member`: for the chosen unit's group, also `b` in a state
/// of 2 or more) within the player's influence distance on the ground (between where each last
/// moved to, slot 0x520: `FUN_14087e560` with the height zeroed). Within [`SPLIT_PATH_REACH`]
/// the game measures a path instead, when the planner of `b`'s restriction exists: from `b`'s
/// place to `a`'s (`FUN_140f7c530` with `a`'s radius on x as the radii and `b`'s as the range,
/// no tile search), which joins `b` when it reaches the goal (result 1) no longer than the
/// influence distance.
fn splits_together(
    a: &Splittable,
    b: &Splittable,
    f: crate::turn::Formations,
    member: bool,
    path: &mut SplitPath<'_>,
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
        return Ok(code == 1 && length <= f.influence);
    }
    Ok(d <= f.influence)
}

/// The class `FUN_140c89760` lets scout without the master's flag when a game setting allows it
/// (raw `0x3a`).
const AUTO_SCOUT_CLASS: i16 = 58;
/// The trophy master `TRANSFORM_OBJECT` changes ("Trophy None") and the trophies it may take.
const TROPHY: i16 = 1649;
const TROPHIES: std::ops::RangeInclusive<i16> = 1650..=1653;

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
        for &id in &loose {
            self.leave_group(id)?;
        }
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
    /// its units. An order to more than 60 units (`FUN_140fd9ae0`'s runs of 60) is not modelled.
    pub(crate) fn split(
        &mut self,
        units: &[i32],
        kept: &dyn Fn(i32) -> bool,
        control: bool,
    ) -> Result<Split, Unsupported> {
        if 60 < units.len() {
            return Err(Unsupported("an order to more than 60 units"));
        }
        // FUN_140fd9ae0 splits only when the list's first unit exists (its owner): else no unit
        // takes the order
        let Some((owner, _)) = units.first().and_then(|&id| self.object(id)) else {
            return Ok(Split::default());
        };
        let mut sorted = units.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        let mut left: Vec<Splittable> = Vec::with_capacity(sorted.len());
        for id in sorted {
            let Some((p, u)) = self.object(id) else {
                continue;
            };
            if !kept(id) {
                continue;
            }
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
        let mut split = Split::default();
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
            return Ok(split);
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
                    n += usize::from(splits_together(a, b, formations, false, &mut path)?);
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
                    if splits_together(&chosen, &left[i], formations, true, &mut path)? {
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
        Ok(split)
    }

    /// A loose unit of an order leaves its group (`FUN_14102c160` on the group of its `+0x6d0`
    /// that the world holds): not modelled.
    fn leave_group(&self, id: i32) -> Result<(), Unsupported> {
        let group = self.object(id).map_or(-1, |(_, u)| u.group.id);
        if self.groups.get(group).is_some() {
            return Err(Unsupported(
                "a unit in a group that an order moves alone (FUN_14102c160)",
            ));
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
        mover.ignore_units.extend_from_slice(others);
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
                self.leave_group(id)?;
                return self.move_unit(player, id, &[], None, order);
            }
            _ => {
                let loose = self
                    .split(units, &|_| true, order.control)
                    .and_then(|s| self.move_groups(player, &s.groups, order).map(|()| s.loose));
                self.groups.count_command();
                let loose = loose?;
                for &id in &loose {
                    self.leave_group(id)?;
                }
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
    /// order when it holds just them (a kept group, `FUN_140fd9f40`, not modelled); one the world
    /// does not hold takes nothing; else a new group of the player's (`FUN_14152efc0`) takes them
    /// ([`Turns::add_to_group`]), its statistics ([`Turns::group_statistics`]) and its place
    /// ([`Turns::set_initial_position`]). Each listed group (60 at most) takes the command count
    /// as its differentiator, with Ctrl held locks its formation to a line
    /// ([`Turns::lock_formation`] with 2) and takes the command ([`Turns::group_command`]); a new
    /// group of a queued MOVE first takes its first unit's target (not modelled). Then every
    /// listed group lists them all as its differentiator groups (`setDifferentiatorGroups`,
    /// `FUN_1410395a0`).
    fn move_groups(
        &mut self,
        player: usize,
        groups: &[Vec<i32>],
        order: &MoveOrder,
    ) -> Result<(), Unsupported> {
        /// The groups one command lists (`DAT_143f152b8`, below `0x3c`).
        const LISTED: usize = 60;
        let mut listed: Vec<i32> = Vec::new();
        for units in groups {
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
            if !new {
                return Err(Unsupported(
                    "a MOVE of a group the splitter keeps (FUN_140fd9f40)",
                ));
            }
            let id = self.groups.create(player as u32).id;
            for &unit in units {
                let _ = self.add_to_group(id, unit)?;
            }
            self.group_statistics(id, false)?;
            self.set_initial_position(id)?;
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
            if order.queued {
                return Err(Unsupported(
                    "a queued MOVE's new group on its first unit's target (FUN_14100efd0)",
                ));
            }
            let _ =
                self.group_command(id, order.target, order.point, (order.queued, order.direct))?;
        }
        let list: Vec<u32> = listed.iter().map(|&g| g as u32).collect();
        for &id in &listed {
            if let Some(g) = self.groups.get_mut(id) {
                g.differentiator_groups.clone_from(&list);
            }
        }
        Ok(())
    }

    /// FOLLOW (0x14, `FUN_140e66a60` -> `FUN_14161c920`) of `target`, which the caller found in
    /// the world (else the command does nothing), after the resets: one unit forms no group
    /// (`FUN_140fd9ae0`) and takes the order ([`Turns::follow_unit`]); several units, which the
    /// splitter may group and which follow at their offsets, are not modelled.
    pub(crate) fn follow(
        &mut self,
        player: usize,
        units: &[i32],
        target: i32,
    ) -> Result<(), Unsupported> {
        self.groups.count_command();
        match units {
            [] => Ok(()),
            &[id] => self.follow_unit(player, id, target),
            _ => Err(Unsupported("a FOLLOW of several units")),
        }
    }

    /// FOLLOW's one unit (`FUN_14161c920`'s loop for an order without groups): a unit the world
    /// does not hold takes nothing, nor does one of a type below 30 or without speed. Its ignore
    /// list empties (`FUN_141197da0`), it waits to move (slot 0x2a0 with 1) and takes the move to
    /// the target (slot 0x138, [`Turns::move_to_object`]). With slot
    /// 0x138's 0 the unit AI takes the follow order and action (`FUN_140c64b60` with 1,
    /// `FUN_140c64b50`); then the target with its class, for a target master with a footprint
    /// (`+0x5d8` or `+0x5dc` above 0; else none), and its place on the ground (`FUN_140c64be0`).
    /// Slot 0x2d8's walk and show targets and `+0x888` are not held; another player's unit is not
    /// modelled.
    fn follow_unit(&mut self, player: usize, id: i32, target: i32) -> Result<(), Unsupported> {
        use crate::unit_ai::{FOLLOW_ACTION, FOLLOW_ORDER};
        let Some(place) = self.place(id) else {
            return Ok(());
        };
        let Place::Live(p, idx) = place else {
            return Err(Unsupported(
                "a FOLLOW of a unit the slice does not hold live",
            ));
        };
        if p != player {
            return Err(Unsupported("a FOLLOW of another player's unit"));
        }
        let master = self.master_of(player, self.players[player].objects[idx].life.master_id)?;
        let speed = master.moving.as_ref().map_or(0.0, |m| m.speed);
        if master.kind < MOVING || speed <= 0.0 {
            return Ok(());
        }
        let (target_class, target_place) = {
            let (tp, t) = self
                .object(target)
                .ok_or(Unsupported("a FOLLOW whose target the slice does not hold"))?;
            let tm = self.master_of(tp, t.life.master_id)?;
            let footprint = 0.0 < tm.radius[0] || 0.0 < tm.radius[1];
            (footprint.then_some(i32::from(tm.class)), t.position)
        };
        let unit = &mut self.players[player].objects[idx];
        let mover = unit
            .mover
            .as_mut()
            .ok_or(Unsupported("a followed unit without movement state"))?;
        mover.ignore_units.clear();
        mover.waiting = 1;
        self.move_to_object(player, idx, target, [target_place[0], target_place[1], 0.0])?;
        let unit = &mut self.players[player].objects[idx];
        if let Some(ai) = unit.ai.as_mut() {
            ai.set_order(FOLLOW_ORDER, self.streams.get(Stream::Ai));
            ai.set_action(FOLLOW_ACTION);
            let (t, class) = target_class.map_or((-1, -1), |class| (target, class));
            ai.set_target(t, class, [target_place[0], target_place[1], 0.0]);
        }
        Ok(())
    }

    /// Slot 0x138 to `target` (`FUN_141216f10`): a villager is no longer under the town bell
    /// (`+0x9dc`); with `+0xa14` (1 but inside slot 0x128's work command with 0) its unit AI's
    /// order queue empties (`FUN_140c5e010`); then `FUN_14106d340`: unless the unit is its own
    /// target, its unit AI's notifies go with the world time (`FUN_140c5dc10`) and, but after a
    /// kill it stops on (`FUN_140c41c10`), its current action's slot 0x40 may take the target and
    /// `point` (a Hunt's, [`Turns::hunt_take_move`]; an Artifact's, `FUN_140aaca30`, would: not
    /// modelled; a `MoveTo`'s or an Explore's never); else a `MoveTo` to the target
    /// (`FUN_140aa0df0`: range 0 from slot 0x268, the walking sprite) replaces its actions (slot
    /// 0x808, [`MoveToAction::to_target`]). Its `command_flag` (`+0x888`) is not held.
    ///
    /// [`MoveToAction::to_target`]: crate::move_to::MoveToAction::to_target
    pub(crate) fn move_to_object(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        point: [f32; 3],
    ) -> Result<(), Unsupported> {
        let world_time = self.world_time;
        let villager = self
            .master_of(player, self.players[player].objects[idx].life.master_id)?
            .class
            == VILLAGER;
        let unit = &mut self.players[player].objects[idx];
        if villager {
            unit.town_bell.flag = false;
        }
        if let Some(ai) = unit.ai.as_mut() {
            ai.orders.clear();
        }
        if target == unit.id {
            return Ok(());
        }
        if let Some(ai) = unit.ai.as_mut() {
            ai.clear_notifies(world_time)?;
        }
        let stop_after_kill = unit.ai.as_ref().is_some_and(|ai| ai.stop_after_kill);
        let top = unit.top_action_kind();
        if unit.ai.is_some() && !stop_after_kill {
            match top {
                Some(HUNT) => {
                    if self.hunt_take_move(player, idx, Some(target), point)? {
                        return Ok(());
                    }
                }
                // the Gather's (0x140a8b740) always takes it
                Some(crate::gather::GATHER) => {
                    let _ = self.gather_take_move(player, idx, Some(target), point)?;
                    return Ok(());
                }
                // the base slot 0x40 (FUN_140a78490) takes nothing: a MoveTo's, an Explore's, a
                // Build's, an Attack's, an Enter's, a Fly's, a missile's, a Make's
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
                ) => {}
                // the Artifact's slot 8 (FUN_140aaca30) takes the target: set_state(5)
                Some(crate::artifact::ARTIFACT_TYPE) => {
                    return self.artifact_take_object(player, idx, target);
                }
                Some(_) => {
                    return Err(Unsupported(
                        "a move to an object of a unit with an action (its slot 0x40)",
                    ));
                }
            }
        }
        let unit = &self.players[player].objects[idx];
        if unit.unmodelled_action.is_some() {
            return Err(Unsupported("a move to an object of a unit with an action"));
        }
        self.clear_actions(player, idx)?;
        let unit = &mut self.players[player].objects[idx];
        let mover = unit
            .mover
            .as_mut()
            .ok_or(Unsupported("a moved unit without movement state"))?;
        let move_to = crate::move_to::MoveToAction::to_target(
            &self.spatial,
            unit.position,
            mover,
            target,
            0.0,
            false,
        )?;
        if move_to.arrived() {
            unit.shows_standing_sprite(
                self.masters.get(player),
                &self.graphics,
                self.streams.get(Stream::Sprite),
            )?;
        }
        unit.move_to = Some(move_to);
        Ok(())
    }

    /// An animal's Artifact takes a move to the object `target` (its slot 8, `FUN_140aaca30`,
    /// [`crate::artifact::Artifact::take_object`]): livestock (class 58) goes to a building
    /// within 1, to the town centre (master 109) at its point; a `MoveTo` it walked goes, whose
    /// teardown stops the unit (slot 0x5d0).
    pub(crate) fn artifact_take_object(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
    ) -> Result<(), Unsupported> {
        let unit = &self.players[player].objects[idx];
        let livestock = self.master_of(player, unit.life.master_id)?.class == LIVESTOCK;
        let (tp, t) = self.object(target).ok_or(Unsupported(
            "an Artifact's move to an object the slice does not hold",
        ))?;
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
        let walked = unit
            .artifact
            .as_mut()
            .is_some_and(|a| a.take_object(target, at, to_building, to_town_centre));
        if walked {
            unit.mover
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
        let at = self
            .spatial
            .get(sheep)
            .map(|q| q.position)
            .ok_or(Unsupported("a herded animal without a place"))?;
        let (x, y) = (at[0] as i32, at[1] as i32);
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

    /// `UNIT_AI_STATE` (`do_command` case 0x12: the check `FUN_140e55d30`, then `FUN_140e6a7c0`):
    /// each of `ids` the world holds, of the player's, with a unit AI takes the stance `state`
    /// (slot 0x1c0, `FUN_140c64d50`: `+0x1a9`, its place as the stance's, `+0x1ac`), but not
    /// stances 1 and 2 for classes 54 and 51 (and, the siege AI's slot, masters 42 and 331). An id
    /// gone or another player's counts for nothing (a game setting, `FUN_1412ffe00`, or the
    /// world's `+0xb12` may let another player's through, not modelled). `NoAttack` (3) on a unit
    /// that attacks (order 700 or action 600: slot 0xb0 with 1, `FUN_140c5e1e0`) and the UI's
    /// flag are not modelled.
    pub(crate) fn unit_ai_state(
        &mut self,
        player: usize,
        ids: &[i32],
        state: u8,
    ) -> Result<(), Unsupported> {
        for &id in ids {
            let Some(place) = self.place(id) else {
                continue;
            };
            let (p, idx) = match place {
                Place::Live(p, i) => (p, i),
                Place::Sleeping(..) => {
                    return Err(Unsupported("a UNIT_AI_STATE of a sleeping object"));
                }
            };
            if p != player {
                continue;
            }
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
            let unit = &mut self.players[p].objects[idx];
            let [x, y, _] = unit.position;
            let ai = unit.ai.as_mut().expect("checked above");
            ai.stance = state;
            ai.state_position = [x, y];
            if state == 3 && (ai.current_order == 700 || ai.current_action == 600) {
                return Err(Unsupported(
                    "NoAttack on an attacking unit (slot 0xb0, FUN_140c5e1e0)",
                ));
            }
        }
        Ok(())
    }

    /// GATE (`do_command` case 0x72): the object `id` (any player's: a missing one does nothing)
    /// of a master of type 80 toggles its gate lock (slot 0x670 with slot 0x668's value
    /// negated, `FUN_1411dba30`): a lock sets its last move time to the world time plus 1. The
    /// player must own it (another player's object, which a game setting, `FUN_1412ffe00`, or
    /// the world's `+0xb12` may let through, is not modelled). Its sound (slot 0x4c0) and the
    /// UI's flag are not held.
    pub(crate) fn gate(&mut self, player: usize, id: i32) -> Result<(), Unsupported> {
        let Some(place) = self.place(id) else {
            return Ok(());
        };
        let (p, idx) = match place {
            Place::Live(p, i) => (p, i),
            Place::Sleeping(..) => return Err(Unsupported("a GATE of a sleeping object")),
        };
        if p != player {
            return Err(Unsupported("a GATE of another player's object"));
        }
        let master = self.players[p].objects[idx].life.master_id;
        if self.master_of(p, master)?.kind != 80 {
            return Ok(());
        }
        let now = self.world_time;
        let unit = &mut self.players[p].objects[idx];
        let gate = unit
            .gate
            .as_mut()
            .ok_or(Unsupported("a GATE of a building without its gate state"))?;
        gate.locked = !gate.locked;
        if gate.locked {
            unit.mover
                .as_mut()
                .ok_or(Unsupported("a locked gate without its moving part"))?
                .last_move = now.wrapping_add(1);
        }
        Ok(())
    }

    /// `AUTO_SCOUT` (0x26, `FUN_140e666f0`): each unit of `ids` with a unit AI that may scout
    /// (`FUN_140c89760`: its master's object flags `+0x654` have bit 4; class 58's rule behind a
    /// game setting, `FUN_140eca780`, is not modelled) takes an order at once (slot 0x30 ->
    /// slot 0x28 with Immediate, [`crate::unit_ai::UnitAi::take_order_now`]): the stop order
    /// when its action is the auto scout's already, else the auto scout order, from the player,
    /// without a target, at the point (−1, −1) at the unit's height, range 1, priority 100, shown
    /// at (−1, −1) and ready. The handler looks ids up in every player's objects; another
    /// player's unit is not modelled. Its sound and the UI's flags are not held.
    pub(crate) fn auto_scout(&mut self, player: usize, ids: &[i32]) -> Result<(), Unsupported> {
        use crate::unit_ai::{
            AUTO_SCOUT_ACTION, AUTO_SCOUT_ORDER, Effect, QueuedOrder, STOP_ORDER,
        };
        for &id in ids {
            let Some(place) = self.place(id) else {
                continue;
            };
            let Place::Live(p, idx) = place else {
                return Err(Unsupported("an AUTO_SCOUT of a sleeping object"));
            };
            if p != player {
                return Err(Unsupported("an AUTO_SCOUT of another player's unit"));
            }
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
                issuer: player as i32,
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

    /// STOP (0x01, `RGE_Command::do_command` case 1, `FUN_14161e200`) of the player's units
    /// `ids` (the lookup `FUN_140e6b750`, in the packet's order, an id the world does not hold
    /// left out; the command's check `FUN_140e558b0`'s twin `FUN_140e55a30` drops another
    /// player's unit unless the game setting `+0x1074`, which the slice does not hold: it stops
    /// there). First each unit AI's saved patrol goes (`FUN_140c68de0`) and it resets as for an
    /// order (`FUN_141631040`, [`crate::unit_ai::UnitAi::reset_for_order`]); several units
    /// forming exactly one unit group would stop as the group (`FUN_14152ec80`,
    /// `FUN_1410110c0`), and the slice holds no groups. Then each unit leaves its group (none),
    /// its unit AI's order queue empties (`FUN_140c5e010`) and its idle timer takes its adjusted
    /// timeout (`+0x160` = `+0x164`), its ignore list empties (`FUN_141197da0`), the object stops
    /// (slot 0x148, [`Turns::object_stop`]) and its movement stands (slot 0x560,
    /// `FUN_1411616f0`: slot 0x5d0, then `FUN_140f98e80` on its walked level).
    pub(crate) fn stop_command(&mut self, player: usize, ids: &[i32]) -> Result<(), Unsupported> {
        let mut units = Vec::with_capacity(ids.len());
        for &id in ids {
            match self.place(id) {
                None => {}
                Some(Place::Live(p, idx)) if p == player => units.push(idx),
                Some(Place::Live(..)) => {
                    return Err(Unsupported(
                        "a STOP of another player's unit (the setting +0x1074)",
                    ));
                }
                Some(Place::Sleeping(..)) => {
                    return Err(Unsupported("a STOP of a sleeping object"));
                }
            }
        }
        for &idx in &units {
            let rng = self.streams.get(Stream::Ai);
            if let Some(ai) = self.players[player].objects[idx].ai.as_mut() {
                ai.patrol_path = false;
                ai.reset_for_order(rng);
            }
        }
        for &idx in &units {
            let unit = &mut self.players[player].objects[idx];
            if let Some(ai) = unit.ai.as_mut() {
                ai.orders.clear();
                ai.idle_timer = ai.adjusted_idle_timeout;
            }
            if let Some(m) = unit.mover.as_mut() {
                m.ignore_units.clear();
            }
            self.object_stop(player, idx)?;
            if let Some(m) = self.players[player].objects[idx].mover.as_mut() {
                m.stand()?;
            }
        }
        Ok(())
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
    fn standing_sprite(&mut self, p: usize, idx: usize) -> Result<(), Unsupported> {
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
        mover.ignore_units.extend_from_slice(others);
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
    /// (`FUN_140c5dc10` clears its notifies with the world time) lets its current action's slot
    /// 8 take the point (`FUN_140a97730`) unless it stops after a kill (`FUN_140c41c10`); else a
    /// `MoveTo` to the point replaces its actions. True when the action took it.
    pub(crate) fn move_to_point(
        &mut self,
        player: usize,
        idx: usize,
        point: [f32; 3],
    ) -> Result<bool, Unsupported> {
        // a unit with a unit AI (FUN_140c5dc10 clears its notifies with the world time) lets its
        // current action's slot 8 take the point (FUN_140a97730) unless it stops after a kill
        // (FUN_140c41c10): an Artifact action's does (FUN_140aaca30), a MoveTo's
        // (`FUN_140a78490`) never
        let world_time = self.world_time;
        let unit = &mut self.players[player].objects[idx];
        if let Some(ai) = unit.ai.as_mut() {
            ai.clear_notifies(world_time)?;
        }
        let top = unit.top_action_kind();
        let stop_after_kill = unit.ai.as_ref().is_some_and(|ai| ai.stop_after_kill);
        let taken = match (unit.ai.is_some() && !stop_after_kill, top) {
            (true, Some(crate::artifact::ARTIFACT_TYPE)) => {
                let walked = unit.artifact.as_mut().is_some_and(|a| a.take_point(point));
                if walked {
                    unit.mover
                        .as_mut()
                        .ok_or(Unsupported("a walker without movement state"))?
                        .stop()?;
                }
                true
            }
            // the Hunt's (0x140ac52c0, [`Turns::hunt_take_move`]) to a point: a task with a work
            // range (`+0x48`'s `+0x30`) does not take it, else the hunt walks there (state 15)
            // and lets its target go
            (true, Some(HUNT)) => self.hunt_take_move(player, idx, None, point)?,
            // the Gather's (0x140a8b740) walks to the point (set_state 15)
            (true, Some(crate::gather::GATHER)) => {
                self.gather_take_move(player, idx, None, point)?
            }
            // an Explore's slot 0x40 is the base's (`FUN_140a78490`), as a MoveTo's, a Build's
            // (0x140aade60), an Attack's, an Enter's, a Fly's, a missile's and a Make's
            (true, Some(t))
                if !matches!(
                    t,
                    crate::move_to::MOVE_TO
                        | crate::explore::EXPLORE
                        | crate::build::BUILD
                        | crate::attack::ATTACK
                        | crate::garrison::ENTER
                        | crate::fly::FLY
                        | crate::missile::MISSILE
                        | crate::make::MAKE
                ) =>
            {
                return Err(Unsupported("a move of a unit with an action (its slot 8)"));
            }
            _ => false,
        };
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
    /// `FUN_1410866e0`: its current action's slot 0x40 may take the point (the Artifact's,
    /// the Hunt's and the Gather's, as [`Turns::move_to_point`]'s), else a `MoveTo` to the point
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
        let top = unit.top_action_kind();
        let point = m.point;
        let taken = match top {
            Some(crate::artifact::ARTIFACT_TYPE) => {
                let unit = &mut self.players[player].objects[idx];
                let walked = unit.artifact.as_mut().is_some_and(|a| a.take_point(point));
                if walked {
                    unit.mover
                        .as_mut()
                        .ok_or(Unsupported("a walker without movement state"))?
                        .stop()?;
                }
                true
            }
            Some(HUNT) => self.hunt_take_move(player, idx, None, point)?,
            Some(crate::gather::GATHER) => self.gather_take_move(player, idx, None, point)?,
            Some(t)
                if !matches!(
                    t,
                    crate::move_to::MOVE_TO
                        | crate::explore::EXPLORE
                        | crate::build::BUILD
                        | crate::attack::ATTACK
                        | crate::garrison::ENTER
                        | crate::fly::FLY
                        | crate::missile::MISSILE
                        | crate::make::MAKE
                ) =>
            {
                return Err(Unsupported(
                    "a move off a footprint of a unit with an action (its slot 0x40)",
                ));
            }
            _ => false,
        };
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

    /// Slot 0x7e8 (`FUN_141227190`) of a unit on a target object: a unit in state 3 or later
    /// ignores it; without a target the world holds (none, or one gone) the work command goes to
    /// the point; against an unallied target the unit's least range must not exceed the gap
    /// (`FUN_1410862c0`); a villager drops its town-bell flag (`FUN_1412067f0`); then the work
    /// command; then it leaves the pasture it is in (`FUN_141189ca0`).
    pub(crate) fn order_work(
        &mut self,
        player: usize,
        idx: usize,
        order: &Work,
    ) -> Result<(), Unsupported> {
        let unit = &self.players[player].objects[idx];
        let master = self.master_of(player, unit.life.master_id)?;
        if master.class == CLASS_54 {
            return Err(Unsupported("an order to a unit of class 54"));
        }
        if 2 < unit.life.state {
            return Ok(());
        }
        let Some((tp, _)) = self.object(order.target) else {
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
        };
        let world = World {
            players: &self.players,
            drop_sites: &self.drop_sites,
        };
        if !world.allied(player, tp, true) {
            let (me, them) = (
                self.spatial
                    .get(unit.id)
                    .ok_or(Unsupported("an ordered unit without a place"))?,
                self.spatial
                    .get_known(order.target)?
                    .ok_or(Unsupported("a target without a place"))?,
            );
            let (_, gap) = gaps(me.position, me.radius, them.position, them.radius);
            if gap < master.min_range {
                return Err(Unsupported(
                    "an order on a target closer than the least range",
                ));
            }
            if master.class == CLASS_51 {
                return Err(Unsupported("an order to a unit of class 51"));
            }
        }
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
    /// true when it enters, false when nothing comes of it. It is entering the target already
    /// (its top action an Enter on it); else slot 0x820 (`FUN_14106c6a0`) looks for a task on
    /// the target with the garrison filter (3), without the master's change, whose Enter it would
    /// take.
    fn livestock_enters(
        &self,
        player: usize,
        idx: usize,
        target: i32,
    ) -> Result<bool, Unsupported> {
        let unit = &self.players[player].objects[idx];
        if unit.top_action_kind() == Some(crate::garrison::ENTER)
            && unit.top_target() == Some(target)
        {
            return Ok(true);
        }
        let masters = self
            .masters
            .get(player)
            .ok_or(Unsupported("an enter command: no master table"))?;
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
        Ok(masters
            .work_master(&actor, &t, &general, GARRISON, &world)?
            .is_some())
    }

    /// Slot 0x138 (`FUN_141216f10`) without a target, the no-task end of a work command on a
    /// point: a villager drops its town-bell flag (not held otherwise), the unit AI's order queue
    /// empties (`+0xa14` 1, `FUN_140c5e010`), then the move to the point
    /// ([`Turns::move_to_point`], `FUN_14106d340`).
    pub(crate) fn move_point_order(
        &mut self,
        player: usize,
        idx: usize,
        point: [f32; 3],
    ) -> Result<(), Unsupported> {
        let villager = self
            .master_of(player, self.players[player].objects[idx].life.master_id)?
            .class
            == VILLAGER;
        let unit = &mut self.players[player].objects[idx];
        if villager {
            unit.town_bell.flag = false;
        }
        if let Some(ai) = unit.ai.as_mut() {
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
            let own = unit.ai.as_ref().and_then(|ai| {
                ai.orders
                    .iter()
                    .find(|o| 0 <= o.target && o.issuer == unit.id)
                    .map(|o| o.target)
            });
            found = own.and_then(held).map(|(_, t)| t);
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
            return self.work_move(player, idx, target, point, clears);
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
            crate::build::CART_TASK => self.work_move(player, idx, target, point, clears),
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
    /// its speed `+0x740`) is told of a move (slot 0x828) and, in state 2 or of class 51, moves to
    /// the target or the point (slot 0x138); one that does not move does nothing.
    fn work_move(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        point: [f32; 3],
        clears: bool,
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
                self.move_point_order(player, idx, point)?;
            } else {
                self.move_to_object(player, idx, target, point)?;
            }
        }
        Ok(())
    }

    /// `HuntObject`'s unit part (`FUN_140c4e220`, [`crate::unit_ai::Effect::HuntObject`], after the
    /// unit's update): its stopAction (slot 0x3c0: the action list cleared,
    /// [`Turns::clear_actions`], and in state 2 the standing sprite), then its hunt command (object
    /// slot 0x360, `FUN_1412151f0`, forced): with no action left slot 0x820 finds the task as the
    /// work command does ([`crate::masters::Masters::work_master`] with any filter; the master
    /// switch, slot 0x90) and a Hunt task installs a new Hunt ([`Turns::start_hunt`] without the
    /// unit AI's notify: `HuntObject` set the AI itself). A target the world does not hold, no task,
    /// another task's action or a task not enabled would fail the command, which the unit AI took
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
        let (tp, t) = self.object(target).ok_or(Unsupported(
            "a hunt command on an object the world does not hold",
        ))?;
        let point = t.position;
        let _ = tp;
        let unit = &self.players[player].objects[idx];
        let found = {
            let masters = self
                .masters
                .get(player)
                .ok_or(Unsupported("a hunt command: no master table"))?;
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
                .work_master(&actor, &t, &general, -1, &world)?
                .map(|(with, task)| (with, *task))
        };
        let (with, task) = found.ok_or(Unsupported("a hunt command without a task"))?;
        if task.action_type != HUNT {
            return Err(Unsupported("a hunt command whose task is not a Hunt"));
        }
        if self.players[player].objects[idx].life.master_id != with {
            self.change_master(player, idx, with)?;
        }
        if !enabled(&self.players[player].attributes, task.enablement) {
            return Err(Unsupported("a hunt command whose task is not enabled"));
        }
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
    /// unit's update): slot 0x820 (`FUN_14106c6a0`) finds the task as the work command does
    /// ([`crate::masters::Masters::work_master`] with any filter; the master switch, slot 0x90);
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
        let unit = &self.players[player].objects[idx];
        let found = {
            let masters = self
                .masters
                .get(player)
                .ok_or(Unsupported("a gather command: no master table"))?;
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
                .work_master(&actor, &t, &general, -1, &world)?
                .map(|(with, task)| (with, *task))
        };
        let (with, task) = found.ok_or(Unsupported("a gather command without a task"))?;
        if self.players[player].objects[idx].life.master_id != with {
            self.change_master(player, idx, with)?;
        }
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
            if old.added {
                self.leave_builders(old.target)?;
            }
            if old.target != -1 {
                let by = self.releaser(player, idx);
                self.release(old.target, by)?;
            }
            self.take(target);
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
        let site = self.players[fp].objects[fi].construction.as_mut();
        let r = build
            .as_mut()
            .expect("found above")
            .retake(gap, mover.as_mut(), site, computer);
        let unit = &mut self.players[player].objects[idx];
        (unit.build, unit.mover) = (build, mover);
        r?;
        Ok(class)
    }

    /// `FUN_140aaddb0` and the builder count: the foundation `id` a Build counted its builder
    /// among (an annex of type 80 its main building, `+0xbb8`), when the world still holds it,
    /// counts one builder less (`+0x9e8`).
    fn leave_builders(&mut self, id: i32) -> Result<(), Unsupported> {
        let Some((p, f)) = self.object(id) else {
            return Ok(());
        };
        let kind = self.master_of(p, f.life.master_id)?.kind;
        let site = if kind == BUILDING {
            f.annex_of.unwrap_or(id)
        } else {
            id
        };
        let Some(Place::Live(sp, si)) = self.place(site) else {
            return Err(Unsupported("a Build's foundation that sleeps"));
        };
        let c = self.players[sp].objects[si]
            .construction
            .as_mut()
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
            if let Some(old) = old {
                let by = self.releaser(player, idx);
                self.release(old, by)?;
            }
            self.take(target);
        }
        Ok(())
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
        let farm_stops = self.farm_stops;
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
        let mut gather = crate::gather::GatherAction::new(task, &resource, self.farm_stops, rate)?;
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
        let carry_rule = self.carry_rule;
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
        let carry_rule = self.carry_rule;
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
                owner.objects.push(woken);
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

    /// The action list's clearing (`FUN_140a98620`): each action's teardown. A Build action takes
    /// its builder out of the foundation's builders (`FUN_140aadbf0`); a `MoveTo`'s teardown stops
    /// the unit (slot 0x5d0); the Build and the Hunt release their targets (slot 0x180,
    /// [`Turns::release`]). A building's make action is in its production's own list, which
    /// stays.
    pub(crate) fn clear_actions(&mut self, player: usize, idx: usize) -> Result<(), Unsupported> {
        let by = self.releaser(player, idx);
        let objects = &mut self.players[player].objects;
        let unit = &mut objects[idx];
        if unit.artifact.is_some() || unit.fly.is_some() || unit.enter.is_some() {
            return Err(Unsupported(
                "clearing an action list the slice does not model",
            ));
        }
        let build = unit.build.take();
        let explore = unit.explore.take();
        let mut mover = unit.mover.as_mut();
        // an Explore's teardown (0x140a87dd0): its MoveTo's stops the unit
        if explore.is_some_and(|e| e.move_to.is_some())
            && let Some(m) = mover.as_deref_mut()
        {
            m.stop()?;
        }
        if let Some(b) = build
            && b.move_to.is_some()
            && let Some(m) = mover.as_deref_mut()
        {
            m.stop()?;
        }
        let hunt = unit.hunt.take();
        if let Some(mut h) = hunt {
            h.tear_down(mover.as_deref_mut())?;
        }
        let gather = unit.gather.take();
        if let Some(mut g) = gather {
            g.tear_down(mover.as_deref_mut())?;
        }
        // its own Attack's teardown: the MoveTo's stops the unit; it lets its target go
        let attack = unit.attack.take();
        if let Some(mut a) = attack {
            a.tear_down(mover.as_deref_mut())?;
        }
        if unit.move_to.take().is_some()
            && let Some(m) = mover
        {
            m.stop()?;
        }
        if let Some(b) = build {
            if b.added {
                let site = objects
                    .iter_mut()
                    .find(|u| u.id == b.target)
                    .and_then(|u| u.construction.as_mut())
                    .ok_or(Unsupported(
                        "a builder's foundation the slice does not hold",
                    ))?;
                site.builders = site.builders.saturating_sub(1);
            }
            self.release(b.target, by)?;
        }
        if let Some(h) = hunt {
            self.release(h.target, by)?;
        }
        // the base teardown (FUN_140a73fc0) lets the target, then the second target go
        if let Some(g) = gather {
            for t in [g.target, g.carry_to] {
                if t != -1 {
                    self.release(t, by)?;
                }
            }
        }
        if let Some(a) = attack {
            for t in [a.target, a.target2] {
                if t != -1 {
                    self.release(t, by)?;
                }
            }
        }
        Ok(())
    }

    /// `TRANSFORM_OBJECT` (`RGE_Command::do_command` case 0x29): the object `id`, the player's
    /// (another player's is refused, the cheat flag `+0xb12` aside), of the trophy master
    /// (1649) takes one of the trophies 1650 to 1653 the player has (`FUN_141659d20`): slot 0x90
    /// with 0, the building's (`FUN_1411e6d90`): [`Turns::change_master`], then a built
    /// building's build points (`+0xc50`) take the new master's build time.
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
        self.change_master(p, idx, master)?;
        let build_time = self.master_of(p, master)?.build_time;
        let unit = &mut self.players[p].objects[idx];
        if unit.life.state == 2
            && let Some(c) = unit.construction.as_mut()
        {
            c.progress = f32::from(build_time);
        }
        Ok(())
    }

    /// The player's gate toggle (`FUN_14162f8c0`, from its slots 0x340 and 0x338): a gate of its
    /// player's masters, without masters of its own ([`crate::turn::Gate::other_masters`]),
    /// changes to its master's `transform_unit` (slot 0x90, the building's `FUN_1411e6d90`:
    /// [`Turns::change_master`], then a built gate's build points take the new master's build
    /// time). A gate that shut (the new master's height above 0) kills the units in it (slot 0xd8
    /// on those its count finds, `FUN_141211340`: not modelled) and takes the world time plus 1
    /// as its last move time (`+0x870`).
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
        self.change_master(player, idx, new)?;
        let next = self.master_of(player, new)?;
        let (build_time, height) = (next.build_time, next.height);
        let now = self.world_time;
        let unit = &mut self.players[player].objects[idx];
        if unit.life.state == 2
            && let Some(c) = unit.construction.as_mut()
        {
            c.progress = f32::from(build_time);
        }
        if 0.0 < height {
            let id = unit.id;
            let annexes = crate::turn::annexes_of(id, &self.players[player].objects);
            let (units, _) = crate::turn::gate_units(&self.spatial, id, &annexes)?;
            for u in units {
                let killed = self.object(u).is_some_and(|(owner, o)| {
                    self.master_of(owner, o.life.master_id)
                        .is_ok_and(|m| m.class != 42 && (m.class != 58 || o.life.amount <= 0.0))
                });
                if killed {
                    return Err(Unsupported("a gate shutting on a unit (slot 0xd8)"));
                }
            }
            self.players[player].objects[idx]
                .mover
                .as_mut()
                .ok_or(Unsupported("a shut gate without its moving part"))?
                .last_move = now.wrapping_add(1);
        }
        Ok(())
    }

    /// Slot 0x90 (`FUN_141225b70` with 0): the unit changes its master. Its action list is
    /// cleared; in a state below 3 (`FUN_1411a6a70`) the old master's storage leaves its player's
    /// attributes and the new one's comes in (`FUN_1411a56c0`, flag 2), its hit points keep their
    /// share of the new master's (whole, at least 1), its sight goes and the new master's comes
    /// (slots 0x1d8, 0x1d0), its place and obstruction record take the new master's
    /// (`FUN_141055b40`, `FUN_14118c540`) and it moves with the new master's values; its powerups
    /// go (`FUN_1412066b0`); it takes the new master's standing sprite (slot 0x68). Its
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
            let owner = &mut self.players[player];
            for &(kind, amount, flag) in &next.storage {
                if kind >= 0 && flag == 2 {
                    owner.add_attribute(kind, amount)?;
                }
            }
        }
        let unit = &mut self.players[player].objects[idx];
        unit.work_rate_powerup = 0.0;
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
        let scout = unit(9185, true, [42.047_367, 94.338_486, 0.0]);
        let sheep = unit(9529, false, [45.5, 93.5, 0.0]);
        assert!(!splits_together(&scout, &sheep, f, false, &mut none).unwrap());
        // nearer than 20 tiles the planner's path decides: its goal reached (1) within the
        // influence distance
        let other = unit(9186, true, [44.0, 94.0, 0.0]);
        let mut short = |_: &Splittable, _: &Splittable| Ok(Some((1, 2.5)));
        assert!(splits_together(&scout, &other, f, false, &mut short).unwrap());
        let mut long = |_: &Splittable, _: &Splittable| Ok(Some((1, 3.5)));
        assert!(!splits_together(&scout, &other, f, false, &mut long).unwrap());
        let mut nearest = |_: &Splittable, _: &Splittable| Ok(Some((2, 2.5)));
        assert!(!splits_together(&scout, &other, f, false, &mut nearest).unwrap());
        let far = unit(9187, true, [80.0, 94.0, 0.0]);
        assert!(!splits_together(&scout, &far, f, false, &mut none).unwrap());
        let wide = crate::turn::Formations {
            influence: 40.0,
            ..f
        };
        assert!(splits_together(&scout, &far, wide, false, &mut none).unwrap());
        // a member must stand in state 2 or more
        let born = Splittable { state: 0, ..far };
        assert!(!splits_together(&scout, &born, wide, true, &mut none).unwrap());
    }

    #[test]
    fn a_point_off_the_map_stays_inside_its_edge() {
        let c = [10.0, 10.0];
        assert_eq!(
            offset_point([5.0, 15.0], c, [2.0, 118.0], [120, 120]),
            [1.0, 119.0]
        );
    }
}
