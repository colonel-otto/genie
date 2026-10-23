//! The per-tick skeleton the unit-AI slice needs: the frame-seed chain and the unit-AI turn.
//!
//! 158041 `BaseWorld::update` (`0x14154fc70`), in order: advance the world time by dt,
//! `SeedAll(frame seed)`, the player loop (each player's time slot `world+0x18c[p]` gains dt; only the
//! turn player `world+0x188` runs the full unit-AI update), `s1 = World.rand15`, `SeedAll(s1)`, zero
//! the turn player's slot and pass the turn on, then the next frame seed `= World.rand15`. The slice
//! assumes no other World draw runs in a tick (no triggers, XS or unit groups).
//!
//! A player's update walks its live list in order (`RGE_Object_List::update`, `0x141166550`). Each
//! object moves by its velocity ([`Mover::step_velocity`]), runs its static state step
//! ([`crate::lifecycle::Lifecycle`]), then its actions (the livestock default action, a builder's
//! Build action with its walk), then its unit AI. An object whose update returns 1 leaves: the
//! last object takes its slot and updates there in the same tick.
//!
//! The module's files: the commands and their dispatch (`command`), an object and its update
//! (`unit`), a player (`player`), the views a unit's update reads the world through (`view`), the
//! lookups and releases across the players (`world`).

mod command;
mod player;
mod unit;
mod view;
mod world;

use crate::lifecycle::seconds;
use crate::masters::Masters;
use crate::movement::Mover;
use crate::pathing::Pathing;
use crate::rng::{Stream, Streams};
use crate::spatial::Spatial;
use crate::sprite::Graphic;
use crate::tasks::DropSite;
pub use crate::turn::command::{Command, GatherPoint, Order};
pub use crate::turn::player::{Formations, Player};
pub(crate) use crate::turn::unit::EnterWorld;
pub use crate::turn::unit::Unit;
pub(crate) use crate::turn::unit::annexes_of;
use crate::turn::unit::{AiResume, Captured, Step, same_members};
pub(crate) use crate::turn::view::DropWorld;
use crate::turn::view::{Context, Foreign};
pub(crate) use crate::turn::world::{
    Releaser, World, built_death, combat_dies, count_class, farm_bonus, forget_resource, refund_of,
    released, storage_leaves, victim_of,
};
use crate::unit_ai::Unsupported;

/// What every building holds of a gate: `mGateLocked` (`+0xc7c`; a lock also sets the
/// building's last move time, [`Mover::last_move`], slot 0x670, `FUN_1411dba30`), `mFirstUpdate`
/// (`+0xc80`) and `mCloseTimer` (`+0xc84`) ([`Unit::gate_update`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Gate {
    /// `mGateLocked`: the gate is locked.
    pub locked: bool,
    /// `mFirstUpdate`: a new building's is set; a gate's first update in state 2 takes its
    /// annexes' places as its endpoints (`mEndpoint1`, `mEndpoint2`, not held) and clears it.
    pub first_update: bool,
    /// `mCloseTimer`: the seconds an open gate with nothing in it waits before it shuts.
    pub close_timer: f32,
    /// The building has a saved open or shut master (`+0xc58`, the save's
    /// `has_open_closed_master_object`) or its original owner's masters (`mOriginalOwner`,
    /// `+0xc88`): a gate's toggle then takes another master, not modelled. A master of its own
    /// (`+0x6c0`, which a foundation holds until its first builder starts) the slice does not
    /// hold.
    pub other_masters: bool,
}

/// A gate's height (its master's `radius_z`) above this shuts it (`DAT_142eb0c60`); a unit
/// slower than it counts in a gate only when it is a player's.
const GATE_EPSILON: f32 = crate::image::turn::GATE_EPSILON;
/// A gate's query grows its records and its own box by this (`FUN_1410562d0` with
/// `DAT_142eb3630`).
const GATE_GROW: f32 = crate::image::turn::GATE_GROW;
/// The least height of a shut gate its player opens (`FUN_14162f7e0`, `DAT_14330eb8c`).
const GATE_SHUT_HEIGHT: f32 = crate::image::turn::GATE_SHUT_HEIGHT;
/// The seconds an open gate with nothing in it waits before it shuts (`FUN_1411e7730`'s
/// immediate `0x3f000000`).
const GATE_CLOSE_DELAY: f32 = 0.5;
/// The most records a gate's query lists (`FUN_1411d0570`'s list of 10) and the most units it
/// counts (25).
const GATE_QUERY_RECORDS: usize = 10;
const GATE_COUNT_CAP: usize = 25;
/// The world constructor's history period (`FUN_141599e50`'s immediate 0x19 at `world+0x8b8`),
/// until the `.dat`'s `time_slice` replaces it.
pub const HISTORY_PERIOD: i32 = 25;

/// The units in gate `gate` (`FUN_1411d0570`): the obstruction manager's query from its first
/// annex to its second (`+0xbc0`, `+0xbc8`) with flags 0x10, the gate and its annexes ignored
/// (`FUN_1410562e0`), every record and the query grown by [`GATE_GROW`] (`FUN_1410562d0`), any
/// kind and player ([`Spatial::walk_hits`]; its list holds 10 records, more are not modelled).
/// Of the objects it lists, those not of the classes 27, 39 and 52, nor of class 3 standing
/// still, and either moving (a top speed of at least [`GATE_EPSILON`]) or not Gaia's count, at
/// most 25; the second answer is how many of them are of a player the gate's counts an ally
/// (`FUN_141663d90` with the ally flag).
pub(crate) fn gate_units(
    spatial: &Spatial,
    gate: i32,
    annexes: &[i32],
) -> Result<(Vec<i32>, usize), Unsupported> {
    let [a, b] = annexes[..] else {
        return Err(Unsupported("a gate without its two annexes"));
    };
    let place = |id| {
        spatial
            .get(id)
            .map(|p| [p.position[0], p.position[1]])
            .ok_or(Unsupported("a gate's annex without a place"))
    };
    let (from, to) = (place(a)?, place(b)?);
    let rect = [
        from[0].min(to[0]),
        from[1].min(to[1]),
        from[0].max(to[0]),
        from[1].max(to[1]),
    ];
    let owner = spatial
        .get(gate)
        .map(|p| p.player)
        .ok_or(Unsupported("a gate without a place"))?;
    spatial.check_known_records([
        rect[0] - GATE_GROW,
        rect[1] - GATE_GROW,
        rect[2] + GATE_GROW,
        rect[3] + GATE_GROW,
    ])?;
    let hits = spatial.walk_hits(from, to, rect, GATE_GROW, 1.0, Some(owner), &[gate, a, b])?;
    if GATE_QUERY_RECORDS < hits.len() {
        return Err(Unsupported("a gate's query of more than 10 records"));
    }
    let (mut units, mut allied) = (Vec::new(), 0);
    for id in hits {
        if GATE_COUNT_CAP <= units.len() {
            break;
        }
        let Some(p) = spatial.get(id) else {
            continue;
        };
        let counts = !matches!(p.class, 27 | 39 | 52)
            && (p.class != 3 || p.speed != 0.0)
            && (GATE_EPSILON <= p.speed || p.player != 0);
        if counts {
            units.push(id);
            if spatial.allied_with(owner, p.player) {
                allied += 1;
            }
        }
    }
    Ok((units, allied))
}

/// The building unit AI's action value a research it queued sets (`FUN_140c64b50` with 0x26c).
pub(crate) const RESEARCH_ACTION: i32 = 0x26c;
/// The building top actions that keep RESEARCH from a building (`FUN_141227a00`: 0x7b to 0x7d).
const BLOCKS_RESEARCH: std::ops::RangeInclusive<i16> = 0x7b..=0x7d;
/// The building master type RESEARCH asks for (`'P'`, 80).
const BUILDING_KIND: u8 = 80;
/// The first master type whose slot 0x128 (a work at an object) does something: the action
/// objects' (`FUN_14106c590`) and up; below it `FUN_141071bd0` returns at once.
const STATIC_TYPES_END: u8 = 40;
/// The buildings' paid costs (attribute 247), which the combat setup adds for a building with a
/// train location (`FUN_14121df60`).
const BUILDING_COST: i16 = 247;
/// The masters whose restrictions the world's load gives planners (`World::load_world`'s two
/// calls of `FUN_140f7d310`: Gaia's entries `+0x298` and `+0x10b8`).
const PLANNED_MASTERS: [i16; 2] = [83, 535];

