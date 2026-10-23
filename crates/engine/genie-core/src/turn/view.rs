//! The views a unit's update and its unit AI read the world through ([`UnitView`],
//! [`Context`], [`Foreign`], [`OwnerView`], the pickers).

use crate::build::Walk;
use crate::damage::Victim;
use crate::masters::Masters;
use crate::movement::{Me, Mover, Planning, footprint_gap_squared};
use crate::pathing::Pathing;
use crate::rng::{Stream, Streams, Well512};
use crate::spatial::{Placed, Spatial};
use crate::sprite::Graphic;
use crate::tasks::{Actor, DropSite};
use crate::turn::player::Player;
use crate::turn::unit::{AiResume, Unit};
use crate::turn::world::{
    Releaser, ResourceWorld, SideWorld, closest_resource_of, herded, task_search, task_target_with,
};
use crate::turn::{BuildingDeath, PETARD, Recycling};
use crate::unit_ai::{AiMove, Search, UnitAi, Unsupported};

/// What a tick's update of one object needs from the world.
/// What a hunt's drop-off ([`Unit::drop_off_in`]) reads besides its unit: the owner's masters,
/// the world with its zone maps and planners, the world's time, the Object stream (the drop-site
/// search's plans), the owner's type and construction rate.
pub(crate) struct DropWorld<'a> {
    pub masters: Option<&'a Masters>,
    pub spatial: &'a Spatial,
    pub zones: &'a mut crate::zones::Zones,
    pub pathing: &'a mut Pathing,
    pub object: &'a mut Well512,
    pub world_time: u32,
    pub computer: bool,
    pub construction_rate: f32,
    pub groups: &'a crate::group::UnitGroups,
}

