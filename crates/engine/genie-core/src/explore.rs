//! The Explore action (`RGE_Action_Explore`, type 4, made by `FUN_140a87d30`) and the auto scout's
//! search for a point to explore (`FUN_140c65490`, the unit AI's "unexploredArea").
//!
//! 158041: first (slot 0x28) `0x140a87e10`, update (slot 0x30) `0x140a880c0`, the point (slot
//! 0x48) `0x140a881a0`, set_state (slot 0x80) `0x140a87e30`. An Explore without a point waits in
//! state 3 for its unit AI to pick one (notify 0x202 with action 605, which the AI handles on its
//! player's turn); with a point it walks there in state 11 with a quiet MoveTo, and when the MoveTo
//! is done it goes back to state 3.

use crate::move_to::MoveTo;
use crate::unit_ai::Unsupported;

/// The Explore action's type.
pub const EXPLORE: i16 = 4;

/// The states the slice models.
pub const DONE: u8 = 1;
pub const STOPPED: u8 = 2;
pub const WAIT: u8 = 3;
pub const WALK: u8 = 11;
pub const FAILED: u8 = 13;

/// A point of −1 is no point (`DAT_14330ec20`).
const NO_POINT: f32 = crate::image::explore::NO_POINT;

/// The action's notify to its unit AI when it waits for a point (state 3): 0x202 with the
/// explore action value 605.
pub const WANTS_POINT: i32 = 0x202;
pub const EXPLORE_ACTION: i32 = 0x25d;

/// What an Explore's set_state asks of its owner besides its own fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    /// Nothing.
    Nothing,
    /// State 3: notify 0x202 (605) to its unit AI (slot 0x328 with its own id twice), then the
    /// standing sprite (slot 0x68 with the master's `+0x470`).
    WantsPoint,
    /// States 1, 13 and 14: the standing sprite.
    Standing,
    /// State 2: the owner's slot 0x68 and the action's own, its target none: not modelled.
    Stopped,
}

/// An Explore action.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExploreAction {
    /// `+0x18`.
    pub state: u8,
    /// The point it walks to (`+0x38`..`+0x40`), x −1 for none.
    pub point: [f32; 3],
    /// Its MoveTo to the point (state 11's sub-action, quiet: `+0x60` set, range 0, the master's
    /// moving sprite `+0x748` as no task names one).
    pub move_to: Option<MoveTo>,
}

impl ExploreAction {
    /// `FUN_140a87d30`, then its first (slot 0x28, `0x140a87e10`) as the action list adds it:
    /// state 3 without a point, else 11.
    pub fn new(point: [f32; 3]) -> (Self, Owner) {
        let mut a = ExploreAction {
            state: 0,
            point,
            move_to: None,
        };
        let state = if a.point[0] != NO_POINT { WALK } else { WAIT };
        let owner = a.set_state(state);
        (a, owner)
    }

    /// Set_state (slot 0x80, `0x140a87e30`): its sub-actions go (`FUN_140a98620`), then by state:
    /// 3 asks its unit AI for a point, 11 makes the MoveTo to the point (`FUN_140aa0d30` with
    /// range 0, made quiet by `FUN_140a7a170`; the action list starts it, `FUN_140a977e0`: state
    /// 3 for a unit with a speed).
    pub fn set_state(&mut self, state: u8) -> Owner {
        self.move_to = None;
        self.state = state;
        match state {
            STOPPED => {
                self.point = [-1.0; 3];
                Owner::Stopped
            }
            WAIT => Owner::WantsPoint,
            DONE | FAILED | 14 => Owner::Standing,
            WALK => {
                self.move_to = Some(MoveTo::Start);
                Owner::Nothing
            }
            _ => Owner::Nothing,
        }
    }

    /// The point (slot 0x48, `0x140a881a0`): one on the map's quarter (both coordinates at least
    /// 0) becomes its point and it walks there (set_state 11); any other makes it wait for one
    /// (set_state 3).
    pub fn take_point(&mut self, point: [f32; 3]) -> Owner {
        if 0.0 <= point[0] && 0.0 <= point[1] {
            self.point = point;
            return self.set_state(WALK);
        }
        self.set_state(WAIT)
    }

    /// What the update does after its MoveTo's update returned `code` (`0x140a880c0` in state 11,
    /// `FUN_140a992b0`): 0 goes on; 1, 2, 3 and 5 make it wait for a new point (set_state 3); 4
    /// runs the action's slot 0x68 and walks again (set_state 11), which is not modelled.
    pub fn after_walk(&mut self, code: u8) -> Result<Owner, Unsupported> {
        match code {
            0 => Ok(Owner::Nothing),
            1 | 2 | 3 | 5 => Ok(self.set_state(WAIT)),
            4 => Err(Unsupported(
                "an Explore whose MoveTo returned 4 (its slot 0x68)",
            )),
            _ => Ok(Owner::Nothing),
        }
    }
}

/// The search's rings around the town centre: 10, 15 and 20 tiles (`FUN_140c65490`).
const RINGS: [i32; 3] = [10, 15, 20];

/// The eight points of a ring in the order the search tries them (`FUN_140c65490`'s switch):
/// down-left first, then round through down, down-right, right, up-right, up, up-left and left.
const AROUND: [[i32; 2]; 8] = [
    [-1, -1],
    [0, -1],
    [1, -1],
    [1, 0],
    [1, 1],
    [0, 1],
    [-1, 1],
    [-1, 0],
];

