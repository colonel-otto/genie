//! The enter command and the Enter's construction, install and start, at once or with the
//! transport's move.

use crate::GenieError;
use crate::garrison::{
    ENTER, ENTER_SLEEPING, EnterAction, EnterTarget, FAILED, NO_MASTERS, UNPLACED, UNREACHED,
    Walking, enter_view, ship_reaches,
};
use crate::objects::masters::Task;
use crate::orders::{BUILDING, GARRISON};
use crate::unit_ai::ActionKind;
use crate::unit_ai::OrderKind;
use crate::world::World;

impl World {
    /// The work command's Enter (its action type 3, [`World::new_enter`]), the unit AI hearing of
    /// it (slot 0x828: the enter order and action, the target with its class and the work's
    /// point), then its install ([`World::install_enter`]).
    pub(crate) fn start_enter(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        task: &Task,
        point: [f32; 3],
        clears: bool,
    ) -> Result<(), GenieError> {
        let (action, target, class) = self.new_enter(player, idx, target, task)?;
        if let Some(ai) = self.players[player].objects[idx].ai.as_mut() {
            ai.notify_work(
                Some(OrderKind::Enter),
                Some(ActionKind::Enter),
                Some((target, class)),
                point,
                clears,
                self.streams.get(crate::math::rng::Stream::Ai),
            )?;
            self.remember_order(
                player,
                idx,
                (Some(OrderKind::Enter), Some(ActionKind::Enter)),
            )?;
        }
        self.install_enter(player, idx, target, action, task)
    }

    /// Slot 0x3a0 (`FUN_1410847e0`, the enter command) of the unit at `idx` of `player` on
    /// `target`, `forced` its third argument: a target the world does not hold is false;
    /// unforced, a unit entering the target already (its top action an Enter on it) is true.
    /// Slot 0x820 ([`World::find_work_task`]) looks for a task on the target with the garrison
    /// filter (3) and the switch (the unit takes the master that gave it, slot 0x90): none is
    /// false; else its Enter ([`World::enter_with`]): true.
    ///
    /// # Errors
    ///
    /// What the Enter does not model.
    pub(crate) fn enter_command(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        forced: bool,
    ) -> Result<bool, GenieError> {
        if self.object(target).is_none() {
            return Ok(false);
        }
        let unit = &self.players[player].objects[idx];
        let top = unit.top_action_kind();
        if !forced && top == Some(ENTER) && unit.top_target() == Some(target) {
            return Ok(true);
        }
        let Some(task) = self.find_work_task(player, idx, target, GARRISON, true)? else {
            return Ok(false);
        };
        let under_artifact = top == Some(crate::ownership::artifact::ARTIFACT_TYPE);
        self.enter_with(player, idx, target, &task, under_artifact)?;
        Ok(true)
    }

    /// The unit's enter command its unit AI gave ([`crate::unit_ai::world_view::EnterOrder`]), after the
    /// update: the master switch (slot 0x90 with 0, [`World::change_master`]), then the Enter
    /// ([`World::enter_with`]).
    ///
    /// # Errors
    ///
    /// What the master change and the Enter do not model.
    pub(crate) fn enter_order(
        &mut self,
        player: usize,
        idx: usize,
        order: crate::unit_ai::world_view::EnterOrder,
    ) -> Result<(), GenieError> {
        if let Some(master) = order.switch {
            self.change_master(player, idx, master)?;
        }
        self.enter_with(player, idx, order.target, &order.task, order.under_artifact)
    }

    /// The enter command's Enter on `task` (`FUN_140a855d0`, [`World::new_enter`]): under an
    /// Artifact on top (`under_artifact`, read before the task search) it would become the
    /// Artifact's sub-action, which is not modelled; else the action list empties and the Enter
    /// is installed (slot 0x808, [`World::install_enter`]).
    ///
    /// # Errors
    ///
    /// What the Enter does not model.
    pub(crate) fn enter_with(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        task: &Task,
        under_artifact: bool,
    ) -> Result<(), GenieError> {
        let (action, target, _) = self.new_enter(player, idx, target, task)?;
        self.install_enter_as(player, idx, target, action, task, under_artifact)
    }

