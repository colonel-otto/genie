//! A unit's strikes back and its volleys: the missiles it fires and launches.

use crate::GenieError;
use crate::combat::damage::Victim;
use crate::map::spatial::Placed;
use crate::math::rng::Stream;
use crate::movement::Mover;
use crate::objects::lifecycle::{Class, Lifecycle};
use crate::objects::masters::Master;
use crate::objects::release::Releaser;
use crate::objects::sprite::ActiveSprite;
use crate::objects::unit::Unit;
use crate::unit_ai::{ACTION_EXECUTED, Notify};
use crate::vision::visibility::{Seer, capture_kind};
use crate::world::ARMED_FIREPOWER;
use crate::world::context::Context;

impl Unit {
    /// The unit's own part of its hit, written back: what a retaliation or reflected damage left
    /// of its hit points and powerups, its charge and `+0x8a8`, the damage notify (500) a
    /// retaliation posts to its unit AI; a retaliation that killed it is the caller's (Gaia
    /// taking a killed attacker is not modelled).
    pub(crate) fn hit_back(
        &mut self,
        own: Victim,
    ) -> Result<Option<crate::combat::damage::Kill>, GenieError> {
        self.life.hp = own.hp;
        if let Some(a) = own.armored.as_ref() {
            self.hp_powerups = a.hp_powerups;
        }
        self.charge = own.source.charge;
        self.charge_hit = own.source.charge_hit;
        if let Some(ai) = self.ai.as_mut() {
            for &(attacker, p) in &own.damaged {
                let n = Notify {
                    caller: attacker,
                    recipient: self.id,
                    kind: crate::unit_ai::DAMAGED,
                    p,
                };
                ai.post(n, self.life.state)?;
            }
        }
        if own.killed.is_some_and(|k| k.to_gaia) {
            return Err(GenieError::unsupported(
                "a killed attacker that goes to Gaia",
            ));
        }
        Ok(own.killed)
    }

    /// The action list's deletion of the unit's own Attack (`FUN_140a98e20`): its teardown (a
    /// `MoveTo`'s stops the unit) and the base teardown (`FUN_140a73fc0`), which lets its target,
    /// then its second target go (slot 0x180).
    pub(crate) fn delete_attack(&mut self, c: &mut Context<'_>) -> Result<(), GenieError> {
        let Some(mut a) = self.attack.take() else {
            return Ok(());
        };
        a.tear_down(self.body.mover_mut())?;
        let by = self.releaser(c);
        for t in [a.target, a.target2] {
            if t != -1 {
                c.releases.push((t, by));
            }
        }
        Ok(())
    }

