//! The tribe player's own state, from its tech states on (158041: the tribe player save
//! `0x141631420`, after `WorldPlayerBase::save`). Read in the binary's write order.
//!
//! aoe2rec's pattern read the part before the tech tree as `UnknownInit` plus five fixed-size
//! sections, and the part after it as two runs up to the next `0x0b`. Three of the fixed sizes only
//! hold for some games: the offer messages, the per-player `mTCFlared` flags and the history lists
//! vary, and the runs after the tech tree hold the scout map, the computer player's AI and Gaia's
//! own fields. The `0x0b`, `0x16` and `0x21` bytes are the engine's serialization markers
//! (`SerializeMarker.h`, written by `0x141553ef0`).

use super::primitives::DeString;
use binrw::{BinRead, binread};

/// The marker the tribe player save writes between its parts (158041: `0x143cd8a28`).
pub(super) const MARKER: &[u8] = b"\x0b";

/// The state from the tech states to the tech tree.
#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little, import(v: u32, num_players: usize))]
pub struct TribeState {
    #[br(temp)]
    tech_count: i16,
    #[br(count = tech_count.max(0) as usize, args { inner: (v,) })]
    pub techs: Vec<TechState>,
    pub update_history_count: u32,
    pub history: History,
    pub ruin_held_time: f32,
    pub artifact_held_time: f32,
    #[br(magic = b"\x0b")]
    pub offers: [Offer; 9],
    pub fealty: i16,
    #[br(magic = b"\x0b")]
    pub offboard_trade_route_explored: [u8; 20],
    pub offboard_trade_route_being_explored: [u8; 20],
    #[br(magic = b"\x0b")]
    pub trade: Trade,
    #[br(magic = b"\x0b")]
    pub sudden_death_can_be_eliminated: u8,
    /// One flag per player, Gaia included.
    #[br(count = num_players)]
    pub tc_flared: Vec<u8>,
    pub old_kills: i32,
    pub old_razings: i32,
    pub battle_flag: i32,
    pub razings_flag: i32,
    pub total_kills: i32,
    pub total_razings: i32,
    pub old_hit_points: i32,
    pub total_hit_points: i32,
    pub old_player_kills: [i32; 9],
}

/// One entry of the player's tech states (`tech.cpp`, `0x1413d1a60`).
#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little, import(v: u32))]
pub struct TechState {
    pub research_done: f32,
    pub state: i16,
    pub cost_adjustments: [i16; 3],
    pub times_researched: i16,
    pub allow_multiple_research: u8,
    /// Two strings written by `0x140936c30` (from the tech at `+0x10` and `+0x28`); a third from
    /// save 68.9.
    pub strings: [DeString; 2],
    #[br(if(v >= 6890))]
    pub string_68_9: Option<DeString>,
    pub stack_tech: u8,
    pub research_cap: i16,
    pub pending_count: i16,
    /// Per cost slot: attribute used, attribute type, amount.
    pub cost_overrides: [[i16; 3]; 3],
    pub icon_override: i16,
    pub hotkey_override: i32,
    pub enabled_tech: u8,
    pub effect_override: i16,
    #[br(temp)]
    location_count: i16,
    #[br(count = location_count.max(0) as usize)]
    pub locations: Vec<ResearchLocation>,
    pub research_building_main_id: i16,
}

#[binread]
#[derive(Debug, Clone, Copy, PartialEq)]
#[br(little)]
pub struct ResearchLocation {
    pub location_id: i16,
    pub research_time: i16,
    pub button_location_id: u8,
    pub hotkey_id: i32,
    pub research_adjustment: f32,
}

/// The player's history (`history->save`, `0x141565560`), between markers `0x0b` … `0x21`. Field
/// names are not in the binary; offsets are the history object's.
#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little, magic = b"\x0b")]
pub struct History {
    #[br(temp)]
    entry_count: u32,
    pub value_18: u32,
    pub value_20: u32,
    #[br(magic = b"\x16", count = entry_count)]
    pub entries: Vec<[u16; 2]>,
    #[br(temp)]
    node_count: u32,
    #[br(count = node_count)]
    pub nodes: Vec<HistoryNode>,
    /// `+0x2c` … `+0x60`, then `+0x28`, `+0x64`, `+0x68`.
    pub values: [u32; 17],
    #[br(temp)]
    list_len: u32,
    #[br(count = list_len)]
    pub list_a: Vec<u32>,
    #[br(count = list_len)]
    pub list_b: Vec<u32>,
    pub values_e0: [u32; 3],
    pub values_ec: [u16; 2],
    #[br(magic = b"\x21", temp)]
    end: (),
}

/// A node of the history's linked list: the bytes at `+0`, then `+4`, `+0xc`, `+0x10`, `+0x14`, `+8`.
#[binread]
#[derive(Debug, Clone, Copy, PartialEq)]
#[br(little)]
pub struct HistoryNode {
    pub kind: u8,
    pub values: [u32; 5],
}

/// One player's diplomacy offer, with the three per-player bytes written before it.
#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little)]
pub struct Offer {
    pub diplomacy: u8,
    pub intelligence: u8,
    pub trade: u8,
    pub sequence: u8,
    pub started_by: u8,
    pub game_time: u32,
    pub declare: u8,
    pub old_diplomacy: u8,
    pub new_diplomacy: u8,
    pub old_intelligence: u8,
    pub new_intelligence: u8,
    pub old_trade: u8,
    pub new_trade: u8,
    pub demand: u8,
    pub gold: i32,
    #[br(temp)]
    message_len: u8,
    #[br(count = message_len)]
    pub message: Vec<u8>,
    pub status: u8,
}

