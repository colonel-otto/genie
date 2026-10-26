//! Where an Enter meets its target (`FUN_140a85f30`): the landing tile, the walled-off test,
//! the meeting point and the reach.

use crate::GenieError;
use crate::checkpoint::cvttss2si;
use crate::garrison::{EnterAction, EnterTarget, SIEGE_CLASS};
use crate::masters::{Master, Masters};
use crate::movement::footprint_gap_squared;
use crate::order::BUILDING;

/// How near the point a transport must reach for an Enter (slot 0x3d8's range), and the range up
/// to which an Enter's reach of a target with speed tests the tiles about the two
/// (`FUN_140a85c30`): `DAT_14330eb8c`, 1.0.
pub(super) const ENTER_NEAR: f32 = crate::math::image::garrison::ENTER_NEAR;
/// Half a tile (`DAT_142e7f384`, 0.5): what an Enter's meeting point adds to a tile or to a
/// place before `floorf` (`FUN_140a85f30`), and how far a target leaves its point before the
/// Enter walks again (`FUN_140a7bd10`).
pub(super) const HALF_TILE: f32 = crate::math::image::garrison::HALF_TILE;

/// An Enter whose transport cannot reach where it meets the unit (`set_state(13)` in
/// `set_state(4)`), which is not modelled.
pub(crate) const UNREACHED: GenieError =
    GenieError::unsupported("an Enter's transport that cannot reach the unit (state 13)");

/// What [`meeting`] (`FUN_140a85f30`) reads of an Enter's target.
#[derive(Clone, Debug)]
pub struct MeetingTarget {
    /// It has room for the unit (slot 0x198 with the unit).
    pub room: bool,
    /// Its speed (slot 0x208).
    pub speed: f32,
    /// Its top action's state (slot 0x2b0, `RGE_Action_Object::actionState`; 0 without one).
    pub state: u8,
    /// Its class (`+0x410`).
    pub class: i16,
    /// Its place, x and y.
    pub position: [f32; 2],
    /// Its top action's point (`FUN_140a7be40`, `FUN_140a7be50`), or why the slice has not got it.
    pub action_point: Result<[f32; 2], GenieError>,
}

/// `FUN_140a85f30`: where an Enter meets its target and the code its `set_state(4)` reads (1:
/// the transport moves there). A target without room for the unit (slot 0x198), or without speed
/// (`COMISS 0, speed` with `JNC`: a NaN speed has some), is met at its place. One idle (its
/// action state 0) or in state 2 and with room meets the unit where its slot 0x428
/// ([`landing_tile`], `landing`) finds the shore, at the tile's centre (code 1), else, the unit
/// standing on a beach (`beach`, [`crate::terrain::Terrain::beach`]), at the unit's place rounded
/// to the nearest corner (`floorf` of each plus 0.5, code 1), else at its place. One under an
/// action in state 3, 4 or 11 meets it where that action goes, but a siege weapon (class 13)
/// at its place; any other state at its place.
pub fn meeting(
    t: &MeetingTarget,
    unit: [f32; 2],
    landing: impl FnOnce() -> Result<Option<[i32; 2]>, GenieError>,
    beach: impl FnOnce() -> bool,
) -> Result<([f32; 2], u8), GenieError> {
    let at = (t.position, 0);
    if !t.room || t.speed <= 0.0 {
        return Ok(at);
    }
    if matches!(t.state, 0 | 2) {
        if let Some([x, y]) = landing()? {
            return Ok(([x as f32 + HALF_TILE, y as f32 + HALF_TILE], 1));
        }
        // the state is 0 or 2 here, never 11, 4 or 3
        if beach() {
            return Ok((
                [(unit[0] + HALF_TILE).floor(), (unit[1] + HALF_TILE).floor()],
                1,
            ));
        }
        return Ok(at);
    }
    if t.class == SIEGE_CLASS || !matches!(t.state, 11 | 4 | 3) {
        return Ok(at);
    }
    Ok((t.action_point.clone()?, 0))
}

