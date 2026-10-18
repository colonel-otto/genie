//! The Explore action (`RGE_Action_Explore`, type 4, made by `FUN_140a87d30`) and the auto scout's
//! search for a point to explore (`FUN_140c65490`, the unit AI's "unexploredArea").
//!
//! 158041: first (slot 0x28) `0x140a87e10`, update (slot 0x30) `0x140a880c0`, the point (slot
//! 0x48) `0x140a881a0`, `set_state` (slot 0x80) `0x140a87e30`. An Explore without a point waits in
//! state 3 for its unit AI to pick one (notify 0x202 with action 605, which the AI handles on its
//! player's turn); with a point it walks there in state 11 with a quiet `MoveTo`, and when the `MoveTo`
//! is done it goes back to state 3.

use crate::move_to::MoveTo;
use crate::unit_ai::Unsupported;

/// The Explore action's type.
pub const EXPLORE: i16 = 4;

/// The states the slice models.
pub const DONE: u8 = 1;
/// State 2: stopped.
pub const STOPPED: u8 = 2;
/// State 3: waiting for a point.
pub const WAIT: u8 = 3;
/// State 11: walking to the point.
pub const WALK: u8 = 11;
/// State 13: failed.
pub const FAILED: u8 = 13;

/// A point of −1 is no point (`DAT_14330ec20`).
const NO_POINT: f32 = crate::image::explore::NO_POINT;

/// The action's notify to its unit AI when it waits for a point (state 3): 0x202 with the
/// explore action value 605.
pub const WANTS_POINT: i32 = 0x202;
/// The explore action value (605).
pub const EXPLORE_ACTION: i32 = 0x25d;

/// What an Explore's `set_state` asks of its owner besides its own fields.
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
    /// Its `MoveTo` to the point (state 11's sub-action, quiet: `+0x60` set, range 0, the master's
    /// moving sprite `+0x748` as no task names one).
    pub move_to: Option<MoveTo>,
}

impl ExploreAction {
    /// `FUN_140a87d30`, then its first (slot 0x28, `0x140a87e10`) as the action list adds it:
    /// state 3 without a point, else 11.
    #[must_use]
    pub fn new(point: [f32; 3]) -> (Self, Owner) {
        let mut a = Self {
            state: 0,
            point,
            move_to: None,
        };
        let state = if a.point[0] == NO_POINT { WAIT } else { WALK };
        let owner = a.set_state(state);
        (a, owner)
    }

