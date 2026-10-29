//! A farm's food: `RGE_Action_Make` (action type 21, vtable `RGE_Action_Make::vftable`), the
//! default task of the farms (50, 1187), the fish trap (199) and the pastures (1893, 1897). The
//! RGE factory makes it (`FUN_140a97e00`, case 0x15, constructor `FUN_140a99430`); the TRIBE
//! factory (`FUN_140aca4b0`) has no case of its own for it.
//!
//! The farm holds what it made (`+0x308`, `+0x310`) up to its master's capacity
//! (`attribute_max_amount`, `+0x61c`): the Make adds its master's work rate (plus the farm's
//! powerup) times the task's `work_val1` per second, and the farmers take from it. The food
//! still to make starts at the player's farm food (attribute 36) or fish trap food (88); once it
//! is made and the farmers took the last of it, the farm's slot 0xd8 sets its hit points to 0.

use crate::GenieError;
use crate::objects::lifecycle::seconds;
use crate::objects::masters::Task;
use crate::objects::teardown::HeadCode;
use crate::objects::unit::Unit;
use crate::objects::unit::object_master;
use crate::world::context::Context;
use genie_diff::Diff;

/// The action type (`+0x8`, 0x15).
pub const MAKE: i16 = 21;

/// `+0x18`: the state its update runs in (`set_state(6)`, `first_in_stack`'s).
pub const MAKING: u8 = 6;

/// `+0x18`: the state whose update reports the action done (`FUN_140a99e30` returns 1).
pub const DONE: u8 = 1;

/// The task's resource for which `set_state(6)` reads the farm food (`FUN_140a99d60`'s
/// `task+0x20 == 0x10`): the farms' and the pastures' food kind.
pub const FARM_FOOD_KIND: i16 = 16;

/// The player's attribute `set_state(6)` reads for a task of [`FARM_FOOD_KIND`]
/// (`player+0x100` at `+0x120`): Farm Food Amount.
pub const FARM_FOOD: usize = 36;

/// The player's attribute `set_state(6)` reads for any other task (`player+0x100` at `+0x2c0`):
/// Fish Trap Food Amount.
pub const FISH_TRAP_FOOD: usize = 88;

/// The masters whose exhaustion the update does not check (`FUN_140a99e30`: the master id, not
/// 0x765 and not 0x769): the pastures.
pub const UNEXHAUSTED: [i16; 2] = [1893, 1897];

/// The class whose slot 0xd8 (`FUN_141211340`, a combat object's or building's) does nothing
/// (decoded 42); class 58 is spared only while it holds some, which an exhausted farm does not.
const NEVER_EXHAUSTED_CLASS: i16 = 42;

/// The first master type whose slot 0xd8 is `FUN_141211340` (70, `TRIBE_Combat_Object`, and 80);
/// below it slot 0xd8 is `FUN_14118f580` itself.
const TRIBE_COMBAT_TYPE: u8 = 70;

/// `RGE_Action_Make`: its state (`+0x18`), the food still to make (`+0x44`, the base's
/// `mTimer`), the food it makes in all (`+0x68`, the save's `work_timer`) and its task (`+0x48`).
#[derive(Clone, Copy, Debug, PartialEq, Diff)]
pub struct MakeAction {
    /// `+0x18`.
    pub state: u8,
    /// `+0x44`: the food still to make.
    pub remaining: f32,
    /// `+0x68`: the food made in all; the constructor's 120.0 never shows, as
    /// [`MakeAction::start`] sets it at once.
    pub total: f32,
    /// `+0x48`: the master's default task.
    pub task: Task,
}

/// What the farm holds: `+0x308` (its attribute type) and `+0x310` (the amount).
#[derive(Debug)]
pub struct Held<'a> {
    /// `+0x308`.
    pub attribute_type: &'a mut i16,
    /// `+0x310`.
    pub amount: &'a mut f32,
}

/// What an update reads of the farm and its owner besides what it holds.
#[derive(Clone, Copy, Debug)]
pub struct Farm {
    /// Its master's id (the exhaustion's test).
    pub master_id: i16,
    /// Its master's type (`+0x8`: which slot 0xd8 it has).
    pub kind: u8,
    /// Its master's class (slot 0xd8's test).
    pub class: i16,
    /// Its master's work rate (`+0x798`) plus its powerup (slot 0x6e8, `+0xa30`).
    pub work_rate: f32,
    /// Its master's capacity (`attribute_max_amount`, `+0x61c`).
    pub capacity: i16,
    /// The world's seconds of this update (`world+0x164`).
    pub dt: f32,
}

/// What an update tells its object.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MakeStep {
    /// The update returned 1: the action list removes it.
    pub done: bool,
    /// The farm's slot 0xd8 ran: its hit points go to 0 (`FUN_14118f580`).
    pub exhausted: bool,
    /// What a task with a work flag adds to its player's attribute `task.attribute_type`
    /// instead (the player's slot 0xf8, `FUN_141616820` with 0 and 1).
    pub to_player: Option<(i16, f32)>,
}

