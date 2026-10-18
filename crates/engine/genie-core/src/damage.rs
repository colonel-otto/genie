//! The damage side of a hit: what it reads of its target and its attacker and what it changes.
//! The target's slot 0xe8 takes the damage: a static object's (`FUN_14118efd0`), a combat
//! object's of type 50 or 60 (`FUN_141082e80`) and of type 70 (`FUN_1412108e0`, which adds the
//! special attack, the retaliation, the least damage and the reflected damage). The damage is the
//! armor's (slot 0xf0, `FUN_141082530`), less what a shield blocks (`FUN_14108d8a0`); a charge
//! (slots 0x2f0, 0x300, 0x318) adds to it, and a blast (slot 0x848, `FUN_14107f690`) spreads it.
//! When a melee hit lands is [`crate::attack`]'s.
//!
//! Of these, MarvinBlue/426048731 runs only the armor and the damage of a villager's hits on
//! sheep (update 1170). The charges, shields, blasts, terrain bonuses, special attacks,
//! retaliations and reflections are ports of the code, checked by the unit tests alone.

use crate::fmath;
use crate::unit_ai::Unsupported;

/// Elevation: a higher attacker hits at 1.25 (`DAT_142eb56a8`), a lower one at 0.75
/// (`DAT_142e7f388`), each plus its player's and the target's player's attributes (211/272,
/// 212/273).
const HIGHER: f32 = crate::image::damage::HIGHER;
const LOWER: f32 = crate::image::damage::LOWER;
/// The gap below which the game takes a distance as zero (`DAT_142eb0c60`), and the share of
/// reflected damage above which a target reflects.
const EPSILON: f32 = crate::image::damage::EPSILON;
/// π, 2π and their negatives, which a facing wraps by (`FUN_14114eaa0`).
const PI: f32 = crate::image::damage::PI;
const MINUS_PI: f32 = crate::image::damage::MINUS_PI;
const TWO_PI: f32 = crate::image::damage::TWO_PI;
const MINUS_TWO_PI: f32 = crate::image::damage::MINUS_TWO_PI;
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

/// An object as a hit reads and changes it: the target, and the attacker as the target of a
/// retaliation or of reflected damage.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Victim {
    /// The object.
    pub id: i32,
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
    /// Its master's `area_effect_object_level` (`+0x62b`): the blast levels that reach it.
    pub blast_defense: u8,
    /// Flagged to sleep (`+0x641`, `+0x643`): a killing hit wakes it (`FUN_14119deb0` with 0).
    pub goto_sleep: bool,
    /// Its master's decay (`+0x620`), which a hunt's decay stop reads (`FUN_140ac44b0`).
    pub decay: f32,
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
    /// Its own attack (a retaliation's): its values as a hit's source and its master's weapons
    /// (class, amount).
    pub source: Attacker,
    /// Its master's weapons (class, amount).
    pub weapons: Vec<(i16, i16)>,
    /// A combat object's armor and what its damage reads (types 50 and up); None for others.
    pub armored: Option<Armored>,
    /// The damage notify (500, `FUN_141082e80`) a hit posts to its unit AI: the attacker, its hit
    /// points after (truncated) and its master's hit points.
    pub damaged: Option<(i32, [i32; 3])>,
    /// A hit killed it, for the caller's death handling.
    pub killed: Option<Kill>,
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
    /// Its master's hit points.
    pub master_hp: i16,
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
    /// Its attack powerup (`+0xa24`), the game's setting `+0x680`
    /// ([`crate::turn::Turns::mode_680`]: at 1 a hit may do no damage), and whether its master
    /// has a task of action type 0xa0 (bonus damage, `FUN_14107ce60`).
    pub attack_powerup: f32,
    /// The game's setting `+0x680` ([`crate::turn::Turns::mode_680`]).
    pub mode_680: i32,
    /// Its master has a task of action type 0xa0 (bonus damage).
    pub bonus_tasks: bool,
    /// Its player and whether its master has a task of action type 0x9a (a kill bonus,
    /// `FUN_1415c6510`).
    pub player: usize,
    /// Its master has a task of action type 0x9a (a kill bonus).
    pub kill_bonus_tasks: bool,
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

