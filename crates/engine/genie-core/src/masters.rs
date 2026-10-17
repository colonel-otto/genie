//! What the engine reads of a player's masters: the tasks a unit can do and the masters a unit
//! changes into to do them. The values are game data: the player's masters in the save, with the
//! `.dat`'s values for what the save does not carry.

use crate::movement::MoveStats;
use crate::tasks::{Actor, Diplomacy, TaskTarget, find_task};
use crate::unit_ai::Unsupported;

/// One task of a master's task list (`RGE_Task`), with the memory offsets the task finder reads
/// (the save's names; the `.dat`'s in brackets where they differ).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Task {
    /// `+0xa`.
    pub id: i16,
    /// `+0xe`.
    pub action_type: i16,
    /// `+0x10`: the target class the task is for, -1 for any.
    pub object_group: i16,
    /// `+0x12`: the target master, -1 for any.
    pub object_id: i16,
    /// `+0x14`, `combat_level` (`enable_targeting`): the task is for a target's combat level.
    pub combat_level: u8,
    /// `+0x16`: the terrain the task is for, -1 for none.
    pub terrain: i16,
    /// `+0x18`, `owner_type` (`target_diplomacy`): whose targets.
    pub owner_type: u8,
    /// `+0x19`, `holding_attr` (`carry_check`): 1 wants a target holding the task's resource.
    pub holding_attr: u8,
    /// `+0x20` to `+0x26`, `attribute_type` to `attribute_type4` (`resource_in`,
    /// `resource_multiplier`: the attribute that scales the gathering, `resource_out`,
    /// `unused_resource`).
    pub attribute_type: i16,
    pub attribute_type2: i16,
    pub attribute_type3: i16,
    pub attribute_type4: i16,
    /// `+0x28`, the save's `work_val1`.
    pub work_val1: f32,
    /// `+0x30`: how near the target the work is done.
    pub work_range: f32,
    /// `+0x38`, `enablement_resource_id` (`enabled`): the resource that enables the task, -1 none.
    pub enablement: i16,
    /// `+0x40`..`+0x58`: the moving, proceeding, working and carrying sprites (graphic ids, -1 for
    /// none).
    pub sprites: [i16; 4],
}

/// A task for any target, with no resource, terrain or requirement.
impl Default for Task {
    fn default() -> Self {
        Task {
            id: 0,
            action_type: 0,
            object_group: -1,
            object_id: -1,
            combat_level: 0,
            terrain: -1,
            owner_type: 0,
            holding_attr: 0,
            attribute_type: -1,
            attribute_type2: -1,
            attribute_type3: -1,
            attribute_type4: -1,
            work_val1: 0.0,
            work_range: 0.0,
            enablement: -1,
            sprites: [-1; 4],
        }
    }
}

/// The master values the task finder and the work command read.
#[derive(Clone, Debug, Default, PartialEq)]
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
    /// `radius_z` (`+0x5e0`) is above 0: its objects count as tall on their tiles.
    pub tall: bool,
    /// `line_of_sight` (`+0x5d0`).
    pub line_of_sight: f32,
    /// `obstruction_type` (`+0x651`) and `can_be_built_on` (`+0x5e9`).
    pub obstruction_type: u8,
    pub buildable_on: u8,
    /// `obstruction_class` (`+0x652`).
    pub obstruction_class: u8,
    /// `object_flags` (`+0x654`): bit 0 takes garrisons, bit 4 marks a hunter master.
    pub object_flags: u32,
    /// The `.dat`'s `combat_level` (`+0x62c`).
    pub combat_level: u8,
    pub tasks: Vec<Task>,
    /// The build cost `(resource, amount, paid)`.
    pub cost: Vec<(i16, i16, bool)>,
    /// `work_rate` (`+0x798`): what a builder of this master adds to the work per second.
    pub work_rate: f32,
    /// `search_radius` (`+0x794`, the object's slot 0x2c8).
    pub search_radius: f32,
    /// The hit points (`+0x530`) and the build time: the first train location's train time
    /// (`+0x898`'s first entry), 0 without one.
    pub hit_points: i16,
    pub build_time: i16,
    /// The combat master's `weapon_range` (`+0x828`, object slot 0x230) and
    /// `minimum_weapon_range` (`+0x850`, slot 0x240).
    pub weapon_range: f32,
    pub min_range: f32,
    /// `charge_type` (`+0x922`): 6 and 7 add `charge_event` (`+0x920`) to the range against some
    /// targets (slot 0x2f8).
    pub charge_type: i16,
    /// `+0x61c`, `attribute_max_amount`: how much an object of this master carries.
    pub capacity: i16,
    /// The resource storage (`attribute_type_held`, `attribute_amount_held`, `attribute_flag`:
    /// `+0x60a`, `+0x610`, `+0x628`): what an object of this master adds to its player's
    /// attributes, by its flag.
    pub storage: Vec<(i16, f32, u8)>,
    /// The master has damage sprites (`+0x638`..`+0x640`): a master change checks them
    /// (`FUN_1411859a0`).
    pub damage_sprites: bool,
    /// What a combat master's attack reads: its standing sprite (`+0x470`), its attack sprite
    /// (`+0x7f8`, `fight_sprite_id`), a second attack sprite
    /// (`+0x878`, the `.dat`'s `attack_graphic2`), `missile_id`, `fire_missile_at_frame`,
    /// `interrupt_frame`, `base_hit_chance`, `speed_of_attack` (the reload time) and its weapons
    /// (class, amount).
    pub standing_sprite: i16,
    pub attack_sprite: i16,
    pub attack_sprite2: i16,
    pub missile: i16,
    pub fire_frame: i16,
    pub interrupt_frame: i16,
    pub hit_chance: i16,
    pub reload: f32,
    pub weapons: Vec<(i16, i16)>,
    /// What a unit of this master moves with, its speed with the master's walking sprite (the
    /// run sprite `+0x750` if it has one, else the move sprite `+0x748`); None for a master that
    /// does not move.
    pub moving: Option<MoveStats>,
}