/// `FUN_141157840` (a unit's slot 0x428, its last argument 1): from tile `from` (a transport's)
/// toward tile `to` (the unit's) in twice as many steps as the longer axis's tiles (the `y`
/// axis only when it is strictly longer), each step adding the whole difference over the step
/// count in floats, the first new tile on the way that `lands` (its tile and the float place
/// reached: a beach in the unit's zone where the unit could stand, slot 0x278) is where the two
/// meet. None when the tiles are one, or no tile lands.
pub fn landing_tile(
    from: [i32; 2],
    to: [i32; 2],
    mut lands: impl FnMut([i32; 2], [f32; 2]) -> Result<bool, GenieError>,
) -> Result<Option<[i32; 2]>, GenieError> {
    let (dx, dy) = (to[0].wrapping_sub(from[0]), to[1].wrapping_sub(from[1]));
    let (fx, fy) = (dx as f32, dy as f32);
    if fx == 0.0 && fy == 0.0 {
        return Ok(None);
    }
    let longer = if fy.abs() <= fx.abs() { dx } else { dy };
    let steps = cvttss2si(longer as f32).wrapping_abs().wrapping_mul(2);
    let (mut x, mut y) = (from[0] as f32, from[1] as f32);
    let mut last = [-999, -999];
    for _ in 0..steps.max(0) {
        x += fx / steps as f32;
        y += fy / steps as f32;
        let tile = [cvttss2si(x), cvttss2si(y)];
        if tile != last {
            last = tile;
            if lands(tile, [x, y])? {
                return Ok(Some(tile));
            }
        }
    }
    Ok(None)
}

/// `FUN_140a85c30`'s tile test for a range of at most 1 to a target with speed: about the tiles
/// of the unit's and the target's movement places (slot 0x520, `+0x824`; `me` and `them`),
/// their box (when it spans more than one tile on an axis, its first tile and the next, within
/// the map `map`), each tile but those two where the unit could not stand (`stands`, slot 0x278
/// at the tile's corner) counts the buildings on it (`buildings`, type 80 of the tile's list).
/// True when they are exactly two: the unit is not in reach.
pub fn walled_off(
    me: [f32; 2],
    them: [f32; 2],
    map: [i32; 2],
    mut stands: impl FnMut(i32, i32) -> Result<bool, GenieError>,
    mut buildings: impl FnMut(i32, i32) -> Result<usize, GenieError>,
) -> Result<bool, GenieError> {
    let (mx, my) = (cvttss2si(me[0]), cvttss2si(me[1]));
    let (tx, ty) = (cvttss2si(them[0]), cvttss2si(them[1]));
    let (x0, y0) = (mx.min(tx), my.min(ty));
    let (mut x1, mut y1) = (mx.max(tx), my.max(ty));
    if 1 < x1.wrapping_sub(x0) || 1 < y1.wrapping_sub(y0) {
        x1 = (x0 + 1).min(map[0] - 1);
        y1 = (y0 + 1).min(map[1] - 1);
    }
    let mut count = 0usize;
    for x in x0..=x1 {
        for y in y0..=y1 {
            if (x, y) == (mx, my) || (x, y) == (tx, ty) || stands(x, y)? {
                continue;
            }
            count += buildings(x, y)?;
        }
    }
    Ok(count == 2)
}

/// [`MeetingTarget`] of the target `t` (of master `tm`) with `room` for the unit: its top speed
/// ([`crate::movement::MoveStats::speed`]), its top action's state and its point (`+0x38`,
/// `+0x3c`): a `MoveTo`'s goal, or for one on an object the mover's
/// [`crate::movement::Mover::move_to_point`]; a Transport's point. An action the slice does not
/// hold has neither; another no point the slice keeps.
pub(crate) fn meeting_target(
    t: &crate::turn::Unit,
    tm: &Master,
    room: bool,
) -> Result<MeetingTarget, GenieError> {
    if let Some(e) = t.unmodelled_action.clone() {
        return Err(e);
    }
    let action_point = match (t.top_action_kind(), t.move_to) {
        (Some(crate::move_to::MOVE_TO), Some(m)) => m
            .goal
            .or_else(|| t.mover.as_ref().and_then(|v| v.move_to_point))
            .map(|p| [p[0], p[1]])
            .ok_or(UNPLACED),
        // the Transport's point (+0x38, +0x3c), where it sails
        (Some(crate::transport::TRANSPORT), _) => t
            .transport
            .map(|tr| [tr.point[0], tr.point[1]])
            .ok_or(UNPLACED),
        _ => Err(UNPLACED),
    };
    Ok(MeetingTarget {
        room,
        speed: t.mover.as_ref().map_or(0.0, |m| m.stats.speed),
        state: t.top_action().map_or(0, |a| a.state),
        class: tm.class,
        position: [t.position[0], t.position[1]],
        action_point,
    })
}

