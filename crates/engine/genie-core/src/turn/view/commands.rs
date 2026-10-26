//! The repair and enter commands of the unit AI and the auto scout's area.

use crate::movement::{Me, Mover, Planning};
use crate::spatial::Spatial;
use crate::tasks::Actor;
use crate::turn::unit::Unit;
use crate::turn::view::{AiStart, EnterOrder, RepairOrder, UnitView};
use crate::turn::world::{SideWorld, task_search, task_target_with};
use crate::unit_ai::GenieError;

impl UnitView<'_> {
    /// The unit's repair command (slot 0x378, `FUN_141217c00`, forced) on `target`: an object the
    /// world (`FUN_1415417a0`) does not hold is 0. A Repair on top takes it (its slot 0x48,
    /// `FUN_140adb480`): 1. Else a unit of task swap group 1 or 2 switches to its player's
    /// repairer ([`crate::repair::REPAIRERS`], slot 0x90 with 0), and its slot 0x820 with the
    /// repair's action type (`0x6a`) and the switch ([`task_search`] on the target and its main
    /// building, the new master's list first) finds a task: a new Repair on it
    /// (`FUN_140ad7fa0`) replaces the unit's actions (slot 0x808): 1; none: 0. The caller makes
    /// the switches and the Repair after the update ([`RepairOrder`], [`Turns::repair_order`]).
    pub(super) fn repair_command(&mut self, target: i32) -> Result<bool, GenieError> {
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
        if find(target).is_none() {
            return Ok(false);
        }
        if unit.top_action_kind() == Some(crate::repair::REPAIR) {
            self.starts
                .push(AiStart::Repair(RepairOrder::Retake(target)));
            return Ok(true);
        }
        let masters = self.masters.ok_or(crate::turn::UNIT_AI_MASTER)?;
        let own = masters
            .get(unit.life.master_id)
            .ok_or(crate::turn::UNIT_AI_MASTER)?;
        let switch = crate::repair::REPAIRERS
            .iter()
            .find(|&&(group, _)| group == own.task_by_group)
            .and_then(|&(_, id)| masters.get(id));
        let (Some(t), Some(general)) = (
            task_target_with(target, 1, &find, self.every)?,
            task_target_with(target, 2, &find, self.every)?,
        ) else {
            return Err(GenieError::unreachable(
                "a repair command's target the world holds without its task target",
            ));
        };
        let actor = Actor {
            id: unit.id,
            master: switch.unwrap_or(own),
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
        let found = task_search(masters, &actor, &t, &general, crate::repair::REPAIR, &dip)?;
        if switch.is_none() && found.is_none() {
            return Ok(false);
        }
        self.starts.push(AiStart::Repair(RepairOrder::Start {
            target,
            switches: [
                switch.map(|m| m.id),
                found.and_then(|(_, m)| m.map(|m| m.id)),
            ],
            task: found.map(|(task, _)| task),
        }));
        Ok(found.is_some())
    }

    /// The unit's enter command (slot 0x3a0, `FUN_1410847e0`, forced) on `target`: an object the
    /// world (`FUN_1415417a0`) does not hold is 0; its slot 0x820 with the garrison filter (3)
    /// and the switch ([`task_search`] on the target and its main building) finds a task, whose
    /// Enter (`FUN_140a855d0`) replaces the unit's actions (an Artifact on top would take it as
    /// its sub-action): 1; none: 0. The caller makes the switch and the Enter after the update
    /// ([`EnterOrder`], [`Turns::enter_order`]).
    pub(super) fn enter_command(&mut self, target: i32) -> Result<bool, GenieError> {
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
        let Some((tp, t_unit)) = find(target) else {
            return Ok(false);
        };
        let under_artifact = unit.top_action_kind() == Some(crate::artifact::ARTIFACT_TYPE);
        let masters = self.masters.ok_or(crate::turn::UNIT_AI_MASTER)?;
        let own = masters
            .get(unit.life.master_id)
            .ok_or(crate::turn::UNIT_AI_MASTER)?;
        let (Some(t), Some(general)) = (
            task_target_with(target, 1, &find, self.every)?,
            task_target_with(target, 2, &find, self.every)?,
        ) else {
            return Err(GenieError::unreachable(
                "an enter command's target the world holds without its task target",
            ));
        };
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
        let Some((task, switch)) =
            task_search(masters, &actor, &t, &general, crate::order::GARRISON, &dip)?
        else {
            return Ok(false);
        };
        // the constructor's target: a building's annex enters its main building
        let building = self
            .every
            .get(tp)
            .and_then(|m| m.get(t_unit.life.master_id))
            .is_some_and(|m| m.kind == crate::order::BUILDING);
        let entered = if building {
            t_unit.annex_of.unwrap_or(target)
        } else {
            target
        };
        self.starts.push(AiStart::Enter(EnterOrder {
            target,
            entered,
            task,
            switch: switch.map(|m| m.id),
            under_artifact,
        }));
        Ok(true)
    }

    /// What an explorer's searches ask of the world, the map's size and the unit's line of sight.
    pub(in crate::turn) fn scout_area(
        &mut self,
    ) -> Result<(ScoutArea<'_>, [i32; 2], f32), GenieError> {
        let unit = &*self.unit;
        let master = self
            .masters
            .and_then(|m| m.get(unit.life.master_id))
            .ok_or(GenieError::unsupported(
                "an auto scout of a master the slice does not have",
            ))?;
        let mover = unit.mover.as_ref().ok_or(GenieError::unsupported(
            "an auto scout without movement state",
        ))?;
        let [w, h] = self.spatial.map;
        let terrain = self
            .spatial
            .terrain
            .as_ref()
            .ok_or(GenieError::unsupported(
                "the map's zones without its terrain",
            ))?;
        let values = terrain
            .restrictions
            .get(mover.stats.restriction)
            .ok_or(crate::zones::UNKNOWN_RESTRICTION)?;
        let zones = self.zones.of(w, h, &terrain.kinds, values);
        let me = Me {
            id: unit.id,
            position: unit.position,
            group: unit.group.id,
            action_state: unit.top_action().map_or(0, |a| a.state),
        };
        let area = ScoutArea {
            spatial: self.spatial,
            zones,
            plan: Planning {
                pathing: self.pathing,
                now: self.world_time,
                computer: self.computer,
                object: self.object,
                construction_rate: self.construction_rate,
                groups: self.groups,
            },
            mover,
            me,
            player: self.player,
            master: master.id,
            radius: master.radius,
            reach: master.line_of_sight - LOS_MARGIN,
            scouted: false,
        };
        Ok((area, [w, h], master.line_of_sight))
    }
}

