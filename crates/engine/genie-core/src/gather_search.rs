//! The searches for a resource to gather: a builder's search for more work around the building it
//! completed (`FUN_1415cc4e0`, over the tiles `FUN_1415cc1a0` reaches) and the unit AI's
//! `ClosestResourceObject` (`FUN_140c49760`, slot 0x80), which the builder's Build asks next
//! (`0x140ab0370` case 0xdf).

use crate::msvc;
use crate::tasks::DropSite;
use crate::unit_ai::Unsupported;

/// What the searches read of an object in a map tile's list.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Candidate {
    /// `+8`.
    pub id: i32,
    /// The owner.
    pub player: usize,
    /// Its master's class (`+0x410`, decoded).
    pub class: i16,
    /// Its master's id.
    pub master: i16,
    /// `+0x640`.
    pub state: u8,
    /// `+0x644`, `worker_num`.
    pub workers: i32,
    /// x, y, z.
    pub position: [f32; 3],
    /// `+0x790`, `parent_pasture_id` (−1 for none).
    pub pasture: i32,
    /// `+0x689` is 0 ([`crate::turn::Unit::untargetable`]).
    pub untargetable: bool,
    /// `+0x308`, `attribute_type_held`.
    pub attribute: i16,
    /// `+0x310`, `attribute_amount_held`.
    pub amount: f32,
    /// Its master's `enable_auto_gather` (`+0x681`).
    pub auto_gather: bool,
    /// Its master's `resource_gather_group` (`+0x683`).
    pub gather_group: u8,
}

/// The farm class (49), which both searches treat apart.
pub(crate) const FARM: i16 = 49;

/// What a builder's search reads of the builder and the building it completed.
#[derive(Clone, Copy, Debug)]
pub struct Builder {
    /// The builder's place.
    pub position: [f32; 3],
    /// Its master's x radius (`+0x5d8`).
    pub radius: f32,
    /// The building's place.
    pub building: [f32; 3],
    /// The search's reach in tiles: the building's line of sight held to 4 to 20 (`DAT_142eb0f48`,
    /// `DAT_142e7f38c`), truncated.
    pub range: i16,
}

