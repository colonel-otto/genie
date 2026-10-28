//! One object and what every object has and runs: the object the engine tracks ([`unit::Unit`])
//! with its update, its actions' holdings and stops, its life and death, its release, its sprite
//! and facing, and the masters it is made from.

pub mod actions;
pub mod facing;
pub(crate) mod hit_points;
pub mod layers;
pub mod lifecycle;
pub mod lines;
pub mod masters;
mod object_id;
pub mod object_list;
pub(crate) mod release;
pub mod sprite;
pub(crate) mod teardown;
pub mod unit;

pub use self::object_id::ObjectId;
