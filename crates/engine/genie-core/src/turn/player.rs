//! A player ([`Player`]): its objects, attributes and formations, and the stop of its update.

use crate::GenieError;
use crate::objects::unit::Unit;
use crate::turn::Turns;

/// A player's settings for the groups an order's splitter forms ([`Player::formations`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Formations {
    /// The splitter groups units at all (`+0x6c0` not 0).
    pub auto: bool,
    /// How near two units join (`+0x6c4`).
    pub influence: f32,
    /// Units whose speeds differ by more than [`crate::orders`]'s gap do not join (`+0x6c8` 1).
    pub by_speed: bool,
    /// `mMinColumnDistance` (`+0x6b8`): a group's move along a longer path forms a column
    /// (`commandMoveTo`).
    pub min_column_distance: i32,
    /// `mColumnToLineDistance` (`+0x6bc`): where a column on its last waypoint turns to a line
    /// (`UpdateDesiredPositions`).
    pub column_to_line_distance: i32,
}

/// The settings of a player's AI the GAME command's sub-command 0xb sets one by one
/// (`FUN_140e7c330`'s inner switch), each saved (`WorldPlayerBase::save`, `FUN_14166b820`) and
/// read by its unit AIs. The two chances it sets too are [`Player::dodge_chance`] and
/// [`Player::keep_distance_chance`], and `mAllowCapturingGaiaUnits` is
/// [`crate::turn::Turns::allow_gaia_capture`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AiSettings {
    /// `easiest_reaction_percentage` (`+0x608`, setting 3).
    pub easiest_reaction_percentage: i32,
    /// `easier_reaction_percentage` (`+0x60c`, setting 4).
    pub easier_reaction_percentage: i32,
    /// `task_ungrouped_soldiers` (`+0x610`, setting 5).
    pub task_ungrouped_soldiers: u8,
    /// `may_attack_walls` (`+0x611`, setting 6).
    pub may_attack_walls: bool,
    /// `livestock_only_to_tc` (`+0x612`, setting 7).
    pub livestock_only_to_tc: bool,
    /// `disable_build_help` (`+0x613`, setting 8).
    pub disable_build_help: bool,
    /// `local_targeting_mode` (`+0x614`, setting 9).
    pub local_targeting_mode: i32,
    /// `targeting_factor_scalar` (`+0x618`, setting 10).
    pub targeting_factor_scalar: i32,
    /// `preferredTradeDistance` (`+0x61c`, setting 0xd).
    pub preferred_trade_distance: f32,
    /// `disableTowerPriority` (`+0x620`, setting 0xe).
    pub disable_tower_priority: bool,
    /// `tradeAttackEvasion` (`+0x621`, setting 0xb).
    pub trade_attack_evasion: bool,
    /// `mVillagerAttackReset` (`+0x622`, setting 0x10).
    pub villager_attack_reset: bool,
    /// `boarLureDestination` (`+0x623`, setting 0xc).
    pub boar_lure_destination: u8,
    /// `mAttackMoveMaxDistance` (`+0x624`, setting 0xf).
    pub attack_move_max_distance: i32,
    /// `mDisableVillagerGarrison` (`+0x650`, setting 0x11).
    pub disable_villager_garrison: i32,
    /// `mAllowSerjeantBuilding` (`+0x660`, setting 0x12).
    pub allow_serjeant_building: bool,
}