pub(in crate::turn) struct Context<'a> {
    /// The tick's dt in milliseconds, and in seconds (`world+0x164`).
    pub(in crate::turn) dt: u32,
    pub(in crate::turn) dt_seconds: f32,
    /// The ticks so far, this one included.
    pub(in crate::turn) tick: u32,
    /// The owner's accumulated time when it has the turn: the full unit-AI update runs.
    pub(in crate::turn) turn: Option<u32>,
    pub(in crate::turn) world_time: u32,
    pub(in crate::turn) computer: bool,
    pub(in crate::turn) streams: &'a mut Streams,
    /// Every object's place, which a walker reads and its moves update.
    pub(in crate::turn) spatial: &'a mut Spatial,
    /// The planners and the mesh, which a walker's path request uses.
    pub(in crate::turn) pathing: &'a mut Pathing,
    /// The world's unit groups, which a walker's long path reads.
    pub(in crate::turn) groups: &'a crate::group::UnitGroups,
    /// The owner's construction rate (attribute 195).
    pub(in crate::turn) construction_rate: f32,
    /// The owner's masters, which an attack reads.
    pub(in crate::turn) masters: Option<&'a Masters>,
    /// The target of the unit's attack, read before its update and written back after it, and
    /// the objects about a unit with an area effect, which its blast may reach.
    pub(in crate::turn) victim: Option<&'a mut Victim>,
    /// The unit AI's current target as its target block reads it ([`Turns::seen`]).
    pub(in crate::turn) ai_target: Result<Option<crate::unit_ai::Seen>, Unsupported>,
    pub(in crate::turn) blasted: Vec<Victim>,
    /// The unit was killed in its update (a retaliation), which the caller handles after it.
    pub(in crate::turn) own_killed: Option<crate::damage::Kill>,
    /// What the unit's death leaves for the caller: the remains its dying sprite's end makes
    /// (`FUN_1411a4e90`: the master, the place and the dead object's facet) and the paid cost its
    /// owner takes back (attributes 98 and 164).
    pub(in crate::turn) remains: Option<(i16, [f32; 3], Option<u8>)>,
    pub(in crate::turn) refund: Vec<(i16, f32)>,
    /// What a built building's death takes from its owner before its combat part's refund: the
    /// counts of the monastery, the wonder and the castle (slot 0xf8, then slot 0xf0 with 0 for
    /// a value below 0), and its master's technology (`+0x98c`) the caller withdraws when no
    /// other building of the master stands ([`Turns::withdraw_tech`]).
    pub(in crate::turn) building_death: Option<BuildingDeath>,
    /// The pauses the unit's walk gave units it met ([`Mover::pauses`]).
    pub(in crate::turn) pauses: Vec<(i32, f32)>,
    /// The gate's update asked its player to open or shut it ([`Turns::toggle_gate`], the
    /// caller's after the update).
    pub(in crate::turn) gate_toggle: bool,
    /// The unit's update stopped before its unit AI for the caller's work order
    /// ([`Unit::ai_part`]); given back, the update runs only that part.
    pub(in crate::turn) ai_resume: Option<AiResume>,
    /// The player the unit's own AI gave it to (slot 0x70, [`Unit::ai_change_owner`]): the
    /// update ends with the takeover ([`Step::Captured`]).
    pub(in crate::turn) taken_over: Option<usize>,
    /// The objects the unit's actions let go (slot 0x180, [`Turns::release`]) and take (slot
    /// 0x178, [`Turns::take`]) in other players' lists, which the caller applies after the hit's
    /// write-backs.
    pub(in crate::turn) releases: Vec<(i32, Releaser)>,
    pub(in crate::turn) takes: Vec<(i32, Releaser)>,
    /// The foundations a counted builder's Build leaves as the unit's action list goes
    /// ([`Unit::delete_actions`], `FUN_140aadbf0`): the building `FUN_140aaddb0` gives counts one
    /// builder less ([`Turns::leave_builders`]), the caller's before the releases.
    pub(in crate::turn) builder_leaves: Vec<i32>,
    /// The unit group a unit that entered a building left (`FUN_1411921d0`), with the
    /// building: the caller runs the group's removeUnit ([`Turns::remove_unit`]).
    pub(in crate::turn) left_group: Option<(i32, i32)>,
    /// The objects the unit made (a ranged unit's missiles), which join the end of its owner's
    /// list after it and update later in the same tick.
    pub(in crate::turn) made: Vec<Unit>,
    /// The objects whose master leaves ghosts that died in this update ([`crate::ghost`]), the
    /// caller's.
    pub(in crate::turn) ghost_deaths: Vec<i32>,
    /// The buildings whose construction started in this update: their owner watches them
    /// ([`Turns::watch_started`]), the caller's.
    pub(in crate::turn) started: Vec<i32>,
    /// The buildings that died in this update: their owner's watched list lets them go
    /// (`FUN_141091310`), the caller's.
    pub(in crate::turn) unwatched: Vec<i32>,
    /// The annexes a building's `set_state` (`FUN_1411dbb70`) took to its own state (slot 0xa0
    /// on each, before its own parts), the caller's after the update ([`Turns::set_state`]).
    pub(in crate::turn) annex_states: Vec<(i32, u8)>,
    /// The objects a builder's footprint clearing deletes (the Build's `set_state(7)`,
    /// `FUN_1415c3620` with `param_6`; their records went already), the caller's after the
    /// update ([`Turns::clear_object`]).
    pub(in crate::turn) cleared: Vec<crate::build::Cleared>,
    /// The units on a foundation's footprint its builder's test met (the foundation, them):
    /// the caller sends off those the test would ([`Turns::evacuate`]).
    pub(in crate::turn) evacuations: Vec<(i32, Vec<i32>)>,
    /// The world's reusable ids and pools.
    pub(in crate::turn) recycling: &'a mut Recycling,
    /// The game's setting at `+0x680` is 1 ([`Turns::mode_680`]).
    pub(in crate::turn) mode_1: bool,
    /// The owner's attributes, which a gatherer's productivity reads.
    pub(in crate::turn) attributes: &'a [f32],
    /// The graphics, for the sprites a hunter takes.
    pub(in crate::turn) graphics: &'a std::collections::BTreeMap<i16, Graphic>,
    /// The drop-site table, whose livestock entries a hunt leads its livestock to.
    pub(in crate::turn) drop_sites: &'a [DropSite],
    /// The owner's id and civ (`player+0x738`).
    pub(in crate::turn) player: usize,
    pub(in crate::turn) civ: u8,
    /// The player shares its gathered resources with its allies (`+0x1426`).
    pub(in crate::turn) sharing: bool,
    /// The game's farm setting ([`crate::settings::Settings::walkable_farms`]).
    pub(in crate::turn) farm_stops: bool,
    /// The game's setting [`crate::settings::Settings::predator_hunts_end`].
    pub(in crate::turn) predator_hunts_end: bool,
    /// Every player's masters and attribute 97 (by player), for a capture.
    pub(in crate::turn) every_masters: &'a [Masters],
    pub(in crate::turn) attribute_97: &'a [f32],
    /// The player whose turn this tick is (`world+0x188`), every player's accumulated time
    /// (`world+0x18c`, as it stands when the object updates) and computer flag.
    pub(in crate::turn) player_turn: usize,
    pub(in crate::turn) player_time: &'a [u32],
    pub(in crate::turn) computers: &'a [bool],
    /// Some player's attribute 204 is not 0, which a building's completion reads.
    pub(in crate::turn) attribute_204: bool,
    /// [`Turns::mode_680`].
    pub(in crate::turn) mode_680: i32,
    /// The game mode is one of `FUN_141302540`'s ([`scenario_mode`]).
    pub(in crate::turn) scenario: bool,
    /// The game mode's descriptor flag `+0x9c` ([`standard_mode`]) and each player's computer
    /// flag and `mAllowCapturingGaiaUnits`, which a takeover by sight reads
    /// ([`crate::visibility::sight_taker`]).
    pub(in crate::turn) standard: bool,
    pub(in crate::turn) sight_takers: &'a [(bool, bool)],
    /// The owner's object tracker, which a completion's first part (`FUN_1411c6bd0`) moves a
    /// building in, and the unit lines its keys read ([`crate::tracker`]).
    pub(in crate::turn) tracker: &'a mut crate::tracker::Tracker,
    pub(in crate::turn) lines: &'a crate::lines::UnitLines,
    /// The attributes the owner gains from the object's update (a building it completed), which
    /// the caller adds after it.
    pub(in crate::turn) attribute_adds: Vec<(i16, f32)>,
    /// The technologies the owner researches from the object's update (a completed building's),
    /// which the caller completes after the attributes ([`Turns::complete_research`]).
    pub(in crate::turn) techs_done: Vec<i16>,
    /// The mule carts the object's update completed, whose search for resources the caller runs
    /// after it ([`Turns::cart_search`]).
    pub(in crate::turn) carts: Vec<(i32, u8)>,
    /// The buildings the object's update completed with a stack unit or that disappear when
    /// built: the caller makes the stack unit ([`Turns::stack_unit`]) before the technologies
    /// and destroys the building (slot 0xd0) after them.
    pub(in crate::turn) completions: Vec<(i32, Option<crate::build::StackUnit>, bool)>,
    /// The work a builder found after its building (slot 0x7e8 from the Build's case 0xdf),
    /// which the caller gives after the update ([`Turns::order_work`]).
    pub(in crate::turn) work_order: Option<crate::order::Work>,
    /// The unit AI's order on a target (`0x2db`) the unit took from its queue, which the caller
    /// runs after the update ([`Turns::order_on_target`]), and the full update's tail that reads
    /// what the order gave ([`crate::unit_ai::UnitAi::after_order`]).
    pub(in crate::turn) order_on_target: Option<(
        crate::unit_ai::QueuedOrder,
        Option<crate::unit_ai::AfterOrder>,
    )>,
    /// The targets of the unit AI's `HuntObject`s, in order, whose unit part (the stop, then the
    /// hunt command) the caller runs after the update ([`Turns::hunt_command`]).
    pub(in crate::turn) hunt_objects: Vec<i32>,
    /// The targets of the unit AI's `GatherObject`s, in order, whose gather command the caller
    /// runs after the update ([`Turns::gather_command`]).
    pub(in crate::turn) gather_objects: Vec<i32>,
    /// The foundation of the unit AI's `BuildObject`, which the caller builds after the update
    /// ([`Turns::build_object`]).
    pub(in crate::turn) build_object: Option<i32>,
    /// The foundation of the unit's own build command (slot 0x380, a Gather's `set_state(6)` on
    /// a target not built yet), which the caller runs after the update ([`Turns::build_command`]).
    pub(in crate::turn) build_command: Option<i32>,
    /// The unit AI's moves off foundations' footprints (slot 0x148), in order, which the caller
    /// runs after the update ([`Turns::move_aside`]).
    pub(in crate::turn) move_aside: Vec<AiMove>,
    /// The waits for foundations the unit's slot 0x388 found while its unit AI ran, which the
    /// caller makes the unit's after the update ([`Turns::wait_for_foundation`]).
    pub(in crate::turn) waits: Vec<Wait>,
    /// The work a builder gave an annex whose main building takes it (the foundation, the
    /// seconds; [`crate::build::Worked::Main`]), which the caller runs after the update
    /// ([`Turns::work_building`]).
    pub(in crate::turn) main_works: Vec<(i32, f32)>,
    /// The owner's diplomacy (`player+0x260`), which the unit's task search reads.
    pub(in crate::turn) relations: &'a [u8],
    /// The livestock a hunt sends to a drop site with its herd, which the caller sends after the
    /// update ([`Turns::lead_herd`]).
    pub(in crate::turn) leads: Option<(i32, i32)>,
    /// The map's zone maps, which an auto scout's search reads.
    pub(in crate::turn) zones: &'a mut crate::zones::Zones,
    /// The other players' lists, as they stand before the unit's update.
    pub(in crate::turn) foreign: Foreign<'a>,
}

