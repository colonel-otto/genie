//! The world player's own update (`WorldPlayer::update`, `0x14163b100`, slot 0x100) and Gaia's
//! (`WorldPlayerGaia::update`, `0x141681cd0`, its slot 0x100). `BaseWorld::update` runs a
//! player's slot 0x100 after its groups (`0x1415518f0`); both end in `WorldPlayerBase::update`
//! (`0x141677450`: the plans, the objects, `Process_New_Tiles`), which [`Turns::tick_with`] runs
//! between [`Turns::update_world_player`] and [`Turns::world_player_tail`].
//!
//! Gaia's update only adds the update's seconds (`world+0x164`) to its own `update_time`
//! (`+0x1468`); the world player's `mUpdateTime` (`+0x7a4`) of Gaia stays 0. The world player's
//! update adds them to `mUpdateTime` and runs a loop once for each whole second of it
//! ([`Turns::world_player_second`]): the history, the explored share (attribute 22), the kill
//! ratio (attribute 44), the defeat check of a human or computer player and, while the player
//! plays, its resource trickles (attributes 230 to 233) and its mills' food (attribute 254).
//! After `WorldPlayerBase::update` its scout map takes the tiles its map explored
//! ([`Turns::world_player_tail`]).
//!
//! Not simulation, not held: the local player's farm and fish trap queues sent as commands once
//! after a load (`+0x1427`, `FUN_140e74d70`, `FUN_140e74e10`: GAME 0x10 and 0x13, which a
//! recording holds; a playback sends none, `FUN_141533c10`), the profiler (`+0x1448`), the nine
//! slots at `+0xeac` (a world time read, `FUN_1415300d0`, whose answer goes unused), the attack
//! alerts' list (`+0x798`, unsaved: `FUN_141612690` posts the spectator API's event,
//! `FUN_141778410`, and the `"?under_attack"` sound and chat; `FUN_1417787d0` reads the spectator
//! API), the statistics of the local or a computer player's income (`FUN_14097daf0`), the
//! resource panel's event (`FUN_140c898a0` 0x80 in the attribute add), the AI's turn and update
//! (`FUN_14154e310` on `world+0x7dc`, unsaved, and the AI's slot 0x28 at `+0x7a8`: a playback
//! runs neither, `FUN_140d2f9c0`'s `+1`). The units of a formation that came out at one gather
//! point (`+0x1428`, `+0x1438` above 1, `FUN_141614170`) are only listed (`FUN_14160c4f0`) under
//! the world's feature flags (`mUseFeatureFlags`, `DAT_143f01830`), 0 in every recording seen.
//!
//! 158041: the explored share `FUN_141285c50`, the human or computer test `FUN_141663f30`, the
//! world player's attribute add (slot 0xf8) `FUN_141616820` and its base's `FUN_14165d040`, the
//! scout map's set `FUN_140f4d5e0`.

use crate::fmath::{at_most_or_unordered, below_or_unordered};
use crate::turn::Turns;
use crate::unit_ai::Unsupported;

/// One second (`DAT_14330eb8c`): the loop's step.
const SECOND: f32 = crate::image::world_player::SECOND;
/// One hundred (`DAT_14330ebcc`): attribute 22 is a percentage.
const PERCENT: f32 = crate::image::world_player::PERCENT;
/// A minute's seconds (`DAT_14330ebc8`): the trickles and the mills' food are rates a minute.
const MINUTE: f32 = crate::image::world_player::MINUTE;
/// The livestock food over which a mill's yield grows by one e-fold (`DAT_142eb0f54`).
const LIVESTOCK_SCALE: f32 = crate::image::world_player::LIVESTOCK_SCALE;
/// Until this world time (ms) the scout map takes only the explored tiles the player sees.
const EARLY_SCOUTING_MS: u32 = 5000;

/// Gaia's player type (`+0x10`), whose class is `WorldPlayerGaia`.
const GAIA_TYPE: u8 = 2;
/// The human player's type.
const HUMAN_TYPE: u8 = 1;
/// The computer player's type: its income fills its escrow.
const COMPUTER_TYPE: u8 = 3;
/// The buildings' type (the object's `+0xf`, `'P'`).
const BUILDING_TYPE: u8 = 80;
/// The mill's master, whose livestock makes food (`FUN_14104f960` with 0x44).
const MILL: i32 = 68;
/// The king's master, which the regicide rule asks the tracker for (`FUN_14104eea0` with 0x1b2).
const KING: i32 = 434;
/// The town centres' masters (`FUN_141658970`'s and `FUN_141658a30`'s keys 0x6d and 0x1bc).
const TOWN_CENTRES: [i32; 2] = [109, 444];
/// The wonder's master (`0x61b0` raw, `0x14163bb68`).
const WONDER: i16 = 276;
/// The builder's master (`FUN_141625a20`'s 0x76).
const BUILDER: i16 = 118;
/// The town centre a builder may place (`FUN_141625a20`'s 0x26d).
const BUILDABLE_TOWN_CENTRE: i16 = 621;
/// The classes that never keep their player (`0x48000800`: other 11, walls 27, flags 30).
const UNKEPT_LOW: u32 = 0x4800_0800;
/// The classes that never keep their player (`0x140204a000384006`: 1, 2, 14, 19, 20, 21, 37,
/// 39, 42, 49, 58, 60).
const UNKEPT_HIGH: u64 = 0x1402_04a0_0038_4006;
/// The buildings' class (3).
const BUILDING_CLASS: u32 = 3;
/// The towers' class (52).
const TOWER_CLASS: u32 = 52;
/// The packed siege's class (51).
const PACKED_CLASS: u32 = 51;
/// The unpacked siege's class (54).
const UNPACKED_CLASS: u32 = 54;
/// The ages' technologies (Feudal, Castle, Imperial: `0x3284`, `0x3304`, `0x3384` in the
/// player's technologies), whose state −1 puts their units out of the player's tree.
const AGES: [i32; 3] = [101, 102, 103];
/// A technology's state once researched.
const RESEARCHED: i16 = 3;
/// `FUN_140c7e6f0`: the civilisation the full tech tree reads, by the game's tagged value at
/// `+0x680` ([`crate::turn::Turns::mode_680`]): 18 for 1, 59 for 2, else 0.
const fn full_tree_civ(mode_680: i32) -> i32 {
    match mode_680 {
        1 => 0x12,
        2 => 0x3b,
        _ => 0,
    }
}
/// The resource storage that keeps its player (`0x14163b86b`): attribute 11 …
const POPULATION_STORAGE: i16 = 11;
/// … with the store mode 0x20 (`0x14163b89d`).
const ALIVE_STORE_MODE: u8 = 0x20;

/// The player's attributes the update reads and writes.
mod attribute {
    /// Food: the first of the four resources (food, wood, stone, gold: 0 to 3).
    pub const FOOD: usize = 0;
    /// Gold.
    pub const GOLD: usize = 3;
    /// The population.
    pub const POPULATION: usize = 11;
    /// The units owned.
    pub const UNITS_OWNED: usize = 19;
    /// The kills.
    pub const KILLS: usize = 20;
    /// The explored share of the map, in percent.
    pub const EXPLORED: usize = 22;
    /// The kill ratio: kills less losses.
    pub const KILL_RATIO: usize = 44;
    /// The survival to the finish, which a defeat clears.
    pub const SURVIVAL: usize = 45;
    /// The gold counter, which gold with a statistic grows (`FUN_141616820`).
    pub const GOLD_COUNTER: usize = 49;
    /// The first of the gathered totals of food, wood and stone (166 to 168).
    pub const GATHERED: usize = 166;
    /// The gathered total of gold.
    pub const GOLD_GATHERED: usize = 169;
    /// The first of the trickles a minute of food, wood, stone and gold (230 to 233).
    pub const TRICKLES: usize = 230;
    /// The food a minute of the livestock in the player's mills.
    pub const MILL_FOOD: usize = 254;
    /// The town centre is not available (0 lets a builder place one, `FUN_141625a20`).
    pub const NO_TOWN_CENTRE: usize = 48;
    /// The town centres a player may have (`FUN_141625a20`).
    pub const TOWN_CENTRE_LIMIT: usize = 218;
}

