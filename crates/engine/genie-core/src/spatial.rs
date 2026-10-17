//! Where the world's objects are, as movement reads them: each live object's position, radii and
//! class, and the obstruction record the game keeps for it in its obstruction manager
//! (`DAT_143f0a088`, queried by `FUN_141051900`).
//!
//! The game files its records in a five-level grid; the slice keeps them by object id. The grid
//! only orders a query's hits, and the slice stops a walker at its first hit
//! ([`Spatial::walk_hits`]), so it needs no order yet.
//!
//! [`Tiles`] holds what the game keeps per tile: the obstruction manager's word (terrain class,
//! static records' marks) and the counts of records and objects whose centre is on the tile.

use crate::registry::relation_kind;
use crate::unit_ai::{Search, Unsupported};
use crate::visibility::Visibility;
use std::collections::BTreeMap;

/// Obstruction kinds the query tests (`FUN_141051e00`): boxes (2, 3, 11, 13) and circles (5).
const TESTED_KINDS: [u8; 5] = [2, 3, 5, 11, 13];

/// What the walk reads of a moving object it meets: whether it moves (its velocity, `+0x868`, is
/// not 0: slot 0x5c8, `FUN_1411583d0`), its last move time (`+0x870`, slot 0x5e8) and its current
/// path level's pause time (`+0x858`'s `+0x34`, slot 0x528), none without a level.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Gait {
    pub moving: bool,
    pub last_move: u32,
    pub pause: Option<f32>,
    /// `+0x650`, `mLastHumanOrderTime`: a met unit's rank against a human walker.
    pub human_order: u32,
}

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

    /// The covered tiles as a rectangle, as the manager hands it to the planners' callback.
    fn covered_rect(&self, p: &Placed) -> [i16; 4] {
        let (xs, ys) = self.covered(p);
        [*xs.start(), *ys.start(), *xs.end(), *ys.end()].map(|v| v as i16)
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
    /// The tile rectangles (x0, y0, x1, y1) whose static marks changed, for the planners
    /// ([`Spatial::take_changes`]).
    changed: Vec<[i16; 4]>,
    /// The seconds since each frozen object froze.
    frozen_for: BTreeMap<i32, f32>,
    /// What a stopped player's units may have done since: the fastest any of the players' unit
    /// masters (type 70) moves and the farthest any acts beyond its footprint (its weapon or work
    /// range), for [`Spatial::check_known`]. 0 when not set.
    pub fastest: f32,
    pub reach: f32,
    /// What the walk reads of the moving objects it meets ([`Gait`]).
    gaits: BTreeMap<i32, Gait>,
    /// Each annex's main building (`+0xbb8`), which the walk's avoidance takes for the annex.
    annexes: BTreeMap<i32, i32>,
    /// `relation[i]` of each player (`player+0x260`), for the walk's query's allies.
    pub relations: Vec<Vec<u8>>,
    /// What the players see, when the slice holds it: the objects' sight follows their tiles
    /// ([`Spatial::set_position`]).
    pub sight: Option<Visibility>,
    /// The map's terrain, when the slice holds it.
    pub terrain: Option<crate::terrain::Terrain>,
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
            changed: Vec::new(),
            frozen_for: BTreeMap::new(),
            fastest: 0.0,
            reach: 0.0,
            gaits: BTreeMap::new(),
            annexes: BTreeMap::new(),
            relations: Vec::new(),
            sight: None,
            terrain: None,
        }
    }

    pub fn tiles(&self) -> &Tiles {
        &self.tiles
    }

    /// An object gets its obstruction record (`FUN_141056460` for a started foundation): a static
    /// one marks its tiles.
    pub fn add_record(&mut self, id: i32, o: Obstruction) {
        let Some(p) = self.objects.get_mut(&id) else {
            return;
        };
        p.obstruction = Some(o);
        if o.marks_tiles() {
            self.tiles.mark(p, o);
            self.changed.push(self.tiles.covered_rect(p));
        }
    }

    /// Whether an object on tile (`x`, `y`) (its centre there) satisfies `pred`.
    pub fn object_on_tile(&self, x: i32, y: i32, pred: impl Fn(&Placed) -> bool) -> bool {
        self.objects
            .values()
            .any(|o| o.position[0] as i32 == x && o.position[1] as i32 == y && pred(o))
    }

    /// Whether another foundation without its record (a master that obstructs, no record yet)
    /// overlaps `site`'s box (`FUN_1411cd0d0`'s test, without its players' visibility).
    pub fn foundations_overlapping(&self, site_id: i32, site: &Placed) -> bool {
        self.objects.iter().any(|(&id, o)| {
            id != site_id
                && o.obstruction.is_none()
                && o.obstruction_type != 0
                && (o.position[0] - site.position[0]).abs() < o.radius[0] + site.radius[0]
                && (o.position[1] - site.position[1]).abs() < o.radius[1] + site.radius[1]
        })
    }

    /// The tile rectangles whose static marks changed since the last call.
    pub fn take_changes(&mut self) -> Vec<[i16; 4]> {
        std::mem::take(&mut self.changed)
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
            self.frozen_for.entry(id).or_insert(0.0);
        }
    }

    /// The object's place is the game's again (the oracle's forced replay): it is known.
    pub fn unfreeze(&mut self, id: i32) {
        if let Some(p) = self.objects.get_mut(&id) {
            p.unknown = None;
        }
        self.frozen_for.remove(&id);
    }

    /// A tick of `dt` seconds passed: every frozen object may have moved its speed times dt.
    pub fn grow_unknown(&mut self, dt: f32) {
        for p in self.objects.values_mut() {
            if let Some(u) = p.unknown.as_mut() {
                *u += p.speed * dt;
            }
        }
        for t in self.frozen_for.values_mut() {
            *t += dt;
        }
    }

    /// Fails when a frozen object may be within `rect` (`x0, y0, x1, y1`) or may have changed
    /// there: the answer of a query over it depends on where that object really is. A frozen
    /// object that does not move stays where it was; it can only have changed when a stopped
    /// player's unit (Gaia's animals do not work on objects) may have reached it: a frozen unit
    /// within what it may have moved, or a unit a stopped player's building may have made since,
    /// at the fastest speed of the players' units, each plus the farthest reach.
    pub fn check_known(&self, rect: [f32; 4]) -> Result<(), Unsupported> {
        self.check_known_of(rect, false)
    }

    /// [`Spatial::check_known`] for a query over obstruction records and the mesh (the motion
    /// step's and the short pather's): a frozen object without a record (a bird) has no say.
    pub fn check_known_records(&self, rect: [f32; 4]) -> Result<(), Unsupported> {
        self.check_known_of(rect, true)
    }

    fn check_known_of(&self, rect: [f32; 4], records: bool) -> Result<(), Unsupported> {
        let near = self.objects.iter().any(|(&id, p)| {
            if records && p.obstruction.is_none() {
                return false;
            }
            p.unknown.is_some_and(|u| {
                let [x0, y0, x1, y1] = p.bounds();
                let inside = rect[0] <= x1 + u
                    && x0 - u <= rect[2]
                    && rect[1] <= y1 + u
                    && y0 - u <= rect[3];
                inside && (0.0 < u || self.touchable(id, [x0, y0, x1, y1]))
            })
        });
        if near {
            return Err(Unsupported("an object of a stopped player may be near"));
        }
        Ok(())
    }

    /// Whether a stopped player's unit may have reached the frozen object `id` within `rect`.
    fn touchable(&self, id: i32, rect: [f32; 4]) -> bool {
        self.objects.iter().any(|(&other, q)| {
            let Some(u) = q.unknown.filter(|_| other != id) else {
                return false;
            };
            if q.player == 0 {
                // Gaia's units (animals) do not work on objects
                return false;
            }
            let moved = if 0.0 < q.speed {
                u
            } else {
                self.fastest * self.frozen_for.get(&other).copied().unwrap_or(0.0)
            };
            let grow = moved + self.reach;
            let [x0, y0, x1, y1] = q.bounds();
            rect[0] <= x1 + grow
                && x0 - grow <= rect[2]
                && rect[1] <= y1 + grow
                && y0 - grow <= rect[3]
        })
    }

    /// A new object, in the order the game adds them: a static record marks only the tiles no
    /// earlier record marked.
    pub fn insert(&mut self, id: i32, placed: Placed) {
        self.remove(id);
        self.tiles.count(placed.position, placed.tall, true);
        if let Some(o) = placed.obstruction.filter(|o| o.marks_tiles()) {
            self.tiles.mark(&placed, o);
            self.changed.push(self.tiles.covered_rect(&placed));
        }
        self.objects.insert(id, placed);
    }

    /// The object leaves the world: its place, its sight's state and its records in the registry
    /// (its destructor, `FUN_141073060`) go.
    pub fn leave(&mut self, id: i32) -> Result<(), Unsupported> {
        if let (Some(v), Some(p)) = (self.sight.as_mut(), self.objects.get(&id)) {
            v.registry_leave(id, p.player, p.position, p.radius, p.class)?;
            v.remove_seer(id);
        }
        self.remove(id);
        Ok(())
    }

    pub fn remove(&mut self, id: i32) {
        if let Some(gone) = self.objects.remove(&id) {
            self.tiles.count(gone.position, gone.tall, false);
            if gone.obstruction.is_some_and(|o| o.marks_tiles()) {
                self.tiles.unmark(&gone);
                self.changed.push(self.tiles.covered_rect(&gone));
            }
        }
    }

    /// Whether the straight walk from `a` to `b` (x, y) of a unit `grow` wide meets an obstruction,
    /// as the path smoothing asks (`FUN_141052b90`, then `FUN_141052350` with flags `0x100`): the
    /// tiles of the walk's box whose terrain class the unit's `mask` lacks (the temporary tile
    /// records of `FUN_1410510c0`) and the static records (kinds 2, 11 and 13) of objects not in
    /// `ignore`, each box grown by `grow`, against the walk's segment (`FUN_141052e10`). A walk
    /// whose grown box leaves the map counts as blocked.
    pub fn segment_blocked(
        &self,
        a: [f32; 2],
        b: [f32; 2],
        grow: f32,
        mask: u8,
        ignore: &[i32],
    ) -> bool {
        let (lo_x, hi_x) = (
            if b[0] <= a[0] { b[0] } else { a[0] },
            if a[0] <= b[0] { b[0] } else { a[0] },
        );
        let (lo_y, hi_y) = (
            if b[1] <= a[1] { b[1] } else { a[1] },
            if a[1] <= b[1] { b[1] } else { a[1] },
        );
        let [w, h] = self.map;
        // FUN_141052b90: the tiles whose terrain the unit cannot cross become boxes
        let tx0 = ((lo_x - grow) as i32).max(0);
        let ty0 = ((lo_y - grow) as i32).max(0);
        let tx1 = if ((hi_x + grow) as i32) < w {
            (hi_x + grow) as i32
        } else {
            w - 1
        };
        let ty1 = if ((hi_y + grow) as i32) < h {
            (hi_y + grow) as i32
        } else {
            h - 1
        };
        if lo_x - grow < 0.0
            || lo_y - grow < 0.0
            || (w as f32) < grow + hi_x
            || (h as f32) < grow + hi_y
        {
            return true;
        }
        let walk = [a[0], a[1], b[0], b[1]];
        let meets = |bx: [f32; 4]| {
            let grown = [bx[0] - grow, bx[1] - grow, grow + bx[2], grow + bx[3]];
            lo_x <= grown[2]
                && grown[0] <= hi_x
                && lo_y <= grown[3]
                && grown[1] <= hi_y
                && box_meets_segment(bx, grow, walk)
        };
        let impassable = u64::from(mask & 0x7f | 0x80);
        for y in ty0..=ty1 {
            for x in tx0..=tx1 {
                let word = self.tiles.word(x, y).expect("a tile on the map");
                if word & impassable == 0
                    && meets([x as f32, y as f32, x as f32 + 1.0, y as f32 + 1.0])
                {
                    return true;
                }
            }
        }
        self.objects.iter().any(|(id, p)| {
            // kind 10 is filed as 2
            let kind = p.obstruction.map(|o| if o.kind == 10 { 2 } else { o.kind });
            matches!(kind, Some(2 | 11 | 13)) && !ignore.contains(id) && meets(p.bounds())
        })
    }

    /// Each obstruction record: the object's id, the record's kind and box.
    pub fn records(&self) -> impl Iterator<Item = (i32, u8, [f32; 4])> + '_ {
        self.objects
            .iter()
            .filter_map(|(&id, p)| Some((id, p.obstruction?.kind, p.bounds())))
    }

    /// The object moved: its record moves with it, and it changes tiles when its centre crosses
    /// a tile's edge (`RGE_Static_Object::new_position`, `0x1411a58b0`).
    /// What the walk reads of a moving object now, None for one without movement state.
    pub fn set_gait(&mut self, id: i32, gait: Option<Gait>) {
        match gait {
            Some(g) => {
                self.gaits.insert(id, g);
            }
            None => {
                self.gaits.remove(&id);
            }
        }
    }

    pub fn gait(&self, id: i32) -> Option<Gait> {
        self.gaits.get(&id).copied()
    }

    /// An annex's main building, or none.
    pub fn set_annex(&mut self, id: i32, main: Option<i32>) {
        match main {
            Some(m) => {
                self.annexes.insert(id, m);
            }
            None => {
                self.annexes.remove(&id);
            }
        }
    }

    pub fn annex_of(&self, id: i32) -> Option<i32> {
        self.annexes.get(&id).copied()
    }

    /// `FUN_141663dc0` with the ally flag ([`Spatial::allied`]), for the walk's avoidance.
    pub fn allied_with(&self, from: usize, to: usize) -> bool {
        self.allied(from, to)
    }

    /// `FUN_141663dc0` with the ally flag: `to` counts for `from` when it is `from`, or `from`
    /// stands to it in relation 0 or 4; Gaia counts for no one else.
    fn allied(&self, from: usize, to: usize) -> bool {
        if from == to {
            return true;
        }
        if to == 0 {
            return false;
        }
        matches!(
            self.relations.get(from).and_then(|r| r.get(to)),
            Some(0 | 4)
        )
    }

    /// The walk's obstruction query (`FUN_14114cc60` -> `FUN_141051900` with flags `0x90`, each
    /// cell's records by `FUN_141051e00`): the records of a tested kind (kind 10 filed as 2),
    /// except the `ignore`d objects', whose box, grown by `grow` (the walker's radius), meets the
    /// step's rectangle `rect` and whose grown box meets the step `from` -> `to`
    /// (`FUN_1410512f0`; a circle's, kind 5, `FUN_141051630`, alike). A moving object of
    /// `player` or an ally grows by `grow × ally_factor` instead (`+0x84`, the walker's master's
    /// `min_collision_size_multiplier`). The ids hit, in id order; the game's are in its grid's,
    /// which no caller reads yet. A rectangle that, grown, leaves the map adds the map edge's
    /// records (`FUN_141057290`), which is not modelled.
    #[allow(clippy::too_many_arguments)]
    pub fn walk_hits(
        &self,
        from: [f32; 2],
        to: [f32; 2],
        rect: [f32; 4],
        grow: f32,
        ally_factor: f32,
        player: Option<usize>,
        ignore: &[i32],
    ) -> Result<Vec<i32>, Unsupported> {
        let [w, h] = self.map;
        if rect[0] - grow < 0.0
            || rect[1] - grow < 0.0
            || (w as f32) < grow + rect[2]
            || (h as f32) < grow + rect[3]
        {
            return Err(Unsupported(
                "a walk query beyond the map's edge (FUN_141057290)",
            ));
        }
        let walk = [from[0], from[1], to[0], to[1]];
        let mut hits = Vec::new();
        for (&id, p) in &self.objects {
            let Some(o) = p.obstruction else {
                continue;
            };
            let kind = if o.kind == 10 { 2 } else { o.kind };
            if !TESTED_KINDS.contains(&kind) || ignore.contains(&id) {
                continue;
            }
            let g = if self.gaits.get(&id).is_some_and(|g| g.moving) {
                let player = player.ok_or(Unsupported("a walker without a place"))?;
                if self.allied(p.player, player) {
                    ally_factor * grow
                } else {
                    grow
                }
            } else {
                grow
            };
            let [x0, y0, x1, y1] = p.bounds();
            let grown = [x0 - g, y0 - g, g + x1, g + y1];
            if rect[0] <= grown[2]
                && grown[0] <= rect[2]
                && rect[1] <= grown[3]
                && grown[1] <= rect[3]
                && box_meets_segment([x0, y0, x1, y1], g, walk)
            {
                hits.push(id);
            }
        }
        Ok(hits)
    }

    /// The object's new place (`new_position`, `0x1411a58b0`): its tile's counts and, on a new
    /// tile, its sight follow.
    pub fn set_position(&mut self, id: i32, position: [f32; 3]) {
        if let Some(p) = self.objects.get_mut(&id) {
            self.tiles.count(p.position, p.tall, false);
            let from = p.position;
            p.position = position;
            self.tiles.count(p.position, p.tall, true);
            if let Some(v) = self.sight.as_mut() {
                v.moved(id, from, position, p.radius);
                v.registry_moved(id, p.class, from, position);
            }
        }
    }

    /// Fails when a frozen object of `player` that may have moved sees, or saw, near `rect`: the
    /// player's visible map there, and the registry's records the objects' masks keep, may differ
    /// from the game's (its sight, plus a tile, beyond what it may have moved).
    pub fn check_sight(&self, player: usize, rect: [f32; 4]) -> Result<(), Unsupported> {
        let Some(v) = self.sight.as_ref() else {
            return Ok(());
        };
        let near = self.objects.iter().any(|(&id, p)| {
            let Some(u) = p.unknown.filter(|&u| 0.0 < u && p.player == player) else {
                return false;
            };
            let los = v.seer(id).map_or(0, |s| s.los as i32);
            if los <= 0 {
                return false;
            }
            let grow = u + los as f32 + 1.0;
            let [x0, y0, x1, y1] = p.bounds();
            rect[0] <= x1 + grow
                && x0 - grow <= rect[2]
                && rect[1] <= y1 + grow
                && y0 - grow <= rect[3]
        });
        if near {
            return Err(Unsupported("a stopped object's sight may reach the search"));
        }
        Ok(())
    }

    /// `ClosestObject` (`FUN_140c488b0`) for object `me` of `player` as a hunted animal calls it
    /// ([`Search`]): the query over the player's registry ([`Registry::visible_units`]), then each
    /// record by relation and order: itself, an object that is gone and one dying (state 7 and up)
    /// are passed; any other would need the reachability check (the unit's slot 0x3d0,
    /// `FUN_140fa0370`, the pathfinders), which is not modelled. Nothing found answers −1. A query
    /// near a frozen object, or that reads nothing (which leaves the game's last answer), fails.
    pub fn closest_object(
        &self,
        me: i32,
        player: usize,
        search: &Search<'_>,
    ) -> Result<i32, Unsupported> {
        let v = self
            .sight
            .as_ref()
            .ok_or(Unsupported("a search without the players' sight"))?;
        let r = v.registry.as_ref().ok_or(Unsupported(
            "a search without the objects-in-sight registry",
        ))?;
        let own = self
            .relations
            .get(player)
            .ok_or(Unsupported("a search of a player without relations"))?;
        let relations = (0..r.players.min(own.len()))
            .map(|o| relation_kind(player, o, own[o]))
            .collect::<Result<Vec<u32>, _>>()?;
        let radius = search.radius.min(1000.0);
        let [x, y] = search.at;
        let [rx, ry] = search.size;
        let rect = [
            (x - (radius + rx)) - 0.001,
            (y - (radius + ry)) - 0.001,
            x + (radius + rx) + 0.001,
            y + (radius + ry) + 0.001,
        ];
        self.check_known(rect)?;
        self.check_sight(player, rect)?;
        let found = r
            .visible_units(
                player,
                search.at,
                search.radius,
                0,
                search.classes,
                &relations,
                search.size,
                0xff,
            )?
            .ok_or(Unsupported(
                "a search that reads nothing (the last answer stands)",
            ))?;
        for (kind, records) in found.iter().enumerate() {
            // the relation filter (`param_3`): 0 friends and itself, 1 neutrals, 2 enemies, 3
            // itself, else any
            let related = match search.relation {
                0 => matches!(kind, 1 | 2),
                1 => kind == 3,
                2 => kind == 4,
                3 => kind == 1,
                _ => true,
            };
            for record in records {
                if record.id == me {
                    continue;
                }
                let (Some(seer), Some(p)) = (v.seer(record.id), self.objects.get(&record.id))
                else {
                    continue;
                };
                if 7 <= seer.state || !related {
                    continue;
                }
                if search.group != -1 && i32::from(p.class) != search.group {
                    continue;
                }
                if search.state != -1 && i32::from(seer.state) != search.state {
                    continue;
                }
                return Err(Unsupported(
                    "ClosestObject's reachability check (slot 0x3d0) of a found object",
                ));
            }
        }
        Ok(-1)
    }

    /// The Artifact action's owner search (`FUN_140aabc40`) for the animal `me`: the objects
    /// whose centre is on a tile within the animal's radius (whole) plus 3 tiles of its own tile,
    /// combat units (type 70) in state 2 of a player that is not out (`+0x89`, 0 for every player
    /// probed), Gaia's only with `+0x924` 0, of no class of 1, 60, 42, 11, 14, 30, 58, 3, 52, 27,
    /// 39, 49, 37, 9, 10 or 38, counted per player with the nearest footprint gap (squared). A
    /// unit of its own owner keeps it, unless that owner's attribute 97 (`attribute_97`, by
    /// player) is above 0 and it is livestock (class 58), which then takes it from nobody; a
    /// player whose attribute 97 is above 0 counts 100 more (the most units then win). One other
    /// player takes it; of several, the nearest (Gaia never). A frozen unit that counts and may
    /// have moved onto the tiles fails it.
    pub fn artifact_owner(
        &self,
        me: i32,
        attribute_97: &[f32],
    ) -> Result<Option<usize>, Unsupported> {
        const KEPT: [i16; 16] = [1, 60, 42, 11, 14, 30, 58, 3, 52, 27, 39, 49, 37, 9, 10, 38];
        // a world without the players' sight (the unit tests') holds nobody near
        let Some(v) = self.sight.as_ref() else {
            return Ok(None);
        };
        let p = self.objects.get(&me).ok_or(Unsupported(
            "an owner search of an object the world does not hold",
        ))?;
        let reach = (p.radius[0] as i32 as i16).wrapping_add(3);
        let [x, y, _] = p.position;
        let (x0, x1) = (
            (x as i32 as i16).wrapping_sub(reach),
            (x as i32 as i16).wrapping_add(reach),
        );
        let (y0, y1) = (
            (y as i32 as i16).wrapping_sub(reach),
            (y as i32 as i16).wrapping_add(reach),
        );
        let (x0, y0) = (i32::from(x0.max(0)), i32::from(y0.max(0)));
        let x1 = i32::from(x1).min(self.map[0] - 1);
        let y1 = i32::from(y1).min(self.map[1] - 1);
        // a frozen unit that could count (of the combat type and a class that counts) may have
        // moved onto the tiles
        let area = [x0 as f32, y0 as f32, (x1 + 1) as f32, (y1 + 1) as f32];
        let unsure = self.objects.iter().any(|(&id, q)| {
            q.unknown.is_some_and(|u| {
                let counts = v.seer(id).is_some_and(|s| s.kind == 70) && !KEPT.contains(&q.class);
                let [bx0, by0, bx1, by1] = q.bounds();
                counts
                    && 0.0 < u
                    && area[0] <= bx1 + u
                    && bx0 - u <= area[2]
                    && area[1] <= by1 + u
                    && by0 - u <= area[3]
            })
        });
        if unsure {
            return Err(Unsupported(
                "an owner search near a stopped unit that counts",
            ));
        }
        let players = self.relations.len().max(attribute_97.len());
        let mut counts = vec![0i32; players];
        let mut nearest = vec![999.0f32; players];
        let mut own_seen = false;
        let mut taker = false;
        let livestock = p.class == 58;
        for (&id, q) in &self.objects {
            let (tx, ty) = (q.position[0] as i32, q.position[1] as i32);
            if !(x0 <= tx && tx <= x1 && y0 <= ty && ty <= y1) {
                continue;
            }
            let Some(s) = v.seer(id) else { continue };
            if s.kind != 70 || s.state != 2 || KEPT.contains(&q.class) {
                continue;
            }
            if q.player == 0 && s.capture != 0 {
                continue;
            }
            let o = q.player;
            if players <= o {
                return Err(Unsupported(
                    "an owner search's unit of a player beyond the list",
                ));
            }
            let strong = livestock && attribute_97.get(o).is_some_and(|&a| 0.0 < a);
            if o == p.player {
                if strong {
                    return Ok(None);
                }
                own_seen = true;
                counts[o] += 1;
            } else {
                counts[o] += 1;
                if strong {
                    taker = true;
                    counts[o] += 100;
                }
            }
            let gap =
                crate::movement::footprint_gap_squared(q.position, q.radius, p.position, p.radius);
            if gap < nearest[o] {
                nearest[o] = gap;
            }
        }
        if own_seen && !taker {
            return Ok(None);
        }
        let takers: Vec<usize> = (0..players).filter(|&o| 0 < counts[o]).collect();
        match takers[..] {
            [] => Ok(None),
            [one] => Ok(Some(one)),
            _ if !taker => {
                let mut best = (999.0f32, None);
                for (o, &d) in nearest.iter().enumerate() {
                    if d < best.0 {
                        best = (d, Some(o));
                    }
                }
                Ok(best.1.filter(|&o| 0 < o))
            }
            _ => {
                let mut best = (0, None);
                for (o, &n) in counts.iter().enumerate() {
                    if best.0 < n {
                        best = (n, Some(o));
                    }
                }
                Ok(best.1)
            }
        }
    }

    /// An ownership change's world part (`FUN_14120f800`): the object's place and records take
    /// the new owner, and its sight moves to the new owner's map ([`Visibility::owner_changed`]).
    pub fn change_owner(&mut self, id: i32, new: usize, los: f32) -> Result<(), Unsupported> {
        let Some(p) = self.objects.get_mut(&id) else {
            return Err(Unsupported(
                "an ownership change of an object the world does not hold",
            ));
        };
        p.player = new;
        let (position, radius) = (p.position, p.radius);
        if let Some(v) = self.sight.as_mut() {
            v.owner_changed(id, position, radius, new, los)?;
        }
        Ok(())
    }

    /// Whether player `player` sees tile (`x`, `y`) (`FUN_141663500` returning 0xf): its bit of
    /// the unified map (its `+0x2a8` mask is its bit and bit 29, which no tile has; bit 61 neither).
    pub fn sees_tile(&self, player: usize, x: i32, y: i32) -> bool {
        let Some(v) = self.sight.as_ref() else {
            return false;
        };
        if x < 0 || y < 0 || v.width <= x || v.height <= y {
            return false;
        }
        v.unified[(y * v.width + x) as usize] & (1u64 << player) != 0
    }

    /// `RGE_Combat_Object::update`'s sight ([`Visibility::combat_sight`]); an annex reads its
    /// main building's place and UMV entry.
    pub fn combat_sight(&mut self, id: i32) -> Result<(), Unsupported> {
        let (Some(v), Some(p)) = (self.sight.as_mut(), self.objects.get(&id)) else {
            return Ok(());
        };
        let main = self.annexes.get(&id).and_then(|main| {
            let place = self.objects.get(main)?.position;
            Some((place, v.seer(*main).map_or(0, |s| s.umv)))
        });
        v.combat_sight(id, p.player, p.position, p.radius, p.class, main)
    }

    /// The object's set_state to `state` ([`Visibility::state_changed`]).
    pub fn sight_state(&mut self, id: i32, state: u8) {
        if let (Some(v), Some(p)) = (self.sight.as_mut(), self.objects.get(&id)) {
            v.state_changed(id, p.position, p.radius, state);
        }
    }

    /// The object's master changes to `master` with radii `radius`, class `class` and line of
    /// sight `los` (`FUN_14108c990`: its records leave the registry's lists of its old class,
    /// [`Visibility::master_changed`], and join those of the new one); the caller files the
    /// object's new radii and class.
    pub fn sight_master(
        &mut self,
        id: i32,
        radius: [f32; 2],
        class: i16,
        master: i16,
        los: f32,
    ) -> Result<(), Unsupported> {
        if let (Some(v), Some(p)) = (self.sight.as_mut(), self.objects.get(&id)) {
            v.master_changed(id, p.position, p.radius, radius, master, los);
            v.registry_master(
                id,
                p.player,
                p.position,
                (p.radius, p.class),
                (radius, class),
            )?;
        }
        Ok(())
    }

    /// The first part of the players' sight the slice met and does not model.
    pub fn take_sight_unsupported(&mut self) -> Option<Unsupported> {
        self.sight.as_mut().and_then(Visibility::take_unsupported)
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

/// `FUN_1410576a0`, Sedgewick's ccw: 1 when `p0`, `p1`, `p2` turn counter-clockwise (or lie in
/// a line with `p2` beyond `p1`), -1 clockwise (or `p0` between), 0 when `p2` lies on the segment.
fn ccw(p0: [f32; 2], p1: [f32; 2], p2: [f32; 2]) -> i32 {
    let (dx1, dy1) = (p1[0] - p0[0], p1[1] - p0[1]);
    let (dx2, dy2) = (p2[0] - p0[0], p2[1] - p0[1]);
    // the game's `<=` is false for a NaN too
    if matches!(
        (dy2 * dx1).partial_cmp(&(dx2 * dy1)),
        Some(std::cmp::Ordering::Greater) | None
    ) {
        return 1;
    }
    if dy2 * dx1 < dx2 * dy1 || dx2 * dx1 < 0.0 || dy2 * dy1 < 0.0 {
        return -1;
    }
    if dy2 * dy2 + dx2 * dx2 <= dy1 * dy1 + dx1 * dx1 {
        return 0;
    }
    1
}

/// `FUN_141054ec0`: whether segments `p1`–`p2` and `p3`–`p4` meet.
fn segments_meet(p1: [f32; 2], p2: [f32; 2], p3: [f32; 2], p4: [f32; 2]) -> bool {
    (ccw(p1, p2, p4) * ccw(p1, p2, p3)) as f32 <= 0.0
        && (ccw(p3, p4, p2) * ccw(p3, p4, p1)) as f32 <= 0.0
}

/// `FUN_141052e10` with the box grown by `r`: the walk `[x0, y0, x1, y1]` crosses one of the grown
/// box's edges or starts or ends inside it.
fn box_meets_segment([bx0, by0, bx1, by1]: [f32; 4], r: f32, walk: [f32; 4]) -> bool {
    let lo_x = if bx1 < bx0 { bx1 } else { bx0 };
    let hi_x = if bx0 < bx1 { bx1 } else { bx0 };
    let lo_y = if by1 < by0 { by1 } else { by0 };
    let hi_y = if by0 < by1 { by1 } else { by0 };
    let (start, end) = ([walk[0], walk[1]], [walk[2], walk[3]]);
    let edges = [
        ([bx0 - r, by0 - r], [r + bx1, by0 - r]),
        ([r + bx1, by0 - r], [r + bx1, r + by1]),
        ([r + bx1, r + by1], [bx0 - r, r + by1]),
        ([bx0 - r, r + by1], [bx0 - r, by0 - r]),
    ];
    if edges
        .iter()
        .any(|&(e0, e1)| segments_meet(start, end, e0, e1))
    {
        return true;
    }
    let outside =
        |p: [f32; 2]| p[0] < lo_x - r || r + hi_x < p[0] || p[1] < lo_y - r || r + hi_y < p[1];
    !(outside(start) && outside(end))
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

    #[test]
    fn a_walk_hits_the_boxes_its_step_crosses_and_a_moving_ally_s_box_shrinks() {
        let mut s = Spatial::new([120, 120]);
        s.relations = vec![vec![1, 4, 4], vec![1, 0, 4], vec![1, 4, 0]];
        s.insert(9175, villager(30.0, 89.0));
        let (from, to) = ([29.5, 88.0], [29.5, 88.3]);
        let rect = [29.5, 88.0, 29.5, 88.3];
        // the box (29.8..30.2, 88.8..89.2) grown by 0.25 reaches 88.55, past the step's end
        assert_eq!(
            s.walk_hits(from, to, rect, 0.25, 0.5, Some(2), &[]),
            Ok(vec![])
        );
        // grown by 0.4 it reaches x 29.4: the step crosses it once it gets to y 88.4
        let longer = ([29.5, 88.0], [29.5, 88.5]);
        let rect = [29.5, 88.0, 29.5, 88.5];
        assert_eq!(
            s.walk_hits(longer.0, longer.1, rect, 0.4, 1.0, Some(2), &[]),
            Ok(vec![9175])
        );
        assert_eq!(
            s.walk_hits(longer.0, longer.1, rect, 0.4, 1.0, Some(2), &[9175]),
            Ok(vec![])
        );
        // moving, of the walker's player, it grows by 0.4 × 0.5 only
        s.set_gait(
            9175,
            Some(Gait {
                moving: true,
                ..Gait::default()
            }),
        );
        assert_eq!(
            s.walk_hits(longer.0, longer.1, rect, 0.4, 0.5, Some(2), &[]),
            Ok(vec![])
        );
        // an enemy's moving box grows in full
        s.relations[2][1] = 3;
        assert_eq!(
            s.walk_hits(longer.0, longer.1, rect, 0.4, 0.5, Some(1), &[]),
            Ok(vec![9175])
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
    fn a_walk_meets_a_static_box_in_its_way_but_not_one_beside_it() {
        // tiles of class 1 everywhere: a unit with mask 1 crosses them all
        let mut s = Spatial::with_tiles([16, 16], Tiles::new(16, 16, vec![1; 256]));
        let mut house = villager(8.0, 8.0);
        house.radius = [1.0, 1.0];
        house.obstruction = Some(Obstruction { kind: 2, class: 3 });
        s.insert(7, house);
        assert!(s.segment_blocked([4.0, 8.0], [12.0, 8.0], 0.2, 1, &[]));
        assert!(!s.segment_blocked([4.0, 11.0], [12.0, 11.0], 0.2, 1, &[]));
        // within its radius of the box, and the box ignored
        assert!(s.segment_blocked([4.0, 9.1], [12.0, 9.1], 0.2, 1, &[]));
        assert!(!s.segment_blocked([4.0, 8.0], [12.0, 8.0], 0.2, 1, &[7]));
        // a moving unit's record never blocks the walk
        s.insert(8, villager(6.0, 3.0));
        assert!(!s.segment_blocked([4.0, 3.0], [12.0, 3.0], 0.2, 1, &[]));
    }

    #[test]
    fn a_walk_meets_a_tile_whose_terrain_the_unit_cannot_cross() {
        let mut classes = vec![1; 256];
        classes[5 * 16 + 9] = 2;
        let s = Spatial::with_tiles([16, 16], Tiles::new(16, 16, classes));
        assert!(s.segment_blocked([2.0, 5.5], [14.0, 5.5], 0.2, 1, &[]));
        assert!(!s.segment_blocked([2.0, 5.5], [14.0, 5.5], 0.2, 3, &[]));
        // a walk whose grown box leaves the map
        assert!(s.segment_blocked([0.1, 5.5], [14.0, 5.5], 0.2, 3, &[]));
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

    #[test]
    fn a_frozen_object_that_does_not_move_is_known_unless_a_stopped_unit_may_reach_it() {
        let mut s = Spatial::new([64, 64]);
        let tree = Placed {
            player: 0,
            speed: 0.0,
            ..villager(24.5, 97.5 - 60.0)
        };
        s.insert(9365, tree);
        s.freeze(9365);
        s.reach = 1.0;
        let near = [24.0, 37.0, 24.6, 38.0];
        assert!(s.check_known(near).is_ok());
        // a stopped villager 10 tiles off, 0.8 tiles a second
        let mut walker = villager(34.5, 37.5);
        walker.player = 1;
        walker.speed = 0.8;
        s.insert(9200, walker);
        s.freeze(9200);
        s.grow_unknown(10.0);
        assert!(s.check_known(near).is_ok());
        s.grow_unknown(2.0);
        assert!(s.check_known(near).is_err());
    }
}