/// One player's objects in live-list order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Player {
    /// Its live objects, in their list's order.
    pub objects: Vec<Unit>,
    /// `player+0x10 == 3`.
    pub computer: bool,
    /// `player+0x10`, the player's type (the save's class byte): 1 human, 2 Gaia, 3 computer
    /// (0 the scenario editor's phantom).
    pub kind: u8,
    /// Food, wood, stone and gold (the player's attributes 0 to 3).
    pub resources: [f32; 4],
    /// Attribute 195, the construction rate modifier, which builders' work reads.
    pub construction_rate: f32,
    /// Every attribute as saved (the first four are `resources`, which the slice keeps); the
    /// task finder reads attribute 95, the work command a task's enabling resource.
    pub attributes: Vec<f32>,
    /// `+0x658` (saved, the save's second attribute array): per attribute what the player's
    /// generation tasks made and did not give yet ([`crate::combat::abilities::store`]).
    pub generated: Vec<f32>,
    /// The civ (`player+0x738`), which drop sites read.
    pub civ: u8,
    /// `relation[i]` (`player+0x260`): how the player stands to player i.
    pub relations: Vec<u8>,
    /// The sleeping objects, in their list's order: they are not updated; a unit's work on one
    /// wakes it.
    pub sleeping: Vec<Unit>,
    /// `autoFarmQueue` (`+0x1424`, saved), which GAME 0x10 flips: a farmer whose farm ran out
    /// buys another one without the mill's queue (`FUN_141618260`).
    pub auto_farm_queue: bool,
    /// `autoFishTrapQueue` (`+0x1425`, saved), which GAME 0x13 flips (`FUN_141618270`).
    pub auto_fishtrap_queue: bool,
    /// `farm_queue_count` (`+0x141c`, saved): the farms and pastures the mill's queue paid for
    /// (GAME 0xd and 0x15 buy, 0xe and 0x16 take back, [`crate::gathering::farm_queue`]).
    pub farm_queue: i32,
    /// `fishTrapQueueCount` (`+0x1420`, saved): the fish traps the dock's queue paid for (GAME
    /// 0x11 buys, 0x12 takes back).
    pub fish_trap_queue: i32,
    /// `prod_queue_on` (`+0x1358`, saved): GAME 9 sets it; nothing in the simulation reads it.
    pub production_queue_on: u8,
    /// `diplomacy[i]` (`+0xe12`, saved, 9 entries): the stance toward player i a diplomacy change
    /// set (1 or 2 enemy, 3 neutral, 4 or 5 ally; [`crate::players::diplomacy`]).
    pub diplomacy: Vec<u8>,
    /// The settings a computer player's GAME 0xb sets (`+0x608`..`+0x670`, saved).
    pub ai_settings: AiSettings,
    /// The order splitter's settings (`+0x6c0` `mAutoFormations`, `+0x6c4`
    /// `mFormationInfluenceDistance`, `+0x6c8` `mBreakAutoFormationsBySpeed`).
    pub formations: Formations,
    /// `mPendingDebit` (`+0x6cc`): what the player's BUILD and RESEARCH commands listed as their
    /// costs, food, wood, stone and gold, taken off (`FUN_141630d10`); nothing modelled reads it.
    pub pending_debit: [f32; 4],
    /// The player's state of each technology (`player+0x760`'s, saved).
    pub techs: Vec<crate::technology::TechState>,
    /// The first update this slice could not model, with the tick (1-based) and object index.
    pub unsupported: Option<(u32, usize, GenieError)>,
    /// Every object the slice stopped updating, with the tick and why.
    pub freezes: Vec<(u32, i32, GenieError)>,
    /// A command the slice could not model stopped the whole player: its objects freeze.
    pub stopped: bool,
    /// `game_status` (`+0x89`, saved): playing, won or lost ([`crate::players::game_status`]).
    pub game_status: u8,
    /// `+0x8b` (saved with `+0x8c` as its bit 1): the player resigned (slot 3, RESIGN).
    pub resigned: bool,
    /// `+0x8c`: its resignation was a drop from the game.
    pub dropped: bool,
    /// `allied_victory` (`+0x2d0`, saved, a byte GAME 5 sets): its allies' victory counts as its
    /// own while it is not 0; the teams count it only at 1 ([`crate::players::diplomacy`]).
    pub allied_victory: u8,
    /// Its victory conditions' state (`player+0x68`'s `+0x20`, the save's `victory`): 2 when
    /// they are met (`FUN_1414f35d0`).
    pub victory: u8,
    /// `fealty` (`+0x12f8`, saved): the player a defeated player's resources and units would go
    /// to.
    pub fealty: i16,
    /// `+0x71c`, the offensive posture (0 or 1, GAME 0x14): not saved, a load sets 0; new soldiers
    /// and scorpions read it (their unit AI's constructor, [`crate::unit_ai::NewObject`]).
    pub posture: i32,
    /// `chance_to_dodge_missiles` (`+0xe10`, saved): a ship's unit AI dodges a missile's warning
    /// (`0x20f`) on a draw below it ([`crate::unit_ai::Surroundings::dodge_chance`]).
    pub dodge_chance: u8,
    /// `chance_for_archers_to_maintain_distance` (`+0xe11`, saved): a computer player's siege
    /// unit backs up from its target after a shot on a draw at least this
    /// ([`crate::unit_ai::Surroundings::keep_distance_chance`]).
    pub keep_distance_chance: u8,
    /// `allied_LOS_Enable` (`+0x2cc`, saved): the player shares its mutual allies' sight
    /// ([`crate::vision`]).
    pub allied_vision: bool,
    /// The players whose town centres the player's reveal (attribute 209) flared
    /// (`mTCFlared[i]`, saved; [`crate::vision`]).
    pub tc_flared: Vec<bool>,
    /// `mSharingIsCaring` (`+0x1426`, `FUN_141626780`, saved): the player's gathered resources go
    /// to its allies too.
    pub sharing: bool,
    /// `commodity_volumn_delta` (`+0x140c`, saved): the lot a market trade moves (100).
    pub commodity_volume: i32,
    /// The objects whose ghosts the player's list follows (`+0x78`, [`crate::vision::ghost`]): not
    /// saved; the load builds it ([`Turns::build_watched`]).
    pub watched: Vec<crate::vision::ghost::Watched>,
    /// The player's ghosts (its doppelganger list `+0x40`), in the list's order.
    pub ghosts: Vec<crate::vision::ghost::Ghost>,
    /// Its history (`+0xdb8`, saved, [`crate::players::history`]).
    pub history: crate::players::history::History,
    /// Its object tracker (`+0x18`, [`crate::players::tracker`]): not saved; the load files the objects
    /// ([`Turns::load_trackers`]).
    pub tracker: crate::players::tracker::Tracker,
    /// `+0x1378` (not saved): in game mode 16 the player has a military unit queued while its
    /// attribute 6 is at most 1 (QUEUE sets it, `FUN_141614450`; a cancel clears it,
    /// `FUN_14161e370`).
    pub one_military_queued: bool,
    /// `mUpdateTime` (`+0x7a4`, saved): the seconds the world player's update has not yet run
    /// its per-second loop for ([`crate::players::world_player`]); Gaia's stays 0.
    pub update_time: f32,
    /// Gaia's own `update_time` (`WorldPlayerGaia` `+0x1468`, saved): its update's seconds.
    pub gaia_update_time: f32,
    /// `escrow` (`+0x6dc`, saved): what a computer player's income set aside of food, wood,
    /// stone and gold ([`Turns::add_resource`]).
    pub escrow: [f32; 4],
    /// `escrow_percentage` (`+0x6ec`, saved): the shares of its income a computer player sets
    /// aside.
    pub escrow_percentage: [f32; 4],
    /// `mSuddenDeathCanBeEliminated` (`+0x1380`, saved): under the sudden death rule the player
    /// had a town centre, and loses without one.
    pub sudden_death_eliminable: bool,
    /// The victory timers that keep a defeated player in the game ([`crate::players::world_player`]).
    pub victory_times: crate::players::world_player::VictoryTimes,
}

