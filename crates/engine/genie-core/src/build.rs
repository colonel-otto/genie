//! A builder's Build action (`TRIBE_Action_Build`, type 101, vtable `0x142ed6628`) up to the
//! builder's walk: the action starts in state 4 with a MoveTo sub-action (`RGE_Action_Move_To`,
//! type 1) toward the foundation; a builder already at the foundation works in its first update
//! (state 6). A walking builder's MoveTo runs the motion step ([`crate::movement`]).
//!
//! A builder at a foundation no one started faces it and checks its footprint (state 7): a builder
//! standing on it walks to a free tile beside it (state 223) and comes back (state 4). The start
//! of the construction, the arrivals and the path request are not modelled.

use crate::move_to::{Goal, start_walk, walk_on};
pub use crate::move_to::{MoveTo, Walk};
use crate::unit_ai::Unsupported;

/// The Build action's type, which word 9 adds.
pub const BUILD: i16 = 101;

/// The gap below which a MoveTo has arrived (`DAT_142eb99cc`).
const ARRIVED: f32 = 0.01;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BuildAction {
    /// The foundation's id.
    pub target: i32,
    /// The action state (`+0x18`), which word 9 adds.
    pub state: u8,
    pub move_to: Option<MoveTo>,
    /// The point the MoveTo walks to (its `+0x38`..`+0x40`), when not to the foundation.
    pub goal: Option<[f32; 3]>,
    /// The builder's Build task's work range (task `+0x30`): its MoveTo to the foundation stops
    /// this near.
    pub range: f32,
}

/// The Build action's states the slice models.
const MOVE: u8 = 4;
const WORK: u8 = 6;
const PLACE: u8 = 7;
/// 223 (`0xdf`): off the footprint, to a free tile beside it.
const CLEAR: u8 = 223;