    /// `Set_state` (slot 0x80, `0x140a87e30`): its sub-actions go (`FUN_140a98620`), then by state:
    /// 3 asks its unit AI for a point, 11 makes the `MoveTo` to the point (`FUN_140aa0d30` with
    /// range 0, made quiet by `FUN_140a7a170`; the action list starts it, `FUN_140a977e0`: state
    /// 3 for a unit with a speed).
    pub const fn set_state(&mut self, state: u8) -> Owner {
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
    /// 0) becomes its point and it walks there (`set_state` 11); any other makes it wait for one
    /// (`set_state` 3).
    pub fn take_point(&mut self, point: [f32; 3]) -> Owner {
        if 0.0 <= point[0] && 0.0 <= point[1] {
            self.point = point;
            return self.set_state(WALK);
        }
        self.set_state(WAIT)
    }

    /// What the update does after its `MoveTo`'s update returned `code` (`0x140a880c0` in state 11,
    /// `FUN_140a992b0`): 0 goes on; 1, 2, 3 and 5 make it wait for a new point (`set_state` 3); 4
    /// runs the action's slot 0x68 and walks again (`set_state` 11), which is not modelled.
    pub const fn after_walk(&mut self, code: u8) -> Result<Owner, Unsupported> {
        match code {
            1 | 2 | 3 | 5 => Ok(self.set_state(WAIT)),
            4 => Err(Unsupported(
                "an Explore whose MoveTo returned 4 (its slot 0x68)",
            )),
            // 0 (and any other code) goes on
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
    /// The unit's own zone (`FUN_14118eea0`).
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

/// The order of an explore to a target point (`0x2da`): the search looks within 7 tiles of it.
const EXPLORE_POINT_ORDER: i32 = 0x2da;
/// How far past its line of sight the search looks, and about an explore's target point, in tiles.
const SEARCH_PAST_SIGHT: i32 = 3;
const SEARCH_ABOUT_POINT: i32 = 7;

/// What `closestUndiscoveredTile` reads of its unit and unit AI.
#[derive(Clone, Copy, Debug)]
pub struct Seeker {
    /// The unit's place.
    pub at: [f32; 3],
    /// Its master's line of sight (`+0x5d0`).
    pub line_of_sight: f32,
    /// The unit AI's current order (`+0x3c`).
    pub order: i32,
    /// The unit AI's target point (`+0x50`, `+0x54`; x is −1 without one).
    pub target_point: [f32; 2],
    /// The owner is a computer player (`FUN_141659cf0`, `player+0x10 == 3`).
    pub computer: bool,
}

/// `FUN_140c4bd40`, the unit AI's slot 0x90 (`closestUndiscoveredTile`, "the closest
/// undiscovered tile"): none for a computer player's unit. Else, after one draw of the AI stream
/// (`ai`), the tiles within the unit's line of sight and 3 more of its tile (within 7 of the target
/// point under an explore to it, order `0x2da`), held on the map (`map`), row by row: a tile the
/// player never saw, in the unit's zone, where the unit could stand (slot 0x278) counts by the
/// squared distance between the tiles (plus that from the target point's, when the AI has one),
/// and takes the place of the best when it is nearer than the best by more than the draw's slack
/// (`d < best + draw % 3`). The best then counts only when the unit reaches it (slot 0x3d8 within
/// its line of sight less a tile). The waypoint queue the search passes by (`+0xd8`, count
/// `+0x158`, not saved) is empty in the slice.
pub fn closest_undiscovered(
    area: &mut dyn Area,
    seeker: Seeker,
    map: [i32; 2],
    ai: &mut crate::rng::Well512,
) -> Result<Option<[i32; 2]>, Unsupported> {
    if seeker.computer {
        return Ok(None);
    }
    let [ux, uy, _] = seeker.at;
    let reach = (seeker.line_of_sight as i32 + SEARCH_PAST_SIGHT) as f32;
    let (mut x0, mut x1) = ((ux - reach) as i32, (reach + ux) as i32);
    let (mut y0, mut y1) = ((uy - reach) as i32, (reach + uy) as i32);
    let [px, py] = seeker.target_point;
    if seeker.order == EXPLORE_POINT_ORDER {
        (x0, x1) = (
            px as i32 - SEARCH_ABOUT_POINT,
            px as i32 + SEARCH_ABOUT_POINT,
        );
        (y0, y1) = (
            py as i32 - SEARCH_ABOUT_POINT,
            py as i32 + SEARCH_ABOUT_POINT,
        );
    }
    let (x0, y0) = (x0.max(0), y0.max(0));
    let (x1, y1) = (x1.min(map[0] - 1), y1.min(map[1] - 1));
    let slack = (ai.rand15() % 3) as i32;
    let own = area.own_zone();
    let (tx, ty) = (ux as i32, uy as i32);
    let mut best: Option<([i32; 2], i32)> = None;
    for y in y0..=y1 {
        for x in x0..=x1 {
            if area.explored(x, y)? || area.zone(x, y) != own || !area.clear(x, y)? {
                continue;
            }
            let mut d = (y - ty) * (y - ty) + (x - tx) * (x - tx);
            if px != NO_POINT {
                d += (y - py as i32) * (y - py as i32) + (x - px as i32) * (x - px as i32);
            }
            if best.is_none_or(|(_, b)| d < slack + b) {
                best = Some(([x, y], d));
            }
        }
    }
    match best {
        Some(([x, y], _)) if area.reaches(x, y)? => Ok(Some([x, y])),
        _ => Ok(None),
    }
}

/// How far past its line of sight the auto scout's last search draws its square, in tiles.
const SQUARE_PAST_SIGHT: i32 = 2;

/// `FUN_140c4b130`, the auto scout's last search when neither the search around the town centre
/// nor the closest undiscovered tile found a point (the base handler's case 0x202 on the auto
/// scout order): the edge of a square about the unit's tile (`at`), its line of sight and 2 more
/// tiles out, side by side (top from its left corner, right, bottom and left from the next
/// corner on, the tiles off the map left out; `map`), in an order `std::shuffle` draws from the AI
/// stream (`ai`): the first tile the player never saw, in the unit's zone, where it could stand
/// (slot 0x278) and that it reaches (slot 0x3d8 within its line of sight less a tile) wins.
/// Else a flood from the unit's tile through the tiles the player saw (left, right, up and down,
/// on the map, each once; the start tile is not marked and comes again) stops at the first
/// unseen tile it meets that counts as above; unseen tiles that do not count end their branch.
/// None when the flood runs out. The player's scout map (`player+0x1370`), which both would read
/// instead of the seen tiles and mark the point found in, is never made in the slice.
pub fn beyond_sight(
    area: &mut dyn Area,
    at: [f32; 3],
    line_of_sight: f32,
    map: [i32; 2],
    ai: &mut crate::rng::Well512,
) -> Result<Option<[i32; 2]>, Unsupported> {
    let own = area.own_zone();
    let [w, h] = map;
    let r = line_of_sight as i32 + SQUARE_PAST_SIGHT;
    let (ux, uy) = (at[0] as i32, at[1] as i32);
    let (x0, x1, y0, y1) = (ux - r, ux + r, uy - r, uy + r);
    let on_x = |x: i32| 0 <= x && x < w;
    let on_y = |y: i32| 0 <= y && y < h;
    let mut edge: Vec<[i32; 2]> = Vec::new();
    if 0 <= y0 {
        edge.extend((x0..x1).filter(|&x| on_x(x)).map(|x| [x, y0]));
    }
    if x1 < w {
        edge.extend((y0..y1).filter(|&y| on_y(y)).map(|y| [x1, y]));
    }
    if y1 < h {
        edge.extend((x0 + 1..=x1).filter(|&x| on_x(x)).map(|x| [x, y1]));
    }
    if 0 <= x0 {
        edge.extend((y0 + 1..=y1).filter(|&y| on_y(y)).map(|y| [x0, y]));
    }
    ai.shuffle(&mut edge);
    let counts = |area: &mut dyn Area, x: i32, y: i32| -> Result<bool, Unsupported> {
        Ok(area.zone(x, y) == own && area.clear(x, y)? && area.reaches(x, y)?)
    };
    for &[x, y] in &edge {
        if !area.explored(x, y)? && counts(area, x, y)? {
            return Ok(Some([x, y]));
        }
    }
    let mut seen = vec![false; (w * h).max(0) as usize];
    let mut queue = vec![[ux, uy]];
    let mut i = 0;
    while i < queue.len() {
        let [x, y] = queue[i];
        i += 1;
        if !area.explored(x, y)? {
            if counts(area, x, y)? {
                return Ok(Some([x, y]));
            }
            continue;
        }
        for [nx, ny] in [[x - 1, y], [x + 1, y], [x, y - 1], [x, y + 1]] {
            if !on_x(nx) || !on_y(ny) {
                continue;
            }
            let k = (ny * w + nx) as usize;
            if !seen[k] {
                seen[k] = true;
                queue.push([nx, ny]);
            }
        }
    }
    Ok(None)
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

    fn rng() -> crate::rng::Well512 {
        let mut r = crate::rng::Well512::default();
        r.set_seed(7);
        r
    }

    /// The last search's square: line of sight 2 makes it 4 tiles out. With every edge tile but
    /// one seen, that one wins whatever the shuffle; with the edge all seen, the flood walks the
    /// seen tiles to the first unseen one it meets.
    #[test]
    fn the_last_search_takes_the_square_then_floods_the_seen_tiles() {
        let square: Vec<[i32; 2]> = (16..=24)
            .flat_map(|x| (88..=96).map(move |y| [x, y]))
            .collect();
        let mut area = Open {
            explored: square.iter().copied().filter(|&p| p != [24, 90]).collect(),
            unreachable: vec![],
        };
        let found = beyond_sight(&mut area, [20.5, 92.5, 0.0], 2.0, [120, 120], &mut rng());
        assert_eq!(found, Ok(Some([24, 90])));
        // all of the square seen: the flood leaves it left first, at (15, 92)
        let mut area = Open {
            explored: square,
            unreachable: vec![],
        };
        let found = beyond_sight(&mut area, [20.5, 92.5, 0.0], 2.0, [120, 120], &mut rng());
        assert_eq!(found, Ok(Some([15, 92])));
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

    /// An area where only the listed tiles are unexplored.
    struct Patch {
        unexplored: Vec<[i32; 2]>,
        unreachable: Vec<[i32; 2]>,
    }

    impl Area for Patch {
        fn explored(&self, x: i32, y: i32) -> Result<bool, Unsupported> {
            Ok(!self.unexplored.contains(&[x, y]))
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
        fn distance(&self, _: i32, _: i32) -> f32 {
            0.0
        }
    }

    #[test]
    fn the_unit_ai_s_search_takes_the_nearest_unexplored_tile_it_reaches() {
        let seeker = Seeker {
            at: [20.5, 92.5, 0.0],
            line_of_sight: 4.0,
            order: -1,
            target_point: [NO_POINT, NO_POINT],
            computer: false,
        };
        let seeded = || {
            let mut r = crate::rng::Well512::default();
            r.set_seed(7);
            r
        };
        let mut area = Patch {
            // within the line of sight and 3: 16 and 25 tiles squared away, and one beyond
            unexplored: vec![[16, 92], [25, 92], [20, 97], [40, 92]],
            unreachable: vec![],
        };
        let mut ai = seeded();
        let found = closest_undiscovered(&mut area, seeker, [120, 120], &mut ai);
        assert_eq!(found, Ok(Some([16, 92])));
        // one draw of the AI stream
        let mut once = seeded();
        let _ = once.rand15();
        assert_eq!(ai.rand15(), once.rand15());
        // the best is the only one tested for reach
        area.unreachable.push([16, 92]);
        assert_eq!(
            closest_undiscovered(&mut area, seeker, [120, 120], &mut seeded()),
            Ok(None)
        );
        // a computer player's unit searches nothing
        let computer = Seeker {
            computer: true,
            ..seeker
        };
        let mut untouched = seeded();
        assert_eq!(
            closest_undiscovered(&mut area, computer, [120, 120], &mut untouched),
            Ok(None)
        );
        assert_eq!(untouched.rand15(), seeded().rand15());
    }
}
