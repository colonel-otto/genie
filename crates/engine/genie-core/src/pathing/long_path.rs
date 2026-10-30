//! The long pather (`FUN_140f9eb70`, run on the global at `0x1439e6910`): a best-first search over
//! the map's tiles from a unit's tile to a goal tile, which the reachability test of a point
//! (`FUN_140fa0370`, [`crate::movement::Mover::reaches`]) runs for a goal more than a tile from
//! the unit.
//!
//! A tile's cost is its distance walked so far plus its squared distance to the goal, both in
//! quarter tiles (a straight step costs 4, a diagonal 8, a change of direction 1 more); the open
//! tiles sit in a binary heap by cost, whose sift down the game runs only while the count's half
//! is above 1 and only to children below that half: the slice keeps both quirks. A tile is
//! entered when its terrain class is in the unit's mask, no static record blocks it (the
//! obstruction manager's word, [`crate::map::spatial::BLOCKED`]) and no tall object stands on it (the
//! map's count at `+0x10`, less the unit's own and the ignored units', `FUN_140f60440`); a
//! diagonal step also needs both tiles beside it so. The search ends at the goal tile or within
//! a tile of it (or, with `within_range`, within the range), after too many expansions, or when
//! the heap runs dry.
//!
//! With a target object the goal is its footprint's tiles (each of which the search enters past
//! its static record, not past a tall object), it counts as reached at once within the range of
//! its footprint (`FUN_14118f680`), and a unit already in range (`FUN_14118f8c0`) needs no search.
//!
//! The path the search writes when asked ([`path`], `FUN_140f9d770`) runs back from the tile it
//! ended on along the directions it entered each tile by; a search that ran out writes the path
//! to the tile it came nearest the goal on and answers no.
//!
//! Not modelled: an allied gate on the way (the tile's record of class 5, slot 0x678; another
//! player's gate is closed) and the search
//! beside a player's objects when the caller names one (argument 11, `FUN_14152f790`). The pather's flag at `+0x2db638` (terrain the unit
//! may cross anyway, `FUN_1413fc2b0`) is never set in this build: no instruction writes it.

use crate::GenieError;
use crate::map::Footprint;
use crate::map::GridSize;
use crate::map::Position;
use crate::map::spatial::{BLOCKED, Tiles};
use crate::math::fmath::BVector;
use crate::pathing::tile_path::Path;
use crate::world::slots::CheckedIndex;

/// A heap pop finds the heap empty where the caller checked its top.
const EMPTY_HEAP: GenieError = GenieError::unreachable("a heap pop without a top");

/// The search expands at most this many tiles (`FUN_140f9eb70`: `20000 < n` breaks).
const MOST_EXPANSIONS: i32 = 20000;

/// A human player's search expands at most this many hundredths of the straight distance in
/// tiles (`DAT_143f0f358`, which the static initializer at `0x140167e90` sets to 7500, or to 10000
/// with `DAT_143f01832`, which no run of this build has set).
const HUMAN_PERCENT: i32 = 7500;

/// A computer player's search expands at most this many tiles per tile of straight distance
/// (`FUN_140f9eb70` after `FUN_141659cf0`).
const COMPUTER_FACTOR: i32 = 25;

/// The heap's slot 0 (`+0x119424`), below every cost: a sift up stops there.
const SENTINEL: i32 = 0x88ca_6c00_u32 as i32;

/// The goal counts as reached within this many tiles, and a range above it ends the search
/// before it starts when the goal lies within the range (`DAT_14330eb8c`).
const REACHED: f32 = crate::math::image::long_path::REACHED;

/// The margin of the reached test within the range (`DAT_142eb0c60`).
const EPSILON: f32 = crate::math::image::long_path::EPSILON;

/// A target's footprint ends this short of its far edges (`DAT_142eb99cc`).
const EDGE_GAP: f32 = crate::math::image::long_path::EDGE_GAP;

/// A path point lies at its tile's centre: the tile plus this on both axes (`DAT_142e7f384`).
const TILE_CENTRE: f32 = crate::math::image::long_path::TILE_CENTRE;

