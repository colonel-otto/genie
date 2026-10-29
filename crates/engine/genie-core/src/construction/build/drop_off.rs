//! State 8: a builder that carries something walks to a drop site first ([`DropSearch`]).

use crate::GenieError;
use crate::construction::build::{
    ARRIVED, BUILDER_UNMOVING, BuildAction, BuildReads, DROP, DROP_REACH, DROP_WAIT,
    DROPPED_ORDERS, NO_TARGET_POINT,
};
use crate::movement::move_to::{
    Goal, MoveTo, MoveWait, Sprites, start_walk_shown, walk_or_give_up,
};
use crate::movement::{Me, PathAsk, footprint_gap};
use crate::objects::lifecycle::seconds;
use crate::objects::sprite::sprite_id;
use crate::objects::teardown::HeadCode;
use crate::objects::unit::Unit;
use crate::orders::action_state::DONE;
use crate::unit_ai::ActionKind;
use crate::unit_ai::task_finder::Actor;
use crate::world::others::OthersRef;
use crate::world::step::Step;

/// What the drop-off's search gives `set_state(8)` ([`BuildAction::set_drop`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DropSearch {
    /// The builder carries nothing (`+0x310` at most 0): no search.
    Empty,
    /// The drop-site table has no drop site for its load (`FUN_141658e20`).
    Nothing,
    /// The drop site the table gives, and the range and sprite of the `MoveTo` to it: those of
    /// the task the builder's slot 0x820 finds on it (any action type, no switch: its `+0x30`
    /// and its carrying sprite `+0x58`), else the Build task's.
    Site {
        /// The drop site.
        id: i32,
        /// The `MoveTo`'s range.
        range: f32,
        /// The found task's carrying sprite, None without the task or the sprite.
        sprite: Option<i16>,
    },
}

impl BuildAction {
    /// `set_state(8)` (`0x140aae200` case 8) from the update (case 7 searching again): the
    /// drop-site table's nearest drop site for what the unit carries
    /// ([`crate::orders::nearest_table_site`] over its player's objects; the sleeping ones,
    /// static only, are never drop sites) and the task its slot 0x820 finds on it
    /// ([`task_search`] with any action type, no switch: its range, else the Build task's),
    /// unless it carries nothing (COMISS: a NaN load neither); then [`BuildAction::set_drop`].
    ///
    /// # Errors
    ///
    /// A builder without its master, and what the search and `set_drop` do not model.
    ///
    /// [`task_search`]: crate::unit_ai::task_finder::world::task_search
    pub(crate) fn drop_again(
        &mut self,
        c: &mut Step<'_>,
        builder: &mut Unit,
        by: crate::objects::release::Releaser,
    ) -> Result<(), GenieError> {
        let search = if 0.0 < builder.life.amount {
            Self::drop_search(c, builder, self.task.range)?
        } else {
            DropSearch::Empty
        };
        self.set_drop(c, builder, search, by)
    }

