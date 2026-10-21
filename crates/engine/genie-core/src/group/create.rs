//! A group's members (FD-036): `AddUnit` (`FUN_1410083c0`) with its sub-formation statistics
//! (`FUN_141008d90`, `FUN_1410116c0`) and a unit's radius in a formation (`FUN_141025920`).

use super::{Member, SpecificMove, UnitGroup, UnitTypeStatistic, sub_formation};
use crate::turn::Turns;
use crate::unit_ai::Unsupported;

/// A unit's radius in a formation where its master has no full footprint (`FUN_141025920`,
/// `DAT_142e7f384`).
const DEFAULT_RADIUS: f32 = crate::image::group::DEFAULT_RADIUS;
/// A radius's share of the spacing (`DAT_142e8c150`): two radii side by side.
const SPACING_FACTOR: f32 = crate::image::group::SPACING_FACTOR;
/// A group holds fewer than this many members (`FUN_1410083c0`'s `0x46`: "No room in this
/// group").
const MAX_MEMBERS: usize = 70;
/// The members' arrays grow from nothing to this many (`FUN_14102d710` with 10, then twice
/// that), then double.
const FIRST_CAPACITY: usize = 20;
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
    /// The members' arrays' room for one more (`FUN_1410083c0` with `FUN_14102d710`): when every
    /// slot is taken they grow (from none to [`FIRST_CAPACITY`], else to twice), the new slots'
    /// `mSecondsSinceAttacking` at `f32::MAX` (`0x7f7fffff`). The next member takes the first
    /// spare slot's value, which a removed member may have left (`FUN_14102c160` shifts the ids
    /// and moves, not this array).
    fn next_seconds(&mut self) -> f32 {
        if self.spare_seconds.is_empty() {
            let capacity = self.members.len();
            let grown = if capacity == 0 {
                FIRST_CAPACITY
            } else {
                capacity * 2
            };
            self.spare_seconds = vec![f32::MAX; grown - capacity];
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
    /// `AddUnit` (`FUN_1410083c0` with `param_4` 0) of unit `id` to group `group`: false for a unit
    /// the world does not hold, of no sub-formation ("Invalid formation category",
    /// [`sub_formation`]) or when the group holds [`MAX_MEMBERS`] less one already; true when it
    /// is a member already. Else the unit's group link names the group (`+0x6d0`), its unit AI
    /// drops the actions and orders of [`CLEARED_ACTIONS`] and [`CLEARED_ORDERS`] (`FUN_140c64b50`
    /// and `FUN_140c64b60` with -1) and empties its order queue (`FUN_140c5e010`); it joins the
    /// members with no move of its own (active 0, path and reason -1), its sub-formation's
    /// statistic and the group's spacing (its two radii, the widest); the group's flags take bit
    /// 1, and bit 16 too when its motion's bit 1 is clear. A unit in another group the world holds
    /// leaves it first (`FUN_14102c160`), not modelled.
    ///
    /// # Panics
    ///
    /// Never: the group is the one found first.
    pub fn add_to_group(&mut self, group: i32, id: i32) -> Result<bool, Unsupported> {
        let Some((p, u)) = self.object(id) else {
            return Ok(false);
        };
        let unit = FormationUnit::of(self.master_of(p, u.life.master_id)?);
        let old = u.group.id;
        let Some(sub) = sub_formation(unit.kind, unit.hero_flag, unit.formation_category) else {
            return Ok(false);
        };
        let g = self
            .groups
            .get(group)
            .ok_or(Unsupported("AddUnit to a group the world does not hold"))?;
        if g.has(id) {
            return Ok(true);
        }
        if MAX_MEMBERS <= g.members.len() {
            return Ok(false);
        }
        if old != -1 && self.groups.get(old).is_some() {
            return Err(Unsupported(
                "AddUnit of a unit in another group (FUN_14102c160)",
            ));
        }
        let Some(crate::order::Place::Live(q, i)) = self.place(id) else {
            return Err(Unsupported("AddUnit of a sleeping unit"));
        };
        let ai_rng = self.streams.get(crate::rng::Stream::Ai);
        let u = &mut self.players[q].objects[i];
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
