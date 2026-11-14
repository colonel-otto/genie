//! The master's default task, which an idle unit starts in its update (slot 0x7f0): a bird's
//! Fly, a farm's Make or the Artifact of livestock, and the default tasks the slice cannot start.

use crate::GenieError;
use crate::GenieResult;
use crate::gathering::make::MAKE;
use crate::gathering::make::MakeAction;
use crate::movement::fly::FLY;
use crate::movement::fly::Fly;
use crate::objects::DefaultStart;
use crate::objects::ObjectState;
use crate::objects::actions::ActionList;
use crate::objects::layers::Action;
use crate::objects::masters::ACTION_FACTORY_KINDS;
use crate::objects::masters::ActionKind;
use crate::objects::masters::Master;
use crate::objects::masters::Task;
use crate::objects::unit::Unit;
use crate::ownership::artifact::ARTIFACT_KIND;
use crate::ownership::artifact::Artifact;
use crate::world::slots::PlayerSlotsExt;
use crate::world::step::Step;

/// A default task the slice cannot start.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Unstartable {
    /// One of an action type the factory makes that the slice does not start.
    Unported(ActionKind),
    /// One the load could not read.
    Unread(UnreadDefault),
}

/// Why the load could not read an object's default task.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnreadDefault {
    /// The object's own master record, which the import could not read.
    OwnMaster,
    /// The load has no master table.
    NoMasterTable,
    /// The object's master is not in the master table.
    NotInTable,
    /// The master's default task id names no task of its list.
    NoSuchTask,
}

impl UnreadDefault {
    /// The reason the load of a default task it could not read fails with.
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::OwnMaster => "the default task of an own master the import could not read",
            Self::NoMasterTable => "no master table: the default task is unknown",
            Self::NotInTable => "a master the table does not have",
            Self::NoSuchTask => "a default task id that names no task",
        }
    }
}

impl Unstartable {
    /// The error its start fails with.
    #[must_use]
    pub fn error(self) -> GenieError {
        let (reason, kind) = match self {
            Self::Unported(kind) => ("a default task the slice does not model", Some(kind)),
            Self::Unread(why) => (why.reason(), None),
        };
        let reason_error = GenieError::unsupported(reason);
        match kind {
            Some(kind) => reason_error.with("action type", kind),
            None => reason_error,
        }
    }
}

impl Unit {
    /// `RGE_Action_Object::update` (`0x14106ac90`) on a unit in state 2 without an action or a
    /// production: the master's default task starts (slot 0x7f0, [`Unit::set_default_task`]),
    /// and the list's update does not run in this update ([`DefaultStart::Started`]), whatever
    /// the slot made: the caller jumps past it after the slot (`0x14106af80`) and for a master
    /// without a default task (`+0x790` −1, `0x14106af45`) alike.
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
        self.set_default_task(actions, step)?;
        Ok(DefaultStart::Started)
    }

    /// Slot 0x7f0 (`FUN_14106eb40`) with the default task id of the unit's master (`+0x790`,
    /// which the caller reads of the master at each call): the task the master holds by that id
    /// (`FUN_14109e0c0`, [`Master::default_task`]), nothing without one or with an action of
    /// its type on the list (`FUN_140a98740`, [`ActionList::holds`]). Else the list's factory
    /// (slot 0x10 with the task, no target and −1 thrice) makes the action of the task's type
    /// (`+0xe`): a bird's Fly ([`Fly::new`]), a farm's Make ([`MakeAction::new`]) or
    /// livestock's Artifact ([`Artifact::new`]), and slot 0x808 installs it
    /// ([`Unit::set_only_own_action`]: the list's deletion, then its add). The list's add starts
    /// what it hands back: the Fly's first ([`Unit::start_fly`]), the Make's
    /// ([`MakeAction::first`] with its owner's attributes); the Artifact goes onto the list as it
    /// is made. A unit in state 3 or later takes none, as the add deletes it unstarted. The
    /// factory makes no action of a type outside [`ACTION_FACTORY_KINDS`] (a wild animal's
    /// Explore, 6), and slot 0x808 is not called.
    ///
    /// # Errors
    ///
    /// A default task of a type the factory makes that the slice does not start
    /// ([`Unstartable::Unported`]), a unit whose master or owner the world lacks, and the
    /// install's and the starts'.
    pub(crate) fn set_default_task(
        &mut self,
        actions: &mut Action,
        step: &mut Step<'_>,
    ) -> GenieResult<()> {
        let Some(task) = self.default_task_to_make(&actions.list, step)? else {
            return Ok(());
        };
        match task.action_kind {
            FLY => {
                let fly = Fly::new(task, self.position());
                if let Some(fly) = self.set_only_own_action(actions, Some(fly), step)? {
                    self.start_fly(actions, step, fly)?;
                }
            }
            MAKE => {
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
            ARTIFACT_KIND => {
                let artifact = Artifact::new(step.player, Some(task));
                if let Some(artifact) = self.set_only_own_action(actions, Some(artifact), step)? {
                    actions.list.set_artifact(Some(artifact));
                }
            }
            kind if ACTION_FACTORY_KINDS.contains(&kind) => {
                return Err(Unstartable::Unported(kind).error());
            }
            // the factory's null: no action to install
            _ => {}
        }
        Ok(())
    }

    /// The task slot 0x7f0 makes an action of: the master's default task (`FUN_14109e0c0`),
    /// none for a master without one or a `list` that holds an action of its type
    /// (`FUN_140a98740`).
    fn default_task_to_make(
        &self,
        list: &ActionList,
        step: &Step<'_>,
    ) -> GenieResult<Option<Task>> {
        let master = self.master(step.world.master_tables(), step.player)?;
        Ok(Master::default_task(master)
            .copied()
            .filter(|task| !list.holds(task.action_kind)))
    }
}