/// The masters whose death lowers a count of their owner's (`FUN_1411dbb70`, raw `0x6104`,
/// `0x61b0`, `0x60ee`): the monastery's ([`MONASTERY`]) in attribute 52, the wonder's in 42,
/// the castle's in 134.
const WONDER: i16 = 276;
const CASTLE: i16 = 82;
const MONASTERIES: i16 = 52;
const WONDERS: i16 = 42;
const CASTLES: i16 = 134;
/// The foundation terrain a building's death covers with terrain 1 (`FUN_1411dbb70`: `0x1c`).
const RESTORED_FOUNDATION: i16 = 28;

/// What a built building's death leaves its owner's update to do after it
/// ([`Unit::building_died`]): the count attribute it lowers, and its master with the master's
/// technology.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BuildingDeath {
    pub(crate) count: Option<i16>,
    pub(crate) tech: Option<(i16, i16)>,
}

/// The villager master QUEUE draws a sex for (`0x53`), and the generic villager id that becomes
/// 0x53 first (`0x712`), both in `FUN_1411cbc40`.
const VILLAGER: i16 = 0x53;
/// The farm, which BUILD places as the rice farm on mangrove or rice farm terrain.
const FARM: i16 = 50;
const RICE_FARM: i16 = 1187;
const VILLAGER_ANY: i16 = 0x712;
/// The female villager master (`0x125`).
const VILLAGER_FEMALE: i16 = 0x125;
/// The attribute naming a building master that changes the terrain around its buildings
/// (`FUN_1411c39b0`, the player's attribute 255).
const BUILDING_TERRAIN: usize = 255;
/// A building at most this share of its hit points lets its garrison out (`DAT_142eae8d0`).
const EJECT_SHARE: f32 = crate::image::turn::EJECT_SHARE;
/// The monastery (master 104, `0x6104` raw), whose relics give goods every second.
const MONASTERY: i16 = 104;
/// The petard (master 440, `0x6254` raw), which a class-10 searcher passes (`FUN_140c488b0`).
const PETARD: i16 = 440;
/// The villager's class (4), which a building's repair by its garrison counts.
const VILLAGER_CLASS: i16 = 4;
/// The building masters `FUN_140fde180` counts as drop sites besides the drop-site table's:
/// 562, 584 and 621 (`0x232` plus the bits of `0x800000000400001`), 68, 1808, 50, 1187 and 199.
const DROP_SITE_MASTERS: [i16; 8] = [562, 584, 621, 68, 1808, 50, 1187, 199];
/// A building's queue holds at most 15 units (`FUN_1411cbc40`).
const QUEUE_LIMIT: usize = 15;
/// A computer player's QUEUE fills a building's queue to at most 2 (`FUN_141614450`).
const COMPUTER_QUEUE_LIMIT: usize = 2;
/// The game type whose QUEUE trains one unit in the first age (`FUN_141614450`, the settings'
/// `+0x34`).
const ONE_UNIT_GAME: u8 = 0x10;
/// The Hunt's search timer: it tells its unit AI again every 10 seconds (`0xc1200000`, in
/// `0x140ac60e0` and `0x140ac7a00`).
const SEARCH_AGAIN: f32 = -10.0;
/// `FUN_140ac4670`'s first argument when a hunt's target went on the walk to it, or its `MoveTo`
/// failed (`DAT_142eb9430`): the hunt searches within this of its point.
const TREE_SEARCH_WITHIN: f32 = crate::image::turn::TREE_SEARCH_WITHIN;
/// `FUN_140ac4670`'s second argument when a hunt's Attack failed (`DAT_14330ebb8`): the tree
/// found counts within this of the unit.
const TREE_NEAR: f32 = crate::image::hunt::TREE_NEAR;
/// The notify "Action %d has been invalidated or requires a search." (0x202) with the action's
/// value: a Build waiting without work posts it ([`BUILD_ACTION`]), a searching Hunt too.
const SEARCH_NEEDED: i32 = 0x202;
/// A velocity longer than this sets the animation speed (`DAT_142eb0c60`).
const SPEED_EPSILON: f32 = crate::image::turn::SPEED_EPSILON;
/// A building attacks with a firepower above this (`FUN_14120ea30`).
const ARMED_FIREPOWER: f32 = crate::image::turn::ARMED_FIREPOWER;

/// The pasture master whose foundation keeps its snow and whose stack unit depends on the rice
/// terrain under it (1889).
const PASTURE_1889: i16 = 1889;
/// The gate's class (39), whose annexes stay in their owner's watched list.
const GATE: i16 = 39;
/// A queued villager's master: male when the 64-bit Action draw times 100 (wrapping) is below 50
/// modulo 100.
const fn villager(draw: u64) -> i16 {
    if draw.wrapping_mul(100) % 100 < 50 {
        VILLAGER
    } else {
        VILLAGER_FEMALE
    }
}

/// The game's state after a tick, for the oracle's forced replay (dev only: the oracle reads it
/// from the tick oracle's saves, genie-core never reads the game): the frame seed and every
/// player's live units and resources as the game holds them.
#[derive(Clone, Debug, Default)]
pub struct GameState {
    /// `world+0x7d0`, the frame seed.
    pub frame_seed: u32,
    /// Every player's live objects.
    pub players: Vec<Vec<Unit>>,
    /// Every player's food, wood, stone and gold.
    pub resources: Vec<[f32; 4]>,
    /// Every player's attributes as saved (the first four are the resources), which the oracle
    /// compares; forcing leaves them alone.
    pub attributes: Vec<Vec<f32>>,
    /// The players' pending debits and technology states.
    pub pending_debit: Vec<[f32; 4]>,
    /// Every player's technology states.
    pub techs: Vec<Vec<crate::tech::TechState>>,
    /// Every player's game status and resignation (`+0x89`, `+0x8b`).
    pub statuses: Vec<(u8, bool)>,
    /// The world's game state (`+0x88`).
    pub game_state: u32,
    /// Objects the slice modelled but got wrong this tick: they take the game's state too.
    pub resync: Vec<i32>,
    /// The players' visible maps and the unified map, when the slice's differ.
    pub sight: Option<(Vec<crate::visibility::VisibleMap>, Vec<u64>)>,
    /// `next_reusable_object_id`: the fog ghosts that take reusable ids are not modelled.
    pub next_reusable_id: i32,
    /// The map tiles' object lists in the game's order (`x`, `y`, ids; each tile of two objects
    /// or more), when the oracle dumped them: the order is the game's history, no save holds it.
    pub tile_lists: Option<Vec<(i32, i32, Vec<i32>)>>,
    /// The pool of recycled missiles ([`Recycling::missiles`]), when the oracle dumped it: no
    /// save holds it.
    pub missile_pool: Option<Vec<(i32, bool)>>,
    /// The world's pool of recycled ghosts, when the oracle dumped it.
    pub ghost_pool: Option<Vec<i32>>,
    /// The world's unit groups, when the slice's differ.
    pub groups: Option<crate::group::UnitGroups>,
}

impl Turns {
    /// The objects' inside lists in the game's order (`+0x2f8`, the tick oracle's dump): each
    /// container listed takes the order when it holds the same objects. Returns how many did.
    pub fn force_garrisons(&mut self, lists: &[(i32, Vec<i32>)]) -> usize {
        let mut n = 0;
        for (id, order) in lists {
            let Some(c) = self
                .players
                .iter_mut()
                .flat_map(|p| p.objects.iter_mut().chain(p.sleeping.iter_mut()))
                .find(|u| u.id == *id)
            else {
                continue;
            };
            if same_members(&c.garrison, order) {
                c.garrison.clone_from(order);
                n += 1;
            }
        }
        n
    }

