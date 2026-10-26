//! A gate's and a farm's update, a building's completion and seconds, the hit points' heal,
//! regeneration and loss, the idle swap and the death blast.

use crate::build::{Completion, Construction, Site};
use crate::damage::Victim;
use crate::lifecycle::Lifecycle;
use crate::masters::Master;
use crate::rng::{Stream, Well512};
use crate::spatial::Spatial;
use crate::sprite::Graphic;
use crate::turn::unit::{Unit, annexes_of};
use crate::turn::view::Context;
use crate::turn::world::{farm_bonus, victim_of};
use crate::turn::{
    BUILDING_TERRAIN, EJECT_SHARE, GATE_CLOSE_DELAY, GATE_EPSILON, GATE_SHUT_HEIGHT, MONASTERY,
    VILLAGER_CLASS, gate_units,
};
use crate::unit_ai::GenieError;

impl Unit {
    /// The gate's update (`FUN_1411e7730`, `TRIBE_Building_Object::update` for a master of class
    /// 39 outside the walls' branch): a gate in state 2 whose master transforms
    /// (`transform_unit`, `+0x9ba`) clears its first update (its endpoints, not held). Unless it
    /// is shut (its master's height above [`GATE_EPSILON`]) and locked, it counts the units in it
    /// ([`Unit::gate_count`]): a shut, unlocked gate with one of an ally (its player counting the
    /// unit's an ally) in it opens (its player's slot 0x340, `FUN_14162f7e0`: a gate of a height
    /// of at least 1, [`Turns::toggle_gate`]) and its close timer is 0; an open one (a height
    /// below the epsilon) with nothing in it starts its close timer at 0.5 s, or runs it down by
    /// the world's dt, and at 0 shuts (slot 0x338, `FUN_1416193e0`: a height of at most 0).
    pub(in crate::turn) fn gate_update(
        &mut self,
        c: &mut Context<'_>,
        others: &[Self],
    ) -> Result<(), GenieError> {
        let Some(master) = c.masters.and_then(|m| m.get(self.life.master_id)) else {
            return Ok(());
        };
        let transforms = master
            .building
            .as_ref()
            .is_some_and(|b| b.transform_unit != -1);
        if master.class != crate::spatial::GATE_CLASS || !transforms || self.life.state != 2 {
            return Ok(());
        }
        let height = master.height;
        let gate = self
            .gate
            .as_mut()
            .ok_or(GenieError::unsupported("a gate without its gate state"))?;
        gate.first_update = false;
        let shut = GATE_EPSILON < height;
        if gate.locked && shut {
            return Ok(());
        }
        let locked = gate.locked;
        let (total, allied) = self.gate_count(c, others)?;
        let gate = self.gate.as_mut().expect("held above");
        if shut && 0 < allied && !locked {
            c.gate_toggle = GATE_SHUT_HEIGHT <= height;
            gate.close_timer = 0.0;
            return Ok(());
        }
        if height < GATE_EPSILON && total == 0 {
            if gate.close_timer <= 0.0 {
                gate.close_timer = GATE_CLOSE_DELAY;
            } else {
                gate.close_timer -= c.dt_seconds;
                if gate.close_timer <= 0.0 {
                    gate.close_timer = 0.0;
                    c.gate_toggle = height <= 0.0;
                }
            }
        }
        Ok(())
    }

    /// [`gate_units`] of this gate, its annexes the units of `others` that are its (in id order,
    /// as made): the units in it and how many of them are of an ally.
    pub(in crate::turn) fn gate_count(
        &self,
        c: &Context<'_>,
        others: &[Self],
    ) -> Result<(usize, usize), GenieError> {
        let (units, allied) = gate_units(c.spatial, self.id, &annexes_of(self.id, others))?;
        Ok((units.len(), allied))
    }

