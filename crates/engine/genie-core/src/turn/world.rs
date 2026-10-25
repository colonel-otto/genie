//! Lookups and releases across the players' objects: victims, drop sites, resources, task
//! targets, deaths and ownership changes.

use crate::damage::Victim;
use crate::masters::{Master, Masters};
use crate::movement::{Me, Mover, Planning, footprint_gap_squared};
use crate::pathing::Pathing;
use crate::rng::Stream;
use crate::spatial::Spatial;
use crate::tasks::{Actor, Diplomacy, DropSite, TaskTarget};
use crate::turn::player::Player;
use crate::turn::unit::{LIVESTOCK, Unit};
use crate::turn::view::Foreign;
use crate::turn::{
    BUILDING_KIND, BuildingDeath, CASTLE, CASTLES, MONASTERIES, MONASTERY, RESTORED_FOUNDATION,
    Turns, WONDER, WONDERS,
};
use crate::unit_ai::{GenieError, Notify, OWNER_CHANGED};

/// The unit a drop-site search goes from ([`drop_site_search`]).
pub(crate) struct Seeker<'a, 'b> {
    pub(in crate::turn) me: Me,
    pub(in crate::turn) mover: &'a Mover,
    pub(in crate::turn) plan: &'a mut Planning<'b>,
    pub(in crate::turn) spatial: &'a Spatial,
    pub(in crate::turn) zones: &'a crate::zones::ZoneMap,
    /// Its master's class.
    pub(in crate::turn) class: i16,
    /// The nearest is by path (`FUN_1411671e0`'s argument 9: a human player's); else by the
    /// squared distance of the centres, of the objects the planner reaches.
    pub(in crate::turn) by_path: bool,
}

/// The fishing ship's class (21), whose drop-site search passes over the zones.
const FISHING_SHIP: i16 = 21;
/// The squared footprint gap within which a fishing ship counts a drop site its path does not
/// reach (`DAT_14330ebcc`).
const FISHING_REACH: f32 = crate::image::turn::FISHING_REACH;

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

/// What `ClosestResourceObject` reads of the world, as the unit's update or its unit AI sees it.
pub(in crate::turn) struct ResourceWorld<'a, 'b> {
    pub(in crate::turn) player: usize,
    pub(in crate::turn) spatial: &'a Spatial,
    pub(in crate::turn) zones: &'a mut crate::zones::Zones,
    pub(in crate::turn) foreign: Foreign<'a>,
    pub(in crate::turn) every: &'a [Masters],
    pub(in crate::turn) masters: Option<&'a Masters>,
    pub(in crate::turn) plan: Planning<'b>,
}

/// Where `ClosestResourceObject` (`FUN_140c49760`) searches from, the unit at `unit`: without a
/// site from `point` (at the unit's height), unless both its parts are exactly −1
/// (`DAT_14330ec20`, `UCOMISS`/`JP`/`JNZ`: a part that is not a number differs); with a site
/// (`has_site`, which replaces the point) and for (−1, −1) from the unit's place.
pub(in crate::turn) fn search_origin(point: [f32; 2], has_site: bool, unit: [f32; 3]) -> [f32; 3] {
    match point {
        [x, y] if !has_site && (x != -1.0 || y != -1.0) => [x, y, unit[2]],
        _ => unit,
    }
}

/// [`crate::gather_search::closest_resource_at`] for `unit` of gather group `kind` with fewer
/// than `cap` workers (−1 any) from the drop site `site` (−1 for none; one the world does not hold
/// counts as none, `FUN_1415417a0`), its unit AI's current target `current` passed over, below
/// state `min_state` nothing found, and without a site from `point` unless that is (−1, −1)
/// (`param_7`, `param_8`; [`crate::unit_ai::NO_POINT`]: the unit's place, as with a site): the
/// site's place, its master's radii and line of sight, and whether the search starts from its
/// footprint (the site's tile in the unit's zone of its restriction, or a dock: masters 45 and
/// 2172, `0x140c49d17`); the answer and the out argument's steps.
pub(in crate::turn) fn closest_resource_of(
    unit: &Unit,
    c: &mut ResourceWorld<'_, '_>,
    others: &[Unit],
    (kind, cap, site): (u8, i32, i32),
    (current, min_state, point): (i32, u8, [f32; 2]),
) -> Result<(i32, Option<i32>), GenieError> {
    use crate::gather_search::{self, Candidate};
    /// The site masters whose search starts from their footprint in any zone.
    const DOCKS: [i16; 2] = [45, 2172];
    let site = if site == -1 || c.spatial.get(site).is_none() {
        None
    } else {
        // the unit itself (a mule cart searches from its own place) or another object
        let (sp, s) = std::iter::once(unit)
            .chain(others.iter())
            .find(|u| u.id == site)
            .map(|u| (c.player, u))
            .or_else(|| c.foreign.find(site))
            .ok_or(GenieError::unsupported(
                "a resource search's drop site the slice does not hold",
            ))?;
        let m = c
            .every
            .get(sp)
            .and_then(|m| m.get(s.life.master_id))
            .ok_or(GenieError::unsupported(
                "a resource search's drop site of a master the slice lacks",
            ))?;
        let footprint = DOCKS.contains(&m.id) || {
            let restriction = unit
                .mover
                .as_ref()
                .ok_or(GenieError::unsupported(
                    "a resource search without movement state",
                ))?
                .stats
                .restriction;
            let [w, h] = c.spatial.map;
            let terrain = c.spatial.terrain.as_ref().ok_or(GenieError::unsupported(
                "the map's zones without its terrain",
            ))?;
            let values = terrain
                .restrictions
                .get(restriction)
                .ok_or(crate::zones::UNKNOWN_RESTRICTION)?;
            let zones = c.zones.of(w, h, &terrain.kinds, values);
            let zone = |p: [f32; 3]| zones.zone(p[0] as i32, p[1] as i32);
            zone(s.position) == zone(unit.position)
        };
        Some(gather_search::SearchSite {
            position: s.position,
            radius: m.radius,
            line_of_sight: m.line_of_sight,
            footprint,
        })
    };
    let masters = c.masters.ok_or(GenieError::unsupported(
        "a resource search without its player's masters",
    ))?;
    let master = masters
        .get(unit.life.master_id)
        .ok_or(GenieError::unsupported(
            "a searcher of a master the slice does not have",
        ))?;
    let mover = unit.mover.as_ref().ok_or(GenieError::unsupported(
        "a resource search without movement state",
    ))?;
    let mask = mover.stats.restriction_mask;
    let (player, me_id, foreign, every) = (c.player, unit.id, c.foreign, c.every);
    let candidate = |id: i32, owner: usize, u: &Unit, position: [f32; 3]| {
        let m = every
            .get(owner)
            .and_then(|m| m.get(u.life.master_id))
            .ok_or(crate::turn::OBJECT_MASTER)?;
        Ok(Candidate {
            id,
            player: owner,
            class: m.class,
            master: m.id,
            state: u.life.state,
            workers: u.workers,
            position,
            pasture: u.pasture,
            untargetable: u.untargetable,
            attribute: u.life.attribute_type,
            amount: u.life.amount,
            auto_gather: m.auto_gather,
            gather_group: m.gather_group,
        })
    };
    let me = candidate(me_id, player, unit, unit.position)?;
    let spatial: &Spatial = c.spatial;
    let objects = |x: i32, y: i32| -> Result<Vec<Candidate>, GenieError> {
        spatial
            .objects_on(x, y)
            .map(|(id, p)| {
                if id == me_id {
                    return Ok(me);
                }
                let (owner, u) = others
                    .iter()
                    .find(|u| u.id == id)
                    .map(|u| (player, u))
                    .or_else(|| foreign.find(id))
                    .ok_or(GenieError::unsupported(
                        "an object on a tile the slice does not hold",
                    ))?;
                candidate(id, owner, u, p.position)
            })
            .collect()
    };
    let position = search_origin(point, site.is_some(), unit.position);
    let seeker = gather_search::Seeker {
        player,
        position,
        line_of_sight: master.line_of_sight,
        state: unit.life.state,
        current_target: current,
        my_pasture: unit.my_pasture,
        min_state,
    };
    let me_walk = Me {
        id: unit.id,
        position: unit.position,
        group: unit.group.id,
        action_state: unit.top_action().map_or(0, |a| a.state),
    };
    let master_id = unit.life.master_id;
    let plan = &mut c.plan;
    let blocked = u64::from(mask ^ 0xff) | crate::spatial::BLOCKED;
    gather_search::closest_resource_at(
        &seeker,
        site.as_ref(),
        kind,
        cap,
        spatial.map,
        &|x, y| spatial.explored(player, x, y),
        &|x, y| spatial.tiles().word(x, y).is_some_and(|w| w & blocked == 0),
        &objects,
        &mut |id| mover.reaches_object(&me_walk, spatial, plan, id, 0.0, master_id),
    )
}

/// The action types the Gather's drop-site searches pass over (`FUN_1411671e0`'s argument 8
/// `0x7d`, which takes `0x7c` with it).
pub(in crate::turn) const UNPACK: i16 = 0x7d;
const PACK: i16 = 0x7c;