/// The record class whose tiles an allied unit may pass when its gate lets it (`0x500` in the
/// obstruction manager's word, bits 8 to 13).
const GATE_CLASS_BITS: u64 = 0x500;

/// The tiles one step away in the order the search tries them (the switch of `FUN_140f9eb70`).
const STEPS: [[i32; 2]; 8] = [
    [-1, -1],
    [0, -1],
    [1, -1],
    [1, 0],
    [1, 1],
    [0, 1],
    [-1, 1],
    [-1, 0],
];

/// The directions a tile entered by a step in direction `came` does not try (the table at
/// `DAT_142f43a28`: the three that turn back, bits `came + 3` to `came + 5`; none for the start,
/// `came` 8).
fn behind(came: u8) -> u8 {
    if 8 <= came {
        return 0;
    }
    (0x38u8).rotate_left(u32::from(came))
}

/// The target object of a search (`FUN_140f9eb70`'s argument 8).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LongTarget {
    /// Its tile, and the tiles of its footprint (x0, y0, x1, y1: the centre less the radius, the
    /// centre plus the radius less `EDGE_GAP`, whole).
    pub tile: [i32; 2],
    /// The tiles of its footprint (x0, y0, x1, y1).
    pub rect: [i32; 4],
    /// The unit's footprint gap to it, squared (`FUN_14118f8c0`) and not (`FUN_14118f680`).
    pub gap_squared: f32,
    /// The gap, not squared (`FUN_14118f680`).
    pub gap: f32,
}

impl LongTarget {
    /// The target at `position` with radii `radius`, `gap` and `gap_squared` from the unit.
    #[must_use]
    pub fn new(position: Position, radius: Footprint, gap: f32, gap_squared: f32) -> Self {
        let (x, y) = (position.x(), position.y());
        Self {
            tile: [x as i32, y as i32],
            rect: [
                (x - radius.x()) as i32,
                (y - radius.y()) as i32,
                ((x + radius.x()) - EDGE_GAP) as i32,
                ((y + radius.y()) - EDGE_GAP) as i32,
            ],
            gap_squared,
            gap,
        }
    }
}

/// What the search is asked: from one tile to another, within `range`.
#[derive(Clone, Debug, PartialEq)]
pub struct LongRequest {
    /// The start tile.
    pub from: [i32; 2],
    /// The goal tile.
    pub to: [i32; 2],
    /// How near counts as there.
    pub range: f32,
    /// The target object, None for a point.
    pub target: Option<LongTarget>,
    /// The unit moves: its speed (slot 0x208, `FUN_141202c70`) is not 0, or its master is 42.
    pub moves: bool,
    /// The terrain classes the unit may stand on (its restriction's mask).
    pub mask: u8,
    /// The owner is a computer player (`FUN_141659cf0`): a shorter budget.
    pub computer: bool,
    /// The tiles of the ignored units the search lowers the tall count of, once each
    /// (`FUN_140f60440`: the unit itself and its moving ignored units, not inside another, whose
    /// master has a height).
    pub lowered: Vec<[i32; 2]>,
    /// Argument 13: the goal counts as reached within the range (else within a tile).
    pub within_range: bool,
    /// Argument 11 is not −1: a blocked tile with no object of the player around counts as open.
    pub near_player: bool,
    /// The players the unit's player counts as allies (`FUN_141663dc0` with the ally flag), one
    /// bit each: a gate's tile of another player is closed.
    pub allies: u64,
}

/// The open tiles: a binary heap from slot 1, slot 0 the sentinel (`+0x119420`, count
/// `+0x2db428`).
struct Heap {
    slots: Vec<([i32; 2], i32)>,
}

impl Heap {
    const fn count(&self) -> i32 {
        self.slots.len() as i32 - 1
    }

    /// `FUN_140fa0bd0` on a slot just filled: it rises while its cost is at most its parent's.
    fn push(&mut self, at: [i32; 2], cost: i32) -> Result<(), GenieError> {
        self.slots.push((at, cost));
        let mut i = self.slots.len() - 1;
        let mut parent = i >> 1;
        if self.slots.checked(parent)?.1 < cost {
            return Ok(());
        }
        loop {
            *self.slots.checked_mut(i)? = *self.slots.checked_mut(parent)?;
            i /= 2;
            parent /= 2;
            if self.slots.checked(parent)?.1 < cost {
                break;
            }
        }
        *self.slots.checked_mut(i)? = (at, cost);
        Ok(())
    }