/// What the unit's slot 0x388 (`FUN_141226fc0`) found while its unit AI ran
/// ([`crate::unit_ai::UnitSide::wait_solidify`]): the foundation, the task its slot 0x820 found
/// on it and the master the search switches the unit to (slot 0x90; None when its own master's
/// list gave the task). The caller makes them the unit's after the update
/// ([`Turns::wait_for_foundation`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Wait {
    pub(crate) target: i32,
    pub(crate) task: crate::masters::Task,
    pub(crate) switch: Option<i16>,
}

/// The players but the owner of the unit that updates, read-only: the players before it and
/// after it.
#[derive(Clone, Copy)]
pub(in crate::turn) struct Foreign<'a> {
    pub(in crate::turn) before: &'a [Player],
    pub(in crate::turn) after: &'a [Player],
    /// The live list of the player whose list is out, once its object changed hands: that
    /// object's AI part reads it as another player's ([`OwnerView`]).
    pub(in crate::turn) left: Option<(usize, &'a [Unit])>,
}

impl<'a> Foreign<'a> {
    /// The object `id` in another player's live or sleeping list, with that player.
    pub(in crate::turn) fn find(&self, id: i32) -> Option<(usize, &'a Unit)> {
        let skip = self.before.len() + 1;
        let before = self.before.iter().enumerate();
        let after = self.after.iter().enumerate().map(|(q, p)| (q + skip, p));
        before
            .chain(after)
            .find_map(|(q, p)| {
                p.objects
                    .iter()
                    .chain(&p.sleeping)
                    .find(|u| u.id == id)
                    .map(|u| (q, u))
            })
            .or_else(|| {
                let (q, list) = self.left?;
                list.iter().find(|u| u.id == id).map(|u| (q, u))
            })
    }

    /// Player `q`, unless it is the one whose list is out.
    pub(in crate::turn) fn player(&self, q: usize) -> Option<&'a Player> {
        let own = self.before.len();
        match q.cmp(&own) {
            std::cmp::Ordering::Less => self.before.get(q),
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => self.after.get(q - own - 1),
        }
    }
}

/// What a unit's AI part reads of its owner's side ([`Unit::ai_part`]): the owner, its other
/// objects, the other players' lists, its masters, diplomacy, attributes, civ and construction
/// rate. A unit another player took over in its update reads its new owner's, with its old
/// owner's list among the others' (`RGE_Combat_Object::update` reads the owner after the
/// change).
#[derive(Clone, Copy)]
pub(in crate::turn) struct OwnerView<'a> {
    pub(in crate::turn) player: usize,
    pub(in crate::turn) others: &'a [Unit],
    pub(in crate::turn) foreign: Foreign<'a>,
    pub(in crate::turn) masters: Option<&'a Masters>,
    pub(in crate::turn) relations: &'a [u8],
    pub(in crate::turn) attributes: &'a [f32],
    pub(in crate::turn) civ: u8,
    pub(in crate::turn) construction_rate: f32,
}

impl<'a> OwnerView<'a> {
    /// The owner's side: player `new`'s when the unit changed hands to it in its update, else
    /// the updating player's (`c`, its other objects `others`).
    pub(in crate::turn) fn of<'c: 'a>(
        c: &Context<'c>,
        others: &'a [Unit],
        new: Option<usize>,
    ) -> Self {
        let own = Self {
            player: c.player,
            others,
            foreign: c.foreign,
            masters: c.masters,
            relations: c.relations,
            attributes: c.attributes,
            civ: c.civ,
            construction_rate: c.construction_rate,
        };
        let Some((new, q)) = new.and_then(|new| Some((new, c.foreign.player(new)?))) else {
            return own;
        };
        Self {
            player: new,
            others: &q.objects,
            foreign: Foreign {
                left: Some((c.player, others)),
                ..c.foreign
            },
            masters: c.every_masters.get(new),
            relations: &q.relations,
            attributes: &q.attributes,
            civ: q.civ,
            construction_rate: q.construction_rate,
        }
    }
}

/// A unit and the world as its unit AI asks them while it runs, the AI taken out of the unit
/// for that time ([`crate::unit_ai::UnitSide`]): the searches and the unit's own commands.
pub(in crate::turn) struct UnitView<'u> {
    /// The unit, without its unit AI.
    pub(in crate::turn) unit: &'u mut Unit,
    /// What the attack check of a search reads of the unit.
    pub(in crate::turn) attacker: crate::unit_ai::Attacker,
    /// The owner, as the AI runs (a captured unit's new one).
    pub(in crate::turn) player: usize,
    /// The owner is a computer player.
    pub(in crate::turn) computer: bool,
    /// The world's objects (and the players' scout maps, which an auto scout makes and marks).
    pub(in crate::turn) spatial: &'u mut Spatial,
    /// The planners and the mesh, which a reach test walks.
    pub(in crate::turn) pathing: &'u mut Pathing,
    /// The world's unit groups, which a reach test's long path reads.
    pub(in crate::turn) groups: &'u crate::group::UnitGroups,
    /// The Object stream, which a reach test's new path draws from.
    pub(in crate::turn) object: &'u mut Well512,
    /// The Sprite stream, which a new sprite draws from.
    pub(in crate::turn) sprite: &'u mut Well512,
    /// The world's time.
    pub(in crate::turn) world_time: u32,
    /// The owner's construction rate.
    pub(in crate::turn) construction_rate: f32,
    /// The owner's other objects.
    pub(in crate::turn) others: &'u [Unit],
    /// The map's terrain zones, which an explorer's search reads.
    pub(in crate::turn) zones: &'u mut crate::zones::Zones,
    /// The other players' lists.
    pub(in crate::turn) foreign: Foreign<'u>,
    /// Every player's masters.
    pub(in crate::turn) every: &'u [Masters],
    /// The owner's masters.
    pub(in crate::turn) masters: Option<&'u Masters>,
    /// The sprites.
    pub(in crate::turn) graphics: &'u std::collections::BTreeMap<i16, Graphic>,
    /// The objects an action took (slot 0x178), the caller's.
    pub(in crate::turn) takes: &'u mut Vec<(i32, Releaser)>,
    /// The objects an action let go (slot 0x180), the caller's.
    pub(in crate::turn) releases: &'u mut Vec<(i32, Releaser)>,
    /// The foundations a counted builder's Build left ([`Context::builder_leaves`]), the
    /// caller's.
    pub(in crate::turn) builder_leaves: &'u mut Vec<i32>,
    /// The waits for foundations its slot 0x388 found ([`Wait`]), the caller's.
    pub(in crate::turn) waits: &'u mut Vec<Wait>,
    /// What its task search reads of its owner: its diplomacy, attributes and civ, and the drop
    /// sites.
    pub(in crate::turn) relations: &'u [u8],
    pub(in crate::turn) attributes: &'u [f32],
    pub(in crate::turn) civ: u8,
    pub(in crate::turn) drop_sites: &'u [DropSite],
}