/// The auto scout's search reaches within its line of sight less this (`FUN_140c65490`,
/// `DAT_14330eb8c`).
pub(in crate::turn) const LOS_MARGIN: f32 = crate::image::explore::LOS_MARGIN;

/// The masters of a town centre (`FUN_1416597e0`: 109, then 444).
pub(in crate::turn) const TOWN_CENTRES: [i16; 2] = [109, 444];

/// `FUN_1416597e0` with 0: the player's first town centre, by the player's index of objects by
/// master (`player+0x18`, `FUN_14104f560`: those of master 109, then 444), as its tile. The
/// index's order within a master is not held: two objects of the first master found are not
/// modelled.
pub(in crate::turn) fn town_centre(units: &[Unit]) -> Result<Option<[i32; 2]>, GenieError> {
    for master in TOWN_CENTRES {
        let mut found = units.iter().filter(|u| u.life.master_id == master);
        let Some(first) = found.next() else { continue };
        if found.next().is_some() {
            return Err(GenieError::unsupported(
                "two town centres (the order of the player's index)",
            ));
        }
        return Ok(Some([first.position[0] as i32, first.position[1] as i32]));
    }
    Ok(None)
}

/// What the auto scout's search asks of the world for one unit ([`crate::explore::Area`]).
pub(in crate::turn) struct ScoutArea<'a> {
    pub(in crate::turn) spatial: &'a Spatial,
    pub(in crate::turn) zones: &'a crate::zones::ZoneMap,
    pub(in crate::turn) plan: Planning<'a>,
    pub(in crate::turn) mover: &'a Mover,
    pub(in crate::turn) me: Me,
    pub(in crate::turn) player: usize,
    pub(in crate::turn) master: i16,
    pub(in crate::turn) radius: [f32; 2],
    /// The reach of slot 0x3d8: the line of sight less a tile.
    pub(in crate::turn) reach: f32,
    /// The search reads the player's scout map for the tiles seen (`FUN_140c4b130` with one).
    pub(in crate::turn) scouted: bool,
}

impl crate::explore::Area for ScoutArea<'_> {
    fn explored(&self, x: i32, y: i32) -> Result<bool, GenieError> {
        if self.scouted {
            return self
                .spatial
                .scout_map(self.player)
                .and_then(|m| m.get(x, y))
                .map(|v| v == 1)
                .ok_or(GenieError::unsupported("a tile off the player's scout map"));
        }
        self.spatial.explored(self.player, x, y)
    }

    fn zone(&mut self, x: i32, y: i32) -> u16 {
        self.zones.zone(x, y)
    }

    fn own_zone(&mut self) -> u16 {
        let [x, y, _] = self.me.position;
        self.zones.zone(x as i32, y as i32)
    }

    fn clear(&self, x: i32, y: i32) -> Result<bool, GenieError> {
        let mask = self.mover.stats.restriction_mask;
        self.spatial
            .point_clear(self.me.id, [x as f32, y as f32], self.radius[0], mask)
    }

    fn reaches(&mut self, x: i32, y: i32) -> Result<bool, GenieError> {
        let to = [x as f32, y as f32];
        self.mover.reaches(
            &self.me,
            self.spatial,
            &mut self.plan,
            to,
            self.reach,
            self.master,
        )
    }

    /// `FUN_14118faf0`: the gap from the footprint on each axis (at least 0), then its length.
    fn distance(&self, x: i32, y: i32) -> f32 {
        let gap = |d: f32, r: f32| {
            let g = d.abs() - r;
            if g <= 0.0 { 0.0 } else { g }
        };
        let gx = gap(x as f32 - self.me.position[0], self.radius[0]);
        let gy = gap(y as f32 - self.me.position[1], self.radius[1]);
        crate::fmath::sqrt(gy * gy + gx * gx)
    }
}
