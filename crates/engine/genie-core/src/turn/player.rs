//! A player ([`Player`]): its objects, attributes and formations, and the stop of its update.

use crate::turn::Turns;
use crate::turn::unit::Unit;
use crate::unit_ai::Unsupported;

/// A player's settings for the groups an order's splitter forms ([`Player::formations`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Formations {
    /// The splitter groups units at all (`+0x6c0` not 0).
    pub auto: bool,
    /// How near two units join (`+0x6c4`).
    pub influence: f32,
    /// Units whose speeds differ by more than [`crate::order`]'s gap do not join (`+0x6c8` 1).
    pub by_speed: bool,
    /// `mMinColumnDistance` (`+0x6b8`): a group's move along a longer path forms a column
    /// (`commandMoveTo`).
    pub min_column_distance: i32,
    /// `mColumnToLineDistance` (`+0x6bc`): where a column on its last waypoint turns to a line
    /// (`UpdateDesiredPositions`).
    pub column_to_line_distance: i32,
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
    /// The civ (`player+0x738`), which drop sites read.
    pub civ: u8,
    /// `relation[i]` (`player+0x260`): how the player stands to player i.
    pub relations: Vec<u8>,
    /// The sleeping objects, in their list's order: they are not updated; a unit's work on one
    /// wakes it.
    pub sleeping: Vec<Unit>,
    /// `autoFarmQueue` (`+0x1424`) and `autoFishTrapQueue` (`+0x1425`), which GAME 0x10 and 0x13
    /// flip; nothing the slice models reads them yet.
    pub auto_farm_queue: bool,
    /// `autoFishTrapQueue` (`+0x1425`).
    pub auto_fishtrap_queue: bool,
    /// The order splitter's settings (`+0x6c0` `mAutoFormations`, `+0x6c4`
    /// `mFormationInfluenceDistance`, `+0x6c8` `mBreakAutoFormationsBySpeed`).
    pub formations: Formations,
    /// `mPendingDebit` (`+0x6cc`): what the player's BUILD and RESEARCH commands listed as their
    /// costs, food, wood, stone and gold, taken off (`FUN_141630d10`); nothing modelled reads it.
    pub pending_debit: [f32; 4],
    /// The player's state of each technology (`player+0x760`'s, saved).
    pub techs: Vec<crate::tech::TechState>,
    /// The first update this slice could not model, with the tick (1-based) and object index.
    pub unsupported: Option<(u32, usize, Unsupported)>,
    /// Every object the slice stopped updating, with the tick and why.
    pub freezes: Vec<(u32, i32, Unsupported)>,
    /// A command the slice could not model stopped the whole player: its objects freeze.
    pub stopped: bool,
    /// `game_status` (`+0x89`, saved): playing, won or lost ([`crate::game_status`]).
    pub game_status: u8,
    /// `+0x8b` (saved with `+0x8c` as its bit 1): the player resigned (slot 3, RESIGN).
    pub resigned: bool,
    /// `+0x8c`: its resignation was a drop from the game.
    pub dropped: bool,
    /// `allied_victory` (`+0x2d0`, saved): its allies' victory counts as its own.
    pub allied_victory: bool,
    /// Its victory conditions' state (`player+0x68`'s `+0x20`, the save's `victory`): 2 when
    /// they are met (`FUN_1414f35d0`).
    pub victory: u8,
    /// `fealty` (`+0x12f8`, saved): the player a defeated player's resources and units would go
    /// to.
    pub fealty: i16,
    /// `+0x71c`, the offensive posture (0 or 1, GAME 0x14): not saved, a load sets 0; new soldiers
    /// read it (their unit AI's constructor, not modelled).
    pub posture: i32,
    /// `allied_LOS_Enable` (`+0x2cc`, saved): the player shares its mutual allies' sight
    /// ([`crate::vision`]).
    pub allied_vision: bool,
    /// The players whose town centres the player's reveal (attribute 209) flared
    /// (`mTCFlared[i]`, saved; [`crate::vision`]).
    pub tc_flared: Vec<bool>,
    /// `mSharingIsCaring` (`+0x1426`, `FUN_141626780`, saved): the player's gathered resources go
    /// to its allies too.
    pub sharing: bool,
    /// The objects whose ghosts the player's list follows (`+0x78`, [`crate::ghost`]): not
    /// saved; the load builds it ([`Turns::build_watched`]).
    pub watched: Vec<crate::ghost::Watched>,
    /// The player's ghosts (its doppelganger list `+0x40`), in the list's order.
    pub ghosts: Vec<crate::ghost::Ghost>,
    /// Its history (`+0xdb8`, saved, [`crate::history`]).
    pub history: crate::history::History,
    /// Its object tracker (`+0x18`, [`crate::tracker`]): not saved; the load files the objects
    /// ([`Turns::load_trackers`]).
    pub tracker: crate::tracker::Tracker,
    /// `mUpdateTime` (`+0x7a4`, saved): the seconds the world player's update has not yet run
    /// its per-second loop for ([`crate::world_player`]); Gaia's stays 0.
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
    /// The victory timers that keep a defeated player in the game ([`crate::world_player`]).
    pub victory_times: crate::world_player::VictoryTimes,
}