/// What `bestUnitToAttack` reads of the world for the unit whose AI runs
/// ([`crate::unit_ai::AttackPicker`]): the objects it found, by what the world holds of them, and
/// the unit's quick path to them.
pub(in crate::turn) struct WorldPicker<'a> {
    /// What the tests read of the unit.
    pub(in crate::turn) attacker: &'a crate::unit_ai::Attacker,
    /// What they read of its unit AI; None outside `bestUnitToAttack`.
    pub(in crate::turn) seeker: Option<crate::unit_ai::Seeker>,
    /// The unit.
    pub(in crate::turn) me: i32,
    /// Its owner.
    pub(in crate::turn) player: usize,
    /// The owner's other objects.
    pub(in crate::turn) others: &'a [Unit],
    /// The other players' lists.
    pub(in crate::turn) foreign: Foreign<'a>,
    /// The world's objects.
    pub(in crate::turn) spatial: &'a Spatial,
    /// Every player's masters.
    pub(in crate::turn) every: &'a [Masters],
    /// The unit's master's terrain restriction (`+0x606`).
    pub(in crate::turn) restriction: i16,
}

impl WorldPicker<'_> {
    /// The object `id` as the attack tests read it, and whether it moves (slot 0x208 above 0).
    pub(in crate::turn) fn target(
        &self,
        id: i32,
    ) -> Result<(crate::unit_ai::Target, bool), Unsupported> {
        let (owner, t) = self
            .foreign
            .find(id)
            .or_else(|| {
                self.others
                    .iter()
                    .find(|u| u.id == id)
                    .map(|u| (self.player, u))
            })
            .ok_or(Unsupported("a found object the slice does not hold"))?;
        let m = self
            .every
            .get(owner)
            .and_then(|ms| ms.get(t.life.master_id))
            .ok_or(Unsupported(
                "a found object of a master the slice does not have",
            ))?;
        let placed = self
            .spatial
            .get_known(id)?
            .ok_or(Unsupported("a found object without a place"))?;
        let capture = self
            .spatial
            .sight
            .as_ref()
            .and_then(|v| v.seer(id))
            .map_or(0, |s| s.capture);
        let target = crate::unit_ai::Target {
            owner,
            class: m.class,
            kind: m.kind,
            master: t.life.master_id,
            combat_level: m.combat_level,
            capture,
            has_ai: t.ai.is_some(),
            inside: t.life.inside.is_some(),
            targeting: !t.untargetable,
            attackable: t.attackable,
            desolid: t.construction.as_ref().is_some_and(|c| c.desolid),
            position: placed.position,
            radius: placed.radius,
        };
        let moving = 0.0 < t.mover.as_ref().map_or(0.0, |m| m.stats.speed);
        Ok((target, moving))
    }
}

impl crate::unit_ai::AttackPicker for WorldPicker<'_> {
    fn candidate(
        &self,
        id: i32,
        list: Option<usize>,
    ) -> Result<Option<crate::unit_ai::AttackStat>, Unsupported> {
        let seeker = self
            .seeker
            .ok_or(Unsupported("an attack test outside bestUnitToAttack"))?;
        let (target, moving) = self.target(id)?;
        let gap = crate::unit_ai::attack_candidate(self.attacker, &seeker, &target, list)?;
        Ok(gap.map(|gap| crate::unit_ai::AttackStat { id, gap, moving }))
    }

    /// The quick path (slot 0x3e0) within the unit's range against the object (slot 0x238), with
    /// its master's terrain mask and radius on x.
    fn choose(&mut self, stats: &[crate::unit_ai::AttackStat]) -> Result<i32, Unsupported> {
        if stats.is_empty() {
            return Ok(-1);
        }
        let range = self.attacker.range_against()?;
        let terrain = self
            .spatial
            .terrain
            .as_ref()
            .ok_or(Unsupported("a quick path without the map's terrain"))?;
        let restriction = usize::try_from(self.restriction)
            .map_err(|_| Unsupported("a quick path of a master without a terrain restriction"))?;
        let mask = terrain.restriction_mask(restriction);
        let (spatial, me, radius) = (self.spatial, self.me, self.attacker.radius[0]);
        crate::unit_ai::choose_attack(stats, &mut |id| {
            crate::hull_path::quick_reach(spatial, me, id, mask, radius, range)
        })
    }
}

/// A unit's slot 0x468 (`FUN_141157c50`, `getPatrolInformation`): its top level's path
/// (`FUN_140f960f0`) copied into a new path ([`crate::tile_path::Path::append`]: its points, its
/// length or one measured) with its flags, and that level's waypoint; None without path data or
/// a path at the top.
pub(in crate::turn) fn patrol_information(
    mover: Option<&Mover>,
) -> Option<(crate::unit_ai::PatrolPath, i32)> {
    let top = mover?.paths.last().filter(|l| l.has_path)?;
    let mut path = crate::tile_path::Path::default();
    path.append(&crate::tile_path::Path {
        points: top.points.clone(),
        length: top.length,
    });
    let saved = crate::unit_ai::PatrolPath {
        path,
        flags: top.path_flags,
    };
    Some((saved, top.waypoint))
}

