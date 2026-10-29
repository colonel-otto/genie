//! A group's statistics (FD-036): `FUN_14100bcb0`, the centroid, radii, ranges, spacing and
//! speed of the members, the objects their searches list, and the formation's type after a
//! change of members (`FUN_14100b940`).

use crate::GenieError;
use crate::groups::create::FormationUnit;
use crate::unit_ai::ActionKind;
use crate::world::World;

/// A group whose `mGroupFlags` bit 9 is set moves at its slowest member's speed divided by this
/// (`FUN_14100bcb0`, `DAT_142ed66b4`).
const SLOW_GROUP_FACTOR: f32 = crate::math::image::group::SLOW_GROUP_FACTOR;
/// A member farther than this from its slot leaves the group (`FUN_14100bcb0`'s far test,
/// `DAT_142e7f38c`).
const FAR: f32 = crate::math::image::group::FAR;
/// The far test's distance in a formation of type 7, or once a member of radius
/// [`WIDE_RADIUS`] or more came before (`DAT_142ecbff0`).
const FAR_WIDE: f32 = crate::math::image::group::FAR_WIDE;
/// The radius from which the far test takes [`FAR_WIDE`] (`DAT_142e7f384`).
const WIDE_RADIUS: f32 = crate::math::image::group::WIDE_RADIUS;
/// The types of action objects (`RGE_Action_Object`), whose action list slot 0x538 reads.
const ACTION_KIND: u8 = 40;
/// The objects of the members' searches a group keeps at most (`FUN_140c65c60`'s `0x46`).
const MOST_OBJECTS: usize = 70;

/// The members' views in the members' order, each with the index the member has then.
type MemberViews = Vec<(usize, MemberView)>;

/// What the statistics read of one member.
struct MemberView {
    id: i32,
    unit: FormationUnit,
    /// Slot 0x520's vector (x, height, y): where the unit last moved to.
    at: [f32; 3],
    /// Slot 0x208: its top speed.
    speed: f32,
    /// Slot 0x230: its weapon range.
    range: f32,
    /// `+0x685`, `under_attack_value` (`FUN_1411a6f80`).
    under_attack: bool,
    /// Its unit AI's current action is the attack.
    attacking: bool,
}

impl World {
    /// `FUN_14100bcb0` with `far` (`param_2`): the statistics start over (the centroid, the
    /// largest radius and range, the spacing sums and the searches' objects at 0, the smallest
    /// radius, range and ranged range at `f32::MAX`); each member the world holds then, in the
    /// members' order, lists the objects its unit AI's search finds ([`World::member_objects`]);
    /// with `far`, one far from its slot ([`World::far_from_slot`]) leaves the group (`removeUnit`
    /// with target -1, [`World::remove_unit`]) and, when it left, stops (slot 0x5d0) and the next
    /// member takes its index; either way it counts in no statistic. Any other
    /// adds its place (slot 0x520) to the centroid and its radius ([`FormationUnit`]) to the
    /// spacing, and takes part in the largest and smallest radius, the slowest speed (slot
    /// 0x208) and the ranges (slot 0x230; the ranged ones only above 0); its seconds since
    /// attacking are 0 in the attack action (600), else they grow by the update's time. With
    /// members, the centroid and the desired height are their means, the spacing twice the largest
    /// radius, the total spacing `(largest + sum) + (largest + sum)` in that order, the stored
    /// positions' desired length the members' count times the spacing; the motion's top speed is
    /// the slowest member's (divided by [`SLOW_GROUP_FACTOR`] with flags bit 9), the seconds not
    /// under attack 0 when a member is under attack, else they grow by the update's time; a
    /// member inside another object stops ([`World::stop_if_inside`]); each sub-formation's
    /// spacing is twice its widest unit's radius and its attack range its units' largest against
    /// the group's target ([`World::range_against`]), its units looked up in the world's table;
    /// with the motion flags' bit 1 the formation type follows
    /// ([`World::calc_formation_type`]). Without one the smallest radius, the spacing and the
    /// smallest ranges are 0.
    ///
    /// Not modelled: `+0x478` (a member's `+0x30c`; the group holds no such field), a charge's
    /// range against a target, and the attack plan's re-attack (`FUN_14100f7f0`) against a
    /// target the world holds.
    pub fn group_statistics(&mut self, group: i32, far: bool) -> Result<(), GenieError> {
        if self.statistics_only(group, far)? {
            self.calc_formation_type(group)?;
        }
        Ok(())
    }

