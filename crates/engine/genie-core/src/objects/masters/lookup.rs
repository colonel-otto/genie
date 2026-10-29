//! A player's masters by id ([`Masters`]) and the work command's choice of master and task.

use crate::GenieError;
use crate::objects::masters::{Master, Masters, Task};
use crate::unit_ai::task_finder::{Actor, Diplomacy, TaskOn, find_task};

/// Player `player`'s master `id` in `every`, the world's masters: one table per player, as the
/// game's player holds its masters' pointers (`player+0x28`); none where the player has no
/// table or its table no such master (a null pointer there). The one lookup of a master by its
/// player and id: an object's master goes through [`crate::objects::unit::Unit::master`], an
/// invariant one through [`crate::world::World::master`].
#[must_use]
pub fn player_master(every: &[Masters], player: usize, id: i16) -> Option<&Master> {
    every.get(player)?.get(id)
}

impl Masters {
    /// The masters, by their ids.
    pub fn new(masters: impl IntoIterator<Item = Master>) -> Self {
        let mut by_id: Vec<Option<Master>> = Vec::new();
        for m in masters {
            let i = m.id as usize;
            if by_id.len() <= i {
                by_id.resize(i + 1, None);
            }
            by_id[i] = Some(m);
        }
        Self { by_id }
    }

    /// The master `id`, if the player has it.
    #[must_use]
    pub fn get(&self, id: i16) -> Option<&Master> {
        self.by_id.get(usize::try_from(id).ok()?)?.as_ref()
    }

    /// Every master, by id.
    pub fn iter(&self) -> impl Iterator<Item = &Master> {
        self.by_id.iter().flatten()
    }

    /// Every master, by id, to change (a technology's effect).
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Master> {
        self.by_id.iter_mut().flatten()
    }

    /// The master `id` to change, if the player has it.
    pub fn get_mut(&mut self, id: i16) -> Option<&mut Master> {
        self.by_id.get_mut(usize::try_from(id).ok()?)?.as_mut()
    }

    /// The work command's choice of master and task (`FUN_14106ff70` up to the task's
    /// requirement) on `on` (its target, none for a point, and the terrain under the point): the
    /// unit's own master's task ([`find_task`]) unless it has none or only an attack (7); then,
    /// with a task swap group, the first master by id of the unit's class, type and group with a
    /// task of another type (its master changes, slot 0x90), except that a gather task (106) for
    /// a farm, a pasture or the other objects the work command lists, with any filter, prefers
    /// the first such master with a task for the target's class and a work value, else the first
    /// with a task for any target. `Ok(None)`: no task.
    pub fn work_master<'m>(
        &'m self,
        actor: &Actor<'_>,
        on: &TaskOn<'_, '_>,
        filter: i16,
        dip: &impl Diplomacy,
    ) -> Result<Option<(i16, &'m Task)>, GenieError> {
        let own = actor.master;
        let own_list = self.get(own.id).ok_or(GenieError::unsupported(
            "a unit of a master the player does not have",
        ))?;
        let own_task = find_task(
            own_list, actor, on.target, on.general, on.terrain, filter, dip,
        )?;
        if own_task.is_none_or(|t| t.action_type == 7) && own.task_by_group != 0 {
            let (mut by_class, mut generic) = (None, None);
            for candidate in self.by_id.iter().flatten() {
                if candidate.class != own.class
                    || candidate.kind != own.kind
                    || candidate.task_by_group != own.task_by_group
                {
                    continue;
                }
                let found = find_task(
                    candidate, actor, on.target, on.general, on.terrain, filter, dip,
                )?;
                let Some(task) = found else { continue };
                if own_task.is_some_and(|t| t.action_type == task.action_type) {
                    continue;
                }
                // a gather task for a point has no target: the game reads none there, as the
                // finder gives a point only a task for its terrain, never a gather one
                let listed = on.target.filter(|t| {
                    matches!(
                        t.master.id,
                        50 | 357 | 1187 | 1188 | 1893 | 1894 | 1897 | 1898
                    )
                });
                let Some(target) = listed.filter(|t| {
                    task.action_type == 0x6a
                        && (filter == -1 || filter == 0x6a)
                        && task.object_id != t.master.id
                }) else {
                    return Ok(Some((candidate.id, task)));
                };
                if by_class.is_none()
                    && task.object_group == target.master.class
                    && 0.0 < task.work_val1
                {
                    by_class = Some((candidate.id, task));
                } else if generic.is_none() && task.object_group == -1 && task.object_id == -1 {
                    generic = Some((candidate.id, task));
                }
            }
            if let Some(pick) = by_class.or(generic) {
                return Ok(Some(pick));
            }
        }
        Ok(own_task.map(|t| (own.id, t)))
    }
}