impl crate::unit_ai::UnitSide for UnitView<'_> {
    fn patrol_information(&self) -> Option<(crate::unit_ai::PatrolPath, i32)> {
        patrol_information(self.unit.mover.as_ref())
    }

    /// Slot 0x278 (`FUN_141158720`, [`Spatial::point_clear`]): the unit, of its radius and its
    /// restriction's terrain classes, could stand at `point`.
    fn can_stand(&mut self, point: [f32; 2]) -> Result<bool, Unsupported> {
        let unit = &*self.unit;
        let mover = unit
            .mover
            .as_ref()
            .ok_or(Unsupported("a stand test of a unit without movement state"))?;
        self.spatial.point_clear(
            unit.id,
            point,
            mover.stats.radius[0],
            mover.stats.restriction_mask,
        )
    }

    /// The base handler's case 0x216: the unit's closest spot beside the foundation `site`
    /// ([`Spatial::spot_off`] with the notify's radii; none without the foundation), its height
    /// and class, its owner, the foundation and its current action's target as orders name them.
    fn off_footprint(
        &mut self,
        site: i32,
        radius: [f32; 2],
    ) -> Result<crate::unit_ai::OffFootprint, Unsupported> {
        let unit = &*self.unit;
        let master = self
            .masters
            .and_then(|m| m.get(unit.life.master_id))
            .ok_or(Unsupported(
                "a unit sent off of a master the slice does not have",
            ))?;
        let (player, others, foreign) = (self.player, self.others, &self.foreign);
        let named = |id: i32| {
            let found = if id == unit.id {
                Some((player, unit))
            } else {
                others
                    .iter()
                    .find(|u| u.id == id)
                    .map(|u| (player, u))
                    .or_else(|| foreign.find(id))
            };
            found.map(|(p, u)| crate::unit_ai::Named {
                id,
                master: i32::from(u.life.master_id),
                owner: p as i32,
                vector: [u.position[0], u.position[2]],
            })
        };
        let spot = match self.spatial.get(site) {
            Some(placed) => {
                let mover = unit
                    .mover
                    .as_ref()
                    .ok_or(Unsupported("a unit sent off without movement state"))?;
                self.spatial.spot_off(
                    unit.position,
                    (unit.id, mover.stats.radius[0]),
                    mover.stats.restriction_mask,
                    (placed.position, radius),
                    false,
                )?
            }
            None => None,
        };
        let target = unit.top_target().filter(|&t| t != -1);
        Ok(crate::unit_ai::OffFootprint {
            spot,
            z: unit.position[2],
            class: master.class,
            computer: self.computer,
            player: player as i32,
            site: (site != -1).then(|| named(site)).flatten(),
            action_target: target.and_then(named),
        })
    }

    /// `FUN_140c1c7c0`: the flood ([`crate::unit_ai::flood_search`]) from the unit's tile over the
    /// larger of its search radius (slot 0x2c8) and its line of sight, truncated, testing each tile
    /// it reaches (`FUN_140c2b950`: the first object of its tile's list, in order, of type 80 or
    /// more in state 0 of the unit's player); −1 for none.
    fn foundation_search(&mut self) -> Result<i32, Unsupported> {
        let unit = &*self.unit;
        let master = self
            .masters
            .and_then(|m| m.get(unit.life.master_id))
            .ok_or(Unsupported("a builder of a master the slice does not have"))?;
        let r = if master.search_radius <= master.line_of_sight {
            master.line_of_sight
        } else {
            master.search_radius
        } as i32;
        let (ux, uy) = (unit.position[0] as i32, unit.position[1] as i32);
        let (player, others, masters, spatial) =
            (self.player, self.others, self.masters, &*self.spatial);
        let foundation_on = |x: i32, y: i32| -> Option<i32> {
            spatial.objects_on(x, y).find_map(|(id, p)| {
                if p.player != player {
                    return None;
                }
                let u = others.iter().find(|u| u.id == id)?;
                let kind = masters.and_then(|m| m.get(u.life.master_id))?.kind;
                (80 <= kind && u.life.state == 0).then_some(id)
            })
        };
        Ok(crate::unit_ai::flood_search((ux, uy), r, foundation_on).unwrap_or(-1))
    }

    fn seen_unit(&mut self, id: i32) -> Result<Option<crate::unit_ai::UnitSeen>, Unsupported> {
        let Some(p) = self.spatial.get(id) else {
            return Ok(None);
        };
        let (owner, u) = self
            .others
            .iter()
            .find(|u| u.id == id)
            .map(|u| (self.player, u))
            .or_else(|| self.foreign.find(id))
            .ok_or(Unsupported("an object the slice does not hold"))?;
        let m = self
            .every
            .get(owner)
            .and_then(|ms| ms.get(u.life.master_id))
            .ok_or(Unsupported("an object of a master the slice does not have"))?;
        let me = self
            .masters
            .and_then(|ms| ms.get(self.unit.life.master_id))
            .ok_or(Unsupported(
                "a unit AI's unit of a master the slice does not have",
            ))?;
        Ok(Some(crate::unit_ai::UnitSeen {
            id,
            class: m.class,
            owner,
            combat_level: m.combat_level,
            state: u.life.state,
            target: u.ai.as_ref().map(|a| a.current_target),
            gap_squared: footprint_gap_squared(self.unit.position, me.radius, p.position, p.radius),
            position: p.position,
        }))
    }

    fn closest_resource(
        &mut self,
        kind: u8,
        cap: i32,
        site: i32,
        current: i32,
    ) -> Result<Option<crate::unit_ai::Gatherable>, Unsupported> {
        let mut w = ResourceWorld {
            player: self.player,
            spatial: self.spatial,
            zones: self.zones,
            foreign: self.foreign,
            every: self.every,
            masters: self.masters,
            plan: Planning {
                pathing: self.pathing,
                now: self.world_time,
                computer: self.computer,
                object: self.object,
                construction_rate: self.construction_rate,
                groups: self.groups,
            },
        };
        let (id, _) = closest_resource_of(
            self.unit,
            &mut w,
            self.others,
            (kind, cap, site),
            (current, 2),
        )?;
        if id == -1 {
            return Ok(None);
        }
        let (owner, u) = self
            .others
            .iter()
            .find(|u| u.id == id)
            .map(|u| (self.player, u))
            .or_else(|| self.foreign.find(id))
            .ok_or(Unsupported("a resource found that the slice does not hold"))?;
        let class = self
            .every
            .get(owner)
            .and_then(|m| m.get(u.life.master_id))
            .ok_or(Unsupported("a resource found of a master the slice lacks"))?
            .class;
        Ok(Some(crate::unit_ai::Gatherable {
            id,
            position: u.position,
            class,
        }))
    }

    fn reaches(&mut self, target: i32, range: f32) -> Result<bool, Unsupported> {
        let mut plan = Planning {
            pathing: self.pathing,
            now: self.world_time,
            computer: self.computer,
            object: self.object,
            construction_rate: self.construction_rate,
            groups: self.groups,
        };
        let unit = &*self.unit;
        let walker = Me {
            id: unit.id,
            position: unit.position,
            group: unit.group.id,
            action_state: unit.top_action().map_or(0, |a| a.state),
        };
        let mover = unit
            .mover
            .as_ref()
            .ok_or(Unsupported("a reach test of a unit that does not move"))?;
        mover.reaches_object(
            &walker,
            self.spatial,
            &mut plan,
            target,
            range,
            unit.life.master_id,
        )
    }

    fn closest(&mut self, search: &Search<'_>) -> Result<crate::unit_ai::Found, Unsupported> {
        let seeker = match search.pick {
            crate::unit_ai::Pick::Attack { seeker, .. } => Some(seeker),
            _ => None,
        };
        let mut picker = WorldPicker {
            attacker: &self.attacker,
            seeker,
            me: self.unit.id,
            player: self.player,
            others: self.others,
            foreign: self.foreign,
            spatial: self.spatial,
            every: self.every,
            restriction: self
                .masters
                .and_then(|m| m.get(self.unit.life.master_id))
                .map_or(-1, |m| m.terrain_restriction),
        };
        let (others, foreign, every) = (self.others, self.foreign, self.every);
        // ClosestObject's reachability check (slot 0x3d0) walks the unit's paths
        let mut plan = Planning {
            pathing: self.pathing,
            now: self.world_time,
            computer: self.computer,
            object: self.object,
            construction_rate: self.construction_rate,
            groups: self.groups,
        };
        let unit = &*self.unit;
        let walker = Me {
            id: unit.id,
            position: unit.position,
            group: unit.group.id,
            action_state: unit.top_action().map_or(0, |a| a.state),
        };
        let (mover, master_id, spatial) =
            (unit.mover.as_ref(), unit.life.master_id, &*self.spatial);
        let mut reach = |id: i32, range: f32| {
            let m = mover.ok_or(Unsupported("a reach test of a unit that does not move"))?;
            m.reaches_object(&walker, spatial, &mut plan, id, range, master_id)
        };
        let moving = |id: i32| herded(others, foreign, id);
        // a class-10 searcher passes master 440 and the masters with object flag 0x10
        let shuns = |p: &Placed| {
            let m = every
                .get(p.player)
                .and_then(|ms| ms.get(p.master))
                .ok_or(Unsupported(
                    "a found object of a master the slice does not have",
                ))?;
            Ok(p.master == PETARD || m.object_flags & 0x10 != 0)
        };
        // a farm's workers (`+0x644`) and its pasture (`+0x790`)
        let farm = |id: i32| {
            others
                .iter()
                .find(|u| u.id == id)
                .or_else(|| foreign.find(id).map(|(_, u)| u))
                .map(|u| (u.workers, u.pasture))
        };
        spatial.search(
            unit.id,
            self.player,
            search,
            &mut picker,
            &mut reach,
            &moving,
            &shuns,
            &farm,
        )
    }

    fn may_attack(
        &mut self,
        target: i32,
        seeker: &crate::unit_ai::Seeker,
    ) -> Result<Option<(i16, [f32; 3])>, Unsupported> {
        let picker = WorldPicker {
            attacker: &self.attacker,
            seeker: Some(*seeker),
            me: self.unit.id,
            player: self.player,
            others: self.others,
            foreign: self.foreign,
            spatial: self.spatial,
            every: self.every,
            restriction: -1,
        };
        let held =
            self.foreign.find(target).is_some() || self.others.iter().any(|u| u.id == target);
        if !held {
            return Ok(None);
        }
        let (t, _) = picker.target(target)?;
        Ok(
            crate::unit_ai::can_attack(&self.attacker, seeker, &t)?
                .then_some((t.class, t.position)),
        )
    }

    /// The attack command (slot 0x338, `FUN_14120cea0`; classes 51 and 54 are not modelled) ->
    /// `FUN_141081a80`: a target the world does not hold fails; `+0x648` takes the world's time
    /// (not held); the unit's own Attack on the target answers true unless forced (its slot 0x48
    /// then, not modelled); a villager's master switch (slot 0x90) is not modelled. Then the new
    /// Attack ([`crate::attack::Attack::command`]): the constructor takes its target (slot 0x178,
    /// the caller's), the action list clears (the old actions' teardowns), and the install's start
    /// clears `+0x900`, puts the unit on its standing sprite (slot 0x68 with `+0x80`) and runs the
    /// range test; a `MoveTo` that starts arrived shows the standing sprite. A notify posted while
    /// the AI runs (its buffer, `FUN_140c55270`) is not modelled.
    fn attack(
        &mut self,
        target: i32,
        force: bool,
        ai: &UnitAi,
    ) -> Result<crate::unit_ai::Attacked, Unsupported> {
        let unit = &mut *self.unit;
        let master = self
            .masters
            .and_then(|m| m.get(unit.life.master_id))
            .ok_or(Unsupported(
                "an attacker of a master the slice does not have",
            ))?;
        if matches!(master.class, 51 | 54) {
            return Err(Unsupported(
                "the attack command of a unit of class 51 or 54",
            ));
        }
        let no = crate::unit_ai::Attacked::default;
        let Some(t) = self.spatial.get_known(target)?.copied() else {
            return Ok(no());
        };
        if unit.attack.is_some_and(|a| a.target == target) {
            if force {
                return Err(Unsupported(
                    "a forced attack command on its own Attack's target (slot 0x48)",
                ));
            }
            return Ok(crate::unit_ai::Attacked {
                attacks: true,
                posted: Vec::new(),
            });
        }
        if master.class == 4 {
            return Err(Unsupported(
                "a villager's attack command (its master switch, slot 0x90)",
            ));
        }
        let me_at = *self
            .spatial
            .get(unit.id)
            .ok_or(Unsupported("an attacker without a place"))?;
        let t_master = self
            .every
            .get(t.player)
            .and_then(|m| m.get(t.master))
            .ok_or(Unsupported(
                "an attack's target of a master the slice does not have",
            ))?;
        let (tx, ty) = (t.position[0] as i32, t.position[1] as i32);
        let visible = self.spatial.sees_tile(self.player, tx, ty)
            || (t_master.fog_visibility != 0 && self.spatial.explored(self.player, tx, ty)?);
        // the attack command's Attack (`FUN_141081a80`) takes no MoveTo sprite: the master's
        let arms = crate::attack::arms(master, self.graphics, [-1; 4], -1);
        let standing = arms
            .idle_sprite
            .ok_or(Unsupported("an attacker without its standing sprite"))?;
        if unit.sprite.is_none() {
            return Err(Unsupported(
                "an attack command of a unit whose sprite the slice does not hold",
            ));
        }
        // the action list's clearing (FUN_140a98620): an Attack on another target tears down (its
        // MoveTo's stops the unit) and lets its targets go (slot 0x180), as do the others
        let by = Releaser {
            id: unit.id,
            class: master.class,
            player: self.player,
        };
        unit.tear_down_actions(self.releases, self.builder_leaves, by)?;
        unit.attack_count = 0;
        crate::attack::set_sprite(&mut unit.sprite, standing, self.sprite);
        let mover = unit
            .mover
            .as_mut()
            .ok_or(Unsupported("an attacker without movement state"))?;
        let e = crate::attack::engagement(
            Some(ai),
            mover.stats.speed,
            master,
            &me_at,
            &t,
            self.world_time,
        )?;
        let crate::attack::Commanded {
            mut attack,
            arrived,
            posted,
            ready,
            failed,
        } = crate::attack::Attack::command(target, &e, arms, visible, t.player, mover)?;
        if failed {
            attack.fail_at_start(&mut unit.sprite, &mut unit.anim_speed, self.sprite)?;
        }
        if ready {
            return Err(Unsupported(
                "an attack command of a unit that cannot move (its set_state 6)",
            ));
        }
        if arrived {
            // the MoveTo's start in state 1 (`0x140aa29e0`, not quiet): the master's standing
            // sprite with its speed, `+0x7b8` cleared; the next update's state 4 ends the MoveTo
            mover.shows_standing();
            unit.shows_standing_sprite(self.masters, self.graphics, self.sprite)?;
        }
        unit.attack = Some(attack);
        self.takes.push((target, by));
        Ok(crate::unit_ai::Attacked {
            attacks: true,
            posted,
        })
    }

    fn set_under_attack(&mut self, value: bool) {
        self.unit.under_attack = value;
    }

    fn stop(&mut self) -> Result<(), Unsupported> {
        let masters = self.masters;
        let by = Releaser {
            id: self.unit.id,
            class: masters
                .and_then(|m| m.get(self.unit.life.master_id))
                .map_or(-1, |m| m.class),
            player: self.player,
        };
        let lets = (&mut *self.releases, &mut *self.builder_leaves);
        self.unit
            .stop_with(lets, by, masters, self.graphics, self.sprite)
    }

    fn seek_unexplored(
        &mut self,
        ai: &UnitAi,
        rng: &mut Well512,
    ) -> Result<Option<[i32; 2]>, Unsupported> {
        let tc = town_centre(self.others)?;
        let (at, computer) = (self.unit.position, self.computer);
        // the search about the town centre only while the player has no scout map
        let scout_map = self.spatial.scout_map(self.player).is_some();
        let (mut area, map, line_of_sight) = self.scout_area()?;
        if !scout_map && let Some(p) = crate::explore::unexplored_area(tc, map, &mut area)? {
            return Ok(Some(p));
        }
        // none about the town centre: the unit AI's own search (slot 0x90)
        let seeker = crate::explore::Seeker {
            at,
            line_of_sight,
            order: ai.current_order,
            target_point: [ai.target_point[0], ai.target_point[1]],
            computer,
        };
        crate::explore::closest_undiscovered(&mut area, seeker, map, rng)
    }

    fn seek_beyond_sight(&mut self, rng: &mut Well512) -> Result<Option<[i32; 2]>, Unsupported> {
        let (at, player) = (self.unit.position, self.player);
        let scouted = self.spatial.scout_map(player).is_some();
        let (mut area, map, line_of_sight) = self.scout_area()?;
        area.scouted = scouted;
        let found = crate::explore::beyond_sight(&mut area, at, line_of_sight, map, rng)?;
        // the point found is marked in the scout map (FUN_140f4d5e0 with 1)
        if let Some([x, y]) = found {
            self.spatial.scout(player, x, y);
        }
        Ok(found)
    }

    /// The player's scout map (`FUN_140c589c0`'s case 0x202 when the auto scout found nothing):
    /// a new `InfluenceMap` of 480 by 480 (`FUN_140f4a240`, the old one goes, `+0x1368` 1) marks
    /// every tile the player sees now (its visible map's `FUN_141285350` 0xf) with 1.
    fn make_scout_map(&mut self) -> Result<(), Unsupported> {
        let size = crate::explore::SCOUT_MAP_SIZE;
        let mut m = crate::explore::InfluenceMap::new([size, size], 0);
        let [w, h] = self.spatial.map;
        for y in 0..h {
            for x in 0..w {
                if self.spatial.scout_sight(self.player, x, y) == Some(0xf) {
                    m.set(x, y, 1);
                }
            }
        }
        self.spatial.set_scout_map(self.player, m);
        Ok(())
    }

    /// The unit's slot 0x388 (`FUN_141226fc0`) on the foundation `target`: the world's object
    /// (`FUN_1415417a0`; the owner's sleeping list, which the unit does not see here, holds
    /// only objects of a type below 30, never a building), then its slot 0x820 with any action
    /// type (`0xffff`) and the switch ([`task_search`] on the target and its main building);
    /// with a task the new wait replaces the unit's actions, which the caller makes after the
    /// update ([`Wait`], [`Turns::wait_for_foundation`]).
    fn wait_solidify(&mut self, target: i32) -> Result<bool, Unsupported> {
        let unit = &*self.unit;
        let (player, others, foreign) = (self.player, self.others, self.foreign);
        let find = |id: i32| {
            if id == unit.id {
                return Some((player, unit));
            }
            others
                .iter()
                .find(|u| u.id == id)
                .map(|u| (player, u))
                .or_else(|| foreign.find(id))
        };
        let (Some(t), Some(general)) = (
            task_target_with(target, 1, &find, self.every)?,
            task_target_with(target, 2, &find, self.every)?,
        ) else {
            return Ok(false);
        };
        let (masters, own) = self
            .masters
            .and_then(|m| Some((m, m.get(unit.life.master_id)?)))
            .ok_or(Unsupported(
                "a unit AI's unit of a master the slice does not have",
            ))?;
        let actor = Actor {
            id: unit.id,
            master: own,
            player,
            held: (unit.life.attribute_type, unit.life.amount),
            attribute_95: self.attributes.get(95).copied().unwrap_or(0.0),
            civ: self.civ,
        };
        let dip = SideWorld {
            player,
            relations: self.relations,
            foreign,
            drop_sites: self.drop_sites,
        };
        let Some((task, switch)) = task_search(masters, &actor, &t, &general, -1, &dip)? else {
            return Ok(false);
        };
        self.waits.push(Wait {
            target,
            task,
            switch: switch.map(|m| m.id),
        });
        Ok(true)
    }
}

