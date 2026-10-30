//! The map: the terrain of each tile and where the world's objects stand (their places,
//! radii, footprints and the grid that finds them).

mod offset;
mod point;
mod position;
mod radii;
pub mod spatial;
pub mod terrain;

pub use self::offset::Offset;
pub use self::point::Point;
pub use self::position::Position;
pub use self::radii::{Footprint, Radii};
