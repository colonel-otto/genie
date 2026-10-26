//! The RGE part of the scenario data ([`RGEScen`]), read and written.

use crate::Result;
use crate::bitmap::Bitmap;
use crate::format::write_opt_string_key;
use crate::player::{PlayerBaseProperties, PlayerFiles};
use byteorder::{LE, ReadBytesExt, WriteBytesExt};
use genie_support::{ReadStringsExt, StringKey, read_opt_u32, write_opt_str, write_str};
use std::io::{Read, Write};

#[derive(Debug, Default, Clone)]
pub(crate) struct RGEScen {
    /// Data version.
    pub(crate) version: f32,
    /// Names for each player.
    pub(super) player_names: Vec<Option<String>>,
    /// Name IDs for each player.
    pub(super) player_string_table: Vec<Option<StringKey>>,
    pub(super) player_base_properties: Vec<PlayerBaseProperties>,
    pub(super) victory_conquest: bool,
    /// File name of this scenario.
    pub(crate) name: String,
    pub(super) description_string_table: Option<StringKey>,
    pub(super) hints_string_table: Option<StringKey>,
    pub(super) win_message_string_table: Option<StringKey>,
    pub(super) loss_message_string_table: Option<StringKey>,
    pub(super) history_string_table: Option<StringKey>,
    pub(super) scout_string_table: Option<StringKey>,
    pub(super) description: Option<String>,
    pub(super) hints: Option<String>,
    pub(super) win_message: Option<String>,
    pub(super) loss_message: Option<String>,
    pub(super) history: Option<String>,
    pub(super) scout: Option<String>,
    pub(super) pregame_cinematic: Option<String>,
    pub(super) victory_cinematic: Option<String>,
    pub(super) loss_cinematic: Option<String>,
    #[allow(dead_code)]
    pub(super) mission_bmp: Option<String>,
    pub(super) player_build_lists: Vec<Option<String>>,
    pub(super) player_city_plans: Vec<Option<String>>,
    pub(super) player_ai_rules: Vec<Option<String>>,
    pub(super) player_files: Vec<PlayerFiles>,
    pub(super) ai_rules_types: Vec<i8>,
}