    /// The pop after the top's expansion check (`FUN_140f9eb70`): the last slot moves to the top
    /// and sinks, the game's way: only while the count's half is above 1, only to a child below
    /// that half, past a child whose cost is not above it.
    fn pop(&mut self) -> Result<(), GenieError> {
        let last = self.slots.pop().ok_or(EMPTY_HEAP)?;
        let count = self.count();
        if count < 1 {
            return Ok(());
        }
        *self.slots.checked_mut(1)? = last;
        let half = count / 2;
        if half <= 1 {
            return Ok(());
        }
        let mut i = 1usize;
        loop {
            let mut child = 2 * i;
            if (child as i32) < count
                && self.slots.checked(child + 1)?.1 < self.slots.checked(child)?.1
            {
                child += 1;
            }
            if last.1 < self.slots.checked(child)?.1 {
                break;
            }
            *self.slots.checked_mut(i)? = *self.slots.checked_mut(child)?;
            i = child;
            if half <= child as i32 {
                break;
            }
        }
        if i != 1 {
            *self.slots.checked_mut(i)? = last;
        }
        Ok(())
    }
}

/// How a search ended ([`search`]).
enum Searched {
    /// An end outside the pather's map, or a unit that does not move: no, and no path.
    Refused,
    /// The goal within the range before the search: yes, and no path.
    InRange,
    /// The search ran: it reached the goal on `end` (`reached`), or ran out with `end` the tile
    /// it came nearest the goal on; `entered` holds the direction each tile was entered by.
    Ran {
        reached: bool,
        end: [i32; 2],
        entered: Vec<Option<u8>>,
    },
}

/// `FUN_140f9eb70` for a goal point without a path to write: whether the unit reaches it.
pub fn reaches(tiles: &Tiles, map: GridSize, req: &LongRequest) -> Result<bool, GenieError> {
    Ok(match search(tiles, map, req)? {
        Searched::Refused => false,
        Searched::InRange => true,
        Searched::Ran { reached, .. } => reached,
    })
}

/// `FUN_140f9eb70` with a path to write (its argument 12): whether the unit reaches the goal, and
/// the path ([`trace`] from the tile the search ended on, at the unit's height `z`; empty when the
/// search did not run). A path the trace cannot write (a point equal to the one beside it) is
/// emptied and answers no.
///
/// # Errors
///
/// What the search does not model.
pub fn path(
    tiles: &Tiles,
    map: GridSize,
    req: &LongRequest,
    z: f32,
) -> Result<(bool, Path), GenieError> {
    Ok(match search(tiles, map, req)? {
        Searched::Refused => (false, Path::default()),
        Searched::InRange => (true, Path::default()),
        Searched::Ran {
            reached,
            end,
            entered,
        } => trace(req.from, end, &entered, map.width(), z)
            .map_or_else(|| (false, Path::default()), |path| (reached, path)),
    })
}

/// `FUN_140f9d770`: the path from `from` to `end`, traced back from `end` along the direction
/// each tile was entered by: a tile whose direction differs from the one after it on the way back
/// (the end's always) puts its centre (x + 0.5, `z`, y + 0.5, the paths' layout) in front
/// (`FUN_140f95a50`), the start tile's last. None when a point equals the one beside it
/// (`FUN_140f95060` refuses it).
fn trace(
    from: [i32; 2],
    end: [i32; 2],
    entered: &[Option<u8>],
    width: i32,
    z: f32,
) -> Option<Path> {
    let direction = |[x, y]: [i32; 2]| {
        entered
            .get((y * width + x) as usize)
            .copied()
            .flatten()
            .unwrap_or(0xff)
    };
    let centre = |[x, y]: [i32; 2]| -> BVector {
        BVector::new(x as f32 + TILE_CENTRE, z, y as f32 + TILE_CENTRE)
    };
    let mut path = Path::default();
    let mut at = end;
    let mut came = direction(at);
    let mut before = 0xff;
    while at != from {
        if came != before && !path.insert(0, centre(at)) {
            return None;
        }
        // back along the step it was entered by
        let step = STEPS.get(usize::from(came))?;
        at = [at[0] - step[0], at[1] - step[1]];
        before = came;
        came = direction(at);
    }
    path.insert(0, centre(at)).then_some(path)
}