/// `FUN_14114eaa0` toward another object: how far `other` lies from where an object at `at`
/// faces, in facets (at least 8): the angle to it (`atan2f`, 0 to 2π) less the facing angle,
/// wrapped into −π to π, times the facets over 2π; 0 on the same point or without facets.
#[must_use]
pub fn facing_offset(at: [f32; 3], facing: Facing, other: [f32; 3]) -> f32 {
    if facing.facets <= 0 {
        return 0.0;
    }
    let n = if 7 < facing.facets { facing.facets } else { 8 };
    let dx = other[0] - at[0];
    let dy = other[1] - at[1];
    if !(EPSILON < dx.abs() || EPSILON < dy.abs()) {
        return 0.0;
    }
    let mut a = fmath::atan2(dy, dx);
    if a < 0.0 {
        a += TWO_PI;
    }
    a -= facing.angle;
    if PI < a {
        a += MINUS_TWO_PI;
    }
    if a < MINUS_PI {
        a += TWO_PI;
    }
    (f32::from(n) / TWO_PI) * a
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
const fn special_charge_ready(a: &Attacker) -> Result<bool, Unsupported> {
    if a.special_attack == 3 {
        return Err(Unsupported("a charge of special attack 3 (FUN_1412161b0)"));
    }
    Ok(false)
}

/// `FUN_141202580`: the damage a ready charge adds, the charge itself; a master of special
/// attack 3 adds it only while [`special_charge_ready`] holds.
fn charge_damage(a: &Attacker) -> Result<f32, Unsupported> {
    if a.special_attack == 3 && !special_charge_ready(a)? {
        return Ok(0.0);
    }
    Ok(a.charge)
}

/// `FUN_14108d8a0`: a shield blocks the hit: a target whose master's charge type is 5 against a
/// melee hit (4 against a missile) spends a charge of 1 when it has one. The type-5 shield's
/// statistics (`FUN_140a497f0`) are not held. A target without combat values has charge type 0.
pub fn shield(v: &mut Victim, missile: bool) -> bool {
    let kind = if missile { 4 } else { 5 };
    if v.source.charge_type == kind && 1.0 <= v.source.charge {
        v.source.charge -= 1.0;
        return true;
    }
    false
}

/// `FUN_141202a80`: the hit points the master's `invulnerability_level` keeps (a share of its hit
/// points when above 0, the level itself when below), 0 for none.
fn hp_floor(a: &Armored) -> f32 {
    let level = a.invulnerability;
    if level == 0.0 {
        0.0
    } else if 0.0 < level {
        f32::from(a.master_hp) * level
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
    let floor = hp_floor(a);
    let left = t.hp - d;
    t.hp = if floor == 0.0 || floor <= left {
        left
    } else {
        floor
    };
}

/// The armor a combat object (type 70 and up) has against melee (class 4; slot 0x868,
/// `FUN_141085450` and `FUN_141213810`) or pierce (class 3; slot 0x870, `FUN_1410857f0`): its
/// best armor of the class, else its best of any (melee) or its base armor and best of any
/// (pierce), plus its powerup (the stingers `+0xb4c`, `+0xb50`, unsaved and written nowhere in
/// this recording, are 0), less the armor removed. A wall foundation (class 27 in state 0) has
/// no melee armor.
fn class_armor(a: &Armored, class: i16, own_class: i16, state: u8) -> Result<i16, Unsupported> {
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
        4 => {
            let mut armor = best(4, i16::MIN);
            if armor == i16::MIN {
                armor = any(i16::MIN);
                if armor == i16::MIN {
                    armor = a.base_armor;
                }
            }
            armor = armor.wrapping_add(a.armor_powerups[0]);
            if own_class == 27 && state == 0 {
                armor = 0;
            }
            Ok(armor.wrapping_sub(a.armor_removed[0]))
        }
        3 => {
            if own_class == 27 && state == 0 {
                return Err(Unsupported("the pierce armor of a wall foundation"));
            }
            let mut armor = best(3, -1);
            if armor == -1 {
                armor = any(a.base_armor);
            }
            Ok(armor
                .wrapping_add(a.armor_powerups[1])
                .wrapping_sub(a.armor_removed[1]))
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
/// the sum, at least 1 (0 under the game's setting `+0x680` at 1). Not modelled: the bonus damage
/// tasks (action type 0xa0, `FUN_14107ce60`).
pub fn armor_damage(
    v: &Victim,
    weapons: &[(i16, i16)],
    factor: f32,
    attacker: &Attacker,
) -> Result<f32, Unsupported> {
    let a = v
        .armored
        .as_ref()
        .ok_or(Unsupported("the armor of an object without combat values"))?;
    let (own_class, kind, state) = (v.class, v.kind, v.state);
    let terrain = if a.terrain_bonus < 0 {
        1.0f32
    } else {
        a.terrain.ok_or(Unsupported(
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
                armor = class_armor(a, k, own_class, state)?;
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
                if attacker.bonus_tasks {
                    return Err(Unsupported(
                        "an attacker with bonus damage tasks (FUN_14107ce60)",
                    ));
                }
                let bonus = 0.0f32 + attacker.attack_powerup;
                attack += 0.0 + bonus;
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
/// already done to it (`blast_taken`, a building's `+0xb90`): a static object
/// (`FUN_14118efd0`), a combat object of type 50 or 60 ([`damage`]) or of type 70
/// ([`take_unit`]). A building's (`FUN_1411cea90`: its link, its least damage, its kill
/// statistics) and an object of types 30 to 49 are not modelled.
pub fn take(
    t: &mut Victim,
    src: &mut Victim,
    weapons: &[(i16, i16)],
    factor: f32,
    blast_taken: f32,
) -> Result<(), Unsupported> {
    match t.kind {
        70 => take_unit(t, src, weapons, factor, blast_taken),
        50 | 60 => damage(t, src, weapons, factor, blast_taken),
        80 => Err(Unsupported(
            "a hit on a building (slot 0xe8, FUN_1411cea90)",
        )),
        30..=49 => Err(Unsupported(
            "a hit on a moving object without combat values",
        )),
        _ => {
            take_static(t, &src.source, weapons, factor);
            Ok(())
        }
    }
}

/// A static object's slot 0xe8 (`FUN_14118efd0`): it has no armor (slot 0xf0 `FUN_14118cab0`:
/// the weapons' sum times the factor) and loses the damage (slot 0xb0); below 1 hit point it has
/// 0 and wakes, and but for a villager's hit it loses what it holds. Its damage sprites (slot
/// 0x4c0) are not held.
fn take_static(t: &mut Victim, src: &Attacker, weapons: &[(i16, i16)], factor: f32) {
    let mut damage = 0.0f32;
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
    weapons: &[(i16, i16)],
    factor: f32,
    blast_taken: f32,
) -> Result<(), Unsupported> {
    let before = effective(t);
    if t.hp < 1.0 {
        return Ok(());
    }
    let a = t
        .armored
        .clone()
        .ok_or(Unsupported("a combat object without its combat values"))?;
    let mut factor = factor;
    let facing_to = |at: [f32; 3], src: &Victim| -> Result<f32, Unsupported> {
        let facing = a.facing.ok_or(Unsupported(
            "the facing of a target with a special attack or a retaliation (+0x7e8)",
        ))?;
        Ok(facing_offset(at, facing, src.position))
    };
    if a.special_attack == 1 && !a.busy {
        let off = facing_to(t.position, src)?;
        if MINUS_FACING < off && off < FACING {
            factor *= SPECIAL;
            if a.special_graphic {
                return Err(Unsupported("a special attack's sprite (master +0x8e8)"));
            }
        }
    }
    let reach = src.source.weapon_range;
    let melee = reach <= RETALIATION_RANGE && RETALIATION_RANGE != reach;
    // the conditions are pure: the ones the slice holds decide first
    if a.special_attack == 2 && a.unit_ai && !a.busy && melee {
        let idle = a.alert_idle.ok_or(Unsupported(
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
                take(src, t, &w, f, 0.0)?;
                if src.hp <= 1.0 {
                    factor = 0.0;
                }
            }
        }
    }
    damage(t, src, weapons, factor, blast_taken)?;
    // a hit takes at least one hit point
    let mut lost = before - effective(t);
    let floor = hp_floor(&a);
    if lost < 1.0 && (floor == 0.0 || floor + 1.0 <= before) {
        t.hp = before - 1.0;
        lost = 1.0;
    }
    if EPSILON < a.reflection {
        let d = a.reflection * lost;
        lose(src, if 1.0 <= d { d } else { 1.0 });
    }
    if t.killed.is_some() && src.source.kill_bonus_tasks {
        return Err(Unsupported(
            "an attacker's kill bonus task (action type 0x9a)",
        ));
    }
    if t.killed.is_none() && 50 <= src.kind && src.source.ability & 0x80 != 0 {
        return Err(Unsupported(
            "an attacker whose hits apply effects (FUN_14107bdf0)",
        ));
    }
    Ok(())
}

/// `FUN_141082e80`: the damage ([`armor_damage`]) beyond what a blast has done: an attacker of
/// type 70 and up records whether the hit is its charge's (slot 0x2f0, `+0x8a8`), which empties
/// a charge without an area effect (slot 0x318 with 0); the target loses it (slot 0xb0). The
/// target's unit AI gets notify 500 (but from an attacker of class 64) with the attacker, the hit
/// points left and its master's. Below 1 hit point it has 0 and is killed: Gaia takes a player's
/// livestock (type 70). Not modelled: an attacker of type 60 and up whose combat ability marks its
/// target (bit 2, `FUN_141206250`), a missile's shooter as the notify's sender (`+0x998`) and a
/// kill reward task (action type 0xa1). Not held: the damaged flag (`+0x683`), the last damage
/// time (`+0x648`), the alerts of another owner (slot 0x2e8), the killer (`+0x690`, `+0x694`),
/// the local player's statistics, the damage sprites (slot 0x4c0) and the trigger events.
fn damage(
    t: &mut Victim,
    src: &mut Victim,
    weapons: &[(i16, i16)],
    factor: f32,
    blast_taken: f32,
) -> Result<(), Unsupported> {
    let d = armor_damage(t, weapons, factor, &src.source)?;
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
        return Err(Unsupported(
            "an attacker whose combat ability marks its target (FUN_141206250)",
        ));
    }
    if src.kind == 60 {
        return Err(Unsupported("a missile's damage (its shooter, +0x998)"));
    }
    let master_hp = t.armored.as_ref().map_or(0, |a| i32::from(a.master_hp));
    if src.source.class != ELEPHANT_ARCHER {
        t.damaged = Some((src.id, [src.id, t.hp as i32, master_hp]));
    }
    if t.hp < 1.0 {
        t.hp = 0.0;
        if t.armored.as_ref().is_some_and(|a| a.kill_tasks) {
            return Err(Unsupported("a kill reward task (action type 0xa1)"));
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
/// which no object passes).
fn blast_tiles(center: [f32; 3], r: f32) -> ([i32; 2], [i32; 2]) {
    let low = |c: f32| {
        let t = ((c - r) as i32 as i16).wrapping_sub(6);
        i32::from(t.max(0))
    };
    let high = |c: f32| i32::from(((c + r) as i32 as i16).wrapping_add(6));
    (
        [low(center[0]), low(center[1])],
        [high(center[0]), high(center[1])],
    )
}

/// Slot 0x848 (`FUN_14107f690`), the blast of `attacker` about `center` with factor `factor`,
/// over the objects the tiles of [`blast_tiles`] hold, row by row (the objects of a tile by id:
/// the slice does not hold the tile's list order): the attacker's hit is first marked as no
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
/// (slot 0x2f0). A building's blast so far (`+0xb90`) and link (`+0xbb8`) are not modelled
/// (buildings are not).
pub fn blast(
    attacker: &mut Victim,
    center: [f32; 3],
    exclude: Option<i32>,
    main: Option<i32>,
    targets: &mut [&mut Victim],
    factor: f32,
) -> Result<(), Unsupported> {
    let a = attacker.source;
    let r = a.area_effect;
    if r <= 0.0 {
        return Ok(());
    }
    let wide = if a.blast_level & 0x80 != 0 { r } else { 0.0 };
    let (from, to) = blast_tiles(center, r);
    let spares_friends = !(1.0 < a.weapon_range && a.blast_level & 4 == 0);
    attacker.source.charge_hit = false;
    targets.sort_by_key(|t| (t.position[1] as i32, t.position[0] as i32, t.id));
    let weapons = attacker.weapons.clone();
    for t in targets.iter_mut() {
        let tile = [t.position[0] as i32, t.position[1] as i32];
        if !(from[0] <= tile[0] && tile[0] < to[0] && from[1] <= tile[1] && tile[1] < to[1]) {
            continue;
        }
        if t.uncertain {
            return Err(Unsupported(
                "a blast over an object whose player's update is not modelled",
            ));
        }
        if (spares_friends && t.allied[0])
            || Some(t.id) == exclude
            || t.id == attacker.id
            || t.class == ARTIFACT
        {
            continue;
        }
        let is_main = Some(t.id) == main;
        let defense = t.blast_defense;
        if !(1 < defense || a.blast_level <= defense) {
            continue;
        }
        if RAMS.contains(&a.master_id) && t.kind != 80 && !is_main {
            continue;
        }
        let gap = |d: f32, radius: f32| if d <= radius { 0.0 } else { d - radius };
        // the hit's slack (its info's +0xc, +0x10) is 0 for a melee hit
        let dx = gap((t.position[0] - center[0]).abs(), t.radius[0]);
        let dy = gap((t.position[1] - center[1]).abs(), t.radius[1]);
        let d2 = dy * dy + dx * dx;
        if r * r <= d2 {
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
            let d = if !is_main || t.kind != 80 {
                fmath::sqrt(d2)
            } else {
                0.0
            };
            f *= (r - d) / r;
        }
        if !is_main {
            if a.blast_level & 0x80 != 0 {
                let facing =
                    attacker
                        .armored
                        .as_ref()
                        .and_then(|x| x.facing)
                        .ok_or(Unsupported(
                            "the facing of an attacker with a cone blast (+0x7e8)",
                        ))?;
                if a.blast_level & 0x20 == 0 {
                    let off = facing_offset(attacker.position, facing, t.position);
                    if !(MINUS_CONE..=CONE).contains(&off) {
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
            if a.area_damage < 0.0 {
                let before = t.hp;
                lose(t, a.area_damage.abs());
                if t.hp < 1.0 {
                    t.hp = 0.0;
                    t.amount = 0.0;
                    if f32::from(t.capacity) < t.amount {
                        t.amount = f32::from(t.capacity);
                    }
                    if 1.0 <= before && 0 < t.player && t.kind == 70 {
                        if a.kill_bonus_tasks {
                            return Err(Unsupported(
                                "an attacker's kill bonus task (action type 0x9a)",
                            ));
                        }
                        t.killed = Some(Kill {
                            attacker: attacker.id,
                            attacker_player: attacker.player,
                            to_gaia: false,
                        });
                    }
                }
                continue;
            }
            f *= a.area_damage;
        }
        take(t, attacker, &weapons, f * factor, 0.0)?;
        if !attacker.source.charge_hit {
            attacker.source.charge_hit = charge_hit(&attacker.source, t);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
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
            armored: Some(Armored {
                armors: vec![(4, 0), (3, 0), (31, 0)],
                terrain_bonus: -1,
                master_hp: 14,
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
            armored: Some(Armored {
                armors: vec![(4, 0), (3, 0)],
                terrain_bonus: -1,
                master_hp: 25,
                friendly_fire: 1.0,
                ..Armored::default()
            }),
            ..Victim::default()
        }
    }

    /// MarvinBlue/426048731's update 1170: the hunter 10029 (master 592: classes 11, 4, 13 and
    /// 17 at 3, 3, 6 and 0) strikes its sheep 9520 (master 705: armor 0 for 4, 3 and 31, base 0):
    /// 3 + 3 + 6 = 12, the game's 14 -> 2 hit points.
    #[test]
    fn a_hunter_strikes_its_sheep_for_twelve() {
        let (mut sheep, mut hunter) = (sheep(), hunter());
        let weapons = hunter.weapons.clone();
        assert_eq!(
            armor_damage(&sheep, &weapons, 1.0, &hunter.source),
            Ok(12.0)
        );
        take(&mut sheep, &mut hunter, &weapons, 1.0, 0.0).unwrap();
        assert_eq!(sheep.hp, 2.0);
        assert_eq!(sheep.damaged, Some((10029, [10029, 2, 14])));
        // the next hit kills it, and Gaia takes a player's livestock
        take(&mut sheep, &mut hunter, &weapons, 1.0, 0.0).unwrap();
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
            armor_damage(&armored, &[(4, 3)], 1.0, &hunter.source),
            Ok(1.0)
        );
        let zero = Attacker {
            mode_680: 1,
            ..hunter.source
        };
        assert_eq!(armor_damage(&armored, &[(4, 3)], 1.0, &zero), Ok(0.0));
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
            armor_damage(&sheep, &[(4, 4)], 1.0, &hunter.source),
            Ok(6.0)
        );
        sheep.armored.as_mut().unwrap().terrain = None;
        assert!(armor_damage(&sheep, &[(4, 4)], 1.0, &hunter.source).is_err());
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
            armor_damage(&sheep, &[(4, 3), (3, 2)], 1.0, &hunter.source),
            Ok(5.0 + 3.0 + 2.0)
        );
        take(&mut sheep, &mut hunter, &[(4, 3)], 1.0, 0.0).unwrap();
        assert_eq!(sheep.hp, 14.0 - 8.0);
        assert!(hunter.source.charge_hit);
        assert_eq!(hunter.source.charge, 0.0);
        // not ready: no charge
        hunter.source.charge = 4.0;
        assert_eq!(
            armor_damage(&sheep, &[(4, 3)], 1.0, &hunter.source),
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
        take(&mut sheep, &mut hunter, &[(4, 10)], 1.0, 0.0).unwrap();
        assert_eq!(sheep.hp, 14.0 - 10.0 * SPECIAL);
        // a busy target does not, and an unknown facing is not modelled
        let mut busy = self::sheep();
        busy.armored.as_mut().unwrap().special_attack = 1;
        busy.armored.as_mut().unwrap().busy = true;
        take(&mut busy, &mut hunter, &[(4, 10)], 1.0, 0.0).unwrap();
        assert_eq!(busy.hp, 4.0);
        let mut unknown = self::sheep();
        unknown.armored.as_mut().unwrap().special_attack = 1;
        assert!(take(&mut unknown, &mut hunter, &[(4, 10)], 1.0, 0.0).is_err());
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
        take(&mut sheep, &mut hunter, &[(4, 3)], 1.0, 0.0).unwrap();
        assert_eq!((hunter.hp, sheep.hp), (20.0, 11.0));
        assert_eq!(hunter.damaged, Some((9520, [9520, 20, 25])));
        hunter.hp = 5.0;
        take(&mut sheep, &mut hunter, &[(4, 3)], 1.0, 0.0).unwrap();
        // the hunter dies; the sheep loses the least, one hit point
        assert_eq!((hunter.hp, sheep.hp), (0.0, 10.0));
        assert!(hunter.killed.is_some());
    }

    /// A target that reflects gives its attacker that share of its loss, at least 1.
    #[test]
    fn reflected_damage_comes_back_at_least_one() {
        let (mut sheep, mut hunter) = (sheep(), hunter());
        sheep.armored.as_mut().unwrap().reflection = 0.5;
        take(&mut sheep, &mut hunter, &[(4, 6)], 1.0, 0.0).unwrap();
        assert_eq!((sheep.hp, hunter.hp), (8.0, 22.0));
        take(&mut sheep, &mut hunter, &[(4, 1)], 1.0, 0.0).unwrap();
        assert_eq!((sheep.hp, hunter.hp), (7.0, 21.0));
        assert!(hunter.damaged.is_none() && hunter.killed.is_none());
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
        blast(&mut elephant, center, Some(1), None, &mut all, 1.0).unwrap();
        assert_eq!(
            (target.hp, near.hp, far.hp, friend.hp),
            (14.0, 9.0, 14.0, 14.0)
        );
        // the falloff: 0.3 of radius 1 away keeps 0.7
        elephant.source.blast_level = 0x42;
        let mut near = enemy(2, 10.8);
        let mut all = [&mut near];
        blast(&mut elephant, center, None, None, &mut all, 1.0).unwrap();
        let dx = (10.8f32 - 10.5f32).abs();
        let f = (1.0 - fmath::sqrt(0.0 * 0.0 + dx * dx)) / 1.0 * 0.5;
        assert_eq!(near.hp, 14.0 - 10.0 * f);
    }
}