/// `FUN_1415cc4e0` with `param_5` and `param_6` 0: the object the builder works next, from the
/// drop site's entry for the building, the builder's class and its owner's civ (`entry`,
/// [`crate::tasks::drop_site`]; none, or one without `auto_work`, finds nothing). The search
/// covers the tiles within the reach plus the builder's radius (rounded) of the building's tile,
/// cut to the map; of those only the tiles the builder reaches count ([`reached`], with
/// `blocked` the builder's restriction's planner, `FUN_140f7d470`; None without a planner, when
/// every tile counts). `objects` lists a tile's objects in the tile's order.
///
/// An object below state 6 whose class the entry takes ([`DropSite::resource_of`]) joins its
/// resource's group (gold 3, stone 2, food 0 and 15 to 17, wood 1), by its distance from the
/// builder truncated to whole tiles: the group keeps the first nearest or, at no lesser distance,
/// the one with fewer workers (its workers less one, at least 0), and its least distance of those
/// kept. Gold comes first, then stone, food and wood, each taking over only at a lesser least
/// distance. A farm (class 49) joins only through the entry's farm target, which is not modelled.
///
/// # Errors
///
/// A farm on a tile for an entry with a farm target, or two objects of the entry's on one tile
/// (whose list order the slice does not hold).
pub fn more_work(
    entry: Option<&DropSite>,
    builder: &Builder,
    map: [i32; 2],
    blocked: Option<&dyn Fn(i32, i32) -> bool>,
    objects: &dyn Fn(i32, i32) -> Result<Vec<Candidate>, Unsupported>,
) -> Result<Option<Candidate>, Unsupported> {
    let Some(entry) = entry.filter(|e| e.auto_work) else {
        return Ok(None);
    };
    // the box: (short)(int) of the building's place, ± the reach plus the builder's radius
    let r = builder
        .range
        .wrapping_add((builder.radius + 0.5) as i32 as i16);
    let (bx, by) = (
        builder.building[0] as i32 as i16,
        builder.building[1] as i32 as i16,
    );
    let x0 = bx.wrapping_sub(r).max(0);
    let y0 = by.wrapping_sub(r).max(0);
    let mut x1 = bx.wrapping_add(r);
    let mut y1 = by.wrapping_add(r);
    if map[0] <= i32::from(x1) {
        x1 = map[0] as i16 - 1;
    }
    if map[1] <= i32::from(y1) {
        y1 = map[1] as i16 - 1;
    }
    let (w, h) = (i32::from(x1 - x0) + 1, i32::from(y1 - y0) + 1);
    let start = [
        builder.position[0] as i32 - i32::from(x0),
        builder.position[1] as i32 - i32::from(y0),
    ];
    let grid = blocked.and_then(|b| {
        reached(start, [w, h], &|x, y| {
            b(i32::from(x0) + x, i32::from(y0) + y)
        })
    });
    // gold, stone, food, wood: (the object, its workers, the group's least distance)
    let mut groups: [Option<(Candidate, i32)>; 4] = [None; 4];
    let mut least = [90.0f32; 4];
    for x in x0..=x1 {
        for y in y0..=y1 {
            let (gx, gy) = (i32::from(x - x0), i32::from(y - y0));
            if grid.as_ref().is_some_and(|g| !g[(gx * h + gy) as usize]) {
                continue;
            }
            let on_tile = objects(i32::from(x), i32::from(y))?;
            let mut taken = 0;
            for o in on_tile {
                if o.class == FARM {
                    // its own tests (owner, pasture, the targeting flag, masters 1893 and 1897)
                    // come before the entry's, which takes no farm without a farm target
                    if entry.targets.iter().any(|t| t.class == FARM) {
                        return Err(Unsupported("a farm in a builder's search for more work"));
                    }
                    continue;
                }
                if 6 <= o.state {
                    continue;
                }
                let Some(resource) = entry.resource_of(o.class, o.state, o.workers) else {
                    continue;
                };
                taken += 1;
                if 1 < taken {
                    return Err(Unsupported(
                        "two objects of a drop site's on one tile in a builder's search",
                    ));
                }
                let g = match resource {
                    3 => 0,
                    2 => 1,
                    0 | 15..=17 => 2,
                    1 => 3,
                    _ => continue,
                };
                let dx = o.position[0] - builder.position[0];
                let dy = o.position[1] - builder.position[1];
                let d = crate::fmath::sqrt(dy * dy + dx * dx) as i32 as f32;
                let workers = (o.workers - 1).max(0);
                let take = match groups[g] {
                    Some((_, kept)) if least[g] <= d => workers < kept,
                    _ => true,
                };
                if take {
                    groups[g] = Some((o, workers));
                    if d < least[g] {
                        least[g] = d;
                    }
                }
            }
        }
    }
    // gold, then stone, food and wood, each at a lesser least distance
    let mut best: Option<(Candidate, f32)> = None;
    for g in 0..4 {
        if let Some((o, _)) = groups[g]
            && best.is_none_or(|(_, d)| least[g] < d)
        {
            best = Some((o, least[g]));
        }
    }
    Ok(best.map(|(o, _)| o))
}

/// `FUN_1415cc1a0`: the cells of a `size[0]` × `size[1]` box a flood from `start` reaches, by
/// columns (`[x * height + y]`): every cell next to (left, right, above, below) a cell it
/// entered and not `blocked`, `start` entered whatever it holds. None when `start` lies outside
/// the box.
#[must_use]
pub fn reached(
    start: [i32; 2],
    size: [i32; 2],
    blocked: &dyn Fn(i32, i32) -> bool,
) -> Option<Vec<bool>> {
    let [w, h] = size;
    let [sx, sy] = start;
    if sx < 0 || sy < 0 || w <= sx || h <= sy {
        return None;
    }
    let mut seen = vec![false; (w * h) as usize];
    seen[(sx * h + sy) as usize] = true;
    let mut todo = vec![(sx, sy)];
    let mut first = true;
    while let Some((x, y)) = todo.pop() {
        if !std::mem::take(&mut first) && blocked(x, y) {
            continue;
        }
        for (nx, ny) in [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)] {
            if 0 <= nx && nx < w && 0 <= ny && ny < h && !seen[(nx * h + ny) as usize] {
                seen[(nx * h + ny) as usize] = true;
                todo.push((nx, ny));
            }
        }
    }
    Some(seen)
}

