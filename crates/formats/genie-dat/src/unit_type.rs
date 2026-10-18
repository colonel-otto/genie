//! Types related to unit types.

use crate::sound::SoundID;
use crate::sprite::{GraphicID, SpriteID};
use crate::task::TaskList;
use crate::terrain::TerrainID;
use arrayvec::ArrayVec;
use byteorder::{LE, ReadBytesExt, WriteBytesExt};
pub use genie_support::UnitTypeID;
use genie_support::{MapInto, StringKey, TechID, read_opt_u16, read_opt_u32};
use std::cmp::{Ordering, PartialOrd};
use std::convert::{TryFrom, TryInto};
use std::io::{self, Read, Result, Write};

/// The base class of a unit indicates which data is available for that unit type.
///
/// # Comparison
/// This type implements a comparison operator. A base class value is greater-than-or-equal-to
/// another value if the base class "inherits" from the other value. For example, the
/// `Doppelganger` base class inherits from the `Animated` base class. No other class inherits from
/// `Doppelganger`. Therefore, it compares like this:
///
/// ```rust
/// # use genie_dat::unit_type::UnitBaseClass;
/// assert!(UnitBaseClass::Doppelganger > UnitBaseClass::Animated);
/// assert!(UnitBaseClass::Doppelganger == UnitBaseClass::Doppelganger);
/// assert_eq!(UnitBaseClass::Doppelganger < UnitBaseClass::Moving, false);
/// assert_eq!(UnitBaseClass::Doppelganger > UnitBaseClass::Moving, false);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum UnitBaseClass {
    /// The base unit type, for units that do not do anything.
    Static = 10,
    /// Unit type that supports animated sprites.
    Animated = 20,
    /// Unit type for the "fake" units you see in the fog of war, after the actual unit has been
    /// destroyed.
    Doppelganger = 25,
    /// Unit type that supports movement.
    Moving = 30,
    /// Unit type that supports being tasked by a player.
    Action = 40,
    /// Unit type that supports combat.
    BaseCombat = 50,
    /// Unit type for projectiles/missiles/arrows.
    Missile = 60,
    /// Unit type that supports combat (with additional Age of Empires specific data).
    Combat = 70,
    /// Unit type for buildings.
    Building = 80,
    /// The tree unit type.
    Tree = 90,
}

impl PartialOrd for UnitBaseClass {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        if self == other {
            return Some(Ordering::Equal);
        }

        let self_n = *self as u8;
        let other_n = *other as u8;

        // handle weird leaves specially
        match self {
            Self::Doppelganger | Self::Missile => {
                if self_n > other_n {
                    Some(Ordering::Greater)
                } else {
                    None
                }
            }
            Self::Tree => match other {
                Self::Static => Some(Ordering::Greater),
                _ => None,
            },
            _ => match other {
                Self::Doppelganger | Self::Missile => {
                    if self_n < other_n {
                        Some(Ordering::Less)
                    } else {
                        None
                    }
                }
                Self::Tree => match self {
                    Self::Static => Some(Ordering::Less),
                    _ => None,
                },
                _ => Some(self_n.cmp(&other_n)),
            },
        }
    }
}

/// An unexpected unit base class was found.
#[derive(Debug, Clone, Copy, thiserror::Error)]
#[error("unknown unit base class: {}", .0)]
pub struct ParseUnitBaseClassError(u8);

impl TryFrom<u8> for UnitBaseClass {
    type Error = ParseUnitBaseClassError;

    fn try_from(n: u8) -> std::result::Result<Self, Self::Error> {
        match n {
            10 => Ok(Self::Static),
            20 => Ok(Self::Animated),
            25 => Ok(Self::Doppelganger),
            30 => Ok(Self::Moving),
            40 => Ok(Self::Action),
            50 => Ok(Self::BaseCombat),
            60 => Ok(Self::Missile),
            70 => Ok(Self::Combat),
            80 => Ok(Self::Building),
            90 => Ok(Self::Tree),
            n => Err(ParseUnitBaseClassError(n)),
        }
    }
}

impl From<UnitBaseClass> for u8 {
    fn from(class: UnitBaseClass) -> Self {
        class as Self
    }
}

/// A unit class, a group identifier for runtime behaviours.
pub type UnitClass = u16;

/// Data for a unit type.
///
/// Unit types have a [base class][] identifier that indicates which data is available for that
/// unit type. Data is split up into several `*Attributes` structs dictated by the unit's base class.
///
/// When editing the unit base class, the available attributes must also be updated. Failing to
/// keep the two in sync will cause a panic if you try to write the unit type data to a file or
/// other output.
///
/// [base class]: ./enum.UnitBaseClass.html
#[derive(Debug, Clone)]
pub struct UnitType {
    /// The base class for this unit type.
    pub unit_base_class: UnitBaseClass,
    /// The static unit type attributes: these are always available.
    pub static_: StaticUnitTypeAttributes,
    /// Animated unit type attributes, available if `self.unit_base_class >= UnitBaseClass::Animated`.
    pub animated: Option<AnimatedUnitTypeAttributes>,
    /// Moving unit type attributes, available if `self.unit_base_class >= UnitBaseClass::Moving`.
    pub moving: Option<MovingUnitTypeAttributes>,
    /// Action unit type attributes, available if `self.unit_base_class >= UnitBaseClass::Action`.
    pub action: Option<ActionUnitTypeAttributes>,
    /// `BaseCombat` unit type attributes, available if `self.unit_base_class >=
    /// UnitBaseClass::BaseCombat`.
    pub base_combat: Option<BaseCombatUnitTypeAttributes>,
    /// Missile unit type attributes, available if `self.unit_base_class >= UnitBaseClass::Missile`.
    pub missile: Option<MissileUnitTypeAttributes>,
    /// Combat unit type attributes, available if `self.unit_base_class >= UnitBaseClass::Combat`.
    pub combat: Option<CombatUnitTypeAttributes>,
    /// Building unit type attributes, available if `self.unit_base_class >=
    /// UnitBaseClass::Building`.
    pub building: Option<BuildingUnitTypeAttributes>,
}

