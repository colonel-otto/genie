//! The embeddable scenario data ([`TribeScen`]) and its reader.

use crate::Result;
use crate::format::RGEScen;
use crate::player::PlayerStartResources;
use crate::types::{DiplomaticStance, StartingAge};
use crate::victory::{LegacyVictoryInfo, VictoryInfo};
use byteorder::{LE, ReadBytesExt};
use genie_support::{ReadStringsExt, f32_eq};
use std::convert::TryFrom;
use std::io::Read;

/// Embeddable scenario data. This includes all scenario settings, but not map data, triggers, and
/// placed objects.
///
/// The game saves this structure in scenario files, and also in saved and recorded game files.
// Each bool is a setting the format stores.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Default, Clone)]
pub struct TribeScen {
    /// "Engine" data.
    ///
    /// This distinction doesn't make much sense as a user of this library, but
    /// it exists internally in `AoC` and affects the storage format (eg.  some
    /// things are duplicate).
    pub(crate) base: RGEScen,
    /// Starting resources for players.
    pub(super) player_start_resources: Vec<PlayerStartResources>,
    /// Victory settings.
    pub(super) victory: VictoryInfo,
    /// Whether all victory conditions need to be met for victory to occur.
    pub(super) victory_all_flag: bool,
    /// Type of victory condition to use in multiplayer games.
    pub(super) mp_victory_type: i32,
    /// Required score to attain multiplayer victory.
    pub(super) victory_score: i32,
    /// Time at which the highest-scoring player will win the multiplayer match.
    pub(super) victory_time: i32,
    /// Initial diplomacy stances between players.
    pub(super) diplomacy: Vec<Vec<DiplomaticStance>>,
    pub(super) legacy_victory_info: Vec<Vec<LegacyVictoryInfo>>,
    /// Whether Allied Victory is enabled for each player.
    pub(super) allied_victory: Vec<i32>,
    pub(super) teams_locked: bool,
    pub(super) can_change_teams: bool,
    pub(super) random_start_locations: bool,
    pub(super) max_teams: u8,
    /// Number of disabled techs per player.
    ///
    /// TODO only use `disabled_techs` for this
    pub(super) num_disabled_techs: Vec<i32>,
    /// Disabled tech IDs per player.
    pub(super) disabled_techs: Vec<Vec<i32>>,
    /// Number of disabled units per player.
    ///
    /// TODO only use `disabled_units` for this
    pub(super) num_disabled_units: Vec<i32>,
    /// Disabled unit IDs per player.
    pub(super) disabled_units: Vec<Vec<i32>>,
    /// Number of disabled buildings per player.
    ///
    /// TODO only use `disabled_buildings` for this
    pub(super) num_disabled_buildings: Vec<i32>,
    /// Disabled building IDs per player.
    pub(super) disabled_buildings: Vec<Vec<i32>>,
    /// (What exactly?)
    ///
    /// According to [`AoE2ScenarioParser`][].
    /// [`AoE2ScenarioParser`]: <https://github.com/KSneijders/AoE2ScenarioParser/blob/8e3abd422164961aa5c7857350475088790804f8/AoE2ScenarioParser/pieces/options.py>
    #[allow(dead_code)]
    pub(super) combat_mode: i32,
    /// (What exactly?)
    ///
    /// According to [`AoE2ScenarioParser`][].
    /// [`AoE2ScenarioParser`]: <https://github.com/KSneijders/AoE2ScenarioParser/blob/8e3abd422164961aa5c7857350475088790804f8/AoE2ScenarioParser/pieces/options.py>
    #[allow(dead_code)]
    pub(super) naval_mode: i32,
    /// Whether "All Techs" is enabled.
    pub(super) all_techs: bool,
    /// The starting age per player.
    pub(super) player_start_ages: Vec<StartingAge>,
    /// The initial camera location.
    pub(super) view: (i32, i32),
    /// The map type.
    pub(super) map_type: Option<i32>,
    pub(super) base_priorities: Vec<i8>,
    /// The water definition type used. (DE2 and up)
    pub(super) water_definition: Option<String>,
    /// The colour mood used. (DE2 and up)
    pub(super) color_mood: Option<String>,
    /// Is collide-and-correct pathing enabled?
    ///
    /// Only supported for DE2 and up; defaults to `false` in earlier versions.
    pub(super) collide_and_correct: bool,
    /// Is villager force drop enabled?
    ///
    /// Only supported for DE2 and up; defaults to `false` in earlier versions.
    pub(super) villager_force_drop: bool,
}

