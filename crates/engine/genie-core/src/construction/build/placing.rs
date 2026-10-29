//! The placement on a foundation no builder started (state 7), a builder's joining and its
//! work, the walk off the footprint (state 223) and back.

use crate::GenieError;
use crate::construction::build::{
    BUILDER_UNMOVING, BuildAction, BuildReads, CLEAR, ClearingSite, EXTRA_BUILDER, FOREIGN_SITE,
    HALTED, MAX_WAITS, PLACE, SITE_GONE, VILLAGER, WAIT, WALL, WONDER, WONDER_RATE,
    footprint_clearing,
};
use crate::movement::footprint_gap;
use crate::movement::move_to::MoveTo;
use crate::objects::lifecycle::seconds;
use crate::objects::unit::Unit;
use crate::world::step::Step;

impl BuildAction {
    /// `set_state(7)` (`0x140aae200` case 7) on a foundation no builder started: clear the
    /// footprint of what may be built over (`FUN_1415c3620` with `param_6`,
    /// [`footprint_clearing`]: the records of what goes go first, then the objects,
    /// [`crate::world::World::clear_object`]), send the units standing on it off but this
    /// builder (`FUN_14123bb20`, [`crate::world::World::evacuate`]; those just cleared are in
    /// state 7, which the test passes over), and send this builder off it if it stands there
    /// itself. Another player's marks on the footprint ([`Spatial::foreign_marks`]) remove a
    /// human owner's foundation ([`BuildAction::remove_marked`]) and halt the Build.
    ///
    /// # Errors
    ///
    /// Another player's foundation, and what the clearing, the send-off and the removal do not
    /// model.
    ///
    /// [`Spatial::foreign_marks`]: crate::map::spatial::Spatial::foreign_marks
    pub(super) fn place(
        &mut self,
        c: &mut Step<'_>,
        builder: &mut Unit,
        reads: &BuildReads,
    ) -> Result<(), GenieError> {
        let desolid = Self::site_unit(c, reads)
            .ok_or(FOREIGN_SITE)?
            .construction
            .is_some_and(|k| k.desolid);
        // every set_state first takes the builder out of FUN_140aaddb0's building's builders
        self.uncount(c)?;
        self.move_to = None;
        self.goal = None;
        self.walk_target = -1;
        self.state = PLACE;
        if !desolid {
            return self.join(c, builder);
        }
        let placed = *c.w.spatial.get_known(self.target)?.ok_or(SITE_GONE)?;
        let clearing = ClearingSite {
            player: placed.player,
            master: placed.master,
            class: placed.class,
            radius: placed.radius,
            at: [placed.position[0], placed.position[1]],
            trees: true,
        };
        let cleared = footprint_clearing(&c.w.spatial, &clearing, &reads.looks)?;
        for cl in cleared.iter().filter(|cl| cl.record_goes) {
            c.w.pathing.removed(cl.id);
            c.w.spatial.drop_record(cl.id);
        }
        c.w.pathing.tiles_changed(c.w.spatial.take_changes());
        let obstructs = !matches!(placed.obstruction_type, 0 | 12);
        let occupants: Vec<i32> =
            c.w.spatial
                .footprint_occupants(self.target, &placed, obstructs, &reads.looks)?
                .into_iter()
                .filter(|&id| !cleared.iter().any(|cl| cl.id == id))
                .collect();
        for cl in cleared {
            c.w.clear_object(cl, true)?;
        }
        let others: Vec<i32> = occupants
            .iter()
            .copied()
            .filter(|&id| id != builder.id)
            .collect();
        if !others.is_empty() {
            c.w.evacuate(self.target, &others)?;
        }
        if c.w.spatial.foreign_marks(&placed) {
            // FUN_1411d4b80: a human owner's foundation goes (FUN_1411ccbb0), the interface hears
            // of it (FUN_140c898a0 0x88, not held), and set_state(2) halts the Build: the
            // standing sprite, the foundation let go, no point
            self.remove_marked(c)?;
            return self.halt_placing(c, builder, reads);
        }
        if occupants.is_empty() {
            return self.join(c, builder);
        }
        let [bx, by, _] = builder.position();
        let [sx, sy, _] = placed.position;
        let inside = (bx - sx).abs() <= placed.radius[0] && (by - sy).abs() <= placed.radius[1];
        if inside {
            return self.clear(c, builder);
        }
        // LAB_140aaf5d4: the builder waits a second (set_state 222) up to 21 times; then the
        // interface hears of it (FUN_140c898a0 0x88, not held), set_state(2) halts the Build
        // (the standing sprite, the foundation let go, no point) and the waits start again
        if MAX_WAITS <= builder.footprint_waits {
            builder.footprint_waits = 0;
            return self.halt_placing(c, builder, reads);
        }
        builder.footprint_waits += 1;
        self.state = WAIT;
        self.timer = 1.0;
        Ok(())
    }

    /// `set_state(2)` from the placement: the builder takes its standing sprite, the
    /// foundation is let go, the point is −1 (the builder counts among no builders here, its
    /// `MoveTo` went).
    ///
    /// # Errors
    ///
    /// The sprite's and the release's.
    fn halt_placing(
        &mut self,
        c: &mut Step<'_>,
        builder: &mut Unit,
        reads: &BuildReads,
    ) -> Result<(), GenieError> {
        Self::stand(c, builder)?;
        let target = std::mem::replace(&mut self.target, -1);
        crate::world::lets::release(c, builder, target, reads.by)?;
        self.point = [-1.0; 3];
        self.state = HALTED;
        Ok(())
    }