impl UnitType {
    /// Read a unit type from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails, or if an ID is out of range.
    ///
    /// # Panics
    ///
    /// Panics if the base class is not a known [`UnitBaseClass`], if the name is not valid UTF-8,
    /// or if the garrison sprite ID does not fit a [`SpriteID`].
    pub fn read_from(mut input: impl Read, version: f32) -> Result<Self> {
        let unit_base_class = input.read_u8()?.try_into().unwrap();
        let static_ = StaticUnitTypeAttributes::read_from(&mut input, version)?;
        let mut unit = Self {
            unit_base_class,
            static_,
            animated: None,
            moving: None,
            action: None,
            base_combat: None,
            missile: None,
            combat: None,
            building: None,
        };
        if unit_base_class >= UnitBaseClass::Animated {
            unit.animated = Some(AnimatedUnitTypeAttributes::read_from(&mut input, version)?);
        }
        if unit_base_class >= UnitBaseClass::Moving {
            unit.moving = Some(MovingUnitTypeAttributes::read_from(&mut input, version)?);
        }
        if unit_base_class >= UnitBaseClass::Action {
            unit.action = Some(ActionUnitTypeAttributes::read_from(&mut input, version)?);
        }
        if unit_base_class >= UnitBaseClass::BaseCombat {
            unit.base_combat = Some(BaseCombatUnitTypeAttributes::read_from(
                &mut input, version,
            )?);
        }
        if unit_base_class >= UnitBaseClass::Missile {
            unit.missile = Some(MissileUnitTypeAttributes::read_from(&mut input, version)?);
        }
        if unit_base_class >= UnitBaseClass::Combat {
            unit.combat = Some(CombatUnitTypeAttributes::read_from(&mut input, version)?);
        }
        if unit_base_class >= UnitBaseClass::Building {
            unit.building = Some(BuildingUnitTypeAttributes::read_from(&mut input, version)?);
        }
        Ok(unit)
    }

    /// Write this unit type to an output stream.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails.
    ///
    /// # Panics
    /// This function panics when trying to write a unit type whose `unit_base_class` property does
    /// not match the available data attributes. For example, when `self.unit_base_class` is
    /// `UnitBaseClass::Animated`, but `self.animated` is `None`.
    pub fn write_to(&self, mut output: impl Write, version: f32) -> Result<()> {
        output.write_u8(self.unit_base_class.into())?;

        self.static_.write_to(&mut output, version)?;

        if self.unit_base_class >= UnitBaseClass::Animated {
            self.animated
                .as_ref()
                .expect("Unit's base class was Animated, but it has no Animated attributes")
                .write_to(&mut output, version)?;
        } else {
            assert!(
                self.animated.is_none(),
                "Unexpected Animated attributes in a unit type whose base class does not support it"
            );
        }

        if self.unit_base_class >= UnitBaseClass::Moving {
            self.moving
                .as_ref()
                .expect("Unit's base class was Moving, but it has no Moving attributes")
                .write_to(&mut output, version)?;
        } else {
            assert!(
                self.moving.is_none(),
                "Unexpected Moving attributes in a unit type whose base class does not support it"
            );
        }

        if self.unit_base_class >= UnitBaseClass::Action {
            self.action
                .as_ref()
                .expect("Unit's base class was Action, but it has no Action attributes")
                .write_to(&mut output, version)?;
        } else {
            assert!(
                self.action.is_none(),
                "Unexpected Action attributes in a unit type whose base class does not support it"
            );
        }

        if self.unit_base_class >= UnitBaseClass::BaseCombat {
            self.base_combat
                .as_ref()
                .expect("Unit's base class was BaseCombat, but it has no BaseCombat attributes")
                .write_to(&mut output, version)?;
        } else {
            assert!(
                self.base_combat.is_none(),
                "Unexpected BaseCombat attributes in a unit type whose base class does not support it"
            );
        }

        if self.unit_base_class >= UnitBaseClass::Missile {
            self.missile
                .as_ref()
                .expect("Unit's base class was Missile, but it has no Missile attributes")
                .write_to(&mut output, version)?;
        } else {
            assert!(
                self.missile.is_none(),
                "Unexpected Missile attributes in a unit type whose base class does not support it"
            );
        }

        if self.unit_base_class >= UnitBaseClass::Combat {
            self.combat
                .as_ref()
                .expect("Unit's base class was Combat, but it has no Combat attributes")
                .write_to(&mut output, version)?;
        } else {
            assert!(
                self.combat.is_none(),
                "Unexpected Combat attributes in a unit type whose base class does not support it"
            );
        }

        if self.unit_base_class >= UnitBaseClass::Building {
            self.building
                .as_ref()
                .expect("Unit's base class was Building, but it has no Building attributes")
                .write_to(&mut output, version)?;
        } else {
            assert!(
                self.building.is_none(),
                "Unexpected Building attributes in a unit type whose base class does not support it"
            );
        }

        Ok(())
    }
}

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

    fn write_empty(mut output: impl Write) -> Result<()> {
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
    name: String,
    /// ID of this unit type.
    pub id: UnitTypeID,
    /// Language string ID of the unit's name.
    pub string_id: StringKey,
    string_id2: Option<StringKey>,
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

/// The attribute of animated unit types.
#[derive(Debug, Default, Clone, Copy)]
pub struct AnimatedUnitTypeAttributes {
    /// Movement speed.
    pub speed: f32,
}

impl AnimatedUnitTypeAttributes {
    /// Read the animated attributes from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    pub fn read_from(mut input: impl Read, _version: f32) -> Result<Self> {
        Ok(Self {
            speed: input.read_f32::<LE>()?,
        })
    }

    /// Write this unit type to an output stream.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails.
    pub fn write_to(&self, mut output: impl Write, _version: f32) -> Result<()> {
        output.write_f32::<LE>(self.speed)?;
        Ok(())
    }
}

