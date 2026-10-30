//! `ClosestResourceObject` (`FUN_140c49760`): where it searches from and what it finds; a
//! herded sheep.

use crate::GenieError;
use crate::map::spatial::Spatial;
use crate::math::rng::Stream;
use crate::movement::{Me, Planning};
use crate::objects::masters::Masters;
use crate::objects::unit::Unit;
use crate::players::PlayerId;
use crate::unit_ai::world_view::owner_view;
use crate::unit_ai::world_view::{Foreign, UnitView};
use crate::world::others::OthersRef;
use crate::world::step::Step;

/// What `ClosestResourceObject` reads of the world, as the unit's update or its unit AI sees it.
pub(crate) struct ResourceWorld<'a, 'b> {
    pub(crate) player: PlayerId,
    pub(crate) spatial: &'a mut Spatial,
    pub(crate) zones: &'a mut crate::pathing::zones::Zones,
    pub(crate) foreign: Foreign<'a>,
    pub(crate) every: &'a [Masters],
    pub(crate) masters: Option<&'a Masters>,
    pub(crate) plan: Planning<'b>,
}

/// Where `ClosestResourceObject` (`FUN_140c49760`) searches from, the unit at `unit`: without a
/// site from `point` (at the unit's height), unless both its parts are exactly −1
/// (`DAT_14330ec20`, `UCOMISS`/`JP`/`JNZ`: a part that is not a number differs); with a site
/// (`has_site`, which replaces the point) and for (−1, −1) from the unit's place.
pub(crate) fn search_origin(point: [f32; 2], has_site: bool, unit: [f32; 3]) -> [f32; 3] {
    match point {
        [x, y] if !has_site && (x != -1.0 || y != -1.0) => [x, y, unit[2]],
        _ => unit,
    }
}

/// [`crate::gathering::gather_search::closest_resource_at`] for `unit` of gather group `kind` with fewer
/// than `cap` workers (−1 any) from the drop site `site` (−1 for none; one the world does not hold
/// counts as none, `FUN_1415417a0`), its unit AI's current target `current` passed over, below
/// state `min_state` nothing found, and without a site from `point` unless that is (−1, −1)
/// (`param_7`, `param_8`; [`crate::unit_ai::NO_POINT`]: the unit's place, as with a site): the
/// site's place, its master's radii and line of sight, and whether the search starts from its
/// footprint (the site's tile in the unit's zone of its restriction, or a dock: masters 45 and
/// 2172, `0x140c49d17`); the answer and the out argument's steps.
pub(crate) fn closest_resource_of(
    unit: &Unit,
    c: &mut ResourceWorld<'_, '_>,
    others: OthersRef<'_>,
    (kind, cap, site): (u8, i32, i32),
    (current, min_state, point): (i32, u8, [f32; 2]),
) -> Result<(i32, Option<i32>), GenieError> {
    use crate::gathering::gather_search::{self, Candidate, ResourceWorld};
    /// The site masters whose search starts from their footprint in any zone.
    const DOCKS: [i16; 2] = [45, 2172];
    let site = if site == -1 || c.spatial.get(site).is_none() {
        None
    } else {
        // the unit itself (a mule cart searches from its own place) or another object
        let (sp, s) = if site == unit.id.get() {
            Some((c.player, unit))
        } else {
            others.find(site)
        }
        .ok_or(GenieError::unsupported(
            "a resource search's drop site the slice does not hold",
        ))?;
        let m = s.master(c.every, sp)?;
        let footprint = DOCKS.contains(&m.id) || {
            let restriction = unit
                .body
                .mover()
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
                .ok_or(crate::pathing::zones::UNKNOWN_RESTRICTION)?;
            let zones = c.zones.of(w, h, &terrain.kinds, values)?;
            let zone = |p: [f32; 3]| zones.zone(p[0] as i32, p[1] as i32);
            zone(s.position()) == zone(unit.position())
        };
        Some(gather_search::SearchSite {
            position: s.position(),
            radius: m.radius,
            line_of_sight: m.line_of_sight,
            footprint,
        })
    };
    let masters = c.masters.ok_or(GenieError::unsupported(
        "a resource search without its player's masters",
    ))?;
    let master = unit.master_in(Some(masters))?;
    let mover = unit.body.mover().ok_or(GenieError::unsupported(
        "a resource search without movement state",
    ))?;
    let mask = mover.stats.restriction_mask;
    let (player, me_id, every) = (c.player, unit.id, c.every);
    let candidate = |id: i32, owner: PlayerId, u: &Unit, position: [f32; 3]| {
        let m = u.master(every, owner)?;
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
    let me = candidate(me_id.get(), player, unit, unit.position())?;
    let objects = |spatial: &Spatial, x: i32, y: i32| -> Result<Vec<Candidate>, GenieError> {
        spatial
            .objects_on(x, y)
            .map(|(id, p)| {
                if id == me_id.get() {
                    return Ok(me);
                }
                let (owner, u) = others.find(id).ok_or(GenieError::unsupported(
                    "an object on a tile the slice does not hold",
                ))?;
                candidate(id, owner, u, p.position)
            })
            .collect()
    };
    let position = search_origin(point, site.is_some(), unit.position());
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
        id: unit.id.get(),
        group: unit.group.id,
        action_state: unit.top_action().map_or(0, |a| a.state),
    };
    let master_id = unit.life.master_id;
    let plan = &mut c.plan;
    let blocked = u64::from(mask ^ 0xff) | crate::map::spatial::BLOCKED;
    gather_search::closest_resource_at(
        &seeker,
        site.as_ref(),
        kind,
        cap,
        ResourceWorld {
            map: c.spatial.map,
            world: c.spatial,
            explored: &|spatial, x, y| spatial.explored(player, x, y),
            passable: &|spatial, x, y| spatial.tiles().word(x, y).is_some_and(|w| w & blocked == 0),
            objects: &objects,
            reach: &mut |spatial, id| {
                mover.reaches_object(&me_walk, spatial, plan, id, 0.0, master_id)
            },
        },
    )
}

