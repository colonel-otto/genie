//! A master's upgrade (slot 0x1a0) and its objects, the objects an effect spawns, the objects'
//! kinds and a negative cost's entries.

use crate::GenieError;
use crate::math::rng::Stream;
use crate::objects::masters::{Master, master_type};
use crate::technology::EffectCommand;
use crate::technology::effect::kind;
use crate::turn::{Turns, Unit};

impl Turns {
    /// Slot 0x1a0 (`FUN_14160e410`) of player `p`: master `from` upgrades to `to` by `mode`
    /// ([`Turns::upgrade_objects`]), with the volleys' hooks around it (the abilities, `+0x930`,
    /// are not held).
    pub(super) fn upgrade_master(
        &mut self,
        p: usize,
        from: i16,
        to: i16,
        mode: i16,
        object: Option<i32>,
    ) -> Result<(), GenieError> {
        self.all_garrison_volleys(p, false)?;
        self.upgrade_objects(p, from, to, mode, object)?;
        self.all_garrison_volleys(p, true)
    }

    /// `FUN_141657d10`, the upgrade's own part: of two masters `from` and `to` of player `p`'s
    /// table, mode 2 with the effect's `object` gives that object a master of its own
    /// (`FUN_141186d10`, a copy of the one it reads) where slot 0xa8 lets it
    /// ([`crate::technology::upgrade::upgradable`]), runs its slot 0x98 to `to` keeping its master
    /// ([`Turns::upgrade_object`]) and copies `to` into its own master (slot 8). Any other mode
    /// goes through the live objects in order: one reading `from` takes its slot 0x98 (its master
    /// kept in modes −1 and 3), or past its use a master of its own and its old death spawn's
    /// copy (`FUN_141201d50`); one reading `from`'s dead unit master (`+0x5e4`, when `to`'s
    /// differs by its copy id) takes a master of its own; in mode 3 one whose own master has
    /// `from`'s id takes slot 0x98 keeping it and copies `to` into it (a building's research
    /// buttons, `FUN_1411c78f0`, are the interface's). Then the sleeping objects reading `from`
    /// take slot 0x98. Upgrades to 0x67f and 0x6a1 move the tracker's list of `from` to `to`'s
    /// (`FUN_14104fd10`, [`crate::players::tracker::Tracker::move_all`]). In modes −1 and 3 two masters of
    /// one type end with `from` taking `to`'s values ([`Master::copy_from`]; the masters' slot
    /// 0xa8, their graphics' loading, is not simulation).
    pub(crate) fn upgrade_objects(
        &mut self,
        p: usize,
        from: i16,
        to: i16,
        mode: i16,
        object: Option<i32>,
    ) -> Result<(), GenieError> {
        use crate::orders::Place;
        use crate::technology::upgrade::upgradable;
        let Some(masters) = self.masters.get(p) else {
            return Ok(());
        };
        let (Some(old), Some(new)) = (masters.get(from), masters.get(to)) else {
            return Ok(());
        };
        // the dead unit master whose objects take masters of their own: `from`'s when `to`'s
        // differs by its copy id
        let dead = masters.get(old.death_spawns[0]).filter(|d| {
            masters
                .get(new.death_spawns[0])
                .is_some_and(|e| d.copy_id != e.copy_id)
        });
        let dead = dead.cloned();
        let keep = mode == -1 || mode == 3;
        let (old, new) = (old.clone(), new.clone());
        if mode == 2
            && let Some(id) = object
        {
            if let Some(place) = self.place(id) {
                let u = self.object_at(place);
                let reads = self.master_read(p, u)?;
                if upgradable(reads.kind, u.life.state) {
                    self.object_at_mut(place).own_master = Some(Box::new(reads.clone()));
                    self.upgrade_object(p, place, (&reads, &new), true)?;
                    if let Some(own) = self.object_at_mut(place).own_master.as_mut() {
                        own.copy_from(&new);
                    }
                }
            }
        } else {
            for i in 0..self.players[p].objects.len() {
                let place = Place::Live(p, i);
                let u = &self.players[p].objects[i];
                let table = u.own_master.is_none();
                if table && u.life.master_id == from {
                    if upgradable(old.kind, u.life.state) {
                        self.upgrade_object(p, place, (&old, &new), keep)?;
                    } else {
                        self.own_and_dead_master(p, i, &old);
                    }
                } else if table && dead.as_ref().is_some_and(|d| d.id == u.life.master_id) {
                    let d = dead.clone().expect("checked");
                    self.players[p].objects[i].own_master = Some(Box::new(d));
                } else if mode == 3
                    && let Some(own) = u.own_master.as_deref().filter(|m| m.id == from)
                    && upgradable(own.kind, u.life.state)
                {
                    let own = own.clone();
                    self.upgrade_object(p, place, (&own, &new), true)?;
                    if let Some(m) = self.players[p].objects[i].own_master.as_mut() {
                        m.copy_from(&new);
                    }
                }
            }
            for i in 0..self.players[p].sleeping.len() {
                let u = &self.players[p].sleeping[i];
                if u.own_master.is_none() && u.life.master_id == from {
                    self.upgrade_object(p, Place::Sleeping(p, i), (&old, &new), keep)?;
                }
            }
        }
        // the upgrades to 0x67f and 0x6a1 move the tracker's list of `from` (FUN_14104fd10)
        if matches!(to, 0x67f | 0x6a1) {
            self.track_move_all(p, from, to);
        }
        if keep
            && old.kind == new.kind
            && let Some(m) = self.masters[p].get_mut(from)
        {
            m.copy_from(&new);
        }
        Ok(())
    }