    /// The Enter's constructor (`FUN_140a855d0`) on `target` with `task`: its target, an annex
    /// its main building; the point [`meeting`] gives at the target's height
    /// ([`World::enter_meeting`]); it takes the target (slot 0x178). With the target and its
    /// class.
    ///
    /// [`meeting`]: crate::garrison::meeting::meeting
    pub(super) fn new_enter(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        task: &Task,
    ) -> Result<(EnterAction, i32, i32), GenieError> {
        let target = self
            .object(target)
            .and_then(|(tp, t)| {
                let kind = self.masters.get(tp)?.get(t.life.master_id)?.kind;
                Some(if kind == BUILDING {
                    t.annex_of.unwrap_or(target)
                } else {
                    target
                })
            })
            .ok_or(GenieError::unsupported("an Enter whose building is gone"))?;
        let (tp, t) = self.object(target).expect("found above");
        let tm = self.master_of(tp, t.life.master_id)?;
        let (class, height) = (i32::from(tm.class), t.position[2]);
        let (at, _) = self.enter_meeting(player, idx, target)?;
        let action = EnterAction::new(
            target,
            [at[0], at[1], height],
            task.work_range,
            task.object_group,
        );
        self.take(target, self.releaser(player, idx));
        Ok((action, target, class))
    }

    /// The Enter's install (slot 0x808): the action list empties and the Enter starts (slot 5,
    /// `FUN_140a86f20`): a unit inside an object (`set_state(13)`) and a target without room
    /// (`FUN_140a864d0`, `set_state(11)`) are not modelled; else its `set_state(4)`
    /// ([`EnterAction::walk_again`]): a unit in reach enters at once ([`World::enter_now`]),
    /// else it walks, a transport to the point first ([`World::move_transport`]).
    pub(super) fn install_enter(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        action: EnterAction,
        task: &Task,
    ) -> Result<(), GenieError> {
        self.install_enter_as(player, idx, target, action, task, false)
    }

    /// [`World::install_enter`], or with `sub` the Enter as the sub-action of the Artifact on top
    /// (`FUN_1410847e0`): the Artifact's sub-action goes (`FUN_140a98e20`,
    /// [`crate::ownership::artifact::Artifact::hold_enter`]: an Enter lets its target go, slot 0x180
    /// ([`World::release`]), and a walking `MoveTo`'s teardown stops the unit) and the Enter
    /// starts in its list (`FUN_140a977e0`), the Artifact's update running it from then on.
    pub(super) fn install_enter_as(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        mut action: EnterAction,
        task: &Task,
        sub: bool,
    ) -> Result<(), GenieError> {
        let id = self.players[player].objects[idx].id;
        if sub {
            let by = self.releaser(player, idx);
            let unit = &mut self.players[player].objects[idx];
            let art = unit.actions.artifact_mut().ok_or(GenieError::unreachable(
                "an Artifact on top without its action",
            ))?;
            let gone = art.hold_enter(action);
            // the old Enter's teardown lets its target go before its MoveTo's stops the unit
            if let Some(t) = gone.released {
                self.release(t, by)?;
            }
            let unit = &mut self.players[player].objects[idx];
            if gone.stops
                && let Some(m) = unit.body.mover_mut()
            {
                m.stop()?;
            }
        } else {
            self.clear_actions(player, idx)?;
        }
        let unit = &self.players[player].objects[idx];
        if unit.life.inside.is_some() {
            // set_state(13): the unit takes its master's standing sprite (slot 0x68 with
            // `+0x470`) and, its `+0x60` 0 in a new Enter, posts notify 0x1f9 with the enter's
            // action value to its unit AI; the update returns 3 then
            action.state = FAILED;
            let masters = self.masters.get(player);
            let unit = &mut self.players[player].objects[idx];
            unit.place_enter(action, sub);
            unit.shows_standing_sprite(
                masters,
                &self.graphics,
                self.streams.get(crate::math::rng::Stream::Sprite),
            )?;
            let state = unit.life.state;
            if let Some(ai) = unit.ai.as_mut() {
                let n = crate::unit_ai::Notify::own(
                    id,
                    crate::unit_ai::ACTION_FAILED,
                    [ActionKind::Enter.value(), 0, 0],
                );
                ai.post(n, state)?;
            }
            return Ok(());
        }
        let (tp, t) = self.object(target).expect("found above");
        if !self.has_room(tp, t, Some(id))? {
            return Err(GenieError::unsupported(
                "an Enter into a building without room (state 11)",
            ));
        }
        let (t, (gap, speed), meeting) = self.entering(player, idx, target, task.work_range)?;
        let walking = action.walk_again(&t, gap, speed, &mut || meeting.clone())?;
        match walking {
            // the enter's set_state(1) reads the Enter as the unit's; a sub-action goes back
            Walking::Enter => {
                self.players[player].objects[idx]
                    .actions
                    .set_enter(Some(action));
                let entered = self.enter_now(player, idx, target);
                if sub {
                    let unit = &mut self.players[player].objects[idx];
                    let e = unit.actions.take_enter();
                    if let (Some(e), Some(art)) = (e, unit.actions.artifact_mut()) {
                        art.enter = Some(e);
                    }
                }
                entered
            }
            Walking::Walk(to) => {
                self.players[player].objects[idx].place_enter(action, sub);
                to.map_or(Ok(()), |to| self.move_transport(target, to))
            }
        }
    }