/// What `ClosestResourceObject` reads of the unit that searches.
#[derive(Clone, Copy, Debug)]
pub struct Seeker {
    /// The owner.
    pub player: usize,
    /// The unit's place.
    pub position: [f32; 3],
    /// Its master's line of sight (`+0x5d0`).
    pub line_of_sight: f32,
    /// `+0x640`.
    pub state: u8,
    /// Its unit AI's current target (`+0x48`), which the search passes over.
    pub current_target: i32,
    /// `+0x794`, `my_pasture_id`.
    pub my_pasture: i32,
    /// `param_6`: below this state the unit finds nothing (2 for the unit AIs' searches, 0 for
    /// a mule cart's).
    pub min_state: u8,
}

/// One cell of the search's queue: a tile and its steps from the start.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Cell {
    x: i32,
    y: i32,
    steps: i32,
}

/// The most objects whose reach a search tests before it settles for what it has (`0x1e`).
const TESTED: i32 = 30;

/// The drop site a resource search starts from (`ClosestResourceObject`'s `param_3`).
#[derive(Clone, Copy, Debug)]
pub struct SearchSite {
    /// The site's place.
    pub position: [f32; 3],
    /// Its master's radii (`+0x5d8`, `+0x5dc`).
    pub radius: [f32; 2],
    /// Its master's line of sight (`+0x5d0`).
    pub line_of_sight: f32,
    /// The search starts from the site's footprint: the site's tile lies in the unit's zone (its
    /// restriction's zone of the tile, `FUN_1411958c0`, is the unit's own, `FUN_14118eea0`), or
    /// the site's master is 45 or 2172 (the docks). Else it starts from the site's tile alone.
    pub footprint: bool,
}

/// The tiles a pass of the search starts from and how far it goes.
struct Pass {
    /// The tile the steps' distance counts from (the search's point, truncated).
    origin: (i32, i32),
    /// The farthest distance a tile may lie from the origin.
    max_steps: i32,
    /// The queue's first cells, sorted.
    seeds: Vec<Cell>,
}

/// `ClosestResourceObject` (`FUN_140c49760`) with no point (`param_7`, `param_8` −1: the unit's
/// place), no drop site (`param_3` −1) and `param_6` 2: the closest object of gather group `kind`
/// that the unit reaches (`reach`, slot 0x3d0 with the range 0: `FUN_14152b6e0` holds in the
/// games the slice runs), −1 for none. A unit below state 2 is not modelled.
///
/// A breadth-first walk over the tiles from the unit's tile, its queue after the tile in hand kept
/// sorted by the distance of the tiles' centres from the unit ([`msvc::sort`]): a tile the player
/// has explored (`explored`), within the line of sight plus 5 of the unit (`1.4` per diagonal
/// step, `+ 0.61`, truncated) and that the unit can stand on (`passable`, `FUN_141185690`) adds
/// its four neighbours not seen yet that lie on the map. On each such tile the first object in a
/// pasture or targetable whose master gathers by itself (`enable_auto_gather` 1) as `kind`, with
/// some amount held, and that is not the unit AI's current target, is tested: with fewer workers
/// than `max_workers` (−1 for any) a reach makes it the answer; else a reach makes it the
/// fallback when it is fewer steps away than the last. After 30 tests, or the queue's end, the
/// fallback is the answer. A tile whose last object fails the first test counts it as found
/// (the game's loop leaves it in hand).
///
/// # Errors
///
/// A unit with a pasture of its own, or a tile whose order of objects decides (two candidates,
/// or another object beside one that fails the first test).
#[allow(clippy::too_many_arguments)]
pub fn closest_resource(
    seeker: &Seeker,
    kind: u8,
    max_workers: i32,
    map: [i32; 2],
    explored: &dyn Fn(i32, i32) -> Result<bool, Unsupported>,
    passable: &dyn Fn(i32, i32) -> bool,
    objects: &dyn Fn(i32, i32) -> Result<Vec<Candidate>, Unsupported>,
    reach: &mut dyn FnMut(i32) -> Result<bool, Unsupported>,
) -> Result<i32, Unsupported> {
    closest_resource_at(
        seeker,
        None,
        kind,
        max_workers,
        map,
        explored,
        passable,
        objects,
        reach,
    )
    .map(|(id, _)| id)
}

