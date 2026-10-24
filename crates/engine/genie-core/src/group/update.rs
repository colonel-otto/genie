//! A group's update (FD-036): `BUnitGroup::update` (`FUN_14103d910`), which the world runs for
//! the groups a player owns before the player's objects, and the deletion of a group whose update
//! returns 0 (the world list's unlink, then `BUnitGroup`'s destructor, `FUN_140fe1920`).

use crate::GenieError;
use crate::fmath::Vec3;
use crate::group::command::MOVE_COMMAND;
use crate::group::{SpecificMove, UnitGroup, WorldObject};
use crate::turn::Turns;

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
/// A follow (`mCurrentCommand` 2).
const FOLLOW: i32 = 2;
/// A turn of the motion above this lays the formation out anew (`DAT_142eb6e44`, π/4).
const TURN_LIMIT: f32 = crate::image::facing::QUARTER_PI;
/// How far ahead of the group its front is looked for (`DAT_14330ebcc`).
const FRONT_AHEAD: f32 = crate::image::movement::LOOK_AHEAD;
/// A total spacing above this lays the formation out anew on a changed count
/// (`DAT_142ec9eb4`).
const RELAYOUT_SPACING: f32 = crate::image::missile::VOLLEY_REACH_MARGIN;
/// The share of the lead's line of sight a follow keeps within (`DAT_142e7f384`).
const FOLLOW_SHARE: f32 = crate::image::group::FRONT_SHARE;
/// A follow's target this near the top path's end (squared, `DAT_14330eb8c`) keeps the move.
const CLOSE: f32 = crate::image::group::CLOSE;
/// The differentiator groups' members the update lists for the members' moves, at most this
/// many (`DAT_143f15860`: `0x8b < n` ends the list).
const DIFFERENTIATORS: usize = 0x8c;
/// The idle timer past which an idle group draws it anew (the immediate `0xbb8`).
pub const IDLE_LIMIT: u32 = 3000;
/// A trade cart's standing and walking sprites are its trade task's while it carries goods
/// (`FUN_14106a940`: its carried attributes, `+0x7a0`, which the slice does not hold).
const TRADE_CART_SPRITES: GenieError =
    GenieError::unsupported("a trade cart's standing or walking sprite (its goods, FUN_14106a940)");

/// The sprites slots 0x660, 0x658 and 0x650 show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shown {
    /// Slot 0x660's standing sprite.
    Standing,
    /// Slot 0x658's walking sprite.
    Walking,
    /// Slot 0x650's running sprite.
    Running,
}

/// The updates of an attack (`FUN_14103f4e0`) and of the other commands on a target
/// (`FUN_141040bd0`), and the attack formation's refill (`FUN_1410163f0`): not modelled.
const OTHER_COMMANDS: GenieError = GenieError::unsupported(
    "a group command's update other than a move or a follow (FUN_14103f4e0, FUN_141040bd0)",
);

impl UnitGroup {
    /// `IsIdle` (`FUN_141017ad0`): idle with flags bit 5 (`cConsiderIdle`), or on a move
    /// (command 1) while it patrols ([`UnitGroup::patrols`]); else not while its motion moves or
    /// waits to move; else idle.
    #[must_use]
    pub fn is_idle(&self) -> bool {
        if self.flags >> 5 & 1 != 0 {
            return true;
        }
        if self.current_command == 1 && self.patrols() {
            return true;
        }
        !(self.motion.moving() || self.motion.waits_to_move())
    }

