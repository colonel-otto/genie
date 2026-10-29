//! A research's completion and the effects it gives: `FUN_1413cdcd0` (the technology done, its
//! effect, the technologies that depend on it, the player's statistics), the effects'
//! `TRIBE_Effects::do_effect` (`FUN_141462440`, then the base `FUN_140ec2ac0`) with every kind
//! for every party, and what they reach: the player's attribute modifier (slots 0x150, 0x158,
//! 0x160 -> slot 0x148, `FUN_1416121b0`: the hooks `FUN_141614f40` around `FUN_14165a000`, the
//! objects' slots 0x78, 0x80, 0x88 and the masters' 0x18, 0x20, 0x28 of
//! [`crate::objects::masters`]), the resource commands (slots 0x1b0, 0x1b8, the player's attribute set
//! and add), the technology changes (`'e'`, `'f'`, `'g'`, kind 8), the enables, upgrades,
//! spawns and cost entries. The objects' parts an upgrade, a spawn and the objects' own masters
//! need are the objects' routines, which this file leaves to them.

mod attribute_modifier;
mod commands;
mod completion;
mod object_slots;
mod resources;
mod tech_change;
mod upgrade;

pub(crate) use self::attribute_modifier::{MasterSprites, follow_master};
use self::object_slots::{CAPPED, ONE, ROUND, has_actions, shows_sprite, within_table};
#[cfg(test)]
use self::object_slots::{POPULATION, RESOURCE_19, charge_after};
#[cfg(test)]
use crate::math::rng::Stream;
#[cfg(test)]
use crate::technology::EffectCommand;
#[cfg(test)]
use crate::world::World;

/// The attribute counting the technologies researched (`0x15`) and the one summing their costs
/// (`0x63`), which a research done at a location with a button adds to.
/// A tech effect finds an object it changes gone from the lists it was found in.
const OBJECT_GONE: crate::GenieError =
    crate::GenieError::unreachable("an object a tech effect changes gone from its list");

const TECHS_RESEARCHED: i16 = 0x15;
const RESEARCH_SPENT: i16 = 0x63;
/// The attribute the dependents' update sets to 1 when none of them is left (`+0x138`).
const ATTRIBUTE_39: usize = 39;
/// The master whose enabling first asks the game's mode (`0x26d`, `FUN_1416665b0`), and the
/// mode that never enables it (11).
const MODE_TESTED_MASTER: i16 = 0x26d;
const MODE_WITHOUT_0X26D: i32 = 11;
/// The `TRIBE_Effects` command kinds that change technologies (`'e'`, `'f'`, `'g'`): a cost, the
/// disabling and a research time.
const TECH_COST: u8 = b'e';
const TECH_DISABLE: u8 = b'f';
const TECH_TIME: u8 = b'g';
/// The technology type that a disable command makes available instead (`FUN_1413cdb00`: 0x21).
const REENABLED_KIND: i16 = 0x21;

/// The `RGE_Effects` command kinds the base (`FUN_140ec2ac0`) switches on: the player's own (0 to
/// 8), the same for its mutual allies (10 to 18), enemies (20 to 28), neutrals (30 to 38) and Gaia
/// (40 to 48), and the objects' (200 to 206).
mod kind {
    pub const SET: u8 = 0;
    pub const RESOURCE: u8 = 1;
    pub const ENABLE: u8 = 2;
    pub const UPGRADE: u8 = 3;
    pub const ADD: u8 = 4;
    pub const MULTIPLY: u8 = 5;
    pub const RESOURCE_MULTIPLY: u8 = 6;
    pub const SPAWN: u8 = 7;
    pub const TECH: u8 = 8;
    /// A set, add and multiply on the objects with masters of their own of a master or class
    /// (200 to 202), on the effect's object (203 to 205), and its master's change (206).
    pub const OBJECTS_SET: u8 = 200;
    pub const OBJECT_SET: u8 = 203;
    pub const OBJECT_UPGRADE: u8 = 206;
}

/// The cost attributes a multiply by a negative amount turns into a cost entry's removal or
/// addition (`FUN_1415c1f70`): 103 to 106.
const COST_ATTRIBUTES: std::ops::RangeInclusive<i16> = 103..=106;
/// The resource whose change (kind 1) grows the food the player's farms hold after the effect
/// (`0x24`, `FUN_141615270`).
const FARM_FOOD: i16 = 0x24;

/// How slot 0x148 changes an attribute (its second argument): set (0, slot 0x150), add (1, slot
/// 0x158) or multiply (2, slot 0x160); the masters' slots 0x18, 0x20, 0x28 and the objects' slots
/// 0x78, 0x80, 0x88 follow it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// The attribute takes the amount.
    Set,
    /// The amount adds to it.
    Add,
    /// It is multiplied by the amount.
    Multiply,
}

#[cfg(test)]
mod tests;