/// A drop site within this path length is measured by the short path too (`FUN_1411671e0`,
/// `DAT_142eb0f4c`).
pub(in crate::turn) const SHORT_DROP_REACH: f32 = crate::image::turn::SHORT_DROP_REACH;
/// The range the short path measures a drop site within (`DAT_142eb5690`).
pub(in crate::turn) const DROP_REACH: f32 = crate::image::turn::DROP_REACH;
/// The distance a drop-site search starts from (`DAT_14330ebf4`, the largest float).
pub(in crate::turn) const NO_DROP_DISTANCE: f32 = crate::image::turn::NO_DROP_DISTANCE;

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

/// `ClosestObject`'s "Sheep is moving" (`FUN_140c488b0`): the object's top action is the Artifact
/// (slot 0x530 107) with a point (slot 0x288, the action's `+0x38`, not −1).
pub(in crate::turn) fn herded(others: &[Unit], foreign: Foreign<'_>, id: i32) -> bool {
    let foreign = || foreign.find(id).map(|(_, u)| u);
    others
        .iter()
        .find(|u| u.id == id)
        .or_else(foreign)
        .is_some_and(|u| {
            u.top_action_kind() == Some(crate::artifact::ARTIFACT_TYPE)
                && u.artifact
                    .as_ref()
                    .is_some_and(|a| a.goal.is_some_and(|g| g[0] != -1.0))
        })
}

/// Slot 0x180 (`Turns::release`) on an object of the unit's own live list, from inside the unit's
/// update ([`released`]; the releaser's own player is allied).
pub(in crate::turn) fn release_own(
    others: &mut [Unit],
    masters: Option<&Masters>,
    id: i32,
    by: Releaser,
) -> Result<(), GenieError> {
    let u = others
        .iter_mut()
        .find(|u| u.id == id)
        .ok_or(GenieError::unsupported(
            "a release of an object not in the unit's live list",
        ))?;
    let kind = masters
        .and_then(|m| m.get(u.life.master_id))
        .ok_or(GenieError::unsupported(
            "a released object of a master the slice does not have",
        ))?
        .kind;
    // one of the releaser's own player: allied, never listed
    released(u, kind, false, by)
}

/// The object whose action lets its target go (the target's slot 0x180 takes it): its id, its
/// master's class and its owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Releaser {
    pub(crate) id: i32,
    pub(crate) class: i16,
    pub(crate) player: usize,
}

/// The first master type whose slot 0x180 is not the static object's (`FUN_141197d00`, types 10
/// and 15): the animated object's (`FUN_141071ea0`, types 20 to 40) only counts a worker less.
const ANIMATED_TYPE: u8 = 20;

/// Slot 0x180 on object `u` of master type `kind`, let go by `by`: the static object's
/// (`FUN_141197d00`, types 10 and 15) counts a worker less and a static one without workers
/// left, alive (state 2 with hit points, or 6), goes to sleep; the animated object's
/// (`FUN_141071ea0`, types 20 to 40) only counts the worker less. A combat object's (`FUN_141086e60`) counts the worker
/// less (`FUN_141071ea0`) and tells its unit AI that the releaser no longer acts on it (notify
/// 699, the releaser's master class as its first parameter; none from an object of a negative
/// id). A building's (`FUN_1411d6020`) first drops the releaser's first entry from its attacking
/// units (`+0xce0`, the rest keeping their order) when it lists it (`listed`, [`Unit::lists_taker`]).
pub(crate) fn released(
    u: &mut Unit,
    kind: u8,
    listed: bool,
    by: Releaser,
) -> Result<(), GenieError> {
    if listed && let Some(i) = u.attacking_units.iter().position(|&t| t == by.id) {
        let _ = u.attacking_units.remove(i);
    }
    if 0 < u.workers {
        u.workers -= 1;
    }
    // COMISS hp with 0, JNC: a NaN does not count as alive
    let alive = (u.life.state == 2 && 0.0 <= u.life.hp) || u.life.state == 6;
    if kind < ANIMATED_TYPE && u.workers == 0 && alive {
        u.life.goto_sleep = true;
    }
    if !u.combat {
        return Ok(());
    }
    if 0 <= by.id
        && let Some(ai) = u.ai.as_mut()
    {
        let n = Notify {
            caller: by.id,
            recipient: u.id,
            kind: crate::unit_ai::NO_LONGER_ON_ME,
            p: [i32::from(by.class), 0, 0],
        };
        ai.post(n, u.life.state)?;
    }
    Ok(())
}

impl Turns {
    /// The leave of object `gone`, out of player `p`'s live list already (its list's update that
    /// returned 1, `FUN_14116b5e0`): its player's remove (slot 0x2d8) takes it out of the tracker,
    /// it leaves the visible-resource lists ([`forget_resource`]), a combat object's destructor
    /// lowers its player's class counters (`FUN_141200450`), it leaves the world's places and its
    /// owner's watched list (`FUN_141091310`), a recyclable master's missile goes to the pool of
    /// type 60 (the world's `+0x1d8`), and it leaves the mesh. Every step runs; returns the
    /// world's leave of the place that the slice does not model.
    pub(in crate::turn) fn leave_world(&mut self, p: usize, gone: &Unit) -> Result<(), GenieError> {
        self.track_remove(p, gone.id, gone.life.master_id);
        let master = self.masters.get(p).and_then(|m| m.get(gone.life.master_id));
        forget_resource(&mut self.spatial, master, gone.id, gone.position);
        if let Some(m) = master.filter(|m| 70 <= m.kind) {
            count_class(&mut self.players[p].attributes, m, -1.0);
        }
        let recyclable = master.is_some_and(|m| m.recyclable);
        let left = self.spatial.leave(gone.id);
        self.unwatch(p, gone.id);
        if let Some(m) = gone.missile.as_ref().filter(|_| recyclable) {
            self.recycling.missiles.push((gone.id, m.flying));
        }
        self.pathing.removed(gone.id);
        self.pathing.tiles_changed(self.spatial.take_changes());
        left
    }

    /// `TRIBE_Combat_Object::update`'s regeneration ([`Unit::regenerate`]) of the object `id`, a
    /// combat object or building (types 70 and 80) by its own master, else its player's.
    pub(in crate::turn) fn regenerate_object(&mut self, id: i32, dt_seconds: f32) {
        let Some(crate::order::Place::Live(p, i)) = self.place(id) else {
            return;
        };
        let unit = &self.players[p].objects[i];
        let Some(master) = unit
            .own_master
            .as_deref()
            .or_else(|| self.masters.get(p).and_then(|m| m.get(unit.life.master_id)))
            .filter(|m| matches!(m.kind, 70 | 80))
            .cloned()
        else {
            return;
        };
        self.players[p].objects[i].regenerate(&master, dt_seconds);
    }

    /// `TRIBE_Combat_Object::update`'s hit point powerup check ([`Unit::clamp_hp_powerup`]) of the
    /// object `id`, a combat object or building (types 70 and 80).
    pub(in crate::turn) fn clamp_object_powerup(&mut self, id: i32) {
        let Some(crate::order::Place::Live(p, i)) = self.place(id) else {
            return;
        };
        let unit = &self.players[p].objects[i];
        let combat = unit
            .own_master
            .as_deref()
            .or_else(|| self.masters.get(p).and_then(|m| m.get(unit.life.master_id)))
            .is_some_and(|m| matches!(m.kind, 70 | 80));
        if combat {
            self.players[p].objects[i].clamp_hp_powerup();
        }
    }

    /// What a hit of an attacker of player `attacker` reads of the object `id` ([`Victim`]): an
    /// object the slice holds, live or asleep (the world's table holds both, and a blast walks
    /// the tiles' lists of both), with its relations to the attacker's player and whether its
    /// place is a guess.
    pub(crate) fn victim(&self, id: i32, attacker: usize) -> Option<Victim> {
        let (p, u) = self.players.iter().enumerate().find_map(|(p, player)| {
            player
                .objects
                .iter()
                .chain(&player.sleeping)
                .find(|u| u.id == id)
                .map(|u| (p, u))
        })?;
        let master = self.masters.get(p)?.get(u.life.master_id)?;
        let world = World {
            players: &self.players,
            drop_sites: &self.drop_sites,
        };
        Some(victim_in(
            u,
            (p, master, &self.players[p].attributes),
            (attacker, &world),
            &self.spatial,
            self.mode_680,
        ))
    }

    /// The object `id` as the unit AI of a unit of player `viewer` finds it in the world's object
    /// table (`FUN_1415417a0`, [`crate::unit_ai::Seen`]), `None` when no list holds it. An object
    /// the slice holds as a place alone cannot be read. Without the players' sight its tile
    /// counts as unseen.
    pub(in crate::turn) fn seen(
        &self,
        id: i32,
        viewer: usize,
    ) -> Result<Option<crate::unit_ai::Seen>, GenieError> {
        for (p, player) in self.players.iter().enumerate() {
            if let Some(u) = player
                .objects
                .iter()
                .chain(&player.sleeping)
                .find(|u| u.id == id)
            {
                let master = self
                    .masters
                    .get(p)
                    .and_then(|m| m.get(u.life.master_id))
                    .ok_or(GenieError::unsupported(
                        "a unit AI's target of a master the slice does not have",
                    ))?;
                let [x, y, _] = u.position;
                return Ok(Some(crate::unit_ai::Seen {
                    state: u.life.state,
                    kind: master.kind,
                    owner: p,
                    class: master.class,
                    position: u.position,
                    radius: master.radius,
                    visible: self.spatial.visible(viewer, x as i32, y as i32),
                    flags: master.object_flags,
                    sight: self
                        .spatial
                        .tile_sight(
                            viewer,
                            crate::checkpoint::cvttss2si(x),
                            crate::checkpoint::cvttss2si(y),
                        )
                        .unwrap_or(0),
                }));
            }
        }
        match self.spatial.get(id) {
            Some(_) => Err(GenieError::unsupported(
                "a unit AI's target the slice cannot read",
            )),
            None => Ok(None),
        }
    }

