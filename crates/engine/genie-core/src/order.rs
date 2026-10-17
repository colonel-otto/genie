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
use crate::turn::{Turns, Unit, World};
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
    pub direct: bool,
    pub human: bool,
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
const MASTER_84: i16 = 84;
const MASTER_1808: i16 = 1808;
const MASTER_1893: i16 = 1893;
const MASTER_1897: i16 = 1897;
/// A building's master type (`'P'`); types below 30 do not move.
const BUILDING: u8 = 80;
const MOVING: u8 = 30;
/// The task filter of garrisoning (3).
const GARRISON: i16 = 3;

/// Where an object is: its player and its index in the live or the sleeping list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Place {
    Live(usize, usize),
    Sleeping(usize, usize),
}

impl Turns {
    fn place(&self, id: i32) -> Option<Place> {
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
        let loose = self.loose_units(units)?;
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

    /// The order splitter (`FUN_140fd9dc0` -> `FUN_140fd9ae0` -> `FUN_140fd76d0`) on units that
    /// form no group: it sorts them by id (`FUN_140fd7280` with `LAB_140fd89b0`), takes at most
    /// 60, drops an id the next one repeats, and leaves out of any group a unit above type 69 whose
    /// master's formation category is 0 (villagers). A unit in a group (`+0x6d0`), one it may
    /// group and an order to more than 60 units are not modelled.
    fn loose_units(&self, units: &[i32]) -> Result<Vec<i32>, Unsupported> {
        if 60 < units.len() {
            return Err(Unsupported("an order to more than 60 units"));
        }
        let mut sorted = units.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        let mut loose = Vec::with_capacity(sorted.len());
        for id in sorted {
            let Some((p, u)) = self.object(id) else {
                continue;
            };
            let master = self.master_of(p, u.life.master_id)?;
            if master.kind <= 69 || master.formation_category != 0 {
                return Err(Unsupported("a unit the order splitter may group"));
            }
            loose.push(id);
        }
        Ok(loose)
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
        if p != player {
            return Err(Unsupported("an order to another player's unit"));
        }
        let master = self.master_of(player, self.players[player].objects[idx].life.master_id)?;
        let speed = master.moving.as_ref().map_or(0.0, |m| m.speed);
        if master.kind < MOVING || (master.class != CLASS_54 && speed <= 0.0) {
            return Err(Unsupported("an order to an object that does not move"));
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
            issuer: player as i32,
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
        ai.queue_order(state, queued, false, false)?;
        if !order.queued && !order.direct {
            return Err(Unsupported(
                "the readiness pass after an order neither queued nor direct (FUN_140c42b60)",
            ));
        }
        Ok(())
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

    /// MOVE (`FUN_140e67800`) to its units, after their unit AIs reset: one unit takes the order
    /// (the loop of `LAB_140e6838c`); several units, which the handler sets out in a grid around
    /// the point, are not modelled. An object that does not move ignores an order not queued. The
    /// unit's ignore list empties and it waits to move; an order that is queued, or neither
    /// direct nor to a unit AI with every queued order ready (`FUN_140c4dfa0`), goes to its unit
    /// AI's queue as order `0x2c6` at the point (slot 0x30 -> 0x28: range 0, priority 100, the
    /// point shown, ready when direct); a unit without a unit AI then does nothing. Otherwise the
    /// unit takes the move itself (slot 0x138).
    pub(crate) fn move_order(
        &mut self,
        player: usize,
        units: &[i32],
        order: &MoveOrder,
    ) -> Result<(), Unsupported> {
        let id = match units {
            [] => return Ok(()),
            &[id] => id,
            _ => return Err(Unsupported("a move of several units (their grid)")),
        };
        let Some(place) = self.place(id) else {
            return Ok(());
        };
        let Place::Live(p, idx) = place else {
            return Err(Unsupported("a move of a unit the slice does not hold live"));
        };
        if p != player {
            return Err(Unsupported("a move of another player's unit"));
        }
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
                issuer: player as i32,
                order: MOVE_ORDER,
                priority: 100,
                target: if target.is_some() { order.target } else { -1 },
                target_owner,
                point: [order.point[0], order.point[1], 0.0],
                range: 0.0,
                shown: order.point,
                ready: order.direct,
                target_type,
            };
            ai.queue_order(state, queued, false, false)?;
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
        // a unit with a unit AI (FUN_140c5dc10 clears its notifies with the world time) lets its
        // current action's slot 8 take the point (FUN_140a97730) unless it stops after a kill
        // (FUN_140c41c10): an Artifact action's does (FUN_140aaca30), a MoveTo's
        // (`FUN_140a78490`) never
        let point = [order.point[0], order.point[1], 0.0];
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
            (true, Some(t)) if t != crate::move_to::MOVE_TO => {
                return Err(Unsupported("a move of a unit with an action (its slot 8)"));
            }
            _ => false,
        };
        if !taken {
            // the MoveTo to the point (FUN_140aa0d30, range 0) and its install (slot 0x808)
            self.clear_actions(player, idx)?;
            let unit = &mut self.players[player].objects[idx];
            unit.move_to = Some(crate::move_to::MoveToAction {
                // FUN_140aa2100: no target, so far from it: state 3 for a unit with speed
                state: crate::move_to::MoveTo::Start,
                goal: Some(point),
                range: 0.0,
            });
        }
        // the order handler's notify: a unit that took the move as a MoveTo gets the move order
        // and action, its target forgotten (FUN_140c4dcb0); every unit then the point
        // (FUN_140c64be0); slot 0x2d8's walk and show targets are not held
        let unit = &mut self.players[player].objects[idx];
        if let Some(ai) = unit.ai.as_mut() {
            ai.notify_move(!taken, None, order.point, self.streams.get(Stream::Ai));
        }
        Ok(())
    }

    /// Slot 0x7e8 (`FUN_141227190`) of a unit on a target object: a unit in state 3 or later
    /// ignores it; against an unallied target the unit's least range must not exceed the gap
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
        let (tp, _) = self.object(order.target).ok_or(Unsupported(
            "an order to a point or to an object the slice does not hold",
        ))?;
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
        if master.class == LIVESTOCK {
            return Err(Unsupported("an order to livestock"));
        }
        if (master.class == VILLAGER || master.id == MASTER_1808) && order.filter == GARRISON {
            return Err(Unsupported(
                "a villager's order to garrison (its town-bell record)",
            ));
        }
        // FUN_141206a20's town-bell record (`+0x9ac`..) and the flag `+0x9dc` are not held
        self.work_command(player, idx, order)?;
        let unit = &self.players[player].objects[idx];
        let master = self.master_of(player, unit.life.master_id)?;
        let target = self.task_target(order.target, 0)?;
        if master.class == VILLAGER
            && target.master.kind == BUILDING
            && matches!(target.master.id, MASTER_1893 | MASTER_1897)
        {
            return Err(Unsupported("a villager's order on master 1893 or 1897"));
        }
        let unit = &mut self.players[player].objects[idx];
        if unit.my_pasture != -1 {
            return Err(Unsupported("a unit that leaves its pasture"));
        }
        Ok(())
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
        let found = {
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
            return Err(Unsupported(
                "a work command without a task (the unit's move)",
            ));
        };
        if with != self.players[player].objects[idx].life.master_id {
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
        if matches!(top, Some(9 | 0x7e)) || (top == Some(0x6b) && task.action_type == 3) {
            return Err(Unsupported(
                "a work command on a unit attacking, gathering or 0x7e",
            ));
        }
        if kept {
            return Ok(());
        }
        match task.action_type {
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
            ),
            _ => Err(Unsupported(
                "the action factory for a task the slice does not model",
            )),
        }
    }

