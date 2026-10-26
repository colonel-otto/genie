//! The attributes every unit type has: the resource storage, the damage sprites and the static
//! part ([`StaticUnitTypeAttributes`]).

use crate::sound::SoundID;
use crate::sprite::{GraphicID, SpriteID};
use crate::unit_type::{UnitClass, write_opt_string_key};
use arrayvec::ArrayVec;
use byteorder::{LE, ReadBytesExt, WriteBytesExt};
use genie_support::{StringKey, UnitTypeID, read_opt_u16};
use std::convert::TryInto;
use std::io::{Read, Result, Write};

/// A resource a unit stores (e.g. the food in a sheep), with how it is stored.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnitAttribute {
    /// Resource type; 0xFFFF marks an unused slot.
    pub attribute_type: u16,
    /// How much of the resource the unit holds.
    pub amount: f32,
    /// When the resource is stored or released (genieutils' storage `Flag`); not decoded here.
    pub flag: u8,
}

impl UnitAttribute {
    /// Read a resource storage slot from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    pub fn read_from(mut input: impl Read) -> Result<Self> {
        Ok(Self {
            attribute_type: input.read_u16::<LE>()?,
            amount: input.read_f32::<LE>()?,
            flag: input.read_u8()?,
        })
    }

    /// Write this resource storage slot to an output stream.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails.
    pub fn write_to(self, mut output: impl Write) -> Result<()> {
        output.write_u16::<LE>(self.attribute_type)?;
        output.write_f32::<LE>(self.amount)?;
        output.write_u8(self.flag)?;
        Ok(())
    }

    pub(super) fn write_empty(mut output: impl Write) -> Result<()> {
        output.write_u16::<LE>(0xFFFF)?;
        output.write_f32::<LE>(0.0)?;
        output.write_u8(0)?;
        Ok(())
    }
}

/// A sprite drawn on a unit once its damage passes a threshold (e.g. fire on a building).
#[derive(Debug, Default, Clone, Copy)]
pub struct DamageSprite {
    /// The sprite drawn.
    pub sprite: SpriteID,
    /// Damage, in percent of the unit's hit points, from which the sprite is drawn.
    pub damage_percent: u16,
    /// How the sprite is drawn over the unit (genieutils' `ApplyMode`); not decoded here.
    pub flag: u8,
}

impl DamageSprite {
    /// Read a damage sprite from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    pub fn read_from(mut input: impl Read) -> Result<Self> {
        Ok(Self {
            sprite: input.read_u16::<LE>()?.into(),
            damage_percent: input.read_u16::<LE>()?,
            flag: input.read_u8()?,
        })
    }

    /// Write this damage sprite to an output stream.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails.
    pub fn write_to(&self, mut output: impl Write) -> Result<()> {
        output.write_u16::<LE>(self.sprite.into())?;
        output.write_u16::<LE>(self.damage_percent)?;
        output.write_u8(self.flag)?;
        Ok(())
    }
}