/// A player's masters by id.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Masters {
    by_id: Vec<Option<Master>>,
}

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

    /// Every master, by id.
    pub fn iter(&self) -> impl Iterator<Item = &Master> {
        self.by_id.iter().flatten()
    }

    /// The work command's choice of master and task (`FUN_14106ff70` up to the task's
    /// requirement): the unit's own master's task for the target ([`find_task`]) unless it has
    /// none or only an attack (7); then, with a task swap group, the first master by id of the
    /// unit's class, type and group with a task of another type (its master changes, slot 0x90),
    /// except that a gather task (106) for a farm, a pasture or the other objects the work command
    /// lists, with any filter, prefers the first such master with a task for the target's class
    /// and a work value, else the first with a task for any target. `Ok(None)`: no task.
    pub fn work_master<'m>(
        &'m self,
        actor: &Actor<'_>,
        target: &TaskTarget<'_>,
        general: &TaskTarget<'_>,
        filter: i16,
        dip: &impl Diplomacy,
    ) -> Result<Option<(i16, &'m Task)>, Unsupported> {
        let own = actor.master;
        let own_list = self
            .get(own.id)
            .ok_or(Unsupported("a unit of a master the player does not have"))?;
        let own_task = find_task(
            own_list,
            actor,
            Some(target),
            Some(general),
            None,
            filter,
            dip,
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
                    candidate,
                    actor,
                    Some(target),
                    Some(general),
                    None,
                    filter,
                    dip,
                )?;
                let Some(task) = found else { continue };
                if own_task.is_some_and(|t| t.action_type == task.action_type) {
                    continue;
                }
                let listed = matches!(
                    target.master.id,
                    50 | 357 | 1187 | 1188 | 1893 | 1894 | 1897 | 1898
                );
                if task.action_type != 0x6a
                    || (filter != -1 && filter != 0x6a)
                    || !listed
                    || task.object_id == target.master.id
                {
                    return Ok(Some((candidate.id, task)));
                }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn task(action_type: i16, object_group: i16, object_id: i16, work_val1: f32) -> Task {
        Task {
            action_type,
            object_group,
            object_id,
            work_val1,
            ..Task::default()
        }
    }

    fn master(id: i16, task_by_group: u8, tasks: Vec<Task>) -> Master {
        Master {
            id,
            kind: 70,
            class: 4,
            task_by_group,
            radius: [0.2, 0.2],
            tall: true,
            obstruction_type: 0,
            buildable_on: 0,
            tasks,
            cost: vec![],
            obstruction_class: 0,
            object_flags: 0,
            combat_level: 0,
            work_rate: 1.0,
            hit_points: 25,
            build_time: 0,
            ..Master::default()
        }
    }

    fn house() -> Master {
        Master {
            id: 70,
            kind: 80,
            class: 3,
            task_by_group: 0,
            radius: [1.0, 1.0],
            tall: true,
            obstruction_type: 0,
            buildable_on: 0,
            tasks: vec![],
            cost: vec![(1, 25, true)],
            obstruction_class: 2,
            object_flags: 0,
            combat_level: 0,
            work_rate: 0.0,
            hit_points: 550,
            build_time: 25,
            ..Master::default()
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

    /// Every player on its own, no drop sites.
    struct Alone;

    impl Diplomacy for Alone {
        fn allied(&self, from: usize, to: usize, _: bool) -> bool {
            from == to
        }

        fn drops_at(&self, _: i16, _: &Actor<'_>) -> bool {
            false
        }
    }

    #[test]
    fn a_villager_takes_the_first_master_of_its_swap_group_with_a_task_for_its_foundation() {
        let m = villagers();
        let site = TaskTarget {
            id: 10028,
            master: m.get(70).unwrap(),
            player: 2,
            state: 0,
            held: (-1, 0.0),
            hp: 1.0,
            max_hp: 550.0,
            attackable: true,
            pasture: -1,
            top_action: None,
        };
        let pick = |own| {
            let actor = Actor {
                id: 9181,
                master: m.get(own).unwrap(),
                player: 2,
                held: (-1, 0.0),
                attribute_95: 0.0,
                civ: 1,
            };
            m.work_master(&actor, &site, &site, -1, &Alone)
                .map(|w| w.map(|(id, t)| (id, t.action_type)))
        };
        assert_eq!(pick(83), Ok(Some((118, 101))));
        assert_eq!(pick(293), Ok(Some((212, 101))));
        assert_eq!(pick(118), Ok(Some((118, 101))));
    }
}
