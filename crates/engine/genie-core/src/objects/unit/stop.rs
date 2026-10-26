//! A unit's stops: its actions torn down or cleared, its standing sprite, an action's effects
//! applied.

use crate::math::rng::{Stream, Well512};
use crate::objects::masters::Masters;
use crate::objects::sprite::Graphic;
use crate::objects::unit::Unit;
use crate::turn::world::Releaser;
use crate::unit_ai::world_view::{AiWork, Context};
use crate::unit_ai::{Effect, GenieError};

impl Unit {
    /// stopAction (slot 0x3c0, `FUN_14108c160`): the action list goes ([`Unit::clear_own_actions`])
    /// and a unit in state 2 takes its standing sprite (slot 0x68 with slot 0x798's,
    /// [`crate::combat::attack::set_sprite`]) where the slice holds its sprite.
    pub(crate) fn stop_action(&mut self, c: &mut Context<'_>) -> Result<(), GenieError> {
        let by = self.releaser(c);
        let sprite = c.streams.get(Stream::Sprite);
        let lets = (&mut c.releases, &mut c.builder_leaves);
        self.stop_with(lets, by, c.masters, c.graphics, sprite)
    }

    /// [`Unit::stop_action`] on the parts of the world it changes: the targets its actions let
    /// go and the foundations its Build leaves (`by` the unit, [`Unit::clear_own_actions`]), its
    /// masters and sprites, and the Sprite stream.
    pub(crate) fn stop_with(
        &mut self,
        (releases, builder_leaves): (&mut Vec<(i32, Releaser)>, &mut Vec<i32>),
        by: Releaser,
        masters: Option<&Masters>,
        graphics: &std::collections::BTreeMap<i16, Graphic>,
        sprite: &mut Well512,
    ) -> Result<(), GenieError> {
        self.tear_down_actions(releases, builder_leaves, by)?;
        if self.life.state == 2 {
            self.show_standing(masters, graphics, sprite)?;
        }
        Ok(())
    }

    /// The action list's clearing (`FUN_140a98620`) as the unit's own commands and stops run it
    /// ([`Unit::clear_own_actions`]); an action the slice does not hold has a teardown it cannot
    /// run.
    pub(crate) fn tear_down_actions(
        &mut self,
        releases: &mut Vec<(i32, Releaser)>,
        builder_leaves: &mut Vec<i32>,
        by: Releaser,
    ) -> Result<(), GenieError> {
        if self.unmodelled_action.is_some() {
            return Err(GenieError::unsupported(
                "a unit AI's stop of an action list the slice does not model",
            ));
        }
        self.clear_own_actions(releases, builder_leaves, by)
    }

    /// The unit's slot 0x68 with its master's standing sprite (slot 0x798's, the master's
    /// `+0x470`): its top speed becomes the standing sprite's ([`Mover::shows_standing`]) and,
    /// where the slice holds its sprite, the sprite starts ([`crate::combat::attack::set_sprite`] on the
    /// Sprite stream). Its master is its own or its owner's (`masters`).
    ///
    /// [`Mover::shows_standing`]: crate::movement::Mover::shows_standing
    pub(crate) fn show_standing(
        &mut self,
        masters: Option<&Masters>,
        graphics: &std::collections::BTreeMap<i16, Graphic>,
        sprite: &mut Well512,
    ) -> Result<(), GenieError> {
        if let Some(m) = self.mover.as_mut() {
            m.shows_standing();
        }
        if self.sprite.is_some() {
            let standing = self
                .own_master
                .as_deref()
                .or_else(|| masters.and_then(|m| m.get(self.life.master_id)))
                .map(|m| m.standing_sprite)
                .and_then(|id| graphics.get(&id).copied())
                .ok_or(GenieError::unsupported(
                    "a stopped unit without its standing sprite",
                ))?;
            crate::combat::attack::set_sprite(&mut self.sprite, standing, sprite);
        }
        Ok(())
    }

