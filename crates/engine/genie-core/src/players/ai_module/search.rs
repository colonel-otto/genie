//! The explorer's search for an unexplored tile (`FUN_140b71990`, "closestUnexploredTile") and
//! its helpers on the information module: the path map's reads and the stack of points to try.
//!
//! The path map holds a byte per tile: [`UNEXPLORED`] (-1, the module's start), [`EXPLORED`]
//! (the explored-tile hook's 0), [`PUNTED`] (a tile its unit cannot path to) and [`DEAD_END`]
//! (a tile no search reached). A civilian takes a tile no search touched, any other unit one that
//! is not explored.
//!
//! The search floods from a tile over the map: the first tile that the player does not see now
//! and that counts takes the unit's pick, when the unit reaches it. A tile it cannot reach is
//! punted, and the search then starts again from the unit's own place; a search that already
//! starts there picks a corner of a quadrant by the scores the save holds and draws from the
//! unsynced stream.

use crate::GenieError;
use crate::map::Point;
use crate::math::conversions::cvttss2si;
use crate::math::fmath::{cos, sin};
use crate::math::rng::Well512;
use crate::movement::explore::Area;
use crate::objects::ObjectId;
use crate::players::ai_module::InfoModule;

/// A path map's byte of a tile no search has touched (-1).
pub const UNEXPLORED: u8 = 0xff;
/// The byte of an explored tile (the hook's, and a tile the search picked).
pub const EXPLORED: u8 = 0;
/// The byte of a tile its unit cannot path to.
pub const PUNTED: u8 = 1;
/// The byte of a tile that no search reached.
pub const DEAD_END: u8 = 2;

/// A tile in the map that the path map does not hold (the game reads what follows it).
const OFF_THE_GRID: GenieError = GenieError::unreachable("a tile the AI's path map does not hold");

/// The spacing of the points of a ring: the unit's line of sight and this (`DAT_142eb1c98`, the
/// constant the group port's catch-up span also reads).
pub const RING_SLACK: f32 = crate::math::image::group::CATCH_UP_SPAN;

/// What the search asks of the world beyond [`Area`].
pub trait SearchArea: Area {
    /// The player sees the tile now (`FUN_141285350` gives 0xf).
    fn visible_now(&self, x: i32, y: i32) -> bool;
}

/// What the search reads of the explorer.
#[derive(Clone, Copy, Debug)]
pub struct SearchUnit {
    /// Its id.
    pub id: ObjectId,
    /// Its place on the ground.
    pub position: Point,
    /// Its master's class is the civilian's (4).
    pub civilian: bool,
}

/// How a flood from one tile ended.
enum Flood {
    /// A tile (or a quadrant's corner) the unit goes to.
    Found([i32; 2]),
    /// The search starts again from the unit's place.
    Again,
    /// The flood reached every tile it could.
    Exhausted,
}

/// `!=` of two floats as the game's `UCOMISS` and `JNZ`/`JP` see it: a NaN differs.
fn differs(a: f32, b: f32) -> bool {
    a != b
}

impl InfoModule {
    /// The byte of tile (`x`, `y`) (`+0x105c8`).
    fn cell(&self, x: i32, y: i32) -> Result<u8, GenieError> {
        self.path_map.get(x, y).ok_or(OFF_THE_GRID)
    }

    /// `FUN_140b90270` ("tileUncovered"): the tile is on the map and neither unexplored (-1) nor
    /// a dead end (2).
    ///
    /// # Errors
    ///
    /// A tile of the map that the path map does not hold.
    pub fn tile_uncovered(&self, x: i32, y: i32) -> Result<bool, GenieError> {
        if x < 0 || y < 0 || self.width <= x || self.height <= y {
            return Ok(false);
        }
        let cell = self.cell(x, y)?;
        Ok(cell != UNEXPLORED && cell != DEAD_END)
    }

    /// `FUN_140b96860`: the stack's top, (-1, -1) for none.
    #[must_use]
    pub fn top_point(&self) -> [i32; 2] {
        self.search.last().copied().unwrap_or([-1, -1])
    }

    /// `FUN_140b97220`: the stack's top goes and is given, (-1, -1) for none.
    pub fn pop_point(&mut self) -> [i32; 2] {
        self.search.pop().unwrap_or([-1, -1])
    }

    /// `FUN_140b7d920`: the point is held on the map (the module's size); true when it moved.
    const fn clamp(&self, p: &mut [i32; 2]) -> bool {
        let mut moved = false;
        if p[0] < 0 {
            p[0] = 0;
            moved = true;
        } else if self.width <= p[0] {
            p[0] = self.width - 1;
            moved = true;
        }
        if p[1] < 0 {
            p[1] = 0;
            moved = true;
        } else if self.height <= p[1] {
            p[1] = self.height - 1;
            moved = true;
        }
        moved
    }

    /// `FUN_140b96cf0`: the eight points of a ring of `radius` about `centre`, every eighth of a
    /// turn from 0, go onto the stack when they lie on the map.
    pub fn push_ring(&mut self, radius: f32, centre: Point) {
        let (cx, cy) = (cvttss2si(centre.x()) as f32, cvttss2si(centre.y()) as f32);
        for i in 0..8 {
            let angle = i as f32 * crate::math::image::facing::QUARTER_PI;
            let mut p = [
                cvttss2si(cos(angle) * radius + cx),
                cvttss2si(sin(angle) * radius + cy),
            ];
            if !self.clamp(&mut p) {
                self.search.push(p);
            }
        }
    }

