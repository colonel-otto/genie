//! The map: the terrain of each tile and where the world's objects stand (their positions,
//! radii, footprints and the grid that finds them).

mod point;
pub mod spatial;
pub mod terrain;

pub use self::point::Point;
