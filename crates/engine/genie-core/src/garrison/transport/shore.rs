//! The points ashore: a new point after none went ashore, the drop location
//! (`find_drop_location`) and the map as the Transport reads it ([`MapShore`]).

use crate::GenieError;
use crate::garrison::transport::{
    DEEP_WATER, DRAW_SCALE, DROP_GAP, DROP_TRIES, RETRIES, RETRY_SPREAD, RETRY_STEP, STACK_PLACE,
    Shore, WATER,
};
use crate::math::rng::Well512;
use crate::world::checkpoint::cvttss2si;

/// `FUN_140aa3ba0`'s new point once no unit went ashore: from the old point (x, y), draws of the
/// Action stream ("Unload": x, then y), each `r / 0xccc` tiles plus the old coordinate less
/// [`RETRY_SPREAD`], while the terrain a map read sees at the last draw's tile (`terrain`, kept
/// while the tile is off the map) is water (22, 23 or 1), at most 100; the last draw becomes the
/// point either way. It sails on (true) only when a draw left the water with more than one draw
/// to spare: the 99th and 100th never count.
pub fn retry_point(
    point: [f32; 2],
    map: [i32; 2],
    terrain: impl Fn(i16, i16) -> u8,
    rng: &mut Well512,
) -> ([f32; 2], bool) {
    let [w, h] = map;
    let mut seen = DEEP_WATER;
    let mut at = point;
    let mut left = RETRIES;
    loop {
        let tries = left;
        left = tries - 1;
        if !matches!(seen.wrapping_sub(DEEP_WATER), 0 | 1) && seen != WATER {
            break;
        }
        let x = (rng.rand15() / RETRY_STEP) as f32 + point[0] - RETRY_SPREAD;
        let y = (rng.rand15() / RETRY_STEP) as f32 + point[1] - RETRY_SPREAD;
        at = [x, y];
        let (tx, ty) = (cvttss2si(x), cvttss2si(y));
        if 0 <= tx && 0 <= ty && tx < w && ty < h {
            seen = terrain(tx as i16, ty as i16);
        }
        if left <= 0 {
            left = tries - 2;
            break;
        }
    }
    (at, 0 < left)
}

/// Where [`find_drop_location`] puts a unit: about a centre within radii.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Around {
    /// The centre (x, y).
    pub center: [f32; 2],
    /// The radii (x, y).
    pub radius: [f32; 2],
}

/// `FUN_141192d10` (`find_drop_location`) for a unit of master radii `unit_radius` about
/// `around`: up to 50 draws of the Object stream ("`find_drop_location`"). Each picks a side
/// (`r * 4 / 0x7fff`: east, west, north or south past the radii, the unit's own and
/// [`DROP_GAP`]), then a place along it (`r * 2 radius / 32767 - radius`, [`DRAW_SCALE`]); a draw
/// of `0x7fff` (side 4) keeps the place the last draw left in `last` (the caller's across its
/// calls; none yet is [`STACK_PLACE`]). A place on the map (`COMISS`: a NaN coordinate passes)
/// in `zone` (`zone_ok`, the caller's: any for −1) that the unit's master may take and the unit
/// could stand on (`free`, the caller's) is the drop location.
///
/// # Errors
///
/// [`STACK_PLACE`], and what the caller's tests give.
#[allow(clippy::too_many_arguments)]
pub fn find_drop_location(
    around: Around,
    unit_radius: [f32; 2],
    map: [i32; 2],
    last: &mut Option<[f32; 2]>,
    rng: &mut Well512,
    zone_ok: &mut dyn FnMut([f32; 2]) -> Result<bool, GenieError>,
    free: &mut dyn FnMut([f32; 2]) -> Result<bool, GenieError>,
) -> Result<Option<[f32; 2]>, GenieError> {
    let [cx, cy] = around.center;
    let [rx, ry] = around.radius;
    let off_x = rx + (unit_radius[0] + DROP_GAP);
    let off_y = ry + (unit_radius[1] + DROP_GAP);
    let along = |r: u32, radius: f32, centre: f32| {
        ((r as i32 as f32 * (radius + radius)) / DRAW_SCALE - radius) + centre
    };
    let (w, h) = (map[0] as f32, map[1] as f32);
    for _ in 0..DROP_TRIES {
        let side = (rng.rand15() as i32 * 4) / 0x7fff;
        let place = match side {
            0 => [cx + off_x, along(rng.rand15(), ry, cy)],
            1 => [cx - off_x, along(rng.rand15(), ry, cy)],
            2 => {
                let y = cy + off_y;
                [along(rng.rand15(), rx, cx), y]
            }
            3 => {
                let y = cy - off_y;
                [along(rng.rand15(), rx, cx), y]
            }
            _ => last.ok_or(STACK_PLACE)?,
        };
        *last = Some(place);
        let [x, y] = place;
        if 0.0 > x || 0.0 > y || x >= w || y >= h {
            continue;
        }
        if zone_ok(place)? && free(place)? {
            return Ok(Some(place));
        }
    }
    Ok(None)
}

