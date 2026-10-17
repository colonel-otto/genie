//! A bird's Fly action (type 10, vtable `0x142ed4c88`; update `0x140a84b40`, set_state
//! `0x140a84a50`): it flies to a random point (state 3 picks it from the Action stream, state 4
//! flies there with a MoveTo sub-action) and rests between flights (state 2). Only the flight is
//! modelled, until its timer runs out or its MoveTo ends.

use crate::move_to::{MoveTo, Walk, walk_on};
use crate::unit_ai::Unsupported;

/// The Fly action's type, which word 9 adds.
pub const FLY: i16 = 10;

const FLYING: u8 = 4;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fly {
    /// `+0x18`.
    pub state: u8,
    /// `+0x44`, in seconds: the flight's time left.
    pub timer: f32,
    /// The MoveTo sub-action's state.
    pub move_to: Option<MoveTo>,
}

impl Fly {
    /// One update: the timer counts down by dt; a flight whose timer runs out rests (set_state 2),
    /// which the slice does not model; else the MoveTo flies on.
    pub fn update(&mut self, walk: Walk<'_>) -> Result<(), Unsupported> {
        if 0.0 < self.timer {
            self.timer -= walk.dt;
        }
        if self.state != FLYING {
            return Err(Unsupported("a bird resting or choosing its flight"));
        }
        if self.timer <= 0.0 {
            return Err(Unsupported("a bird's flight time runs out (set_state 2)"));
        }
        match self.move_to {
            Some(MoveTo::Moving) => {
                self.move_to = Some(walk_on(walk)?);
                Ok(())
            }
            _ => Err(Unsupported("a flight whose MoveTo is not moving")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::movement::Me;
    use crate::spatial::Spatial;

    #[test]
    fn a_flight_counts_down_and_stops_the_slice_when_its_time_runs_out() {
        let mut world = Spatial::default();
        fn walk(world: &mut Spatial) -> Walk<'_> {
            Walk {
                mover: None,
                me: Me {
                    id: 9925,
                    position: [0.0; 3],
                },
                dt: 0.5,
                world,
                plan: None,
            }
        }
        let mut fly = Fly {
            state: FLYING,
            timer: 0.75,
            move_to: Some(MoveTo::Moving),
        };
        // without movement state the walk itself is not modelled, after the countdown
        assert!(fly.update(walk(&mut world)).is_err());
        assert_eq!(fly.timer, 0.25);
        assert_eq!(
            fly.update(walk(&mut world)),
            Err(Unsupported("a bird's flight time runs out (set_state 2)"))
        );
    }
}
