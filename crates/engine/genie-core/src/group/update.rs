//! A group's update (FD-036): `BUnitGroup::update` (`FUN_14103d910`), which the world runs for
//! the groups a player owns after the player's objects, and the deletion of a group whose update
//! returns 0 (the world list's unlink, then `BUnitGroup`'s destructor, `FUN_140fe1920`).

use crate::group::command::MOVE_COMMAND;
use crate::group::{UnitGroup, WorldObject};
use crate::turn::Turns;
use crate::unit_ai::Unsupported;

/// The group's current command (`mCurrentCommand`, `+0x3a8`) without one.
const NO_COMMAND: i32 = -1;
/// The group commands the update tells apart: an attack, a build, a repair, a relic's pick-up
/// and its delivery.
const ATTACK: i32 = 9;
const BUILD: i32 = 0x65;
const REPAIR: i32 = 0x6a;
const PICK_UP_RELIC: i32 = 0x84;
const DELIVER_RELIC: i32 = 0x88;
/// The action types of the tasks that pick up and deliver relics (`FUN_1415c60b0`'s third
/// argument, as the commands).
const RELIC_PICKUP_ACTION: i16 = 0x84;
const RELIC_DELIVERY_ACTION: i16 = 0x88;
/// The relics' master class (`FUN_141017c70`'s test of the target's class).
const RELIC_CLASS: i16 = 0x2a;
/// The monastery's master id (`FUN_141017850`'s test of the target's master).
const MONASTERY: i16 = 104;
/// The unit AI's order on a target (`+0x3c`) and its action on a relic (`+0x44`) that
/// `FUN_141017c70` and `FUN_141017850` read.
const TARGET_ORDER: i32 = crate::unit_ai::ORDER_ON_TARGET;
const RELIC_ACTION: i32 = 0x277;
/// The unit AI's action that defers to its order (`FUN_141017f00`).
const CHOSEN_ACTION: i32 = 0x25b;
/// The unit AI's attack action (`FUN_141007ca0`'s test, 600).
const ATTACK_ACTION: i32 = 0x258;
/// The master types from which an object moves (`RGE_Moving_Object`), acts
/// (`RGE_Action_Object`) and is a TRIBE combat unit.
const MOVING_KIND: u8 = 30;
const ACTION_KIND: u8 = 40;
const COMBAT_KIND: u8 = 70;
/// The trade cart's and the trade boat's master classes (`FUN_141017f00`).
const TRADE_CART: i16 = 19;
const TRADE_BOAT: i16 = 2;
/// The nearest member's target starts this far (`FUN_141007ca0`, `DAT_14330ebf4`).
const FARTHEST: f32 = crate::image::group::FARTHEST;

impl UnitGroup {
    /// The group's vtable slot 8 after its update (`BUnitGroup::updateEnd`, `FUN_141040b50`): a
    /// group whose motion moved (motion flags bit 0) adds its new position to the front of its
    /// stored positions ([`crate::group::PointLine::add_at_front`]); the motion's flags lose bit
    /// 0, and bit 1 with it when set, which sets the group's flags bit 1 (`FUN_14104a240`).
    pub fn update_end(&mut self) {
        let g = self;
        if g.motion.flags & 1 != 0 {
            let [point, forward, up, right] = g.motion.new_position;
            g.stored_positions.add_at_front(crate::group::LinePoint {
                point,
                forward,
                up,
                right,
            });
        }
        let flags = g.motion.flags;
        g.motion.flags &= !1;
        if flags & 2 != 0 {
            g.motion.flags &= !3;
            g.flags |= 2;
        }
    }
}

impl Turns {
    /// The groups player `p` owns update after its objects (`BaseWorld::update` at
    /// `0x141551796`): `BUnitGroup::update` ([`Turns::group_update`]) for each group of the
    /// world's list the player owns; a group whose update returns 0 goes
    /// ([`Turns::delete_group`]), any other runs its vtable slot 8 (`FUN_141040b50`). A stopped
    /// player's groups wait with it.
    ///
    /// # Errors
    ///
    /// What the update does not model.
    pub(crate) fn update_groups(&mut self, p: usize) -> Result<(), Unsupported> {
        if self.players.get(p).is_some_and(|q| q.stopped) {
            return Ok(());
        }
        let ids: Vec<i32> = self
            .groups
            .groups
            .iter()
            .filter(|g| g.owner as usize == p)
            .map(|g| g.id)
            .collect();
        for id in ids {
            if self.group_update(id)? {
                self.group_update_end(id);
            } else {
                self.delete_group(id)?;
            }
        }
        Ok(())
    }

    /// The group's vtable slot 8 after its update ([`UnitGroup::update_end`]) of group `group`,
    /// which its update kept.
    ///
    /// # Panics
    ///
    /// When the world does not hold the group: the update loop passes only the groups it holds.
    fn group_update_end(&mut self, group: i32) {
        self.groups
            .get_mut(group)
            .expect("a group the update kept")
            .update_end();
    }

    /// A group whose update returned 0 goes (`BaseWorld::update` at `0x1415517a6`): the world's
    /// list loses it (and its count, `world+0x58`, one), then its destructor (`FUN_141006d10` ->
    /// `FUN_140fe1920`) runs. Each member the world holds whose link names the group leaves it
    /// (`+0x6d0` -1); one that is idle ([`Turns::member_idle`], `FUN_141017f00`) stands (slot
    /// 0x560, [`crate::movement::Mover::stand`]) and shows its standing sprite (slot 0x660,
    /// [`Turns::show_standing`]). The rest of the destructor frees the group's memory (its
    /// command queue emptied by `FUN_14102bb40` first); the destroyed id it hands the spectator
    /// stream (`FUN_141778770`) and its lookup of the group's target (the object, or a target
    /// group's first member, whose answer it drops) are no game state. Nothing for a group the
    /// world does not hold.
    ///
    /// # Errors
    ///
    /// A member the slice cannot stand or show standing.
    pub(crate) fn delete_group(&mut self, group: i32) -> Result<(), Unsupported> {
        let Some(index) = self.groups.groups.iter().position(|g| g.id == group) else {
            return Ok(());
        };
        let g = self.groups.groups.remove(index);
        for m in &g.members {
            let Some((_, u)) = self.object_mut(m.id) else {
                continue;
            };
            if u.group.id != g.id {
                continue;
            }
            u.group.id = -1;
            if self.member_idle(m.id)? {
                if let Some((_, u)) = self.object_mut(m.id)
                    && let Some(mover) = u.mover.as_mut()
                {
                    mover.stand()?;
                }
                self.show_standing(m.id)?;
            }
        }
        Ok(())
    }

    /// `BUnitGroup::update` (`FUN_14103d910`) of group `group`: true keeps the group, false (the
    /// routine's 0) has the world delete it. Flags bit 13 clears; the units the group keeps apart
    /// are checked ([`Turns::keep_apart`], `FUN_141007ca0`); a member the world does not hold,
    /// without a unit AI (a ghost has none), below 1 hit point (`COMISS` then `JA`: not a NaN) or
    /// of another player leaves the group (`removeUnit` with target -1, [`Turns::remove_unit`])
    /// and then stops (slot 0x5d0 on the object the world holds, [`crate::movement::Mover::
    /// stop`]); any such leave sets flags bit 16. A hibernating group (`mHibernating`, `+0x13c`)
    /// runs its own check ([`Turns::hibernating_update`]) and, unless that deletes it, stays
    /// asleep (true) without motion flags bit 1; with it, and for an awake group, the update
    /// goes on ([`Turns::awake_update`]).
    ///
    /// # Errors
    ///
    /// What the update does not model.
    ///
    /// # Panics
    ///
    /// When the world does not hold `group`: the game's routine gets the group itself, from a
    /// caller that holds it.
    pub fn group_update(&mut self, group: i32) -> Result<bool, Unsupported> {
        self.groups
            .get_mut(group)
            .expect("the caller holds the group")
            .flags &= !0x2000;
        self.keep_apart(group)?;
        self.drop_lost_members(group)?;
        if self.groups.get(group).expect("held").hibernating != 0 {
            if !self.hibernating_update(group)? {
                return Ok(false);
            }
            if self.groups.get(group).expect("held").motion.flags & 2 == 0 {
                return Ok(true);
            }
        }
        self.awake_update(group)
    }

