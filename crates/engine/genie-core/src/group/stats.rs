//! A group's statistics (FD-036): `FUN_14100bcb0`, the centroid, radii, ranges, spacing and
//! speed of the members, the objects their searches list, and the formation's type after a
//! change of members (`FUN_14100b940`).

use crate::group::create::FormationUnit;
use crate::turn::Turns;
use crate::unit_ai::Unsupported;

/// A group whose `mGroupFlags` bit 9 is set moves at its slowest member's speed divided by this
/// (`FUN_14100bcb0`, `DAT_142ed66b4`).
const SLOW_GROUP_FACTOR: f32 = crate::image::group::SLOW_GROUP_FACTOR;
/// The objects of the members' searches a group keeps at most (`FUN_140c65c60`'s `0x46`).
const MOST_OBJECTS: usize = 70;
/// The current action of attacking (600): a member in it has attacked just now
/// (`mSecondsSinceAttacking` 0).
const ATTACK_ACTION: i32 = 600;

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

impl Turns {
    /// `FUN_14100bcb0` with `far` (`param_2`): the statistics start over (the centroid, the
    /// largest radius and range, the spacing sums and the searches' objects at 0, the smallest
    /// radius, range and ranged range at `f32::MAX`); each member the world holds then, in the
    /// members' order, lists the objects its unit AI's search finds ([`Turns::member_objects`]),
    /// adds its place (slot 0x520) to the centroid and its radius ([`FormationUnit`]) to the
    /// spacing, and takes part in the largest and smallest radius, the slowest speed (slot
    /// 0x208) and the ranges (slot 0x230; the ranged ones only above 0); its seconds since
    /// attacking are 0 in the attack action (600), else they grow by the update's time. With
    /// members, the centroid and the desired height are their means, the spacing twice the largest
    /// radius, the total spacing `(largest + sum) + (largest + sum)` in that order, the stored
    /// positions' desired length the members' count times the spacing; the motion's top speed is
    /// the slowest member's (divided by [`SLOW_GROUP_FACTOR`] with flags bit 9), the seconds not
    /// under attack 0 when a member is under attack, else they grow by the update's time; each
    /// sub-formation's spacing is twice its widest radius and its attack range its members'
    /// largest; with flags bit 1 the formation type follows ([`Turns::calc_formation_type`]).
    /// Without one the smallest radius, the spacing and the smallest ranges are 0.
    ///
    /// Not modelled: `far` (members far from the formation leave it), a member's range against
    /// the group's target (slot 0x238 with one), and the attack plan's re-attack
    /// (`FUN_14100f7f0`).
    pub fn group_statistics(&mut self, group: i32, far: bool) -> Result<(), Unsupported> {
        if self.statistics_only(group, far)? {
            self.calc_formation_type(group)?;
        }
        Ok(())
    }