/// The attributes of moving unit types.
#[derive(Debug, Default, Clone, Copy)]
pub struct MovingUnitTypeAttributes {
    /// Sprite shown while walking, or `None`.
    pub move_sprite: Option<SpriteID>,
    /// Sprite shown while running, or `None`.
    pub run_sprite: Option<SpriteID>,
    /// Rotation speed.
    pub turn_speed: f32,
    /// Old size class (genieutils' `OldSizeClass`).
    pub size_class: u8,
    /// Unit type left behind as a trail while moving (e.g. footprints; genieutils' `TrackingUnit`),
    /// or `None`.
    pub trailing_unit: Option<UnitTypeID>,
    /// When the trailing unit is left (genieutils' `TrackingUnitMode`).
    pub trailing_options: u8,
    /// Spacing of the trailing units (genieutils' `TrackingUnitDensity`).
    pub trailing_spacing: f32,
    /// Old movement algorithm (genieutils' `OldMoveAlgorithm`).
    pub move_algorithm: u8,
    /// Turning radius.
    pub turn_radius: f32,
    /// genieutils' `TurnRadiusSpeed`: the speed that goes with the turning radius.
    pub turn_radius_speed: f32,
    /// The most the unit turns per second while moving.
    pub maximum_yaw_per_second_moving: f32,
    /// Time for a full turn while standing.
    pub stationary_yaw_revolution_time: f32,
    /// The most the unit turns per second while standing.
    pub maximum_yaw_per_second_stationary: f32,
}

impl MovingUnitTypeAttributes {
    /// Read the moving attributes from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails, or if an ID is out of range.
    pub fn read_from(mut input: impl Read, _version: f32) -> Result<Self> {
        Ok(Self {
            move_sprite: read_opt_u16(&mut input)?,
            run_sprite: read_opt_u16(&mut input)?,
            turn_speed: input.read_f32::<LE>()?,
            size_class: input.read_u8()?,
            trailing_unit: read_opt_u16(&mut input)?,
            trailing_options: input.read_u8()?,
            trailing_spacing: input.read_f32::<LE>()?,
            move_algorithm: input.read_u8()?,
            turn_radius: input.read_f32::<LE>()?,
            turn_radius_speed: input.read_f32::<LE>()?,
            maximum_yaw_per_second_moving: input.read_f32::<LE>()?,
            stationary_yaw_revolution_time: input.read_f32::<LE>()?,
            maximum_yaw_per_second_stationary: input.read_f32::<LE>()?,
        })
    }

    /// Write this unit type to an output stream.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails.
    ///
    /// # Panics
    ///
    /// Panics if a sprite or unit ID does not fit the format's `i16`.
    pub fn write_to(&self, mut output: impl Write, _version: f32) -> Result<()> {
        output.write_i16::<LE>(self.move_sprite.map_or(-1, |id| id.try_into().unwrap()))?;
        output.write_i16::<LE>(self.run_sprite.map_or(-1, |id| id.try_into().unwrap()))?;
        output.write_f32::<LE>(self.turn_speed)?;
        output.write_u8(self.size_class)?;
        output.write_i16::<LE>(self.trailing_unit.map_or(-1, |id| id.try_into().unwrap()))?;
        output.write_u8(self.trailing_options)?;
        output.write_f32::<LE>(self.trailing_spacing)?;
        output.write_u8(self.move_algorithm)?;
        output.write_f32::<LE>(self.turn_radius)?;
        output.write_f32::<LE>(self.turn_radius_speed)?;
        output.write_f32::<LE>(self.maximum_yaw_per_second_moving)?;
        output.write_f32::<LE>(self.stationary_yaw_revolution_time)?;
        output.write_f32::<LE>(self.maximum_yaw_per_second_stationary)?;
        Ok(())
    }
}

