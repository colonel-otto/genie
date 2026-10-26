//! What the unit AI asks of the world through its view: [`crate::unit_ai::UnitSide`] for
//! [`UnitView`]. The long answers live in their systems as inherent `UnitView` methods
//! (FD-041-D09): the attack command in `combat/attack/unit_ai_side.rs`, the build command and the
//! foundation searches in `construction/build/unit_ai_side.rs`, the hunt command in
//! `gathering/hunt/unit_ai_side.rs`, the wait for a foundation in `construction/wait_solidify.rs`
//! and the resource search in `gathering/closest_resource.rs`.

use crate::map::spatial::Placed;
use crate::math::rng::Well512;
use crate::movement::{Me, Planning, footprint_gap_squared};
use crate::unit_ai::world_view::{
    AiStart, RepairOrder, UnitView, WorldPicker, patrol_information, town_centre,
};
use crate::unit_ai::{AiMove, GenieError, Search, UnitAi};
use crate::world::PETARD;
use crate::world::lookups::{Releaser, herded};

impl crate::unit_ai::UnitSide for UnitView<'_> {
    fn table_drop_site(&mut self) -> Result<Option<i32>, GenieError> {
        let class = self
            .masters
            .and_then(|m| m.get(self.unit.life.master_id))
            .map(|m| m.class)
            .ok_or(crate::world::OBJECT_MASTER)?;
        let at = [self.unit.position[0], self.unit.position[1]];
        let look = |id: i32| {
            self.others
                .iter()
                .find(|u| u.id == id)
                .map(|u| (u.life.state, u.position))
        };
        Ok(crate::orders::nearest_table_site(
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
    /// [`Spatial::point_clear`]: crate::map::spatial::Spatial::point_clear
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

    fn off_footprint(
        &mut self,
        site: i32,
        radius: [f32; 2],
    ) -> Result<crate::unit_ai::OffFootprint, GenieError> {
        self.spot_off_footprint(site, radius)
    }

    fn foundation_search(&mut self) -> Result<i32, GenieError> {
        self.own_foundation_near()
    }

    fn call_for_help(&mut self, attacker: i32) -> Result<(), GenieError> {
        /// The classes that never help: 1 to 3, 18 to 21, 27 and 39.
        const NO_HELP_CLASSES: u64 = 0x0080_083c_000e;
        /// The siege tower master (`FUN_141194f10`'s 0x451).
        const SIEGE_TOWER: i16 = 0x451;
        /// `AskForHelp`'s reach (`DAT_142ecccc4`): the squared footprint gap of 3 tiles.
        const HELP_REACH: f32 = crate::math::image::unit_ai::HELP_REACH;
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
            .ok_or(crate::world::UNIT_AI_MASTER)?;
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
            .ok_or(crate::world::OBJECT_MASTER)?;
        let me = self
            .masters
            .and_then(|ms| ms.get(self.unit.life.master_id))
            .ok_or(crate::world::UNIT_AI_MASTER)?;
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
            allied: crate::unit_ai::task_finder::allied(self.relations, self.player, owner, true),
        }))
    }

    fn closest_resource(
        &mut self,
        search: crate::unit_ai::ResourceSearch,
    ) -> Result<Option<crate::unit_ai::Gatherable>, GenieError> {
        self.closest_gatherable(search)
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

    fn attack(
        &mut self,
        target: i32,
        force: bool,
        ai: &UnitAi,
    ) -> Result<crate::unit_ai::Attacked, GenieError> {
        self.start_attack_now(target, force, ai)
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
        if !scout_map
            && let Some(p) = crate::movement::explore::unexplored_area(tc, map, &mut area)?
        {
            return Ok(Some(p));
        }
        // none about the town centre: the unit AI's own search (slot 0x90)
        let seeker = crate::movement::explore::Seeker {
            at,
            line_of_sight,
            order: ai.current_order,
            target_point: [ai.target_point[0], ai.target_point[1]],
            computer,
        };
        crate::movement::explore::closest_undiscovered(&mut area, seeker, map, rng)
    }

    fn seek_beyond_sight(&mut self, rng: &mut Well512) -> Result<Option<[i32; 2]>, GenieError> {
        let (at, player) = (self.unit.position, self.player);
        let scouted = self.spatial.scout_map(player).is_some();
        let (mut area, map, line_of_sight) = self.scout_area()?;
        area.scouted = scouted;
        let found = crate::movement::explore::beyond_sight(&mut area, at, line_of_sight, map, rng)?;
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
        let size = crate::movement::explore::SCOUT_MAP_SIZE;
        let mut m = crate::movement::explore::InfluenceMap::new([size, size], 0);
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

    fn build_now(&mut self, foundation: i32) -> Result<(), GenieError> {
        self.start_build_now(foundation)
    }

    fn hunt_now(&mut self, target: i32, ai: &UnitAi) -> Result<(), GenieError> {
        self.start_hunt_now(target, ai)
    }

    /// The unit's slot 0x340 on its unit AI's move at once ([`Unit::ai_move_with`]; a Transport
    /// on top sails there, [`Unit::transport_sail`]), its releases and its standing sprite as
    /// the move after the update gives them.
    ///
    /// [`Unit::ai_move_with`]: crate::objects::unit::Unit::ai_move_with
    /// [`Unit::transport_sail`]: crate::objects::unit::Unit::transport_sail
    fn ai_move_now(&mut self, m: AiMove) -> Result<(), GenieError> {
        let class = self
            .masters
            .and_then(|ms| ms.get(self.unit.life.master_id))
            .ok_or(crate::world::UNIT_AI_MASTER)?
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
        self.start_wait_now(target)
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
    /// [`Unit::top_target`]: crate::objects::unit::Unit::top_target
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
