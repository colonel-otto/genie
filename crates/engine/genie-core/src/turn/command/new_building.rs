//! A new building: BUILD's foundation and the building constructor, with its combat setup,
//! its storage and its annexes.

use crate::construction::build::Construction;
use crate::lifecycle::{Class, Lifecycle};
use crate::masters::Master;
use crate::math::rng::Stream;
use crate::movement::Mover;
use crate::turn::unit::Unit;
use crate::turn::world::count_class;
use crate::turn::{BUILDING_COST, BUILDING_KIND, FARM, GATE, Gate, PASTURE_1889, Turns};
use crate::unit_ai::{GenieError, UnitAi};

impl Turns {
    /// The new foundation of BUILD and of `BUILD_WALL`'s tiles (the master's slot 0x108,
    /// [`Turns::make_building`]), then the player pays the master's cost (each entry flagged
    /// paid). Its id and position.
    pub(crate) fn found(
        &mut self,
        player: usize,
        master: i16,
        unit_ai: Result<Option<u32>, GenieError>,
        position: [f32; 3],
    ) -> Result<(i32, [f32; 3]), GenieError> {
        let made = self.make_building(player, master, unit_ai, position)?;
        let cost = self.master_of(player, master)?.cost.clone();
        let owner = &mut self.players[player];
        for &(resource, amount, paid) in &cost {
            let slot = usize::try_from(resource)
                .ok()
                .and_then(|r| owner.resources.get_mut(r))
                .ok_or(GenieError::unsupported(
                    "a cost in a resource other than the four",
                ))?;
            if paid {
                *slot -= f32::from(amount);
            }
        }
        Ok(made)
    }

    /// [`Turns::make_building`]'s combat setup (`FUN_14121df60`): its class counters
    /// (`FUN_141226060`), its powerup offset (`+0xa54`, the Object stream's "setup" draw) and,
    /// for a building with a train location (an entry of `+0x898` other than −1), its paid cost
    /// in attribute 247 (slot 0xf8).
    pub(super) fn building_combat_setup(
        &mut self,
        player: usize,
        master: i16,
    ) -> Result<(), GenieError> {
        if let Some(m) = self.masters.get(player).and_then(|m| m.get(master)) {
            count_class(&mut self.players[player].attributes, m, 1.0);
        }
        let _ = self.streams.get(Stream::Object).rand15();
        if let Some(m) = self
            .masters
            .get(player)
            .and_then(|m| m.get(master))
            .filter(|m| m.kind == BUILDING_KIND && m.trained_at.iter().any(|&t| t != -1))
        {
            let paid: i32 = m
                .cost
                .iter()
                .filter(|&&(resource, _, paid)| 0 <= resource && paid)
                .map(|&(_, amount, _)| i32::from(amount))
                .sum();
            self.players[player].add_attribute(BUILDING_COST, paid as f32)?;
        }
        Ok(())
    }

    /// [`Turns::make_building`]'s static init (`FUN_1411a45a0`): the storage entries of flags 1, 2
    /// and 32 go to the owner (`FUN_141193860`); returns the last entry of flag 0, which the
    /// building holds (`FUN_141193710`; none: −1 and 0).
    pub(super) fn building_storage(
        &mut self,
        player: usize,
        master: i16,
    ) -> Result<(i16, f32), GenieError> {
        let storage = self
            .masters
            .get(player)
            .and_then(|m| m.get(master))
            .map(|m| m.storage.clone())
            .unwrap_or_default();
        let mut held = (-1, 0.0);
        let owner = &mut self.players[player];
        for &(kind, amount, flag) in &storage {
            if flag == 0 && kind != -1 {
                held = (kind, amount);
            }
            if 0 <= kind && matches!(flag, 1 | 2 | 32) {
                owner.add_attribute(kind, amount)?;
            }
        }
        Ok(held)
    }

