//! Where the world's objects are, as movement reads them: each live object's position, radii and
//! class, and the obstruction record the game keeps for it in its obstruction manager
//! (`DAT_143f0a088`, queried by `FUN_141051900`).
//!
//! The game files its records in a five-level grid; the slice keeps them by object id. The grid
//! only orders a query's hits, and the slice stops a walker at the first possible hit
//! ([`Spatial::obstruction_candidates`]), so it needs no order yet.
//!
//! [`Tiles`] holds what the game keeps per tile: the obstruction manager's word (terrain class,
//! static records' marks) and the counts of records and objects whose centre is on the tile.

use crate::unit_ai::Unsupported;
use std::collections::BTreeMap;

/// Obstruction kinds the query tests (`FUN_141051e00`): boxes (2, 3, 11, 13) and circles (5).
const TESTED_KINDS: [u8; 5] = [2, 3, 5, 11, 13];

/// One object as movement sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    /// x, y, z on the map.
    pub position: [f32; 3],
    /// The master's radii on the ground (master `+0x5d8`, `+0x5dc`).
    pub radius: [f32; 2],
    /// The master's class (`+0x410`, decoded): the motion step treats gates (39) apart.
    pub class: i16,
    /// The owning player.
    pub player: usize,
    /// The master's id.
    pub master: i16,
    /// The master's `obstruction_type` (`+0x651`), whether or not the object has its record yet.
    pub obstruction_type: u8,
    /// The master's `can_be_built_on` (`+0x5e9`): a foundation clears such objects off its
    /// footprint.
    pub buildable_on: u8,
    /// The master's height radius (`+0x5e0`) is above 0: the tile counts it as tall.
    pub tall: bool,
    /// The top speed, tiles per second; 0 for an object that does not move.
    pub speed: f32,
    /// How far from `position` the object may be, when its player's update is not modelled: it
    /// grows by its speed every tick ([`Spatial::grow_unknown`]). None for an object the slice
    /// updates.
    pub unknown: Option<f32>,
    /// The obstruction record, if the object has one: a foundation gets its record only when
    /// construction starts.
    pub obstruction: Option<Obstruction>,
}

/// An obstruction record's kind (its byte `+1`, the master's `obstruction_type`, master `+0x651`)
/// and the master's `obstruction_class` (`+0x652`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Obstruction {
    pub kind: u8,
    pub class: u8,
}

impl Obstruction {
    /// The kinds that mark the tiles under them (`FUN_141056460`: 0, 1, 2, 11, 13 and 10, which the
    /// game files as 2).
    fn marks_tiles(self) -> bool {
        self.kind == 10 || (self.kind < 14 && (0x2807u32 >> self.kind) & 1 != 0)
    }
}

impl Placed {
    /// The record's box `x0, y0, x1, y1` (`+0x20`..`+0x2c`): the position plus and minus the radii.
    pub fn bounds(&self) -> [f32; 4] {
        let [x, y, _] = self.position;
        let [rx, ry] = self.radius;
        [x - rx, y - ry, x + rx, y + ry]
    }
}

/// The tile bit a static record sets (`0x4000`, in `FUN_141056460`): the tile is blocked.
pub const BLOCKED: u64 = 0x4000;

/// What the game keeps per tile, row by row.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tiles {
    width: i32,
    height: i32,
    /// The obstruction manager's word (`DAT_143f0a088 + 0x98`): bits 0 to 7 the terrain's class,
    /// bits 8 to 13 a static record's obstruction class, [`BLOCKED`], bit 15 for class 5, bits 16
    /// to 22 the record's player, bits 23 and up a bit per player that marked the tile.
    words: Vec<u64>,
    /// Tall objects (a height radius above 0) whose centre is on the tile (the map's bytes at
    /// `+0x10`, `FUN_140f5f0a0`); the passability test queries the obstructions of a tile with two.
    tall: Vec<u8>,
    /// Objects whose centre is on the tile (the tile's count at `+0x1b`).
    objects: Vec<u8>,
}

impl Tiles {
    /// A map whose tiles hold `classes` (one terrain class bit per tile, row by row) and nothing
    /// else.
    pub fn new(width: i32, height: i32, classes: Vec<u8>) -> Self {
        let n = classes.len();
        assert_eq!(
            n,
            (width.max(0) * height.max(0)) as usize,
            "one class per tile"
        );
        Tiles {
            width,
            height,
            words: classes.into_iter().map(u64::from).collect(),
            tall: vec![0; n],
            objects: vec![0; n],
        }
    }

