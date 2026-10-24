//! The damage side of a hit: what it reads of its target and its attacker and what it changes.
//! The target's slot 0xe8 takes the damage: a static object's (`FUN_14118efd0`), a combat
//! object's of type 50 or 60 (`FUN_141082e80`), of type 70 (`FUN_1412108e0`, which adds the
//! special attack, the retaliation, the least damage and the reflected damage) and a building's
//! (`FUN_1411cea90`: an annex's hit goes to its main building, its least hit, its razing's
//! counts and history event). The damage is the
//! armor's (slot 0xf0, `FUN_141082530`), less what a shield blocks (`FUN_14108d8a0`); a charge
//! (slots 0x2f0, 0x300, 0x318) adds to it, and a blast (slot 0x848, `FUN_14107f690`) spreads it.
//! When a melee hit lands is [`crate::attack`]'s.
//!
//! Of these, MarvinBlue/426048731 runs only the armor and the damage of a villager's hits on
//! sheep (update 1170). The charges, shields, blasts, terrain bonuses, special attacks,
//! retaliations and reflections are ports of the code, checked by the unit tests alone.

use crate::GenieError;
use crate::checkpoint::cvttss2si;
use crate::fmath;

/// Elevation: a higher attacker hits at 1.25 (`DAT_142eb56a8`), a lower one at 0.75
/// (`DAT_142e7f388`), each plus its player's and the target's player's attributes (211/272,
/// 212/273).
const HIGHER: f32 = crate::image::damage::HIGHER;
const LOWER: f32 = crate::image::damage::LOWER;
/// A building's hits with a specific relic inside (`DAT_142ed66b4`, 1.2), and with any relic
/// (`DAT_142e86c14`, 1.1), [`building_elevation_factor`].
const SPECIFIC_RELIC_BOOST: f32 = crate::image::damage::SPECIFIC_RELIC_BOOST;
const RELIC_BOOST: f32 = crate::image::damage::RELIC_BOOST;
/// The gap below which the game takes a distance as zero (`DAT_142eb0c60`), and the share of
/// reflected damage above which a target reflects.
const EPSILON: f32 = crate::image::damage::EPSILON;
/// A special attack and a retaliation need their attacker within 1.2 facets of where the target
/// faces (`DAT_142ed66b4`, `DAT_142eb99e4`); the special attack takes the hit's damage to 0.66
/// (`DAT_142f3855c`); a retaliation answers attackers of a weapon range below 0.2
/// (`DAT_142eae8d0`), four times over an attacker whose charge is ready (`DAT_142eb0f48`).
const FACING: f32 = crate::image::damage::FACING;
const MINUS_FACING: f32 = crate::image::damage::MINUS_FACING;
const SPECIAL: f32 = crate::image::damage::SPECIAL;
const RETALIATION_RANGE: f32 = crate::image::damage::RETALIATION_RANGE;
const RETALIATION_SCALE: f32 = crate::image::damage::RETALIATION_SCALE;
/// A melee attacker (weapon range below 0.99, `DAT_142eb569c`) with an area effect tramples:
/// its blast goes about itself (`FUN_1412115e0`).
pub const BLAST_RANGE: f32 = crate::image::damage::BLAST_RANGE;
/// A cone blast (its level's bit 7) reaches objects within half a facet of where its attacker
/// faces (`DAT_142e7f384`, `DAT_142eb6e58`), or within 1.67 facets (`DAT_142f4fea8`) for a wide
/// one (bit 5).
const CONE: f32 = crate::image::damage::CONE;
const MINUS_CONE: f32 = crate::image::damage::MINUS_CONE;
const WIDE_CONE: f32 = crate::image::damage::WIDE_CONE;

/// The villager class (4): its hits leave what a static target holds.
const VILLAGER: i16 = 4;
/// The class whose damage alerts its target's owner itself (`-0xe6c` raw): its target's unit AI
/// gets no notify 500.
const ELEPHANT_ARCHER: i16 = 64;
/// The livestock class, which goes to Gaia when killed.
const LIVESTOCK: i16 = 58;
/// The artifact class (1), which no blast reaches.
const ARTIFACT: i16 = 1;
/// The rams (masters 35, 422, 548 and 1258): their blast reaches only buildings and their
/// target.
const RAMS: [i16; 4] = [35, 422, 548, 1258];
/// The building type (`'P'`): its slot 0xe8 is `FUN_1411cea90` ([`take_building`]).
const BUILDING: u8 = 80;
/// The wall class (27): its razing counts nothing.
const WALL: i16 = 27;
/// 1 (`DAT_14330eb8c`, read into `XMM9` at `0x1411cec8e`): a building's least hit, the hit
/// points below which a hit razes a building or kills a unit, and what each razing count adds.
const ONE: f32 = crate::image::damage::ONE;
/// The least hit of the attacker classes [`LEAST_HIT_CLASSES`] under the game's setting `+0x680`
/// at 1 (`DAT_142eb5690`, read into `XMM6` at `0x1411ceea4`).
const LEAST_CLASS_HIT: f32 = crate::image::damage::LEAST_CLASS_HIT;
/// The attacker classes whose least hit on a building is [`LEAST_CLASS_HIT`] (decoded `0`, `0x24`,
/// `0x16`, `0x2c`, `0x34` at `0x1411ced23`..`0x1411cee9e`).
const LEAST_HIT_CLASSES: [i16; 5] = [0, 36, 22, 44, 52];

/// An object as a hit reads and changes it: the target, and the attacker as the target of a
/// retaliation or of reflected damage.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Victim {
    /// The object.
    pub id: i32,
    /// Its place in its tile's object list (the map tile's `+0x28`, in the order
    /// `FUN_140f5f0a0` and `FUN_140f5f560` keep), which a blast walks a tile by ([`blast`]).
    pub tile_place: usize,
    /// Its hit points (`+0x278`), what it holds (`+0x310`), its state and place.
    pub hp: f32,
    /// What it holds (`+0x310`).
    pub amount: f32,
    /// Its state (`+0x640`).
    pub state: u8,
    /// Its place.
    pub position: [f32; 3],
    /// Its master's type, class, id, radii, first storage amount (`+0x610`, which a gatherer
    /// reads), `attribute_max_amount` and `area_effect_object_level` (`+0x62b`, the blast
    /// levels that reach it).
    pub kind: u8,
    /// Its master's class.
    pub class: i16,
    /// Its master.
    pub master_id: i16,
    /// Its master's radii.
    pub radius: [f32; 2],
    /// Its master's `radius_z` (`+0x5e0`): a missile meets it within that height above it
    /// (`FUN_14113f740`).
    pub height: f32,
    /// Its master's first storage amount (`+0x610`), which a gatherer reads.
    pub storage: f32,
    /// Its master's `attribute_max_amount`.
    pub capacity: i16,
    /// `+0x644`, `worker_num`: the actions that have it as their target (a farm another farmer
    /// works).
    pub workers: i32,
    /// Its master's `area_effect_object_level` (`+0x62b`): the blast levels that reach it.
    pub blast_defense: u8,
    /// Flagged to sleep (`+0x641`, `+0x643`): a killing hit wakes it (`FUN_14119deb0` with 0).
    pub goto_sleep: bool,
    /// Its master's decay (`+0x620`), which a hunt's decay stop negates (`FUN_140ac44b0`; the
    /// write-back gives it to the object's own master).
    pub decay: f32,
    /// Its master's `fog_visibility` (`+0x605`): an Attack goes on against it in the fog.
    pub fog_visibility: u8,
    /// Its player, and that player's attributes 272 and 273 (a higher or lower attacker's hits
    /// add them) and 211 and 212 (its own hits from higher or lower up add them).
    pub player: usize,
    /// Its player's attributes 272 and 273: what a higher or lower attacker's hits add.
    pub elevation: [f32; 2],
    /// Its player's attributes 211 and 212: what its own hits from higher or lower up add.
    pub attack_elevation: [f32; 2],
    /// The attacker's player counts its player as its own or allied (`FUN_141663d90` with the
    /// ally flag), and the other way round: a blast spares the first and scales its damage on
    /// the second by `friendly_fire_damage`.
    pub allied: [bool; 2],
    /// Its place is a guess: its player's update is not modelled.
    pub uncertain: bool,
    /// It is inside another object (`+0x2f0`): an Attack on it ends.
    pub inside: bool,
    /// Its master's hit points (`+0x530`).
    pub master_hp: i16,
    /// Its master's tasks of action type 0xa0 ([`BONUS_DAMAGE`]), in their order: the damage its
    /// hits add by hit points ([`bonus_damage`]).
    pub bonus_tasks: Vec<crate::masters::Task>,
    /// Its own attack (a retaliation's): its values as a hit's source and its master's weapons
    /// (class, amount).
    pub source: Attacker,
    /// Its master's weapons (class, amount).
    pub weapons: Vec<(i16, i16)>,
    /// A combat object's armor and what its damage reads (types 50 and up); None for others.
    pub armored: Option<Armored>,
    /// The damage notifies (500, `FUN_141082e80`) its hits post to its unit AI, in order (a
    /// blast's hit on an annex and on its main building post two to the main building): the
    /// attacker, its hit points after (truncated) and its master's hit points.
    pub damaged: Vec<(i32, [i32; 3])>,
    /// A hit killed it, for the caller's death handling.
    pub killed: Option<Kill>,
    /// A building's (type 80) part of a hit ([`Building`]); None for other objects.
    pub building: Option<Building>,
}

/// What a hit on a building (`TRIBE_Building_Object`, slot 0xe8 `FUN_1411cea90`) reads of it
/// besides a combat object's values, and what it changes for the write-back.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Building {
    /// `+0xbb8`: an annex's main building, whose slot 0xe8 takes a hit on the annex.
    pub main: Option<i32>,
    /// `+0xbc0`..`+0xbd8`: its annexes, which take its hit points after each hit.
    pub annexes: Vec<i32>,
    /// `+0xb90` (not saved): the damage the blast under way has done to it ([`blast`]).
    pub blast_taken: f32,
    /// `FUN_140fde380` on its master: of type 70 and up, of class 51 or 54, with a weapon
    /// (`+0x810` above 0): a trebuchet, whose kill counts as a unit's.
    pub trebuchet: bool,
    /// Its master's paid cost (`+0x880`: each entry with a resource and the paid flag, summed as
    /// floats in their order).
    pub cost: f32,
    /// `damaged_lately_timer` (`+0x30d`, saved): 60 after each hit, a second off in its update.
    pub damaged_lately: u8,
    /// A hit got past its hit point test: the write-back gives its annexes its hit points and
    /// its `damaged_lately`.
    pub struck: bool,
    /// What each hit that left it below 1 hit point counted, in order, for the write-back.
    pub razed: Vec<Razed>,
}

/// What a hit that leaves a player's building below 1 hit point counts (`FUN_1411cea90`), for
/// the write-back: the history event of the attacker's player (`FUN_141560ae0`, `data1` the
/// owner's id) and the players' attribute adds (player slot 0xf8) in the game's order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Razed {
    /// The attacker's player (`param_5`), whose history takes the event.
    pub attacker_player: usize,
    /// The building's owner (`+0x718`), the event's `data1`.
    pub owner: usize,
    /// A castle's (master 82) or a wonder's (276) destruction
    /// ([`crate::history::CASTLE_DESTROYED`], [`crate::history::WONDER_DESTROYED`]).
    pub event: Option<u8>,
    /// The attribute adds: (player, attribute, amount).
    pub adds: Vec<(usize, i16, f32)>,
}

/// What a killing hit leaves for the caller ([`crate::turn::Turns`]'s write-back).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kill {
    /// The object that killed it.
    pub attacker: i32,
    /// That object's player.
    pub attacker_player: usize,
    /// Gaia takes it: a player's livestock (`FUN_1412108e0`).
    pub to_gaia: bool,
}

/// Where an object faces (`FUN_14114eaa0`'s inputs): its angle (`+0x7e8`, which follows its
/// facet, `FUN_14114d780`) and its sprite's facets (`+0x140` -> `+0xa`, 0 without a sprite).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Facing {
    /// `+0x7e8`, where it faces.
    pub angle: f32,
    /// Its sprite's facets, 0 without a sprite.
    pub facets: i16,
}

/// What a hit on a combat object reads of it (its master's and its own values) and changes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Armored {
    /// Its master's armor classes (class, amount), base armor, `defense_terrain_bonus`,
    /// `bonus_damage_resistance`, `combat_ability`, `damage_reflection`,
    /// `invulnerability_level`, `special_attack_flag`, hit points and `friendly_fire_damage`.
    pub armors: Vec<(i16, i16)>,
    /// Its master's base armor for a class it does not list.
    pub base_armor: i16,
    /// Its master's `defense_terrain_bonus`.
    pub terrain_bonus: i16,
    /// Its master's `bonus_damage_resistance`.
    pub resistance: f32,
    /// Its master's `combat_ability`.
    pub ability: u8,
    /// Its master's `damage_reflection`.
    pub reflection: f32,
    /// Its master's `invulnerability_level`.
    pub invulnerability: f32,
    /// Its master's `special_attack_flag`.
    pub special_attack: u8,
    /// Its master's `friendly_fire_damage`.
    pub friendly_fire: f32,
    /// Its master has a special graphic (`+0x8e8`), which its special attack shows.
    pub special_graphic: bool,
    /// Its terrain defense: `defense_terrain_bonus`'s restriction's value for its tile's terrain
    /// (the terrain to restore, else the terrain; 0 counts as 1), 1 off the map; None when the
    /// slice lacks the map or the restriction.
    pub terrain: Option<f32>,
    /// The object's armor powerups and armor removed (melee, pierce) and its hit point powerups
    /// (`+0xa38`, `+0xa3c`, which absorb damage first).
    pub armor_powerups: [i16; 2],
    /// The melee and pierce armor taken off it.
    pub armor_removed: [i16; 2],
    /// Its hit point powerups (`+0xa38`, `+0xa3c`), which absorb damage first.
    pub hp_powerups: [f32; 2],
    /// Its master has a task of action type 0xa1 with a work value (a kill reward,
    /// `FUN_14120b5f0`).
    pub kill_tasks: bool,
    /// It has a current action (`FUN_140a98730` on its action list), a unit AI, and
    /// `FUN_140c42b30` on that AI (alert mode 1, `+0x1b8`; no attacking units, `+0xc8`; no
    /// current action): a special attack and a retaliation need it idle. The slice does not hold
    /// the alert mode and the attacking units: None.
    pub busy: bool,
    /// It has a unit AI.
    pub unit_ai: bool,
    /// Its unit AI is idle in alert mode, when the slice knows it (None: it does not).
    pub alert_idle: Option<bool>,
    /// Where it faces; None: the slice does not hold the facet.
    pub facing: Option<Facing>,
}

