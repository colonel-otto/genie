//! The master's default task, which an idle unit starts in its update: a bird's Fly, a farm's
//! Make or the Artifact of livestock.

use crate::GenieResult;
use crate::gathering::make::MakeAction;
use crate::movement::fly::Fly;
use crate::objects::DefaultStart;
use crate::objects::ObjectState;
use crate::objects::layers::Action;
use crate::objects::layers::DefaultTask;
use crate::objects::masters::Master;
use crate::objects::masters::Task;
use crate::objects::unit::Unit;
use crate::ownership::artifact::Artifact;
use crate::world::slots::PlayerSlotsExt;
use crate::world::step::Step;

impl Unit {
    /// `RGE_Action_Object::update` (`0x14106ac90`) on a unit in state 2 without an action or a
    /// production: the master's default task starts (slot 0x7f0, [`Unit::set_default_task`]),
    /// and its first update comes in the next update ([`DefaultStart::Started`]).
    ///
    /// # Errors
    ///
    /// As [`Unit::set_default_task`].
    pub(crate) fn start_default_task(
        &mut self,
        actions: &mut Action,
        step: &mut Step<'_>,
    ) -> GenieResult<DefaultStart> {
        if self.life.state != ObjectState::Alive
            || actions.top_action_kind().is_some()
            || step.world.parts.production.contains(self.id)
        {
            return Ok(DefaultStart::NotStarted);
        }
        self.set_default_task(actions, step)
    }

    /// Slot 0x7f0 (`FUN_14106eb40`) with the master's default task id (`+0x790`, which
    /// [`DefaultTask`] holds by the action it makes): the task the master holds by that id
    /// (`FUN_14109e0c0`, [`Master::default_task`]), nothing without one or with an action of
    /// its type on the list (`FUN_140a98740`, [`ActionList::holds`]). Else the list's factory
    /// (slot 0x10 with the task, no target and −1 thrice) makes the action, a bird's Fly
    /// ([`Fly::new`]), a farm's Make ([`MakeAction::new`], once) or livestock's Artifact
    /// ([`Artifact::new`]), and slot 0x808 installs it ([`Unit::set_only_own_action`]: the
    /// list's deletion, then its add). The list's add starts what it hands back: the Fly's
    /// first ([`Unit::start_fly`]), the Make's ([`MakeAction::first`] with its owner's
    /// attributes); the Artifact goes onto the list as it is made. A unit in state 3 or later
    /// takes none, as the add deletes it unstarted. [`DefaultStart::Started`] for a default
    /// task, whatever it made: the caller then runs no list update. A default task that the
    /// slice does not hold is an error.
    ///
    /// [`ActionList::holds`]: crate::objects::actions::ActionList::holds
    ///
    /// # Errors
    ///
    /// A default task the slice cannot start, a unit whose master or owner the world lacks, and
    /// the install's and the starts'.
    pub(crate) fn set_default_task(
        &mut self,
        actions: &mut Action,
        step: &mut Step<'_>,
    ) -> GenieResult<DefaultStart> {
        match actions.default_task {
            DefaultTask::Nothing => return Ok(DefaultStart::NotStarted),
            DefaultTask::Fails(unstartable) => return Err(unstartable.error()),
            DefaultTask::Fly => {
                if let Some(task) = self.default_task_to_make(actions, step)? {
                    let fly = Fly::new(task, self.position());
                    if let Some(fly) = self.set_only_own_action(actions, Some(fly), step)? {
                        self.start_fly(actions, step, fly)?;
                    }
                }
            }
            DefaultTask::Make => {
                // it starts once
                actions.default_task = DefaultTask::Nothing;
                if let Some(task) = self.default_task_to_make(actions, step)? {
                    let make = MakeAction::new(task);
                    if let Some(mut make) = self.set_only_own_action(actions, Some(make), step)? {
                        make.first(
                            &step.world.players.player(step.player)?.attributes,
                            &mut self.life,
                            &step.world.model.roles,
                            step.world.model.slots,
                        );
                        actions.list.set_make(Some(make));
                    }
                }
            }
            DefaultTask::Artifact => {
                if let Some(task) = self.default_task_to_make(actions, step)? {
                    let artifact = Artifact::new(step.player, Some(task));
                    if let Some(artifact) =
                        self.set_only_own_action(actions, Some(artifact), step)?
                    {
                        actions.list.set_artifact(Some(artifact));
                    }
                }
            }
        }
        Ok(DefaultStart::Started)
    }

    /// The task slot 0x7f0 makes an action of: the master's default task (`FUN_14109e0c0`),
    /// none for a master without one or a list that holds an action of its type
    /// (`FUN_140a98740`).
    fn default_task_to_make(&self, actions: &Action, step: &Step<'_>) -> GenieResult<Option<Task>> {
        let master = self.master(step.world.master_tables(), step.player)?;
        Ok(Master::default_task(master)
            .copied()
            .filter(|task| !actions.list.holds(task.action_kind)))
    }
}