/// [`closest_resource`] from a drop site (`param_3`, None for −1 or a site not found), with what
/// its out argument (`param_5`) takes: the distance of the answer's tile from the pass's origin
/// (the search's 1.4-per-diagonal measure, truncated), None for no answer (the argument keeps its
/// value).
///
/// With a site the search's point is the site's place, and the queue sorts by the distance from
/// a point a quarter of the way from the site to the unit (`0.75` of the site's place plus `0.25`
/// of the unit's). From the site's footprint ([`SearchSite::footprint`]) the first pass starts
/// from every tile of the footprint the unit can stand on (its x from the site's x less its
/// radius, truncated, at least 1, less 1, to the site's x plus its radius, truncated, at most the
/// map's last column; y alike; row by row), reaches the site's line of sight plus 10 and walks a
/// box of the larger line of sight of the two; when it finds nothing a second pass starts from
/// the unit's tile as the search without a site does, its tests counted on from the first pass
/// unless that one stopped after 30, its queue still sorted by the point between. Otherwise the
/// one pass starts from the site's tile and reaches the unit's line of sight plus 5.
///
/// # Errors
///
/// As [`closest_resource`].
#[allow(clippy::too_many_arguments)]
pub fn closest_resource_at(
    seeker: &Seeker,
    site: Option<&SearchSite>,
    kind: u8,
    max_workers: i32,
    map: [i32; 2],
    explored: &dyn Fn(i32, i32) -> Result<bool, Unsupported>,
    passable: &dyn Fn(i32, i32) -> bool,
    objects: &dyn Fn(i32, i32) -> Result<Vec<Candidate>, Unsupported>,
    reach: &mut dyn FnMut(i32) -> Result<bool, Unsupported>,
) -> Result<(i32, Option<i32>), Unsupported> {
    /// The fallback's steps before it has one (`1000000`).
    const NO_STEPS: i32 = 1_000_000;
    const DIAGONAL: f32 = crate::image::gather_search::DIAGONAL;
    const ROUNDING: f32 = crate::image::gather_search::ROUNDING;
    const SITE_SHARE: f32 = crate::image::gather_search::SITE_SHARE;
    const UNIT_SHARE: f32 = crate::image::gather_search::UNIT_SHARE;
    const TILE_CENTRE: f32 = crate::image::gather_search::TILE_CENTRE;
    if seeker.state < seeker.min_state {
        return Ok((-1, None));
    }
    if seeker.my_pasture != -1 {
        return Err(Unsupported(
            "a resource search by a unit with a pasture of its own",
        ));
    }
    let [ux, uy, _] = seeker.position;
    let los = seeker.line_of_sight as i32;
    let unit_tile = (ux as i32, uy as i32);
    let single = |at: (i32, i32)| Pass {
        origin: at,
        max_steps: los + 5,
        seeds: vec![Cell {
            x: at.0,
            y: at.1,
            steps: 0,
        }],
    };
    // the sort's point, and the box's side (fixed for both passes)
    let (sort_at, side) = site.map_or(((ux, uy), (los + 10) * 3), |s| {
        let [sx, sy, _] = s.position;
        let at = (
            sx * SITE_SHARE + ux * UNIT_SHARE,
            sy * SITE_SHARE + uy * UNIT_SHARE,
        );
        let site_los = s.line_of_sight as i32;
        let wide = if site_los + 10 <= los + 10 {
            los
        } else {
            site_los
        };
        (
            at,
            if s.footprint {
                (wide + 10) * 3
            } else {
                (los + 10) * 3
            },
        )
    });
    let half = side / 2;
    let nearer = |a: Cell, b: Cell| {
        let d = |c: Cell| {
            let dx = sort_at.0 - (c.x as f32 + TILE_CENTRE);
            let dy = sort_at.1 - (c.y as f32 + TILE_CENTRE);
            dy * dy + dx * dx
        };
        d(a) < d(b)
    };
    let mut passes = Vec::with_capacity(2);
    match site {
        Some(s) if s.footprint => {
            let [sx, sy, _] = s.position;
            let [rx, ry] = s.radius;
            let x0 = ((sx - rx) as i32).max(1) - 1;
            let x1 = ((sx + rx) as i32).min(map[0] - 1);
            let y0 = ((sy - ry) as i32).max(1) - 1;
            let y1 = ((sy + ry) as i32).min(map[1] - 1);
            let mut seeds = Vec::new();
            for y in y0..=y1 {
                for x in x0..=x1 {
                    if passable(x, y) {
                        seeds.push(Cell { x, y, steps: 0 });
                    }
                }
            }
            msvc::sort(&mut seeds, &nearer);
            passes.push(Pass {
                origin: (sx as i32, sy as i32),
                max_steps: s.line_of_sight as i32 + 10,
                seeds,
            });
            passes.push(single(unit_tile));
        }
        Some(s) => passes.push(single((s.position[0] as i32, s.position[1] as i32))),
        None => passes.push(single(unit_tile)),
    }
    let cap = if max_workers == -1 {
        i32::MAX
    } else {
        max_workers
    };
    let (mut fallback, mut fallback_steps, mut fallback_d, mut tested) = (-1, NO_STEPS, 0, 0);
    for pass in passes {
        let (ox, oy) = (half - pass.origin.0, half - pass.origin.1);
        let mut seen = vec![false; (side * side) as usize];
        let at = |x: i32, y: i32| ((y + oy) * side + x + ox) as usize;
        for c in &pass.seeds {
            seen[at(c.x, c.y)] = true;
        }
        let mut queue = pass.seeds;
        let mut i = 0;
        while i < queue.len() {
            let Cell { x, y, steps } = queue[i];
            i += 1;
            if !explored(x, y)? {
                continue;
            }
            let (dx, dy) = ((pass.origin.0 - x).abs(), (pass.origin.1 - y).abs());
            let d = if dy < dx {
                ((dy as f32 * DIAGONAL + ROUNDING) as i32 - dy) + dx
            } else {
                ((dx as f32 * DIAGONAL + ROUNDING) as i32 - dx) + dy
            };
            if pass.max_steps < d || x < 0 || y < 0 || map[0] <= x || map[1] <= y {
                continue;
            }
            if passable(x, y) {
                for (nx, ny) in [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)] {
                    let c = at(nx, ny);
                    if !seen[c] {
                        seen[c] = true;
                        if 0 <= nx && nx < map[0] && 0 <= ny && ny < map[1] {
                            queue.push(Cell {
                                x: nx,
                                y: ny,
                                steps: steps + 1,
                            });
                        }
                    }
                }
                msvc::sort(&mut queue[i..], &nearer);
            }
            let on_tile = objects(x, y)?;
            let Some(o) = first_resource(&on_tile, kind)? else {
                continue;
            };
            tested += 1;
            if o.id == seeker.current_target {
                continue;
            }
            if o.workers < cap {
                if reach(o.id)? {
                    return Ok((o.id, Some(d)));
                }
            } else if steps < fallback_steps && reach(o.id)? {
                fallback = o.id;
                fallback_steps = steps;
                fallback_d = d;
            }
            if TESTED < tested {
                tested = 0;
                break;
            }
        }
        if fallback != -1 {
            return Ok((fallback, Some(fallback_d)));
        }
    }
    Ok((-1, None))
}

