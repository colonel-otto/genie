use crate::ai::PlayerAI;
use crate::element::{OptionalReadableElement, ReadableHeaderElement, WritableHeaderElement};
use crate::reader::RecordingHeaderReader;
use crate::unit::Unit;
use crate::unit_type::CompactUnitType;
use crate::{ObjectID, PlayerID, Result};
use byteorder::{LE, ReadBytesExt, WriteBytesExt};
use genie_dat::CivilizationID;
use genie_dat::tech_tree::TechTree;
use genie_scx::VictoryConditions;
use genie_support::{ReadStringsExt, read_opt_u32};
use std::convert::TryInto;
use std::io::{Read, Write};

/// A player's state in the recording header: settings, resources (attributes), technologies,
/// unit types, visibility and units.
#[derive(Debug, Default, Clone)]
pub struct Player {
    /// The player type: 2 is Gaia, 3 a computer (AI) player.
    kind: u8,
    relations: Vec<u8>,
    diplomacy: [u32; 9],
    allied_los: bool,
    allied_victory: bool,
    name: String,
    /// The player's attributes (resources): food, wood, stone, gold, population …, indexed by
    /// attribute id.
    pub attributes: Vec<f32>,
    initial_view: (f32, f32),
    saved_views: Vec<(f32, f32)>,
    spawn_location: (u16, u16),
    culture_id: u8,
    /// The player's civilization.
    pub civilization_id: CivilizationID,
    game_status: u8,
    resigned: bool,
    /// `UserPatch` 1.5's extra player data (save version 11.97 only).
    pub userpatch_data: Option<UserPatchData>,
    /// The state of each technology for this player.
    pub tech_state: PlayerTech,
    /// The player's population history and statistics.
    pub history_info: HistoryInfo,
    /// The player's tech tree, from save version 9.38 on.
    pub tech_tree: Option<TechTree>,
    /// Gaia's extra data (creature growth, wolf attacks); only for the Gaia player.
    pub gaia: Option<GaiaData>,
    /// The player's copy of each unit type, by unit type id; `None` for the ones it does not have.
    pub unit_types: Vec<Option<CompactUnitType>>,
    /// Which tiles the player has explored and sees.
    pub visible_map: VisibleMap,
    /// The resource objects the player knows of.
    pub visible_resources: VisibleResources,
    /// The player's active units.
    pub units: Vec<Unit>,
    /// The player's sleeping (inactive) units.
    pub sleeping_units: Vec<Unit>,
    /// The player's doppelganger objects: the stand-ins for objects seen under the fog of war.
    pub doppelganger_units: Vec<Unit>,
    /// The player's victory conditions.
    pub victory: VictoryConditions,
}

impl Player {
    /// Return the name of this player.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Read the player's victory conditions, which the header stores after all players.
    ///
    /// # Errors
    ///
    /// Returns an error if they cannot be read.
    pub fn read_info<R: Read>(&mut self, input: &mut RecordingHeaderReader<R>) -> Result<()> {
        self.victory = VictoryConditions::read_from(input, true)?;
        Ok(())
    }
}

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

/// A player's view of the map: which tiles it has explored and currently sees.
#[derive(Debug, Default, Clone)]
pub struct VisibleMap {
    /// The map width in tiles.
    pub width: u32,
    /// The map height in tiles.
    pub height: u32,
    /// The number of tiles the player has explored; 0 before save version 6.70.
    pub explored_tiles_count: u32,
    /// The player the map belongs to.
    pub player_id: PlayerID,
    /// The visibility of each tile, row by row.
    pub tiles: Vec<i8>,
}

impl ReadableHeaderElement for VisibleMap {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        let mut map = Self {
            width: input.read_u32::<LE>()?,
            height: input.read_u32::<LE>()?,
            ..Default::default()
        };
        if input.version() >= 6.70 {
            map.explored_tiles_count = input.read_u32::<LE>()?;
        }
        map.player_id = input.read_u16::<LE>()?.try_into().unwrap();
        map.tiles = vec![0; (map.width * map.height).try_into().unwrap()];
        input.read_i8_into(&mut map.tiles)?;
        Ok(map)
    }
}