/// An attribute the player does not have.
pub(crate) const NO_ATTRIBUTE: GenieError =
    GenieError::unsupported("an attribute the player does not have");

impl crate::technology::Attributes for Player {
    /// An attribute's value (the resources' from [`Player::resources`]).
    fn get(&self, i: i16) -> Result<f32, GenieError> {
        match usize::try_from(i) {
            Ok(r @ 0..=3) => Ok(self.resources[r]),
            Ok(k) => self.attributes.get(k).copied().ok_or(NO_ATTRIBUTE),
            Err(_) => Err(GenieError::unsupported("a negative attribute")),
        }
    }

    /// An attribute set to `v`, as a technology's payment writes it (no side effects).
    fn set(&mut self, i: i16, v: f32) -> Result<(), GenieError> {
        match usize::try_from(i) {
            Ok(r @ 0..=3) => self.resources[r] = v,
            Ok(k) => {
                *self.attributes.get_mut(k).ok_or(NO_ATTRIBUTE)? = v;
            }
            Err(_) => return Err(GenieError::unsupported("a negative attribute")),
        }
        Ok(())
    }
}

impl Turns {
    /// A command or a pass the slice does not model for player `p`: the player stops
    /// ([`Player::stop`]), and a subscriber hears why ([`crate::trace`]).
    pub(crate) fn stop_player(&mut self, p: usize, why: GenieError) {
        crate::trace::player_stops(self.ticks, p, &why);
        self.players[p].stop(self.ticks, why);
    }
}

impl Player {
    /// A command the slice does not model for this player: the player stops.
    pub(crate) fn stop(&mut self, tick: u32, why: GenieError) {
        let _ = self.unsupported.get_or_insert((tick, 0, why));
        self.stopped = true;
    }

    /// Attribute `i` (`player+0x100`, decoded; the first four are [`Player::resources`]), None
    /// past its count (`+0x90`, the saved attributes').
    #[must_use]
    pub fn attribute(&self, i: usize) -> Option<f32> {
        let value = self.attributes.get(i).copied()?;
        Some(if i < 4 { self.resources[i] } else { value })
    }

    /// `FUN_141616820` for the attributes an object's storage holds: the attribute's value grows
    /// by `amount` (`FUN_14165d040`; the peak of attribute 11, `+0x6fc`, and the resource events of
    /// `FUN_140c898a0` are not held). Food, wood, stone and gold are the slice's `resources`; the
    /// attributes whose change has side effects (50, 183, 196) are not modelled.
    pub fn add_attribute(&mut self, kind: i16, amount: f32) -> Result<(), GenieError> {
        let i =
            usize::try_from(kind).map_err(|_| GenieError::unsupported("a negative attribute"))?;
        match i {
            50 | 183 | 196 => Err(GenieError::unsupported(
                "an attribute whose change has side effects",
            )),
            0..=3 => {
                self.resources[i] += amount;
                Ok(())
            }
            _ => {
                let v = self.attributes.get_mut(i).ok_or(NO_ATTRIBUTE)?;
                *v += amount;
                Ok(())
            }
        }
    }
}