/// What a hit reads of its attacker's attack, and the charge it changes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Attacker {
    /// The attacking object.
    pub id: i32,
    /// Its master's type, class, id, `combat_ability` and `missile_id`.
    pub kind: u8,
    /// Its master's class.
    pub class: i16,
    /// Its master.
    pub master_id: i16,
    /// Its master's `combat_ability`.
    pub ability: u8,
    /// Its master's missile, −1 for none.
    pub missile: i16,
    /// Its master's weapon range (`+0x828`), `area_effect_range` (`+0x82c`: the blast's radius),
    /// `area_effect_level` (`+0x830`: the levels it reaches, bit 2 its friends too, bit 5 a wide
    /// cone, bit 6 a falloff, bit 7 a cone) and `area_damage` (`+0x834`: the share of the damage
    /// the objects besides its target take, a fixed damage below 0).
    pub weapon_range: f32,
    /// Its master's `area_effect_range` (`+0x82c`): the blast's radius.
    pub area_effect: f32,
    /// Its master's `area_effect_level` (`+0x830`).
    pub blast_level: u8,
    /// Its master's `area_damage` (`+0x834`).
    pub area_damage: f32,
    /// Its master's `charge_type`, `max_charge`, `charge_target` and `special_attack_flag`; its
    /// charge (`+0x9e0`) and `mHitSomethingValidForChargeAttack` (`+0x8a8`), which its hits set.
    pub charge_type: i16,
    /// Its master's `max_charge`.
    pub max_charge: f32,
    /// Its master's `charge_target`.
    pub charge_target: i16,
    /// Its master's `special_attack_flag`.
    pub special_attack: u8,
    /// Its charge (`+0x9e0`).
    pub charge: f32,
    /// `mHitSomethingValidForChargeAttack` (`+0x8a8`), which its hits set.
    pub charge_hit: bool,
    /// Its attack powerup (`+0xa24`) and the game's setting `+0x680`
    /// ([`crate::turn::Turns::mode_680`]: at 1 a hit may do no damage).
    pub attack_powerup: f32,
    /// The game's setting `+0x680` ([`crate::turn::Turns::mode_680`]).
    pub mode_680: i32,
    /// Its player and whether its master has a task of action type 0x9a (a kill bonus,
    /// `FUN_1415c6510`).
    pub player: usize,
    /// Its master has a task of action type 0x9a (a kill bonus).
    pub kill_bonus_tasks: bool,
    /// What a missile was fired from (`+0x998`), its damage notify's sender; a missile without
    /// one is −1.
    pub fired_from: i32,
}

/// Slot 0x1f0 of a combat object (`FUN_14120d780`) at height `z` whose player has `up_down` as
/// attributes 211 and 212: a hit from higher up gains 25% and attributes 211 and the target's
/// 272, one from lower down 75% and attributes 212 and 273.
#[must_use]
pub fn elevation_factor(z: f32, up_down: [f32; 2], to: &Victim) -> f32 {
    let tz = to.position[2];
    if tz < z {
        up_down[0] + HIGHER + to.elevation[0]
    } else if z < tz {
        up_down[1] + LOWER + to.elevation[1]
    } else {
        1.0
    }
}

/// Slot 0x1f0 of a building (`FUN_1411cc890`): the combat object's `factor`
/// ([`elevation_factor`]), times [`SPECIFIC_RELIC_BOOST`] while its `specific_relic_count`
/// (`+0xc24`, `relics[1]`) is above 0, else [`RELIC_BOOST`] while its `relic_count` (`+0xc20`,
/// `relics[0]`) is; a building victim (type 80, `kind`) divides it by the same of its own counts
/// (`victim_relics`). Each count compares signed.
#[must_use]
pub fn building_elevation_factor(
    factor: f32,
    relics: [i32; 2],
    kind: u8,
    victim_relics: [i32; 2],
) -> f32 {
    let mut f = factor;
    if 0 < relics[1] {
        f *= SPECIFIC_RELIC_BOOST;
    } else if 0 < relics[0] {
        f *= RELIC_BOOST;
    }
    if kind == 80 {
        if 0 < victim_relics[1] {
            return f / SPECIFIC_RELIC_BOOST;
        }
        if 0 < victim_relics[0] {
            f /= RELIC_BOOST;
        }
    }
    f
}

/// `FUN_14114eaa0` toward another object: how far `other` lies from where an object at `at`
/// faces, in facets (at least 8): the angle to it (`atan2f`, 0 to 2π) less the facing angle,
/// wrapped into −π to π, times the facets over 2π; 0 on the same point or without facets.
#[must_use]
pub fn facing_offset(at: [f32; 3], facing: Facing, other: [f32; 3]) -> f32 {
    if facing.facets <= 0 {
        return 0.0;
    }
    let n = if 7 < facing.facets { facing.facets } else { 8 };
    crate::facing::offset_in_facets(
        other[0] - at[0],
        other[1] - at[1],
        facing.angle,
        i32::from(n),
    )
}

/// `FUN_1412623d0`: a charge (its master's `charge_target`, `+0x924`, one bit per group of
/// classes) works on an object of `class` and `kind`: on every object for −1 or charge type −4;
/// for 0, on every object but buildings (type 80) other than classes 51 and 54; else by the bit
/// of the class's group, none for a class in no group.
#[must_use]
pub const fn charge_works(charge_target: i16, charge_type: i16, class: i16, kind: u8) -> bool {
    let targets = charge_target as u16;
    if targets == 0xffff
        || charge_type == -4
        || (targets == 0 && !(80 <= kind && class != 51 && class != 54))
    {
        return true;
    }
    let bit = match class {
        6 | 28 | 45 | 46 | 50 | 56 => 0,
        12 | 24 | 47 | 57 => 1,
        0 | 44 => 2,
        23 | 36 => 3,
        17 | 18 | 43 => 4,
        4 | 19 | 59 => 5,
        2 | 20 | 21 | 22 | 53 => 6,
        13 | 35 | 51 | 54 | 55 => 7,
        3 | 27 | 39 | 49 | 52 | 60 => 8,
        _ => return false,
    };
    targets >> bit & 1 != 0
}

/// Slot 0x300 (`FUN_141203e30`): the charge is ready, at least `max_charge` for charge types 1
/// and 3, else at least 1.
#[must_use]
pub fn charged(a: &Attacker) -> bool {
    if matches!(a.charge_type, 1 | 3) {
        a.max_charge <= a.charge
    } else {
        1.0 <= a.charge
    }
}

/// Slot 0x2f0 of a combat object of type 70 or 80 (`FUN_14120e920`): a charge of type 1 or 3
/// that works on `v` ([`charge_works`]) and is ready.
#[must_use]
pub fn charge_hit(a: &Attacker, v: &Victim) -> bool {
    matches!(a.charge_type, 1 | 3)
        && charge_works(a.charge_target, a.charge_type, v.class, v.kind)
        && charged(a)
}

/// `FUN_1412161b0`: a master of special attack 3 whose charge is ready on its unit AI's (or
/// group's) target in reach; false for other masters. Special attack 3 is not modelled.
const fn special_charge_ready(a: &Attacker) -> Result<bool, GenieError> {
    if a.special_attack == 3 {
        return Err(GenieError::unsupported(
            "a charge of special attack 3 (FUN_1412161b0)",
        ));
    }
    Ok(false)
}

/// `FUN_141202580`: the damage a ready charge adds, the charge itself; a master of special
/// attack 3 adds it only while [`special_charge_ready`] holds.
fn charge_damage(a: &Attacker) -> Result<f32, GenieError> {
    if a.special_attack == 3 && !special_charge_ready(a)? {
        return Ok(0.0);
    }
    Ok(a.charge)
}

/// Slot 0x2f8 of a combat unit or building (`FUN_14120f510`): a charge of type 6 or 7 (slot
/// 0x308, `FUN_141203b50`) that works on `t` ([`charge_works`]) and is ready ([`charged`]).
#[must_use]
pub fn range_charge_hit(a: &Attacker, t: &Victim) -> bool {
    matches!(a.charge_type, 6 | 7)
        && charge_works(a.charge_target, a.charge_type, t.class, t.kind)
        && charged(a)
}

/// The action type of a bonus damage task (`FUN_14107ce60`).
pub const BONUS_DAMAGE: i16 = 0xa0;

impl Victim {
    /// A combat unit or building (types 70 and 80, `TRIBE_Combat_Object` and
    /// `TRIBE_Building_Object`): its powerups and charges count; a combat object of type 50 or
    /// 60 (`RGE_Combat_Object`) answers 0 for them.
    const fn tribe(&self) -> bool {
        matches!(self.kind, 70 | 80)
    }

    /// Slot 0xc0, its hit points as a share reads them: a combat unit's or building's
    /// (`FUN_141202990`) most hit points powerup less its hit points powerup (`+0xa3c` less
    /// `+0xa38`), plus its hit points; any other object's its hit points (`FUN_141188830`).
    #[must_use]
    pub fn shown_hp(&self) -> f32 {
        self.armored
            .as_ref()
            .filter(|_| self.tribe())
            .map_or(self.hp, |a| (a.hp_powerups[1] - a.hp_powerups[0]) + self.hp)
    }

    /// Slot 0xc8, its most hit points: a combat unit's or building's (`FUN_141202bb0`) hit point
    /// stinger (`+0xb44`, slot 0x760) plus its master's hit points plus its most hit points
    /// powerup (`+0xa3c`); any other object's its master's hit points (`FUN_1411889a0`). The
    /// stingers (`+0xb38`..`+0xb50`) are 0: `TRIBE_Combat_Object::update` zeroes them and rebuilds
    /// them from the object's stinger list (`FUN_1412092e0`), which the slice holds empty.
    #[must_use]
    pub fn most_hp(&self) -> f32 {
        let hp = f32::from(self.master_hp);
        self.armored
            .as_ref()
            .filter(|_| self.tribe())
            .map_or(hp, |a| hp + a.hp_powerups[1])
    }

    /// `FUN_141188880`: the share of its most hit points it has (slot 0xc0, at most slot 0xc8 by
    /// `MINSS`, over slot 0xc8), with `lacking` the share it lacks (1 less that); 0 when its most
    /// hit points are not above 0 (`COMISS 0, most` with `JNC`: a NaN divides).
    #[must_use]
    pub fn hp_share(&self, lacking: bool) -> f32 {
        let most = self.most_hp();
        let least = fmath::minss(most, self.shown_hp());
        if !fmath::below_or_unordered(0.0, most) {
            return 0.0;
        }
        let share = least / most;
        if lacking { ONE - share } else { share }
    }
}

/// `FUN_14107ce60`: the damage `attacker`'s bonus damage tasks ([`Victim::bonus_tasks`]) add
/// against `target`. A task for the target's master (`object_id`) or class (`object_group`), or
/// for neither (both −1) on a target of class 51 or 54 or of a type below 80, adds its
/// `work_val1` for each whole `work_val2` of a hit point share ([`Victim::hp_share`], rounded
/// down after [`EPSILON`] is added): the target's with a `work_range` above 0, else the
/// attacker's (`COMISS` with `CMOVBE`: a NaN reads the attacker's), the share lacking with
/// `attribute_type4` not 0. A `work_val2` not above 0, or a target without hit points above 0
/// (slot 0xc0), adds nothing (`COMISS` with `JBE`). The sum is at least 0 (`MAXSS`).
#[must_use]
pub fn bonus_damage(attacker: &Victim, target: &Victim) -> f32 {
    let mut sum = 0.0f32;
    for t in attacker
        .bonus_tasks
        .iter()
        .filter(|t| t.action_type == BONUS_DAMAGE)
    {
        let any = t.object_id == -1
            && t.object_group == -1
            && (matches!(target.class, 51 | 54) || target.kind < BUILDING);
        if !(any || t.object_id == target.master_id || t.object_group == target.class) {
            continue;
        }
        let whose = if fmath::at_most_or_unordered(t.work_range, 0.0) {
            attacker
        } else {
            target
        };
        let share = whose.hp_share(t.attribute_type4 != 0);
        if fmath::at_most_or_unordered(t.work_val2, 0.0)
            || fmath::at_most_or_unordered(target.shown_hp(), 0.0)
        {
            continue;
        }
        sum += libm::floorf(share / t.work_val2 + EPSILON) * t.work_val1;
    }
    fmath::maxss(sum, 0.0)
}

/// What the attack and the range a combat object shows (slots 0x878 and 0x880) read besides the
/// object as a hit reads it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Shown<'a> {
    /// The target of its top action when that attacks (action type 9 or 7, `FUN_14107cc30`).
    pub target: Option<&'a Victim>,
    /// The weapons (class, amount) of its master's charge projectile (`+0x928`) when its player
    /// holds that master (`FUN_141659d20`).
    pub projectile: Option<&'a [(i16, i16)]>,
    /// Its master's `charge_event` (`+0x920`), which a ready charge of type 6 or 7 adds to its
    /// range.
    pub charge_event: i16,
    /// Its max range powerup (`+0xa40`, slot 0x700).
    pub range_powerup: f32,
}

/// Slot 0x320 of a combat unit or building (`FUN_141211440`): its charge is ready (slot 0x300,
/// [`charged`]) and, unless its master's `charge_target` is −1, works on `target` (slot 0x2f0,
/// [`charge_hit`], or slot 0x2f8, [`range_charge_hit`]); with a `charge_target` but no target,
/// not. A combat object of type 50 or 60 has no charge (`FUN_14106c580`).
fn charge_shown(v: &Victim, target: Option<&Victim>) -> bool {
    let a = &v.source;
    v.tribe()
        && charged(a)
        && (a.charge_target == -1
            || target.is_some_and(|t| charge_hit(a, t) || range_charge_hit(a, t)))
}

/// The best amount of `weapons` above 0 (the first of a tie), of a melee, pierce or class-31
/// weapon with `fighting`.
fn best_attack(weapons: &[(i16, i16)], fighting: bool) -> i16 {
    weapons
        .iter()
        .filter(|w| !fighting || matches!(w.0, 3 | 4 | 31))
        .fold(0, |m, w| if m < w.1 { w.1 } else { m })
}

/// A float as `(short)(int)` takes it: `CVTTSS2SI`, then its low 16 bits.
fn whole(f: f32) -> i16 {
    cvttss2si(f) as i16
}