/// A transport's top action whose point the slice does not keep (`FUN_140a7be40`).
pub(super) const UNPLACED: GenieError =
    GenieError::unsupported("an Enter's transport under an action the slice does not place");

/// [`meeting`] for `unit` (with its movement state `stats`) and its target `t`: the shore on the
/// way from the target's tile to the unit's ([`landing_tile`]): a beach
/// ([`crate::terrain::Terrain::beach`]) whose zone for the unit's restriction (`FUN_1411958c0`:
/// 0 for a tile below 0) is the zone of the unit's own tile (`FUN_14118eea0`), where the unit
/// could stand (slot 0x278, [`crate::spatial::Spatial::point_clear`] with the master's x radius
/// and the restriction's mask); else a beach under the unit (`FUN_1413fbf30`).
pub(crate) fn meeting_point(
    t: &MeetingTarget,
    unit: (i32, [f32; 3], &crate::movement::MoveStats),
    spatial: &crate::spatial::Spatial,
    zones: &mut crate::pathing::zones::Zones,
) -> Result<([f32; 2], u8), GenieError> {
    let (id, [ux, uy, _], stats) = unit;
    let terrain = spatial.terrain.as_ref();
    let landing = || {
        let terrain = terrain.ok_or(crate::pathing::zones::NO_TERRAIN)?;
        let values = terrain
            .restrictions
            .get(stats.restriction)
            .ok_or(crate::pathing::zones::UNKNOWN_RESTRICTION)?;
        let [w, h] = spatial.map;
        let map = zones.of(w, h, &terrain.kinds, values);
        let own = map.zone(cvttss2si(ux), cvttss2si(uy));
        let from = [cvttss2si(t.position[0]), cvttss2si(t.position[1])];
        landing_tile(from, [cvttss2si(ux), cvttss2si(uy)], |[x, y], [fx, fy]| {
            if !terrain.beach(x, y) {
                return Ok(false);
            }
            let zone = if x < 0 || y < 0 { 0 } else { map.zone(x, y) };
            if zone != own {
                return Ok(false);
            }
            spatial.point_clear(id, [fx, fy], stats.radius[0], stats.restriction_mask)
        })
    };
    let beach = || terrain.is_some_and(|tr| tr.beach(cvttss2si(ux), cvttss2si(uy)));
    meeting(t, [ux, uy], landing, beach)
}

/// [`EnterAction::in_reach`] of the unit `id` (its movement state `stats`, its place in
/// `spatial`) for its target `target` (its top speed and where it last moved to): the squared
/// footprints' gap of their places, and the tile test ([`walled_off`]) about where each last
/// moved to (slot 0x520), its buildings those whose master (`masters`, each player's) is of
/// type 80.
pub(crate) fn enter_reach(
    spatial: &crate::spatial::Spatial,
    masters: &[Masters],
    (id, stats, moved_at): (i32, &crate::movement::MoveStats, [f32; 3]),
    (target, target_mover): (i32, Option<&crate::movement::Mover>),
    range: f32,
) -> Result<bool, GenieError> {
    let me = spatial
        .get_known(id)?
        .ok_or(GenieError::unsupported("an entering unit without a place"))?;
    let them = spatial.get_known(target)?.ok_or(GenieError::unsupported(
        "an Enter's building without a place",
    ))?;
    let gap = footprint_gap_squared(me.position, me.radius, them.position, them.radius);
    let speed = target_mover.map_or(0.0, |m| m.stats.speed);
    let them_at = target_mover.map_or(them.position, |m| m.moved_at);
    EnterAction::in_reach(range, gap, speed, || {
        walled_off(
            [moved_at[0], moved_at[1]],
            [them_at[0], them_at[1]],
            spatial.map,
            |x, y| {
                spatial.point_clear(
                    id,
                    [x as f32, y as f32],
                    stats.radius[0],
                    stats.restriction_mask,
                )
            },
            |x, y| {
                let mut n = 0;
                for (_, o) in spatial.objects_on(x, y) {
                    let m = masters.get(o.player).and_then(|m| m.get(o.master)).ok_or(
                        GenieError::unsupported("an object on a tile of a master the slice lacks"),
                    )?;
                    n += usize::from(m.kind == BUILDING);
                }
                Ok(n)
            },
        )
    })
}