    /// The hunt's slot 9 (`0x140ac97c0`) on the work command's new target: a new target is
    /// released for the old one (slot 0x68, `FUN_140ac7980`, through `FUN_140a7a230`) and the hunt
    /// starts over.
    fn retarget_hunt(&mut self, player: usize, idx: usize, target: i32) -> Result<(), Unsupported> {
        let quarry = self.quarry(player, target)?;
        let engagement = self.engagement(player, idx, target)?;
        let sprites = self.players[player].objects[idx]
            .hunt
            .map_or([-1; 4], |h| h.task.sprites);
        let arms = self.arms(player, idx, sprites)?;
        let unit = &mut self.players[player].objects[idx];
        let old = unit.hunt.map(|h| h.target);
        let (Some(hunt), Some(mover)) = (unit.hunt.as_mut(), unit.mover.as_mut()) else {
            return Err(Unsupported("a hunter without its hunt or movement state"));
        };
        let unit_player = player;
        if hunt.retarget(&quarry, unit_player, &engagement, arms, mover)? {
            if let Some(old) = old {
                self.release(old)?;
            }
            self.take(target);
        }
        Ok(())
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
        })
    }

    /// [`crate::attack::arms`] of the unit at `idx`.
    fn arms(&self, player: usize, idx: usize, sprites: [i16; 4]) -> Result<Arms, Unsupported> {
        let unit = &self.players[player].objects[idx];
        let master = self.master_of(player, unit.life.master_id)?;
        Ok(crate::attack::arms(master, &self.graphics, sprites))
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
        crate::attack::engagement(unit.ai.as_ref(), speed, master, me, them)
    }

    /// The Hunt from the factory (`FUN_140aca4b0` case 0x6e): the action takes its target (slot
    /// 0x178); the unit AI's notify (`FUN_14106e4e0`) sets the hunt order; the install
    /// (`FUN_14106e990`) clears the action list and starts the hunt.
    fn start_hunt(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        task: HuntTask,
        point: [f32; 3],
        clears: bool,
    ) -> Result<(), Unsupported> {
        let quarry = self.quarry(player, target)?;
        self.take(target);
        let unit = &mut self.players[player].objects[idx];
        if let Some(ai) = unit.ai.as_mut() {
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
        let arms = self.arms(player, idx, task.sprites)?;
        let computer = self.players[player].computer;
        let carry_rule = self.carry_rule;
        let unit = &mut self.players[player].objects[idx];
        let mover = unit
            .mover
            .as_mut()
            .ok_or(Unsupported("a hunter without movement state"))?;
        let mut held = (unit.life.attribute_type, unit.life.amount);
        let hunt = HuntAction::start(
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
        unit.hunt = Some(hunt);
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
    pub(crate) fn release(&mut self, id: i32) -> Result<(), Unsupported> {
        let Some(Place::Live(p, i)) = self.place(id) else {
            return Ok(());
        };
        let kind = {
            let u = &self.players[p].objects[i];
            self.master_of(p, u.life.master_id)?.kind
        };
        let u = &mut self.players[p].objects[i];
        if 0 < u.workers {
            u.workers -= 1;
        }
        let alive = (u.life.state == 2 && 0.0 <= u.life.hp) || u.life.state == 6;
        if kind < MOVING && u.workers == 0 && alive {
            u.life.goto_sleep = true;
        }
        Ok(())
    }

    /// The action list's clearing (`FUN_140a98620`): each action's teardown. A Build action takes
    /// its builder out of the foundation's builders (`FUN_140aadbf0`); a MoveTo's teardown stops
    /// the unit (slot 0x5d0); the Build and the Hunt release their targets (slot 0x180,
    /// [`Turns::release`]).
    pub(crate) fn clear_actions(&mut self, player: usize, idx: usize) -> Result<(), Unsupported> {
        let objects = &mut self.players[player].objects;
        let unit = &mut objects[idx];
        if unit.artifact.is_some() || unit.fly.is_some() || unit.production.is_some() {
            return Err(Unsupported(
                "clearing an action list the slice does not model",
            ));
        }
        let build = unit.build.take();
        let mut mover = unit.mover.as_mut();
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
            self.release(b.target)?;
        }
        if let Some(h) = hunt {
            self.release(h.target)?;
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
    fn change_master(&mut self, player: usize, idx: usize, new: i16) -> Result<(), Unsupported> {
        self.clear_actions(player, idx)?;
        let unit = &self.players[player].objects[idx];
        let old = self.master_of(player, unit.life.master_id)?.clone();
        let next = self.master_of(player, new)?.clone();
        if unit.life.state < 3 {
            if unit.life.inside {
                return Err(Unsupported("a master change of an object inside another"));
            }
            if old.damage_sprites || next.damage_sprites {
                return Err(Unsupported(
                    "a master change with damage sprites (FUN_1411859a0)",
                ));
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
            unit.life.hp = f32::from(hp as i32 as i16);
            unit.life.master_id = new;
            let moving = unit.mover.as_ref().is_some_and(|m| m.is_moving());
            if let Some(m) = unit.mover.as_mut() {
                m.stats = next
                    .moving
                    .clone()
                    .ok_or(Unsupported("a moving unit's new master does not move"))?;
            }
            let id = unit.id;
            if let Some(mut placed) = self.spatial.get(id).cloned() {
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
                self.spatial.insert(id, placed.clone());
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
                    crate::sprite::ActiveSprite::start(g, rng);
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
    /// `DAT_14330eb8c`.
    const EDGE: f32 = crate::image::order::EDGE;
    let edge = |v: f32, size: i32| {
        if v < 0.0 {
            EDGE
        } else if size as f32 <= v {
            size as f32 - EDGE
        } else {
            v
        }
    };
    [
        edge(position[0] - centroid[0] + point[0], map[0]),
        edge(position[1] - centroid[1] + point[1], map[1]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// MarvinBlue/426048731's INTERACT of 927: player 1's builders 9171 and 9173 stand side by
    /// side; the queued orders of the save after 927 hold their points around the order's.
    #[test]
    fn units_of_an_order_take_their_offsets_from_their_centroid() {
        let (a, b) = ([104.32466, 39.299885], [103.67509, 39.299885]);
        let c = centroid(&[a, b]).unwrap();
        let point = [108.37596, 43.91338];
        assert_eq!(
            offset_point(a, c, point, [120, 120]),
            [108.700745, 43.91338]
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

    #[test]
    fn a_point_off_the_map_stays_inside_its_edge() {
        let c = [10.0, 10.0];
        assert_eq!(
            offset_point([5.0, 15.0], c, [2.0, 118.0], [120, 120]),
            [1.0, 119.0]
        );
    }
}
