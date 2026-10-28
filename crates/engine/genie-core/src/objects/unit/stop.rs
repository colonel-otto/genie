//! A unit's stops: its actions torn down or cleared, its standing sprite, an action's effects
//! applied.

use crate::GenieError;
use crate::math::rng::{Stream, Well512};
use crate::objects::masters::Masters;
use crate::objects::release::Releaser;
use crate::objects::sprite::Graphic;
use crate::objects::unit::Unit;
use crate::unit_ai::Effect;
use crate::unit_ai::world_view::AiWork;
use crate::world::context::Context;

impl Unit {
    /// stopAction (slot 0x3c0, `FUN_14108c160`): the action list goes ([`Unit::clear_own_actions`])
    /// and a unit in state 2 takes its standing sprite (slot 0x68 with slot 0x798's,
    /// [`crate::combat::attack::set_sprite`]) where the slice holds its sprite.
    pub(crate) fn stop_action(&mut self, c: &mut Context<'_>) -> Result<(), GenieError> {
        let by = self.releaser(c);
        let sprite = c.w.streams.get(Stream::Sprite);
        let (mut releases, mut leaves) = (Vec::new(), Vec::new());
        self.stop_with(
            (&mut releases, &mut leaves),
            by,
            c.w.masters.get(c.player),
            &c.w.graphics,
            sprite,
        )?;
        crate::world::lets::torn_down(c, (leaves, releases))
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
        if let Some(m) = self.body.mover_mut() {
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

    /// The action list's deletion (`FUN_140a98620`, [`ActionList::delete_all`]), the one every
    /// caller shares (the unit's death, its stops and commands,
    /// [`crate::world::World::clear_actions`]): each action's destructor
    /// ([`crate::objects::actions::Action::tear_down`]) in the list's order. The targets they let
    /// go go into `releases` with the unit as `by`, the buildings a counted builder or repairer
    /// leaves into `builder_leaves` ([`crate::world::World::leave_builders`]). A building's
    /// production is a list of its own, which stays.
    ///
    /// [`ActionList::delete_all`]: crate::objects::actions::ActionList::delete_all
    ///
    /// # Errors
    ///
    /// The unit's stop that the motion step does not model, and the list's deletion's.
    pub(crate) fn clear_own_actions(
        &mut self,
        releases: &mut Vec<(i32, Releaser)>,
        builder_leaves: &mut Vec<i32>,
        by: Releaser,
    ) -> Result<(), GenieError> {
        let torn = self.actions.delete_all(&mut self.body, self.life.state)?;
        releases.extend(torn.releases.into_iter().map(|t| (t, by)));
        builder_leaves.extend(torn.builder_leaves);
        Ok(())
    }

    /// `FUN_140a98e20` on the unit's own list ([`ActionList::delete_head`]): the head's
    /// destructor, what it lets go applied at once
    /// ([`crate::objects::teardown::TornDown::let_go`]).
    ///
    /// [`ActionList::delete_head`]: crate::objects::actions::ActionList::delete_head
    ///
    /// # Errors
    ///
    /// The deletion's, and what it lets go.
    pub(crate) fn delete_own_head(&mut self, c: &mut Context<'_>) -> Result<(), GenieError> {
        let by = self.releaser(c);
        let torn = self.actions.delete_head(&mut self.body, self.life.state)?;
        torn.let_go(c, by)
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
                    c.w.spatial.set_gait(self.id, self.gait());
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