    /// The action list's deletion (`FUN_140a98620`), the one every caller shares (the unit's
    /// death, its stops and commands, [`crate::turn::Turns::clear_actions`]): each action's
    /// destructor. A `MoveTo`'s (`FUN_140aa1f30`, an action's sub-action's too) stops the unit
    /// (slot 0x5d0). The base teardown (`FUN_140a73fc0`) lets the target go (slot 0x180) unless
    /// the action is quiet (`+0x60`; the Hunt's Attack is, so only the Hunt lets its target go),
    /// then the second target, into `releases` with the unit as `by`. The Build's
    /// (`FUN_140aadbf0`) first takes a counted builder (`+0x68`) out of its building's builders
    /// (`FUN_140aaddb0`, `+0x9e8`), its foundation into `builder_leaves`
    /// ([`crate::turn::Turns::leave_builders`]), as the Repair's (`FUN_140ad8fd0`) a counted
    /// repairer (`+0x6c`) its target's; the Artifact's (`FUN_140aab0d0`) lets its
    /// target go and its `MoveTo`'s stops the unit; the Fly's (`FUN_140a84750`) gives the unit
    /// its top speed as its animation speed (slot 0x208 into `+0x7b8`); the missile's lets its
    /// target and the object it was fired from go. The Make, the wait for a foundation
    /// (`0x140ae8780`) and the Explore (`0x140a87dd0`) hold no target. A building's production
    /// is a list of its own, which stays. Of the list's deletion itself, the next action's slot
    /// 0x28 for a unit below state 3 needs a second action, which the slice never holds.
    ///
    /// # Errors
    ///
    /// The unit's stop that the motion step does not model.
    pub(crate) fn clear_own_actions(
        &mut self,
        releases: &mut Vec<(i32, Releaser)>,
        builder_leaves: &mut Vec<i32>,
        by: Releaser,
    ) -> Result<(), GenieError> {
        let mut release = |t: i32| {
            if t != -1 {
                releases.push((t, by));
            }
        };
        self.wait_solidify = None;
        self.make = None;
        if let Some(mut a) = self.artifact.take() {
            let (released, gone) = a.tear_down();
            if let Some(t) = released {
                release(t);
            }
            if let Some(t) = gone.released {
                release(t);
            }
            self.stop_if_moving(gone.stops)?;
        }
        if let Some(f) = self.fly.take() {
            self.anim_speed = Some(self.mover.as_ref().map_or(0.0, |m| m.stats.speed));
            self.stop_if_moving(f.move_to.is_some())?;
        }
        if let Some(e) = self.enter.take() {
            self.stop_if_moving(e.move_to.is_some())?;
            release(e.target);
        }
        // the Transport's teardown: its MoveTo's stops the unit; it holds no target
        let moves = self.transport.take().is_some_and(|t| t.move_to.is_some());
        self.stop_if_moving(moves)?;
        if let Some(b) = self.build.take() {
            if b.added {
                builder_leaves.push(b.target);
            }
            release(b.target);
            self.stop_if_moving(b.move_to.is_some())?;
        }
        // the Repair's destructor (FUN_140ad8fd0): a counted repairer leaves its target's
        // repairers, then the base teardown's sub-actions and target
        if let Some(r) = self.repair.take() {
            if r.counted {
                builder_leaves.push(r.target);
            }
            release(r.target);
            self.stop_if_moving(r.move_to.is_some())?;
        }
        let moves = self.explore.take().is_some_and(|e| e.move_to.is_some());
        self.stop_if_moving(moves)?;
        if let Some(mut h) = self.hunt.take() {
            h.tear_down(self.mover.as_mut())?;
            release(h.target);
            release(h.carry_to);
        }
        if let Some(mut g) = self.gather.take() {
            g.tear_down(self.mover.as_mut())?;
            release(g.target);
            release(g.carry_to);
        }
        if let Some(mut a) = self.attack.take() {
            a.tear_down(self.mover.as_mut())?;
            release(a.target);
            release(a.target2);
        }
        if let Some(old) = self.move_to.take() {
            self.stop_if_moving(true)?;
            release(old.target);
        }
        if let Some(a) = self.missile.as_mut().and_then(|m| m.action.take()) {
            release(a.target);
            release(a.fired_from);
        }
        Ok(())
    }

    /// [`Unit::clear_own_actions`]'s stop (slot 0x5d0) for a torn-down action's `MoveTo`, when
    /// it had one (`moves`) and the unit moves at all.
    ///
    /// # Errors
    ///
    /// The unit's stop that the motion step does not model.
    pub(super) fn stop_if_moving(&mut self, moves: bool) -> Result<(), GenieError> {
        if moves && let Some(m) = self.mover.as_mut() {
            m.stop()?;
        }
        Ok(())
    }

    /// Its unit AI's effects, in order.
    pub(crate) fn apply(
        &mut self,
        effects: Vec<Effect>,
        class: Option<i16>,
        c: &mut Context<'_>,
    ) -> Result<(), GenieError> {
        for e in effects {
            match e {
                Effect::Stop => self.stop_action(c)?,
                Effect::CapHumanOrder(cap) => {
                    self.last_human_order = self.last_human_order.min(cap);
                    c.spatial.set_gait(self.id, self.gait());
                }
                Effect::Move(_) | Effect::RestorePatrol => {
                    let class = class.ok_or(GenieError::unsupported(
                        "an AI move of a master the slice does not have",
                    ))?;
                    let m = match e {
                        Effect::Move(m) => Some(m),
                        _ => self.restore_patrol(),
                    };
                    if let Some(m) = m {
                        self.ai_move(m, class, c)?;
                    }
                }
                Effect::MoveAside(m) => c.move_aside.push(m),
                Effect::OrderOnTarget(order) => {
                    if c.order_on_target.replace((order, None)).is_some() {
                        return Err(GenieError::unsupported(
                            "two orders on targets in one update",
                        ));
                    }
                }
                Effect::AfterOrder(after) => {
                    let (_, tail) = c.order_on_target.as_mut().ok_or(GenieError::unsupported(
                        "a unit AI's tail after an order it did not take",
                    ))?;
                    *tail = Some(after);
                }
                Effect::HuntObject(target) => c.works.push(AiWork::Hunt(target)),
                Effect::GatherObject(target) => c.works.push(AiWork::Gather(target)),
                Effect::WorkOn { target, point } => {
                    // slot 0x7e8 with the carried-resource test, any task, +0xa14 0
                    c.works.push(AiWork::Work(crate::orders::Work {
                        target,
                        point,
                        carry_test: true,
                        filter: -1,
                        notified: false,
                    }));
                }
                Effect::BuildObject(target) => c.works.push(AiWork::Build(target)),
                Effect::DropElsewhere { site, back } => {
                    c.works.push(AiWork::DropElsewhere(site, back));
                }
                Effect::ChangeOwner(new) => self.ai_change_owner(new, c)?,
                Effect::Explore(point) => self.explore_to(point, c)?,
                Effect::MoveToObject {
                    target,
                    state,
                    range,
                } => self.move_to_object((target, state), range, c)?,
            }
        }
        Ok(())
    }
}
