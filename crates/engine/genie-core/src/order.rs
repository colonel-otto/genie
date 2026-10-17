//! INTERACT's order to a single unit (`0x140e68b90`, its loop over the units without a group) and
//! the work command it gives (object slot 37, `FUN_14106ff70`, through slot 0x128 `FUN_141211b80`
//! and slot 0x7e8 `FUN_141227190`): the unit takes the master and the task the task finder picks
//! for the target ([`crate::tasks`]), and the task's action replaces its action list. The slice
//! builds the Hunt action ([`crate::hunt`]); other actions, orders to points and orders to several
//! units are not modelled yet.

use crate::attack::{Arms, Engagement, gaps};
use crate::hunt::{HUNT, HuntAction, HuntTask, Quarry};
use crate::masters::Master;
use crate::rng::Stream;
use crate::tasks::{Actor, Diplomacy, TaskTarget, enabled};
use crate::turn::{Turns, Unit, World};
use crate::unit_ai::{HUNT_ACTION, HUNT_ORDER, Unsupported};

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

    fn object(&self, id: i32) -> Option<(usize, &Unit)> {
        match self.place(id)? {
            Place::Live(p, i) => Some((p, &self.players[p].objects[i])),
            Place::Sleeping(p, i) => Some((p, &self.players[p].sleeping[i])),
        }
    }

    fn master_of(&self, player: usize, master: i16) -> Result<&Master, Unsupported> {
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

    /// INTERACT (`0x140e68b90`) to its units, after their unit AIs reset: one unit takes the order
    /// itself (the loop of `LAB_140e69a3c`); several units, which the handler gives places around
    /// the target by their offsets, are not modelled.
    pub(crate) fn interact(
        &mut self,
        player: usize,
        units: &[i32],
        order: &Interact,
    ) -> Result<(), Unsupported> {
        let id = match units {
            [] => return Ok(()),
            &[id] => id,
            _ => return Err(Unsupported("an order to several units (their offsets)")),
        };
        let Some(Place::Live(p, idx)) = self.place(id) else {
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
        let unit = &mut self.players[player].objects[idx];
        // FUN_141197da0 empties the unit's ignore list (`+0x678`), which the slice does not hold;
        // slot 0x2a0 (`FUN_14115d450`) with 1: it waits to move
        unit.mover
            .as_mut()
            .ok_or(Unsupported("an ordered unit without movement state"))?
            .waiting = 1;
        if unit.ai.is_some() && (order.queued || !order.direct) {
            return Err(Unsupported(
                "an order its unit AI takes (its slots 0x28, 0x30)",
            ));
        }
        // slot 0x128 (FUN_141211b80): `+0xa14` and the unit AI's `+0x18`, which the slice does not
        // hold; FUN_14106c590 passes the order on unless the target is the unit itself
        if order.target == id {
            return Ok(());
        }
        self.order_work(player, idx, order)
    }

    /// MOVE (`FUN_140e67800`) to its units, after their unit AIs reset: one unit takes the order
    /// itself (the loop of `LAB_140e6838c`); several units, which the handler gives places around
    /// the point by their offsets, are not modelled. An object that does not move ignores it.
    pub(crate) fn move_order(
        &mut self,
        player: usize,
        units: &[i32],
        order: &MoveOrder,
    ) -> Result<(), Unsupported> {
        let id = match units {
            [] => return Ok(()),
            &[id] => id,
            _ => return Err(Unsupported("a move of several units (their offsets)")),
        };
        let Some(Place::Live(p, idx)) = self.place(id) else {
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
        let unit = &mut self.players[player].objects[idx];
        // FUN_141197da0, then slot 0x2a0 with 1: it waits to move
        unit.mover
            .as_mut()
            .ok_or(Unsupported("a moved unit without movement state"))?
            .waiting = 1;
        if order.queued || (unit.ai.is_some() && !order.direct) {
            return Err(Unsupported("a move its unit AI takes (its slot 0x30)"));
        }
        // slot 0x138 (FUN_141216f10): a villager's town-bell flag and the unit AI's `+0x18`
        // (neither held), then FUN_14106d340
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
    fn order_work(
        &mut self,
        player: usize,
        idx: usize,
        order: &Interact,
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
        // slot 0x7e8 with the order's point on the ground
        let point = [order.point[0], order.point[1], 0.0];
        self.work_command(
            player,
            idx,
            order.target,
            order.filter,
            order.carry_test,
            point,
        )?;
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
    fn work_command(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        filter: i16,
        carry_test: bool,
        point: [f32; 3],
    ) -> Result<(), Unsupported> {
        let unit = &self.players[player].objects[idx];
        if unit.id == target {
            return Ok(());
        }
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

    /// The Attack constructor's sprites and frames (`FUN_140a7c210` from Hunt's set_state 12): the
    /// task's proceeding sprite (else the master's attack sprite), the master's second attack
    /// sprite and standing sprite, missile, firing and interrupting frames.
    fn arms(&self, player: usize, idx: usize, sprites: [i16; 4]) -> Result<Arms, Unsupported> {
        let unit = &self.players[player].objects[idx];
        let master = self.master_of(player, unit.life.master_id)?;
        let g = |id: i16| (0 <= id).then(|| self.graphics.get(&id).copied()).flatten();
        Ok(Arms {
            sprite: g(sprites[1]).or(g(master.attack_sprite)),
            second_sprite: g(master.attack_sprite2),
            idle_sprite: g(master.standing_sprite),
            missile: master.missile,
            fire_frame: master.fire_frame,
            interrupt_frame: master.interrupt_frame,
        })
    }

    /// What an Attack of the unit reads of it and its target.
    fn engagement(
        &self,
        player: usize,
        idx: usize,
        target: i32,
    ) -> Result<Engagement, Unsupported> {
        let unit = &self.players[player].objects[idx];
        let master = self.master_of(player, unit.life.master_id)?;
        if matches!(master.charge_type, 6 | 7) {
            return Err(Unsupported(
                "an attacker whose charge adds to its range (slot 0x2f8)",
            ));
        }
        let me = self
            .spatial
            .get(unit.id)
            .ok_or(Unsupported("an attacker without a place"))?;
        let them = self
            .spatial
            .get_known(target)?
            .ok_or(Unsupported("an attack's target without a place"))?;
        let (gap_squared, gap) = gaps(me.position, me.radius, them.position, them.radius);
        let ai = unit.ai.as_ref();
        Ok(Engagement {
            gap_squared,
            gap,
            target_position: them.position,
            speed: unit.mover.as_ref().map_or(0.0, |m| m.stats.speed),
            range: master.weapon_range,
            min_range: master.min_range,
            can_move: ai.is_some_and(|ai| ai.current_order != -1 || ai.stance <= 1),
            stance: ai.map_or(0, |ai| ai.stance),
        })
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
    /// go (`FUN_1412066b0`). Its per-master counts, sprite, facet, class registry
    /// (`DAT_143f69170`) and particles are not held.
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