    /// The objects besides `unit` (player `p`) and its `target` that its hit may reach: for a
    /// master with an area effect, the objects on the tiles of the blast's rectangle about the
    /// unit, a tile wider each way (the unit may step in its update), each with its place in its
    /// tile's list as the unit's update starts (an object the slice cannot read stands in with
    /// its place alone, as a guess); then the main buildings (`+0xbb8`) of the target and of the
    /// annexes among them, which the hits on those annexes go to.
    pub(in crate::turn) fn blasted(&self, unit: &Unit, p: usize, target: i32) -> Vec<Victim> {
        let reach = self
            .masters
            .get(p)
            .and_then(|m| m.get(unit.life.master_id))
            .map(|m| m.area_effect)
            .filter(|&r| 0.0 < r)
            .map(|r| r + 6.0 + 2.0);
        let mut near: Vec<Victim> = reach.map_or_else(Vec::new, |reach| {
            let [x, y, _] = unit.position;
            self.spatial
                .objects_within(x - reach, y - reach, x + reach, y + reach)
                .filter(|&(id, _)| id != unit.id && id != target)
                .map(|(id, o)| {
                    self.victim(id, p).unwrap_or_else(|| Victim {
                        id,
                        position: o.position,
                        uncertain: true,
                        tile_place: self.spatial.tile_place(id),
                        ..Victim::default()
                    })
                })
                .collect()
        });
        let mains: Vec<i32> = std::iter::once(target)
            .chain(near.iter().map(|v| v.id))
            .filter_map(|id| self.spatial.annex_of(id))
            .collect();
        for m in mains {
            if m != unit.id
                && m != target
                && !near.iter().any(|v| v.id == m)
                && let Some(v) = self.victim(m, p)
            {
                near.push(v);
            }
        }
        near
    }

    /// Slot 0x5f0 (`FUN_141158b90`): the unit's walked level, if it has one, pauses for `pause`
    /// seconds; the world's view of it follows.
    pub(crate) fn pause(&mut self, id: i32, pause: f32) {
        let Some(u) = self
            .players
            .iter_mut()
            .flat_map(|p| p.objects.iter_mut())
            .find(|u| u.id == id)
        else {
            return;
        };
        if let Some(level) = u.mover.as_mut().and_then(|m| m.paths.first_mut()) {
            level.pause_time = pause;
            let gait = u.gait();
            self.spatial.set_gait(id, gait);
        }
    }

    /// An attack's hit on its target, written back: its hit points, what it holds, its charge
    /// and powerups, its damage notify (500), its kill ([`Turns::killed`]); a struck building's
    /// `damaged_lately_timer` and its annexes' hit points (`+0xbc0`..), then each razing
    /// ([`crate::damage::Razed`]: the history event of the attacker's player, the attribute adds
    /// in their order).
    pub(in crate::turn) fn set_victim(&mut self, v: &Victim) -> Result<(), GenieError> {
        let (q, j) = match self.place(v.id) {
            Some(crate::order::Place::Live(q, j)) => (q, j),
            // an object asleep (a tree a blast reached) holds what the hit left; below 1 hit
            // point (COMISS/JBE: a NaN sleeps on) the hit woke it (FUN_14119deb0 with 0) to the
            // end of its player's live list, where the rest is written back
            Some(crate::order::Place::Sleeping(q, j)) => {
                let u = &mut self.players[q].sleeping[j];
                u.life.hp = v.hp;
                u.life.amount = v.amount;
                if crate::fmath::at_most_or_unordered(1.0, v.hp) {
                    return Ok(());
                }
                let j = self.wake(v.id);
                (q, j)
            }
            None => return Ok(()),
        };
        let u = &mut self.players[q].objects[j];
        u.life.hp = v.hp;
        u.life.amount = v.amount;
        u.life.goto_sleep = v.goto_sleep;
        // a hunt that stopped its decay gave it a master of its own (FUN_140ac44b0)
        if let Some(m) = u.life.master.as_mut() {
            m.decay = v.decay;
        }
        if let Some(a) = v.armored.as_ref() {
            u.hp_powerups = a.hp_powerups;
        }
        // a shield spends its charge
        u.charge = v.source.charge;
        let struck = v.building.as_ref().filter(|b| b.struck);
        if let Some(b) = struck {
            u.damaged_lately = b.damaged_lately;
        }
        // the damage notify (500) to its unit AI
        if let Some(ai) = u.ai.as_mut() {
            for &(attacker, p) in &v.damaged {
                let n = Notify {
                    caller: attacker,
                    recipient: v.id,
                    kind: crate::unit_ai::DAMAGED,
                    p,
                };
                ai.post(n, u.life.state)?;
            }
        }
        for &a in struck.map_or(&[][..], |b| &b.annexes) {
            if let Some(x) = self
                .players
                .iter_mut()
                .flat_map(|p| p.objects.iter_mut())
                .find(|x| x.id == a)
            {
                x.life.hp = v.hp;
            }
        }
        if let Some(kill) = v.killed {
            self.killed(q, j, kill)?;
        }
        for r in v.building.iter().flat_map(|b| &b.razed) {
            if let Some(kind) = r.event {
                let now = self.world_time;
                self.players[r.attacker_player].history.add_event(
                    kind,
                    r.owner as f32,
                    0.0,
                    0,
                    now,
                );
            }
            for &(p, attribute, amount) in &r.adds {
                self.players[p].add_attribute(attribute, amount)?;
            }
        }
        Ok(())
    }

    /// A combat object's death by a hit (`FUN_141082e80`, `FUN_1412108e0`): its owner's computer
    /// AI is told (slot 0x2f0, not modelled); the killed handling (slot 0x8b0, `FUN_141205880` ->
    /// `FUN_1412005b0`): unless its class is one of the non-military ones the attacker's player
    /// counts a kill (attribute 20) and its owner a loss (154); the attacker's player counts it
    /// against the owner (300 + the owner) and the owner against the attacker (325 + the
    /// attacker's player); both count its paid cost (170 and 152, and 400 + the owner). A killed
    /// villager first counts for its owner (`FUN_141082e80`): an attacker of class 9 or 10 (an
    /// animal) in attribute 227, and by the attacker's player type a human's in 229, Gaia's in
    /// 226, a computer's in 228. Gaia takes a player's livestock (slot 0x70): its place and sight, its Artifact's
    /// wait, its unit AI's notifies (cleared, then 0x20b), its storage (the old owner loses flags 2,
    /// 16 and 32, Gaia gains 1, 2 and 32) and its list (the old owner's last object takes its
    /// place). Not held: the killer (`+0x690`, `+0x694`), `+0x954`, the statistics of the local
    /// player and the particles (`FUN_141158bb0`).
    pub(in crate::turn) fn killed(
        &mut self,
        q: usize,
        j: usize,
        kill: crate::damage::Kill,
    ) -> Result<(), GenieError> {
        if self.players[q].computer {
            return Err(GenieError::unsupported(
                "a computer player's killed object (its AI's slot 0x2f0)",
            ));
        }
        let u = &self.players[q].objects[j];
        let master = self
            .masters
            .get(q)
            .and_then(|m| m.get(u.life.master_id))
            .ok_or(GenieError::unsupported(
                "a killed object of a master the slice does not have",
            ))?
            .clone();
        let ap = kill.attacker_player;
        if master.class == 4 {
            let (bp, by) = self.object(kill.attacker).ok_or(GenieError::unsupported(
                "a killed villager's attacker the slice does not hold",
            ))?;
            if matches!(self.master_of(bp, by.life.master_id)?.class, 9 | 10) {
                self.players[q].add_attribute(227, 1.0)?;
            }
            let by_kind = match self.players[ap].kind {
                1 => Some(229),
                2 => Some(226),
                3 => Some(228),
                _ => None,
            };
            if let Some(kind) = by_kind {
                self.players[q].add_attribute(kind, 1.0)?;
            }
        }
        self.count_kill_of(q, &master, ap)?;
        if kill.to_gaia {
            self.give_gaia(q, j)?;
        }
        Ok(())
    }

    /// `FUN_1412005b0` on the object `id` killed for player `killer` (a movement damage's,
    /// [`crate::abilities`]): the counts of [`Turns::killed`] by its master (its own one, else
    /// its player's).
    ///
    /// # Errors
    ///
    /// An object the slice does not hold, and an attribute whose change the slice does not
    /// model.
    pub(crate) fn count_kill(&mut self, id: i32, killer: usize) -> Result<(), GenieError> {
        let (q, u) = self.object(id).ok_or(crate::build::NOT_HELD)?;
        let master = u
            .own_master
            .as_deref()
            .or_else(|| self.masters.get(q).and_then(|m| m.get(u.life.master_id)))
            .ok_or(crate::build::NOT_HELD)?
            .clone();
        self.count_kill_of(q, &master, killer)
    }