    /// `FUN_140b71990` for `unit`, starting at `start`: the tile its pick goes to, None when the
    /// flood reaches no tile that counts ([`SearchArea`]: `map` is the map's size, `rng` the
    /// unsynced stream).
    ///
    /// # Errors
    ///
    /// A tile of the map that the path map does not hold, or what the area's reads do not model.
    pub fn closest_unexplored_tile(
        &mut self,
        unit: &SearchUnit,
        start: Point,
        area: &mut dyn SearchArea,
        (map, rng): ([i32; 2], &mut Well512),
    ) -> Result<Option<[i32; 2]>, GenieError> {
        let own = area.own_zone() as u8;
        let mut start = start;
        loop {
            match self.flood(unit, start, own, area, (map, &mut *rng))? {
                Flood::Found(p) => return Ok(Some(p)),
                Flood::Again => start = unit.position,
                Flood::Exhausted => {
                    self.mark_dead_ends(own, area, map)?;
                    return Ok(None);
                }
            }
        }
    }

    /// One flood of the search: from the start tile (held on the module's map), each tile in
    /// the order the flood met them.
    fn flood(
        &mut self,
        unit: &SearchUnit,
        start: Point,
        own: u8,
        area: &mut dyn SearchArea,
        (map, rng): ([i32; 2], &mut Well512),
    ) -> Result<Flood, GenieError> {
        let mut first = [cvttss2si(start.x()), cvttss2si(start.y())];
        let _ = self.clamp(&mut first);
        let mut visited = vec![false; (map[0].max(0) * map[1].max(0)) as usize];
        let mut queue = vec![first];
        let mut i = 0;
        while let Some(&[x, y]) = queue.get(i) {
            i += 1;
            let cell = self.cell(x, y)?;
            // a civilian takes a tile no search touched, another unit any that is not explored
            let wanted = if unit.civilian {
                cell == UNEXPLORED
            } else {
                cell != EXPLORED
            };
            if area.visible_now(x, y) || !wanted {
                push_neighbours(&mut queue, &mut visited, [x, y], map);
                continue;
            }
            if area.zone(x, y) as u8 != own || !area.clear(x, y)? {
                continue;
            }
            if area.reaches(x, y)? {
                self.path_map.set(x, y, EXPLORED);
                return Ok(Flood::Found([x, y]));
            }
            self.path_map.set(x, y, PUNTED);
            if differs(start.x(), unit.position.x()) || differs(start.y(), unit.position.y()) {
                return Ok(Flood::Again);
            }
            return Ok(Flood::Found(self.punt(rng)));
        }
        Ok(Flood::Exhausted)
    }

    /// The punt search's pick: the quadrant of the map's 4 by 4 whose record scores the least
    /// (`first + draw % 100 + second * 25`, a draw for each record from the first column down),
    /// and one of its corners by a draw.
    fn punt(&self, rng: &mut Well512) -> [i32; 2] {
        let (w, h) = (self.width, self.height);
        let mut best = -1_i32;
        let mut corner = [0, 0];
        for (index, record) in self.quadrants.iter().enumerate() {
            let (column, row) = ((index / 4) as i32, (index % 4) as i32);
            let draw = (rng.rand15() as i32) % 100;
            let score = record[0]
                .wrapping_add(draw)
                .wrapping_add(record[1].wrapping_mul(0x19));
            if best == -1 || score < best {
                corner = [w * column / 4, row * h / 4];
                best = score;
            }
        }
        let far = [w / 4 - 1 + corner[0], h / 4 - 1 + corner[1]];
        match rng.rand15() & 3 {
            0 => corner,
            1 => [corner[0], far[1]],
            2 => far,
            _ => [far[0], corner[1]],
        }
    }

    /// `FUN_140b71490`, after a flood that found nothing: every tile of the map that is neither
    /// unexplored nor a dead end, lies in the unit's zone and is not seen now becomes a dead end.
    fn mark_dead_ends(
        &mut self,
        own: u8,
        area: &mut dyn SearchArea,
        map: [i32; 2],
    ) -> Result<(), GenieError> {
        for y in 0..map[1] {
            for x in 0..map[0] {
                let cell = self.cell(x, y)?;
                if cell != UNEXPLORED
                    && cell != DEAD_END
                    && area.zone(x, y) as u8 == own
                    && !area.visible_now(x, y)
                {
                    self.path_map.set(x, y, DEAD_END);
                }
            }
        }
        Ok(())
    }
}

/// `FUN_140b6c2d0`: the tile's neighbours on the map that the flood has not met join the queue
/// (left, right, up, down).
fn push_neighbours(queue: &mut Vec<[i32; 2]>, visited: &mut [bool], at: [i32; 2], map: [i32; 2]) {
    let [x, y] = at;
    for [nx, ny] in [[x - 1, y], [x + 1, y], [x, y - 1], [x, y + 1]] {
        if nx < 0 || ny < 0 || map[0] <= nx || map[1] <= ny {
            continue;
        }
        let index = (ny * map[0] + nx) as usize;
        if let Some(met) = visited.get_mut(index)
            && !*met
        {
            *met = true;
            queue.push([nx, ny]);
        }
    }
}