/// A resource object a player knows of.
#[derive(Debug, Default, Clone, Copy)]
pub struct VisibleResource {
    /// The resource object.
    pub object_id: ObjectID,
    /// Its distance, in tiles; the point it is measured from is not known yet.
    pub distance: u8,
    /// The zone it is in.
    pub zone: i8,
    /// Its tile position (x, y).
    pub location: (u8, u8),
}

impl ReadableHeaderElement for VisibleResource {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        Ok(Self {
            object_id: input.read_u32::<LE>()?.into(),
            distance: input.read_u8()?,
            zone: input.read_i8()?,
            location: (input.read_u8()?, input.read_u8()?),
        })
    }
}

/// The resource objects a player knows of, in several lists (probably one per resource kind).
#[allow(dead_code)]
#[derive(Debug, Default, Clone)]
pub struct VisibleResources {
    lists: Vec<Vec<VisibleResource>>,
}

impl ReadableHeaderElement for VisibleResources {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        let num_lists = input.read_u32::<LE>()?;
        let mut sizes = vec![];
        for _ in 0..num_lists {
            let _capacity = input.read_u32::<LE>()?;
            sizes.push(input.read_u32::<LE>()?);
        }
        let mut lists = Vec::with_capacity(sizes.len());
        for size in sizes {
            let mut list = Vec::with_capacity(size.try_into().unwrap());
            for _ in 0..size {
                list.push(VisibleResource::read_from(input)?);
            }
            lists.push(list);
        }
        Ok(Self { lists })
    }
}

/// Gaia's extra state: creature (animal) growth and the wolf attacks on villagers.
#[derive(Debug, Default, Clone)]
pub struct GaiaData {
    #[allow(dead_code)]
    update_time: u32,
    #[allow(dead_code)]
    update_nature: u32,
    creatures: [GaiaCreature; 5],
    next_wolf_attack_update_time: u32,
    wolf_attack_update_interval: u32,
    wolf_attack_stop_time: u32,
    min_villager_distance: f32,
    tc_positions: [(f32, f32); 9],
    wolf_current_player: u32,
    wolf_current_villagers: [u32; 10],
    wolf_current_villager: Option<ObjectID>,
    wolf_villager_count: u32,
    wolves: [GaiaWolfInfo; 25],
    current_wolf: Option<ObjectID>,
    wolf_counts: [u32; 10],
}

impl ReadableHeaderElement for GaiaData {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        let mut gaia = Self {
            update_time: input.read_u32::<LE>()?,
            update_nature: input.read_u32::<LE>()?,
            ..Default::default()
        };
        for creature in &mut gaia.creatures {
            *creature = GaiaCreature::read_from(input)?;
        }
        gaia.next_wolf_attack_update_time = input.read_u32::<LE>()?;
        gaia.wolf_attack_update_interval = input.read_u32::<LE>()?;
        gaia.wolf_attack_stop_time = input.read_u32::<LE>()?;
        gaia.min_villager_distance = input.read_f32::<LE>()?;
        for pos in &mut gaia.tc_positions {
            pos.0 = input.read_f32::<LE>()?;
        }
        for pos in &mut gaia.tc_positions {
            pos.1 = input.read_f32::<LE>()?;
        }
        gaia.wolf_current_player = input.read_u32::<LE>()?;
        for v in &mut gaia.wolf_current_villagers {
            *v = input.read_u32::<LE>()?;
        }
        gaia.wolf_current_villager = read_opt_u32(input)?;
        gaia.wolf_villager_count = input.read_u32::<LE>()?;
        for wolf in &mut gaia.wolves {
            *wolf = GaiaWolfInfo::read_from(input)?;
        }
        gaia.current_wolf = read_opt_u32(input)?;
        input.read_u32_into::<LE>(&mut gaia.wolf_counts[..])?;
        Ok(gaia)
    }
}

/// The growth of one kind of Gaia creature (animal).
#[derive(Debug, Default, Clone, Copy)]
pub struct GaiaCreature {
    /// How fast the creatures grow in number.
    pub growth_rate: f32,
    /// The fraction of growth carried over to the next update.
    pub remainder: f32,
    /// The most creatures of this kind there can be.
    pub max: u32,
}

impl ReadableHeaderElement for GaiaCreature {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        Ok(Self {
            growth_rate: input.read_f32::<LE>()?,
            remainder: input.read_f32::<LE>()?,
            max: input.read_u32::<LE>()?,
        })
    }
}

