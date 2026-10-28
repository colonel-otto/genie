//! The master's default task, which an idle unit starts in its update: a bird's Fly, a farm's
//! Make or the Artifact of livestock.

use crate::GenieError;
use crate::objects::masters::Master;
use crate::objects::unit::Unit;
use crate::ownership::artifact::Artifact;
use crate::world::context::Context;

/// The default task that an update started. Its first update comes in the next update.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct DefaultTaskStarted {
    /// A bird's Fly.
    pub(crate) fly: bool,
    /// A farm's Make.
    pub(crate) make: bool,
    /// The Artifact of livestock.
    pub(crate) artifact: bool,
}

impl DefaultTaskStarted {
    /// Whether the update started a default task.
    pub(crate) const fn any(self) -> bool {
        self.fly || self.make || self.artifact
    }
}

impl Unit {
    /// `RGE_Action_Object::update` (`0x14106ac90`) on a unit in state 2 without an action or a
    /// production: the master's default task starts (slot 0x7f0). A default task that the slice
    /// does not hold is an error.
    pub(crate) fn start_default_task(
        &mut self,
        c: &mut Context<'_>,
    ) -> Result<DefaultTaskStarted, GenieError> {
        // RGE_Action_Object::update with no action in state 2 starts the master's default task
        // (slot 0x7f0): a bird's Fly, whose first update comes in the next tick
        let mut flies_from_now = false;
        if self.life.state == 2
            && self.top_action_kind().is_none()
            && self.production.is_none()
            && let Some(task) = self.default_fly
        {
            self.start_fly(c, task)?;
            flies_from_now = true;
        }
        let mut makes_from_now = false;
        if self.life.state == 2
            && self.top_action_kind().is_none()
            && self.production.is_none()
            && std::mem::take(&mut self.default_make)
        {
            let task = c
                .masters
                .and_then(|m| m.get(self.life.master_id))
                .and_then(Master::default_task)
                .copied()
                .ok_or(GenieError::unsupported(
                    "a Make whose master the slice does not have",
                ))?;
            self.actions
                .set_make(Some(crate::gathering::make::MakeAction::start(
                    task,
                    c.attributes,
                    &mut crate::gathering::make::Held {
                        attribute_type: &mut self.life.attribute_type,
                        amount: &mut self.life.amount,
                    },
                )));
            makes_from_now = true;
        }
        let mut artifact_from_now = false;
        if self.life.state == 2
            && self.top_action_kind().is_none()
            && self.production.is_none()
            && self.default_artifact
        {
            self.actions
                .set_artifact(Some(Artifact::new(c.player == 0)));
            artifact_from_now = true;
        }
        if self.life.state == 2
            && self.top_action_kind().is_none()
            && self.production.is_none()
            && let Some(e) = self.default_task.clone()
        {
            return Err(e);
        }
        Ok(DefaultTaskStarted {
            fly: flies_from_now,
            make: makes_from_now,
            artifact: artifact_from_now,
        })
    }
}