/// Slot 0x878 of a combat object, the attack it shows (`FUN_141085c70`; a combat unit's or
/// building's `FUN_141213990`). With a ready charge of type 6 or 7 ([`charge_shown`]) and its
/// charge projectile's master among its player's, that master's best melee, pierce or class-31
/// attack (`FUN_1410d5740`). Else its best melee, pierce or class-31 attack, without one its best
/// attack (from 0), plus its attack powerup (`+0xa24`, slot 0x6d8) and stinger (`+0xb48`, slot
/// 0x768, 0 as [`Victim::most_hp`] says), each as a whole number ([`whole`]), plus the bonus
/// damage of its tasks against its attack's target ([`bonus_damage`]); a ready charge of type 1
/// or 3 adds the charge (`+0x9e0`). A combat object of type 50 or 60 has no powerups (its slots
/// answer 0). The sums wrap at 16 bits.
#[must_use]
pub fn shown_attack(v: &Victim, s: &Shown<'_>) -> i16 {
    let a = &v.source;
    let charge = charge_shown(v, s.target);
    if charge
        && matches!(a.charge_type, 6 | 7)
        && let Some(weapons) = s.projectile
    {
        return best_attack(weapons, true);
    }
    let mut attack = best_attack(&v.weapons, true);
    if attack == 0 {
        attack = best_attack(&v.weapons, false);
    }
    let powerup = if v.tribe() { a.attack_powerup } else { 0.0 };
    attack = attack.wrapping_add(whole(powerup)).wrapping_add(whole(0.0));
    if let Some(t) = s.target {
        attack = attack.wrapping_add(whole(bonus_damage(v, t)));
    }
    if charge && matches!(a.charge_type, 1 | 3) {
        attack = attack.wrapping_add(whole(a.charge));
    }
    attack
}

/// Slot 0x868 of a combat object, the melee armor it shows: a combat unit's or building's
/// [`melee_armor`], a type 50's or 60's [`master_melee_armor`] (`FUN_141085450`, its slots 0x708
/// and 0x770 answer 0).
///
/// # Errors
///
/// An object without combat values (a type below 50).
pub fn shown_melee_armor(v: &Victim) -> Result<i16, GenieError> {
    let a = v.armored.as_ref().ok_or(GenieError::unreachable(
        "the melee armor of an object without combat values",
    ))?;
    Ok(if v.tribe() {
        melee_armor(a, v.class, v.state)
    } else {
        master_melee_armor(a)
    })
}

/// Slot 0x880 of a combat object (`FUN_1410861a0`), the range it shows: its master's weapon
/// range (`+0x828`), to which a ready charge of type 6 or 7 ([`charge_shown`]) adds its master's
/// `charge_event`, plus its max range powerup (`+0xa40`, slot 0x700; 0 for a type 50 or 60).
#[must_use]
pub fn shown_range(v: &Victim, s: &Shown<'_>) -> f32 {
    let a = &v.source;
    let mut range = a.weapon_range;
    if matches!(a.charge_type, 6 | 7) && charge_shown(v, s.target) {
        range += f32::from(s.charge_event);
    }
    let powerup = if v.tribe() { s.range_powerup } else { 0.0 };
    powerup + range
}

/// `FUN_14108d8a0`: a shield blocks the hit: a target whose master's charge type is 5 against a
/// melee hit (4 against a missile) spends a charge of 1 when it has one (`COMISS 1.0, charge`,
/// `JA` past: a NaN charge blocks too). The type-5 shield's local-player statistics
/// (`FUN_140a497f0`) change no simulation state (FD-037-D20). A target without combat values has
/// charge type 0.
pub fn shield(v: &mut Victim, missile: bool) -> bool {
    let kind = if missile { 4 } else { 5 };
    if v.source.charge_type == kind && fmath::at_most_or_unordered(1.0, v.source.charge) {
        v.source.charge -= 1.0;
        return true;
    }
    false
}

/// `FUN_141202a80`: the hit points the master's `invulnerability_level` keeps (a share of its hit
/// points when above 0, the level itself when below), 0 for none.
fn hp_floor(a: &Armored, master_hp: i16) -> f32 {
    invulnerability_floor(a.invulnerability, master_hp)
}

/// [`hp_floor`] of a master's `invulnerability_level` and hit points.
#[must_use]
pub(crate) fn invulnerability_floor(level: f32, master_hp: i16) -> f32 {
    if level == 0.0 {
        0.0
    } else if 0.0 < level {
        f32::from(master_hp) * level
    } else {
        level.abs()
    }
}

/// Slot 0xc0: the hit points with the powerups' room.
fn effective(v: &Victim) -> f32 {
    let pu = v.armored.as_ref().map_or([0.0; 2], |a| a.hp_powerups);
    (pu[1] - pu[0]) + v.hp
}

/// Slot 0xb0, the loss of `d` hit points: a combat object of type 70 or 80 (`FUN_141205c80`)
/// takes it on its hit point powerups' room first, and its hit points stay above its
/// `invulnerability_level` floor; another object (`FUN_141189160`) just loses it. Not modelled
/// (no update of this recording fills either list): the kill credits of a dying object's active
/// stingers (`+0xb58`) and a surviving object's hit point transforms (`+0xb20`,
/// `FUN_141204450`).
pub fn lose(t: &mut Victim, d: f32) {
    let unit = matches!(t.kind, 70 | 80);
    let Some(a) = t.armored.as_mut().filter(|_| unit) else {
        t.hp -= d;
        return;
    };
    let mut d = d;
    let [up, room_max] = a.hp_powerups;
    if up < room_max {
        let room = room_max - up;
        if d <= room {
            a.hp_powerups[0] = up + d;
            return;
        }
        a.hp_powerups[0] = up + room;
        d -= room;
    }
    let floor = hp_floor(a, t.master_hp);
    let left = t.hp - d;
    t.hp = if floor == 0.0 || floor <= left {
        left
    } else {
        floor
    };
}

/// Slot 0xb8 of a combat object (`FUN_141203be0`) on hit points `hp` with the powerups
/// `hp_powerups` (`+0xa38`, its room `+0xa3c`) of a master of `master_hp`: `amount` added up to
/// its most (the master's plus the room), an overflow taken off the powerup (never below 0); a
/// powerup above its room first takes the room. The hit point transforms of `FUN_141204450`,
/// empty in the recordings, are not held.
pub fn gain_hp(hp: &mut f32, hp_powerups: &mut [f32; 2], master_hp: i16, amount: f32) {
    let [mut up, room] = *hp_powerups;
    let most = f32::from(master_hp) + room;
    if room < up {
        up = room;
    }
    if most <= *hp {
        if 0.0 < up {
            up = (up - amount).max(0.0);
        }
    } else {
        let healed = *hp + amount;
        *hp = healed;
        if most < healed {
            *hp = most;
            if 0.0 < up {
                up = (up - (healed - most)).max(0.0);
            }
        }
    }
    hp_powerups[0] = up;
}

/// `FUN_141193b30`: slot 0xb8 adds `amount` ([`gain_hp`]); a loss (`amount` below 0) leaves at
/// least 1 hit point; healed to its full hit points (slot 0xc0 at least slot 200, `COMISS` with
/// `JC`: a NaN is not), it holds exactly its master's. True then.
pub fn heal_hp(hp: &mut f32, hp_powerups: &mut [f32; 2], master_hp: i16, amount: f32) -> bool {
    gain_hp(hp, hp_powerups, master_hp, amount);
    if amount < 0.0 && *hp < 1.0 {
        *hp = 1.0;
    }
    let [up, room] = *hp_powerups;
    let most = f32::from(master_hp) + room;
    if fmath::below_or_unordered((room - up) + *hp, most) {
        return false;
    }
    *hp = f32::from(master_hp);
    true
}

/// `FUN_141085450`, a combat object's melee armor as its master holds it: its best armor of
/// class 4 above −32768, else its best of any, else its base armor.
fn master_melee_armor(a: &Armored) -> i16 {
    let best = |of: Option<i16>| {
        a.armors
            .iter()
            .filter(|&&(k, _)| of.is_none_or(|c| c == k))
            .fold(i16::MIN, |m, &(_, n)| if m < n { n } else { m })
    };
    let mut armor = best(Some(4));
    if armor == i16::MIN {
        armor = best(None);
        if armor == i16::MIN {
            armor = a.base_armor;
        }
    }
    armor
}

/// Slot 0x868 of a combat unit or building (`FUN_141213810`): [`master_melee_armor`] plus its
/// melee armor powerup (`+0xa48`, slot 0x708) and stinger (`+0xb4c`, slot 0x770, 0 as
/// [`Victim::most_hp`] says), each as 16 bits; a wall foundation (class 27 in state 0) has none.
fn melee_armor(a: &Armored, own_class: i16, state: u8) -> i16 {
    let armor = master_melee_armor(a).wrapping_add(a.armor_powerups[0]);
    if own_class == 27 && state == 0 {
        return 0;
    }
    armor
}

/// The armor a combat object (type 70 and up) has against melee (class 4; slot 0x868,
/// [`melee_armor`]) or pierce (class 3; slot 0x870, `FUN_1410857f0`: its best armor of the
/// class, else its base armor and best of any, plus its powerup; the stinger `+0xb50` is 0, as
/// [`Victim::most_hp`] says), less the armor removed.
fn class_armor(a: &Armored, class: i16, own_class: i16, state: u8) -> i16 {
    let best = |c: i16, from: i16| {
        a.armors
            .iter()
            .filter(|&&(k, _)| k == c)
            .fold(from, |m, &(_, n)| if m < n { n } else { m })
    };
    let any = |from: i16| {
        a.armors
            .iter()
            .fold(from, |m, &(_, n)| if m < n { n } else { m })
    };
    match class {
        4 => melee_armor(a, own_class, state).wrapping_sub(a.armor_removed[0]),
        3 => {
            let mut armor = best(3, -1);
            if armor == -1 {
                armor = any(a.base_armor);
            }
            armor
                .wrapping_add(a.armor_powerups[1])
                .wrapping_sub(a.armor_removed[1])
        }
        _ => unreachable!("only melee and pierce armor have their own slots"),
    }
}

/// Slot 0xf0 of a combat object (`FUN_141082530`): per weapon (class, amount) the armor of its
/// class (the last armor entry of the class wins: its amount, or for a type of 70 and up the
/// melee or pierce armor, [`class_armor`]; an attacker of type 60 and up whose combat ability
/// has bit 0 ignores melee and pierce armor unless the target's has bit 1), else the base armor.
/// For an attacker of type 70 and up: the first melee, pierce or class-31 weapon with an attack
/// adds a ready charge of type 1 or 3 that works on the target ([`charge_works`],
/// `FUN_141202580`), and the first such weapon adds the attack powerup (the stinger `+0xb48`,
/// unsaved and never written here, is 0); a weapon of class 11 adds ten per `+0xad8` (unsaved,
/// written only by `FUN_141209d60`, which no update of this recording runs). Another class is
/// scaled by 1 less the `bonus_damage_resistance` when it does damage. Each weapon's attack less
/// armor, times the elevation factor and the terrain defense, adds when above 0 (or below 0
/// against an armor below the base that rises above the attack: then at most 0). The damage is
/// the sum, at least 1 (0 under the game's setting `+0x680` at 1). `bonus` is the attacker's
/// bonus damage against `v` ([`bonus_damage`]), which the first such weapon adds with the
/// powerup.
pub fn armor_damage(
    v: &Victim,
    weapons: &[(i16, i16)],
    factor: f32,
    attacker: &Attacker,
    bonus: f32,
) -> Result<f32, GenieError> {
    let a = v.armored.as_ref().ok_or(GenieError::unsupported(
        "the armor of an object without combat values",
    ))?;
    let (own_class, kind, state) = (v.class, v.kind, v.state);
    let terrain = if a.terrain_bonus < 0 {
        1.0f32
    } else {
        a.terrain.ok_or(GenieError::unsupported(
            "a terrain defense bonus without the map's terrain or the restriction",
        ))?
    };
    let mut sum = 0.0f32;
    let mut charged = false;
    let mut bonused = false;
    for &(class, amount) in weapons {
        let mut armor = a.base_armor;
        for &(k, n) in &a.armors {
            if k != class {
                continue;
            }
            if 60 <= attacker.kind
                && matches!(k, 3 | 4)
                && attacker.ability & 1 != 0
                && a.ability & 2 == 0
            {
                armor = 0;
                continue;
            }
            armor = n;
            if 70 <= kind && matches!(k, 3 | 4) {
                armor = class_armor(a, k, own_class, state);
            }
        }
        let armor_f = f32::from(armor);
        let mut attack = f32::from(amount);
        let pierce_or_melee = matches!(class, 3 | 4 | 31);
        let mut resist = 1.0f32;
        if 70 <= attacker.kind {
            let charge = !charged
                && 0.0 < attack
                && charge_works(
                    attacker.charge_target,
                    attacker.charge_type,
                    own_class,
                    kind,
                )
                && matches!(attacker.charge_type, 1 | 3)
                && attacker.max_charge <= attacker.charge
                && pierce_or_melee;
            let c = if charge {
                charge_damage(attacker)?
            } else {
                0.0
            };
            if 0.0 < c {
                charged = true;
                attack += c + 0.0;
            } else {
                attack += 0.0;
            }
            if pierce_or_melee && !bonused {
                // the bonus, then the stinger plus the powerup
                attack += bonus + (0.0 + attacker.attack_powerup);
                bonused = true;
            }
        }
        if !pierce_or_melee && a.resistance != 0.0 && 0.0 < attack - armor_f {
            resist = 1.0 - a.resistance;
        }
        let mut armor_f = armor_f;
        let mut below = false;
        if attack < 0.0 && attack < armor_f && armor_f != f32::from(a.base_armor) {
            armor_f = if armor_f < 0.0 { armor_f } else { 0.0 };
            below = true;
        }
        let v = (attack - armor_f) * factor * terrain * resist;
        if 0.0 < v || below {
            sum += v;
        }
    }
    let least = if attacker.mode_680 == 1 { 0.0 } else { 1.0 };
    Ok(if weapons.is_empty() {
        sum
    } else if least > sum {
        least
    } else {
        sum
    })
}

