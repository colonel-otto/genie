//! The update's entry: a player's groups, a group's update and its deletion, the members'
//! check and a hibernating group.

use crate::GenieError;
use crate::groups::WorldObject;
use crate::groups::command::MOVE_COMMAND;
use crate::groups::update::{ATTACK, BUILD, DELIVER_RELIC, NO_COMMAND, PICK_UP_RELIC, REPAIR};
use crate::world::World;

impl World {
    /// The groups player `p` owns update before its objects (`BaseWorld::update` calls them at
    /// `0x141551796`, then the player's own update, slot 0x100 at `0x1415518f0`, which updates
    /// its objects): `BUnitGroup::update` ([`World::group_update`]) for each group of the
    /// world's list the player owns; a group whose update returns 0 goes
    /// ([`World::delete_group`]), any other runs its vtable slot 8 (`FUN_141040b50`). A stopped
    /// player's groups wait with it.
    ///
    /// # Errors
    ///
    /// What the update does not model.
    pub(crate) fn update_groups(&mut self, p: usize) -> Result<(), GenieError> {
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
            let members: Vec<i32> = self
                .groups
                .get(id)
                .map(|g| g.members.iter().map(|m| m.id).collect())
                .unwrap_or_default();
            if self.group_update(id)? {
                self.group_update_end(id);
            } else {
                self.delete_group(id)?;
            }
            // the walks after it (other groups', the objects' own) read the members as it left them
            self.refresh_gaits(members);
        }
        Ok(())
    }

    /// The group's vtable slot 8 after its update ([`UnitGroup::update_end`]) of group `group`,
    /// which its update kept.
    ///
    /// # Panics
    ///
    /// When the world does not hold the group: the update loop passes only the groups it holds.
    ///
    /// [`UnitGroup::update_end`]: crate::groups::UnitGroup::update_end
    pub(super) fn group_update_end(&mut self, group: i32) {
        self.groups
            .get_mut(group)
            .expect("a group the update kept")
            .update_end();
    }

    /// A group whose update returned 0 goes (`BaseWorld::update` at `0x1415517a6`): the world's
    /// list loses it (and its count, `world+0x58`, one), then its destructor (`FUN_141006d10` ->
    /// `FUN_140fe1920`) runs. Each member the world holds whose link names the group leaves it
    /// (`+0x6d0` -1); one that is idle ([`World::member_idle`], `FUN_141017f00`) stands (slot
    /// 0x560, [`crate::movement::Mover::stand`]) and shows its standing sprite (slot 0x660,
    /// [`World::show_standing`]). The rest of the destructor frees the group's memory (its
    /// command queue emptied by `FUN_14102bb40` first); the destroyed id it hands the spectator
    /// stream (`FUN_141778770`) and its lookup of the group's target (the object, or a target
    /// group's first member, whose answer it drops) are no game state. Nothing for a group the
    /// world does not hold.
    ///
    /// # Errors
    ///
    /// A member the slice cannot stand or show standing.
    pub(crate) fn delete_group(&mut self, group: i32) -> Result<(), GenieError> {
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
            // the walks that meet it read its group
            let gait = u.gait();
            self.spatial.set_gait(m.id, gait);
            if self.member_idle(m.id)? {
                if let Some((_, u)) = self.object_mut(m.id)
                    && let Some(mover) = u.body.mover_mut()
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
    /// are checked ([`World::keep_apart`], `FUN_141007ca0`); a member the world does not hold,
    /// without a unit AI (a ghost has none), below 1 hit point (`COMISS` then `JA`: not a NaN) or
    /// of another player leaves the group (`removeUnit` with target -1, [`World::remove_unit`])
    /// and then stops (slot 0x5d0 on the object the world holds, [`crate::movement::Mover::
    /// stop`]); any such leave sets flags bit 16. A hibernating group (`mHibernating`, `+0x13c`)
    /// runs its own check ([`World::hibernating_update`]) and, unless that deletes it, stays
    /// asleep (true) without motion flags bit 1; with it, and for an awake group, the update
    /// goes on ([`World::awake_update`]).
    ///
    /// # Errors
    ///
    /// What the update does not model.
    ///
    /// # Panics
    ///
    /// When the world does not hold `group`: the game's routine gets the group itself, from a
    /// caller that holds it.
    pub fn group_update(&mut self, group: i32) -> Result<bool, GenieError> {
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
    /// [`World::group_update`]. A member that left takes its index with it, so the next member
    /// is checked at the same index.
    pub(super) fn drop_lost_members(&mut self, group: i32) -> Result<(), GenieError> {
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
                    && let Some(mover) = u.body.mover_mut()
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
    /// else the queued commands run (`checkQueuedOrders`, [`World::check_queued_orders`]) when
    /// [`World::hibernation`] asks for them.
    pub(super) fn hibernating_update(&mut self, group: i32) -> Result<bool, GenieError> {
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
    /// and 9 (an attack) wait while the group's target is pending ([`World::target_pending`],
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
    pub(super) fn hibernation(&self, group: i32) -> Result<Option<bool>, GenieError> {
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
    pub(super) fn repair_waits(&self, target: i32, owner: usize) -> Result<bool, GenieError> {
        let Some(object) = self.world_object(target) else {
            return Ok(false);
        };
        let WorldObject::Unit(p, u) = object else {
            return Err(GenieError::unsupported(
                "a hibernating repair of a ghost (its hit points, +0x278)",
            ));
        };
        let most = f32::from(self.master_of(p, u.life.master_id)?.hit_points);
        Ok(crate::math::fmath::below_or_unordered(u.life.hp, most)
            && self.spatial.allied_with(owner, p)
            && self.spatial.allied_with(p, owner))
    }
}