    fn index(&self, x: i32, y: i32) -> Option<usize> {
        (0 <= x && x < self.width && 0 <= y && y < self.height)
            .then(|| (y * self.width + x) as usize)
    }

    pub fn word(&self, x: i32, y: i32) -> Option<u64> {
        self.index(x, y).map(|i| self.words[i])
    }

    pub fn tall(&self, x: i32, y: i32) -> Option<u8> {
        self.index(x, y).map(|i| self.tall[i])
    }

    pub fn objects(&self, x: i32, y: i32) -> Option<u8> {
        self.index(x, y).map(|i| self.objects[i])
    }

    /// Count an object in or out of the tile its centre is on.
    fn count(&mut self, position: [f32; 3], tall: bool, add: bool) {
        let Some(i) = self.index(position[0] as i32, position[1] as i32) else {
            return;
        };
        let step = |c: &mut u8| {
            *c = if add {
                c.wrapping_add(1)
            } else {
                c.wrapping_sub(1)
            }
        };
        step(&mut self.objects[i]);
        if tall {
            step(&mut self.tall[i]);
        }
    }

    /// The tiles a static record covers (`FUN_141056460`): from the box's low edge to its high
    /// edge less 0.001, held on the map.
    fn covered(
        &self,
        p: &Placed,
    ) -> (std::ops::RangeInclusive<i32>, std::ops::RangeInclusive<i32>) {
        let [x, y, _] = p.position;
        let [rx, ry] = p.radius;
        let hold = |v: i32, size: i32| v.clamp(0, size - 1);
        let xs = hold((x - rx) as i32, self.width)..=hold(((x + rx) - 0.001) as i32, self.width);
        let ys = hold((y - ry) as i32, self.height)..=hold(((y + ry) - 0.001) as i32, self.height);
        (xs, ys)
    }

    /// A static record marks the tiles it covers that no player marked yet.
    fn mark(&mut self, p: &Placed, o: Obstruction) {
        let player = p.player as u64;
        let mut bits =
            ((player << 8 | u64::from(o.class & 0x3f) | 0x40) << 8) | (0x80_0000 << player);
        if o.class == 5 {
            bits |= 0x8000;
        }
        let (xs, ys) = self.covered(p);
        for y in ys {
            for x in xs.clone() {
                let i = self.index(x, y).expect("covered tiles are on the map");
                if self.words[i] < 0x80_0000 {
                    self.words[i] = (self.words[i] & !0xff_ff00) | bits;
                }
            }
        }
    }

    /// A static record leaves: its tiles keep only their terrain class (`FUN_141055980`).
    fn unmark(&mut self, p: &Placed) {
        let (xs, ys) = self.covered(p);
        for y in ys {
            for x in xs.clone() {
                let i = self.index(x, y).expect("covered tiles are on the map");
                self.words[i] &= 0xff;
            }
        }
    }
}

/// The world's objects by id, the map's size in tiles and what the game keeps per tile.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Spatial {
    pub map: [i32; 2],
    objects: BTreeMap<i32, Placed>,
    tiles: Tiles,
}

impl Spatial {
    /// A map without terrain: every tile's class is 0.
    pub fn new(map: [i32; 2]) -> Self {
        let n = (map[0].max(0) * map[1].max(0)) as usize;
        Self::with_tiles(map, Tiles::new(map[0], map[1], vec![0; n]))
    }

    pub fn with_tiles(map: [i32; 2], tiles: Tiles) -> Self {
        Spatial {
            map,
            objects: BTreeMap::new(),
            tiles,
        }
    }

    pub fn tiles(&self) -> &Tiles {
        &self.tiles
    }

    pub fn get(&self, id: i32) -> Option<&Placed> {
        self.objects.get(&id)
    }

    /// An object whose place the slice knows: an object of a player the slice stopped is known
    /// while it cannot have moved.
    pub fn get_known(&self, id: i32) -> Result<Option<&Placed>, Unsupported> {
        match self.objects.get(&id) {
            Some(p) if p.unknown.is_some_and(|u| 0.0 < u) => Err(Unsupported(
                "an object of a stopped player (its place is unknown)",
            )),
            found => Ok(found),
        }
    }

    /// The slice stops updating the object: from now on its place is known only within its speed
    /// times the time since.
    pub fn freeze(&mut self, id: i32) {
        if let Some(p) = self.objects.get_mut(&id) {
            p.unknown.get_or_insert(0.0);
        }
    }