    /// `TRIBE_Building_Object::update`'s farm part for a foundation (state 0) with a build time
    /// (`0x1411ca2ef`): its stage, its build points times 3 over its build time (truncated, at
    /// most 2), lays terrain under it each update (`FUN_1411ce250` -> `FUN_1411ddd90`,
    /// [`crate::terrain::Terrain::cover`]): 29 plus the stage under a farm (master 50); under a
    /// rice farm or a pasture (1187, 1889) 65 plus the stage on mangrove or rice terrain (`+0xcd0`,
    /// [`Unit::rice`]), else its foundation terrain (`+0x988`) plus 2 plus the stage (bytes).
    /// `FUN_1411ce250` maps the terrains 8, 27 and 64 by the local player's view and the tile's
    /// own (`+0x2e8`'s `+10`), which the slice does not hold; the masters' foundations (7, 63,
    /// 117) never give them.
    pub(in crate::turn) fn farm_stage(&self, c: &mut Context<'_>) -> Result<(), GenieError> {
        let Some(cons) = self
            .construction
            .filter(|k| self.life.state == 0 && 0 < k.build_time)
        else {
            return Ok(());
        };
        let Some(m) = c.masters.and_then(|m| m.get(self.life.master_id)) else {
            return Ok(());
        };
        let foundation = m.building.as_ref().map_or(-1, |b| b.foundation_terrain);
        let Some(terrain) = farm_stage_terrain(
            m.id,
            (self.rice, foundation),
            cons.progress,
            cons.build_time,
        ) else {
            return Ok(());
        };
        if matches!(terrain, 8 | 27 | 64) {
            return Err(GenieError::unsupported(
                "a farm's stage terrain the local player's view maps (8, 27, 64)",
            ));
        }
        let f = crate::terrain::Foundation {
            position: self.position,
            clearance: m.clearance,
            terrain,
            snow: false,
            master: m.id,
            class: m.class,
            terrain_bonus: m.terrain_bonus,
            mode_680: c.mode_680,
        };
        c.spatial
            .terrain
            .as_mut()
            .ok_or(GenieError::unsupported(
                "a farm's foundation without the map's terrain",
            ))?
            .cover(&f)
    }

    /// `TRIBE_Building_Object::update` (`0x1411c9000`) before its combat update: a building in
    /// state 0 whose build points (`+0xc50`, the construction's progress) reached its build time
    /// (its master's first train time, `FUN_140ace5e0`) completes (`set_state(2)`,
    /// [`Construction::complete`]); outside the walls' branch (class 27, master 70) only one that
    /// is not an annex (`+0xbb8`). A builder's work completes a foundation itself
    /// (`FUN_1411cc430`); this one completes a building made without a build time (a death
    /// spawn's). The walls' branch's facet and the other branch's work before it are not modelled
    /// for such a building.
    pub(in crate::turn) fn complete_on_update(
        &mut self,
        c: &mut Context<'_>,
        others: &[Self],
    ) -> Result<(), GenieError> {
        self.farm_stage(c)?;
        let due = self.life.state == 0
            && self
                .construction
                .is_some_and(|cons| f32::from(cons.build_time) <= cons.progress);
        if !due {
            return Ok(());
        }
        let master = c.masters.and_then(|m| m.get(self.life.master_id));
        let walls = master.is_some_and(|m| m.class == 27 || m.id == 70);
        if !walls && self.annex_of.is_some() {
            return Ok(());
        }
        let annexed = others.iter().any(|u| u.annex_of == Some(self.id));
        let Some(construction) = self.construction.as_mut() else {
            return Ok(());
        };
        let mut site = Site {
            id: self.id,
            life: &mut self.life,
            construction,
            mover: self.mover.as_mut(),
            firepower: Some(&mut self.firepower),
            roo_called: Some(&mut self.roo_called),
            store: Some(&mut self.store),
            completion: Completion {
                master,
                annexed,
                attribute_204: c.attribute_204,
                mode_680: c.mode_680,
                farm_bonus: farm_bonus(c.attributes),
                masters: c.masters,
                computer: c.computer,
                ai: self.ai.is_some(),
                main: self.annex_of.is_some(),
                now: c.world_time,
                rice: self.rice,
            },
            chain: crate::build::WorkChain::default(),
            main_builders: None,
        };
        let built = Construction::complete(&mut site, c.spatial, c.pathing)?;
        if let Some(m) = c.masters.and_then(|m| m.get(self.life.master_id)) {
            crate::build::show_built(
                &mut self.sprite,
                m,
                self.build_frame,
                c.graphics,
                c.streams.get(Stream::Sprite),
            );
        }
        // the completion's first part's tracker part (FUN_1411c6bd0 -> FUN_14104eac0)
        let k = crate::tracker::key(c.masters, c.lines, self.life.master_id);
        c.tracker.complete(self.id, k);
        // its set_state(2) took its annexes along first (FUN_1411dbb70)
        for annex in c.spatial.annexes_of(self.id) {
            c.annex_states.push((annex, 2));
        }
        c.attribute_adds.extend(built.attributes);
        c.techs_done.extend(built.tech);
        if built.pasture {
            c.pastures.push(self.id);
        }
        if let Some(state) = built.cart {
            c.carts.push((self.id, state));
        }
        if built.started {
            if let Some(k) = self.construction.as_mut() {
                k.started = false;
            }
            c.started.push(self.id);
        }
        if built.stack.is_some() || built.vanishes {
            c.completions.push((self.id, built.stack, built.vanishes));
        }
        Ok(())
    }