impl UnitView<'_> {
    /// What an explorer's searches ask of the world, the map's size and the unit's line of sight.
    pub(in crate::turn) fn scout_area(
        &mut self,
    ) -> Result<(ScoutArea<'_>, [i32; 2], f32), Unsupported> {
        let unit = &*self.unit;
        let master = self
            .masters
            .and_then(|m| m.get(unit.life.master_id))
            .ok_or(Unsupported(
                "an auto scout of a master the slice does not have",
            ))?;
        let mover = unit
            .mover
            .as_ref()
            .ok_or(Unsupported("an auto scout without movement state"))?;
        let [w, h] = self.spatial.map;
        let terrain = self
            .spatial
            .terrain
            .as_ref()
            .ok_or(Unsupported("the map's zones without its terrain"))?;
        let values = terrain
            .restrictions
            .get(mover.stats.restriction)
            .ok_or(Unsupported("a restriction the slice's data has not got"))?;
        let zones = self.zones.of(w, h, &terrain.kinds, values);
        let me = Me {
            id: unit.id,
            position: unit.position,
            group: unit.group.id,
            action_state: unit.top_action().map_or(0, |a| a.state),
        };
        let area = ScoutArea {
            spatial: self.spatial,
            zones,
            plan: Planning {
                pathing: self.pathing,
                now: self.world_time,
                computer: self.computer,
                object: self.object,
                construction_rate: self.construction_rate,
                groups: self.groups,
            },
            mover,
            me,
            player: self.player,
            master: master.id,
            radius: master.radius,
            reach: master.line_of_sight - LOS_MARGIN,
            scouted: false,
        };
        Ok((area, [w, h], master.line_of_sight))
    }
}