/// The attributes of unit types that can be given tasks.
#[derive(Debug, Default, Clone)]
pub struct ActionUnitTypeAttributes {
    /// Index of the task the unit does by default, or `None`.
    pub default_task: Option<u16>,
    /// How far the unit searches for targets.
    pub search_radius: f32,
    /// Base rate of the unit's work (gathering, building, …).
    pub work_rate: f32,
    /// Unit type at which the unit drops off resources, or `None`.
    pub drop_site: Option<UnitTypeID>,
    /// A second drop site, or `None`.
    pub backup_drop_site: Option<UnitTypeID>,
    /// Task swap group (genieutils' `TaskSwapGroup`): units of one group turn into each other with
    /// the task, like the kinds of villager.
    pub task_by_group: u8,
    /// Sound played when the unit is given a command (genieutils' `AttackSound`), or `None`.
    pub command_sound: Option<SoundID>,
    /// Sound played when the unit is ordered to move, or `None`.
    pub move_sound: Option<SoundID>,
    /// Task list for older versions; newer game versions store the task list at the root of the
    /// dat file, and use `unit_type.copy_id` to refer to one of those task lists.
    pub tasks: Option<TaskList>,
    /// genieutils' `RunPattern`; its use is not known yet.
    pub run_pattern: u8,
}

impl ActionUnitTypeAttributes {
    /// Read the action attributes from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails, or if an ID is out of range.
    pub fn read_from(mut input: impl Read, _version: f32) -> Result<Self> {
        Ok(Self {
            default_task: read_opt_u16(&mut input)?,
            search_radius: input.read_f32::<LE>()?,
            work_rate: input.read_f32::<LE>()?,
            drop_site: read_opt_u16(&mut input)?,
            backup_drop_site: read_opt_u16(&mut input)?,
            task_by_group: input.read_u8()?,
            command_sound: read_opt_u16(&mut input)?,
            move_sound: read_opt_u16(&mut input)?,
            run_pattern: input.read_u8()?,
            ..Default::default()
        })
    }

    /// Write this unit type to an output stream.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails.
    ///
    /// # Panics
    ///
    /// Panics if the default task, a unit or a sound ID does not fit the format's `i16`.
    pub fn write_to(&self, mut output: impl Write, _version: f32) -> Result<()> {
        output.write_i16::<LE>(self.default_task.map_or(-1, |id| id.try_into().unwrap()))?;
        output.write_f32::<LE>(self.search_radius)?;
        output.write_f32::<LE>(self.work_rate)?;
        output.write_i16::<LE>(self.drop_site.map_or(-1, |id| id.try_into().unwrap()))?;
        output.write_i16::<LE>(
            self.backup_drop_site
                .map_or(-1, |id| id.try_into().unwrap()),
        )?;
        output.write_u8(self.task_by_group)?;
        output.write_i16::<LE>(self.command_sound.map_or(-1, |id| id.try_into().unwrap()))?;
        output.write_i16::<LE>(self.move_sound.map_or(-1, |id| id.try_into().unwrap()))?;
        output.write_u8(self.run_pattern)?;
        Ok(())
    }
}

/// One attack or armour value: a class and the amount for it.
#[derive(Debug, Default, Clone, Copy)]
pub struct WeaponInfo {
    /// The attack or armour class.
    pub weapon_type: i16,
    /// The amount for that class.
    pub value: i16,
}

impl WeaponInfo {
    /// Read an attack or armour value from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    pub fn read_from(mut input: impl Read) -> Result<Self> {
        Ok(Self {
            weapon_type: input.read_i16::<LE>()?,
            value: input.read_i16::<LE>()?,
        })
    }

    /// Write this attack or armour value to an output stream.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails.
    pub fn write_to(self, mut output: impl Write) -> Result<()> {
        output.write_i16::<LE>(self.weapon_type)?;
        output.write_i16::<LE>(self.value)?;
        Ok(())
    }
}

/// The attributes of unit types that fight.
#[derive(Debug, Default, Clone)]
pub struct BaseCombatUnitTypeAttributes {
    /// Base armour; stored as a u8 before data version 11.52.
    pub base_armor: u16,
    /// Attack values, one per class.
    pub weapons: Vec<WeaponInfo>,
    /// Armour values, one per class.
    pub armors: Vec<WeaponInfo>,
    /// Terrain restriction whose values scale damage by the terrain the unit stands on (genieutils'
    /// `DefenseTerrainBonus`), or `None`.
    pub defense_terrain_bonus: Option<u16>,
    /// Maximum attack range.
    pub weapon_range_max: f32,
    /// Radius of the area (blast) damage (genieutils' `BlastWidth`).
    pub area_effect_range: f32,
    /// Time between attacks (reload time).
    pub attack_speed: f32,
    /// Projectile unit type fired, or `None`.
    pub missile_id: Option<UnitTypeID>,
    /// Chance to hit, in percent (genieutils' `AccuracyPercent`).
    pub base_hit_chance: i16,
    /// genieutils' `BreakOffCombat`; its use is not known yet.
    pub break_off_combat: i8,
    /// Frame of the attack animation at which the attack happens.
    pub frame_delay: i16,
    /// Where the projectile starts, relative to the unit (x, y, z).
    pub weapon_offset: (f32, f32, f32),
    /// Which units the area damage hits (genieutils' `BlastAttackLevel`).
    pub blast_level_offense: i8,
    /// Minimum attack range.
    pub weapon_range_min: f32,
    /// How far missed projectiles scatter (genieutils' `AccuracyDispersion`).
    pub missed_missile_spread: f32,
    /// Sprite shown while attacking, or `None`.
    pub fight_sprite: Option<SpriteID>,
    /// Melee armour shown in the interface.
    pub displayed_armor: i16,
    /// Attack shown in the interface.
    pub displayed_attack: i16,
    /// Range shown in the interface.
    pub displayed_range: f32,
    /// Reload time shown in the interface.
    pub displayed_reload_time: f32,
}

