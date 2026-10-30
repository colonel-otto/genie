//! An object's id as one type: the id that names an object across ticks, as the commands,
//! their unit lists and their targets hold it.

use crate::GenieError;

/// An object's id (`+0x8`), which names the object across ticks: a live or sleeping object's
/// is 0 or more, a recyclable master's object (a missile, a ghost) has a negative one. Every
/// field that names an object takes −1 for none, so no id is −1: such a field holds
/// `Option<ObjectId>` ([`ObjectId::of_field`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectId(i32);

impl ObjectId {
    /// Its value, as the object and the fields that name it hold it.
    #[must_use]
    pub const fn get(self) -> i32 {
        self.0
    }

    /// The id the world gives the next new object (`mNextObjectId` counts up).
    #[must_use]
    pub const fn successor(self) -> Self {
        Self(self.0 + 1)
    }

    /// The id a new recyclable object takes after this one (`next_reusable_object_id` counts
    /// down).
    #[must_use]
    pub const fn predecessor(self) -> Self {
        Self(self.0 - 1)
    }

    /// The stand-in for a saved object without a static part, which the slice does not model:
    /// no object's id, and not [`ObjectId::TAKEN_OUT`] (it stays in its list).
    pub const UNSAVED: Self = Self(i32::MIN + 1);

    /// The stand-in a list leaves in the slot of a unit taken out of it
    /// ([`crate::objects::unit::TAKEN_OUT`]): no object's id.
    pub(crate) const TAKEN_OUT: Self = Self(i32::MIN);

    /// The value every field takes for none.
    const NONE: i32 = -1;

    /// The id of a raw field that holds −1 for none.
    #[must_use]
    pub const fn of_field(value: i32) -> Option<Self> {
        if value == Self::NONE {
            None
        } else {
            Some(Self(value))
        }
    }

    /// The value of a field that holds −1 for none.
    #[must_use]
    pub fn field(id: Option<Self>) -> i32 {
        id.map_or(Self::NONE, |id| id.0)
    }

    /// The values of fields that hold −1 for none, for a routine that takes them raw
    /// (FD-001-D23's migration reaches it later).
    #[must_use]
    pub fn fields(ids: &[Option<Self>]) -> Vec<i32> {
        ids.iter().map(|&id| Self::field(id)).collect()
    }
}

impl genie_diff::Diff for ObjectId {
    fn diff(&self, other: &Self, d: &mut genie_diff::Differ) {
        d.equal(self, other);
    }
}

impl std::fmt::Display for ObjectId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl TryFrom<i32> for ObjectId {
    type Error = GenieError;

    /// The id of value `value`.
    ///
    /// # Errors
    ///
    /// −1, which every field takes for none.
    fn try_from(value: i32) -> Result<Self, GenieError> {
        Self::of_field(value).ok_or_else(|| {
            GenieError::invalid_input("an object id of −1, which names none").with("id", value)
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::ErrorKind;
    use crate::objects::ObjectId;

    /// An id of `value`, which the test knows is one.
    fn id(value: i32) -> ObjectId {
        ObjectId::try_from(value).unwrap()
    }

    /// A field's −1 is none, any other value an id, and each converts back to its value.
    #[test]
    fn a_field_s_minus_one_is_none_and_any_other_value_an_id() {
        assert_eq!(ObjectId::of_field(-1), None);
        for value in [0, 5, -2, i32::MIN, i32::MAX] {
            assert_eq!(ObjectId::of_field(value).map(ObjectId::get), Some(value));
            assert_eq!(ObjectId::field(ObjectId::of_field(value)), value);
        }
        assert_eq!(ObjectId::field(None), -1);
    }

    /// −1 is no id; a negative one other than it is (a recyclable master's object).
    #[test]
    fn minus_one_is_no_id() {
        assert_eq!(
            ObjectId::try_from(-1).map_err(|e| e.kind()),
            Err(ErrorKind::InvalidInput)
        );
        assert_eq!(id(-2).get(), -2);
        assert_eq!(id(7).get(), 7);
    }

    /// A list's values are its ids', none's −1, in its order.
    #[test]
    fn a_list_s_values_are_its_ids_in_order() {
        assert_eq!(
            ObjectId::fields(&[Some(id(7)), None, Some(id(-2))]),
            [7, -1, -2]
        );
    }
}