/// The target's slot 0xe8 with the source's `weapons` at `factor`, less the damage a blast has
/// already done to it (`blast_taken`, a building's `+0xb90`): a static, animated, doppelganger,
/// moving or action object, or a tree (types below 50 and 90: `FUN_14118efd0`,
/// [`take_static`]), a combat object of type 50 or 60 ([`damage`]), of type 70
/// ([`take_unit`]) or a building ([`take_building`]). `others` are the objects besides the two
/// the hit may reach: an annex's main building, which takes a hit on the annex, and the annexes
/// that take a main building's hit points (every caller passes the flag that forwards an
/// annex's hit, 1).
///
/// # Errors
///
/// A part of the hit the slice does not model.
pub fn take(
    t: &mut Victim,
    src: &mut Victim,
    others: &mut [&mut Victim],
    weapons: &[(i16, i16)],
    factor: f32,
    blast_taken: f32,
) -> Result<(), GenieError> {
    match t.kind {
        70 => take_unit(t, src, others, weapons, factor, blast_taken),
        50 | 60 => damage(t, src, weapons, factor, blast_taken),
        BUILDING => take_building(t, src, others, weapons, factor, true, blast_taken).map(|_| ()),
        _ => {
            take_static(t, &src.source, weapons, factor);
            Ok(())
        }
    }
}

/// A building's slot 0xe8 (`FUN_1411cea90`), `forward` the flag every caller sets (clear for
/// the main building's hit an annex forwards). Its wake from semi-asleep (`+0xc98`: the flag
/// cleared, `FUN_14119deb0` with 0, the owner's `+0x80` list, `FUN_14107db90`) never runs here:
/// only the building's load sets that flag (`FUN_1411df160` -> `FUN_1411ddd20` with the save's
/// `temp_semi_asleep`), and no saved building of the 158041 recordings has it (1473 headers
/// read); the slice does not hold it. With `forward`, an annex (`+0xbb8`) gives the hit to its
/// main building among `others` (whose slot 0xe8 runs with the flag clear and the same
/// `blast_taken`) and takes its hit points when that hit got past its hit point test. Else at 0
/// hit points or fewer (`COMISS 0, hp` / `JNC`: a NaN goes on) nothing happens. The damage
/// ([`damage`]); without a blast's damage so far (`UCOMISS blast_taken, 0`: exactly 0), a hit
/// that took less than its least ([`ONE`]; [`LEAST_CLASS_HIT`] from the
/// [`LEAST_HIT_CLASSES`] under the game's setting `+0x680` at 1) takes the least again (slot
/// 0xb0 with it; `COMISS least, lost` / `JBE` past). Not held: the statistics (`FUN_1417783d0`)
/// and the local player's telemetry (`FUN_140a4b540`). A player's building (`+0x718` above 0)
/// left below 1 hit point (`COMISS 1, hp` / `JBE` past) is razed ([`Razed`]): a castle's or a
/// wonder's history event; but for an annex or a wall (class 27), the razing counts, or a
/// trebuchet's kill counts unless its kill is counted (`+0x954`, which its damage's kill sets:
/// slot 0x8b0's `FUN_14107da00`; a building at 0 hit points takes no hit, so no earlier one
/// did), then the kill bonus ([`kill_bonus`]) and the razing values (172, 153). Then
/// `damaged_lately_timer` (`+0x30d`) is 60 and its annexes take its hit points (the write-back
/// gives them to those `others` does not hold, [`Building::struck`]); with `forward`, an
/// attacker of type 50 and up leaves a building of 1 hit point or more (`COMISS hp, 1` / `JC`
/// past) to its effects ([`attack_effects`]). Answers whether the hit got past the hit point
/// test.
fn take_building(
    t: &mut Victim,
    src: &mut Victim,
    others: &mut [&mut Victim],
    weapons: &[(i16, i16)],
    factor: f32,
    forward: bool,
    blast_taken: f32,
) -> Result<bool, GenieError> {
    let before = t.hp;
    let main = t.building.as_ref().and_then(|b| b.main);
    if forward && let Some(main) = main {
        let k = others
            .iter()
            .position(|v| v.id == main)
            .ok_or(crate::build::NOT_HELD)?;
        let mut m = std::mem::replace(&mut *others[k], Victim::placeholder());
        let struck = take_building(&mut m, src, others, weapons, factor, false, blast_taken);
        let annexed = m
            .building
            .as_ref()
            .is_some_and(|b| b.annexes.contains(&t.id));
        if matches!(struck, Ok(true)) && annexed {
            t.hp = m.hp;
        }
        *others[k] = m;
        return struck.map(|_| false);
    }
    if 0.0 >= t.hp {
        return Ok(false);
    }
    damage(t, src, weapons, factor, blast_taken)?;
    // the kill of its damage ran slot 0x8b0, which counts it (+0x954)
    let counted = t.killed.is_some();
    if blast_taken == 0.0 {
        let least = if src.source.mode_680 == 1 && LEAST_HIT_CLASSES.contains(&src.source.class) {
            LEAST_CLASS_HIT
        } else {
            ONE
        };
        if least > before - t.hp {
            lose(t, least);
        }
    }
    let b = t.building.get_or_insert_with(Building::default);
    if 0 < t.player && ONE > t.hp {
        let (ap, q) = (src.player, t.player);
        let event = match t.master_id {
            crate::turn::CASTLE => Some(crate::history::CASTLE_DESTROYED),
            crate::turn::WONDER => Some(crate::history::WONDER_DESTROYED),
            _ => None,
        };
        let mut adds = Vec::new();
        if b.main.is_none() && t.class != WALL {
            let cost = b.cost;
            // 16-bit adds of the players' ids (`ADD DX, word ptr [.. + 0x718]`)
            let (qa, apa) = (q as i16, ap as i16);
            if !b.trebuchet {
                adds.extend([
                    (ap, 43, ONE),
                    (ap, 170, cost),
                    (q, 152, cost),
                    (q, 155, ONE),
                    (ap, 350 + qa, ONE),
                    (q, 375 + apa, ONE),
                    (ap, 425 + qa, cost),
                ]);
            } else if !counted {
                adds.extend([
                    (ap, 20, ONE),
                    (ap, 170, cost),
                    (q, 154, ONE),
                    (q, 152, cost),
                    (ap, 300 + qa, ONE),
                    (q, 325 + apa, ONE),
                    (ap, 400 + qa, cost),
                ]);
            }
            kill_bonus(&src.source)?;
            adds.extend([(ap, 172, cost), (q, 153, cost)]);
        }
        b.razed.push(Razed {
            attacker_player: ap,
            owner: q,
            event,
            adds,
        });
    }
    b.damaged_lately = DAMAGED_LATELY;
    b.struck = true;
    let annexes = b.annexes.clone();
    for a in others.iter_mut().filter(|a| annexes.contains(&a.id)) {
        a.hp = t.hp;
    }
    if forward && 50 <= src.kind && t.hp >= ONE {
        attack_effects(&src.source, t)?;
    }
    Ok(true)
}

/// `damaged_lately_timer` after a hit on a building (`MOV byte ptr [RBX + 0x30d],0x3c` at
/// `0x1411cf4d2`).
const DAMAGED_LATELY: u8 = 0x3c;

/// `FUN_1415c6510`, the kill bonus of the attacker `src` (type 40 and up) for an object it
/// killed: its master's tasks of action type 0x9a pay their owner's attributes, research
/// technologies and run effects (not modelled); an attacker without one does nothing.
const fn kill_bonus(src: &Attacker) -> Result<(), GenieError> {
    if 40 <= src.kind && src.kill_bonus_tasks {
        return Err(GenieError::unsupported(
            "an attacker's kill bonus task (action type 0x9a)",
        ));
    }
    Ok(())
}

/// `FUN_14107bdf0`, the attacker `src`'s combat ability's effects on its target `t`: only a
/// combat ability with bit 7, against a target whose player is not allied with the attacker's
/// both ways (`FUN_141663d90` with the ally flag, [`Victim::allied`]), does anything (its
/// master's effects on the target, not modelled).
const fn attack_effects(src: &Attacker, t: &Victim) -> Result<(), GenieError> {
    if src.ability & 0x80 != 0 && !(t.allied[0] && t.allied[1]) {
        return Err(GenieError::unsupported(
            "an attacker whose hits apply effects (FUN_14107bdf0)",
        ));
    }
    Ok(())
}

impl Victim {
    /// What stands in a slice of objects for one a hit has taken out for the time it changes it:
    /// no object's id.
    fn placeholder() -> Self {
        Self {
            id: -1,
            ..Self::default()
        }
    }
}

/// A static object's slot 0xe8 (`FUN_14118efd0`), also the animated, doppelganger, moving and
/// action objects' and a tree's: it has no armor (slot 0xf0 `FUN_14118cab0`: a TRIBE attacker's
/// ready charge on it, then the weapons times the factor) and loses the damage (slot 0xb0,
/// `FUN_141189160`); below 1 hit point (`COMISS`/`JBE`: a NaN stays) it has 0 and wakes
/// (`FUN_14119deb0` with 0: `goto_sleep` clears; an object asleep in its player's sleeping list
/// moves to the live list, the caller's), and but for a villager's hit it loses what it holds.
/// The routine's test for no attacker cannot be reached: slot 0xf0 has called the attacker's
/// slot 0x2f0 before it. Its last call (slot 0x4c0, `FUN_141196a00` or `FUN_141158bb0`) plays
/// the master's damage sound (`+0x700`), whose draws are the Unsynced stream's: no simulation
/// state.
fn take_static(t: &mut Victim, src: &Attacker, weapons: &[(i16, i16)], factor: f32) {
    // slot 0x2f0 of a TRIBE attacker (types 70, 80; [`charge_hit`]) starts the sum at its raw
    // charge (slot 0x310, `+0x9e0`) plus 0
    let mut damage = if 70 <= src.kind && charge_hit(src, t) {
        src.charge + 0.0
    } else {
        0.0f32
    };
    for &(_, amount) in weapons {
        damage += f32::from(amount) * factor;
    }
    t.hp -= damage;
    if t.hp < 1.0 {
        t.hp = 0.0;
        t.goto_sleep = false;
        if src.class != VILLAGER {
            t.amount = 0.0;
        }
    }
}

/// A combat object's slot 0xe8 (`FUN_1412108e0`): none on an object below 1 hit point. The
/// special attack (`special_attack_flag` 1) of an idle target facing its attacker takes the
/// damage to 0.66 (its sprite, master `+0x8e8`, is not modelled); the retaliation (flag 2) of an
/// idle target with a unit AI facing an attacker of weapon range below 0.2 strikes the attacker
/// first with its own weapons (four times over a unit whose charge is ready,
/// [`special_charge_ready`]), and an attacker left with at most 1 hit point
/// takes the hit to nothing. Then the damage ([`damage`]); a hit takes at least one hit point
/// (but past the hit point floor); a target with `damage_reflection` gives its attacker that
/// share of the loss, at least 1 (slot 0xb0, no kill handling). Not held: the statistics
/// (`FUN_1417783d0`); not modelled: a kill bonus task of the attacker (action type 0x9a,
/// `FUN_1415c6510`) and its combat ability's effects (bit 7, `FUN_14107bdf0`).
fn take_unit(
    t: &mut Victim,
    src: &mut Victim,
    others: &mut [&mut Victim],
    weapons: &[(i16, i16)],
    factor: f32,
    blast_taken: f32,
) -> Result<(), GenieError> {
    let before = effective(t);
    if t.hp < 1.0 {
        return Ok(());
    }
    let a = t.armored.clone().ok_or(GenieError::unsupported(
        "a combat object without its combat values",
    ))?;
    let mut factor = factor;
    let facing_to = |at: [f32; 3], src: &Victim| -> Result<f32, GenieError> {
        let facing = a.facing.ok_or(GenieError::unsupported(
            "the facing of a target with a special attack or a retaliation (+0x7e8)",
        ))?;
        Ok(facing_offset(at, facing, src.position))
    };
    if a.special_attack == 1 && !a.busy {
        let off = facing_to(t.position, src)?;
        if MINUS_FACING < off && off < FACING {
            factor *= SPECIAL;
            if a.special_graphic {
                return Err(GenieError::unsupported(
                    "a special attack's sprite (master +0x8e8)",
                ));
            }
        }
    }
    let reach = src.source.weapon_range;
    let melee = reach <= RETALIATION_RANGE && RETALIATION_RANGE != reach;
    // the conditions are pure: the ones the slice holds decide first
    if a.special_attack == 2 && a.unit_ai && !a.busy && melee {
        let idle = a.alert_idle.ok_or(GenieError::unsupported(
            "the alert mode and attackers of a retaliating target's unit AI (FUN_140c42b30)",
        ))?;
        if idle {
            let off = facing_to(t.position, src)?;
            if MINUS_FACING < off && off < FACING {
                let mut f = elevation_factor(t.position[2], t.attack_elevation, src);
                if src.kind == 70 && special_charge_ready(&src.source)? {
                    f *= RETALIATION_SCALE;
                }
                let w = t.weapons.clone();
                take(src, t, others, &w, f, 0.0)?;
                if src.hp <= 1.0 {
                    factor = 0.0;
                }
            }
        }
    }
    damage(t, src, weapons, factor, blast_taken)?;
    // a hit takes at least one hit point
    let mut lost = before - effective(t);
    let floor = hp_floor(&a, t.master_hp);
    if lost < 1.0 && (floor == 0.0 || floor + 1.0 <= before) {
        t.hp = before - 1.0;
        lost = 1.0;
    }
    if EPSILON < a.reflection {
        let d = a.reflection * lost;
        lose(src, if 1.0 <= d { d } else { 1.0 });
    }
    // COMISS 1, hp / JBE past: below 1 (not NaN) the kill bonus; COMISS hp, 1 / JC past: at 1
    // or more (not NaN) the attacker's effects
    if ONE > t.hp {
        kill_bonus(&src.source)?;
    }
    if 50 <= src.kind && t.hp >= ONE {
        attack_effects(&src.source, t)?;
    }
    Ok(())
}

