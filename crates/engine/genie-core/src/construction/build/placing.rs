//! The placement on a foundation no builder started (state 7), a builder's joining and its
//! work, the walk off the footprint (state 223) and back.

use crate::GenieError;
use crate::construction::build::{
    BUILDER_UNMOVING, BuildAction, BuildStep, Built, CLEAR, ClearingSite, Construction, Dropped,
    EXTRA_BUILDER, FOREIGN_SITE, HALTED, Load, Looks, MAX_WAITS, PLACE, SITE_GONE, Site, VILLAGER,
    WAIT, WALL, WONDER, WONDER_RATE, Worked, footprint_clearing,
};
use crate::movement::footprint_gap;
use crate::movement::move_to::{MoveTo, Walk};

impl BuildAction {
    /// `set_state(7)` (`0x140aae200` case 7) on a foundation no builder started: clear the
    /// footprint of what may be built over (`FUN_1415c3620` with `param_6`,
    /// [`footprint_clearing`]: the records of what goes go now, the deletes are the caller's
    /// after the update, [`BuildStep::cleared`]), find who stands on it (those just cleared are in
    /// state 7, which the test passes over), and send this builder off it if it stands there
    /// itself. Another player's marks on the footprint ([`Spatial::foreign_marks`]) halt it
    /// ([`BuildStep::removes`]).
    ///
    /// [`Spatial::foreign_marks`]: crate::map::spatial::Spatial::foreign_marks
    pub(super) fn place(
        &mut self,
        walk: &mut Walk<'_>,
        site: Option<Site<'_>>,
        (waits, look): (&mut u8, Looks<'_>),
        step: &mut BuildStep,
    ) -> Result<(), GenieError> {
        let mut site = site.ok_or(FOREIGN_SITE)?;
        // every set_state first takes the builder out of FUN_140aaddb0's building's builders
        if self.added {
            let b = site.builders();
            *b = b.wrapping_sub(1);
            self.added = false;
        }
        self.move_to = None;
        self.goal = None;
        self.walk_target = -1;
        self.state = PLACE;
        if !site.construction.desolid {
            return self.join(walk, &mut site);
        }
        let placed = *walk.world.get_known(self.target)?.ok_or(SITE_GONE)?;
        let clearing = ClearingSite {
            player: placed.player,
            master: placed.master,
            class: placed.class,
            radius: placed.radius,
            at: [placed.position[0], placed.position[1]],
            trees: true,
        };
        let cleared = footprint_clearing(walk.world, &clearing, look.0)?;
        for cl in cleared.iter().filter(|cl| cl.record_goes) {
            if let Some(p) = walk.plan.as_mut() {
                p.pathing.removed(cl.id);
            }
            walk.world.drop_record(cl.id);
        }
        if let Some(p) = walk.plan.as_mut() {
            p.pathing.tiles_changed(walk.world.take_changes());
        }
        let site_ref = &mut site;
        let site = &placed;
        let world = &*walk.world;
        let obstructs = !matches!(site.obstruction_type, 0 | 12);
        let occupants: Vec<i32> = world
            .footprint_occupants(self.target, site, obstructs, look.0)?
            .into_iter()
            .filter(|&id| !cleared.iter().any(|cl| cl.id == id))
            .collect();
        step.cleared = cleared;
        step.occupants = occupants
            .iter()
            .copied()
            .filter(|&id| id != walk.me.id)
            .collect();
        if world.foreign_marks(site) {
            // FUN_1411d4b80: a human owner's foundation goes (FUN_1411ccbb0, the caller's after
            // the update), the interface hears of it (FUN_140c898a0 0x88, not held), and
            // set_state(2) halts the Build: the standing sprite, the foundation let go, no point
            step.removes = Some(self.target);
            step.stands = true;
            step.released = true;
            self.target = -1;
            self.point = [-1.0; 3];
            self.state = HALTED;
            return Ok(());
        }
        if occupants.is_empty() {
            return self.join(walk, site_ref);
        }
        let [bx, by, _] = walk.position();
        let [sx, sy, _] = site.position;
        let inside = (bx - sx).abs() <= site.radius[0] && (by - sy).abs() <= site.radius[1];
        if inside {
            return self.clear(walk);
        }
        // LAB_140aaf5d4: the builder waits a second (set_state 222) up to 21 times; then the
        // interface hears of it (FUN_140c898a0 0x88, not held), set_state(2) halts the Build
        // (the standing sprite, the foundation let go, no point) and the waits start again
        if MAX_WAITS <= *waits {
            step.stands = true;
            step.released = true;
            self.target = -1;
            self.point = [-1.0; 3];
            self.state = HALTED;
            *waits = 0;
            return Ok(());
        }
        *waits += 1;
        self.state = WAIT;
        self.timer = 1.0;
        Ok(())
    }

    /// `LAB_140aaf9f4`: the builder joins the work: unless walls of other players stand on both
    /// tiles beside it on the footprint's diagonal (`set_state` 2, not modelled), it counts among
    /// the foundation's builders. Its work sprite and sound are not held by the slice.
    pub(super) fn join(&mut self, walk: &Walk<'_>, site: &mut Site<'_>) -> Result<(), GenieError> {
        let placed = walk.world.get(self.target).ok_or(SITE_GONE)?;
        let [bx, by] = [walk.position()[0] as i32, walk.position()[1] as i32];
        let [tx, mut ty] = [placed.position[0] as i32, placed.position[1] as i32];
        let [rx, ry] = placed.radius;
        if (rx as i32) as f32 == rx && (ry as i32) as f32 == ry {
            if by < ty {
                if tx < bx {
                    ty -= 1;
                }
            } else if ty < by && bx < tx {
                ty -= 1;
            }
        }
        let (dx, dy) = (bx - tx, by - ty);
        if dx.abs() == dy.abs() {
            let (a, b) = beside_diagonal(bx, by, dx, dy);
            let player = placed.player;
            let walled = |[x, y]: [i32; 2]| {
                walk.world
                    .object_on_tile(x, y, |o| o.class == WALL && o.player != player)
            };
            if walled(a) && walled(b) {
                return Err(GenieError::unsupported(
                    "walls beside a diagonal builder (set_state 2)",
                ));
            }
        }
        // FUN_140aaddb0's building counts it: an annex's main building
        if !self.added {
            let b = site.builders();
            *b = b.wrapping_add(1);
            self.added = true;
        }
        Ok(())
    }

    /// The work (state 7, `0x140ab0370` case 6): the builder adds its rate times dt to the
    /// foundation's construction: its work rate times the task's work value, times the owner's
    /// construction rate (attribute 195) when that is above 0 (the Wonder's at most 1.2), each
    /// builder after the first adding a third ([`Construction::work`]). A finished building, a
    /// farm's reseeding and the instant-build cheat (`game+0xc14`) are not modelled.
    pub(super) fn work(
        &mut self,
        mut walk: Walk<'_>,
        site: Option<Site<'_>>,
        load: Load,
    ) -> Result<(Option<Built>, Dropped, Option<f32>), GenieError> {
        let mut site = site.ok_or(FOREIGN_SITE)?;
        let plan = walk
            .plan
            .as_mut()
            .ok_or(GenieError::unsupported("work without the world's planners"))?;
        let mut rate = self.task.work_rate * self.task.work_value * walk.dt;
        let mut factor = plan.construction_rate;
        if site.life.master_id == WONDER && WONDER_RATE < factor {
            factor = WONDER_RATE;
        }
        if 0.0 < factor {
            rate *= factor;
        }
        let n = f32::from(*site.builders());
        if 1.0 < n {
            rate *= ((n - 1.0) * EXTRA_BUILDER + 1.0) / n;
        }
        let (pathing, now) = (&mut *plan.pathing, plan.now);
        let worked = Construction::work(&mut site, rate, walk.world, pathing, now)?;
        let (completed, passed) = match worked {
            Worked::Going => return Ok((None, None, None)),
            Worked::Main {
                amount,
                going: true,
            } => return Ok((None, None, Some(amount))),
            Worked::Main { amount, .. } => (None, Some(amount)),
            Worked::Done => (None, None),
            Worked::Completed(built) => (Some(built), None),
        };
        // a finished foundation with hit points ends the work (its `+0x278`, decoded); one
        // without (deleted, it dies in its own update) leaves the Build as it is
        if 0.0 < site.life.hp {
            let dropped = self.finish(&mut walk, Some(site.builders()), load)?;
            Ok((completed, dropped, passed))
        } else {
            Ok((completed, None, passed))
        }
    }

    /// `set_state(4)` after the walk off the footprint (`0x140aae200` case 4): the goal is the
    /// foundation again and a new `MoveTo` to it (`FUN_140aa0df0`) starts arrived if the footprints
    /// touch, as [`BuildAction::start`]'s. Only a villager returns at once (`0x140ab0370` case
    /// 0xde; any other builder in range would work, state 6).
    pub(super) fn back(&mut self, walk: &Walk<'_>) -> Result<(), GenieError> {
        let mover = walk.body.mover().ok_or(BUILDER_UNMOVING)?;
        if walk.world.get(walk.me.id).map(|p| p.class) != Some(VILLAGER) {
            return Err(GenieError::unsupported(
                "a builder other than a villager returns",
            ));
        }
        let site = walk.world.get_known(self.target)?.ok_or(SITE_GONE)?;
        let ground = [mover.stats.radius[0], mover.stats.radius[1]];
        let gap = footprint_gap(walk.position(), ground, site.position, site.radius);
        *self = self.restarted(self.target, site.position, gap);
        Ok(())
    }

    /// `set_state(223)`: a `MoveTo` to the free tile beside the footprint (`FUN_1411566b0`); it starts
    /// in state 3, as a `MoveTo` to a point is far from it (`FUN_140aa2100`).
    pub(super) fn clear(&mut self, walk: &Walk<'_>) -> Result<(), GenieError> {
        let mover = walk.body.mover().ok_or(BUILDER_UNMOVING)?;
        let site = walk.world.get(self.target).expect("place found it");
        let spot = walk
            .world
            .spot_beside(
                walk.position(),
                (walk.me.id, mover.stats.radius[0]),
                mover.stats.restriction_mask,
                site,
            )?
            .ok_or(GenieError::unsupported(
                "no free tile beside the footprint (set_state 2)",
            ))?;
        if mover.stats.speed == 0.0 {
            return Err(GenieError::unsupported(
                "a builder that cannot move (MoveTo state 13)",
            ));
        }
        self.state = CLEAR;
        self.move_to = Some(MoveTo::Start);
        self.walk_target = -1;
        self.goal = Some([spot[0], spot[1], walk.position()[2]]);
        Ok(())
    }

    /// The waiting `MoveTo`'s `set_state(3)` asks again: a wait (`+0x6c` 1) becomes asked (2).
    pub(super) const fn asked_again(&mut self) {
        if self.move_wait.state == 1 {
            self.move_wait.state = 2;
        }
    }
}

/// The two tiles beside a builder on a footprint's diagonal (`LAB_140aafc09`'s arguments).
pub(super) const fn beside_diagonal(bx: i32, by: i32, dx: i32, dy: i32) -> ([i32; 2], [i32; 2]) {
    if dx < 1 {
        if dx < 0 && dy < 0 {
            ([bx + 1, by], [bx, by + 1])
        } else if dx < 0 && 0 < dy {
            ([bx, by - 1], [bx + 1, by])
        } else {
            ([bx - 1, by], [bx, by + 1])
        }
    } else if dy < 1 {
        ([bx - 1, by], [bx, by + 1])
    } else {
        ([bx, by - 1], [bx - 1, by])
    }
}