/// The attributes every unit type has (genieutils' `Unit` without its class-specific parts).
// The bools are flags of the file format.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Default, Clone)]
pub struct StaticUnitTypeAttributes {
    pub(super) name: String,
    /// ID of this unit type.
    pub id: UnitTypeID,
    /// Language string ID of the unit's name.
    pub string_id: StringKey,
    pub(super) string_id2: Option<StringKey>,
    /// The unit's class (archer, infantry, building, …).
    pub unit_class: UnitClass,
    /// Sprite shown while the unit stands still, or `None`.
    pub standing_sprite_1: Option<SpriteID>,
    /// A second standing sprite (genieutils' second `StandingGraphic`), or `None`.
    pub standing_sprite_2: Option<SpriteID>,
    /// Sprite played when the unit dies, or `None`.
    pub dying_sprite: Option<SpriteID>,
    /// Sprite played after the dying sprite if `undead_flag` is set (genieutils' `UndeadGraphic`),
    /// or `None`.
    pub undead_sprite: Option<SpriteID>,
    /// Whether the unit plays its undead sprite after dying (genieutils' `UndeadMode`).
    pub undead_flag: u8,
    /// Hit points.
    pub hp: u16,
    /// Line of sight, in tiles.
    pub los: f32,
    /// How many units can garrison in this unit.
    pub garrison_capacity: u8,
    /// Collision size (x, y, z), in tiles.
    pub radius: (f32, f32, f32),
    /// Sound played when the unit is created, or `None`.
    pub train_sound: Option<SoundID>,
    /// Sound played when the unit takes damage, or `None`.
    pub damage_sound: Option<SoundID>,
    /// Unit type that replaces this unit when it dies (e.g. its corpse), or `None`.
    pub death_spawn: Option<UnitTypeID>,
    /// genieutils' `SortNumber`; its use is not known yet.
    pub sort_number: u8,
    /// Whether buildings can be placed over this unit.
    pub can_be_built_on: bool,
    /// Icon of the unit's button and selection panel, or `None`.
    pub button_picture: Option<GraphicID>,
    /// Whether the scenario editor hides this unit type.
    pub hide_in_scenario_editor: bool,
    /// Old portrait picture ID (genieutils' `OldPortraitPict`), or `None`.
    pub portrait_picture: Option<GraphicID>,
    /// Whether the unit type is available at the start of a game.
    pub enabled: bool,
    /// genieutils' `Disabled` flag; its effect is not known yet.
    pub disabled: bool,
    /// Two terrains of which one must be next to the unit to place it, -1 for none (genieutils'
    /// `PlacementSideTerrain`).
    pub tile_req: (i16, i16),
    /// Two terrains of which one must be under the unit to place it, -1 for none (genieutils'
    /// `PlacementTerrain`).
    pub center_tile_req: (i16, i16),
    /// Clearance size (x, y): the free area needed to place the unit, in tiles.
    pub construction_radius: (f32, f32),
    /// How the unit can be placed on hills (genieutils' `HillMode`), read as a flag.
    pub elevation_flag: bool,
    /// Whether the unit stays visible under the fog of war (genieutils' `FogVisibility`), read as a
    /// flag.
    pub fog_flag: bool,
    /// The terrain restriction (terrain table) that says where the unit can be.
    pub terrain_restriction_id: u16,
    /// Whether and how the unit flies (genieutils' `FlyMode`).
    pub movement_type: u8,
    /// The most resource the unit can hold (genieutils' `ResourceCapacity`).
    pub attribute_max_amount: u16,
    /// How fast the unit's resource decays (genieutils' `ResourceDecay`).
    pub attribute_rot: f32,
    /// Which area (blast) damage the unit takes (genieutils' `BlastDefenseLevel`).
    pub area_effect_level: u8,
    /// Combat level (genieutils' `CombatLevel`); not decoded here.
    pub combat_level: u8,
    /// How the player can interact with the unit (genieutils' `InteractionMode`).
    pub select_level: u8,
    /// How the unit is shown on the minimap (genieutils' `MinimapMode`).
    pub map_draw_level: u8,
    /// Which interface the unit's selection uses (genieutils' `InterfaceKind`).
    pub unit_level: u8,
    /// genieutils' `MultipleAttributeMode`; its use is not known yet.
    pub multiple_attribute_mod: f32,
    /// Minimap colour (a palette index).
    pub map_color: u8,
    /// Language string ID of the unit's help text.
    pub help_string_id: StringKey,
    /// Language string ID that follows the help text (genieutils' `LanguageDLLHotKeyText`).
    pub help_page_id: u32,
    /// Hotkey ID of the unit's button.
    pub hotkey_id: u32,
    /// genieutils' `Recyclable` flag; its effect is not known yet.
    pub recyclable: bool,
    /// Whether gatherers pick the unit up as a resource on their own (genieutils'
    /// `EnableAutoGather`).
    pub track_as_resource: bool,
    /// Whether a doppelganger (the copy seen under the fog of war) is left when the unit dies.
    pub create_doppleganger: bool,
    /// The group of resources the unit is gathered as (genieutils' `ResourceGatherGroup`).
    pub resource_group: u8,
    /// How the unit is drawn when it is hidden behind others (genieutils' `OcclusionMode`).
    pub occlusion_mask: u8,
    /// Shape of the unit's obstruction (genieutils' `ObstructionType`).
    pub obstruction_type: u8,
    /// The byte genieutils reads as `ObstructionClass`; genie-rs takes it as the selection shape.
    pub selection_shape: u8,
    /// Object flags, stored as a u32 from data version 11.55 on; 0 for older files, which lack
    /// them.
    pub object_flags: u32,
    /// The civilization this unit type belongs to.
    pub civilization: u8,
    /// Not known yet (genieutils' `Nothing`).
    pub attribute_piece: u8,
    /// Size of the selection outline (x, y, z), in tiles.
    pub outline_radius: (f32, f32, f32),
    /// The resources the unit stores, at most 3; unused slots are dropped when reading.
    pub attributes: ArrayVec<UnitAttribute, 3>,
    /// Sprites drawn on the unit as it takes damage.
    pub damage_sprites: Vec<DamageSprite>,
    /// Sound played when the unit is selected, or `None`.
    pub selected_sound: Option<SoundID>,
    /// Sound played when the unit dies, or `None`.
    pub death_sound: Option<SoundID>,
    /// How the unit reacts to attacks (genieutils' `OldAttackReaction`).
    pub attack_reaction: u8,
    /// genieutils' `ConvertTerrain` flag; its effect is not known yet.
    pub convert_terrain_flag: u8,
    /// ID of the unit type this one copies; newer files also use it to pick the unit's task list
    /// (see [`ActionUnitTypeAttributes::tasks`]).
    ///
    /// [`ActionUnitTypeAttributes::tasks`]: crate::unit_type::movers::ActionUnitTypeAttributes::tasks
    pub copy_id: u16,
    /// ID of the unit's base unit type (genieutils' `BaseID`).
    pub unit_group: u16,
}

