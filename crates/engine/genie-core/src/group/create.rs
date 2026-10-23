//! A group's members (FD-036): `AddUnit` (`FUN_1410083c0`) with its sub-formation statistics
//! (`FUN_141008d90`, `FUN_1410116c0`) and a unit's radius in a formation (`FUN_141025920`).

use crate::GenieError;
use crate::group::{Member, SpecificMove, UnitGroup, UnitTypeStatistic};
use crate::turn::Turns;

/// A unit's radius in a formation where its master has no full footprint (`FUN_141025920`,
/// `DAT_142e7f384`).
const DEFAULT_RADIUS: f32 = crate::image::group::DEFAULT_RADIUS;
/// A radius's share of the spacing (`DAT_142e8c150`): two radii side by side.
const SPACING_FACTOR: f32 = crate::image::group::SPACING_FACTOR;
/// A group holds fewer than this many members (`FUN_1410083c0`'s `0x46`: "No room in this
/// group").
const MAX_MEMBERS: usize = 70;
/// The members' arrays of a group without any first grow to this many (`FUN_1410083c0`'s
/// `FUN_14102d710` with 10), then, as every full array, to twice their capacity.
const FIRST_CAPACITY: i32 = 10;
/// The unit AI's actions and orders `AddUnit` clears (`FUN_1410083c0`): actions 0x259, 0x2bf and
/// 0x2c0, orders 0x2bd, 0x323 and 0x324.
const CLEARED_ACTIONS: [i32; 3] = [0x259, 0x2bf, 0x2c0];
const CLEARED_ORDERS: [i32; 3] = [0x2bd, 0x323, 0x324];

/// What a group reads of a unit's master for its formation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FormationUnit {
    pub(crate) master: i16,
    pub(crate) kind: u8,
    pub(crate) hero_flag: u8,
    pub(crate) formation_category: u8,
    /// `FUN_141025920`: the unit's radius in a formation.
    pub(crate) radius: f32,
}

impl FormationUnit {
    /// The unit's radius in a formation (`FUN_141025920`): a master above type 69 with a footprint
    /// and a height (`FUN_141194060` with 1) has its spacing modifier (`+0x8b8`) times its radius
    /// on x (`+0x5d8`), any other [`DEFAULT_RADIUS`].
    pub(crate) fn of(master: &crate::masters::Master) -> Self {
        let full = 0.0 < master.height && 0.0 < master.radius[0] && 0.0 < master.radius[1];
        let radius = if 69 < master.kind && full {
            master.spacing_modifier * master.radius[0]
        } else {
            DEFAULT_RADIUS
        };
        Self {
            master: master.id,
            kind: master.kind,
            hero_flag: master.hero_flag,
            formation_category: master.formation_category,
            radius,
        }
    }
}

impl UnitGroup {
    /// The members' arrays' capacity (`+0xe8`): the members and the spare places past them.
    #[must_use]
    pub const fn member_capacity(&self) -> usize {
        self.members.len() + self.spare_seconds.len()
    }

    /// `FUN_14102d710`: the members' arrays (ids `+0xf0`, own moves `+0xf8`,
    /// `mSecondsSinceAttacking` `+0x480`) grow to `n`. False, with nothing changed, for `n` of 0
    /// or less or not above the capacity (`+0xe8`). Else the members keep their ids, moves and
    /// seconds, and every place past them is fresh: no unit (-1), no move (active 0, path and
    /// reason -1, the rest 0) and its seconds at `f32::MAX` (`0x7f7fffff`), an earlier spare
    /// value gone; the capacity is `n`. Allocation failure is Rust's abort.
    pub fn resize_members(&mut self, n: i32) -> bool {
        if n <= 0 || n as usize <= self.member_capacity() {
            return false;
        }
        self.spare_seconds = vec![f32::MAX; n as usize - self.members.len()];
        true
    }