/// `ClosestObject`'s "Sheep is moving" (`FUN_140c488b0`): the object's top action is the Artifact
/// (slot 0x530 107) with a point (slot 0x288, the action's `+0x38`, not −1).
pub(crate) fn herded(others: OthersRef<'_>, id: i32) -> bool {
    others.get(id).is_some_and(|u| {
        u.top_action_kind() == Some(crate::ownership::artifact::ARTIFACT_TYPE)
            && u.actions
                .artifact()
                .is_some_and(|a| a.goal.is_some_and(|g| g[0] != -1.0))
    })
}

impl UnitView<'_, '_> {
    /// The unit AI's `ClosestResourceObject` while it runs
    /// ([`crate::unit_ai::UnitSide::closest_resource`]): what it found, with its place and class.
    pub(crate) fn closest_gatherable(
        &mut self,
        search: crate::unit_ai::ResourceSearch,
    ) -> Result<Option<crate::unit_ai::Gatherable>, GenieError> {
        let mut w = ResourceWorld {
            player: self.player,
            spatial: &mut self.c.w.spatial,
            zones: &mut self.c.w.zones,
            foreign: owner_view(&self.c.w.players, self.c.player, self.new_owner)?.players(),
            every: &self.c.w.masters,
            masters: self.c.w.masters.get(self.player.index()),
            plan: Planning {
                pathing: &mut self.c.w.pathing,
                now: self.c.w.world_time,
                computer: self.computer,
                object: self.c.w.streams.get(Stream::Object),
                construction_rate: self.construction_rate,
                groups: &self.c.w.groups,
            },
        };
        let (id, _) = closest_resource_of(
            self.unit,
            &mut w,
            owner_view(&self.c.w.players, self.c.player, self.new_owner)?,
            (search.kind, search.cap, search.site),
            (search.current, search.min_state, search.point),
        )?;
        if id == -1 {
            return Ok(None);
        }
        let (owner, u) = owner_view(&self.c.w.players, self.c.player, self.new_owner)?
            .find(id)
            .ok_or(GenieError::unsupported(
                "a resource found that the slice does not hold",
            ))?;
        let class = u.master(&self.c.w.masters, owner)?.class;
        Ok(Some(crate::unit_ai::Gatherable {
            id,
            position: u.position(),
            class: i32::from(class.get()),
        }))
    }
}

impl Unit {
    /// The unit AI's `ClosestResourceObject` (slot 0x80, `FUN_140c49760`) with no drop site, no
    /// point and `param_6` 2 ([`crate::gathering::gather_search::closest_resource`]): the closest object of
    /// gather group `kind` with fewer than `cap` workers (−1 any) the unit reaches (slot 0x3d0
    /// with range 0), from the tiles the player explored that the unit's restriction lets it
    /// stand on; −1 for none.
    pub(crate) fn resource_search(
        &self,
        c: &mut Step<'_>,
        kind: u8,
        cap: i32,
    ) -> Result<i32, GenieError> {
        self.resource_search_from(c, kind, cap, -1)
    }

    /// [`Unit::resource_search`] from the drop site `site` (−1 for none;
    /// [`crate::gathering::gather_search::closest_resource_at`]): the site's place, its master's radii and
    /// line of sight, and whether the search starts from its footprint (the site's tile in the
    /// unit's zone of its restriction, or a dock: masters 45 and 2172, `0x140c49d17`).
    pub(crate) fn resource_search_from(
        &self,
        c: &mut Step<'_>,
        kind: u8,
        cap: i32,
        site: i32,
    ) -> Result<i32, GenieError> {
        let others = OthersRef::of(&c.w.players, c.player)?;
        let current = self
            .ai
            .as_ref()
            .ok_or(GenieError::unsupported(
                "a resource search without a unit AI",
            ))?
            .current_target;
        let mut w = ResourceWorld {
            player: c.player,
            spatial: &mut c.w.spatial,
            zones: &mut c.w.zones,
            foreign: others.players(),
            every: &c.w.masters,
            masters: c.w.masters.get(c.player.index()),
            plan: Planning {
                pathing: &mut c.w.pathing,
                now: c.w.world_time,
                computer: c.w.players[c.player.index()].computer,
                object: c.w.streams.get(Stream::Object),
                construction_rate: c.w.players[c.player.index()].construction_rate,
                groups: &c.w.groups,
            },
        };
        let search = (current, 2, crate::unit_ai::NO_POINT);
        closest_resource_of(self, &mut w, others, (kind, cap, site), search).map(|(id, _)| id)
    }
}