    /// `FUN_1412005b0`'s counts of a kill of an object of player `q` and `master` for player `ap`:
    /// unless its class is one of the non-military ones (`FUN_141188b20`) `ap` counts a kill
    /// (attribute 20) and `q` a loss (154); `ap` counts it against `q` (300 + `q`) and `q`
    /// against `ap` (325 + `ap`); both count its paid cost (170 and 152, and 400 + `q`).
    fn count_kill_of(&mut self, q: usize, master: &Master, ap: usize) -> Result<(), GenieError> {
        const PEACEFUL: [i16; 26] = [
            1, 3, 5, 7, 8, 9, 10, 11, 14, 15, 16, 27, 29, 30, 31, 32, 33, 34, 38, 39, 40, 48, 52,
            58, 60, 63,
        ];
        if !PEACEFUL.contains(&master.class) {
            self.players[ap].add_attribute(20, 1.0)?;
            self.players[q].add_attribute(154, 1.0)?;
        }
        self.players[ap].add_attribute(q as i16 + 300, 1.0)?;
        self.players[q].add_attribute(ap as i16 + 325, 1.0)?;
        let cost = paid_cost(master);
        self.players[ap].add_attribute(170, cost)?;
        self.players[q].add_attribute(152, cost)?;
        self.players[ap].add_attribute(q as i16 + 400, cost)?;
        Ok(())
    }

    /// Slot 0x70 ([`change_hands`]) to Gaia of the object at `j` of player `q`, from outside its
    /// update: once Gaia has its master it leaves `q`'s list (`FUN_141630930`, the last object
    /// in its place) for the end of Gaia's (`FUN_14167f670` -> `FUN_141616200`) with its
    /// attributes ([`Turns::hand_over_attributes`]). Its only callers give livestock, which
    /// Gaia's add does not take into its list of class 10 of combat level 4 (`+0x14a0`, a draw
    /// on the Player stream).
    pub(in crate::turn) fn give_gaia(&mut self, q: usize, j: usize) -> Result<(), GenieError> {
        let mut unit = std::mem::replace(&mut self.players[q].objects[j], Unit::taken_out());
        let changed = change_hands(
            &mut unit,
            (q, 0),
            &self.masters,
            &mut self.spatial,
            self.world_time,
        );
        self.players[q].objects[j] = unit;
        if !changed? {
            return Ok(());
        }
        let unit = self.players[q].objects.swap_remove(j);
        let handed = self.hand_over_attributes(q, 0, &unit);
        // the old owner's remove and Gaia's add (slots 0x2d8, 0x2d0) move it between the
        // trackers; a building whose build points reach its build time completes in Gaia's
        // (`FUN_1411cd860`, `COMISS`/`JC`: a NaN does not)
        let (id, master_id) = (unit.id, unit.life.master_id);
        let built = self
            .masters
            .first()
            .and_then(|m| m.get(master_id))
            .is_some_and(|m| m.kind == BUILDING_KIND)
            && unit
                .construction
                .is_some_and(|c| f32::from(c.build_time) <= c.progress);
        self.players[0].objects.push(unit);
        self.track_remove(q, id, master_id);
        self.track_add(0, id, master_id);
        if built {
            self.track_complete(0, id, master_id);
        }
        handed
    }

    /// A building's slot 0x70 (`FUN_1411cd860`) from player `old` to `new`, of the building `id`
    /// in `old`'s live list (from its own update, after it: [`Step::Captured`]'s `building`). In
    /// order:
    ///
    /// - its annexes (`+0xbc0`..), each a building: one its old owner watches leaves the watched
    ///   list (`FUN_141091310`, `+0xca4` 0), then the annex's own slot 0x70, depth first;
    /// - below state 3 its record and mesh shape go (`FUN_141055b40`);
    /// - in state 2 the old owner loses its master's storage of flag 4 and a held attribute 4
    ///   (slot 0xf8, the amount's sign flipped, `DAT_142e88e80`);
    /// - a foundation's (state 0) footprint leaves the old owner's map (slot 0x1d8);
    /// - its action list and production list go (`FUN_140a98620`, [`Turns::clear_actions`],
    ///   [`Turns::end_production_action`]), then its whole queue ([`Turns::cancel_queue`]);
    /// - a watched one leaves the old owner's watched list;
    /// - the combat object's slot 0x70 (`FUN_14120f800`, [`change_hands`]; the lists' moves and the
    ///   attributes as [`Turns::give_gaia`] does them), unless the new owner lacks its master;
    /// - built (its build time at most its build points: `COMISS`, a NaN is not), it completes in
    ///   the new owner's tracker (`FUN_14104eac0`);
    /// - a watched one joins the new owner's watched list at its tile (`FUN_141090430`);
    /// - a foundation's footprint comes on the new owner's map (slot 0x1d0);
    /// - in state 2 its master's stack unit (`+0x986`; 1889's on rice terrain 1893) comes at its
    ///   place (the master's slot 0x40, [`Turns::remains`]) and its master's technology is
    ///   researched ([`Turns::research_built`]); then its owner gains the storage of flags 4 and 8
    ///   and a held attribute 4;
    /// - the units inside leave it toward no point, from the last ([`Turns::let_out`] with −1, −1,
    ///   `DAT_14330ec20`); one that finds no spot leaves and is destroyed (slots 0x1a8, 0xd0);
    /// - below state 3 its record and mesh shape come again (`FUN_141056460`);
    /// - its gather point goes (`+0xc30`..`+0xc44`).
    ///
    /// Not held: the head unit's sprite (`+0xc90`, the drawing's), the semi-asleep lists
    /// (`+0xc98`, `FUN_14107db90`, `FUN_14107bc80`) and a computer player's AI memory of it
    /// (`+0x7a8`, `FUN_14160b900`; a playback runs no AI).
    ///
    /// # Errors
    ///
    /// A building asleep ([`crate::turn::unit::HANDS_LEFT`]), and what the parts do not model.
    pub(in crate::turn) fn building_changes_hands(
        &mut self,
        old: usize,
        id: i32,
        new: usize,
        watched: bool,
    ) -> Result<(), GenieError> {
        for annex in self.spatial.annexes_of(id) {
            let Some((q, a)) = self.object(annex) else {
                continue;
            };
            if self.master_of(q, a.life.master_id)?.kind != BUILDING_KIND {
                continue;
            }
            let annex_watched = self.players[old].watched.iter().any(|w| w.id == annex);
            if annex_watched {
                self.unwatch(old, annex);
            }
            self.building_changes_hands(q, annex, new, false)?;
        }
        let Some(crate::order::Place::Live(_, idx)) = self.place(id) else {
            return Err(crate::turn::unit::HANDS_LEFT);
        };
        let u = &self.players[old].objects[idx];
        let master = match u.own_master.as_deref() {
            Some(m) => m.clone(),
            None => self.master_of(old, u.life.master_id)?.clone(),
        };
        let (state, held, amount) = (u.life.state, u.life.attribute_type, u.life.amount);
        let built = u
            .construction
            .is_some_and(|c| f32::from(c.build_time) <= c.progress);
        let rice = u.rice;
        let placed = self.spatial.get(id).copied();
        if state < 3 {
            self.spatial.drop_record(id);
            self.pathing.removed(id);
            self.pathing.tiles_changed(self.spatial.take_changes());
        }
        if state == 2 {
            for &(kind, amount, flag) in &master.storage {
                if flag == STORAGE_WHILE_BUILT && 0 <= kind {
                    self.players[old].add_attribute(kind, -amount)?;
                }
            }
            if held == POPULATION_HEADROOM {
                self.players[old].add_attribute(POPULATION_HEADROOM, -amount)?;
            }
        }
        if state == 0
            && let (Some(p), Some(v)) = (placed, self.spatial.sight.as_mut())
        {
            v.sight_off(id, p.position, p.radius, true, false);
        }
        self.clear_actions(old, idx)?;
        self.end_production_action(old, idx)?;
        self.cancel_queue(old, idx)?;
        if watched {
            self.unwatch(old, id);
        }
        let mut unit = std::mem::replace(&mut self.players[old].objects[idx], Unit::taken_out());
        let changed = change_hands(
            &mut unit,
            (old, new),
            &self.masters,
            &mut self.spatial,
            self.world_time,
        );
        self.players[old].objects[idx] = unit;
        let owner = if changed? {
            let unit = self.players[old].objects.swap_remove(idx);
            let handed = self.hand_over_attributes(old, new, &unit);
            let master_id = unit.life.master_id;
            self.players[new].objects.push(unit);
            self.track_remove(old, id, master_id);
            self.track_add(new, id, master_id);
            handed?;
            new
        } else {
            old
        };
        if built {
            self.track_complete(new, id, master.id);
        }
        if watched && let Some(p) = placed {
            let _ = self.watch(
                new,
                id,
                crate::checkpoint::cvttss2si(p.position[0]),
                crate::checkpoint::cvttss2si(p.position[1]),
            )?;
        }
        if state == 0
            && let (Some(p), Some(v)) = (placed, self.spatial.sight.as_mut())
        {
            let _ = v.sight_on(id, p.position, p.radius, true, false);
        }
        if state == 2 {
            let building = master.building.as_ref();
            if let Some(stack) = building.map(|b| b.stack_unit).filter(|&s| 0 <= s) {
                let stack = if master.id == crate::build::MASTER_1889 && rice {
                    crate::build::RICE_PASTURE
                } else {
                    stack
                };
                let at = self.object(id).map_or([0.0; 3], |(_, u)| u.position);
                self.remains(owner, stack, at, None)?;
            }
            if let Some(tech) = building.map(|b| b.tech).filter(|&t| 0 <= t) {
                self.research_built(owner, tech)?;
            }
            for &(kind, amount, flag) in &master.storage {
                if matches!(flag, STORAGE_WHILE_BUILT | STORAGE_8) && 0 <= kind {
                    self.players[owner].add_attribute(kind, amount)?;
                }
            }
            if held == POPULATION_HEADROOM {
                self.players[owner].add_attribute(POPULATION_HEADROOM, amount)?;
            }
        }
        let inside = self
            .object(id)
            .map_or_else(Vec::new, |(_, u)| u.garrison.clone());
        let mut stayed = Vec::new();
        for &g in inside.iter().rev() {
            let Some(crate::order::Place::Live(_, bi)) = self.place(id) else {
                break;
            };
            if !self.let_out(owner, bi, g, [HANDOVER_NO_POINT; 2], (None, true))? {
                stayed.push(g);
            }
        }
        for g in stayed {
            if let Some((q, _)) = self.object(g) {
                self.destroy(q, g)?;
            }
        }
        if state < 3 {
            let record = crate::spatial::Obstruction {
                kind: master.obstruction_type,
                class: master.obstruction_class,
            };
            if record.recorded() {
                self.spatial.add_record(id, record);
                if let Some(p) = self.spatial.get(id).copied() {
                    self.pathing.place_shape(id, &p);
                }
                self.pathing.tiles_changed(self.spatial.take_changes());
            }
        }
        if let Some((_, u)) = self.object_mut(id) {
            u.gather_point = None;
        }
        Ok(())
    }