    /// [`World::group_statistics`] up to the formation type: true when that follows (motion
    /// flags bit 1).
    ///
    /// # Panics
    ///
    /// When the world does not hold `group`: the game's routine gets the group itself, from a
    /// caller that holds it.
    pub fn statistics_only(&mut self, group: i32, far: bool) -> Result<bool, GenieError> {
        let dt = self.dt_seconds;
        let (views, objects) = self.member_views(group, far)?;
        let g = self.groups.get_mut(group).expect("the group read above");
        let (held, mut slowest, under_attack) = member_totals(g, &views, objects, dt);
        if held == 0 {
            g.smallest_unit_radius = 0.0;
            g.desired_spacing = 0.0;
            g.smallest_unit_range = 0.0;
            g.smallest_ranged_unit_range = 0.0;
            return Ok(false);
        }
        // a member inside another object (`+0x2f0`) stops (slot 0x5d0), in the members' order
        for (_, v) in &views {
            self.stop_if_inside(v.id)?;
        }
        let g = self.groups.get_mut(group).expect("the group read above");
        let n = held as f32;
        g.centroid = [g.centroid[0] / n, g.centroid[1] / n, g.centroid[2] / n];
        g.desired_height /= n;
        let largest = g.largest_unit_radius;
        g.desired_spacing = largest + largest;
        g.total_spacing = largest + g.total_spacing + largest + g.total_spacing;
        g.stored_positions.desired_length = g.members.len() as f32 * (largest + largest);
        if g.flags >> 9 & 1 != 0 {
            slowest /= SLOW_GROUP_FACTOR;
        }
        g.motion.maximum_velocity_forward = slowest;
        if under_attack {
            g.seconds_not_under_attack = 0.0;
        } else {
            g.seconds_not_under_attack += dt;
        }
        let old_plan = g.attack_plan_type;
        g.attack_plan_type = 0;
        let target = g.current_target.target;
        // each sub-formation with units: its attack range the largest of its units' slot 0x238
        // against the group's target, its spacing twice its widest unit (the first unit's
        // place counts even when the world does not hold it); its units are looked up in the
        // world's table, members or not
        self.sub_formation_stats(group, target)?;
        let g = self.groups.get(group).expect("the group read above");
        // the re-attack: an attack plan of 1 before, command 9 (attack) and a target the
        // world's table holds; a target it does not hold does nothing
        if old_plan == 1
            && g.current_command == 9
            && target != -1
            && self.world_object(target).is_some()
        {
            return Err(GenieError::unsupported(
                "a group's attack plan that attacks again (FUN_14100f7f0)",
            ));
        }
        // the motion flags' bit 1 (`+0xb8`, 0x14100c657), not the group flags'
        Ok(g.motion.flags >> 1 & 1 != 0)
    }

    /// [`World::statistics_only`]'s member loop: each member's view in the members' order with the
    /// index it has then, and the objects of their searches.
    fn member_views(
        &mut self,
        group: i32,
        far: bool,
    ) -> Result<(MemberViews, Vec<i32>), GenieError> {
        // each member's view in the members' order, its search first (FUN_140c65c60 before
        // anything else of it), then the far test; the index the member has then, which a
        // later member's leaving does not move
        let mut views: MemberViews = Vec::new();
        let mut objects = Vec::new();
        // `+0x154` as the loop goes, which the far test reads
        let mut largest = 0.0f32;
        let mut i = 0;
        loop {
            let g = self.groups.get(group).expect("the caller holds the group");
            let Some(m) = g.members.get(i) else {
                break;
            };
            let id = m.id;
            let Some((p, u)) = self.object(id) else {
                i += 1;
                continue;
            };
            let master = u.master(&self.masters, p)?;
            let unit = FormationUnit::of(master);
            let ai =
                u.ai.as_ref()
                    .ok_or(GenieError::unsupported("a group member without a unit AI"))?;
            let view = MemberView {
                id,
                unit,
                // the game's vector (x, height, y) of where it last moved to (x, y, height)
                at: u.body.mover().map_or_else(
                    || {
                        let [x, y, h] = u.position();
                        [x, h, y]
                    },
                    |m| [m.moved_at[0], m.moved_at[2], m.moved_at[1]],
                ),
                speed: u.body.mover().map_or(0.0, |m| m.stats.speed),
                range: weapon_range(master),
                under_attack: u.under_attack,
                attacking: ai.current_action == Some(ActionKind::Attack),
            };
            self.member_objects(id, &mut objects)?;
            if far && self.far_from_slot(group, &view, largest)? {
                if self.remove_unit(group, id, -1, false)? {
                    if let Some(m) = self.object_mut(id).and_then(|(_, u)| u.body.mover_mut()) {
                        m.stop()?;
                    }
                    // the index steps back: the next member takes this one's place
                    continue;
                }
                i += 1;
                continue;
            }
            if largest < view.unit.radius {
                largest = view.unit.radius;
            }
            views.push((i, view));
            i += 1;
        }
        Ok((views, objects))
    }

    /// [`World::statistics_only`]'s sub-formations: each one with units gets its attack range
    /// against `target` and its spacing.
    fn sub_formation_stats(&mut self, group: i32, target: i32) -> Result<(), GenieError> {
        let g = self.groups.get(group).expect("the caller holds the group");
        let stats: Vec<Vec<i32>> = g.unit_types.iter().map(|u| u.units.clone()).collect();
        for (n, units) in stats.iter().enumerate() {
            if units.is_empty() {
                continue;
            }
            let (mut range, mut widest) = (0.0f32, 0.0f32);
            for (k, &id) in units.iter().enumerate() {
                let Some(object) = self.world_object(id) else {
                    continue;
                };
                let r = self.range_against(object, self.world_object(target))?;
                // COMISS/JBE at 0x14100c506 and 0x14100c51e
                if range < r {
                    range = r;
                }
                let radius = match object {
                    crate::groups::WorldObject::Unit(p, u) => {
                        FormationUnit::of(u.master(&self.masters, p)?).radius
                    }
                    crate::groups::WorldObject::Ghost(p, _) => {
                        FormationUnit::of(self.master(p, object.master_id())?).radius
                    }
                };
                if k == 0 || widest < radius {
                    widest = radius;
                }
            }
            let g = self.groups.get_mut(group).expect("the group read above");
            g.unit_types[n].attack_range = range;
            g.unit_types[n].desired_spacing = widest + widest;
        }
        Ok(())
    }

