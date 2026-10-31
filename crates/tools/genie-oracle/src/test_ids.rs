//! The tests' object ids, through the type's checked constructor.

use genie_core::objects::ObjectId;

/// The object id `value` (not −1).
pub(crate) const fn object_id(value: i32) -> ObjectId {
    match ObjectId::of_field(value) {
        Some(id) => id,
        None => panic!("a test's object id is not −1"),
    }
}
