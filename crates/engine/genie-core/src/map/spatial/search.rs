//! A position's change, the sight check and the unit AI's searches: the closest object and the
//! best unit to attack.

use crate::map::spatial::{
    AttackField, CLOSEST_RANGE, ClosestChecks, FARMER_CLASS, FISH_TRAP, FarmLook, Filter,
    LIVESTOCK_CLASS, PASTURE_CLASS, PASTURES, Placed, QUERY_HITS, Reach, SEARCH_REUSE_SQUARED,
    Spatial, TESTED_KINDS, TRAPPERS, players_seen, record_meets_segment, tile_record_id,
};
use crate::unit_ai::{Found, GenieError, Pick, Search};
use crate::vision::registry::Record;
use crate::vision::visibility::Visibility;

impl Spatial {
    /// `FormationTransform`'s line query (`FUN_141051900` with flags 0x40, any player, the
    /// segment test `FUN_141051e00`) with the buffer `grow` the formation finds
    /// ([`Spatial::manager_grow`]): whether the line `from` -> `to` meets a record, the
    /// `ignore`d objects' and the moving objects' (flag 0x40, [`Gait::moving`]; the game's list
    /// of moving objects still tested holds only ignored ones here) left out. The records: the
    /// map edge's where the line's rectangle, grown by `grow`, leaves the map
    /// ([`Spatial::edge_records`]), the temporary tile records still put
    /// ([`Spatial::put_tile_records`]: the formation's over its box, kind 3, the tile's box) and
    /// the objects' of a tested kind (kind 10 filed as 2), in the cells the query visits; a
    /// record counts when its box, grown by `grow`, meets the line's rectangle and the line
    /// (`FUN_1410512f0`; a circle's alike).
    ///
    /// # Errors
    ///
    /// A record whose place the slice does not know near the line.
    ///
    /// [`Gait::moving`]: crate::map::spatial::Gait::moving
    pub fn still_line_hit(
        &self,
        from: [f32; 2],
        to: [f32; 2],
        grow: f32,
        ignore: &[i32],
    ) -> Result<bool, GenieError> {
        let rect = [
            if to[0] <= from[0] { to[0] } else { from[0] },
            if to[1] <= from[1] { to[1] } else { from[1] },
            if from[0] <= to[0] { to[0] } else { from[0] },
            if from[1] <= to[1] { to[1] } else { from[1] },
        ];
        let query = [
            rect[0] - grow,
            rect[1] - grow,
            grow + rect[2],
            grow + rect[3],
        ];
        let [w, h] = self.map;
        if (query[0] < 0.0 || query[1] < 0.0 || (w as f32) < query[2] || (h as f32) < query[3])
            && !self.edge_records(query, QUERY_HITS).is_empty()
        {
            return Ok(true);
        }
        self.check_known_records(query)?;
        let walk = [from[0], from[1], to[0], to[1]];
        let meets = |kind: u8, bx: [f32; 4]| {
            let grown = [bx[0] - grow, bx[1] - grow, grow + bx[2], grow + bx[3]];
            rect[0] <= grown[2]
                && grown[0] <= rect[2]
                && rect[1] <= grown[3]
                && grown[1] <= rect[3]
                && record_meets_segment(kind, bx, grow, walk)
        };
        let level = self.grid.level(0);
        let shift = self.grid.shift(level);
        let tile = self.tile_records.borrow().iter().any(|&(y, x)| {
            self.visits_cell(level, x >> shift, y >> shift, query)
                && !ignore.contains(&tile_record_id(x, y))
                && meets(3, [x as f32, y as f32, x as f32 + 1.0, y as f32 + 1.0])
        });
        if tile {
            return Ok(true);
        }
        Ok(self.records_in(query).any(|(id, p)| {
            let Some(o) = p.obstruction else {
                return false;
            };
            let kind = if o.kind == 10 { 2 } else { o.kind };
            TESTED_KINDS.contains(&kind)
                && !ignore.contains(&id)
                && !self.gaits.get(&id).is_some_and(|g| g.moving)
                && self.visits(p, kind, query)
                && meets(kind, p.bounds())
        }))
    }