impl RGEScen {
    // A sequential reader of the format's fields, in file order. The format stores the embedded
    // file lengths as i32; valid files never make them negative.
    #[allow(clippy::too_many_lines, clippy::cast_sign_loss)]
    pub fn read_from(mut input: impl Read) -> Result<Self> {
        let version = input.read_f32::<LE>()?;
        log::debug!("RGEScen version {version}");
        let mut player_names = vec![None; 16];
        if version > 1.13 {
            for name in &mut player_names {
                *name = input.read_str(256)?;
            }
        }

        let mut player_string_table = vec![None; 16];
        if version > 1.16 {
            for string_id in &mut player_string_table {
                *string_id = read_opt_u32(&mut input)?;
            }
        }

        let mut player_base_properties = vec![PlayerBaseProperties::default(); 16];
        if version > 1.13 {
            for properties in &mut player_base_properties {
                properties.active = input.read_i32::<LE>()?;
                properties.player_type = input.read_i32::<LE>()?;
                properties.civilization = input.read_i32::<LE>()?;
                properties.posture = input.read_i32::<LE>()?;
            }
        }

        let victory_conquest = if version >= 1.07 {
            input.read_u8()? != 0
        } else {
            true
        };

        dbg!(victory_conquest);

        {
            let timeline_count = input.read_i16::<LE>()?;
            let timeline_available = input.read_i16::<LE>()?;
            let old_time = input.read_f32::<LE>()?;
            dbg!(timeline_count, timeline_available, old_time);
            assert_eq!(timeline_count, 0, "Unexpected RGE_Timeline");
            // assert_eq!(timeline_available, 0, "Unexpected RGE_Timeline");
        }

        if version >= 1.28 {
            let civ_lock = &mut [0; 16];
            input.read_u32_into::<LE>(civ_lock)?;
        }

        // File name may be empty for embedded scenario data inside recorded games.
        let name = input.read_u16_length_prefixed_str()?.unwrap_or_default();

        let (
            description_string_table,
            hints_string_table,
            win_message_string_table,
            loss_message_string_table,
            history_string_table,
        ) = if version >= 1.16 {
            (
                read_opt_u32(&mut input)?,
                read_opt_u32(&mut input)?,
                read_opt_u32(&mut input)?,
                read_opt_u32(&mut input)?,
                read_opt_u32(&mut input)?,
            )
        } else {
            Default::default()
        };

        let scout_string_table = if version >= 1.22 {
            read_opt_u32(&mut input)?
        } else {
            None
        };

        let description = input.read_u16_length_prefixed_str()?;

        let (hints, win_message, loss_message, history) = if version >= 1.11 {
            let hints = input.read_u16_length_prefixed_str()?;
            let win_message = input.read_u16_length_prefixed_str()?;
            let loss_message = input.read_u16_length_prefixed_str()?;
            let history = input.read_u16_length_prefixed_str()?;
            (hints, win_message, loss_message, history)
        } else {
            (None, None, None, None)
        };

        let scout = if version >= 1.22 {
            input.read_u16_length_prefixed_str()?
        } else {
            None
        };

        if version < 1.03 {
            // skip some stuff
        }

        let pregame_cinematic = input.read_u16_length_prefixed_str()?;
        let victory_cinematic = input.read_u16_length_prefixed_str()?;
        let loss_cinematic = input.read_u16_length_prefixed_str()?;

        let mission_bmp = if version >= 1.09 {
            input.read_u16_length_prefixed_str()?
        } else {
            None
        };

        let _mission_picture = if version >= 1.10 {
            Bitmap::read_from(&mut input)?
        } else {
            None
        };

        let mut player_build_lists = vec![None; 16];
        for build_list in &mut player_build_lists {
            *build_list = input.read_u16_length_prefixed_str()?;
        }

        let mut player_city_plans = vec![None; 16];
        for city_plan in &mut player_city_plans {
            *city_plan = input.read_u16_length_prefixed_str()?;
        }

        let mut player_ai_rules = vec![None; 16];
        if version >= 1.08 {
            for ai_rules in &mut player_ai_rules {
                *ai_rules = input.read_u16_length_prefixed_str()?;
            }
        }

        let mut player_files = vec![PlayerFiles::default(); 16];
        for files in &mut player_files {
            let build_list_length = input.read_i32::<LE>()? as usize;
            let city_plan_length = input.read_i32::<LE>()? as usize;
            let ai_rules_length = if version >= 1.08 {
                input.read_i32::<LE>()? as usize
            } else {
                0
            };

            files.build_list = input.read_str(build_list_length)?;
            files.city_plan = input.read_str(city_plan_length)?;
            files.ai_rules = input.read_str(ai_rules_length)?;
        }

        let mut ai_rules_types = vec![0; 16];
        if version >= 1.20 {
            for rule_type in &mut ai_rules_types {
                *rule_type = input.read_i8()?;
            }
        }

        if version >= 1.02 {
            let sep = input.read_i32::<LE>()?;
            debug_assert_eq!(sep, -99);
        }

        Ok(Self {
            version,
            player_names,
            player_string_table,
            player_base_properties,
            victory_conquest,
            name,
            description_string_table,
            hints_string_table,
            win_message_string_table,
            loss_message_string_table,
            history_string_table,
            scout_string_table,
            description,
            hints,
            win_message,
            loss_message,
            history,
            scout,
            pregame_cinematic,
            victory_cinematic,
            loss_cinematic,
            mission_bmp,
            player_build_lists,
            player_city_plans,
            player_ai_rules,
            player_files,
            ai_rules_types,
        })
    }