impl MakeAction {
    /// The default task's start (slot 0x7f0, `FUN_14106eb40`): the factory's action
    /// (`FUN_140a99430`), then `set_only_action` (slot 0x808, `FUN_14106e990`), whose push
    /// (`FUN_140a977e0`) runs `first_in_stack` (`FUN_140a99980`): `set_state(6)`, then a task
    /// resource other than the farm's own holding replaces it, with nothing held
    /// (`FUN_14119d310` with 0).
    #[must_use]
    pub fn start(task: Task, attributes: &[f32], held: &mut Held<'_>) -> Self {
        // the factory's constructor (FUN_140a99430) sets +0x68 to 120.0, which set_state(6)
        // replaces at once
        let mut make = Self {
            state: 0,
            remaining: 0.0,
            total: 120.0,
            task,
        };
        make.set_state(MAKING, attributes);
        if 0 <= task.attribute_type && *held.attribute_type != task.attribute_type {
            *held.attribute_type = task.attribute_type;
            *held.amount = 0.0;
        }
        make
    }

    /// `set_state` (`FUN_140a99d60`): state 6 takes the player's food to make (attribute 36 for
    /// a task of the farm food kind, else 88) as both what is left and what is made in all.
    pub fn set_state(&mut self, state: u8, attributes: &[f32]) {
        self.state = state;
        if state == MAKING {
            let kind = if self.task.attribute_type == FARM_FOOD_KIND {
                FARM_FOOD
            } else {
                FISH_TRAP_FOOD
            };
            self.remaining = attributes.get(kind).copied().unwrap_or(0.0);
            self.total = self.remaining;
        }
    }

    /// The update (`FUN_140a99e30`). State 1 is done. In state 6, while food is left to make (or
    /// none was ever to be made), the farm's holding grows by its work rate times the task's
    /// `work_val1` and the update's seconds, within its capacity (`FUN_14119d320` with 1 and 1),
    /// and what it gained is no longer left to make; a task with a work flag (`+0x34`) adds to
    /// its player instead, so nothing is taken off. Once all is made, a farm that holds nothing
    /// (not a pasture) runs its slot 0xd8. The game's `+0xc14` (`FUN_140c884c0`), which makes
    /// 1000 at once, is 0, as the tick oracle holds it. The other states do nothing.
    pub fn update(&mut self, held: &mut Held<'_>, farm: Farm) -> MakeStep {
        let mut step = MakeStep::default();
        match self.state {
            DONE => step.done = true,
            MAKING if 0.0 < self.remaining || self.total <= 0.0 => {
                let rate = farm.work_rate * self.task.work_val1 * farm.dt;
                let before = *held.amount;
                if self.task.work_flag == 0 {
                    // FUN_14119d320: add, then at most the capacity
                    *held.amount += rate;
                    let capacity = f32::from(farm.capacity);
                    if capacity < *held.amount {
                        *held.amount = capacity;
                    }
                } else {
                    step.to_player = Some((self.task.attribute_type, rate));
                }
                self.remaining -= *held.amount - before;
            }
            // slot 0xd8: a combat object's or building's (FUN_141211340) spares class 42, then
            // FUN_14118f580
            MAKING if !UNEXHAUSTED.contains(&farm.master_id) && *held.amount <= 0.0 => {
                step.exhausted =
                    farm.kind < TRIBE_COMBAT_TYPE || farm.class != NEVER_EXHAUSTED_CLASS;
            }
            _ => {}
        }
        step
    }
}