    /// The attributes an ownership change ([`change_hands`]) moves with object `u` from player
    /// `old` to `new`, by its master of its own, in the order of `FUN_14120f800`: the storage
    /// the old owner loses (`FUN_1411a56c0` with 1: flags 2, 16 and 32, the amount's sign
    /// flipped), its class counters ([`count_class`], −1 for the old owner, then 1 for the new)
    /// and the storage the new owner gains (`FUN_141193860`: flags 1, 2 and 32). The lists'
    /// moves come between the first two, which nothing here reads.
    pub(in crate::turn) fn hand_over_attributes(
        &mut self,
        old: usize,
        new: usize,
        u: &Unit,
    ) -> Result<(), GenieError> {
        let Some(m) = u.own_master.as_deref() else {
            return Ok(());
        };
        for &(kind, amount, flag) in &m.storage {
            if 0 <= kind && matches!(flag, 2 | 16 | 32) {
                self.players[old].add_attribute(kind, -amount)?;
            }
        }
        count_class(&mut self.players[old].attributes, m, -1.0);
        count_class(&mut self.players[new].attributes, m, 1.0);
        for &(kind, amount, flag) in &m.storage {
            if 0 <= kind && matches!(flag, 1 | 2 | 32) {
                self.players[new].add_attribute(kind, amount)?;
            }
        }
        Ok(())
    }
}

/// The players' relations and the drop sites, as the task finder reads them.
pub(crate) struct World<'a> {
    pub(crate) players: &'a [Player],
    pub(crate) drop_sites: &'a [DropSite],
}

impl Diplomacy for World<'_> {
    /// [`crate::tasks::allied`] with `from`'s diplomacy.
    fn allied(&self, from: usize, to: usize, with_ally: bool) -> bool {
        let relations = self.players.get(from).map_or(&[][..], |p| &p.relations[..]);
        crate::tasks::allied(relations, from, to, with_ally)
    }

    fn drops_at(&self, building: i16, actor: &Actor<'_>) -> bool {
        crate::tasks::drops_at(self.drop_sites, building, actor)
    }
}

/// The players' relations and the drop sites as the task finder reads them while a unit's
/// owner's list is out for its update ([`UnitView`]): the owner's relations, the others'.
pub(in crate::turn) struct SideWorld<'a> {
    pub(in crate::turn) player: usize,
    pub(in crate::turn) relations: &'a [u8],
    pub(in crate::turn) foreign: Foreign<'a>,
    pub(in crate::turn) drop_sites: &'a [DropSite],
}

impl Diplomacy for SideWorld<'_> {
    /// [`crate::tasks::allied`] with `from`'s diplomacy.
    fn allied(&self, from: usize, to: usize, with_ally: bool) -> bool {
        let relations = if from == self.player {
            self.relations
        } else {
            self.foreign
                .player(from)
                .map_or(&[][..], |p| &p.relations[..])
        };
        crate::tasks::allied(relations, from, to, with_ally)
    }

    fn drops_at(&self, building: i16, actor: &Actor<'_>) -> bool {
        crate::tasks::drops_at(self.drop_sites, building, actor)
    }
}

/// The build points (`+0xc50`) a wait for a foundation reads of object `id`
/// ([`crate::wait_solidify::WaitSolidify::update`]) among its owner's other objects and the
/// other players' lists: None when the world does not hold it. Its target is the building that
/// sent the unit off (the unit AI's notify 0x216), whose build points are its construction's;
/// the world never gives its id to another object, so an object without a construction is not
/// met.
pub(in crate::turn) fn wait_progress(
    id: i32,
    others: &[Unit],
    foreign: Foreign<'_>,
) -> Option<f32> {
    others
        .iter()
        .find(|u| u.id == id)
        .or_else(|| foreign.find(id).map(|(_, u)| u))
        .and_then(|u| u.construction.map(|k| k.progress))
}

/// The state (`+0x640`) of object `id` among its owner's other objects and the other players'
/// lists: None when they do not hold it (the unit itself, or one of its owner's sleeping objects:
/// static objects only, never a drop site, which the callers ask for).
pub(in crate::turn) fn state_of(id: i32, others: &[Unit], foreign: Foreign<'_>) -> Option<u8> {
    others
        .iter()
        .find(|u| u.id == id)
        .or_else(|| foreign.find(id).map(|(_, u)| u))
        .map(|u| u.life.state)
}

/// What the task finder reads of object `id`, as `find` holds the objects (with their owners)
/// and `every` the players' masters; an annex stands for its main building (`+0xbb8`), `steps`
/// times (`Turns::task_target`'s reading). None when `find` does not hold it.
pub(in crate::turn) fn task_target_with<'a>(
    id: i32,
    steps: usize,
    find: &impl Fn(i32) -> Option<(usize, &'a Unit)>,
    every: &'a [Masters],
) -> Result<Option<TaskTarget<'a>>, GenieError> {
    let mut id = id;
    for _ in 0..steps {
        match find(id).and_then(|(_, u)| u.annex_of) {
            Some(main) => id = main,
            None => break,
        }
    }
    let Some((player, u)) = find(id) else {
        return Ok(None);
    };
    let master = every
        .get(player)
        .and_then(|m| m.get(u.life.master_id))
        .ok_or(crate::turn::OBJECT_MASTER)?;
    let [powerup, max_powerup] = u.hp_powerups;
    Ok(Some(TaskTarget {
        id,
        master,
        player,
        state: u.life.state,
        held: (u.life.attribute_type, u.life.amount),
        hp: u.life.hp + (max_powerup - powerup),
        max_hp: f32::from(master.hit_points) + max_powerup,
        attackable: u.attackable,
        pasture: u.pasture,
        top_action: u.top_action_kind(),
    }))
}

/// The search of a unit's slot 0x820 (`FUN_14106c6a0`) on target `t` (`general` the target the
/// general search reads, its main building's) with `filter`: the list of its own master
/// (`actor.master`) gives the task ([`crate::tasks::find_task`] in mode 0); without one, a
/// master with a task swap group (`+0x7b8`) tries its player's masters (`masters`) in id order
/// of its class, type and group, its own too, with any filter, and the first with a task gives
/// it. The task, with the master that gave it when the swap group did: with its `param_7` the
/// routine switches the unit to that master (slot 0x90 with 0), its own too, which is the
/// caller's. None: no task. The terrain under the target's place, which only a task for a
/// terrain reads, is not passed (as `Turns::find_work_task`, the same routine's port in
/// `crate::order`, which this module cannot call).
pub(in crate::turn) fn task_search<'m>(
    masters: &'m Masters,
    actor: &Actor<'_>,
    t: &TaskTarget<'_>,
    general: &TaskTarget<'_>,
    filter: i16,
    dip: &impl Diplomacy,
) -> Result<Option<(crate::masters::Task, Option<&'m Master>)>, GenieError> {
    let own = actor.master;
    let find = |list: &Master, filter: i16| {
        crate::tasks::find_task(list, actor, Some(t), Some(general), None, filter, dip)
            .map(Option::<&crate::masters::Task>::copied)
    };
    if let Some(task) = find(own, filter)? {
        return Ok(Some((task, None)));
    }
    if own.task_by_group == 0 {
        return Ok(None);
    }
    for candidate in masters.iter() {
        if candidate.class != own.class
            || candidate.kind != own.kind
            || candidate.task_by_group != own.task_by_group
        {
            continue;
        }
        if let Some(task) = find(candidate, -1)? {
            return Ok(Some((task, Some(candidate))));
        }
    }
    Ok(None)
}

impl Turns {
    /// A new object of player `p`'s `master` placed at `at` (the static `new_position`,
    /// `0x1411a58b0`, slot 0x60's `FUN_141193a10`): its height the ground's by the master's fly
    /// mode; without the map's elevations the height stays.
    pub(crate) fn on_ground(&self, p: usize, master: i16, at: [f32; 3]) -> [f32; 3] {
        let fly_mode = self
            .masters
            .get(p)
            .and_then(|m| m.get(master))
            .map_or(0, |m| m.fly_mode);
        self.spatial
            .terrain
            .as_ref()
            .and_then(|t| t.ground(at[0], at[1]).ok())
            .map_or(at, |g| {
                [
                    at[0],
                    at[1],
                    crate::missile::grounded(at[2], g, false, fly_mode),
                ]
            })
    }

