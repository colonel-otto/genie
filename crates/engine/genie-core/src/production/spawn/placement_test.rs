//! The master's placement test (slot 0x50): the objects near a candidate, BUILD's placement
//! test and the spawn's.

use crate::production::spawn::{
    BUILDING, BuildSite, CLASS_51, CLASS_54, FARM, GATE, INSIDE, PASSABLE, REACH, RELIC, TREE,
    Tested, WALL, hill_fails, placement_terrains,
};
use crate::unit_ai::GenieError;

/// An object near a candidate, as the placement test and the blocker test read it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Near {
    /// The object.
    pub id: i32,
    /// Its place.
    pub position: [f32; 3],
    /// Its master's radii on the ground and whether its height radius is above 0.
    pub radius: [f32; 2],
    /// Its master's height radius is above 0.
    pub tall: bool,
    /// Its master's type (`+0x8`), class, id, `can_be_built_on` (`+0x5e9`), `obstruction_type`
    /// (`+0x651`) and `enable_auto_gather` (`+0x681`).
    pub kind: u8,
    /// Its master's class.
    pub class: i16,
    /// Its master.
    pub master: i16,
    /// Its master's `can_be_built_on` (`+0x5e9`).
    pub buildable_on: u8,
    /// Its master's `obstruction_type` (`+0x651`).
    pub obstruction_type: u8,
    /// Its master's `enable_auto_gather` (`+0x681`).
    pub auto_gather: bool,
    /// Its state (`+0x640`) and, for a building, its `desolid_flag` (`+0xc60`).
    pub state: u8,
    /// A building's `desolid_flag` (`+0xc60`).
    pub desolid: bool,
    /// Its owner.
    pub player: usize,
    /// Its master moves (speed above 0).
    pub moving: bool,
    /// Its master's `fog_visibility` (`+0x605`).
    pub fog_visibility: u8,
    /// A building's visibility mask (`+0x8f8`, the registry's) when its footprint has a UMV entry
    /// (`+0x904` not 0); None otherwise.
    pub fog_mask: Option<u64>,
    /// It holds an amount (`+0x310` above 0) of food, wood, stone or gold (its attribute,
    /// `+0x308`, through `FUN_140fdd470` below 4: 0 to 3 and the meats 15 to 17).
    pub holds: bool,
    /// Its hit points.
    pub hp: f32,
    /// The terrain of its tile (`+0x2e8`, taken as the tile it stands on) is a forest
    /// ([`crate::map::terrain::TerrainKind::forest`]).
    pub forest_tile: bool,
}

impl Near {
    /// The blocker tests of `FUN_1411c5b60`: a building (`FUN_1411942c0`), a tree past state 2
    /// (`FUN_141195660`), the relic (`FUN_141194e80`), and `FUN_141194170`'s master with
    /// `enable_auto_gather` or of a class that stays from state 2 on (5, 9, 10, 29, 31, 33, 38,
    /// 40, 41, 58, 61) or from state 3 on (7, 8, 15, 32, 48, 63).
    #[must_use]
    pub const fn stays(&self) -> bool {
        if self.kind == BUILDING || (self.class == TREE && 2 < self.state) || self.master == RELIC {
            return true;
        }
        if self.auto_gather {
            return true;
        }
        match self.class {
            5 | 9 | 10 | 29 | 31 | 33 | 38 | 40 | 41 | 58 | 61 => 1 < self.state,
            7 | 8 | 15 | 32 | 48 | 63 => 2 < self.state,
            _ => false,
        }
    }
}

/// A unit to put down, beside a building or ashore, of a master that does not move (no
/// terrain restriction to test its place by).
pub(crate) const NOT_MOVING: GenieError =
    GenieError::unsupported("a unit to put down of a master that does not move");

