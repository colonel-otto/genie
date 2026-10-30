//! The points ashore: a new point after none went ashore, the drop location
//! (`find_drop_location`) and the map as the Transport reads it ([`MapShore`]).

use crate::GenieError;
use crate::garrison::transport::{
    DEEP_WATER, DRAW_SCALE, DROP_GAP, DROP_TRIES, RETRIES, RETRY_SPREAD, RETRY_STEP, STACK_PLACE,
    Shore, WATER,
};
use crate::map::Footprint;
use crate::map::Position;
use crate::map::terrain::Terrain;
use crate::math::conversions::cvttss2si;
use crate::math::rng::Well512;

/// `FUN_140aa3ba0`'s new point once no unit went ashore: from the old point (x, y), draws of the
/// Action stream ("Unload": x, then y), each `r / 0xccc` tiles plus the old coordinate less
/// [`RETRY_SPREAD`], while the terrain a map read sees at the last draw's tile
/// ([`Terrain::read`], kept while the tile is off the `map`) is water (22, 23 or 1), at most
/// 100; the last draw becomes the point either way. It sails on (true) only when a draw left
/// the water with more than one draw to spare: the 99th and 100th never count.
pub(crate) fn retry_point(
    point: [f32; 2],
    map: [i32; 2],
    terrain: &Terrain,
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
            seen = terrain.read(tx as i16, ty as i16);
        }
        if left <= 0 {
            left = tries - 2;
            break;
        }
    }
    (at, 0 < left)
}

/// Where a drop location ([`DropDraws`]) puts a unit: about a centre within radii.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Around {
    /// The centre (x, y).
    pub center: [f32; 2],
    /// The radii (x, y).
    pub radius: Footprint,
}

/// `FUN_141192d10`'s draws (`find_drop_location`) for a unit of master radii `unit_radius` about
/// `around`: up to 50 draws of the Object stream ("`find_drop_location`"). Each picks a side
/// (`r * 4 / 0x7fff`: east, west, north or south past the radii, the unit's own and
/// [`DROP_GAP`]), then a place along it (`r * 2 radius / 32767 - radius`, [`DRAW_SCALE`]); a draw
/// of `0x7fff` (side 4) keeps the place the last draw left in `last` (the caller's across its
/// calls; none yet is [`STACK_PLACE`]). The caller tests each place on the map
/// ([`DropDraws::next_on_map`]): the first in its zone that the unit's master may take and the
/// unit could stand on is the drop location.
pub(crate) struct DropDraws<'a> {
    around: Around,
    unit_radius: Footprint,
    map: [i32; 2],
    last: &'a mut Option<[f32; 2]>,
    rng: &'a mut Well512,
    left: u16,
}

impl<'a> DropDraws<'a> {
    /// The draws for a unit of master radii `unit_radius` about `around` on a map of `map`
    /// tiles, the place the last draw left in `last`, drawing from `rng`.
    pub(crate) const fn new(
        around: Around,
        unit_radius: Footprint,
        map: [i32; 2],
        last: &'a mut Option<[f32; 2]>,
        rng: &'a mut Well512,
    ) -> Self {
        Self {
            around,
            unit_radius,
            map,
            last,
            rng,
            left: DROP_TRIES,
        }
    }

    /// The next draw's place on the map (`COMISS`: a NaN coordinate passes); a place off it
    /// uses its draw up. None once the 50 draws are spent.
    ///
    /// # Errors
    ///
    /// [`STACK_PLACE`].
    pub(crate) fn next_on_map(&mut self) -> Result<Option<[f32; 2]>, GenieError> {
        let (w, h) = (self.map[0] as f32, self.map[1] as f32);
        while 0 < self.left {
            self.left -= 1;
            let [x, y] = self.draw()?;
            if 0.0 > x || 0.0 > y || x >= w || y >= h {
                continue;
            }
            return Ok(Some([x, y]));
        }
        Ok(None)
    }

    /// One draw: its side, then its place along it, which `last` keeps.
    fn draw(&mut self) -> Result<[f32; 2], GenieError> {
        let [cx, cy] = self.around.center;
        let (rx, ry) = (self.around.radius.x(), self.around.radius.y());
        let off_x = rx + (self.unit_radius.x() + DROP_GAP);
        let off_y = ry + (self.unit_radius.y() + DROP_GAP);
        let along = |r: u32, radius: f32, centre: f32| {
            ((r as i32 as f32 * (radius + radius)) / DRAW_SCALE - radius) + centre
        };
        let side = (self.rng.rand15() as i32 * 4) / 0x7fff;
        let place = match side {
            0 => [cx + off_x, along(self.rng.rand15(), ry, cy)],
            1 => [cx - off_x, along(self.rng.rand15(), ry, cy)],
            2 => {
                let y = cy + off_y;
                [along(self.rng.rand15(), rx, cx), y]
            }
            3 => {
                let y = cy - off_y;
                [along(self.rng.rand15(), rx, cx), y]
            }
            _ => self.last.ok_or(STACK_PLACE)?,
        };
        *self.last = Some(place);
        Ok(place)
    }
}

/// A drop location's zone test (`FUN_141192d10`'s, for the Transport's unload): any for `zone`
/// −1 (`u32::MAX`); else the zone of the place's tile on the transport's zone map (`zone_map`:
/// `FUN_1411958c0` on its `CVTTSS2SI` tile, 0 below 0 or without a map), cut to its low byte,
/// is `zone` or touches it (`FUN_141414f70` on both low bytes; without a map none does).
///
/// # Errors
///
/// A zone past the adjacency table.
pub(crate) fn in_zone(
    [x, y]: [f32; 2],
    zone: u32,
    zone_map: Option<&crate::pathing::zones::ZoneMap>,
) -> Result<bool, GenieError> {
    if zone == u32::MAX {
        return Ok(true);
    }
    let (tx, ty) = (cvttss2si(x), cvttss2si(y));
    let tile_zone = match zone_map {
        Some(m) if 0 <= tx && 0 <= ty => m.zone(tx, ty),
        _ => 0,
    };
    let low = tile_zone & 0xff;
    if u32::from(low) == zone {
        return Ok(true);
    }
    let Some(m) = zone_map else {
        return Ok(false);
    };
    m.adjacent(low, (zone & 0xff) as u16)
        .ok_or(GenieError::unreachable("a zone adjacency past its table"))
}

/// The map as a unit's Transport reads it ([`Shore`]): the map's terrain, the zone maps and the
/// planners, the unit's terrain restriction (its master's `+0x606`; none below 0) and its tile.
pub(crate) struct MapShore<'a> {
    pub(super) terrain: &'a Terrain,
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
        position: Position,
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
            tile: [cvttss2si(position.x()), cvttss2si(position.y())],
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
        Ok(Some(self.zones.of(w, h, &self.terrain.kinds, values)?))
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