/// The object of a tile's list `ClosestResourceObject` takes: the first in a pasture or
/// targetable whose master gathers by itself as `kind` with some amount held; with none, the
/// last object when it is neither in a pasture nor targetable.
fn first_resource(on_tile: &[Candidate], kind: u8) -> Result<Option<Candidate>, Unsupported> {
    let open = |o: &Candidate| o.pasture != -1 || !o.untargetable;
    let found: Vec<&Candidate> = on_tile
        .iter()
        .filter(|o| open(o) && o.auto_gather && o.gather_group == kind && 0.0 < o.amount)
        .collect();
    match found.as_slice() {
        [one] => Ok(Some(**one)),
        [] if on_tile.iter().all(open) => Ok(None),
        [] if on_tile.len() == 1 => Ok(on_tile.first().copied()),
        _ => Err(Unsupported(
            "a tile whose order of objects decides a resource search",
        )),
    }
}

/// `FUN_140fdd470` then `FUN_140fdd5f0`: the gather group of a resource held, and the most workers
/// the Build's search for more work lets an object of it have (`0x140ab0c57`): food (0, 15 to 17)
/// 1 and 6, wood 0 and 2, stone 3 and 3, gold 4 and 3, the rest 2 and 2.
#[must_use]
pub const fn gather_group(attribute: i16) -> (u8, i32) {
    let resource = match attribute {
        0..=3 | 9 | 0x38 => attribute,
        15..=17 => 0,
        _ => -1,
    };
    let kind = match resource {
        0 => 1,
        1 => 0,
        2 => 3,
        3 => 4,
        _ => 2,
    };
    let workers = match kind {
        1 => 6,
        3 | 4 => 3,
        _ => 2,
    };
    (kind, workers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::DropTarget;

    fn tree(id: i32, x: f32, y: f32) -> Candidate {
        Candidate {
            id,
            player: 0,
            class: 15,
            master: 349,
            state: 2,
            workers: 0,
            position: [x, y, 0.0],
            pasture: -1,
            untargetable: false,
            attribute: 1,
            amount: 100.0,
            auto_gather: true,
            gather_group: 0,
        }
    }

    fn lumber_camp() -> DropSite {
        DropSite {
            building: 562,
            worker_class: 4,
            workers: Vec::new(),
            civs: Vec::new(),
            auto_work: true,
            targets: vec![DropTarget {
                class: 15,
                workers: -1,
                state: -1,
                attribute: 1,
            }],
            livestock: false,
        }
    }

    #[test]
    fn a_flood_enters_the_blocked_cells_next_to_what_it_reaches_but_not_past_them() {
        // a wall at x = 2 in a 5 x 3 box: the flood from (0, 1) reaches x 0..=1 and enters the
        // wall's cells, never x 3 or 4
        let seen = reached([0, 1], [5, 3], &|x, _| x == 2).unwrap();
        let col = |x: i32| {
            (0..3)
                .map(|y| seen[(x * 3 + y) as usize])
                .collect::<Vec<_>>()
        };
        assert_eq!(col(1), [true; 3]);
        assert_eq!(col(2), [true; 3]);
        assert_eq!(col(3), [false; 3]);
        assert!(reached([5, 0], [5, 3], &|_, _| false).is_none());
    }

    #[test]
    fn more_work_takes_the_first_nearest_tree_then_fewer_workers_at_no_lesser_distance() {
        let entry = lumber_camp();
        let b = Builder {
            position: [10.5, 10.5, 0.0],
            radius: 0.2,
            building: [12.0, 10.0, 0.0],
            range: 6,
        };
        // 3 and 4 both 3 tiles away (truncated): x outer, so 3 (x 7) comes first; 4 is as far
        // with fewer workers than 3's kept 1 and takes over
        let mut trees = [tree(3, 7.5, 10.5), tree(4, 13.5, 10.5)];
        trees[0].workers = 2;
        let objects = |x: i32, y: i32| {
            Ok(trees
                .iter()
                .filter(|t| t.position[0] as i32 == x && t.position[1] as i32 == y)
                .copied()
                .collect())
        };
        let found = more_work(Some(&entry), &b, [40, 40], None, &objects).unwrap();
        assert_eq!(found.map(|o| o.id), Some(4));
        let mut off = lumber_camp();
        off.auto_work = false;
        assert_eq!(
            more_work(Some(&off), &b, [40, 40], None, &objects).unwrap(),
            None
        );
    }

    #[test]
    fn gold_wins_over_nearer_wood_unless_the_wood_is_nearer_than_the_gold() {
        let mut entry = lumber_camp();
        entry.targets.push(DropTarget {
            class: 32,
            workers: -1,
            state: -1,
            attribute: 3,
        });
        let b = Builder {
            position: [10.5, 10.5, 0.0],
            radius: 0.2,
            building: [10.0, 10.0, 0.0],
            range: 6,
        };
        let mut gold = tree(7, 14.5, 10.5);
        gold.class = 32;
        let wood = tree(8, 12.5, 10.5);
        let all = [gold, wood];
        let objects = |x: i32, y: i32| {
            Ok(all
                .iter()
                .filter(|t| t.position[0] as i32 == x && t.position[1] as i32 == y)
                .copied()
                .collect())
        };
        // the wood (2 tiles) is nearer than the gold (4 tiles): it takes over
        let found = more_work(Some(&entry), &b, [40, 40], None, &objects).unwrap();
        assert_eq!(found.map(|o| o.id), Some(8));
    }

    #[test]
    fn the_resource_search_walks_out_from_the_unit_and_takes_the_first_it_reaches() {
        let seeker = Seeker {
            player: 1,
            position: [10.2, 10.7, 0.0],
            line_of_sight: 4.0,
            state: 2,
            current_target: -1,
            my_pasture: -1,
            min_state: 2,
        };
        // trees at (12, 10) and (8, 11): the second is nearer the unit's place
        let trees = [tree(1, 12.5, 10.5), tree(2, 8.5, 11.5)];
        let objects = |x: i32, y: i32| {
            Ok(trees
                .iter()
                .filter(|t| t.position[0] as i32 == x && t.position[1] as i32 == y)
                .copied()
                .collect())
        };
        let tree_at = |x: i32, y: i32| {
            trees
                .iter()
                .any(|t| t.position[0] as i32 == x && t.position[1] as i32 == y)
        };
        let mut asked = Vec::new();
        let found = closest_resource(
            &seeker,
            0,
            2,
            [40, 40],
            &|_, _| Ok(true),
            &|x, y| !tree_at(x, y),
            &objects,
            &mut |id| {
                asked.push(id);
                Ok(true)
            },
        )
        .unwrap();
        assert_eq!((found, asked), (2, vec![2]));
        // with the tree as the unit AI's target, the other one
        let busy = Seeker {
            current_target: 2,
            ..seeker
        };
        let found = closest_resource(
            &busy,
            0,
            2,
            [40, 40],
            &|_, _| Ok(true),
            &|x, y| !tree_at(x, y),
            &objects,
            &mut |_| Ok(true),
        )
        .unwrap();
        assert_eq!(found, 1);
    }

    #[test]
    fn a_search_from_a_drop_site_starts_from_its_footprint_then_from_the_unit() {
        let seeker = Seeker {
            player: 1,
            position: [10.2, 10.7, 0.0],
            line_of_sight: 4.0,
            state: 2,
            current_target: -1,
            my_pasture: -1,
            min_state: 2,
        };
        // a 2x2 site at (14, 10): its own tiles are blocked
        let site = SearchSite {
            position: [14.0, 10.0, 0.0],
            radius: [1.0, 1.0],
            line_of_sight: 6.0,
            footprint: true,
        };
        let search = |trees: &[Candidate], site: Option<&SearchSite>| {
            let objects = |x: i32, y: i32| {
                Ok(trees
                    .iter()
                    .filter(|t| t.position[0] as i32 == x && t.position[1] as i32 == y)
                    .copied()
                    .collect())
            };
            let blocked = |x: i32, y: i32| {
                (13..=14).contains(&x) && (9..=10).contains(&y)
                    || trees
                        .iter()
                        .any(|t| t.position[0] as i32 == x && t.position[1] as i32 == y)
            };
            closest_resource_at(
                &seeker,
                site,
                0,
                2,
                [40, 40],
                &|_, _| Ok(true),
                &|x, y| !blocked(x, y),
                &objects,
                &mut |_| Ok(true),
            )
            .unwrap()
        };
        // a tree by the unit and one by the site: without the site the unit's, with it the site's
        // (3 from the site's tile)
        let trees = [tree(1, 8.5, 11.5), tree(2, 17.5, 10.5)];
        assert_eq!(search(&trees, None).0, 1);
        assert_eq!(search(&trees, Some(&site)), (2, Some(3)));
        // the site's tile outside the unit's zone: one pass from the site's tile alone, which is
        // blocked, so nothing
        let apart = SearchSite {
            footprint: false,
            ..site
        };
        assert_eq!(search(&trees, Some(&apart)), (-1, None));
        // nothing within the site's line of sight plus 10 of it: the second pass from the unit
        // finds the tree 8 columns and a row off (its distance from the unit's tile)
        let blind = SearchSite {
            line_of_sight: 0.0,
            ..site
        };
        assert_eq!(search(&[tree(3, 2.5, 11.5)], Some(&blind)), (3, Some(9)));
    }

    #[test]
    fn a_held_resource_names_its_gather_group_and_the_workers_the_build_lets_it_have() {
        assert_eq!(gather_group(1), (0, 2));
        assert_eq!(gather_group(16), (1, 6));
        assert_eq!(gather_group(2), (3, 3));
        assert_eq!(gather_group(3), (4, 3));
        assert_eq!(gather_group(-1), (2, 2));
    }
}
