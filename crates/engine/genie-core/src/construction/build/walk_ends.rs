//! The ends of the builder's walk: at its end, failed, its foundation lost; the walk to a
//! point (state 11).

use crate::GenieError;
use crate::construction::build::{
    BUILDER_UNMOVING, BuildAction, BuildStep, Builder, FOREIGN_SITE, REACH, SEEN, SITE_GONE, Site,
    UNSEEN_SITE, VILLAGER, WALK_POINT, WORK,
};
use crate::movement::move_to::{MoveTo, Walk};

impl BuildAction {
    /// The update's case 3 with its `MoveTo` at its end (1), or without one (the list's update
    /// returns 1): without a foundation, a villager, or within the task's range (the footprints'
    /// squared gap, `FUN_14118f8c0`, at most the range squared) or [`REACH`] of it, it works
    /// (`set_state(6)`; one without a foundation waits instead, `set_state(3)`); else it walks to
    /// the foundation again (`set_state(4)`, [`BuildAction::restart`]).
    ///
    /// # Errors
    ///
    /// What the restart and the wait do not model.
    pub(super) fn walk_ended(
        &mut self,
        walk: &mut Walk<'_>,
        site: Option<Site<'_>>,
        builder: Builder<'_>,
        step: &mut BuildStep,
    ) -> Result<(), GenieError> {
        if self.target == -1 {
            return self.idle(walk, site, builder, step);
        }
        if builder.class != VILLAGER {
            let at = walk.position();
            let ground = walk
                .body
                .mover()
                .map(|m| [m.stats.radius[0], m.stats.radius[1]])
                .ok_or(BUILDER_UNMOVING)?;
            let t = *walk.world.get_known(self.target)?.ok_or(SITE_GONE)?;
            let gap = crate::movement::footprint_gap_squared(at, ground, t.position, t.radius);
            let range = self.task.range;
            if !(gap <= range * range || gap <= REACH) {
                let state = site.as_ref().map(|s| s.life.state);
                return self.restart(walk, site.map(Site::into_builders), state);
            }
        }
        self.leave(walk, None)?;
        self.state = WORK;
        Ok(())
    }

    /// The update's case 3 or 0xde with its `MoveTo` failed (3): a foundation no builder started
    /// (`+0xc60`) with another player's marks on its footprint ([`Spatial::foreign_marks`]) while
    /// its owner sees it (`FUN_1411d4b80` with 1: its slot 0x1e0, [`Spatial::building_sight`])
    /// goes when its owner is human ([`BuildStep::removes`]), the interface hears of it
    /// (`FUN_140c898a0` 0x88, not held) and `set_state(2)` halts the Build. Else the tile planner
    /// of the builder's terrain restriction (`FUN_140f7cb30`) asks for a path from where the
    /// builder last moved to (slot 0x520) to the foundation's place within their radii
    /// (`FUN_140f7c530`): a result below 2 walks again (`set_state(4)`), any other, no planner or
    /// no foundation waits (`set_state(3)`).
    ///
    /// # Errors
    ///
    /// Another player's foundation, one its owner sees only through its ghosts, and what the
    /// restart and the wait do not model.
    ///
    /// [`Spatial::building_sight`]: crate::map::spatial::Spatial::building_sight
    /// [`Spatial::foreign_marks`]: crate::map::spatial::Spatial::foreign_marks
    pub(super) fn walk_failed(
        &mut self,
        walk: &mut Walk<'_>,
        site: Option<Site<'_>>,
        builder: Builder<'_>,
        step: &mut BuildStep,
    ) -> Result<(), GenieError> {
        if self.target != -1 {
            let s = site.as_ref().ok_or(FOREIGN_SITE)?;
            let placed = *walk.world.get_known(self.target)?.ok_or(SITE_GONE)?;
            if s.construction.desolid && walk.world.foreign_marks(&placed) {
                let seen = walk
                    .world
                    .building_sight(self.target, placed.position, placed.player)
                    .ok_or(UNSEEN_SITE)?
                    == SEEN;
                if seen {
                    step.removes = Some(self.target);
                    return self.halt_in(walk, site, step);
                }
            }
        }
        let mover = walk.body.mover().ok_or(BUILDER_UNMOVING)?;
        let (restriction, ground, [mx, my, _]) = (
            mover.stats.restriction,
            mover.stats.radius[0],
            mover.moved_at,
        );
        let mut code = 3;
        if self.target != -1
            && let Some(plan) = walk.plan.as_mut()
            && let Some(planner) = plan.pathing.planner(restriction, walk.world.tiles())
        {
            let t = *walk.world.get_known(self.target)?.ok_or(SITE_GONE)?;
            let radius = site
                .as_ref()
                .and_then(|s| s.completion.master)
                .map_or(t.radius[0], |m| m.radius[0]);
            let req = crate::pathing::tile_path::Request {
                from: [mx, my],
                to: [t.position[0], t.position[1]],
                radii: radius + ground,
                range: 0.0,
                closed: Vec::new(),
                full: false,
            };
            let mut path = crate::pathing::tile_path::Path::default();
            code = planner
                .plan::<GenieError>(&req, false, &mut path, None)?
                .code();
        }
        if code < 2 {
            let state = site.as_ref().map(|s| s.life.state);
            self.restart(walk, site.map(Site::into_builders), state)
        } else {
            self.idle(walk, site, builder, step)
        }
    }

    /// Its `MoveTo` returned 4 or 5 in state 4 or 223 (`0x140ab0370` cases 3 and 0xde): slot
    /// 0x68 with none (`FUN_140ab0300`: a counted builder leaves the builders of
    /// `FUN_140aaddb0`'s building, the foundation is let go, [`BuildStep::released`]) and
    /// `set_state(11)`: the prelude ([`BuildAction::leave`]: the `MoveTo`'s teardown stops the
    /// unit), then a quiet `MoveTo` to the action's point (`FUN_140aa0d30`: range 0, the task's
    /// moving sprite; `FUN_140a7a170` with 1), which starts in state 3 (`FUN_140aa2100`: a
    /// `MoveTo` without a target is far from it), in 13 for a unit without speed.
    pub(super) fn lose_foundation(
        &mut self,
        walk: &mut Walk<'_>,
        site: Option<Site<'_>>,
        step: &mut BuildStep,
    ) -> Result<(), GenieError> {
        let builders = site.map(Site::into_builders);
        self.leave(walk, builders)?;
        if self.target != -1 {
            self.target = -1;
            step.released = true;
        }
        self.walk_to_point(walk, None)
    }

    /// `set_state(11)` (`0x140aae200` case 0xb): the prelude ([`BuildAction::leave`]), then a
    /// quiet `MoveTo` to the action's point (`FUN_140aa0d30`: range 0, the task's moving sprite;
    /// `FUN_140a7a170` with 1), which starts in state 3 (`FUN_140aa2100`: a `MoveTo` without a
    /// target is far from it), in 13 for a unit without speed.
    pub(super) fn walk_to_point(
        &mut self,
        walk: &mut Walk<'_>,
        builders: Option<&mut u8>,
    ) -> Result<(), GenieError> {
        self.leave(walk, builders)?;
        self.state = WALK_POINT;
        self.goal = Some(self.point);
        let moves = walk.body.mover().is_some_and(|m| m.stats.speed != 0.0);
        self.move_to = Some(if moves { MoveTo::Start } else { MoveTo::Failed });
        Ok(())
    }
}
