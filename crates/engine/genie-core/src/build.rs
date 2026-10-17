//! A builder's Build action (`TRIBE_Action_Build`, type 101, vtable `0x142ed6628`) up to the
//! builder's walk: the action starts in state 4 with a MoveTo sub-action (`RGE_Action_Move_To`,
//! type 1) toward the foundation; a builder already at the foundation works in its first update
//! (state 6). A walking builder's MoveTo runs the motion step ([`crate::movement`]); the arrival,
//! and what a builder at the foundation does next, are not modelled.

use crate::movement::{Me, Motion, Mover};
use crate::spatial::Spatial;
use crate::unit_ai::Unsupported;

/// The Build action's type, which word 9 adds.
pub const BUILD: i16 = 101;

/// The gap below which a MoveTo has arrived (`DAT_142eb99cc`).
const ARRIVED: f32 = 0.01;

/// The MoveTo sub-action's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveTo {
    /// 1: at the target when it started.
    Arrived,
    /// 3: it starts to move (`0x140aa2100`).
    Start,
    /// 4: its first update asked for a path; the unit moves from the next update on.
    Moving,
}

/// What a builder's update needs to walk: its movement state, where it is, and the world.
pub struct Walk<'a> {
    pub mover: Option<&'a mut Mover>,
    pub me: Me,
    pub dt: f32,
    pub world: &'a Spatial,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuildAction {
    /// The foundation's id.
    pub target: i32,
    /// The action state (`+0x18`), which word 9 adds.
    pub state: u8,
    pub move_to: Option<MoveTo>,
}

impl BuildAction {
    /// The action's start (slot 5, `0x140aadd80`): state 4 for a target, whose set_state
    /// (`0x140aae200`) creates the MoveTo; the MoveTo starts arrived if the footprints touch.
    pub fn start(target: i32, gap: f32) -> Self {
        BuildAction {
            target,
            state: 4,
            move_to: Some(if gap > ARRIVED {
                MoveTo::Start
            } else {
                MoveTo::Arrived
            }),
        }
    }

    /// One update (slot 6, `0x140ab0370`), which first updates the MoveTo (`0x140aa2a70`); a
    /// moving MoveTo calls the motion step (object slot 193).
    pub fn update(&mut self, walk: Walk<'_>) -> Result<(), Unsupported> {
        match (self.state, self.move_to) {
            (4, Some(MoveTo::Arrived)) => {
                self.state = 6;
                self.move_to = None;
                Ok(())
            }
            (4, Some(MoveTo::Start)) => {
                self.move_to = Some(MoveTo::Moving);
                Ok(())
            }
            (4, Some(MoveTo::Moving)) => {
                let mover = walk
                    .mover
                    .ok_or(Unsupported("a walker without movement state"))?;
                if mover.paths.is_empty() {
                    return Err(Unsupported("the path request (FUN_14114e410)"));
                }
                match mover.motion(&walk.me, walk.dt, walk.world)? {
                    Motion::Moving => Ok(()),
                    Motion::Arrived => Err(Unsupported("a builder arrives at its foundation")),
                    Motion::Done => Err(Unsupported("a MoveTo whose unit cannot move on")),
                }
            }
            (4, None) => Err(Unsupported("a Build action in state 4 without its MoveTo")),
            _ => Err(Unsupported("a builder at its foundation (Build state 7)")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_builder_at_the_foundation_works_in_its_first_update_the_others_ask_for_a_path() {
        let mut near = BuildAction::start(10028, 0.0);
        let mut far = BuildAction::start(10028, 0.8);
        assert_eq!((near.state, near.move_to), (4, Some(MoveTo::Arrived)));
        assert_eq!((far.state, far.move_to), (4, Some(MoveTo::Start)));
        let world = Spatial::default();
        let walk = |id| Walk {
            mover: None,
            me: Me {
                id,
                position: [0.0; 3],
            },
            dt: 0.02,
            world: &world,
        };
        near.update(walk(1)).unwrap();
        far.update(walk(2)).unwrap();
        assert_eq!(
            (near.state, far.state, far.move_to),
            (6, 4, Some(MoveTo::Moving))
        );
        assert!(near.update(walk(1)).is_err());
        assert!(far.update(walk(2)).is_err());
    }
}
