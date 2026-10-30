//! The obstruction grid's cells and visits, the scout map, gaits, annexes, alliances and the line
//! queries.

use crate::GenieError;
use crate::map::Point;
use crate::map::spatial::{
    Gait, Placed, QUERY_HITS, Spatial, TESTED_KINDS, record_meets_segment, tile_record_id,
};
use crate::map::{Rect, Segment};
use crate::objects::{ObjectId, ObjectKey};
use crate::players::PlayerId;

/// A walk's obstruction query ([`Spatial::walk_hits`]'s arguments).
#[derive(Clone, Copy, Debug)]
pub struct WalkQuery<'a> {
    /// The step's start.
    pub start: Point,
    /// The step's end.
    pub end: Point,
    /// The step's rectangle.
    pub rect: Rect,
    /// The walker's radius, which grows the records' boxes and the rectangle.
    pub grow: f32,
    /// The walker's master's `min_collision_size_multiplier`, for a moving object of `player` or
    /// an ally.
    pub ally_factor: f32,
    /// The walker's player.
    pub player: Option<PlayerId>,
    /// The objects the query ignores.
    pub ignore: &'a [i32],
}

impl Spatial {
    /// Each obstruction record: the object's id, the record's kind and box.
    pub fn records(&self) -> impl Iterator<Item = (i32, u8, Rect)> + '_ {
        self.objects
            .iter()
            .filter_map(|(&id, p)| Some((id, p.obstruction?.kind, p.bounds())))
    }

    /// Each obstruction record's grid cell ([`Spatial::filed`]): the object's id, the level and
    /// the cell's x and y.
    pub fn record_cells(&self) -> impl Iterator<Item = (i32, (i32, i32, i32))> + '_ {
        self.objects
            .iter()
            .filter_map(|(&id, p)| Some((id, self.filed(p, p.obstruction?.kind))))
    }

    /// The grid cell an object's record of kind `kind` is filed in (`FUN_141056460`): level and
    /// cell x, y. Its box's tiles, held on the map; a box of kind 2 (10), 11 or 13 whose centre
    /// sits on a half tile ends 0.001 short on both axes. The level is the finest whose cell holds
    /// both corners' tiles, the cell the one of the low corner's tile.
    pub(super) fn filed(&self, p: &Placed, kind: u8) -> (i32, i32, i32) {
        let b = p.bounds();
        let (x0, y0, mut x1, mut y1) = (b.x0(), b.y0(), b.x1(), b.y1());
        let (x, y) = (p.position.x(), p.position.y());
        if matches!(kind, 2 | 10 | 11 | 13) && x % 0.5 < 0.001 && y % 0.5 < 0.001 {
            x1 -= 0.001;
            y1 -= 0.001;
        }
        let (w, h) = (self.map.width(), self.map.height());
        let tile = |v: f32, n: i32| (v as i32).clamp(0, n - 1);
        let (tx0, ty0, tx1, ty1) = (tile(x0, w), tile(y0, h), tile(x1, w), tile(y1, h));
        let level = self
            .grid
            .level((tx1 ^ tx0) >> 3)
            .min(self.grid.level((ty1 ^ ty0) >> 3));
        let shift = self.grid.shift(level);
        (level, tx0 >> shift, ty0 >> shift)
    }

    /// Whether a query whose rectangle, grown as the query grows it, is `rect` (`x0, y0, x1, y1`)
    /// visits the cell an object's record of kind `kind` is filed in ([`Spatial::filed`]): the
    /// query's tiles run from the low corner's (at least 0) to the high corner's (at most the
    /// map's size), and on each level it visits the cells they cover.
    pub(super) fn visits(&self, p: &Placed, kind: u8, rect: Rect) -> bool {
        let (level, cx, cy) = self.filed(p, kind);
        self.visits_cell(level, cx, cy, rect)
    }

    /// Whether a query whose grown rectangle is `rect` visits cell (`cx`, `cy`) of `level`
    /// ([`Spatial::visits`]).
    pub(super) fn visits_cell(&self, level: i32, cx: i32, cy: i32, rect: Rect) -> bool {
        let [x0, y0, x1, y1] = self.visited_cells(level, rect);
        x0 <= cx && cx <= x1 && y0 <= cy && cy <= y1
    }

    /// The cells of `level` a query whose grown rectangle is `rect` visits
    /// ([`Spatial::visits_cell`]): x and y from, then x and y to, each inclusive.
    pub(super) fn visited_cells(&self, level: i32, rect: Rect) -> [i32; 4] {
        let shift = self.grid.shift(level);
        let (w, h) = (self.map.width(), self.map.height());
        let (qx0, qy0) = ((rect.x0() as i32).max(0), (rect.y0() as i32).max(0));
        let (qx1, qy1) = ((rect.x1() as i32).min(w), (rect.y1() as i32).min(h));
        [qx0 >> shift, qy0 >> shift, qx1 >> shift, qy1 >> shift]
    }

    /// The object's record is filed again ([`RecordCells`]): in the cell [`Spatial::filed`] gives
    /// it now, in none without a record or out of the world. Every change of an object's place,
    /// radii or record runs it.
    ///
    /// [`RecordCells`]: crate::map::spatial::RecordCells
    pub(super) fn refile(&mut self, id: i32) {
        let cell = self
            .objects
            .get(&id)
            .and_then(|p| p.obstruction.map(|o| self.filed(p, o.kind)));
        let records = &mut self.records;
        let old = records.filed.get(&id).copied();
        if old == cell {
            return;
        }
        if let Some(old) = old
            && let Some(ids) = records.cells.get_mut(&old)
        {
            let _ = ids.remove(&id);
            if ids.is_empty() {
                let _ = records.cells.remove(&old);
            }
        }
        match cell {
            Some(c) => {
                let _ = records.filed.insert(id, c);
                let _ = records.cells.entry(c).or_default().insert(id);
            }
            None => {
                let _ = records.filed.remove(&id);
            }
        }
    }

    /// The objects whose record is filed in a cell a query whose grown rectangle is `rect`
    /// visits, in id order: those [`Spatial::visits`] passes, read from the cells it visits on
    /// each level ([`RecordCells`]) instead of tested one by one.
    ///
    /// [`RecordCells`]: crate::map::spatial::RecordCells
    pub(super) fn visited_records(&self, rect: Rect) -> Vec<i32> {
        let mut ids = Vec::new();
        for level in -1..self.grid.levels {
            let [x0, y0, x1, y1] = self.visited_cells(level, rect);
            let span = (i64::from(x1) - i64::from(x0) + 1) * (i64::from(y1) - i64::from(y0) + 1);
            if span <= 0 {
                continue;
            }
            if span as usize <= self.records.cells.len() {
                for cy in y0..=y1 {
                    for cx in x0..=x1 {
                        if let Some(c) = self.records.cells.get(&(level, cx, cy)) {
                            ids.extend(c);
                        }
                    }
                }
            } else {
                let level_cells = self.records.cells.range((level, x0, y0)..=(level, x1, y1));
                for (&(_, _, cy), c) in level_cells {
                    if y0 <= cy && cy <= y1 {
                        ids.extend(c);
                    }
                }
            }
        }
        ids.sort_unstable();
        ids
    }

    /// [`Spatial::visited_records`] with each object's place.
    pub(super) fn records_in(&self, rect: Rect) -> impl Iterator<Item = (i32, &Placed)> + '_ {
        self.visited_records(rect)
            .into_iter()
            .filter_map(|id| self.objects.get(&id).map(|p| (id, p)))
    }

    /// The object moved: its record moves with it, and it changes tiles when its centre crosses
    /// a tile's edge (`RGE_Static_Object::new_position`, `0x1411a58b0`).
    /// Player `player`'s scout map, once made (`player+0x1370`).
    #[must_use]
    pub fn scout_map(&self, player: PlayerId) -> Option<&crate::movement::explore::InfluenceMap> {
        self.scout_maps.get(&player)
    }

    /// Player `player`'s scout map becomes `map` (the old one goes).
    pub fn set_scout_map(&mut self, player: PlayerId, map: crate::movement::explore::InfluenceMap) {
        let _ = self.scout_maps.insert(player, map);
    }

    /// Tile (`x`, `y`) of player `player`'s scout map, if it has one, takes 1 (`FUN_140f4d5e0`).
    pub fn scout(&mut self, player: PlayerId, x: i32, y: i32) {
        if let Some(m) = self.scout_maps.get_mut(&player) {
            m.set(x, y, 1);
        }
    }

    /// What the walk reads of a moving object now, None for one without movement state.
    pub fn set_gait(&mut self, id: impl ObjectKey, gait: Option<Gait>) {
        let Some(id) = id.key().map(ObjectId::get) else {
            return;
        };
        match gait {
            Some(g) => {
                let _ = self.gaits.insert(id, g);
            }
            None => {
                let _ = self.gaits.remove(&id);
            }
        }
    }

    /// The walk's view of the moving object `id`'s last human order time (`+0x650`) as the field
    /// changes; nothing for an object without one.
    pub fn set_human_order(&mut self, id: impl ObjectKey, human_order: u32) {
        let Some(id) = id.key().map(ObjectId::get) else {
            return;
        };
        if let Some(g) = self.gaits.get_mut(&id) {
            g.human_order = human_order;
        }
    }

    /// The walk's view of the moving object `id`'s last move time (`+0x870`, slot 0x5e8) as
    /// the field changes outside its own update (a construction's start, a gate that shuts or
    /// locks): a later walker of the same update reads it at once; nothing for an object without
    /// movement state.
    pub fn set_last_move(&mut self, id: impl ObjectKey, last_move: u32) {
        let Some(id) = id.key().map(ObjectId::get) else {
            return;
        };
        if let Some(g) = self.gaits.get_mut(&id) {
            g.last_move = last_move;
        }
    }

    /// What the walk reads of the moving object `id`.
    #[must_use]
    pub fn gait(&self, id: i32) -> Option<Gait> {
        self.gaits.get(&id).copied()
    }

    /// An annex's main building, or none.
    pub fn set_annex(&mut self, id: i32, main: Option<i32>) {
        match main {
            Some(m) => {
                let _ = self.annexes.insert(id, m);
            }
            None => {
                let _ = self.annexes.remove(&id);
            }
        }
    }

    /// The obstruction query's ignore list for `ids` (−1 for none), as `FUN_14114e8a0` adds each
    /// object: its id, then a building's annexes (`+0xbc0`..`+0xbd8`, in id order).
    #[must_use]
    pub fn query_ignore(&self, ids: &[i32]) -> Vec<i32> {
        let mut ignore = Vec::new();
        for &id in ids.iter().filter(|&&id| id != -1) {
            ignore.push(id);
            ignore.extend(self.annexes_of(id));
        }
        ignore
    }

    /// The annexes of the building `main`, in id order.
    #[must_use]
    pub fn annexes_of(&self, main: i32) -> Vec<i32> {
        self.annexes
            .iter()
            .filter(|&(_, &m)| m == main)
            .map(|(&a, _)| a)
            .collect()
    }

    /// The main building of the annex `id`, if it is one.
    #[must_use]
    pub fn annex_of(&self, id: i32) -> Option<i32> {
        self.annexes.get(&id).copied()
    }

    /// `FUN_141663dc0` with the ally flag ([`Spatial::allied`]), for the walk's avoidance.
    #[must_use]
    pub fn allied_with(&self, from: PlayerId, to: PlayerId) -> bool {
        self.allied(from, to)
    }

    /// The players `from` counts as allies ([`Spatial::allied_with`]), one bit each.
    #[must_use]
    pub fn allies_of(&self, from: PlayerId) -> u64 {
        PlayerId::each(64)
            .filter(|&to| self.allied(from, to))
            .fold(0, |bits, to| bits | (1 << to.get()))
    }

    /// `FUN_141663dc0` with the ally flag: `to` counts for `from` when it is `from`, or `from`
    /// stands to it in relation 0 or 4; Gaia counts for no one else.
    pub(super) fn allied(&self, from: PlayerId, to: PlayerId) -> bool {
        if from == to {
            return true;
        }
        if to == PlayerId::new(0) {
            return false;
        }
        matches!(
            self.relations
                .get(from.index())
                .and_then(|r| r.get(to.index())),
            Some(0 | 4)
        )
    }

    /// `FUN_1410562e0` on the obstruction manager: its ignore list becomes `ids` (none for an
    /// empty list), until the next call.
    pub fn set_manager_ignore(&mut self, ids: &[i32]) {
        self.manager_ignore.clear();
        self.manager_ignore.extend_from_slice(ids);
    }

    /// The obstruction manager's ignore list as the last `FUN_1410562e0` left it
    /// ([`Spatial::set_manager_ignore`]).
    #[must_use]
    pub fn manager_ignore(&self) -> Vec<i32> {
        self.manager_ignore.clone()
    }

    /// `FUN_1410562d0` on the obstruction manager: its buffer becomes `buffer` (None: not set),
    /// until the next call.
    pub const fn set_manager_buffer(&mut self, buffer: Option<f32>) {
        self.manager_buffer = buffer;
    }

    /// The obstruction manager's buffer as the last `FUN_1410562d0` left it
    /// ([`Spatial::set_manager_buffer`]).
    #[must_use]
    pub const fn manager_buffer(&self) -> Option<f32> {
        self.manager_buffer
    }

    /// What the queries that set no buffer grow the records by: the buffer the last
    /// `FUN_1410562d0` left, 0 when not set (the `MoveTo`'s target check `FUN_140aa2a70`, the
    /// motion step's waypoint query, the formation's lines `FUN_14103a1f0`, the group's clear
    /// spot `FUN_141007630`).
    #[must_use]
    pub fn manager_grow(&self) -> f32 {
        self.manager_buffer.unwrap_or(0.0)
    }

    /// The walk's obstruction query (`FUN_14114cc60` -> `FUN_141051900` with flags `0x90`, each
    /// cell's records by `FUN_141051e00`): the records of a tested kind (kind 10 filed as 2),
    /// except the `ignore`d objects', in the cells the query visits ([`Spatial::visits`], `rect`
    /// grown by `grow`), whose box, grown by `grow` (the walker's radius), meets the step's
    /// rectangle `rect` and whose grown box meets the step `from` -> `to`
    /// (`FUN_1410512f0`; a circle's, kind 5, `FUN_141051630`, alike). A moving object of
    /// `player` or an ally grows by `grow × ally_factor` instead (`+0x84`, the walker's master's
    /// `min_collision_size_multiplier`). The ids hit, in id order; the game's are in its grid's,
    /// which no caller reads yet. A rectangle that, grown, leaves the map meets the map edge's
    /// records first ([`Spatial::edge_records`], `FUN_141057290`). Both its callers
    /// (`FUN_14114cc60`, the gate's `FUN_1411d0570`) set the manager's ignore list to `ignore`
    /// first ([`Spatial::set_manager_ignore`]).
    pub fn walk_hits(&mut self, walk: WalkQuery<'_>) -> Result<Vec<i32>, GenieError> {
        let WalkQuery {
            start: from,
            end: to,
            rect,
            grow,
            ally_factor,
            player,
            ignore,
        } = walk;
        self.set_manager_ignore(ignore);
        // its callers reset the buffer after the query (FUN_1410562d0 with 0)
        self.set_manager_buffer(None);
        let walk = Segment { from, to };
        let query = rect.grown(grow);
        // a query beyond the map's edge meets the edge's records first (FUN_141057290)
        let mut hits: Vec<i32> = if query.leaves_map(self.map) {
            self.edge_records(query, QUERY_HITS)
                .iter()
                .map(|e| e.id)
                .collect()
        } else {
            Vec::new()
        };
        // a box that misses the step's rectangle even grown by the larger of the two grows cannot
        // be hit (a smaller grow's box lies inside it): skip it before the grid's filing test,
        // which costs the most. Without a player a moving object's test fails, so then all go
        // through it
        let reach = grow.max(ally_factor * grow);
        let skip = |p: &Placed| !rect.overlaps(p.bounds().grown(reach));
        for (id, p) in self.records_in(query) {
            let Some(o) = p.obstruction else {
                continue;
            };
            if player.is_some() && skip(p) {
                continue;
            }
            let kind = if o.kind == 10 { 2 } else { o.kind };
            if !TESTED_KINDS.contains(&kind) || ignore.contains(&id) || !self.visits(p, kind, query)
            {
                continue;
            }
            let g = if self.gaits.get(&id).is_some_and(|g| g.moving) {
                let player = player.ok_or(GenieError::unsupported("a walker without a place"))?;
                if self.allied(p.player, player) {
                    ally_factor * grow
                } else {
                    grow
                }
            } else {
                grow
            };
            let b = p.bounds();
            if rect.overlaps(b.grown(g)) && record_meets_segment(kind, b, g, walk) {
                hits.push(id);
            }
        }
        Ok(hits)
    }

    /// `FUN_1411570c0` (the caller holds the ends inside the map): the manager's ignore list
    /// becomes `ignore` (`FUN_1410562e0`), with `mask` the temporary records of the tiles of the
    /// line's box grown by `grow` that the mask lacks go in ([`Spatial::put_tile_records`]), the
    /// records the line meets ([`Spatial::line_hits`]), then the buffer the routine set is reset
    /// (`FUN_1410562d0` with 0) and, with `mask`, the tile records go (`FUN_1410561f0`).
    ///
    /// # Errors
    ///
    /// As [`Spatial::line_hits`] (the records go all the same).
    pub fn line_query(
        &mut self,
        from: Point,
        to: Point,
        grow: f32,
        mask: Option<u8>,
        ignore: &[i32],
    ) -> Result<Vec<i32>, GenieError> {
        self.set_manager_ignore(ignore);
        if let Some(mask) = mask {
            self.put_tile_records(from, to, grow, mask);
        }
        let hits = self.line_hits(from, to, grow, true, ignore);
        self.set_manager_buffer(None);
        if mask.is_some() {
            self.clear_tile_records();
        }
        hits
    }

    /// `FUN_1411570c0`'s query (`FUN_141051900` with flags 0x10, any player): the records the
    /// line `from` -> `to` meets, each box grown by `grow` (the manager's buffer,
    /// `FUN_1410562d0`), but the `ignore`d objects' (`FUN_1410562e0`), at most 25 (the query
    /// stops there). First the map edge's records when the grown rectangle leaves the map
    /// ([`Spatial::edge_records`]); with `tiles` the temporary tile records still put in the
    /// cells the query visits ([`Spatial::put_tile_records`], kind 3, the tile's box), in the
    /// tiles' row order; then the objects' records of a tested kind (kind 10 filed as 2) in the
    /// cells the query visits whose grown box meets the line's rectangle and the line
    /// (`FUN_1410512f0`; a circle's alike), in id order. Ids: an object's, a tile record's
    /// ([`tile_record_id`]) or an edge record's.
    ///
    /// # Errors
    ///
    /// A record whose place the slice does not know near the line.
    pub fn line_hits(
        &self,
        from: Point,
        to: Point,
        grow: f32,
        tiles: bool,
        ignore: &[i32],
    ) -> Result<Vec<i32>, GenieError> {
        const MOST_HITS: usize = 25;
        let rect = Rect::spanning(from, to);
        let query = rect.grown(grow);
        let mut hits: Vec<i32> = Vec::new();
        if query.leaves_map(self.map) {
            hits.extend(self.edge_records(query, QUERY_HITS).iter().map(|e| e.id));
            if MOST_HITS <= hits.len() {
                hits.truncate(MOST_HITS);
                return Ok(hits);
            }
        }
        self.check_known_records(query)?;
        let walk = Segment { from, to };
        let meets = |kind: u8, bx: Rect| {
            rect.overlaps(bx.grown(grow)) && record_meets_segment(kind, bx, grow, walk)
        };
        if tiles {
            let level = self.grid.level(0);
            let shift = self.grid.shift(level);
            for &(y, x) in &self.tile_records {
                let id = tile_record_id(x, y);
                let bx = Rect::new(x as f32, y as f32, x as f32 + 1.0, y as f32 + 1.0);
                if self.visits_cell(level, x >> shift, y >> shift, query)
                    && !ignore.contains(&id)
                    && meets(3, bx)
                {
                    hits.push(id);
                    if MOST_HITS <= hits.len() {
                        return Ok(hits);
                    }
                }
            }
        }
        for (id, p) in self.records_in(query) {
            let Some(o) = p.obstruction else {
                continue;
            };
            let kind = if o.kind == 10 { 2 } else { o.kind };
            if !TESTED_KINDS.contains(&kind) || ignore.contains(&id) || !self.visits(p, kind, query)
            {
                continue;
            }
            if !meets(kind, p.bounds()) {
                continue;
            }
            hits.push(id);
            if MOST_HITS <= hits.len() {
                break;
            }
        }
        Ok(hits)
    }
}
