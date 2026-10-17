//! What the engine reads of a player's masters: the tasks a unit can do and the masters a unit
//! changes into to do them. The values are game data: the player's masters in the save, with the
//! `.dat`'s values for what the save does not carry.

use crate::build::BUILD;
use crate::unit_ai::Unsupported;

/// One task of a master's task list (`RGE_Task`), with the memory offsets the task finder reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Task {
    /// `+0xe`.
    pub action_type: i16,
    /// `+0x10`: the target class the task is for, -1 for any.
    pub object_group: i16,
    /// `+0x12`: the target master, -1 for any.
    pub object_id: i16,
    /// `+0x28`, the save's `work_val1`.
    pub work_val1: f32,
}

/// The master values the task finder and the work command read.
#[derive(Clone, Debug, PartialEq)]
pub struct Master {
    pub id: i16,
    /// The master's type (`+0x8`): 70 for a combat unit, 80 (`'P'`) for a building.
    pub kind: u8,
    /// `+0x410`.
    pub class: i16,
    /// `+0x7b8`, the `.dat`'s TaskSwapGroup (the save does not carry it); 0 for none.
    pub task_by_group: u8,
    /// `radius_x`, `radius_y` (`+0x5d8`, `+0x5dc`).
    pub radius: [f32; 2],
    pub tasks: Vec<Task>,
    /// The build cost `(resource, amount, paid)`.
    pub cost: Vec<(i16, i16, bool)>,
}

/// A player's masters by id.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Masters {
    by_id: Vec<Option<Master>>,
}

/// What the task finder needs of a target object.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target<'a> {
    pub id: i32,
    pub owner: usize,
    /// The object state (`+0x640`); a new foundation has 0.
    pub state: u8,
    pub master: &'a Master,
}

/// A building's master type.
const BUILDING: u8 = 80;

impl Masters {
    pub fn new(masters: impl IntoIterator<Item = Master>) -> Self {
        let mut by_id: Vec<Option<Master>> = Vec::new();
        for m in masters {
            let i = m.id as usize;
            if by_id.len() <= i {
                by_id.resize(i + 1, None);
            }
            by_id[i] = Some(m);
        }
        Masters { by_id }
    }

    pub fn get(&self, id: i16) -> Option<&Master> {
        self.by_id.get(usize::try_from(id).ok()?)?.as_ref()
    }