impl WritableHeaderElement for GaiaCreature {
    fn write_to<W: Write>(&self, output: &mut W) -> Result<()> {
        output.write_f32::<LE>(self.growth_rate)?;
        output.write_f32::<LE>(self.remainder)?;
        output.write_u32::<LE>(self.max)?;
        Ok(())
    }
}

/// A wolf that takes part in Gaia's wolf attacks.
#[derive(Debug, Default, Clone, Copy)]
pub struct GaiaWolfInfo {
    /// The wolf's object id.
    pub id: u32,
    /// Its distance to its target villager.
    pub distance: f32,
}

impl ReadableHeaderElement for GaiaWolfInfo {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        Ok(Self {
            id: input.read_u32::<LE>()?,
            distance: input.read_f32::<LE>()?,
        })
    }
}

impl WritableHeaderElement for GaiaWolfInfo {
    fn write_to<W: Write>(&self, output: &mut W) -> Result<()> {
        output.write_u32::<LE>(self.id)?;
        output.write_f32::<LE>(self.distance)?;
        Ok(())
    }
}

#[allow(dead_code)]
#[derive(Debug, Default, Clone)]
struct DiplomacyOffer {
    sequence: u8,
    started_by: u8,
    actual_time: u32,
    game_time: u32,
    declare: u8,
    old_diplomacy: u8,
    new_diplomacy: u8,
    old_intelligence: u8,
    new_intelligence: u8,
    old_trade: u8,
    new_trade: u8,
    demand: u8,
    gold: u32,
    message: Option<String>,
    status: u8,
}

impl ReadableHeaderElement for DiplomacyOffer {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        let mut offer = Self {
            sequence: input.read_u8()?,
            started_by: input.read_u8()?,
            actual_time: 0,
            game_time: input.read_u32::<LE>()?,
            declare: input.read_u8()?,
            old_diplomacy: input.read_u8()?,
            new_diplomacy: input.read_u8()?,
            old_intelligence: input.read_u8()?,
            new_intelligence: input.read_u8()?,
            old_trade: input.read_u8()?,
            new_trade: input.read_u8()?,
            demand: input.read_u8()?,
            gold: input.read_u32::<LE>()?,
            ..Default::default()
        };
        let message_len = input.read_u8()?;
        offer.message = input.read_str(usize::from(message_len))?;
        offer.status = input.read_u8()?;
        Ok(offer)
    }
}

/// A player's history, the data of the post-game timeline.
#[derive(Debug, Default, Clone)]
pub struct HistoryInfo {
    /// The population at each history update.
    pub entries: Vec<HistoryEntry>,
    /// The events (age advances, battles …) on the timeline.
    pub events: Vec<HistoryEvent>,
}

impl ReadableHeaderElement for HistoryInfo {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        let _padding = input.read_u8()?;
        let num_entries = input.read_u32::<LE>()?;
        let _num_events = input.read_u32::<LE>()?;
        let entries_capacity = input.read_u32::<LE>()?;
        let mut entries = Vec::with_capacity(entries_capacity.try_into().unwrap());
        for _ in 0..num_entries {
            entries.push(HistoryEntry::read_from(input)?);
        }

        let _padding = input.read_u8()?;

        let num_events = input.read_u32::<LE>()?;
        let mut events = Vec::with_capacity(num_events.try_into().unwrap());
        for _ in 0..num_events {
            events.push(HistoryEvent::read_from(input)?);
        }

