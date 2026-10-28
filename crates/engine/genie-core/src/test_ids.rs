//! The tests' newtypes: an object id, a player or a master of a value the test knows is one,
//! and a list of ids, through the types' checked constructors.

use crate::commands::UnitList;
use crate::objects::ObjectId;
use crate::objects::masters::MasterId;
use crate::players::PlayerId;

/// The object id `value` (not −1).
pub(crate) fn object_id(value: i32) -> ObjectId {
    ObjectId::try_from(value).expect("a test's object id is not −1")
}

/// The entries of a list of ids of `values`, −1 for none.
pub(crate) fn object_ids(values: impl IntoIterator<Item = i32>) -> Vec<Option<ObjectId>> {
    values.into_iter().map(ObjectId::of_field).collect()
}

/// The player `number`.
pub(crate) fn player_id(number: u8) -> PlayerId {
    PlayerId::from(number)
}

/// The master `value` (0 or more).
pub(crate) fn master_id(value: i16) -> MasterId {
    MasterId::try_from(value).expect("a test's master is 0 or more")
}

/// The unit list of `ids`, its count theirs (fewer than `0xffff`).
pub(crate) fn unit_list(ids: Vec<Option<ObjectId>>) -> UnitList {
    UnitList::try_from(ids).expect("a test's unit list is shorter than 0xffff ids")
}