    /// Slot 0x858 (`FUN_14120c0a0`), a combat object's hit at its target from the Attack's swing
    /// when it has a missile, a volley missile or a charge projectile
    /// ([`crate::combat::attack::volleys`]; the others' melee hit is the Attack's). A master without
    /// `max_attacks_in_volley` fires its missile ([`Unit::fire`]). One with it fires its volley
    /// (`next_volley`, `+0xa04`, already fired of [`crate::combat::missile::volley_count`] missiles):
    /// unless its `combat_ability` has bit 4, a master with a spread fires 1 to 3 at a time (an
    /// Object-stream draw, "attack"), the rest of the volley next with its attack timer at 0.05;
    /// the volley's last (masters 73 and 559 tell their unit AI the action executed) leaves
    /// `next_volley` 0. Of the missiles at a time, the first of a volley is its missile (none
    /// without one) and the others its extra missiles ([`Unit::volley_missile`]), each scattered
    /// by two draws. A target of a player but Gaia has its player make a flare at the attacker
    /// ([`Context::flares`]). Not modelled: the self-destructing masters (527, 528, 1104,
    /// 2133..2135, classes 35 and 64 alive), a charge (charge types 6 and 7) and a target of a
    /// computer player (its notify 0x214). Not held: the local player's statistics
    /// (`FUN_140a4ba80`).
    pub(crate) fn volley(&mut self, c: &mut Context<'_>, aimed: Aimed) -> Result<(), GenieError> {
        let master = c
            .masters
            .and_then(|m| m.get(self.life.master_id))
            .ok_or(crate::world::ATTACKER_MASTER)?;
        if (matches!(master.id, 527 | 528 | 1104 | 2133..=2135) || matches!(master.class, 35 | 64))
            && self.life.state == 2
        {
            return Err(GenieError::unsupported(
                "a self-destructing attacker's hit (its object 112 and its death)",
            ));
        }
        if matches!(master.charge_type, 6 | 7) {
            return Err(GenieError::unsupported(
                "a charged hit (slot 0x2f8 with charge types 6 and 7)",
            ));
        }
        let missile = 0 <= master.missile;
        let (first, extra) = if master.max_volley == 0 {
            (true, 0)
        } else {
            let count = crate::combat::missile::volley_count(self.firepower, master.max_volley);
            let old = self.next_volley;
            let mut n = count - i32::from(old);
            let [x_spread, y_spread, _] = master.volley_spread;
            let mut all = true;
            if master.combat_ability & 0x10 == 0 {
                let k = if 0.0 < x_spread || 0.0 < y_spread {
                    let draw = c.streams.get(Stream::Object).rand15() as i32;
                    (draw * 2) / 0x7fff + 1
                } else {
                    1
                };
                if k < n {
                    self.next_volley = self.next_volley.wrapping_add(k as u8);
                    // an immediate 0.05
                    self.attack_timer = 0.05;
                    n = k;
                    all = false;
                }
            }
            if all {
                self.next_volley = 0;
                if matches!(master.id, 73 | 559) {
                    self.post_own(ACTION_EXECUTED, [600, -1, 0])?;
                }
            }
            (old == 0, if old == 0 { n - 1 } else { n })
        };
        if missile || 0 < extra {
            // a target of a player but Gaia: its player makes a flare (master 112, slot 0x1c0)
            // at the attacker's place, which lasts twice as long from a trebuchet (class 54:
            // its amount, +0x310, doubled); the caller's after the update
            if 0 < aimed.player {
                c.flares
                    .push((aimed.player, self.position, master.class == 54));
            }
            let [x_spread, y_spread, _] = master.volley_spread;
            for _ in 0..extra {
                let object = c.streams.get(Stream::Object);
                let (x, y) = (object.rand15(), object.rand15());
                let scatter = [
                    crate::combat::missile::scatter(x, x_spread),
                    crate::combat::missile::scatter(y, y_spread),
                ];
                self.volley_missile(c, &aimed, scatter)?;
            }
        }
        if (extra != 0 || missile) && c.computers.get(aimed.player).copied().unwrap_or(false) {
            return Err(GenieError::unsupported(
                "a hit on a computer player's object (its notify 0x214)",
            ));
        }
        if first && missile {
            self.fire(c, &aimed)?;
        }
        Ok(())
    }

    /// `FUN_141081020`, a ranged unit's swing at its target (from its volley): its attack count
    /// (`+0x900`) rises; its master's missile (`missile_id`; a charge projectile, slot 0x2f8 with
    /// charge types 6 and 7, is not modelled) at its weapon offset turned by its facing
    /// ([`crate::combat::missile::spawn_point`]), its reach [`crate::combat::missile::reach`], aimed at the
    /// target's place, the shooter its origin ([`Unit::launch`], `FUN_14113f520`). Last the
    /// shooter tells its unit AI the action executed (0x200 with 600, but for masters 73 and
    /// 559). Not modelled: a target of a computer player, which a shooter whose blast does not
    /// spare friends notifies (0x20f). Not held: the damaged flag of masters 83, 293, 122 and 216
    /// (`+0x683`).
    pub(crate) fn fire(&mut self, c: &mut Context<'_>, aimed: &Aimed) -> Result<(), GenieError> {
        self.attack_count += 1;
        let master =
            c.masters
                .and_then(|m| m.get(self.life.master_id))
                .ok_or(GenieError::unsupported(
                    "a ranged unit of a master the slice does not have",
                ))?;
        if matches!(master.charge_type, 6 | 7) {
            return Err(GenieError::unsupported(
                "a charge projectile (charge types 6 and 7, slot 0x2f8)",
            ));
        }
        let facing = self
            .body
            .mover()
            .map(|m| m.angle)
            .ok_or(GenieError::unsupported("a ranged unit that does not move"))?;
        let shot = Shot {
            made: master.missile,
            at: crate::combat::missile::spawn_point(self.position, facing, master.weapon_offset),
            point: aimed.position,
            reach: crate::combat::missile::reach(master.radius, master.weapon_range),
            shooter: true,
            aimer: crate::combat::missile::Aimer {
                hit_chance: master.hit_chance,
                spread: master.missed_spread,
                class: master.class,
            },
            attack_count: self.attack_count,
        };
        self.launch(c, aimed, &shot)?;
        if !matches!(self.life.master_id, 73 | 559) {
            self.post_own(ACTION_EXECUTED, [600, aimed.target, 0])?;
        }
        // FUN_1410bfcb0: a blast of a unit of range above 1 without the level's bit 2 does not
        // spare friends
        let spares_friends = !(1.0 < master.weapon_range && master.blast_level & 4 == 0);
        if !spares_friends && c.computers.get(aimed.player).copied().unwrap_or(false) {
            return Err(GenieError::unsupported(
                "a missile fired at a computer player's object (its notify 0x20f)",
            ));
        }
        Ok(())
    }