/// The map as a unit's Transport reads it ([`Shore`]): the map's terrain, the zone maps and the
/// planners, the unit's terrain restriction (its master's `+0x606`; none below 0) and its tile.
pub(crate) struct MapShore<'a> {
    pub(super) terrain: &'a crate::map::terrain::Terrain,
    pub(super) zones: &'a mut crate::pathing::zones::Zones,
    pub(super) pathing: &'a mut crate::pathing::Pathing,
    pub(super) tiles: &'a crate::map::spatial::Tiles,
    pub(super) map: [i32; 2],
    pub(super) restriction: Option<usize>,
    pub(super) tile: [i32; 2],
}

impl<'a> MapShore<'a> {
    /// The map about a unit of terrain restriction `restriction` at `position`.
    ///
    /// # Errors
    ///
    /// A map without its terrain.
    pub(crate) fn new(
        spatial: &'a crate::map::spatial::Spatial,
        zones: &'a mut crate::pathing::zones::Zones,
        pathing: &'a mut crate::pathing::Pathing,
        restriction: i16,
        position: [f32; 3],
    ) -> Result<Self, GenieError> {
        let terrain = spatial
            .terrain
            .as_ref()
            .ok_or(crate::pathing::zones::NO_TERRAIN)?;
        Ok(Self {
            terrain,
            zones,
            pathing,
            tiles: spatial.tiles(),
            map: spatial.map,
            restriction: usize::try_from(restriction).ok(),
            tile: [cvttss2si(position[0]), cvttss2si(position[1])],
        })
    }

    /// The unit's zone map (`FUN_1411931c0`: the map's for its restriction, `FUN_1414138d0`).
    pub(super) fn zone_map(
        &mut self,
    ) -> Result<Option<&crate::pathing::zones::ZoneMap>, GenieError> {
        let Some(r) = self.restriction else {
            return Ok(None);
        };
        let values = self
            .terrain
            .restrictions
            .get(r)
            .ok_or(crate::pathing::zones::UNKNOWN_RESTRICTION)?;
        let [w, h] = self.map;
        Ok(Some(self.zones.of(w, h, &self.terrain.kinds, values)))
    }
}

impl Shore for MapShore<'_> {
    fn zone(&mut self, x: i32, y: i32) -> Result<u16, GenieError> {
        if x < 0 || y < 0 {
            return Ok(0);
        }
        Ok(self.zone_map()?.map_or(0, |m| m.zone(x, y)))
    }

    fn near(&mut self, zone: u8, reach: f32) -> Result<bool, GenieError> {
        let tile = self.tile;
        Ok(self
            .zone_map()?
            .is_some_and(|m| m.near(u16::from(zone), tile, reach).is_some()))
    }

    fn flags(&self, x: i16, y: i16) -> u8 {
        self.terrain.flags_at(x, y)
    }

    fn shore_point(&mut self, from: [f32; 2], to: [f32; 2]) -> Option<[f32; 2]> {
        let planner = self.pathing.planner(self.restriction?, self.tiles)?;
        planner.last_open_point(from, to)
    }
}
