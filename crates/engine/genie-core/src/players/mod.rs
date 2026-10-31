//! The players beside their objects: their game status (playing, won, lost), diplomacy,
//! tribute, treason, the market, their history, their object trackers, and the world player's
//! own update.

pub mod ai_module;
pub mod attribute;
pub mod diplomacy;
pub mod game_status;
pub mod history;
pub mod market;
mod player_id;
mod player_type;
pub mod tracker;
pub mod treason;
pub mod tribute;
pub mod world_player;

pub use self::player_id::PlayerId;
pub use self::player_type::PlayerType;