    /// What the Enter's start reads of unit `idx` of `player` and its target ([`enter_view`],
    /// room by slot 0x198).
    #[allow(clippy::type_complexity)]
    pub(super) fn entering(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        range: f32,
    ) -> Result<(EnterTarget, (f32, f32), Result<([f32; 2], u8), GenieError>), GenieError> {
        let id = self.players[player].objects[idx].id;
        let (tp, t) = self.object(target).ok_or(UNPLACED)?;
        let tm = self.master_of(tp, t.life.master_id)?.clone();
        let room = self.has_room(tp, t, Some(id))?;
        let t = match self.place(target) {
            Some(crate::orders::Place::Live(p, i)) => &self.players[p].objects[i],
            Some(crate::orders::Place::Sleeping(p, i)) => &self.players[p].sleeping[i],
            None => return Err(UNPLACED),
        };
        enter_view(
            &self.spatial,
            &mut self.zones,
            &self.masters,
            &self.players[player].objects[idx],
            (t, &tm, room),
            range,
        )
    }

    /// `FUN_140a85f30` ([`meeting`]) for unit `idx` of `player` and its target, as the
    /// constructor reads it ([`enter_view`]).
    ///
    /// [`meeting`]: crate::garrison::meeting::meeting
    pub(super) fn enter_meeting(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
    ) -> Result<([f32; 2], u8), GenieError> {
        self.entering(player, idx, target, 0.0)?.2
    }

    /// The transport's part of the Enter's `set_state(4)` with code 1: its slot 0x3d8
    /// (`FUN_14114ffc0`) asks whether it reaches within 1 of the point's tile (the point
    /// truncated, [`crate::movement::Mover::reaches`]); then its slot 0x340 (`FUN_141216c50`:
    /// [`World::move_aside`], range 0, forced, running) moves it to the point at its height. One
    /// that cannot reach it is [`UNREACHED`].
    pub(crate) fn move_transport(
        &mut self,
        transport: i32,
        to: [f32; 3],
    ) -> Result<(), GenieError> {
        let Some(crate::orders::Place::Live(tp, ti)) = self.place(transport) else {
            return Err(UNREACHED);
        };
        let ship = &self.players[tp].objects[ti];
        let mut plan = crate::movement::Planning {
            pathing: &mut self.pathing,
            now: self.world_time,
            computer: self.players[tp].computer,
            object: self.streams.get(crate::math::rng::Stream::Object),
            construction_rate: self.players[tp].construction_rate,
            groups: &self.groups,
        };
        if !ship_reaches(ship, &self.spatial, &mut plan, to)? {
            return Err(UNREACHED);
        }
        self.ship_move(transport, to)
    }

    /// The transport's slot 0x340 (`FUN_141216c50`, [`World::move_aside`]: range 0, forced,
    /// running) to `to`, where it meets an entering unit.
    pub(crate) fn ship_move(&mut self, ship: i32, to: [f32; 3]) -> Result<(), GenieError> {
        let Some(crate::orders::Place::Live(p, i)) = self.place(ship) else {
            return Err(UNREACHED);
        };
        let walk = crate::unit_ai::AiMove {
            point: to,
            range: 0.0,
            force: true,
            walk: false,
        };
        self.move_aside(p, i, walk)
    }

