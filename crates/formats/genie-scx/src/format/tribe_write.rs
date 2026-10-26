//! The writer of the embeddable scenario data ([`TribeScen::write_to`]).

use crate::format::TribeScen;
use crate::{Error, Result};
use byteorder::{LE, WriteBytesExt};
use genie_support::{f32_eq, write_opt_str};
use std::io::Write;

impl TribeScen {
    /// Write scenario data to an output stream.
    ///
    /// # Errors
    ///
    /// Returns an error if writing fails, if a string cannot be encoded or a string key does not
    /// fit a u32, or if `version` cannot hold the disabled technologies, units or buildings.
    ///
    /// # Panics
    ///
    /// Panics if a per-player list does not have 16 entries, or if a player name is longer than
    /// 256 bytes.
    // A sequential writer of the format's fields, in file order. The disabled counts are never
    // negative in valid data.
    #[allow(clippy::too_many_lines, clippy::cast_sign_loss)]
    pub fn write_to(&self, mut output: impl Write, version: f32, num_triggers: u32) -> Result<()> {
        self.base.write_to(&mut output, version)?;

        if version <= 1.13 {
            assert_eq!(self.base.player_names.len(), 16);
            for name in &self.base.player_names {
                let mut padded_bytes = Vec::with_capacity(256);
                if let Some(name) = name {
                    let name_bytes = name.as_bytes();
                    padded_bytes.write_all(name_bytes)?;
                }
                padded_bytes.extend(vec![0; 256 - padded_bytes.len()]);
                output.write_all(&padded_bytes)?;
            }

            assert_eq!(self.base.player_base_properties.len(), 16);
            assert_eq!(self.player_start_resources.len(), 16);
            for i in 0..16 {
                let properties = &self.base.player_base_properties[i];
                let resources = &self.player_start_resources[i];
                output.write_i32::<LE>(properties.active)?;
                resources.write_to(&mut output, version)?;
                output.write_i32::<LE>(properties.player_type)?;
                output.write_i32::<LE>(properties.civilization)?;
                output.write_i32::<LE>(properties.posture)?;
            }
        } else {
            assert_eq!(self.player_start_resources.len(), 16);
            for start_resources in &self.player_start_resources {
                start_resources.write_to(&mut output, version)?;
            }
        }

        if version >= 1.02 {
            output.write_i32::<LE>(-99)?;
        }

        self.victory.write_to(&mut output)?;
        output.write_i32::<LE>(i32::from(self.victory_all_flag))?;

        if version >= 1.13 {
            output.write_i32::<LE>(self.mp_victory_type)?;
            output.write_i32::<LE>(self.victory_score)?;
            output.write_i32::<LE>(self.victory_time)?;
        }

        assert_eq!(self.diplomacy.len(), 16);
        for player_diplomacy in &self.diplomacy {
            assert_eq!(player_diplomacy.len(), 16);
            for stance in player_diplomacy {
                output.write_i32::<LE>((*stance).into())?;
            }
        }

        assert_eq!(self.legacy_victory_info.len(), 16);
        for list in &self.legacy_victory_info {
            for entry in list {
                entry.write_to(&mut output)?;
            }
        }

        if version >= 1.02 {
            output.write_i32::<LE>(-99)?;
        }

        for value in &self.allied_victory {
            output.write_i32::<LE>(*value)?;
        }

        if version >= 1.24 {
            output.write_i8(i8::from(self.teams_locked))?;
            output.write_i8(i8::from(self.can_change_teams))?;
            output.write_i8(i8::from(self.random_start_locations))?;
            output.write_u8(self.max_teams)?;
        } else if f32_eq!(version, 1.23) {
            output.write_i32::<LE>(i32::from(self.teams_locked))?;
        }

        if version >= 1.28 {
            for num in &self.num_disabled_techs {
                output.write_i32::<LE>(*num)?;
            }
            for (player_disabled_techs, &num) in self
                .disabled_techs
                .iter()
                .zip(self.num_disabled_techs.iter())
            {
                for i in 0..num as usize {
                    output.write_i32::<LE>(*player_disabled_techs.get(i).unwrap_or(&-1))?;
                }
            }

            for num in &self.num_disabled_units {
                output.write_i32::<LE>(*num)?;
            }
            for (player_disabled_units, &num) in self
                .disabled_units
                .iter()
                .zip(self.num_disabled_units.iter())
            {
                for i in 0..num as usize {
                    output.write_i32::<LE>(*player_disabled_units.get(i).unwrap_or(&-1))?;
                }
            }

            for num in &self.num_disabled_buildings {
                output.write_i32::<LE>(*num)?;
            }
            for (player_disabled_buildings, &num) in self
                .disabled_buildings
                .iter()
                .zip(self.num_disabled_buildings.iter())
            {
                for i in 0..num as usize {
                    output.write_i32::<LE>(*player_disabled_buildings.get(i).unwrap_or(&-1))?;
                }
            }
        } else if version >= 1.18 {
            let max_disabled_buildings = if version >= 1.25 { 30 } else { 20 };
            let most = *self.num_disabled_buildings.iter().max().unwrap_or(&0);
            if most > max_disabled_buildings {
                return Err(Error::TooManyDisabledBuildingsError(
                    most,
                    max_disabled_buildings,
                ));
            }

            for num in &self.num_disabled_techs {
                output.write_i32::<LE>(*num)?;
            }
            for player_disabled_techs in &self.disabled_techs {
                for i in 0..30 {
                    output.write_i32::<LE>(*player_disabled_techs.get(i).unwrap_or(&-1))?;
                }
            }

            for num in &self.num_disabled_units {
                output.write_i32::<LE>(*num)?;
            }
            for player_disabled_units in &self.disabled_units {
                for i in 0..30 {
                    output.write_i32::<LE>(*player_disabled_units.get(i).unwrap_or(&-1))?;
                }
            }

            for num in &self.num_disabled_buildings {
                output.write_i32::<LE>(*num)?;
            }
            for player_disabled_buildings in &self.disabled_buildings {
                for i in 0..max_disabled_buildings as usize {
                    output.write_i32::<LE>(*player_disabled_buildings.get(i).unwrap_or(&-1))?;
                }
            }
        } else if version > 1.03 {
            let most = *self.num_disabled_techs.iter().max().unwrap_or(&0);
            if most > 20 {
                return Err(Error::TooManyDisabledTechsError(most));
            }
            if self.num_disabled_units.iter().any(|&n| n > 0) {
                return Err(Error::CannotDisableUnitsError);
            }
            if self.num_disabled_buildings.iter().any(|&n| n > 0) {
                return Err(Error::CannotDisableBuildingsError);
            }

            // Old scenarios only allowed disabling up to 20 techs per player.
            for player_disabled_techs in &self.disabled_techs {
                for i in 0..20 {
                    output.write_i32::<LE>(*player_disabled_techs.get(i).unwrap_or(&-1))?;
                }
            }
        } else {
            // <= 1.03 did not support disabling anything
            if self.num_disabled_techs.iter().any(|&n| n > 0) {
                return Err(Error::CannotDisableTechsError);
            }
            if self.num_disabled_units.iter().any(|&n| n > 0) {
                return Err(Error::CannotDisableUnitsError);
            }
            if self.num_disabled_buildings.iter().any(|&n| n > 0) {
                return Err(Error::CannotDisableBuildingsError);
            }
        }

        if version > 1.04 {
            output.write_i32::<LE>(0)?;
        }
        if version >= 1.12 {
            output.write_i32::<LE>(0)?;
            output.write_i32::<LE>(i32::from(self.all_techs))?;
        }

        if version > 1.05 {
            for start_age in &self.player_start_ages {
                output.write_i32::<LE>(start_age.to_i32(version))?;
            }
        }

        if version >= 1.02 {
            output.write_i32::<LE>(-99)?;
        }

        if version >= 1.19 {
            output.write_i32::<LE>(self.view.0)?;
            output.write_i32::<LE>(self.view.1)?;
        }

        if version >= 1.21 {
            output.write_i32::<LE>(self.map_type.unwrap_or(-1))?;
        }

        if version >= 1.24 {
            assert_eq!(self.base_priorities.len(), 16);
            for priority in &self.base_priorities {
                output.write_i8(*priority)?;
            }
        }

        if version >= 1.28 {
            output.write_u32::<LE>(num_triggers)?;
            output.write_u16::<LE>(0)?;
            write_opt_str(&mut output, &self.water_definition)?;
        }

        if version >= 1.36 {
            output.write_u8(0)?;
            output.write_u8(0)?;
            write_opt_str(&mut output, &self.color_mood)?;
            output.write_u8(u8::from(self.collide_and_correct))?;
        }
        if version >= 1.37 {
            output.write_u8(u8::from(self.villager_force_drop))?;
        }

        Ok(())
    }
}