    /// The members' arrays' room for one more (`FUN_1410083c0`): a full array (the members at
    /// the capacity) grows ([`UnitGroup::resize_members`]): one of none to [`FIRST_CAPACITY`]
    /// and then, as any full one, to twice its capacity (20 the first time). The next member
    /// takes the first spare place's seconds, which a removed member may have left
    /// (`FUN_14102c160` shifts the ids and moves, not this array).
    fn next_seconds(&mut self) -> f32 {
        if self.spare_seconds.is_empty() {
            if self.member_capacity() == 0 {
                let _ = self.resize_members(FIRST_CAPACITY);
            }
            let _ = self.resize_members(self.member_capacity() as i32 * 2);
        }
        self.spare_seconds.remove(0)
    }

    /// `FUN_141008d90` on the statistic of sub-formation `sub` (a new one at the list's end when
    /// none has it, `FUN_1410083c0`): the unit joins its units, the statistic's spacing is the
    /// widest member's two radii, and its unit types are counted again ([`count_unit_types`]).
    fn join_statistic(
        &mut self,
        sub: i32,
        id: i32,
        unit: FormationUnit,
        masters_of: &dyn Fn(i32) -> Option<i16>,
        scratch: &mut Vec<u32>,
    ) {
        let at = self
            .unit_types
            .iter()
            .position(|s| s.formation_category == sub)
            .unwrap_or_else(|| {
                self.unit_types.push(UnitTypeStatistic {
                    formation_category: sub,
                    ..UnitTypeStatistic::default()
                });
                self.unit_types.len() - 1
            });
        let stat = &mut self.unit_types[at];
        let spacing = unit.radius * SPACING_FACTOR;
        if stat.desired_spacing < spacing {
            stat.desired_spacing = spacing;
        }
        stat.units.push(id);
        count_unit_types(stat, masters_of, scratch);
    }
}

impl UnitGroup {
    /// [`Turns::leave_statistic`]'s part on the group, for unit `id` of sub-formation `category`
    /// (None for a unit the world does not hold): it leaves the first statistic of that category
    /// (none for -1), or, for a unit the world does not hold, the first statistic that holds it
    /// (`FUN_14102c320`: its units close up and its unit types are counted again,
    /// [`count_unit_types`]). False when none held it.
    fn drop_from_statistic(
        &mut self,
        id: i32,
        category: Option<i32>,
        masters_of: &dyn Fn(i32) -> Option<i16>,
        scratch: &mut Vec<u32>,
    ) -> bool {
        let leave = |stat: &mut UnitTypeStatistic, scratch: &mut Vec<u32>| {
            let Some(i) = stat.units.iter().position(|&u| u == id) else {
                return false;
            };
            let _ = stat.units.remove(i);
            count_unit_types(stat, masters_of, scratch);
            true
        };
        match category {
            Some(-1) => false,
            Some(c) => self
                .unit_types
                .iter_mut()
                .find(|s| s.formation_category == c)
                .is_some_and(|stat| leave(stat, scratch)),
            None => self.unit_types.iter_mut().any(|stat| leave(stat, scratch)),
        }
    }
}

