//! A building's production: the queue QUEUE fills and the make action that works it off
//! (`TRIBE_Action_Make_Obj`, update `0x140ad0490`). Only the part before a unit spawns is modelled.

use crate::unit_ai::Unsupported;
use std::collections::VecDeque;

/// One unit in a building's queue.
#[derive(Clone, Debug, PartialEq)]
pub struct Queued {
    /// The master that spawns (a villager's male or female master is drawn when it is queued).
    pub master: i16,
    /// The seconds of work it takes (the master's train time at this building).
    pub train_time: f32,
}

/// A building's queue and its make action.
#[derive(Clone, Debug, PartialEq)]
pub struct Production {
    pub queue: VecDeque<Queued>,
    /// `+0x6c`: the seconds of work done on the first unit.
    pub progress: f32,
    /// The building's work rate: its `mWorkRatePowerup` plus its master's `work_rate`.
    pub rate: f32,
}

impl Production {
    /// The make action's state while it works, which the building's action-state slot reports.
    pub const MAKING: u8 = 6;

    /// One update of `dt_seconds`: the progress grows by the rate times dt. A unit whose train
    /// time is reached spawns, which the slice does not model.
    pub fn update(&mut self, dt_seconds: f32) -> Result<(), Unsupported> {
        self.progress += self.rate * dt_seconds;
        match self.queue.front() {
            Some(unit) if unit.train_time <= self.progress => Err(Unsupported(
                "a trained unit spawns (placement not modelled)",
            )),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(train_time: f32) -> Production {
        Production {
            queue: VecDeque::from([Queued {
                master: 83,
                train_time,
            }]),
            progress: 0.0,
            rate: 1.0,
        }
    }

    #[test]
    fn the_progress_grows_by_rate_times_dt_until_the_unit_is_due() {
        let mut p = one(0.25);
        p.update(0.104).unwrap();
        p.update(0.104).unwrap();
        assert_eq!(p.progress, 0.104f32 + 0.104f32);
        assert!(p.update(0.104).is_err());
    }
}