    /// The master object `u` of player `p` reads: its own, else its player's.
    pub(super) fn master_read(&self, p: usize, u: &Unit) -> Result<Master, GenieError> {
        u.own_master.as_deref().map_or_else(
            || self.master_of(p, u.life.master_id).cloned(),
            |m| Ok(m.clone()),
        )
    }

    /// An object of master `old` past its use (slot 0xa8 false) in an upgrade: a master of its own
    /// (`FUN_141186d10`: a copy of `old` unless it has one) and its dead master
    /// (`FUN_141201d50`: unless it has one, a copy of `old`'s death spawn, `+0x5e4`, of type 10 to
    /// 40, 70 or 80, from its player's table; its slot 0xa8, the graphics, is not simulation).
    pub(super) fn own_and_dead_master(&mut self, p: usize, i: usize, old: &Master) {
        let dead = self
            .masters
            .get(p)
            .and_then(|m| m.get(old.death_spawns[0]))
            .cloned();
        let u = &mut self.players[p].objects[i];
        if u.own_master.is_none() {
            u.own_master = Some(Box::new(old.clone()));
        }
        if u.dead_master.is_none() && 0 <= old.death_spawns[0] {
            u.dead_master = dead
                .filter(|d| crate::turn::DEAD_COPY_TYPES.contains(&d.kind))
                .map(Box::new);
        }
    }

    /// Slot 0x1a8 (`FUN_1416136f0`) of player `p`: `count` objects of master `master` (Gaia's when
    /// the player lacks it) at each of its built objects of building master `at` (state 2, in its
    /// list's order), up to attribute 234's count of buildings when above 0, in a playback (the
    /// screen's mode is not 4). Each object: a male villager (0x53) draws its sex from the Player
    /// stream ("SpawnObject": `r % 100` above 49 a female, 0x125); the owner's slot 0x1c0 makes
    /// it at the building's place with its flag ([`Turns::make_new_object`]); its slot 0x1a0 takes
    /// it into the building where the building has room for one more (slot 0x198,
    /// [`Turns::has_room`]); unless the player's attribute 283 is above 0 it comes out at once
    /// toward no point (`FUN_1411e7220`, [`Turns::let_out`]); a relic (0x11d) counts in the
    /// world's `+0x844` ([`Turns::spawned_relics`]). The local player's statistics of effect
    /// 0x318's spawns are not simulation.
    pub(super) fn spawn_at(
        &mut self,
        p: usize,
        master: i16,
        at: i16,
        count: i16,
    ) -> Result<(), GenieError> {
        /// The male and female villagers' masters.
        const MALE_VILLAGER: i16 = 0x53;
        const FEMALE_VILLAGER: i16 = 0x125;
        /// The relic's master, which the world counts.
        const RELIC: i16 = 0x11d;
        let has = |q: usize, id: i16| self.masters.get(q).is_some_and(|m| m.get(id).is_some());
        let building = self
            .masters
            .get(p)
            .and_then(|m| m.get(at))
            .is_some_and(|m| m.kind == master_type::BUILDING);
        if !building || !(has(p, master) || has(0, master)) || count <= 0 {
            return Ok(());
        }
        let owner = if has(p, master) { p } else { 0 };
        let attribute =
            |q: &crate::turn::Player, i: usize| q.attributes.get(i).copied().unwrap_or(0.0);
        let limit = attribute(&self.players[p], 234) as i32;
        let buildings: Vec<i32> = self.players[p]
            .objects
            .iter()
            .filter(|u| u.life.master_id == at && u.life.state == 2)
            .map(|u| u.id)
            .collect();
        for (k, &b) in buildings.iter().enumerate() {
            if 0 < limit && limit <= k as i32 {
                return Ok(());
            }
            for _ in 0..count {
                let made = if master == MALE_VILLAGER {
                    let r = self.streams.get(Stream::Player).rand15() as i32;
                    if 49 < r % 100 {
                        FEMALE_VILLAGER
                    } else {
                        MALE_VILLAGER
                    }
                } else {
                    master
                };
                let Some((_, building)) = self.object(b) else {
                    break;
                };
                let position = building.position;
                let Some(new) = self.make_new_object(owner, made, position, true)? else {
                    continue;
                };
                let Some((bp, bu)) = self.object(b) else {
                    break;
                };
                if self.has_room(bp, bu, None)? {
                    let Some(crate::orders::Place::Live(np, ni)) = self.place(new) else {
                        continue;
                    };
                    self.enter_forced(np, ni, b)?;
                }
                if attribute(&self.players[p], 283) <= 0.0 {
                    let Some(crate::orders::Place::Live(bp, bi)) = self.place(b) else {
                        break;
                    };
                    let _ = self.let_out(bp, bi, new, [-1.0, -1.0], (None, false))?;
                }
                if master == RELIC {
                    self.spawned_relics += 1;
                }
            }
        }
        Ok(())
    }