impl BaseCombatUnitTypeAttributes {
    /// Read the combat attributes from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails, or if an ID is out of range.
    pub fn read_from(mut input: impl Read, version: f32) -> Result<Self> {
        let mut attrs = Self {
            base_armor: if version < 11.52 {
                input.read_u8()?.into()
            } else {
                input.read_u16::<LE>()?
            },
            ..Default::default()
        };
        let num_weapons = input.read_u16::<LE>()?;
        for _ in 0..num_weapons {
            attrs.weapons.push(WeaponInfo::read_from(&mut input)?);
        }
        let num_armors = input.read_u16::<LE>()?;
        for _ in 0..num_armors {
            attrs.armors.push(WeaponInfo::read_from(&mut input)?);
        }
        attrs.defense_terrain_bonus = read_opt_u16(&mut input)?;
        attrs.weapon_range_max = input.read_f32::<LE>()?;
        attrs.area_effect_range = input.read_f32::<LE>()?;
        attrs.attack_speed = input.read_f32::<LE>()?;
        attrs.missile_id = read_opt_u16(&mut input)?;
        attrs.base_hit_chance = input.read_i16::<LE>()?;
        attrs.break_off_combat = input.read_i8()?;
        attrs.frame_delay = input.read_i16::<LE>()?;
        attrs.weapon_offset = (
            input.read_f32::<LE>()?,
            input.read_f32::<LE>()?,
            input.read_f32::<LE>()?,
        );
        attrs.blast_level_offense = input.read_i8()?;
        attrs.weapon_range_min = input.read_f32::<LE>()?;
        attrs.missed_missile_spread = input.read_f32::<LE>()?;
        attrs.fight_sprite = read_opt_u16(&mut input)?;
        attrs.displayed_armor = input.read_i16::<LE>()?;
        attrs.displayed_attack = input.read_i16::<LE>()?;
        attrs.displayed_range = input.read_f32::<LE>()?;
        attrs.displayed_reload_time = input.read_f32::<LE>()?;
        Ok(attrs)
    }

    /// Write this unit type to an output stream.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails.
    ///
    /// # Panics
    ///
    /// Panics if `version` is below 11.52 and the base armour does not fit the u8 that such files
    /// store it in.
    // The format stores the attack and armour counts as u16 (truncated, as before).
    #[allow(clippy::cast_possible_truncation)]
    pub fn write_to(&self, mut output: impl Write, version: f32) -> Result<()> {
        if version < 11.52 {
            output.write_u8(self.base_armor.try_into().unwrap())?;
        } else {
            output.write_u16::<LE>(self.base_armor)?;
        }
        output.write_u16::<LE>(self.weapons.len() as u16)?;
        for weapon in &self.weapons {
            weapon.write_to(&mut output)?;
        }
        output.write_u16::<LE>(self.armors.len() as u16)?;
        for armor in &self.armors {
            armor.write_to(&mut output)?;
        }
        output.write_u16::<LE>(self.defense_terrain_bonus.unwrap_or(0xFFFF))?;
        output.write_f32::<LE>(self.weapon_range_max)?;
        output.write_f32::<LE>(self.area_effect_range)?;
        output.write_f32::<LE>(self.attack_speed)?;
        output.write_u16::<LE>(self.missile_id.map_into().unwrap_or(0xFFFF))?;
        output.write_i16::<LE>(self.base_hit_chance)?;
        output.write_i8(self.break_off_combat)?;
        output.write_i16::<LE>(self.frame_delay)?;
        output.write_f32::<LE>(self.weapon_offset.0)?;
        output.write_f32::<LE>(self.weapon_offset.1)?;
        output.write_f32::<LE>(self.weapon_offset.2)?;
        output.write_i8(self.blast_level_offense)?;
        output.write_f32::<LE>(self.weapon_range_min)?;
        output.write_f32::<LE>(self.missed_missile_spread)?;
        output.write_u16::<LE>(self.fight_sprite.map_into().unwrap_or(0xFFFF))?;
        output.write_i16::<LE>(self.displayed_armor)?;
        output.write_i16::<LE>(self.displayed_attack)?;
        output.write_f32::<LE>(self.displayed_range)?;
        output.write_f32::<LE>(self.displayed_reload_time)?;
        Ok(())
    }
}

/// The attributes of projectiles.
#[derive(Debug, Default, Clone, Copy)]
pub struct MissileUnitTypeAttributes {
    /// Projectile type (genieutils' `ProjectileType`).
    pub missile_type: u8,
    /// Whether the projectile aims ahead of moving targets (genieutils' `SmartMode`).
    pub targetting_type: u8,
    /// What the projectile hits (genieutils' `HitMode`).
    pub missile_hit_info: u8,
    /// Whether the projectile vanishes when it hits (genieutils' `VanishMode`).
    pub missile_die_info: u8,
    /// Area-effect specials (genieutils' `AreaEffectSpecials`); not decoded here.
    pub area_effect_specials: u8,
    /// The arc of the trajectory (genieutils' `ProjectileArc`).
    pub ballistics_ratio: f32,
}