    /// `CalcFormationType` (`FUN_14100b940`): the members' unit AIs' formation types (`+0x1d4`,
    /// those below 9) counted, the most common the type (the first of equals); one below 3 unlocks
    /// the group (`mFormationLockedType` 0) and makes its formation anew
    /// ([`World::update_desired_positions`] with -1), the members' stances its state and their
    /// formation types 0 ([`World::stances_state`]); a higher one locks the group to it
    /// ([`World::lock_formation`]).
    ///
    /// # Panics
    ///
    /// When the world does not hold `group`: the game's routine gets the group itself, from a
    /// caller that holds it.
    pub fn calc_formation_type(&mut self, group: i32) -> Result<(), GenieError> {
        let ids: Vec<i32> = self
            .groups
            .get(group)
            .expect("the caller holds the group")
            .members
            .iter()
            .map(|m| m.id)
            .collect();
        let mut counts = [0i32; 9];
        for &id in &ids {
            if let Some(ai) = self.object(id).and_then(|(_, u)| u.ai.as_ref())
                && ai.formation_type < 9
            {
                counts[ai.formation_type as usize] += 1;
            }
        }
        let mut best = 0;
        for k in 1..9 {
            if counts[best] < counts[k] {
                best = k;
            }
        }
        if 3 <= best {
            return self.lock_formation(group, best as i32);
        }
        if let Some(g) = self.groups.get_mut(group) {
            g.formation_locked_type = 0;
        }
        self.update_desired_positions(group, -1)?;
        self.stances_state(group, 0);
        // FUN_1415417a0 finds sleeping members too
        for &id in &ids {
            if let Some(ai) = self.object_mut(id).and_then(|(_, u)| u.ai.as_mut()) {
                ai.formation_type = 0;
            }
        }
        Ok(())
    }

    /// `LockFormation` (`FUN_141024e70`) of group `group` to type `kind`: a group without a
    /// spacing yet (`mDesiredSpacing` 0) takes its held members' widest radius twice
    /// ([`FormationUnit`]); a column or a line (1, 2) unlocks it (`mFormationLockedType` 0) to
    /// its own choice ([`World::update_desired_positions`] with -1), any other type locks it to
    /// that type and lays it; then the members' stances give its state and their formation
    /// types are the lock's ([`World::stances_state`]). The analytics event it sends (each
    /// member's master counted, `FUN_140986b60`) changes nothing in the world.
    ///
    /// # Errors
    ///
    /// What [`World::update_desired_positions`] does not
    /// model.
    ///
    /// # Panics
    ///
    /// When the world does not hold `group`: the game's routine gets the group itself, from a
    /// caller that holds it.
    pub fn lock_formation(&mut self, group: i32, kind: i32) -> Result<(), GenieError> {
        let g = self.groups.get(group).expect("the caller holds the group");
        // the spacing is read again for every member held (0x14102500a): the first whose
        // radius gives one above it sets it, and every later member finds it set
        let mut spacing = g.desired_spacing;
        for m in &g.members {
            if spacing != 0.0 {
                break;
            }
            if let Some((p, u)) = self.object(m.id) {
                let unit = FormationUnit::of(u.master(&self.masters, p)?);
                let s = unit.radius * crate::math::image::group::SPACING_FACTOR;
                if spacing <= s && s != spacing {
                    spacing = s;
                }
            }
        }
        self.groups
            .get_mut(group)
            .expect("the group found above")
            .desired_spacing = spacing;
        let own_choice = matches!(kind, 1 | 2);
        let locked = if own_choice { 0 } else { kind };
        self.groups
            .get_mut(group)
            .expect("the group found above")
            .formation_locked_type = locked;
        self.update_desired_positions(group, if own_choice { -1 } else { kind })?;
        self.stances_state(group, locked as u8);
        Ok(())
    }

    /// `FUN_14100d140` with 1: the group's state (`mState`) is the members' most common stance
    /// (`+0x1a9`: 0 aggressive, 1 defensive, 2 stand ground, 3 no attack; on equal counts the
    /// lower), 0 without members whose unit AI the world holds; each such member's formation
    /// type (`+0x1d4`) becomes `formation_type`.
    pub fn stances_state(&mut self, group: i32, formation_type: u8) {
        let Some(g) = self.groups.get(group) else {
            return;
        };
        let ids: Vec<i32> = g.members.iter().map(|m| m.id).collect();
        let mut n = [0i32; 4];
        for &id in &ids {
            if let Some(ai) = self.object(id).and_then(|(_, u)| u.ai.as_ref())
                && ai.stance < 4
            {
                n[ai.stance as usize] += 1;
            }
        }
        let state = if ids.is_empty() || (n[1] <= n[0] && n[2] <= n[0] && n[3] <= n[0]) {
            0
        } else if n[2] <= n[1] && n[3] <= n[1] {
            1
        } else if n[3] <= n[2] {
            2
        } else {
            3
        };
        if let Some(g) = self.groups.get_mut(group) {
            g.state = state;
        }
        // FUN_1415417a0 finds sleeping members too
        for &id in &ids {
            if let Some(ai) = self.object_mut(id).and_then(|(_, u)| u.ai.as_mut()) {
                ai.formation_type = formation_type;
            }
        }
    }