impl StaticUnitTypeAttributes {
    /// Read the static attributes from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails, or if an ID is out of range.
    ///
    /// # Panics
    ///
    /// Panics if the name is not valid UTF-8.
    pub fn read_from(mut input: impl Read, version: f32) -> Result<Self> {
        let mut unit_type = Self::default();
        let name_len = input.read_u16::<LE>()?;
        unit_type.id = input.read_u16::<LE>()?.into();
        unit_type.string_id = input.read_u16::<LE>()?.into();
        unit_type.string_id2 = read_opt_u16(&mut input)?;
        unit_type.unit_class = input.read_u16::<LE>()?;
        unit_type.standing_sprite_1 = read_opt_u16(&mut input)?;
        unit_type.standing_sprite_2 = read_opt_u16(&mut input)?;
        unit_type.dying_sprite = read_opt_u16(&mut input)?;
        unit_type.undead_sprite = read_opt_u16(&mut input)?;
        unit_type.undead_flag = input.read_u8()?;
        unit_type.hp = input.read_u16::<LE>()?;
        unit_type.los = input.read_f32::<LE>()?;
        unit_type.garrison_capacity = input.read_u8()?;
        unit_type.radius = (
            input.read_f32::<LE>()?,
            input.read_f32::<LE>()?,
            input.read_f32::<LE>()?,
        );
        unit_type.train_sound = read_opt_u16(&mut input)?;
        unit_type.damage_sound = read_opt_u16(&mut input)?;
        unit_type.death_spawn = read_opt_u16(&mut input)?;
        unit_type.sort_number = input.read_u8()?;
        unit_type.can_be_built_on = input.read_u8()? != 0;
        unit_type.button_picture = read_opt_u16(&mut input)?;
        unit_type.hide_in_scenario_editor = input.read_u8()? != 0;
        unit_type.portrait_picture = read_opt_u16(&mut input)?;
        unit_type.enabled = input.read_u8()? != 0;
        unit_type.disabled = input.read_u8()? != 0;
        unit_type.tile_req = (input.read_i16::<LE>()?, input.read_i16::<LE>()?);
        unit_type.center_tile_req = (input.read_i16::<LE>()?, input.read_i16::<LE>()?);
        unit_type.construction_radius = (input.read_f32::<LE>()?, input.read_f32::<LE>()?);
        unit_type.elevation_flag = input.read_u8()? != 0;
        unit_type.fog_flag = input.read_u8()? != 0;
        unit_type.terrain_restriction_id = input.read_u16::<LE>()?;
        unit_type.movement_type = input.read_u8()?;
        unit_type.attribute_max_amount = input.read_u16::<LE>()?;
        unit_type.attribute_rot = input.read_f32::<LE>()?;
        unit_type.area_effect_level = input.read_u8()?;
        unit_type.combat_level = input.read_u8()?;
        unit_type.select_level = input.read_u8()?;
        unit_type.map_draw_level = input.read_u8()?;
        unit_type.unit_level = input.read_u8()?;
        unit_type.multiple_attribute_mod = input.read_f32::<LE>()?;
        unit_type.map_color = input.read_u8()?;
        unit_type.help_string_id = input.read_u32::<LE>()?.into();
        unit_type.help_page_id = input.read_u32::<LE>()?;
        unit_type.hotkey_id = input.read_u32::<LE>()?;
        unit_type.recyclable = input.read_u8()? != 0;
        unit_type.track_as_resource = input.read_u8()? != 0;
        unit_type.create_doppleganger = input.read_u8()? != 0;
        unit_type.resource_group = input.read_u8()?;
        unit_type.occlusion_mask = input.read_u8()?;
        unit_type.obstruction_type = input.read_u8()?;
        unit_type.selection_shape = input.read_u8()?;
        unit_type.object_flags = if version < 11.55 {
            0
        } else {
            input.read_u32::<LE>()?
        };
        unit_type.civilization = input.read_u8()?;
        unit_type.attribute_piece = input.read_u8()?;
        unit_type.outline_radius = (
            input.read_f32::<LE>()?,
            input.read_f32::<LE>()?,
            input.read_f32::<LE>()?,
        );
        for _ in 0..3 {
            let attr = UnitAttribute::read_from(&mut input)?;
            if attr.attribute_type != 0xFFFF {
                unit_type.attributes.push(attr);
            }
        }
        unit_type.damage_sprites = {
            let num_damage_sprites = input.read_u8()?;
            let mut damage_sprites = vec![];
            for _ in 0..num_damage_sprites {
                damage_sprites.push(DamageSprite::read_from(&mut input)?);
            }
            damage_sprites
        };
        unit_type.selected_sound = read_opt_u16(&mut input)?;
        unit_type.death_sound = read_opt_u16(&mut input)?;
        unit_type.attack_reaction = input.read_u8()?;
        unit_type.convert_terrain_flag = input.read_u8()?;
        unit_type.name = {
            // TODO use not-UTF8 for the name
            let mut bytes = vec![0; usize::from(name_len)];
            input.read_exact(&mut bytes)?;
            String::from_utf8(bytes.iter().copied().take_while(|b| *b != 0).collect()).unwrap()
        };
        unit_type.copy_id = input.read_u16::<LE>()?;
        unit_type.unit_group = input.read_u16::<LE>()?;
        Ok(unit_type)
    }

