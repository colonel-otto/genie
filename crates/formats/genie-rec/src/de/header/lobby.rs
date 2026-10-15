//! The lobby settings (pattern: `LobbySettings`, `Player`, `DeStringArray`, `AIFile`).

use super::primitives::{DeString, StringList, string_list};
use binrw::binread;

/// One of the eight lobby slots.
#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little, import(v: u32))]
pub struct LobbyPlayer {
    pub dlc_id: u32,
    pub color_id: i32,
    pub selected_color: u8,
    pub selected_team_id: u8,
    pub resolved_team_id: u8,
    pub datamod_checksum: u32,
    pub dat_crc: [u8; 4],
    pub mp_game_version: u8,
    pub civ_id: u32,
    #[br(temp)]
    custom_civ_count: u32,
    #[br(count = custom_civ_count)]
    pub custom_civ_ids: Vec<u32>,
    pub ai_type: DeString,
    pub ai_civ_name_index: u8,
    pub ai_name: DeString,
    pub censored_name: DeString,
    pub name: DeString,
    pub player_type: u32,
    pub profile_id: i32,
    pub ai_unknown: [u8; 4],
    pub player_number: i32,
    pub prefer_random: u8,
    pub custom_ai: u8,
    pub unknown_handicap: [u8; 4],
    pub handicap: i32,
    pub unknown_64_4: u32,
    #[br(if(v >= 6720))]
    pub unknown_67_2: Option<DeString>,
}

/// An AI script file named in the lobby.
#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little)]
pub struct AiFile {
    pub unknown: u32,
    pub name: DeString,
    pub unknown2: u32,
}

/// The lobby settings. The 15 single-byte flags follow aoe2rec's names.
#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little, import(v: u32))]
pub struct LobbySettings {
    pub game_options_version: u32,
    #[br(temp)]
    num_dlc: u32,
    #[br(count = num_dlc)]
    pub dlcs: Vec<u32>,
    pub difficulty: i32,
    pub map_size: u32,
    pub selected_map_id: u32,
    pub resolved_map_id: u32,
    pub reveal_map: u32,
    pub victory_type_id: u32,
    pub starting_resources_id: u32,
    pub starting_age_id: u32,
    pub ending_age_id: u32,
    pub game_type: u32,
    #[br(magic = b"\xa3\x5f\x02\x00\xa3\x5f\x02\x00")]
    pub speed: f32,
    pub treaty_length: u32,
    pub population_limit: u32,
    pub n_players: u32,
    /// `mMaxPlayersForLobby`: the game reads one byte here (FD-014 load oracle).
    pub max_players_for_lobby: u8,
    pub unused_player_color: u32,
    pub victory_amount: i32,
    /// trade, team bonus disabled, random positions, all techs, starting units, lock teams, lock
    /// speed, multiplayer, cheats, record game, animals, predators, turbo, shared exploration, team
    /// positions.
    #[br(magic = b"\xa3\x5f\x02\x00")]
    pub flags: [u8; 15],
    pub sub_game_mode: u32,
    pub battle_royale_time: u32,
    pub handicap: u8,
    pub unknown2: u8,
    #[br(magic = b"\xa3\x5f\x02\x00", args(v))]
    pub players: [LobbyPlayer; 8],
    pub unknown3: [u8; 9],
    pub fog_of_war: u8,
    pub cheat_notification: u8,
    pub colored_chat: u8,
    #[br(magic = b"\xa3\x5f\x02\x00")]
    pub ranked: u8,
    pub allow_spec: u8,
    pub lobby_visibility: u32,
    pub hidden_civs: u8,
    pub matchmaking: u8,
    pub spec_delay: u32,
    pub scenario_civ: u8,
    #[br(parse_with = string_list, args(1000))]
    pub rms_strings: StringList,
    #[br(parse_with = string_lists, args(22))]
    pub other_strings: Vec<StringList>,
    #[br(temp)]
    num_strategic_numbers: u32,
    #[br(count = num_strategic_numbers)]
    pub strategic_numbers: Vec<i32>,
    #[br(temp)]
    num_ai_files: u32,
    #[br(count = num_ai_files)]
    pub ai_files: Vec<AiFile>,
    pub unknown4: u32,
    pub unknown5: u32,
    pub unknown6: u32,
    pub guid: [u8; 16],
    pub lobby_name: DeString,
    pub unknown7: [u8; 8],
    pub modded_dataset: DeString,
    pub unknown8: [u8; 19],
    pub unknown9: [u8; 5],
    pub unknown10: [u8; 3],
    pub unknown11: DeString,
    pub unknown12: [u8; 3],
    pub unknown13: u8,
    pub unknown14: [u8; 8],
    pub unknown15: [u8; 21],
    pub unknown16: [u8; 4],
    pub unknown17: [u8; 8],
    pub unknown18: [u8; 3],
    pub unknown19: [u8; 8],
    pub unknown24: [u8; 5],
    #[br(temp)]
    unknown_66_count: u32,
    pub unknown_66_1: u32,
    #[br(count = unknown_66_count)]
    pub unknown_66_values: Vec<u32>,
    pub unknown_66_2: u32,
    pub unknown_66_3: u32,
    pub unknown20: DeString,
    #[br(if(v >= 6720))]
    pub unknown_67_2: Option<(DeString, DeString)>,
    pub unknown21: [u8; 5],
    pub unknown22: u8,
    pub unknown23: [u8; 2],
    pub timestamp_repeat: u32,
    pub ver37: u32,
}

/// The 22 lists of 100 strings after the RMS strings.
#[binrw::parser(reader, endian)]
fn string_lists(count: usize) -> binrw::BinResult<Vec<StringList>> {
    (0..count)
        .map(|_| string_list(reader, endian, (100,)))
        .collect()
}
