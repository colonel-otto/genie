//! The JSON data tables Age of Empires II: Definitive Edition loads from `resources/_common/dat`.
//!
//! Every key the installed files hold is typed, and an unknown key is an error: a game update that
//! adds one shows up instead of vanishing. A key some entries leave out is an `Option` and stays
//! out when written. Text values the game data defines (a map's style, a linked-tech group's type)
//! stay strings: new game versions add values.
//!
//! The files are read and written as values, not bytes: [`to_string`] indents by two spaces, while
//! the installed files differ in indentation.

#![deny(unsafe_code)]

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// Read a table from its JSON text.
pub fn from_str<'a, T: Deserialize<'a>>(text: &'a str) -> serde_json::Result<T> {
    serde_json::from_str(text)
}

/// The JSON text of a table, indented by two spaces.
pub fn to_string<T: Serialize>(table: &T) -> serde_json::Result<String> {
    serde_json::to_string_pretty(table)
}

/// `linkedTechs.json`: groups of techs that exclude, cycle or toggle each other.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkedTechs {
    #[serde(rename = "LinkedTechs")]
    pub linked_techs: Vec<LinkedTechGroup>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "PascalCase")]
pub struct LinkedTechGroup {
    pub name_id: i32,
    pub comment: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local: Option<bool>,
    /// `MutuallyExclusive`, `Cycle` or `Toggleable` in the current game.
    #[serde(rename = "Type")]
    pub kind: String,
    pub techs: Vec<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub starting_tech: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instant: Option<bool>,
}

/// `dropsites.json`: which buildings take which resources back, and from whom.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DropSites {
    pub drop_site_list: Vec<DropSite>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DropSite {
    pub name: String,
    pub building_id: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_object_group: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepts_livestock: Option<bool>,
    pub target_list: Vec<DropSiteTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_ai_resource_types: Option<Vec<AiResourceType>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_object_list: Option<Vec<i32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_work: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_civs: Option<Vec<i32>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DropSiteTarget {
    pub name: String,
    pub object_group: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_state: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_state_forced_return_state: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attribute_type: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_num: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_resource_distance_update: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiResourceType {
    pub resource_type: i32,
}

/// `futuravailableunits.json` (and `paphosfutureavailableunits.json`): per civilization, in the
/// file's order, the buildings and what each makes available.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FutureUnits(pub IndexMap<String, CivFutureUnits>);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "PascalCase")]
pub struct CivFutureUnits {
    pub buildings: Vec<FutureBuilding>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "PascalCase")]
pub struct FutureBuilding {
    #[serde(rename = "ID")]
    pub id: i32,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub techs: Option<Vec<FutureTech>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub units: Option<Vec<FutureUnit>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_age: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prereq_tech: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prereq_icon_set: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prereq_icon_index: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prereq_style: Option<i32>,
    #[serde(
        rename = "PrereqStringID",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub prereq_string_id: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "PascalCase")]
pub struct FutureTech {
    #[serde(rename = "ID")]
    pub id: i32,
    pub name: String,
    pub required_age: i32,
    #[serde(
        rename = "RequiredTechID",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub required_tech_id: Option<i32>,
}

/// The prerequisite fields repeat in [`FutureBuilding`]: serde cannot refuse unknown keys through
/// a flattened struct.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "PascalCase")]
pub struct FutureUnit {
    #[serde(rename = "ID")]
    pub id: i32,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_age: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prereq_tech: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prereq_icon_set: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prereq_icon_index: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prereq_style: Option<i32>,
    #[serde(
        rename = "PrereqStringID",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub prereq_string_id: Option<i32>,
    #[serde(
        rename = "RequiredUnitID",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub required_unit_id: Option<i32>,
}

/// `maps.json`: the map list of the lobby.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Maps {
    pub default_map_type: i32,
    pub default_nomad_map_type: i32,
    pub default_battle_royale_map_type: i32,
    pub map_list: Vec<MapEntry>,
}

/// A string reference: a string table id, or a key-value string's name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StringId {
    Number(i64),
    Name(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub const_name: Option<String>,
    pub data_name: String,
    pub name_string_id: i64,
    pub rollover_string_id: StringId,
    pub icon_reference: String,
    pub map_icon_filename: String,
    pub script_filename: String,
    pub storage_id: i32,
    /// `random`, `real world` or `special` in the current game.
    pub style: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub land: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub random_map: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub death_match: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub regicide: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub king_of_the_hill: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_the_relic: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sudden_death: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defend_the_wonder: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wonder_race: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub empire_wars: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub water: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mixed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prohibited_game_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub migration: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nomad: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scenario_filename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub battle_royale: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mappool_icon_filename: Option<String>,
}

/// `buttons.json`: the command buttons of the unit panel.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Buttons {
    pub command_button_list: Vec<CommandButton>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandButton {
    pub name: String,
    pub button_action_id: i32,
    pub sequence_id: i32,
    pub icon_id: i32,
    pub help_string_id: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub more_help_string_id: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub button_shift_action_id: Option<i32>,
}