impl Turns {
    /// `removeUnit` (`FUN_14102c160`) of unit `id` from group `group`, `target` (`param_3`) the
    /// object its sub-formation is reckoned against (`FUN_141011300`'s rams rule, which no code
    /// enables) and `keep_moving` (`param_4`): false for a unit not a member. A member the world
    /// holds whose group link names the group stands (slot 0x560, [`crate::movement::Mover::
    /// stand`], unless `keep_moving`) and its link clears (`+0x6d0` -1). The members' ids and
    /// own moves close up; `mSecondsSinceAttacking` keeps its places (the last member's value
    /// goes to the spare ones). It leaves its sub-formation's statistic (`FUN_14102c480`); a
    /// lead that leaves gives way to the member nearest the target (`FUN_141016870`,
    /// [`Turns::nearest_to_target`]); the flags take `0x10002`.
    ///
    /// # Errors
    ///
    /// A member of a master the slice does not have, or a stand the slice does not model.
    ///
    /// # Panics
    ///
    /// When the world does not hold `group`: the game's routine gets the group itself, from a
    /// caller that holds it.
    pub fn remove_unit(
        &mut self,
        group: i32,
        id: i32,
        target: i32,
        keep_moving: bool,
    ) -> Result<bool, GenieError> {
        let _ = target;
        let Some(g) = self.groups.get(group) else {
            return Ok(false);
        };
        let Some(index) = g.members.iter().position(|m| m.id == id) else {
            return Ok(false);
        };
        let group_id = g.id;
        if let Some(place) = self.place(id) {
            let u = match place {
                crate::order::Place::Live(p, i) => &mut self.players[p].objects[i],
                crate::order::Place::Sleeping(p, i) => &mut self.players[p].sleeping[i],
            };
            if u.group.id == group_id {
                if !keep_moving && let Some(m) = u.mover.as_mut() {
                    m.stand()?;
                }
                u.group.id = -1;
                // the walks that meet it read its group
                let gait = u.gait();
                self.spatial.set_gait(id, gait);
            }
        }
        let g = self.groups.get_mut(group).expect("the group found above");
        let seconds: Vec<f32> = g
            .members
            .iter()
            .map(|m| m.seconds_since_attacking)
            .collect();
        let _ = g.members.remove(index);
        for (m, &s) in g.members.iter_mut().zip(&seconds) {
            m.seconds_since_attacking = s;
        }
        g.spare_seconds.insert(0, seconds[seconds.len() - 1]);
        let _ = self.leave_statistic(group, id)?;
        let g = self.groups.get(group).expect("the group found above");
        if g.motion.lead == id {
            let lead = self.nearest_to_target(group);
            self.groups
                .get_mut(group)
                .expect("the group found above")
                .motion
                .lead = lead;
        }
        self.groups
            .get_mut(group)
            .expect("the group found above")
            .flags |= 0x1_0002;
        Ok(true)
    }
}

impl Turns {
    /// `FUN_14102c480` of unit `id` and group `group`'s statistics: for an object the world's
    /// table holds ([`Turns::world_object`]) its sub-formation ([`Turns::unit_category`]; -1:
    /// false) and the first statistic of it (none: false), which the unit leaves; for one it
    /// does not hold, the first statistic that held it ([`UnitGroup`]'s
    /// `drop_from_statistic`, `FUN_14102c320`). The routine's target (`param_4`) enters only the
    /// rams' rule of `FUN_141011300`, which never holds: the port takes none.
    ///
    /// # Errors
    ///
    /// A held object of a master the slice does not have.
    ///
    /// # Panics
    ///
    /// When the world does not hold `group`: its caller (`removeUnit`) holds it.
    pub(crate) fn leave_statistic(&mut self, group: i32, id: i32) -> Result<bool, GenieError> {
        let category = match self.world_object(id) {
            Some(object) => Some(self.unit_category(Some(object))?),
            None => None,
        };
        let masters: Vec<(i32, i16)> = self
            .players
            .iter()
            .flat_map(|q| q.objects.iter().chain(&q.sleeping))
            .map(|u| (u.id, u.life.master_id))
            .collect();
        let masters_of = |id: i32| masters.iter().find(|(u, _)| *u == id).map(|&(_, m)| m);
        let mut scratch = std::mem::take(&mut self.groups.unit_types_scratch);
        let left = self
            .groups
            .get_mut(group)
            .expect("removeUnit holds the group")
            .drop_from_statistic(id, category, &masters_of, &mut scratch);
        self.groups.unit_types_scratch = scratch;
        Ok(left)
    }
}

/// `FUN_1410116c0`: a sub-formation's unit types in the order its units first have them, each with
/// its count, counted in the process's scratch arrays (`DAT_143f23b70`, `DAT_143f23c40`: the counts
/// are cleared first, the types not), whose 50 words each the statistic takes whole. A unit the
/// world does not hold is passed over; past 51 types the count stops.
pub(crate) fn count_unit_types(
    stat: &mut UnitTypeStatistic,
    masters_of: &dyn Fn(i32) -> Option<i16>,
    scratch: &mut Vec<u32>,
) {
    scratch.resize(50, 0);
    let mut counts = vec![0u32; 50];
    let mut n = 0usize;
    for &id in &stat.units {
        let Some(master) = masters_of(id) else {
            continue;
        };
        let master = i32::from(master) as u32;
        if let Some(k) = scratch[..n].iter().position(|&t| t == master) {
            counts[k] += 1;
            continue;
        }
        if n < 50 {
            scratch[n] = master;
            counts[n] = 1;
        }
        n += 1;
        if 50 < n {
            break;
        }
    }
    stat.unit_types.clone_from(scratch);
    stat.unit_type_counts = counts;
    stat.number_unit_types = n as i32;
}