    /// The drop site the drop-off walks to ([`BuildAction::drop_again`]), with `task_range`
    /// the Build task's range for none found on it.
    fn drop_search(
        c: &Step<'_>,
        builder: &Unit,
        task_range: f32,
    ) -> Result<DropSearch, GenieError> {
        let view = OthersRef::of(&c.w.players, c.player);
        let masters =
            c.w.masters
                .get(c.player)
                .ok_or(crate::world::OBJECT_MASTER)?;
        let own = builder.master_in(Some(masters))?;
        let at = [builder.position()[0], builder.position()[1]];
        let look = |id: i32| view.own(id).map(|u| (u.life.state, u.position()));
        let found = crate::orders::nearest_table_site(
            (
                &c.w.drop_sites,
                c.w.players[c.player].civ,
                &c.w.players[c.player].tracker,
            ),
            (at, builder.life.attribute_type, own.class),
            look,
        );
        let Some(id) = found else {
            return Ok(DropSearch::Nothing);
        };
        let (player, foreign) = (c.player, view.players());
        let find = |t: i32| {
            if t == builder.id {
                return Some((player, builder));
            }
            view.find(t)
        };
        let every = &c.w.masters;
        let task = match (
            crate::unit_ai::task_finder::world::task_target_with(id, 1, &find, every)?,
            crate::unit_ai::task_finder::world::task_target_with(id, 2, &find, every)?,
        ) {
            (Some(t), Some(general)) => {
                use crate::unit_ai::task_finder::world::SideWorld;
                use crate::unit_ai::task_finder::world::task_search;
                let actor = Actor {
                    id: builder.id,
                    master: own,
                    player,
                    held: (builder.life.attribute_type, builder.life.amount),
                    attribute_95: c.w.players[c.player]
                        .attributes
                        .get(95)
                        .copied()
                        .unwrap_or(0.0),
                    civ: c.w.players[c.player].civ,
                };
                let dip = SideWorld {
                    player,
                    relations: &c.w.players[c.player].relations,
                    foreign,
                    drop_sites: &c.w.drop_sites,
                };
                // slot 0x820's point: x, y and z −1, no terrain
                task_search(masters, &actor, &t, &general, None, -1, &dip)?
            }
            _ => None,
        };
        let range = task.map_or(task_range, |(t, _)| t.work_range);
        let sprite = task.and_then(|(t, _)| sprite_id(t.sprites[3]));
        Ok(DropSearch::Site { id, range, sprite })
    }

    /// `set_state(8)` (`0x140aae200` case 8) with what the search found (`search`), the builder
    /// `by` letting its targets go: the prelude ([`BuildAction::leave`]: out of the builders,
    /// the `MoveTo`'s teardown stops the unit), then a builder that carries nothing walks to its
    /// foundation (`set_state(4)`, [`BuildAction::restart`]). Else the drop-site table's nearest
    /// drop site for its load (`FUN_141658e20`) becomes the second target at once
    /// (`FUN_140a7a190`: the old one let go, slot 0x180, the new one taken, slot 0x178); with
    /// none the unit tells its unit AI (notify 0x203, the Build's action value) and walks to its
    /// foundation (`set_state(4)`); with one a quiet `MoveTo` (`+0x61` 1) to it within the
    /// search's range starts (`FUN_140aa0df0` with its point the site's place and the found
    /// task's carrying sprite, else the Build task's moving one): arrived within 0.01 of its
    /// footprint (`FUN_140aa2100`), in state 13 for a unit without speed.
    ///
    /// # Errors
    ///
    /// A builder without movement state, a drop site or foundation without a place.
    pub(crate) fn set_drop(
        &mut self,
        c: &mut Step<'_>,
        builder: &mut Unit,
        search: DropSearch,
        by: crate::objects::release::Releaser,
    ) -> Result<(), GenieError> {
        self.leave(c, builder)?;
        self.state = DROP;
        let found = match search {
            DropSearch::Empty => return self.restart(c, builder, None),
            DropSearch::Nothing => None,
            DropSearch::Site { id, range, sprite } => {
                self.move_sprite = sprite.or(self.task.moving);
                Some((id, range))
            }
        };
        let new = found.map_or(-1, |(id, _)| id);
        if self.site != new {
            let old = std::mem::replace(&mut self.site, new);
            if old != -1 {
                crate::world::lets::release(c, builder, old, by)?;
            }
            if new != -1 {
                crate::world::lets::take(c, builder, new, by);
            }
        }
        let Some((id, range)) = found else {
            builder.post_own(
                crate::unit_ai::NO_DROP_SITE,
                [ActionKind::Build.value(), 0, 0],
            )?;
            return self.restart(c, builder, None);
        };
        let mover = builder.body.mover().ok_or(BUILDER_UNMOVING)?;
        let at = builder.position();
        let to =
            c.w.spatial
                .get_known(id)?
                .ok_or(GenieError::unsupported("a drop site without a place"))?;
        let ground = [mover.stats.radius[0], mover.stats.radius[1]];
        let gap = footprint_gap(at, ground, to.position, to.radius);
        let moves = mover.stats.speed != 0.0;
        self.drop_range = range;
        self.walk_target = id;
        self.goal = Some(to.position);
        self.move_wait = MoveWait::default();
        self.move_to = Some(if gap <= ARRIVED {
            MoveTo::Arrived
        } else if moves {
            MoveTo::Start
        } else {
            MoveTo::Failed
        });
        Ok(())
    }