    /// The forced replay's step after a tick: the frame seed is the game's, and so is every
    /// object the slice did not model (it froze, or its player stopped) or got wrong (`resync`),
    /// and the players' sight where it differs: its unit is the game's
    /// (with what the save does not hold, a hunted animal's origin and a boar's flag, kept),
    /// placed where the game has it, and it runs again from the next tick; a frozen object the
    /// game no longer holds goes. A stopped player takes the game's resources and runs again.
    /// The world's next reusable id is the game's (the fog ghosts that take them are not
    /// modelled). A stopped player's pending debit and technologies are the game's too, and so
    /// is the tiles' objects' order where the game's dump has it, and a forced object's inside
    /// list keeps the slice's order while it holds the same objects. Returns
    /// what could not be forced (an object that changed hands or masters, or
    /// that the game made and the slice did not). Not part of a replay: a dev hook to see past a
    /// freeze.
    pub fn force(&mut self, game: &GameState) -> Vec<(usize, i32, Unsupported)> {
        let mut notes = Vec::new();
        self.frame_seed = game.frame_seed;
        self.game_state = game.game_state;
        self.recycling.next_id = game.next_reusable_id;
        if let Some(pool) = game.ghost_pool.as_ref() {
            self.recycling.ghosts.clone_from(pool);
        }
        if let Some(pool) = game.missile_pool.as_ref() {
            self.recycling.missiles.clone_from(pool);
        }
        if let Some(groups) = game.groups.as_ref() {
            self.groups.clone_from(groups);
        }
        if let (Some((maps, unified)), Some(sight)) =
            (game.sight.as_ref(), self.spatial.sight.as_mut())
        {
            sight.maps.clone_from(maps);
            sight.unified.clone_from(unified);
        }
        // the objects the slice holds, sorted: forcing only takes away objects the game no
        // longer holds
        let mut known: Vec<i32> = self
            .players
            .iter()
            .flat_map(|q| q.objects.iter().chain(&q.sleeping).map(|u| u.id))
            .collect();
        known.sort_unstable();
        for p in 0..self.players.len() {
            let Some(theirs) = game.players.get(p) else {
                continue;
            };
            let stopped = self.players[p].stopped;
            if stopped {
                if let Some(r) = game.resources.get(p) {
                    self.players[p].resources = *r;
                }
                if let Some(d) = game.pending_debit.get(p) {
                    self.players[p].pending_debit = *d;
                }
                if let Some(t) = game.techs.get(p) {
                    self.players[p].techs.clone_from(t);
                }
                self.players[p].stopped = false;
            }
            if let Some(&(status, resigned)) = game.statuses.get(p) {
                self.players[p].game_status = status;
                self.players[p].resigned = resigned;
            }
            // each update's first unmodelled part is its own: a freeze's would hide a later stop
            self.players[p].unsupported = None;
            let mut i = 0;
            while i < self.players[p].objects.len() {
                let unit = &self.players[p].objects[i];
                let resync = game.resync.contains(&unit.id);
                let (id, frozen) = (unit.id, stopped || unit.stopped || resync);
                // a dead object never freezes, but one that differs takes the game's state too
                // (a felled tree's wood that a frozen gatherer did not take)
                if !frozen || (!unit.life.is_alive() && !resync) {
                    i += 1;
                    continue;
                }
                let Some(g) = theirs.iter().find(|g| g.id == id) else {
                    let elsewhere = game.players.iter().any(|q| q.iter().any(|g| g.id == id));
                    if elsewhere {
                        notes.push((p, id, Unsupported("a forced object changed hands")));
                        i += 1;
                        continue;
                    }
                    let gone = self.players[p].objects.swap_remove(i);
                    if let Err(e) = self.leave_world(p, &gone) {
                        notes.push((p, id, e));
                    }
                    continue;
                };
                if g.life.master_id != unit.life.master_id {
                    notes.push((p, id, Unsupported("a forced object changed masters")));
                    i += 1;
                    continue;
                }
                let mut forced = g.clone();
                // the objects inside it keep the slice's order (the game's history, which no save
                // holds) while they are the same
                if same_members(&unit.garrison, &forced.garrison) {
                    forced.garrison.clone_from(&unit.garrison);
                }
                if let (Some(old), Some(new)) = (unit.ai.as_ref(), forced.ai.as_mut()) {
                    new.hunted = old.hunted;
                    if let (Some(o), Some(n)) = (old.home, new.home.as_mut()) {
                        n.go_home = o.go_home;
                    }
                }
                let (before, state) = (unit.life.state, forced.life.state);
                if forced.position != unit.position {
                    self.spatial.set_position(id, forced.position);
                }
                if state != before {
                    if before <= 2 && 3 <= state {
                        self.spatial.sight_state(id, 3);
                    }
                    if state != 3 {
                        self.spatial.sight_state(id, state);
                    }
                    if before < 4 && 6 < state {
                        let master = self
                            .masters
                            .get(p)
                            .and_then(|m| m.get(forced.life.master_id));
                        forget_resource(&mut self.spatial, master, id, forced.position);
                    }
                }
                self.spatial.unfreeze(id);
                self.spatial.set_gait(id, forced.gait());
                if let Some(placed) = self.spatial.get(id).copied() {
                    let moving = forced.mover.as_ref().is_some_and(Mover::is_moving);
                    self.pathing.moved(id, &placed, moving);
                }
                forced.stopped = false;
                self.players[p].objects[i] = forced;
                i += 1;
            }
            for g in theirs {
                if known.binary_search(&g.id).is_err() {
                    notes.push((
                        p,
                        g.id,
                        Unsupported("an object the game made and the slice did not"),
                    ));
                }
            }
        }
        if let Some(lists) = game.tile_lists.as_ref() {
            // the tiles whose objects differ keep the slice's order: their objects' places
            // differ, and that is reported
            let _ = self.spatial.force_tile_lists(lists);
        }
        self.pathing.tiles_changed(self.spatial.take_changes());
        notes
    }
}

/// World state the slice carries from tick to tick. All of it is in the header save.
#[derive(Clone, Debug, PartialEq)]
pub struct Turns {
    /// The world's time in milliseconds (`world_time`).
    pub world_time: u32,
    /// `world+0x7d0`.
    pub frame_seed: u32,
    /// `world+0x188`, `player_turn` in the save.
    pub player_turn: usize,
    /// `world+0x18c[p]`, the time each player accumulated since its last turn.
    pub player_time: Vec<u32>,
    /// Every player, Gaia first.
    pub players: Vec<Player>,
    /// The updates run so far.
    pub ticks: u32,
    /// `mNextObjectId`, the id the next new object gets.
    pub next_object_id: i32,
    /// Each player's masters, which BUILD reads; a player without them stops at its own BUILD.
    pub masters: Vec<Masters>,
    /// Every object's place on the map.
    pub spatial: Spatial,
    /// The planners and the mesh.
    pub pathing: Pathing,
    /// What the sprite clock reads of each graphic (the `.dat`'s).
    pub graphics: std::collections::BTreeMap<i16, Graphic>,
    /// The game's drop-site table (`dropsites.json`).
    pub drop_sites: Vec<DropSite>,
    /// The game's unit lines (`unitlines.json`, [`crate::lines`]); empty when not loaded.
    pub lines: crate::lines::UnitLines,
    /// The game's linked technologies (`linkedTechs.json`, the table at the game settings'
    /// `+0x18` that `FUN_140f39f40` looks up), by technology; empty when not loaded.
    pub linked_techs: std::collections::BTreeMap<i16, crate::tech::Linked>,
    /// The game options the simulation reads ([`crate::settings::Settings`]), from the header.
    pub settings: crate::settings::Settings,
    /// The game object's tagged value at `+0x680` (`DAT_143f07a58`, read through
    /// `FUN_140ec0cd0`; at 1 a boar does not wander and a hit may do no damage). `GamePC::start4`
    /// (`FUN_140f29b60`) compares it with the loaded world's `+0x6b4` and reloads the data when
    /// they differ. It is not the options' content mode: MarvinBlue/426048731 saves
    /// `mSelectedContentMode` 2 and the tick oracle reads this as 0. Its writer is not located
    /// and the save does not hold it; genie-oracle leaves it at 0.
    pub mode_680: i32,
    /// The Empire Wars start's pass is due ([`crate::start_work`]): the world's `+0x6c6` is
    /// clear in game mode 13, which a game's start has.
    pub start_work_due: bool,
    /// The world's game state (`+0x88`, the save's `tempGameState`): 1 once the game is over.
    pub game_state: u32,
    /// The scenario's `victory_conquest` (`world+0x7e0`'s `+0x2c0`, `FUN_140ca4d20`): the game
    /// ends when one side is left.
    pub victory_conquest: bool,
    /// The map's zone maps (`map+0x5e28`), made when first asked for.
    pub zones: crate::zones::Zones,
    /// The world's ids and pools for the objects of recyclable masters ([`Recycling`]).
    pub recycling: Recycling,
    pub(crate) streams: Streams,
    /// The `.dat`'s technologies, by id.
    pub techs: Vec<crate::tech::TechDef>,
    /// The world's unit groups ([`crate::group`]).
    pub groups: crate::group::UnitGroups,
    /// The current update's time in seconds (`world+0x164`, set before the commands run).
    pub dt_seconds: f32,
    /// The current update's time in milliseconds (`world+0x7c`, set before the commands run).
    pub dt: u32,
    /// The history period (`world+0x8b8`, `time_slice`): the `.dat`'s, which the world reads
    /// after its constructor's [`HISTORY_PERIOD`] ([`crate::history::History::update`]).
    pub history_period: i32,
    /// Each player's `mAllowCapturingGaiaUnits` (`player+0x670`, saved; false for a player it
    /// lacks): a computer player takes Gaia's objects over by seeing them in the game modes
    /// outside the standard ones ([`crate::visibility::sight_taker`]).
    pub allow_gaia_capture: Vec<bool>,
}