    /// Write this unit type to an output stream.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails, or if the second string ID does not fit an
    /// `i16`.
    ///
    /// # Panics
    ///
    /// Panics if the name or help string key is a name rather than a number or does not fit the
    /// format's field, if a sprite, sound or unit ID does not fit an `i16`, or if there are more
    /// than 255 damage sprites.
    // The format stores the name length as a u16 (truncated, as before).
    #[allow(clippy::cast_possible_truncation)]
    pub fn write_to(&self, mut output: impl Write, _version: f32) -> Result<()> {
        // TODO use not-UTF8 for the name
        output.write_u16::<LE>(self.name.len() as u16)?;
        output.write_u16::<LE>(self.id.into())?;
        output.write_i16::<LE>((&self.string_id).try_into().unwrap())?;
        write_opt_string_key(&mut output, self.string_id2.as_ref())?;
        output.write_u16::<LE>(self.unit_class)?;
        output.write_i16::<LE>(
            self.standing_sprite_1
                .map_or(-1, |id| id.try_into().unwrap()),
        )?;
        output.write_i16::<LE>(
            self.standing_sprite_2
                .map_or(-1, |id| id.try_into().unwrap()),
        )?;
        output.write_i16::<LE>(self.dying_sprite.map_or(-1, |id| id.try_into().unwrap()))?;
        output.write_i16::<LE>(self.undead_sprite.map_or(-1, |id| id.try_into().unwrap()))?;
        output.write_u8(self.undead_flag)?;
        output.write_u16::<LE>(self.hp)?;
        output.write_f32::<LE>(self.los)?;
        output.write_u8(self.garrison_capacity)?;
        output.write_f32::<LE>(self.radius.0)?;
        output.write_f32::<LE>(self.radius.1)?;
        output.write_f32::<LE>(self.radius.2)?;
        output.write_i16::<LE>(self.train_sound.map_or(-1, |id| id.try_into().unwrap()))?;
        output.write_i16::<LE>(self.damage_sound.map_or(-1, |id| id.try_into().unwrap()))?;
        output.write_i16::<LE>(self.death_spawn.map_or(-1, |id| id.try_into().unwrap()))?;
        output.write_u8(self.sort_number)?;
        output.write_u8(u8::from(self.can_be_built_on))?;
        output.write_i16::<LE>(self.button_picture.map_or(-1, |id| id.try_into().unwrap()))?;
        output.write_u8(u8::from(self.hide_in_scenario_editor))?;
        output.write_i16::<LE>(
            self.portrait_picture
                .map_or(-1, |id| id.try_into().unwrap()),
        )?;
        output.write_u8(u8::from(self.enabled))?;
        output.write_u8(u8::from(self.disabled))?;
        output.write_i16::<LE>(self.tile_req.0)?;
        output.write_i16::<LE>(self.tile_req.1)?;
        output.write_i16::<LE>(self.center_tile_req.0)?;
        output.write_i16::<LE>(self.center_tile_req.1)?;
        output.write_f32::<LE>(self.construction_radius.0)?;
        output.write_f32::<LE>(self.construction_radius.1)?;
        output.write_u8(u8::from(self.elevation_flag))?;
        output.write_u8(u8::from(self.fog_flag))?;
        output.write_u16::<LE>(self.terrain_restriction_id)?;
        output.write_u8(self.movement_type)?;
        output.write_u16::<LE>(self.attribute_max_amount)?;
        output.write_f32::<LE>(self.attribute_rot)?;
        output.write_u8(self.area_effect_level)?;
        output.write_u8(self.combat_level)?;
        output.write_u8(self.select_level)?;
        output.write_u8(self.map_draw_level)?;
        output.write_u8(self.unit_level)?;
        output.write_f32::<LE>(self.multiple_attribute_mod)?;
        output.write_u8(self.map_color)?;
        output.write_u32::<LE>((&self.help_string_id).try_into().unwrap())?;
        output.write_u32::<LE>(self.help_page_id)?;
        output.write_u32::<LE>(self.hotkey_id)?;
        output.write_u8(u8::from(self.recyclable))?;
        output.write_u8(u8::from(self.track_as_resource))?;
        output.write_u8(u8::from(self.create_doppleganger))?;
        output.write_u8(self.resource_group)?;
        output.write_u8(self.occlusion_mask)?;
        output.write_u8(self.obstruction_type)?;
        output.write_u8(self.selection_shape)?;
        output.write_u32::<LE>(self.object_flags)?;
        output.write_u8(self.civilization)?;
        output.write_u8(self.attribute_piece)?;
        output.write_f32::<LE>(self.outline_radius.0)?;
        output.write_f32::<LE>(self.outline_radius.1)?;
        output.write_f32::<LE>(self.outline_radius.2)?;
        for index in 0..self.attributes.capacity() {
            match self.attributes.get(index) {
                Some(attr) => attr.write_to(&mut output)?,
                None => UnitAttribute::write_empty(&mut output)?,
            }
        }
        output.write_u8(self.damage_sprites.len().try_into().unwrap())?;
        for sprite in &self.damage_sprites {
            sprite.write_to(&mut output)?;
        }
        output.write_i16::<LE>(self.selected_sound.map_or(-1, |id| id.try_into().unwrap()))?;
        output.write_i16::<LE>(self.death_sound.map_or(-1, |id| id.try_into().unwrap()))?;
        output.write_u8(self.attack_reaction)?;
        output.write_u8(self.convert_terrain_flag)?;
        output.write_all(self.name.as_bytes())?;
        output.write_u16::<LE>(self.copy_id)?;
        output.write_u16::<LE>(self.unit_group)?;
        Ok(())
    }
}