    /// The objects' kinds (`FUN_140ec2ac0`'s 200 to 206): 200 to 202 change the
    /// objects with masters of their own of master `a` or class `b` (`FUN_141659dd0`; the slice
    /// holds no own masters: none); 203 to 205 the effect's object, which first takes a master of
    /// its own (`FUN_14165abe0`, `FUN_141186d10`, not held); 206 changes the effect's object's
    /// master (`FUN_14165ae40`). Without an object only 200 to 202 run.
    pub(super) const fn object_effect(
        c: &EffectCommand,
        object: Option<i32>,
    ) -> Result<(), GenieError> {
        match (c.kind, object) {
            (kind::OBJECTS_SET..kind::OBJECT_SET, _) | (_, None) => Ok(()),
            (kind::OBJECT_UPGRADE, Some(_)) => Err(GenieError::unsupported(
                "an effect's master change of its object (kind 206, FUN_14165ae40)",
            )),
            (_, Some(_)) => Err(GenieError::unsupported(
                "an effect on its object's own master (kinds 203 to 205, FUN_141186d10)",
            )),
        }
    }

    /// `FUN_1415c1f70` (a multiply of a cost by a negative amount) of player `p`: the cost
    /// attribute (103 to 106) names the resource (food, wood, gold 3, stone 2); `how` −2 removes
    /// its first cost entry from master `unit` (`FUN_141612df0`), −1 adds one of it, amount −1
    /// and paid, in the first empty entry (`FUN_14160c450`); a `unit` of 900 and up names class
    /// `unit − 900`, every master of it by id. Only a master from type 70 on has costs. The game
    /// scans four entries, the fourth past the three (not modelled); the slice holds the entries
    /// of a resource only, so an added one goes after them.
    pub(super) fn cost_entries(&mut self, p: usize, unit: i16, attribute: i16, how: i32) {
        let resource = match attribute {
            103 => 0,
            104 => 1,
            105 => 3,
            106 => 2,
            _ => return,
        };
        if how != -2 && how != -1 {
            return;
        }
        let Some(masters) = self.masters.get_mut(p) else {
            return;
        };
        let class = (900..1000).contains(&unit).then(|| unit - 900);
        for m in masters.iter_mut() {
            let named = class.map_or(m.id == unit, |c| m.class == c);
            if !named || m.kind < master_type::COMBAT {
                continue;
            }
            if how == -2 {
                if let Some(i) = m.cost.iter().position(|c| c.0 == resource) {
                    let _ = m.cost.remove(i);
                }
            } else if m.cost.len() < 3 {
                m.cost.push((resource, -1, true));
            }
        }
    }
}
