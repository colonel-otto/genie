//! The indices of the player's attributes (`player+0x8`'s array of `f32`): the one definition of
//! the attributes the engine reads and writes by number. The first four are the resources
//! (food, wood, stone, gold). A master's attribute types are a different set
//! ([`crate::objects::masters::attribute`]).

use crate::GenieError;
use getset::CopyGetters;

/// A player's attribute number (`player+0x100`'s array, the `.dat`'s attribute types, an effect's
/// attribute argument): 0 or more. The first four are the resources (food, wood, stone, gold).
/// The simulation names the ones it tells apart (below); any other is a number the data holds
/// ([`AttributeId::numbered`]). A field that holds −1 for none is `Option<AttributeId>`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, CopyGetters)]
pub struct AttributeId(
    /// Its value, the attribute's place in the player's attributes.
    #[getset(get_copy = "pub")]
    i16,
);

impl AttributeId {
    /// An attribute the code names, in a constant: only this module makes them.
    const fn named(value: i16) -> Self {
        assert!(0 <= value, "a negative attribute number");
        Self(value)
    }

    /// An attribute the code names by its number, where the game's own number is the name (the
    /// attributes the engine reads once, and the tests').
    #[must_use]
    pub const fn numbered(value: i16) -> Self {
        Self::named(value)
    }

    /// Its place in a table by attribute number (the player's attributes).
    #[must_use]
    pub const fn index(self) -> usize {
        self.0.unsigned_abs() as usize
    }

    /// The attribute `by` places on: a table of attributes that run by player or by kind from this
    /// one (the trickle rates, the gathered totals).
    ///
    /// # Errors
    ///
    /// A number past the `i16` range.
    pub fn offset(self, by: usize) -> Result<Self, GenieError> {
        i16::try_from(by)
            .ok()
            .and_then(|by| self.0.checked_add(by))
            .map(Self)
            .ok_or_else(|| {
                GenieError::invalid_input("an attribute number past the table")
                    .with("attribute", self.0)
                    .with("offset", by)
            })
    }

    /// The attribute of a table place (a number of `usize`, as a command or a loop holds it).
    #[must_use]
    pub fn of_index(index: usize) -> Option<Self> {
        i16::try_from(index).ok().and_then(Self::of_data)
    }

    /// Whether it is one of the four resources (food, wood, stone, gold).
    #[must_use]
    pub const fn is_resource(self) -> bool {
        self.0 <= 3
    }

    /// The attribute a data field names, none for a negative value (−1 in the `.dat`).
    #[must_use]
    pub fn of_data(value: i16) -> Option<Self> {
        (0 <= value).then_some(Self(value))
    }

    /// The attribute of an entry that must name one (a storage or cost entry of −1 is a slot
    /// the game's payment and storage code does not take).
    ///
    /// # Errors
    ///
    /// A negative value: the slice does not model what the game does with a −1 entry.
    pub fn required(value: i16) -> Result<Self, GenieError> {
        Self::of_data(value)
            .ok_or_else(|| GenieError::unsupported("a negative attribute").with("value", value))
    }

    /// The value of a field that holds −1 for none.
    #[must_use]
    pub const fn field(attribute: Option<Self>) -> i16 {
        match attribute {
            Some(a) => a.0,
            None => -1,
        }
    }
}

impl std::fmt::Display for AttributeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl genie_diff::Diff for AttributeId {
    fn diff(&self, other: &Self, d: &mut genie_diff::Differ) {
        d.equal(self, other);
    }
}

impl TryFrom<i16> for AttributeId {
    type Error = GenieError;

    /// The attribute of value `value`.
    ///
    /// # Errors
    ///
    /// A negative value, which no attribute is.
    fn try_from(value: i16) -> Result<Self, GenieError> {
        Self::of_data(value)
            .ok_or_else(|| GenieError::unsupported("a negative attribute").with("attribute", value))
    }
}