    /// [`Turns::group_statistics`] up to the formation type: true when that follows (motion
    /// flags bit 1).
    ///
    /// # Panics
    ///
    /// When the world does not hold `group`: the game's routine gets the group itself, from a
    /// caller that holds it.
    pub fn statistics_only(&mut self, group: i32, far: bool) -> Result<bool, Unsupported> {
        if far {
            return Err(Unsupported(
                "a group's statistics that drop far members (FUN_14100bcb0 with 1)",
            ));
        }
        let dt = self.dt_seconds;
        let ids: Vec<i32> = self
            .groups
            .get(group)
            .expect("the caller holds the group")
            .members
            .iter()
            .map(|m| m.id)
            .collect();
        // each member's view, its search first (FUN_140c65c60 before anything else of it)
        let mut views: Vec<Option<MemberView>> = Vec::with_capacity(ids.len());
        let mut objects = Vec::new();
        for &id in &ids {
            let Some((p, u)) = self.object(id) else {
                views.push(None);
                continue;
            };
            let master = self.master_of(p, u.life.master_id)?;
            let unit = FormationUnit::of(master);
            let ai =
                u.ai.as_ref()
                    .ok_or(Unsupported("a group member without a unit AI"))?;
            let view = MemberView {
                id,
                unit,
                // the game's vector (x, height, y) of where it last moved to (x, y, height)
                at: u
                    .mover
                    .as_ref()
                    .map_or([u.position[0], u.position[2], u.position[1]], |m| {
                        [m.moved_at[0], m.moved_at[2], m.moved_at[1]]
                    }),
                speed: u.mover.as_ref().map_or(0.0, |m| m.stats.speed),
                range: master.weapon_range,
                under_attack: u.under_attack,
                attacking: ai.current_action == ATTACK_ACTION,
            };
            let room = MOST_OBJECTS.saturating_sub(objects.len());
            for o in self.member_objects(id, room)? {
                if !objects.contains(&o) {
                    objects.push(o);
                }
            }
            views.push(Some(view));
        }
        let g = self.groups.get_mut(group).expect("the group read above");
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
        g.objects = objects;
        let mut slowest = f32::MAX;
        let mut held = 0;
        let mut under_attack = false;
        for (i, v) in views.iter().enumerate() {
            let Some(v) = v else {
                continue;
            };
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
        if held == 0 {
            g.smallest_unit_radius = 0.0;
            g.desired_spacing = 0.0;
            g.smallest_unit_range = 0.0;
            g.smallest_ranged_unit_range = 0.0;
            return Ok(false);
        }
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
        let has_target = g.current_target.target != -1;
        for stat in &mut g.unit_types {
            if stat.units.is_empty() {
                continue;
            }
            stat.attack_range = 0.0;
            let mut widest = 0.0;
            for (k, id) in stat.units.iter().enumerate() {
                let Some(v) = views.iter().flatten().find(|v| v.id == *id) else {
                    continue;
                };
                if has_target {
                    return Err(Unsupported(
                        "a sub-formation's range against the group's target (slot 0x238)",
                    ));
                }
                if stat.attack_range < v.range {
                    stat.attack_range = v.range;
                }
                if k == 0 || widest < v.unit.radius {
                    widest = v.unit.radius;
                }
            }
            stat.desired_spacing = widest + widest;
        }
        if old_plan == 1 && g.current_command == 9 && has_target {
            return Err(Unsupported(
                "a group's attack plan that attacks again (FUN_14100f7f0)",
            ));
        }
        // the motion flags' bit 1 (`+0xb8`, 0x14100c657), not the group flags'
        Ok(g.motion.flags >> 1 & 1 != 0)
    }

    /// `CalcFormationType` (`FUN_14100b940`): the members' unit AIs' formation types (`+0x1d4`,
    /// those below 9) counted, the most common the type (the first of equals); one below 3 unlocks
    /// the group (`mFormationLockedType` 0) and makes its formation anew
    /// ([`Turns::update_desired_positions`] with -1), the members' stances its state and their
    /// formation types 0 ([`Turns::stances_state`]); a higher one locks the group to it
    /// ([`Turns::lock_formation`]).
    ///
    /// # Panics
    ///
    /// When the world does not hold `group`: the game's routine gets the group itself, from a
    /// caller that holds it.
    pub fn calc_formation_type(&mut self, group: i32) -> Result<(), Unsupported> {
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
    /// its own choice ([`Turns::update_desired_positions`] with -1), any other type locks it to
    /// that type and lays it; then the members' stances give its state and their formation
    /// types are the lock's ([`Turns::stances_state`]). The analytics event it sends (each
    /// member's master counted, `FUN_140986b60`) changes nothing in the world.
    ///
    /// # Errors
    ///
    /// What [`Turns::update_desired_positions`] does not
    /// model.
    ///
    /// # Panics
    ///
    /// When the world does not hold `group`: the game's routine gets the group itself, from a
    /// caller that holds it.
    pub fn lock_formation(&mut self, group: i32, kind: i32) -> Result<(), Unsupported> {
        let g = self.groups.get(group).expect("the caller holds the group");
        // the spacing is read again for every member held (0x14102500a): the first whose
        // radius gives one above it sets it, and every later member finds it set
        let mut spacing = g.desired_spacing;
        for m in &g.members {
            if spacing != 0.0 {
                break;
            }
            if let Some((p, u)) = self.object(m.id) {
                let unit = FormationUnit::of(self.master_of(p, u.life.master_id)?);
                let s = unit.radius * crate::image::group::SPACING_FACTOR;
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

    /// `FUN_140c65c60` with -1 and 70 for member `id`: its unit AI's search from its place (slot
    /// 0x50, `FUN_140c63e20` mode 1, as [`crate::unit_ai::Pick::Look`]), then the objects of its
    /// lists 4, 3 and 2 in that order, of any owner, in a state below 3 (the third list's only of
    /// a class its unit AI cares for, slot 0xa8), each once, into `objects`, at most
    /// [`MOST_OBJECTS`]. Only a soldier's unit AI (13) is modelled, and a search that lists
    /// objects is not.
    fn member_objects(&self, id: i32, room: usize) -> Result<Vec<i32>, Unsupported> {
        if room == 0 {
            return Ok(Vec::new());
        }
        let (p, u) = self
            .object(id)
            .ok_or(Unsupported("a group member's search for a unit not held"))?;
        let ai =
            u.ai.as_ref()
                .ok_or(Unsupported("a group member without a unit AI"))?;
        if ai.ai_type != crate::unit_ai::SOLDIER_AI {
            return Err(Unsupported(
                "a group member's search of a unit-AI type not modelled (FUN_140c65c60)",
            ));
        }
        if self.players[p].computer {
            return Err(Unsupported(
                "a computer player's group member's search (its cared classes)",
            ));
        }
        let master = self.master_of(p, u.life.master_id)?;
        let search = crate::unit_ai::Search {
            at: [u.position[0], u.position[1]],
            radius: crate::unit_ai::search_radius(master.line_of_sight, master.search_radius),
            classes: &crate::unit_ai::SOLDIER_CARED,
            size: master.radius,
            pick: crate::unit_ai::Pick::Look,
        };
        let _ = self.spatial.search_query(p, &search)?;
        let lists = self.spatial.searched().ok_or(Unsupported(
            "a group member's search that reads the registry's last answer (before any query)",
        ))?;
        if lists[2..=4].iter().any(|l| !l.is_empty()) {
            return Err(Unsupported(
                "a group member's search that lists objects (FUN_140c65c60)",
            ));
        }
        Ok(Vec::new())
    }
}