impl MissileUnitTypeAttributes {
    /// Read this unit type from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    pub fn read_from(mut input: impl Read, _version: f32) -> Result<Self> {
        Ok(Self {
            missile_type: input.read_u8()?,
            targetting_type: input.read_u8()?,
            missile_hit_info: input.read_u8()?,
            missile_die_info: input.read_u8()?,
            area_effect_specials: input.read_u8()?,
            ballistics_ratio: input.read_f32::<LE>()?,
        })
    }

    /// Write this unit type to an output stream.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails.
    pub fn write_to(&self, mut output: impl Write, _version: f32) -> Result<()> {
        output.write_u8(self.missile_type)?;
        output.write_u8(self.targetting_type)?;
        output.write_u8(self.missile_hit_info)?;
        output.write_u8(self.missile_die_info)?;
        output.write_u8(self.area_effect_specials)?;
        output.write_f32::<LE>(self.ballistics_ratio)?;
        Ok(())
    }
}

/// Resource cost for a unit.
#[derive(Debug, Default, Clone, Copy)]
pub struct AttributeCost {
    /// The player attribute type to give/take.
    pub attribute_type: i16,
    /// The amount of that attribute that should be taken/given.
    pub amount: i16,
    /// Flag determining how and when this cost is counted.
    ///
    /// TODO make this an enum
    pub flag: u8,
}

impl AttributeCost {
    /// Read a cost (and its padding byte) from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    pub fn read_from(mut input: impl Read) -> Result<Self> {
        let cost = Self {
            attribute_type: input.read_i16::<LE>()?,
            amount: input.read_i16::<LE>()?,
            flag: input.read_u8()?,
        };
        let _padding = input.read_u8()?;
        Ok(cost)
    }

    /// Write this cost (and a zero padding byte) to an output stream.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails.
    pub fn write_to(self, mut output: impl Write) -> Result<()> {
        output.write_i16::<LE>(self.attribute_type)?;
        output.write_i16::<LE>(self.amount)?;
        output.write_u8(self.flag)?;
        output.write_u8(0)?;
        Ok(())
    }
}

/// The attributes of combat unit types that players create (genieutils' `Creatable`).
#[derive(Debug, Default, Clone)]
pub struct CombatUnitTypeAttributes {
    /// The costs of creating a unit of this type.
    pub costs: ArrayVec<AttributeCost, 3>,
    /// Time to create the unit.
    pub create_time: u16,
    /// Unit type ID of the building or unit where this unit can be created.
    pub create_at_building: Option<UnitTypeID>,
    /// Button location index where the button to create this unit should be shown when a
    /// `create_at_building` unit is selected.
    pub create_button: i8,
    /// Damage multiplier for attacks from behind.
    pub rear_attack_modifier: f32,
    /// Damage multiplier for attacks from the side.
    pub flank_attack_modifier: f32,
    /// Is this unit a hero unit?
    ///
    /// TODO what is special about hero units? Does it just opt into the healing behaviour?
    pub hero_flag: u8,
    /// Sprite shown while units are garrisoned, or `None`.
    pub garrison_sprite: Option<SpriteID>,
    /// Projectiles fired per attack (genieutils' `TotalProjectiles`).
    pub volley_fire_amount: f32,
    /// The most projectiles fired per attack (genieutils' `MaxTotalProjectiles`).
    pub max_attacks_in_volley: i8,
    /// Width and length of the area the projectiles start from (genieutils'
    /// `ProjectileSpawningArea`).
    pub volley_spread: (f32, f32),
    /// Randomness of the projectiles' start (the third value of genieutils'
    /// `ProjectileSpawningArea`).
    pub volley_start_spread_adjustment: f32,
    /// Unit type of the extra projectiles (genieutils' `SecondaryProjectileUnit`), or `None`.
    pub volley_missile: Option<UnitTypeID>,
    /// Sprite of the special attack (genieutils' `SpecialGraphic`), or `None`.
    pub special_attack_sprite: Option<SpriteID>,
    /// Kind of special ability (genieutils' `SpecialAbility`).
    pub special_attack_flag: i8,
    /// Pierce armour shown in the interface.
    pub displayed_pierce_armor: i16,
}