/// The master a placement test places (its slot 0x50 with the spawn's arguments: by its radii,
/// the terrain checked, the player's explored tiles needed, the elevation checked by its hill
/// mode, the objects checked).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placing<'a> {
    /// The unit's player.
    pub player: usize,
    /// Its master's radii.
    pub radius: [f32; 2],
    /// Its master's height radius is above 0.
    pub tall: bool,
    /// Its master's class.
    pub class: i16,
    /// Its terrain restriction's value per terrain.
    pub restriction: &'a [f32],
    /// `hill_mode` (`+0x604`).
    pub hill_mode: u8,
    /// The placement terrains (`+0x5f2`..`+0x5f8`, [`placement_terrains`]).
    pub placement: [i16; 4],
}

/// What the placement test reads of the map.
pub trait PlaceMap {
    /// The map's size in tiles.
    fn size(&self) -> [i32; 2];
    /// A tile's terrain (the map tile's `+0x9`).
    fn terrain(&self, x: i32, y: i32) -> Result<u8, GenieError>;
    /// Terrain `t`'s flags (the `.dat`'s `is_water` byte, the map's terrain record `+2`), None
    /// beyond the terrains.
    fn terrain_flags(&self, _t: usize) -> Option<u8> {
        None
    }
    /// How many terrains the map's table holds.
    fn terrain_count(&self) -> usize {
        0
    }
    /// `FUN_141663500` is not 0: the player sees or saw the tile.
    fn explored(&self, player: usize, x: i32, y: i32) -> bool;
    /// The player sees the tile now (its mask `+0x2a8` meets the tile's visible bits).
    fn visible(&self, player: usize, x: i32, y: i32) -> bool;
    /// The player's masks: what it sees (`+0x2a8`) and what it explored (`+0x2a0`).
    fn vision(&self, player: usize) -> [u64; 2];
    /// A tile's elevation (`+0x14`), when the map holds it.
    fn elevation(&self, x: i32, y: i32) -> Option<u8>;
    /// A tile's shape (`+0x8`, [`crate::map::terrain::Terrain::set_elevations`]), when the map holds
    /// it.
    fn shape(&self, x: i32, y: i32) -> Option<u8>;
    /// Player `a` counts player `b` as an ally (`FUN_141663dc0`): itself; never Gaia; relation 0,
    /// or with `with_4` relation 4 too (the zone mode's relation 1 is not modelled).
    fn allied(&self, a: usize, b: usize, with_4: bool) -> bool;
    /// The objects on tile (`x`, `y`) (their centre there), in the tile list's order; `Err` where
    /// it cannot tell.
    fn objects(&self, x: i32, y: i32) -> Result<Vec<Near>, GenieError>;
}

/// The tiles within `REACH` of a point, clamped to the map (`(int)` of each bound, then the map's
/// edges), row by row.
pub(super) fn reach(map: [i32; 2], [x, y]: [f32; 2]) -> impl Iterator<Item = (i32, i32)> {
    let x0 = ((x - REACH) as i32 as i16).max(0);
    let x1 = ((x + REACH) as i32 as i16).min((map[0] - 1) as i16);
    let y0 = ((y - REACH) as i32 as i16).max(0);
    let y1 = ((y + REACH) as i32 as i16).min((map[1] - 1) as i16);
    (y0..=y1).flat_map(move |ty| (x0..=x1).map(move |tx| (i32::from(tx), i32::from(ty))))
}

/// An object a placement test's scan found, with its tile.
pub(super) type Found = ((i32, i32), Near);

/// The objects whose box overlaps a box of radii `r` at `p` among those `counts` lets through,
/// with their tile, in the scan's order.
pub(super) fn overlapping(
    map: &dyn PlaceMap,
    p: [f32; 2],
    r: [f32; 2],
    counts: &dyn Fn(&Near) -> Option<bool>,
) -> Result<Vec<Found>, GenieError> {
    let mut out = Vec::new();
    for (tx, ty) in reach(map.size(), p) {
        for o in map.objects(tx, ty)? {
            let Some(tall) = counts(&o) else { continue };
            if !(0.0 < o.radius[0] && 0.0 < o.radius[1] && tall) {
                continue;
            }
            let dx = (o.position[0] - p[0]).abs();
            let dy = (o.position[1] - p[1]).abs();
            if dx < o.radius[0] + r[0] && dy < o.radius[1] + r[1] {
                out.push(((tx, ty), o));
            }
        }
    }
    Ok(out)
}

