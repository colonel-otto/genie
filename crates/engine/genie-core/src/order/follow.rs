//! FOLLOW and GUARD, a move to an object and the head action's move.

use crate::gathering::hunt::HUNT;
use crate::math::rng::Stream;
use crate::order::{
    Followed, GUARD_RANGE, LIVESTOCK, MOVING, Place, VILLAGER, follow_axis, follow_spread,
    ignore_unit,
};
use crate::turn::Turns;
use crate::unit_ai::{GenieError, MOVE_ORDER};

impl Turns {
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
    ) -> Result<(), GenieError> {
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
            return Err(GenieError::unsupported(
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

    /// GUARD (0x13) after its check and its unit AI resets (the caller's, [`crate::turn`]):
    /// `FUN_140e67620`'s first 60 units (500 for a computer player) go, gone ones null
    /// (`FUN_140e6b750`), to `FUN_14161dee0`: each unit the world holds leaves the unit group the
    /// world holds of its link (`FUN_14102c160` with −1 and 0), and its unit AI takes the defend
    /// order (`0x2bd`) on `target` at once (slot 0x30 -> slot 0x28 with Immediate set and inFront
    /// clear, [`crate::unit_ai::UnitAi::take_order_now_with`], run outside the unit's update by
    /// [`Turns::run_unit_ai`]): issued by the unit's owner, on the target with its master as the
    /// target type and its owner, at the target's place, within [`GUARD_RANGE`], priority 100. Of
    /// the record's last three fields the call writes only the first's low byte (1, the shown x:
    /// its upper bytes are the stack's, not held); the stack slots it leaves as the other two hold
    /// the place's decoded y (the shown y) and x, whose low byte becomes the ready flag.
    pub(crate) fn guard(
        &mut self,
        player: usize,
        units: &[i32],
        target: i32,
    ) -> Result<(), GenieError> {
        let Some((tp, t)) = self.object(target) else {
            return Ok(());
        };
        let tm = self.master_of(tp, t.life.master_id)?;
        let [x, y, z] = t.position;
        let order = crate::unit_ai::QueuedOrder {
            issuer: -1,
            order: crate::unit_ai::DEFEND_ORDER,
            priority: 100,
            target,
            target_owner: tp as i32,
            point: [x, y, z],
            range: GUARD_RANGE,
            shown: [f32::from_bits(1), y],
            ready: x.to_bits() as u8 != 0,
            target_type: i32::from(tm.id),
        };
        let cap = if self.players[player].computer {
            500
        } else {
            60
        };
        for &id in &units[..units.len().min(cap)] {
            let Some(place) = self.place(id) else {
                continue;
            };
            self.leave_group_of(id, -1)?;
            let (p, idx) = match place {
                Place::Live(p, i) => (p, i),
                // only slot 0x180 puts an object to sleep, one of a type below 30 without a
                // unit AI
                Place::Sleeping(p, i) if self.players[p].sleeping[i].ai.is_none() => continue,
                Place::Sleeping(..) => {
                    return Err(GenieError::unsupported(
                        "a GUARD of a sleeping object with a unit AI",
                    ));
                }
            };
            if self.players[p].objects[idx].ai.is_none() {
                continue;
            }
            let order = crate::unit_ai::QueuedOrder {
                issuer: p as i32,
                ..order
            };
            self.run_unit_ai((p, idx), |ai, at, side, computer, rng| {
                let mut fx = Vec::new();
                ai.take_order_now_with(order, (at, side, computer), rng, &mut fx)?;
                Ok(fx)
            })?;
        }
        Ok(())
    }

    /// Unit `id` leaves the unit group the world holds of its link (`FUN_14102c160` with `target`
    /// and 0, [`Turns::remove_unit`]); a unit the world does not hold or in no group, nothing.
    pub(crate) fn leave_group_of(&mut self, id: i32, target: i32) -> Result<(), GenieError> {
        let link = self.object(id).map_or(-1, |(_, u)| u.group.id);
        if link != -1 {
            let _ = self.remove_unit(link, id, target, false)?;
        }
        Ok(())
    }

    /// FOLLOW's test of a unit (`FUN_14161c920`): one the world holds, of a type of 30 or more
    /// whose master moves (slot 0xe8 above 0; `COMISS`, `JNC`: a NaN speed moves), gives its
    /// place on the ground.
    pub(super) fn follower(&self, id: i32) -> Result<Option<[f32; 2]>, GenieError> {
        let Some((p, u)) = self.object(id) else {
            return Ok(None);
        };
        let master = self.master_of(p, u.life.master_id)?;
        let speed = master.moving.as_ref().map_or(0.0, |m| m.speed);
        let moves = MOVING <= master.kind && crate::math::fmath::below_or_unordered(0.0, speed);
        Ok(moves.then_some([u.position[0], u.position[1]]))
    }

    /// A follower's start (`FUN_14161c920`, both loops): its ignore list empties
    /// (`FUN_141197da0`), it waits to move (slot 0x2a0 with 1) and lists the other loose units
    /// ([`ignore_unit`]); its place in the live list. A sleeping object is of a type below 30
    /// (only slot 0x180 puts one to sleep), which [`Turns::follower`] passed over.
    pub(super) fn follower_start(
        &mut self,
        loose: &[i32],
        i: usize,
    ) -> Result<(usize, usize), GenieError> {
        let Some(Place::Live(p, idx)) = self.place(loose[i]) else {
            return Err(GenieError::unsupported(
                "a FOLLOW of a sleeping object of a type of 30 or more",
            ));
        };
        let mover = self.players[p].objects[idx]
            .mover
            .as_mut()
            .ok_or(GenieError::unsupported(
                "a followed unit without movement state",
            ))?;
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
    pub(super) fn follow_unit(
        &mut self,
        loose: &[i32],
        i: usize,
        followed: Followed,
    ) -> Result<(), GenieError> {
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
    pub(super) fn follow_offset(
        &mut self,
        loose: &[i32],
        i: usize,
        followed: Followed,
        mean: [f32; 2],
    ) -> Result<(), GenieError> {
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
    ) -> Result<(), GenieError> {
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
    /// (`FUN_140aa0df0`: no range, so slot 0x268's, [`crate::move_to::object_range`]; the
    /// walking sprite) takes the target it holds
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
    ) -> Result<bool, GenieError> {
        let world_time = self.world_time;
        let class = self
            .master_of(player, self.players[player].objects[idx].life.master_id)?
            .class;
        let villager = class == VILLAGER;
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
        let place = ([unit.position[0], unit.position[1]], class);
        if let Some(ai) = unit.ai.as_mut()
            && ai.clear_notifies_at(world_time, place)?
        {
            // the completion's stop (slot 0xb0 with 1): the unit's stopAction at once
            self.command_stop(player, idx)?;
        }
        let unit = &mut self.players[player].objects[idx];
        let stop_after_kill = unit.ai.as_ref().is_some_and(|ai| ai.stop_after_kill);
        if unit.ai.is_some()
            && !stop_after_kill
            && self.head_takes_move(player, idx, Some(target), point)?
        {
            return Ok(true);
        }
        let unit = &self.players[player].objects[idx];
        if unit.unmodelled_action.is_some() {
            return Err(GenieError::unsupported(
                "a move to an object of a unit with an action",
            ));
        }
        // FUN_140aa0df0 makes the MoveTo first, which takes its target (slot 0x178, quiet 0);
        // slot 0x808 then clears the list (the old actions' teardowns) and starts it
        let livestock = self.master_of(player, unit.life.master_id)?.class == LIVESTOCK;
        let (tp, t) = self.object(target).ok_or(crate::move_to::TARGET_LACKING)?;
        let t_state = t.life.state;
        let t_master = self.master_of(tp, t.life.master_id)?.id;
        if crate::move_to::holds_object(livestock, t_master, t_state) {
            self.take(target, self.releaser(player, idx));
        }
        self.clear_actions(player, idx)?;
        let master = self.master_of(player, self.players[player].objects[idx].life.master_id)?;
        let range = crate::move_to::object_range(0.0, Some(master));
        let unit = &mut self.players[player].objects[idx];
        let mover = unit.mover.as_mut().ok_or(GenieError::unsupported(
            "a moved unit without movement state",
        ))?;
        let move_to = crate::move_to::MoveToAction::to_object(
            &self.spatial,
            unit.position,
            mover,
            (target, t_state),
            range,
            (false, livestock),
        )?;
        if move_to.shows_standing() {
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
    /// toward the object `target` if any: true when the action takes the move. Without a target
    /// it is the unit's own [`crate::turn::Unit::head_takes_point`], whose released target goes
    /// here ([`Turns::release`]). Toward an object, an empty list takes nothing, nor does the
    /// base slot (`FUN_140a78490`) of a `MoveTo`, an Explore, a Build (0x140aade60), an Attack,
    /// an Enter, a Fly, a missile, a Make or a wait for a foundation. The Artifact's
    /// (`FUN_140aaca30`) always takes it ([`Turns::artifact_take_object`]); the Hunt's
    /// (`0x140ac52c0`, [`Turns::hunt_take_move`]) and the Gather's (`0x140a8b740`,
    /// [`Turns::gather_take_move`]) decide themselves.
    pub(super) fn head_takes_move(
        &mut self,
        player: usize,
        idx: usize,
        target: Option<i32>,
        point: [f32; 3],
    ) -> Result<bool, GenieError> {
        let unit = &mut self.players[player].objects[idx];
        let Some(target) = target else {
            return self.point_taken(player, idx, point);
        };
        if unit.unmodelled_action.is_some() {
            return Err(GenieError::unsupported(
                "slot 0x40 of an action the slice does not hold",
            ));
        }
        match unit.top_action_kind() {
            None
            | Some(
                crate::move_to::MOVE_TO
                | crate::explore::EXPLORE
                | crate::construction::build::BUILD
                | crate::combat::attack::ATTACK
                | crate::garrison::ENTER
                | crate::fly::FLY
                | crate::combat::missile::MISSILE
                | crate::gathering::make::MAKE
                | crate::construction::wait_solidify::WAIT_SOLIDIFY,
            ) => Ok(false),
            Some(crate::ownership::artifact::ARTIFACT_TYPE) => {
                self.artifact_take_object(player, idx, target)?;
                Ok(true)
            }
            Some(HUNT) => self.hunt_take_move(player, idx, target, point),
            Some(crate::gathering::gather::GATHER) => {
                self.gather_take_move(player, idx, target, point)
            }
            // the Repair's slot 0x40 (FUN_140ad9700) walks to the point whatever the target, and
            // the Transport's (0x140aa3e40) sails there
            Some(crate::construction::repair::REPAIR | crate::garrison::transport::TRANSPORT) => {
                self.point_taken(player, idx, point)
            }
            Some(other) => {
                unreachable!("a top action of type {other}, which top_action never holds")
            }
        }
    }

    /// The head action's slot 0x40 on a move to `point` ([`crate::turn::Unit::head_takes_point`]):
    /// the targets it let go are released and a counted repairer leaves its target's repairers.
    pub(super) fn point_taken(
        &mut self,
        player: usize,
        idx: usize,
        point: [f32; 3],
    ) -> Result<bool, GenieError> {
        let by = self.releaser(player, idx);
        let (released, left) =
            match self.players[player].objects[idx].head_takes_point(point, player)? {
                crate::turn::PointMove::Declined => return Ok(false),
                crate::turn::PointMove::Taken(released) => (released, None),
                crate::turn::PointMove::Repairs(released, left) => (released, left),
                crate::turn::PointMove::Sails => {
                    self.transport_move(player, idx, point)?;
                    return Ok(true);
                }
            };
        if let Some(t) = left {
            self.leave_builders(t)?;
        }
        if let Some(t) = released {
            self.release(t, by)?;
        }
        Ok(true)
    }
}