impl CombatUnitTypeAttributes {
    /// Read this unit type from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails, or if an ID is out of range.
    ///
    /// # Panics
    ///
    /// Panics if the garrison sprite ID does not fit a [`SpriteID`].
    pub fn read_from(mut input: impl Read, _version: f32) -> Result<Self> {
        let mut attrs = Self::default();
        for _ in 0..3 {
            let attr = AttributeCost::read_from(&mut input)?;
            if attr.attribute_type >= 0 {
                attrs.costs.push(attr);
            }
        }
        attrs.create_time = input.read_u16::<LE>()?;
        attrs.create_at_building = read_opt_u16(&mut input)?;
        attrs.create_button = input.read_i8()?;
        attrs.rear_attack_modifier = input.read_f32::<LE>()?;
        attrs.flank_attack_modifier = input.read_f32::<LE>()?;
        let _tribe_unit_type = input.read_u8()?;
        attrs.hero_flag = input.read_u8()?;
        attrs.garrison_sprite = {
            let n = input.read_i32::<LE>()?;
            if n < 0 {
                None
            } else {
                Some(n.try_into().unwrap())
            }
        };
        attrs.volley_fire_amount = input.read_f32::<LE>()?;
        attrs.max_attacks_in_volley = input.read_i8()?;
        attrs.volley_spread = (input.read_f32::<LE>()?, input.read_f32::<LE>()?);
        attrs.volley_start_spread_adjustment = input.read_f32::<LE>()?;
        attrs.volley_missile = read_opt_u32(&mut input)?;
        attrs.special_attack_sprite = read_opt_u32(&mut input)?;
        attrs.special_attack_flag = input.read_i8()?;
        attrs.displayed_pierce_armor = input.read_i16::<LE>()?;
        Ok(attrs)
    }

    /// Write this unit type to an output stream.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails.
    pub fn write_to(&self, mut output: impl Write, _version: f32) -> Result<()> {
        for i in 0..3 {
            match self.costs.get(i) {
                Some(cost) => cost.write_to(&mut output)?,
                None => AttributeCost {
                    attribute_type: -1,
                    amount: 0,
                    flag: 0,
                }
                .write_to(&mut output)?,
            }
        }
        output.write_u16::<LE>(self.create_time)?;
        output.write_u16::<LE>(self.create_at_building.map_into().unwrap_or(0xFFFF))?;
        output.write_i8(self.create_button)?;
        output.write_f32::<LE>(self.rear_attack_modifier)?;
        output.write_f32::<LE>(self.flank_attack_modifier)?;
        output.write_u8(0)?;
        output.write_u8(self.hero_flag)?;
        output.write_u32::<LE>(self.garrison_sprite.map_into().unwrap_or(0xFFFF_FFFF))?;
        output.write_f32::<LE>(self.volley_fire_amount)?;
        output.write_i8(self.max_attacks_in_volley)?;
        output.write_f32::<LE>(self.volley_spread.0)?;
        output.write_f32::<LE>(self.volley_spread.1)?;
        output.write_f32::<LE>(self.volley_start_spread_adjustment)?;
        output.write_u32::<LE>(self.volley_missile.map_into().unwrap_or(0xFFFF_FFFF))?;
        output.write_u32::<LE>(self.special_attack_sprite.map_into().unwrap_or(0xFFFF_FFFF))?;
        output.write_i8(self.special_attack_flag)?;
        output.write_i16::<LE>(self.displayed_pierce_armor)?;
        Ok(())
    }
}

/// A linked, or "Annex" building. These allow for buildings made up of multiple pieces
/// with different behaviour, like the Town Centre with some walkable tiles and some non-walkable
/// tiles.
#[derive(Debug, Default, Clone, Copy)]
pub struct LinkedBuilding {
    /// Unit type ID for this linked building.
    pub unit_id: UnitTypeID,
    /// X offset in tiles from the centre of the "owner" building.
    pub x_offset: f32,
    /// Y offset in tiles from the centre of the "owner" building.
    pub y_offset: f32,
}

impl LinkedBuilding {
    /// Read an annex from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    pub fn read_from(mut input: impl Read) -> Result<Self> {
        Ok(Self {
            unit_id: input.read_u16::<LE>()?.into(),
            x_offset: input.read_f32::<LE>()?,
            y_offset: input.read_f32::<LE>()?,
        })
    }

    /// Write this annex to an output stream.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails.
    pub fn write_to(&self, mut output: impl Write) -> Result<()> {
        output.write_u16::<LE>(self.unit_id.into())?;
        output.write_f32::<LE>(self.x_offset)?;
        output.write_f32::<LE>(self.y_offset)?;
        Ok(())
    }
    fn write_empty(mut output: impl Write) -> Result<()> {
        output.write_u16::<LE>(0xFFFF)?;
        output.write_f32::<LE>(0.0)?;
        output.write_f32::<LE>(0.0)?;
        Ok(())
    }
}

/// Unit type class for buildings.
#[derive(Debug, Default, Clone)]
pub struct BuildingUnitTypeAttributes {
    /// Sprite to use during construction.
    pub construction_sprite: Option<SpriteID>,
    /// Sprite to use when this building is finished and built on snow.
    pub snow_sprite: Option<SpriteID>,
    /// TODO document
    pub connect_flag: u8,
    /// TODO document
    pub facet: i16,
    /// Whether the building should be immediately destroyed on completion.
    pub destroy_on_build: bool,
    /// Unit to spawn at the build site on completion.
    pub on_build_make_unit: Option<UnitTypeID>,
    /// Change the underlying terrain to this terrain ID on completion.
    pub on_build_make_tile: Option<TerrainID>,
    /// TODO document
    pub on_build_make_overlay: i16,
    /// Research this tech on completion.
    pub on_build_make_tech: Option<TechID>,
    /// Whether this building…can burn?
    ///
    /// TODO document the details
    pub can_burn: bool,
    /// Annex buildings, at most 4; unused slots are dropped when reading.
    pub linked_buildings: ArrayVec<LinkedBuilding, 4>,
    /// The head unit of a building made of several units (genieutils' `HeadUnit`), or `None`.
    pub construction_unit: Option<UnitTypeID>,
    /// Unit type the building transforms into (genieutils' `TransformUnit`), or `None`.
    pub transform_unit: Option<UnitTypeID>,
    /// Sound played when the building transforms, or `None`.
    pub transform_sound: Option<SoundID>,
    /// Sound played while the building is built, or `None`.
    pub construction_sound: Option<SoundID>,
    /// Which kinds of unit can garrison inside (genieutils' `GarrisonType`, a bit mask).
    pub garrison_type: i8,
    /// Rate at which garrisoned units heal.
    pub garrison_heal_rate: f32,
    /// Rate at which the building is repaired by garrisoned units (genieutils'
    /// `GarrisonRepairRate`).
    pub garrison_repair_rate: f32,
    /// Unit type left behind when the building is destroyed (genieutils' `PileUnit`), or `None`.
    pub salvage_unit: Option<UnitTypeID>,
    /// The six values of genieutils' `LootingTable`: what is looted from the building.
    pub salvage_attributes: ArrayVec<i8, 6>,
}

