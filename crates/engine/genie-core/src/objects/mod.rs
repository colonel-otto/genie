//! One object and what every object has and runs: the object the engine tracks ([`unit::Unit`])
//! with its update, its actions' holdings and stops, its life and death, its release, its sprite
//! and facing, and the masters it is made from.

pub mod actions;
pub mod facing;
pub mod layers;
pub mod lifecycle;
pub mod lines;
pub mod masters;
pub(crate) mod release;
pub mod sprite;
pub mod unit;