/// BUILD's placement test (`FUN_140e7a400`'s call of the master's slot 0x50, the building
/// master's `FUN_14123aee0` with BUILD's flags: `p6` human, `p7`, `p8`, `p9` 1, `p10` 0, `p11` 1,
/// no object of its own, `p13` 0, `p14` 1): 0 when the foundation may go to `p`, else the game's
/// code. First the static test (`FUN_141123f80` through `FUN_141264db0`, whose scans are off)
/// over the clearance box: off the map 7; per tile, columns then rows, a terrain the
/// restriction forbids 2 (`DAT_142eb7e5c`'s threshold, the spawn's); a human who explored none of
/// the tiles 5; a hill mode's tile ([`hill_fails`]) 3. Then the building master's scan of the tiles within 8 (rows, then columns; each
/// tile's objects in its list's order): an object the player sees, or on a tile it never
/// explored (the test's explored flag `+0x730` is set by then), or in the fog with a fog
/// visibility, and only its own in the fog without one, that is below state 3 (another type
/// below 7), standing (or the master obstructs, or it is below type 30), not a wall under its
/// own gate, a desolid building only of an ally both ways, not a moving unit of the player, Gaia
/// or a mutual ally, not a felled tree off a forest (at most 1 hit point; `+0x2e8` taken as its
/// own tile), not buildable on, whose box with a
/// height overlaps the clearance box: 6. Not modelled: the placement terrains, hill modes, the
/// zone test (`FUN_1415c6400`), the editor (game mode 4), a fogged object of fog visibility 3,
/// game type 7's test of allies, the
/// outline radii of
/// obstruction types 11 to 13.
pub fn build_placement(
    site: &BuildSite<'_>,
    map: &dyn PlaceMap,
    p: [f32; 2],
) -> Result<u8, GenieError> {
    let [x, y] = p;
    let [rx, ry] = site.clearance;
    let [w, h] = map.size();
    let (far_x, far_y) = ((x + rx) - INSIDE, (y + ry) - INSIDE);
    if x - rx < 0.0 || w as f32 <= far_x || y - ry < 0.0 || h as f32 <= far_y {
        return Ok(7);
    }
    let mut explored = false;
    let (x0, x1, y0, y1) = ((x - rx) as i32, far_x as i32, (y - ry) as i32, far_y as i32);
    if !placement_terrains(site.placement, map, p, [x0, y0, x1, y1])? {
        return Ok(1);
    }
    // hill mode 3 measures the box's edge against the elevation of its first column's last row
    let base = if site.hill_mode == 3 {
        Some(map.elevation(x0, y1).ok_or(GenieError::unsupported(
            "a hill mode where the elevations are not known",
        ))?)
    } else {
        None
    };
    for tx in x0..=x1 {
        for ty in y0..=y1 {
            let t = map.terrain(tx, ty)?;
            let value = site
                .restriction
                .get(usize::from(t))
                .ok_or(GenieError::unsupported(
                    "a terrain its restriction does not list",
                ))?;
            if *value <= PASSABLE {
                return Ok(2);
            }
            explored = explored || (site.human && map.explored(site.player, tx, ty));
            if hill_fails(site.hill_mode, map, (tx, ty), (x0, y1), base)? {
                return Ok(3);
            }
        }
    }
    if site.human && !explored {
        return Ok(5);
    }
    if (0.0 < rx || 0.0 < ry) && site.zones {
        return Err(GenieError::unsupported(
            "BUILD's placement test in a zone mode",
        ));
    }
    // FUN_14123aee0's scan
    let near = |v: f32, size: i32| {
        let low = (v - BUILD_SCAN) as i32 as i16;
        let high = (v + BUILD_SCAN) as i32 as i16;
        (
            i32::from(low.max(0)),
            if i32::from(high) < size {
                i32::from(high)
            } else {
                size - 1
            },
        )
    };
    let ((x0, x1), (y0, y1)) = (near(x, w), near(y, h));
    let player = site.player;
    for ty in y0..=y1 {
        for tx in x0..=x1 {
            let (seen, visible) = (map.explored(player, tx, ty), map.visible(player, tx, ty));
            for o in map.objects(tx, ty)? {
                if site.own.contains(&o.id) {
                    continue;
                }
                if !(seen && visible) {
                    let fog = if seen { o.fog_visibility } else { 1 };
                    if fog == 3 {
                        return Err(GenieError::unsupported(
                            "a fogged object of fog visibility 3 near a BUILD",
                        ));
                    }
                    // without a fog visibility only the player's own objects count, and a
                    // building (not a farm) whose visibility mask meets both the player's masks
                    if fog == 0 {
                        let [sees, explored] = map.vision(player);
                        let shown = o.class != 49
                            && 50 <= o.kind
                            && o.fog_mask
                                .is_some_and(|m| m & explored != 0 && m & sees != 0);
                        if !shown && (o.player != player || !map.allied(o.player, player, true)) {
                            continue;
                        }
                    }
                } else if o.fog_visibility == 3 {
                    continue;
                }
                let alive = o.state < 3 || (o.kind != BUILDING && o.state < 7);
                let standing = site.obstructs || o.kind < 30 || !o.moving;
                let gate_wall = site.class == GATE && o.class == WALL && o.player == player;
                let foundation = o.kind == BUILDING
                    && !matches!(o.class, CLASS_51 | CLASS_54)
                    && o.desolid
                    && !(map.allied(player, o.player, true) && map.allied(o.player, player, true));
                if !alive || !standing || gate_wall || foundation {
                    continue;
                }
                // a moving unit of the player, of Gaia or of a mutual ally passes (the game type
                // 7's own test of allies is not modelled)
                let passes = 30 <= o.kind
                    && o.state == 2
                    && o.moving
                    && (o.player == player
                        || o.player == 0
                        || (map.allied(player, o.player, false)
                            && map.allied(o.player, player, false)));
                if passes || o.buildable_on == 1 {
                    continue;
                }
                // a felled tree (at most 1 hit point) passes off a forest (`FUN_140f5efb0` on its
                // tile's terrain)
                if o.class == TREE && o.hp <= 1.0 && !o.forest_tile {
                    continue;
                }
                if (11..=13).contains(&o.obstruction_type) {
                    return Err(GenieError::unsupported(
                        "an object of obstruction types 11 to 13 near a BUILD (its outline radii)",
                    ));
                }
                // no height: a building's counts as 1, and so does that of an object holding food,
                // wood, stone or gold
                let tall = o.tall || o.kind == BUILDING || o.holds;
                let [orx, ory] = o.radius;
                if 0.0 < orx
                    && 0.0 < ory
                    && tall
                    && (o.position[0] - x).abs() < orx + rx
                    && (o.position[1] - y).abs() < ory + ry
                {
                    return Ok(6);
                }
            }
        }
    }
    Ok(0)
}