impl BuildAction {
    /// The action's start (slot 5, `0x140aadd80`): state 4 for a target, whose set_state
    /// (`0x140aae200`) creates the MoveTo; the MoveTo starts arrived if the footprints touch.
    pub fn start(target: i32, gap: f32, range: f32) -> Self {
        BuildAction {
            target,
            state: MOVE,
            move_to: Some(if gap > ARRIVED {
                MoveTo::Start
            } else {
                MoveTo::Arrived
            }),
            goal: None,
            range,
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
                start_walk(walk, Goal::Target(self.target), self.range)
            }
            (4, Some(MoveTo::Moving)) => walk_on(walk),
            (4, None) => Err(Unsupported("a Build action in state 4 without its MoveTo")),
            (WORK, _) => {
                // case 5: FUN_141162db0 turns the builder to face its foundation
                let mover = walk
                    .mover
                    .as_deref()
                    .ok_or(Unsupported("a builder without movement state"))?;
                if !mover.stats.turns_at_once {
                    return Err(Unsupported(
                        "a builder that turns over time to face its work",
                    ));
                }
                self.place(&walk)
            }
            (CLEAR, Some(MoveTo::Start)) => {
                self.move_to = Some(MoveTo::Moving);
                let goal = self.goal.expect("set_state(223) sets the goal");
                start_walk(walk, Goal::Point(goal), 0.0)
            }
            (CLEAR, Some(MoveTo::Moving)) => walk_on(walk),
            (CLEAR, _) => Err(Unsupported(
                "a builder off its footprint (its return, set_state 4)",
            )),
            _ => Err(Unsupported("a Build state not modelled")),
        }
    }

    /// set_state(7) (`0x140aae200` case 7) on a foundation no builder started: clear the
    /// footprint of what may be built over, find who stands on it, and send this builder off it
    /// if it stands there itself.
    fn place(&mut self, walk: &Walk<'_>) -> Result<(), Unsupported> {
        self.move_to = None;
        self.goal = None;
        self.state = PLACE;
        let world = walk.world;
        let site = world
            .get_known(self.target)?
            .ok_or(Unsupported("a foundation that is gone"))?;
        if site.obstruction.is_some() {
            return Err(Unsupported(
                "a builder at a started foundation (LAB_140aaf9f4)",
            ));
        }
        if !world.footprint_clearing(self.target, site)?.is_empty() {
            return Err(Unsupported("a foundation clears objects off its footprint"));
        }
        if site.obstruction_type == 0 || site.obstruction_type == 12 {
            return Err(Unsupported("a foundation whose master does not obstruct"));
        }
        let occupants = world.footprint_occupants(self.target, site)?;
        if occupants.iter().any(|&id| id != walk.me.id) {
            return Err(Unsupported(
                "another unit on the footprint (its evacuation)",
            ));
        }
        if world.foreign_marks(site) {
            return Err(Unsupported("another player's marks on the footprint"));
        }
        if occupants.is_empty() {
            return Err(Unsupported(
                "a builder starts the construction (LAB_140aaf9f4)",
            ));
        }
        let [bx, by, _] = walk.me.position;
        let [sx, sy, _] = site.position;
        let inside = (bx - sx).abs() <= site.radius[0] && (by - sy).abs() <= site.radius[1];
        if !inside {
            return Err(Unsupported(
                "a builder waits for its footprint to clear (state 222)",
            ));
        }
        self.clear(walk)
    }

    /// set_state(223): a MoveTo to the free tile beside the footprint (`FUN_1411566b0`); it starts
    /// in state 3, as a MoveTo to a point is far from it (`FUN_140aa2100`).
    fn clear(&mut self, walk: &Walk<'_>) -> Result<(), Unsupported> {
        let mover = walk
            .mover
            .as_deref()
            .ok_or(Unsupported("a builder without movement state"))?;
        let site = walk.world.get(self.target).expect("place found it");
        let spot = walk
            .world
            .spot_beside(walk.me.position, mover.stats.restriction_mask, site)?
            .ok_or(Unsupported(
                "no free tile beside the footprint (set_state 2)",
            ))?;
        if mover.stats.speed == 0.0 {
            return Err(Unsupported("a builder that cannot move (MoveTo state 13)"));
        }
        self.state = CLEAR;
        self.move_to = Some(MoveTo::Start);
        self.goal = Some([spot[0], spot[1], walk.me.position[2]]);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::movement::Me;
    use crate::spatial::Spatial;

    /// MarvinBlue/426048731 after tick 679: builder 9175 stands on the house foundation 10028's
    /// footprint, works on it (state 6) and walks off it in update 681 (state 223) to the free tile
    /// the game picks, (29.5, 89.5).
    #[test]
    fn a_builder_on_its_footprint_faces_it_then_walks_to_a_free_tile_beside_it() {
        use crate::movement::{MoveStats, Mover};
        use crate::spatial::{Placed, Tiles};
        let mut world = Spatial::with_tiles([40, 120], Tiles::new(40, 120, vec![4; 40 * 120]));
        let unit = |x, y, radius, master, kind| Placed {
            position: [x, y, 0.0],
            radius: [radius, radius],
            class: 4,
            player: 2,
            master,
            obstruction_type: kind,
            buildable_on: 0,
            tall: true,
            speed: 0.0,
            unknown: None,
            obstruction: None,
        };
        world.insert(9175, unit(30.0, 89.0, 0.2, 118, 5));
        world.insert(10028, unit(30.0, 88.0, 1.0, 70, 2));
        let mut mover = Mover {
            forward: [1.0, 0.0, 0.0],
            right: [0.0, 0.0, -1.0],
            velocity: None,
            paths: vec![],
            factor: 1.0,
            stats: MoveStats {
                speed: 0.8,
                turn_rate: f32::MAX,
                turn_radius: 0.0,
                turn_radius_speed: f32::MAX,
                radius: [0.2, 0.2, 0.5],
                restriction: 7,
                restriction_mask: 6,
                turns_at_once: true,
            },
        };
        let mut build = BuildAction {
            target: 10028,
            state: 6,
            move_to: None,
            goal: None,
            range: 0.1,
        };
        build
            .update(Walk {
                mover: Some(&mut mover),
                me: Me {
                    id: 9175,
                    position: [30.0, 89.0, 0.0],
                },
                dt: 0.02,
                world: &world,
                plan: None,
            })
            .unwrap();
        assert_eq!(
            (build.state, build.move_to, build.goal),
            (223, Some(MoveTo::Start), Some([29.5, 89.5, 0.0]))
        );
    }

    #[test]
    fn a_builder_at_the_foundation_works_in_its_first_update_the_others_walk_to_it() {
        let mut near = BuildAction::start(10028, 0.0, 0.1);
        let mut far = BuildAction::start(10028, 0.8, 0.1);
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
            plan: None,
        };
        near.update(walk(1)).unwrap();
        assert_eq!((near.state, near.move_to), (6, None));
        // the far one asks for a path to its foundation, which must be on the map
        assert_eq!(
            far.update(walk(2)),
            Err(Unsupported("a MoveTo whose target is gone"))
        );
        assert_eq!(far.move_to, Some(MoveTo::Moving));
    }
}