    /// The object's new place (`new_position`, `0x1411a58b0`): its tile's counts and, on a new
    /// tile, its sight follow.
    pub fn set_position(&mut self, id: i32, position: [f32; 3]) {
        if let Some(p) = self.objects.get_mut(&id) {
            let from = p.position;
            p.position = position;
            let (old, new) = (self.tiles.tile_of(from), self.tiles.tile_of(position));
            if old != new {
                let objects = &self.objects;
                if let Some(i) = old {
                    self.tiles.take_off(i, id, |o| Self::is_tall(objects, o));
                }
                if let Some(i) = new {
                    self.tiles.append(i, id, |o| Self::is_tall(objects, o));
                }
            }
            self.refile(id);
            let Some(p) = self.objects.get(&id) else {
                return;
            };
            if let Some(v) = self.sight.as_mut() {
                v.moved(id, from, position, p.radius);
                v.registry_moved(id, p.class, from, position);
            }
        }
    }

    /// Fails when a frozen object of `player` that may have moved sees, or saw, near `rect`: the
    /// player's visible map there, and the registry's records the objects' masks keep, may differ
    /// from the game's (its sight, plus a tile, beyond what it may have moved).
    pub fn check_sight(&self, player: usize, rect: [f32; 4]) -> Result<(), GenieError> {
        let Some(v) = self.sight.as_ref() else {
            return Ok(());
        };
        let near = self.frozen().any(|(id, p)| {
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
            return Err(GenieError::unsupported(
                "a stopped object's sight may reach the search",
            ));
        }
        Ok(())
    }

    /// A unit AI's search ([`Search`]) for object `me` of `player` (`FUN_140c63e20`): the query
    /// over the player's registry ([`crate::vision::registry::Registry::visible_units`]) unless the
    /// [`SearchCache`] holds near the place (`fresh` false: the last answer stands); then its
    /// [`Pick`], an attack by `attack`'s tests of the objects found and its choice among them. A query that reads nothing leaves the last answer too (and so is not fresh). A
    /// query near a frozen object, or a last answer the slice has not got, fails. `shuns`
    /// answers for a found object whether a class-10 searcher passes it ([`Spatial::closest_object`]).
    ///
    /// [`SearchCache`]: crate::map::spatial::SearchCache
    #[allow(clippy::too_many_arguments)]
    pub fn search(
        &self,
        me: i32,
        player: usize,
        search: &Search<'_>,
        attack: &mut dyn crate::unit_ai::AttackPicker,
        reach: &mut Reach<'_>,
        moving: &dyn Fn(i32) -> bool,
        shuns: &dyn Fn(&Placed) -> Result<bool, GenieError>,
        farm: &FarmLook<'_>,
    ) -> Result<Found, GenieError> {
        let v = self.sight.as_ref().ok_or(GenieError::unsupported(
            "a search without the players' sight",
        ))?;
        let fresh = self.search_query(player, search)?;
        let cache = self.search_cache.borrow();
        let found = cache.last.as_ref().ok_or(GenieError::unsupported(
            "a search that reads the registry's last answer (before any query)",
        ))?;
        self.pick_found(
            v,
            (me, player),
            search,
            found,
            fresh,
            attack,
            reach,
            moving,
            shuns,
            farm,
        )
    }

    /// What player `player`'s queries make of the first `players` players' objects
    /// ([`Spatial::unit_diplomacy`], `player+0x278`).
    ///
    /// # Errors
    ///
    /// A player whose kinds the slice did not load.
    pub fn query_kinds(&self, player: usize, players: usize) -> Result<Vec<u32>, GenieError> {
        let kinds = self
            .unit_diplomacy
            .get(player)
            .ok_or(GenieError::unsupported(
                "a search of a player without its unit diplomacy",
            ))?;
        Ok(kinds.iter().take(players).map(|&k| k as u32).collect())
    }

    /// The query of a unit AI's search (`FUN_140c63e20`'s first part): over the player's
    /// registry unless the [`SearchCache`] holds near the place; true when it queried afresh. The
    /// lists it leaves are [`Spatial::searched`]'s.
    ///
    /// [`SearchCache`]: crate::map::spatial::SearchCache
    pub fn search_query(&self, player: usize, search: &Search<'_>) -> Result<bool, GenieError> {
        let v = self.sight.as_ref().ok_or(GenieError::unsupported(
            "a search without the players' sight",
        ))?;
        let r = v.registry.as_ref().ok_or(GenieError::unsupported(
            "a search without the objects-in-sight registry",
        ))?;
        let relations = self.query_kinds(player, r.players)?;
        let [x, y] = search.at;
        let mut fresh = false;
        {
            let mut cache = self.search_cache.borrow_mut();
            let (dx, dy) = (x - cache.at[0], y - cache.at[1]);
            if SEARCH_REUSE_SQUARED < dy * dy + dx * dx || !cache.holds {
                cache.holds = true;
                cache.at = search.at;
                let radius = search.radius.min(1000.0);
                let [rx, ry] = search.size;
                let rect = [
                    (x - (radius + rx)) - 0.001,
                    (y - (radius + ry)) - 0.001,
                    x + (radius + rx) + 0.001,
                    y + (radius + ry) + 0.001,
                ];
                self.check_known_classes(rect, search.classes)?;
                self.check_sight(player, rect)?;
                if let Some(lists) = r.visible_units(
                    player,
                    search.at,
                    search.radius,
                    0,
                    search.classes,
                    &relations,
                    search.size,
                    0xff,
                )? {
                    cache.last = Some(lists);
                    fresh = true;
                }
            }
        }
        Ok(fresh)
    }

    /// The lists the last search left (`DAT_143f69178`, by relation kind; 4 the enemies), as
    /// [`Spatial::search_query`] left them.
    pub fn searched(&self) -> Option<[Vec<Record>; 5]> {
        self.search_cache.borrow().last.clone()
    }

    /// The search's pick of what it found ([`Spatial::search`]).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn pick_found(
        &self,
        v: &Visibility,
        (me, player): (i32, usize),
        search: &Search<'_>,
        found: &[Vec<Record>; 5],
        fresh: bool,
        attack: &mut dyn crate::unit_ai::AttackPicker,
        reach: &mut Reach<'_>,
        moving: &dyn Fn(i32) -> bool,
        shuns: &dyn Fn(&Placed) -> Result<bool, GenieError>,
        farm: &FarmLook<'_>,
    ) -> Result<Found, GenieError> {
        match search.pick {
            Pick::Closest {
                group,
                relation,
                state,
                master,
                workers,
                spared,
                reach: reaching,
            } => {
                let filter = Filter {
                    group,
                    relation,
                    state,
                    master,
                    workers,
                    spared,
                    reach: reaching,
                };
                let checks = (moving, shuns, farm);
                let (id, distance) =
                    self.closest_object(v, me, player, found, filter, reach, checks)?;
                Ok(Found {
                    id,
                    enemies: !found[4].is_empty(),
                    position: self.objects.get(&id).map(|p| p.position),
                    fresh,
                    players: players_seen(found),
                    distance: crate::checkpoint::cvttss2si(distance),
                    retries: [None; 3],
                })
            }
            Pick::Look => Ok(Found {
                id: -1,
                enemies: !found[4].is_empty(),
                position: None,
                fresh,
                players: 0,
                distance: -1,
                retries: [None; 3],
            }),
            Pick::Attack { ask, .. } => {
                let choice = Self::best_unit_to_attack(found, &ask, search.memory, attack, reach)?;
                let id = choice.target;
                let mut retries = [None; 3];
                for (slot, r) in retries.iter_mut().zip(choice.retries) {
                    *slot = Some(r);
                }
                Ok(Found {
                    id,
                    enemies: !found[4].is_empty(),
                    position: self.objects.get(&id).map(|p| p.position),
                    fresh,
                    players: 0,
                    distance: -1,
                    retries,
                })
            }
        }
    }