impl Turns {
    /// `AddUnit` (`FUN_1410083c0` with `param_4` 0) of unit `id` to group `group`: false for an
    /// object the world's table does not hold ([`Turns::world_object`]), of no sub-formation
    /// ("Invalid formation category", [`Turns::unit_category`]) or when the group holds
    /// [`MAX_MEMBERS`] already; true when it is a member already. Else the members' arrays make
    /// room ([`UnitGroup::resize_members`]); a unit in another group the world holds leaves it
    /// ([`Turns::remove_unit`]); the unit's group link names the group (`+0x6d0`), its unit AI
    /// drops the actions and orders of [`CLEARED_ACTIONS`] and [`CLEARED_ORDERS`]
    /// (`FUN_140c64b50` and `FUN_140c64b60` with -1) and empties its order queue
    /// (`FUN_140c5e010`); it joins the members with no move of its own (active 0, path and
    /// reason -1), its sub-formation's statistic and the group's spacing (its two radii, the
    /// widest: `COMISS`/`JBE` at 0x141008ced); the group's flags take bit 1, and bit 16 too
    /// when its motion's bit 1 is clear. A sleeping unit joins as a live one.
    ///
    /// The routine's target (`param_3`), which it hands to `removeUnit` and to its second
    /// `FUN_141011300`, enters only the rams' rule, which never holds
    /// ([`Turns::unit_category`]): the port takes none.
    ///
    /// # Panics
    ///
    /// When the world does not hold `group`: the game's routine gets the group itself, from a
    /// caller that holds it.
    pub fn add_to_group(&mut self, group: i32, id: i32) -> Result<bool, GenieError> {
        let Some(object) = self.world_object(id) else {
            return Ok(false);
        };
        let sub = self.unit_category(Some(object))?;
        // a ghost (type 25) has no sub-formation: only a unit gets past this
        let crate::group::WorldObject::Unit(p, u) = object else {
            return Ok(false);
        };
        if sub == -1 {
            return Ok(false);
        }
        let unit = FormationUnit::of(self.master_of(p, u.life.master_id)?);
        let old = u.group.id;
        let g = self.groups.get(group).expect("the caller holds the group");
        if g.has(id) {
            return Ok(true);
        }
        if MAX_MEMBERS <= g.members.len() {
            return Ok(false);
        }
        if old != -1 && self.groups.get(old).is_some() {
            // the unit leaves its old group first (removeUnit with -1, moving stopped)
            let _ = self.remove_unit(old, id, -1, false)?;
        }
        let place = self.place(id).expect("the unit found above");
        let ai_rng = self.streams.get(crate::rng::Stream::Ai);
        // a sleeping unit joins as a live one
        let u = match place {
            crate::order::Place::Live(q, i) => &mut self.players[q].objects[i],
            crate::order::Place::Sleeping(q, i) => &mut self.players[q].sleeping[i],
        };
        u.group.id = group;
        if let Some(ai) = u.ai.as_mut() {
            if CLEARED_ACTIONS.contains(&ai.current_action)
                || CLEARED_ORDERS.contains(&ai.current_order)
            {
                ai.set_action(-1);
                ai.set_order(-1, ai_rng);
            }
            ai.orders.clear();
        }
        // the walks that meet it read its group
        let gait = u.gait();
        self.spatial.set_gait(id, gait);
        let masters: Vec<(i32, i16)> = self
            .players
            .iter()
            .flat_map(|q| q.objects.iter().chain(&q.sleeping))
            .map(|u| (u.id, u.life.master_id))
            .collect();
        let masters_of = |id: i32| masters.iter().find(|(u, _)| *u == id).map(|&(_, m)| m);
        let mut scratch = std::mem::take(&mut self.groups.unit_types_scratch);
        let g = self.groups.get_mut(group).expect("the group found above");
        let seconds = g.next_seconds();
        g.members.push(Member {
            id,
            specific_move: SpecificMove {
                path_id: -1,
                reason: -1,
                ..SpecificMove::default()
            },
            seconds_since_attacking: seconds,
        });
        g.join_statistic(sub, id, unit, &masters_of, &mut scratch);
        let spacing = unit.radius * SPACING_FACTOR;
        if g.desired_spacing < spacing {
            g.desired_spacing = spacing;
        }
        g.flags |= 2;
        if g.motion.flags & 2 == 0 {
            g.flags |= 0x1_0002;
        }
        self.groups.unit_types_scratch = scratch;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use crate::group::{Member, UnitGroup, UnitGroups, UnitTypeStatistic};
    use crate::masters::{Master, Masters};
    use crate::turn::{Player, Turns, Unit};

    /// Group 1 of player 1's units 10, 11 (master 4, category 1) and 12 (master 6, category 3),
    /// their seconds since attacking 1, 2 and 3, 11 the lead and walking.
    fn three() -> Turns {
        let unit = |id: i32, master: i16, x: f32| {
            let mut u = Unit {
                id,
                position: [x, 0.0, 0.0],
                ..Unit::default()
            };
            u.life.master_id = master;
            u.group.id = 1;
            u
        };
        let mut walking = unit(11, 4, 2.0);
        let mut mover = crate::hunt::tests::walker();
        mover.paths = vec![
            mover
                .paths
                .first()
                .cloned()
                .unwrap_or(crate::movement::PathData::without_path()),
        ];
        mover.paths[0].state = 3;
        walking.mover = Some(mover);
        let player = Player {
            objects: vec![unit(10, 4, 1.0), walking, unit(12, 6, 3.0)],
            ..Player::default()
        };
        let mut t = Turns::new(0, 0, 0, vec![0, 0], vec![Player::default(), player]);
        let master = |id: i16, category: u8| Master {
            id,
            kind: 70,
            formation_category: category,
            ..Master::default()
        };
        let masters = Masters::new([master(4, 1), master(6, 3)]);
        t.masters = vec![masters.clone(), masters];
        let mut g = UnitGroup::new(1);
        g.members = [(10, 1.0), (11, 2.0), (12, 3.0)]
            .iter()
            .map(|&(id, s)| Member {
                id,
                seconds_since_attacking: s,
                ..Member::default()
            })
            .collect();
        g.spare_seconds = vec![f32::MAX];
        g.unit_types = vec![
            UnitTypeStatistic {
                formation_category: 1,
                units: vec![10, 11],
                ..UnitTypeStatistic::default()
            },
            UnitTypeStatistic {
                formation_category: 3,
                units: vec![12],
                ..UnitTypeStatistic::default()
            },
        ];
        g.motion.lead = 11;
        t.groups = UnitGroups {
            groups: vec![g],
            ..UnitGroups::default()
        };
        t
    }

    /// `FUN_14102d710`: nothing for 0, a negative size or one not above the capacity; a larger
    /// one keeps the members and makes every place past them fresh, an old spare value gone.
    /// `AddUnit` grows a group without places to 10, then 20.
    #[test]
    fn the_members_arrays_grow_with_fresh_places_past_the_members() {
        let mut g = UnitGroup::new(1);
        g.members = vec![Member {
            id: 5,
            seconds_since_attacking: 2.0,
            ..Member::default()
        }];
        g.spare_seconds = vec![7.0, 8.0];
        assert!(!g.resize_members(0));
        assert!(!g.resize_members(-4));
        assert!(!g.resize_members(3));
        assert_eq!(g.spare_seconds, [7.0, 8.0]);
        assert!(g.resize_members(4));
        assert_eq!(
            (g.members[0].id, g.members[0].seconds_since_attacking),
            (5, 2.0)
        );
        assert_eq!(g.spare_seconds, [f32::MAX; 3]);
        assert_eq!(g.member_capacity(), 4);
        // the place taken for the next member leaves 19 spare of 20
        let mut empty = UnitGroup::new(2);
        assert_eq!(empty.next_seconds(), f32::MAX);
        assert_eq!(empty.spare_seconds.len(), 19);
    }

    /// `FUN_1410083c0` adds a sleeping unit as a live one: its link names the group, it joins
    /// the members and its sub-formation's statistic.
    #[test]
    fn a_sleeping_unit_joins_a_group_as_a_live_one() {
        let mut t = three();
        let mut asleep = Unit {
            id: 13,
            ..Unit::default()
        };
        asleep.life.master_id = 4;
        t.players[1].sleeping.push(asleep);
        assert!(t.add_to_group(1, 13).unwrap());
        assert_eq!(t.players[1].sleeping[0].group.id, 1);
        let g = t.groups.get(1).unwrap();
        assert_eq!(g.members.last().map(|m| m.id), Some(13));
        assert_eq!(g.unit_types[0].units, [10, 11, 13]);
        // an object the world's table does not hold, or of no sub-formation, joins nothing
        assert!(!t.add_to_group(1, 99).unwrap());
    }

    /// `FUN_14102c480`: a held unit of no sub-formation (type below 70) or of one no statistic
    /// has leaves nothing (false); a held unit leaves its sub-formation's first statistic.
    #[test]
    fn a_unit_leaves_only_its_own_sub_formation_s_statistic() {
        let mut t = three();
        let unit = |id: i32, master: i16| {
            let mut u = Unit {
                id,
                ..Unit::default()
            };
            u.life.master_id = master;
            u
        };
        t.players[1].objects.push(unit(14, 7));
        t.players[1].objects.push(unit(15, 8));
        for masters in &mut t.masters {
            let extra = [
                Master {
                    id: 7,
                    kind: 60,
                    formation_category: 1,
                    ..Master::default()
                },
                Master {
                    id: 8,
                    kind: 70,
                    formation_category: 2,
                    ..Master::default()
                },
            ];
            *masters = Masters::new(masters.iter().cloned().chain(extra));
        }
        t.groups.groups[0].unit_types[0].units.push(14);
        assert!(!t.leave_statistic(1, 14).unwrap());
        assert!(!t.leave_statistic(1, 15).unwrap());
        assert_eq!(t.groups.get(1).unwrap().unit_types[0].units, [10, 11, 14]);
        assert!(t.leave_statistic(1, 12).unwrap());
        assert_eq!(
            t.groups.get(1).unwrap().unit_types[1].units,
            Vec::<i32>::new()
        );
    }

    /// `FUN_14102c160`: 11 leaves: the ids close up, the seconds keep their places (11's 2 now
    /// 12's, 12's 3 the first spare); its statistic counts 10 alone; it stands and its link
    /// clears; the lead becomes the member nearest the target (the origin: 10); flags `0x10002`.
    #[test]
    fn a_removed_member_leaves_its_slot_statistic_and_lead() {
        let mut t = three();
        assert!(t.remove_unit(1, 11, -1, false).unwrap());
        let g = t.groups.get(1).unwrap();
        let members: Vec<(i32, f32)> = g
            .members
            .iter()
            .map(|m| (m.id, m.seconds_since_attacking))
            .collect();
        assert_eq!(members, [(10, 1.0), (12, 2.0)]);
        assert_eq!(g.spare_seconds, [3.0, f32::MAX]);
        assert_eq!(g.unit_types[0].units, [10]);
        assert_eq!(
            (
                g.unit_types[0].number_unit_types,
                g.unit_types[0].unit_type_counts[0]
            ),
            (1, 1)
        );
        assert_eq!(g.unit_types[1].units, [12]);
        assert_eq!((g.motion.lead, g.flags & 0x1_0002), (10, 0x1_0002));
        let u = &t.players[1].objects[1];
        assert_eq!(u.group.id, -1);
        let m = u.mover.as_ref().unwrap();
        assert_eq!(m.paths[0].state, 0);
        // not a member, or a group the world does not hold
        assert!(!t.remove_unit(1, 11, -1, false).unwrap());
        assert!(!t.remove_unit(2, 10, -1, false).unwrap());
    }

    /// `FUN_14100bcb0`'s closing `CalcFormationType` is gated on the motion flags' bit 1
    /// (`+0xb8`, `SHR EAX, 1; TEST AL, 1` at 0x14100c657), not the group flags'.
    #[test]
    fn the_statistics_call_for_the_formation_type_by_the_motion_flags() {
        let mut t = three();
        for u in &mut t.players[1].objects {
            u.ai = Some(crate::unit_ai::UnitAi {
                ai_type: crate::unit_ai::SOLDIER_AI,
                ..crate::unit_ai::UnitAi::default()
            });
        }
        // the members' searches find nothing: an empty registry, no relations
        let mut v = crate::visibility::Visibility::new(20, 20, vec![], vec![], vec![], 0, vec![]);
        v.registry = Some(crate::registry::Registry::new(2, 65));
        t.spatial.sight = Some(v);
        t.spatial.relations = vec![vec![0, 0], vec![0, 0]];
        t.spatial.unit_diplomacy = vec![vec![1, 2], vec![0, 1]];
        let g = t.groups.get_mut(1).unwrap();
        g.flags |= 2;
        g.motion.flags &= !2;
        assert!(!t.statistics_only(1, false).unwrap());
        let g = t.groups.get_mut(1).unwrap();
        g.flags &= !2;
        g.motion.flags |= 2;
        assert!(t.statistics_only(1, false).unwrap());
    }

    /// `FUN_14100d140` writes the formation type of every member `FUN_1415417a0` finds with a
    /// unit AI, a sleeping one too.
    #[test]
    fn the_stances_reach_a_sleeping_member() {
        let mut t = three();
        for u in &mut t.players[1].objects {
            u.ai = Some(crate::unit_ai::UnitAi::default());
        }
        let asleep = t.players[1].objects.remove(2);
        t.players[1].sleeping.push(asleep);
        t.stances_state(1, 5);
        assert_eq!(
            t.players[1].sleeping[0].ai.as_ref().unwrap().formation_type,
            5
        );
        assert_eq!(
            t.players[1].objects[0].ai.as_ref().unwrap().formation_type,
            5
        );
    }

    /// `LockFormation` (`FUN_141024e70`) re-reads the spacing (`+0x15c`) for every member
    /// (`MOVSS`/`UCOMISS` at 0x14102500a): the first held member with a radius sets it, the later
    /// ones are skipped.
    #[test]
    fn lock_formation_takes_the_first_member_s_spacing() {
        let mut t = three();
        for masters in &mut t.masters {
            for (id, r) in [(4, 0.2f32), (6, 0.5)] {
                let m = masters.get_mut(id).unwrap();
                m.radius = [r, r];
                m.height = 1.0;
                m.spacing_modifier = 1.0;
            }
        }
        t.groups.get_mut(1).unwrap().desired_spacing = 0.0;
        let _ = t.lock_formation(1, crate::group::formation::LINE);
        let spacing = t.groups.get(1).unwrap().desired_spacing;
        assert_eq!(spacing, 0.2 * crate::image::group::SPACING_FACTOR);
    }

    /// Kept moving, the unit's walk stays; a unit the world does not hold leaves the first
    /// statistic that holds it.
    #[test]
    fn a_unit_kept_moving_walks_on_and_a_gone_one_leaves_its_statistic() {
        let mut t = three();
        assert!(t.remove_unit(1, 11, -1, true).unwrap());
        assert_eq!(
            t.players[1].objects[1].mover.as_ref().unwrap().paths[0].state,
            3
        );
        let mut t = three();
        let _ = t.players[1].objects.remove(2);
        assert!(t.remove_unit(1, 12, -1, false).unwrap());
        let g = t.groups.get(1).unwrap();
        assert_eq!(
            (g.unit_types[0].units.len(), g.unit_types[1].units.len()),
            (2, 0)
        );
    }
}
