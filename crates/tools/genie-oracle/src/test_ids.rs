//! The tests' object ids, through the type's checked constructor.

use genie_core::objects::ObjectId;

/// The object id `value` (not −1).
pub(crate) fn object_id(value: i32) -> ObjectId {
    ObjectId::try_from(value).expect("a test's object id is not −1")
}