    /// `LAB_140aaf9f4`: the builder joins the work: unless walls of other players stand on both
    /// tiles beside it on the footprint's diagonal (`set_state` 2, not modelled), it counts among
    /// the builders of `FUN_140aaddb0`'s building ([`BuildAction::count`]). Its work sprite and
    /// sound are not held by the slice.
    ///
    /// # Errors
    ///
    /// A foundation gone from the map, the walls beside a diagonal builder, and the count's.
    pub(super) fn join(&mut self, c: &mut Step<'_>, builder: &Unit) -> Result<(), GenieError> {
        let placed = *c.w.spatial.get(self.target).ok_or(SITE_GONE)?;
        let [bx, by] = [builder.position()[0] as i32, builder.position()[1] as i32];
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
                c.w.spatial
                    .object_on_tile(x, y, |o| o.class == WALL && o.player != player)
            };
            if walled(a) && walled(b) {
                return Err(GenieError::unsupported(
                    "walls beside a diagonal builder (set_state 2)",
                ));
            }
        }
        if !self.added {
            Self::count(c, self.target, 1)?;
            self.added = true;
        }
        Ok(())
    }

    /// The work (state 7, `0x140ab0370` case 6): the builder adds its rate times dt to the
    /// foundation's construction: its work rate times the task's work value, times the owner's
    /// construction rate (attribute 195) when that is above 0 (the Wonder's at most 1.2), each
    /// builder after the first adding a third ([`BuildAction::work_on`]). A finished
    /// foundation with hit points then ends the work ([`BuildAction::finish`]). A farm's
    /// reseeding and the instant-build cheat (`game+0xc14`) are not modelled.
    ///
    /// # Errors
    ///
    /// Another player's foundation, and what the work and its end do not model.
    pub(super) fn work(
        &mut self,
        c: &mut Step<'_>,
        builder: &mut Unit,
        reads: &BuildReads,
    ) -> Result<(), GenieError> {
        let site = Self::site_unit(c, reads).ok_or(FOREIGN_SITE)?;
        let mut rate = self.task.work_rate * self.task.work_value * seconds(c.dt);
        let mut factor = c.w.players[c.player].construction_rate;
        if site.life.master_id == WONDER && WONDER_RATE < factor {
            factor = WONDER_RATE;
        }
        if 0.0 < factor {
            rate *= factor;
        }
        let n = f32::from(Self::builders(c, self.target)?);
        if 1.0 < n {
            rate *= ((n - 1.0) * EXTRA_BUILDER + 1.0) / n;
        }
        if self.work_on(c, builder, rate)? {
            return Ok(());
        }
        // a finished foundation with hit points ends the work (its `+0x278`, decoded); one
        // without (deleted, it dies in its own update) leaves the Build as it is
        if Self::site_unit(c, reads).is_some_and(|u| 0.0 < u.life.hp) {
            self.finish(c, builder, reads)?;
        }
        Ok(())
    }

    /// `set_state(4)` after the walk off the footprint (`0x140aae200` case 4): the goal is the
    /// foundation again and a new `MoveTo` to it (`FUN_140aa0df0`) starts arrived if the footprints
    /// touch, as [`BuildAction::start`]'s. Only a villager returns at once (`0x140ab0370` case
    /// 0xde; any other builder in range would work, state 6).
    ///
    /// # Errors
    ///
    /// A builder without movement state, one other than a villager, and a foundation gone from
    /// the map.
    pub(super) fn back(&mut self, c: &Step<'_>, builder: &Unit) -> Result<(), GenieError> {
        let mover = builder.body.mover().ok_or(BUILDER_UNMOVING)?;
        if c.w.spatial.get(builder.id).map(|p| p.class) != Some(VILLAGER) {
            return Err(GenieError::unsupported(
                "a builder other than a villager returns",
            ));
        }
        let site = c.w.spatial.get_known(self.target)?.ok_or(SITE_GONE)?;
        let ground = [mover.stats.radius[0], mover.stats.radius[1]];
        let gap = footprint_gap(builder.position(), ground, site.position, site.radius);
        *self = self.restarted(self.target, site.position, gap);
        Ok(())
    }

    /// `set_state(223)`: a quiet `MoveTo` to the free tile beside the footprint (`FUN_1411566b0`,
    /// `FUN_140aa0d30` with the task's moving sprite); it starts in state 3, as a `MoveTo` to a
    /// point is far from it (`FUN_140aa2100`).
    ///
    /// # Errors
    ///
    /// A builder without movement state, a foundation gone from the map, no free tile beside
    /// it, and a builder that cannot move.
    pub(super) fn clear(&mut self, c: &Step<'_>, builder: &Unit) -> Result<(), GenieError> {
        let mover = builder.body.mover().ok_or(BUILDER_UNMOVING)?;
        let site = c.w.spatial.get(self.target).ok_or(SITE_GONE)?;
        let at = builder.position();
        let spot =
            c.w.spatial
                .spot_beside(
                    at,
                    (builder.id, mover.stats.radius[0]),
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
        self.move_sprite = self.task.moving;
        self.walk_target = -1;
        self.goal = Some([spot[0], spot[1], at[2]]);
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