    /// State 8's update (`0x140ab0370` case 7). While its timer is below 0 (COMISS: a NaN runs
    /// it out) it runs by dt; at its end the drop-off searches again (`set_state(8)`,
    /// [`BuildAction::drop_again`]). Else its quiet `MoveTo` walks to the drop site
    /// ([`BuildAction::walk_to_drop`]): its target in a state above 6 (code 5), a failed walk
    /// (codes 3 and 4) and a drop site gone or not in state 2 search again. At its end (codes 1
    /// and 2: the list deletes the `MoveTo`, whose destructor stops the unit) a drop site that
    /// does not take the load (`FUN_140fdd190`) halts the Build (`set_state(2)`); one beyond
    /// [`DROP_REACH`] (`FUN_14118f8c0`) makes it wait 3 seconds with the unit's animation stopped
    /// (`FUN_141161850`). Else the load goes to the player (slot 0xf8 with 1 and 1, the resource
    /// of what it carries, `FUN_140fdd4f0`, [`BuildAction::drop_load`]; its sound, slot 0x4c0,
    /// and the statistics are not held), the unit carries nothing (`FUN_14119d310`) and the
    /// Build goes on ([`BuildAction::after_deposit`]).
    ///
    /// # Errors
    ///
    /// What the walk, the search and the states do not model.
    pub(super) fn update_drop(
        &mut self,
        c: &mut Step<'_>,
        builder: &mut Unit,
        me: Me,
        reads: &BuildReads,
    ) -> Result<HeadCode, GenieError> {
        if crate::math::fmath::below_or_unordered(self.timer, 0.0) {
            self.timer += seconds(c.dt);
            if self.timer < 0.0 {
                return Ok(HeadCode::Keeps);
            }
            self.timer = 0.0;
            self.drop_again(c, builder, reads.by)?;
            return Ok(HeadCode::Keeps);
        }
        // the quiet MoveTo lets a target in a state above 6 go (slot 0x68 with none): code 5
        if self.move_to.is_some()
            && self.walk_target != -1
            && reads.walk_target_state.is_some_and(|s| 6 < s)
        {
            self.walk_target = -1;
            self.drop_again(c, builder, reads.by)?;
            return Ok(HeadCode::Keeps);
        }
        if self.walk_to_drop(c, builder, me, reads)? {
            return Ok(HeadCode::Keeps);
        }
        if self.move_to.take().is_some()
            && let Some(m) = builder.body.mover_mut()
        {
            m.stop()?;
        }
        let Some((2, drop)) = reads.deposit.filter(|_| self.site != -1) else {
            self.drop_again(c, builder, reads.by)?;
            return Ok(HeadCode::Keeps);
        };
        if !drop.takes {
            self.halt_in(c, builder, reads)?;
            return Ok(HeadCode::Keeps);
        }
        if DROP_REACH < drop.gap_squared {
            self.timer = DROP_WAIT;
            builder.body.set_anim_speed(0.0);
            return Ok(HeadCode::Keeps);
        }
        let attributes = c.w.players[c.player].attributes.len();
        let kind = crate::gathering::hunt::resource_kind(builder.life.attribute_type, attributes);
        let amount = builder.life.amount;
        Self::drop_load(c, builder, kind, amount)?;
        self.after_deposit(c, builder, reads)
    }

