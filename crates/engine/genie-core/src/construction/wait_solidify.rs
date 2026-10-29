//! A villager's wait for a foundation: `TRIBE_Action_Wait_Solidify` (action type 0x9a, vtable
//! `0x142ed82f8`). A human player's villager that a foundation sent off its footprint (the unit
//! AI's notify 0x216) queues the foundation's order (0x2de) in front of its work; the order's
//! slot 0x108 (`FUN_140c68a60`) runs the unit's slot 0x388 (`FUN_141226fc0`), whose new action
//! (`FUN_140ae81c0`, with the task slot 0x820 found on the foundation) replaces the unit's
//! actions. It waits until the foundation's construction has started (its build points past
//! 0.01) or the world no longer holds it; the action list then removes it and the unit AI's
//! queue goes on with the work.
//!
//! Its start (slot 0x28, `FUN_140ae87c0`) does nothing; its teardown (`FUN_140ae8780`) is the
//! base one (`FUN_140a73fc0`), which lets no target go: the constructor leaves the base's target
//! (`+0x20`, `+0x30`) unset and keeps the foundation's id at `+0x68` instead.

use crate::GenieError;
use crate::objects::masters::Task;
use crate::unit_ai::task_finder::Actor;
use crate::unit_ai::task_finder::world::{SideWorld, point_terrain, task_search, task_target_with};
use crate::unit_ai::world_view::owner_view;
use crate::unit_ai::world_view::{AiStart, UnitView, Wait};
use genie_diff::Diff;

/// The action type (`+0x8`, 0x9a).
pub const WAIT_SOLIDIFY: i16 = 0x9a;

/// `+0x18`: the state the constructor gives it; only its `set_state` (slot 0x80) changes it.
pub const WAITING: u8 = 10;

/// `DAT_142eb56a0` (a double): the foundation's build points (`+0xc50`, widened) at most which
/// it is not solid yet.
const SOLID_FROM: f64 = crate::math::image::wait_solidify::SOLID_FROM;

/// `TRIBE_Action_Wait_Solidify`: its state (`+0x18`), its task (`+0x48`) and the foundation it
/// waits for (`+0x68`).
#[derive(Clone, Copy, Debug, PartialEq, Diff)]
pub struct WaitSolidify {
    /// `+0x18`.
    pub state: u8,
    /// `+0x48`: the task slot 0x820 found on the foundation, whatever its action type.
    pub task: Task,
    /// `+0x68`: the foundation's id.
    pub target: i32,
}

impl WaitSolidify {
    /// The constructor (`FUN_140ae81c0`): the base action's (`FUN_140a73e90` with 1, which
    /// leaves the target unset), the task, the type 0x9a, the foundation's id and state
    /// [`WAITING`].
    #[must_use]
    pub const fn new(task: Task, target: i32) -> Self {
        Self {
            state: WAITING,
            task,
            target,
        }
    }

    /// Its update (slot 0x30, `FUN_140ae8b70`) with the foundation's build points (`+0xc50`),
    /// None when the world does not hold it (`FUN_1415417a0`): true (1, done) unless the world
    /// holds it with build points at most [`SOLID_FROM`]. `COMISD` with `JA`: build points that
    /// are not a number keep it waiting.
    #[must_use]
    pub fn update(&self, progress: Option<f32>) -> bool {
        progress.is_none_or(|p| f64::from(p) > SOLID_FROM)
    }

    /// Its `set_state` (slot 0x80, `FUN_140ae8b60`): the state, nothing else.
    pub const fn set_state(&mut self, state: u8) {
        self.state = state;
    }
}

impl UnitView<'_, '_> {
    /// The unit AI's wait for a foundation while it runs ([`crate::unit_ai::UnitSide::wait_solidify`]):
    /// the task the unit's slot 0x820 finds on `target` (any action type, with the master switch)
    /// gives the wait the caller makes after the update ([`AiStart::Wait`]); false without one.
    pub(crate) fn start_wait_now(&mut self, target: i32) -> Result<bool, GenieError> {
        let unit = &*self.unit;
        let (player, others, foreign) = (
            self.player,
            owner_view(&self.c.w.players, self.c.player, self.new_owner),
            owner_view(&self.c.w.players, self.c.player, self.new_owner).players(),
        );
        let find = |id: i32| {
            if id == unit.id {
                return Some((player, unit));
            }
            others.find(id)
        };
        let (Some(t), Some(general)) = (
            task_target_with(target, 1, &find, &self.c.w.masters)?,
            task_target_with(target, 2, &find, &self.c.w.masters)?,
        ) else {
            return Ok(false);
        };
        let masters = self
            .c
            .w
            .masters
            .get(self.player)
            .ok_or(crate::world::UNIT_AI_MASTER)?;
        let own = unit.master(&self.c.w.masters, self.player)?;
        let actor = Actor {
            id: unit.id,
            master: own,
            player,
            held: (unit.life.attribute_type, unit.life.amount),
            attribute_95: self.c.w.players[self.player]
                .attributes
                .get(95)
                .copied()
                .unwrap_or(0.0),
            civ: self.civ,
        };
        let dip = SideWorld {
            player,
            relations: &self.c.w.players[self.player].relations,
            foreign,
            drop_sites: &self.c.w.drop_sites,
        };
        // slot 0x820's point: the target's place
        let at = find(target).map_or([0.0; 3], |(_, u)| u.position());
        let terrain = point_terrain(self.c.w.spatial.terrain.as_ref(), at);
        let Some((task, switch)) = task_search(masters, &actor, &t, &general, terrain, -1, &dip)?
        else {
            return Ok(false);
        };
        self.c.starts.push(AiStart::Wait(Wait {
            target,
            task,
            switch: switch.map(|m| m.id),
        }));
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task() -> Task {
        Task {
            id: 3,
            action_type: 101,
            ..Task::default()
        }
    }

    #[test]
    fn a_new_wait_holds_its_task_and_foundation_in_state_10() {
        let w = WaitSolidify::new(task(), 8123);
        assert_eq!((w.state, w.task, w.target), (WAITING, task(), 8123));
        assert_eq!(WAITING, 10);
        assert_eq!(WAIT_SOLIDIFY, 0x9a);
    }

    #[test]
    fn the_wait_ends_once_the_foundation_is_past_a_hundredth_or_gone() {
        let w = WaitSolidify::new(task(), 8123);
        assert!(w.update(None), "a foundation the world does not hold");
        assert!(!w.update(Some(0.0)));
        // 0.01f32 widens to 0.009999999776…, at most the double 0.01
        assert!(!w.update(Some(0.01)));
        assert!(w.update(Some(0.010_001)));
        assert!(w.update(Some(1.5)));
        // COMISD with JA: unordered is not above
        assert!(!w.update(Some(f32::NAN)));
    }

    #[test]
    fn its_set_state_sets_the_state_alone() {
        let mut w = WaitSolidify::new(task(), 8123);
        w.set_state(2);
        assert_eq!(
            w,
            WaitSolidify {
                state: 2,
                ..WaitSolidify::new(task(), 8123)
            }
        );
    }
}