/// The timers that keep a player in the game without a living object (the defeat check's first
/// test): each counts at 0 or more; −1 is none.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VictoryTimes {
    /// The game's wonder countdown of the player (the game object's `+0x4ec[id]`,
    /// `FUN_140c88300`): the least time left of its wonders' actions (`TRIBE_Action_Wonder`,
    /// `FUN_140c91500`), −1 without one (`FUN_140c90040`).
    pub wonder: f32,
    /// `ruin_held_time` (`+0xdb0`, saved).
    pub ruins: f32,
    /// `artifact_held_time` (`+0xdb4`, saved).
    pub artifacts: f32,
}

/// No timer runs.
impl Default for VictoryTimes {
    fn default() -> Self {
        Self {
            wonder: -1.0,
            ruins: -1.0,
            artifacts: -1.0,
        }
    }
}

impl VictoryTimes {
    /// One of the timers runs (`COMISS` with 0, `JNC`: 0 or more, not NaN): the player stays in
    /// the game.
    #[must_use]
    pub fn running(&self) -> bool {
        [self.wonder, self.ruins, self.artifacts]
            .into_iter()
            .any(|t| !below_or_unordered(t, 0.0))
    }
}

/// One unit of a building in a civilisation's tech tree (`0x48` bytes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TreeUnit {
    /// `+0x0`: the unit's master.
    pub id: i32,
    /// `+0x4`: the master it links to ([`TechTree::link`]).
    pub link: i32,
    /// `+0x8`: its age (2 Feudal, 3 Castle, 4 Imperial).
    pub age: i32,
    /// `+0xc`: the technology that brings it ([`TechTree::tech`]).
    pub tech: i32,
}

/// One building of a civilisation's tech tree (`0x48` bytes): its master and its units.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TreeBuilding {
    /// `+0x0`: the building's master.
    pub id: i32,
    /// `+0x8` (count `+0x18`): its units, in order.
    pub units: Vec<TreeUnit>,
}

/// A civilisation's unique unit (its data's `+0xec`, `+0xf0`, `+0xf4`, `FUN_140e868f0`,
/// `FUN_140e868b0`, `FUN_140e86630`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UniqueUnit {
    /// The technology that makes it elite.
    pub elite_tech: i32,
    /// The unit.
    pub unit: i32,
    /// The elite unit.
    pub elite: i32,
}

/// The game's tech tree table (`FUN_140c7e080`: the game object's `+0xa60`, or `+0xa68` for a
/// local player's civilisation in `FUN_140e9a7e0`'s list, 14 and 17) by civilisation index
/// (`0x20` bytes each: its buildings, count `+0x10`), and each civilisation's unique unit by its
/// index (`FUN_140e87930`, `civilizations.json`'s `unique_unit_upgrade_id`, `unique_unit_id`,
/// `elite_unique_unit_id`).
///
/// The game's data load (`FUN_140c81bd0`) reads `futuravailableunits.json` into `+0xa60` and
/// `paphosfutureavailableunits.json` into `+0xa68` (`FUN_1414f3720`): civilisation indices 0 to
/// the full tech tree's (`FUN_140c7e6f0`), each its name's buildings (`ID`) with their units
/// (`ID`, `RequiredUnitID`, `RequiredAge`, `RequiredTechID`, `FUN_1414f5c40`), the last
/// `FullTechCiv`'s. With the game's `+0x680` at 0, as genie and the tick oracle hold it, that is
/// index 0 alone: a player of civilisation 1 or more without the full tech tree finds no
/// civilisation in it, as in the empty table genie-oracle leaves; the full tech tree's index 0
/// is not loaded yet.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TechTree {
    /// The civilisations' buildings.
    pub civs: Vec<Vec<TreeBuilding>>,
    /// The civilisations' unique units.
    pub unique_units: Vec<Option<UniqueUnit>>,
}

/// What a tree lookup asks: the civilisation, the building, the unit, the ages the player
/// cannot reach (technologies 101 to 103 at state −1) and the game's options.
#[derive(Clone, Copy, Debug)]
pub struct TreeQuery<'a> {
    /// The civilisation index.
    pub civ: i32,
    /// The building's master.
    pub building: i32,
    /// The unit's master.
    pub unit: i32,
    /// The Feudal, Castle and Imperial Ages out of reach.
    pub ages_out: [bool; 3],
    /// The game's options: the full tech tree and the last age.
    pub settings: &'a crate::settings::Settings,
    /// The civilisation the full tech tree falls back to (`FUN_140c7e6f0`).
    pub fallback: i32,
}

/// What [`TechTree::find`] answers.
enum Found<'a> {
    /// The table has no such civilisation (`FUN_1414f5340`'s and `FUN_1414f4c80`'s 0).
    NoCiv,
    /// The unit is out of the game's ages or of the player's (0).
    OutOfAges,
    /// The building's units, the first that is the unit, if any.
    Unit(Option<&'a TreeUnit>),
    /// No such building, also in the fallback civilisation (−1).
    NoBuilding,
}

impl TechTree {
    /// The tree lookups' common part (`FUN_1414f5970`, `FUN_1414f5340`, `FUN_1414f4c80`): the
    /// civilisation's first building that is `q.building`; its first unit that is `q.unit` gives
    /// the age (0 for none), out of the game when the last age less 1 is below it; then that
    /// unit's age 2 out with the Feudal Age, 3 with the Feudal or Castle and 4 with any. A
    /// building the civilisation lacks under the full tech tree tries the fallback civilisation
    /// once it is another.
    fn find(&self, q: &TreeQuery<'_>) -> Found<'_> {
        let mut civ = q.civ;
        loop {
            let Some(buildings) = usize::try_from(civ).ok().and_then(|c| self.civs.get(c)) else {
                return Found::NoCiv;
            };
            if let Some(b) = buildings.iter().find(|b| b.id == q.building) {
                let unit = b.units.iter().find(|u| u.id == q.unit);
                let age = unit.map_or(0, |u| u.age);
                if q.settings.last_age() - 1 < age {
                    return Found::OutOfAges;
                }
                let [feudal, castle, imperial] = q.ages_out;
                let out = unit.is_some_and(|u| {
                    (u.age == 2 && feudal)
                        || (u.age == 3 && (feudal || castle))
                        || (u.age == 4 && (feudal || castle || imperial))
                });
                if out {
                    return Found::OutOfAges;
                }
                return Found::Unit(unit);
            }
            if civ == q.fallback || !q.settings.full_tech_tree {
                return Found::NoBuilding;
            }
            civ = q.fallback;
        }
    }

    /// `FUN_1414f5970`: the unit is in the civilisation's tree at the building, within the ages.
    #[must_use]
    pub fn has(&self, q: &TreeQuery<'_>) -> bool {
        matches!(self.find(q), Found::Unit(Some(_)))
    }

    /// `FUN_1414f5340`: the unit's link (`+0x4`); 0 out of the ages or without the civilisation,
    /// −1 without the building or the unit (a building found without the unit tries the
    /// fallback civilisation as one not found does).
    #[must_use]
    pub fn link(&self, q: &TreeQuery<'_>) -> i32 {
        self.field(q, |u| u.link)
    }

    /// `FUN_1414f4c80`: the unit's technology (`+0xc`), as [`TechTree::link`].
    #[must_use]
    pub fn tech(&self, q: &TreeQuery<'_>) -> i32 {
        self.field(q, |u| u.tech)
    }

    fn field(&self, q: &TreeQuery<'_>, f: impl Fn(&TreeUnit) -> i32) -> i32 {
        match self.find(q) {
            Found::NoCiv | Found::OutOfAges => 0,
            Found::Unit(Some(u)) => f(u),
            Found::Unit(None) if q.civ != q.fallback && q.settings.full_tech_tree => {
                let again = TreeQuery {
                    civ: q.fallback,
                    ..*q
                };
                self.field(&again, f)
            }
            Found::Unit(None) | Found::NoBuilding => -1,
        }
    }
}

impl Turns {
    /// Player `p`'s slot 0x100 up to `WorldPlayerBase::update`. Gaia (`WorldPlayerGaia::update`)
    /// adds the update's seconds to its own update time. A world player adds them to
    /// `mUpdateTime` (`ADDSS` of `world+0x164` and the time) and, while it is at least a second
    /// (`COMISS` with 1, `JC` ends the loop, also for a NaN), takes a second off and runs the
    /// second's work ([`Turns::world_player_second`]) with the time before the step.
    ///
    /// # Errors
    ///
    /// The second's work met a part of the game the slice does not model.
    pub fn update_world_player(&mut self, p: usize) -> Result<(), Unsupported> {
        let dt = self.dt_seconds;
        let player = &mut self.players[p];
        if player.kind == GAIA_TYPE {
            player.gaia_update_time += dt;
            return Ok(());
        }
        player.update_time += dt;
        loop {
            let time = self.players[p].update_time;
            if below_or_unordered(time, SECOND) {
                return Ok(());
            }
            self.players[p].update_time = time - SECOND;
            self.world_player_second(p, time)?;
        }
    }