    /// The command check's units (`FUN_140e553f0`, `FUN_140e55250`): each unit the world holds
    /// that is the player's own, or any while players may order each other's units
    /// ([`crate::settings::Settings::cheats_enabled`]); −1 for the others.
    pub(crate) fn checked_units(&self, player: usize, units: &[i32]) -> Vec<i32> {
        units
            .iter()
            .map(|&id| match self.object(id) {
                Some((p, _)) if p == player || self.settings.cheats_enabled => id,
                _ => -1,
            })
            .collect()
    }

    /// A mule cart's completion (`FUN_1411c6bd0`'s tail, a building whose master has a task of
    /// action type 0x99, [`crate::build::Built::cart`]) with a unit AI: with no current target a
    /// human player's runs its AI's slot 0x1e8 (`FUN_140c16240`; its world test `FUN_141533b60`
    /// and the game mode's, `FUN_140c87f20` not 4, hold in every recording probed): per task of
    /// its master in order, the target class of the last of type 0x99 so far names a gather group
    /// (`FUN_140fdd5c0` with 1: 1 for 7, 3 for 8, 0 for 15, 4 for 32 and 63), each searched by
    /// `ClosestResourceObject` (slot 0x80) from the cart as the drop site, any workers, from
    /// state 0 on; of those found, the nearest by the squared footprint gap (`FUN_14118f8c0`,
    /// below 99999, `DAT_142ef9008`); then slot 0x140 (`FUN_141070c90`): the work command toward
    /// it at its place, else at the cart's own (slot 0x7e8: no carried-resource test, any task;
    /// `+0x888`'s count is not held). A cart with a target moves to it (slot 0x138): not modelled.
    pub(crate) fn cart_search(&mut self, p: usize, id: i32, state: u8) -> Result<(), GenieError> {
        const CART_REACH: f32 = crate::image::turn::CART_REACH;
        let Some(idx) = self.players[p].objects.iter().position(|u| u.id == id) else {
            return Ok(());
        };
        let Some(current) = self.players[p].objects[idx]
            .ai
            .as_ref()
            .map(|ai| ai.current_target)
        else {
            return Ok(());
        };
        let found = if current == -1 && !self.players[p].computer {
            if self.players[p].objects[idx].mover.is_none() {
                return Err(GenieError::unsupported(
                    "a mule cart's search without its movement state (a moving building's setup)",
                ));
            }
            let master = self.master_of(p, self.players[p].objects[idx].life.master_id)?;
            let mut kinds = Vec::new();
            let mut class = -1;
            for t in &master.tasks {
                if t.action_type == crate::build::CART_TASK {
                    class = t.object_group;
                }
                let kind = match class {
                    7 => 1,
                    8 => 3,
                    15 => 0,
                    32 | 63 => 4,
                    _ => continue,
                };
                kinds.push(kind);
            }
            let cart = std::mem::replace(&mut self.players[p].objects[idx], Unit::taken_out());
            let result = (|| {
                let (before, rest) = self.players.split_at_mut(p);
                let (player, after) = rest.split_first_mut().expect("p is a player");
                let mut w = ResourceWorld {
                    player: p,
                    spatial: &self.spatial,
                    zones: &mut self.zones,
                    foreign: Foreign {
                        before: &*before,
                        after: &*after,
                        left: None,
                    },
                    every: &self.masters,
                    masters: self.masters.get(p),
                    plan: Planning {
                        pathing: &mut self.pathing,
                        now: self.world_time,
                        computer: player.computer,
                        object: self.streams.get(Stream::Object),
                        construction_rate: player.construction_rate,
                        groups: &self.groups,
                    },
                };
                let mut found = Vec::new();
                for kind in kinds {
                    let (f, _) = closest_resource_of(
                        &cart,
                        &mut w,
                        &player.objects,
                        (kind, -1, id),
                        (current, 0, crate::unit_ai::NO_POINT),
                    )?;
                    if f != -1 {
                        found.push(f);
                    }
                }
                Ok::<_, GenieError>(found)
            })();
            self.players[p].objects[idx] = cart;
            let found = result?;
            let cart_master = self.master_of(p, self.players[p].objects[idx].life.master_id)?;
            let at = self.players[p].objects[idx].position;
            let mut best: Option<(i32, [f32; 3])> = None;
            let mut best_d = CART_REACH;
            for f in found {
                let Some(o) = self.spatial.get(f) else {
                    continue;
                };
                let d = footprint_gap_squared(at, cart_master.radius, o.position, o.radius);
                if d < best_d {
                    best = Some((f, o.position));
                    best_d = d;
                }
            }
            Some(best.unwrap_or((-1, at)))
        } else {
            None
        };
        // all of it runs inside the completion's set_state(2), before the cart is in state 2
        let built = std::mem::replace(&mut self.players[p].objects[idx].life.state, state);
        let result = (|| {
            if let Some((target, point)) = found {
                let work = crate::order::Work {
                    target,
                    point,
                    carry_test: false,
                    filter: -1,
                    notified: true,
                };
                self.order_work(p, idx, &work)?;
            }
            // with a target (the one the work command gave the unit AI) the cart moves to it
            // (slot 0x138 at its place)
            let target = self.players[p].objects[idx]
                .ai
                .as_ref()
                .map_or(-1, |ai| ai.current_target);
            if target != -1
                && let Some(at) = self.spatial.get(target).map(|o| o.position)
            {
                self.move_to_object(p, idx, target, at)?;
            }
            Ok::<_, GenieError>(())
        })();
        self.players[p].objects[idx].life.state = built;
        result
    }
}

/// The owner's attributes 237, 238 and 239: a built farm's bonus ([`Completion::farm_bonus`]);
/// −1 for those the player has not got.
pub(crate) fn farm_bonus(attributes: &[f32]) -> [f32; 3] {
    let at = |i: usize| attributes.get(i).copied().unwrap_or(-1.0);
    [at(237), at(238), at(239)]
}

/// `FUN_1411980b0`'s visible resources, from `set_state` leaving a state below 4 for one above 6
/// (`FUN_14119d710`) and from the object's deletion: an object of id 0 and up whose master
/// gathers by itself (`+0x681`) leaves the lists of the players that explored its tile
/// ([`Visibility::forget_resource`]); the world's state (`+0x88`) is 0 in a game. The deletion's
/// call is the destructors' (`FUN_14116b5e0`). The routine's other parts are the callers': a
/// master of `create_doppelganger_on_death` 2 leaves its owner's watched list
/// (`FUN_141091310`).
///
/// [`Visibility::forget_resource`]: crate::visibility::Visibility::forget_resource
pub(crate) fn forget_resource(
    spatial: &mut Spatial,
    master: Option<&Master>,
    id: i32,
    position: [f32; 3],
) {
    if id < 0 {
        return;
    }
    if let (Some(m), Some(v)) = (master.filter(|m| m.auto_gather), spatial.sight.as_mut()) {
        v.forget_resource(id, position, m.gather_group);
    }
}

/// The combat object's death part (`FUN_14121cc40`'s block for a state of 3 or more from below
/// 3), of a unit of type 70 and of a building: its pasture lets it go (`FUN_141189ca0` on its
/// `+0x794`), its hit points go to 0, and unless its record went already (`mROOAlreadyCalled`,
/// `+0x686`): a unit's, or a building's not desolid whose master is tall (`FUN_1411c3f70`:
/// COMISS, a NaN height is not), mesh shape and obstruction record go (`FUN_141055b40`); one of
/// type 70 and up has no height after (its own master copy, `FUN_141186d10`: no longer tall); a
/// building on a tile that is no annex lets its tiles go (`FUN_140f65580`); then `+0x686` is
/// set. Not modelled: a unit in a pasture, the artifact class (1, `FUN_141218020`), class 43
/// (Gaia's object 0x11d at its place) and class 13's units inside. Not held: the damage sprites
/// (`FUN_1413b9e20`) and the hero attachments (`FUN_141201930`).
///
/// # Errors
///
/// What it does not model.
pub(crate) fn combat_dies(
    u: &mut Unit,
    master: &Master,
    spatial: &mut Spatial,
    pathing: &mut Pathing,
) -> Result<(), GenieError> {
    if u.pasture != -1 || u.my_pasture != -1 {
        return Err(GenieError::unsupported(
            "a dying unit in a pasture (FUN_141189ca0)",
        ));
    }
    if matches!(master.class, 1 | 13 | 43) {
        return Err(GenieError::unsupported("a dying unit of class 1, 13 or 43"));
    }
    u.life.hp = 0.0;
    if u.roo_called {
        return Ok(());
    }
    let desolid = u.construction.is_some_and(|c| c.desolid);
    if master.kind < 80 || (!desolid && 0.0 < master.height) {
        pathing.removed(u.id);
        spatial.drop_record(u.id);
    }
    if 0x45 < master.kind {
        spatial.set_tall(u.id, false);
    }
    if master.kind == 80
        && u.annex_of.is_none()
        && u.life.inside.is_none()
        && let Some(v) = spatial.sight.as_mut()
    {
        v.unlink_building(u.position, master.radius);
    }
    u.roo_called = true;
    Ok(())
}