        let _razings = input.read_i32::<LE>()?;
        let _hit_points_razed = input.read_i32::<LE>()?;
        let _razed_by_others = input.read_i32::<LE>()?;
        let _hit_points_razed_by_others = input.read_i32::<LE>()?;
        let _kills = input.read_i32::<LE>()?;
        let _hit_points_killed = input.read_i32::<LE>()?;
        let _killed_by_others = input.read_i32::<LE>()?;
        let _hit_points_killed_by_others = input.read_i32::<LE>()?;
        let _razings_weight = input.read_i32::<LE>()?;
        let _kills_weight = input.read_i32::<LE>()?;
        let _razings_percent = input.read_i32::<LE>()?;
        let _kills_percent = input.read_i32::<LE>()?;
        let _razing_mode = input.read_i32::<LE>()?;
        let _battle_mode = input.read_i32::<LE>()?;
        let _update_count = input.read_i32::<LE>()?;
        let _old_current_units_created = input.read_i32::<LE>()?;
        let _old_current_buildings_built = input.read_i32::<LE>()?;
        let mut old_kills = [0; 8];
        input.read_u16_into::<LE>(&mut old_kills[..])?;
        let mut old_kill_bvs = [0; 8];
        input.read_u32_into::<LE>(&mut old_kill_bvs[..])?;
        let mut old_razings = [0; 8];
        input.read_u16_into::<LE>(&mut old_razings[..])?;
        let mut old_razing_bvs = [0; 8];
        input.read_u32_into::<LE>(&mut old_razing_bvs[..])?;
        let _running_average_bv_percent = input.read_i32::<LE>()?;
        let _running_total_bv_kills = input.read_i32::<LE>()?;
        let _running_total_bv_razings = input.read_i32::<LE>()?;
        let _running_total_kills = input.read_i16::<LE>()?;
        let _running_total_razings = input.read_i16::<LE>()?;

        let _padding = input.read_u8()?;

        Ok(Self { entries, events })
    }
}

/// An event on a player's timeline.
#[derive(Debug, Default, Clone, Copy)]
pub struct HistoryEvent {
    /// The kind of event.
    pub event_type: i8,
    /// The history update the event falls into.
    pub time_slice: u32,
    /// The game time of the event.
    pub world_time: u32,
    /// The event's three parameters; their meaning depends on the event type.
    pub params: (f32, f32, f32),
}

impl ReadableHeaderElement for HistoryEvent {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        Ok(Self {
            event_type: input.read_i8()?,
            time_slice: input.read_u32::<LE>()?,
            world_time: input.read_u32::<LE>()?,
            params: (
                input.read_f32::<LE>()?,
                input.read_f32::<LE>()?,
                input.read_f32::<LE>()?,
            ),
        })
    }
}

/// A player's population at one history update.
#[derive(Debug, Default, Clone, Copy)]
pub struct HistoryEntry {
    /// The number of civilian units.
    pub civilian_population: u16,
    /// The number of military units.
    pub military_population: u16,
}

impl ReadableHeaderElement for HistoryEntry {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        let civilian_population = input.read_u16::<LE>()?;
        let military_population = input.read_u16::<LE>()?;
        Ok(Self {
            civilian_population,
            military_population,
        })
    }
}

/// The state of one technology for a player.
#[derive(Debug, Default, Clone, Copy)]
pub struct TechState {
    /// How far the research has progressed.
    pub progress: f32,
    /// The research state (unavailable, available, researching, done …).
    pub state: i16,
    /// Changes to the technology's three costs.
    pub modifiers: (i16, i16, i16),
    /// The change to its research time.
    pub time_modifier: i16,
}

impl ReadableHeaderElement for TechState {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        Ok(Self {
            progress: input.read_f32::<LE>()?,
            state: input.read_i16::<LE>()?,
            modifiers: (
                input.read_i16::<LE>()?,
                input.read_i16::<LE>()?,
                input.read_i16::<LE>()?,
            ),
            time_modifier: input.read_i16::<LE>()?,
        })
    }
}

/// The state of every technology for a player.
#[derive(Debug, Default, Clone)]
pub struct PlayerTech {
    /// The state of each technology, by technology id.
    pub tech_states: Vec<TechState>,
}

impl ReadableHeaderElement for PlayerTech {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        let num_techs = input.read_u16::<LE>()?;
        let mut tech_states = Vec::with_capacity(usize::from(num_techs));
        for _ in 0..num_techs {
            tech_states.push(TechState::read_from(input)?);
        }
        Ok(Self { tech_states })
    }
}

/// `UserPatch` 1.5's extra player data. Read past but not kept yet.
#[derive(Debug, Clone, Copy)]
pub struct UserPatchData {}

impl ReadableHeaderElement for UserPatchData {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        {
            let mut bytes = vec![0; 4080];
            input.read_exact(&mut bytes)?;
        }

        let mut category_priorities = vec![0; 900];
        let mut group_priorities = vec![0; 100];

        input.read_u16_into::<LE>(&mut category_priorities)?;
        input.read_u16_into::<LE>(&mut group_priorities)?;

        {
            let mut bytes = vec![0; 2096];
            input.read_exact(&mut bytes)?;
        }

        Ok(Self {})
    }
}