    /// `FUN_141226690`, a volley's extra missile scattered by `scatter`: the master's volley
    /// missile (`FUN_141203760`, a master its player has), shooterless, else its missile with
    /// the shooter; placed by [`crate::combat::missile::volley_spawn_point`], its reach
    /// [`crate::combat::missile::volley_reach`], aimed at the target's place plus the scatter turned by
    /// the facing (`FUN_14113f410`); the missile's own aim when shooterless; it records the
    /// object as what it was fired from (`+0x998`). Last the object tells its unit AI the action
    /// executed (0x200 with 600 and no target, but for masters 73 and 559). Not modelled: a
    /// missile of master 527 or 528 (whose aim reads its attack count, set after it) and a
    /// master without a missile of either kind.
    pub(crate) fn volley_missile(
        &mut self,
        c: &mut Context<'_>,
        aimed: &Aimed,
        scatter: [f32; 2],
    ) -> Result<(), GenieError> {
        let masters = c
            .masters
            .ok_or(GenieError::unsupported("a volley without the masters"))?;
        let master = masters
            .get(self.life.master_id)
            .ok_or(GenieError::unsupported(
                "a ranged unit of a master the slice does not have",
            ))?;
        let facing = self
            .body
            .mover()
            .map(|m| m.angle)
            .ok_or(GenieError::unsupported("a ranged unit that does not move"))?;
        let building = master.class == crate::garrison::BUILDING_CLASS;
        let at = crate::combat::missile::volley_spawn_point(
            self.position,
            facing,
            master.weapon_offset,
            master.volley_spread[2],
            scatter,
            building.then_some([aimed.position[0], aimed.position[1]]),
        );
        let volley = master.volley_missile.and_then(|id| masters.get(id));
        let made = match volley {
            Some(v) => v,
            None if 0 <= master.missile => masters.get(master.missile).ok_or(
                GenieError::unsupported("a missile its player's masters do not have"),
            )?,
            None => {
                return Err(GenieError::unsupported(
                    "a volley's extra missile of a master without a missile",
                ));
            }
        };
        if matches!(made.id, 527 | 528) {
            return Err(GenieError::unsupported(
                "a volley's extra missile of master 527 or 528 (its attack count)",
            ));
        }
        let shooter = volley.is_none();
        let aimer = if shooter { master } else { made };
        let shot = Shot {
            made: made.id,
            at,
            point: crate::combat::missile::spawn_point(
                aimed.position,
                facing,
                [scatter[0], scatter[1], 0.0],
            ),
            reach: crate::combat::missile::volley_reach(master.radius, master.weapon_range),
            shooter,
            aimer: crate::combat::missile::Aimer {
                hit_chance: aimer.hit_chance,
                spread: aimer.missed_spread,
                class: aimer.class,
            },
            attack_count: 0,
        };
        self.launch(c, aimed, &shot)?;
        if !matches!(self.life.master_id, 73 | 559) {
            self.post_own(ACTION_EXECUTED, [600, -1, 0])?;
        }
        Ok(())
    }