impl Unit {
    /// A farm's Make in its update: the food it makes, the player's share, the farm that runs
    /// out, and its code (the end's 1).
    pub(crate) fn make_part(&mut self, c: &mut Context<'_>) -> Result<HeadCode, GenieError> {
        let Some(make) = self.actions.make_mut() else {
            return Ok(HeadCode::Keeps);
        };
        let master = object_master(
            self.own_master.as_deref(),
            (self.id, self.life.master_id),
            c.w.masters.get(c.player),
        )?;
        let farm = Farm {
            master_id: master.id,
            kind: master.kind,
            class: master.class,
            work_rate: master.work_rate + self.work_rate_powerup,
            capacity: master.capacity,
            dt: seconds(c.dt),
        };
        let step = make.update(
            &mut Held {
                attribute_type: &mut self.life.attribute_type,
                amount: &mut self.life.amount,
            },
            farm,
        );
        if let Some((kind, amount)) = step.to_player {
            c.w.players[c.player].add_attribute(kind, amount)?;
        }
        // slot 0xd8 (FUN_14118f580): no hit points left; the object is in its list, so not
        // asleep, and the special-object list (+0x30c bit 0) is not held
        if step.exhausted {
            self.life.hp = 0.0;
        }
        // its 1 deletes it
        Ok(HeadCode::ended(step.done))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The farm's default task in build 158041's `.dat` (master 50, task 0).
    fn farm_task() -> Task {
        Task {
            action_type: MAKE,
            attribute_type: FARM_FOOD_KIND,
            attribute_type2: 0,
            work_val1: 1.0,
            ..Task::default()
        }
    }

    fn farm(master_id: i16) -> Farm {
        Farm {
            master_id,
            kind: 80,
            class: 49,
            work_rate: 0.4,
            capacity: 15,
            dt: 0.104,
        }
    }

    fn attributes() -> Vec<f32> {
        let mut a = vec![0.0; 100];
        (a[FARM_FOOD], a[FISH_TRAP_FOOD]) = (175.0, 225.0);
        a
    }

    #[test]
    fn the_start_takes_the_farm_food_and_holds_the_task_s_resource_from_nothing() {
        let (mut kind, mut amount) = (0, 15.0);
        let make = MakeAction::start(
            farm_task(),
            &attributes(),
            &mut Held {
                attribute_type: &mut kind,
                amount: &mut amount,
            },
        );
        assert_eq!(
            (make.state, make.remaining, make.total),
            (MAKING, 175.0, 175.0)
        );
        assert_eq!((kind, amount), (FARM_FOOD_KIND, 0.0));
    }

    #[test]
    fn a_fish_trap_takes_the_fish_trap_food_and_keeps_a_holding_of_its_kind() {
        let task = Task {
            attribute_type: 17,
            ..farm_task()
        };
        let (mut kind, mut amount) = (17, 15.0);
        let make = MakeAction::start(
            task,
            &attributes(),
            &mut Held {
                attribute_type: &mut kind,
                amount: &mut amount,
            },
        );
        assert_eq!((make.remaining, kind, amount), (225.0, 17, 15.0));
    }

    #[test]
    fn the_update_fills_the_farm_within_its_capacity_and_counts_what_it_made() {
        let (mut kind, mut amount) = (FARM_FOOD_KIND, 14.98);
        let mut make = MakeAction {
            state: MAKING,
            remaining: 100.0,
            total: 175.0,
            task: farm_task(),
        };
        let mut held = Held {
            attribute_type: &mut kind,
            amount: &mut amount,
        };
        let step = make.update(&mut held, farm(50));
        assert_eq!(step, MakeStep::default());
        let gained = 15.0f32 - 14.98;
        assert_eq!((*held.amount, make.remaining), (15.0, 100.0 - gained));
        // at the capacity nothing more is made
        let _ = make.update(&mut held, farm(50));
        assert_eq!(make.remaining, 100.0 - gained);
        // a farmer took 5: the rate's share comes back
        *held.amount = 10.0;
        let _ = make.update(&mut held, farm(50));
        let rate = 0.4f32 * 1.0 * 0.104;
        assert_eq!(*held.amount, 10.0 + rate);
        assert_eq!(make.remaining, 100.0 - gained - (10.0 + rate - 10.0));
    }

    #[test]
    fn a_farm_with_nothing_left_to_make_and_nothing_held_is_exhausted_unless_a_pasture() {
        let (mut kind, mut amount) = (FARM_FOOD_KIND, 0.0);
        let mut make = MakeAction {
            state: MAKING,
            remaining: 0.0,
            total: 175.0,
            task: farm_task(),
        };
        let mut held = Held {
            attribute_type: &mut kind,
            amount: &mut amount,
        };
        assert!(make.update(&mut held, farm(50)).exhausted);
        assert!(!make.update(&mut held, farm(1897)).exhausted);
        let wall = Farm {
            class: NEVER_EXHAUSTED_CLASS,
            ..farm(50)
        };
        assert!(!make.update(&mut held, wall).exhausted);
        *held.amount = 3.0;
        assert!(!make.update(&mut held, farm(50)).exhausted);
    }

    #[test]
    fn a_task_with_a_work_flag_makes_for_its_player_and_never_runs_out() {
        let task = Task {
            work_flag: 1,
            ..farm_task()
        };
        let (mut kind, mut amount) = (FARM_FOOD_KIND, 0.0);
        let mut make = MakeAction {
            state: MAKING,
            remaining: 5.0,
            total: 5.0,
            task,
        };
        let step = make.update(
            &mut Held {
                attribute_type: &mut kind,
                amount: &mut amount,
            },
            farm(50),
        );
        assert_eq!(step.to_player, Some((FARM_FOOD_KIND, 0.4 * 0.104)));
        assert_eq!((amount, make.remaining), (0.0, 5.0));
    }

    #[test]
    fn state_1_is_done() {
        let (mut kind, mut amount) = (0, 0.0);
        let mut make = MakeAction {
            state: DONE,
            remaining: 0.0,
            total: 0.0,
            task: farm_task(),
        };
        let step = make.update(
            &mut Held {
                attribute_type: &mut kind,
                amount: &mut amount,
            },
            farm(50),
        );
        assert!(step.done);
    }
}
