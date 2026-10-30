//! A unit's move, to a point or aside, a work order, its reach and a point's move order.

use crate::GenieError;
use crate::combat::attack::gaps;
use crate::map::Point;
use crate::math::rng::Stream;
use crate::objects::ObjectId;
use crate::objects::masters::Master;
use crate::objects::masters::class;
use crate::objects::masters::ids::PASTURE;
use crate::objects::masters::ids::PASTURE_MANGROVE;
use crate::objects::masters::master_type::BUILDING;
use crate::objects::masters::master_type::MOVING;
use crate::orders::{MoveOrder, Place, TOO_CLOSE, Work, ignore_unit, packs, readiness, work_reach};
use crate::unit_ai::ActionKind;
use crate::unit_ai::OrderKind;
use crate::unit_ai::task_finder::Diplomacy;
use crate::unit_ai::task_finder::world::TaskWorld;
use crate::world::World;

impl World {
    /// One unit of MOVE: an object that does not move ignores an order not queued. The unit's
    /// ignore list empties (`FUN_141197da0`), it waits to move (slot 0x2a0 with 1) and lists the
    /// order's `others` (`FUN_14118c790`). An order that is queued, or neither direct nor to a
    /// unit AI with every queued order ready (`FUN_140c4dfa0`), goes to its unit AI's queue as
    /// order `0x2c6` at its point (slot 0x30, or in a grid slot 0x28: range 0, priority 100, the
    /// order's point shown, ready when direct); a unit without a unit AI then does nothing.
    /// Otherwise the unit takes the move itself (slot 0x138) to the point, or to its grid `cell`.
    pub(super) fn move_unit(
        &mut self,
        player: usize,
        id: i32,
        others: &[i32],
        cell: Option<[f32; 2]>,
        order: &MoveOrder,
    ) -> Result<(), GenieError> {
        let point2 = cell.unwrap_or_else(|| order.point.xy());
        let Some(place) = self.place(id) else {
            return Ok(());
        };
        let Place::Live(p, idx) = place else {
            return Err(GenieError::unsupported(
                "a move of a unit the slice does not hold live",
            ));
        };
        // the command's check (`FUN_140e553f0`, [`World::checked_units`]) dropped another
        // player's unit (its id -1) unless the game setting `+0x1074` lets players order each
        // other's units ([`crate::world::settings::Settings::cheats_enabled`]): such a unit takes the
        // move as its own player's, the issuer only queuing it
        let issuer = player;
        let player = p;
        let master = self.players[player].objects[idx].master(&self.masters, player)?;
        let speed = master.moving.as_ref().map_or(0.0, |m| m.speed);
        if master.kind < MOVING || speed <= 0.0 {
            if order.queued {
                return Err(GenieError::unsupported(
                    "a queued move of an object that does not move",
                ));
            }
            return Ok(());
        }
        let target = self
            .object(ObjectId::field(order.target))
            .map(|(tp, t)| (t.life.master_id, tp));
        let unit = &mut self.players[player].objects[idx];
        // FUN_141197da0, then slot 0x2a0 with 1: it waits to move
        let mover = unit.body.mover_mut().ok_or(GenieError::unsupported(
            "a moved unit without movement state",
        ))?;
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
                order: OrderKind::Move,
                priority: 100,
                target: if target.is_some() {
                    ObjectId::field(order.target)
                } else {
                    -1
                },
                target_owner,
                point: [point2[0], point2[1], 0.0],
                range: 0.0,
                shown: order.point.xy(),
                ready: order.direct,
                target_type,
            };
            let _ = ai.queue_order(state, queued, false, false)?;
            if !order.queued && !order.direct {
                readiness(&mut ai.orders, true);
                // the pass's tail: the unit's stop (its slot 0x148)
                self.object_stop(player, idx)?;
            }
            return Ok(());
        }
        // slot 0x138 (FUN_141216f10): a villager's town-bell flag (not held) and, `+0xa14` being
        // 1, the unit AI's order queue empties (FUN_140c5e010); then FUN_14106d340
        if let Some(ai) = unit.ai.as_mut() {
            ai.orders.clear();
        }
        if ObjectId::field(order.target) == id {
            return Ok(());
        }
        if ObjectId::field(order.target) != -1 {
            return Err(GenieError::unsupported("a move that follows an object"));
        }
        let order = &MoveOrder {
            point: Point::new(point2[0], point2[1]),
            ..*order
        };
        let taken = self.move_to_point(player, idx, [order.point.x(), order.point.y(), 0.0])?;
        // the order handler's notify: a unit that took the move as a MoveTo gets the move order
        // and action, its target forgotten (FUN_140c4dcb0); every unit then the point
        // (FUN_140c64be0); in a grid every unit gets the order, the action and its cell, its
        // target kept; slot 0x2d8's walk and show targets are not held
        let unit = &mut self.players[player].objects[idx];
        if let Some(ai) = unit.ai.as_mut() {
            let ai_stream = self.streams.get(Stream::Ai);
            match cell {
                None => ai.notify_move(!taken, None, order.point.xy(), ai_stream),
                Some(c) => ai.notify_grid_move(c, ai_stream),
            }
        }
        Ok(())
    }

    /// `FUN_14106d340` on a move of the unit at `idx` to `point`: a unit with a unit AI
    /// (`FUN_140c5dc10` clears its notifies with the world time) lets its head action take the
    /// point ([`World::head_takes_move`]) unless it stops after a kill (`FUN_140c41c10`); else a
    /// `MoveTo` to the point replaces its actions. True when the action took it.
    pub(crate) fn move_to_point(
        &mut self,
        player: usize,
        idx: usize,
        point: [f32; 3],
    ) -> Result<bool, GenieError> {
        // a unit with a unit AI (FUN_140c5dc10 clears its notifies with the world time) lets its
        // head action's slot 0x40 take the point unless it stops after a kill (FUN_140c41c10)
        let world_time = self.world_time;
        let class = self.players[player].objects[idx]
            .master(&self.masters, player)?
            .class;
        let unit = &mut self.players[player].objects[idx];
        let place = ([unit.position()[0], unit.position()[1]], class);
        if unit.ai.is_some() {
            // a completion's stop (slot 0xb0 with 1): the unit's stopAction at once
            self.run_ai_calls((player, idx), |ai, side, rng| {
                ai.clear_notifies_at(world_time, place, side, rng)
            })?;
        }
        let unit = &mut self.players[player].objects[idx];
        let stop_after_kill = unit.ai.as_ref().is_some_and(|ai| ai.stop_after_kill);
        let taken = unit.ai.is_some()
            && !stop_after_kill
            && self.head_takes_move(player, idx, None, point)?;
        if !taken {
            // the MoveTo to the point (FUN_140aa0d30: range 0, the walking sprite) and its
            // install (slot 0x808: the MoveTo's slot 0x70, FUN_140a78050, does nothing; the list
            // clears, then the start)
            self.clear_actions(player, idx)?;
            let sprite = self.point_move_sprite(player, idx)?;
            let unit = &mut self.players[player].objects[idx];
            let move_to = crate::movement::move_to::MoveToAction::to_point(
                point,
                0.0,
                sprite,
                unit.body.mover_mut(),
            );
            if move_to.shows_standing() {
                unit.shows_standing_sprite(
                    self.masters.get(player),
                    &self.graphics,
                    self.streams.get(Stream::Sprite),
                )?;
            }
            unit.actions.set_move_to(Some(move_to));
        }
        Ok(taken)
    }

    /// The sprite (`+0x58`) of the unit at `idx`'s `MoveTo` to a point whose maker gives none
    /// ([`point_sprite`] with the unit's terrain look; a trade cart's goods are not held, so it
    /// carries none).
    ///
    /// [`point_sprite`]: crate::movement::move_to::point_sprite
    pub(crate) fn point_move_sprite(
        &self,
        player: usize,
        idx: usize,
    ) -> Result<Option<crate::objects::sprite::Graphic>, GenieError> {
        let unit = &self.players[player].objects[idx];
        let master = unit.master(&self.masters, player)?;
        let look = unit.abilities.look.as_ref();
        Ok(crate::movement::move_to::point_sprite(master, look, false)
            .and_then(|id| self.graphics.get(&id).copied()))
    }

    /// The unit's slot 0x340 from its unit AI's move off a footprint (slot 0x148,
    /// [`crate::objects::unit::Unit::ai_move_with`]); then what its actions let go: a counted builder
    /// leaves its building ([`World::leave_builders`]), the targets go ([`World::release`]).
    pub(crate) fn move_aside(
        &mut self,
        player: usize,
        idx: usize,
        m: crate::unit_ai::AiMove,
    ) -> Result<(), GenieError> {
        let unit = &self.players[player].objects[idx];
        let class = unit.master(&self.masters, player)?.class;
        let by = self.releaser(player, idx);
        let (mut releases, mut builder_leaves) = (Vec::new(), Vec::new());
        let sails = self.players[player].objects[idx].ai_move_with(
            m,
            (class, player),
            (&mut releases, &mut builder_leaves, by),
            (
                self.masters.get(player),
                &self.graphics,
                self.streams.get(Stream::Sprite),
            ),
        )?;
        if sails {
            self.transport_move(player, idx, m.point)?;
        }
        for id in builder_leaves {
            self.leave_builders(id)?;
        }
        for (id, by) in releases {
            self.release(id, by)?;
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
    /// town-bell flag (`FUN_1412067f0`); then the work command. A villager on a pasture (1893,
    /// 1897) whose livestock's workers (slot 0x780, [`World::livestock_workers`]) are below its
    /// owner's [`crate::gathering::pasture::PASTURE_WORKERS`] (COMISS, JBE) works the livestock
    /// [`World::pasture_pick`] gives at its place with the order's flags (slot 0x7e8 again) and
    /// then tends the pasture; none given ends the order there; a full pasture has its unit AI
    /// take a search (slot 0x38: 0x202 with the hunt action, class 49 and −1). Any other unit
    /// leaves the pasture it tends ([`World::tend_pasture`], `FUN_141189ca0`).
    pub(crate) fn order_work(
        &mut self,
        player: usize,
        idx: usize,
        order: &Work,
    ) -> Result<(), GenieError> {
        let unit = &self.players[player].objects[idx];
        let master = unit.master(&self.masters, player)?;
        // the class-54 test of its top action comes first in the game, but either way the order
        // is ignored
        if 2 < unit.life.state {
            return Ok(());
        }
        let class = master.class;
        let target = self.object(order.target).map(|(tp, _)| tp);
        let reach = self.order_reach(player, unit.id, order, target, master)?;
        let packs = packs(
            class,
            reach,
            target.is_none(),
            self.players[player].computer,
        );
        // a class 54 under an action the slice does not hold may be under its Pack or Unpack
        if packs || (class == class::CLASS_54 && unit.unmodelled_action.is_some()) {
            return Err(GenieError::unsupported(
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
            // FUN_141189ca0 with −1: the unit leaves the pasture it tends
            return self.tend_pasture(player, idx, -1);
        }
        let master = self.players[player].objects[idx].master(&self.masters, player)?;
        // livestock (class 58) takes the enter command (slot 0x3a0); where it enters nothing, the
        // work command goes to the target's place without a target
        let mut order = *order;
        if master.class == class::LIVESTOCK
            && !self.enter_command(player, idx, order.target, false)?
        {
            let at = self
                .spatial
                .get_known(order.target)?
                .ok_or(GenieError::unsupported("a target without a place"))?
                .position;
            (order.point[0], order.point[1], order.target) = (at[0], at[1], -1);
        }
        let order = &order;
        self.town_bell_order(player, idx, order.target, order.filter)?;
        self.work_command(player, idx, order)?;
        let Some(tends) = self.work_pasture(player, idx, order)? else {
            return Ok(());
        };
        self.tend_pasture(player, idx, tends)
    }

    /// [`World::order_work`]'s reach against `target` (the world's player holding the order's
    /// target) when the unit `id` is not allied with it: [`work_reach`] of the gap between the
    /// two's places; none against an ally's target or none.
    pub(super) fn order_reach(
        &self,
        player: usize,
        id: i32,
        order: &Work,
        target: Option<usize>,
        master: &Master,
    ) -> Result<Option<(bool, bool)>, GenieError> {
        let world = TaskWorld {
            players: &self.players,
            drop_sites: &self.drop_sites,
        };
        Ok(match target {
            Some(tp) if !world.allied(player, tp, true) => {
                let (me, them) = (
                    self.spatial
                        .get(id)
                        .ok_or(GenieError::unsupported("an ordered unit without a place"))?,
                    self.spatial
                        .get_known(order.target)?
                        .ok_or(GenieError::unsupported("a target without a place"))?,
                );
                let (_, gap) = gaps(me.position, me.radius, them.position, them.radius);
                Some(work_reach(gap, master.weapon_range, master.min_range))
            }
            _ => None,
        })
    }

    /// [`World::order_work`]'s pasture after the work command: the pasture the unit then tends
    /// (−1 for none), or none when the order ends there (no livestock given). A villager ordered
    /// onto a pasture (1893, 1897) with room for workers works the livestock
    /// [`World::pasture_pick`] gives and tends the pasture; a full one has its unit AI take a
    /// search.
    pub(super) fn work_pasture(
        &mut self,
        player: usize,
        idx: usize,
        order: &Work,
    ) -> Result<Option<i32>, GenieError> {
        use crate::gathering::pasture::PASTURE_WORKERS;
        use crate::technology::Attributes;
        let unit = &self.players[player].objects[idx];
        let master = unit.master(&self.masters, player)?;
        if order.target == -1 {
            return Ok(Some(-1));
        }
        let target = self.task_target(order.target, 0)?;
        let pasture = master.class == class::VILLAGER
            && target.master.kind == BUILDING
            && matches!(target.master.id, PASTURE_MANGROVE | PASTURE);
        if !pasture {
            return Ok(Some(-1));
        }
        let room = Attributes::get(&self.players[player], PASTURE_WORKERS)?;
        let workers = self.livestock_workers(order.target) as f32;
        // COMISS room with the workers, JBE: a NaN is full
        if room > workers {
            let pick = self.pasture_pick(order.target, player, idx)?;
            if pick == -1 {
                return Ok(None);
            }
            let point = self
                .object(pick)
                .map(|(_, l)| l.position())
                .ok_or(GenieError::unreachable("the livestock picked"))?;
            let on_it = Work {
                target: pick,
                point,
                ..*order
            };
            self.order_work(player, idx, &on_it)?;
            return Ok(Some(order.target));
        }
        // its unit AI's slot 0x38 takes a search (0x202 with the hunt action, class 49 and −1,
        // from and to the unit)
        let unit = &mut self.players[player].objects[idx];
        let (id, state) = (unit.id, unit.life.state);
        if let Some(ai) = unit.ai.as_mut() {
            let search = crate::unit_ai::Notify::own(
                id,
                crate::unit_ai::REQUIRES_SEARCH,
                [ActionKind::Hunt.value(), i32::from(class::FARM), -1],
            );
            let _ = ai.accept(search, state);
        }
        Ok(Some(-1))
    }

    /// Slot 0x138 (`FUN_141216f10`) without a target, outside slot 0x128's work command (the
    /// unit's `+0xa14` 1): [`World::move_point_order_with`].
    pub(crate) fn move_point_order(
        &mut self,
        player: usize,
        idx: usize,
        point: [f32; 3],
    ) -> Result<(), GenieError> {
        self.move_point_order_with(player, idx, point, true)
    }

    /// Slot 0x138 (`FUN_141216f10`) without a target, the no-task end of a work command on a
    /// point: a villager drops its town-bell flag (not held otherwise), the unit AI's order queue
    /// empties (`FUN_140c5e010`) while the unit's `+0xa14` (`a14`) is set, then the move to the
    /// point ([`World::move_to_point`], `FUN_14106d340`). `+0xa14` is 1 but inside slot 0x128
    /// (`FUN_141211b80`), which holds it at its argument while its work command runs.
    pub(crate) fn move_point_order_with(
        &mut self,
        player: usize,
        idx: usize,
        point: [f32; 3],
        a14: bool,
    ) -> Result<(), GenieError> {
        let villager = self.players[player].objects[idx]
            .master(&self.masters, player)?
            .class
            == class::VILLAGER;
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
}
