//! The repair part of the update and the unit's attackers and seen target.

use crate::GenieError;
use crate::construction::repair::payment::PaidAttributes;
use crate::map::spatial::Spatial;
use crate::math::rng::Stream;
use crate::movement::Me;
use crate::movement::move_to::{MoveDefaults, Sprites};
use crate::objects::masters::{Master, Masters};
use crate::objects::sprite::sprite_id;
use crate::objects::teardown::HeadCode;
use crate::objects::unit::Unit;
use crate::unit_ai::ActionKind;
use crate::unit_ai::AiMove;
use crate::world::World;
use crate::world::others::OthersRef;
use crate::world::scenario_mode;
use crate::world::step::Step;

impl Unit {
    /// The Repair's update ([`crate::construction::repair::RepairAction::update`]) with what it
    /// reads of its target ([`repair_reads`]), the unit's work rate and quick path, and the
    /// target's repair over the objects it reaches ([`crate::construction::repair::repair_building`],
    /// [`crate::construction::repair::repair_unit`]) paid from the player's resources. Its changes
    /// go to the objects ([`repair_write_back`]), the payments join the attribute adds, then what
    /// the step leaves ([`Unit::after_repair_step`]).
    pub(crate) fn repair_part(&mut self, c: &mut Step<'_>, me: Me) -> Result<HeadCode, GenieError> {
        let Some(target) = self.actions.repair().map(|r| r.target) else {
            return Ok(HeadCode::Keeps);
        };
        let own = repair_master(&c.w.masters, c.player, self)
            .ok_or(GenieError::missing_data("a repairer's master"))?;
        let RepairReads {
            repairee,
            counted_in,
            mut set,
        } = repair_reads(c, OthersRef::of(&c.w.players, c.player), target)?;
        let original = set.clone();
        let (own_radius, own_class) = (own.radius, own.class);
        // the sprites its MoveTos' constructors take for none (FUN_140aa0d30, FUN_140aa0df0)
        let defaults = MoveDefaults::of(&own, self.abilities.look.as_ref());
        let standing = sprite_id(own.standing_sprite);
        let (restriction, rate) = (
            usize::try_from(own.terrain_restriction).ok(),
            self.work_rate_powerup + own.work_rate,
        );
        let (player, pace) = (c.player, c.pace());
        // what the owner holds now: the update may have paid or gained before the repair
        let resources = c.w.players[player].resources;
        let (scenario, now, computer) = (
            scenario_mode(c.w.settings.game_mode),
            c.w.world_time,
            c.w.players[c.player].computer,
        );
        let (my_id, stance) = (self.id, self.ai.as_ref().map_or(0, |ai| ai.stance));
        let repair_action = self.actions.repair_mut().expect("checked above");
        let (walk, beside) = c.w.walk_beside(pace, &mut self.body, me);
        let masters = beside
            .masters
            .get(player)
            .ok_or(GenieError::missing_data("a repairer's player's masters"))?;
        let mut paid = PaidAttributes::new(resources, &beside.players[player].attributes);
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
                own_radius[0],
                crate::construction::repair::REACH,
            )
        };
        let view = crate::construction::repair::RepairView {
            target: repairee,
            radius: own_radius,
            scenario,
            rate,
            repairers: counted_in.map(|(_, n)| n),
            now,
            waits: own_class == 4 && !computer,
            stance,
            defaults,
            standing,
            sprites: Sprites {
                sprite: &mut self.sprite,
                rng: beside.sprite_rng,
                graphics: beside.graphics,
            },
            can_path: &mut can_path,
            repair: &mut repair,
        };
        let step = repair_action.update(walk, view)?;
        c.w.players[player].add_attributes(paid.adds)?;
        repair_write_back(c.w, &set, &original);
        self.after_repair_step(c, (target, target_kind, counted_in), step)
    }

    /// What the Repair's step leaves besides the repair (`step`; `target` of type `target_kind`,
    /// `counted_in` the object whose repairers count the unit): a target of a type above 69 has
    /// its armor back (`FUN_1412066a0`), the repairers change, the target is let go, the unit AI
    /// hears the notifies and the unit stands; its code (the end's 1).
    fn after_repair_step(
        &mut self,
        c: &mut Step<'_>,
        (target, target_kind, counted_in): (i32, u8, Option<(i32, u8)>),
        step: crate::construction::repair::RepairStep,
    ) -> Result<HeadCode, GenieError> {
        if step.restored
            && 0x45 < target_kind
            && let Some((_, u)) = c.w.object_mut(target)
        {
            u.armor_removed = [0; 2];
        }
        if step.repairers != 0
            && let Some((id, _)) = counted_in
            && let Some((_, u)) = c.w.object_mut(id)
        {
            let n = u.repairers_mut();
            *n = n.wrapping_add_signed(step.repairers);
        }
        let by = self.releaser(c);
        if step.released {
            crate::world::lets::release(c, target, by)?;
        }
        for kind in &step.notifies {
            self.post_own(*kind, [ActionKind::Repair.value(), 0, 0])?;
        }
        if step.stands {
            self.show_standing(
                c.w.masters.get(c.player),
                &c.w.graphics,
                c.w.streams.get(Stream::Sprite),
            )?;
        }
        // its end's 1 deletes it; the destructor (FUN_140ad8fd0) finds it out of the repairers
        // (set_state left them) and lets the target it still holds go
        Ok(HeadCode::ended(step.done))
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
            position: self.position(),
            radius: master.map_or([0.0; 2], |m| m.radius),
            speed: self.body.mover().map_or(0.0, |m| m.stats.speed),
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
        c: &Step<'_>,
        others: OthersRef<'_>,
    ) -> Option<crate::unit_ai::TopTarget> {
        let id = self.top_target().filter(|&t| t != -1)?;
        let (owner, u) = others.find(id)?;
        let m = u.master(&c.w.masters, owner).ok()?;
        Some(crate::unit_ai::TopTarget {
            id,
            kind: m.kind,
            position: u.position(),
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
        if let Some(m) = self.body.mover_mut() {
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

/// What the Repair's update reads of its target ([`Unit::repair_part`]).
struct RepairReads {
    /// The target as the update reads it, None when the world holds no object of its id.
    repairee: Option<crate::construction::repair::Repairee>,
    /// The object `FUN_140ad9050` counts the repairers in (the target, an annex's main building)
    /// and their count.
    counted_in: Option<(i32, u8)>,
    /// The objects the repair reaches ([`reached_ids`]), as the repair reads and changes them.
    set: Vec<crate::construction::repair::Repaired>,
}

/// Object `u`'s master: its own (`+0x6c0`), else player `q`'s of its id.
fn repair_master(every: &[Masters], q: usize, u: &Unit) -> Option<Master> {
    u.master(every, q).ok().cloned()
}

/// What the Repair's update reads of its `target` ([`RepairReads`]): nothing for a target the
/// world does not hold.
fn repair_reads(
    c: &Step<'_>,
    others: OthersRef<'_>,
    target: i32,
) -> Result<RepairReads, GenieError> {
    use crate::construction::repair::Repairee;
    let mut reads = RepairReads {
        repairee: None,
        counted_in: None,
        set: Vec::new(),
    };
    let held = |id: i32| others.find(id);
    let Some((q, t)) = held(target).filter(|_| target != -1) else {
        return Ok(reads);
    };
    let tm = repair_master(&c.w.masters, q, t)
        .ok_or(GenieError::missing_data("a repaired object's master"))?;
    reads.repairee = Some(Repairee {
        state: t.life.state,
        hp: t.life.hp,
        owner: q,
        allied: crate::unit_ai::task_finder::allied(
            &c.w.players[c.player].relations,
            c.player,
            q,
            true,
        ),
        position: t.position(),
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
        reads.counted_in = Some((counting, k.repairers()));
    }
    for id in reached_ids(&c.w.spatial, t, target, building) {
        let Some(o) = held(id) else {
            continue;
        };
        reads.set.push(repaired(c, others, o, id)?);
    }
    Ok(reads)
}

/// The objects a repair of `target` (`t`) reaches, in order: the target; for a building, an
/// annex's main building, then the main building's annexes by id, the target left out.
fn reached_ids(spatial: &Spatial, t: &Unit, target: i32, building: bool) -> Vec<i32> {
    let mut ids = vec![target];
    if building {
        let head = t.annex_of.unwrap_or(target);
        if head != target {
            ids.push(head);
        }
        let mut annexes = spatial.annexes_of(head);
        annexes.sort_unstable();
        ids.extend(annexes.iter().copied().filter(|&a| a != target));
    }
    ids
}

/// Object `o` of player `oq` (id `id`) as the repair reads and changes it: its master's type,
/// class, id and hit points, its hit points and powerups, a building's main building or annexes
/// (sorted), and its owner's repair cost rates.
fn repaired(
    c: &Step<'_>,
    others: OthersRef<'_>,
    (oq, o): (usize, &Unit),
    id: i32,
) -> Result<crate::construction::repair::Repaired, GenieError> {
    let om = repair_master(&c.w.masters, oq, o)
        .ok_or(GenieError::missing_data("a repaired object's master"))?;
    let attributes: &[f32] = if oq == c.player {
        &c.w.players[c.player].attributes
    } else {
        others
            .players()
            .player(oq)
            .map_or(&[][..], |p| &p.attributes[..])
    };
    let rate = |k: usize| attributes.get(k).copied().unwrap_or(0.0);
    let annexes = if om.kind == 80 && o.annex_of.is_none() {
        let mut a = c.w.spatial.annexes_of(id);
        a.sort_unstable();
        a
    } else {
        Vec::new()
    };
    Ok(crate::construction::repair::Repaired {
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
    })
}

/// The repair's changes to the objects it reached (`set` against `original`): each takes its
/// hit points at once, any player's.
fn repair_write_back(
    w: &mut World,
    set: &[crate::construction::repair::Repaired],
    original: &[crate::construction::repair::Repaired],
) {
    for (now, was) in set.iter().zip(original) {
        if now == was {
            continue;
        }
        if let Some((_, u)) = w.object_mut(now.id) {
            u.life.hp = now.hp;
            u.hp_powerups = now.hp_powerups;
        }
    }
}