    /// One pass of the world player's loop, `time` its update time before the step:
    /// - the history's update with the `.dat`'s period ([`crate::history::History::update`]);
    /// - attribute 22: 100 when every player explored the map from the start
    ///   ([`crate::settings::Settings::explored_all`]), else the explored share
    ///   ([`Turns::explored_share`]) times 100;
    /// - attribute 44: attribute 20 less (attribute 19 less attribute 11), 0 when that is below 0
    ///   (`COMISS` 0 with it, `JBE`: a NaN stays);
    /// - for a human or computer player (`FUN_141663f30`: type 1 or 3) the defeat check
    ///   ([`Turns::defeat_check`]), then, while it plays, the trickles ([`Turns::trickles`]) and
    ///   the mills' food ([`Turns::mill_food`]).
    ///
    /// # Errors
    ///
    /// The defeat check's ([`Turns::defeat_check`]).
    fn world_player_second(&mut self, p: usize, time: f32) -> Result<(), Unsupported> {
        let (world_time, period) = (self.world_time, self.history_period);
        let player = &mut self.players[p];
        player
            .history
            .update(&player.attributes, world_time, period);
        let explored = if self.settings.explored_all() {
            Some(PERCENT)
        } else {
            self.explored_share(p).map(|share| share * PERCENT)
        };
        let player = &mut self.players[p];
        if let Some(e) = explored {
            set(&mut player.attributes, attribute::EXPLORED, e);
        }
        let a = &player.attributes;
        let ratio = get(a, attribute::KILLS)
            - (get(a, attribute::UNITS_OWNED) - get(a, attribute::POPULATION));
        set(&mut player.attributes, attribute::KILL_RATIO, ratio);
        if !at_most_or_unordered(0.0, get(&player.attributes, attribute::KILL_RATIO)) {
            set(&mut player.attributes, attribute::KILL_RATIO, 0.0);
        }
        if !matches!(player.kind, HUMAN_TYPE | COMPUTER_TYPE) {
            return Ok(());
        }
        self.defeat_check(p)?;
        if self.players[p].game_status == crate::game_status::PLAYING {
            self.trickles(p);
            self.mill_food(p, time);
        }
        Ok(())
    }

    /// `FUN_141285c50` on player `p`'s visible map (`+0x70`): its explored tiles (`+0x28`) over
    /// its tiles (`+0x2c`, the width times the height its load sets), each converted
    /// (`CVTDQ2PS`) before the division. None in a slice without the players' sight, whose
    /// attribute 22 the update then leaves.
    #[must_use]
    pub fn explored_share(&self, p: usize) -> Option<f32> {
        let sight = self.spatial.sight.as_ref()?;
        let map = sight.maps.get(p)?;
        Some(map.explored as f32 / (sight.width * sight.height) as f32)
    }

    /// The defeat check of player `p` (`0x14163b72b`). The player stays in the game while one of
    /// its victory timers runs ([`VictoryTimes::running`]) or else one of its objects, in its
    /// list's order, keeps it ([`Turns::keeps_alive`]). Under the sudden death rule
    /// ([`crate::settings::Settings::sudden_death`]), asked only for a player that stays, its
    /// first built town centre ([`Turns::first_town_centre`]) decides once the player had one
    /// (`mSuddenDeathCanBeEliminated`, `+0x1380`, which the first one found sets). A player not
    /// yet lost then loses when it does not stay, or when it stays without a king (master 434 in
    /// its tracker, `FUN_14104eea0`) under the regicide rule
    /// ([`crate::settings::Settings::regicide`]): the status setter takes 2
    /// ([`Turns::set_status`]) and attribute 45 is cleared.
    ///
    /// # Errors
    ///
    /// An object of a master the slice does not have; the status setter's.
    pub fn defeat_check(&mut self, p: usize) -> Result<(), Unsupported> {
        let mut stays = self.players[p].victory_times.running();
        if !stays {
            for unit in &self.players[p].objects {
                if self.keeps_alive(p, unit)? {
                    stays = true;
                    break;
                }
            }
        }
        if stays && self.settings.sudden_death() {
            let centre = self.first_town_centre(p).is_some();
            let player = &mut self.players[p];
            if player.sudden_death_eliminable {
                stays = centre;
            } else if centre {
                player.sudden_death_eliminable = true;
            }
        }
        if self.players[p].game_status == crate::game_status::LOST {
            return Ok(());
        }
        let kingless = self.settings.regicide() && self.players[p].tracker.list(KING).is_empty();
        if !stays || kingless {
            self.set_status(p, crate::game_status::LOST)?;
            set(&mut self.players[p].attributes, attribute::SURVIVAL, 0.0);
        }
        Ok(())
    }