/// What an Enter's update reads of its target `t` (of master `tm`, with `room` for `unit`): the
/// [`EnterTarget`] (allied: its own player's; the reach by [`enter_reach`] with the task's
/// `range`), the footprints' gap (`FUN_14118f680`) and the unit's speed for a new `MoveTo`, and
/// for a transport where the two meet ([`meeting_point`]; nothing the update does before its
/// `set_state(4)` moves either, so it holds there).
#[allow(clippy::type_complexity)]
pub(crate) fn enter_view(
    spatial: &crate::spatial::Spatial,
    zones: &mut crate::pathing::zones::Zones,
    masters: &[Masters],
    unit: &crate::turn::Unit,
    (t, tm, room): (&crate::turn::Unit, &Master, bool),
    range: f32,
) -> Result<(EnterTarget, (f32, f32), Result<([f32; 2], u8), GenieError>), GenieError> {
    let building = tm.kind == BUILDING;
    let mover = unit.mover.as_ref();
    let stats = mover.map(|m| m.stats).ok_or(GenieError::unsupported(
        "an Enter's MoveTo of a unit without speed",
    ))?;
    let moved_at = mover.map_or(unit.position, |m| m.moved_at);
    let target_mover = if building { None } else { t.mover.as_ref() };
    let in_reach = enter_reach(
        spatial,
        masters,
        (unit.id, &stats, moved_at),
        (t.id, target_mover),
        range,
    )?;
    let (me, them) = (
        spatial.get(unit.id).expect("placed: in reach above"),
        spatial.get(t.id).expect("placed: in reach above"),
    );
    let gap = crate::movement::footprint_gap(me.position, me.radius, them.position, them.radius);
    let meeting = if building {
        Ok(([t.position[0], t.position[1]], 0))
    } else {
        meeting_target(t, tm, room)
            .and_then(|mt| meeting_point(&mt, (unit.id, unit.position, &stats), spatial, zones))
    };
    Ok((
        EnterTarget {
            state: t.life.state,
            allied: true,
            room,
            position: t.position,
            in_reach,
            building,
        },
        (gap, stats.speed),
        meeting,
    ))
}

/// A transport's slot 0x3d8 (`FUN_14114ffc0`) for an Enter: whether `ship` reaches within
/// [`ENTER_NEAR`] of the point's tile (the point truncated), [`crate::movement::Mover::reaches`]
/// from where it last moved to; one without movement state does not.
pub(crate) fn ship_reaches(
    ship: &crate::turn::Unit,
    world: &crate::spatial::Spatial,
    plan: &mut crate::movement::Planning<'_>,
    to: [f32; 3],
) -> Result<bool, GenieError> {
    let Some(m) = ship.mover.as_ref() else {
        return Ok(false);
    };
    let me = crate::movement::Me {
        id: ship.id,
        position: ship.position,
        action_state: ship.top_action().map_or(0, |a| a.state),
        group: ship.group.id,
    };
    let goal = [cvttss2si(to[0]) as f32, cvttss2si(to[1]) as f32];
    m.reaches(&me, world, plan, goal, ENTER_NEAR, ship.life.master_id)
}
