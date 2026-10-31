//! How a player stands to another as one type: `relation[i]` (`player+0x260`).

/// How a player stands to player i (`relation[i]`, `player+0x260`, which `FUN_141676b80`
/// writes from a stance, `FUN_1416397d0`): an ally, a neutral or an enemy. Every byte is a
/// relation (the save holds what the scenario set).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Relation(u8);

impl Relation {
    /// An ally (0, the stances 4 and 5): also a player's relation to itself.
    pub const ALLY: Self = Self(0);
    /// A neutral (1, the stance 3).
    pub const NEUTRAL: Self = Self(1);
    /// An enemy (3, the stances 1 and 2).
    pub const ENEMY: Self = Self(3);
    /// The relation the stances 1 and 2 make while `FUN_1415e8e10` holds (the world's `+0x820`
    /// object in its state 1): the queries take it for an ally's, the task finder only with
    /// its ally flag.
    pub const RELATION_4: Self = Self(4);

    /// Its value.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }

    /// Whether the relation counts as allied (`FUN_141663dc0`): an ally always, relation 4 with
    /// the ally flag `with_4`.
    #[must_use]
    pub const fn allied(self, with_4: bool) -> bool {
        self.0 == Self::ALLY.0 || (with_4 && self.0 == Self::RELATION_4.0)
    }
}

/// A test's relation row (`player+0x260`) of saved bytes.
#[cfg(test)]
pub(crate) fn relation_row(values: impl IntoIterator<Item = u8>) -> Vec<Relation> {
    values.into_iter().map(Relation::from).collect()
}

impl From<u8> for Relation {
    /// The relation a saved byte holds: every value is one.
    fn from(value: u8) -> Self {
        Self(value)
    }
}

impl genie_diff::Diff for Relation {
    fn diff(&self, other: &Self, d: &mut genie_diff::Differ) {
        d.equal(self, other);
    }
}

#[cfg(test)]
mod tests {
    use crate::players::Relation;

    /// An ally counts as allied; relation 4 only with the flag; a neutral or an enemy never.
    #[test]
    fn an_ally_and_relation_4_with_the_flag_count_as_allied() {
        assert!(Relation::ALLY.allied(false));
        assert!(!Relation::RELATION_4.allied(false));
        assert!(Relation::RELATION_4.allied(true));
        assert!(!Relation::NEUTRAL.allied(true));
        assert!(!Relation::ENEMY.allied(true));
        assert_eq!(Relation::from(3), Relation::ENEMY);
        assert_eq!(Relation::default().get(), 0);
    }
}
