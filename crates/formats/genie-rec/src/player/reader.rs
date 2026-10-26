//! The reader of a player's state ([`Player`]) in the recording header.

use crate::ai::PlayerAI;
use crate::element::{OptionalReadableElement, ReadableHeaderElement};
use crate::player::{
    DiplomacyOffer, GaiaData, HistoryInfo, Player, PlayerTech, UserPatchData, VisibleMap,
    VisibleResources,
};
use crate::reader::RecordingHeaderReader;
use crate::unit::Unit;
use crate::unit_type::CompactUnitType;
use crate::{ObjectID, Result};
use byteorder::{LE, ReadBytesExt};
use genie_dat::tech_tree::TechTree;
use genie_support::ReadStringsExt;
use std::convert::TryInto;
use std::io::Read;

impl ReadableHeaderElement for Player {
    // Reads the player's fields in file order.
    #[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        let mut player = Self {
            kind: input.read_u8()?,
            ..Default::default()
        };
        if input.version() >= 10.55 {
            assert_eq!(input.read_u8()?, 11);
        }
        player.relations = vec![0; input.num_players() as usize];
        input.read_exact(&mut player.relations)?;
        input.read_u32_into::<LE>(&mut player.diplomacy)?;
        player.allied_los = input.read_u32::<LE>()? != 0;
        player.allied_victory = input.read_u8()? != 0;
        player.name = input.read_u16_length_prefixed_str()?.unwrap_or_default();
        dbg!(input.version());
        if input.version() >= 10.55 {
            assert_eq!(input.read_u8()?, 22);
        }
        let num_attributes = input.read_u32::<LE>()?;
        if input.version() >= 10.55 {
            assert_eq!(input.read_u8()?, 33);
        }
        player.attributes = vec![0.0; num_attributes.try_into().unwrap()];
        input.read_f32_into::<LE>(&mut player.attributes)?;
        if input.version() >= 10.55 {
            assert_eq!(input.read_u8()?, 11);
        }
        player.initial_view = (input.read_f32::<LE>()?, input.read_f32::<LE>()?);
        if input.version() >= 11.62 {
            let num_saved_views = input.read_i32::<LE>()?;
            // saved view count can be negative
            player.saved_views = vec![(0.0, 0.0); num_saved_views.try_into().unwrap_or(0)];
            for sv in &mut player.saved_views {
                *sv = (input.read_f32::<LE>()?, input.read_f32::<LE>()?);
            }
        }
        player.spawn_location = (input.read_u16::<LE>()?, input.read_u16::<LE>()?);
        player.culture_id = input.read_u8()?;
        player.civilization_id = input.read_u8()?.into();
        player.game_status = input.read_u8()?;
        player.resigned = input.read_u8()? != 0;
        if input.version() >= 10.55 {
            assert_eq!(input.read_u8()?, 11);
        }
        let _color = input.read_u8()?;
        if input.version() >= 10.55 {
            assert_eq!(input.read_u8()?, 11);
        }
        let _pathing_attempt_cap = input.read_u32::<LE>()?;
        let _pathing_delay_cap = input.read_u32::<LE>()?;

        // Unit counts
        let counts = if input.version() >= 11.65 {
            (900, 100, 900, 100)
        } else if input.version() >= 11.51 {
            (850, 100, 850, 100)
        } else {
            (750, 100, 750, 100)
        };
        let mut object_categories_count = vec![0; counts.0];
        input.read_u16_into::<LE>(&mut object_categories_count)?;
        let mut object_groups_count = vec![0; counts.1];
        input.read_u16_into::<LE>(&mut object_groups_count)?;

        let mut built_object_categories_count = vec![0; counts.2];
        input.read_u16_into::<LE>(&mut built_object_categories_count)?;
        let mut built_object_groups_count = vec![0; counts.3];
        input.read_u16_into::<LE>(&mut built_object_groups_count)?;

        let _total_units_count = input.read_u16::<LE>()?;
        let _total_buildings_count = input.read_u16::<LE>()?;
        let _built_units_count = input.read_u16::<LE>()?;
        let _built_buildings_count = input.read_u16::<LE>()?;

        // formations
        let _line_ratio = input.read_u32::<LE>()?;
        let _column_ratio = input.read_u32::<LE>()?;
        let _min_column_distance = input.read_u32::<LE>()?;
        let _column_to_line_distance = input.read_u32::<LE>()?;
        let _auto_formations = input.read_u32::<LE>()?;
        let _formations_influence_distance = input.read_f32::<LE>()?;
        let _break_auto_formations_by_speed = if input.version() >= 10.81 {
            input.read_f32::<LE>()?
        } else {
            0.0
        };

        // escrow
        let _pending_debits = (
            input.read_f32::<LE>()?,
            input.read_f32::<LE>()?,
            input.read_f32::<LE>()?,
            input.read_f32::<LE>()?,
        );
        let _escrow_amounts = (
            input.read_f32::<LE>()?,
            input.read_f32::<LE>()?,
            input.read_f32::<LE>()?,
            input.read_f32::<LE>()?,
        );
        let _escrow_percents = (
            input.read_f32::<LE>()?,
            input.read_f32::<LE>()?,
            input.read_f32::<LE>()?,
            input.read_f32::<LE>()?,
        );

        // view scrolling
        if input.version() >= 10.51 {
            let _scroll_vector = (input.read_f32::<LE>()?, input.read_f32::<LE>()?);
            let _scroll_end = (input.read_f32::<LE>()?, input.read_f32::<LE>()?);
            let _scroll_start = (input.read_f32::<LE>()?, input.read_f32::<LE>()?);
            let _scroll_total_distance = input.read_f32::<LE>()?;
            let _scroll_distance = input.read_f32::<LE>()?;
        }

        // AI state
        if input.version() >= 11.45 {
            let _easiest_reaction_percent = input.read_f32::<LE>()?;
            let _easier_reaction_percent = input.read_f32::<LE>()?;
            let _task_ungrouped_soldiers = input.read_u8()? != 0;
        }

        // selected units
        if input.version() >= 11.72 {
            let num_selections = input.read_u32::<LE>()?;
            let _selection = if num_selections > 0 {
                let object_id: ObjectID = input.read_u32::<LE>()?.into();
                let object_properties = input.read_u32::<LE>()?;
                let mut selected_ids = vec![ObjectID(0); num_selections.try_into().unwrap()];
                for id in &mut selected_ids {
                    *id = input.read_u32::<LE>()?.into();
                }
                Some((object_id, object_properties, selected_ids))
            } else {
                None
            };
        }

        if input.version() >= 10.55 {
            assert_eq!(input.read_u8()?, 11);
            assert_eq!(input.read_u8()?, 11);
        }

        let _ty = input.read_u8()?;
        let _update_count = input.read_u32::<LE>()?;
        let _update_count_need_help = input.read_u32::<LE>()?;

        // ai attack data
        if input.version() >= 10.02 {
            let _alerted_enemy_count = input.read_u32::<LE>()?;
            let _regular_attack_count = input.read_u32::<LE>()?;
            let _regular_attack_mode = input.read_u8()?;
            let _regular_attack_location = (input.read_f32::<LE>()?, input.read_f32::<LE>()?);
            let _town_attack_count = input.read_u32::<LE>()?;
            let _town_attack_mode = input.read_u8()?;
            let _town_attack_location = (input.read_f32::<LE>()?, input.read_f32::<LE>()?);
        }

        let _fog_update = input.read_u32::<LE>()?;
        let _update_time = input.read_f32::<LE>()?;

        // if is userpatch
        if genie_support::f32_eq!(input.version(), 11.97) {
            player.userpatch_data = Some(UserPatchData::read_from(input)?);
        }

        player.tech_state = PlayerTech::read_from(input)?;

        let _update_history_count = input.read_u32::<LE>()?;
        player.history_info = HistoryInfo::read_from(input)?;

        if input.version() >= 5.30 {
            let _ruin_held_time = input.read_u32::<LE>()?;
            let _artifact_held_time = input.read_u32::<LE>()?;
        }

        if input.version() >= 10.55 {
            assert_eq!(input.read_u8()?, 11);
        }

        // diplomacy
        if input.version() >= 9.13 {
            let mut diplomacy = [0; 9];
            let mut intelligence = [0; 9];
            let mut trade = [0; 9];
            for i in 0..9 {
                diplomacy[i] = input.read_u8()?;
                intelligence[i] = input.read_u8()?;
                trade[i] = input.read_u8()?;

                let _offer = DiplomacyOffer::read_from(input)?;
            }
            let _fealty = input.read_u16::<LE>()?;
        }

        if input.version() >= 10.55 {
            assert_eq!(input.read_u8()?, 11);
        }

        // off-map trade
        if input.version() >= 9.17 {
            let mut off_map_trade_route_explored = [0; 20];
            input.read_exact(&mut off_map_trade_route_explored)?;
        }

        if input.version() >= 9.18 {
            let mut off_map_trade_route_being_explored = [0; 20];
            input.read_exact(&mut off_map_trade_route_being_explored)?;
        }

        if input.version() >= 10.55 {
            assert_eq!(input.read_u8()?, 11);
        }

        // market trading
        if input.version() >= 9.22 {
            let _max_trade_amount = input.read_u32::<LE>()?;
            let _old_max_trade_amount = input.read_u32::<LE>()?;
            let _max_trade_limit = input.read_u32::<LE>()?;
            let _current_wood_limit = input.read_u32::<LE>()?;
            let _current_food_limit = input.read_u32::<LE>()?;
            let _current_stone_limit = input.read_u32::<LE>()?;
            let _current_ore_limit = input.read_u32::<LE>()?;
            let _commodity_volume_delta = input.read_i32::<LE>()?;
            let _trade_vig_rate = input.read_f32::<LE>()?;
            let _trade_refresh_timer = input.read_u32::<LE>()?;
            let _trade_refresh_rate = input.read_u32::<LE>()?;
        }

        let _prod_queue_enabled = if input.version() >= 9.67 {
            input.read_u8()? != 0
        } else {
            true
        };

        // ai dodging ability
        if input.version() >= 9.90 {
            let _chance_to_dodge_missiles = input.read_u8()?;
            let _chance_for_archers_to_maintain_distance = input.read_u8()?;
        }

        let _open_gates_for_pathing_count = if input.version() >= 11.42 {
            input.read_u32::<LE>()?
        } else {
            0
        };
        let _farm_queue_count = if input.version() >= 11.57 {
            input.read_u32::<LE>()?
        } else {
            0
        };
        let _nomad_build_lock = if input.version() >= 11.75 {
            input.read_u32::<LE>()? != 0
        } else {
            false
        };

        if input.version() >= 9.30 {
            let _old_kills = input.read_u32::<LE>()?;
            let _old_razings = input.read_u32::<LE>()?;
            let _battle_mode = input.read_u32::<LE>()?;
            let _razings_mode = input.read_u32::<LE>()?;
            let _total_kills = input.read_u32::<LE>()?;
            let _total_razings = input.read_u32::<LE>()?;
        }

        if input.version() >= 9.31 {
            let _old_hit_points = input.read_u32::<LE>()?;
            let _total_hit_points = input.read_u32::<LE>()?;
        }

        if input.version() >= 9.32 {
            let mut old_player_kills = [0; 9];
            input.read_u32_into::<LE>(&mut old_player_kills)?;
        }

        player.tech_tree = if input.version() >= 9.38 {
            Some(TechTree::read_from(&mut *input)?)
        } else {
            None
        };

        if input.version() >= 10.55 {
            assert_eq!(input.read_u8()?, 11);
        }

        let _player_ai = if player.kind == 3 && input.read_u32::<LE>()? == 1 {
            Some(PlayerAI::read_from(input)?)
        } else {
            None
        };

        if input.version() >= 10.55 {
            assert_eq!(input.read_u8()?, 11);
        }

        player.gaia = if player.kind == 2 {
            Some(GaiaData::read_from(input)?)
        } else {
            None
        };

        if input.version() >= 10.55 {
            assert_eq!(input.read_u8()?, 11);
        }

        let num_unit_types = input.read_u32::<LE>()?;
        let mut available_unit_types = vec![false; num_unit_types.try_into().unwrap()];
        for available in &mut available_unit_types {
            *available = input.read_u32::<LE>()? != 0;
        }

        if input.version() >= 10.55 {
            assert_eq!(input.read_u8()?, 11);
        }

        player.unit_types.reserve(available_unit_types.len());
        for available in available_unit_types {
            player.unit_types.push(if available {
                if input.version() >= 10.55 {
                    assert_eq!(input.read_u8()?, 22);
                }
                let ty = CompactUnitType::read_from(input)?;
                if input.version() >= 10.55 {
                    assert_eq!(input.read_u8()?, 33);
                }
                Some(ty)
            } else {
                None
            });
        }

        if input.version() >= 10.55 {
            assert_eq!(input.read_u8()?, 11);
        }

        player.visible_map = VisibleMap::read_from(input)?;
        player.visible_resources = VisibleResources::read_from(input)?;

        if input.version() >= 10.55 {
            assert_eq!(input.read_u8()?, 11);
        }

        player.units = {
            let _list_size = input.read_u32::<LE>()?;
            let _grow_size = input.read_u32::<LE>()?;
            let mut units = vec![];
            while let Some(unit) = Unit::read_from(input)? {
                units.push(unit);
            }
            units
        };

        if input.version() >= 10.55 {
            assert_eq!(input.read_u8()?, 11);
        }

        player.sleeping_units = {
            let _list_size = input.read_u32::<LE>()?;
            let _grow_size = input.read_u32::<LE>()?;
            let mut units = vec![];
            while let Some(unit) = Unit::read_from(input)? {
                units.push(unit);
            }
            units
        };

        if input.version() >= 10.55 {
            assert_eq!(input.read_u8()?, 11);
        }

        player.doppelganger_units = {
            let _list_size = input.read_u32::<LE>()?;
            let _grow_size = input.read_u32::<LE>()?;
            let mut units = vec![];
            while let Some(unit) = Unit::read_from(input)? {
                units.push(unit);
            }
            units
        };

        if input.version() >= 10.55 {
            assert_eq!(input.read_u8()?, 11);
        }

        Ok(player)
    }
}
