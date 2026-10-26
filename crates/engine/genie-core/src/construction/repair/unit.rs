//! The repair part of the update and the unit's attackers and seen target.

use crate::GenieError;
use crate::map::spatial::Spatial;
use crate::math::rng::Stream;
use crate::movement::Me;
use crate::objects::masters::Master;
use crate::objects::release::release_own;
use crate::objects::unit::{PaidAttributes, Unit};
use crate::unit_ai::AiMove;
use crate::unit_ai::world_view::RepairWrite;
use crate::world::context::Context;

impl Unit {
    /// The Repair's update ([`crate::construction::repair::RepairAction::update`]) with what it reads of its
    /// target (in the unit's player's list or another player's), the repairers of the object
    /// `FUN_140ad9050` gives (the target, an annex's main building), the unit's work rate and
    /// quick path, and the target's repair over the objects it reaches (the target, an annex's
    /// main building, a main building's annexes: [`crate::construction::repair::repair_building`],
    /// [`crate::construction::repair::repair_unit`]) paid from the player's resources. Its changes go to the
    /// unit's player's objects at once and to another player's after the update
    /// ([`Context::repair_writes`]); the payments join the attribute adds.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn repair_part(
        &mut self,
        c: &mut Context<'_>,
        others: &mut [Self],
        me: Me,
    ) -> Result<(), GenieError> {
        use crate::construction::repair::{Repaired, Repairee};
        let Some(target) = self.repair.map(|r| r.target) else {
            return Ok(());
        };
        let (player, foreign, every) = (c.player, c.foreign, c.every_masters);
        let held = |id: i32| -> Option<(usize, &Self)> {
            others
                .iter()
                .find(|u| u.id == id)
                .map(|u| (player, u))
                .or_else(|| foreign.find(id))
        };
        let master_of = |q: usize, u: &Self| -> Option<Master> {
            u.own_master
                .as_deref()
                .or_else(|| every.get(q)?.get(u.life.master_id))
                .cloned()
        };
        let own = master_of(player, self).ok_or(GenieError::missing_data("a repairer's master"))?;
        // the target as the update reads it, and the object FUN_140ad9050 counts repairers in
        let mut repairee = None;
        let mut counted_in = None;
        let mut set: Vec<Repaired> = Vec::new();
        if target != -1
            && let Some((q, t)) = held(target)
        {
            let tm =
                master_of(q, t).ok_or(GenieError::missing_data("a repaired object's master"))?;
            repairee = Some(Repairee {
                state: t.life.state,
                hp: t.life.hp,
                owner: q,
                allied: crate::unit_ai::task_finder::allied(c.relations, player, q, true),
                position: t.position,
                radius: tm.radius,
                class: tm.class,
                moves: 29 < tm.kind && tm.moving.as_ref().is_some_and(|m| 0.0 < m.speed),
            });
            let building = tm.kind == 80;
            let counting = if building {
                t.annex_of.unwrap_or(target)
            } else {
                target
            };
            if let Some((_, k)) = held(counting) {
                counted_in = Some((counting, k.repairers()));
            }
            // the objects the repair reaches
            let main = if building { t.annex_of } else { None };
            let head = main.unwrap_or(target);
            let mut ids = vec![target];
            if building {
                if head != target {
                    ids.push(head);
                }
                let mut annexes = c.spatial.annexes_of(head);
                annexes.sort_unstable();
                ids.extend(annexes.iter().copied().filter(|&a| a != target));
            }
            for id in ids {
                let Some((oq, o)) = held(id) else {
                    continue;
                };
                let om = master_of(oq, o)
                    .ok_or(GenieError::missing_data("a repaired object's master"))?;
                let attributes: &[f32] = if oq == player {
                    c.attributes
                } else {
                    foreign.player(oq).map_or(&[][..], |p| &p.attributes[..])
                };
                let rate = |k: usize| attributes.get(k).copied().unwrap_or(0.0);
                let annexes = if om.kind == 80 && o.annex_of.is_none() {
                    let mut a = c.spatial.annexes_of(id);
                    a.sort_unstable();
                    a
                } else {
                    Vec::new()
                };
                set.push(Repaired {
                    id,
                    kind: om.kind,
                    class: om.class,
                    master: om.id,
                    master_hp: om.hit_points,
                    hp: o.life.hp,
                    hp_powerups: o.hp_powerups,
                    main: if om.kind == 80 { o.annex_of } else { None },
                    annexes,
                    cost_rates: [
                        rate(crate::construction::repair::UNIT_REPAIR_COST),
                        rate(crate::construction::repair::BUILDING_REPAIR_COST),
                    ],
                });
            }
        }
        let original = set.clone();
        let masters = c
            .masters
            .ok_or(GenieError::missing_data("a repairer's player's masters"))?;
        let mut paid = PaidAttributes::new(c.resources, c.attributes);
        let target_kind = set.first().map_or(0, |o| o.kind);
        let mut repair = |amount: f32| -> Result<u8, GenieError> {
            if set.is_empty() {
                return Err(GenieError::unreachable(
                    "a Repair working on an object the world does not hold",
                ));
            }
            let mut payer = crate::construction::repair::Payer {
                masters,
                attributes: &mut paid,
            };
            match target_kind {
                80 => crate::construction::repair::repair_building(
                    &mut set, 0, amount, true, &mut payer,
                ),
                70 => crate::construction::repair::repair_unit(&mut set, 0, amount, &mut payer),
                // the task finder repairs buildings and combat units of its classes alone
                _ => Err(GenieError::unreachable(
                    "a repaired object of a type below 70 (slot 0x110)",
                )),
            }
        };
        let restriction = usize::try_from(own.terrain_restriction).ok();
        let (my_radius, my_id) = (own.radius, self.id);
        let mut can_path = |spatial: &Spatial| -> Result<bool, GenieError> {
            let terrain = spatial
                .terrain
                .as_ref()
                .ok_or(crate::pathing::hull_path::NO_TERRAIN)?;
            let r = restriction.ok_or(crate::pathing::hull_path::NO_RESTRICTION)?;
            crate::pathing::hull_path::quick_reach(
                spatial,
                my_id,
                target,
                terrain.restriction_mask(r),
                my_radius[0],
                crate::construction::repair::REACH,
            )
        };
        let view = crate::construction::repair::RepairView {
            target: repairee,
            radius: own.radius,
            scenario: c.scenario,
            rate: self.work_rate_powerup + own.work_rate,
            repairers: counted_in.map(|(_, n)| n),
            now: c.world_time,
            waits: own.class == 4 && !c.computer,
            stance: self.ai.as_ref().map_or(0, |ai| ai.stance),
            can_path: &mut can_path,
            repair: &mut repair,
        };
        let repair_action = self.repair.as_mut().expect("checked above");
        let step = repair_action.update(c.walk(self.mover.as_mut(), me), view)?;
        // the repair's changes: the unit's player's objects at once, another player's after
        for (now, was) in set.iter().zip(&original) {
            if now == was {
                continue;
            }
            if let Some(u) = others.iter_mut().find(|u| u.id == now.id) {
                u.life.hp = now.hp;
                u.hp_powerups = now.hp_powerups;
            } else {
                c.repair_writes.push(RepairWrite {
                    id: now.id,
                    hp: Some((now.hp, now.hp_powerups)),
                    repairers: 0,
                    restored: false,
                });
            }
        }
        c.attribute_adds.extend(paid.adds);
        // FUN_1412066a0: a target of a type above 69 has its armor back
        if step.restored && 0x45 < target_kind {
            if let Some(u) = others.iter_mut().find(|u| u.id == target) {
                u.armor_removed = [0; 2];
            } else {
                c.repair_writes.push(RepairWrite {
                    id: target,
                    hp: None,
                    repairers: 0,
                    restored: true,
                });
            }
        }
        if step.repairers != 0
            && let Some((id, _)) = counted_in
        {
            if let Some(u) = others.iter_mut().find(|u| u.id == id) {
                let n = u.repairers_mut();
                *n = n.wrapping_add_signed(step.repairers);
            } else {
                c.repair_writes.push(RepairWrite {
                    id,
                    hp: None,
                    repairers: step.repairers,
                    restored: false,
                });
            }
        }
        let by = self.releaser(c);
        if step.released {
            if others.iter().any(|u| u.id == target) {
                release_own(others, c.masters, target, by)?;
            } else {
                c.releases.push((target, by));
            }
        }
        for kind in &step.notifies {
            self.post_own(*kind, [crate::construction::repair::REPAIR_ACTION, 0, 0])?;
        }
        if step.stands {
            self.show_standing(c.masters, c.graphics, c.streams.get(Stream::Sprite))?;
        }
        if step.done {
            // the action list removes it; its destructor (FUN_140ad8fd0) finds it out of the
            // repairers (set_state left them) and lets the target it still holds go
            let holds = self.repair.map_or(-1, |r| r.target);
            self.repair = None;
            if holds != -1 {
                if others.iter().any(|u| u.id == holds) {
                    release_own(others, c.masters, holds, by)?;
                } else {
                    c.releases.push((holds, by));
                }
            }
        }
        Ok(())
    }

    /// `+0x9e8` (`num_builders`): a building's in its construction, any other's its own.
    pub(crate) const fn repairers(&self) -> u8 {
        match &self.construction {
            Some(k) => k.builders,
            None => self.builders,
        }
    }

    /// [`Unit::repairers`] to change.
    pub(crate) const fn repairers_mut(&mut self) -> &mut u8 {
        match &mut self.construction {
            Some(k) => &mut k.builders,
            None => &mut self.builders,
        }
    }

    /// What the attack tests read of the unit as an attacker ([`crate::unit_ai::Attacker`]) with
    /// a unit AI of type `ai_type`, its master `master`, its owner and the owner's type, the game
    /// mode (a scenario's) and its `+0x924`.
    pub(crate) fn attacker_of(
        &self,
        ai_type: u32,
        master: Option<&Master>,
        (player, computer): (usize, bool),
        scenario: bool,
        capture: u8,
    ) -> crate::unit_ai::Attacker {
        crate::unit_ai::Attacker {
            ai_type,
            player,
            computer,
            state: self.life.state,
            held: self.life.attribute_type,
            top_action: self.top_action_kind().map(i32::from),
            master: self.life.master_id,
            kind: master.map_or(0, |m| m.kind),
            class: master.map_or(-1, |m| m.class),
            attack_priority: master.map_or(0, |m| m.attack_priority),
            missile: master.map_or(-1, |m| m.missile),
            range: master.map_or(0.0, |m| m.weapon_range),
            min_range: master.map_or(0.0, |m| m.min_range),
            charge_type: master.map_or(0, |m| m.charge_type),
            area_effect: master.map_or(0.0, |m| m.area_effect),
            blast_level: master.map_or(0, |m| m.blast_level),
            los: master.map_or(0.0, |m| m.line_of_sight),
            position: self.position,
            radius: master.map_or([0.0; 2], |m| m.radius),
            speed: self.mover.as_ref().map_or(0.0, |m| m.stats.speed),
            scenario,
            firepower: self.firepower,
            capture,
        }
    }

    /// Slot 400 (`FUN_14106d090`) as the unit AI reads it: its top action's target as the world
    /// holds it (its owner's other objects as they stand, the other players' lists), with its
    /// type, place and its master's radii; None without one.
    pub(crate) fn top_target_seen(
        &self,
        c: &Context<'_>,
        others: &[Self],
    ) -> Option<crate::unit_ai::TopTarget> {
        let id = self.top_target().filter(|&t| t != -1)?;
        let (owner, u) = others
            .iter()
            .find(|u| u.id == id)
            .map(|u| (c.player, u))
            .or_else(|| c.foreign.find(id))?;
        let m = c.every_masters.get(owner)?.get(u.life.master_id)?;
        Some(crate::unit_ai::TopTarget {
            id,
            kind: m.kind,
            position: u.position,
            radius: m.radius,
        })
    }

    /// `restorePatrolInformation`'s unit part ([`crate::unit_ai::Effect::RestorePatrol`]): its own
    /// waypoints go (slot 0x450) and become the saved patrol's points (slot 0x448 each, as x, y
    /// and height), and it moves to the saved waypoint (slot 0x340: range 0, forced, running; a
    /// waypoint outside the path is the game's static zero point): that move, the caller's
    /// ([`Unit::ai_move`]). None without a saved patrol.
    pub(crate) fn restore_patrol(&mut self) -> Option<AiMove> {
        let ai = self.ai.as_ref()?;
        let saved = ai.patrol_path.as_ref()?;
        let points = saved.path.points.clone();
        let w = usize::try_from(ai.patrol_waypoint)
            .ok()
            .and_then(|i| points.get(i))
            .copied()
            .unwrap_or([0.0; 3]);
        if let Some(m) = self.mover.as_mut() {
            m.clear_user_waypoints();
            for p in &points {
                m.add_user_waypoint(p[0], p[2], p[1]);
            }
        }
        Some(AiMove {
            point: [w[0], w[2], w[1]],
            range: 0.0,
            force: true,
            walk: false,
        })
    }
}