impl BuildingUnitTypeAttributes {
    /// Read this unit type from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails, or if an ID is out of range.
    pub fn read_from(mut input: impl Read, version: f32) -> Result<Self> {
        let mut attrs = Self {
            construction_sprite: read_opt_u16(&mut input)?,
            snow_sprite: if version < 11.53 {
                None
            } else {
                read_opt_u16(&mut input)?
            },
            connect_flag: input.read_u8()?,
            facet: input.read_i16::<LE>()?,
            destroy_on_build: input.read_u8()? != 0,
            on_build_make_unit: read_opt_u16(&mut input)?,
            on_build_make_tile: read_opt_u16(&mut input)?,
            on_build_make_overlay: input.read_i16::<LE>()?,
            on_build_make_tech: read_opt_u16(&mut input)?,
            can_burn: input.read_u8()? != 0,
            ..Default::default()
        };
        for _ in 0..attrs.linked_buildings.capacity() {
            let link = LinkedBuilding::read_from(&mut input)?;
            if link.unit_id != 0xFFFF.into() {
                attrs.linked_buildings.push(link);
            }
        }

        attrs.construction_unit = read_opt_u16(&mut input)?;
        attrs.transform_unit = read_opt_u16(&mut input)?;
        attrs.transform_sound = read_opt_u16(&mut input)?;
        attrs.construction_sound = read_opt_u16(&mut input)?;
        attrs.garrison_type = input.read_i8()?;
        attrs.garrison_heal_rate = input.read_f32::<LE>()?;
        attrs.garrison_repair_rate = input.read_f32::<LE>()?;
        attrs.salvage_unit = read_opt_u16(&mut input)?;
        for _ in 0..attrs.salvage_attributes.capacity() {
            let attr = input.read_i8()?;
            attrs.salvage_attributes.push(attr);
        }
        Ok(attrs)
    }

    /// Write the unit type to an output stream.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails.
    pub fn write_to(&self, mut output: impl Write, version: f32) -> Result<()> {
        output.write_u16::<LE>(self.construction_sprite.map_into().unwrap_or(0xFFFF))?;
        if version >= 11.53 {
            output.write_u16::<LE>(self.snow_sprite.map_into().unwrap_or(0xFFFF))?;
        }
        output.write_u8(self.connect_flag)?;
        output.write_i16::<LE>(self.facet)?;
        output.write_u8(u8::from(self.destroy_on_build))?;
        output.write_u16::<LE>(self.on_build_make_unit.map_into().unwrap_or(0xFFFF))?;
        output.write_u16::<LE>(self.on_build_make_tile.map_into().unwrap_or(0xFFFF))?;
        output.write_i16::<LE>(self.on_build_make_overlay)?;
        output.write_u16::<LE>(self.on_build_make_tech.map_into().unwrap_or(0xFFFF))?;
        output.write_u8(u8::from(self.can_burn))?;
        for i in 0..self.linked_buildings.capacity() {
            match self.linked_buildings.get(i) {
                Some(link) => link.write_to(&mut output)?,
                None => LinkedBuilding::write_empty(&mut output)?,
            }
        }
        output.write_u16::<LE>(self.construction_unit.map_into().unwrap_or(0xFFFF))?;
        output.write_u16::<LE>(self.transform_unit.map_into().unwrap_or(0xFFFF))?;
        output.write_u16::<LE>(self.transform_sound.map_into().unwrap_or(0xFFFF))?;
        output.write_u16::<LE>(self.construction_sound.map_into().unwrap_or(0xFFFF))?;
        output.write_i8(self.garrison_type)?;
        output.write_f32::<LE>(self.garrison_heal_rate)?;
        output.write_f32::<LE>(self.garrison_repair_rate)?;
        output.write_u16::<LE>(self.salvage_unit.map_into().unwrap_or(0xFFFF))?;
        for attr in &self.salvage_attributes {
            output.write_i8(*attr)?;
        }
        Ok(())
    }
}

fn write_opt_string_key(mut output: impl Write, opt_key: Option<&StringKey>) -> Result<()> {
    output.write_i16::<LE>(if let Some(key) = opt_key {
        key.try_into().map_err(io::Error::other)?
    } else {
        -1
    })?;
    Ok(())
}