    /// The enter at the Enter's start ([`crate::objects::unit::Unit::enter_building`] into its target,
    /// any player's live object, then the start's own `set_state(1)`); the load it gives goes to
    /// the owner's attributes at once.
    pub(super) fn enter_now(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
    ) -> Result<(), GenieError> {
        let inside = self
            .players
            .iter()
            .flat_map(|p| p.objects.iter().chain(&p.sleeping))
            .filter(|u| u.life.inside == Some(target))
            .count();
        let Some(crate::orders::Place::Live(tp, ti)) = self.place(target) else {
            return Err(ENTER_SLEEPING);
        };
        let mut unit = self.players[player].objects.take_out(idx);
        let mut adds = Vec::new();
        let result = (|| {
            let building = &self.players[tp].objects[ti];
            let owner = &self.players[player];
            let mut w = crate::garrison::unit::EnterWorld {
                spatial: &mut self.spatial,
                pathing: &mut self.pathing,
                masters: self.masters.get(player),
                target_masters: self.masters.get(tp),
                graphics: &self.graphics,
                sprite_rng: self.streams.get(crate::math::rng::Stream::Sprite),
                drop_sites: &self.drop_sites,
                attributes: &owner.attributes,
                civ: owner.civ,
                sharing: owner.sharing,
                player,
                attribute_adds: &mut adds,
                firepower: 0.0,
                left_group: None,
            };
            if !unit.enter_building(building, inside, &mut w)? {
                return Err(GenieError::unsupported(
                    "an Enter in reach that did not get in",
                ));
            }
            unit.entered(&mut w)?;
            Ok((w.firepower, w.left_group))
        })();
        let id = unit.id;
        let _ = self.put_back_own(player, unit)?;
        let (firepower, left_group) = result?;
        // its removeUnit from the unit group it left (FUN_14102c160), when the world holds it
        if let Some(g) = left_group.filter(|&g| self.groups.get(g).is_some()) {
            let _ = self.remove_unit(g, id, target, false)?;
        }
        let b = &mut self.players[tp].objects[ti];
        b.firepower += firepower;
        b.garrison.push(id);
        for (kind, amount) in adds {
            self.players[player].add_attribute(kind, amount)?;
        }
        Ok(())
    }

    /// The unit at `idx` of player `player` goes into `target` at once, its slot 0x1a0 forced
    /// (`FUN_141212e10` with 1: `FUN_1411921d0` passes over the room test, slot 0x198;
    /// [`crate::objects::unit::Unit::go_inside`]): no load given, no notify, no state change. The
    /// building's list takes it and its volleys its share.
    pub(crate) fn enter_forced(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
    ) -> Result<(), GenieError> {
        let Some(crate::orders::Place::Live(tp, ti)) = self.place(target) else {
            return Err(ENTER_SLEEPING);
        };
        let me = self
            .master_of(player, self.players[player].objects[idx].life.master_id)?
            .clone();
        let bm = self
            .master_of(tp, self.players[tp].objects[ti].life.master_id)?
            .clone();
        let mut unit = self.players[player].objects.take_out(idx);
        let mut adds = Vec::new();
        let result: Result<_, GenieError> = (|| {
            let building = &self.players[tp].objects[ti];
            let owner = &self.players[player];
            let masters = self.masters.get(player).ok_or(NO_MASTERS)?;
            let mut w = crate::garrison::unit::EnterWorld {
                spatial: &mut self.spatial,
                pathing: &mut self.pathing,
                masters: Some(masters),
                target_masters: self.masters.get(tp),
                graphics: &self.graphics,
                sprite_rng: self.streams.get(crate::math::rng::Stream::Sprite),
                drop_sites: &self.drop_sites,
                attributes: &owner.attributes,
                civ: owner.civ,
                sharing: owner.sharing,
                player,
                attribute_adds: &mut adds,
                firepower: 0.0,
                left_group: None,
            };
            unit.go_inside(building, (masters, &me, &bm), &mut w)?;
            Ok((w.firepower, w.left_group))
        })();
        let id = unit.id;
        let _ = self.put_back_own(player, unit)?;
        let (firepower, left_group) = result?;
        if let Some(g) = left_group.filter(|&g| self.groups.get(g).is_some()) {
            let _ = self.remove_unit(g, id, target, false)?;
        }
        let b = &mut self.players[tp].objects[ti];
        b.firepower += firepower;
        b.garrison.push(id);
        Ok(())
    }
}