    /// `TRIBE_Building_Object::update`'s whole seconds of a building in state 2 (`0x1411c9000`,
    /// before its combat update): `FUN_1411c39b0` (attribute 255 naming its master: a terrain
    /// effect around it, not modelled); then for each whole second since `last_garrison_time`
    /// (`+0xbe0`, which moves on by them): a monastery's (master 104) relics give their goods
    /// (not modelled with relics inside); each unit inside (from the last) whose
    /// `raider_build_countdown` (`+0x9a0`) runs would be let out at its end (not modelled); the
    /// units inside (from the first) heal by the master's `garrison_heal_rate` below their
    /// master's hit points (slot 0x110, [`Unit::heal`]); with villagers inside, a damaged
    /// building with a `garrison_repair_rate` repairs itself at a cost (`FUN_1411d33e0`, not
    /// modelled). Its `damaged_lately_timer` (`+0x30d`) loses one when whole seconds passed.
    pub(in crate::turn) fn building_seconds(
        &mut self,
        c: &Context<'_>,
        others: &mut [Self],
    ) -> Result<(), GenieError> {
        if self.life.state != 2 {
            return Ok(());
        }
        let Some(master) = c.masters.and_then(|m| m.get(self.life.master_id)) else {
            return Ok(());
        };
        let Some(bm) = master.building else {
            return Ok(());
        };
        // at most a fifth of its hit points (DAT_142eae8d0), it lets its garrison out
        // (FUN_1411e7220 for each from the last, then their slots 0x1a8 and 0xd0); its master's
        // slot 0x110 only sets a sound flag
        if self.life.hp <= f32::from(master.hit_points) * EJECT_SHARE && !self.garrison.is_empty() {
            return Err(GenieError::unsupported(
                "a damaged building lets its garrison out (a fifth of its hit points)",
            ));
        }
        if c.attributes.get(BUILDING_TERRAIN) == Some(&f32::from(master.id)) {
            return Err(GenieError::unsupported(
                "a building whose master attribute 255 names (FUN_1411c39b0)",
            ));
        }
        let seconds = c.world_time.wrapping_sub(self.garrison_time) / 1000;
        if seconds == 0 {
            return Ok(());
        }
        self.garrison_time = self.garrison_time.wrapping_add(seconds * 1000);
        if self.damaged_lately != 0 {
            self.damaged_lately -= 1;
        }
        if master.id == MONASTERY && self.relics != [0; 2] {
            return Err(GenieError::unsupported(
                "a monastery's relics' goods (its seconds)",
            ));
        }
        let inside = |id: i32, others: &[Self]| others.iter().position(|u| u.id == id);
        for &id in self.garrison.iter().rev() {
            let Some(i) = inside(id, others) else {
                continue;
            };
            let kind = c
                .masters
                .and_then(|m| m.get(others[i].life.master_id))
                .map(|m| m.kind);
            if kind == Some(70) && 0 < others[i].raider_countdown {
                return Err(GenieError::unsupported(
                    "a unit inside whose raider_build_countdown runs (FUN_1411e7220)",
                ));
            }
        }
        let rate = bm.garrison_heal_rate;
        let mut villagers = 0;
        for &id in &self.garrison {
            let Some(i) = inside(id, others) else {
                continue;
            };
            let m = c
                .masters
                .and_then(|m| m.get(others[i].life.master_id))
                .ok_or(GenieError::unsupported(
                    "a unit inside of a master the slice does not have",
                ))?;
            if m.class == VILLAGER_CLASS {
                villagers += 1;
            }
            if 0.0 < rate && others[i].life.hp < f32::from(m.hit_points) {
                others[i].heal(m, rate)?;
            }
        }
        if 0 < villagers
            && 0.0 < bm.garrison_repair_rate
            && self.life.hp < f32::from(master.hit_points)
        {
            return Err(GenieError::unsupported(
                "a building its villagers inside repair (FUN_1411d33e0)",
            ));
        }
        Ok(())
    }

