//! A building's production: the queue QUEUE fills and the make action that works it off
//! (`TRIBE_Action_Make_Obj`, update `0x140ad0490`), which the building's update runs after its
//! combat update (`TRIBE_Building_Object::update` -> `FUN_1411e7dc0`). The unit's spawn is
//! [`crate::spawn`]'s.

use crate::unit_ai::Unsupported;
use std::collections::VecDeque;

/// One unit or research in a building's queue.
#[derive(Clone, Debug, PartialEq)]
pub struct Queued {
    /// The master that spawns (a villager's male or female master is drawn when it is queued),
    /// −1 for a research.
    pub master: i16,
    /// The technology researched, −1 for a unit.
    pub tech: i16,
    /// What its queue entry paid (`+0x8`: three attributes and amounts), which a research's start
    /// reads; for a unit QUEUE queues, not held ((−1, 0) each).
    pub costs: [(i32, f32); 3],
    /// The seconds of work it takes (the master's train time at this building).
    pub train_time: f32,
}

/// A building's queue and its make action.
#[derive(Clone, Debug, PartialEq)]
pub struct Production {
    pub queue: VecDeque<Queued>,
    /// The make action exists (the building's production action list holds it).
    pub action: bool,
    /// `+0x6c`: the seconds of work done on the first unit.
    pub progress: f32,
    /// The building's work rate: its `mWorkRatePowerup` plus its master's `work_rate`.
    pub rate: f32,
}

impl Production {
    /// The make action's state while it works, which the building's action-state slot reports.
    pub const MAKING: u8 = 6;

    /// The make action's update (state 6) for `dt_seconds`: the progress grows by the rate times
    /// dt; the first unit is due once its train time is reached.
    pub fn work(&mut self, dt_seconds: f32) -> Option<&Queued> {
        self.progress += self.rate * dt_seconds;
        self.queue
            .front()
            .filter(|unit| unit.train_time <= self.progress)
    }

    /// The first unit spawned: its make action ends (its update returns 1 and the action list
    /// deletes it) and the queue lets it go (`FUN_1411e7dc0` after the list's update).
    pub fn spawned(&mut self) {
        self.queue.pop_front();
        self.action = false;
        self.progress = 0.0;
    }
}

/// The population a master's unit takes (`FUN_14163b0b0`): minus its storage of attribute 4,
/// the population headroom, over the master's three storage entries.
#[must_use]
pub fn population(storage: &[(i16, f32, u8)]) -> f32 {
    storage
        .iter()
        .filter(|&&(kind, _, _)| kind == HEADROOM)
        .fold(0.0, |pop, &(_, amount, _)| pop - amount)
}

/// The population headroom (attribute 4), the population (11), the population cap (32), the
/// units queued (80) and the units in production (81).
pub const HEADROOM: i16 = 4;
pub const POPULATION: usize = 11;
pub const POPULATION_CAP: usize = 32;
pub const QUEUED: i16 = 80;
pub const IN_PRODUCTION: i16 = 81;

/// `FUN_1411e7dc0`'s test before it makes a unit's make action: the population with the units in
/// production and this one stays within the cap, and the headroom covers them. A building that
/// fails it waits (housed), which the slice does not model.
pub fn housed(attributes: &[f32], pop: f32) -> Result<bool, Unsupported> {
    let a = |i: usize| {
        attributes
            .get(i)
            .copied()
            .ok_or(Unsupported("an attribute the player does not have"))
    };
    let in_production = a(IN_PRODUCTION as usize)?;
    Ok(a(POPULATION_CAP)? < a(POPULATION)? + in_production + pop
        || (a(HEADROOM as usize)? - in_production) - pop < 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(train_time: f32) -> Production {
        Production {
            queue: VecDeque::from([Queued {
                master: 83,
                tech: -1,
                train_time,
                costs: [(-1, 0.0); 3],
            }]),
            action: true,
            progress: 0.0,
            rate: 1.0,
        }
    }

    #[test]
    fn the_progress_grows_by_rate_times_dt_until_the_unit_is_due() {
        let mut p = one(0.25);
        assert!(p.work(0.104).is_none());
        assert!(p.work(0.104).is_none());
        assert_eq!(p.progress, 0.104f32 + 0.104f32);
        assert!(p.work(0.104).is_some());
        p.spawned();
        assert!(p.queue.is_empty() && !p.action && p.progress == 0.0);
    }

    /// A villager stores −1 headroom, +1 population and +1 of attribute 19.
    #[test]
    fn a_villager_takes_one_population() {
        assert_eq!(population(&[(4, -1.0, 2), (11, 1.0, 2), (19, 1.0, 1)]), 1.0);
        let mut a = vec![0.0; 82];
        (a[4], a[11], a[32], a[81]) = (6.0, 4.0, 5.0, 0.0);
        assert!(!housed(&a, 1.0).unwrap());
        a[81] = 1.0;
        assert!(housed(&a, 1.0).unwrap());
    }
}
