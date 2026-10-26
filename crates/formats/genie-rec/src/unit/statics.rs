//! The attributes every object has, and the animated base class's.

use crate::element::{ReadableHeaderElement, WritableHeaderElement};
use crate::reader::RecordingHeaderReader;
use crate::unit::SpriteList;
use crate::{ObjectID, PlayerID, Result};
use byteorder::{LE, ReadBytesExt, WriteBytesExt};
use genie_dat::sprite::SpriteID;
use genie_support::{UnitTypeID, read_opt_u32};
use std::convert::TryInto;
use std::io::{Read, Write};

/// The attributes every object has.
// The bools are the format's on/off flags.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Default, Clone)]
pub struct StaticUnitAttributes {
    /// The player who owns the object.
    pub owner_id: PlayerID,
    /// The object's unit type.
    pub unit_type_id: UnitTypeID,
    /// The sprite the object shows.
    pub sprite_id: SpriteID,
    /// The object it is garrisoned in, if any.
    pub garrisoned_in_id: Option<ObjectID>,
    /// The hit points.
    pub hit_points: f32,
    /// The object's life state (alive, dying, dead …).
    pub object_state: u8,
    /// Whether the object is asleep (not updated).
    pub sleep_flag: bool,
    /// Whether the object has a doppelganger, a stand-in shown under the fog of war.
    pub dopple_flag: bool,
    /// Whether the object falls asleep at its next update.
    pub go_to_sleep_flag: bool,
    /// The object's id.
    pub id: ObjectID,
    /// The facet (direction) the object faces.
    pub facet: u8,
    /// The position (x, y, z).
    pub position: (f32, f32, f32),
    /// The offset at which the object is drawn on screen.
    pub screen_offset: (u16, u16),
    /// The offset of the object's shadow.
    pub shadow_offset: (u16, u16),
    /// The control group the object is in; stored before save version 11.58 only.
    pub selected_group: Option<u8>,
    /// The attribute (resource) the object holds: the food of a sheep, the gold a villager
    /// carries.
    pub attribute_type_held: u16,
    /// The amount of that attribute.
    pub attribute_amount_held: f32,
    /// The number of units working on the object (gatherers, builders …).
    pub worker_count: u8,
    /// The damage stage the object shows.
    pub current_damage: u8,
    /// The timer since the object was last damaged.
    pub damaged_lately_timer: u8,
    /// Whether the object is under attack.
    pub under_attack: bool,
    /// The objects in the object's pathing group (units that move together).
    pub pathing_group_members: Vec<ObjectID>,
    /// The object's group id, if any.
    pub group_id: Option<u32>,
    /// Not known yet.
    pub roo_already_called: u8,
    /// The sprites drawn for the object, if it has a sprite list.
    pub sprite_list: Option<SpriteList>,
}

impl ReadableHeaderElement for StaticUnitAttributes {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        let mut attrs = Self {
            owner_id: input.read_u8()?.into(),
            unit_type_id: input.read_u16::<LE>()?.into(),
            sprite_id: input.read_u16::<LE>()?.into(),
            garrisoned_in_id: read_opt_u32(input)?,
            hit_points: input.read_f32::<LE>()?,
            object_state: input.read_u8()?,
            sleep_flag: input.read_u8()? != 0,
            dopple_flag: input.read_u8()? != 0,
            go_to_sleep_flag: input.read_u8()? != 0,
            id: input.read_u32::<LE>()?.into(),
            facet: input.read_u8()?,
            position: (
                input.read_f32::<LE>()?,
                input.read_f32::<LE>()?,
                input.read_f32::<LE>()?,
            ),
            screen_offset: (input.read_u16::<LE>()?, input.read_u16::<LE>()?),
            shadow_offset: (input.read_u16::<LE>()?, input.read_u16::<LE>()?),
            ..Default::default()
        };
        if input.version() < 11.58 {
            attrs.selected_group = match input.read_i8()? {
                -1 => None,
                id => Some(id.try_into().unwrap()),
            };
        }
        attrs.attribute_type_held = input.read_u16::<LE>()?;
        attrs.attribute_amount_held = input.read_f32::<LE>()?;
        attrs.worker_count = input.read_u8()?;
        attrs.current_damage = input.read_u8()?;
        attrs.damaged_lately_timer = input.read_u8()?;
        attrs.under_attack = input.read_u8()? != 0;
        attrs.pathing_group_members = {
            let num_members = input.read_u32::<LE>()?;
            let mut members = vec![ObjectID(0); num_members.try_into().unwrap()];
            for m in &mut members {
                *m = input.read_u32::<LE>()?.into();
            }
            members
        };
        attrs.group_id = read_opt_u32(input)?;
        attrs.roo_already_called = input.read_u8()?;
        if input.read_u8()? != 0 {
            attrs.sprite_list = Some(SpriteList::read_from(input)?);
        }
        Ok(attrs)
    }
}

impl WritableHeaderElement for StaticUnitAttributes {
    fn write_to<W: Write>(&self, output: &mut W) -> Result<()> {
        output.write_u8(self.owner_id.into())?;
        output.write_u16::<LE>(self.unit_type_id.into())?;
        output.write_u16::<LE>(self.sprite_id.into())?;
        todo!()
    }
}

/// The attributes the animated base class adds.
#[derive(Debug, Default, Clone, Copy)]
pub struct AnimatedUnitAttributes {
    /// The current movement speed.
    pub speed: f32,
}

impl ReadableHeaderElement for AnimatedUnitAttributes {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        let speed = input.read_f32::<LE>()?;
        Ok(Self { speed })
    }
}

impl WritableHeaderElement for AnimatedUnitAttributes {
    fn write_to<W: Write>(&self, output: &mut W) -> Result<()> {
        output.write_f32::<LE>(self.speed)?;
        Ok(())
    }
}