    /// Whether object `unit` keeps player `p` in the game (`0x14163b790`), by its master's class
    /// (`+0x410`, the decoded short, compared unsigned) and its state (`+0x640`):
    /// - in states 1 to 4, a resource storage of the master holding attribute 11 with the store
    ///   mode 0x20 keeps it;
    /// - classes 11, 27 and 30 (other, walls, flags) and 1, 2, 14, 19, 20, 21, 37, 39, 42, 49, 58
    ///   and 60 do not;
    /// - classes 3 and 52 (buildings, towers) keep it in state 2 when the player can train or
    ///   will be able to train there (`FUN_141625a20` and `FUN_141610470`'s counts,
    ///   [`Turns::trainable_count`], [`Turns::upgradable_count`], summed as shorts);
    /// - any other class keeps it in state 2, and classes 51 and 54 (the packed and unpacked
    ///   siege) in state 0;
    /// - whatever the class, a wonder (master 276) not in state 0 keeps it in the scenario modes
    ///   ([`crate::turn::scenario_mode`]; the game asks the lobby's settings, `FUN_14152f9c0`,
    ///   which hold the game's options).
    ///
    /// # Errors
    ///
    /// An object of a master the slice does not have.
    pub fn keeps_alive(&self, p: usize, unit: &crate::turn::Unit) -> Result<bool, Unsupported> {
        let master = self.master_of(p, unit.life.master_id)?;
        let state = unit.life.state;
        if (1..=4).contains(&state)
            && master
                .storage
                .iter()
                .any(|&(kind, _, mode)| kind == POPULATION_STORAGE && mode == ALIVE_STORE_MODE)
        {
            return Ok(true);
        }
        let class = i32::from(master.class) as u32;
        let none = class <= 30 && (UNKEPT_LOW >> class) & 1 != 0;
        let builds = class == TOWER_CLASS || class == BUILDING_CLASS;
        let never = class <= 60 && (UNKEPT_HIGH >> class) & 1 != 0;
        let other = !(none || builds || never);
        if other && state == 2 {
            return Ok(true);
        }
        if builds && state == 2 {
            let count = self
                .trainable_count(p, master.id)
                .wrapping_add(self.upgradable_count(p, master.id));
            if 0 < count {
                return Ok(true);
            }
        }
        if (builds || other) && (class == PACKED_CLASS || class == UNPACKED_CLASS) && state == 0 {
            return Ok(true);
        }
        Ok(
            master.id == WONDER
                && state != 0
                && crate::turn::scenario_mode(self.settings.game_mode),
        )
    }

    /// `FUN_141658970`: player `p`'s first built town centre (state 2) among the ids its tracker
    /// files under masters 109 and 444 (`FUN_14104f560`, `FUN_14104f4b0`: the first list, then
    /// the second), the objects the world holds.
    #[must_use]
    pub fn first_town_centre(&self, p: usize) -> Option<i32> {
        let tracker = &self.players[p].tracker;
        TOWN_CENTRES
            .iter()
            .flat_map(|&k| tracker.list(k).iter().copied())
            .find(|&id| self.object(id).is_some_and(|(_, u)| u.life.state == 2))
    }

    /// `FUN_141658a30`: player `p`'s town centres not dying (state below 3): those its tracker
    /// files under masters 109 and 444, then those waiting under 109's pending list
    /// (`FUN_14104fa50`, key + 90000).
    #[must_use]
    pub fn town_centre_count(&self, p: usize) -> i32 {
        let tracker = &self.players[p].tracker;
        let pending = tracker.list(TOWN_CENTRES[0] + crate::tracker::PENDING);
        TOWN_CENTRES
            .iter()
            .flat_map(|&k| tracker.list(k).iter())
            .chain(pending)
            .filter(|&&id| self.object(id).is_some_and(|(_, u)| u.life.state < 3))
            .count() as i32
    }

    /// `FUN_141625a20`'s count with no object (its fifth argument 0): the masters of player `p`,
    /// by id, of type 70 and up that the player has (`+0x5f0` not 0), that train at `building`
    /// ([`Turns::trains_at`]) and whose `+0x5f1` has no bit 0; for each, the limited units'
    /// test of its linked master (`FUN_140c7e1f0`) with `+0x5f1`'s bit 3 (`FUN_1416116c0`,
    /// `FUN_141611520`). With `building` 118 (a builder), attribute 48 at 0 and the town centre
    /// 621 neither had nor its `+0x5f1` set, one more when the town centres
    /// ([`Turns::town_centre_count`]) are fewer than attribute 218. genie's masters keep `+0x5f0`
    /// and `+0x5f1` as booleans ([`crate::masters::Master::available`],
    /// [`crate::masters::Master::disabled`]), a set one as 1: the limit bits 1 to 3 are none, and
    /// the limited units' tests not reached.
    #[must_use]
    pub fn trainable_count(&self, p: usize, building: i16) -> i16 {
        let Some(masters) = self.masters.get(p) else {
            return 0;
        };
        let mut count = 0i16;
        for m in masters.iter() {
            let flags = u8::from(m.disabled);
            if 70 <= m.kind && m.available && self.trains_at(m, building) && flags & 1 == 0 {
                count = count.wrapping_add(1);
            }
        }
        if building == BUILDER {
            let a = &self.players[p].attributes;
            let centre = masters.get(BUILDABLE_TOWN_CENTRE);
            if get(a, attribute::NO_TOWN_CENTRE) == 0.0
                && centre.is_some_and(|c| !c.disabled && !c.available)
                && (self.town_centre_count(p) as f32) < get(a, attribute::TOWN_CENTRE_LIMIT)
            {
                count = count.wrapping_add(1);
            }
        }
        count
    }

    /// `FUN_141610470`'s count with its fifth argument 1: the masters of player `p`, by id, of
    /// type 70 and up that train at `building` ([`Turns::trains_at`]): one with `+0x5f1`'s bit
    /// 1 or 2 counts; any other counts when the tech tree ([`TechTree`]) of the player's
    /// civilisation (the full tech tree's, `FUN_140c7e6f0`, under it) has it at the building
    /// within the ages (unless it is the civilisation's unique unit under the full tech tree:
    /// its elite one once the elite technology is researched, state 3), its link
    /// ([`TechTree::link`]) is no master of the player's or one it has, and its technology
    /// ([`TechTree::tech`]) is none of the player's or researched. With `building` 118,
    /// attribute 48 at 0 and the town centre 621 had with no `+0x5f1`, one more when the town
    /// centres are fewer than attribute 218. genie's masters keep `+0x5f0` and `+0x5f1` as
    /// booleans ([`Turns::trainable_count`]).
    #[must_use]
    pub fn upgradable_count(&self, p: usize, building: i16) -> i16 {
        let Some(masters) = self.masters.get(p) else {
            return 0;
        };
        let player = &self.players[p];
        let full = self.settings.full_tech_tree;
        let fallback = full_tree_civ(self.mode_680);
        let civ = if full {
            fallback
        } else {
            i32::from(player.civ)
        };
        let state = |t: i32| {
            usize::try_from(t)
                .ok()
                .and_then(|t| player.techs.get(t))
                .map(|t| t.state)
        };
        let ages_out = AGES.map(|t| state(t) == Some(-1));
        let unique = self
            .tech_tree
            .unique_units
            .get(usize::from(player.civ))
            .copied()
            .flatten()
            .map_or(-1, |u| match state(u.elite_tech) {
                Some(RESEARCHED) => u.elite,
                Some(_) => u.unit,
                None => -1,
            });
        let mut count = 0i16;
        for m in masters.iter() {
            if m.kind < 70 || !self.trains_at(m, building) {
                continue;
            }
            let flags = u8::from(m.disabled);
            if flags & 6 != 0 {
                count = count.wrapping_add(1);
                continue;
            }
            let q = TreeQuery {
                civ,
                building: i32::from(building),
                unit: i32::from(m.id),
                ages_out,
                settings: &self.settings,
                fallback,
            };
            if (!full || i32::from(m.id) != unique) && !self.tech_tree.has(&q) {
                continue;
            }
            let link = self.tech_tree.link(&q);
            let had = i16::try_from(link)
                .ok()
                .and_then(|l| masters.get(l))
                .map(|l| l.available);
            if had.is_none_or(|a| a) {
                let tech = self.tech_tree.tech(&q);
                if state(tech).is_none_or(|s| s == RESEARCHED) {
                    count = count.wrapping_add(1);
                }
            }
        }
        if building == BUILDER {
            let a = &player.attributes;
            let centre = masters.get(BUILDABLE_TOWN_CENTRE);
            if get(a, attribute::NO_TOWN_CENTRE) == 0.0
                && centre.is_some_and(|c| !c.disabled && c.available)
                && (self.town_centre_count(p) as f32) < get(a, attribute::TOWN_CENTRE_LIMIT)
            {
                count = count.wrapping_add(1);
            }
        }
        count
    }