/// Food: the first of the four resources (food, wood, stone, gold: 0 to 3).
pub const FOOD: AttributeId = AttributeId::named(0);
/// Wood.
pub const WOOD: AttributeId = AttributeId::named(1);
/// Stone.
pub const STONE: AttributeId = AttributeId::named(2);
/// Gold: a sale gives it and a purchase costs it, the treason costs it.
pub const GOLD: AttributeId = AttributeId::named(3);
/// The current age (`+0x30`, `player+0x100`), as the `i16` game mode 16's QUEUE, a new soldier's
/// unit AI (`FUN_140c12300`) and a command compare; the mode's bound is `DAT_14330eb8c`.
pub const CURRENT_AGE: AttributeId = AttributeId::named(6);
/// The population headroom (`+0x308` 4): a building's held amount gives it, and a cost check
/// skips it (`FUN_141618db0` with its fifth argument 1).
pub const HEADROOM: AttributeId = AttributeId::named(4);
/// The population.
pub const POPULATION: AttributeId = AttributeId::named(11);
/// The units owned.
pub const UNITS_OWNED: AttributeId = AttributeId::named(19);
/// The kills.
pub const KILLS: AttributeId = AttributeId::named(20);
/// The explored share of the map, in percent.
pub const EXPLORED: AttributeId = AttributeId::named(22);
/// The civilian population.
pub const CIVILIAN_POPULATION: AttributeId = AttributeId::named(37);
/// The military population.
pub const MILITARY_POPULATION: AttributeId = AttributeId::named(40);
/// The razings.
pub const RAZINGS: AttributeId = AttributeId::named(43);
/// The kill ratio: kills less losses.
pub const KILL_RATIO: AttributeId = AttributeId::named(44);
/// The survival to the finish, which a defeat clears.
pub const SURVIVAL: AttributeId = AttributeId::named(45);
/// The town centre is not available (0 lets a builder place one, `FUN_141625a20`).
pub const NO_TOWN_CENTRE: AttributeId = AttributeId::named(48);
/// The gold counter, which gold with a statistic grows (`FUN_141616820`).
pub const GOLD_COUNTER: AttributeId = AttributeId::named(49);
/// The research cost modifier, as the `i16` the cost code compares.
pub const RESEARCH_COST_MODIFIER: AttributeId = AttributeId::named(85);
/// The kills' value of the other players.
pub const KILLS_VALUE_OTHER: AttributeId = AttributeId::named(152);
/// The razings' value of the other players.
pub const RAZINGS_VALUE_OTHER: AttributeId = AttributeId::named(153);
/// The kills of the other players.
pub const KILLS_OTHER: AttributeId = AttributeId::named(154);
/// The razings of the other players.
pub const RAZINGS_OTHER: AttributeId = AttributeId::named(155);
/// The units' value.
pub const UNITS_VALUE: AttributeId = AttributeId::named(164);
/// The buildings' value.
pub const BUILDINGS_VALUE: AttributeId = AttributeId::named(165);
/// The first of the gathered totals of food, wood and stone (166 to 168).
pub const GATHERED: AttributeId = AttributeId::named(166);
/// The gathered total of gold.
pub const GOLD_GATHERED: AttributeId = AttributeId::named(169);
/// The kills' value.
pub const KILLS_VALUE: AttributeId = AttributeId::named(170);
/// The razings' value.
pub const RAZINGS_VALUE: AttributeId = AttributeId::named(172);
/// The town centres a player may have (`FUN_141625a20`).
pub const TOWN_CENTRE_LIMIT: AttributeId = AttributeId::named(218);
/// The first of the trickles a minute of food, wood, stone and gold (230 to 233).
pub const TRICKLES: AttributeId = AttributeId::named(230);
/// The food a minute of the livestock in the player's mills.
pub const MILL_FOOD: AttributeId = AttributeId::named(254);

/// A villager's death by a hunted animal (a prey or predator attacker).
pub const VILLAGER_KILLED_BY_ANIMAL: AttributeId = AttributeId::named(227);
/// A villager's death by a player of type 1.
pub const VILLAGER_KILLED_BY_TYPE_1: AttributeId = AttributeId::named(229);
/// A villager's death by a player of type 2.
pub const VILLAGER_KILLED_BY_TYPE_2: AttributeId = AttributeId::named(226);
/// A villager's death by a player of type 3.
pub const VILLAGER_KILLED_BY_TYPE_3: AttributeId = AttributeId::named(228);

#[cfg(test)]
mod tests {
    use super::{AttributeId, FOOD, GOLD, POPULATION};

    /// The four resources come first, a negative value is no attribute.
    #[test]
    fn the_resources_are_the_first_four_and_a_negative_value_is_none() {
        assert!(FOOD.is_resource() && GOLD.is_resource());
        assert!(!POPULATION.is_resource());
        assert_eq!(AttributeId::of_data(-1), None);
        assert_eq!(AttributeId::of_data(11), Some(POPULATION));
        assert_eq!(AttributeId::field(Some(GOLD)), 3);
        assert_eq!(AttributeId::field(None), -1);
        assert!(AttributeId::try_from(-2).is_err());
    }

    #[test]
    fn an_offset_past_the_range_is_an_error_not_a_panic() {
        assert_eq!(FOOD.offset(3).unwrap(), GOLD);
        assert!(AttributeId::numbered(i16::MAX).offset(1).is_err());
        assert!(FOOD.offset(usize::MAX).is_err());
    }
}