/// What the combat object's `set_state` to 3 from state 2 (`FUN_14121cc40`) takes from its
/// owner's attributes: its paid cost (the cost entries of a resource flagged paid, each as a
/// float, summed in order) from 98 and 164 for a unit; from 98 and 165 for a building (98 alone
/// for the castle, 82, and the wonder, 276), nothing for an annex (`+0xbb8`). Each attribute is
/// held at 0 or more after (slot 0xf0 with 0), the caller's.
#[must_use]
pub(crate) fn refund_of(master: &Master, main: Option<i32>) -> Vec<(i16, f32)> {
    let mut cost = 0.0f32;
    for &(resource, amount, paid) in &master.cost {
        if 0 <= resource && paid {
            cost += f32::from(amount);
        }
    }
    if master.kind != 80 {
        return vec![(98, cost), (164, cost)];
    }
    if main.is_some() {
        return Vec::new();
    }
    if matches!(master.id, CASTLE | WONDER) {
        return vec![(98, cost)];
    }
    vec![(98, cost), (165, cost)]
}

/// What a built building's death (`FUN_1411dbb70` to state 3 from 2) does before the combat
/// object's part, for its caller to apply: the attributes its storage gave its owner
/// ([`storage_leaves`]), the place whose tiles it lets go (`FUN_140f65580`, a building on a tile
/// that is no annex), the count it lowers and its master's technology ([`BuildingDeath`]); its
/// owner's watched list lets it go (`FUN_141091310`, the caller's).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BuiltDeath {
    pub(crate) adds: Vec<(i16, f32)>,
    pub(crate) unlink: Option<[f32; 3]>,
    pub(crate) building: BuildingDeath,
}

/// [`BuiltDeath`] of the building `u` of `master`: the monastery (104), the wonder (276) and the
/// castle (82) count one less in attributes 52, 42 and 134 (slot 0xf8 with −1, then slot 0xf0
/// with 0 below 0); its master's technology (`+0x98c`) goes when no other building of the master
/// stands ([`Turns::withdraw_tech`]). A foundation terrain stays. Not modelled: a computer
/// player's (`FUN_1411e7ca0`), one with a pasture or its workers (`+0xcb8`), units inside, the
/// terrain 1 a foundation terrain of 28 lays at the death (`FUN_1411ce250`). Not held: its
/// linked objects (`+0xb78`, none held), `+0xca4` and the snow decal (`FUN_1413b9e20`).
///
/// # Errors
///
/// What it does not model.
pub(crate) fn built_death(
    u: &mut Unit,
    master: &Master,
    computer: bool,
) -> Result<BuiltDeath, GenieError> {
    let building = master.building.ok_or(GenieError::unsupported(
        "a dying building of a master without its building part",
    ))?;
    if computer || u.pasture != -1 || u.my_pasture != -1 || !u.garrison.is_empty() {
        return Err(GenieError::unsupported(
            "a dying building of a computer player, with a pasture or units inside",
        ));
    }
    let adds = storage_leaves(u, master);
    if building.foundation_terrain == RESTORED_FOUNDATION {
        return Err(GenieError::unsupported(
            "a dying building of foundation terrain 28 (its terrain 1, FUN_1411ce250)",
        ));
    }
    let unlink = (u.annex_of.is_none() && u.life.inside.is_none()).then_some(u.position);
    let count = match master.id {
        MONASTERY => Some(MONASTERIES),
        WONDER => Some(WONDERS),
        CASTLE => Some(CASTLES),
        _ => None,
    };
    let tech = (0 <= building.tech).then_some((master.id, building.tech));
    Ok(BuiltDeath {
        adds,
        unlink,
        building: BuildingDeath { count, tech },
    })
}

/// What a built building's storage gave its owner leaves (`FUN_1411e63f0`, then
/// `FUN_1411e6580`): each entry of mode 4 with an attribute and an amount above 0 (the
/// population headroom of a house, a town centre or a castle; slot 0xf8 with the amount
/// negated), then the population it holds itself (`+0x308` 4: the amount leaves, and it holds
/// nothing after, `FUN_14119d310` with −1 and 0); then each entry of mode 16 leaves its own store
/// ([`Unit::store`]) when the store holds its attribute.
pub(crate) fn storage_leaves(u: &mut Unit, master: &Master) -> Vec<(i16, f32)> {
    let mut adds = Vec::new();
    for &(kind, amount, flag) in &master.storage {
        if 0 <= kind && 0.0 < amount && flag == 4 {
            adds.push((kind, -amount));
        }
    }
    if u.life.attribute_type == 4 {
        adds.push((4, -u.life.amount));
        u.life.attribute_type = -1;
        u.life.amount = 0.0;
    }
    // FUN_1411e6580: each entry of mode 16 leaves its own store, when the store holds it
    for &(kind, amount, flag) in &master.storage {
        if 0 <= kind
            && 0.0 < amount
            && flag & 0x10 != 0
            && let Some(e) = u.store.iter_mut().find(|e| e.0 == kind)
        {
            e.1 -= amount;
        }
    }
    adds
}

/// What the footprint clearing reads of the object `u` ([`crate::build::ClearLook`]) with its
/// owner's `masters`; None without its master.
pub(in crate::turn) fn clear_look_of(
    u: &Unit,
    masters: Option<&Masters>,
) -> Option<crate::build::ClearLook> {
    let m = masters?.get(u.life.master_id)?;
    Some(crate::build::ClearLook {
        hp: u.life.hp,
        height: m.height,
        roo_called: u.roo_called,
        kind: m.kind,
        state: u.life.state,
        desolid: u.construction.is_some_and(|c| c.desolid),
    })
}

/// The classes a combat object of which counts in its player's attribute 40
/// (`FUN_141226060`).
pub(in crate::turn) const MILITARY_CLASSES: [i16; 24] = [
    0, 6, 12, 13, 18, 22, 23, 24, 26, 28, 35, 36, 43, 44, 45, 46, 47, 50, 51, 53, 54, 55, 56, 57,
];
/// The masters whose objects count in attribute 235 besides 40 (`FUN_141226060`).
pub(in crate::turn) const MILITARY_235: [i16; 2] = [1663, 1697];
/// The classes that count in attribute 37 (`FUN_141226060`); the villagers' (4) too.
pub(in crate::turn) const CIVILIAN_CLASSES: [i16; 4] = [2, 19, 20, 21];

/// `FUN_141226060`: a combat object (type 70 and up) of `master` counts `by` (1 or -1) in its
/// player's `attributes` by its class: a military class (0, 6, 12, 13, 18, 22 to 24, 26, 28, 35,
/// 36, 43 to 47, 50, 51, 53 to 57) in attribute 40, masters 1663 and 1697 also in 235; a
/// villager (class 4) in 38 when its master's task swap group (`+0x7b8`) is 1 or 2, and in 37
/// as the classes 2, 19, 20 and 21 do. The attributes take it as they are (no player slot).
/// Its callers: the combat setup (`FUN_14121df60`, 1), the destructors (`FUN_141200450`,
/// `FUN_1411ec020`, -1), the ownership change (`FUN_14120f800`, -1 then 1 for the new owner) and
/// the master change (`FUN_141186580` through `FUN_141209d90`, -1 then 1 with the new master).
pub(crate) fn count_class(attributes: &mut [f32], master: &Master, by: f32) {
    let mut add = |i: usize| {
        if let Some(v) = attributes.get_mut(i) {
            *v += by;
        }
    };
    if MILITARY_CLASSES.contains(&master.class) {
        add(40);
        if MILITARY_235.contains(&master.id) {
            add(235);
        }
        return;
    }
    if master.class == crate::spawn::VILLAGER && matches!(master.task_by_group, 1 | 2) {
        add(38);
    }
    if master.class == crate::spawn::VILLAGER || CIVILIAN_CLASSES.contains(&master.class) {
        add(37);
    }
}

/// The master types whose death-spawn master an ownership change copies (`FUN_14120f800`'s
/// switch on the master's `+0x8`): static to moving objects, combat units and buildings.
pub(crate) const DEAD_COPY_TYPES: [u8; 6] = [10, 20, 30, 40, 70, 80];