    /// `FUN_141263610`: master `m` trains at `building`: one of its train locations (`+0x898`)
    /// is `building`, or the building's stack unit (`+0x986`) when the local player's master of
    /// it ([`Turns::local_player`], `FUN_140c88440`) is of class 3, disappears when built
    /// (`+0x984`) and has one (0 or more). A master without train locations trains nowhere.
    #[must_use]
    pub fn trains_at(&self, m: &crate::masters::Master, building: i16) -> bool {
        if m.trained_at.is_empty() {
            return false;
        }
        let mut at = building;
        let local = self
            .masters
            .get(self.local_player)
            .and_then(|t| t.get(building));
        if let Some(b) = local.filter(|b| b.class == BUILDING_CLASS as i16)
            && let Some(site) = b.building.as_ref()
            && site.disappears_when_built
            && 0 <= site.stack_unit
        {
            at = site.stack_unit;
        }
        m.trained_at.contains(&at)
    }

    /// The trickles of player `p`: each of attributes 230 to 233 above 0 (`COMISS` with 0,
    /// `JBE` skips a NaN) adds its sixtieth to food, wood, stone and gold in turn, through the
    /// attribute add with its statistic and its sharing ([`Turns::add_resource`] with 1 and 1).
    pub fn trickles(&mut self, p: usize) {
        for kind in 0..4 {
            let rate = get(&self.players[p].attributes, attribute::TRICKLES + kind);
            if at_most_or_unordered(rate, 0.0) {
                continue;
            }
            self.add_resource(p, kind, rate / MINUTE, 1, true);
        }
    }

    /// The food of player `p`'s mills' livestock: over the ids its tracker files under the mill
    /// (master 68, [`crate::tracker::Tracker::list`]), each object the world holds whose type is
    /// a building's sums its livestock (`mLivestockSummation`, `+0xc28`, in the list's order from
    /// 0); food grows by attribute 254 over 60 times (`time` times logf(sum / 200 + 1))
    /// ([`crate::fmath::log`]), with its statistic and its sharing ([`Turns::add_resource`]),
    /// also when that is 0. `time` is the update time before the loop's step.
    pub fn mill_food(&mut self, p: usize, time: f32) {
        let mut sum = 0.0;
        for &id in self.players[p].tracker.list(MILL) {
            let Some((owner, unit)) = self.object(id) else {
                continue;
            };
            let building = self
                .masters
                .get(owner)
                .and_then(|m| m.get(unit.life.master_id))
                .is_some_and(|m| m.kind == BUILDING_TYPE);
            if building {
                sum += unit.livestock_summation;
            }
        }
        let grows = crate::fmath::log(sum / LIVESTOCK_SCALE + SECOND);
        let rate = get(&self.players[p].attributes, attribute::MILL_FOOD) / MINUTE;
        self.add_resource(p, attribute::FOOD, rate * (time * grows), 1, true);
    }

    /// The world player's attribute add (slot 0xf8, `FUN_141616820`) of `amount` to resource
    /// `kind` (food, wood, stone, gold: 0 to 3) of player `p`, with `stat` (its fourth argument)
    /// and `share` (its fifth): with `stat` 1 the gathered total grows first (166 to 168; gold:
    /// any `stat` but 0 adds to attribute 49 unchecked, then 1 adds to 169), then the resource
    /// ([`Turns::add_attribute_base`]). The resource panel's event when a resource crosses 100
    /// (`FUN_140c898a0` 0x80) is the interface's. The kinds 50, 183 and 196 and the others are
    /// not resources: their branches are [`crate::turn::Player::add_attribute`]'s.
    pub fn add_resource(&mut self, p: usize, kind: usize, amount: f32, stat: u8, share: bool) {
        if kind == attribute::GOLD {
            if stat != 0 {
                let a = &mut self.players[p].attributes;
                let counter = get(a, attribute::GOLD_COUNTER);
                set(a, attribute::GOLD_COUNTER, amount + counter);
                if stat == 1 {
                    self.add_attribute_base(p, attribute::GOLD_GATHERED, amount, 1, share);
                }
            }
        } else if stat == 1 {
            self.add_attribute_base(p, attribute::GATHERED + kind, amount, 1, share);
        }
        self.add_attribute_base(p, kind, amount, stat, share);
    }

    /// The base player's attribute add (`FUN_14165d040`) of `amount` to attribute `kind` of
    /// player `p`, for a kind out of 0 to the attribute count nothing: the attribute takes
    /// `amount` plus its value (the resources are [`crate::turn::Player::resources`]). A
    /// computer player's resource fills its escrow: an `amount` above 0 with a share above 0
    /// (`COMISS`, `JBE`) adds the share times it; an escrow above the resource (`JBE` skips a
    /// NaN) falls to it, or to 0 when the resource is below 0. With `share`, a player who shares
    /// its gathered resources (`+0x1426`, [`crate::turn::Player::sharing`]) gives each resource
    /// to the players, in order, it counts as allies and who count it as theirs
    /// ([`Turns::shares_with`]): their slot 0xf8 with `stat` and no sharing. Attribute 11's peak
    /// (`+0x6fc`, unsaved) is not held; no resource add reaches it.
    fn add_attribute_base(&mut self, p: usize, kind: usize, amount: f32, stat: u8, share: bool) {
        let player = &mut self.players[p];
        if player.attributes.len() <= kind {
            return;
        }
        let value = if kind < 4 {
            &mut player.resources[kind]
        } else {
            &mut player.attributes[kind]
        };
        *value += amount;
        if player.kind == COMPUTER_TYPE && kind < 4 {
            let pct = player.escrow_percentage[kind];
            if !at_most_or_unordered(amount, 0.0) && !at_most_or_unordered(pct, 0.0) {
                player.escrow[kind] += pct * amount;
            }
            let held = player.resources[kind];
            if !at_most_or_unordered(player.escrow[kind], held) {
                player.escrow[kind] = if at_most_or_unordered(0.0, held) {
                    held
                } else {
                    0.0
                };
            }
        }
        if share && player.sharing && kind < 4 {
            for q in 0..self.players.len() {
                if q != p && self.shares_with(p, q) {
                    self.add_resource(q, kind, amount, stat, false);
                }
            }
        }
    }

    /// `FUN_14165d040`'s test of the players a sharing player `p` gives to: player `q` is not
    /// Gaia and one of the world's, `p` counts it as an ally (relation 0 or 4), and `q` counts
    /// `p` as one (its relation to `p` 0 or 4, `& 0xfb`). A relation 1 would count both ways
    /// while the world's treaty holds (`FUN_1415e8e10`: `world+0x820`'s `+0x10` is 1), which the
    /// slice does not model.
    #[must_use]
    pub fn shares_with(&self, p: usize, q: usize) -> bool {
        let n = self.players.len();
        if q == 0 || n <= q || p == q {
            return false;
        }
        let mine = self.players[p].relations.get(q).copied();
        if !matches!(mine, Some(0 | 4)) {
            return false;
        }
        p != 0
            && p < n
            && self.players[q]
                .relations
                .get(p)
                .is_some_and(|&r| r & 0xfb == 0)
    }

