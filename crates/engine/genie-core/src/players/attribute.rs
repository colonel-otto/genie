//! The indices of the player's attributes (`player+0x8`'s array of `f32`): the one definition of
//! the attributes the engine reads and writes by number. The first four are the resources
//! (food, wood, stone, gold). A master's attribute types are a different set
//! ([`crate::objects::masters::attribute`]).

/// Food: the first of the four resources (food, wood, stone, gold: 0 to 3).
pub const FOOD: usize = 0;
/// Gold: a sale gives it and a purchase costs it, the treason costs it.
pub const GOLD: usize = 3;
/// The current age (`+0x30`, `player+0x100`), as the `i16` game mode 16's QUEUE, a new soldier's
/// unit AI (`FUN_140c12300`) and a command compare; the mode's bound is `DAT_14330eb8c`.
pub const CURRENT_AGE: i16 = 6;
/// The population.
pub const POPULATION: usize = 11;
/// The units owned.
pub const UNITS_OWNED: usize = 19;
/// The kills.
pub const KILLS: usize = 20;
/// The explored share of the map, in percent.
pub const EXPLORED: usize = 22;
/// The civilian population.
pub const CIVILIAN_POPULATION: usize = 37;
/// The military population.
pub const MILITARY_POPULATION: usize = 40;
/// The razings.
pub const RAZINGS: usize = 43;
/// The kill ratio: kills less losses.
pub const KILL_RATIO: usize = 44;
/// The survival to the finish, which a defeat clears.
pub const SURVIVAL: usize = 45;
/// The town centre is not available (0 lets a builder place one, `FUN_141625a20`).
pub const NO_TOWN_CENTRE: usize = 48;
/// The gold counter, which gold with a statistic grows (`FUN_141616820`).
pub const GOLD_COUNTER: usize = 49;
/// The research cost modifier, as the `i16` the cost code compares.
pub const RESEARCH_COST_MODIFIER: i16 = 85;
/// The kills' value of the other players.
pub const KILLS_VALUE_OTHER: usize = 152;
/// The razings' value of the other players.
pub const RAZINGS_VALUE_OTHER: usize = 153;
/// The kills of the other players.
pub const KILLS_OTHER: usize = 154;
/// The razings of the other players.
pub const RAZINGS_OTHER: usize = 155;
/// The units' value.
pub const UNITS_VALUE: usize = 164;
/// The buildings' value.
pub const BUILDINGS_VALUE: usize = 165;
/// The first of the gathered totals of food, wood and stone (166 to 168).
pub const GATHERED: usize = 166;
/// The gathered total of gold.
pub const GOLD_GATHERED: usize = 169;
/// The kills' value.
pub const KILLS_VALUE: usize = 170;
/// The razings' value.
pub const RAZINGS_VALUE: usize = 172;
/// The town centres a player may have (`FUN_141625a20`).
pub const TOWN_CENTRE_LIMIT: usize = 218;
/// The first of the trickles a minute of food, wood, stone and gold (230 to 233).
pub const TRICKLES: usize = 230;
/// The food a minute of the livestock in the player's mills.
pub const MILL_FOOD: usize = 254;