    /// A missile's making and start, which [`Unit::fire`] and [`Unit::volley_missile`] share.
    /// The missile master comes from the player's master's slot 0x40 (`FUN_1410d5920`): from the
    /// pool of type 60 when it holds one (keeping its id), else new with the next reusable id
    /// ([`Recycling`]). The setups draw as a new object's do (its standing sprite,
    /// [`ActiveSprite::start`] on the Sprite stream; its facet for a sprite with a random first
    /// frame and several facets, then its angle for a turning one: Object stream) and put it on
    /// the map (slot 0x58: held inside it; a pooled missile that last flew no farther than 0.1
    /// takes the ground by its fly mode); it joins the end of its owner's list. Then its action
    /// (`FUN_140a9a040`): the shot's point, the master's hit mode, the target taken (slot 0x178)
    /// and, with the shooter, the shooter taken too and its place, master and owner with the
    /// missile's radii as its origin (else none); the list's add starts it (`FUN_140a9c5b0`):
    /// `set_state` 3 aims ([`crate::combat::missile::aim`], Action stream) and launches
    /// ([`crate::combat::missile::MissileAction::launch`], its angle). The missile keeps the object's id
    /// as what it was fired from, its reach and the object's attack count. Not held: the
    /// missile's sprite and pitch facet, and a shooter's own master copy (its origin's master
    /// −1).
    pub(crate) fn launch(
        &mut self,
        c: &mut Context<'_>,
        aimed: &Aimed,
        shot: &Shot,
    ) -> Result<(), GenieError> {
        let made = c
            .masters
            .and_then(|m| m.get(shot.made))
            .ok_or(GenieError::unsupported(
                "a missile its player's masters do not have",
            ))?;
        let stats = made.missile_stats.ok_or(GenieError::unsupported(
            "a missile master without its flight",
        ))?;
        if !made.recyclable {
            return Err(GenieError::unsupported(
                "a missile of a master that is not recyclable",
            ));
        }
        let moving = made.moving.ok_or(GenieError::unsupported(
            "a missile master that does not move",
        ))?;
        let (id, flying) = if let Some(pooled) = c.recycling.missiles.pop() {
            pooled
        } else {
            let id = c.recycling.next_id;
            c.recycling.next_id -= 1;
            (id, true)
        };
        // the setups' draws: static (its sprite, its facet), then moving (its angle)
        let standing = (0 <= made.standing_sprite)
            .then(|| c.graphics.get(&made.standing_sprite).copied())
            .flatten();
        if let Some(g) = standing {
            let _ = ActiveSprite::start(g, c.streams.get(Stream::Sprite));
            if g.flags & 4 != 0 && 1 < g.facets {
                let _ = c.streams.get(Stream::Object).rand15();
            }
        }
        let facets = standing.map(|g| crate::objects::facing::SpriteFacets {
            count: g.facets,
            flag_1: g.flags & 2 != 0,
        });
        let mut angle = 0.0;
        if let Some(f) = facets.filter(|_| crate::objects::facing::draws_angle(facets)) {
            let r = c.streams.get(Stream::Object).rand15();
            angle = crate::objects::facing::drawn_angle(r, f);
        }
        let mut mover = Mover {
            animated: crate::objects::layers::Animated::default(),
            forward: [1.0, 0.0, 0.0],
            right: [0.0, 0.0, crate::production::spawn::START_RIGHT_Z],
            velocity: None,
            paths: Vec::new(),
            factor: 1.0,
            stats: moving,
            waiting: 0,
            wait_delays: 0,
            attack_moving: false,
            last_move: c.world_time,
            moved_at: [0.0; 3],
            substitute_count: 0,
            ignore_units: Vec::new(),
            user_waypoints: Vec::new(),
            last_turn: 0.0,
            angle,
            move_to_point: None,
            pauses: Vec::new(),
            in_the_way: Vec::new(),
            sprite_facets: facets,
        };
        let mut position = mover.set_position(shot.at, c.spatial.map);
        if !flying {
            let ground = c
                .spatial
                .terrain
                .as_ref()
                .ok_or(GenieError::unsupported(
                    "a missile without the map's terrain",
                ))?
                .ground(position[0], position[1])?;
            position[2] =
                crate::combat::missile::grounded(position[2], ground, false, stats.fly_mode);
        }
        mover.mark_move(position, c.world_time);
        let solid = 0.0 < made.radius[0] && 0.0 < made.radius[1] && made.tall;
        let placed = Placed {
            position,
            radius: made.radius,
            class: made.class,
            player: c.player,
            master: made.id,
            obstruction_type: made.obstruction_type,
            buildable_on: made.buildable_on,
            tall: made.tall,
            speed: mover.stats.speed,
            unknown: None,
            obstruction: (made.obstruction_type != 0 && solid).then_some(
                crate::map::spatial::Obstruction {
                    kind: made.obstruction_type,
                    class: made.obstruction_class,
                },
            ),
        };
        c.spatial.insert(id, placed);
        c.pathing.moved(id, &placed, false);
        if let Some(v) = c.spatial.sight.as_mut() {
            v.add_seer(
                id,
                Seer {
                    player: c.player,
                    master: made.id,
                    los: made.line_of_sight,
                    state: 2,
                    explored: false,
                    building: None,
                    umv: 0,
                    capture: capture_kind(c.player, made.class, made.id),
                    kind: made.kind,
                },
            );
            let _ = v.sight_on(id, position, made.radius, false, false);
        }
        let mut held = (-1, 0.0);
        for &(kind, amount, flag) in &made.storage {
            if kind >= 0 && matches!(flag, 1 | 2 | 32) {
                c.attribute_adds.push((kind, amount));
            }
            if kind != -1 && flag == 0 {
                held = (kind, amount);
            }
        }
        // its action: the target and the shooter taken (slot 0x178), then the start
        let mut action = crate::combat::missile::MissileAction {
            state: 0,
            target: aimed.target,
            fired_from: if shot.shooter { self.id } else { -1 },
            point: shot.point,
            timer: 0.0,
            direction: [0.0; 3],
            ballistic_velocity: 0.0,
            ballistic_acceleration: 0.0,
            distance: 0.0,
            hit_mode: stats.hit_mode,
            result: 0,
            origin: if shot.shooter {
                crate::combat::missile::Origin {
                    position: self.position,
                    radius: made.radius,
                    master: i32::from(self.life.master_id),
                    owner: c.player as i32,
                }
            } else {
                crate::combat::missile::Origin::NONE
            },
            met: Vec::new(),
        };
        let missile = Releaser {
            id,
            class: made.class,
            player: c.player,
        };
        c.takes.push((aimed.target, missile));
        if shot.shooter {
            // the shooter, of the missile's own player: never listed
            self.taken(false, id);
        }
        let point = {
            let draws = c.streams.get(Stream::Action);
            crate::combat::missile::aim(
                action.point,
                position,
                Some((aimed.radius, aimed.class)),
                shot.aimer,
                made.id,
                shot.attack_count,
                &mut action.hit_mode,
                &mut || draws.rand15(),
            )?
        };
        if let Some(a) = action.launch(position, point, &stats)? {
            mover.angle = a;
        }
        let remains = made.death_spawns[1];
        c.made.push(Self {
            id,
            position,
            life: Lifecycle {
                class: Class::Animated,
                state: 2,
                hp: f32::from(made.hit_points),
                attribute_type: held.0,
                amount: held.1,
                dying_sprite: (0 <= made.death_sprite).then_some(made.death_sprite),
                death_spawn: 0 <= made.death_spawns[0],
                remains,
                death_facets: None,
                inside: None,
                goto_sleep: false,
                master_id: made.id,
                master: made.life,
            },
            body: crate::objects::layers::Body::new(None, Some(mover)),
            missile: Some(crate::combat::missile::Missile {
                max_range: shot.reach,
                fired_from: self.id,
                charge: false,
                flying,
                step: [0.0; 3],
                attack_count: self.attack_count,
                action: Some(action),
            }),
            attackable: true,
            pasture: -1,
            my_pasture: -1,
            ..Self::default()
        });
        Ok(())
    }
}