    /// Slot 0x110 of a combat object (`FUN_141213b10`): the classes 2, 20, 21, 22 and 53 repair
    /// at a cost (not modelled); any other heals ([`Unit::heal_hp`]).
    pub(in crate::turn) fn heal(&mut self, master: &Master, amount: f32) -> Result<(), GenieError> {
        if matches!(master.class, 2 | 20 | 21 | 22 | 53) {
            return Err(GenieError::unsupported(
                "a heal at a cost (FUN_141213b10's classes)",
            ));
        }
        let _ = self.heal_hp(master, amount);
        Ok(())
    }

    /// `FUN_141193b30`: slot 0xb8 adds `amount` ([`Unit::gain_hp`]); a loss (`amount` below 0)
    /// leaves at least 1 hit point; healed to its full hit points (slot 0xc0 at least slot 200,
    /// `COMISS` with `JC`: a NaN is not), it holds exactly its master's. True then.
    pub(in crate::turn) fn heal_hp(&mut self, master: &Master, amount: f32) -> bool {
        crate::damage::heal_hp(
            &mut self.life.hp,
            &mut self.hp_powerups,
            master.hit_points,
            amount,
        )
    }

    /// Slot 0xb8 of a combat object (`FUN_141203be0`): `amount` added up to its hit points with
    /// its powerups' room, an overflow taken off the powerup (`+0xa38`, never below 0; the hit
    /// point transforms of `FUN_141204450`, empty in the recordings, are not held).
    pub(in crate::turn) fn gain_hp(&mut self, master: &Master, amount: f32) {
        crate::damage::gain_hp(
            &mut self.life.hp,
            &mut self.hp_powerups,
            master.hit_points,
            amount,
        );
    }

    /// Slot 0xb0 of a combat object (`FUN_141205c80`), the loss of `d` hit points: the hit point
    /// powerups' room (`+0xa38` up to `+0xa3c`) takes it first, the rest comes off the hit points,
    /// which stay above the master's `invulnerability_level` floor (`FUN_141202a80`). Not held
    /// (no update of the recordings fills either list): a dying object's kill credits of its
    /// active stingers (`+0xb58`, `+0x954`) and a surviving one's hit point transforms
    /// (`FUN_141204450`); [`crate::damage::lose`] is the same loss on a hit's victim.
    pub(in crate::turn) fn lose_hp(&mut self, master: &Master, d: f32) {
        let mut d = d;
        let [up, room_max] = self.hp_powerups;
        if up < room_max {
            let room = room_max - up;
            if d <= room {
                self.hp_powerups[0] = up + d;
                return;
            }
            self.hp_powerups[0] = up + room;
            d -= room;
        }
        let floor = crate::damage::invulnerability_floor(master.invulnerability, master.hit_points);
        let left = self.life.hp - d;
        self.life.hp = if floor == 0.0 || floor <= left {
            left
        } else {
            floor
        };
    }