/// How far BUILD's placement scan looks around the point (`FUN_14123aee0`, `DAT_142eb9430`).
pub(super) const BUILD_SCAN: f32 = crate::math::image::spawn::BUILD_SCAN;

/// The first object of a scan, which the game reports as the blocker: the tiles row by row, each
/// tile's objects in its list's order ([`crate::map::spatial::Spatial::objects_on`], the game's: the
/// load files them in the save's order, as the game's load does).
pub(super) fn first(found: &[Found]) -> Option<Near> {
    found.first().map(|&(_, o)| o)
}

/// The master's slot 0x50 (`0x141264db0`) with the spawn's arguments: `FUN_141123f80`'s test
/// (the map's edge, then per tile of the box the terrain restriction and the explored tiles,
/// then the objects overlapping), then its own scan of the objects overlapping, which reports
/// the first one. The editor's case (the game's mode 4), the placement terrains, the hill modes,
/// the zone test (`FUN_1415c6400`, the world's `+0x820` mode 1) and the moving objects' own
/// check (`DAT_143f01833`, probed 0) are not modelled.
pub fn placement(
    me: &Placing<'_>,
    map: &dyn PlaceMap,
    p: [f32; 2],
    zones: bool,
) -> Result<Tested, GenieError> {
    let [x, y] = p;
    let [rx, ry] = me.radius;
    let [w, h] = map.size();
    let (far_x, far_y) = ((x + rx) - INSIDE, (y + ry) - INSIDE);
    // FUN_141123f80's own result: 0 free; the objects of its scan report the blocker (6)
    let mut base_blocker = None;
    let off_map = x - rx < 0.0 || w as f32 <= far_x || y - ry < 0.0 || h as f32 <= far_y;
    let base_free = if off_map
        || !placement_terrains(
            me.placement,
            map,
            p,
            [(x - rx) as i32, (y - ry) as i32, far_x as i32, far_y as i32],
        )? {
        false
    } else {
        let (x0, y0, x1, y1) = ((x - rx) as i32, (y - ry) as i32, far_x as i32, far_y as i32);
        let mut passable = true;
        let mut seen = false;
        'tiles: for tx in x0..=x1 {
            for ty in y0..=y1 {
                let t = map.terrain(tx, ty)?;
                let value = me
                    .restriction
                    .get(usize::from(t))
                    .ok_or(GenieError::unsupported(
                        "a terrain its restriction does not list",
                    ))?;
                if *value <= PASSABLE {
                    passable = false;
                    break 'tiles;
                }
                seen = seen || map.explored(me.player, tx, ty);
                if me.hill_mode != 0 {
                    return Err(GenieError::unsupported("a placement test of a hill mode"));
                }
            }
        }
        if !passable || !seen {
            false
        } else if 0.0 < rx || 0.0 < ry {
            if zones {
                return Err(GenieError::unsupported("a placement test in a zone mode"));
            }
            let counts = |o: &Near| {
                let listed =
                    o.state < 7 && o.buildable_on != 1 && (o.kind != BUILDING || !o.desolid);
                let tall = o.tall
                    || (me.class == TREE && o.class == FARM)
                    || (o.kind == BUILDING && o.obstruction_type != 0);
                listed.then_some(tall && me.tall)
            };
            let found = if me.tall {
                overlapping(map, p, me.radius, &counts)?
            } else {
                Vec::new()
            };
            base_blocker = first(&found);
            base_blocker.is_none()
        } else {
            true
        }
    };
    // slot 0x50's own scan
    let counts = |o: &Near| {
        let listed = o.state < 7
            && o.buildable_on != 1
            && (o.kind != BUILDING || o.class == CLASS_51 || o.class == CLASS_54 || !o.desolid);
        let tall = o.tall || (o.kind == BUILDING && !matches!(o.obstruction_type, 0 | 12));
        listed.then_some(tall && me.tall)
    };
    let found = if me.tall && (0.0 < rx || 0.0 < ry) {
        overlapping(map, p, me.radius, &counts)?
    } else {
        Vec::new()
    };
    let blocker = first(&found).or(base_blocker);
    if base_free && found.is_empty() {
        return Ok(Tested::Free);
    }
    Ok(Tested::Blocked {
        stays: blocker.is_some_and(|b| b.stays()),
    })
}