/// The trade and queue settings between two markers.
#[binread]
#[derive(Debug, Clone, Copy, PartialEq)]
#[br(little)]
pub struct Trade {
    pub max_trade_amount: i32,
    pub old_max_trade_amount: i32,
    pub max_trade_limit: i32,
    pub current_wood_limit: i32,
    pub current_food_limit: i32,
    pub current_stone_limit: i32,
    pub current_ore_limit: i32,
    pub commodity_volume_delta: i32,
    pub trade_vig_rate: f32,
    pub trade_refresh_timer: i32,
    pub trade_refresh_rate: i32,
    pub prod_queue_on: u8,
    pub chance_to_dodge_missiles: u8,
    pub chance_for_archers_to_maintain_distance: u8,
    pub open_gates_for_pathing_count: i32,
    pub farm_queue_count: i32,
    pub auto_farm_queue: u8,
    pub fish_trap_queue_count: i32,
    pub auto_fish_trap_queue: u8,
    pub cp_nomad_tc_build_lock: i32,
}

/// The state after the tech tree, which depends on the player type (`0x141631420` from
/// `0x1416355f0` on, Gaia's part from `0x14167fc40`).
#[derive(Debug, Clone, PartialEq)]
pub struct TribeEnd {
    /// Players of type 1 only: the enabled flag and, when the player has one, the scout map.
    pub scout_map: Option<(u32, Option<ScoutMap>)>,
    pub final_battle_royale_position: u32,
    pub sharing_is_caring: u8,
    pub is_first_update: u8,
    pub d3kt_military_unit_built: u8,
    /// Gaia (type 2) only.
    pub gaia: Option<GaiaState>,
}

/// A player's scout map (`0x140f4cb50`).
#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little)]
pub struct ScoutMap {
    pub size_x: i32,
    pub size_y: i32,
    pub reference_x: i32,
    pub reference_y: i32,
    pub unchangeable_limit: u8,
    #[br(count = size_x.max(0) as usize * size_y.max(0) as usize)]
    pub cells: Vec<u8>,
}

/// Gaia's own fields after the tribe player save (`0x14167fc40`).
#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little)]
pub struct GaiaState {
    pub update_time: f32,
    pub update_nature: i32,
    /// Per creature the words at `+0x1478`, `+0x147c`, `+0x1480`: `max`, `remainder` and
    /// `growth_rate` in some order (all zero in the corpus).
    pub creatures: [[u32; 3]; 5],
    pub next_wolf_attack_update_time: u32,
    pub wolf_attack_update_interval: u32,
    pub wolf_attack_stop_time: u32,
    pub min_villager_distance: f32,
    pub tc_position_x: [f32; 9],
    pub tc_position_y: [f32; 9],
    pub wolf_current_player: i32,
    pub wolf_current_villager_list: [i32; 10],
    pub wolf_current_villager: i32,
    pub wolf_villager_count: i32,
    pub wolf_list: [i32; 50],
    pub current_wolf: i32,
    pub wolf_count: [i32; 10],
}

/// The player types the tribe player save branches on (`player + 0x10`).
pub(super) const HUMAN: u8 = 1;
pub(super) const GAIA: u8 = 2;
pub(super) const COMPUTER: u8 = 3;

/// Read the state after the tech tree, through the second marker and Gaia's fields.
pub(super) fn read_end<R: binrw::io::Read + binrw::io::Seek>(
    r: &mut R,
    player_type: u8,
) -> binrw::BinResult<TribeEnd> {
    use super::primitives::expect;
    let scout_map = if player_type == HUMAN {
        let enabled = u32::read_le(r)?;
        let map = (enabled != 0).then(|| ScoutMap::read(r)).transpose()?;
        Some((enabled, map))
    } else {
        None
    };
    let final_battle_royale_position = u32::read_le(r)?;
    let [sharing_is_caring, is_first_update, d3kt_military_unit_built] = <[u8; 3]>::read_le(r)?;
    expect(r, MARKER)?;
    if player_type == COMPUTER && u32::read_le(r)? != 0 {
        return Err(unsupported(r, "computer player AI state (playerAI->save)"));
    }
    expect(r, MARKER)?;
    let gaia = (player_type == GAIA)
        .then(|| GaiaState::read(r))
        .transpose()?;
    Ok(TribeEnd {
        scout_map,
        final_battle_royale_position,
        sharing_is_caring,
        is_first_update,
        d3kt_military_unit_built,
        gaia,
    })
}

/// Marks a feature this port does not read yet, so callers can tell it from a parse error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnsupportedFeature(pub &'static str);

impl std::fmt::Display for UnsupportedFeature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unsupported: {}", self.0)
    }
}

impl std::error::Error for UnsupportedFeature {}

pub(super) fn unsupported<R: binrw::io::Seek>(reader: &mut R, what: &'static str) -> binrw::Error {
    binrw::Error::Custom {
        pos: reader.stream_position().unwrap_or_default(),
        err: Box::new(UnsupportedFeature(what)),
    }
}

/// The feature named by an `Unsupported` marker anywhere in a read error, with its position.
pub fn unsupported_in(e: &binrw::Error) -> Option<(&'static str, usize)> {
    let root = e.root_cause();
    let what = root.custom_err::<UnsupportedFeature>()?.0;
    let at = match root {
        binrw::Error::Custom { pos, .. } => *pos as usize,
        _ => 0,
    };
    Some((what, at))
}
