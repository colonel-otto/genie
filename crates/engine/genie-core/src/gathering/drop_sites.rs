//! The drop-site searches (`FUN_1411671e0`): from a unit, or from a point without one.

use crate::GenieError;
use crate::objects::masters::action_type;
use crate::objects::masters::class;
use crate::objects::masters::master_type::ACTION;

use crate::map::spatial::Spatial;
use crate::movement::{Me, Mover, Planning, footprint_gap_squared};
use crate::objects::masters::{Master, Masters};
use crate::objects::unit::Unit;
use crate::world::others::OthersRef;

/// The unit a drop-site search goes from ([`drop_site_search`]).
pub(crate) struct Seeker<'a, 'b> {
    /// The unit, whose top action's target the search reads (slot 0x190).
    pub(crate) unit: &'a Unit,
    pub(crate) me: Me,
    pub(crate) mover: &'a Mover,
    pub(crate) plan: &'a mut Planning<'b>,
    pub(crate) spatial: &'a mut Spatial,
    pub(crate) zones: &'a crate::pathing::zones::ZoneMap,
    /// Its master's class.
    pub(crate) class: i16,
    /// The nearest is by path (`FUN_1411671e0`'s argument 9: a human player's); else by the
    /// squared distance of the centres, of the objects the planner reaches.
    pub(crate) by_path: bool,
}

/// The point of a search from no point (`DAT_14330ec20`): at or below it on either axis the
/// search has no zone and goes by distance; at it on both axes the first drop site wins.
pub(crate) const NO_POINT: f32 = crate::math::image::turn::DROP_NO_POINT;

impl Seeker<'_, '_> {
    /// A fishing ship ([`class::FISHING_SHIP`]) seeks in every zone and counts a drop site its path
    /// does not reach while near it.
    const fn fishing(&self) -> bool {
        // The fishing ship's class (21), whose drop-site search passes over the zones.
        // The villager's class (4), which searches from the farm it works.
        self.class == class::FISHING_SHIP
    }

    /// What `FUN_1411671e0` measures to the drop site `o` (of `master`, its centre `squared`
    /// away), or none when the planner passes it over: the planner's result decides whether it
    /// counts ([`counted`]); by path its path's length, within [`SHORT_DROP_REACH`] the shorter
    /// of that and the short path ([`DROP_REACH`]), kept in `last`; without a planner `last`;
    /// else `squared`.
    ///
    /// # Errors
    ///
    /// The planner's and the short path's.
    fn measure(
        &mut self,
        o: &Unit,
        master: &Master,
        by_path: bool,
        squared: f32,
        last: &mut f32,
    ) -> Result<Option<f32>, GenieError> {
        let radii = self.mover.stats.radius[0] + master.radius[0];
        let to = [o.position()[0], o.position()[1]];
        let ground = [self.mover.stats.radius[0], self.mover.stats.radius[1]];
        let near = || {
            footprint_gap_squared(self.mover.position(), ground, o.position(), master.radius)
                <= FISHING_REACH
        };
        let planned = self.mover.plan_length(self.spatial, self.plan, to, radii)?;
        Ok(match planned {
            Some((code, length)) if counted(code, by_path, self.fishing(), near) => {
                if !by_path {
                    return Ok(Some(squared));
                }
                *last = length;
                if length <= SHORT_DROP_REACH
                    && let Some(short) =
                        self.mover
                            .short_reach(&self.me, self.spatial, o.id, DROP_REACH)?
                    && short < length
                {
                    *last = short;
                }
                Some(*last)
            }
            // the planner's other results pass the object over
            Some(_) => None,
            None if by_path => Some(*last),
            None => Some(squared),
        })
    }

