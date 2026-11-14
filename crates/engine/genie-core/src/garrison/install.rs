//! The Enter's install (slot 0x808, or the Artifact's sub-list under one on top) and its start
//! there: the failure inside an object, the walk to a target without room, the enter or the walk.

use crate::garrison::ContainerId;
use crate::garrison::EnterSlot;
use crate::garrison::EnterState;
use crate::garrison::{EnterAction, Walking};
use crate::map::Point;
use crate::math::rng::Stream;
use crate::movement::move_to::MoveToReads;
use crate::objects::ListIndex;
use crate::objects::ObjectId;
use crate::objects::masters::OwnerMasters;
use crate::objects::masters::PlayersMastersExt;
use crate::objects::masters::Task;
use crate::objects::teardown::TornWalk;
use crate::orders::object_gone;
use crate::ownership::artifact::ArtifactSub;
use crate::players::PlayerId;
use crate::unit_ai::AiAction;
use crate::unit_ai::{Notify, NotifyKind};
use crate::world::World;
use crate::world::slots::PlayerSlotsExt;
use crate::{GenieError, GenieResult};

impl World {
    /// The Enter's install (slot 0x808): the action list empties and the Enter starts (slot 5,
    /// `FUN_140a86f20`): a unit inside an object fails it (`set_state(13)`, its notify for the
    /// unit's own AI the answer, [`World::install_enter_as`]); a target without room walks to its
    /// place ([`World::enter_without_room`]); else its `set_state(4)`
    /// ([`EnterAction::walk_again`]): a unit in reach enters at once ([`World::enter_now`]),
    /// else it walks, a transport to the point first ([`World::move_transport`]).
    pub(super) fn install_enter(
        &mut self,
        player: PlayerId,
        idx: ListIndex,
        target: ContainerId,
        action: EnterAction,
        task: &Task,
    ) -> GenieResult<Option<Notify>> {
        self.install_enter_as(player, idx, target, action, task, EnterSlot::Own)
    }

    /// [`World::install_enter`], or with `sub` the Enter as the sub-action of the Artifact on top
    /// (`FUN_1410847e0`): the Artifact's sub-action goes (`FUN_140a98e20`,
    /// [`crate::ownership::artifact::Artifact::hold_enter`]: an Enter lets its target go, slot 0x180
    /// ([`World::release`]), and a walking `MoveTo`'s teardown stops the unit) and the Enter
    /// starts in its list (`FUN_140a977e0`), the Artifact's update running it from then on.
    /// Returns the notify a unit inside an object posts its own unit AI (0x1f9 with the Enter's
    /// action value), which the caller posts: the unit AI that gave the enter command holds it
    /// out of the unit while it runs.
    pub(super) fn install_enter_as(
        &mut self,
        player: PlayerId,
        idx: ListIndex,
        target: ContainerId,
        mut action: EnterAction,
        task: &Task,
        slot: EnterSlot,
    ) -> GenieResult<Option<Notify>> {
        let id = self.players.live(player, idx)?.id;
        if slot == EnterSlot::UnderArtifact {
            self.enter_under_artifact(player, idx, id, action)?;
        } else {
            self.clear_actions(player, idx)?;
        }
        if self.players.live(player, idx)?.life.inside.is_some() {
            return self.enter_from_inside(player, idx, action, slot).map(Some);
        }
        let (tp, unit) = self.object(target).ok_or_else(|| object_gone(target))?;
        if !self.has_room(tp, unit, Some(id))? {
            let meet = unit.position().ground();
            return self
                .enter_without_room(player, idx, action, meet, slot)
                .map(|()| None);
        }
        let (enter_target, start) = self.entering(player, idx, target, task.work_range)?;
        let walking = self.enter_walk_again(player, idx, &mut action, &enter_target, start)?;
        match walking {
            // the enter's set_state(1) reads the Enter as the unit's; a sub-action goes back
            Walking::Enter => {
                self.parts
                    .actions
                    .of_mut(self.players.live(player, idx)?.id)?
                    .list
                    .set_enter(Some(action));
                let entered = self.enter_now(player, idx, target);
                if slot == EnterSlot::UnderArtifact {
                    let actions = self.parts.actions.of_mut(id)?;
                    let take_enter = actions.list.take_enter();
                    if let (Some(enter_action), Some(art)) =
                        (take_enter, actions.list.artifact_mut())
                    {
                        art.sub = Some(ArtifactSub::Enter {
                            enter: enter_action,
                        });
                    }
                }
                entered.map(|()| None)
            }
            Walking::Walk(to) => {
                self.parts.actions.of_mut(id)?.place_enter(action, slot);
                to.map_or(Ok(()), |to| self.move_transport(Some(target), to))
                    .map(|()| None)
            }
        }
    }