    /// `WorldPlayer::update` after `WorldPlayerBase::update`: player `p`'s scout map (`+0x1370`,
    /// once made) takes 1 on each tile its tile list (`+0x180`) holds, in order: until the world
    /// time is 5000 ms only on the tiles the player sees ([`crate::spatial::Spatial::scout_sight`]
    /// is 0xf), after it on all. The list's new count (`+8`, `FUN_141662720`), which it reads and
    /// clears, is the list's length: only the AI's `FUN_140b90b90`, which a playback does not
    /// run, reads it too, and the player's update ends with the list emptied (`FUN_141661030`).
    /// Gaia's update has no tail.
    pub fn world_player_tail(&mut self, p: usize) {
        if self.players[p].kind == GAIA_TYPE || self.spatial.scout_map(p).is_none() {
            return;
        }
        let Some(tiles) = self
            .spatial
            .sight
            .as_ref()
            .and_then(|v| v.new_tiles.get(p))
            .cloned()
        else {
            return;
        };
        for [x, y] in tiles {
            let seen = self.spatial.scout_sight(p, x, y) == Some(0xf);
            if (self.world_time < EARLY_SCOUTING_MS && seen) || EARLY_SCOUTING_MS <= self.world_time
            {
                self.spatial.scout(p, x, y);
            }
        }
    }
}

/// Attribute `k` of `attributes`, 0 past them (the game's array holds them all).
fn get(attributes: &[f32], k: usize) -> f32 {
    attributes.get(k).copied().unwrap_or(0.0)
}

/// Attribute `k` of `attributes` takes `v`; none past them (the game's array holds them all).
fn set(attributes: &mut [f32], k: usize, v: f32) {
    if let Some(a) = attributes.get_mut(k) {
        *a = v;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_status::{LOST, PLAYING};
    use crate::masters::{BuildingMaster, Master, Masters};
    use crate::tracker::Key;
    use crate::turn::{Player, Unit};
    use crate::visibility::{Visibility, VisibleMap};

    /// Attributes 0 to 299, all 0.
    fn attributes() -> Vec<f32> {
        vec![0.0; 300]
    }

    /// A human player (type 1) still playing, with 300 attributes, whose ruins timer keeps it in
    /// the game (the defeat check passes it by).
    fn human() -> Player {
        Player {
            kind: HUMAN_TYPE,
            attributes: attributes(),
            relations: vec![1, 0, 3, 3],
            victory_times: VictoryTimes {
                ruins: 0.0,
                ..VictoryTimes::default()
            },
            ..Player::default()
        }
    }

    /// Gaia and three humans, at world time 10 s, one second's update ahead.
    fn world() -> Turns {
        let gaia = Player {
            kind: GAIA_TYPE,
            attributes: attributes(),
            ..Player::default()
        };
        let mut t = Turns::new(
            10_000,
            1,
            0,
            vec![0; 4],
            vec![gaia, human(), human(), human()],
        );
        t.dt_seconds = 1.0;
        t
    }

    /// A master of `kind` and `class`, `id`.
    fn master(id: i16, kind: u8, class: i16) -> Master {
        Master {
            id,
            kind,
            class,
            available: true,
            ..Master::default()
        }
    }

    /// An object `id` of master `m` in `state`.
    fn object(id: i32, m: i16, state: u8) -> Unit {
        let mut u = Unit {
            id,
            ..Unit::default()
        };
        u.life.master_id = m;
        u.life.state = state;
        u
    }

    /// The update time grows by the update's seconds; each whole second runs the loop once (the
    /// history's update counts it) with the time before its step; Gaia's own time grows alone.
    #[test]
    fn each_whole_second_of_update_time_runs_the_loop_once() {
        let mut t = world();
        t.dt_seconds = 0.25;
        t.players[1].update_time = 0.9;
        t.update_world_player(1).unwrap();
        assert_eq!(t.players[1].update_time, (0.25f32 + 0.9) - 1.0);
        assert_eq!(t.players[1].history.update_count, 1);
        t.dt_seconds = 2.5;
        t.players[2].update_time = 0.0;
        t.update_world_player(2).unwrap();
        assert_eq!(t.players[2].update_time, 0.5);
        assert_eq!(t.players[2].history.update_count, 2);
        // below a second, or NaN, no loop
        t.dt_seconds = 0.5;
        t.players[3].update_time = f32::NAN;
        t.update_world_player(3).unwrap();
        assert_eq!(t.players[3].history.update_count, 0);
        // Gaia: its own update time, no loop
        t.update_world_player(0).unwrap();
        assert_eq!(
            (t.players[0].gaia_update_time, t.players[0].update_time),
            (0.5, 0.0)
        );
        assert_eq!(t.players[0].history.update_count, 0);
    }

    /// Attribute 22: the explored tiles over the map's, in percent; 100 under the explored and
    /// all-visible types; left in a slice without the players' sight.
    #[test]
    fn attribute_22_is_the_explored_share_in_percent() {
        let mut t = world();
        t.players[1].attributes[22] = 7.0;
        t.update_world_player(1).unwrap();
        assert_eq!(t.players[1].attributes[22], 7.0);
        let map = |explored| VisibleMap {
            counts: vec![-1; 100],
            explored,
        };
        t.spatial.sight = Some(Visibility::new(
            10,
            10,
            vec![map(0), map(37), map(0), map(0)],
            vec![0; 100],
            Vec::new(),
            0,
            vec![false; 4],
        ));
        t.update_world_player(1).unwrap();
        assert_eq!(t.players[1].attributes[22], 37.0 / 100.0 * 100.0);
        t.settings.visibility_type = 2;
        t.update_world_player(1).unwrap();
        assert_eq!(t.players[1].attributes[22], 100.0);
    }

    /// Attribute 44: kills (20) less the units owned (19) less the population (11); below 0 it
    /// is 0, a NaN stays.
    #[test]
    fn attribute_44_is_the_kills_less_the_losses() {
        let mut t = world();
        let a = &mut t.players[1].attributes;
        (a[20], a[19], a[11]) = (5.0, 12.0, 10.0);
        t.update_world_player(1).unwrap();
        assert_eq!(t.players[1].attributes[44], 3.0);
        t.players[1].attributes[19] = 20.0;
        t.update_world_player(1).unwrap();
        assert_eq!(t.players[1].attributes[44], 0.0);
        t.players[1].attributes[20] = f32::NAN;
        t.update_world_player(1).unwrap();
        assert!(t.players[1].attributes[44].is_nan());
    }

    /// The trickles: each of attributes 230 to 233 above 0 gives its sixtieth of the resource
    /// and of its gathered total (gold's also of attribute 49); 0, below 0 and NaN give nothing.
    /// A lost player's trickles stop.
    #[test]
    fn the_trickles_give_a_sixtieth_of_their_rate_each_second() {
        let mut t = world();
        let a = &mut t.players[1].attributes;
        (a[230], a[231], a[232], a[233]) = (6.0, -1.0, f32::NAN, 60.0);
        t.update_world_player(1).unwrap();
        let p = &t.players[1];
        assert_eq!(p.resources, [6.0 / 60.0, 0.0, 0.0, 1.0]);
        assert_eq!(
            [166, 167, 168, 169, 49].map(|k| p.attributes[k]),
            [6.0 / 60.0, 0.0, 0.0, 1.0, 1.0]
        );
        t.players[1].game_status = LOST;
        t.update_world_player(1).unwrap();
        assert_eq!(t.players[1].resources[3], 1.0);
    }

    /// The mills' food: the livestock of the objects filed under the mill (68) that are
    /// buildings sums (another type's is passed by); food grows by attribute 254 over 60 times
    /// (the update time before the step times logf(sum / 200 + 1)), its gathered total too.
    #[test]
    fn a_mill_s_livestock_makes_food() {
        let mut t = world();
        let masters = Masters::new([master(68, 80, 3), master(5, 70, 58)]);
        t.masters = vec![masters.clone(), masters.clone(), masters.clone(), masters];
        let mut mill = object(10, 68, 2);
        mill.livestock_summation = 100.0;
        let mut other = object(11, 68, 2);
        other.livestock_summation = 300.0;
        let mut sheep = object(12, 5, 2);
        sheep.livestock_summation = 1000.0;
        let key = Key {
            master: 68,
            class: 3,
            pending: false,
            military: false,
        };
        let p = &mut t.players[1];
        p.objects = vec![mill, other, sheep];
        for id in [10, 11, 12] {
            p.tracker.add(id, key);
        }
        p.attributes[254] = 15.0;
        p.update_time = 0.5;
        t.update_world_player(1).unwrap();
        let grows = crate::fmath::log(400.0 / 200.0 + 1.0);
        let food = 15.0 / 60.0 * (1.5 * grows);
        assert_eq!(t.players[1].resources[0], food);
        assert_eq!(t.players[1].attributes[166], food);
    }

    /// A computer player's income fills its escrow by the share, held to the resource; with
    /// sharing on, the allies that count it as theirs get the resource and its gathered total,
    /// an enemy, Gaia and a one-sided ally nothing.
    #[test]
    fn the_income_fills_a_computer_s_escrow_and_goes_to_mutual_allies() {
        let mut t = world();
        t.players[1].kind = COMPUTER_TYPE;
        t.players[1].escrow_percentage = [0.5, 0.0, 0.0, 0.0];
        t.add_resource(1, 0, 10.0, 1, true);
        assert_eq!(
            (t.players[1].resources[0], t.players[1].escrow[0]),
            (10.0, 5.0)
        );
        t.add_resource(1, 0, -20.0, 1, true);
        assert_eq!(t.players[1].escrow[0], 0.0);
        // sharing: 1 and 2 count each other, 3 counts 1 but not the other way round
        let mut t = world();
        t.players[1].sharing = true;
        t.players[1].relations = vec![1, 0, 0, 3];
        t.players[2].relations = vec![1, 0, 0, 3];
        t.players[3].relations = vec![1, 0, 3, 0];
        t.add_resource(1, 1, 4.0, 1, true);
        assert_eq!(
            [1, 2, 3].map(|p| (t.players[p].resources[1], t.players[p].attributes[167])),
            [(4.0, 4.0), (4.0, 4.0), (0.0, 0.0)]
        );
        assert_eq!(t.players[0].resources[1], 0.0);
        assert!(t.shares_with(1, 2) && !t.shares_with(1, 3) && !t.shares_with(1, 0));
    }

    /// The defeat check: a unit in state 2 keeps its player; a wall or a farm does not; a
    /// building keeps it while something trains there; a packed trebuchet in state 0; a wonder in
    /// a scenario mode; a storage of attribute 11 in store mode 0x20. A player kept by nothing
    /// loses: the status setter takes 2 and attribute 45 is cleared.
    #[test]
    fn a_player_without_an_object_that_keeps_it_loses() {
        let masters = Masters::new([
            master(83, 70, 4),
            master(117, 80, 27),
            master(50, 80, 49),
            master(109, 80, 3),
            master(12, 80, 3),
            master(331, 70, 51),
            master(276, 80, 3),
            Master {
                storage: vec![(11, 1.0, 0x20), (-1, 0.0, 0), (-1, 0.0, 0)],
                ..master(900, 70, 11)
            },
            Master {
                trained_at: vec![109],
                ..master(84, 70, 4)
            },
        ]);
        let keeps = |objects: Vec<Unit>, game_mode: i32| {
            let mut t = world();
            t.masters = vec![masters.clone(); 4];
            t.settings.game_mode = game_mode;
            t.players[1].objects = objects;
            let p = &t.players[1];
            let kept: Vec<bool> = p
                .objects
                .iter()
                .map(|u| t.keeps_alive(1, u).unwrap())
                .collect();
            kept
        };
        assert_eq!(
            keeps(
                vec![
                    object(1, 83, 2),
                    object(2, 83, 3),
                    object(3, 117, 2),
                    object(4, 50, 2),
                    object(5, 109, 2),
                    object(6, 12, 2),
                    object(7, 331, 0),
                    object(8, 331, 1),
                    object(9, 276, 2),
                    object(10, 900, 3),
                    object(11, 900, 5),
                ],
                0
            ),
            [
                true, false, false, false, true, false, true, false, false, true, false
            ]
        );
        assert_eq!(keeps(vec![object(9, 276, 2)], 3), [true]);
        assert_eq!(keeps(vec![object(9, 276, 0)], 3), [false]);
        // a player with only a wall loses, attribute 45 cleared
        let mut t = world();
        t.masters = vec![masters.clone(); 4];
        t.players[1].victory_times = VictoryTimes::default();
        t.players[1].objects = vec![object(3, 117, 2)];
        t.players[1].attributes[45] = 1.0;
        t.update_world_player(1).unwrap();
        assert_eq!(t.players[1].game_status, LOST);
        assert_eq!(t.players[1].attributes[45], 0.0);
        // one that a villager keeps plays on
        let mut t = world();
        t.masters = vec![masters; 4];
        t.players[1].victory_times = VictoryTimes::default();
        t.players[1].objects = vec![object(3, 117, 2), object(1, 83, 2)];
        t.players[1].attributes[45] = 1.0;
        t.update_world_player(1).unwrap();
        assert_eq!(t.players[1].game_status, PLAYING);
        assert_eq!(t.players[1].attributes[45], 1.0);
    }

    /// Each victory timer at 0 or more keeps the player; −1 and NaN do not.
    #[test]
    fn a_running_victory_timer_keeps_the_player() {
        assert!(!VictoryTimes::default().running());
        for t in [
            VictoryTimes {
                wonder: 0.0,
                ..VictoryTimes::default()
            },
            VictoryTimes {
                ruins: 12.5,
                ..VictoryTimes::default()
            },
            VictoryTimes {
                artifacts: 0.0,
                ..VictoryTimes::default()
            },
        ] {
            assert!(t.running(), "{t:?}");
        }
        let nan = VictoryTimes {
            wonder: f32::NAN,
            ..VictoryTimes::default()
        };
        assert!(!nan.running());
    }

    /// Sudden death: the first built town centre found sets the latch; once set, a player
    /// without one loses. Regicide: a player without a king loses though its units keep it.
    #[test]
    fn sudden_death_and_regicide_end_a_player_their_ways() {
        let masters = Masters::new([master(83, 70, 4), master(109, 80, 3)]);
        let tc_key = Key {
            master: 109,
            class: 3,
            pending: false,
            military: false,
        };
        let mut t = world();
        t.masters = vec![masters.clone(); 4];
        t.settings.game_mode = 11;
        t.players[1].victory_times = VictoryTimes::default();
        t.players[1].objects = vec![object(1, 83, 2), object(5, 109, 2)];
        t.players[1].tracker.add(5, tc_key);
        t.update_world_player(1).unwrap();
        assert!(t.players[1].sudden_death_eliminable);
        assert_eq!(t.players[1].game_status, PLAYING);
        assert_eq!(t.first_town_centre(1), Some(5));
        // the town centre falls (state 3): the villager no longer keeps the player
        t.players[1].objects[1].life.state = 3;
        assert_eq!(t.first_town_centre(1), None);
        t.update_world_player(1).unwrap();
        assert_eq!(t.players[1].game_status, LOST);
        // regicide (sub-mode bit 2): no king, a villager notwithstanding
        let mut t = world();
        t.masters = vec![masters; 4];
        t.settings.sub_game_mode_flag = 4;
        t.players[1].objects = vec![object(1, 83, 2)];
        t.update_world_player(1).unwrap();
        assert_eq!(t.players[1].game_status, LOST);
        let mut t2 = world();
        t2.settings.game_mode = 1;
        t2.players[1].tracker.add(
            7,
            Key {
                master: KING,
                class: 59,
                pending: false,
                military: true,
            },
        );
        t2.update_world_player(1).unwrap();
        assert_eq!(t2.players[1].game_status, PLAYING);
    }

    /// `FUN_141625a20`'s count: masters of type 70 and up, had, not disabled, trained at the
    /// building or at the stack unit of the local player's building of class 3 that disappears
    /// when built.
    #[test]
    fn the_trainable_count_takes_the_had_masters_trained_at_the_building() {
        let mut t = world();
        let site = BuildingMaster {
            disappears_when_built: true,
            stack_unit: 109,
            ..BuildingMaster::default()
        };
        let masters = Masters::new([
            Master {
                trained_at: vec![109],
                ..master(83, 70, 4)
            },
            Master {
                trained_at: vec![109],
                available: false,
                ..master(84, 70, 4)
            },
            Master {
                trained_at: vec![109],
                disabled: true,
                ..master(85, 70, 4)
            },
            Master {
                trained_at: vec![109],
                ..master(86, 60, 4)
            },
            Master {
                trained_at: vec![12],
                ..master(74, 70, 6)
            },
            master(109, 80, 3),
            Master {
                building: Some(site),
                ..master(621, 80, 3)
            },
        ]);
        t.masters = vec![masters.clone(); 4];
        assert_eq!(t.trainable_count(1, 109), 1);
        assert_eq!(t.trainable_count(1, 12), 1);
        assert_eq!(t.trainable_count(1, 101), 0);
        // the local player's 621 stands for 109
        assert_eq!(t.trainable_count(1, 621), 1);
        // a builder with attribute 48 at 0 and the town centre not had: one more below the
        // town centre limit (218)
        let mut builder = masters;
        builder.get_mut(621).unwrap().available = false;
        t.masters[1] = builder;
        t.players[1].attributes[218] = 1.0;
        assert_eq!(t.trainable_count(1, BUILDER), 1);
        t.players[1].attributes[48] = 1.0;
        assert_eq!(t.trainable_count(1, BUILDER), 0);
    }

    /// The tech tree's lookups: a unit at its building, its link and technology; a later age
    /// than the game's last less 1 is out, and so are ages the player cannot reach; a building
    /// the civilisation lacks under the full tech tree is the fallback's.
    #[test]
    fn the_tech_tree_answers_by_civilisation_building_and_unit() {
        let unit = |id, link, age, tech| TreeUnit {
            id,
            link,
            age,
            tech,
        };
        let tree = TechTree {
            civs: vec![
                vec![TreeBuilding {
                    id: 12,
                    units: vec![unit(74, 75, 1, 87), unit(93, -1, 3, 197)],
                }],
                vec![TreeBuilding {
                    id: 87,
                    units: vec![unit(4, 24, 2, -1)],
                }],
            ],
            unique_units: Vec::new(),
        };
        let settings = crate::settings::Settings::default();
        let q = |civ, building, unit, ages_out| TreeQuery {
            civ,
            building,
            unit,
            ages_out,
            settings: &settings,
            fallback: 0,
        };
        let none = [false; 3];
        assert!(tree.has(&q(0, 12, 74, none)));
        assert_eq!(
            (
                tree.link(&q(0, 12, 74, none)),
                tree.tech(&q(0, 12, 74, none))
            ),
            (75, 87)
        );
        assert!(!tree.has(&q(0, 12, 4, none)));
        assert_eq!(tree.link(&q(0, 12, 4, none)), -1);
        assert!(!tree.has(&q(0, 87, 4, none)));
        assert_eq!(tree.link(&q(5, 12, 74, none)), 0);
        // the Castle Age out of reach
        assert!(!tree.has(&q(0, 12, 93, [false, true, false])));
        assert_eq!(tree.tech(&q(0, 12, 93, [false, true, false])), 0);
        // the last age 3: age 3 is out
        let early = crate::settings::Settings {
            ending_age: 3,
            ..crate::settings::Settings::default()
        };
        let late = TreeQuery {
            settings: &early,
            ..q(0, 12, 93, none)
        };
        assert!(!tree.has(&late));
        // the full tech tree: civilisation 1 lacks the barracks, the fallback 0 has it
        let full = crate::settings::Settings {
            full_tech_tree: true,
            ..crate::settings::Settings::default()
        };
        let fell = TreeQuery {
            settings: &full,
            ..q(1, 12, 74, none)
        };
        assert!(tree.has(&fell));
        assert_eq!(tree.link(&fell), 75);
        assert!(!tree.has(&q(1, 12, 74, none)));
    }

    /// `FUN_141610470`'s count: a master trained at the building counts when the tree has it,
    /// its link is had (or none) and its technology researched (or none).
    #[test]
    fn the_upgradable_count_follows_the_tech_tree() {
        let mut t = world();
        let masters = Masters::new([
            Master {
                trained_at: vec![12],
                available: false,
                ..master(74, 70, 6)
            },
            Master {
                trained_at: vec![12],
                available: false,
                ..master(75, 70, 6)
            },
            master(12, 80, 3),
        ]);
        t.masters = vec![masters; 4];
        t.players[1].civ = 0;
        t.players[1].techs = vec![crate::tech::TechState::default(); 200];
        t.tech_tree = TechTree {
            civs: vec![vec![TreeBuilding {
                id: 12,
                units: vec![
                    TreeUnit {
                        id: 74,
                        link: -1,
                        age: 1,
                        tech: -1,
                    },
                    TreeUnit {
                        id: 75,
                        link: 74,
                        age: 2,
                        tech: 150,
                    },
                ],
            }]],
            unique_units: Vec::new(),
        };
        // 74 counts (no link, no technology); 75's link 74 is not had
        assert_eq!(t.upgradable_count(1, 12), 1);
        t.masters[1].get_mut(74).unwrap().available = true;
        // 75: its link had, its technology not researched
        assert_eq!(t.upgradable_count(1, 12), 1);
        t.players[1].techs[150].state = RESEARCHED;
        assert_eq!(t.upgradable_count(1, 12), 2);
        // the Feudal Age out of reach puts 75 out
        t.players[1].techs[101].state = -1;
        assert_eq!(t.upgradable_count(1, 12), 1);
    }

    /// The scout map takes the tiles of the player's tile list: before 5000 ms only those it
    /// sees, after it all; a player without a scout map, and Gaia, take nothing.
    #[test]
    fn the_scout_map_takes_the_new_tiles() {
        let mut t = world();
        let mut unified = vec![0u64; 100];
        unified[0] = 1 << 1;
        let map = VisibleMap {
            counts: vec![-1; 100],
            explored: 0,
        };
        let mut sight = Visibility::new(
            10,
            10,
            vec![map.clone(), map.clone(), map.clone(), map],
            unified,
            Vec::new(),
            0,
            vec![false; 4],
        );
        sight.new_tiles[1] = vec![[0, 0], [1, 0]];
        t.spatial.sight = Some(sight);
        t.world_player_tail(1);
        t.spatial
            .set_scout_map(1, crate::explore::InfluenceMap::new([10, 10], 0));
        t.world_time = 4999;
        t.world_player_tail(1);
        let m = t.spatial.scout_map(1).unwrap();
        assert_eq!((m.get(0, 0), m.get(1, 0)), (Some(1), Some(0)));
        t.world_time = 5000;
        t.world_player_tail(1);
        assert_eq!(t.spatial.scout_map(1).unwrap().get(1, 0), Some(1));
    }
}