    /// The members' check of `BUnitGroup::update` (`0x14103d9c0`..`0x14103daf7`): see
    /// [`Turns::group_update`]. A member that left takes its index with it, so the next member
    /// is checked at the same index.
    fn drop_lost_members(&mut self, group: i32) -> Result<(), Unsupported> {
        let mut removed = false;
        let mut i = 0;
        loop {
            let g = self.groups.get(group).expect("the caller holds the group");
            let Some(m) = g.members.get(i) else {
                break;
            };
            let (id, owner) = (m.id, g.owner);
            let leaves = match self.world_object(id) {
                None | Some(WorldObject::Ghost(..)) => true,
                Some(WorldObject::Unit(p, u)) => {
                    u.ai.is_none() || u.life.hp < 1.0 || p as u32 != owner
                }
            };
            if leaves && self.remove_unit(group, id, -1, false)? {
                if let Some((_, u)) = self.object_mut(id)
                    && let Some(mover) = u.mover.as_mut()
                {
                    mover.stop()?;
                }
                removed = true;
                continue;
            }
            i += 1;
        }
        if removed {
            self.groups.get_mut(group).expect("held").flags |= 0x1_0000;
        }
        Ok(())
    }

    /// A hibernating group's check (`0x14103db22`..`0x14103dd5b`): false deletes the group;
    /// else the queued commands run (`checkQueuedOrders`, [`Turns::check_queued_orders`]) when
    /// [`Turns::hibernation`] asks for them.
    fn hibernating_update(&mut self, group: i32) -> Result<bool, Unsupported> {
        let Some(runs_queue) = self.hibernation(group)? else {
            return Ok(false);
        };
        if runs_queue {
            self.check_queued_orders(group)?;
        }
        Ok(true)
    }

    /// What a hibernating group does by its current command (`+0x3a8`): None deletes it, else
    /// whether its queued commands run. Commands 132 and 136 (a relic's pick-up and delivery)
    /// and 9 (an attack) wait while the group's target is pending ([`Turns::target_pending`],
    /// `FUN_1410176b0`); otherwise (the command's name logged, `FUN_1410171d0`) the queued
    /// commands run. A repair (106) waits while its target is below its master's hit points
    /// (`COMISS` then `JNC`: a NaN waits too) and its owner and the group's count each other as
    /// allies (`FUN_141663d90` with the ally flag both ways); else, or without the target, the
    /// queued commands run. No command (-1) and a move (1) run the queued commands when there
    /// are any. A build (101) waits while its target stands below state 2 for the group's own
    /// player; else the queued commands run. Every other command deletes the group.
    ///
    /// # Errors
    ///
    /// What the target tests read that the slice does not hold.
    ///
    /// # Panics
    ///
    /// When the world does not hold `group`.
    fn hibernation(&self, group: i32) -> Result<Option<bool>, Unsupported> {
        let g = self.groups.get(group).expect("the caller holds the group");
        let (target, owner) = (g.current_target.target, g.owner as usize);
        Ok(Some(match g.current_command {
            PICK_UP_RELIC | DELIVER_RELIC | ATTACK => !self.target_pending(group)?,
            REPAIR => !self.repair_waits(target, owner)?,
            NO_COMMAND | MOVE_COMMAND => !g.command_queue.is_empty(),
            BUILD => !self
                .world_object(target)
                .is_some_and(|t| t.state() < 2 && t.player() == owner),
            _ => return Ok(None),
        }))
    }

    /// A hibernating repair's wait (`0x14103dbc4`..`0x14103dcee`): its target `target` is held,
    /// below its master's hit points (the short at `+0x530`, `CVTDQ2PS`), and its player and
    /// `owner` count each other as allies.
    ///
    /// # Errors
    ///
    /// A ghost target, whose hit points (`+0x278`) the slice does not hold, and a target of a
    /// master the slice does not have.
    fn repair_waits(&self, target: i32, owner: usize) -> Result<bool, Unsupported> {
        let Some(object) = self.world_object(target) else {
            return Ok(false);
        };
        let WorldObject::Unit(p, u) = object else {
            return Err(Unsupported(
                "a hibernating repair of a ghost (its hit points, +0x278)",
            ));
        };
        let most = f32::from(self.master_of(p, u.life.master_id)?.hit_points);
        Ok(crate::fmath::below_or_unordered(u.life.hp, most)
            && self.spatial.allied_with(owner, p)
            && self.spatial.allied_with(p, owner))
    }

    /// `BUnitGroup::update` from `0x14103dd65` on, for an awake group (or a hibernating one
    /// whose motion has flags bit 1). Without queued commands: when the group keeps every member
    /// apart (`+0x468` equal to the member count), its target is not pending ([`Turns::
    /// target_pending`]), it walks no patrol (the top path's flags bit 0) and flags bit 11 is
    /// clear, each kept-apart unit leaves (`removeUnit` with target -1; one that is no member
    /// leaves the list instead, `FUN_140a63bd0`). The statistics' far flag is no queued command
    /// and motion flags bit 1 clear. With flags bit 18 the lead is the member nearest the
    /// target's place ([`Turns::nearest_to_target`]); a lead found has the motion update with 0
    /// ([`Turns::motion_update`]), then without motion flags bit 1 the statistics
    /// ([`Turns::group_statistics`]) and the centroid (`+0x144`) as the motion's position and
    /// new position (`+0x8`, `+0x38`), and bit 18 clears. Then the group goes (false) without
    /// members; without a queued command or a current one but with a formation, it goes when
    /// every slot is in state 3 (or it has none). With a member held and flags bit 16 the
    /// statistics run; with flags bit 6 the group goes; the lead is again the member nearest the
    /// target, and without a held lead (`FUN_1410465d0`) the group goes. A held lead re-places
    /// the group ([`Turns::set_initial_position`]) when motion flags bit 1 is set, flags bit 10
    /// is clear and the formation is not a horde (kind 0), or flags bit 15 is set (which then
    /// clears); the sub-formations without units go. Then `BMotion`'s update, with a budget
    /// while flags bit 8 is clear.
    ///
    /// # Errors
    ///
    /// What the update does not model, and the routine past `BMotion`'s update (from
    /// `0x14103e014`).
    fn awake_update(&mut self, group: i32) -> Result<bool, Unsupported> {
        let g = self.groups.get(group).expect("the caller holds the group");
        let mut far = false;
        if g.command_queue.is_empty() {
            let patrol = crate::group::formation::top_path(&g.motion.paths)
                .is_some_and(|p| p.path_flags & 1 != 0);
            // the dword compare at 0x14103dd78: the list's size (64 bits) against the count
            if g.objects.len() as u32 == g.members.len() as u32
                && !self.target_pending(group)?
                && !patrol
                && g.flags >> 11 & 1 == 0
            {
                while let Some(&id) = self.groups.get(group).expect("held").objects.first() {
                    if !self.remove_unit(group, id, -1, false)? {
                        let _ = self.groups.get_mut(group).expect("held").objects.remove(0);
                    }
                }
            }
            let g = self.groups.get(group).expect("held");
            far = g.command_queue.is_empty() && g.motion.flags & 2 == 0;
        }
        if self.groups.get(group).expect("held").flags & 0x4_0000 != 0 {
            let lead = self.nearest_to_target(group);
            self.groups.get_mut(group).expect("held").motion.lead = lead;
            if lead != -1 {
                self.motion_update(group, false)?;
                if self.groups.get(group).expect("held").motion.flags & 2 == 0 {
                    self.group_statistics(group, far)?;
                    let g = self.groups.get_mut(group).expect("held");
                    g.motion.new_position[0] = g.centroid;
                    g.motion.position[0] = g.centroid;
                }
                self.groups.get_mut(group).expect("held").flags &= !0x4_0000;
            }
        }
        let g = self.groups.get(group).expect("held");
        if g.members.is_empty() {
            return Ok(false);
        }
        if g.command_queue.is_empty()
            && g.current_command == NO_COMMAND
            && g.current_formation
                .as_ref()
                .is_some_and(|f| f.slots.iter().all(|s| s.state == 3))
        {
            return Ok(false);
        }
        if self.any_member_held(group) && g.flags & 0x1_0000 != 0 {
            self.group_statistics(group, far)?;
        }
        if self.groups.get(group).expect("held").flags & 0x40 != 0 {
            return Ok(false);
        }
        let lead = self.nearest_to_target(group);
        self.groups.get_mut(group).expect("held").motion.lead = lead;
        let g = self.groups.get(group).expect("held");
        if self.lead_object(&g.motion).is_none() {
            return Ok(false);
        }
        let horde = g
            .current_formation
            .as_ref()
            .is_some_and(|f| f.kind == crate::group::formation::HORDE);
        if (g.motion.flags & 2 != 0 && g.flags & 0x400 == 0 && !horde) || g.flags & 0x8000 != 0 {
            self.set_initial_position(group)?;
            self.groups.get_mut(group).expect("held").flags &= !0x8000;
        }
        let g = self.groups.get_mut(group).expect("held");
        g.unit_types.retain(|s| !s.units.is_empty());
        let budget = g.flags & 0x100 == 0;
        self.motion_update(group, budget)?;
        Err(Unsupported(
            "BUnitGroup::update after BMotion's update (FUN_14103d910 from 0x14103e014)",
        ))
    }