    /// A new building (the master's slot 0x40 or 0x108, `FUN_1411ac5c0` / `FUN_1411aca00` with
    /// their init: the static, animated, moving and combat setups, then the constructor
    /// `FUN_1411de5f0`) of `master` for `player` at `position`, the ground's height: its id
    /// (`mNextObjectId`), its unit AI of the given type if its class gets one, the setups' draws
    /// with its master's standing sprite current ([`Turns::setup_draws`]: the angle it faces), its
    /// movement state ([`Mover::new_object`]), the combat setup's class counters
    /// ([`count_class`]), its powerup offset (the Object stream's "setup" draw)
    /// and a building with a train location its paid cost (attribute 247); the static init's
    /// storage; its place and sight (its place links its tiles to its anchor, `FUN_1411e6780`).
    /// The constructor: masters 50 and 1889 on snow keep their snow (`+0xc61` 2); a master
    /// neither of classes 51, 54 nor 357, 1188, nor an annex (`+0x9b8` −1) takes its build time
    /// as a foundation (a build time above 0: hit points 1, a farm's terrain,
    /// [`Turns::construction_terrain`]; build points 0), the others are built already (build
    /// points their train time, their master's hit points); one with build points 0 and a
    /// build time above 0, not of classes 51 or 54, turns desolid (`FUN_1411cf770`: no record,
    /// its tiles unlinked); its footprint is cleared ([`Turns::clear_footprint`]); each annex
    /// slot (`+0x990`) of a master the player has makes its annex the same way at the slot's
    /// offset (`+0x998`, `+0x9a8`), depth first, linked to the building (`+0xbb8`), out of its
    /// owner's watched list unless the building is a gate (39), desolid when its master is tall,
    /// its own footprint cleared; `+0xcd0` notes mangrove or rice terrain under it
    /// (`FUN_141188db0`); last its owner watches it (`FUN_1411cc980`, [`Turns::watch_started`]).
    /// Not held: the construction sprite and its facets, the world's `+0x154`, slot 0x840's
    /// `+0xba0` and `+0xbf8`, and the wonder's `FUN_1411665c0`.
    ///
    /// # Errors
    ///
    /// A master the player has not got or without its building part, an annex of another type,
    /// and what the clearing and the attributes do not model.
    #[track_caller]
    pub(crate) fn make_building(
        &mut self,
        player: usize,
        master: i16,
        unit_ai: Result<Option<u32>, GenieError>,
        position: [f32; 3],
    ) -> Result<(i32, [f32; 3]), GenieError> {
        let made = self.master_of(player, master)?.clone();
        let building = made.building.ok_or(GenieError::unsupported(
            "a building constructor of a master without its building part",
        ))?;
        let given_z = position[2];
        // the new object's new_position: the ground's height (slot 0x60)
        let position = self.on_ground(player, master, position);
        let id = self.take_object_id(player, master);
        // what its first update as a built building starts (slot 0x7f0)
        let (default_make, default_task) = self
            .masters
            .get(player)
            .and_then(|m| m.get(master))
            .map_or((false, None), Master::default_start);
        let ai = unit_ai
            .and_then(|t| Ok((t, self.new_object(player, master, &made, position)?)))
            .map(|(t, new)| {
                t.and_then(|t| UnitAi::constructed(t, &new, self.streams.get(Stream::Ai)))
            });
        let ai = ai.unwrap_or_else(|e| {
            // the constructor's draws are unknown, so the turn player's are too
            crate::trace::player_unsupported(self.ticks, self.player_turn, &e);
            let turn = &mut self.players[self.player_turn];
            let at = turn.objects.len();
            let _ = turn.unsupported.get_or_insert((self.ticks, at, e));
            turn.stopped = true;
            None
        });
        // the setups the combat setup runs first, with its master's standing sprite current
        // (slot 0x798, FUN_1412029f0): their draws, the sprite and the angle it faces
        let standing = (0 <= made.standing_sprite)
            .then(|| self.graphics.get(&made.standing_sprite).copied())
            .flatten();
        let (mut sprite, angle) = standing.map_or((None, 0.0), |g| {
            let (s, _, angle) = self.setup_draws(g);
            (Some(s), angle)
        });
        self.building_combat_setup(player, master)?;
        let held = self.building_storage(player, master)?;
        // what its death reads of the master: its dying sprite, death spawns and decay
        let death = self.masters.get(player).and_then(|m| m.get(master)).map_or(
            (None, false, -1, None),
            |m| {
                (
                    (0 <= m.death_sprite).then_some(m.death_sprite),
                    0 <= m.death_spawns[0],
                    m.death_spawns[1],
                    m.life,
                )
            },
        );
        // the moving setup (FUN_1411580a0) of every building, a moving object though it never
        // moves (a mule cart does): the walk reads its last move time
        let mover = self
            .masters
            .get(player)
            .and_then(|m| m.get(master))
            .and_then(|m| m.moving)
            .map(|stats| Mover::new_object(stats, position, self.world_time, angle, None));
        // the constructor (FUN_1411de5f0)
        let [x, y, _] = position;
        let (tx, ty) = (
            crate::checkpoint::cvttss2si(x),
            crate::checkpoint::cvttss2si(y),
        );
        let terrain = self.spatial.terrain.as_ref();
        // masters 50 and 1889 on snow keep their snow (+0xc61 2)
        let snow = matches!(master, FARM | PASTURE_1889)
            && -1 < tx
            && -1 < ty
            && terrain
                .map(|t| t.snow_under(tx, ty))
                .transpose()?
                .unwrap_or(false);
        // +0xcd0 (FUN_141188db0): mangrove or rice terrain under its place
        let rice = terrain.is_some_and(|t| matches!(t.kind(tx, ty), Some(54 | 55 | 63..=67)));
        let foundation = !matches!(made.class, 51 | 54)
            && !matches!(master, 357 | 1188)
            && building.head_unit == -1;
        let hp = if foundation && 0 < made.build_time {
            1.0
        } else {
            f32::from(made.hit_points)
        };
        // a foundation shows its master's construction sprite (+0x950; its standing one for
        // none, FUN_141158620) through slot 0x68 (FUN_141217000)
        if foundation && 0 < made.build_time {
            let shown = (0 <= building.construction_sprite)
                .then(|| self.graphics.get(&building.construction_sprite).copied())
                .flatten()
                .or(standing);
            if let Some(g) = shown {
                crate::combat::attack::set_sprite(&mut sprite, g, self.streams.get(Stream::Sprite));
            }
        }
        let progress = if foundation {
            0.0
        } else {
            f32::from(made.build_time)
        };
        // FUN_1411cf770 with no master copy of its own and no annex link yet
        let desolid = 0 < made.build_time && !matches!(made.class, 51 | 54) && progress == 0.0;
        let owner = &mut self.players[player];
        owner.objects.push(Unit {
            id,
            footprint_waits: 0,
            position,
            life: Lifecycle {
                class: Class::Animated,
                state: 0,
                hp,
                master_id: master,
                attribute_type: held.0,
                amount: held.1,
                dying_sprite: death.0,
                death_spawn: death.1,
                remains: death.2,
                master: death.3,
                ..Lifecycle::default()
            },
            ai,
            artifact: None,
            unmodelled_action: None,
            production: None,
            gather_point: None,
            build: None,
            repair: None,
            builders: 0,
            mover,
            move_to: None,
            fly: None,
            construction: Some(Construction {
                progress,
                desolid,
                builders: 0,
                hit_points: made.hit_points,
                build_time: made.build_time,
                record: crate::map::spatial::Obstruction {
                    kind: made.obstruction_type,
                    class: made.obstruction_class,
                },
                snow,
                sprite_override: false,
                started: false,
            }),
            work_rate_powerup: 0.0,
            attackable: true,
            untargetable: false,
            under_attack: false,
            last_human_order: 0,
            town_bell: crate::town_bell::TownBell::default(),
            pasture: -1,
            my_pasture: -1,
            undeletable: false,
            capture_flag: 0,
            annex_of: None,
            group: crate::group::GroupLink::default(),
            hp_powerups: [0.0; 2],
            regen_timer: 0.0,
            regen_powerups: [0.0; 2],
            hunt: None,
            gather: None,
            attack: None,
            explore: None,
            enter: None,
            transport: None,
            missile: None,
            attack_count: 0,
            // the TRIBE combat setup (FUN_14121df60): the master's volley fire
            firepower: self
                .masters
                .get(player)
                .and_then(|m| m.get(master))
                .map_or(0.0, |m| m.volley_fire),
            next_volley: 0,
            reload_worldtime: 0,
            garrison: Vec::new(),
            garrison_time: 0,
            relics: [0; 2],
            damaged_lately: 0,
            livestock_summation: 0.0,
            livestock: Vec::new(),
            pasture_workers: Vec::new(),
            build_frame: 0,
            raider_countdown: 0,
            // every building holds a gate's lock, open
            gate: Some(Gate {
                first_update: true,
                ..Gate::default()
            }),
            workers: 0,
            combat: true,
            default_task,
            default_fly: None,
            default_make,
            default_artifact: false,
            make: None,
            wait_solidify: None,
            stopped: false,
            sprite,
            // the animated object's constructor (FUN_141070cf0): animation speed (+0x7b8) 0
            anim_speed: Some(0.0),
            attack_timer: 0.0,
            abilities: crate::combat::abilities::Abilities::default(),
            reload_stinger: 0.0,
            reload_powerup: 0.0,
            attack_powerup: 0.0,
            range_powerup: 0.0,
            armor_powerups: [0; 2],
            armor_removed: [0; 2],
            charge: 0.0,
            charge_hit: false,
            idle_variant: 0,
            roo_called: false,
            rice,
            store: Vec::new(),
            own_master: None,
            dead_master: None,
            stack_made: None,
            attacking_units: Vec::new(),
        });
        // the static init's add to its player (slot 0x2d0): it waits in the tracker's pending
        // lists
        self.track_add(player, id, master);
        self.building_place(player, id, &made, position, desolid);
        if foundation && 0 < made.build_time {
            self.construction_terrain(player, master, position)?;
        }
        self.clear_footprint(player, master, [position[0], position[1]], false)?;
        for (slot, &annex) in building.annexes.iter().enumerate() {
            if annex == -1 {
                continue;
            }
            // a master the player has not got makes nothing (its slot 0x40 is not called)
            let Some(am) = self.masters.get(player).and_then(|m| m.get(annex)).cloned() else {
                continue;
            };
            // the .dat's annexes are buildings (type 80): another type's master has no building
            // part, which the constructor below refuses
            let [ox, oy] = building.annex_offsets[slot];
            let ai = Ok(crate::unit_ai::factory_type(
                am.class,
                annex,
                am.volley_missile.unwrap_or(-1),
                am.combat_level,
            ));
            let at = [position[0] + ox, position[1] + oy, given_z];
            let (aid, at) = self.make_building(player, annex, ai, at)?;
            self.link_annex(player, id, aid, &made, &am)?;
            self.clear_footprint(player, annex, [at[0], at[1]], false)?;
        }
        self.watch_started(player, id)?;
        Ok((id, position))
    }