    /// A tick of `dt` seconds passed: every frozen object may have moved its speed times dt.
    pub fn grow_unknown(&mut self, dt: f32) {
        for p in self.objects.values_mut() {
            if let Some(u) = p.unknown.as_mut() {
                *u += p.speed * dt;
            }
        }
    }

    /// Fails when a frozen object may be within `rect` (`x0, y0, x1, y1`): the answer of a query
    /// over it depends on where that object really is.
    pub fn check_known(&self, rect: [f32; 4]) -> Result<(), Unsupported> {
        let near = self.objects.values().any(|p| {
            p.unknown.is_some_and(|u| {
                let [x0, y0, x1, y1] = p.bounds();
                rect[0] <= x1 + u && x0 - u <= rect[2] && rect[1] <= y1 + u && y0 - u <= rect[3]
            })
        });
        if near {
            return Err(Unsupported("an object of a stopped player may be near"));
        }
        Ok(())
    }

    /// A new object, in the order the game adds them: a static record marks only the tiles no
    /// earlier record marked.
    pub fn insert(&mut self, id: i32, placed: Placed) {
        self.remove(id);
        self.tiles.count(placed.position, placed.tall, true);
        if let Some(o) = placed.obstruction.filter(|o| o.marks_tiles()) {
            self.tiles.mark(&placed, o);
        }
        self.objects.insert(id, placed);
    }

    pub fn remove(&mut self, id: i32) {
        if let Some(gone) = self.objects.remove(&id) {
            self.tiles.count(gone.position, gone.tall, false);
            if gone.obstruction.is_some_and(|o| o.marks_tiles()) {
                self.tiles.unmark(&gone);
            }
        }
    }