/// `FUN_141082e80`: the damage ([`armor_damage`]) beyond what a blast has done: an attacker of
/// type 70 and up records whether the hit is its charge's (slot 0x2f0, `+0x8a8`), which empties
/// a charge without an area effect (slot 0x318 with 0); the target loses it (slot 0xb0). The
/// target's unit AI gets notify 500 (but from an attacker of class 64) with the attacker, the hit
/// points left and its master's, sent by the attacker (a missile, type 60: by what it was fired
/// from, `+0x998`). Below 1 hit point it has 0 and is killed: Gaia takes a player's livestock
/// (type 70). Not modelled: an attacker of type 60 and up whose combat ability marks its target
/// (bit 2, `FUN_141206250`) and a kill reward task (action type 0xa1). Not held: the damaged flag (`+0x683`), the last damage
/// time (`+0x648`), the alerts of another owner (slot 0x2e8), the killer (`+0x690`, `+0x694`),
/// the local player's statistics, the damage sprites (slot 0x4c0) and the trigger events.
fn damage(
    t: &mut Victim,
    src: &mut Victim,
    weapons: &[(i16, i16)],
    factor: f32,
    blast_taken: f32,
) -> Result<(), GenieError> {
    let d = armor_damage(t, weapons, factor, &src.source, bonus_damage(src, t))?;
    if blast_taken < d {
        if 70 <= src.kind {
            let hit = charge_hit(&src.source, t);
            src.source.charge_hit = hit;
            if hit && src.source.area_effect <= 0.0 {
                src.source.charge = 0.0;
            }
        }
        lose(t, d - blast_taken);
    }
    let marks = t.kind == 70 || (t.kind == 80 && matches!(t.class, 51 | 54));
    if 60 <= src.kind && src.source.ability & 4 != 0 && marks {
        return Err(GenieError::unsupported(
            "an attacker whose combat ability marks its target (FUN_141206250)",
        ));
    }
    let master_hp = i32::from(t.master_hp);
    if src.source.class != ELEPHANT_ARCHER {
        let sender = if src.kind == 60 {
            src.source.fired_from
        } else {
            src.id
        };
        t.damaged.push((sender, [src.id, t.hp as i32, master_hp]));
    }
    if t.hp < 1.0 {
        t.hp = 0.0;
        if t.armored.as_ref().is_some_and(|a| a.kill_tasks) {
            return Err(GenieError::unsupported(
                "a kill reward task (action type 0xa1)",
            ));
        }
        t.killed = Some(Kill {
            attacker: src.id,
            attacker_player: src.player,
            to_gaia: t.kind == 70 && t.class == LIVESTOCK && t.player != 0,
        });
    }
    Ok(())
}

/// The blast's rectangle of tiles about `center` for radius `r` (`FUN_14107f690`): the tiles
/// from `(int)(c - r) - 6` (at least 0) to before `(int)(c + r) + 6` (at most the map's edge,
/// which no object passes), each `CVTTSS2SI` into 32 bits kept as a short.
fn blast_tiles(center: [f32; 3], r: f32) -> ([i32; 2], [i32; 2]) {
    let low = |c: f32| {
        let t = (cvttss2si(c - r) as i16).wrapping_sub(6);
        i32::from(t.max(0))
    };
    let high = |c: f32| i32::from((cvttss2si(c + r) as i16).wrapping_add(6));
    (
        [low(center[0]), low(center[1])],
        [high(center[0]), high(center[1])],
    )
}