impl TribeScen {
    #[deprecated = "Use TribeScen::read_from instead"]
    #[doc(hidden)]
    pub fn from(input: impl Read) -> Result<Self> {
        Self::read_from(input)
    }

    /// Read scenario data from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading fails, if a string cannot be decoded, or if a diplomatic
    /// stance or starting age is unknown.
    // A sequential reader of the format's fields, in file order. The format stores the disabled
    // counts as i32 (never negative in valid files); a guessed count is at most 20.
    #[allow(
        clippy::too_many_lines,
        clippy::cast_sign_loss,
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap
    )]
    pub fn read_from(mut input: impl Read) -> Result<Self> {
        let mut base = RGEScen::read_from(&mut input)?;
        let version = base.version;

        let mut player_start_resources = vec![PlayerStartResources::default(); 16];

        // Moved to RGEScen in 1.13
        if version <= 1.13 {
            for name in &mut base.player_names {
                *name = input.read_str(256)?;
            }

            for (properties, start_resources) in base
                .player_base_properties
                .iter_mut()
                .zip(player_start_resources.iter_mut())
            {
                properties.active = input.read_i32::<LE>()?;
                *start_resources = PlayerStartResources::read_from(&mut input, version)?;
                properties.player_type = input.read_i32::<LE>()?;
                properties.civilization = input.read_i32::<LE>()?;
                properties.posture = input.read_i32::<LE>()?;
            }
        } else {
            for resources in &mut player_start_resources {
                *resources = PlayerStartResources::read_from(&mut input, version)?;
            }
        }

        if version >= 1.02 {
            let sep = input.read_i32::<LE>()?;
            debug_assert_eq!(sep, -99);
        }

        let victory = VictoryInfo::read_from(&mut input)?;
        let victory_all_flag = input.read_i32::<LE>()? != 0;

        let mp_victory_type = if version >= 1.13 {
            input.read_i32::<LE>()?
        } else {
            4
        };
        let victory_score = if version >= 1.13 {
            input.read_i32::<LE>()?
        } else {
            900
        };
        let victory_time = if version >= 1.13 {
            input.read_i32::<LE>()?
        } else {
            9000
        };

        log::debug!("Victory values: {mp_victory_type} {victory_score} {victory_time}");

        let mut diplomacy = vec![vec![DiplomaticStance::Neutral; 16]; 16];
        for player_diplomacy in &mut diplomacy {
            for stance in player_diplomacy.iter_mut() {
                *stance = DiplomaticStance::try_from(input.read_i32::<LE>()?)?;
            }
        }

        let mut legacy_victory_info = vec![vec![LegacyVictoryInfo::default(); 12]; 16];
        for list in &mut legacy_victory_info {
            for victory_info in list.iter_mut() {
                *victory_info = LegacyVictoryInfo::read_from(&mut input)?;
            }
        }

        if version >= 1.02 {
            let sep = input.read_i32::<LE>()?;
            debug_assert_eq!(sep, -99);
        }

        let mut allied_victory = vec![0i32; 16];
        for setting in &mut allied_victory {
            *setting = input.read_i32::<LE>()?;
        }

        let (teams_locked, can_change_teams, random_start_locations, max_teams) = if version >= 1.24
        {
            (
                input.read_i8()? != 0,
                input.read_i8()? != 0,
                input.read_i8()? != 0,
                input.read_u8()?,
            )
        } else if f32_eq!(version, 1.23) {
            (input.read_i32::<LE>()? != 0, true, true, 4)
        } else {
            (false, true, true, 4)
        };

        let mut num_disabled_techs = vec![0; 16];
        let mut disabled_techs = vec![vec![]; 16];
        let mut num_disabled_units = vec![0; 16];
        let mut disabled_units = vec![vec![]; 16];
        let mut num_disabled_buildings = vec![0; 16];
        let mut disabled_buildings = vec![vec![]; 16];

        if version >= 1.28 {
            // Definitive Edition only stores the exact number of disabled techs/units/buildings.
            input.read_i32_into::<LE>(&mut num_disabled_techs)?;
            for (player_disabled_techs, &num) in
                disabled_techs.iter_mut().zip(num_disabled_techs.iter())
            {
                *player_disabled_techs = vec![0; num as usize];
                input.read_i32_into::<LE>(player_disabled_techs)?;
            }

            input.read_i32_into::<LE>(&mut num_disabled_units)?;
            for (player_disabled_units, &num) in
                disabled_units.iter_mut().zip(num_disabled_units.iter())
            {
                *player_disabled_units = vec![0; num as usize];
                input.read_i32_into::<LE>(player_disabled_units)?;
            }

            input.read_i32_into::<LE>(&mut num_disabled_buildings)?;
            for (player_disabled_buildings, &num) in disabled_buildings
                .iter_mut()
                .zip(num_disabled_buildings.iter())
            {
                *player_disabled_buildings = vec![0; num as usize];
                input.read_i32_into::<LE>(player_disabled_buildings)?;
            }
        } else if version >= 1.18 {
            // AoC and friends store up to 20 or 30 of each.
            input.read_i32_into::<LE>(&mut num_disabled_techs)?;
            for player_disabled_techs in &mut disabled_techs {
                *player_disabled_techs = vec![0; 30];
                input.read_i32_into::<LE>(player_disabled_techs)?;
            }

            input.read_i32_into::<LE>(&mut num_disabled_units)?;
            for player_disabled_units in &mut disabled_units {
                *player_disabled_units = vec![0; 30];
                input.read_i32_into::<LE>(player_disabled_units)?;
            }

            input.read_i32_into::<LE>(&mut num_disabled_buildings)?;
            let max_disabled_buildings = if version >= 1.25 { 30 } else { 20 };
            for player_disabled_buildings in &mut disabled_buildings {
                *player_disabled_buildings = vec![0; max_disabled_buildings];
                input.read_i32_into::<LE>(player_disabled_buildings)?;
            }
        } else if version > 1.03 {
            // Old scenarios only allowed disabling up to 20 techs per player.
            for i in 0..16 {
                let player_disabled_techs = &mut disabled_techs[i];
                *player_disabled_techs = vec![0; 20];
                input.read_i32_into::<LE>(player_disabled_techs)?;
                // The number of disabled techs wasn't stored either, so we need to guess it!
                num_disabled_techs[i] = player_disabled_techs
                    .iter()
                    .position(|val| *val <= 0)
                    .map_or(0, |index| (index as i32) + 1);
            }
        } else {
            // <= 1.03 did not support disabling anything
        }

        let combat_mode = if version > 1.04 {
            input.read_i32::<LE>()?
        } else {
            0
        };
        let (naval_mode, all_techs) = if version >= 1.12 {
            (input.read_i32::<LE>()?, input.read_i32::<LE>()? != 0)
        } else {
            (0, false)
        };

        let mut player_start_ages = vec![StartingAge::Default; 16];
        if version > 1.05 {
            for start_age in &mut player_start_ages {
                *start_age = StartingAge::try_from(input.read_i32::<LE>()?, version)?;
            }
        }

        log::debug!("starting ages: {player_start_ages:?}");

        if version >= 1.02 {
            let sep = input.read_i32::<LE>()?;
            debug_assert_eq!(sep, -99);
        }

        let view = if version >= 1.19 {
            (input.read_i32::<LE>()?, input.read_i32::<LE>()?)
        } else {
            (-1, -1)
        };

        let map_type = if version >= 1.21 {
            match input.read_i32::<LE>()? {
                // HD Edition uses -2 instead of -1?
                -2 | -1 => None,
                id => Some(id),
            }
        } else {
            None
        };

        let mut base_priorities = vec![0; 16];
        if version >= 1.24 {
            input.read_i8_into(&mut base_priorities)?;
        }

        let mut water_definition = None;
        let mut color_mood = None;
        let mut collide_and_correct = false;
        let mut villager_force_drop = false;

        if version >= 1.35 {
            // Duplicated here from TriggerSystem … we can discard it because the TriggerSystem
            // will read the same number later.
            let _trigger_count = input.read_u32::<LE>()?;
        }
        if version >= 1.30 {
            let _str_signature = input.read_u16::<LE>()?;
            water_definition = input.read_u16_length_prefixed_str()?;
        }

        if version >= 1.32 {
            let _str_signature = input.read_u16::<LE>()?;
            color_mood = input.read_u16_length_prefixed_str()?;
        }
        if version >= 1.36 {
            collide_and_correct = input.read_u8()? != 0;
        }
        if version >= 1.37 {
            villager_force_drop = input.read_u8()? != 0;
        }

        Ok(Self {
            base,
            player_start_resources,
            victory,
            victory_all_flag,
            mp_victory_type,
            victory_score,
            victory_time,
            diplomacy,
            legacy_victory_info,
            allied_victory,
            teams_locked,
            can_change_teams,
            random_start_locations,
            max_teams,
            num_disabled_techs,
            disabled_techs,
            num_disabled_units,
            disabled_units,
            num_disabled_buildings,
            disabled_buildings,
            combat_mode,
            naval_mode,
            all_techs,
            player_start_ages,
            view,
            map_type,
            base_priorities,
            water_definition,
            color_mood,
            collide_and_correct,
            villager_force_drop,
        })
    }
}