/// The objects of recyclable masters (`+0x680`, the `.dat`'s `recyclable`): a new one takes the
/// world's next reusable id (`FUN_1415301b0`), counting down from −1; one that leaves its list
/// goes to the world's pool for its type (`RGE_Static_Object::recycle_out_of_game`,
/// `FUN_141546960`), from which the next of that type comes back, the last in first
/// (`FUN_141546870`), keeping its id (`recycle_in_to_game` gives none). The pools are not saved:
/// a game loaded from a save starts them empty.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Recycling {
    /// `next_reusable_object_id`.
    pub next_id: i32,
    /// The pool of type 60 (`world+0x1d8`): each missile's id and its flying flag (`+0x994`,
    /// which the object keeps), the last in last.
    pub missiles: Vec<(i32, bool)>,
    /// The pool of type 25 (`world+0x1e0`): the ghosts' ids, the last in last.
    pub ghosts: Vec<i32>,
}

impl Turns {
    /// The world as the header save holds it: its time, frame seed, turn order and players.
    #[must_use]
    pub fn new(
        world_time: u32,
        frame_seed: u32,
        player_turn: usize,
        player_time: Vec<u32>,
        players: Vec<Player>,
    ) -> Self {
        Self {
            world_time,
            frame_seed,
            player_turn,
            player_time,
            players,
            ticks: 0,
            next_object_id: 0,
            masters: Vec::new(),
            spatial: Spatial::default(),
            pathing: Pathing::default(),
            graphics: std::collections::BTreeMap::default(),
            drop_sites: Vec::new(),
            lines: crate::lines::UnitLines::default(),
            linked_techs: std::collections::BTreeMap::new(),
            settings: crate::settings::Settings::default(),
            start_work_due: false,
            mode_680: 0,
            game_state: 0,
            victory_conquest: false,
            zones: crate::zones::Zones::default(),
            recycling: Recycling::default(),
            streams: Streams::default(),
            techs: Vec::new(),
            groups: crate::group::UnitGroups::default(),
            dt_seconds: 0.0,
            dt: 0,
            history_period: HISTORY_PERIOD,
            allow_gaia_capture: Vec::new(),
        }
    }

    /// Player `q`'s gate list (`+0x1360`, `FUN_141616200` adds to its front): its live objects
    /// of class 39 whose master transforms ([`crate::masters::BuildingMaster::transform_unit`]),
    /// newest first.
    fn gate_list(&self, q: usize) -> Vec<usize> {
        let masters = self.masters.get(q);
        let owner = &self.players[q];
        (0..owner.objects.len())
            .rev()
            .filter(|&i| {
                masters
                    .and_then(|m| m.get(owner.objects[i].life.master_id))
                    .is_some_and(|m| {
                        m.class == crate::spatial::GATE_CLASS
                            && m.building.as_ref().is_some_and(|b| 0 <= b.transform_unit)
                    })
            })
            .collect()
    }

    /// A gate's slot 0x678 (`FUN_1411d4a30`): player `p` may pass object `i` of player `q` when it
    /// stands (state below 3), its master is of class 39 and transforms (`+0x9ba` not -1), it is
    /// not locked (`mGateLocked`) and its owner and `p` count each other as allies
    /// (`FUN_141663dc0`, `FUN_141663d90`; a player counts itself).
    fn gate_passes(&self, q: usize, i: usize, p: usize) -> bool {
        let gate = &self.players[q].objects[i];
        let transforming = self
            .masters
            .get(q)
            .and_then(|m| m.get(gate.life.master_id))
            .is_some_and(|m| {
                m.class == crate::spatial::GATE_CLASS
                    && m.building.as_ref().is_some_and(|b| b.transform_unit != -1)
            });
        gate.life.state < 3
            && transforming
            && !gate.gate.is_some_and(|g| g.locked)
            && self.spatial.allied_with(q, p)
            && self.spatial.allied_with(p, q)
    }

    /// `BaseWorld::update` before player `p` updates (not Gaia): the gates of each player `q`
    /// that is `p` or counts `p` an ally and is counted one by it, unless `q`'s are open already
    /// (`open`, the update's flags), open for `p` (`q`'s slot 0x318, `FUN_14162faf0`): each gate
    /// `p` may pass ([`Turns::gate_passes`]) loses its record, its tiles' marks and its mesh shape
    /// (`FUN_141055b40`), so `p`'s units walk and plan through it.
    fn open_gates(&mut self, p: usize, open: &mut [bool]) -> Result<(), Unsupported> {
        if p == 0 {
            return Ok(());
        }
        let players = self.players.len();
        for (q, is_open) in open.iter_mut().enumerate().take(players).skip(1) {
            if *is_open
                || (q != p && !(self.spatial.allied_with(p, q) && self.spatial.allied_with(q, p)))
            {
                continue;
            }
            for i in self.gate_list(q) {
                if !self.gate_passes(q, i, p) {
                    continue;
                }
                let id = self.players[q].objects[i].id;
                if self
                    .spatial
                    .get(id)
                    .is_some_and(|g| g.obstruction.is_some_and(|o| o.kind == 10))
                {
                    return Err(Unsupported(
                        "a gate of obstruction kind 10 opens (FUN_140f65580)",
                    ));
                }
                self.spatial.drop_record(id);
                self.pathing.removed(id);
            }
            *is_open = true;
        }
        self.pathing.tiles_changed(self.spatial.take_changes());
        Ok(())
    }

    /// `BaseWorld::update` after player `p` updated (not Gaia): each open player `q`'s gates close
    /// (`q`'s slot 0x320, `FUN_141619480`) unless the next player counts `q` an ally and is
    /// counted one by it (they stay open for it): each gate `p` may pass gets its record, its
    /// tiles' marks and its mesh shape again (`FUN_141056460`), but for a part of obstruction
    /// kind 0 or 12 or of class 0 ([`crate::spatial::Obstruction::recorded`]).
    fn close_gates(&mut self, p: usize, open: &mut [bool]) {
        if p == 0 {
            return;
        }
        let (next, players) = (p + 1, self.players.len());
        for (q, is_open) in open.iter_mut().enumerate().take(players).skip(1) {
            if !*is_open {
                continue;
            }
            if next < players
                && self.spatial.allied_with(next, q)
                && self.spatial.allied_with(q, next)
            {
                continue;
            }
            for i in self.gate_list(q) {
                if !self.gate_passes(q, i, p) {
                    continue;
                }
                let gate = &self.players[q].objects[i];
                let (id, master) = (gate.id, gate.life.master_id);
                let Some(m) = self.masters.get(q).and_then(|t| t.get(master)) else {
                    continue;
                };
                let record = crate::spatial::Obstruction {
                    kind: m.obstruction_type,
                    class: m.obstruction_class,
                };
                // a gate part of obstruction kind 0 (the gate's middle, master 91) gets none
                if record.recorded()
                    && self
                        .spatial
                        .get(id)
                        .is_some_and(|g| g.obstruction.is_none())
                {
                    self.spatial.add_record(id, record);
                    if let Some(placed) = self.spatial.get(id).copied() {
                        self.pathing.place_shape(id, &placed);
                    }
                }
            }
            *is_open = false;
        }
        self.pathing.tiles_changed(self.spatial.take_changes());
    }