/// Slot 0x848 (`FUN_14107f690`), the blast of `attacker` about `center` with factor `factor`,
/// over the objects the tiles of [`blast_tiles`] hold, row by row and a tile's objects in its
/// list's order ([`Victim::tile_place`]): the attacker's hit is first marked as no
/// charge's (`+0x8a8`). An object is passed when the blast spares its friends (a weapon range of
/// at most 1 or the level's bit 2: `FUN_1410bfcb0`) and its player is the attacker's or allied,
/// when it is `exclude`, the attacker or an artifact, when its `area_effect_object_level` is
/// at most 1 and below the blast's level, when a ram's blast meets neither a building nor `main`,
/// when it lies beyond the radius (the gap past its radii) or its shield blocks the blast. Its
/// damage factor is the attacker's elevation factor, times its `friendly_fire_damage` when it is
/// of type 70 and up and its player counts the source's as its own or allied; times the falloff
/// `(r - d) / r` (a melee attacker's only with the level's bit 6, and not on `main`; a missile's
/// everywhere, at distance 0 on a `main` building). An object other than `main` lies within the
/// cone of a cone blast (bit 7: within half a facet, or 1.67 for a wide one, bit 5) and takes the
/// attacker's `area_damage` share; a fixed damage (`area_damage` below 0) takes its hit points
/// alone (slot 0xb0; below 1 it has 0 and holds nothing, `FUN_14119d320`; a combat unit of a
/// player that had 1 is counted killed, `FUN_1412005b0`). Then the object's slot 0xe8 with the
/// attacker's weapons ([`take`]), and the attacker's hit is its charge's if any hit was
/// (slot 0x2f0). Every building on the tiles starts the blast with nothing taken (`+0xb90`, a
/// first pass over the tiles); a building's hit leaves out what the blast has done to it so far,
/// its main building's for an annex (`+0xbb8`, which also gives the annex its `area_damage`
/// share and its blast level, the main building's master's), and adds its loss to it; the
/// main building of an annex it reaches is among `targets`. The compares are the assembly's: a NaN gap past a radius is 0
/// (`COMISS`/`JBE`), a NaN square distance is beyond the radius (`JBE`), a NaN facet offset
/// stays in a cone (`JA` past, both the narrow and the wide one), a NaN `area_damage` leaves the
/// factor as it is. Answers whether the radius is above 0 (or NaN: `COMISS 0, r`/`JC`), which
/// neither caller reads.
pub fn blast(
    attacker: &mut Victim,
    center: [f32; 3],
    exclude: Option<i32>,
    main: Option<i32>,
    targets: &mut [&mut Victim],
    factor: f32,
) -> Result<bool, GenieError> {
    let a = attacker.source;
    let r = a.area_effect;
    if !fmath::below_or_unordered(0.0, r) {
        return Ok(false);
    }
    let wide = if a.blast_level & 0x80 != 0 { r } else { 0.0 };
    let (from, to) = blast_tiles(center, r);
    let on_tiles = |t: &Victim| {
        let tile = [t.position[0] as i32, t.position[1] as i32];
        from[0] <= tile[0] && tile[0] < to[0] && from[1] <= tile[1] && tile[1] < to[1]
    };
    let spares_friends = !(1.0 < a.weapon_range && a.blast_level & 4 == 0);
    targets.sort_by_key(|t| (t.position[1] as i32, t.position[0] as i32, t.tile_place));
    // every building on the tiles starts the blast with nothing taken (+0xb90, first pass)
    for t in targets
        .iter_mut()
        .filter(|t| t.kind == BUILDING && on_tiles(t))
    {
        if let Some(b) = t.building.as_mut() {
            b.blast_taken = 0.0;
        }
    }
    attacker.source.charge_hit = false;
    let weapons = attacker.weapons.clone();
    for i in 0..targets.len() {
        let t = &*targets[i];
        if !on_tiles(t) {
            continue;
        }
        if t.uncertain {
            return Err(GenieError::unsupported(
                "a blast over an object whose player's update is not modelled",
            )
            .with("blasted", t.id));
        }
        if (spares_friends && t.allied[0])
            || Some(t.id) == exclude
            || t.id == attacker.id
            || t.class == ARTIFACT
        {
            continue;
        }
        let is_main = Some(t.id) == main;
        // a building keeps the blast's damage so far on its main building (+0xbb8), else on
        // itself; an annex reads its main building's area damage and blast level
        let ledger = if t.kind == BUILDING {
            match t.building.as_ref().and_then(|b| b.main) {
                Some(m) => Some(
                    targets
                        .iter()
                        .position(|v| v.id == m)
                        .ok_or(crate::build::NOT_HELD)?,
                ),
                None => Some(i),
            }
        } else {
            None
        };
        let taken = |v: &Victim| v.building.as_ref().map_or(0.0, |b| b.blast_taken);
        let (share, defense, blast_taken) = match ledger {
            Some(k) if k != i => {
                let m = &*targets[k];
                (m.source.area_damage, m.blast_defense, taken(m))
            }
            Some(_) => (a.area_damage, t.blast_defense, taken(t)),
            None => (a.area_damage, t.blast_defense, 0.0),
        };
        if !(1 < defense || a.blast_level <= defense) {
            continue;
        }
        let t = &mut *targets[i];
        if RAMS.contains(&a.master_id) && t.kind != BUILDING && !is_main {
            continue;
        }
        let gap = |d: f32, radius: f32| {
            if fmath::at_most_or_unordered(d, radius) {
                0.0
            } else {
                d - radius
            }
        };
        // the hit's slack (its info's +0xc, +0x10) is 0 for a melee hit
        let dx = gap((t.position[0] - center[0]).abs(), t.radius[0]);
        let dy = gap((t.position[1] - center[1]).abs(), t.radius[1]);
        let d2 = dy * dy + dx * dx;
        if fmath::at_most_or_unordered(r * r, d2) {
            continue;
        }
        if 70 <= t.kind && shield(t, false) {
            continue;
        }
        let mut f = elevation_factor(attacker.position[2], attacker.attack_elevation, t);
        if 70 <= t.kind && (t.player == attacker.player || t.allied[1]) {
            f *= t.armored.as_ref().map_or(1.0, |x| x.friendly_fire);
        }
        if a.missile < 0 {
            if a.blast_level & 0x40 != 0 && !is_main {
                f *= (r - fmath::sqrt(d2)) / r;
            }
        } else {
            let d = if !is_main || t.kind != BUILDING {
                fmath::sqrt(d2)
            } else {
                0.0
            };
            f *= (r - d) / r;
        }
        if !is_main {
            if a.blast_level & 0x80 != 0 {
                let facing = attacker.armored.as_ref().and_then(|x| x.facing).ok_or(
                    GenieError::unsupported("the facing of an attacker with a cone blast (+0x7e8)"),
                )?;
                if a.blast_level & 0x20 == 0 {
                    let off = facing_offset(attacker.position, facing, t.position);
                    if CONE < off || off < MINUS_CONE {
                        continue;
                    }
                } else {
                    if wide * wide < d2 {
                        continue;
                    }
                    let off = facing_offset(attacker.position, facing, t.position).abs();
                    if WIDE_CONE < off {
                        continue;
                    }
                }
            }
            // COMISS share, 0 / JC, then COMISS 0, share / JBE: a share below 0 is a fixed
            // damage, a NaN one goes on to the hit with the factor as it is
            if share < 0.0 {
                let before = t.hp;
                lose(t, share.abs());
                if t.hp < 1.0 {
                    t.hp = 0.0;
                    t.amount = 0.0;
                    if f32::from(t.capacity) < t.amount {
                        t.amount = f32::from(t.capacity);
                    }
                    if 1.0 <= before && 0 < t.player && t.kind == 70 {
                        t.killed = Some(Kill {
                            attacker: attacker.id,
                            attacker_player: attacker.player,
                            to_gaia: false,
                        });
                        kill_bonus(&a)?;
                    }
                }
                continue;
            }
            if !share.is_nan() {
                f *= share;
            }
        }
        // the target leaves the slice while its hit may reach the others (its main building)
        let before = t.hp;
        let mut t = std::mem::replace(&mut *targets[i], Victim::placeholder());
        let hit = take(&mut t, attacker, targets, &weapons, f * factor, blast_taken);
        if hit.is_ok() {
            if !attacker.source.charge_hit {
                attacker.source.charge_hit = charge_hit(&attacker.source, &t);
            }
            let lost = before - t.hp;
            let ledger = match ledger {
                Some(k) if k == i => t.building.as_mut(),
                Some(k) => targets[k].building.as_mut(),
                None => None,
            };
            // ADDSS of the loss and the blast so far (commutative)
            if let Some(b) = ledger {
                b.blast_taken += lost;
            }
        }
        *targets[i] = t;
        hit?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    /// `FUN_1411cc890`: a building's relics raise its factor (a specific one 1.2, else any 1.1),
    /// a building victim's lower it back the same way; another victim's count for nothing.
    #[test]
    fn a_building_s_relics_scale_its_elevation_factor() {
        let up = 1.25_f32;
        assert_eq!(
            building_elevation_factor(up, [0, 0], 70, [5, 5]).to_bits(),
            up.to_bits()
        );
        assert_eq!(
            building_elevation_factor(up, [3, 1], 70, [0, 0]).to_bits(),
            (up * SPECIFIC_RELIC_BOOST).to_bits()
        );
        assert_eq!(
            building_elevation_factor(up, [3, 0], 70, [0, 0]).to_bits(),
            (up * RELIC_BOOST).to_bits()
        );
        assert_eq!(
            building_elevation_factor(up, [3, 0], 80, [0, 2]).to_bits(),
            (up * RELIC_BOOST / SPECIFIC_RELIC_BOOST).to_bits()
        );
        assert_eq!(
            building_elevation_factor(up, [0, 0], 80, [1, -1]).to_bits(),
            (up / RELIC_BOOST).to_bits()
        );
        // the counts compare signed
        assert_eq!(
            building_elevation_factor(up, [-1, -1], 80, [-1, -1]).to_bits(),
            up.to_bits()
        );
    }

    use super::*;

    fn sheep() -> Victim {
        Victim {
            id: 9520,
            hp: 14.0,
            kind: 70,
            class: LIVESTOCK,
            master_id: 705,
            player: 1,
            position: [10.5, 10.5, 0.0],
            master_hp: 14,
            armored: Some(Armored {
                armors: vec![(4, 0), (3, 0), (31, 0)],
                terrain_bonus: -1,
                friendly_fire: 1.0,
                ..Armored::default()
            }),
            ..Victim::default()
        }
    }

    fn hunter() -> Victim {
        Victim {
            id: 10029,
            hp: 25.0,
            kind: 70,
            class: 4,
            master_id: 592,
            player: 1,
            position: [10.5, 11.0, 0.0],
            source: Attacker {
                id: 10029,
                kind: 70,
                class: 4,
                master_id: 592,
                missile: -1,
                player: 1,
                ..Attacker::default()
            },
            weapons: vec![(11, 3), (4, 3), (13, 6), (17, 0)],
            master_hp: 25,
            armored: Some(Armored {
                armors: vec![(4, 0), (3, 0)],
                terrain_bonus: -1,
                friendly_fire: 1.0,
                ..Armored::default()
            }),
            ..Victim::default()
        }
    }

    /// MarvinBlue/426048731's update 1170: the hunter 10029 (master 592: classes 11, 4, 13 and
    /// 17 at 3, 3, 6 and 0) strikes its sheep 9520 (master 705: armor 0 for 4, 3 and 31, base 0):
    /// 3 + 3 + 6 = 12, the game's 14 -> 2 hit points.
    /// Slot 0x870 (`FUN_1410857f0`): the best pierce armor, else the base armor and the best of
    /// any; a wall foundation (class 27, state 0) has no case of its own.
    #[test]
    fn a_wall_foundation_s_pierce_armor_is_computed_as_any_other() {
        let a = Armored {
            armors: vec![(4, 2), (3, 5)],
            base_armor: 1,
            ..Armored::default()
        };
        assert_eq!(class_armor(&a, 3, 27, 0), 5);
        assert_eq!(class_armor(&a, 3, 27, 2), 5);
        let none = Armored {
            armors: vec![(4, 2)],
            base_armor: 1,
            ..Armored::default()
        };
        assert_eq!(class_armor(&none, 3, 27, 0), 2);
        // its melee armor (FUN_141213810's wall foundation) is 0
        assert_eq!(class_armor(&a, 4, 27, 0), 0);
    }

    /// Slot 0xf0 (`FUN_14118cab0`): a TRIBE attacker whose charge hits the target (slot 0x2f0)
    /// starts the static damage at its charge; any other starts at 0.
    #[test]
    fn a_static_object_takes_a_ready_charge_first() {
        let tree = || Victim {
            hp: 100.0,
            class: 15,
            kind: 10,
            ..Victim::default()
        };
        let charging = Attacker {
            kind: 70,
            charge_type: 1,
            charge_target: -1,
            charge: 5.0,
            max_charge: 5.0,
            ..Attacker::default()
        };
        let mut t = tree();
        take_static(&mut t, &charging, &[(4, 3)], 1.0);
        assert_eq!(t.hp, 100.0 - (5.0 + 3.0));
        let mut t = tree();
        let plain = Attacker {
            charge_type: 0,
            ..charging
        };
        take_static(&mut t, &plain, &[(4, 3)], 1.0);
        assert_eq!(t.hp, 97.0);
    }

    /// Slot 0xe8 of a moving or action object (types 30 to 49) is the static object's
    /// (`FUN_14118efd0`): the weapons times the factor, no armor; below 1 hit point it has 0,
    /// wakes and loses what it holds but to a villager. A NaN hit point count stays (`COMISS`/`JBE`).
    #[test]
    fn a_moving_object_takes_a_hit_as_a_static_one() {
        let bird = |hp: f32| Victim {
            hp,
            amount: 50.0,
            kind: 40,
            class: 38,
            goto_sleep: true,
            ..Victim::default()
        };
        let mut hunter = hunter();
        let mut t = bird(10.0);
        take(&mut t, &mut hunter, &mut [], &[(4, 3), (3, 2)], 1.0, 0.0).unwrap();
        assert_eq!((t.hp, t.amount, t.goto_sleep), (5.0, 50.0, true));
        take(&mut t, &mut hunter, &mut [], &[(4, 3), (3, 2)], 1.0, 0.0).unwrap();
        // a villager's hit leaves what it holds
        assert_eq!((t.hp, t.amount, t.goto_sleep), (0.0, 50.0, false));
        let mut archer = hunter.clone();
        archer.source.class = 0;
        let mut t = bird(4.0);
        take(&mut t, &mut archer, &mut [], &[(3, 4)], 1.0, 0.0).unwrap();
        assert_eq!((t.hp, t.amount), (0.0, 0.0));
        let mut t = bird(f32::NAN);
        take(&mut t, &mut archer, &mut [], &[(3, 4)], 1.0, 0.0).unwrap();
        assert!(t.hp.is_nan() && t.amount == 50.0 && t.goto_sleep);
    }

    #[test]
    fn a_hunter_strikes_its_sheep_for_twelve() {
        let (mut sheep, mut hunter) = (sheep(), hunter());
        let weapons = hunter.weapons.clone();
        assert_eq!(
            armor_damage(&sheep, &weapons, 1.0, &hunter.source, 0.0),
            Ok(12.0)
        );
        take(&mut sheep, &mut hunter, &mut [], &weapons, 1.0, 0.0).unwrap();
        assert_eq!(sheep.hp, 2.0);
        assert_eq!(sheep.damaged, [(10029, [10029, 2, 14])]);
        // the next hit kills it, and Gaia takes a player's livestock
        take(&mut sheep, &mut hunter, &mut [], &weapons, 1.0, 0.0).unwrap();
        assert_eq!(sheep.hp, 0.0);
        assert_eq!(
            sheep.killed,
            Some(Kill {
                attacker: 10029,
                attacker_player: 1,
                to_gaia: true
            })
        );
        // armor above the attack does nothing, and a hit does at least one
        let mut armored = self::sheep();
        let x = armored.armored.as_mut().unwrap();
        (x.armors, x.base_armor) = (vec![(4, 10)], 10000);
        assert_eq!(
            armor_damage(&armored, &[(4, 3)], 1.0, &hunter.source, 0.0),
            Ok(1.0)
        );
        let zero = Attacker {
            mode_680: 1,
            ..hunter.source
        };
        assert_eq!(armor_damage(&armored, &[(4, 3)], 1.0, &zero, 0.0), Ok(0.0));
    }

    /// Slots 0xc0 and 0xc8 of a combat unit read its hit point powerups; a type-50 object's do
    /// not. The share is at most 1 (`MINSS`), 0 without most hit points (lacking too), and a
    /// NaN most divides.
    #[test]
    fn a_hit_point_share_reads_the_powerups_and_the_master() {
        let mut v = hunter();
        assert_eq!(
            (v.shown_hp(), v.most_hp(), v.hp_share(false)),
            (25.0, 25.0, 1.0)
        );
        assert_eq!(v.hp_share(true), 0.0);
        v.hp = 10.0;
        v.armored.as_mut().unwrap().hp_powerups = [2.0, 5.0];
        assert_eq!((v.shown_hp(), v.most_hp()), (13.0, 30.0));
        assert_eq!(v.hp_share(false), 13.0 / 30.0);
        assert_eq!(v.hp_share(true), 1.0 - 13.0 / 30.0);
        v.hp = 40.0;
        assert_eq!(v.hp_share(false), 1.0);
        let mut rge = hunter();
        rge.kind = 50;
        rge.hp = 10.0;
        rge.armored.as_mut().unwrap().hp_powerups = [2.0, 5.0];
        assert_eq!((rge.shown_hp(), rge.most_hp()), (10.0, 25.0));
        let mut none = hunter();
        none.master_hp = 0;
        assert_eq!((none.hp_share(false), none.hp_share(true)), (0.0, 0.0));
        let mut nan = hunter();
        nan.armored.as_mut().unwrap().hp_powerups = [0.0, f32::NAN];
        assert!(nan.hp_share(false).is_nan());
    }

    /// `FUN_14107ce60`: each task of action type 0xa0 that fits the target adds its amount per
    /// whole step of a share (the target's with a work range above 0, else the attacker's; the
    /// share lacking with `attribute_type4`), and the sum is at least 0.
    #[test]
    fn bonus_damage_counts_whole_steps_of_a_hit_point_share() {
        let task =
            |object_group: i16, object_id: i16, range: f32, lacking: i16| crate::masters::Task {
                action_type: BONUS_DAMAGE,
                object_group,
                object_id,
                work_range: range,
                attribute_type4: lacking,
                work_val1: 2.0,
                work_val2: 0.25,
                ..crate::masters::Task::default()
            };
        let (mut sheep, mut hunter) = (sheep(), hunter());
        sheep.hp = 7.0;
        // any target below type 80: the sheep lacks half its hit points, two steps of 0.25
        hunter.bonus_tasks = vec![task(-1, -1, 1.0, 1)];
        let steps = libm::floorf(0.5 / 0.25 + EPSILON);
        assert_eq!(bonus_damage(&hunter, &sheep), steps * 2.0);
        // the attacker's share (a work range of 0), whole: four steps
        hunter.bonus_tasks = vec![task(-1, -1, 0.0, 0)];
        assert_eq!(bonus_damage(&hunter, &sheep), 4.0 * 2.0);
        // by class or master; another class, master or action type adds nothing
        hunter.bonus_tasks = vec![
            task(LIVESTOCK, 0, 0.0, 0),
            task(-1, 705, 0.0, 0),
            task(3, 0, 0.0, 0),
            task(-1, 9, 0.0, 0),
            crate::masters::Task {
                action_type: 0xa1,
                ..task(-1, -1, 0.0, 0)
            },
        ];
        assert_eq!(bonus_damage(&hunter, &sheep), 16.0);
        // a building of another class than 51 and 54 needs its class or master
        let mut tc = sheep.clone();
        (tc.kind, tc.class) = (BUILDING, 3);
        hunter.bonus_tasks = vec![task(-1, -1, 0.0, 0)];
        assert_eq!(bonus_damage(&hunter, &tc), 0.0);
        tc.class = 51;
        assert_eq!(bonus_damage(&hunter, &tc), 8.0);
        // no step, no target hit points, or a negative sum: 0
        hunter.bonus_tasks = vec![crate::masters::Task {
            work_val2: 0.0,
            ..task(-1, -1, 0.0, 0)
        }];
        assert_eq!(bonus_damage(&hunter, &sheep), 0.0);
        hunter.bonus_tasks = vec![task(-1, -1, 0.0, 0)];
        sheep.hp = 0.0;
        assert_eq!(bonus_damage(&hunter, &sheep), 0.0);
        sheep.hp = 7.0;
        hunter.bonus_tasks = vec![crate::masters::Task {
            work_val1: -3.0,
            ..task(-1, -1, 0.0, 0)
        }];
        assert_eq!(bonus_damage(&hunter, &sheep), 0.0);
    }

    /// The first melee weapon adds the bonus with the powerup.
    #[test]
    fn the_bonus_damage_adds_to_the_first_melee_weapon() {
        let (sheep, mut hunter) = (sheep(), hunter());
        hunter.source.attack_powerup = 1.0;
        assert_eq!(
            armor_damage(&sheep, &[(4, 3), (3, 2)], 1.0, &hunter.source, 2.0),
            Ok(3.0 + 2.0 + 1.0 + 2.0)
        );
    }

    /// Slots 0x878, 0x868 and 0x880: the best fighting attack (else any) with the whole
    /// powerup, the bonus against the attack's target and a ready charge of type 1; a ready
    /// charge of type 6 or 7 shows the projectile's attack and adds the charge event to the
    /// range; type 50 shows neither powerups nor charges.
    #[test]
    fn a_combat_object_shows_its_attack_armor_and_range() {
        let (sheep, mut v) = (sheep(), hunter());
        let none = Shown::default();
        assert_eq!(shown_attack(&v, &none), 3);
        v.source.attack_powerup = 1.7;
        assert_eq!(shown_attack(&v, &none), 4);
        let mut far = v.clone();
        far.weapons = vec![(13, 6), (11, -2)];
        assert_eq!(shown_attack(&far, &none), 7);
        far.kind = 50;
        assert_eq!(shown_attack(&far, &none), 6);
        // the bonus against its attack's target
        v.bonus_tasks = vec![crate::masters::Task {
            action_type: BONUS_DAMAGE,
            attribute_type4: 0,
            work_val1: 1.5,
            work_val2: 0.5,
            ..crate::masters::Task::default()
        }];
        let at_sheep = Shown {
            target: Some(&sheep),
            ..Shown::default()
        };
        assert_eq!(shown_attack(&v, &at_sheep), 4 + 3);
        v.bonus_tasks.clear();
        // a ready charge of type 1 for every target, or for its target's class group
        (v.source.charge_type, v.source.max_charge, v.source.charge) = (1, 5.0, 5.5);
        v.source.charge_target = -1;
        assert_eq!(shown_attack(&v, &none), 4 + 5);
        v.source.charge_target = 0;
        assert_eq!(shown_attack(&v, &none), 4);
        assert_eq!(shown_attack(&v, &at_sheep), 4 + 5);
        // a ready charge of type 6 shows the projectile's best fighting attack, and its range
        // the charge event
        (v.source.charge_type, v.source.charge_target) = (6, -1);
        let projectile = [(13, 20), (3, 9), (4, 2)];
        let shot = Shown {
            projectile: Some(&projectile),
            charge_event: 2,
            range_powerup: 1.0,
            ..Shown::default()
        };
        assert_eq!(shown_attack(&v, &shot), 9);
        v.source.weapon_range = 4.0;
        assert_eq!(shown_range(&v, &shot), 1.0 + (2.0 + 4.0));
        v.source.charge = 0.5;
        assert_eq!(shown_attack(&v, &shot), 4);
        assert_eq!(shown_range(&v, &shot), 5.0);
        v.kind = 50;
        v.source.charge = 1.0;
        assert_eq!(shown_range(&v, &shot), 4.0);
        // the melee armor: its best of class 4 plus the powerup; a wall foundation none; type 50
        // without the powerup; else its best of any, else its base armor
        let mut a = hunter();
        a.armored.as_mut().unwrap().armor_powerups = [2, 0];
        assert_eq!(shown_melee_armor(&a), Ok(2));
        (a.class, a.state) = (27, 0);
        assert_eq!(shown_melee_armor(&a), Ok(0));
        a.kind = 50;
        a.armored.as_mut().unwrap().armors = vec![(3, 5), (11, 7)];
        assert_eq!(shown_melee_armor(&a), Ok(7));
        a.armored.as_mut().unwrap().armors.clear();
        a.armored.as_mut().unwrap().base_armor = -1;
        assert_eq!(shown_melee_armor(&a), Ok(-1));
        a.armored = None;
        assert!(shown_melee_armor(&a).is_err());
    }

    /// Slot 0x2f8: a ready charge of type 6 or 7 that works on the target.
    #[test]
    fn a_range_charge_hits_a_target_its_class_group_names() {
        let (sheep, mut v) = (sheep(), hunter());
        (v.source.charge_type, v.source.max_charge, v.source.charge) = (7, 5.0, 1.0);
        v.source.charge_target = 0;
        assert!(range_charge_hit(&v.source, &sheep));
        v.source.charge = 0.5;
        assert!(!range_charge_hit(&v.source, &sheep));
        (v.source.charge_type, v.source.charge) = (1, 9.0);
        assert!(!range_charge_hit(&v.source, &sheep));
    }

    /// The terrain defense scales each weapon's damage; a restriction value of 0 counts as 1 for
    /// the caller ([`crate::terrain::Terrain::defense`]).
    #[test]
    fn a_terrain_defense_bonus_scales_the_damage() {
        let hunter = hunter();
        let mut sheep = sheep();
        let x = sheep.armored.as_mut().unwrap();
        (x.terrain_bonus, x.terrain) = (4, Some(1.5));
        assert_eq!(
            armor_damage(&sheep, &[(4, 4)], 1.0, &hunter.source, 0.0),
            Ok(6.0)
        );
        sheep.armored.as_mut().unwrap().terrain = None;
        assert!(armor_damage(&sheep, &[(4, 4)], 1.0, &hunter.source, 0.0).is_err());
    }

    /// A ready charge of type 1 adds itself to the first melee weapon; the hit is the charge's,
    /// which empties it (no area effect).
    #[test]
    fn a_ready_charge_adds_to_the_first_melee_weapon_and_empties() {
        let (mut sheep, mut hunter) = (sheep(), hunter());
        hunter.source.charge_type = 1;
        hunter.source.charge_target = -1;
        hunter.source.max_charge = 5.0;
        hunter.source.charge = 5.0;
        assert_eq!(
            armor_damage(&sheep, &[(4, 3), (3, 2)], 1.0, &hunter.source, 0.0),
            Ok(5.0 + 3.0 + 2.0)
        );
        take(&mut sheep, &mut hunter, &mut [], &[(4, 3)], 1.0, 0.0).unwrap();
        assert_eq!(sheep.hp, 14.0 - 8.0);
        assert!(hunter.source.charge_hit);
        assert_eq!(hunter.source.charge, 0.0);
        // not ready: no charge
        hunter.source.charge = 4.0;
        assert_eq!(
            armor_damage(&sheep, &[(4, 3)], 1.0, &hunter.source, 0.0),
            Ok(3.0)
        );
    }

    #[test]
    fn a_charge_target_picks_the_class_groups() {
        // 0: everything but buildings outside classes 51 and 54
        assert!(charge_works(0, 1, 6, 70));
        assert!(!charge_works(0, 1, 3, 80));
        assert!(charge_works(0, 1, 51, 70));
        // bit 0 infantry (6), bit 8 buildings (3)
        assert!(charge_works(1, 1, 6, 70));
        assert!(!charge_works(1, 1, 12, 70));
        assert!(charge_works(0x100, 1, 3, 80));
        assert!(!charge_works(0x100, 1, 99, 70));
        assert!(charge_works(0x10, -4, 99, 70));
    }

    /// A shield of type 5 spends a charge of 1 on a melee hit; type 4 only on a missile.
    #[test]
    fn a_shield_spends_a_charge_per_blocked_hit() {
        let mut sheep = sheep();
        sheep.source.charge_type = 5;
        sheep.source.charge = 1.5;
        assert!(!shield(&mut sheep, true));
        assert!(shield(&mut sheep, false));
        assert_eq!(sheep.source.charge, 0.5);
        assert!(!shield(&mut sheep, false));
        // COMISS 1.0, charge; JA past: a NaN charge blocks the hit and stays NaN
        sheep.source.charge = f32::NAN;
        assert!(shield(&mut sheep, false));
        assert!(sheep.source.charge.is_nan());
    }

    #[test]
    fn a_facing_offset_counts_facets_from_the_angle() {
        let at = [10.0, 10.0, 0.0];
        let facing = Facing {
            angle: 0.0,
            facets: 16,
        };
        assert_eq!(facing_offset(at, facing, [11.0, 10.0, 0.0]), 0.0);
        // a quarter turn is four of 16 facets; fewer than 8 facets count as 8
        let q = facing_offset(at, facing, [10.0, 11.0, 0.0]);
        assert!((q - 4.0).abs() < 1e-5);
        let few = Facing {
            facets: 4,
            ..facing
        };
        assert!((facing_offset(at, few, [10.0, 11.0, 0.0]) - 2.0).abs() < 1e-5);
        // a NaN difference goes on to atan2f (COMISS/JC, JNC: unordered is not "within 1e-7")
        assert!(facing_offset(at, facing, [f32::NAN, 10.0, 0.0]).is_nan());
        // behind wraps into −π..π, and no sprite or the same point gives 0
        let back = facing_offset(at, facing, [9.0, 9.999, 0.0]);
        assert!(7.9 < back.abs() && back.abs() <= 8.0);
        assert_eq!(
            facing_offset(
                at,
                Facing {
                    facets: 0,
                    ..facing
                },
                [11.0, 12.0, 0.0]
            ),
            0.0
        );
        assert_eq!(facing_offset(at, facing, at), 0.0);
    }

    /// A special attack (flag 1) of an idle target facing its attacker takes the damage to 0.66.
    #[test]
    fn a_special_attack_takes_a_hit_from_the_front_to_two_thirds() {
        let (mut sheep, mut hunter) = (sheep(), hunter());
        let x = sheep.armored.as_mut().unwrap();
        x.special_attack = 1;
        x.facing = Some(Facing {
            angle: std::f32::consts::FRAC_PI_2,
            facets: 8,
        });
        take(&mut sheep, &mut hunter, &mut [], &[(4, 10)], 1.0, 0.0).unwrap();
        assert_eq!(sheep.hp, 14.0 - 10.0 * SPECIAL);
        // a busy target does not, and an unknown facing is not modelled
        let mut busy = self::sheep();
        busy.armored.as_mut().unwrap().special_attack = 1;
        busy.armored.as_mut().unwrap().busy = true;
        take(&mut busy, &mut hunter, &mut [], &[(4, 10)], 1.0, 0.0).unwrap();
        assert_eq!(busy.hp, 4.0);
        let mut unknown = self::sheep();
        unknown.armored.as_mut().unwrap().special_attack = 1;
        assert!(take(&mut unknown, &mut hunter, &mut [], &[(4, 10)], 1.0, 0.0).is_err());
    }

    /// A retaliation (flag 2) strikes a melee attacker first; an attacker it leaves at 1 hit
    /// point or less takes the hit to nothing.
    #[test]
    fn a_retaliation_strikes_the_melee_attacker_first() {
        let (mut sheep, mut hunter) = (sheep(), hunter());
        sheep.weapons = vec![(4, 5)];
        let x = sheep.armored.as_mut().unwrap();
        x.special_attack = 2;
        x.unit_ai = true;
        x.alert_idle = Some(true);
        x.facing = Some(Facing {
            angle: std::f32::consts::FRAC_PI_2,
            facets: 8,
        });
        take(&mut sheep, &mut hunter, &mut [], &[(4, 3)], 1.0, 0.0).unwrap();
        assert_eq!((hunter.hp, sheep.hp), (20.0, 11.0));
        assert_eq!(hunter.damaged, [(9520, [9520, 20, 25])]);
        hunter.hp = 5.0;
        take(&mut sheep, &mut hunter, &mut [], &[(4, 3)], 1.0, 0.0).unwrap();
        // the hunter dies; the sheep loses the least, one hit point
        assert_eq!((hunter.hp, sheep.hp), (0.0, 10.0));
        assert!(hunter.killed.is_some());
    }

    /// A target that reflects gives its attacker that share of its loss, at least 1.
    #[test]
    fn reflected_damage_comes_back_at_least_one() {
        let (mut sheep, mut hunter) = (sheep(), hunter());
        sheep.armored.as_mut().unwrap().reflection = 0.5;
        take(&mut sheep, &mut hunter, &mut [], &[(4, 6)], 1.0, 0.0).unwrap();
        assert_eq!((sheep.hp, hunter.hp), (8.0, 22.0));
        take(&mut sheep, &mut hunter, &mut [], &[(4, 1)], 1.0, 0.0).unwrap();
        assert_eq!((sheep.hp, hunter.hp), (7.0, 21.0));
        assert!(hunter.damaged.is_empty() && hunter.killed.is_none());
    }

    /// `FUN_14107f690`'s float compares as its assembly has them: a NaN gap past a radius counts
    /// as 0 (`COMISS`/`JBE`), a NaN offset stays in a narrow cone (two `JA`s past), a NaN
    /// `area_damage` leaves the factor as it is (`JC`, then `JBE` past the multiply); the tiles'
    /// rectangle truncates with `CVTTSS2SI`; the answer is whether the radius is above 0 or NaN.
    #[test]
    fn a_blast_s_nan_paths_are_the_game_s() {
        let mut elephant = hunter();
        elephant.source.area_effect = 1.0;
        elephant.source.blast_level = 0x42;
        elephant.source.area_damage = 0.5;
        elephant.weapons = vec![(4, 10)];
        let center = elephant.position;
        let enemy = |x: f32, y: f32| Victim {
            id: 2,
            player: 2,
            position: [x, y, 0.0],
            blast_defense: 2,
            ..sheep()
        };
        // a NaN radius on x: the gap on x is 0, the falloff takes the gap on y alone
        let mut t = Victim {
            radius: [f32::NAN, 0.0],
            ..enemy(10.8, 11.5)
        };
        assert_eq!(
            blast(&mut elephant, center, None, None, &mut [&mut t], 1.0),
            Ok(true)
        );
        let f = (1.0 - fmath::sqrt(0.5 * 0.5)) / 1.0 * 0.5;
        assert_eq!(t.hp, 14.0 - 10.0 * f);
        // a narrow cone keeps a NaN offset
        elephant.source.blast_level = 0x82;
        elephant.armored.as_mut().unwrap().facing = Some(Facing {
            angle: f32::NAN,
            facets: 8,
        });
        let mut t = enemy(10.8, 11.0);
        assert!(
            facing_offset(
                center,
                Facing {
                    angle: f32::NAN,
                    facets: 8
                },
                t.position
            )
            .is_nan()
        );
        assert_eq!(
            blast(&mut elephant, center, None, None, &mut [&mut t], 1.0),
            Ok(true)
        );
        assert_eq!(t.hp, 9.0);
        // a NaN area damage share leaves the factor at 1
        elephant.source.blast_level = 2;
        elephant.source.area_damage = f32::NAN;
        let mut t = enemy(10.8, 11.0);
        assert_eq!(
            blast(&mut elephant, center, None, None, &mut [&mut t], 1.0),
            Ok(true)
        );
        assert_eq!(t.hp, 4.0);
        // no radius, no blast; a NaN radius blasts
        elephant.source.area_effect = 0.0;
        assert_eq!(
            blast(&mut elephant, center, None, None, &mut [], 1.0),
            Ok(false)
        );
        elephant.source.area_effect = f32::NAN;
        assert_eq!(
            blast(&mut elephant, center, None, None, &mut [], 1.0),
            Ok(true)
        );
        // CVTTSS2SI gives 0x80000000 past the i32 range: 0 in the low short
        assert_eq!(blast_tiles([3e9, 3e9, 0.0], 1.0), ([0, 0], [6, 6]));
        assert_eq!(blast_tiles([10.5, 4.5, 0.0], 1.0), ([3, 0], [17, 11]));
    }

    /// A melee blast (radius 0.5, level 2, half the damage) reaches the objects about the
    /// attacker but its friends, its target excluded; a falloff (bit 6) scales by the distance.
    #[test]
    fn a_blast_reaches_the_objects_about_its_attacker() {
        let mut elephant = hunter();
        elephant.source.area_effect = 1.0;
        elephant.source.blast_level = 2;
        elephant.source.area_damage = 0.5;
        elephant.source.player = 1;
        elephant.weapons = vec![(4, 10)];
        let enemy = |id: i32, x: f32| Victim {
            id,
            player: 2,
            position: [x, 11.0, 0.0],
            blast_defense: 2,
            ..sheep()
        };
        let mut target = enemy(1, 10.6);
        let mut near = enemy(2, 10.8);
        let mut far = enemy(3, 12.0);
        let mut friend = Victim {
            allied: [true, true],
            ..enemy(4, 10.7)
        };
        let center = elephant.position;
        let mut all = [&mut target, &mut near, &mut far, &mut friend];
        assert_eq!(
            blast(&mut elephant, center, Some(1), None, &mut all, 1.0),
            Ok(true)
        );
        assert_eq!(
            (target.hp, near.hp, far.hp, friend.hp),
            (14.0, 9.0, 14.0, 14.0)
        );
        // the falloff: 0.3 of radius 1 away keeps 0.7
        elephant.source.blast_level = 0x42;
        let mut near = enemy(2, 10.8);
        let mut all = [&mut near];
        assert_eq!(
            blast(&mut elephant, center, None, None, &mut all, 1.0),
            Ok(true)
        );
        let dx = (10.8f32 - 10.5f32).abs();
        let f = (1.0 - fmath::sqrt(0.0 * 0.0 + dx * dx)) / 1.0 * 0.5;
        assert_eq!(near.hp, 14.0 - 10.0 * f);
    }

    /// A player's house (master 70, class 3, type 80) of 550 hit points: melee armor 0, pierce
    /// armor 7; it cost 25 wood, paid.
    fn house() -> Victim {
        Victim {
            id: 500,
            hp: 550.0,
            kind: BUILDING,
            class: 3,
            master_id: 70,
            player: 2,
            position: [20.5, 20.5, 0.0],
            master_hp: 550,
            armored: Some(Armored {
                armors: vec![(4, 0), (3, 7)],
                terrain_bonus: -1,
                friendly_fire: 1.0,
                ..Armored::default()
            }),
            building: Some(Building {
                cost: 25.0,
                ..Building::default()
            }),
            ..Victim::default()
        }
    }

    /// `FUN_1411cea90`: a hit on a building is its damage (`FUN_141082e80`), then 60 for
    /// `damaged_lately_timer` (`+0x30d`). Without a blast's damage so far (`param_8`, `UCOMISS`
    /// with 0: exactly 0), a hit that took less than its least takes the least again (slot
    /// 0xb0 with it, not the difference: `MOVAPS XMM1,XMM6` at `0x1411cef02`): 1, or 0.1 for an
    /// attacker of class 0, 22, 36, 44 or 52 under the game's setting `+0x680` at 1.
    #[test]
    fn a_building_takes_its_damage_and_at_least_its_least_hit() {
        let mut villager = hunter();
        let mut h = house();
        take(&mut h, &mut villager, &mut [], &[(4, 3)], 1.0, 0.0).unwrap();
        assert_eq!(h.hp, 547.0);
        assert_eq!(h.damaged, [(10029, [10029, 547, 550])]);
        let b = h.building.as_ref().unwrap();
        assert_eq!((b.damaged_lately, b.struck, b.razed.len()), (60, true, 0));
        // at 1 a hit may do nothing: the least takes 1 more
        villager.source.mode_680 = 1;
        let mut h = house();
        h.armored.as_mut().unwrap().armors = vec![(4, 10)];
        take(&mut h, &mut villager, &mut [], &[(4, 3)], 1.0, 0.0).unwrap();
        assert_eq!(h.hp, 549.0);
        // half a hit point taken: the least is taken whole again
        let mut h = house();
        take(&mut h, &mut villager, &mut [], &[(4, 1)], 0.5, 0.0).unwrap();
        assert_eq!(h.hp, 548.5);
        // an archer's least is a tenth
        let mut archer = villager.clone();
        archer.source.class = 0;
        let mut h = house();
        h.armored.as_mut().unwrap().armors = vec![(4, 10)];
        take(&mut h, &mut archer, &mut [], &[(4, 3)], 1.0, 0.0).unwrap();
        assert_eq!(h.hp, 550.0 - LEAST_CLASS_HIT);
        // a blast's damage so far, even a NaN one, takes no least
        let mut h = house();
        h.armored.as_mut().unwrap().armors = vec![(4, 10)];
        take(&mut h, &mut archer, &mut [], &[(4, 3)], 1.0, 2.0).unwrap();
        assert_eq!(h.hp, 550.0);
        take(&mut h, &mut archer, &mut [], &[(4, 3)], 1.0, f32::NAN).unwrap();
        assert_eq!(h.hp, 550.0);
    }

    /// `COMISS 0, hp` / `JNC` past: a building at 0 hit points or fewer takes nothing (and is not
    /// struck); a NaN goes on.
    #[test]
    fn a_building_without_hit_points_takes_no_hit() {
        let mut villager = hunter();
        let mut h = Victim { hp: 0.0, ..house() };
        take(&mut h, &mut villager, &mut [], &[(4, 3)], 1.0, 0.0).unwrap();
        assert_eq!((h.hp, h.damaged.is_empty()), (0.0, true));
        assert!(!h.building.as_ref().unwrap().struck);
        let mut h = Victim {
            hp: f32::NAN,
            ..house()
        };
        take(&mut h, &mut villager, &mut [], &[(4, 3)], 1.0, 0.0).unwrap();
        assert!(!h.damaged.is_empty() && h.building.as_ref().unwrap().struck);
        // a NaN is not below 1: no razing
        assert_eq!(h.building.as_ref().unwrap().razed, Vec::new());
    }

    /// An annex (`+0xbb8`) gives a hit with the flag set (every caller's) to its main building's
    /// slot 0xe8 with the flag clear: the main building's armor takes it, and each of its annexes
    /// (`+0xbc0`..) takes its hit points; the annex itself is not struck. A main building the hit
    /// does not hold is not modelled.
    #[test]
    fn a_hit_on_an_annex_goes_to_its_main_building() {
        let mut villager = hunter();
        let mut main = Victim {
            id: 10,
            hp: 2400.0,
            building: Some(Building {
                annexes: vec![11, 12],
                cost: 275.0,
                ..Building::default()
            }),
            ..house()
        };
        main.armored.as_mut().unwrap().armors = vec![(4, 1)];
        let annex = |id| Victim {
            id,
            hp: 2400.0,
            building: Some(Building {
                main: Some(10),
                ..Building::default()
            }),
            ..house()
        };
        let (mut a, mut other) = (annex(11), annex(12));
        take(
            &mut a,
            &mut villager,
            &mut [&mut main, &mut other],
            &[(4, 3)],
            1.0,
            0.0,
        )
        .unwrap();
        assert_eq!((main.hp, a.hp, other.hp), (2398.0, 2398.0, 2398.0));
        assert!(main.building.as_ref().unwrap().struck);
        assert!(!a.building.as_ref().unwrap().struck);
        assert_eq!(main.damaged, [(10029, [10029, 2398, 550])]);
        assert_eq!(a.damaged, []);
        let mut alone = annex(11);
        assert_eq!(
            take(&mut alone, &mut villager, &mut [], &[(4, 3)], 1.0, 0.0),
            Err(crate::build::NOT_HELD)
        );
    }

    /// A player's building left below 1 hit point (`COMISS 1, hp` / `JBE` past) is razed: a
    /// castle (82) adds event 7 and a wonder (276) event 5 to the attacker's player's history;
    /// then, but for an annex or a wall (class 27), the razing counts (attacker's player 43,
    /// 170; owner 152, 155; 350 + owner; owner 375 + attacker's player; 425 + owner) and 172
    /// and 153 with the paid cost. Gaia's buildings count nothing.
    #[test]
    fn a_player_s_building_below_one_hit_point_is_razed() {
        let mut villager = hunter();
        let mut castle = Victim {
            hp: 1.5,
            master_id: 82,
            ..house()
        };
        take(&mut castle, &mut villager, &mut [], &[(4, 3)], 1.0, 0.0).unwrap();
        assert_eq!(castle.hp, 0.0);
        assert!(castle.killed.is_some());
        let razed = &castle.building.as_ref().unwrap().razed;
        assert_eq!(
            razed,
            &[Razed {
                attacker_player: 1,
                owner: 2,
                event: Some(crate::history::CASTLE_DESTROYED),
                adds: vec![
                    (1, 43, 1.0),
                    (1, 170, 25.0),
                    (2, 152, 25.0),
                    (2, 155, 1.0),
                    (1, 352, 1.0),
                    (2, 376, 1.0),
                    (1, 427, 25.0),
                    (1, 172, 25.0),
                    (2, 153, 25.0),
                ],
            }]
        );
        let mut wonder = Victim {
            hp: 1.5,
            master_id: 276,
            class: WALL,
            ..house()
        };
        take(&mut wonder, &mut villager, &mut [], &[(4, 3)], 1.0, 0.0).unwrap();
        let razed = &wonder.building.as_ref().unwrap().razed;
        assert_eq!(
            (razed[0].event, razed[0].adds.len()),
            (Some(crate::history::WONDER_DESTROYED), 0)
        );
        let mut gaia = Victim {
            hp: 1.5,
            player: 0,
            ..house()
        };
        take(&mut gaia, &mut villager, &mut [], &[(4, 3)], 1.0, 0.0).unwrap();
        assert_eq!(gaia.building.as_ref().unwrap().razed, Vec::new());
    }

    /// A trebuchet (`FUN_140fde380`) counts as a unit's kill (attacker's player 20, 170; owner
    /// 154, 152; 300 + owner; owner 325 + attacker's player; 400 + owner) unless its kill is
    /// counted (`+0x954`: its damage's kill ran slot 0x8b0); 172 and 153 either way.
    #[test]
    fn a_trebuchet_s_razing_counts_a_kill_unless_its_damage_counted_it() {
        let trebuchet = |hp| Victim {
            hp,
            class: 54,
            master_id: 42,
            building: Some(Building {
                trebuchet: true,
                cost: 400.0,
                ..Building::default()
            }),
            ..house()
        };
        let mut villager = hunter();
        villager.source.mode_680 = 1;
        // no damage at 1, the least takes it below 1 but its damage did not kill it
        let mut t = trebuchet(1.5);
        t.armored.as_mut().unwrap().armors = vec![(4, 10)];
        take(&mut t, &mut villager, &mut [], &[(4, 3)], 1.0, 0.0).unwrap();
        assert_eq!((t.hp, t.killed), (0.5, None));
        assert_eq!(
            t.building.as_ref().unwrap().razed[0].adds,
            vec![
                (1, 20, 1.0),
                (1, 170, 400.0),
                (2, 154, 1.0),
                (2, 152, 400.0),
                (1, 302, 1.0),
                (2, 326, 1.0),
                (1, 402, 400.0),
                (1, 172, 400.0),
                (2, 153, 400.0),
            ]
        );
        let mut t = trebuchet(2.0);
        take(&mut t, &mut villager, &mut [], &[(4, 3)], 1.0, 0.0).unwrap();
        assert!(t.killed.is_some());
        assert_eq!(
            t.building.as_ref().unwrap().razed[0].adds,
            vec![(1, 172, 400.0), (2, 153, 400.0)]
        );
    }

    /// With the flag, an attacker of type 50 and up leaves a building of 1 hit point or more
    /// (`COMISS hp, 1` / `JC` past) to its combat ability's effects (`FUN_14107bdf0`): only bit 7
    /// against a player not allied both ways does anything, which is not modelled. The kill
    /// bonus (`FUN_1415c6510`) of a razing: only an attacker of type 40 and up with a task of
    /// action type 0x9a does anything, which is not modelled.
    #[test]
    fn a_building_s_attacker_effects_and_kill_bonus_are_the_game_s() {
        let mut villager = hunter();
        villager.source.ability = 0x80;
        let mut h = house();
        assert!(take(&mut h, &mut villager, &mut [], &[(4, 3)], 1.0, 0.0).is_err());
        let mut h = Victim {
            allied: [true, true],
            ..house()
        };
        take(&mut h, &mut villager, &mut [], &[(4, 3)], 1.0, 0.0).unwrap();
        let mut h = Victim { hp: 2.0, ..house() };
        take(&mut h, &mut villager, &mut [], &[(4, 3)], 1.0, 0.0).unwrap();
        let mut villager = hunter();
        villager.source.kill_bonus_tasks = true;
        let mut h = Victim { hp: 2.0, ..house() };
        assert!(take(&mut h, &mut villager, &mut [], &[(4, 3)], 1.0, 0.0).is_err());
        let mut h = Victim {
            hp: 2.0,
            class: WALL,
            ..house()
        };
        take(&mut h, &mut villager, &mut [], &[(4, 3)], 1.0, 0.0).unwrap();
    }

    /// `FUN_14107f690` on buildings: every building on its tiles starts it with nothing taken
    /// (`+0xb90`); an annex reads its main building's `area_damage` (`+0x834`), blast level
    /// (`+0x62b`) and blast so far, which its loss adds to; a building's hit is less what the
    /// blast has done to it (`FUN_141082e80`'s `param_8`). Here the annex (area damage 0: a
    /// least hit of 1 on the main building) comes first in the walk; the main building's own hit
    /// of 5 then takes 4.
    #[test]
    fn a_blast_takes_a_main_building_less_what_its_annexes_took() {
        let mut mangonel = hunter();
        mangonel.source.area_effect = 2.0;
        mangonel.source.blast_level = 2;
        mangonel.source.area_damage = 0.5;
        mangonel.weapons = vec![(4, 10)];
        mangonel.position = [10.5, 10.5, 0.0];
        let center = mangonel.position;
        let mut main = Victim {
            id: 20,
            hp: 100.0,
            position: [11.5, 10.5, 0.0],
            radius: [1.0, 1.0],
            blast_defense: 2,
            building: Some(Building {
                annexes: vec![21],
                blast_taken: 7.0,
                ..Building::default()
            }),
            ..house()
        };
        main.source.area_damage = 0.0;
        let mut annex = Victim {
            id: 21,
            hp: 100.0,
            position: [10.5, 10.5, 0.0],
            radius: [0.5, 0.5],
            blast_defense: 0,
            building: Some(Building {
                main: Some(20),
                ..Building::default()
            }),
            ..house()
        };
        let mut all = [&mut main, &mut annex];
        assert_eq!(
            blast(&mut mangonel, center, None, None, &mut all, 1.0),
            Ok(true)
        );
        assert_eq!(all.map(|t| t.id), [21, 20]);
        assert_eq!((main.hp, annex.hp), (95.0, 95.0));
        assert_eq!(main.building.as_ref().unwrap().blast_taken, 5.0);
        // each hit posts its damage notify to the main building's unit AI: the annex's
        // forwarded one, then its own (426048706's gate 7228: 271, then -58)
        let hp_after: Vec<i32> = main.damaged.iter().map(|&(_, p)| p[1]).collect();
        assert_eq!(hp_after, [99, 95]);
        assert_eq!(annex.damaged, []);
    }

    /// `FUN_14107f690` walks the rows (y outer, x inner) and a tile's objects in its list's
    /// order (`+0x28`), not by id: the blast's targets end in that order.
    #[test]
    fn a_blast_walks_a_tile_s_objects_in_its_list_order() {
        let mut elephant = hunter();
        elephant.source.area_effect = 1.0;
        elephant.source.blast_level = 2;
        elephant.source.area_damage = 0.5;
        elephant.source.player = 1;
        elephant.weapons = vec![(4, 10)];
        let enemy = |id: i32, x: f32, y: f32, tile_place| Victim {
            id,
            tile_place,
            player: 2,
            position: [x, y, 0.0],
            blast_defense: 2,
            ..sheep()
        };
        let (mut a, mut b, mut c) = (
            enemy(3, 10.6, 11.0, 1),
            enemy(7, 10.8, 11.2, 0),
            enemy(5, 11.2, 10.4, 0),
        );
        let center = elephant.position;
        let mut all = [&mut a, &mut b, &mut c];
        assert_eq!(
            blast(&mut elephant, center, None, None, &mut all, 1.0),
            Ok(true)
        );
        assert_eq!(all.map(|t| t.id), [5, 7, 3]);
    }

    /// `FUN_141203be0`: hit points gain up to the master's plus the powerup room; the overflow
    /// comes off the powerup, never below 0; a powerup above its room first takes the room; at
    /// its most a gain only takes off the powerup.
    #[test]
    fn a_gain_fills_hit_points_and_its_overflow_takes_off_the_powerup() {
        let mut hp = 50.0;
        let mut ups = [0.0, 0.0];
        gain_hp(&mut hp, &mut ups, 60, 5.0);
        assert_eq!((hp, ups), (55.0, [0.0, 0.0]));
        gain_hp(&mut hp, &mut ups, 60, 10.0);
        assert_eq!((hp, ups), (60.0, [0.0, 0.0]));
        let mut hp = 65.0;
        let mut ups = [8.0, 10.0];
        gain_hp(&mut hp, &mut ups, 60, 10.0);
        assert_eq!((hp, ups), (70.0, [3.0, 10.0]));
        gain_hp(&mut hp, &mut ups, 60, 5.0);
        assert_eq!((hp, ups), (70.0, [0.0, 10.0]));
        let mut ups = [12.0, 10.0];
        gain_hp(&mut hp, &mut ups, 60, 0.0);
        assert_eq!(ups, [10.0, 10.0]);
    }

    /// `FUN_141193b30`: a loss leaves at least 1 hit point; full hit points (room less powerup
    /// plus hit points at least the most) become exactly the master's and answer true; a NaN is
    /// not full.
    #[test]
    fn a_heal_to_full_holds_the_master_s_hit_points() {
        let mut hp = 30.0;
        let mut ups = [0.0; 2];
        assert!(!heal_hp(&mut hp, &mut ups, 60, 10.0));
        assert_eq!(hp, 40.0);
        assert!(heal_hp(&mut hp, &mut ups, 60, 25.0));
        assert_eq!(hp, 60.0);
        let mut hp = 0.5;
        assert!(!heal_hp(&mut hp, &mut ups, 60, -3.0));
        assert_eq!(hp, 1.0);
        let mut hp = 55.0;
        let mut ups = [5.0, 5.0];
        assert!(!heal_hp(&mut hp, &mut ups, 60, 4.0));
        assert_eq!(hp, 59.0);
        let mut hp = f32::NAN;
        assert!(!heal_hp(&mut hp, &mut [0.0; 2], 60, 1.0));
    }
}