impl crate::tech::Attributes for Player {
    /// An attribute's value (the resources' from [`Player::resources`]).
    fn get(&self, i: i16) -> Result<f32, Unsupported> {
        match usize::try_from(i) {
            Ok(r @ 0..=3) => Ok(self.resources[r]),
            Ok(k) => self
                .attributes
                .get(k)
                .copied()
                .ok_or(Unsupported("an attribute the player does not have")),
            Err(_) => Err(Unsupported("a negative attribute")),
        }
    }

    /// An attribute set to `v`, as a technology's payment writes it (no side effects).
    fn set(&mut self, i: i16, v: f32) -> Result<(), Unsupported> {
        match usize::try_from(i) {
            Ok(r @ 0..=3) => self.resources[r] = v,
            Ok(k) => {
                *self
                    .attributes
                    .get_mut(k)
                    .ok_or(Unsupported("an attribute the player does not have"))? = v;
            }
            Err(_) => return Err(Unsupported("a negative attribute")),
        }
        Ok(())
    }
}

impl Turns {
    /// A command or a pass the slice does not model for player `p`: the player stops
    /// ([`Player::stop`]), and a subscriber hears why ([`crate::trace`]).
    pub(crate) fn stop_player(&mut self, p: usize, why: Unsupported) {
        crate::trace::player_stops(self.ticks, p, why);
        self.players[p].stop(self.ticks, why);
    }
}

impl Player {
    /// A command the slice does not model for this player: the player stops.
    pub(crate) fn stop(&mut self, tick: u32, why: Unsupported) {
        let _ = self.unsupported.get_or_insert((tick, 0, why));
        self.stopped = true;
    }

    /// `FUN_141616820` for the attributes an object's storage holds: the attribute's value grows
    /// by `amount` (`FUN_14165d040`; the peak of attribute 11, `+0x6fc`, and the resource events of
    /// `FUN_140c898a0` are not held). Food, wood, stone and gold are the slice's `resources`; the
    /// attributes whose change has side effects (50, 183, 196) are not modelled.
    pub fn add_attribute(&mut self, kind: i16, amount: f32) -> Result<(), Unsupported> {
        let i = usize::try_from(kind).map_err(|_| Unsupported("a negative attribute"))?;
        match i {
            50 | 183 | 196 => Err(Unsupported("an attribute whose change has side effects")),
            0..=3 => {
                self.resources[i] += amount;
                Ok(())
            }
            _ => {
                let v = self
                    .attributes
                    .get_mut(i)
                    .ok_or(Unsupported("an attribute the player does not have"))?;
                *v += amount;
                Ok(())
            }
        }
    }
}