/// [`search`]'s answers before the search: an end off the pather's width (`+0x8`, the far edge
/// included) is refused; a target within range, or without a target a goal within a range
/// above [`REACHED`] (`gap2` the ends' squared gap), is in range; a unit that does not move is
/// refused. None: the search runs.
fn search_refused(map: GridSize, req: &LongRequest, gap2: i32) -> Option<Searched> {
    let (w, _) = (map.width(), map.height());
    let held = |v: i32| 0 <= v && v <= w;
    let [x0, y0] = req.from;
    let [x1, y1] = req.to;
    if !(held(x0) && held(y0) && held(x1) && held(y1)) {
        return Some(Searched::Refused);
    }
    if req
        .target
        .is_some_and(|t| t.gap_squared <= req.range * req.range)
    {
        return Some(Searched::InRange);
    }
    if REACHED < req.range && req.target.is_none() && (gap2 as f32) < req.range * req.range {
        return Some(Searched::InRange);
    }
    (!req.moves).then_some(Searched::Refused)
}

/// [`search`]'s goal test of the popped tile `d` tiles from the goal: a target's gap within the
/// range ([`EPSILON`]) or the tile within [`REACHED`]; without a target, a search to within
/// range by `d` against the range, else `d` within [`REACHED`].
fn search_reached(req: &LongRequest, d: f32) -> bool {
    match req.target {
        Some(t) => t.gap - req.range < EPSILON || d < REACHED,
        None if req.within_range => d - req.range < EPSILON,
        None => d < REACHED,
    }
}

/// [`search`]'s cost of the step from `from` (its cost `here`) to `to`: the step's length in
/// quarter tiles squared over 4, and the estimate to the goal (`goal`, quarter tiles squared
/// per axis) moved from `from`'s to `to`'s, in the game's wrapping arithmetic.
const fn step_cost(here: i32, from: [i32; 2], to: [i32; 2], goal: [i32; 2]) -> i32 {
    const fn quad(a: i32) -> i32 {
        (a * 4).wrapping_mul(a * 4)
    }
    let walked = quad(to[1] - from[1]).wrapping_add(quad(to[0] - from[0])) >> 2;
    walked
        .wrapping_add(here)
        .wrapping_sub(quad(goal[1] - from[1]))
        .wrapping_sub(quad(goal[0] - from[0]))
        .wrapping_add(quad(goal[1] - to[1]))
        .wrapping_add(quad(goal[0] - to[0]))
}

/// The tiles beside a diagonal step (directions 0, 2, 4, 6) onto `n`, which the step needs as
/// clear as the tile.
const fn beside_diagonal(dir: u8, [nx, ny]: [i32; 2]) -> Option<[[i32; 2]; 2]> {
    match dir {
        0 => Some([[nx + 1, ny], [nx, ny + 1]]),
        2 => Some([[nx, ny + 1], [nx - 1, ny]]),
        4 => Some([[nx - 1, ny], [nx, ny - 1]]),
        6 => Some([[nx, ny - 1], [nx + 1, ny]]),
        _ => None,
    }
}

/// What [`search`] reads of the tiles: the obstruction words, the tall objects less those the
/// request lowers (`lowered`, one each), and the unit's blocked terrain classes with and
/// without [`BLOCKED`].
struct Grid<'a> {
    tiles: &'a Tiles,
    width: i32,
    lowered: Vec<u8>,
    blocked_terrain: u64,
    blocked: u64,
}

impl<'a> Grid<'a> {
    fn new(tiles: &'a Tiles, map: GridSize, req: &LongRequest) -> Self {
        let (w, h) = (map.width(), map.height());
        let blocked_terrain = u64::from(req.mask ^ 0xff);
        let mut lowered = vec![0u8; (w.max(0) * h.max(0)) as usize];
        for &[x, y] in &req.lowered {
            if 0 <= x && x < w && 0 <= y && y < h {
                let i = (y * w + x) as usize;
                if let Some(cell) = lowered.get_mut(i) {
                    *cell = cell.wrapping_add(1);
                }
            }
        }
        Self {
            tiles,
            width: w,
            lowered,
            blocked_terrain,
            blocked: blocked_terrain | BLOCKED,
        }
    }

