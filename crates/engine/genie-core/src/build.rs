//! A builder's Build action (`TRIBE_Action_Build`, type 101, vtable `0x142ed6628`) up to the
//! builder's first step: the action starts in state 4 with a MoveTo sub-action
//! (`RGE_Action_Move_To`, type 1) toward the foundation; a builder already at the foundation works
//! in its first update (state 6). Movement, and what a builder at the foundation does next, are not
//! modelled.

use crate::unit_ai::Unsupported;

/// The Build action's type, which word 9 adds.
pub const BUILD: i16 = 101;

/// The gap below which a MoveTo has arrived (`DAT_142eb99cc`).
const ARRIVED: f32 = 0.01;

/// The distance between two objects' footprints (`FUN_14118f680`): per axis the centre distance
/// minus both radii, at least 0, then the length of the two. The game's square root is a software
/// `sqrtf` (`0x14099ef70`); this takes IEEE's correctly rounded one.
pub fn footprint_gap(a: [f32; 2], a_radius: [f32; 2], b: [f32; 2], b_radius: [f32; 2]) -> f32 {
    let axis = |i: usize| ((b[i] - a[i]).abs() - (b_radius[i] + a_radius[i])).max(0.0);
    let (dx, dy) = (axis(0), axis(1));
    (dx * dx + dy * dy).sqrt()
}

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

    /// One update (slot 6, `0x140ab0370`), which first updates the MoveTo.
    pub fn update(&mut self) -> Result<(), Unsupported> {
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
            (4, _) => Err(Unsupported("a builder walks to its foundation (movement)")),
            _ => Err(Unsupported("a builder at its foundation (Build state 7)")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VILLAGER: [f32; 2] = [0.2, 0.2];
    const HOUSE: [f32; 2] = [1.0, 1.0];

    #[test]
    fn the_gap_is_the_distance_between_the_footprints() {
        // MarvinBlue/426048731: a house at (30, 88), builders at (30, 89), (28, 89), (29, 99)
        assert_eq!(
            footprint_gap([30.0, 89.0], VILLAGER, [30.0, 88.0], HOUSE),
            0.0
        );
        assert_eq!(
            footprint_gap([28.0, 89.0], VILLAGER, [30.0, 88.0], HOUSE),
            2.0f32 - 1.2
        );
        let far = footprint_gap([29.0, 99.0], VILLAGER, [30.0, 88.0], HOUSE);
        assert_eq!(far, (11.0f32 - 1.2).hypot(0.0));
    }

    #[test]
    fn a_builder_at_the_foundation_works_in_its_first_update_the_others_ask_for_a_path() {
        let mut near = BuildAction::start(10028, 0.0);
        let mut far = BuildAction::start(10028, 0.8);
        assert_eq!((near.state, near.move_to), (4, Some(MoveTo::Arrived)));
        assert_eq!((far.state, far.move_to), (4, Some(MoveTo::Start)));
        near.update().unwrap();
        far.update().unwrap();
        assert_eq!(
            (near.state, far.state, far.move_to),
            (6, 4, Some(MoveTo::Moving))
        );
        assert!(near.update().is_err());
        assert!(far.update().is_err());
    }
}