    /// `FUN_141007ca0`: the units the group keeps apart (`+0x458`, the save's `objId`, size
    /// `+0x468`), in order. One the world does not hold leaves the list (`FUN_140a63bd0`); one
    /// that may not be interrupted (slot 0x2d0 with 1, [`Turns::may_interrupt`]) stays. One whose
    /// unit AI attacks (its action 600) looks for the target of a member's top action (slot
    /// 0x190: the action's target id when not negative, `FUN_140a78450`) nearest it (the
    /// footprints' squared gap, `FUN_14118f8c0`) within its reach squared (`MAXSS` of its slot
    /// 0x2c8, the master's search radius, a static object's slot 0x210, its line of sight, and its
    /// master's line of sight; `COMISS` then `JBE`: a NaN is no nearer), the first of equals; any
    /// other, and an attacker that found none, leaves the list and stands (slot 0x560,
    /// [`crate::movement::Mover::stand`]).
    ///
    /// # Errors
    ///
    /// An attacker that found a member's target (the group's check of it, `FUN_14100d2a0`, and
    /// the unit AI's `AttackObject`, slot 0xb8, are not modelled), and what the checks read that
    /// the slice does not hold.
    ///
    /// # Panics
    ///
    /// When the world does not hold `group`.
    pub(crate) fn keep_apart(&mut self, group: i32) -> Result<(), Unsupported> {
        let mut i = 0;
        loop {
            let g = self.groups.get(group).expect("the caller holds the group");
            let Some(&id) = g.objects.get(i) else {
                break;
            };
            let Some(object) = self.world_object(id) else {
                let _ = self.groups.get_mut(group).expect("held").objects.remove(i);
                continue;
            };
            if !self.may_interrupt(object)? {
                i += 1;
                continue;
            }
            if let WorldObject::Unit(p, u) = object
                && u.ai
                    .as_ref()
                    .is_some_and(|ai| ai.current_action == ATTACK_ACTION)
            {
                let master = self.master_of(p, u.life.master_id)?;
                let search = if master.kind < ACTION_KIND {
                    master.line_of_sight
                } else {
                    master.search_radius
                };
                let reach = crate::fmath::maxss(search, master.line_of_sight);
                let reach = reach * reach;
                let mut nearest = FARTHEST;
                let mut found = false;
                for m in &g.members {
                    let Some(other) = self.world_object(m.id) else {
                        continue;
                    };
                    if other.id() == object.id() {
                        continue;
                    }
                    let Some(target) = self.action_target(other)? else {
                        continue;
                    };
                    let gap = self.footprint_gap_squared(object, target)?;
                    if reach > gap && nearest > gap {
                        nearest = gap;
                        found = true;
                    }
                }
                if found {
                    return Err(Unsupported(
                        "a kept-apart attacker near a member's target (FUN_14100d2a0, AttackObject)",
                    ));
                }
            }
            let _ = self.groups.get_mut(group).expect("held").objects.remove(i);
            if let Some((_, u)) = self.object_mut(id)
                && let Some(mover) = u.mover.as_mut()
            {
                mover.stand()?;
            }
        }
        Ok(())
    }

    /// Slot 0x2d0 with 1 of `object`: false for an object below type 40 (a ghost's, a static,
    /// animated or moving object's `FUN_141071d80`); an action object's (`FUN_14106d170`) is
    /// false while its group (`+0x6d0`, by id) is held and not at rest ([`Turns::at_rest`],
    /// `FUN_141007a50`) or its unit AI has queued orders (`+0x18`), else whether its top action
    /// may be interrupted ([`crate::turn::Unit::interruptible`]).
    ///
    /// # Errors
    ///
    /// An object of a master the slice does not have, and what [`crate::turn::Unit::interruptible`] does not
    /// model.
    fn may_interrupt(&self, object: WorldObject<'_>) -> Result<bool, Unsupported> {
        let WorldObject::Unit(p, u) = object else {
            return Ok(false);
        };
        if self.master_of(p, u.life.master_id)?.kind < ACTION_KIND {
            return Ok(false);
        }
        if u.group.id != -1 && self.groups.get(u.group.id).is_some() && !self.at_rest(u.group.id) {
            return Ok(false);
        }
        if u.ai.as_ref().is_some_and(|ai| !ai.orders.is_empty()) {
            return Ok(false);
        }
        u.interruptible()
    }

    /// `FUN_141007a50` of group `group`: at rest without a command (-1), its motion not moving
    /// ([`crate::group::Motion::moving`]) and no member the world holds moving (slot 0x5c8,
    /// [`crate::movement::Mover::is_moving`]; a static object's `FUN_141071d90` 0). False for a
    /// group the world does not hold.
    #[must_use]
    pub fn at_rest(&self, group: i32) -> bool {
        let Some(g) = self.groups.get(group) else {
            return false;
        };
        if g.current_command != NO_COMMAND || g.motion.moving() {
            return false;
        }
        !g.members.iter().any(|m| {
            matches!(self.world_object(m.id), Some(WorldObject::Unit(_, u))
                if u.mover.as_ref().is_some_and(crate::movement::Mover::is_moving))
        })
    }