    /// Each obstruction record: the object's id, the record's kind and box.
    pub fn records(&self) -> impl Iterator<Item = (i32, u8, [f32; 4])> + '_ {
        self.objects
            .iter()
            .filter_map(|(&id, p)| Some((id, p.obstruction?.kind, p.bounds())))
    }

    /// The object moved: its record moves with it, and it changes tiles when its centre crosses
    /// a tile's edge (`RGE_Static_Object::new_position`, `0x1411a58b0`).
    pub fn set_position(&mut self, id: i32, position: [f32; 3]) {
        if let Some(p) = self.objects.get_mut(&id) {
            self.tiles.count(p.position, p.tall, false);
            p.position = position;
            self.tiles.count(p.position, p.tall, true);
        }
    }

    /// A player's ghost of an object it saw (a doppelganger, type 25): it counts on its tile, and
    /// movement sees nothing else of it. The slice keeps the ghosts of the save and makes none.
    pub fn insert_ghost(&mut self, position: [f32; 3]) {
        self.tiles.count(position, false, true);
    }

    /// Whether a unit with terrain mask `mask` can stand on tile (`x`, `y`) (`FUN_14114d460`): the
    /// tile is on the map, its class is in the mask and no static record blocks it. A tile with
    /// two tall objects needs the game's point query, which the slice does not model.
    pub fn passable(&self, x: i32, y: i32, mask: u8) -> Result<bool, Unsupported> {
        let Some(word) = self.tiles.word(x, y) else {
            return Ok(false);
        };
        if word & (u64::from(mask ^ 0xff) | BLOCKED) != 0 {
            return Ok(false);
        }
        if 1 < self.tiles.tall(x, y).expect("a tile with a word") {
            return Err(Unsupported(
                "a tile with two tall objects (its point query)",
            ));
        }
        Ok(true)
    }

    /// The tile beside `site`'s footprint a unit at `from` with terrain mask `mask` goes to
    /// (`FUN_1411566b0`, closest): a flood over passable tiles from the footprint's own, at most
    /// `(int)(rx + ry) + 1` tiles from its centre; of the tiles found outside the footprint, the
    /// first empty one, else the closest to the unit, nearer ones replacing it in the flood's
    /// order. Returns its centre.
    pub fn spot_beside(
        &self,
        from: [f32; 3],
        mask: u8,
        site: &Placed,
    ) -> Result<Option<[f32; 2]>, Unsupported> {
        let [tx, ty, _] = site.position;
        let [rx, ry] = site.radius;
        let size = (((ry + rx) as i32) + 2).max(1) * 2;
        let half = size / 2;
        let (cx, cy) = (tx as i32, ty as i32);
        let (fx, fy) = ((cx - half) as f32, (cy - half) as f32);
        self.check_known([fx, fy, fx + size as f32, fy + size as f32])?;
        let (ox, oy) = (half - cx, half - cy);
        let (x0, y0) = ((tx - rx) as i32, (ty - ry) as i32);
        let (x1, y1) = (((rx + tx) - 0.001) as i32, ((ry + ty) - 0.001) as i32);
        let mut list: Vec<(i32, i32)> = (x0..=x1)
            .flat_map(|x| (y0..=y1).map(move |y| (x, y)))
            .collect();
        let mut visited = vec![false; (size * size) as usize];
        let reach = half - 1;
        let mut i = 0;
        while i < list.len() {
            let (x, y) = list[i];
            if (cx - x).abs() < reach && (cy - y).abs() < reach {
                for (nx, ny) in [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)] {
                    let at = ((ox + nx) * size + ny + oy) as usize;
                    if !visited[at] && self.passable(nx, ny, mask)? {
                        visited[at] = true;
                        list.push((nx, ny));
                    }
                }
            }
            i += 1;
        }
        let mut best = f32::MAX;
        let mut spot = None;
        let mut empty = false;
        for &(x, y) in &list {
            if x0 <= x && x <= x1 && y0 <= y && y <= y1 {
                continue;
            }
            let was_empty = empty;
            let count = self.tiles.objects(x, y);
            if empty && count.is_some_and(|n| n != 0) {
                continue;
            }
            if count == Some(0) {
                empty = true;
            }
            let centre = [x as f32 + 0.5, from[2], y as f32 + 0.5];
            let d = [
                from[0] - centre[0],
                from[2] - centre[1],
                from[1] - centre[2],
            ];
            let distance = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
            if (empty && !was_empty) || distance < best {
                spot = Some([centre[0], centre[2]]);
                best = distance;
            }
        }
        Ok(spot)
    }

    /// The objects whose centre is on `site`'s footprint, edges included, and that the game counts
    /// as occupying it (`FUN_14123bb20` for a site whose master obstructs): objects with all three
    /// radii, other than the site, unless the site may be built over them (`can_be_built_on` 1).
    pub fn footprint_occupants(
        &self,
        site_id: i32,
        site: &Placed,
    ) -> Result<Vec<i32>, Unsupported> {
        let [sx, sy, _] = site.position;
        let [rx, ry] = site.radius;
        self.check_known([sx - rx, sy - ry, sx + rx, sy + ry])?;
        Ok(self
            .objects
            .iter()
            .filter(|&(&id, o)| {
                id != site_id
                    && o.buildable_on != 1
                    && o.tall
                    && o.radius.iter().all(|&r| 0.0 < r)
                    && (o.position[0] - sx).abs() <= rx
                    && (o.position[1] - sy).abs() <= ry
            })
            .map(|(&id, _)| id)
            .collect())
    }

    /// The objects a foundation clears off its footprint when its first builder arrives
    /// (`FUN_1415c3620`): those whose footprint overlaps the site's, of another master, that may
    /// be built over (`can_be_built_on`) or are trees (class 15, which it clears when their hit
    /// points are at most 1).
    pub fn footprint_clearing(&self, site_id: i32, site: &Placed) -> Result<Vec<i32>, Unsupported> {
        let [sx, sy, _] = site.position;
        let [rx, ry] = site.radius;
        self.check_known([sx - rx, sy - ry, sx + rx, sy + ry])?;
        Ok(self
            .objects
            .iter()
            .filter(|&(&id, o)| {
                id != site_id
                    && o.master != site.master
                    && (o.buildable_on != 0 || o.class == 15)
                    && (o.position[0] - sx).abs() < o.radius[0] + rx
                    && (o.position[1] - sy).abs() < o.radius[1] + ry
            })
            .map(|(&id, _)| id)
            .collect())
    }

    /// Whether a static record of another player marks a tile of `site`'s footprint
    /// (`FUN_1411d4b80`).
    pub fn foreign_marks(&self, site: &Placed) -> bool {
        let [sx, sy, _] = site.position;
        let [rx, ry] = site.radius;
        let (w, h) = (self.tiles.width, self.tiles.height);
        let (x0, y0) = (((sx - rx) as i32).max(0), ((sy - ry) as i32).max(0));
        let x1 = (((sx + rx) - 0.001) as i32).min(w - 1);
        let y1 = (((sy + ry) - 0.001) as i32).min(h - 1);
        let own = 1u64 << site.player;
        (y0..=y1).any(|y| {
            (x0..=x1).any(|x| {
                let players = self.tiles.word(x, y).unwrap_or(0) >> 23;
                players != 0 && players != own
            })
        })
    }

    /// The records a move query could report (`FUN_14114cc60` -> `FUN_141051900`): those of a
    /// tested kind whose box, grown by `grow`, meets `rect` (`x0, y0, x1, y1`, edges included),
    /// except the `ignore`d objects'. The game then tests each candidate against the move's
    /// segment and shrinks moving allies' boxes (master `+0x780`, at most 1); every hit is a
    /// candidate, so no candidate means no hit.
    pub fn obstruction_candidates(&self, rect: [f32; 4], grow: f32, ignore: &[i32]) -> Vec<i32> {
        self.objects
            .iter()
            .filter(|(id, p)| {
                p.obstruction
                    .is_some_and(|o| TESTED_KINDS.contains(&o.kind))
                    && !ignore.contains(id)
            })
            .filter(|(_, p)| {
                let [x0, y0, x1, y1] = p.bounds();
                let (x0, y0, x1, y1) = (x0 - grow, y0 - grow, grow + x1, grow + y1);
                rect[0] <= x1 && x0 <= rect[2] && rect[1] <= y1 && y0 <= rect[3]
            })
            .map(|(&id, _)| id)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn villager(x: f32, y: f32) -> Placed {
        Placed {
            position: [x, y, 0.0],
            radius: [0.2, 0.2],
            class: 4,
            player: 2,
            master: 0,
            obstruction_type: 0,
            buildable_on: 0,
            tall: true,
            speed: 0.0,
            unknown: None,
            obstruction: Some(Obstruction { kind: 5, class: 2 }),
        }
    }

    #[test]
    fn a_units_box_is_its_position_plus_and_minus_its_radii() {
        // MarvinBlue/426048731 tick 680: 9177's record box in the game's obstruction manager
        assert_eq!(villager(28.0, 89.0).bounds(), [27.8, 88.8, 28.2, 89.2]);
    }

    #[test]
    fn a_query_finds_the_boxes_its_rectangle_meets_but_not_the_ignored_or_unobstructing() {
        let mut s = Spatial::new([120, 120]);
        s.insert(9177, villager(28.0, 89.0));
        s.insert(9175, villager(30.0, 89.0));
        let mut foundation = villager(30.0, 88.0);
        foundation.radius = [1.0, 1.0];
        foundation.obstruction = None;
        foundation.class = 3;
        s.insert(10028, foundation);
        // 9177's first step: from (28, 89) toward (28.0143, 88.9928), grown by its radius
        let step = [28.0, 88.99284, 28.01431, 89.0];
        assert_eq!(
            s.obstruction_candidates(step, 0.2, &[9177]),
            Vec::<i32>::new()
        );
        assert_eq!(s.obstruction_candidates(step, 0.2, &[]), vec![9177]);
        // 9175's box grown by 0.2 starts at x 29.6
        assert_eq!(
            s.obstruction_candidates([29.0, 89.0, 29.59, 89.0], 0.2, &[9177]),
            Vec::<i32>::new()
        );
        assert_eq!(
            s.obstruction_candidates([29.0, 89.0, 29.61, 89.0], 0.2, &[9177]),
            vec![9175]
        );
    }

    /// MarvinBlue/426048731's Town Centre 9157 of player 2 (class 3) and a tree of Gaia (class 1),
    /// with the words the game's obstruction manager holds for their tiles.
    #[test]
    fn a_static_record_marks_its_tiles_with_its_class_and_player() {
        let mut s = Spatial::with_tiles([4, 4], Tiles::new(4, 4, vec![4; 16]));
        let centre = Placed {
            position: [2.0, 2.0, 0.0],
            radius: [1.0, 1.0],
            class: 3,
            player: 2,
            master: 0,
            obstruction_type: 0,
            buildable_on: 0,
            tall: true,
            speed: 0.0,
            unknown: None,
            obstruction: Some(Obstruction { kind: 2, class: 3 }),
        };
        s.insert(9157, centre);
        let tree = Placed {
            position: [0.5, 0.5, 0.0],
            radius: [0.5, 0.5],
            class: 15,
            player: 0,
            master: 0,
            obstruction_type: 0,
            buildable_on: 0,
            tall: true,
            speed: 0.0,
            unknown: None,
            obstruction: Some(Obstruction { kind: 2, class: 1 }),
        };
        s.insert(48, tree);
        let t = s.tiles();
        assert_eq!(t.word(1, 1), Some(0x202_4304));
        assert_eq!(t.word(2, 2), Some(0x202_4304));
        assert_eq!(t.word(3, 3), Some(4));
        assert_eq!(t.word(0, 0), Some(0x80_4104));
        assert_eq!((t.objects(2, 2), t.tall(2, 2)), (Some(1), Some(1)));
        s.remove(9157);
        assert_eq!(s.tiles().word(1, 1), Some(4));
        assert_eq!(s.tiles().objects(2, 2), Some(0));
    }

    #[test]
    fn a_unit_counts_on_the_tile_its_centre_is_on_and_marks_nothing() {
        let mut s = Spatial::with_tiles([4, 4], Tiles::new(4, 4, vec![4; 16]));
        s.insert(9177, villager(1.9, 1.0));
        assert_eq!(s.tiles().objects(1, 1), Some(1));
        assert_eq!(s.tiles().word(1, 1), Some(4));
        s.set_position(9177, [2.05, 1.0, 0.0]);
        assert_eq!(
            (s.tiles().objects(1, 1), s.tiles().objects(2, 1)),
            (Some(0), Some(1))
        );
        assert_eq!(s.tiles().tall(2, 1), Some(1));
    }

    /// The house foundation 10028 at (30, 88) of MarvinBlue/426048731 with its builder 9175 on its
    /// footprint at (30, 89), on open land (class 4) and a villager's mask (restriction 7: 6).
    fn site() -> (Spatial, Placed) {
        let mut s = Spatial::with_tiles([40, 120], Tiles::new(40, 120, vec![4; 40 * 120]));
        s.insert(9175, villager(30.0, 89.0));
        let mut house = villager(30.0, 88.0);
        house.radius = [1.0, 1.0];
        house.class = 3;
        house.master = 70;
        house.obstruction_type = 2;
        house.obstruction = None;
        s.insert(10028, house.clone());
        (s, house)
    }

    #[test]
    fn a_tile_is_passable_when_its_class_is_in_the_mask_and_nothing_blocks_it() {
        let (mut s, _) = site();
        assert_eq!(s.passable(29, 89, 6), Ok(true));
        assert_eq!(s.passable(29, 89, 1), Ok(false));
        assert_eq!(s.passable(-1, 89, 6), Ok(false));
        let mut tree = villager(29.5, 90.5);
        tree.radius = [0.5, 0.5];
        tree.obstruction = Some(Obstruction { kind: 2, class: 1 });
        s.insert(48, tree);
        assert_eq!(s.passable(29, 90, 6), Ok(false));
    }

    #[test]
    fn the_free_spot_beside_a_footprint_is_the_closest_empty_tile_around_it() {
        // the game sends 9175 to (29.5, 89.5): (30, 89) is as close but holds 9175 itself
        let (s, house) = site();
        assert_eq!(
            s.spot_beside([30.0, 89.0, 0.0], 6, &house),
            Ok(Some([29.5, 89.5]))
        );
    }

    #[test]
    fn the_footprint_s_occupants_are_the_objects_whose_centre_is_on_it() {
        let (mut s, house) = site();
        assert_eq!(s.footprint_occupants(10028, &house), Ok(vec![9175]));
        s.insert(9177, villager(28.0, 89.0));
        assert_eq!(s.footprint_occupants(10028, &house), Ok(vec![9175]));
        assert_eq!(s.footprint_clearing(10028, &house), Ok(vec![]));
        let mut flower = villager(30.5, 87.5);
        flower.buildable_on = 1;
        s.insert(77, flower);
        assert_eq!(s.footprint_clearing(10028, &house), Ok(vec![77]));
        assert!(!s.foreign_marks(&house));
    }

    #[test]
    fn a_frozen_object_may_be_anywhere_its_speed_could_take_it() {
        let (mut s, house) = site();
        let mut deer = villager(34.0, 88.0);
        deer.speed = 1.0;
        s.insert(9656, deer);
        s.freeze(9656);
        // frozen at once, the deer is where the save has it
        assert!(s.footprint_occupants(10028, &house).is_ok());
        assert!(s.get_known(9656).is_ok());
        // its box reaches x 33.8; 3 s later it may stand on the footprint, which ends at 31
        s.grow_unknown(2.5);
        assert!(s.footprint_occupants(10028, &house).is_ok());
        s.grow_unknown(0.5);
        assert!(s.footprint_occupants(10028, &house).is_err());
        assert!(s.get_known(9656).is_err());
    }
}