impl Context<'_> {
    /// A walk and the Sprite stream, for an attacker's update.
    pub(in crate::turn) fn walk_and_sprites<'b>(
        &'b mut self,
        mover: Option<&'b mut Mover>,
        me: Me,
    ) -> (Walk<'b>, &'b mut Well512) {
        let (object, sprite) = self.streams.two(Stream::Object, Stream::Sprite);
        (
            Walk {
                mover,
                me,
                dt: self.dt_seconds,
                world: self.spatial,
                plan: Some(Planning {
                    pathing: self.pathing,
                    now: self.world_time,
                    computer: self.computer,
                    object,
                    construction_rate: self.construction_rate,
                    groups: self.groups,
                }),
            },
            sprite,
        )
    }

    /// A walk with the Sprite and the Action streams (a farmer's stops draw from the second).
    pub(in crate::turn) fn walk_sprites_action<'b>(
        &'b mut self,
        mover: Option<&'b mut Mover>,
        me: Me,
    ) -> (Walk<'b>, &'b mut Well512, &'b mut Well512) {
        let [object, sprite, action] =
            self.streams
                .three(Stream::Object, Stream::Sprite, Stream::Action);
        (
            Walk {
                mover,
                me,
                dt: self.dt_seconds,
                world: self.spatial,
                plan: Some(Planning {
                    pathing: self.pathing,
                    now: self.world_time,
                    computer: self.computer,
                    object,
                    construction_rate: self.construction_rate,
                    groups: self.groups,
                }),
            },
            sprite,
            action,
        )
    }

    /// A walk and another stream than the walk's Object stream.
    pub(in crate::turn) fn walk_with<'b>(
        &'b mut self,
        mover: Option<&'b mut Mover>,
        me: Me,
        other: Stream,
    ) -> (Walk<'b>, &'b mut Well512) {
        let (object, other) = self.streams.two(Stream::Object, other);
        (
            Walk {
                mover,
                me,
                dt: self.dt_seconds,
                world: self.spatial,
                plan: Some(Planning {
                    pathing: self.pathing,
                    now: self.world_time,
                    computer: self.computer,
                    object,
                    construction_rate: self.construction_rate,
                    groups: self.groups,
                }),
            },
            other,
        )
    }

    /// What an object's walking action needs from the tick.
    pub(in crate::turn) const fn walk<'b>(
        &'b mut self,
        mover: Option<&'b mut Mover>,
        me: Me,
    ) -> Walk<'b> {
        Walk {
            mover,
            me,
            dt: self.dt_seconds,
            world: self.spatial,
            plan: Some(Planning {
                pathing: self.pathing,
                now: self.world_time,
                computer: self.computer,
                object: self.streams.get(Stream::Object),
                construction_rate: self.construction_rate,
                groups: self.groups,
            }),
        }
    }
}