    /// `TRIBE_Combat_Object::update`'s regeneration (`0x141208440` after
    /// `RGE_Combat_Object::update`): a living object (at least 1 hit point, `COMISS`/`JC`: a NaN
    /// is not; in state 2) whose rate ([`regeneration_rate`], slot 0x260) is above
    /// [`REGENERATION_EPSILON`] in size (a NaN is) earns a whole hit point per 60 / |rate|
    /// seconds: the world's dt plus the time held back (`berserker_timer`, `+0xa10`) over that
    /// period (none for a period of at most [`REGENERATION_EPSILON`]), its fraction (`fmodf`)
    /// held back as time. Whole points above 0 heal (a positive rate: slot 0xb8,
    /// [`Unit::gain_hp`]) at most the room below its most (slot 200: its master's hit points plus
    /// its powerups' room; the hit point stinger `+0xb44`, written only by auras, is not held), or
    /// come off (slot 0xb0, [`Unit::lose_hp`]) at most its hit points with its powerups' room
    /// (slot 0xc0). Then an object at its most (slot 0xc0 at least slot 200) has the armor its
    /// charge's hits took back (`+0x9e4` and `+0x9e6`, one dword, zeroed).
    pub(in crate::turn) fn regenerate(&mut self, master: &Master, dt_seconds: f32) {
        use crate::fmath::{at_most_or_unordered, below_or_unordered};
        let rate = regeneration_rate(master, self);
        if below_or_unordered(self.life.hp, REGENERATION_LEAST_HP) || self.life.state != 2 {
            return;
        }
        if !below_or_unordered(REGENERATION_EPSILON, rate.abs()) {
            return;
        }
        let held = dt_seconds + self.regen_timer;
        let per = (REGENERATION_MINUTE / rate).abs();
        let points = if below_or_unordered(REGENERATION_EPSILON, per.abs()) {
            held / per
        } else {
            0.0
        };
        let fraction = points % REGENERATION_POINT;
        let whole = points - fraction;
        self.regen_timer = fraction * per;
        if at_most_or_unordered(whole, 0.0) {
            return;
        }
        let [up, room] = self.hp_powerups;
        let most = f32::from(master.hit_points) + room;
        let now = (room - up) + self.life.hp;
        if at_most_or_unordered(rate, 0.0) {
            let amount = if at_most_or_unordered(whole, now) {
                whole
            } else {
                now
            };
            if !at_most_or_unordered(amount, 0.0) {
                self.lose_hp(master, amount);
            }
        } else {
            let left = most - now;
            let amount = if at_most_or_unordered(whole, left) {
                whole
            } else {
                left
            };
            if !at_most_or_unordered(amount, 0.0) {
                self.gain_hp(master, amount);
            }
        }
        // at its most hit points (slot 0xc0 at least slot 200, `COMISS`: a NaN is not) the
        // armor its charge's hits took (`+0x9e4`, `+0x9e6`) comes back
        let [up, room] = self.hp_powerups;
        if most <= (room - up) + self.life.hp {
            self.armor_removed = [0; 2];
        }
    }

    /// `TRIBE_Combat_Object::update`'s hit point powerup check (after its abilities): a powerup
    /// (`+0xa38`) above its room (`+0xa3c`; `COMISS`/`JBE`, a NaN is not) comes off the hit points
    /// (`MAXSS` with 1: not below 1, a NaN becomes 1) and the powerup takes the room.
    pub(in crate::turn) fn clamp_hp_powerup(&mut self) {
        let [up, room] = self.hp_powerups;
        if crate::fmath::at_most_or_unordered(up, room) {
            return;
        }
        self.life.hp = crate::fmath::maxss(self.life.hp - (room - up).abs(), HP_POWERUP_FLOOR);
        self.hp_powerups[0] = room;
    }