    /// The place `FUN_1411671e0` searches from instead of the point: for a villager, the farm
    /// (class 49) its top action targets (slot 0x190), if that is one.
    ///
    /// # Errors
    ///
    /// The villager's top action is not modelled, or the farm's place is unknown.
    fn farm_worked(&self) -> Result<Option<[f32; 2]>, GenieError> {
        if self.class != class::VILLAGER {
            return Ok(None);
        }
        let Some(target) = self.unit.top_action_object()? else {
            return Ok(None);
        };
        Ok(self
            .spatial
            .get_known(target)?
            .filter(|p| p.class == class::FARM)
            .map(|p| [p.position[0], p.position[1]]))
    }
}
/// The squared footprint gap within which a fishing ship counts a drop site its path does not
/// reach (`DAT_14330ebcc`).
pub(crate) const FISHING_REACH: f32 = crate::math::image::turn::FISHING_REACH;

/// Whether the planner's result `code` lets a drop site count in [`drop_site_search`]: a path
/// to it or into its range (below 2), or for a fishing ship seeking by path one the planner
/// does not reach (3 and up) whose footprint lies `near` ([`FISHING_REACH`]).
pub(crate) fn counted(code: u8, by_path: bool, fishing: bool, near: impl FnOnce() -> bool) -> bool {
    code < 2 || (by_path && fishing && 3 <= code && near())
}

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
/// working a farm measures from the farm ([`Seeker::farm_worked`]), by the squared distance. The
/// point [`NO_POINT`] on both axes takes the first object of a master in `sites` in state 2; a
/// point at or below it on either axis has no zone and goes by the squared distance. The first
/// of equals wins.
pub(crate) fn drop_site_search<'u>(
    units: impl IntoIterator<Item = &'u Unit>,
    masters: &Masters,
    sites: &[i16],
    at: [f32; 3],
    mut seeker: Option<Seeker<'_, '_>>,
    (outside, at_work): (bool, i16),
) -> Result<Option<i32>, GenieError> {
    let [mut x, mut y, _] = at;
    let pointless = x <= NO_POINT || y <= NO_POINT;
    let fishing = seeker.as_ref().is_some_and(Seeker::fishing);
    let own_zone = seeker
        .as_ref()
        .filter(|_| !fishing && !pointless)
        .map(|s| s.zones.zone(x as i32, y as i32));
    let mut by_path = seeker.as_ref().is_some_and(|s| s.by_path) && !pointless;
    let mut best: Option<i32> = None;
    let mut best_d = NO_DROP_DISTANCE;
    let mut last = NO_DROP_DISTANCE;
    for o in units {
        if !sites.contains(&o.life.master_id) || o.life.state != 2 {
            continue;
        }
        if x == NO_POINT && y == NO_POINT {
            return Ok(Some(o.id));
        }
        let master = o.master_in(Some(masters))?;
        if let (Some(s), Some(zone)) = (seeker.as_ref(), own_zone)
            && s.zones.zone(o.position()[0] as i32, o.position()[1] as i32) != zone
        {
            continue;
        }
        let busy = at_work != -1
            && ACTION <= master.kind
            && o.top_action_kind().is_some_and(|k| {
                // The action types the Gather's drop-site searches pass over (`FUN_1411671e0`'s argument 8
                // `0x7d`, which takes `0x7c` with it).
                k == at_work || (at_work == action_type::UNPACK && k == action_type::PACK)
            });
        if busy || (outside && o.life.inside.is_some()) {
            continue;
        }
        if let Some(farm) = seeker
            .as_ref()
            .map(Seeker::farm_worked)
            .transpose()?
            .flatten()
        {
            [x, y] = farm;
            by_path = false;
        }
        let (dx, dy) = (x - o.position()[0], y - o.position()[1]);
        let squared = dy * dy + dx * dx;
        let d = match seeker.as_mut() {
            Some(s) => match s.measure(o, master, by_path, squared, &mut last)? {
                Some(d) => d,
                None => continue,
            },
            None => squared,
        };
        if d < best_d {
            best = Some(o.id);
            best_d = d;
        }
    }
    Ok(best)
}

/// A drop site within this path length is measured by the short path too (`FUN_1411671e0`,
/// `DAT_142eb0f4c`).
pub(crate) const SHORT_DROP_REACH: f32 = crate::math::image::turn::SHORT_DROP_REACH;
/// The range the short path measures a drop site within (`DAT_142eb5690`).
pub(crate) const DROP_REACH: f32 = crate::math::image::turn::DROP_REACH;
/// The distance a drop-site search starts from (`DAT_14330ebf4`, the largest float).
pub(crate) const NO_DROP_DISTANCE: f32 = crate::math::image::turn::NO_DROP_DISTANCE;

