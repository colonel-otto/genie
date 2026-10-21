//! A group's update (FD-036): `BUnitGroup::update` (`FUN_14103d910`), which the world runs for
//! the groups a player owns after the player's objects.

use crate::turn::Turns;
use crate::unit_ai::Unsupported;

impl Turns {
    /// The groups player `p` owns update after its objects (`BaseWorld::update` at
    /// `0x141551796`): `BUnitGroup::update` ([`Turns::group_update`]) for each group of the
    /// world's list the player owns; a group whose update returns 0 is deleted, any other runs
    /// its vtable slot 8 (`FUN_141040b50`). A stopped player's groups wait with it.
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
            if !self.group_update(id)? {
                return Err(Unsupported("a group's deletion after its update"));
            }
            self.group_update_end(id)?;
        }
        Ok(())
    }

    /// The group's vtable slot 8 after its update (`BUnitGroup::updateEnd`, `FUN_141040b50`): a
    /// group whose motion moved (motion flags bit 0) adds its new position to the front of its
    /// stored positions ([`super::PointLine::add_at_front`]); the motion's flags lose bit 0, and
    /// bit 1 with it when set, which sets the group's flags bit 1 (`FUN_14104a240`).
    ///
    /// # Errors
    ///
    /// A group the world does not hold.
    pub fn group_update_end(&mut self, group: i32) -> Result<(), Unsupported> {
        let g = self.groups.get_mut(group).ok_or(Unsupported(
            "the update end of a group the world does not hold",
        ))?;
        if g.motion.flags & 1 != 0 {
            let [point, forward, up, right] = g.motion.new_position;
            g.stored_positions.add_at_front(super::LinePoint {
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
        Ok(())
    }

    /// `BUnitGroup::update` (`FUN_14103d910`) of group `group`, as far as it is ported: flags bit
    /// 13 clears; the units the group keeps apart (`+0x458`, `+0x468`, `FUN_141007ca0`; no save
    /// holds them and nothing the slice runs adds any) are none; a member the world does not
    /// hold, without a unit AI, below 1 hit point or of another player would leave the group
    /// (`removeUnit`, `FUN_14102c160`, not modelled). A hibernating group (`+0x13c`) is not
    /// modelled. The statistics' far flag is set while no command is queued and motion flags bit
    /// 1 is clear. Flags bit 18 (the group placed anew) is not modelled. With members: without a
    /// command, queued or current, and with a formation, a group whose slots are all in state 3
    /// is not modelled; otherwise the statistics run when a member is held and flags bit 16 is
    /// set, and with flags bit 6 clear the lead is the member nearest the target's place
    /// ([`Turns::nearest_to_target`]); a held lead re-places the group
    /// ([`Turns::set_initial_position`]) when motion flags bit 1 is set, flags bit 10 is clear and
    /// the formation is not a horde, or flags bit 15 is set (which then clears), and the
    /// sub-formations without units go; then `BMotion`'s update (`FUN_14104a090`, not modelled).
    ///
    /// # Errors
    ///
    /// What the update does not model.
    ///
    /// # Panics
    ///
    /// Never: the group is the one found first.
    pub fn group_update(&mut self, group: i32) -> Result<bool, Unsupported> {
        let g = self
            .groups
            .get_mut(group)
            .ok_or(Unsupported("the update of a group the world does not hold"))?;
        g.flags &= !0x2000;
        let ids: Vec<i32> = g.members.iter().map(|m| m.id).collect();
        let owner = g.owner;
        for id in ids {
            let leaves = match self.object(id) {
                None => true,
                Some((p, u)) => u.ai.is_none() || u.life.hp < 1.0 || p as u32 != owner,
            };
            if leaves {
                return Err(Unsupported(
                    "a member gone, dead or of another player leaving its group (removeUnit)",
                ));
            }
        }
        let g = self.groups.get(group).expect("the group found above");
        if g.hibernating != 0 {
            return Err(Unsupported("a hibernating group's update"));
        }
        let far = g.command_queue.is_empty() && g.motion.flags & 2 == 0;
        if g.flags & 0x4_0000 != 0 {
            return Err(Unsupported("a group placed anew (flags bit 18)"));
        }
        if g.members.is_empty() {
            return Err(Unsupported("the update of a group without members"));
        }
        if g.command_queue.is_empty()
            && g.current_command == -1
            && g.current_formation
                .as_ref()
                .is_some_and(|f| f.slots.iter().all(|s| s.state == 3))
        {
            return Err(Unsupported("an idle group whose slots are all in state 3"));
        }
        let held = g.members.iter().any(|m| self.object(m.id).is_some());
        if held && g.flags & 0x1_0000 != 0 {
            self.group_statistics(group, far)?;
        }
        let g = self.groups.get(group).expect("the group found above");
        if g.flags & 0x40 != 0 {
            return Err(Unsupported("a group's update with flags bit 6"));
        }
        let lead = self.nearest_to_target(group);
        self.groups.get_mut(group).expect("found above").motion.lead = lead;
        if lead == -1 || self.object(lead).is_none() {
            return Err(Unsupported("a group's update without its lead"));
        }
        let g = self.groups.get(group).expect("found above");
        let horde = g
            .current_formation
            .as_ref()
            .is_some_and(|f| f.kind == super::formation::HORDE);
        if (g.motion.flags & 2 != 0 && g.flags & 0x400 == 0 && !horde) || g.flags & 0x8000 != 0 {
            self.set_initial_position(group)?;
            self.groups.get_mut(group).expect("found above").flags &= !0x8000;
        }
        let g = self.groups.get_mut(group).expect("found above");
        g.unit_types.retain(|s| !s.units.is_empty());
        Err(Unsupported("BMotion's update (FUN_14104a090)"))
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
