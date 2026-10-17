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
}