    fn word(&self, x: i32, y: i32) -> u64 {
        self.tiles.word(x, y).unwrap_or(0)
    }

    fn tall(&self, x: i32, y: i32) -> u8 {
        let Some(t) = self.tiles.tall(x, y) else {
            return 0;
        };
        t.wrapping_sub(
            self.lowered
                .get((y * self.width + x) as usize)
                .copied()
                .unwrap_or(0),
        )
    }

    /// The step onto `n` (with the tiles `beside` a diagonal): its terrain and the beside tiles'
    /// passable, else it is skipped; then open when the tile, the beside tiles and their tall
    /// objects are clear, or, at the goal (`at_goal`) of a target, the target's own static
    /// record; another's gate is closed.
    ///
    /// # Errors
    ///
    /// An allied gate's tile (slot 0x678) and a closed tile beside a player's objects
    /// (`FUN_14152f790`).
    fn step_open(
        &self,
        [nx, ny]: [i32; 2],
        beside: Option<[[i32; 2]; 2]>,
        at_goal: bool,
        req: &LongRequest,
    ) -> Result<bool, GenieError> {
        let corners = |test: &dyn Fn(i32, i32) -> bool| {
            beside.is_none_or(|b| b.iter().all(|&[x, y]| test(x, y)))
        };
        let clear = |test: &dyn Fn(i32, i32) -> bool| test(nx, ny) && corners(test);
        if !clear(&|x, y| self.word(x, y) & self.blocked_terrain == 0) {
            return Ok(false);
        }
        let passes = || {
            corners(&|x, y| self.word(x, y) & self.blocked == 0)
                && self.tall(nx, ny) == 0
                && corners(&|x, y| self.tall(x, y) == 0)
        };
        let word = self.word(nx, ny);
        let open = if word & self.blocked == 0 {
            passes()
        } else if word & 0x3f00 == GATE_CLASS_BITS {
            // a gate's record: its owner (bits 16 to 22) an ally of the unit's player
            // (`FUN_141663dc0` with the ally flag) asks the gate on the tile's anchor (slot
            // 0x678); another's gate is closed
            let owner = (word >> 16) & 0x7f;
            if req.allies & (1 << owner) != 0 {
                return Err(GenieError::unsupported(
                    "the long pather at an allied gate's tile (slot 0x678)",
                ));
            }
            false
        } else {
            // the target's own tiles pass their static record
            at_goal && req.target.is_some() && passes()
        };
        if !open && req.near_player {
            return Err(GenieError::unsupported(
                "the long pather beside a player's objects (FUN_14152f790)",
            ));
        }
        Ok(open)
    }
}

/// The search's tables: the cost of each tile, the direction it was entered by this search and
/// the heap of the tiles to expand.
struct Front {
    cost: Vec<i32>,
    entered: Vec<Option<u8>>,
    heap: Heap,
}

/// The tile the search expands from: its place, its cost and the direction it was entered by.
struct Origin {
    at: [i32; 2],
    cost: i32,
    came: Option<u8>,
}

/// What the search's steps share: the tiles, the request, the map's size and the goal.
struct Relax<'a> {
    grid: Grid<'a>,
    req: &'a LongRequest,
    map: GridSize,
    goal: [i32; 2],
    rect: [i32; 4],
}

