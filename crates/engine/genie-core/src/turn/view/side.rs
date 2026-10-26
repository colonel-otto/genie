//! What the unit AI asks of the world through its view: [`crate::unit_ai::UnitSide`] for
//! [`UnitView`].

use crate::movement::{Me, Planning, footprint_gap_squared};
use crate::rng::Well512;
use crate::spatial::Placed;
use crate::tasks::Actor;
use crate::turn::PETARD;
use crate::turn::view::{
    AiStart, HuntPlan, RepairOrder, UnitView, Wait, WorldPicker, patrol_information, town_centre,
};
use crate::turn::world::{
    Releaser, ResourceWorld, SideWorld, closest_resource_of, herded, task_search, task_target_with,
};
use crate::unit_ai::{AiMove, GenieError, Search, UnitAi};

impl crate::unit_ai::UnitSide for UnitView<'_> {
    fn table_drop_site(&mut self) -> Result<Option<i32>, GenieError> {
        let class = self
            .masters
            .and_then(|m| m.get(self.unit.life.master_id))
            .map(|m| m.class)
            .ok_or(crate::turn::OBJECT_MASTER)?;
        let at = [self.unit.position[0], self.unit.position[1]];
        let look = |id: i32| {
            self.others
                .iter()
                .find(|u| u.id == id)
                .map(|u| (u.life.state, u.position))
        };
        Ok(crate::order::nearest_table_site(
            (self.drop_sites, self.civ, self.tracker),
            (at, self.unit.life.attribute_type, class),
            look,
        ))
    }

    fn patrol_information(&self) -> Option<(crate::unit_ai::PatrolPath, i32)> {
        patrol_information(self.unit.mover.as_ref())
    }

    /// Slot 0x278 (`FUN_141158720`, [`Spatial::point_clear`]): the unit, of its radius and its
    /// restriction's terrain classes, could stand at `point`.
    ///
    /// [`Spatial::point_clear`]: crate::spatial::Spatial::point_clear
    fn can_stand(&mut self, point: [f32; 2]) -> Result<bool, GenieError> {
        let unit = &*self.unit;
        let mover = unit.mover.as_ref().ok_or(GenieError::unsupported(
            "a stand test of a unit without movement state",
        ))?;
        self.spatial.point_clear(
            unit.id,
            point,
            mover.stats.radius[0],
            mover.stats.restriction_mask,
        )
    }

    /// The base handler's case 0x216: the unit's closest spot beside the foundation `site`
    /// ([`Spatial::spot_off`] with the notify's radii; none without the foundation), its height
    /// and class, its owner, the foundation and its current action's target as orders name them.
    ///
    /// [`Spatial::spot_off`]: crate::spatial::Spatial::spot_off
    fn off_footprint(
        &mut self,
        site: i32,
        radius: [f32; 2],
    ) -> Result<crate::unit_ai::OffFootprint, GenieError> {
        let unit = &*self.unit;
        let master = self
            .masters
            .and_then(|m| m.get(unit.life.master_id))
            .ok_or(GenieError::unsupported(
                "a unit sent off of a master the slice does not have",
            ))?;
        let (player, others, foreign) = (self.player, self.others, &self.foreign);
        let named = |id: i32| {
            let found = if id == unit.id {
                Some((player, unit))
            } else {
                others
                    .iter()
                    .find(|u| u.id == id)
                    .map(|u| (player, u))
                    .or_else(|| foreign.find(id))
            };
            found.map(|(p, u)| crate::unit_ai::Named {
                id,
                master: i32::from(u.life.master_id),
                owner: p as i32,
                vector: [u.position[0], u.position[2]],
            })
        };
        let spot = match self.spatial.get(site) {
            Some(placed) => {
                let mover = unit.mover.as_ref().ok_or(GenieError::unsupported(
                    "a unit sent off without movement state",
                ))?;
                self.spatial.spot_off(
                    unit.position,
                    (unit.id, mover.stats.radius[0]),
                    mover.stats.restriction_mask,
                    (placed.position, radius),
                    false,
                )?
            }
            None => None,
        };
        let target = unit.top_target().filter(|&t| t != -1);
        Ok(crate::unit_ai::OffFootprint {
            spot,
            z: unit.position[2],
            class: master.class,
            computer: self.computer,
            player: player as i32,
            site: (site != -1).then(|| named(site)).flatten(),
            action_target: target.and_then(named),
        })
    }

    /// `FUN_140c1c7c0`: the flood ([`crate::unit_ai::flood_search`]) from the unit's tile over the
    /// larger of its search radius (slot 0x2c8) and its line of sight, truncated, testing each tile
    /// it reaches (`FUN_140c2b950`: the first object of its tile's list, in order, of type 80 or
    /// more in state 0 of the unit's player); −1 for none.
    fn foundation_search(&mut self) -> Result<i32, GenieError> {
        let unit = &*self.unit;
        let master = self
            .masters
            .and_then(|m| m.get(unit.life.master_id))
            .ok_or(GenieError::unsupported(
                "a builder of a master the slice does not have",
            ))?;
        let r = if master.search_radius <= master.line_of_sight {
            master.line_of_sight
        } else {
            master.search_radius
        } as i32;
        let (ux, uy) = (unit.position[0] as i32, unit.position[1] as i32);
        let (player, others, masters, spatial) =
            (self.player, self.others, self.masters, &*self.spatial);
        let foundation_on = |x: i32, y: i32| -> Option<i32> {
            spatial.objects_on(x, y).find_map(|(id, p)| {
                if p.player != player {
                    return None;
                }
                let u = others.iter().find(|u| u.id == id)?;
                let kind = masters.and_then(|m| m.get(u.life.master_id))?.kind;
                (80 <= kind && u.life.state == 0).then_some(id)
            })
        };
        Ok(crate::unit_ai::flood_search((ux, uy), r, foundation_on).unwrap_or(-1))
    }

    fn call_for_help(&mut self, attacker: i32) -> Result<(), GenieError> {
        /// The classes that never help: 1 to 3, 18 to 21, 27 and 39.
        const NO_HELP_CLASSES: u64 = 0x0080_083c_000e;
        /// The siege tower master (`FUN_141194f10`'s 0x451).
        const SIEGE_TOWER: i16 = 0x451;
        /// `AskForHelp`'s reach (`DAT_142ecccc4`): the squared footprint gap of 3 tiles.
        const HELP_REACH: f32 = crate::image::unit_ai::HELP_REACH;
        let lists = self.spatial.searched().ok_or(GenieError::unsupported(
            "a search that reads the registry's last answer (before any query)",
        ))?;
        let (others, foreign, player) = (self.others, self.foreign, self.player);
        let find = |id: i32| {
            others
                .iter()
                .find(|u| u.id == id)
                .map(|u| (player, u))
                .or_else(|| foreign.find(id))
        };
        let gaia_attacker = find(attacker).is_some_and(|(owner, _)| owner == 0);
        let me = self.unit.id;
        let my_radius = self
            .masters
            .and_then(|ms| ms.get(self.unit.life.master_id))
            .map(|m| m.radius)
            .ok_or(crate::turn::UNIT_AI_MASTER)?;
        for r in lists[1].iter().chain(&lists[2]) {
            if r.id == me {
                continue;
            }
            let Some((owner, u)) = find(r.id) else {
                continue;
            };
            if u.ai.is_none() || 3 <= u.life.state {
                continue;
            }
            let m = self
                .every
                .get(owner)
                .and_then(|ms| ms.get(u.life.master_id))
                .ok_or(GenieError::unsupported(
                    "a helper of a master the slice does not have",
                ))?;
            let class = i64::from(m.class);
            let no_help = (0..=0x27).contains(&class) && (NO_HELP_CLASSES >> class) & 1 != 0;
            if no_help || m.id == SIEGE_TOWER {
                continue;
            }
            if !gaia_attacker && m.class == 4 {
                continue;
            }
            if owner != self.player && !self.spatial.allied_with(owner, self.player) {
                continue;
            }
            let gap = footprint_gap_squared(self.unit.position, my_radius, u.position, m.radius);
            if HELP_REACH < gap {
                continue;
            }
            if !self.helper_free(owner, u, m, attacker)? {
                continue;
            }
            return crate::unit_ai::helper_answers();
        }
        Ok(())
    }

    fn duel(&mut self, other: i32) -> Result<crate::unit_ai::Duel, GenieError> {
        self.picker(None).duel(other)
    }

    fn harmless(&mut self, attacker: i32) -> Result<bool, GenieError> {
        self.picker(None).harmless(attacker)
    }

    fn defended_attacker(&mut self, target: i32) -> Result<Option<i32>, GenieError> {
        Ok(self.picker(None).nearest_attacker_of(target))
    }

    fn seen_unit(&mut self, id: i32) -> Result<Option<crate::unit_ai::UnitSeen>, GenieError> {
        let Some(p) = self.spatial.get(id) else {
            return Ok(None);
        };
        let (owner, u) = self
            .others
            .iter()
            .find(|u| u.id == id)
            .map(|u| (self.player, u))
            .or_else(|| self.foreign.find(id))
            .ok_or(GenieError::unsupported("an object the slice does not hold"))?;
        let m = self
            .every
            .get(owner)
            .and_then(|ms| ms.get(u.life.master_id))
            .ok_or(crate::turn::OBJECT_MASTER)?;
        let me = self
            .masters
            .and_then(|ms| ms.get(self.unit.life.master_id))
            .ok_or(crate::turn::UNIT_AI_MASTER)?;
        Ok(Some(crate::unit_ai::UnitSeen {
            id,
            class: m.class,
            owner,
            combat_level: m.combat_level,
            state: u.life.state,
            target: u.ai.as_ref().map(|a| a.current_target),
            gap_squared: footprint_gap_squared(self.unit.position, me.radius, p.position, p.radius),
            position: p.position,
            hp: u.life.hp,
            master: u.life.master_id,
            kind: m.kind,
            main: u.annex_of,
            allied: crate::tasks::allied(self.relations, self.player, owner, true),
        }))
    }

    fn closest_resource(
        &mut self,
        search: crate::unit_ai::ResourceSearch,
    ) -> Result<Option<crate::unit_ai::Gatherable>, GenieError> {
        let mut w = ResourceWorld {
            player: self.player,
            spatial: self.spatial,
            zones: self.zones,
            foreign: self.foreign,
            every: self.every,
            masters: self.masters,
            plan: Planning {
                pathing: self.pathing,
                now: self.world_time,
                computer: self.computer,
                object: self.object,
                construction_rate: self.construction_rate,
                groups: self.groups,
            },
        };
        let (id, _) = closest_resource_of(
            self.unit,
            &mut w,
            self.others,
            (search.kind, search.cap, search.site),
            (search.current, search.min_state, search.point),
        )?;
        if id == -1 {
            return Ok(None);
        }
        let (owner, u) = self
            .others
            .iter()
            .find(|u| u.id == id)
            .map(|u| (self.player, u))
            .or_else(|| self.foreign.find(id))
            .ok_or(GenieError::unsupported(
                "a resource found that the slice does not hold",
            ))?;
        let class = self
            .every
            .get(owner)
            .and_then(|m| m.get(u.life.master_id))
            .ok_or(GenieError::unsupported(
                "a resource found of a master the slice lacks",
            ))?
            .class;
        Ok(Some(crate::unit_ai::Gatherable {
            id,
            position: u.position,
            class,
        }))
    }

    /// The player's master table (`FUN_14152fe40`, `+0x28`): the masters the slice holds of each
    /// player.
    fn master_class(&mut self, player: i32, master: i32) -> Result<Option<i16>, GenieError> {
        let class = usize::try_from(player)
            .ok()
            .and_then(|p| self.every.get(p))
            .zip(i16::try_from(master).ok())
            .and_then(|(ms, id)| ms.get(id))
            .map(|m| m.class);
        Ok(class)
    }

    fn reaches(&mut self, target: i32, range: f32) -> Result<bool, GenieError> {
        let mut plan = Planning {
            pathing: self.pathing,
            now: self.world_time,
            computer: self.computer,
            object: self.object,
            construction_rate: self.construction_rate,
            groups: self.groups,
        };
        let unit = &*self.unit;
        let walker = Me {
            id: unit.id,
            position: unit.position,
            group: unit.group.id,
            action_state: unit.top_action().map_or(0, |a| a.state),
        };
        let mover = unit.mover.as_ref().ok_or(GenieError::unsupported(
            "a reach test of a unit that does not move",
        ))?;
        mover.reaches_object(
            &walker,
            self.spatial,
            &mut plan,
            target,
            range,
            unit.life.master_id,
        )
    }

    fn closest(&mut self, search: &Search<'_>) -> Result<crate::unit_ai::Found, GenieError> {
        let seeker = match search.pick {
            crate::unit_ai::Pick::Attack { seeker, .. } => Some(seeker),
            _ => None,
        };
        let mut picker = WorldPicker {
            attacker: &self.attacker,
            seeker,
            me: self.unit.id,
            player: self.player,
            others: self.others,
            foreign: self.foreign,
            spatial: self.spatial,
            every: self.every,
            restriction: self
                .masters
                .and_then(|m| m.get(self.unit.life.master_id))
                .map_or(-1, |m| m.terrain_restriction),
            unit: &*self.unit,
            attributes: self.attributes,
            relations: self.relations,
            ai_settings: self.ai_settings,
            now: self.world_time,
            mode_680: self.mode_680,
        };
        let (others, foreign, every) = (self.others, self.foreign, self.every);
        // ClosestObject's reachability check (slot 0x3d0) walks the unit's paths
        let mut plan = Planning {
            pathing: self.pathing,
            now: self.world_time,
            computer: self.computer,
            object: self.object,
            construction_rate: self.construction_rate,
            groups: self.groups,
        };
        let unit = &*self.unit;
        let walker = Me {
            id: unit.id,
            position: unit.position,
            group: unit.group.id,
            action_state: unit.top_action().map_or(0, |a| a.state),
        };
        let (mover, master_id, spatial) =
            (unit.mover.as_ref(), unit.life.master_id, &*self.spatial);
        let mut reach = |id: i32, range: f32| {
            let m = mover.ok_or(GenieError::unsupported(
                "a reach test of a unit that does not move",
            ))?;
            m.reaches_object(&walker, spatial, &mut plan, id, range, master_id)
        };
        let moving = |id: i32| herded(others, foreign, id);
        // a class-10 searcher passes master 440 and the masters with object flag 0x10
        let shuns = |p: &Placed| {
            let m = every.get(p.player).and_then(|ms| ms.get(p.master)).ok_or(
                GenieError::unsupported("a found object of a master the slice does not have"),
            )?;
            Ok(p.master == PETARD || m.object_flags & 0x10 != 0)
        };
        // a farm's workers (`+0x644`) and its pasture (`+0x790`)
        let farm = |id: i32| {
            others
                .iter()
                .find(|u| u.id == id)
                .or_else(|| foreign.find(id).map(|(_, u)| u))
                .map(|u| (u.workers, u.pasture))
        };
        spatial.search(
            unit.id,
            self.player,
            search,
            &mut picker,
            &mut reach,
            &moving,
            &shuns,
            &farm,
        )
    }

    fn may_attack(
        &mut self,
        target: i32,
        seeker: &crate::unit_ai::Seeker,
    ) -> Result<Option<(i16, [f32; 3])>, GenieError> {
        let picker = self.picker(Some(*seeker));
        let held =
            self.foreign.find(target).is_some() || self.others.iter().any(|u| u.id == target);
        if !held {
            return Ok(None);
        }
        let (t, _) = picker.target(target)?;
        Ok(
            crate::unit_ai::can_attack(&self.attacker, seeker, &t)?
                .then_some((t.class, t.position)),
        )
    }

    /// The attack command (slot 0x338, `FUN_14120cea0`; classes 51 and 54 are not modelled) ->
    /// `FUN_141081a80`: a target the world does not hold fails; `+0x648` takes the world's time
    /// (not held); the unit's own Attack on the target answers true, forced after its slot 0x48
    /// on it ([`UnitView::attack_again`]); a villager's master switch (slot 0x90) is not modelled.
    /// Then the new
    /// Attack ([`crate::attack::Attack::command`]): the constructor takes its target (slot 0x178,
    /// the caller's) and clears the unit's `+0x900` (its target is new), the action list clears
    /// (the old actions' teardowns), and the install's start (`0x140a7f8c0`) of a unit inside
    /// another fails it at once (`set_state(13)`, [`crate::attack::Attack::fail_at_start`]); else
    /// it puts the unit on its standing sprite (slot 0x68 with `+0x80`) and runs the range test
    /// ([`crate::attack::Attack::begin`]): readyToAttack's failure
    /// before a `set_state(4)` leaves the animation at speed 1; a `MoveTo` that starts arrived
    /// shows the standing sprite; a unit that cannot move in reach goes ready (`set_state(6)`,
    /// [`crate::attack::Attack::enter_ready`]: readyToAttack again, then the swing, `set_state(7)`,
    /// when it faces its target and may attack), whose notifies join the start's. A notify posted
    /// while the AI runs (its buffer, `FUN_140c55270`) is not modelled.
    fn attack(
        &mut self,
        target: i32,
        force: bool,
        ai: &UnitAi,
    ) -> Result<crate::unit_ai::Attacked, GenieError> {
        let unit = &mut *self.unit;
        let master = self
            .masters
            .and_then(|m| m.get(unit.life.master_id))
            .ok_or(crate::turn::ATTACKER_MASTER)?;
        if matches!(master.class, 51 | 54) {
            return Err(GenieError::unsupported(
                "the attack command of a unit of class 51 or 54",
            ));
        }
        let no = crate::unit_ai::Attacked::default;
        let Some(t) = self.spatial.get_known(target)?.copied() else {
            return Ok(no());
        };
        if unit.attack.is_some_and(|a| a.target == target) {
            if force {
                self.attack_again(target, &t, master, ai)?;
            }
            return Ok(crate::unit_ai::Attacked {
                attacks: true,
                posted: Vec::new(),
            });
        }
        // a villager switches to its player's villager of its task swap group (`+0x7b8` 1: the
        // master table's `+0x298`, 83; else `+0x928`, 293) when the player has it (slot 0x90 with
        // 0: the action list clears, then the master's unit part; its owner's part after the
        // update)
        let master = match self.villager_swap(master) {
            Some(next) => {
                self.switch_unit_master(master, next)?;
                next
            }
            None => master,
        };
        let me_at = *self
            .spatial
            .get(self.unit.id)
            .ok_or(crate::turn::ATTACKER_UNPLACED)?;
        let t_master = self
            .every
            .get(t.player)
            .and_then(|m| m.get(t.master))
            .ok_or(GenieError::unsupported(
                "an attack's target of a master the slice does not have",
            ))?;
        let visible = self.target_visible(&t, t_master)?;
        let unit = &mut *self.unit;
        // the attack command's Attack (`FUN_141081a80`) takes no MoveTo sprite: the unit's
        let look = unit.abilities.look.as_ref();
        let arms = crate::attack::arms_looking(master, look, self.graphics, [-1; 4], -1);
        let standing = arms.idle_sprite.ok_or(crate::turn::NO_STANDING_SPRITE)?;
        if unit.sprite.is_none() {
            return Err(GenieError::unsupported(
                "an attack command of a unit whose sprite the slice does not hold",
            ));
        }
        // the action list's clearing (FUN_140a98620): an Attack on another target tears down (its
        // MoveTo's stops the unit) and lets its targets go (slot 0x180), as do the others
        let by = Releaser {
            id: unit.id,
            class: master.class,
            player: self.player,
        };
        unit.tear_down_actions(self.releases, self.builder_leaves, by)?;
        let speed = unit
            .mover
            .as_ref()
            .ok_or(crate::turn::ATTACKER_UNMOVING)?
            .stats
            .speed;
        let e = crate::attack::engagement(Some(ai), speed, master, &me_at, &t, self.world_time)?;
        // the constructor clears +0x900 for a target other than its own (none yet)
        unit.attack_count = 0;
        if unit.life.inside.is_some() {
            let mut attack = crate::attack::Attack::own(target, &e, arms);
            attack.fail_at_start(&mut unit.sprite, &mut unit.anim_speed, self.sprite)?;
            unit.attack = Some(attack);
            self.takes.push((target, by));
            return Ok(crate::unit_ai::Attacked {
                attacks: true,
                posted: Vec::new(),
            });
        }
        crate::attack::set_sprite(&mut unit.sprite, standing, self.sprite);
        let mover = unit.mover.as_mut().expect("its speed read above");
        let crate::attack::Commanded {
            mut attack,
            arrived,
            mut posted,
            ready,
            failed,
            failed_first,
        } = crate::attack::Attack::command(
            (target, self.spatial),
            &e,
            arms,
            (visible, t.player),
            mover,
        )?;
        if failed_first {
            unit.anim_speed = Some(1.0);
        }
        if failed {
            attack.fail_at_start(&mut unit.sprite, &mut unit.anim_speed, self.sprite)?;
        }
        if ready {
            posted.extend(self.ready_at_start(&mut attack, master, ai)?);
        }
        let unit = &mut *self.unit;
        let mover = unit.mover.as_mut().expect("its speed read above");
        if arrived {
            // the MoveTo's start in state 1 (`0x140aa29e0`, not quiet): the master's standing
            // sprite with its speed, `+0x7b8` cleared; the next update's state 4 ends the MoveTo
            mover.shows_standing();
            unit.shows_standing_sprite(self.masters, self.graphics, self.sprite)?;
        }
        unit.attack = Some(attack);
        self.takes.push((target, by));
        Ok(crate::unit_ai::Attacked {
            attacks: true,
            posted,
        })
    }

    fn set_under_attack(&mut self, value: bool) {
        self.unit.under_attack = value;
    }

    fn stop(&mut self) -> Result<(), GenieError> {
        let masters = self.masters;
        let by = Releaser {
            id: self.unit.id,
            class: masters
                .and_then(|m| m.get(self.unit.life.master_id))
                .map_or(-1, |m| m.class),
            player: self.player,
        };
        let lets = (&mut *self.releases, &mut *self.builder_leaves);
        self.unit
            .stop_with(lets, by, masters, self.graphics, self.sprite)
    }

    fn cap_human_order(&mut self, cap: u32) -> Result<(), GenieError> {
        self.unit.last_human_order = self.unit.last_human_order.min(cap);
        self.spatial.set_gait(self.unit.id, self.unit.gait());
        Ok(())
    }

    fn seek_unexplored(
        &mut self,
        ai: &UnitAi,
        rng: &mut Well512,
    ) -> Result<Option<[i32; 2]>, GenieError> {
        let tc = town_centre(self.others)?;
        let (at, computer) = (self.unit.position, self.computer);
        // the search about the town centre only while the player has no scout map
        let scout_map = self.spatial.scout_map(self.player).is_some();
        let (mut area, map, line_of_sight) = self.scout_area()?;
        if !scout_map && let Some(p) = crate::explore::unexplored_area(tc, map, &mut area)? {
            return Ok(Some(p));
        }
        // none about the town centre: the unit AI's own search (slot 0x90)
        let seeker = crate::explore::Seeker {
            at,
            line_of_sight,
            order: ai.current_order,
            target_point: [ai.target_point[0], ai.target_point[1]],
            computer,
        };
        crate::explore::closest_undiscovered(&mut area, seeker, map, rng)
    }

    fn seek_beyond_sight(&mut self, rng: &mut Well512) -> Result<Option<[i32; 2]>, GenieError> {
        let (at, player) = (self.unit.position, self.player);
        let scouted = self.spatial.scout_map(player).is_some();
        let (mut area, map, line_of_sight) = self.scout_area()?;
        area.scouted = scouted;
        let found = crate::explore::beyond_sight(&mut area, at, line_of_sight, map, rng)?;
        // the point found is marked in the scout map (FUN_140f4d5e0 with 1)
        if let Some([x, y]) = found {
            self.spatial.scout(player, x, y);
        }
        Ok(found)
    }

    /// The player's scout map (`FUN_140c589c0`'s case 0x202 when the auto scout found nothing):
    /// a new `InfluenceMap` of 480 by 480 (`FUN_140f4a240`, the old one goes, `+0x1368` 1) marks
    /// every tile the player sees now (its visible map's `FUN_141285350` 0xf) with 1.
    fn make_scout_map(&mut self) -> Result<(), GenieError> {
        let size = crate::explore::SCOUT_MAP_SIZE;
        let mut m = crate::explore::InfluenceMap::new([size, size], 0);
        let [w, h] = self.spatial.map;
        for y in 0..h {
            for x in 0..w {
                if self.spatial.scout_sight(self.player, x, y) == Some(0xf) {
                    m.set(x, y, 1);
                }
            }
        }
        self.spatial.set_scout_map(self.player, m);
        Ok(())
    }

    /// The unit's slot 0x388 (`FUN_141226fc0`) on the foundation `target`: the world's object
    /// (`FUN_1415417a0`; the owner's sleeping list, which the unit does not see here, holds
    /// only objects of a type below 30, never a building), then its slot 0x820 with any action
    /// type (`0xffff`) and the switch ([`task_search`] on the target and its main building);
    /// with a task the new wait replaces the unit's actions, which the caller makes after the
    /// update ([`Wait`], [`Turns::wait_for_foundation`]).
    fn command(&mut self, command: crate::unit_ai::UnitCommand) -> Result<bool, GenieError> {
        match command {
            crate::unit_ai::UnitCommand::Repair(target) => self.repair_command(target),
            crate::unit_ai::UnitCommand::Enter(target) => self.enter_command(target),
            // slot 0x3a8 (FUN_14108d050): its Transport, made after the update, answers 1
            crate::unit_ai::UnitCommand::Unload {
                to,
                over_wall: false,
            } => {
                self.starts.push(AiStart::Transport(to));
                Ok(true)
            }
            _ => Err(crate::unit_ai::UNMODELLED_COMMAND),
        }
    }

    /// The build command at once ([`Turns::build_command`]'s path without a Build on top, which
    /// a stop before it leaves none): slot 0x820 with any action type and the switch
    /// ([`task_search`] on the foundation and its main building) finds a task; the switch (slot
    /// 0x90 with 0: the actions go, then the master's unit part, [`crate::order::switch_master`];
    /// its owner's part after the update); a Build on the task (`FUN_140aad4e0`, the footprints'
    /// gap, the task's range and work value, the builder's work rate with the unit's powerup)
    /// takes the foundation (slot 0x178) and replaces the actions (slot 0x808). A Build on top
    /// (its slot 0x48) is not modelled here.
    fn build_now(&mut self, foundation: i32) -> Result<(), GenieError> {
        use crate::build::{BUILD, BuildAction, BuildTask};
        if self.unit.top_action_kind() == Some(BUILD) {
            return Err(crate::unit_ai::PENDING_BEFORE_ATTACK);
        }
        let unit = &*self.unit;
        let (player, others, foreign) = (self.player, self.others, self.foreign);
        let find = |id: i32| {
            if id == unit.id {
                return Some((player, unit));
            }
            others
                .iter()
                .find(|u| u.id == id)
                .map(|u| (player, u))
                .or_else(|| foreign.find(id))
        };
        let (fp, f) = find(foundation).ok_or(crate::turn::command::FOUNDATION_GONE)?;
        let site = self
            .every
            .get(fp)
            .and_then(|m| m.get(f.life.master_id))
            .ok_or(crate::turn::OBJECT_MASTER)?;
        let f_at = f.position;
        let (Some(t), Some(general)) = (
            task_target_with(foundation, 1, &find, self.every)?,
            task_target_with(foundation, 2, &find, self.every)?,
        ) else {
            return Err(crate::turn::command::FOUNDATION_GONE);
        };
        let masters = self.masters.ok_or(crate::turn::UNIT_AI_MASTER)?;
        let own = masters
            .get(unit.life.master_id)
            .ok_or(crate::turn::UNIT_AI_MASTER)?;
        let actor = Actor {
            id: unit.id,
            master: own,
            player,
            held: (unit.life.attribute_type, unit.life.amount),
            attribute_95: self.attributes.get(95).copied().unwrap_or(0.0),
            civ: self.civ,
        };
        let dip = SideWorld {
            player,
            relations: self.relations,
            foreign,
            drop_sites: self.drop_sites,
        };
        let (task, switch) = task_search(masters, &actor, &t, &general, -1, &dip)?
            .ok_or(crate::turn::command::NO_BUILD_TASK)?;
        let builder = switch.unwrap_or(own);
        let gap = crate::movement::footprint_gap(unit.position, builder.radius, f_at, site.radius);
        let build = BuildAction::start(
            foundation,
            f_at,
            gap,
            BuildTask {
                range: task.work_range,
                work_value: task.work_val1,
                work_rate: builder.work_rate + unit.work_rate_powerup,
                resource: task.attribute_type,
                id: task.id,
            },
        );
        let id = unit.id;
        let unit = &mut *self.unit;
        if let Some(next) = switch {
            let by = Releaser {
                id,
                class: own.class,
                player,
            };
            unit.tear_down_actions(self.releases, self.builder_leaves, by)?;
            let world = crate::order::SwitchWorld {
                spatial: self.spatial,
                pathing: self.pathing,
                graphics: self.graphics,
                sprite: self.sprite,
            };
            let switched = crate::order::switch_master(unit, own, next, world)?;
            self.attribute_adds.extend(switched.leaving);
            if switched.promoted {
                self.promotions.push((id, own.id, next.id));
            }
            self.attribute_adds.extend(switched.coming);
        }
        let by = Releaser {
            id,
            class: builder.class,
            player,
        };
        self.takes.push((foundation, by));
        unit.tear_down_actions(self.releases, self.builder_leaves, by)?;
        unit.build = Some(build);
        Ok(())
    }

    /// `HuntObject`'s unit part at once ([`Turns::hunt_command`] in the view): the stopAction
    /// (the actions go, and in state 2 the master's standing sprite, slot 0x68), then the hunt
    /// command: slot 0x820's task with any filter and the switch ([`task_search`]; the switch,
    /// slot 0x90 with 0, [`crate::order::switch_master`], its owner's part after the update) and a
    /// Hunt on it ([`Turns::start_hunt`]'s: the target taken, slot 0x178; the actions cleared;
    /// [`crate::hunt::HuntAction::start`] with the target as it stands, its master among the
    /// unit's drop sites, the engagement and arms; an Attack that starts arrived shows the idle
    /// sprite; a load to its own drop site takes the target as the second and lets it go as the
    /// first).
    fn hunt_now(&mut self, target: i32, ai: &UnitAi) -> Result<(), GenieError> {
        use crate::hunt::{HuntAction, HuntTask};
        let player = self.player;
        let own_class = self
            .masters
            .and_then(|m| m.get(self.unit.life.master_id))
            .ok_or(crate::turn::HUNTER_MASTER)?
            .class;
        let by = Releaser {
            id: self.unit.id,
            class: own_class,
            player,
        };
        self.unit
            .tear_down_actions(self.releases, self.builder_leaves, by)?;
        if self.unit.life.state == 2 {
            self.unit
                .show_standing(self.masters, self.graphics, self.sprite)?;
        }
        let HuntPlan {
            task,
            switch,
            own,
            quarry,
            them,
        } = self.hunt_plan(target)?;
        let hunter = switch.unwrap_or(own);
        let id = self.unit.id;
        if let Some(next) = switch {
            self.switch_unit_master(own, next)?;
        }
        let unit = &mut *self.unit;
        let hunt_task = HuntTask {
            id: task.id,
            range: task.work_range,
            resource: task.attribute_type,
            sprites: task.sprites,
            work_value: task.work_val1,
            productivity: task.attribute_type2,
            kind4: task.attribute_type4,
        };
        let by = Releaser {
            id,
            class: hunter.class,
            player,
        };
        self.takes.push((target, by));
        unit.tear_down_actions(self.releases, self.builder_leaves, by)?;
        let me = *self.spatial.get(id).ok_or(crate::turn::ATTACKER_UNPLACED)?;
        let speed = unit.mover.as_ref().map_or(0.0, |m| m.stats.speed);
        let engagement =
            crate::attack::engagement(Some(ai), speed, hunter, &me, &them, self.world_time)?;
        let moving = hunt_task.walk_sprite_id(unit.life.attribute_type, unit.life.amount);
        let arms = crate::attack::arms(hunter, self.graphics, hunt_task.sprites, moving);
        let mover = unit.mover.as_mut().ok_or(crate::turn::HUNTER_UNMOVING)?;
        let mut held = (unit.life.attribute_type, unit.life.amount);
        let (hunt, arrived) = HuntAction::start(
            hunt_task,
            &quarry,
            player,
            self.computer,
            self.carry_rule,
            &mut held,
            &engagement,
            arms,
            mover,
        )?;
        (unit.life.attribute_type, unit.life.amount) = held;
        if arrived {
            let g = arms.idle_sprite.ok_or(crate::turn::NO_STANDING_SPRITE)?;
            crate::attack::set_sprite(&mut unit.sprite, g, self.sprite);
            if let Some(a) = unit.anim_speed.as_mut() {
                *a = 0.0;
            }
        }
        let drops = hunt.target == -1 && hunt.carry_to == target;
        unit.hunt = Some(hunt);
        if drops {
            self.takes.push((target, by));
            self.releases.push((target, by));
        }
        Ok(())
    }

    /// The unit's slot 0x340 on its unit AI's move at once ([`Unit::ai_move_with`]; a Transport
    /// on top sails there, [`Unit::transport_sail`]), its releases and its standing sprite as
    /// the move after the update gives them.
    ///
    /// [`Unit::ai_move_with`]: crate::turn::unit::Unit::ai_move_with
    /// [`Unit::transport_sail`]: crate::turn::unit::Unit::transport_sail
    fn ai_move_now(&mut self, m: AiMove) -> Result<(), GenieError> {
        let class = self
            .masters
            .and_then(|ms| ms.get(self.unit.life.master_id))
            .ok_or(crate::turn::UNIT_AI_MASTER)?
            .class;
        let by = Releaser {
            id: self.unit.id,
            class,
            player: self.player,
        };
        let sails = self.unit.ai_move_with(
            m,
            (class, self.player),
            (self.releases, self.builder_leaves, by),
            (self.masters, self.graphics, self.sprite),
        )?;
        if sails {
            self.unit.transport_sail(
                m.point,
                (self.spatial, self.zones, self.pathing),
                (self.masters, self.graphics, self.sprite),
            )?;
        }
        Ok(())
    }

    fn wait_solidify(&mut self, target: i32) -> Result<bool, GenieError> {
        let unit = &*self.unit;
        let (player, others, foreign) = (self.player, self.others, self.foreign);
        let find = |id: i32| {
            if id == unit.id {
                return Some((player, unit));
            }
            others
                .iter()
                .find(|u| u.id == id)
                .map(|u| (player, u))
                .or_else(|| foreign.find(id))
        };
        let (Some(t), Some(general)) = (
            task_target_with(target, 1, &find, self.every)?,
            task_target_with(target, 2, &find, self.every)?,
        ) else {
            return Ok(false);
        };
        let (masters, own) = self
            .masters
            .and_then(|m| Some((m, m.get(unit.life.master_id)?)))
            .ok_or(crate::turn::UNIT_AI_MASTER)?;
        let actor = Actor {
            id: unit.id,
            master: own,
            player,
            held: (unit.life.attribute_type, unit.life.amount),
            attribute_95: self.attributes.get(95).copied().unwrap_or(0.0),
            civ: self.civ,
        };
        let dip = SideWorld {
            player,
            relations: self.relations,
            foreign,
            drop_sites: self.drop_sites,
        };
        let Some((task, switch)) = task_search(masters, &actor, &t, &general, -1, &dip)? else {
            return Ok(false);
        };
        self.starts.push(AiStart::Wait(Wait {
            target,
            task,
            switch: switch.map(|m| m.id),
        }));
        Ok(true)
    }

    /// The soldier's `AttackObject` in a group (`FUN_140c17550`): the unit's future path data
    /// (slot 0x550), which the slice holds none of, goes; then a unit that moves (slot 0x5c8) or
    /// speeds up (slot 0x5a0: its forward acceleration above 0, `COMISS`/`JBE`) stops (slot
    /// 0x5d0).
    fn group_attack_stop(&mut self) -> Result<(), GenieError> {
        let Some(m) = self.unit.mover.as_mut() else {
            return Ok(());
        };
        let speeding = m.velocity.as_ref().is_some_and(|v| 0.0 < v.acceleration[2]);
        if m.is_moving() || speeding {
            m.stop()?;
        }
        Ok(())
    }

    /// Slot 400 at the full update's tail, after what the AI gave (made after the update,
    /// [`AiStart`], the last of them counting): a wait for a foundation holds no object, a
    /// repair its target, a master switch without a task left no action, an enter the object
    /// it enters; else the top action's target ([`Unit::top_target`]); either while the world
    /// holds it.
    ///
    /// [`Unit::top_target`]: crate::turn::unit::Unit::top_target
    fn top_target_now(&mut self) -> Result<Option<i32>, GenieError> {
        let target = match self.starts.last() {
            Some(AiStart::Wait(_)) => return Ok(None),
            Some(AiStart::Repair(
                RepairOrder::Retake(t)
                | RepairOrder::Start {
                    target: t,
                    task: Some(_),
                    ..
                },
            )) => Some(*t),
            // the Transport holds no target
            Some(
                AiStart::Repair(RepairOrder::Start { task: None, .. }) | AiStart::Transport(_),
            ) => None,
            Some(AiStart::Enter(e)) => Some(e.entered),
            None => self.unit.top_target().filter(|&t| t != -1),
        };
        Ok(target.filter(|&t| self.spatial.get(t).is_some()))
    }

    /// The object as the full update's tail reads it, found as [`UnitSide::seen_unit`] finds
    /// it.
    fn target_flags(&mut self, id: i32) -> Result<Option<crate::unit_ai::TargetFlags>, GenieError> {
        let Some(seen) = self.seen_unit(id)? else {
            return Ok(None);
        };
        let u = self
            .others
            .iter()
            .find(|u| u.id == id)
            .or_else(|| self.foreign.find(id).map(|(_, u)| u))
            .expect("seen_unit found it");
        let kind = self
            .every
            .get(seen.owner)
            .and_then(|ms| ms.get(u.life.master_id))
            .expect("seen_unit found its master")
            .kind;
        Ok(Some(crate::unit_ai::TargetFlags {
            owner: seen.owner,
            kind,
            capture: u.capture_flag,
            pasture: u.pasture,
            untargetable: u.untargetable,
            attackable: u.attackable,
        }))
    }
}