/// The auto scout's search reaches within its line of sight less this (`FUN_140c65490`,
/// `DAT_14330eb8c`).
pub(in crate::turn) const LOS_MARGIN: f32 = crate::image::explore::LOS_MARGIN;

/// The masters of a town centre (`FUN_1416597e0`: 109, then 444).
pub(in crate::turn) const TOWN_CENTRES: [i16; 2] = [109, 444];

/// `FUN_1416597e0` with 0: the player's first town centre, by the player's index of objects by
/// master (`player+0x18`, `FUN_14104f560`: those of master 109, then 444), as its tile. The
/// index's order within a master is not held: two objects of the first master found are not
/// modelled.
pub(in crate::turn) fn town_centre(units: &[Unit]) -> Result<Option<[i32; 2]>, Unsupported> {
    for master in TOWN_CENTRES {
        let mut found = units.iter().filter(|u| u.life.master_id == master);
        let Some(first) = found.next() else { continue };
        if found.next().is_some() {
            return Err(Unsupported(
                "two town centres (the order of the player's index)",
            ));
        }
        return Ok(Some([first.position[0] as i32, first.position[1] as i32]));
    }
    Ok(None)
}

/// What the auto scout's search asks of the world for one unit ([`crate::explore::Area`]).
pub(in crate::turn) struct ScoutArea<'a> {
    pub(in crate::turn) spatial: &'a Spatial,
    pub(in crate::turn) zones: &'a crate::zones::ZoneMap,
    pub(in crate::turn) plan: Planning<'a>,
    pub(in crate::turn) mover: &'a Mover,
    pub(in crate::turn) me: Me,
    pub(in crate::turn) player: usize,
    pub(in crate::turn) master: i16,
    pub(in crate::turn) radius: [f32; 2],
    /// The reach of slot 0x3d8: the line of sight less a tile.
    pub(in crate::turn) reach: f32,
    /// The search reads the player's scout map for the tiles seen (`FUN_140c4b130` with one).
    pub(in crate::turn) scouted: bool,
}

impl crate::explore::Area for ScoutArea<'_> {
    fn explored(&self, x: i32, y: i32) -> Result<bool, Unsupported> {
        if self.scouted {
            return self
                .spatial
                .scout_map(self.player)
                .and_then(|m| m.get(x, y))
                .map(|v| v == 1)
                .ok_or(Unsupported("a tile off the player's scout map"));
        }
        self.spatial.explored(self.player, x, y)
    }

    fn zone(&mut self, x: i32, y: i32) -> u16 {
        self.zones.zone(x, y)
    }

    fn own_zone(&mut self) -> u16 {
        let [x, y, _] = self.me.position;
        self.zones.zone(x as i32, y as i32)
    }

    fn clear(&self, x: i32, y: i32) -> Result<bool, Unsupported> {
        let mask = self.mover.stats.restriction_mask;
        self.spatial
            .point_clear(self.me.id, [x as f32, y as f32], self.radius[0], mask)
    }

    fn reaches(&mut self, x: i32, y: i32) -> Result<bool, Unsupported> {
        let to = [x as f32, y as f32];
        self.mover.reaches(
            &self.me,
            self.spatial,
            &mut self.plan,
            to,
            self.reach,
            self.master,
        )
    }

    /// `FUN_14118faf0`: the gap from the footprint on each axis (at least 0), then its length.
    fn distance(&self, x: i32, y: i32) -> f32 {
        let gap = |d: f32, r: f32| {
            let g = d.abs() - r;
            if g <= 0.0 { 0.0 } else { g }
        };
        let gx = gap(x as f32 - self.me.position[0], self.radius[0]);
        let gy = gap(y as f32 - self.me.position[1], self.radius[1]);
        crate::fmath::sqrt(gy * gy + gx * gx)
    }
}