impl Relax<'_> {
    /// The step from `from` onto tile `to` in direction `dir`: skipped when the tile was reached
    /// as cheaply or is not open; else its cost and direction are kept and it joins the heap.
    /// Returns the tile when it is the goal.
    fn step(
        &self,
        front: &mut Front,
        from: &Origin,
        (to, dir): ([i32; 2], u8),
    ) -> Result<Option<[i32; 2]>, GenieError> {
        let [nx, ny] = to;
        let [x1, y1] = self.req.to;
        let i = (ny * self.map.width() + nx) as usize;
        let new_cost = step_cost(from.cost, from.at, to, [x1, y1]);
        let seen = *front.entered.checked(i)?;
        let old = *front.cost.checked(i)?;
        if seen.is_some() && new_cost >= old {
            return Ok(None);
        }
        let rect = self.rect;
        let in_rect = rect[0] <= nx && nx <= rect[2] && rect[1] <= ny && ny <= rect[3];
        let at_goal = (nx == self.goal[0] && ny == self.goal[1]) || in_rect;
        if !self
            .grid
            .step_open(to, beside_diagonal(dir, to), at_goal, self.req)?
        {
            return Ok(None);
        }
        let cur_dir = from.came.unwrap_or(0xff);
        let mut c = if dir == cur_dir {
            new_cost
        } else {
            new_cost.wrapping_add(1)
        };
        if at_goal {
            c = 0;
        }
        if seen.is_none() || c < old {
            *front.cost.checked_mut(i)? = c;
            *front.entered.checked_mut(i)? = Some(dir);
            front.heap.push(to, c)?;
            if at_goal {
                return Ok(Some(to));
            }
        }
        Ok(None)
    }
}

