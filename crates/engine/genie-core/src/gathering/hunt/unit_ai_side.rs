//! The hunt the unit AI commands while it runs ([`crate::unit_ai::UnitSide::hunt_now`] through
//! its view, [`UnitView`]): `HuntObject`'s unit part at once and its plan.

use crate::GenieError;
use crate::map::spatial::Placed;
use crate::objects::release::Releaser;
use crate::unit_ai::UnitAi;
use crate::unit_ai::task_finder::Actor;
use crate::unit_ai::task_finder::world::{SideWorld, point_terrain, task_search, task_target_with};
use crate::unit_ai::world_view::UnitView;

/// What [`UnitView::start_hunt_now`] hunts by ([`UnitView::hunt_plan`]): the task, the master
/// the unit switches to (none to keep its own), its own master, the quarry and the target's
/// place.
struct HuntPlan<'u> {
    task: crate::objects::masters::Task,
    switch: Option<&'u crate::objects::masters::Master>,
    own: &'u crate::objects::masters::Master,
    quarry: crate::gathering::hunt::Quarry,
    them: Placed,
}

impl<'u> UnitView<'u> {
    /// `HuntObject`'s unit part at once ([`World::hunt_command`] in the view): the stopAction
    /// (the actions go, and in state 2 the master's standing sprite, slot 0x68), then the hunt
    /// command: slot 0x820's task with any filter and the switch ([`task_search`]; the switch,
    /// slot 0x90 with 0, [`crate::orders::switch_master`], its owner's part after the update) and a
    /// Hunt on it ([`World::start_hunt`]'s: the target taken, slot 0x178; the actions cleared;
    /// [`crate::gathering::hunt::HuntAction::start`] with the target as it stands, its master among the
    /// unit's drop sites, the engagement and arms; an Attack that starts arrived shows the idle
    /// sprite; a load to its own drop site takes the target as the second and lets it go as the
    /// first).
    pub(crate) fn start_hunt_now(&mut self, target: i32, ai: &UnitAi) -> Result<(), GenieError> {
        use crate::gathering::hunt::{HuntAction, HuntTask};
        let player = self.player;
        let own_class = self
            .masters
            .and_then(|m| m.get(self.unit.life.master_id))
            .ok_or(crate::world::HUNTER_MASTER)?
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
        let me = *self
            .spatial
            .get(id)
            .ok_or(crate::world::ATTACKER_UNPLACED)?;
        let speed = unit.body.mover().map_or(0.0, |m| m.stats.speed);
        let engagement = crate::combat::attack::engagement(
            Some(ai),
            speed,
            hunter,
            &me,
            &them,
            self.world_time,
        )?;
        let moving = hunt_task.walk_sprite_id(unit.life.attribute_type, unit.life.amount);
        let arms = crate::combat::attack::arms(hunter, self.graphics, hunt_task.sprites, moving);
        let mover = unit.body.mover_mut().ok_or(crate::world::HUNTER_UNMOVING)?;
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
            let g = arms.idle_sprite.ok_or(crate::world::NO_STANDING_SPRITE)?;
            crate::combat::attack::set_sprite(&mut unit.sprite, g, self.sprite);
            unit.body.set_anim_speed(0.0);
        }
        let drops = hunt.target == -1 && hunt.carry_to == target;
        unit.hunt = Some(hunt);
        if drops {
            self.takes.push((target, by));
            self.releases.push((target, by));
        }
        Ok(())
    }

    /// [`UnitView::start_hunt_now`]'s plan: the target as the unit's player finds it (an annex
    /// stands for its main building's master among the hunter's drop sites), slot 0x820's task
    /// with any filter and the master it switches to ([`task_search`]), the unit's own master,
    /// the quarry the Hunt starts on and the target's place.
    ///
    /// # Errors
    ///
    /// A target gone or of a master the slice does not have, no hunt task, and a target without
    /// its place.
    fn hunt_plan(&self, target: i32) -> Result<HuntPlan<'u>, GenieError> {
        use crate::gathering::hunt::Quarry;
        use crate::unit_ai::task_finder::Diplomacy;
        let (player, others, foreign) = (self.player, self.others, self.foreign);
        let unit = &*self.unit;
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
        let (tp, t) = find(target).ok_or(crate::orders::HUNT_TARGET_GONE)?;
        let t_master = self
            .every
            .get(tp)
            .and_then(|m| m.get(t.life.master_id))
            .ok_or(crate::world::OBJECT_MASTER)?;
        let main = if t_master.kind == crate::orders::BUILDING {
            t.annex_of.unwrap_or(target)
        } else {
            target
        };
        let main_master = match find(main) {
            Some((mp, m)) => {
                self.every
                    .get(mp)
                    .and_then(|ms| ms.get(m.life.master_id))
                    .ok_or(crate::world::OBJECT_MASTER)?
                    .id
            }
            None => t_master.id,
        };
        let (Some(task_t), Some(general)) = (
            task_target_with(target, 1, &find, self.every)?,
            task_target_with(target, 2, &find, self.every)?,
        ) else {
            return Err(crate::orders::HUNT_TARGET_GONE);
        };
        let masters = self.masters.ok_or(crate::world::HUNTER_MASTER)?;
        let own = masters
            .get(unit.life.master_id)
            .ok_or(crate::world::HUNTER_MASTER)?;
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
        // slot 0x820's point: the target's place
        let terrain = point_terrain(self.spatial.terrain.as_ref(), t.position);
        let (task, switch) = task_search(masters, &actor, &task_t, &general, terrain, -1, &dip)?
            .ok_or(crate::orders::NO_HUNT_TASK)?;
        let hunter = switch.unwrap_or(own);
        let quarry = Quarry {
            id: target,
            position: t.position,
            class: t_master.class,
            kind: t_master.kind,
            state: t.life.state,
            player: tp,
            amount: t.life.amount,
            allied: dip.allied(player, tp, true),
            top_action: t.top_action_kind(),
            drop_site: hunter.drop_sites.contains(&main_master),
        };
        let them = *self
            .spatial
            .get_known(target)?
            .ok_or(crate::world::TARGET_UNPLACED)?;
        Ok(HuntPlan {
            task,
            switch,
            own,
            quarry,
            them,
        })
    }
}