    /// One update, and whether the object stays in its list, leaves it or goes to sleep. `others`
    /// are the rest of the owner's objects, whose foundations a builder works on.
    /// The animated object's idle swap (the end of `RGE_Animated_Object::update`,
    /// `0x141071880`): an object whose master has a second standing sprite (`+0x478`) and whose
    /// slot 0x5d8 answers 0 (an animated object's always; a moving one's while its master's move
    /// algorithm, `+0x768`, has bit 0 clear), showing either standing sprite, whose sprite's node
    /// looped in this update (the sprite list's slot 0x40, the node's slot 12), draws on the
    /// Sprite stream ("Update"); above 0x3fff it shows the other (`FUN_141189760` with the other
    /// index and 0x7fff: `+0x78c` takes the index, slot 0x68 the master's sprite of that index,
    /// [`crate::attack::set_sprite`], and the list's slot 0x28 puts its node back to its first
    /// frame, [`crate::sprite::ActiveSprite::reset_frame`]).
    pub(in crate::turn) fn idle_swap(
        &mut self,
        master: &Master,
        graphics: &std::collections::BTreeMap<i16, Graphic>,
        sprite_rng: &mut Well512,
    ) {
        let Some(second) = master.standing_sprite_2 else {
            return;
        };
        if self
            .mover
            .as_ref()
            .is_some_and(|m| m.stats.move_algorithm & 1 != 0)
        {
            return;
        }
        let Some(shown) = self.sprite.as_ref() else {
            return;
        };
        let id = shown.graphic.id;
        if (id != master.standing_sprite && id != second) || !shown.looped {
            return;
        }
        if sprite_rng.rand15() <= 0x3fff {
            return;
        }
        self.idle_variant = i32::from(self.idle_variant == 0);
        let next = if self.idle_variant == 0 {
            master.standing_sprite
        } else {
            second
        };
        if let Some(&g) = graphics.get(&next) {
            crate::attack::set_sprite(&mut self.sprite, g, sprite_rng);
        }
        if let Some(s) = self.sprite.as_mut() {
            s.reset_frame();
        }
    }

    /// `TRIBE_Combat_Object::update` (`0x141208440`) before `RGE_Combat_Object::update`: a unit
    /// that blasts at its death ([`blasts_at_death`]) blasts about its place (slot 0x848,
    /// `FUN_14107f690`, [`crate::damage::blast`]: itself left out, no main target, the factor
    /// 1, the hit info `FUN_140aa8720` gives) over the objects its caller gathered
    /// ([`Context::blasted`], written back after the update); its death follows in the static
    /// update. Its own part of the blast is written back ([`Unit::hit_back`]); a kill of it
    /// changes nothing, as it dies anyway.
    ///
    /// # Errors
    ///
    /// What the blast does not model.
    pub(super) fn death_blast(&mut self, c: &mut Context<'_>) -> Result<(), GenieError> {
        let Some(master) = c.masters.and_then(|m| m.get(self.life.master_id)) else {
            return Ok(());
        };
        if !blasts_at_death(master, &self.life) {
            return Ok(());
        }
        let terrain = c.spatial.terrain.as_ref();
        let mut own = victim_of(self, master, c.attributes, c.player, terrain, c.mode_680);
        let mut targets: Vec<&mut Victim> = c.blasted.iter_mut().collect();
        let _ = crate::damage::blast(
            &mut own,
            self.position,
            Some(self.id),
            None,
            &mut targets,
            DEATH_BLAST_FACTOR,
        )?;
        let _ = self.hit_back(own)?;
        Ok(())
    }

    /// `RGE_Moving_Object::update`: a last human order time (`+0x650`) at most `FUN_140fe0d10`'s
    /// counts down by the world's dt (`+0x7c`, in ms), not below 0. The walk reads the field
    /// live, so the world's view follows at once: the unit's own walk later in this update ranks
    /// the units it meets by the counted-down time.
    pub(in crate::turn) fn count_down_human_order(&mut self, dt: u32, spatial: &mut Spatial) {
        if self.last_human_order <= crate::unit_ai::HUMAN_ORDER_CAP {
            self.last_human_order = self.last_human_order.saturating_sub(dt);
            spatial.set_human_order(self.id, self.last_human_order);
        }
    }
}

/// The terrain a farm foundation of master `master` lays at its stage ([`Unit::farm_stage`]):
/// its build points `progress` times 3 over its `build_time` (truncated, at most 2) onto 29
/// (`0x1d`) for a farm (50), onto 65 (`0x41`) for a rice farm or a pasture (1187, 1889) on
/// mangrove or rice (`rice`), else onto its `foundation` terrain plus 2 (bytes); None for
/// another master.
pub(in crate::turn) fn farm_stage_terrain(
    master: i16,
    (rice, foundation): (bool, i16),
    progress: f32,
    build_time: i16,
) -> Option<u8> {
    /// The stages over the build time (`DAT_142eb1c98`).
    const STAGES: f32 = crate::image::turn::FARM_STAGES;
    /// A farm's first stage terrain (`0x1d`).
    const FIRST_STAGE: u8 = 29;
    /// A rice farm's first stage terrain on mangrove or rice (`0x41`).
    const RICE_STAGE: u8 = 65;
    let stage = (((progress * STAGES) / f32::from(build_time)) as i32 as u8).min(2);
    match master {
        50 => Some(FIRST_STAGE + stage),
        1187 | 1889 if rice => Some(RICE_STAGE + stage),
        1187 | 1889 => Some((foundation as u8).wrapping_add(2).wrapping_add(stage)),
        _ => None,
    }
}