/// The search of `FUN_140f9eb70` ([`reaches`], [`path`]).
fn search(tiles: &Tiles, map: GridSize, req: &LongRequest) -> Result<Searched, GenieError> {
    let (w, h) = (map.width(), map.height());
    let [x0, y0] = req.from;
    let [x1, y1] = req.to;
    let (dx, dy) = (x1 - x0, y1 - y0);
    let gap2 = dy.wrapping_mul(dy).wrapping_add(dx.wrapping_mul(dx));
    if let Some(early) = search_refused(map, req, gap2) {
        return Ok(early);
    }
    // the goal: the target's tile and footprint, else the goal tile
    let (goal, rect) = req
        .target
        .map_or(([x1, y1], [x1, y1, x1, y1]), |t| (t.tile, t.rect));
    let in_rect = |x: i32, y: i32| rect[0] <= x && x <= rect[2] && rect[1] <= y && y <= rect[3];
    let n = (w.max(0) * h.max(0)) as usize;
    let index = |x: i32, y: i32| (y * w + x) as usize;
    let relax = Relax {
        grid: Grid::new(tiles, map, req),
        req,
        map,
        goal,
        rect,
    };
    let quad = |a: i32| (a * 4).wrapping_mul(a * 4);
    // a tile's cost (`+0x38420`) and, entered this search, the direction it was entered by
    // (`+0x20`, under the search's generation `+0x2db430`: an older one reads as never entered)
    let mut front = Front {
        cost: vec![0i32; n],
        entered: vec![None; n],
        heap: Heap { slots: Vec::new() },
    };
    let start_cost = quad(dy).wrapping_add(quad(dx));
    *front.cost.checked_mut(index(x0, y0))? = start_cost;
    front.heap.slots = vec![([0, 0], SENTINEL), ([x0, y0], start_cost)];
    let straight = crate::math::fmath::sqrt(gap2 as f32) as i32;
    let budget = if req.computer {
        straight.wrapping_mul(COMPUTER_FACTOR)
    } else {
        straight.wrapping_mul(HUMAN_PERCENT) / 100
    };
    // the popped tile nearest the goal (`local_52c`, its distance `local_4f8`: the start's own
    // in quarter tiles squared, a popped tile's a quarter of that)
    let mut nearest = ([x0, y0], start_cost);
    let mut expansions = 0;
    while 0 < front.heap.count() {
        if budget < expansions || MOST_EXPANSIONS < expansions {
            break;
        }
        let ([cx, cy], _) = *front.heap.slots.checked(1)?;
        let (gx, gy) = (x1 - cx, y1 - cy);
        let d =
            crate::math::fmath::sqrt(gx.wrapping_mul(gx).wrapping_add(gy.wrapping_mul(gy)) as f32);
        if in_rect(cx, cy) || search_reached(req, d) {
            return Ok(Searched::Ran {
                reached: true,
                end: [cx, cy],
                entered: front.entered,
            });
        }
        let near = quad(y1 - cy).wrapping_add(quad(x1 - cx)) >> 2;
        if near < nearest.1 {
            nearest = ([cx, cy], near);
        }
        front.heap.pop()?;
        let came = *front.entered.checked(index(cx, cy))?;
        let back = behind(came.unwrap_or(8));
        let here = *front.cost.checked(index(cx, cy))?;
        for (dir, step) in STEPS.iter().enumerate() {
            let dir = dir as u8;
            let (nx, ny) = (cx + step[0], cy + step[1]);
            if back & (1 << dir) != 0
                || nx < 0
                || ny < 0
                || w <= nx
                || h <= ny
                || (nx == x0 && ny == y0)
            {
                continue;
            }
            let from = Origin {
                at: [cx, cy],
                cost: here,
                came,
            };
            if let Some(end) = relax.step(&mut front, &from, ([nx, ny], dir))? {
                return Ok(Searched::Ran {
                    reached: true,
                    end,
                    entered: front.entered,
                });
            }
        }
        expansions += 1;
    }
    // the nearest tile goes on the heap with cost 0, so the path runs to it
    Ok(Searched::Ran {
        reached: false,
        end: nearest.0,
        entered: front.entered,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A map of `rows` ('#' a blocked tile, 'T' a tall object, 'G' player 4's gate record, '.'
    /// open), class 1 everywhere.
    fn tiles(rows: &[&str]) -> (Tiles, GridSize) {
        let (w, h) = (rows[0].len() as i32, rows.len() as i32);
        let mut classes = Vec::new();
        for r in rows {
            for c in r.chars() {
                classes.push(if c == '#' { 2 } else { 1 });
            }
        }
        let mut t = Tiles::new(w, h, classes);
        for (y, r) in rows.iter().enumerate() {
            for (x, c) in r.chars().enumerate() {
                if c == 'T' {
                    t.add_tall_for_test(x as i32, y as i32);
                }
                if c == 'G' {
                    t.mark_for_test(x as i32, y as i32, BLOCKED | GATE_CLASS_BITS | 4 << 16);
                }
            }
        }
        (t, GridSize::new(w, h))
    }

    fn request(from: [i32; 2], to: [i32; 2]) -> LongRequest {
        LongRequest {
            from,
            to,
            range: 3.0,
            target: None,
            moves: true,
            mask: 1,
            computer: false,
            lowered: Vec::new(),
            within_range: false,
            near_player: false,
            allies: 0b10,
        }
    }

    #[test]
    fn the_path_runs_from_the_start_over_its_turns_to_the_goal() {
        // around the wall: from (1, 2) up past its end and down to (8, 2); each point a tile's
        // centre at the unit's height, where the direction changes
        let (t, map) = tiles(&[
            "..........",
            ".....#....",
            ".....#....",
            ".....#....",
            "..........",
        ]);
        let (reached, walk) = path(&t, map, &request([1, 2], [8, 2]), 0.25).unwrap();
        assert!(reached);
        let first = walk.points[0];
        let last = *walk.points.last().unwrap();
        assert_eq!(
            (first, last),
            (BVector::new(1.5, 0.25, 2.5), BVector::new(8.5, 0.25, 2.5))
        );
        // every stretch between two points is a straight or diagonal run of tiles
        for w in walk.points.windows(2) {
            let (dx, dy) = (w[1].x() - w[0].x(), w[1].y() - w[0].y());
            assert!(dx == 0.0 || dy == 0.0 || dx.abs() == dy.abs(), "{w:?}");
        }
        assert!(
            walk.points
                .iter()
                .all(|p| p.y() != 2.5 || p.x() < 5.0 || 6.0 < p.x())
        );
        // behind a closed wall the search runs out: the path to the tile nearest the goal
        let (t, map) = tiles(&[
            ".....#....",
            ".....#....",
            ".....#....",
            ".....#....",
            ".....#....",
        ]);
        let (reached, walk) = path(&t, map, &request([1, 2], [8, 2]), 0.0).unwrap();
        assert!(!reached);
        assert_eq!(walk.points.first(), Some(&BVector::new(1.5, 0.0, 2.5)));
        assert_eq!(walk.points.last(), Some(&BVector::new(4.5, 0.0, 2.5)));
        // a goal within the range: yes, and no path
        let mut req = request([0, 1], [4, 1]);
        req.range = 4.5;
        assert_eq!(path(&t, map, &req, 0.0).unwrap(), (true, Path::default()));
    }

    #[test]
    fn a_trace_puts_a_point_where_the_direction_changes() {
        // entered: (1, 0) from (0, 0) by step 3 ([1, 0]), (2, 0) by 3, (3, 1) by 4 ([1, 1])
        let w = 4;
        let mut entered = vec![None; 8];
        entered[1] = Some(3);
        entered[2] = Some(3);
        entered[w + 3] = Some(4);
        let p = trace([0, 0], [3, 1], &entered, w as i32, 0.0).unwrap();
        assert_eq!(
            p.points,
            [
                BVector::new(0.5, 0.0, 0.5),
                BVector::new(2.5, 0.0, 0.5),
                BVector::new(3.5, 0.0, 1.5)
            ]
        );
        // the end on the start: its centre only
        let p = trace([0, 0], [0, 0], &entered, w as i32, 0.0).unwrap();
        assert_eq!(p.points, [BVector::new(0.5, 0.0, 0.5)]);
    }

    #[test]
    fn the_table_of_turning_back() {
        let table: Vec<u8> = (0..9).map(behind).collect();
        assert_eq!(
            table,
            [0x38, 0x70, 0xe0, 0xc1, 0x83, 0x07, 0x0e, 0x1c, 0x00]
        );
    }

    #[test]
    fn a_goal_around_a_wall_is_reached_one_behind_a_closed_wall_not() {
        let (t, map) = tiles(&[
            "..........",
            ".....#....",
            ".....#....",
            ".....#....",
            "..........",
        ]);
        assert_eq!(reaches(&t, map, &request([1, 2], [8, 2])), Ok(true));
        let (t, map) = tiles(&[
            ".....#....",
            ".....#....",
            ".....#....",
            ".....#....",
            ".....#....",
        ]);
        assert_eq!(reaches(&t, map, &request([1, 2], [8, 2])), Ok(false));
    }

    /// The 4v4's scout 27272 (player 1) in update 4627 tests a goal across the wall of its enemy,
    /// player 4, whose gate's tiles hold their record: the gate is closed to it; an ally's gate
    /// asks the gate (slot 0x678, not modelled).
    #[test]
    fn an_enemy_gate_is_closed() {
        let (t, map) = tiles(&[
            ".....#....",
            ".....#....",
            ".....G....",
            ".....#....",
            ".....#....",
        ]);
        assert_eq!(reaches(&t, map, &request([1, 2], [8, 2])), Ok(false));
        let mut req = request([1, 2], [8, 2]);
        req.allies |= 1 << 4;
        assert!(reaches(&t, map, &req).is_err());
    }

    #[test]
    fn a_diagonal_step_needs_both_tiles_beside_it() {
        // the only way through is a corner between two tall objects
        let (t, map) = tiles(&[
            "....T....",
            "....T....",
            "....T....",
            "...T.....",
            "...T.....",
        ]);
        assert_eq!(reaches(&t, map, &request([0, 0], [8, 4])), Ok(false));
    }

    #[test]
    fn a_goal_within_the_range_ends_before_the_search() {
        let (t, map) = tiles(&["..#..", "..#..", "..#.."]);
        let mut req = request([0, 1], [4, 1]);
        req.moves = false;
        assert_eq!(reaches(&t, map, &req), Ok(false));
        req.range = 4.5;
        assert_eq!(reaches(&t, map, &req), Ok(true));
    }

    #[test]
    fn the_heap_sinks_the_games_way() {
        let mut h = Heap {
            slots: vec![([0, 0], SENTINEL)],
        };
        for (i, c) in [5, 3, 8, 1, 9, 2].into_iter().enumerate() {
            h.push([i as i32, 0], c).unwrap();
        }
        assert_eq!(h.slots[1].1, 1);
        h.pop().unwrap();
        // count 5, half 2: no sinking past the top's children below 2
        let costs: Vec<i32> = h.slots[1..].iter().map(|s| s.1).collect();
        assert_eq!(costs, [2, 3, 8, 5, 9]);
    }
}