    /// `FUN_140c65c60` with -1 and [`MOST_OBJECTS`] for member `id` into the group's
    /// sighted objects (`+0x49c`, its count `+0x5b4`, [`crate::groups::UnitGroup::sighted`]):
    /// nothing once they are full. Else its unit AI's
    /// slot 0x50 with 1 from its place (x, y): the search (`FUN_140c63e20` mode 1, every type's,
    /// [`crate::unit_ai::Pick::Look`] over its type's area, [`crate::unit_ai::search_area`]),
    /// which, when it queried
    /// afresh, sets the alert mode (`+0x1b8`) to whether it found enemies and, with enemies,
    /// restarts the time since a sighting (`+0x1b4`; a computer player's notify after 10 s is
    /// not modelled). Then the objects of the search's lists 4, 3 and 2 in that order
    /// ([`crate::map::spatial::Spatial::searched`]) that the world's table holds in a state below 3
    /// (`+0x640`), of any owner, the third list's only of a class the unit AI attacks among
    /// neutrals (slot 0xa8, [`crate::unit_ai::type_attacks_neutral`]), join the list: each
    /// once when it held any on entry (the duplicate test runs only then), and the list stops at
    /// [`MOST_OBJECTS`].
    ///
    /// # Errors
    ///
    /// A member without a unit AI (the game calls through its null pointer), of a unit-AI type
    /// whose classes are not held or of a computer player (its notify), and the search's.
    fn member_objects(&mut self, id: i32, objects: &mut Vec<i32>) -> Result<(), GenieError> {
        if MOST_OBJECTS <= objects.len() {
            return Ok(());
        }
        let on_entry = objects.len();
        let (p, u) = self.object(id).ok_or(GenieError::unsupported(
            "a group member's search for a unit not held",
        ))?;
        let ai =
            u.ai.as_ref()
                .ok_or(GenieError::unsupported("a group member without a unit AI"))?;
        let ai_type = ai.ai_type;
        if self.players[p].computer {
            return Err(GenieError::unsupported(
                "a computer player's group member's search (its cared classes)",
            ));
        }
        let master = u.master(&self.masters, p)?;
        let attack_priority = master.attack_priority;
        let (classes, radius) = crate::unit_ai::search_area(
            ai_type,
            master.class,
            master.line_of_sight,
            master.search_radius,
            false,
        )?;
        let search = crate::unit_ai::Search {
            at: [u.position()[0], u.position()[1]],
            radius,
            classes,
            size: master.radius,
            pick: crate::unit_ai::Pick::Look,
            memory: crate::unit_ai::Memory::default(),
        };
        let fresh = self.spatial.search_query(p, &search)?;
        let lists = self.spatial.searched().ok_or(GenieError::unsupported(
            "a group member's search that reads the registry's last answer (before any query)",
        ))?;
        if fresh {
            let enemies = !lists[4].is_empty();
            if let Some(ai) = self.object_mut(id).and_then(|(_, u)| u.ai.as_mut()) {
                ai.alert_mode = enemies;
                if enemies {
                    ai.time_since_enemy = 0.0;
                }
            }
        }
        for list in [4usize, 3, 2] {
            for record in &lists[list] {
                let Some(object) = self.world_object(record.id) else {
                    continue;
                };
                let state = match object {
                    crate::groups::WorldObject::Unit(_, u) => u.life.state,
                    crate::groups::WorldObject::Ghost(_, g) => g.state,
                };
                if 3 <= state {
                    continue;
                }
                if list == 3 {
                    let class = self.master(object.player(), object.master_id())?.class;
                    if !crate::unit_ai::type_attacks_neutral(ai_type, attack_priority, class) {
                        continue;
                    }
                }
                if 0 < on_entry && objects.contains(&record.id) {
                    continue;
                }
                objects.push(record.id);
                if MOST_OBJECTS <= objects.len() {
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    /// Slot 0x238 of `object` against `target` (the group's, as the world's table gives it): a
    /// combat object's (types 50 to 80, `FUN_14108d9b0`) weapon range (slot 0x230,
    /// [`weapon_range`]) plus its master's `charge_event` (`+0x920`) when slot 0x2f8
    /// (`FUN_14120f510`, [`crate::combat::damage::range_charge_hit`]) holds against the target, or
    /// without one its top action's target: a charge of type 6 or 7 (slot 0x308) that works on
    /// it ([`crate::combat::damage::charge_works`]) and is ready (slot 0x300: at least 1); any other
    /// object's 0 (`FUN_14106ff60`).
    ///
    /// # Errors
    ///
    /// An object of a master the slice does not have.
    fn range_against(
        &self,
        object: crate::groups::WorldObject<'_>,
        target: Option<crate::groups::WorldObject<'_>>,
    ) -> Result<f32, GenieError> {
        let master = self.master(object.player(), object.master_id())?;
        if !COMBAT_KINDS.contains(&master.kind) {
            return Ok(0.0);
        }
        let range = weapon_range(master);
        let crate::groups::WorldObject::Unit(_, u) = object else {
            return Ok(range);
        };
        let target = target.or_else(|| {
            u.top_target()
                .filter(|&t| -1 < t)
                .and_then(|t| self.world_object(t))
        });
        let Some(t) = target else {
            return Ok(range);
        };
        if !matches!(master.charge_type, 6 | 7) {
            return Ok(range);
        }
        let tm = self.master(t.player(), t.master_id())?;
        let works = crate::combat::damage::charge_works(
            master.charge_target,
            master.charge_type,
            tm.class,
            tm.kind,
        );
        // slot 0x300 of a charge type other than 1 and 3: at least 1 (`COMISS`, a NaN is not)
        Ok(if works && 1.0 <= u.charge {
            range + f32::from(master.charge_event)
        } else {
            range
        })
    }

    /// `FUN_14100bcb0`'s far test (`0x14100be04`..`0x14100c1c9`, `param_2` 1) of member
    /// `view` of group `group`, `largest` the largest radius of the members before it: the
    /// group walks no patrol (the top path's flags bit 0, `FUN_141046500`), its flags lack bit 11,
    /// the member's action list holds no Attack (slot 0x538 with 9) and it has a slot in the
    /// formation (the first by its id) farther on the ground from where it last moved to (slot
    /// 0x520) than [`FAR`], or [`FAR_WIDE`] in a formation of type 7 or once a member of radius
    /// [`WIDE_RADIUS`] or more came before (`COMISS` with `JC`: a NaN radius is not wide;
    /// `COMISS` with `JBE`: a NaN distance is not far).
    ///
    /// # Errors
    ///
    /// An action the slice does not model, and a member of a master the slice does not have.
    fn far_from_slot(
        &self,
        group: i32,
        view: &MemberView,
        largest: f32,
    ) -> Result<bool, GenieError> {
        let g = self.groups.get(group).expect("the caller holds the group");
        let patrol = crate::groups::formation::top_path(&g.motion.paths)
            .is_some_and(|p| p.path_flags & 1 != 0);
        if patrol || g.flags >> 11 & 1 != 0 {
            return Ok(false);
        }
        let (p, u) = self.object(view.id).expect("the member is held");
        if u.master(&self.masters, p)?.kind >= ACTION_KIND {
            if let Some(e) = u.unmodelled_action.clone() {
                return Err(e);
            }
            if u.top_action_kind() == Some(crate::combat::attack::ATTACK) {
                return Ok(false);
            }
        }
        let Some(f) = g.current_formation.as_ref() else {
            return Ok(false);
        };
        let Some(slot) = f.slots.iter().find(|s| s.unit_id == view.id) else {
            return Ok(false);
        };
        let mut d = crate::math::fmath::sub(view.at, slot.transform_position);
        d[1] = 0.0;
        let limit = if f.kind == 7 || !crate::math::fmath::below_or_unordered(largest, WIDE_RADIUS)
        {
            FAR_WIDE
        } else {
            FAR
        };
        let gap = crate::math::fmath::sqrt(d[0] * d[0] + 0.0 + d[2] * d[2]);
        Ok(!crate::math::fmath::at_most_or_unordered(gap, limit))
    }

    /// A member inside another object (`+0x2f0`: another object's garrison holds it) stops
    /// (slot 0x5d0, `FUN_14108c2f0`, [`crate::movement::Mover`]'s stop), as `FUN_14100bcb0` does
    /// for each such member it reads.
    ///
    /// # Errors
    ///
    /// What the stop does not model.
    fn stop_if_inside(&mut self, id: i32) -> Result<(), GenieError> {
        let inside = self
            .players
            .iter()
            .flat_map(|q| q.objects.iter().chain(&q.sleeping))
            .any(|o| o.garrison.contains(&id));
        if !inside {
            return Ok(());
        }
        if let Some(m) = self.object_mut(id).and_then(|(_, u)| u.body.mover_mut()) {
            m.stop()?;
        }
        Ok(())
    }
}

/// The master types of the combat objects (`RGE_Combat_Object` and its subclasses: combat,
/// missile, TRIBE combat and building), whose slot 0x230 is `FUN_14108d960`.
const COMBAT_KINDS: [u8; 4] = [50, 60, 70, 80];

/// [`World::statistics_only`]'s totals: the group's statistics reset, then the members' views
/// added up (centroid, radii, ranges, attack times), `objects` its search list; the members
/// counted, the slowest speed and whether any member is under attack.
fn member_totals(
    g: &mut crate::groups::UnitGroup,
    views: &[(usize, MemberView)],
    objects: Vec<i32>,
    dt: f32,
) -> (i32, f32, bool) {
    g.centroid = [0.0; 3];
    g.desired_height = 0.0;
    g.largest_unit_radius = 0.0;
    g.smallest_unit_radius = f32::MAX;
    g.desired_spacing = 0.0;
    g.smallest_unit_range = f32::MAX;
    g.smallest_ranged_unit_range = f32::MAX;
    g.largest_unit_range = 0.0;
    g.largest_ranged_unit_range = 0.0;
    g.total_spacing = 0.0;
    g.sighted = objects;
    let mut slowest = f32::MAX;
    let mut held = 0;
    let mut under_attack = false;
    for &(i, ref v) in views {
        held += 1;
        g.centroid[0] += v.at[0];
        g.centroid[1] += v.at[1];
        g.centroid[2] += v.at[2];
        let r = v.unit.radius;
        g.total_spacing += r;
        if g.largest_unit_radius < r {
            g.largest_unit_radius = r;
        }
        if r < g.smallest_unit_radius {
            g.smallest_unit_radius = r;
        }
        if v.speed < slowest {
            slowest = v.speed;
        }
        if v.range < g.smallest_unit_range {
            g.smallest_unit_range = v.range;
        }
        if g.largest_unit_range < v.range {
            g.largest_unit_range = v.range;
        }
        if 0.0 < v.range && v.range < g.smallest_ranged_unit_range {
            g.smallest_ranged_unit_range = v.range;
        }
        if 0.0 < v.range && g.largest_ranged_unit_range < v.range {
            g.largest_ranged_unit_range = v.range;
        }
        under_attack |= v.under_attack;
        let m = &mut g.members[i];
        if v.attacking {
            m.seconds_since_attacking = 0.0;
        } else {
            m.seconds_since_attacking += dt;
        }
    }
    (held, slowest, under_attack)
}

/// Slot 0x230 of an object of `master`: a combat object's weapon range (`+0x828`,
/// `FUN_14108d960`), any other's 0 (`FUN_14106ff50`).
pub(crate) fn weapon_range(master: &crate::objects::masters::Master) -> f32 {
    if COMBAT_KINDS.contains(&master.kind) {
        master.weapon_range
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::groups::{Member, UnitGroup, UnitGroups, UnitTypeStatistic};
    use crate::objects::masters::{Master, Masters};
    use crate::objects::unit::Unit;
    use crate::vision::registry::UnitUpdate;
    use crate::world::Player;

    /// Slot 0x238 (`FUN_14108d9b0`): the weapon range, plus `charge_event` for a ready charge of
    /// type 6 or 7 that works on the target (here every class, −1); without the charge ready, of
    /// another type, on no target (and no action's) or on a class outside its groups, the
    /// weapon range alone.
    #[test]
    fn a_ready_charge_of_type_6_adds_its_charge_event_to_the_range() {
        let mut t = searched_world();
        let charging = |t: &mut World, charge_type: i16, charge_target: i16| {
            let m = t.masters[1].get_mut(4).unwrap();
            (m.charge_type, m.charge_target, m.charge_event) = (charge_type, charge_target, 2);
        };
        let range = |t: &World, target: Option<i32>| {
            let me = t.world_object(10).unwrap();
            t.range_against(me, target.and_then(|id| t.world_object(id)))
                .unwrap()
        };
        charging(&mut t, 6, -1);
        assert_eq!(range(&t, Some(20)), 1.0);
        t.players[1].objects[0].charge = 1.0;
        assert_eq!(range(&t, Some(20)), 3.0);
        assert_eq!(range(&t, None), 1.0);
        charging(&mut t, 7, 1 << 5);
        assert_eq!((range(&t, Some(20)), range(&t, Some(31))), (1.0, 3.0));
        charging(&mut t, 1, -1);
        assert_eq!(range(&t, Some(20)), 1.0);
    }

    /// Player 1's soldier 10 (master 4, class 6) at (5, 5) in group 1; player 2 (its enemy)
    /// holds 20 at (6, 5) and 21 (state 3) at (5, 6); player 3 (neutral) holds 30 (class 6) at
    /// (4, 5) and 31 (class 4, a villager) at (5, 4); every one of them in player 1's sight.
    fn searched_world() -> World {
        let unit = |id: i32, master: i16, x: f32, y: f32| {
            let mut u = Unit {
                id,
                body: crate::objects::layers::Body::at([x, y, 0.0]),
                ..Unit::default()
            };
            u.life.master_id = master;
            u.life.state = 2;
            u
        };
        let mut soldier = unit(10, 4, 5.0, 5.0);
        soldier.ai = Some(crate::unit_ai::UnitAi {
            ai_type: crate::unit_ai::SOLDIER_AI,
            ..crate::unit_ai::UnitAi::default()
        });
        let mut dying = unit(21, 4, 5.0, 6.0);
        dying.life.state = 3;
        let players = vec![
            Player::default(),
            Player {
                objects: vec![soldier].into(),
                ..Player::default()
            },
            Player {
                objects: vec![unit(20, 4, 6.0, 5.0), dying].into(),
                ..Player::default()
            },
            Player {
                objects: vec![unit(30, 4, 4.0, 5.0), unit(31, 6, 5.0, 4.0)].into(),
                ..Player::default()
            },
        ];
        let mut t = World::new(0, 0, 0, vec![0; 4], players);
        let master = |id: i16, class: i16| Master {
            id,
            kind: 70,
            class,
            formation_category: 1,
            line_of_sight: 4.0,
            weapon_range: 1.0,
            radius: [0.2, 0.2],
            ..Master::default()
        };
        t.masters = vec![Masters::new([master(4, 6), master(6, 4)]); 4];
        let mut s = crate::map::spatial::Spatial::new([20, 20]);
        s.relations = vec![
            vec![0, 3, 3, 3],
            vec![0, 0, 3, 1],
            vec![0, 3, 0, 3],
            vec![0, 1, 3, 0],
        ];
        s.unit_diplomacy = vec![
            vec![1, 4, 4, 4],
            vec![0, 1, 4, 3],
            vec![0, 4, 1, 4],
            vec![0, 3, 4, 1],
        ];
        let mut r = crate::vision::registry::Registry::new(4, 65);
        for (id, owner, at, class) in [
            (20, 2, [6.0, 5.0], 6),
            (21, 2, [5.0, 6.0], 6),
            (30, 3, [4.0, 5.0], 6),
            (31, 3, [5.0, 4.0], 4),
        ] {
            r.update_unit(UnitUpdate {
                id,
                owner,
                position: at,
                radii: [0.2, 0.2],
                class,
                old: 0,
                new: 1 << 1,
            })
            .unwrap();
        }
        let mut v = crate::vision::visibility::Visibility::new(
            20,
            20,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            0,
            Vec::new(),
        );
        v.registry = Some(r);
        s.sight = Some(v);
        t.spatial = s;
        let mut g = UnitGroup::new(1);
        g.owner = 1;
        g.members = vec![Member {
            id: 10,
            ..Member::default()
        }];
        g.unit_types = vec![UnitTypeStatistic {
            formation_category: 1,
            units: vec![10],
            ..UnitTypeStatistic::default()
        }];
        t.groups = UnitGroups {
            groups: vec![g],
            ..UnitGroups::default()
        };
        t
    }

    /// `FUN_140c65c60`: the member's search lists its enemies (list 4: 20, not 21 in state 3),
    /// then the neutrals it attacks (list 3: 30, not the villager 31, slot 0xa8); the fresh
    /// query sets the alert mode for the enemies and restarts the time since a sighting. Again
    /// from the same place (the cache's answer, not fresh) the list held objects on entry, so
    /// nothing joins twice; a list one short of 70 takes one more.
    #[test]
    fn a_member_s_search_lists_its_enemies_then_the_neutrals_it_attacks() {
        let mut t = searched_world();
        t.players[1].objects[0]
            .ai
            .as_mut()
            .unwrap()
            .time_since_enemy = 7.0;
        let mut objects = Vec::new();
        t.member_objects(10, &mut objects).unwrap();
        assert_eq!(objects, [20, 30]);
        let ai = t.players[1].objects[0].ai.as_ref().unwrap();
        assert_eq!((ai.alert_mode, ai.time_since_enemy), (true, 0.0));
        t.member_objects(10, &mut objects).unwrap();
        assert_eq!(objects, [20, 30]);
        let mut full: Vec<i32> = (1000..1069).collect();
        t.member_objects(10, &mut full).unwrap();
        assert_eq!((full.len(), full[69]), (70, 20));
        t.member_objects(10, &mut full).unwrap();
        assert_eq!(full.len(), 70);
    }

    /// Slot 0x50's search and slot 0xa8 by the member's type: a transport (type 11) cares about
    /// neither class around (6, 4), so it lists nothing and finds no enemy; a civilian (8) lists
    /// its enemies within its grown radius (22, 8 tiles off: its line of sight 4 plus 1 plus 7)
    /// but no neutral (its slot 0xa8 never attacks one); a soldier the near ones only.
    #[test]
    fn a_member_s_search_takes_its_type_s_classes_radius_and_neutral_test() {
        let world = |ai_type: u32| {
            let mut t = searched_world();
            let mut far = t.players[2].objects[0].clone();
            far.id = 22;
            far.place_for_test([13.0, 5.0, 0.0]);
            t.players[2].objects.push(far);
            let sight = t.spatial.sight.as_mut().unwrap();
            sight
                .registry
                .as_mut()
                .unwrap()
                .update_unit(UnitUpdate {
                    id: 22,
                    owner: 2,
                    position: [13.0, 5.0],
                    radii: [0.2, 0.2],
                    class: 6,
                    old: 0,
                    new: 1 << 1,
                })
                .unwrap();
            t.players[1].objects[0].ai.as_mut().unwrap().ai_type = ai_type;
            t
        };
        let found = |ai_type: u32| {
            let mut t = world(ai_type);
            let mut objects = Vec::new();
            t.member_objects(10, &mut objects).unwrap();
            (
                objects,
                t.players[1].objects[0].ai.as_ref().unwrap().alert_mode,
            )
        };
        assert_eq!(found(11), (vec![], false));
        assert_eq!(found(crate::unit_ai::CIVILIAN), (vec![20, 22], true));
        assert_eq!(found(crate::unit_ai::SOLDIER_AI), (vec![20, 30], true));
    }

    /// `FUN_14100bcb0`'s sub-formations: a statistic's unit the world holds that is no member
    /// (50, weapon range 6) counts for its attack range; a group target the world does not hold
    /// is none (slot 0x238 is the weapon range), and with it the re-attack does nothing. The
    /// searches fill the sighted objects and leave the units kept apart.
    #[test]
    fn a_sub_formation_s_range_takes_its_held_units_against_the_group_s_target() {
        let mut t = searched_world();
        let mut far = Unit {
            id: 50,
            ..Unit::default()
        };
        far.life.master_id = 7;
        t.players[1].objects.push(far);
        for masters in &mut t.masters {
            let archer = Master {
                id: 7,
                kind: 70,
                class: 0,
                weapon_range: 6.0,
                radius: [0.3, 0.3],
                ..Master::default()
            };
            *masters = Masters::new(masters.iter().cloned().chain([archer]));
        }
        let g = t.groups.get_mut(1).unwrap();
        g.unit_types[0].units.push(50);
        g.current_target.target = 99;
        g.attack_plan_type = 1;
        g.current_command = 9;
        g.objects = vec![77];
        let _ = t.statistics_only(1, false).unwrap();
        let g = t.groups.get(1).unwrap();
        assert_eq!(g.unit_types[0].attack_range, 6.0);
        // the searches fill their own list (+0x49c), not the units kept apart (+0x458)
        assert_eq!((&g.sighted[..], &g.objects[..]), (&[20, 30][..], &[77][..]));
        assert_eq!(g.attack_plan_type, 0);
        // a target the world holds: the re-attack is not modelled
        let g = t.groups.get_mut(1).unwrap();
        g.current_target.target = 20;
        g.attack_plan_type = 1;
        assert!(t.statistics_only(1, false).is_err());
    }

    /// `FUN_14100bcb0` with `far`: member 11, 24 from its slot on the ground and first, leaves
    /// the group (its link cleared) and stops, the next member taking its index, and counts in
    /// no statistic; in a formation of type 7, or after a member of radius 0.5 or more (10's,
    /// before it), the distance is 40 and it stays; none leaves without `far`, on a patrol or
    /// with flags bit 11.
    #[test]
    fn a_member_far_from_its_slot_leaves_the_group() {
        let world = |spacing: f32, order: [i32; 2]| {
            let mut t = searched_world();
            for masters in &mut t.masters {
                *masters = Masters::new(masters.iter().cloned().map(|mut m| {
                    m.height = 1.0;
                    m.spacing_modifier = spacing;
                    m
                }));
            }
            let mut stray = t.players[1].objects[0].clone();
            stray.id = 11;
            stray.place_for_test([6.0, 6.0, 0.0]);
            stray
                .body
                .put_mover(Some(crate::gathering::hunt::tests::walker()));
            stray.body.mover_mut().unwrap().moved_at = [6.0, 6.0, 0.0];
            stray.group.id = 1;
            t.players[1].objects[0].group.id = 1;
            t.players[1].objects.push(stray);
            let g = t.groups.get_mut(1).unwrap();
            g.flags = 0;
            g.members = order
                .map(|id| Member {
                    id,
                    ..Member::default()
                })
                .to_vec();
            g.unit_types[0].units = order.to_vec();
            g.current_formation = Some(crate::groups::Formation {
                slots: vec![
                    crate::groups::Slot {
                        unit_id: 10,
                        transform_position: [5.0, 0.0, 5.0],
                        ..crate::groups::Slot::default()
                    },
                    crate::groups::Slot {
                        unit_id: 11,
                        transform_position: [30.0, 0.0, 6.0],
                        ..crate::groups::Slot::default()
                    },
                ],
                ..crate::groups::Formation::default()
            });
            t
        };
        let ids = |t: &World| -> Vec<i32> {
            t.groups
                .get(1)
                .unwrap()
                .members
                .iter()
                .map(|m| m.id)
                .collect()
        };
        let mut t = world(1.0, [11, 10]);
        let _ = t.statistics_only(1, true).unwrap();
        assert_eq!(ids(&t), [10]);
        let u = t.object(11).unwrap().1;
        assert_eq!(u.group.id, -1);
        assert_eq!(u.body.mover().unwrap().animated.speed, 0.0);
        let g = t.groups.get(1).unwrap();
        assert_eq!(g.centroid, [5.0, 0.0, 5.0]);
        assert_eq!(g.largest_unit_radius, 0.2);
        // a formation of type 7, a wide member before it: 40
        let mut t = world(1.0, [11, 10]);
        t.groups
            .get_mut(1)
            .unwrap()
            .current_formation
            .as_mut()
            .unwrap()
            .kind = 7;
        let _ = t.statistics_only(1, true).unwrap();
        assert_eq!(ids(&t), [11, 10]);
        let mut t = world(2.5, [10, 11]);
        let _ = t.statistics_only(1, true).unwrap();
        assert_eq!(ids(&t), [10, 11]);
        let mut t = world(2.5, [11, 10]);
        let _ = t.statistics_only(1, true).unwrap();
        assert_eq!(ids(&t), [10]);
        // no far test without far, on a patrol, with flags bit 11
        let mut t = world(1.0, [11, 10]);
        let _ = t.statistics_only(1, false).unwrap();
        assert_eq!(ids(&t), [11, 10]);
        let mut t = world(1.0, [11, 10]);
        t.groups.get_mut(1).unwrap().flags = 0x800;
        let _ = t.statistics_only(1, true).unwrap();
        assert_eq!(ids(&t), [11, 10]);
        let mut t = world(1.0, [11, 10]);
        let paths = &mut t.groups.get_mut(1).unwrap().motion.paths;
        paths[0].has_path = true;
        paths[0].path_flags = 1;
        let _ = t.statistics_only(1, true).unwrap();
        assert_eq!(ids(&t), [11, 10]);
    }
}