    /// The group patrols: its top path's flags have bit 0 (`FUN_141017c40`, the path of
    /// `FUN_141046500`) or its flags bit 11 (`FUN_141017c30`).
    #[must_use]
    pub fn patrols(&self) -> bool {
        crate::group::formation::top_path(&self.motion.paths).is_some_and(|p| p.path_flags & 1 != 0)
            || self.flags >> 11 & 1 != 0
    }

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
    /// The groups player `p` owns update before its objects (`BaseWorld::update` calls them at
    /// `0x141551796`, then the player's own update, slot 0x100 at `0x1415518f0`, which updates
    /// its objects): `BUnitGroup::update` ([`Turns::group_update`]) for each group of the
    /// world's list the player owns; a group whose update returns 0 goes
    /// ([`Turns::delete_group`]), any other runs its vtable slot 8 (`FUN_141040b50`). A stopped
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
    /// [`Turns::group_update`]. A member that left takes its index with it, so the next member
    /// is checked at the same index.
    fn drop_lost_members(&mut self, group: i32) -> Result<(), GenieError> {
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
    fn hibernating_update(&mut self, group: i32) -> Result<bool, GenieError> {
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
    fn hibernation(&self, group: i32) -> Result<Option<bool>, GenieError> {
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
    fn repair_waits(&self, target: i32, owner: usize) -> Result<bool, GenieError> {
        let Some(object) = self.world_object(target) else {
            return Ok(false);
        };
        let WorldObject::Unit(p, u) = object else {
            return Err(GenieError::unsupported(
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
    /// while flags bit 8 is clear, and the rest ([`Turns::after_motion`]).
    ///
    /// # Errors
    ///
    /// What the update does not model.
    fn awake_update(&mut self, group: i32) -> Result<bool, GenieError> {
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
        let forward = g.motion.position[1];
        self.motion_update(group, budget)?;
        self.after_motion(group, far, forward)
    }

    /// `BUnitGroup::update` after `BMotion`'s update (`0x14103e017` on), `forward` the motion's
    /// forward before it. With flags bit 8 the formation turns ([`Turns::turn_formation`]). The
    /// member count is kept; without flags bit 16 the statistics run (far as before), and bit 16
    /// clears; the group's log (`FUN_141015370`) holds no state. The differentiator groups'
    /// members (`DAT_143f15860`, at most `0x8c`: [`crate::group::UnitGroups::differentiator_units`])
    /// are only read by the members' moves below. Each formation line's attack range is its members' least
    /// ([`Turns::line_ranges`]); the relayout is chosen ([`Turns::relayout_choice`]) and the
    /// current command updates ([`Turns::command_update`], which may delete the group or let it
    /// hibernate). Without a member held there is no relayout; else the formation is laid out
    /// anew (`UpdateDesiredPositions(-1)`). An awake group's motion test (`FUN_141046600`) is
    /// unread, flags bit 9 clears, and a formation with slots moves its members to them
    /// ([`Turns::members_to_slots`], with the differentiator groups' members listed). Then
    /// the members' own moves ([`Turns::specific_moves_step`]), the order queue's due orders go
    /// (`FUN_14100d940`, [`Turns::drop_due_orders`], when it has any), and the idle timer
    /// ([`Turns::idle_timer_step`]). The debug drawing (`DAT_143f15a9c`) and the owner's
    /// computer test (`FUN_141659cf0`, its answer unread) hold no state. True while the group has
    /// members.
    ///
    /// # Errors
    ///
    /// What the steps do not model: a member's way round a record to its slot, a member's own
    /// move's step, an attack's update, among others.
    fn after_motion(&mut self, group: i32, far: bool, forward: Vec3) -> Result<bool, GenieError> {
        if self.groups.get(group).expect("held").flags & 0x100 != 0 {
            self.turn_formation(group, forward)?;
        }
        let count = self.groups.get(group).expect("held").members.len();
        if self.groups.get(group).expect("held").flags & 0x1_0000 == 0 {
            self.group_statistics(group, far)?;
        }
        self.groups.get_mut(group).expect("held").flags &= !0x1_0000;
        // DAT_143f15860 (0x14103e3a5..0x14103e442), which only the members' moves read
        let g = self.groups.get(group).expect("held");
        let differentiators = self.groups.differentiator_units(g, DIFFERENTIATORS);
        self.line_ranges(group)?;
        let (mut relayout, refill) = self.relayout_choice(group, count);
        if !self.command_update(group, &mut relayout)? {
            return Ok(false);
        }
        if !self.any_member_held(group) {
            relayout = false;
        }
        let g = self.groups.get(group).expect("held");
        if refill && g.current_formation.is_some() {
            // FUN_1410163f0: an attack's formation of kind 6 fills its slots anew; only an
            // attack (9) chooses it, whose update stops above
            return Err(OTHER_COMMANDS);
        }
        if relayout {
            self.update_desired_positions(group, -1)?;
        }
        let g = self.groups.get_mut(group).expect("held");
        if g.hibernating == 0 {
            g.flags &= !0x200;
            if g.current_formation
                .as_ref()
                .is_some_and(|f| !f.slots.is_empty())
            {
                self.members_to_slots(group, &differentiators)?;
            }
        }
        self.specific_moves_step(group)?;
        if !self.groups.get(group).expect("held").order_queue.is_empty() {
            self.drop_due_orders(group);
        }
        self.idle_timer_step(group);
        Ok(!self.groups.get(group).expect("held").members.is_empty())
    }

    /// The formation's turn (`0x14103e037`..`0x14103e37b`, flags bit 8): when the motion turned
    /// more than π/4 from `forward` (the angle `FUN_14087e650` in size; `COMISS` then `JA`, a NaN
    /// does not), or motion flags bit 1 or flags bit 14 is set, the stored positions lose their
    /// points (`FUN_14102bae0`), the group moves to its front ([`Turns::move_position_to_front`],
    /// 100 ahead; [`Turns::front_by_centroid`] when the walked level holds no path, its id
    /// `+0x20` -1), lays its formation out anew (`UpdateDesiredPositions(-1)`) whose slots lose
    /// their units and take them again (`FUN_141041810`, [`Turns::populate_slots`]), and flags
    /// bit 14 clears. Then the walked levels below the top go (`FUN_1410481f0`), `BMotion`'s
    /// update runs again with a budget, the formation transforms at the new place
    /// ([`Turns::formation_transform`], `param_4` 0) and flags bit 8 clears. The debug drawing
    /// (`DAT_143f15a9c`) holds no state.
    fn turn_formation(&mut self, group: i32, forward: Vec3) -> Result<(), GenieError> {
        if self.turns_formation(group, forward) {
            let g = self.groups.get(group).expect("held");
            let without_path = g.motion.paths.first().is_some_and(|l| !l.has_path);
            self.groups
                .get_mut(group)
                .expect("held")
                .stored_positions
                .clear_points();
            if without_path {
                self.front_by_centroid(group);
            } else {
                self.move_position_to_front(group, FRONT_AHEAD)?;
            }
            self.update_desired_positions(group, -1)?;
            if let Some(mut f) = self
                .groups
                .get_mut(group)
                .expect("held")
                .current_formation
                .take()
            {
                f.clear_units();
                let g = self.groups.get(group).expect("held").clone();
                let placed = self.populate_slots(&mut f, &g);
                self.groups.get_mut(group).expect("held").current_formation = Some(f);
                placed?;
            }
            self.groups.get_mut(group).expect("held").flags &= !0x4000;
        }
        self.groups
            .get_mut(group)
            .expect("held")
            .motion
            .drop_to_high_level();
        self.motion_update(group, true)?;
        if let Some(mut f) = self
            .groups
            .get_mut(group)
            .expect("held")
            .current_formation
            .take()
        {
            let g = self.groups.get(group).expect("held").clone();
            let moved = self.formation_transform(&mut f, &g, false);
            self.groups.get_mut(group).expect("held").current_formation = Some(f);
            let _ = moved?;
        }
        self.groups.get_mut(group).expect("held").flags &= !0x100;
        Ok(())
    }

    /// Whether the formation's turn lays it out anew: the motion turned more than π/4 from
    /// `forward` (the size of `FUN_14087e650`'s angle; `COMISS` then `JA` at `0x14103e154`, a NaN
    /// does not), or motion flags bit 1 or flags bit 14 is set.
    fn turns_formation(&self, group: i32, forward: Vec3) -> bool {
        let g = self.groups.get(group).expect("held");
        let turn = crate::fmath::angle_between(g.motion.position[1], forward).abs();
        TURN_LIMIT < turn || g.motion.flags & 2 != 0 || g.flags & 0x4000 != 0
    }

    /// `movePositionToFrontUsingCentroid` (inline at `0x14103e199`, its log aside): the group's
    /// place, and its new place, are its members' mean place (slot 0x520's, summed in order and
    /// divided by the member count, `CVTDQ2PS`) plus the forward scaled by 0 (`FUN_14087e530`:
    /// a zero but for an infinite or NaN forward).
    ///
    /// # Panics
    ///
    /// When the world does not hold a member: the game reads its null object; the members'
    /// check at the update's start leaves only held ones.
    fn front_by_centroid(&mut self, group: i32) {
        let g = self.groups.get(group).expect("held");
        let mut sum = [0.0f32; 3];
        for m in &g.members {
            let v = self
                .world_object(m.id)
                .expect("the update's start keeps only held members")
                .moved_vector();
            sum = [sum[0] + v[0], sum[1] + v[1], sum[2] + v[2]];
        }
        let n = g.members.len() as i32 as f32;
        let ahead = crate::fmath::scale(0.0, g.motion.position[1]);
        let point = [
            sum[0] / n + ahead[0],
            sum[1] / n + ahead[1],
            sum[2] / n + ahead[2],
        ];
        let g = self.groups.get_mut(group).expect("held");
        g.motion.new_position[0] = point;
        g.motion.position[0] = point;
    }

    /// Each formation line in use's (`mNumberLines`, `+0x2c`) attack range (`+0x10`,
    /// `0x14103e448`..`0x14103e535`): `f32::MAX`
    /// (the immediate `0x7f7fffff`), lowered to the weapon range (slot 0x230, a combat object's,
    /// else 0) of each of its slots' units the world holds that is below it (`COMISS` then
    /// `JBE`: a NaN range is not).
    ///
    /// # Errors
    ///
    /// A line place without a slot (index -1 or past the slots, where the game reads outside the
    /// slots' array), and a unit of a master the slice does not have.
    pub(crate) fn line_ranges(&mut self, group: i32) -> Result<(), GenieError> {
        let g = self.groups.get(group).expect("the caller holds the group");
        let Some(f) = g.current_formation.as_ref() else {
            return Ok(());
        };
        let lines = usize::try_from(f.number_lines).unwrap_or(0);
        let mut ranges = Vec::with_capacity(lines);
        for line in f.lines.iter().take(lines) {
            let mut range = f32::MAX;
            for &index in &line.indices {
                // the slots' array: those in use, then the spare ones
                let slot = usize::try_from(index as i32)
                    .ok()
                    .and_then(|i| {
                        f.slots
                            .get(i)
                            .or_else(|| f.spare_slots.get(i.checked_sub(f.slots.len())?))
                    })
                    .ok_or(GenieError::unsupported(
                        "a formation line's place without a slot (the update reads outside the slots, 0x14103e4ce)",
                    ))?;
                let Some(object) = self.world_object(slot.unit_id) else {
                    continue;
                };
                let master = self.master_of(object.player(), object.master_id())?;
                let reach = crate::group::stats::weapon_range(master);
                if range > reach {
                    range = reach;
                }
            }
            ranges.push(range);
        }
        let f = self
            .groups
            .get_mut(group)
            .expect("held")
            .current_formation
            .as_mut()
            .expect("found above");
        for (line, range) in f.lines.iter_mut().zip(ranges) {
            line.attack_range = range;
        }
        Ok(())
    }

    /// The relayout's choice (`0x14103e53f`..`0x14103e5f7`) after the update's start counted
    /// `count` members: (relayout, refill). A changed count refills an attack's (9) formation of
    /// kind 6 (`FUN_1410163f0` later), else sets flags bit 7 for a total spacing (`+0x47c`) of
    /// at most 6 (`COMISS` then `JBE`: a NaN too), else relays out. Flags bit 7 with a spacing
    /// above 6 relays out and clears bits 7 and 1; then flags bit 1 with a spacing above 6 and an
    /// idle group ([`Turns::is_idle`]) relays out and clears bit 1.
    fn relayout_choice(&mut self, group: i32, count: usize) -> (bool, bool) {
        let idle = self.is_idle(group);
        let g = self.groups.get_mut(group).expect("held");
        let (mut relayout, mut refill) = (false, false);
        if count != g.members.len() {
            if g.current_command == ATTACK
                && g.current_formation.as_ref().is_some_and(|f| f.kind == 6)
            {
                refill = true;
            } else if crate::fmath::at_most_or_unordered(g.total_spacing, RELAYOUT_SPACING) {
                g.flags |= 0x80;
            } else {
                relayout = true;
            }
        }
        if g.flags & 0x80 != 0 && RELAYOUT_SPACING < g.total_spacing {
            relayout = true;
            g.flags &= !0x82;
        }
        if g.flags & 2 != 0 && RELAYOUT_SPACING < g.total_spacing && idle {
            g.flags &= !2;
            relayout = true;
        }
        (relayout, refill)
    }

    /// The current command's update (`0x14103e5f7`..`0x14103e865`): false deletes the group. No
    /// command does nothing; a move ([`Turns::move_update`], `FUN_141041420`) that finished
    /// hibernates the group (`+0x13c` 1) while commands are queued, else deletes it. A follow (2)
    /// with its target held and a lead (the member nearest the target's place, [`Turns::
    /// nearest_to_target`]) moves the group to the target (`commandMoveTo` with the target's x
    /// and y, the range half the lead's master's line of sight, at once) when the target is
    /// that range or more (`COMISS` then `JC`: a NaN is not) from where the group's target was
    /// (`+0x3c8`, the height left out), and sets flags bit 12; without the target or a lead it
    /// ends as a finished move. An attack (`FUN_14103f4e0`) and the other commands
    /// (`FUN_141040bd0`) are not modelled.
    fn command_update(&mut self, group: i32, relayout: &mut bool) -> Result<bool, GenieError> {
        let g = self.groups.get(group).expect("held");
        let finished = match g.current_command {
            NO_COMMAND => false,
            MOVE_COMMAND => self.move_update(group, relayout)?,
            FOLLOW => {
                let target = g.current_target.target;
                let from = g.current_target.original_position;
                let to = self
                    .world_object(target)
                    .map(|t| (t.moved_vector(), t.position()));
                let lead = if to.is_some() {
                    let lead = self.nearest_to_target(group);
                    self.groups.get_mut(group).expect("held").motion.lead = lead;
                    lead
                } else {
                    -1
                };
                match to {
                    Some((moved, [x, y, _])) if lead != -1 => {
                        let g = self.groups.get(group).expect("held");
                        let leader = self
                            .lead_object(&g.motion)
                            .expect("the nearest member is held");
                        let sight = self
                            .master_of(leader.player(), leader.master_id())?
                            .line_of_sight;
                        let range = sight * FOLLOW_SHARE;
                        let d = crate::fmath::sub(from, moved);
                        let squared = d[0] * d[0] + 0.0 + d[2] * d[2];
                        if !crate::fmath::below_or_unordered(squared, range * range) {
                            let _ = self.command_move_to(
                                group,
                                target,
                                [x, y],
                                crate::group::command::MoveTo {
                                    range,
                                    leg: false,
                                    forget_target: false,
                                    hold: false,
                                    queued: false,
                                    direct: true,
                                    keep_flag: false,
                                },
                            )?;
                        }
                        self.groups.get_mut(group).expect("held").flags |= 0x1000;
                        return Ok(true);
                    }
                    _ => true,
                }
            }
            _ => return Err(OTHER_COMMANDS),
        };
        if finished {
            if self
                .groups
                .get(group)
                .expect("held")
                .command_queue
                .is_empty()
            {
                return Ok(false);
            }
            self.groups.get_mut(group).expect("held").hibernating = 1;
        }
        Ok(true)
    }

    /// `FUN_141041420`, a move's update: true when the move finished. Motion flags bit 1 asks
    /// for a relayout. A moving group (`FUN_141046600`) asks for one too, unless with flags bit
    /// 12 (a follow's) its target the world holds stands a squared 1 or more (`COMISS` then
    /// `JNC`: not a NaN) from the top path's end (`FUN_140f95fb0`: its last point, the static
    /// zero point `DAT_143f0dc78` for fewer than two), when the command becomes a follow (2);
    /// such a group without its target does nothing. A standing group with flags bit 12 follows
    /// too. A group with wait delays (`+0xc0`) or waiting for a path (`FUN_141046640`) does
    /// nothing. Then, off a patrol (the top path's flags bit 0) and with flags bit 11 clear,
    /// without queued commands but with a formation whose slots' array exists: the units of the
    /// slots in state 3 leave the group (`removeUnit` with the group's target); some left and
    /// some not: the rest move to their places ([`Turns::move_to_desired_positions`]) and the
    /// update ends. With a path walked (the walked level's path, `+0x18`, holding points) it
    /// goes on; without, a formation none of whose slots was in state 3 locks to its kind
    /// ([`Turns::lock_formation`], `FUN_141024e70`) and the command ends (-1); else the command
    /// ends. Any other group without a path walked ends its command. Then flags bit 2 clears, bit
    /// 5 sets and the members move to their places; finished: no path walked and not the
    /// patrol's case.
    ///
    /// # Errors
    ///
    /// What the leaves, the lock and the members' moves do not model.
    ///
    /// # Panics
    ///
    /// When the world does not hold `group`: the group update passes its own.
    pub fn move_update(&mut self, group: i32, relayout: &mut bool) -> Result<bool, GenieError> {
        let g = self.groups.get(group).expect("the caller holds the group");
        if g.motion.flags & 2 != 0 {
            *relayout = true;
            return Ok(false);
        }
        let walks = g
            .motion
            .paths
            .first()
            .is_some_and(|l| l.has_path && !l.points.is_empty());
        let follows = g.flags >> 12 & 1 != 0;
        if g.motion.moving() {
            if follows {
                let Some(target) = self.world_object(g.current_target.target) else {
                    return Ok(false);
                };
                if let Some(top) = crate::group::formation::top_path(&g.motion.paths) {
                    let end = match top.points.as_slice() {
                        [.., _, last] => *last,
                        // DAT_143f0dc78, zero-initialised (.bss) and written by nothing
                        _ => [0.0; 3],
                    };
                    let d = crate::fmath::sub(end, target.moved_vector());
                    let squared = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
                    if !crate::fmath::below_or_unordered(squared, CLOSE) {
                        self.groups.get_mut(group).expect("held").current_command = FOLLOW;
                        return Ok(false);
                    }
                }
            }
            *relayout = true;
            return Ok(false);
        }
        if follows {
            self.groups.get_mut(group).expect("held").current_command = FOLLOW;
            return Ok(false);
        }
        if g.motion.number_wait_delays != 0 || g.motion.waits_to_move() {
            return Ok(false);
        }
        let patrol = crate::group::formation::top_path(&g.motion.paths)
            .is_some_and(|p| p.path_flags & 1 != 0);
        let mut patrolling = true;
        if !patrol && g.flags >> 11 & 1 == 0 {
            patrolling = false;
            let slots = g
                .current_formation
                .as_ref()
                .filter(|f| !f.slots.is_empty() || !f.spare_slots.is_empty())
                .map(|f| {
                    (
                        f.kind,
                        f.slots
                            .iter()
                            .map(|s| (s.unit_id, s.state))
                            .collect::<Vec<_>>(),
                    )
                });
            if g.command_queue.is_empty()
                && let Some((kind, slots)) = slots
            {
                let target = g.current_target.target;
                let mut kept = 0;
                for &(unit, state) in &slots {
                    if state == 3 {
                        let _ = self.remove_unit(group, unit, target, false)?;
                    } else {
                        kept += 1;
                    }
                }
                if kept != 0 && kept != slots.len() {
                    self.move_to_desired_positions(group, MOVE_COMMAND, -1)?;
                    return Ok(false);
                }
                if !walks {
                    if kept == slots.len() {
                        self.lock_formation(group, kind)?;
                        self.groups.get_mut(group).expect("held").current_command = NO_COMMAND;
                        return Ok(false);
                    }
                    self.groups.get_mut(group).expect("held").current_command = NO_COMMAND;
                }
                return self.move_members_on(group, walks, patrolling);
            }
        }
        if !walks {
            self.groups.get_mut(group).expect("held").current_command = NO_COMMAND;
        }
        self.move_members_on(group, walks, patrolling)
    }

    /// `FUN_141041420`'s end (`0x1410416dc`): flags bit 2 clears, bit 5 sets, the members move
    /// to their places ([`Turns::move_to_desired_positions`] with a move and no target); the
    /// move finished when no path is walked and it was no patrol.
    fn move_members_on(
        &mut self,
        group: i32,
        walks: bool,
        patrolling: bool,
    ) -> Result<bool, GenieError> {
        let g = self.groups.get_mut(group).expect("held");
        g.flags = (g.flags & !4) | 0x20;
        self.move_to_desired_positions(group, MOVE_COMMAND, -1)?;
        Ok(!walks && !patrolling)
    }

    /// `AutomaticMoveUnitsToDesiredPositions` (`FUN_14100aaa0`) of group `group`'s formation
    /// for command `command` on target `target` (the group update's are a move and -1); its log
    /// lines aside. Nothing without a formation or its slots' array (`FUN_140fd75d0` lets go of
    /// the formation's reference). For each slot with a unit (`-1` none) the world holds that is
    /// free ([`Turns::member_idle`], `FUN_141017f00`): one in its place (slot state 3) on an
    /// attack (9) on a held target takes the target (slot 0x128 with its x and y); any other
    /// gets a path to the slot.
    ///
    /// # Errors
    ///
    /// A unit's attack on the target (its slot 0x128) and its path to the slot (a world path
    /// record `FUN_14152eea0`, the long or short pather, the path's length against 1.01,
    /// `DAT_142eb6e48`, which has no image role, then slot 0x5d0, `FUN_140f9b6c0`, slot 0x548
    /// and the unit's move, slot 0x128): not modelled.
    ///
    /// # Panics
    ///
    /// When the world does not hold `group`: its callers pass their own.
    pub fn move_to_desired_positions(
        &mut self,
        group: i32,
        command: i32,
        target: i32,
    ) -> Result<(), GenieError> {
        let g = self.groups.get(group).expect("the caller holds the group");
        let Some(f) = g
            .current_formation
            .as_ref()
            .filter(|f| !f.slots.is_empty() || !f.spare_slots.is_empty())
        else {
            return Ok(());
        };
        let held_target = self.world_object(target).is_some();
        for slot in &f.slots {
            if slot.unit_id == -1 || self.world_object(slot.unit_id).is_none() {
                continue;
            }
            if !self.member_idle(slot.unit_id)? {
                continue;
            }
            if slot.state == 3 {
                if command == ATTACK && held_target {
                    return Err(GenieError::unsupported(
                        "AutomaticMoveUnitsToDesiredPositions' attack on the target (slot 0x128)",
                    ));
                }
                continue;
            }
            return Err(GenieError::unsupported(
                "AutomaticMoveUnitsToDesiredPositions' path to a slot (FUN_14100aaa0: the unit's path, 1.01 of DAT_142eb6e48, slots 0x548 and 0x128)",
            ));
        }
        Ok(())
    }

    /// The members' own moves past the slots' (`0x14103efc4`..`0x14103f355`): each member whose
    /// move is active (`mSpecificMoves[i]`) and that is free ([`Turns::member_idle`]) and held:
    /// one moved aside (reason 2) stops (slot 0x5d0), shows standing (slot 0x660), counts its
    /// timer down by the update's seconds (`FUN_1415300e0`) and, once at most 0 (`COMISS` then
    /// `JC`: a NaN keeps it), drops its move (active 0, path and reason -1, the rest 0) and stands
    /// (slot 0x560). Any other steps (slot 0x608, [`Turns::member_step`]): once the step is done
    /// (not 0) its move is dropped; else it shows its walking sprite (slot 0x658) and its move's
    /// distance loses the ground distance from its last position to where it last moved to
    /// (slot 0x520), which becomes its last position.
    ///
    /// # Errors
    ///
    /// What the motion step, the stop, the stand and the walking and standing sprites do not
    /// model.
    fn specific_moves_step(&mut self, group: i32) -> Result<(), GenieError> {
        let mut i = 0;
        loop {
            let g = self.groups.get(group).expect("held");
            let Some(m) = g.members.get(i).copied() else {
                break;
            };
            i += 1;
            if m.specific_move.active == 0 || !self.member_idle(m.id)? {
                continue;
            }
            let Some((_, u)) = self.object_mut(m.id) else {
                continue;
            };
            if m.specific_move.reason != 2 {
                // slot 0x608 (FUN_1411581e0): the unit's own motion step (FUN_141150f00), then
                // its facet from its angle (FUN_14114d8b0, not held)
                if self.member_step(m.id)? {
                    self.groups.get_mut(group).expect("held").members[i - 1].specific_move =
                        SpecificMove {
                            path_id: -1,
                            reason: -1,
                            ..SpecificMove::default()
                        };
                    continue;
                }
                let _ = self.show_walking(m.id)?;
                let at = self
                    .object(m.id)
                    .map(|(_, u)| crate::group::motion::moved_vector(u))
                    .expect("the member is held");
                let own =
                    &mut self.groups.get_mut(group).expect("held").members[i - 1].specific_move;
                let mut d = crate::fmath::sub(at, own.last_position);
                d[1] = 0.0;
                own.distance -= crate::fmath::sqrt(d[0] * d[0] + 0.0 + d[2] * d[2]);
                own.last_position = at;
                continue;
            }
            if let Some(mover) = u.mover.as_mut() {
                mover.stop()?;
            }
            self.show_standing(m.id)?;
            let dt = self.dt_seconds;
            let g = self.groups.get_mut(group).expect("held");
            let timer = &mut g.members[i - 1].specific_move.timer;
            *timer -= dt;
            if !crate::fmath::below_or_unordered(0.0, *timer) {
                g.members[i - 1].specific_move = SpecificMove {
                    path_id: -1,
                    reason: -1,
                    ..SpecificMove::default()
                };
                if let Some((_, u)) = self.object_mut(m.id)
                    && let Some(mover) = u.mover.as_mut()
                {
                    mover.stand()?;
                }
            }
        }
        Ok(())
    }

    /// Slot 0x608 (`FUN_1411581e0`) of member `id`: the unit's own motion step (`FUN_141150f00`,
    /// [`crate::movement::Mover::motion`] with its top action's state, the update's seconds and
    /// the world's planners); true when it is done (the step answers other than 0: arrived or
    /// given up). Its place follows the world's; the units its walk paused take their pauses
    /// (slot 0x5f0). Not held: its facet from its angle (`FUN_14114d8b0`).
    ///
    /// # Errors
    ///
    /// What the motion step does not model, and a walk's unit in the way (notify 0x215).
    pub(crate) fn member_step(&mut self, id: i32) -> Result<bool, GenieError> {
        use crate::movement::{Me, Motion, Planning};
        let Some(place) = self.place(id) else {
            return Ok(true);
        };
        let crate::order::Place::Live(p, idx) = place else {
            return Err(GenieError::unsupported("a sleeping member's own move"));
        };
        let computer = self.players[p].computer;
        let construction_rate = self.players[p].construction_rate;
        let unit = &mut self.players[p].objects[idx];
        let mut me = Me {
            id,
            position: unit.position,
            group: unit.group.id,
            action_state: unit.top_action().map_or(0, |a| a.state),
        };
        let mover = unit.mover.as_mut().ok_or(crate::turn::WALKER_UNMOVING)?;
        let mut plan = Planning {
            pathing: &mut self.pathing,
            now: self.world_time,
            computer,
            object: self.streams.get(crate::rng::Stream::Object),
            construction_rate,
            groups: &self.groups,
        };
        let motion = mover.motion(&mut me, self.dt_seconds, &mut self.spatial, Some(&mut plan))?;
        let pauses = std::mem::take(&mut mover.pauses);
        if !mover.in_the_way.is_empty() {
            return Err(crate::turn::IN_THE_WAY);
        }
        if let Some(at) = self.spatial.get(id).map(|o| o.position) {
            self.players[p].objects[idx].position = at;
        }
        for (other, pause) in pauses {
            self.pause(other, pause);
        }
        Ok(motion != Motion::Moving)
    }

    /// `FUN_14100d940` (its log lines aside): the order queue's orders whose time (`+0x18`) is not
    /// past the world's time go, in order; the others stay.
    pub fn drop_due_orders(&mut self, group: i32) {
        let now = self.world_time;
        if let Some(g) = self.groups.get_mut(group) {
            g.order_queue.retain(|o| now < o.execute_time);
        }
    }

    /// The idle timer (`0x14103f370`..`0x14103f3a3`): an idle group ([`Turns::is_idle`]) adds
    /// the update's milliseconds (`world+0x7c`) to its timer (`+0x3e4`, wrapping), and past
    /// [`IDLE_LIMIT`] (unsigned) draws it anew ([`Turns::reset_idle_timer`]); a group that is not
    /// idle draws it anew.
    fn idle_timer_step(&mut self, group: i32) {
        if self.is_idle(group) {
            let dt = self.dt_ms();
            let g = self.groups.get_mut(group).expect("held");
            g.idle_timer = g.idle_timer.wrapping_add(dt);
            if g.idle_timer <= IDLE_LIMIT {
                return;
            }
        }
        self.reset_idle_timer(group);
    }

    /// The update's milliseconds (`world+0x7c`), of which [`Turns::dt_seconds`] (`world+0x164`)
    /// is [`crate::lifecycle::seconds`]: the round trip gives the milliseconds back exactly for
    /// any update below 2^22 of them.
    fn dt_ms(&self) -> u32 {
        (self.dt_seconds * 1000.0).round() as u32
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
    pub(crate) fn keep_apart(&mut self, group: i32) -> Result<(), GenieError> {
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
                    return Err(GenieError::unsupported(
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
    fn may_interrupt(&self, object: WorldObject<'_>) -> Result<bool, GenieError> {
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
    ) -> Result<Option<WorldObject<'_>>, GenieError> {
        let WorldObject::Unit(p, u) = object else {
            return Ok(None);
        };
        if self.master_of(p, u.life.master_id)?.kind < ACTION_KIND {
            return Ok(None);
        }
        if let Some(e) = u.unmodelled_action.clone() {
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
    ) -> Result<f32, GenieError> {
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
    pub fn target_pending(&self, group: i32) -> Result<bool, GenieError> {
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
    fn relic_pickup_pending(&self, g: &UnitGroup) -> Result<bool, GenieError> {
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
    fn relic_delivery_pending(&self, g: &UnitGroup) -> Result<bool, GenieError> {
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
    fn has_task(&self, player: usize, master: i16, action: i16) -> Result<bool, GenieError> {
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
    fn sees_object(&self, object: WorldObject<'_>, player: usize) -> Result<bool, GenieError> {
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
    pub fn member_idle(&self, id: i32) -> Result<bool, GenieError> {
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
            if let Some(e) = u.unmodelled_action.clone() {
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
    pub(crate) fn show_standing(&mut self, id: i32) -> Result<(), GenieError> {
        self.show_sprite(id, Shown::Standing).map(|_| ())
    }

    /// Slot 0x658 of object `id` (an action object's `FUN_14106e340`): it shows its walking
    /// sprite; false when it has none. A trade cart (class 19) with goods (`FUN_14106a940`)
    /// shows its trade task's (slot 0x7c8 then slot 0x68); an action object's top action is
    /// asked next (slot 0x430, then the action's slot 0x58 with 0: only a trade's answers, the
    /// slice holds none); then `FUN_14115d0e0` shows the walking sprite (slot 0x7c8: the
    /// master's, `+0x748`; the unit's own `+0x8a9` override is not held) when it has one (slot
    /// 0x68: the top speed is the walking sprite's, [`crate::movement::MoveStats::walk_speed`]).
    /// The members it serves are units (type 70).
    ///
    /// # Errors
    ///
    /// As [`Turns::show_standing`].
    pub(crate) fn show_walking(&mut self, id: i32) -> Result<bool, GenieError> {
        self.show_sprite(id, Shown::Walking)
    }

    /// Slot 0x650 of object `id` (an action object's `FUN_14106e3c0`): it shows its running
    /// sprite; false when it has none. An action object's top action is asked first (slot 0x430,
    /// then the action's slot 0x58 with 1: only a trade's answers); then `FUN_14115d300` shows the
    /// master's running sprite (`+0x750`) when it has one (slot 0x68: the top speed is the running
    /// sprite's, [`crate::movement::MoveStats::run_speed`]).
    ///
    /// # Errors
    ///
    /// As [`Turns::show_standing`], a trade cart aside.
    pub(crate) fn show_running(&mut self, id: i32) -> Result<bool, GenieError> {
        self.show_sprite(id, Shown::Running)
    }

    /// Slots 0x660, 0x658 and 0x650 of object `id` ([`Shown`]): whether it shows the sprite.
    fn show_sprite(&mut self, id: i32, shown: Shown) -> Result<bool, GenieError> {
        let Some((p, u)) = self.object(id) else {
            return Ok(false);
        };
        let master = self.master_of(p, u.life.master_id)?;
        if master.kind < MOVING_KIND {
            return Ok(false);
        }
        let trade_cart = match shown {
            Shown::Standing => master.kind == COMBAT_KIND && master.class == TRADE_CART,
            Shown::Walking => ACTION_KIND <= master.kind && master.class == TRADE_CART,
            Shown::Running => false,
        };
        if trade_cart {
            return Err(TRADE_CART_SPRITES);
        }
        if ACTION_KIND <= master.kind
            && let Some(e) = u.unmodelled_action.clone()
        {
            return Err(e);
        }
        // the slots of a combat object read its terrain look ([`crate::sprite::standing`]); a
        // trade cart's goods are passed above
        let look = u.abilities.look.as_ref();
        let sprite = match shown {
            Shown::Standing => crate::sprite::standing(master, look, false),
            Shown::Walking => crate::sprite::walking(master, look, false),
            Shown::Running => crate::sprite::running(master, look),
        };
        let Some(sprite) = sprite else {
            return Ok(false);
        };
        let graphic = self.graphics.get(&sprite).copied();
        let (_, u) = self.object_mut(id).expect("found above");
        if let Some(m) = u.mover.as_mut() {
            m.stats.speed = match shown {
                Shown::Standing => m.stats.stand_speed,
                Shown::Walking => m.stats.walk_speed,
                Shown::Running => m.stats.run_speed,
            };
        }
        if u.sprite.is_none() {
            return Ok(true);
        }
        let g = graphic.ok_or(GenieError::unsupported(
            "a unit without its sprite's graphic",
        ))?;
        let place = self.place(id).expect("found above");
        let rng = self.streams.get(crate::rng::Stream::Sprite);
        let u = match place {
            crate::order::Place::Live(p, i) => &mut self.players[p].objects[i],
            crate::order::Place::Sleeping(p, i) => &mut self.players[p].sleeping[i],
        };
        crate::attack::set_sprite(&mut u.sprite, g, rng);
        Ok(true)
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
        self.groups.get(group).is_some_and(UnitGroup::is_idle)
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
            formation_category: 1,
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
        s.unit_diplomacy = vec![
            vec![1, 4, 4, 4],
            vec![0, 1, 4, 2],
            vec![0, 4, 1, 4],
            vec![0, 2, 4, 1],
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
            goto: [-1.0; 3],
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
            same_owner: false,
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

    /// Slots 0x660, 0x658 and 0x650: a unit with the sprite takes its speed (and the sprite,
    /// where the slice holds the unit's), one without (-1) shows nothing; a trade cart's
    /// standing and walking sprites are its goods', which the slice does not hold, its running
    /// one the master's.
    #[test]
    fn a_unit_shows_its_standing_walking_or_running_sprite() {
        let mut t = world();
        let mut walker = crate::hunt::tests::walker();
        walker.stats.walk_speed = 0.9;
        walker.stats.run_speed = 1.5;
        walker.stats.stand_speed = 0.1;
        t.players[1].objects[0].mover = Some(walker);
        let speed = |t: &Turns| t.object(10).unwrap().1.mover.as_ref().unwrap().stats.speed;
        let master = |t: &mut Turns, class: i16, sprites: [i16; 3]| {
            let m = Master {
                id: 4,
                kind: 70,
                class,
                hit_points: 60,
                radius: [0.2, 0.2],
                standing_sprite: sprites[0],
                move_sprite: sprites[1],
                run_sprite: sprites[2],
                formation_category: 1,
                ..Master::default()
            };
            t.masters = vec![Masters::new([m]); 4];
        };
        master(&mut t, 6, [-1; 3]);
        assert_eq!(t.show_walking(10), Ok(false));
        assert_eq!(t.show_running(10), Ok(false));
        t.show_standing(10).unwrap();
        assert_eq!(speed(&t), 0.8);
        master(&mut t, 6, [6, 7, 8]);
        assert_eq!(t.show_walking(10), Ok(true));
        assert_eq!(speed(&t), 0.9);
        assert_eq!(t.show_running(10), Ok(true));
        assert_eq!(speed(&t), 1.5);
        t.show_standing(10).unwrap();
        assert_eq!(speed(&t), 0.1);
        // a held sprite needs the graphic
        let graphic = crate::sprite::Graphic {
            id: 1,
            frames: 1,
            facets: 8,
            speed_multiplier: 1.0,
            frame_duration: 0.0,
            replay_delay: 0.0,
            flags: 0,
        };
        t.players[1].objects[0].sprite = Some(crate::sprite::ActiveSprite {
            graphic,
            frame: 0,
            interval: 0.0,
            last: 0.0,
            last_frame: 0,
            changed: false,
            looped: false,
            animate: 0,
            speed: 0.0,
        });
        assert!(t.show_walking(10).is_err());
        master(&mut t, crate::group::update::TRADE_CART, [6, 7, 8]);
        let trade = crate::group::update::TRADE_CART_SPRITES;
        assert_eq!(t.show_walking(10), Err(trade.clone()));
        assert_eq!(t.show_standing(10), Err(trade));
        t.players[1].objects[0].sprite = None;
        assert_eq!(t.show_running(10), Ok(true));
        assert_eq!(t.show_walking(99), Ok(false));
    }

    /// The formation's turn: past π/4 (not a NaN), or with motion flags bit 1 or flags bit 14.
    #[test]
    fn the_formation_turns_past_a_quarter_turn() {
        let mut t = world();
        let g = t.groups.get_mut(1).unwrap();
        g.flags = 0;
        g.motion.flags = 0;
        g.motion.position[1] = [1.0, 0.0, 0.0];
        assert!(!t.turns_formation(1, [0.72, 0.0, 0.693_974_1]));
        assert!(t.turns_formation(1, [0.70, 0.0, 0.714_142_8]));
        assert!(t.turns_formation(1, [0.0, 0.0, -1.0]));
        assert!(!t.turns_formation(1, [f32::NAN, 0.0, 0.0]));
        t.groups.get_mut(1).unwrap().flags = 0x4000;
        assert!(t.turns_formation(1, [1.0, 0.0, 0.0]));
        let g = t.groups.get_mut(1).unwrap();
        g.flags = 0;
        g.motion.flags = 2;
        assert!(t.turns_formation(1, [1.0, 0.0, 0.0]));
    }

    /// `movePositionToFrontUsingCentroid`: the members' mean place (x, height, y) plus the
    /// forward times 0, as the group's place and new place.
    #[test]
    fn the_centroid_front_is_the_members_mean_place() {
        let mut t = world();
        t.players[1].objects[2].position = [1.0, 4.0, 3.0];
        t.groups.get_mut(1).unwrap().motion.position[1] = [1.0, 0.0, 0.0];
        t.front_by_centroid(1);
        let g = t.groups.get(1).unwrap();
        let mean = [4.0 / 3.0, 1.0, 2.0];
        assert_eq!(g.motion.position[0], mean);
        assert_eq!(g.motion.new_position[0], mean);
        // an infinite forward makes the place NaN (inf times 0)
        t.groups.get_mut(1).unwrap().motion.position[1] = [f32::INFINITY, 0.0, 0.0];
        t.front_by_centroid(1);
        assert!(t.groups.get(1).unwrap().motion.position[0][0].is_nan());
    }

    /// The formation lines' attack ranges: each its slots' units' least weapon range (a combat
    /// master's; 0 for another type), `f32::MAX` without a held one; a spare slot's index reads
    /// the spare slot; a place without a slot is not modelled.
    #[test]
    fn each_line_takes_its_units_least_range() {
        let mut t = world();
        let combat = |id: i16, kind: u8, range: f32| Master {
            id,
            kind,
            weapon_range: range,
            ..Master::default()
        };
        t.masters =
            vec![Masters::new([combat(4, 70, 3.0), combat(5, 70, 1.5), combat(6, 30, 2.0)]); 4];
        t.players[1].objects[1].life.master_id = 5;
        t.players[1].objects[2].life.master_id = 6;
        let slot = |unit| crate::group::Slot {
            unit_id: unit,
            ..crate::group::Slot::default()
        };
        let line = |indices: Vec<u32>| crate::group::FormationLine {
            indices,
            ..crate::group::FormationLine::default()
        };
        t.groups.get_mut(1).unwrap().current_formation = Some(crate::group::Formation {
            slots: vec![slot(10), slot(11), slot(99)],
            spare_slots: vec![slot(12)],
            lines: vec![
                line(vec![0, 1]),
                line(vec![2]),
                line(vec![3, 0]),
                line(vec![0]),
            ],
            number_lines: 3,
            ..crate::group::Formation::default()
        });
        t.line_ranges(1).unwrap();
        let f = t.groups.get(1).unwrap().current_formation.clone().unwrap();
        // the fourth line is not in use: it keeps its range
        assert_eq!(
            f.lines.iter().map(|l| l.attack_range).collect::<Vec<_>>(),
            [1.5, f32::MAX, 0.0, 0.0]
        );
        t.groups
            .get_mut(1)
            .unwrap()
            .current_formation
            .as_mut()
            .unwrap()
            .lines[1]
            .indices = vec![u32::MAX];
        assert!(t.line_ranges(1).is_err());
    }

    /// The relayout's choice: a changed count sets flags bit 7 at a spacing of at most 6 (a NaN
    /// too) and relays out above it; an attack's kind-6 formation refills instead; bit 7 above 6
    /// relays out and clears bits 7 and 1; bit 1 above 6 relays out an idle group.
    #[test]
    fn the_relayout_follows_the_count_and_the_spacing() {
        let mut t = world();
        let set = |t: &mut Turns, flags: u32, spacing: f32| {
            let g = t.groups.get_mut(1).unwrap();
            g.flags = flags;
            g.total_spacing = spacing;
        };
        set(&mut t, 0, 6.0);
        assert_eq!(t.relayout_choice(1, 3), (false, false));
        assert_eq!(t.relayout_choice(1, 4), (false, false));
        assert_eq!(t.groups.get(1).unwrap().flags, 0x80);
        set(&mut t, 0, f32::NAN);
        assert_eq!(t.relayout_choice(1, 4), (false, false));
        assert_eq!(t.groups.get(1).unwrap().flags, 0x80);
        set(&mut t, 0, 6.5);
        assert_eq!(t.relayout_choice(1, 4), (true, false));
        set(&mut t, 0x82, 6.5);
        assert_eq!(t.relayout_choice(1, 3), (true, false));
        assert_eq!(t.groups.get(1).unwrap().flags, 0);
        set(&mut t, 0x2, 6.5);
        assert_eq!(t.relayout_choice(1, 3), (true, false));
        assert_eq!(t.groups.get(1).unwrap().flags, 0);
        t.groups.get_mut(1).unwrap().motion.velocity = [0.0, 0.0, 1.0];
        set(&mut t, 0x2, 6.5);
        assert_eq!(t.relayout_choice(1, 3), (false, false));
        assert_eq!(t.groups.get(1).unwrap().flags, 2);
        let g = t.groups.get_mut(1).unwrap();
        g.current_command = 9;
        g.current_formation = Some(crate::group::Formation {
            kind: 6,
            ..crate::group::Formation::default()
        });
        set(&mut t, 0, 6.0);
        assert_eq!(t.relayout_choice(1, 4), (false, true));
        assert_eq!(t.groups.get(1).unwrap().flags, 0);
    }

    /// A follow: its target near where the group's target was keeps the group (flags bit 12);
    /// far, the group moves to it (`commandMoveTo`: the members' seconds since attacking go to
    /// `f32::MAX`); without the target it ends as a finished move: deleted without queued
    /// commands, else hibernating.
    #[test]
    fn a_follow_moves_to_its_target_when_it_strays() {
        let mut t = world();
        let g = t.groups.get_mut(1).unwrap();
        g.current_command = 2;
        g.current_target.target = 20;
        g.current_target.original_position = [3.5, 0.0, 4.5];
        g.flags = 0;
        let mut relayout = false;
        // 1 from the target: within half the lead's line of sight (2)
        assert!(t.command_update(1, &mut relayout).unwrap());
        let g = t.groups.get(1).unwrap();
        assert_eq!(
            (g.flags, g.members[0].seconds_since_attacking),
            (0x1000, 0.0)
        );
        t.groups
            .get_mut(1)
            .unwrap()
            .current_target
            .original_position = [3.5, 0.0, 5.5];
        assert!(t.command_update(1, &mut relayout).unwrap());
        let g = t.groups.get(1).unwrap();
        assert_eq!(
            (g.flags, g.members[0].seconds_since_attacking),
            (0x1000, f32::MAX)
        );
        assert_eq!(g.motion.lead, 12);
        t.groups.get_mut(1).unwrap().current_target.target = 77;
        assert!(!t.command_update(1, &mut relayout).unwrap());
        t.groups.get_mut(1).unwrap().command_queue = vec![crate::group::GroupCommand::default()];
        assert!(t.command_update(1, &mut relayout).unwrap());
        assert_eq!(t.groups.get(1).unwrap().hibernating, 1);
        assert!(!relayout);
        t.groups.get_mut(1).unwrap().current_command = 3;
        assert!(t.command_update(1, &mut relayout).is_err());
    }

    /// `FUN_141041420`: motion flags bit 1 and a moving group ask for a relayout; with flags bit
    /// 12 a strayed target (a squared 1 from the top path's end) turns the move into a follow;
    /// wait delays and a group waiting for a path do nothing.
    #[test]
    fn a_moving_group_s_move_waits_or_follows() {
        let mut t = world();
        let g = t.groups.get_mut(1).unwrap();
        g.current_command = 1;
        g.flags = 0;
        g.motion.flags = 2;
        let mut relayout = false;
        assert!(!t.move_update(1, &mut relayout).unwrap());
        assert!(relayout);
        let g = t.groups.get_mut(1).unwrap();
        g.motion.flags = 0;
        g.motion.velocity = [0.0, 0.0, 1.0];
        let mut relayout = false;
        assert!(!t.move_update(1, &mut relayout).unwrap());
        assert!(relayout);
        // a follow's target 20 at (3.5, 3.5) and the top path's end at (3.5, h, 3.5): no change
        let g = t.groups.get_mut(1).unwrap();
        g.flags = 0x1000;
        g.current_target.target = 20;
        let mut path = crate::movement::PathData::without_path();
        path.has_path = true;
        path.points = vec![[0.0; 3], [3.5, 0.0, 3.5]];
        g.motion.paths = vec![path];
        let mut relayout = false;
        assert!(!t.move_update(1, &mut relayout).unwrap());
        assert_eq!(
            (relayout, t.groups.get(1).unwrap().current_command),
            (true, 1)
        );
        t.groups.get_mut(1).unwrap().motion.paths[0].points[1] = [3.5, 0.0, 4.5];
        let mut relayout = false;
        assert!(!t.move_update(1, &mut relayout).unwrap());
        assert_eq!(
            (relayout, t.groups.get(1).unwrap().current_command),
            (false, 2)
        );
        // standing with bit 12: a follow at once; else wait delays or a level waiting do nothing
        let g = t.groups.get_mut(1).unwrap();
        g.current_command = 1;
        g.motion.velocity = [0.0; 3];
        assert!(!t.move_update(1, &mut relayout).unwrap());
        assert_eq!(t.groups.get(1).unwrap().current_command, 2);
        let g = t.groups.get_mut(1).unwrap();
        g.current_command = 1;
        g.flags = 0;
        g.motion.number_wait_delays = 1;
        assert!(!t.move_update(1, &mut relayout).unwrap());
        let g = t.groups.get_mut(1).unwrap();
        g.motion.number_wait_delays = 0;
        g.motion.paths[0].state = 2;
        assert!(!t.move_update(1, &mut relayout).unwrap());
        assert_eq!(t.groups.get(1).unwrap().current_command, 1);
    }

    /// `FUN_141041420` for a standing group: without a path walked and no formation the move
    /// finishes (command -1, flags bit 2 off and bit 5 on); with one walked it goes on; on a
    /// patrol it does not finish; a formation whose slots are all in their places lets their
    /// units go and ends the command; one with none in place locks to its kind.
    #[test]
    fn a_standing_group_s_move_ends_or_goes_on() {
        let mut t = world();
        let g = t.groups.get_mut(1).unwrap();
        g.current_command = 1;
        g.flags = 4;
        g.motion.flags = 0;
        let mut relayout = false;
        assert!(t.move_update(1, &mut relayout).unwrap());
        let g = t.groups.get(1).unwrap();
        assert_eq!((g.current_command, g.flags), (-1, 0x20));
        let walked = || {
            let mut path = crate::movement::PathData::without_path();
            path.has_path = true;
            path.points = vec![[0.0; 3], [3.5, 0.0, 3.5]];
            path
        };
        let g = t.groups.get_mut(1).unwrap();
        g.current_command = 1;
        g.motion.paths = vec![walked()];
        assert!(!t.move_update(1, &mut relayout).unwrap());
        assert_eq!(t.groups.get(1).unwrap().current_command, 1);
        // on a patrol (flags bit 11) without a path walked: the command ends, unfinished
        let g = t.groups.get_mut(1).unwrap();
        g.motion.paths[0].has_path = false;
        g.flags = 0x800;
        assert!(!t.move_update(1, &mut relayout).unwrap());
        assert_eq!(t.groups.get(1).unwrap().current_command, -1);
        // every slot in place: its units leave and the command ends
        let mut t = world();
        let slot = |unit, state| crate::group::Slot {
            unit_id: unit,
            state,
            ..crate::group::Slot::default()
        };
        let g = t.groups.get_mut(1).unwrap();
        g.current_command = 1;
        g.motion.flags = 0;
        g.current_formation = Some(crate::group::Formation {
            kind: 1,
            slots: vec![slot(10, 3), slot(11, 3)],
            ..crate::group::Formation::default()
        });
        assert!(t.move_update(1, &mut relayout).unwrap());
        let g = t.groups.get(1).unwrap();
        assert_eq!(g.members.iter().map(|m| m.id).collect::<Vec<_>>(), [12]);
        assert_eq!(g.current_command, -1);
        // none in place but no free unit: the formation locks to its kind, the command ends
        let mut t = world();
        for u in &mut t.players[1].objects {
            u.ai.as_mut().unwrap().current_action = 0x2c6;
        }
        let g = t.groups.get_mut(1).unwrap();
        g.current_command = 1;
        g.motion.flags = 0;
        g.current_formation = Some(crate::group::Formation {
            kind: 1,
            slots: vec![slot(10, 0)],
            ..crate::group::Formation::default()
        });
        assert!(!t.move_update(1, &mut relayout).unwrap());
        assert_eq!(t.groups.get(1).unwrap().current_command, -1);
    }

    /// `AutomaticMoveUnitsToDesiredPositions`: a slot without a unit, of a unit the world does
    /// not hold or that is busy, or in its place (on no attack), is passed; a free unit out of
    /// its place gets a path to it, not modelled.
    #[test]
    fn units_out_of_their_places_get_paths() {
        let mut t = world();
        let slot = |unit, state| crate::group::Slot {
            unit_id: unit,
            state,
            ..crate::group::Slot::default()
        };
        t.players[1].objects[1].ai.as_mut().unwrap().current_action = 0x2c6;
        t.groups.get_mut(1).unwrap().current_formation = Some(crate::group::Formation {
            slots: vec![slot(-1, 0), slot(99, 0), slot(11, 0), slot(10, 3)],
            ..crate::group::Formation::default()
        });
        t.move_to_desired_positions(1, 1, -1).unwrap();
        t.move_to_desired_positions(1, 9, -1).unwrap();
        assert!(t.move_to_desired_positions(1, 9, 20).is_err());
        t.groups
            .get_mut(1)
            .unwrap()
            .current_formation
            .as_mut()
            .unwrap()
            .slots[3]
            .state = 0;
        assert!(t.move_to_desired_positions(1, 1, -1).is_err());
    }

    /// The members' own moves: a member moved aside (reason 2) that is free stops, counts its
    /// timer down and, at 0, drops its move and stands; a busy one or one without an active
    /// move is passed; another reason's step is not modelled.
    #[test]
    fn a_member_moved_aside_waits_out_its_timer() {
        let mut t = world();
        t.dt_seconds = 0.5;
        for u in &mut t.players[1].objects {
            u.mover = Some(crate::hunt::tests::walker());
        }
        let aside = crate::group::SpecificMove {
            active: 1,
            path_id: 4,
            reason: 2,
            timer: 0.75,
            ..crate::group::SpecificMove::default()
        };
        let g = t.groups.get_mut(1).unwrap();
        g.members[0].specific_move = aside;
        g.members[1].specific_move = aside;
        t.players[1].objects[1].ai.as_mut().unwrap().current_action = 0x2c6;
        t.specific_moves_step(1).unwrap();
        let g = t.groups.get(1).unwrap();
        assert_eq!(g.members[0].specific_move.timer, 0.25);
        assert_eq!(g.members[1].specific_move, aside);
        assert!(
            t.object(10)
                .unwrap()
                .1
                .mover
                .as_ref()
                .unwrap()
                .speed_stopped
        );
        assert!(
            !t.object(11)
                .unwrap()
                .1
                .mover
                .as_ref()
                .unwrap()
                .speed_stopped
        );
        t.specific_moves_step(1).unwrap();
        let g = t.groups.get(1).unwrap();
        assert_eq!(
            g.members[0].specific_move,
            crate::group::SpecificMove {
                path_id: -1,
                reason: -1,
                ..crate::group::SpecificMove::default()
            }
        );
        // a NaN timer keeps the move
        let g = t.groups.get_mut(1).unwrap();
        g.members[0].specific_move = crate::group::SpecificMove {
            timer: f32::NAN,
            ..aside
        };
        t.specific_moves_step(1).unwrap();
        assert_eq!(t.groups.get(1).unwrap().members[0].specific_move.active, 1);
        // any other reason takes the unit's own motion step: without a path it is done at
        // once, and the move goes
        t.groups.get_mut(1).unwrap().members[0].specific_move.reason = 1;
        t.specific_moves_step(1).unwrap();
        assert_eq!(t.groups.get(1).unwrap().members[0].specific_move.active, 0);
    }

    /// A member's own move of the way round (reason 0) steps the unit along its path (slot
    /// 0x608): moving on (the step turns it toward its waypoint; the velocity update moves it
    /// later), the move's distance loses the ground distance from its last position to where the
    /// unit last moved to (here 1 tile short of it on y), which becomes its last position.
    #[test]
    fn a_member_s_own_move_walks_its_path() {
        let mut t = world();
        t.dt_seconds = 0.5;
        let mut walker = crate::hunt::tests::walker();
        walker.moved_at = [1.0, 1.0, 0.0];
        walker.take_record_path(vec![[1.0, 0.0, 1.0], [5.0, 0.0, 1.0]], 4.0, 0, false);
        t.players[1].objects[0].mover = Some(walker);
        t.spatial.insert(
            10,
            crate::spatial::Placed {
                position: [1.0, 1.0, 0.0],
                radius: [0.2, 0.2],
                class: 6,
                player: 1,
                master: 4,
                obstruction_type: 0,
                buildable_on: 0,
                tall: false,
                speed: 0.8,
                unknown: None,
                obstruction: None,
            },
        );
        t.groups.get_mut(1).unwrap().members[0].specific_move = crate::group::SpecificMove {
            active: 1,
            path_id: -1,
            reason: 0,
            timer: 3.0,
            distance: 3.0,
            last_position: [1.0, 0.0, 0.0],
            ..crate::group::SpecificMove::default()
        };
        t.specific_moves_step(1).unwrap();
        let (_, u) = t.object(10).unwrap();
        let at = crate::group::motion::moved_vector(u);
        let own = t.groups.get(1).unwrap().members[0].specific_move;
        assert_eq!(own.active, 1);
        assert_eq!((at, own.last_position), ([1.0, 0.0, 1.0], [1.0, 0.0, 1.0]));
        assert_eq!(own.distance, 2.0);
    }

    /// `FUN_14100d940`: the orders whose time has come go; later ones stay.
    #[test]
    fn due_orders_leave_the_queue() {
        let mut t = world();
        t.world_time = 100;
        let order = |execute_time| crate::group::GroupOrder {
            execute_time,
            ..crate::group::GroupOrder::default()
        };
        t.groups.get_mut(1).unwrap().order_queue = vec![order(99), order(101), order(100)];
        t.drop_due_orders(1);
        assert_eq!(t.groups.get(1).unwrap().order_queue, [order(101)]);
    }

    /// The idle timer: an idle group adds the update's milliseconds and draws anew past 3000; a
    /// busy one draws anew. The milliseconds come back exactly from the update's seconds.
    #[test]
    fn the_idle_timer_counts_while_idle() {
        let mut t = world();
        t.dt_seconds = crate::lifecycle::seconds(69);
        t.groups.get_mut(1).unwrap().idle_timer = 2931;
        t.idle_timer_step(1);
        assert_eq!(t.groups.get(1).unwrap().idle_timer, 3000);
        let mut streams = t.streams.clone();
        let draw = streams.get(crate::rng::Stream::World).rand15();
        t.idle_timer_step(1);
        assert_eq!(t.groups.get(1).unwrap().idle_timer, draw % 750);
        t.groups.get_mut(1).unwrap().motion.velocity = [0.0, 0.0, 1.0];
        t.groups.get_mut(1).unwrap().idle_timer = 5;
        t.idle_timer_step(1);
        assert!(t.groups.get(1).unwrap().idle_timer < 750);
        for dt in 0..=100_000 {
            t.dt_seconds = crate::lifecycle::seconds(dt);
            assert_eq!(t.dt_ms(), dt);
        }
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

    /// `BaseWorld::update` runs a player's groups (`FUN_14103d910` at `0x14155179b`) before the
    /// player's own update (slot 0x100 at `0x1415518f0`, which updates its objects): a group
    /// whose member leaves it (no unit AI) stops the member, which then does not move in the
    /// same update.
    #[test]
    fn a_player_s_groups_update_before_its_objects() {
        let mut t = world();
        let walker = crate::hunt::tests::walker();
        for u in &mut t.players[1].objects {
            u.mover = Some(walker.clone());
            u.mover.as_mut().unwrap().moved_at = u.position;
            u.ai = None;
        }
        let before = t.object(10).unwrap().1.position;
        t.tick(69);
        let u = t.object(10).unwrap().1;
        assert_eq!(u.group.id, -1);
        assert_eq!(u.position, before);
        assert_eq!(u.mover.as_ref().unwrap().velocity.unwrap().velocity[2], 0.0);
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
