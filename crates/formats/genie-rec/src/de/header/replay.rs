//! The replay block (pattern: `Replay`).

use binrw::binread;

#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little)]
pub struct Replay {
    pub old_time: u32,
    pub world_time: u32,
    pub old_world_time: u32,
    pub game_speed_id: u32,
    pub world_time_delta_seconds: u32,
    pub timer: f32,
    pub game_speed: f32,
    pub temp_pause: u8,
    pub next_object_id: u32,
    pub next_reusable_object_id: i32,
    pub random_seed: u32,
    pub random_seed_2: u32,
    pub rec_player: u16,
    pub num_players: u8,
    pub instant_build: u8,
    pub cheats_enabled: u8,
    pub game_mode: u16,
    pub campaign: u32,
    pub campaign_player: u32,
    pub campaign_scenario: u32,
    pub king_campaign: u32,
    pub king_campaign_player: u8,
    pub king_campaign_scenario: u8,
    pub player_turn: u32,
    #[br(count = num_players)]
    pub player_turns: Vec<u32>,
    pub unknown_1: [u8; 8],
}