    // A sequential writer of the format's fields, in file order. The format stores the embedded
    // file lengths as u32.
    #[allow(clippy::too_many_lines, clippy::cast_possible_truncation)]
    pub fn write_to(&self, mut output: impl Write, version: f32) -> Result<()> {
        output.write_f32::<LE>(version)?;

        if version > 1.13 {
            assert_eq!(self.player_names.len(), 16);
            for name in &self.player_names {
                let mut padded_bytes = Vec::with_capacity(256);
                if let Some(name) = name {
                    let name_bytes = name.as_bytes();
                    padded_bytes.write_all(name_bytes)?;
                }
                padded_bytes.extend(vec![0; 256 - padded_bytes.len()]);
                output.write_all(&padded_bytes)?;
            }
        }

        if version > 1.16 {
            assert_eq!(self.player_string_table.len(), 16);
            for id in &self.player_string_table {
                write_opt_string_key(&mut output, id.as_ref())?;
            }
        }

        if version > 1.13 {
            assert_eq!(self.player_base_properties.len(), 16);
            for props in &self.player_base_properties {
                output.write_i32::<LE>(props.active)?;
                output.write_i32::<LE>(props.player_type)?;
                output.write_i32::<LE>(props.civilization)?;
                output.write_i32::<LE>(props.posture)?;
            }
        }

        if version >= 1.07 {
            output.write_u8(u8::from(self.victory_conquest))?;
        }

        // RGE_Timeline
        output.write_i16::<LE>(0)?;
        output.write_i16::<LE>(0)?;
        output.write_f32::<LE>(-1.0)?;

        if version >= 1.28 {
            // Civ Lock data
            for _ in 0..16 {
                output.write_u32::<LE>(0)?;
            }
        }

        write_str(&mut output, &self.name)?;

        if version >= 1.16 {
            write_opt_string_key(&mut output, self.description_string_table.as_ref())?;
            write_opt_string_key(&mut output, self.hints_string_table.as_ref())?;
            write_opt_string_key(&mut output, self.win_message_string_table.as_ref())?;
            write_opt_string_key(&mut output, self.loss_message_string_table.as_ref())?;
            write_opt_string_key(&mut output, self.history_string_table.as_ref())?;
        }
        if version >= 1.22 {
            write_opt_string_key(&mut output, self.scout_string_table.as_ref())?;
        }

        write_opt_str(&mut output, &self.description)?;
        if version >= 1.11 {
            write_opt_str(&mut output, &self.hints)?;
            write_opt_str(&mut output, &self.win_message)?;
            write_opt_str(&mut output, &self.loss_message)?;
            write_opt_str(&mut output, &self.history)?;
        }
        if version >= 1.22 {
            write_opt_str(&mut output, &self.scout)?;
        }

        write_opt_str(&mut output, &self.pregame_cinematic)?;
        write_opt_str(&mut output, &self.victory_cinematic)?;
        write_opt_str(&mut output, &self.loss_cinematic)?;
        if version >= 1.09 {
            // mission_bmp
            write_opt_str(&mut output, &None)?;
        }

        if version >= 1.10 {
            // mission_picture
            output.write_u32::<LE>(0)?;
            output.write_u32::<LE>(0)?;
            output.write_u32::<LE>(0)?;
            output.write_u16::<LE>(1)?;
        }

        assert_eq!(self.player_build_lists.len(), 16);
        for build_list in &self.player_build_lists {
            write_opt_str(&mut output, build_list)?;
        }

        assert_eq!(self.player_city_plans.len(), 16);
        for city_plan in &self.player_city_plans {
            write_opt_str(&mut output, city_plan)?;
        }

        if version >= 1.08 {
            assert_eq!(self.player_ai_rules.len(), 16);
            for ai_rules in &self.player_ai_rules {
                write_opt_str(&mut output, ai_rules)?;
            }
        }

        assert_eq!(self.player_files.len(), 16);
        for files in &self.player_files {
            if let Some(build_list) = &files.build_list {
                output.write_u32::<LE>(build_list.len() as u32)?;
            } else {
                output.write_u32::<LE>(0)?;
            }
            if let Some(city_plan) = &files.city_plan {
                output.write_u32::<LE>(city_plan.len() as u32)?;
            } else {
                output.write_u32::<LE>(0)?;
            }
            if version >= 1.08 {
                if let Some(ai_rules) = &files.ai_rules {
                    output.write_u32::<LE>(ai_rules.len() as u32)?;
                } else {
                    output.write_u32::<LE>(0)?;
                }
            }
            if let Some(build_list) = &files.build_list {
                output.write_all(build_list.as_bytes())?;
            }
            if let Some(city_plan) = &files.city_plan {
                output.write_all(city_plan.as_bytes())?;
            }
            if version >= 1.08
                && let Some(ai_rules) = &files.ai_rules
            {
                output.write_all(ai_rules.as_bytes())?;
            }
        }

        if version >= 1.20 {
            assert_eq!(self.ai_rules_types.len(), 16);
            for ai_rules_type in &self.ai_rules_types {
                output.write_i8(*ai_rules_type)?;
            }
        }

        output.write_i32::<LE>(-99)?;

        Ok(())
    }
}
