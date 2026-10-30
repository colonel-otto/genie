//! The map: the terrain of each tile and where the world's objects stand (their places,
//! radii, footprints and the grid that finds them).

mod grid_size;
mod offset;
mod point;
mod position;
mod radii;
mod rect;
pub mod spatial;
pub mod terrain;

pub use self::grid_size::GridSize;
pub use self::offset::{GroundOffset, Offset};
pub use self::point::{Axis, Point};
pub use self::position::Position;
pub use self::radii::{Footprint, Radii};
pub use self::rect::{Rect, Segment};