/// `FUN_14120f800`, slot 0x70 of a combat object (`TRIBE_Combat_Object`'s): unit `u` of player
/// `old` goes to player `new` (`masters`, every player's). First its records take the new owner
/// ([`Spatial::records_owner`]); without its master in the new owner's table (`player+0x28`)
/// nothing else changes and the answer is false. Then, in order: its sight goes from the old
/// owner's map (slot 0x1d8) when it sees (state 2, not inside another); its master's death
/// spawn (`+0x5e4`) in the old owner's table becomes its dead master ([`Unit::dead_master`], a
/// copy of type 10 to 40, 70 or 80) unless it has one; its owner
/// changes; it takes a master of its own (`FUN_141186d10`: a copy of the master it reads unless
/// it has one, [`Unit::own_master`]) with the new owner's master's `fog_visibility` (`+0x605`)
/// and, for livestock (class 58), line of sight (`+0x5d0`); its sight comes to the new owner's
/// map with that line of sight (slot 0x1d0); its unit AI's notifies go with `world_time`
/// (`FUN_140c5dc10`) and it tells its unit AI it changed hands (slot 0x328, 0x20b with the new
/// owner). The caller's: the storage (`FUN_1411a56c0` with 1 for the old owner,
/// `FUN_141193860` for the new), the lists (the players' slots 0x2d8 and 0x2d0) and the class
/// counters ([`count_class`], −1 then 1), from the master of its own, and the units a siege
/// weapon (class 13) carries. Not simulation: the names (`FUN_141189810`, `+0x6a8`, `+0x758`,
/// `+0x770`), the old owner's selection (`FUN_141677380` for `+0x30c` bit 0), the log line and
/// the message of game mode 12 for master 1377; nor are the slot 0xa8 calls on the copies (the
/// masters' graphics).
pub(crate) fn change_hands(
    u: &mut Unit,
    (old, new): (usize, usize),
    masters: &[Masters],
    spatial: &mut Spatial,
    world_time: u32,
) -> Result<bool, GenieError> {
    spatial.records_owner(u.id, new)?;
    let Some(to) = masters.get(new).and_then(|m| m.get(u.life.master_id)) else {
        return Ok(false);
    };
    let old_table = masters.get(old);
    let mut own = match u.own_master.take() {
        Some(own) => own,
        None => Box::new(
            old_table
                .and_then(|m| m.get(u.life.master_id))
                .ok_or(GenieError::unsupported(
                    "an ownership change of an object whose master the slice does not have",
                ))?
                .clone(),
        ),
    };
    // the dead master comes before the copy (FUN_141186d10), from the master it reads
    if u.dead_master.is_none() && 0 <= own.death_spawns[0] {
        u.dead_master = old_table
            .and_then(|m| m.get(own.death_spawns[0]))
            .filter(|d| DEAD_COPY_TYPES.contains(&d.kind))
            .map(|d| Box::new(d.clone()));
    }
    own.fog_visibility = to.fog_visibility;
    if own.class == LIVESTOCK {
        own.line_of_sight = to.line_of_sight;
    }
    let los = own.line_of_sight;
    u.own_master = Some(own);
    let sees = u.life.state == 2 && u.life.inside.is_none();
    spatial.change_owner(u.id, new, los, sees)?;
    if let Some(a) = u.artifact.as_mut() {
        a.gaia = new == 0;
    }
    if let Some(ai) = u.ai.as_mut() {
        ai.clear_notifies(world_time)?;
    }
    u.post_own(OWNER_CHANGED, [new as i32, 0, 0])?;
    Ok(true)
}

/// What a hit of an attacker of player `attacker` reads of the object `u` of player `p`, of
/// `master`, its player's `attributes` ([`victim_of`]): its relations to the attacker's player
/// both ways (`dip`, [`Diplomacy::allied`] with allies), whether its place is a guess (the world
/// `spatial` holds it as unknown, or not at all), its place in its tile's list and a building's
/// annexes (the world's links).
pub(crate) fn victim_in(
    u: &Unit,
    (p, master, attributes): (usize, &Master, &[f32]),
    (attacker, dip): (usize, &dyn Diplomacy),
    spatial: &Spatial,
    mode_680: i32,
) -> Victim {
    let mut v = victim_of(u, master, attributes, p, spatial.terrain.as_ref(), mode_680);
    v.allied = [dip.allied(attacker, p, true), dip.allied(p, attacker, true)];
    v.uncertain = spatial.get(u.id).is_none_or(|o| o.unknown.is_some());
    v.tile_place = spatial.tile_place(u.id);
    if let Some(b) = v.building.as_mut() {
        b.annexes = spatial.annexes_of(u.id);
    }
    v
}

/// What a hit reads of the object `u` of player `p` ([`Victim`]): its master's values and its
/// own, its player's elevation attributes, the terrain defense of its tile (`terrain`, the map's;
/// 1 for an object inside another, which has no tile) and the game's setting `mode_680`. Its
/// relations to an attacker (`allied`) and whether its place is a guess (`uncertain`) are the
/// caller's. The slice holds neither its unit AI's alert mode and attackers nor its facing.
pub(crate) fn victim_of(
    u: &Unit,
    master: &Master,
    attributes: &[f32],
    p: usize,
    terrain: Option<&crate::terrain::Terrain>,
    mode_680: i32,
) -> Victim {
    let at = |i: usize| attributes.get(i).copied().unwrap_or(0.0);
    let kind = master.kind;
    let attack_elevation = if 212 < attributes.len() {
        [attributes[211], attributes[212]]
    } else {
        [0.0; 2]
    };
    let defense = if master.terrain_bonus < 0 || u.life.inside.is_some() {
        Some(1.0)
    } else {
        terrain.and_then(|t| {
            t.defense(
                master.terrain_bonus,
                u.position[0] as i32,
                u.position[1] as i32,
            )
        })
    };
    let tasks = |kind: i16| master.tasks.iter().any(|t| t.action_type == kind);
    Victim {
        id: u.id,
        // the caller's, which holds the tile lists
        tile_place: 0,
        hp: u.life.hp,
        amount: u.life.amount,
        state: u.life.state,
        position: u.position,
        kind,
        class: master.class,
        master_id: master.id,
        radius: master.radius,
        height: master.height,
        storage: master.storage.first().map_or(0.0, |s| s.1),
        capacity: master.capacity,
        workers: u.workers,
        blast_defense: master.blast_defense,
        fog_visibility: master.fog_visibility,
        goto_sleep: u.life.goto_sleep,
        decay: u.life.master.map_or(0.0, |m| m.decay),
        player: p,
        elevation: [at(272), at(273)],
        attack_elevation,
        allied: [false; 2],
        uncertain: false,
        inside: u.life.inside.is_some(),
        source: crate::damage::Attacker {
            id: u.id,
            kind,
            class: master.class,
            master_id: master.id,
            ability: master.combat_ability,
            missile: master.missile,
            weapon_range: master.weapon_range,
            area_effect: master.area_effect,
            blast_level: master.blast_level,
            area_damage: master.area_damage,
            charge_type: master.charge_type,
            max_charge: master.max_charge,
            charge_target: master.charge_target,
            special_attack: master.special_attack,
            charge: u.charge,
            charge_hit: u.charge_hit,
            attack_powerup: u.attack_powerup,
            mode_680,
            player: p,
            kill_bonus_tasks: tasks(0x9a),
            fired_from: u.missile.as_ref().map_or(-1, |m| m.fired_from),
        },
        weapons: master.weapons.clone(),
        master_hp: master.hit_points,
        bonus_tasks: master
            .tasks
            .iter()
            .filter(|t| t.action_type == crate::damage::BONUS_DAMAGE)
            .copied()
            .collect(),
        // a combat object's slot 0xf0 is FUN_141082530 (types 50 and up)
        armored: (50 <= kind).then(|| crate::damage::Armored {
            armors: master.armors.clone(),
            base_armor: master.base_armor,
            terrain_bonus: master.terrain_bonus,
            resistance: master.resistance,
            ability: master.combat_ability,
            reflection: master.reflection,
            invulnerability: master.invulnerability,
            special_attack: master.special_attack,
            friendly_fire: master.friendly_fire,
            special_graphic: master.special_graphic,
            terrain: defense,
            armor_powerups: u.armor_powerups,
            armor_removed: u.armor_removed,
            hp_powerups: u.hp_powerups,
            kill_tasks: master
                .tasks
                .iter()
                .any(|t| t.action_type == 0xa1 && 1e-7 < t.work_val1),
            busy: u.top_action_kind().is_some() || u.unmodelled_action.is_some(),
            unit_ai: u.ai.is_some(),
            alert_idle: None,
            facing: None,
        }),
        damaged: Vec::new(),
        killed: None,
        // a building's slot 0xe8 is FUN_1411cea90
        building: (kind == BUILDING_KIND).then(|| crate::damage::Building {
            main: u.annex_of,
            // the caller's, which holds the world's annex links
            annexes: Vec::new(),
            // +0xb90 is not saved and not held: a blast resets it on every building of its tiles
            // before it reads it (FUN_14107f690's first pass), the only other one it reads being
            // the main building of an annex it reaches, which lies within the annex offset of
            // that annex, inside the six tiles the pass reaches past the radius; a trample reads
            // its target's or its main building's after its own blast
            blast_taken: 0.0,
            trebuchet: is_trebuchet(master),
            cost: paid_cost(master),
            damaged_lately: u.damaged_lately,
            struck: false,
            relics: u.relics,
            razed: Vec::new(),
        }),
    }
}

/// `FUN_140fde380`: `master` is of type 70 and up, of class 51 or 54, with a weapon (`+0x810`
/// above 0): a trebuchet, packed or not, whose kill counts as a unit's.
pub(crate) const fn is_trebuchet(master: &Master) -> bool {
    70 <= master.kind && matches!(master.class, 51 | 54) && !master.weapons.is_empty()
}

/// `master`'s paid cost (`+0x880`: each entry with a resource and the paid flag), summed as
/// floats in its order, as the kill counts read it (`FUN_1412005b0`, `FUN_1411cea90`).
pub(crate) fn paid_cost(master: &Master) -> f32 {
    let mut cost = 0.0f32;
    for &(resource, amount, paid) in &master.cost {
        if resource >= 0 && paid {
            cost += f32::from(amount);
        }
    }
    cost
}

/// A building's storage flag that its owner holds while it stands built (slot 0x70's).
const STORAGE_WHILE_BUILT: u8 = 4;
/// The other storage flag a building's new owner gains on an ownership change (slot 0x70's).
const STORAGE_8: u8 = 8;
/// The population headroom attribute, which a building's held amount gives (`+0x308` 4).
const POPULATION_HEADROOM: i16 = 4;
/// The point a building's units leave toward on an ownership change: none (`DAT_14330ec20`).
const HANDOVER_NO_POINT: f32 = crate::image::turn::HANDOVER_NO_POINT;