    /// The building constructor's link of a new annex `aid` (master `am`) to its building `id`
    /// (master `made`): the building's slot and the annex's `+0xbb8`; its owner's watched list
    /// lets the annex go (`FUN_141091310`) unless the building is a gate (39); a tall annex
    /// master makes it desolid (`FUN_1411cf770`, which an annex's link lets through: its record
    /// and mesh shape go, its own master copy has no height, the watched list lets it go; its
    /// tiles keep their links, as an annex's do).
    pub(in crate::turn) fn link_annex(
        &mut self,
        player: usize,
        id: i32,
        aid: i32,
        made: &Master,
        am: &Master,
    ) -> Result<(), GenieError> {
        let not_held = crate::construction::build::NOT_HELD;
        let (_, a) = self.object_mut(aid).ok_or_else(|| not_held.clone())?;
        a.annex_of = Some(id);
        self.spatial.set_annex(aid, Some(id));
        if made.class != GATE {
            self.unwatch(player, aid);
        }
        if 0.0 < am.height {
            let (_, a) = self.object_mut(aid).ok_or_else(|| not_held.clone())?;
            let c = a
                .construction
                .as_mut()
                .ok_or(crate::construction::build::NO_CONSTRUCTION)?;
            if !c.desolid {
                c.desolid = true;
                self.pathing.removed(aid);
                self.spatial.drop_record(aid);
                self.spatial.set_tall(aid, false);
                self.pathing.tiles_changed(self.spatial.take_changes());
                self.unwatch(player, aid);
            }
        }
        Ok(())
    }
}