/// The masters whose units blast about themselves below 1 hit point
/// (`TRIBE_Combat_Object::update`, `0x141208440`): the demolition ships 527 and 528, the
/// demolition raft 1104 and the incendiary raft and ships 2133 to 2135.
pub(super) const DEATH_BLAST_MASTERS: [i16; 6] = [527, 528, 1104, 2133, 2134, 2135];
/// The classes whose units blast so: 35 (petards) and 64.
pub(super) const DEATH_BLAST_CLASSES: [i16; 2] = [35, 64];
/// The hit points below which they blast (`DAT_14330eb8c`; `COMISS`, a NaN does not).
pub(super) const DEATH_BLAST_HP: f32 = crate::image::turn::DEATH_BLAST_HP;
/// The blast's damage factor (`XMM11`, the same constant).
pub(super) const DEATH_BLAST_FACTOR: f32 = crate::image::turn::DEATH_BLAST_FACTOR;

/// `TRIBE_Combat_Object::update`'s test for the blast at death: a unit of one of
/// [`DEATH_BLAST_MASTERS`] or [`DEATH_BLAST_CLASSES`] below [`DEATH_BLAST_HP`] hit points in state
/// 2. The update runs for the combat objects and buildings (types 70 and 80).
pub(in crate::turn) fn blasts_at_death(master: &Master, life: &Lifecycle) -> bool {
    70 <= master.kind
        && life.hp < DEATH_BLAST_HP
        && life.state == 2
        && (DEATH_BLAST_MASTERS.contains(&master.id) || DEATH_BLAST_CLASSES.contains(&master.class))
}

/// A hero's regeneration (`DAT_142ed06a8`, a minute's hit points).
pub(super) const HERO_REGENERATION: f32 = crate::image::turn::HERO_REGENERATION;
/// The regeneration's minute (`DAT_14330ebc8`).
pub(super) const REGENERATION_MINUTE: f32 = crate::image::turn::REGENERATION_MINUTE;
/// The size below which a rate or a period is none (`DAT_142f49458`).
pub(super) const REGENERATION_EPSILON: f32 = crate::image::turn::REGENERATION_EPSILON;
/// The hit points below which nothing regenerates (`DAT_14330eb8c`).
pub(super) const REGENERATION_LEAST_HP: f32 = crate::image::turn::REGENERATION_LEAST_HP;
/// A whole hit point, the `fmodf` divisor (`XMM11`, `DAT_14330eb8c`).
pub(super) const REGENERATION_POINT: f32 = crate::image::turn::REGENERATION_POINT;
/// The hit points a powerup's check leaves at least (`MAXSS` with `DAT_14330eb8c`).
pub(super) const HP_POWERUP_FLOOR: f32 = crate::image::turn::HP_POWERUP_FLOOR;

/// Slot 0x260 of a combat object (`FUN_1412033a0`), its regeneration rate in hit points a
/// minute: its stinger (`+0xb3c`, slot 0x750) plus (its powerup (`+0xa34`, slot 0x6f0) plus (a
/// hero's [`HERO_REGENERATION`] plus its master's `regeneration_rate`, `+0x8b4`)), in the
/// assembly's order. A hero is a master whose `hero_flag` (`+0x8d9`) has bit 0 or 2, unless it
/// has both bits 7 and 2.
#[must_use]
pub(in crate::turn) fn regeneration_rate(master: &Master, unit: &Unit) -> f32 {
    let flag = master.hero_flag;
    let hero = flag & 5 != 0 && !(flag & 0x80 != 0 && flag & 4 != 0);
    let bonus = if hero { HERO_REGENERATION } else { 0.0 };
    let [powerup, stinger] = unit.regen_powerups;
    stinger + (powerup + (bonus + master.regeneration))
}