/// What a ranged unit's fire reads of its target ([`Unit::fire`]): its id, place, master's radii
/// and class, and player.
pub(crate) struct Aimed {
    pub(crate) target: i32,
    pub(crate) position: [f32; 3],
    pub(crate) radius: [f32; 3],
    pub(crate) class: i16,
    pub(crate) player: usize,
}

/// `FUN_14120ea30` of an attacker of `master` with `firepower` (`+0xa00`): a building can attack
/// when of class 51 or 54, with a missile, or with a firepower above 1; any other attacker can.
pub(crate) fn armed(master: &Master, firepower: f32) -> bool {
    master.kind != crate::orders::BUILDING
        || matches!(master.class, 51 | 54)
        || master.missile != -1
        || ARMED_FIREPOWER < firepower
}

/// Whether two lists hold the same objects, in any order.
pub(crate) fn same_members(a: &[i32], b: &[i32]) -> bool {
    let (mut a, mut b) = (a.to_vec(), b.to_vec());
    a.sort_unstable();
    b.sort_unstable();
    a == b
}

/// What a missile was fired as, which its hits read while its own part is out of the unit: a
/// charge's projectile (`+0x99c`) and what it was fired from (`+0x998`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Fired {
    pub(crate) charge: bool,
    pub(crate) by: i32,
}

/// A missile [`Unit::launch`] makes: its master, where it starts, the point its action aims at,
/// its reach (`+0x990`), whether its action has the object as its shooter (else none), what its
/// aim reads of the firing object (or the missile) and the attack count the aim sees.
pub(crate) struct Shot {
    pub(crate) made: i16,
    pub(crate) at: [f32; 3],
    pub(crate) point: [f32; 3],
    pub(crate) reach: f32,
    pub(crate) shooter: bool,
    pub(crate) aimer: crate::combat::missile::Aimer,
    pub(crate) attack_count: i32,
}
