//! The drop-site searches (`FUN_1411671e0`): from a unit, or from a point without one.

use crate::masters::Masters;
use crate::movement::{Me, Mover, Planning, footprint_gap_squared};
use crate::spatial::Spatial;
use crate::turn::unit::Unit;
use crate::unit_ai::GenieError;

/// The unit a drop-site search goes from ([`drop_site_search`]).
pub(crate) struct Seeker<'a, 'b> {
    pub(in crate::turn) me: Me,
    pub(in crate::turn) mover: &'a Mover,
    pub(in crate::turn) plan: &'a mut Planning<'b>,
    pub(in crate::turn) spatial: &'a Spatial,
    pub(in crate::turn) zones: &'a crate::pathing::zones::ZoneMap,
    /// Its master's class.
    pub(in crate::turn) class: i16,
    /// The nearest is by path (`FUN_1411671e0`'s argument 9: a human player's); else by the
    /// squared distance of the centres, of the objects the planner reaches.
    pub(in crate::turn) by_path: bool,
}

/// The fishing ship's class (21), whose drop-site search passes over the zones.
pub(super) const FISHING_SHIP: i16 = 21;
/// The squared footprint gap within which a fishing ship counts a drop site its path does not
/// reach (`DAT_14330ebcc`).
pub(super) const FISHING_REACH: f32 = crate::math::image::turn::FISHING_REACH;

/// A search for drop sites (`FUN_1411671e0`: in state 2, not at work on the action type `at_work`
/// (its argument 8: an object of type 40 up whose top action is of that type, or `0x7c` too for
/// `0x7d`, as the Gather's `set_state` 8 and 3 call it; −1 none), not inside another when
/// `outside`): of the player's live objects (`units`, in list order) of a
/// master in `sites`, the nearest to `at`. With the `seeker`, only objects in its zone count (its
/// restriction's zone of its tile against each object's tile) and only those the tile planner
/// reaches (its goal or its range; radii the seeker's and the object's, [`Mover::plan_length`]),
/// the nearest by path ([`Seeker::by_path`]): the path's length, within 10 tiles the shorter of
/// it and the seeker's short path (slot 0x3e8 within 0.1, [`Mover::short_reach`]); else by the
/// squared distance of the centres. Without a planner the game keeps the distance it last
/// measured (of an earlier object, else the largest float) by path. Without a seeker, by the
/// squared distance of the centres. A fishing ship (class 21) seeks in every zone, and by path a
/// drop site the planner does not reach (its result 3 or more) counts by its path's length too
/// while the footprints' squared gap (`FUN_14118f8c0`) is at most [`FISHING_REACH`]. A villager
/// tending a pasture (its pasture's place) is not modelled. The first of equals wins.
/// Whether the planner's result `code` lets a drop site count in [`drop_site_search`]: a path
/// to it or into its range (below 2), or for a fishing ship seeking by path one the planner
/// does not reach (3 and up) whose footprint lies `near` ([`FISHING_REACH`]).
pub(in crate::turn) fn counted(
    code: u8,
    by_path: bool,
    fishing: bool,
    near: impl FnOnce() -> bool,
) -> bool {
    code < 2 || (by_path && fishing && 3 <= code && near())
}

pub(crate) fn drop_site_search(
    units: &[Unit],
    masters: &Masters,
    sites: &[i16],
    at: [f32; 3],
    mut seeker: Option<Seeker<'_, '_>>,
    (outside, at_work): (bool, i16),
) -> Result<Option<i32>, GenieError> {
    let [x, y, _] = at;
    let fishing = seeker.as_ref().is_some_and(|s| s.class == FISHING_SHIP);
    let own_zone = seeker
        .as_ref()
        .filter(|_| !fishing)
        .map(|s| s.zones.zone(x as i32, y as i32));
    let mut best: Option<i32> = None;
    let mut best_d = NO_DROP_DISTANCE;
    let mut last = NO_DROP_DISTANCE;
    for o in units {
        if !sites.contains(&o.life.master_id) || o.life.state != 2 {
            continue;
        }
        let master = masters
            .get(o.life.master_id)
            .ok_or(GenieError::unsupported(
                "a drop site of a master the slice does not have",
            ))?;
        if let (Some(s), Some(zone)) = (seeker.as_ref(), own_zone)
            && s.zones.zone(o.position[0] as i32, o.position[1] as i32) != zone
        {
            continue;
        }
        let busy = at_work != -1
            && 40 <= master.kind
            && o.top_action_kind()
                .is_some_and(|k| k == at_work || (at_work == UNPACK && k == PACK));
        if busy || (outside && o.life.inside.is_some()) {
            continue;
        }
        let (dx, dy) = (x - o.position[0], y - o.position[1]);
        let squared = dy * dy + dx * dx;
        let d = if let Some(s) = seeker.as_mut() {
            let radii = s.mover.stats.radius[0] + master.radius[0];
            let to = [o.position[0], o.position[1]];
            let near = || {
                let ground = [s.mover.stats.radius[0], s.mover.stats.radius[1]];
                let gap = footprint_gap_squared(s.me.position, ground, o.position, master.radius);
                gap <= FISHING_REACH
            };
            match s.mover.plan_length(&s.me, s.spatial, s.plan, to, radii)? {
                Some((code, length)) if counted(code, s.by_path, fishing, near) => {
                    if s.by_path {
                        last = length;
                        if length <= SHORT_DROP_REACH
                            && let Some(short) =
                                s.mover.short_reach(&s.me, s.spatial, o.id, DROP_REACH)?
                            && short < length
                        {
                            last = short;
                        }
                        last
                    } else {
                        squared
                    }
                }
                // the planner's other results pass the object over
                Some(_) => continue,
                None if s.by_path => last,
                None => squared,
            }
        } else {
            squared
        };
        if d < best_d {
            best = Some(o.id);
            best_d = d;
        }
    }
    Ok(best)
}

/// The action types the Gather's drop-site searches pass over (`FUN_1411671e0`'s argument 8
/// `0x7d`, which takes `0x7c` with it).
pub(in crate::turn) const UNPACK: i16 = 0x7d;
pub(super) const PACK: i16 = 0x7c;

/// A drop site within this path length is measured by the short path too (`FUN_1411671e0`,
/// `DAT_142eb0f4c`).
pub(in crate::turn) const SHORT_DROP_REACH: f32 = crate::math::image::turn::SHORT_DROP_REACH;
/// The range the short path measures a drop site within (`DAT_142eb5690`).
pub(in crate::turn) const DROP_REACH: f32 = crate::math::image::turn::DROP_REACH;
/// The distance a drop-site search starts from (`DAT_14330ebf4`, the largest float).
pub(in crate::turn) const NO_DROP_DISTANCE: f32 = crate::math::image::turn::NO_DROP_DISTANCE;

/// `FUN_1411671e0` with a point and no unit: of the player's live objects (`others`, in list
/// order) of a master in `sites` and in state 2, the one nearest `at` (its centre's squared
/// distance, the first of equals), with that distance.
pub(in crate::turn) fn nearest_drop_site(
    others: &[Unit],
    sites: &[i16],
    at: [f32; 3],
) -> Option<(i32, f32)> {
    let mut best: Option<(i32, f32)> = None;
    for o in others {
        if !sites.contains(&o.life.master_id) || o.life.state != 2 {
            continue;
        }
        let [x, y, _] = at;
        let (dx, dy) = (x - o.position[0], y - o.position[1]);
        let d = dy * dy + dx * dx;
        if best.is_none_or(|(_, b)| d < b) {
            best = Some((o.id, d));
        }
    }
    best
}