    /// State 8's quiet `MoveTo` to the drop site (its target, else its point): its start asks
    /// for a path within the drop-off range; on its way a drop site that left the `MoveTo`'s
    /// point makes it ask again, else it walks; the motion step giving up makes it wait or fail
    /// ([`MoveWait::give_up`]); a waiting one asks again at its delay; a failed one searches
    /// again ([`BuildAction::drop_again`]). Its start shows its sprite
    /// ([`BuildAction::move_shown`]) as it turns to state 4. True while it walks, waits or
    /// failed (the update ends there); false at its end (codes 1 and 2) or without it.
    fn walk_to_drop(
        &mut self,
        c: &mut Step<'_>,
        builder: &mut Unit,
        me: Me,
        reads: &BuildReads,
    ) -> Result<bool, GenieError> {
        let (waits, stance) = Self::gives_up(c, builder, reads);
        let to = if self.walk_target == -1 {
            Goal::Point(self.goal.unwrap_or(NO_TARGET_POINT))
        } else {
            Goal::Target(self.walk_target)
        };
        match self.move_to {
            Some(MoveTo::Start) => {
                let shown = self.move_shown(reads.defaults);
                let (walk, beside) = c.w.walk_beside(c.pace(), &mut builder.body, me);
                let mut sprites = Sprites {
                    sprite: &mut builder.sprite,
                    rng: beside.sprite_rng,
                    graphics: beside.graphics,
                };
                let shown = sprites.shown(shown)?;
                self.asked_again();
                self.move_to = Some(MoveTo::Moving);
                let range = self.drop_range;
                let ask = PathAsk::default();
                self.move_to = Some(
                    match start_walk_shown(walk, (to, range, true), ask, Some(shown))? {
                        Some(state) => state,
                        None => self.move_wait.give_up(waits, stance),
                    },
                );
            }
            Some(MoveTo::Moving) => {
                let mut walk = c.walk(&mut builder.body, me);
                if self.walk_target != -1
                    && crate::movement::move_to::target_moved(&mut walk, self.walk_target, true)?
                        .again()
                {
                    self.move_to = Some(MoveTo::Start);
                    return Ok(true);
                }
                self.move_to = Some(match walk_or_give_up(walk.reborrow())? {
                    Some(state) => state,
                    None => self.move_wait.give_up(waits, stance),
                });
            }
            Some(MoveTo::Waiting) => {
                self.move_wait.timer += seconds(c.dt);
                if self.move_wait.delay <= self.move_wait.timer {
                    self.move_wait.timer = 0.0;
                    self.move_to = Some(MoveTo::Start);
                }
            }
            Some(MoveTo::Failed) => self.drop_again(c, builder, reads.by)?,
            Some(MoveTo::Arrived) | None => return Ok(false),
        }
        Ok(true)
    }

    /// State 8 after the load went to the player: a Build with a foundation lets the drop site
    /// go (`FUN_140a7a190` with none: its slot 0x180) and walks back to it (`set_state(4)`).
    /// Without one its unit AI drops its orders on targets gone (`FUN_140c5de60`); with orders
    /// left it has no order and no action and the Build is done (`set_state(1)`, the update's
    /// 1); with none it waits (`set_state(3)`). A unit without a unit AI and a point of −1 is
    /// done (a computer player's) or halts; with a point it walks there (`set_state(11)`).
    fn after_deposit(
        &mut self,
        c: &mut Step<'_>,
        builder: &mut Unit,
        reads: &BuildReads,
    ) -> Result<HeadCode, GenieError> {
        if self.target != -1 {
            let site = std::mem::replace(&mut self.site, -1);
            if site != -1 {
                crate::world::lets::release(c, builder, site, reads.by)?;
            }
            let state = Self::site_unit(c, reads).map(|s| s.life.state);
            self.restart(c, builder, state)?;
            return Ok(HeadCode::Keeps);
        }
        if let Some(ai) = builder.ai.as_mut() {
            let gone = &reads.gone_orders;
            ai.orders
                .retain(|o| !(DROPPED_ORDERS.contains(&o.order) && gone.contains(&o.target)));
            if !ai.orders.is_empty() {
                ai.end_order_and_action()?;
                self.end(c, builder, DONE)?;
                return Ok(HeadCode::Deletes);
            }
            self.idle(c, builder, reads)?;
            return Ok(HeadCode::Keeps);
        }
        // UCOMISS against −1 (DAT_14330ec20): a NaN is not
        if self.point[0] == -1.0 || self.point[1] == -1.0 {
            if c.w.players[c.player].computer {
                self.end(c, builder, DONE)?;
            } else {
                self.halt_in(c, builder, reads)?;
            }
            return Ok(HeadCode::Keeps);
        }
        self.walk_to_point(c, builder)?;
        Ok(HeadCode::Keeps)
    }
}