    /// [`World::install_enter_as`] into a target without room for unit `idx` of `player`: the
    /// player's message (`FUN_140a864d0`, through the screen's `FUN_140c85f00`: nothing
    /// simulated), then `set_state(11)` ([`EnterAction::walk_to_point`]): `FUN_140a85f30`, whose
    /// room test answers as the start's, meets the target at its place `meet`, and the Enter
    /// `action` walks there. It goes into its `slot`.
    ///
    /// # Errors
    ///
    /// A unit the list does not hold, whose master the world lacks, and the walk's.
    fn enter_without_room(
        &mut self,
        player: PlayerId,
        idx: ListIndex,
        mut action: EnterAction,
        meet: Point,
        slot: EnterSlot,
    ) -> GenieResult<()> {
        let unit = self.players.live(player, idx)?;
        let reads = MoveToReads::of(
            unit,
            self.top_speed(player, unit)?,
            unit.master(self.master_tables(), player)?,
            &self.model.graphics,
            self.parts.combat_of(unit.id).abilities.look.as_ref(),
            &self.model.roles,
        );
        action.walk_to_point(meet, &reads, &self.model.tuning.move_to)?;
        let id = unit.id;
        self.parts.actions.of_mut(id)?.place_enter(action, slot);
        Ok(())
    }

    /// [`World::install_enter_as`] under the Artifact on top of unit `idx` of `player` (its id
    /// `id`): the Artifact's sub-action goes for the Enter `action`
    /// ([`crate::ownership::artifact::Artifact::hold_enter`]): an old Enter lets its target go
    /// (slot 0x180, [`World::release`]), then a walking `MoveTo`'s teardown stops the unit.
    fn enter_under_artifact(
        &mut self,
        player: PlayerId,
        idx: ListIndex,
        id: ObjectId,
        action: EnterAction,
    ) -> GenieResult<()> {
        let by = self.releaser(player, idx)?;
        // `EnterSlot::under` read the Artifact on top before the task search, which takes no
        // action of the unit
        let art =
            self.parts
                .actions
                .of_mut(id)?
                .list
                .artifact_mut()
                .ok_or(GenieError::unreachable(
                    "an Artifact on top without its action",
                ))?;
        let gone = art.hold_enter(action);
        // the old Enter's teardown lets its target go before its MoveTo's stops the unit
        if let Some(object_id) = gone.released {
            self.release(object_id, by)?;
        }
        let unit = self.players.live(player, idx)?;
        if gone.walk == TornWalk::Stops
            && let Some(mover) = self.parts.mover_mut(unit.id)
        {
            mover.stop()?;
        }
        Ok(())
    }

    /// [`World::install_enter_as`] for unit `idx` of `player` inside an object: the Enter
    /// `action` fails at once (`set_state(13)`): the unit takes its master's standing sprite
    /// (slot 0x68 with `+0x470`) and, its `+0x60` 0 in a new Enter, posts notify 0x1f9 with the
    /// enter's action value to its unit AI, which this returns; the update returns 3 then.
    fn enter_from_inside(
        &mut self,
        player: PlayerId,
        idx: ListIndex,
        mut action: EnterAction,
        slot: EnterSlot,
    ) -> GenieResult<Notify> {
        action.state = EnterState::Failed;
        let masters = OwnerMasters::new(&self.parts.own_master, self.model.masters.of(player));
        let unit = self.players.live_mut(player, idx)?;
        self.parts
            .actions
            .of_mut(unit.id)?
            .place_enter(action, slot);
        unit.shows_standing_sprite(
            self.parts.motion.layer_mut(unit.id),
            masters,
            &self.model.graphics,
            self.streams.get_mut(Stream::Sprite),
            self.model.tuning.sprite,
        )?;
        Ok(Notify::own(
            unit.id,
            NotifyKind::ACTION_FAILED,
            [AiAction::Enter.value(), 0, 0],
        ))
    }
}
