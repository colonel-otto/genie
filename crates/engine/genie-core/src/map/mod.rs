//! The map: the terrain of each tile and where the world's objects stand (their places,
//! radii, footprints and the grid that finds them).

mod point;
mod position;
pub mod spatial;
pub mod terrain;

pub use self::point::Point;
pub use self::position::Position;