    /// `ClosestObject` (`FUN_140c488b0`) for object `me` over what its search `found`, its
    /// reachability check on (argument 8, every caller's): each record by relation and order:
    /// itself, an object that is gone and one dying (state 7 and up) are passed, so is one the
    /// [`Filter`] leaves out; then one the unit reaches within `CLOSEST_RANGE` (slot 0x3d0,
    /// `reach`) and nearer than the nearest so far (by the search's distance; a tie goes to the
    /// lower id) is the nearest. Livestock (class 58) counts when the searcher's player owns it or
    /// it is not in state 2, and not while its top action is the Artifact with a point
    /// (`moving`: slot 0x288 not −1 and slot 0x530 107, "Sheep is moving"). A searcher of class
    /// 10 passes the objects `shuns` names (of master 440 or a master with object flag 0x10; the
    /// game's `+0x6b4` of 1, not met, would let it take them). Not modelled: a workers cap
    /// (argument 6; the objects' `+0x644`), the unbuilt farm test of a search for state 0, the
    /// pastures and farms (class 49). Nothing found answers −1.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn closest_object(
        &self,
        v: &Visibility,
        me: i32,
        player: usize,
        found: &[Vec<Record>; 5],
        filter: Filter,
        reach: &mut Reach<'_>,
        (moving, shuns, farm): ClosestChecks<'_>,
    ) -> Result<(i32, f32), GenieError> {
        let class_10 = self.objects.get(&me).is_some_and(|m| m.class == 10);
        let farmer = self
            .objects
            .get(&me)
            .is_some_and(|m| m.class == FARMER_CLASS);
        let trapper = self
            .objects
            .get(&me)
            .is_some_and(|m| TRAPPERS.contains(&m.master));
        // FUN_140c32b50: the relation argument 9 passes over, as a list
        let spared = match filter.spared {
            0 => Some(2),
            1 => Some(3),
            2 => Some(4),
            3 => Some(1),
            _ => None,
        };
        let mut best: Option<(i32, f32)> = None;
        for (kind, records) in found.iter().enumerate() {
            // the relation filter (`param_3`): 0 friends and itself, 1 neutrals, 2 enemies, 3
            // itself, else any
            let related = match filter.relation {
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
                if 7 <= seer.state || !related || spared == Some(kind) {
                    continue;
                }
                if filter.group != -1 && i32::from(p.class) != filter.group {
                    continue;
                }
                if filter.master != -1 && i32::from(p.master) != filter.master {
                    continue;
                }
                if filter.state != -1 && i32::from(seer.state) != filter.state {
                    continue;
                }
                if filter.workers != -1 {
                    return Err(GenieError::unsupported(
                        "ClosestObject's workers cap (+0x644)",
                    ));
                }
                if filter.state == 0 {
                    return Err(GenieError::unsupported("ClosestObject's unbuilt farm test"));
                }
                // a farm (class 49): not a pasture's livestock (`+0x790`); a pasture is not
                // modelled; without a master asked for, one without workers that the unit
                // gathers from (a fish trap for a fishing ship, any other for a villager)
                if filter.group == PASTURE_CLASS {
                    let (workers, parent) = farm(record.id).ok_or(GenieError::unsupported(
                        "a found farm the slice does not hold",
                    ))?;
                    if parent != -1 {
                        continue;
                    }
                    if PASTURES.contains(&p.master) {
                        return Err(GenieError::unsupported(
                            "ClosestObject over pastures (their workers and livestock)",
                        ));
                    }
                    if filter.master == -1 {
                        if 1 <= workers {
                            continue;
                        }
                        if !trapper && p.master == FISH_TRAP {
                            continue;
                        }
                        if !farmer && p.master != FISH_TRAP {
                            continue;
                        }
                    }
                }
                if filter.group == LIVESTOCK_CLASS
                    && ((p.player != player && seer.state == 2) || moving(record.id))
                {
                    continue;
                }
                if class_10 && shuns(p)? {
                    continue;
                }
                if filter.reach && !reach(record.id, CLOSEST_RANGE)? {
                    continue;
                }
                match best {
                    Some((id, d)) if d <= record.distance => {
                        if record.distance == d && record.id < id {
                            best = Some((record.id, d));
                        }
                    }
                    _ => best = Some((record.id, record.distance)),
                }
            }
        }
        // the distance starts at `DAT_14330ec20` (−1) without a find
        Ok(best.unwrap_or((-1, -1.0)))
    }

    /// `bestUnitToAttack` (`FUN_140c44680`, [`crate::best_target::best_unit_to_attack`]) over
    /// the relation lists its search `found` (each record's id in order), with the unit AI's
    /// `memory`: `attack` reads the unit and the objects, the paths (slot 0x3d0) are `reach`'s.
    ///
    /// # Errors
    ///
    /// What the reads and tests do not model.
    pub(super) fn best_unit_to_attack(
        found: &[Vec<Record>; 5],
        ask: &crate::unit_ai::AttackAsk,
        memory: crate::unit_ai::Memory<'_>,
        attack: &mut dyn crate::unit_ai::AttackPicker,
        reach: &mut Reach<'_>,
    ) -> Result<crate::best_target::Choice, GenieError> {
        let me = attack.searcher(memory)?;
        let lists: [Vec<i32>; 5] = std::array::from_fn(|k| found[k].iter().map(|r| r.id).collect());
        let mut field = AttackField {
            picker: attack,
            reach,
        };
        crate::best_target::best_unit_to_attack(&me, &ask.ask(), &lists, &mut field)
    }
}