/// What the search asks of a point (x, y, whole tiles on the map's grid).
pub trait Area {
    /// The player saw the tile or sees it (`FUN_141285350` not 0).
    fn explored(&self, x: i32, y: i32) -> Result<bool, Unsupported>;
    /// The tile's zone for the unit's restriction (`FUN_1411958c0`), and the unit's own
    /// (`FUN_14118eea0`).
    fn zone(&mut self, x: i32, y: i32) -> u16;
    fn own_zone(&mut self) -> u16;
    /// The unit could stand at the point (slot 0x278, [`crate::spatial::Spatial::point_clear`]).
    fn clear(&self, x: i32, y: i32) -> Result<bool, Unsupported>;
    /// The unit reaches the point within its line of sight less a tile (slot 0x3d8,
    /// [`crate::movement::Mover::reaches`]).
    fn reaches(&mut self, x: i32, y: i32) -> Result<bool, Unsupported>;
    /// The gap from the unit's footprint to the point (`FUN_14118faf0`).
    fn distance(&self, x: i32, y: i32) -> f32;
}

/// `FUN_140c65490`: the closest point the unit may explore around its player's first town centre
/// (`FUN_1416597e0`, its tile `tc`), None without a town centre or a point. The points lie 10,
/// 15 and 20 tiles from the centre's tile in eight directions each; a point counts when it is on
/// the map (`map`, the player's visible map's size), the player has not seen its tile, it lies in
/// the unit's zone, the unit could stand there and reaches it; the first such point, or a
/// strictly closer one, wins. The player's explored-points grid (`player+0x1370`, which the
/// search would read instead of the explored tiles and mark the point found in) is never made
/// in the slice: the base notify makes it only when no point is found, which it does not model.
pub fn unexplored_area(
    tc: Option<[i32; 2]>,
    map: [i32; 2],
    area: &mut dyn Area,
) -> Result<Option<[i32; 2]>, Unsupported> {
    let Some([cx, cy]) = tc else {
        return Ok(None);
    };
    let own = area.own_zone();
    let mut best: Option<([i32; 2], f32)> = None;
    for ring in RINGS {
        for [ax, ay] in AROUND {
            let (x, y) = (cx + ax * ring, cy + ay * ring);
            if x < 0 || y < 0 || map[0] <= x || map[1] <= y {
                continue;
            }
            if area.explored(x, y)? {
                continue;
            }
            if area.zone(x, y) != own || !area.clear(x, y)? || !area.reaches(x, y)? {
                continue;
            }
            let d = area.distance(x, y);
            if best.is_none_or(|(_, b)| d < b) {
                best = Some(([x, y], d));
            }
        }
    }
    Ok(best.map(|(p, _)| p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_explore_without_a_point_waits_for_one_then_walks() {
        let (mut a, owner) = ExploreAction::new([-1.0, -1.0, 0.0]);
        assert_eq!((a.state, owner, a.move_to), (WAIT, Owner::WantsPoint, None));
        assert_eq!(a.take_point([19.0, 84.0, 0.0]), Owner::Nothing);
        assert_eq!((a.state, a.move_to), (WALK, Some(MoveTo::Start)));
        assert_eq!(a.after_walk(0), Ok(Owner::Nothing));
        assert_eq!(a.after_walk(1), Ok(Owner::WantsPoint));
        assert_eq!((a.state, a.move_to), (WAIT, None));
        assert_eq!(a.take_point([-1.0, 5.0, 0.0]), Owner::WantsPoint);
    }

    /// An area where every point but the unexplored ones counts, at its distance from (20, 92).
    struct Open {
        explored: Vec<[i32; 2]>,
        unreachable: Vec<[i32; 2]>,
    }

    impl Area for Open {
        fn explored(&self, x: i32, y: i32) -> Result<bool, Unsupported> {
            Ok(self.explored.contains(&[x, y]))
        }
        fn zone(&mut self, _: i32, _: i32) -> u16 {
            1
        }
        fn own_zone(&mut self) -> u16 {
            1
        }
        fn clear(&self, _: i32, _: i32) -> Result<bool, Unsupported> {
            Ok(true)
        }
        fn reaches(&mut self, x: i32, y: i32) -> Result<bool, Unsupported> {
            Ok(!self.unreachable.contains(&[x, y]))
        }
        fn distance(&self, x: i32, y: i32) -> f32 {
            let (dx, dy) = ((x - 20) as f32, (y - 92) as f32);
            crate::fmath::sqrt(dx * dx + dy * dy)
        }
    }

    #[test]
    fn the_closest_open_point_of_the_rings_wins() {
        // MarvinBlue/426048731's first pick: the town centre at (29, 94), the scout near (21, 92)
        let mut area = Open {
            explored: vec![],
            unreachable: vec![],
        };
        assert_eq!(
            unexplored_area(Some([29, 94]), [120, 120], &mut area),
            Ok(Some([19, 94]))
        );
        area.explored = vec![[19, 94], [19, 104]];
        assert_eq!(
            unexplored_area(Some([29, 94]), [120, 120], &mut area),
            Ok(Some([14, 94]))
        );
        area.explored.push([14, 94]);
        area.unreachable = vec![[19, 84]];
        assert_eq!(
            unexplored_area(Some([29, 94]), [120, 120], &mut area),
            Ok(Some([9, 94]))
        );
        assert_eq!(unexplored_area(None, [120, 120], &mut area), Ok(None));
    }
}