/// `FUN_1411671e0` with a point and no unit: of the player's live objects (`others`, in list
/// order) of a master in `sites` and in state 2, the one nearest `at` (its centre's squared
/// distance, the first of equals), with that distance.
pub(crate) fn nearest_drop_site(
    others: OthersRef<'_>,
    sites: &[i16],
    at: [f32; 3],
) -> Option<(i32, f32)> {
    let mut best: Option<(i32, f32)> = None;
    for o in others.own_iter() {
        if !sites.contains(&o.life.master_id) || o.life.state != 2 {
            continue;
        }
        let [x, y, _] = at;
        let (dx, dy) = (x - o.position()[0], y - o.position()[1]);
        let d = dy * dy + dx * dx;
        if best.is_none_or(|(_, b)| d < b) {
            best = Some((o.id, d));
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::construction::build::{BuildAction, BuildTask};
    use crate::map::spatial::Placed;
    use crate::math::rng::Well512;
    use crate::objects::actions::ActionList;
    use crate::objects::lifecycle::Lifecycle;
    use crate::objects::masters::Master;
    use crate::pathing::Pathing;
    use crate::pathing::zones::ZoneMap;

    /// The town centre (master 109) and the mill (master 68), the villager's drop sites.
    const SITES: [i16; 2] = [109, 68];

    fn object(id: i32, master_id: i16, at: [f32; 2]) -> Unit {
        Unit {
            id,
            body: crate::objects::layers::Body::at([at[0], at[1], 0.0]),
            life: Lifecycle {
                state: 2,
                master_id,
                ..Lifecycle::default()
            },
            ..Unit::default()
        }
    }

    /// The player's town centre 700 at (20, 14) and mill 701 at (20, 25), in that order.
    fn sites() -> Vec<Unit> {
        vec![
            object(700, 109, [20.0, 14.0]),
            object(701, 68, [20.0, 25.0]),
        ]
    }

    fn masters() -> Masters {
        let master = |id, class, radius| Master {
            id,
            kind: 80,
            class,
            radius: [radius, radius],
            ..Master::default()
        };
        Masters::new([
            master(109, 3, 2.0),
            master(68, 3, 1.0),
            master(50, class::FARM, 1.5),
        ])
    }

    /// The villager 10 at (20, 20), building the object `target` when there is one.
    fn villager(target: Option<i32>) -> Unit {
        let task = BuildTask {
            range: 0.1,
            work_value: 1.0,
            work_rate: 1.0,
            resource: -1,
            id: -1,
            moving: None,
        };
        Unit {
            actions: ActionList::default()
                .with_build(target.map(|t| BuildAction::start(t, [20.0, 17.0, 0.0], 0.0, task))),
            ..object(10, 83, [20.0, 20.0])
        }
    }

    /// The map's objects: a farm 800 (class 49) at `farm` and a house 801 (class 3) at (20, 17).
    fn spatial(farm: [f32; 2]) -> Spatial {
        let placed = |at: [f32; 2], class| Placed {
            position: [at[0], at[1], 0.0],
            radius: [1.5, 1.5],
            class,
            player: 1,
            master: 50,
            obstruction_type: 0,
            buildable_on: 0,
            tall: false,
            speed: 0.0,
            unknown: None,
            obstruction: None,
        };
        let mut s = Spatial::default();
        s.insert(800, placed(farm, class::FARM));
        s.insert(801, placed([20.0, 17.0], 3));
        s
    }

    /// The search over `units` from `at` with `unit` (of `class`) as the seeker by path, on an open
    /// map without planners: by path, no object gets a length.
    fn search(
        units: &[Unit],
        unit: &Unit,
        class: i16,
        at: [f32; 3],
        spatial: &Spatial,
    ) -> Result<Option<i32>, GenieError> {
        let mover = crate::gathering::hunt::tests::walker();
        // the search writes the obstruction manager's scratch: it runs on the fixture's copy
        let mut spatial = spatial.clone();
        let zones = ZoneMap::new(40, 40, &[0; 1600], &[1.0]);
        let mut pathing = Pathing::default();
        let mut object = Well512::default();
        let groups = crate::groups::UnitGroups::default();
        let mut plan = Planning {
            pathing: &mut pathing,
            now: 0,
            computer: false,
            object: &mut object,
            construction_rate: 1.0,
            groups: &groups,
        };
        let seeker = Seeker {
            unit,
            me: Me {
                id: unit.id,
                action_state: 0,
                group: -1,
            },
            mover: &mover,
            plan: &mut plan,
            spatial: &mut spatial,
            zones: &zones,
            class,
            by_path: true,
        };
        drop_site_search(units, &masters(), &SITES, at, Some(seeker), (false, -1))
    }

    /// `FUN_1411671e0`'s farm (slot 0x190 of a class-4 seeker on a class-49 object): a villager
    /// building or working a farm measures from the farm by the squared distance, not by path.
    /// From the villager the mill is the nearer; from the farm at (20, 17) the town centre; at
    /// (20, 19.5) both lie 5.5 away and the first in the list wins (428339149's farmers at 420).
    #[test]
    fn a_villager_working_a_farm_searches_from_the_farm_by_distance() {
        let at = [20.0, 20.0, 0.0];
        let near_tc = spatial([20.0, 17.0]);
        // by path without planners: no length, no site
        assert_eq!(
            search(&sites(), &villager(None), class::VILLAGER, at, &near_tc),
            Ok(None)
        );
        assert_eq!(
            search(
                &sites(),
                &villager(Some(800)),
                class::VILLAGER,
                at,
                &near_tc
            ),
            Ok(Some(700))
        );
        let tie = spatial([20.0, 19.5]);
        assert_eq!(
            search(&sites(), &villager(Some(800)), class::VILLAGER, at, &tie),
            Ok(Some(700))
        );
        let reversed: Vec<Unit> = sites().into_iter().rev().collect();
        assert_eq!(
            search(&reversed, &villager(Some(800)), class::VILLAGER, at, &tie),
            Ok(Some(701))
        );
        // another class, or a target of another class: by path as before
        assert_eq!(
            search(&sites(), &villager(Some(800)), 13, at, &near_tc),
            Ok(None)
        );
        assert_eq!(
            search(
                &sites(),
                &villager(Some(801)),
                class::VILLAGER,
                at,
                &near_tc
            ),
            Ok(None)
        );
        // a farm the map does not hold: none
        assert_eq!(
            search(
                &sites(),
                &villager(Some(802)),
                class::VILLAGER,
                at,
                &near_tc
            ),
            Ok(None)
        );
        let mut unmodelled = villager(None);
        unmodelled.unmodelled_action = Some(GenieError::unsupported("an action"));
        assert!(search(&sites(), &unmodelled, class::VILLAGER, at, &near_tc).is_err());
    }

    /// The point (−1, −1) takes the first object of the masters in state 2, before the zones and
    /// the work filters; a point at or below −1 on one axis has no zone and goes by the squared
    /// distance.
    #[test]
    fn a_search_from_no_point_takes_the_first_site_and_one_below_it_goes_by_distance() {
        let map = spatial([20.0, 17.0]);
        let none = [NO_POINT, NO_POINT, 0.0];
        assert_eq!(
            search(&sites(), &villager(None), class::VILLAGER, none, &map),
            Ok(Some(700))
        );
        let mut units = sites();
        units[0].life.state = 0;
        assert_eq!(
            search(&units, &villager(None), class::VILLAGER, none, &map),
            Ok(Some(701))
        );
        // (−2, 30): by distance, the mill (22² + 5²) before the town centre (22² + 16²)
        let below = [-2.0, 30.0, 0.0];
        assert_eq!(
            search(&sites(), &villager(None), class::VILLAGER, below, &map),
            Ok(Some(701))
        );
        let left = [5.0, NO_POINT, 0.0];
        assert_eq!(
            search(&sites(), &villager(None), class::VILLAGER, left, &map),
            Ok(Some(700))
        );
    }
}