    /// Slot 0x190 of `object`: an action object's top action's target (`FUN_14106d090` ->
    /// `FUN_140a78450`: its target id, `+0x30`, when not negative, by the world's table); none
    /// for any other object (`FUN_141071d40`) or without a top action.
    ///
    /// # Errors
    ///
    /// An object of a master the slice does not have, or with an action the slice does not
    /// model.
    fn action_target(
        &self,
        object: WorldObject<'_>,
    ) -> Result<Option<WorldObject<'_>>, Unsupported> {
        let WorldObject::Unit(p, u) = object else {
            return Ok(None);
        };
        if self.master_of(p, u.life.master_id)?.kind < ACTION_KIND {
            return Ok(None);
        }
        if let Some(e) = u.unmodelled_action {
            return Err(e);
        }
        Ok(u.top_target()
            .filter(|&t| -1 < t)
            .and_then(|t| self.world_object(t)))
    }

    /// `FUN_14118f8c0` of `a` and `b`: the squared gap of their footprints (their masters'
    /// radii, `+0x5d8` and `+0x5dc`; [`crate::movement::footprint_gap_squared`]).
    ///
    /// # Errors
    ///
    /// An object of a master the slice does not have.
    fn footprint_gap_squared(
        &self,
        a: WorldObject<'_>,
        b: WorldObject<'_>,
    ) -> Result<f32, Unsupported> {
        let ra = self.master_of(a.player(), a.master_id())?.radius;
        let rb = self.master_of(b.player(), b.master_id())?.radius;
        Ok(crate::movement::footprint_gap_squared(
            a.position(),
            ra,
            b.position(),
            rb,
        ))
    }

    /// `FUN_1410176b0` of group `group`: whether its target is still to act on. None without
    /// members (or a group the world does not hold). A relic's pick-up (132) and delivery (136)
    /// ask their own ([`Turns::relic_pickup_pending`], `FUN_141017c70`;
    /// [`Turns::relic_delivery_pending`], `FUN_141017850`). Otherwise the target (`+0x3c0`) the
    /// world holds, in a state up to 2 (0 for a build, 101), inside no other object
    /// (`FUN_141188da0`), of a player the owner does not count an ally (`FUN_141663d90` with the
    /// ally flag; a build skips this) and seen by the owner ([`Turns::sees_object`],
    /// `FUN_141188ea0`).
    ///
    /// # Errors
    ///
    /// An object of a master the slice does not have, or what the sight test does not hold.
    pub fn target_pending(&self, group: i32) -> Result<bool, Unsupported> {
        let Some(g) = self.groups.get(group) else {
            return Ok(false);
        };
        if g.members.is_empty() {
            return Ok(false);
        }
        match g.current_command {
            PICK_UP_RELIC => return self.relic_pickup_pending(g),
            DELIVER_RELIC => return self.relic_delivery_pending(g),
            _ => {}
        }
        let Some(target) = self.world_object(g.current_target.target) else {
            return Ok(false);
        };
        let owner = g.owner as usize;
        let most = if g.current_command == BUILD { 0 } else { 2 };
        if most < target.state() || target.inside() {
            return Ok(false);
        }
        if g.current_command != BUILD && self.spatial.allied_with(owner, target.player()) {
            return Ok(false);
        }
        self.sees_object(target, owner)
    }

    /// `FUN_141017c70`: a member the world holds with a unit AI whose order is `0x2db` or whose
    /// action is `0x277`, on a group still picking up a relic (132), whose master's tasks
    /// ([`Turns::has_task`]) pick up relics (132) and its target (the unit AI's, `+0x48`) is a
    /// relic (its master's class `0x2a`), or deliver them (136) and its target is a relic or
    /// none the world holds.
    fn relic_pickup_pending(&self, g: &UnitGroup) -> Result<bool, Unsupported> {
        let owner = g.owner as usize;
        for m in &g.members {
            let Some(WorldObject::Unit(_, u)) = self.world_object(m.id) else {
                continue;
            };
            let Some(ai) = u.ai.as_ref() else {
                continue;
            };
            let picks = self.has_task(owner, u.life.master_id, RELIC_PICKUP_ACTION)?;
            let delivers = self.has_task(owner, u.life.master_id, RELIC_DELIVERY_ACTION)?;
            let target = self.world_object(ai.current_target);
            let relic = match target {
                Some(t) => self.master_of(t.player(), t.master_id())?.class == RELIC_CLASS,
                None => false,
            };
            if g.current_command == PICK_UP_RELIC
                && (ai.current_order == TARGET_ORDER || ai.current_action == RELIC_ACTION)
                && ((picks && relic) || (delivers && (relic || target.is_none())))
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// `FUN_141017850`: as [`Turns::relic_pickup_pending`], on a group still delivering a relic
    /// (136): the target is a monastery (master 104) and the member's master's tasks deliver or
    /// pick up relics.
    fn relic_delivery_pending(&self, g: &UnitGroup) -> Result<bool, Unsupported> {
        let owner = g.owner as usize;
        for m in &g.members {
            let Some(WorldObject::Unit(_, u)) = self.world_object(m.id) else {
                continue;
            };
            let Some(ai) = u.ai.as_ref() else {
                continue;
            };
            let picks = self.has_task(owner, u.life.master_id, RELIC_PICKUP_ACTION)?;
            let delivers = self.has_task(owner, u.life.master_id, RELIC_DELIVERY_ACTION)?;
            let monastery = self
                .world_object(ai.current_target)
                .is_some_and(|t| t.master_id() == MONASTERY);
            if g.current_command == DELIVER_RELIC
                && (ai.current_order == TARGET_ORDER || ai.current_action == RELIC_ACTION)
                && monastery
                && (delivers || picks)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// `FUN_1415c60b0`: player `player` has master `master` (`FUN_141659d20`), of a type above
    /// 39, with a task of action type `action` (`+0xe` of its task list, `+0x788`).
    ///
    /// # Errors
    ///
    /// A player whose masters the slice does not have.
    fn has_task(&self, player: usize, master: i16, action: i16) -> Result<bool, Unsupported> {
        let Some(masters) = self.masters.get(player) else {
            return self.master_of(player, master).map(|_| false);
        };
        Ok(masters
            .get(master)
            .is_some_and(|m| 39 < m.kind && m.tasks.iter().any(|t| t.action_type == action)))
    }

    /// `FUN_141188ea0` of `object` for player `player`: its tile (`CVTTSS2SI` of its x and y)
    /// seen by the player now (slot 0x1e0, `FUN_1416635e0`, 0xf: [`crate::spatial::Spatial::
    /// sees_tile`]), or, for a master whose `fog_visibility` (`+0x605`) is set, seen or explored
    /// ([`crate::spatial::Spatial::explored`]).
    ///
    /// # Errors
    ///
    /// An object of a master the slice does not have, and the explored test without the
    /// players' sight.
    fn sees_object(&self, object: WorldObject<'_>, player: usize) -> Result<bool, Unsupported> {
        let master = self.master_of(object.player(), object.master_id())?;
        let [x, y, _] = object.position();
        let (x, y) = (
            crate::checkpoint::cvttss2si(x),
            crate::checkpoint::cvttss2si(y),
        );
        if master.fog_visibility == 0 {
            Ok(self.spatial.sees_tile(player, x, y))
        } else {
            self.spatial.explored(player, x, y)
        }
    }

    /// `FUN_141017f00` of object `id` (the group it gets is not read): whether the member is
    /// free for the group's moves. Not for an object the world does not hold. Its top action's
    /// state (slot 0x2b0; 0 without one, and for an object below type 40): above 1, state 2 is
    /// free for a trade cart (class 19) or a trade boat (2) and not for an Attack (slot 0x530,
    /// type 9), and states 3, 13 and 14 go on to the unit AI, any other is not free. A unit AI
    /// whose action is not `0x2c0` ([`crate::unit_ai::GROUP_ASIDE_ACTION`]) is not free while
    /// its action (its order for action `0x25b`) is not -1; without a unit AI it is free.
    ///
    /// # Errors
    ///
    /// An object of a master the slice does not have, or with an action the slice does not
    /// model.
    pub fn member_idle(&self, id: i32) -> Result<bool, Unsupported> {
        let Some(object) = self.world_object(id) else {
            return Ok(false);
        };
        let WorldObject::Unit(p, u) = object else {
            return Ok(true);
        };
        let master = self.master_of(p, u.life.master_id)?;
        let state = if master.kind < ACTION_KIND {
            0
        } else {
            if let Some(e) = u.unmodelled_action {
                return Err(e);
            }
            u.top_action().map_or(0, |a| a.state)
        };
        if 1 < state {
            if state == 2 {
                if master.class == TRADE_CART || master.class == TRADE_BOAT {
                    return Ok(true);
                }
                if u.top_action_kind() == Some(crate::attack::ATTACK) {
                    return Ok(false);
                }
            } else if !matches!(state, 3 | 13 | 14) {
                return Ok(false);
            }
        }
        if let Some(ai) = u.ai.as_ref() {
            let mut action = ai.current_action;
            if action != crate::unit_ai::GROUP_ASIDE_ACTION {
                if action == CHOSEN_ACTION {
                    action = ai.current_order;
                }
                if action != -1 {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    /// Slot 0x660 of object `id`: it shows its standing sprite. Nothing for an object below type
    /// 30 (`FUN_141072270`) or one the world does not hold. An action object's
    /// (`FUN_14106e2e0`) asks its top action first (slot 0x430, then the action's slot 0x58 with
    /// 3, which only a trade's, `FUN_140ae22b0`, answers: the slice holds no trades); a moving
    /// object's (`FUN_14115cf80`) shows its standing sprite (slot 0x798: its master's, `+0x470`;
    /// a trade cart's with goods, `FUN_14106a940`, is its trade task's, and the unit's own
    /// `+0x8a9` override is not held) when it has one (slot 0x68, as [`crate::order`]'s: the top
    /// speed is the standing sprite's, [`crate::movement::Mover::shows_standing`], and the
    /// sprite itself where the slice holds it).
    ///
    /// # Errors
    ///
    /// An object of a master the slice does not have, with an action the slice does not model,
    /// a trade cart (whose goods the slice does not hold), and a standing sprite the slice's
    /// graphics lack.
    pub(crate) fn show_standing(&mut self, id: i32) -> Result<(), Unsupported> {
        let Some((p, u)) = self.object(id) else {
            return Ok(());
        };
        let master = self.master_of(p, u.life.master_id)?;
        if master.kind < MOVING_KIND {
            return Ok(());
        }
        if ACTION_KIND <= master.kind
            && let Some(e) = u.unmodelled_action
        {
            return Err(e);
        }
        if master.kind == COMBAT_KIND && master.class == TRADE_CART {
            return Err(Unsupported(
                "a trade cart's standing sprite (its goods, FUN_14106a940)",
            ));
        }
        let standing = master.standing_sprite;
        if standing == -1 {
            return Ok(());
        }
        let graphic = self.graphics.get(&standing).copied();
        let (_, u) = self.object_mut(id).expect("found above");
        if let Some(m) = u.mover.as_mut() {
            m.shows_standing();
        }
        if u.sprite.is_none() {
            return Ok(());
        }
        let g = graphic.ok_or(Unsupported("a unit without its standing sprite"))?;
        let place = self.place(id).expect("found above");
        let rng = self.streams.get(crate::rng::Stream::Sprite);
        let u = match place {
            crate::order::Place::Live(p, i) => &mut self.players[p].objects[i],
            crate::order::Place::Sleeping(p, i) => &mut self.players[p].sleeping[i],
        };
        crate::attack::set_sprite(&mut u.sprite, g, rng);
        Ok(())
    }

    /// `FUN_141016870`: the group's lead (`+0xc4`) is the member the world holds nearest (in 3D,
    /// slot 0x520's place) the target's place (`mCurrentTarget`'s original position), the first
    /// of equals; -1 without one.
    #[must_use]
    pub fn nearest_to_target(&self, group: i32) -> i32 {
        let Some(g) = self.groups.get(group) else {
            return -1;
        };
        let to = g.current_target.original_position;
        let mut best = f32::MAX;
        let mut lead = -1;
        for m in &g.members {
            let Some((_, u)) = self.object(m.id) else {
                continue;
            };
            // the game's vector (x, height, y) of where it last moved to (x, y, height)
            let at = u
                .mover
                .as_ref()
                .map_or([u.position[0], u.position[2], u.position[1]], |mv| {
                    [mv.moved_at[0], mv.moved_at[2], mv.moved_at[1]]
                });
            let d = [at[0] - to[0], at[1] - to[1], at[2] - to[2]];
            let squared = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
            if squared < best {
                lead = m.id;
                best = squared;
            }
        }
        lead
    }
}

impl Turns {
    /// `FUN_141017630`: a member of group `group` the world holds (false for a group it does not
    /// hold).
    #[must_use]
    pub fn any_member_held(&self, group: i32) -> bool {
        self.groups
            .get(group)
            .is_some_and(|g| g.members.iter().any(|m| self.object(m.id).is_some()))
    }

    /// `resetIdleTimer` (`FUN_14102d370`): group `group`'s idle timer (`+0x3e4`) is a draw of the
    /// World stream (`rand15`, `DAT_143f07040`) modulo [`IDLE_SPAN`]; nothing for a group the
    /// world does not hold (the game draws for its own).
    pub fn reset_idle_timer(&mut self, group: i32) {
        if self.groups.get(group).is_none() {
            return;
        }
        let draw = self.streams.get(crate::rng::Stream::World).rand15();
        if let Some(g) = self.groups.get_mut(group) {
            g.idle_timer = draw % IDLE_SPAN;
        }
    }

    /// `IsIdle` (`FUN_141017ad0`) of group `group`: idle with flags bit 5 (`cConsiderIdle`), or on a
    /// move (command 1) along a patrol (the top path's flags bit 0, `FUN_141046500`) or with
    /// flags bit 11; else not while its motion moves or waits to move; else idle. False for a
    /// group the world does not hold.
    #[must_use]
    pub fn is_idle(&self, group: i32) -> bool {
        let Some(g) = self.groups.get(group) else {
            return false;
        };
        if g.flags >> 5 & 1 != 0 {
            return true;
        }
        let patrol = crate::group::formation::top_path(&g.motion.paths)
            .is_some_and(|p| p.path_flags & 1 != 0);
        if g.current_command == 1 && (patrol || g.flags >> 11 & 1 != 0) {
            return true;
        }
        !(g.motion.moving() || g.motion.waits_to_move())
    }
}

/// The idle timer's span (`FUN_14102d370`'s `% 0x2ee`, its division at `0x14102d395`).
pub const IDLE_SPAN: u32 = 750;

#[cfg(test)]
mod tests {
    use crate::group::{Member, UnitGroup, UnitGroups};
    use crate::masters::{Master, Masters, Task};
    use crate::turn::{Player, Turns, Unit};
    use crate::unit_ai::UnitAi;

    /// A unit of master 4 at (`x`, `y`) in state 2 with 50 hit points and a unit AI.
    fn soldier(id: i32, x: f32, y: f32) -> Unit {
        let mut u = Unit {
            id,
            position: [x, y, 0.0],
            ai: Some(UnitAi {
                current_order: -1,
                current_action: -1,
                current_target: -1,
                ..UnitAi::default()
            }),
            ..Unit::default()
        };
        u.life.master_id = 4;
        u.life.state = 2;
        u.life.hp = 50.0;
        u
    }

    /// Player 1 holds soldiers 10, 11 and 12 (master 4: a combat unit of class 6, 60 hit
    /// points, line of sight 4, radius 0.2) in group 1; player 2, its enemy, holds 20 at (3, 3)
    /// and player 3, its ally, 30 at (4, 4). An 8 by 8 map whose tiles player 1 sees none of.
    fn world() -> Turns {
        let players = vec![
            Player::default(),
            Player {
                objects: vec![
                    soldier(10, 1.0, 1.0),
                    soldier(11, 2.0, 1.0),
                    soldier(12, 1.0, 2.0),
                ],
                ..Player::default()
            },
            Player {
                objects: vec![soldier(20, 3.5, 3.5)],
                ..Player::default()
            },
            Player {
                objects: vec![soldier(30, 4.5, 4.5)],
                ..Player::default()
            },
        ];
        let mut t = Turns::new(0, 0, 0, vec![0; 4], players);
        let master = Master {
            id: 4,
            kind: 70,
            class: 6,
            hit_points: 60,
            line_of_sight: 4.0,
            search_radius: 3.0,
            radius: [0.2, 0.2],
            standing_sprite: -1,
            ..Master::default()
        };
        t.masters = vec![Masters::new([master]); 4];
        let mut s = crate::spatial::Spatial::new([8, 8]);
        s.relations = vec![
            vec![0, 3, 3, 3],
            vec![0, 0, 3, 0],
            vec![0, 3, 0, 3],
            vec![0, 0, 3, 0],
        ];
        s.sight = Some(crate::visibility::Visibility::new(
            8,
            8,
            Vec::new(),
            vec![0; 64],
            Vec::new(),
            0,
            Vec::new(),
        ));
        t.spatial = s;
        let member = |id| Member {
            id,
            ..Member::default()
        };
        let mut g = UnitGroup::new(1);
        g.owner = 1;
        g.members = vec![member(10), member(11), member(12)];
        for u in &mut t.players[1].objects {
            u.group.id = 1;
        }
        t.groups = UnitGroups {
            groups: vec![g],
            ..UnitGroups::default()
        };
        t
    }

    /// Player `p` sees tile (`x`, `y`) (`seen`) or only explored it.
    fn light(t: &mut Turns, p: usize, x: usize, y: usize, seen: bool) {
        let v = t.spatial.sight.as_mut().unwrap();
        v.unified[y * 8 + x] |= if seen { 1 << p } else { 1 << (32 + p) };
    }

    /// The members' check of `FUN_14103d910`: 11 below 1 hit point, 12 of another player, 13
    /// without a unit AI and 99 the world does not hold leave the group (their links clear and
    /// the held ones stop, slot 0x5d0), which sets flags bit 16; 10 stays.
    #[test]
    fn lost_members_leave_the_group_and_stop() {
        let mut t = world();
        t.players[1].objects[1].life.hp = 0.5;
        let mut stray = soldier(12, 1.0, 2.0);
        stray.group.id = 1;
        let _ = t.players[1].objects.remove(2);
        t.players[2].objects.push(stray);
        let mut mute = soldier(13, 2.0, 2.0);
        mute.ai = None;
        mute.group.id = 1;
        t.players[1].objects.push(mute);
        for u in &mut t.players[1].objects {
            u.mover = Some(crate::hunt::tests::walker());
        }
        let g = t.groups.get_mut(1).unwrap();
        for id in [13, 99] {
            g.members.push(Member {
                id,
                ..Member::default()
            });
        }
        g.flags = 0;
        t.drop_lost_members(1).unwrap();
        let g = t.groups.get(1).unwrap();
        assert_eq!(g.members.iter().map(|m| m.id).collect::<Vec<_>>(), [10]);
        assert_eq!(g.flags & 0x1_0000, 0x1_0000);
        let unit = |t: &Turns, id| t.object(id).unwrap().1.clone();
        assert_eq!(unit(&t, 10).group.id, 1);
        for id in [11, 12, 13] {
            assert_eq!(unit(&t, id).group.id, -1, "{id}");
        }
        let stopped = unit(&t, 11).mover.unwrap();
        assert!(stopped.speed_stopped);
        assert_eq!(stopped.velocity.unwrap().velocity[2], 0.0);
        // a NaN's hit points stay (COMISS then JA)
        let mut t = world();
        t.players[1].objects[0].life.hp = f32::NAN;
        t.drop_lost_members(1).unwrap();
        assert_eq!(t.groups.get(1).unwrap().members.len(), 3);
        assert_eq!(t.groups.get(1).unwrap().flags & 0x1_0000, 0);
    }

    /// A hibernating group by its command: without one or on a move it runs its queued commands
    /// when it has any; a build waits on its own target below state 2; a repair waits on an
    /// allied target below its hit points (a NaN too) and runs the queue otherwise; an attack
    /// waits while its target is pending; a follow (2) and a command above 106 but the relics'
    /// go.
    #[test]
    fn a_hibernating_group_waits_runs_its_queue_or_goes() {
        let mut t = world();
        let set = |t: &mut Turns, command: i32, target: i32, queued: bool| {
            let g = t.groups.get_mut(1).unwrap();
            g.current_command = command;
            g.current_target.target = target;
            g.command_queue = if queued {
                vec![crate::group::GroupCommand::default()]
            } else {
                Vec::new()
            };
        };
        for command in [-1, 1] {
            set(&mut t, command, -1, false);
            assert_eq!(t.hibernation(1).unwrap(), Some(false));
            set(&mut t, command, -1, true);
            assert_eq!(t.hibernation(1).unwrap(), Some(true));
        }
        for command in [2, 3, 0x69, 0x6b, 0x2711] {
            set(&mut t, command, -1, false);
            assert_eq!(t.hibernation(1).unwrap(), None, "{command}");
        }
        // a build on the group's own 11 in state 1 waits, in state 2 or of another player not
        set(&mut t, 0x65, 11, false);
        t.players[1].objects[1].life.state = 1;
        assert_eq!(t.hibernation(1).unwrap(), Some(false));
        t.players[1].objects[1].life.state = 2;
        assert_eq!(t.hibernation(1).unwrap(), Some(true));
        set(&mut t, 0x65, 20, false);
        t.players[2].objects[0].life.state = 0;
        assert_eq!(t.hibernation(1).unwrap(), Some(true));
        // a repair of the ally's 30 below its 60 hit points waits; at them, or of the enemy's
        // 20, or of nothing, the queue runs
        set(&mut t, 0x6a, 30, false);
        assert_eq!(t.hibernation(1).unwrap(), Some(false));
        t.players[3].objects[0].life.hp = f32::NAN;
        assert_eq!(t.hibernation(1).unwrap(), Some(false));
        t.players[3].objects[0].life.hp = 60.0;
        assert_eq!(t.hibernation(1).unwrap(), Some(true));
        set(&mut t, 0x6a, 20, false);
        assert_eq!(t.hibernation(1).unwrap(), Some(true));
        set(&mut t, 0x6a, 77, false);
        assert_eq!(t.hibernation(1).unwrap(), Some(true));
        // an attack on the enemy's 20 waits while player 1 sees it
        set(&mut t, 9, 20, false);
        assert_eq!(t.hibernation(1).unwrap(), Some(true));
        light(&mut t, 1, 3, 3, true);
        assert_eq!(t.hibernation(1).unwrap(), Some(false));
    }

    /// `FUN_1410176b0`: a target the world holds, in state 2 at most (0 for a build), inside no
    /// other object, of a player the owner does not count an ally, that the owner sees (or only
    /// explored, for a master with `fog_visibility`).
    #[test]
    fn a_target_is_pending_while_seen_and_no_ally_s() {
        let mut t = world();
        let target = |t: &mut Turns, command: i32, id: i32| {
            let g = t.groups.get_mut(1).unwrap();
            g.current_command = command;
            g.current_target.target = id;
        };
        target(&mut t, 9, 20);
        assert!(!t.target_pending(1).unwrap());
        light(&mut t, 1, 3, 3, false);
        assert!(!t.target_pending(1).unwrap());
        for m in &mut t.masters {
            let mut master = m.get(4).unwrap().clone();
            master.fog_visibility = 1;
            *m = Masters::new([master]);
        }
        assert!(t.target_pending(1).unwrap());
        t.players[2].objects[0].life.state = 3;
        assert!(!t.target_pending(1).unwrap());
        t.players[2].objects[0].life.state = 2;
        t.players[2].objects[0].life.inside = Some(5);
        assert!(!t.target_pending(1).unwrap());
        t.players[2].objects[0].life.inside = None;
        assert!(t.target_pending(1).unwrap());
        // an ally's: not pending, but for a build in state 0
        light(&mut t, 1, 4, 4, true);
        target(&mut t, 9, 30);
        assert!(!t.target_pending(1).unwrap());
        target(&mut t, 0x65, 30);
        assert!(!t.target_pending(1).unwrap());
        t.players[3].objects[0].life.state = 0;
        assert!(t.target_pending(1).unwrap());
        // without members, none
        t.groups.get_mut(1).unwrap().members.clear();
        assert!(!t.target_pending(1).unwrap());
        assert!(!t.target_pending(2).unwrap());
    }

    /// `FUN_141017c70` and `FUN_141017850`: a member whose unit AI works on a relic (order
    /// `0x2db` or action `0x277`) keeps a pick-up pending while its master picks up relics and
    /// its target is a relic (class `0x2a`), or delivers them and it has a relic or no target;
    /// a delivery pending on a monastery (master 104).
    #[test]
    fn relic_carriers_keep_their_command_pending() {
        let mut t = world();
        let relic = Master {
            id: 285,
            kind: 70,
            class: 0x2a,
            ..Master::default()
        };
        let monastery = Master {
            id: 104,
            kind: 80,
            class: 3,
            ..Master::default()
        };
        let task = |action| Task {
            action_type: action,
            ..Task::default()
        };
        let monk = |tasks: Vec<Task>| Master {
            id: 4,
            kind: 70,
            class: 18,
            tasks,
            ..Master::default()
        };
        let set_masters = |t: &mut Turns, tasks: Vec<Task>| {
            t.masters = vec![Masters::new([monk(tasks), relic.clone(), monastery.clone()]); 4];
        };
        let mut r = soldier(40, 6.0, 6.0);
        r.life.master_id = 285;
        let mut m = soldier(41, 7.0, 7.0);
        m.life.master_id = 104;
        t.players[0].objects.extend([r, m]);
        let ai = |t: &mut Turns, order: i32, action: i32, target: i32| {
            let a = t.players[1].objects[0].ai.as_mut().unwrap();
            a.current_order = order;
            a.current_action = action;
            a.current_target = target;
        };
        t.groups.get_mut(1).unwrap().current_command = 0x84;
        set_masters(&mut t, vec![task(0x84)]);
        ai(&mut t, 0x2db, -1, 40);
        assert!(t.target_pending(1).unwrap());
        ai(&mut t, -1, 0x277, 40);
        assert!(t.target_pending(1).unwrap());
        ai(&mut t, -1, -1, 40);
        assert!(!t.target_pending(1).unwrap());
        ai(&mut t, 0x2db, -1, -1);
        assert!(!t.target_pending(1).unwrap());
        set_masters(&mut t, vec![task(0x88)]);
        assert!(t.target_pending(1).unwrap());
        ai(&mut t, 0x2db, -1, 41);
        assert!(!t.target_pending(1).unwrap());
        // the delivery: on the monastery, with either task
        t.groups.get_mut(1).unwrap().current_command = 0x88;
        assert!(t.target_pending(1).unwrap());
        set_masters(&mut t, vec![task(0x84)]);
        assert!(t.target_pending(1).unwrap());
        set_masters(&mut t, vec![task(5)]);
        assert!(!t.target_pending(1).unwrap());
        set_masters(&mut t, vec![task(0x88)]);
        ai(&mut t, 0x2db, -1, 40);
        assert!(!t.target_pending(1).unwrap());
    }

    /// `FUN_141017f00`: a member is free by its top action's state and its unit AI; an object
    /// the world does not hold is not.
    #[test]
    fn a_member_is_free_by_its_action_and_its_unit_ai() {
        let mut t = world();
        assert!(t.member_idle(10).unwrap());
        assert!(!t.member_idle(99).unwrap());
        let ai = |t: &mut Turns, order: i32, action: i32| {
            let a = t.players[1].objects[0].ai.as_mut().unwrap();
            a.current_order = order;
            a.current_action = action;
        };
        ai(&mut t, -1, 0x2c6);
        assert!(!t.member_idle(10).unwrap());
        ai(&mut t, 7, 0x2c0);
        assert!(t.member_idle(10).unwrap());
        ai(&mut t, -1, 0x25b);
        assert!(t.member_idle(10).unwrap());
        ai(&mut t, 0x2c6, 0x25b);
        assert!(!t.member_idle(10).unwrap());
        ai(&mut t, -1, -1);
        t.players[1].objects[0].ai = None;
        assert!(t.member_idle(10).unwrap());
        // an Attack in state 2 is not free, in states 3 and 13 it is; a state 4 is not
        let attack = |state| crate::attack::Attack {
            target: -1,
            target2: -1,
            state,
            timer: 0.0,
            range: 0.0,
            min_range: 0.0,
            point: [0.0; 3],
            seen: [0.0; 3],
            move_to: None,
            move_wait: crate::move_to::MoveWait::default(),
            pending: false,
            swing: 0.0,
            rest: 0.0,
            arms: crate::attack::Arms {
                move_sprite: None,
                sprite: None,
                second_sprite: None,
                idle_sprite: None,
                idle_attack_sprite: None,
                missile: -1,
                fire_frame: -1,
                interrupt_frame: -1,
            },
            sub_action: false,
            repaths: false,
            runs: false,
        };
        for (state, free) in [(2, false), (3, true), (13, true), (14, true), (4, false)] {
            t.players[1].objects[0].attack = Some(attack(state));
            assert_eq!(t.member_idle(10).unwrap(), free, "{state}");
        }
        // a trade cart's state 2 is free, whatever its action
        t.players[1].objects[0].attack = Some(attack(2));
        for m in &mut t.masters {
            let mut master = m.get(4).unwrap().clone();
            master.class = 19;
            *m = Masters::new([master]);
        }
        assert!(t.member_idle(10).unwrap());
    }

    /// `FUN_141007a50`: a group rests without a command, its motion standing and no held
    /// member moving.
    #[test]
    fn a_group_rests_without_command_motion_or_moving_member() {
        let mut t = world();
        t.groups.get_mut(1).unwrap().current_command = -1;
        assert!(t.at_rest(1));
        t.players[1].objects[1].mover = Some(crate::hunt::tests::walker());
        assert!(!t.at_rest(1));
        t.players[1].objects[1].mover = None;
        t.groups.get_mut(1).unwrap().motion.velocity = [0.0, 0.0, 1.0];
        assert!(!t.at_rest(1));
        t.groups.get_mut(1).unwrap().motion.velocity = [0.0; 3];
        t.groups.get_mut(1).unwrap().current_command = 1;
        assert!(!t.at_rest(1));
        assert!(!t.at_rest(2));
    }

    /// `FUN_141007ca0`: a kept-apart unit the world does not hold leaves the list; one that may
    /// not be interrupted (its unit AI has queued orders) stays; any other leaves and stands,
    /// as does an attacker with no member's target within its reach.
    #[test]
    fn kept_apart_units_leave_unless_busy() {
        let mut t = world();
        let mut apart = soldier(15, 6.0, 6.0);
        apart.mover = Some(crate::hunt::tests::walker());
        apart.ai.as_mut().unwrap().orders = vec![crate::unit_ai::QueuedOrder::default()];
        let mut free = soldier(16, 6.0, 6.0);
        free.mover = Some(crate::hunt::tests::walker());
        let mut attacker = soldier(17, 7.0, 7.0);
        attacker.ai.as_mut().unwrap().current_action = 600;
        t.players[1].objects.extend([apart, free, attacker]);
        t.groups.get_mut(1).unwrap().objects = vec![99, 15, 16, 17];
        t.keep_apart(1).unwrap();
        assert_eq!(t.groups.get(1).unwrap().objects, [15]);
        assert!(
            t.object(16)
                .unwrap()
                .1
                .mover
                .as_ref()
                .unwrap()
                .speed_stopped
        );
        assert!(
            !t.object(15)
                .unwrap()
                .1
                .mover
                .as_ref()
                .unwrap()
                .speed_stopped
        );
        // a member's group moving keeps the unit too
        t.players[1].objects[3].ai.as_mut().unwrap().orders.clear();
        t.players[1].objects[3].group.id = 1;
        t.groups.get_mut(1).unwrap().current_command = 1;
        t.keep_apart(1).unwrap();
        assert_eq!(t.groups.get(1).unwrap().objects, [15]);
        t.groups.get_mut(1).unwrap().current_command = -1;
        t.keep_apart(1).unwrap();
        assert_eq!(t.groups.get(1).unwrap().objects, Vec::<i32>::new());
    }

    /// The group's deletion: its held members whose link names it leave it; an idle one stands
    /// and shows standing, a busy one walks on. A group without members goes at its update.
    #[test]
    fn a_deleted_group_lets_its_members_go() {
        let mut t = world();
        for u in &mut t.players[1].objects {
            u.mover = Some(crate::hunt::tests::walker());
        }
        t.players[1].objects[1].ai.as_mut().unwrap().current_action = 0x2c6;
        t.players[1].objects[2].group.id = 7;
        t.delete_group(1).unwrap();
        assert!(t.groups.get(1).is_none());
        let unit = |t: &Turns, id| t.object(id).unwrap().1.clone();
        assert_eq!([10, 11, 12].map(|id| unit(&t, id).group.id), [-1, -1, 7]);
        assert!(unit(&t, 10).mover.unwrap().speed_stopped);
        assert!(!unit(&t, 11).mover.unwrap().speed_stopped);
        assert!(!unit(&t, 12).mover.unwrap().speed_stopped);
        // the update of a group without members deletes it
        let mut t = world();
        t.groups.get_mut(1).unwrap().members.clear();
        assert!(!t.group_update(1).unwrap());
        t.update_groups(1).unwrap();
        assert!(t.groups.get(1).is_none());
    }

    /// The awake update's ends: an idle group (no command, none queued) whose formation's slots
    /// are all in state 3 goes, one with flags bit 6 goes, and one whose lead the world does
    /// not hold goes; with every member kept apart and nothing pending, each leaves the group.
    #[test]
    fn an_idle_group_goes() {
        let mut t = world();
        let g = t.groups.get_mut(1).unwrap();
        g.current_command = -1;
        g.flags = 0;
        g.current_formation = Some(crate::group::Formation {
            kind: 1,
            slots: vec![crate::group::Slot {
                state: 3,
                ..crate::group::Slot::default()
            }],
            ..crate::group::Formation::default()
        });
        assert!(!t.group_update(1).unwrap());
        let mut t = world();
        t.groups.get_mut(1).unwrap().flags = 0x40;
        assert!(!t.group_update(1).unwrap());
        // every member kept apart: each leaves, then the group has none
        let mut t = world();
        let g = t.groups.get_mut(1).unwrap();
        g.flags = 0;
        g.objects = vec![10, 11, 12];
        g.current_command = 1;
        assert!(!t.group_update(1).unwrap());
        assert_eq!(t.groups.get(1).unwrap().members, Vec::new());
        assert_eq!(t.groups.get(1).unwrap().objects, Vec::<i32>::new());
        assert_eq!(t.object(11).unwrap().1.group.id, -1);
    }

    /// `FUN_141040b50`: a motion that moved (bit 0) stores its new position at the front; bit 1
    /// moves to the group's flags, and both motion bits clear. A motion that did not move keeps
    /// its stored positions.
    #[test]
    fn the_update_end_stores_a_moved_group_s_new_position() {
        let mut g = UnitGroup::new(1);
        g.motion.new_position[0] = [3.0, 0.0, 4.0];
        g.motion.flags = 3;
        g.flags = 0;
        g.update_end();
        assert_eq!((g.motion.flags, g.flags & 2), (0, 2));
        assert_eq!(
            g.stored_positions.points.first().map(|p| p.point),
            Some([3.0, 0.0, 4.0])
        );
        let mut still = UnitGroup::new(2);
        still.motion.flags = 0;
        let before = still.stored_positions.points.len();
        still.update_end();
        assert_eq!(still.stored_positions.points.len(), before);
    }

    /// `FUN_141016870`: the lead is the held member nearest (in 3D, where it last moved to) the
    /// target's place, the first of equals; -1 without a held member.
    #[test]
    fn the_lead_is_the_member_nearest_the_targets_place() {
        let at = |id, x: f32, y: f32| Unit {
            id,
            position: [x, y, 0.0],
            ..Unit::default()
        };
        let player = Player {
            objects: vec![at(10, 5.0, 5.0), at(11, 2.0, 1.0), at(12, 1.0, 2.0)],
            ..Player::default()
        };
        let mut t = Turns::new(0, 1, 0, vec![0], vec![player]);
        let member = |id| Member {
            id,
            ..Member::default()
        };
        let mut g = UnitGroup::new(1);
        // 99 is not held; 11 and 12 are as near (1, 1) as each other: 11 comes first
        g.members = vec![member(99), member(10), member(11), member(12)];
        g.current_target.original_position = [1.0, 0.0, 1.0];
        t.groups = UnitGroups {
            groups: vec![g],
            ..UnitGroups::default()
        };
        assert_eq!(t.nearest_to_target(1), 11);
        t.groups.groups[0].members = vec![member(99)];
        assert_eq!(t.nearest_to_target(1), -1);
        assert_eq!(t.nearest_to_target(2), -1);
    }

    /// `FUN_141017630`, `resetIdleTimer` (`FUN_14102d370`) and `IsIdle` (`FUN_141017ad0`).
    #[test]
    fn a_group_holds_members_draws_its_idle_timer_and_tells_when_it_is_idle() {
        let held = Unit {
            id: 10,
            ..Unit::default()
        };
        let player = Player {
            objects: vec![held],
            ..Player::default()
        };
        let mut t = Turns::new(0, 1, 0, vec![0], vec![player]);
        let member = |id| Member {
            id,
            ..Member::default()
        };
        let mut g = UnitGroup::new(1);
        g.members = vec![member(99), member(10)];
        t.groups = UnitGroups {
            groups: vec![g],
            ..UnitGroups::default()
        };
        assert!(t.any_member_held(1));
        t.groups.groups[0].members = vec![member(99)];
        assert!(!t.any_member_held(1));
        assert!(!t.any_member_held(2));
        // the World stream's next draw, modulo 750
        let mut streams = t.streams.clone();
        let draw = streams.get(crate::rng::Stream::World).rand15();
        t.reset_idle_timer(1);
        assert_eq!(t.groups.groups[0].idle_timer, draw % 750);
        assert_eq!(t.streams, streams);
        // idle: standing; not while moving or waiting for a path; idle again with bit 5, or on a
        // move with bit 11
        assert!(t.is_idle(1));
        t.groups.groups[0].motion.velocity = [0.0, 0.0, 1.0];
        assert!(!t.is_idle(1));
        t.groups.groups[0].current_command = 1;
        t.groups.groups[0].flags |= 0x800;
        assert!(t.is_idle(1));
        t.groups.groups[0].flags = 0x20;
        assert!(t.is_idle(1));
        assert!(!t.is_idle(2));
    }

    /// `FUN_14102bb40`: the command queue empties; a hibernating group (1) wakes with flags
    /// `0x4c000`, any other hibernation value wakes without them.
    #[test]
    fn emptying_the_command_queue_wakes_a_hibernating_group() {
        let mut g = UnitGroup::new(1);
        g.command_queue = vec![crate::group::GroupCommand::default()];
        g.hibernating = 1;
        g.clear_command_queue();
        assert_eq!(g.command_queue, Vec::new());
        assert_eq!((g.hibernating, g.flags), (0, 0x100 | 0x4_c000));
        let mut g = UnitGroup::new(1);
        g.hibernating = 2;
        g.clear_command_queue();
        assert_eq!((g.hibernating, g.flags), (0, 0x100));
    }

    /// The long pather's list of a unit in group 1 (whose differentiator groups are 1 and 2) is
    /// 10, 11 and 12; a unit of no group, or of a group the world does not hold, has none.
    #[test]
    fn a_grouped_units_list_is_its_groups_differentiator_units() {
        let member = |id| Member {
            id,
            ..Member::default()
        };
        let mut one = UnitGroup::new(1);
        one.members = vec![member(10), member(11)];
        one.differentiator_groups = vec![1, 2];
        let mut two = UnitGroup::new(2);
        two.members = vec![member(12)];
        let groups = UnitGroups {
            groups: vec![one, two],
            ..UnitGroups::default()
        };
        assert_eq!(groups.group_units(1), Some(vec![10, 11, 12]));
        assert_eq!(groups.group_units(2), Some(Vec::new()));
        assert_eq!(groups.group_units(9), None);
        assert_eq!(groups.group_units(-1), None);
    }
}