    /// The task of `actor` for an object target: the task finder of the units' task lists
    /// (`TRIBE_Task_List` slot 2, `0x1412784d0`), for the one target the slice models, a building
    /// of the actor's player in object state 0, a foundation (`0x1412795f5`; the building's action
    /// check `FUN_1411e8630` holds for a foundation without actions). Of the Build tasks: the one
    /// for the foundation's master, else the last for its class, each only with a work value, else
    /// the last for any target.
    pub fn find_task<'m>(
        actor: &'m Master,
        actor_owner: usize,
        target: &Target<'_>,
    ) -> Result<Option<&'m Task>, Unsupported> {
        if target.owner != actor_owner || target.master.kind != BUILDING || target.state != 0 {
            return Err(Unsupported(
                "the task finder for a target other than an own foundation",
            ));
        }
        let (mut by_class, mut generic) = (None, None);
        for task in actor.tasks.iter().filter(|t| t.action_type == BUILD) {
            if task.object_id == target.master.id {
                return Ok((task.work_val1 > 0.0).then_some(task));
            }
            if task.object_group == target.master.class {
                by_class = Some(task);
            }
            if task.object_group == -1 && task.object_id == -1 {
                generic = Some(task);
            }
        }
        Ok(match by_class {
            Some(task) => (task.work_val1 > 0.0).then_some(task),
            None => generic,
        })
    }

    /// The master a unit works on `target` with (`FUN_14106ff70`): its own, if its own task list
    /// has a task for the target other than attack (7); else the first master by id of the same
    /// class, type and nonzero task swap group whose task list has one of another action type.
    /// `Ok(None)`: no master has.
    pub fn work_master(
        &self,
        actor: i16,
        actor_owner: usize,
        target: &Target<'_>,
    ) -> Result<Option<i16>, Unsupported> {
        let own = self
            .get(actor)
            .ok_or(Unsupported("a unit of a master the player does not have"))?;
        let own_task = Self::find_task(own, actor_owner, target)?;
        if own_task.is_some_and(|t| t.action_type != 7) || own.task_by_group == 0 {
            return Ok(own_task.map(|_| actor));
        }
        for candidate in self.by_id.iter().flatten() {
            if candidate.class != own.class
                || candidate.kind != own.kind
                || candidate.task_by_group != own.task_by_group
            {
                continue;
            }
            let Some(task) = Self::find_task(candidate, actor_owner, target)? else {
                continue;
            };
            if own_task.is_some_and(|t| t.action_type == task.action_type) {
                continue;
            }
            if task.action_type == 0x6a {
                // gathering prefers a task by class with a work value (FUN_14106ff70 0x14107052e)
                return Err(Unsupported("the task swap of a gather task"));
            }
            return Ok(Some(candidate.id));
        }
        Ok(own_task.map(|_| actor))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(action_type: i16, object_group: i16, object_id: i16, work_val1: f32) -> Task {
        Task {
            action_type,
            object_group,
            object_id,
            work_val1,
        }
    }

    fn master(id: i16, task_by_group: u8, tasks: Vec<Task>) -> Master {
        Master {
            id,
            kind: 70,
            class: 4,
            task_by_group,
            radius: [0.2, 0.2],
            tasks,
            cost: vec![],
        }
    }

    fn house() -> Master {
        Master {
            id: 70,
            kind: 80,
            class: 3,
            task_by_group: 0,
            radius: [1.0, 1.0],
            tasks: vec![],
            cost: vec![(1, 25, true)],
        }
    }

    /// The villagers of build 158041's `.dat`, shortened: the base masters have no Build task, the
    /// builders a generic one; a fisherman (56) of the male group has none either.
    fn villagers() -> Masters {
        let base = || {
            vec![
                task(7, -1, -1, 0.0),
                task(3, 3, -1, 0.0),
                task(110, -1, 126, 0.0),
            ]
        };
        let builder = || vec![task(101, -1, -1, 1.0), task(3, 3, -1, 0.0)];
        Masters::new([
            master(56, 1, vec![task(5, -1, 53, 1.0)]),
            master(83, 1, base()),
            master(118, 1, builder()),
            master(212, 2, builder()),
            master(293, 2, base()),
            house(),
        ])
    }

    fn foundation(m: &Masters) -> Target<'_> {
        Target {
            id: 10028,
            owner: 2,
            state: 0,
            master: m.get(70).unwrap(),
        }
    }

    #[test]
    fn a_villager_takes_the_first_master_of_its_swap_group_with_a_task_for_its_foundation() {
        let m = villagers();
        assert_eq!(m.work_master(83, 2, &foundation(&m)), Ok(Some(118)));
        assert_eq!(m.work_master(293, 2, &foundation(&m)), Ok(Some(212)));
        assert_eq!(m.work_master(118, 2, &foundation(&m)), Ok(Some(118)));
    }

    #[test]
    fn a_build_task_for_the_foundation_s_master_or_class_needs_a_work_value() {
        let site = house();
        let target = Target {
            id: 1,
            owner: 2,
            state: 0,
            master: &site,
        };
        let by = |tasks: Vec<Task>| {
            let builder = master(118, 1, tasks);
            Masters::find_task(&builder, 2, &target).map(|t| t.copied())
        };
        let generic = task(101, -1, -1, 0.0);
        assert_eq!(by(vec![generic]), Ok(Some(generic)));
        assert_eq!(
            by(vec![task(101, 3, -1, 2.0), generic]),
            Ok(Some(task(101, 3, -1, 2.0)))
        );
        assert_eq!(by(vec![task(101, 3, -1, 0.0), generic]), Ok(None));
        assert_eq!(by(vec![generic, task(101, -1, 70, 0.0)]), Ok(None));
    }

    #[test]
    fn a_target_other_than_an_own_foundation_is_not_modelled() {
        let m = villagers();
        let builder = m.get(118).unwrap();
        let finished = Target {
            state: 2,
            ..foundation(&m)
        };
        let enemy = Target {
            owner: 1,
            ..foundation(&m)
        };
        assert!(Masters::find_task(builder, 2, &finished).is_err());
        assert!(Masters::find_task(builder, 2, &enemy).is_err());
    }
}