    /// `Process_New_Tiles` (`FUN_1412899a0`, the end of `WorldPlayerBase::update`): on each tile
    /// player `p`'s map explored since its last update, in order, each object (the tile's list,
    /// in its order) of a master that gathers by itself (`enable_auto_gather`, `+0x681`) joins
    /// the player's list of its group (`+0x683`) at its place ([`VisibleResources::add`]). A
    /// ghost's master (243) does not gather; no recyclable master does, so the objects have ids of
    /// 0 and up (`FUN_141288830` reads them from the world's table `DAT_143f69168`).
    ///
    /// # Errors
    ///
    /// An object of a master the slice does not have.
    ///
    /// [`VisibleResources::add`]: crate::visibility::VisibleResources::add
    pub(crate) fn process_new_tiles(&mut self, p: usize) -> Result<(), Unsupported> {
        let Some(sight) = self.spatial.sight.as_ref() else {
            return Ok(());
        };
        let (Some(tiles), Some(lists)) = (sight.new_tiles.get(p), sight.resources.get(p)) else {
            return Ok(());
        };
        if lists.lists.is_empty() {
            return Ok(());
        }
        let mut found = Vec::new();
        for &[x, y] in tiles {
            for (id, _) in self.spatial.objects_on(x, y) {
                let Some((owner, unit)) = self.object(id) else {
                    continue;
                };
                let master = self.master_of(owner, unit.life.master_id)?;
                if master.auto_gather {
                    found.push((id, master.gather_group, unit.position));
                }
            }
        }
        if let Some(r) = self
            .spatial
            .sight
            .as_mut()
            .and_then(|v| v.resources.get_mut(p))
        {
            for (id, group, [x, y, _]) in found {
                r.add(id, group, x, y);
            }
        }
        Ok(())
    }

    /// `WorldPlayer::make_new_object`'s visible resources (`FUN_14162ce00`, player slot 0x1c0,
    /// after the master made the object, the last of player `p`'s list): an object of a master
    /// that gathers by itself (`+0x681`) joins, at its place, the lists of its group (`+0x683`)
    /// of the players, in order, that see or explored its tile (`FUN_141663500` on its x and y
    /// truncated, [`Spatial::explored`]); the world's state (`+0x88`) is 0 in a game.
    ///
    /// # Errors
    ///
    /// A master the slice does not have, or no players' sight.
    pub(crate) fn made_new_object(&mut self, p: usize) -> Result<(), Unsupported> {
        let Some(u) = self.players[p].objects.last() else {
            return Ok(());
        };
        let (id, [x, y, _]) = (u.id, u.position);
        let master = self.master_of(p, u.life.master_id)?;
        if !master.auto_gather {
            return Ok(());
        }
        let group = master.gather_group;
        let (tx, ty) = (x as i32, y as i32);
        let mut explored = Vec::new();
        for q in 0..self.players.len() {
            explored.push(self.spatial.explored(q, tx, ty)?);
        }
        if let Some(v) = self.spatial.sight.as_mut() {
            for (q, _) in explored.iter().enumerate().filter(|&(_, &e)| e) {
                if let Some(r) = v.resources.get_mut(q) {
                    r.add(id, group, x, y);
                }
            }
        }
        Ok(())
    }

    /// `World::load_world`'s planners (`FUN_140f7d310`): the restrictions of Gaia's masters 83
    /// (the villager's) and 535 (the galley's), each unless one of the same values has one; any
    /// other restriction finds none (its units walk without the tile pathfinder).
    pub fn register_planners(&mut self) {
        for master in PLANNED_MASTERS {
            if let Some(r) = self
                .masters
                .first()
                .and_then(|m| m.get(master))
                .and_then(|m| usize::try_from(m.terrain_restriction).ok())
            {
                self.pathing.register(r);
            }
        }
    }

    /// The shape mesh the game's load builds, which no save holds: an empty mesh of the map's
    /// width, then the load's rehook of each object (`RGE_Static_Object::rehook`, slot 9, through
    /// `FUN_141665650` and `FUN_1411689f0`: each player's live list, then its sleeping list):
    /// its position set again (slot 0x58 -> `FUN_14118c540`: below state 7 and tall, then
    /// `FUN_141056460`'s shape by its master's obstruction kind, not 0 or 12, and class, not 0,
    /// for an object that does not move, [`Pathing::place_shape`]), then with an obstruction
    /// record the refresh an update ends with (`FUN_141054fe0`, [`Pathing::refresh`]). An object
    /// inside another has none (it left the obstruction manager); `mROOAlreadyCalled`
    /// (`+0x686`) is not held (0 at a game's start), and a desolid foundation in a save is left
    /// without one, as the game's own placement leaves it (unchecked at a load).
    pub fn build_mesh(&mut self) {
        let width = u32::try_from(self.spatial.map[0]).unwrap_or(0);
        self.pathing.start_mesh(width);
        for (q, p) in self.players.iter().enumerate() {
            for u in p.objects.iter().chain(&p.sleeping) {
                if u.life.inside.is_some() || 7 <= u.life.state {
                    continue;
                }
                let (Some(placed), Some(m)) = (
                    self.spatial.get(u.id),
                    self.masters.get(q).and_then(|m| m.get(u.life.master_id)),
                ) else {
                    continue;
                };
                let moving = u.mover.as_ref().is_some_and(Mover::is_moving);
                let desolid = u.construction.is_some_and(|c| c.desolid);
                let kind = m.obstruction_type;
                if placed.tall
                    && kind != 0
                    && kind != 12
                    && m.obstruction_class != 0
                    && !moving
                    && !desolid
                {
                    self.pathing.place_shape(u.id, placed);
                }
                self.pathing.refresh(u.id, placed, moving, m.outline);
            }
        }
    }

    /// One world update of `dt` game milliseconds without commands.
    pub fn tick(&mut self, dt: u32) {
        self.tick_with(dt, &[]);
    }

    /// An update's start up to the end of its command phase (`BaseWorld::update` up to
    /// `RGE_Command__do_commands`, before `FUN_141552990`): the update's count and the world's
    /// time go on by `dt`, the streams take the frame seed and `commands` run; no player updates.
    /// For a check of the commands against the game's state there (the tick oracle's
    /// `snapshot_at` at `0x141552990`); [`Turns::tick_with`] runs it first.
    pub fn command_phase(&mut self, dt: u32, commands: &[Command]) {
        self.ticks += 1;
        self.world_time = self.world_time.wrapping_add(dt);
        self.dt_seconds = seconds(dt);
        self.dt = dt;
        self.streams.seed_all(self.frame_seed);
        self.sync_moving();
        for command in commands {
            tracing::debug!(target: crate::trace::UPDATE, ?command, "command");
            self.run(command);
        }
        self.sync_moving();
    }

    /// Tell the world which objects move now and how the players stand to each other, for the
    /// walk's query ([`Spatial::walk_hits`]), as each update does first: for a look at the
    /// world between updates.
    pub fn sync_world(&mut self) {
        self.sync_moving();
    }

    /// Tell the world which objects move now and how the players stand to each other, for the
    /// walk's query ([`Spatial::walk_hits`]).
    fn sync_moving(&mut self) {
        for p in &self.players {
            for u in p.objects.iter().chain(&p.sleeping) {
                self.spatial.set_gait(u.id, u.gait());
                self.spatial.set_annex(u.id, u.annex_of);
            }
        }
        if self.spatial.relations.is_empty() {
            self.spatial.relations = self.players.iter().map(|p| p.relations.clone()).collect();
            self.spatial.task_0x99 = self
                .masters
                .iter()
                .map(|m| {
                    m.iter()
                        .filter(|m| m.tasks.iter().any(|t| t.action_type == 0x99))
                        .map(|m| m.id)
                        .collect()
                })
                .collect();
        }
    }

