//! The master ids (`i16`, [`crate::objects::masters::Master::id`]): the one definition of the
//! masters the engine names, by the `.dat`'s unit id. The obfuscated build stores an id as its
//! value plus `0x609c`; the values here are the decoded ones. A master the engine tests by id
//! without a known name is `ID_n` or `MASTER_n`.

/// The master whose footprint a reach test gives more range (raw `0x60c9`, `FUN_140fa0370`).
pub const ID_45: i16 = 45;
/// The farm (`0x32`): a mill's queue buys it, and BUILD places it as the rice farm on mangrove or
/// rice farm terrain.
pub const FARM: i16 = 50;
/// The house (`0x46`), whose update takes the walls' branch.
pub const HOUSE: i16 = 70;
/// The castle (`0x52`), whose death lowers its owner's attribute 134 (`FUN_1411dbb70`).
pub const CASTLE: i16 = 82;
/// The male villager (`0x53`): the master QUEUE draws a sex for, and the one the generic
/// villager (`0x712`) becomes first (`FUN_1411cbc40`).
pub const VILLAGER_MALE: i16 = 83;
/// The market (raw `0x60f0`), whose own trade units the work command never retakes and whose
/// sight the AI players' market lists follow.
pub const MARKET: i16 = 84;
/// The monastery (raw `0x6104`), whose relics give goods every second and whose death lowers its
/// owner's attribute 52 (`FUN_1411dbb70`).
pub const MONASTERY: i16 = 104;
/// The town centre (raw `0x6109`).
pub const TOWN_CENTRE: i16 = 109;
/// The builder (`0x76`) the train locations name for every villager.
pub const BUILDER: i16 = 118;
/// The fish trap (raw `0x6163`), which only the fishing ships gather from.
pub const FISH_TRAP: i16 = 199;
/// The Wonder (`0x114`, raw `0x61b0`), whose death lowers its owner's attribute 42
/// (`FUN_1411dbb70`) and whose construction rate the player's attribute raises at most to
/// `DAT_142ed66b4`.
pub const WONDER: i16 = 276;
/// The relic (`0x11d`).
pub const RELIC: i16 = 285;
/// The female villager (`0x125`).
pub const VILLAGER_FEMALE: i16 = 293;
/// The king (`0x1b2`).
pub const KING: i16 = 434;
/// Master 448 (raw `0x625c`), which the task finder treats apart.
pub const ID_448: i16 = 448;
/// Master 546 (raw `0x62be`), which the task finder treats apart.
pub const ID_546: i16 = 546;
/// A town centre (`0x26d`): a delete also tells the world's `+0x820`, a builder may place it and
/// its availability counts the town centres standing.
pub const TOWN_CENTRE_621: i16 = 621;
/// Master 1105 (raw `0x64ed`), which the task finder treats apart.
pub const ID_1105: i16 = 1105;
/// Master 1808 (`0x710`, raw `0x67ac`), which answers the town bell besides the villagers and
/// which a building's watch leaves out.
pub const MASTER_1808: i16 = 1808;
/// The pasture master (`0x761`) whose foundation keeps its snow and whose stack unit depends on
/// the rice terrain under it.
pub const PASTURE_1889: i16 = 1889;
