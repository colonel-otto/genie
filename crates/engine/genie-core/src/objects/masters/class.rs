//! The master classes (`+0x18`, an `i16`): the one definition of the class values. A master's
//! class groups the masters the game treats alike (villagers, siege, gates, farms); the obfuscated
//! build stores a class as its value minus `0xeac`, the values here are the decoded ones. Import
//! the module and name a class as `class::VILLAGER`: several names also stand for a master type
//! ([`crate::objects::masters::master_type`]).

/// Artifacts (relics' neighbours that no blast reaches).
pub const ARTIFACT: i16 = 1;
/// The trade boat (`-0xeaa` raw): its cared classes and its trade partner are not the cart's.
pub const TRADE_BOAT: i16 = 2;
/// Buildings, which a unit's garrison task names.
pub const BUILDING: i16 = 3;
/// Villagers (`-0xea8` raw).
pub const VILLAGER: i16 = 4;
/// The prey animals (`-0xea3` raw).
pub const PREY: i16 = 9;
/// The predators (`-0xea2` raw), whose unit AI's slot 0x1f0 a failed Attack calls.
pub const PREDATOR: i16 = 10;
/// Siege weapons (`-0xe9f` raw, the short `0xd`).
pub const SIEGE: i16 = 13;
/// Trees.
pub const TREE: i16 = 15;
/// Monks (`-0xe9a` raw), whose stop leaves the state position.
pub const MONK: i16 = 18;
/// The trade cart (decoded `0x13`).
pub const TRADE_CART: i16 = 19;
/// The class whose busy units skip the idle check (`-0xe98` raw).
pub const TRANSPORT: i16 = 20;
/// The fishing ship (decoded 21, `0x15`).
pub const FISHING_SHIP: i16 = 21;
/// The warships (`-0xe96` raw), whose attackers make a computer's unit warn its group.
pub const WARSHIP: i16 = 22;
/// Walls (decoded 27, `0x1b`).
pub const WALL: i16 = 27;
/// Gates (decoded 39), whose annexes stay in their owner's watched list.
pub const GATE: i16 = 39;
/// Relics (`.dat` class 42, `0x2a`, `-0xe82` raw): a delete spares them.
pub const RELIC: i16 = 42;
/// Monks carrying a relic (`.dat` class 43, `-0xe81` raw), which the task finder treats apart.
pub const CLASS_43: i16 = 43;
/// Farms and a pasture's livestock (decoded 49, `0x31`).
pub const FARM: i16 = 49;
/// Packed siege units (`.dat` class 51, `-0xe79` raw: the packed trebuchet and mangonel, master type
/// 80): they block a placement though desolid, and their ghosts stay watched.
pub const CLASS_51: i16 = 51;
/// Towers.
pub const TOWER: i16 = 52;
/// Boarding boats (`.dat` class 53, `-0xe77` raw), which the task finder treats apart.
pub const CLASS_53: i16 = 53;
/// Unpacked siege units (`.dat` class 54, `-0xe76` raw: the trebuchet, the bombard cannon): they
/// block a placement though desolid, and their ghosts stay watched.
pub const CLASS_54: i16 = 54;
/// Scorpions and the like (`.dat` class 55, `-0xe75` raw), which the task finder treats apart.
pub const CLASS_55: i16 = 55;
/// Livestock (decoded 58, `0x3a`, `-0xe72` raw), which goes to Gaia when killed.
pub const LIVESTOCK: i16 = 58;
/// The king's class (`-0xe71` raw).
pub const KING: i16 = 59;
/// Class 12 (`-0xea0` raw): a group's command on an attack task of a target of it lists the
/// group's special units (`FUN_14100c6c0`).
pub const CLASS_12: i16 = 12;
/// Class 24 (`-0xe94` raw), listed as [`CLASS_12`] is.
pub const CLASS_24: i16 = 24;
/// Class 35 (`-0xe89` raw): a target the group's command lists the units by formation category
/// for (`FUN_14100c6c0`).
pub const CLASS_35: i16 = 35;
/// Class 46 (`-0xe7e` raw): the units the group's special units list takes first
/// (`FUN_14100c6c0`).
pub const CLASS_46: i16 = 46;
/// Class 47 (`-0xe7d` raw), listed as [`CLASS_12`] is.
pub const CLASS_47: i16 = 47;
/// Class 57 (`-0xe73` raw), listed as [`CLASS_12`] is.
pub const CLASS_57: i16 = 57;
/// Class 61 (`-0xe6f` raw), listed as [`CLASS_35`] is.
pub const CLASS_61: i16 = 61;