    /// One world update of `dt` game milliseconds, with the commands recorded in it.
    ///
    /// # Panics
    ///
    /// Never: each player updates in turn, so the one it splits off exists.
    pub fn tick_with(&mut self, dt: u32, commands: &[Command]) {
        let _update = tracing::debug_span!(
            target: crate::trace::UPDATE,
            "update",
            tick = self.ticks + 1,
            world_time = self.world_time.wrapping_add(dt)
        )
        .entered();
        self.command_phase(dt, commands);
        // which players' gates are open (BaseWorld::update's flags)
        let mut gates_open = vec![false; self.players.len()];
        // the Empire Wars start's pass, once (the world's +0x6c6 set before the players update)
        let start_work = std::mem::take(&mut self.start_work_due);
        for p in 0..self.players.len() {
            let _player =
                tracing::debug_span!(target: crate::trace::UPDATE, "player", player = p).entered();
            self.player_time[p] = self.player_time[p].wrapping_add(dt);
            let turn = (p == self.player_turn).then_some(self.player_time[p]);
            if let Err(e) = self.open_gates(p, &mut gates_open) {
                self.stop_player(p, e);
            }
            if start_work && let Err(e) = self.start_work(p) {
                self.stop_player(p, e);
            }
            self.pathing.tiles_changed(self.spatial.take_changes());
            // the player's groups (BaseWorld::update at 0x141551796), then its own update (slot
            // 0x100 at 0x1415518f0, WorldPlayerBase::update): its plans start over (`+0x67c`),
            // then its objects update (FUN_141166550 on its list)
            if let Err(e) = self.update_groups(p) {
                self.stop_player(p, e);
            }
            self.pathing.reset_plans(p);
            // An object whose update is not modelled freezes, and so do all live objects of a
            // player a command stopped; dead objects still run their state steps, which draw
            // nothing and depend on nothing the stop changed.
            let attribute_97: Vec<f32> = self
                .players
                .iter()
                .map(|q| q.attributes.get(97).copied().unwrap_or(0.0))
                .collect();
            let computers: Vec<bool> = self.players.iter().map(|q| q.computer).collect();
            let sight_takers: Vec<(bool, bool)> = self
                .players
                .iter()
                .enumerate()
                .map(|(q, player)| {
                    let allows = self.allow_gaia_capture.get(q).copied();
                    (player.computer, allows.unwrap_or(false))
                })
                .collect();
            let attribute_204 = self
                .players
                .iter()
                .any(|q| q.attributes.get(204).is_some_and(|&v| v != 0.0));
            let mut i = 0;
            // a unit whose update stopped before its unit AI for its work order runs that part
            // again once the work is given ([`Unit::ai_part`])
            let mut resume: Option<AiResume> = None;
            while i < self.players[p].objects.len() {
                let player = &self.players[p];
                let stopped = player.stopped || player.objects[i].stopped;
                if stopped && player.objects[i].life.is_alive() {
                    self.spatial.freeze(player.objects[i].id);
                    resume = None;
                    i += 1;
                    continue;
                }
                let mut unit = std::mem::take(&mut self.players[p].objects[i]);
                let _object = tracing::trace_span!(
                    target: crate::trace::UPDATE,
                    "object",
                    id = unit.id,
                    master = unit.life.master_id
                )
                .entered();
                let watch = crate::trace::Watch::of(&unit);
                let mut victim = unit.attack_target().and_then(|t| self.victim(t, p));
                let ai_target = unit
                    .ai
                    .as_ref()
                    .map(|ai| ai.current_target)
                    .filter(|&t| t != -1)
                    .map_or(Ok(None), |t| self.seen(t, p));
                let blasted = victim
                    .as_ref()
                    .map_or_else(Vec::new, |v| self.blasted(&unit, p, v.id));
                let (before, rest) = self.players.split_at_mut(p);
                let (player, after) = rest.split_first_mut().expect("the player updating");
                let mut c = Context {
                    dt,
                    dt_seconds: seconds(dt),
                    tick: self.ticks,
                    turn,
                    world_time: self.world_time,
                    computer: player.computer,
                    streams: &mut self.streams,
                    spatial: &mut self.spatial,
                    pathing: &mut self.pathing,
                    groups: &self.groups,
                    construction_rate: player.construction_rate,
                    masters: self.masters.get(p),
                    victim: victim.as_mut(),
                    ai_target,
                    blasted,
                    own_killed: None,
                    remains: None,
                    refund: Vec::new(),
                    building_death: None,
                    pauses: Vec::new(),
                    releases: Vec::new(),
                    takes: Vec::new(),
                    builder_leaves: Vec::new(),
                    left_group: None,
                    made: Vec::new(),
                    ghost_deaths: Vec::new(),
                    started: Vec::new(),
                    unwatched: Vec::new(),
                    annex_states: Vec::new(),
                    cleared: Vec::new(),
                    evacuations: Vec::new(),
                    recycling: &mut self.recycling,
                    mode_1: self.mode_680 == 1,
                    player: p,
                    civ: player.civ,
                    sharing: player.sharing,
                    farm_stops: self.settings.walkable_farms,
                    predator_hunts_end: self.settings.predator_hunts_end(),
                    attributes: &player.attributes,
                    graphics: &self.graphics,
                    drop_sites: &self.drop_sites,
                    every_masters: &self.masters,
                    attribute_97: &attribute_97,
                    player_turn: self.player_turn,
                    player_time: &self.player_time,
                    computers: &computers,
                    attribute_204,
                    mode_680: self.mode_680,
                    scenario: scenario_mode(self.settings.game_mode),
                    standard: standard_mode(self.settings.game_mode),
                    sight_takers: &sight_takers,
                    tracker: &mut player.tracker,
                    lines: &self.lines,
                    attribute_adds: Vec::new(),
                    techs_done: Vec::new(),
                    carts: Vec::new(),
                    completions: Vec::new(),
                    work_order: None,
                    gate_toggle: false,
                    ai_resume: resume.take(),
                    taken_over: None,
                    order_on_target: None,
                    hunt_objects: Vec::new(),
                    gather_objects: Vec::new(),
                    build_object: None,
                    build_command: None,
                    move_aside: Vec::new(),
                    waits: Vec::new(),
                    main_works: Vec::new(),
                    relations: &player.relations,
                    leads: None,
                    zones: &mut self.zones,
                    foreign: Foreign {
                        before,
                        after,
                        left: None,
                    },
                };
                let mut result = unit.update(&mut c, &mut player.objects);
                if let Some(w) = watch {
                    w.report(&unit, c.tick, p);
                }
                let (adds, ordered) = (
                    std::mem::take(&mut c.attribute_adds),
                    c.order_on_target.take(),
                );
                let hunt_objects = std::mem::take(&mut c.hunt_objects);
                let gather_objects = std::mem::take(&mut c.gather_objects);
                let leads = c.leads.take();
                let build_object = c.build_object.take();
                let build_command = c.build_command.take();
                let move_aside = std::mem::take(&mut c.move_aside);
                let waits = std::mem::take(&mut c.waits);
                let techs_done = std::mem::take(&mut c.techs_done);
                let carts = std::mem::take(&mut c.carts);
                let completions = std::mem::take(&mut c.completions);
                let work_order = c.work_order.take();
                let (blasted, own_killed) = (std::mem::take(&mut c.blasted), c.own_killed.take());
                let (remains, refund) = (c.remains.take(), std::mem::take(&mut c.refund));
                let (annex_states, cleared) = (
                    std::mem::take(&mut c.annex_states),
                    std::mem::take(&mut c.cleared),
                );
                let main_works = std::mem::take(&mut c.main_works);
                let building_death = c.building_death.take();
                let pauses = std::mem::take(&mut c.pauses);
                let gate_toggle = std::mem::take(&mut c.gate_toggle);
                let ai_resume = c.ai_resume.take();
                let releases = std::mem::take(&mut c.releases);
                let builder_leaves = std::mem::take(&mut c.builder_leaves);
                let left_group = c.left_group.take().map(|(g, b)| (g, unit.id, b));
                let (takes, made) = (std::mem::take(&mut c.takes), std::mem::take(&mut c.made));
                let (ghost_deaths, started, unwatched, evacuations) = (
                    std::mem::take(&mut c.ghost_deaths),
                    std::mem::take(&mut c.started),
                    std::mem::take(&mut c.unwatched),
                    std::mem::take(&mut c.evacuations),
                );
                if let Some(e) = self.spatial.take_sight_unsupported() {
                    result = result.and(Err(e));
                }
                self.spatial.set_gait(unit.id, unit.gait());
                player.objects[i] = unit;
                // each made object's add to its player (slot 0x2d0) filed it in the tracker
                for m in &made {
                    let k = crate::tracker::key(self.masters.get(p), &self.lines, m.life.master_id);
                    player.tracker.add(m.id, k);
                }
                player.objects.extend(made);
                // the gate's player opens or shuts it, in the gate's update before the rest
                if gate_toggle && let Err(e) = self.toggle_gate(p, i) {
                    result = result.and(Err(e));
                }
                // the work a builder gave an annex goes to its main building (FUN_1411cc430)
                for (annex, amount) in main_works {
                    if let Err(e) = self.work_building(annex, amount) {
                        result = result.and(Err(e));
                    }
                }
                // a building's set_state took its annexes along first (FUN_1411dbb70)
                for (annex, state) in annex_states {
                    if let Some((q, _)) = self.object(annex)
                        && let Err(e) = self.set_state(q, annex, state)
                    {
                        result = result.and(Err(e));
                    }
                }
                // the objects a builder's footprint clearing deleted (FUN_1415c3620)
                for cl in cleared {
                    if let Err(e) = self.clear_object(cl, true) {
                        result = result.and(Err(e));
                    }
                }
                for (kind, amount) in adds {
                    if let Err(e) = self.players[p].add_attribute(kind, amount) {
                        result = result.and(Err(e));
                    }
                }
                // a completed mule cart's search for resources (FUN_1411c6bd0's tail, before
                // the completion's technology)
                for (cart, state) in carts {
                    if let Err(e) = self.cart_search(p, cart, state) {
                        result = result.and(Err(e));
                    }
                }
                // a completed building's stack unit before its technology (FUN_1411dbb70)
                for &(building, stack, _) in &completions {
                    if let Some(stack) = stack
                        && let Err(e) = self.stack_unit(p, building, stack)
                    {
                        result = result.and(Err(e));
                    }
                }
                for t in techs_done {
                    if let Err(e) = self.research_built(p, t) {
                        result = result.and(Err(e));
                    }
                }
                // then a building that disappears when built is destroyed (slot 0xd0)
                for (building, _, vanishes) in completions {
                    if vanishes && let Err(e) = self.destroy(p, building) {
                        result = result.and(Err(e));
                    }
                }
                // slot 0x5f0 on each unit the walk paused: its walked level's pause
                for (id, pause) in pauses {
                    self.pause(id, pause);
                }
                // a dead building leaves its owner's watched list (FUN_141091310), then the
                // ghosts of a death (FUN_14118e990, while the game state is not 2) and the
                // watch of a started construction (FUN_1411e5670)
                for id in unwatched {
                    self.unwatch(p, id);
                }
                for (foundation, units) in evacuations {
                    if let Err(e) = self.evacuate(foundation, &units) {
                        result = result.and(Err(e));
                    }
                }
                for id in ghost_deaths {
                    if self.game_state != 2
                        && let Err(e) = self.ghosts_at_death(p, id)
                    {
                        result = result.and(Err(e));
                    }
                }
                for id in started {
                    if let Some((owner, _)) = self.object(id)
                        && let Err(e) = self.watch_started(owner, id)
                    {
                        result = result.and(Err(e));
                    }
                }
                // a dead building's count leaves its attribute, at least 0 (FUN_1411dbb70)
                if let Some(BuildingDeath {
                    count: Some(kind), ..
                }) = building_death
                {
                    let owner = &mut self.players[p];
                    if let Err(e) = owner.add_attribute(kind, -1.0) {
                        result = result.and(Err(e));
                    } else if owner.attributes[kind as usize] < 0.0 {
                        owner.attributes[kind as usize] = 0.0;
                    }
                }
                // a dead unit's paid cost leaves attributes 98 and 164 (a building's 98 and 165),
                // each at least 0 (FUN_14121cc40: slots 0xf8, then 0xf0 with 0)
                for (kind, cost) in refund {
                    let owner = &mut self.players[p];
                    if let Err(e) = owner.add_attribute(kind, -cost) {
                        result = result.and(Err(e));
                    } else if owner.attributes[kind as usize] < 0.0 {
                        owner.attributes[kind as usize] = 0.0;
                    }
                }
                // the last building of a master with a technology withdraws it (FUN_141166ab0,
                // FUN_1413d4ec0), after the combat part's state 3
                if let Some(BuildingDeath {
                    tech: Some((master, tech)),
                    ..
                }) = building_death
                    && let Err(e) = self.withdraw_tech(p, master, tech)
                {
                    result = result.and(Err(e));
                }
                // the remains of a dying sprite's end, a new object of the owner (FUN_1411a4e90)
                if let Some((master, at, facet)) = remains
                    && let Err(e) = self.remains(p, master, at, facet)
                {
                    result = result.and(Err(e));
                }
                // TRIBE_Building_Object::update: the production after the combat update (once,
                // with the update's unit AI part)
                if matches!(result, Ok(Step::Stay))
                    && ai_resume.is_none()
                    && let Err(e) = self.produce(p, i, seconds(dt))
                {
                    result = Err(e);
                }
                if let Some(work) = work_order {
                    if !matches!(result, Ok(Step::Stay)) {
                        result = result.and(Err(Unsupported(
                            "a builder's work order of a unit that leaves its list",
                        )));
                    } else if let Err(e) = self.order_work(p, i, &work) {
                        result = Err(e);
                    }
                }
                // the unit AI's HuntObjects: each the unit's stop, then its hunt command
                // a hunt's set_state(6) on its own livestock: the animal's slot 0x138 to the drop
                // site and the herd around it (FUN_140ac4c60)
                if let Some((sheep, site)) = leads
                    && let Err(e) = self.lead_herd(p, sheep, site)
                {
                    result = Err(e);
                }
                for target in hunt_objects {
                    if !matches!(result, Ok(Step::Stay)) {
                        result = result.and(Err(Unsupported(
                            "a HuntObject of a unit that leaves its list",
                        )));
                        break;
                    }
                    if let Err(e) = self.hunt_command(p, i, target) {
                        result = Err(e);
                    }
                }
                // the unit AI's GatherObjects: each the unit's gather command
                for target in gather_objects {
                    if !matches!(result, Ok(Step::Stay)) {
                        result = result.and(Err(Unsupported(
                            "a GatherObject of a unit that leaves its list",
                        )));
                        break;
                    }
                    if let Err(e) = self.gather_command(p, i, target) {
                        result = Err(e);
                    }
                }
                // the unit AI's moves off footprints (slot 0x148)
                for m in move_aside {
                    if !matches!(result, Ok(Step::Stay)) {
                        result = result.and(Err(Unsupported(
                            "a move off a footprint of a unit that leaves its list",
                        )));
                    } else if let Err(e) = self.move_aside(p, i, m) {
                        result = Err(e);
                    }
                }
                // the waits for foundations the unit AI's orders found (slot 0x388)
                for w in waits {
                    // a unit that ran its orders stays in its list: the full update runs them
                    // only for a unit alive and not inside another, between turns only for a
                    // unit of a type below 70 or outside, and nothing after the queue in the
                    // unit's update kills, captures or houses it
                    debug_assert!(matches!(result, Ok(Step::Stay)) || result.is_err());
                    if !matches!(result, Ok(Step::Stay)) {
                        break;
                    }
                    if let Err(e) = self.wait_for_foundation(p, i, w) {
                        result = Err(e);
                    }
                }
                // a Gather's build command on its target not built yet (slot 0x380)
                if let Some(foundation) = build_command {
                    if !matches!(result, Ok(Step::Stay)) {
                        result = result.and(Err(Unsupported(
                            "a build command of a unit that leaves its list",
                        )));
                    } else if let Err(e) = self.build_command(p, i, foundation) {
                        result = Err(e);
                    }
                }
                // the unit AI's BuildObject on a foundation its builder's search found
                if let Some(foundation) = build_object {
                    if !matches!(result, Ok(Step::Stay)) {
                        result = result.and(Err(Unsupported(
                            "a BuildObject of a unit that leaves its list",
                        )));
                    } else if let Err(e) = self.build_object(p, i, foundation) {
                        result = Err(e);
                    }
                }
                if let (Ok(Step::Stay), Some((order, tail))) = (&result, ordered) {
                    let ran = self
                        .order_on_target(p, i, order)
                        .and_then(|()| tail.map_or(Ok(()), |after| self.after_order(p, i, after)));
                    if let Err(e) = ran {
                        result = Err(e);
                    }
                } else if ordered.is_some() && result.is_ok() {
                    result = Err(Unsupported(
                        "an order on a target of a unit that leaves its list",
                    ));
                }
                // a hit's write-backs in the order the hit made them: the blast's objects, the
                // unit itself (a retaliation), its target
                for v in &blasted {
                    if let Err(e) = self.set_victim(v) {
                        result = result.and(Err(e));
                    }
                }
                if let Some(kill) = own_killed
                    && let Err(e) = self.killed(p, i, kill)
                {
                    result = result.and(Err(e));
                }
                if let Some(v) = victim
                    && let Err(e) = self.set_victim(&v)
                {
                    result = result.and(Err(e));
                }
                // an entered unit's removeUnit from its group (FUN_14102c160), when the world
                // holds the group (FUN_14152ff40)
                if let Some((g, id, building)) =
                    left_group.filter(|&(g, ..)| self.groups.get(g).is_some())
                    && let Err(e) = self.remove_unit(g, id, building, false)
                {
                    result = result.and(Err(e));
                }
                for (id, by) in takes {
                    self.take(id, by);
                }
                for id in builder_leaves {
                    if let Err(e) = self.leave_builders(id) {
                        result = result.and(Err(e));
                    }
                }
                for (id, by) in releases {
                    if let Err(e) = self.release(id, by) {
                        result = result.and(Err(e));
                    }
                }
                // a construction start's clearing of the desolid foundations it overlaps
                // (FUN_1411cd0d0), after the builder's update
                for id in self.spatial.take_started() {
                    if let Err(e) = self.clear_foundations_under(id) {
                        result = result.and(Err(e));
                    }
                }
                self.pathing.tiles_changed(self.spatial.take_changes());
                if matches!(result, Ok(Step::Stay)) && ai_resume.is_some() {
                    resume = ai_resume;
                    continue;
                }
                let player = &mut self.players[p];
                match result {
                    Ok(Step::Leave) => {
                        let gone = player.objects.swap_remove(i);
                        if let Err(e) = self.leave_world(p, &gone) {
                            crate::trace::player_unsupported(self.ticks, p, e);
                            let _ = self.players[p]
                                .unsupported
                                .get_or_insert((self.ticks, i, e));
                        }
                    }
                    Ok(Step::Sleep) => {
                        // the list's remove (FUN_1411664a0) puts its last object in the
                        // sleeper's place, and the list's loop (0x141166550) goes on past it:
                        // that object's update waits for the next tick; the player's remove and
                        // add (slots 0x2d8, 0x2d0) file it last in the tracker
                        let asleep = player.objects.swap_remove(i);
                        let k = crate::tracker::key(
                            self.masters.get(p),
                            &self.lines,
                            asleep.life.master_id,
                        );
                        player.tracker.remove(asleep.id, k);
                        player.tracker.add(asleep.id, k);
                        player.sleeping.push(asleep);
                        i += 1;
                    }
                    Ok(Step::Captured(Captured { new, main, also })) => {
                        // the old owner's list loses them (FUN_1411664a0, the last in each
                        // place; the loop goes on past the first) and the new owner's gains
                        // them (FUN_141616200), each with its attributes
                        // ([`Turns::hand_over_attributes`])
                        let mut moved = Vec::new();
                        if main {
                            moved.push(player.objects.swap_remove(i));
                        }
                        for id in also {
                            if let Some(k) = player.objects.iter().position(|u| u.id == id) {
                                moved.push(player.objects.swap_remove(k));
                            }
                        }
                        let mut failed = None;
                        for u in &moved {
                            if let Err(e) = self.hand_over_attributes(p, new, u) {
                                let _ = failed.get_or_insert(e);
                            }
                        }
                        if let Some(e) = failed {
                            crate::trace::player_unsupported(self.ticks, p, e);
                            let _ = self.players[p]
                                .unsupported
                                .get_or_insert((self.ticks, i, e));
                        }
                        // each one's slot 0x70: the old owner's remove (slot 0x2d8), the new
                        // owner's add (slot 0x2d0), each tracker by its own masters
                        for u in &moved {
                            let m = u.life.master_id;
                            let old = crate::tracker::key(self.masters.get(p), &self.lines, m);
                            self.players[p].tracker.remove(u.id, old);
                            let k = crate::tracker::key(self.masters.get(new), &self.lines, m);
                            self.players[new].tracker.add(u.id, k);
                        }
                        self.players[new].objects.extend(moved);
                        i += 1;
                    }
                    Ok(Step::Stay) => i += 1,
                    Err(e) => {
                        crate::trace::object_freezes(self.ticks, p, player.objects[i].id, e);
                        player.freezes.push((self.ticks, player.objects[i].id, e));
                        let _ = player.unsupported.get_or_insert((self.ticks, i, e));
                        player.objects[i].stopped = true;
                        self.spatial.freeze(player.objects[i].id);
                        i += 1;
                    }
                }
            }
            // the end of the player's own update: the tiles its map explored give their
            // resources
            if let Err(e) = self.process_new_tiles(p) {
                self.stop_player(p, e);
            }
            self.close_gates(p, &mut gates_open);
            // its tile list empties (FUN_141661030)
            if let Some(v) = self.spatial.sight.as_mut() {
                v.clear_new_tiles(p);
            }
        }
        self.spatial.grow_unknown(seconds(dt));
        let s1 = self.streams.get(Stream::World).rand15();
        // every player's ghosts (FUN_141166550 on +0x40), then every player's watched list
        // (FUN_1410905e0); the objects' visibility masks (FUN_14107d280 on +0x80, before the
        // draw) are the registry's, kept as the objects update
        for p in 0..self.players.len() {
            if let Err(e) = self.update_ghosts(p) {
                self.stop_player(p, e);
            }
        }
        for p in 0..self.players.len() {
            if let Err(e) = self.watch_pass(p) {
                self.stop_player(p, e);
            }
        }
        self.streams.seed_all(s1);
        self.player_time[self.player_turn] = 0;
        self.player_turn = (self.player_turn + 1) % self.players.len();
        self.victory_check();
        self.frame_seed = self.streams.get(Stream::World).rand15();
    }
}

/// `FUN_141302540`: the game mode (options `+0x34`) is Scenario (3), Campaign (4), King of the
/// Hill (5) or Co-op Campaign (15), by aoc-mgz's game types.
#[must_use]
pub const fn scenario_mode(game_mode: i32) -> bool {
    matches!(game_mode, 3 | 4 | 5 | 15)
}

/// `FUN_141302450`: the game mode's descriptor (`DAT_143f6b330[mode]`, the first for a mode above
/// 16) has `+0x9c` 1, as `FUN_1412d99e0` makes them (`FUN_1412d9360`'s fifth argument): all but
/// the scenario modes 3, 4, 5, 14 and 15.
#[must_use]
pub const fn standard_mode(game_mode: i32) -> bool {
    !matches!(game_mode, 3 | 4 | 5 | 14 | 15)
}

#[cfg(test)]
mod tests;
