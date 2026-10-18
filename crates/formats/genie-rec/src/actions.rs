//! Player actions executed during a game.
//!
//! This is the reader for the body of older recorded games (`AoC`, `UserPatch`, HD and early DE).
//! Command layouts and names follow genie-rs and aoc-mgz; where the meaning of a stored value is
//! not known, its docs say so.

use crate::{ObjectID, PlayerID, Result};
use arrayvec::ArrayVec;
use byteorder::{LE, ReadBytesExt, WriteBytesExt};
use genie_support::{ReadSkipExt, ReadStringsExt, TechID, UnitTypeID, f32_neq, read_opt_u32};
use std::convert::TryInto;
use std::io::{Read, Write};

/// A location with an X and Y coordinate.
pub type Location2 = (f32, f32);
/// A location with an X, Y, and Z coordinate.
///
/// The Z coordinate is usually meaningless.
pub type Location3 = (f32, f32, f32);

/// A viewpoint update, recording where the player is currently looking.
///
/// This is used for the View Lock feature when watching a game.
#[derive(Debug, Default, Clone, Copy)]
pub struct ViewLock {
    /// The X coordinate the player is looking at.
    pub x: f32,
    /// The Y coordinate the player is looking at.
    pub y: f32,
    /// The ID of the POV player.
    pub player: PlayerID,
}

impl ViewLock {
    /// Read a view lock action from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    ///
    /// # Panics
    ///
    /// Panics if the stored player ID is negative or above 255.
    pub fn read_from(mut input: impl Read) -> Result<Self> {
        let x = input.read_f32::<LE>()?;
        let y = input.read_f32::<LE>()?;
        let player = input.read_i32::<LE>()?.try_into().unwrap();
        Ok(Self { x, y, player })
    }

    /// Write a view lock action to an output stream.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails.
    pub fn write_to<W: Write>(&self, output: &mut W) -> Result<()> {
        output.write_f32::<LE>(self.x)?;
        output.write_f32::<LE>(self.y)?;
        output.write_i32::<LE>(i32::from(self.player))?;
        Ok(())
    }
}

/// A list of objects that a command applies to.
///
/// The game uses a special value if a command applies to the same objects as the previous command.
/// That way it does not have to resend 40 object IDs every time a player moves their army. It's
/// encoded as `ObjectsList::SameAsLast` here.
#[derive(Debug, Clone)]
pub enum ObjectsList {
    /// Use the same objects as the previous command.
    SameAsLast,
    /// Use this list of objects.
    List(Vec<ObjectID>),
}

impl Default for ObjectsList {
    fn default() -> Self {
        Self::List(vec![])
    }
}

impl ObjectsList {
    /// Read a list of objects from an input stream.
    ///
    /// A `count` of 0xFF or more means [`ObjectsList::SameAsLast`] and reads nothing.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    ///
    /// # Panics
    ///
    /// Panics if an object ID in the list is negative.
    pub fn read_from(mut input: impl Read, count: i32) -> Result<Self> {
        if count < 0xFF {
            let mut list = vec![];
            for _ in 0..count {
                list.push(input.read_i32::<LE>()?.try_into().unwrap());
            }
            Ok(Self::List(list))
        } else {
            Ok(Self::SameAsLast)
        }
    }

    /// Write a list of objects to an output stream.
    ///
    /// [`ObjectsList::SameAsLast`] writes nothing.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails.
    pub fn write_to<W: Write>(&self, output: &mut W) -> Result<()> {
        if let Self::List(list) = self {
            for entry in list.iter().copied() {
                output.write_u32::<LE>(entry.into())?;
            }
        }
        Ok(())
    }

    /// The amount of objects contained in this list.
    ///
    /// For `ObjectsList::SameAsLast` this returns 0.
    #[must_use]
    pub const fn len(&self) -> usize {
        match self {
            Self::SameAsLast => 0,
            Self::List(list) => list.len(),
        }
    }

    /// Does this list contain 0 objects?
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Task an object to a target object or a target location.
#[derive(Debug, Default, Clone)]
pub struct OrderCommand {
    /// The ID of the player executing this command.
    pub player_id: PlayerID,
    /// The target object of this order.
    pub target_id: Option<ObjectID>,
    /// The target location of this order.
    pub location: Location2,
    /// The objects this command applies to.
    pub objects: ObjectsList,
}

impl OrderCommand {
    /// Read an Order command from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    pub fn read_from(mut input: impl Read) -> Result<Self> {
        let mut command = Self {
            player_id: input.read_u8()?.into(),
            ..Default::default()
        };
        input.skip(2)?;
        command.target_id = read_opt_u32(&mut input)?;
        let selected_count = input.read_i32::<LE>()?;
        command.location = (input.read_f32::<LE>()?, input.read_f32::<LE>()?);
        command.objects = ObjectsList::read_from(input, selected_count)?;
        Ok(command)
    }

    /// Write an Order command to an output stream.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails.
    ///
    /// # Panics
    ///
    /// Panics if the target ID is above `i32::MAX`, or if there are more than `u32::MAX` objects.
    pub fn write_to<W: Write>(&self, output: &mut W) -> Result<()> {
        output.write_u8(self.player_id.into())?;
        output.write_all(&[0, 0])?;
        output.write_i32::<LE>(self.target_id.map_or(-1, |id| id.try_into().unwrap()))?;
        output.write_u32::<LE>(self.objects.len().try_into().unwrap())?;
        output.write_f32::<LE>(self.location.0)?;
        output.write_f32::<LE>(self.location.1)?;
        self.objects.write_to(output)?;
        Ok(())
    }
}

/// Task objects to stop.
#[derive(Debug, Default, Clone)]
pub struct StopCommand {
    /// The objects to stop.
    pub objects: ObjectsList,
}

impl StopCommand {
    /// Read a Stop command from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    pub fn read_from(mut input: impl Read) -> Result<Self> {
        let mut command = Self::default();
        let selected_count = input.read_i8()?;
        command.objects = ObjectsList::read_from(input, i32::from(selected_count))?;
        Ok(command)
    }

    /// Write this Stop command to an output stream.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails.
    ///
    /// # Panics
    ///
    /// Panics if there are more than 127 objects, the most the format's `i8` count holds.
    pub fn write_to<W: Write>(&self, output: &mut W) -> Result<()> {
        output.write_i8(self.objects.len().try_into().unwrap())?;
        self.objects.write_to(output)?;
        Ok(())
    }
}

/// Task an object to work.
#[derive(Debug, Default, Clone)]
pub struct WorkCommand {
    /// The target object of this command.
    pub target_id: Option<ObjectID>,
    /// The target location of this command.
    pub location: Location2,
    /// The objects being tasked.
    pub objects: ObjectsList,
}

impl WorkCommand {
    /// Read a Work command from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    pub fn read_from(mut input: impl Read) -> Result<Self> {
        let mut command = Self::default();
        input.skip(3)?;
        command.target_id = read_opt_u32(&mut input)?;
        let selected_count = input.read_i8()?;
        input.skip(3)?;
        command.location = (input.read_f32::<LE>()?, input.read_f32::<LE>()?);
        command.objects = ObjectsList::read_from(input, i32::from(selected_count))?;
        Ok(command)
    }

    /// Write this Work command to an output stream.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails.
    ///
    /// # Panics
    ///
    /// Panics if there are more than 127 objects, the most the format's `i8` count holds.
    pub fn write_to<W: Write>(&self, output: &mut W) -> Result<()> {
        output.write_all(&[0, 0, 0])?;
        output.write_i32::<LE>(self.target_id.map_or(-1, |u| u32::from(u).cast_signed()))?;
        output.write_i8(self.objects.len().try_into().unwrap())?;
        output.write_all(&[0, 0, 0])?;
        output.write_f32::<LE>(self.location.0)?;
        output.write_f32::<LE>(self.location.1)?;
        self.objects.write_to(output)?;
        Ok(())
    }
}

/// Task an object to move.
#[derive(Debug, Default, Clone)]
pub struct MoveCommand {
    /// The ID of the player issuing this command.
    pub player_id: PlayerID,
    /// The target object of this command.
    pub target_id: Option<ObjectID>,
    /// The target location of this command.
    pub location: Location2,
    /// The objects being tasked.
    pub objects: ObjectsList,
}

impl MoveCommand {
    /// Read a Move command from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    pub fn read_from(mut input: impl Read) -> Result<Self> {
        let mut command = Self {
            player_id: input.read_u8()?.into(),
            ..Default::default()
        };
        input.skip(2)?;
        command.target_id = read_opt_u32(&mut input)?;
        let selected_count = input.read_i8()?;
        input.skip(3)?;
        command.location = (input.read_f32::<LE>()?, input.read_f32::<LE>()?);
        command.objects = ObjectsList::read_from(input, i32::from(selected_count))?;
        Ok(command)
    }

    /// Write this Move command to an output stream.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails.
    ///
    /// # Panics
    ///
    /// Panics if there are more than 127 objects, the most the format's `i8` count holds.
    pub fn write_to<W: Write>(&self, output: &mut W) -> Result<()> {
        output.write_all(&[0, 0, 0])?;
        output.write_i32::<LE>(self.target_id.map_or(-1, |u| u32::from(u).cast_signed()))?;
        output.write_i8(self.objects.len().try_into().unwrap())?;
        output.write_all(&[0, 0, 0])?;
        output.write_f32::<LE>(self.location.0)?;
        output.write_f32::<LE>(self.location.1)?;
        self.objects.write_to(output)?;
        Ok(())
    }
}
/// A command that instantly places a unit type at a given location.
///
/// Typically used for cheats and the like.
#[derive(Debug, Default, Clone, Copy)]
pub struct CreateCommand {
    /// The ID of the player issuing this command.
    pub player_id: PlayerID,
    /// The type of unit to create.
    pub unit_type_id: UnitTypeID,
    /// The location.
    pub location: Location3,
}

impl CreateCommand {
    /// Read a Create command from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    pub fn read_from(mut input: impl Read) -> Result<Self> {
        let mut command = Self::default();
        let _padding = input.read_u8()?;
        command.unit_type_id = input.read_u16::<LE>()?.into();
        command.player_id = input.read_u8()?.into();
        let _padding = input.read_u8()?;
        command.location = (
            input.read_f32::<LE>()?,
            input.read_f32::<LE>()?,
            input.read_f32::<LE>()?,
        );
        Ok(command)
    }

    /// Write this Create command to an output stream.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails.
    pub fn write_to<W: Write>(&self, output: &mut W) -> Result<()> {
        output.write_u8(0)?;
        output.write_u16::<LE>(self.unit_type_id.into())?;
        output.write_u8(self.player_id.into())?;
        output.write_u8(0)?;
        output.write_f32::<LE>(self.location.0)?;
        output.write_f32::<LE>(self.location.1)?;
        output.write_f32::<LE>(self.location.2)?;
        Ok(())
    }
}

/// Add resources to a player's stockpile.
///
/// Typically used for cheats.
#[derive(Debug, Default, Clone, Copy)]
pub struct AddResourceCommand {
    /// The player this command applies to.
    pub player_id: PlayerID,
    /// The resource to add.
    pub resource: u8,
    /// The amount to add to this resource. May be negative for subtracting.
    pub amount: f32,
}

impl AddResourceCommand {
    /// Read an `AddResource` command from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    pub fn read_from(mut input: impl Read) -> Result<Self> {
        let player_id = input.read_u8()?.into();
        let resource = input.read_u8()?;
        let _padding = input.read_u8()?;
        let amount = input.read_f32::<LE>()?;
        Ok(Self {
            player_id,
            resource,
            amount,
        })
    }

    /// Write this `AddResource` command to an output stream.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails.
    pub fn write_to<W: Write>(&self, output: &mut W) -> Result<()> {
        output.write_u8(self.player_id.into())?;
        output.write_u8(self.resource)?;
        output.write_u8(0)?;
        output.write_f32::<LE>(self.amount)?;
        Ok(())
    }
}

/// An order that an AI player gives to its objects (command `0x0a`).
#[derive(Debug, Default, Clone)]
pub struct AIOrderCommand {
    /// The ID of the player the command belongs to (the first player byte).
    pub player_id: PlayerID,
    /// The second player byte, read as the player issuing the order; not verified.
    pub issuer: PlayerID,
    /// The objects receiving the order. A single object is stored in the fixed part of the
    /// command, more than one in a list after it.
    pub objects: ObjectsList,
    /// The kind of order.
    pub order_type: u16,
    /// The priority of the order.
    pub order_priority: i8,
    /// The target object, if any.
    pub target_id: Option<ObjectID>,
    /// The owner of the target, if any.
    pub target_player_id: Option<PlayerID>,
    /// The target location.
    pub target_location: Location3,
    /// The range of the order.
    pub range: f32,
    /// Whether the order is carried out immediately.
    pub immediate: bool,
    /// Whether the order goes to the front of the objects' order queue instead of the back.
    pub add_to_front: bool,
}

impl AIOrderCommand {
    /// Read an AI order command from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    ///
    /// # Panics
    ///
    /// Panics if the target's owner is stored as a negative number other than -1.
    pub fn read_from(mut input: impl Read) -> Result<Self> {
        let mut command = Self::default();
        let selected_count = i32::from(input.read_i8()?);
        command.player_id = input.read_u8()?.into();
        command.issuer = input.read_u8()?.into();
        let object_id = input.read_u32::<LE>()?;
        command.order_type = input.read_u16::<LE>()?;
        command.order_priority = input.read_i8()?;
        let _padding = input.read_u8()?;
        command.target_id = read_opt_u32(&mut input)?;
        command.target_player_id = match input.read_i8()? {
            -1 => None,
            id => Some(id.try_into().unwrap()),
        };
        input.skip(3)?;
        command.target_location = (
            input.read_f32::<LE>()?,
            input.read_f32::<LE>()?,
            input.read_f32::<LE>()?,
        );
        command.range = input.read_f32::<LE>()?;
        command.immediate = input.read_u8()? != 0;
        command.add_to_front = input.read_u8()? != 0;
        let _padding = input.read_u16::<LE>()?;
        command.objects = if selected_count == 1 {
            ObjectsList::List(vec![object_id.into()])
        } else {
            ObjectsList::read_from(input, selected_count)?
        };
        Ok(command)
    }

    /// Write this AI order command to an output stream.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails.
    ///
    /// # Panics
    ///
    /// Panics if there are more than 127 objects (the format's `i8` count), or if the target ID
    /// is above `i32::MAX`.
    pub fn write_to<W: Write>(&self, output: &mut W) -> Result<()> {
        output.write_i8(self.objects.len().try_into().unwrap())?;
        output.write_u8(self.player_id.into())?;
        output.write_u8(self.issuer.into())?;
        match &self.objects {
            ObjectsList::List(list) if list.len() == 1 => {
                output.write_u32::<LE>(list[0].into())?;
            }
            _ => output.write_i32::<LE>(-1)?,
        }
        output.write_u16::<LE>(self.order_type)?;
        output.write_i8(self.order_priority)?;
        output.write_u8(0)?;
        output.write_i32::<LE>(self.target_id.map_or(-1, |id| id.try_into().unwrap()))?;
        output.write_u8(self.player_id.into())?;
        output.write_all(&[0, 0, 0])?;
        output.write_f32::<LE>(self.target_location.0)?;
        output.write_f32::<LE>(self.target_location.1)?;
        output.write_f32::<LE>(self.target_location.2)?;
        output.write_f32::<LE>(self.range)?;
        output.write_u8(u8::from(self.immediate))?;
        output.write_u8(u8::from(self.add_to_front))?;
        output.write_all(&[0, 0])?;
        if self.objects.len() > 1 {
            self.objects.write_to(output)?;
        }
        Ok(())
    }
}

/// A player resigns or drops from the game.
#[derive(Debug, Default, Clone, Copy)]
pub struct ResignCommand {
    /// The ID of the player that is resigning.
    pub player_id: PlayerID,
    /// The multiplayer ID of the player that is resigning.
    pub comm_player_id: PlayerID,
    /// Is this "resignation" because the player dropped from the game?
    pub dropped: bool,
}

impl ResignCommand {
    /// Read a Resign command from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    pub fn read_from(mut input: impl Read) -> Result<Self> {
        let player_id = input.read_u8()?.into();
        let comm_player_id = input.read_u8()?.into();
        let dropped = input.read_u8()? != 0;
        Ok(Self {
            player_id,
            comm_player_id,
            dropped,
        })
    }

    /// Write this Resign command to an output stream.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails.
    pub fn write_to<W: Write>(&self, output: &mut W) -> Result<()> {
        output.write_u8(self.player_id.into())?;
        output.write_u8(self.comm_player_id.into())?;
        output.write_u8(u8::from(self.dropped))?;
        Ok(())
    }
}

/// Set a waypoint for a group of objects (command `0x10`).
#[derive(Debug, Default, Clone)]
pub struct GroupWaypointCommand {
    /// The ID of the player issuing this command.
    pub player_id: PlayerID,
    /// The waypoint's tile coordinates (x, y).
    pub location: (u8, u8),
    /// The objects receiving the waypoint.
    pub objects: ObjectsList,
}

impl GroupWaypointCommand {
    /// Read a group waypoint command from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    pub fn read_from(mut input: impl Read) -> Result<Self> {
        let player_id = input.read_u8()?.into();
        let num_units = input.read_u8()?;
        let x = input.read_u8()?;
        let y = input.read_u8()?;
        Ok(Self {
            player_id,
            location: (x, y),
            objects: ObjectsList::read_from(input, i32::from(num_units))?,
        })
    }

    /// Write this group waypoint command to an output stream.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails.
    ///
    /// # Panics
    ///
    /// Panics if there are more than 255 objects, the most the format's `u8` count holds.
    pub fn write_to(&self, mut output: impl Write) -> Result<()> {
        output.write_u8(self.player_id.into())?;
        output.write_u8(self.objects.len().try_into().unwrap())?;
        output.write_u8(self.location.0)?;
        output.write_u8(self.location.1)?;
        self.objects.write_to(&mut output)?;
        Ok(())
    }
}

/// Set a group of objects's "AI State" (usually known as "stance").
#[derive(Debug, Default, Clone)]
pub struct UnitAIStateCommand {
    /// The new state. Aggressive/Defensive/No Attack/ etc.
    pub state: i8,
    /// The objects whose AI state is being changed.
    pub objects: ObjectsList,
}

impl UnitAIStateCommand {
    /// Read a `UnitAIState` command from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    pub fn read_from(mut input: impl Read) -> Result<Self> {
        let selected_count = input.read_u8()?;
        let state = input.read_i8()?;
        let objects = ObjectsList::read_from(input, i32::from(selected_count))?;
        Ok(Self { state, objects })
    }

    /// Write this `UnitAIState` command to an output stream.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails.
    ///
    /// # Panics
    ///
    /// Panics if there are more than 255 objects, the most the format's `u8` count holds.
    pub fn write_to<W: Write>(&self, output: &mut W) -> Result<()> {
        output.write_u8(self.objects.len().try_into().unwrap())?;
        output.write_i8(self.state)?;
        self.objects.write_to(output)?;
        Ok(())
    }
}

/// Task units to guard an object.
#[derive(Debug, Default, Clone)]
pub struct GuardCommand {
    /// The target object of this order.
    pub target_id: Option<ObjectID>,
    /// The objects this command applies to.
    pub objects: ObjectsList,
}

impl GuardCommand {
    /// Read a Guard command from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    pub fn read_from(mut input: impl Read) -> Result<Self> {
        let mut command = Self::default();
        let selected_count = i32::from(input.read_u8()?);
        input.skip(2)?;
        command.target_id = read_opt_u32(&mut input)?;
        command.objects = ObjectsList::read_from(input, selected_count)?;
        Ok(command)
    }

    /// Write a Guard command to an output stream.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails.
    ///
    /// # Panics
    ///
    /// Panics if there are more than 255 objects (the format's `u8` count), or if the target ID
    /// is above `i32::MAX`.
    pub fn write_to<W: Write>(&self, output: &mut W) -> Result<()> {
        output.write_u8(self.objects.len().try_into().unwrap())?;
        output.write_all(&[0, 0])?;
        output.write_i32::<LE>(self.target_id.map_or(-1, |id| id.try_into().unwrap()))?;
        self.objects.write_to(output)?;
        Ok(())
    }
}

/// Task units to follow an object.
#[derive(Debug, Default, Clone)]
pub struct FollowCommand {
    /// The target object of this order.
    pub target_id: Option<ObjectID>,
    /// The objects this command applies to.
    pub objects: ObjectsList,
}

impl FollowCommand {
    /// Read a Follow command from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    pub fn read_from(mut input: impl Read) -> Result<Self> {
        let mut command = Self::default();
        let selected_count = i32::from(input.read_u8()?);
        input.skip(2)?;
        command.target_id = read_opt_u32(&mut input)?;
        command.objects = ObjectsList::read_from(input, selected_count)?;
        Ok(command)
    }

    /// Write a Follow command to an output stream.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails.
    ///
    /// # Panics
    ///
    /// Panics if there are more than 255 objects (the format's `u8` count), or if the target ID
    /// is above `i32::MAX`.
    pub fn write_to<W: Write>(&self, output: &mut W) -> Result<()> {
        output.write_u8(self.objects.len().try_into().unwrap())?;
        output.write_all(&[0, 0])?;
        output.write_i32::<LE>(self.target_id.map_or(-1, |id| id.try_into().unwrap()))?;
        self.objects.write_to(output)?;
        Ok(())
    }
}

/// Task a group of objects to patrol along a given path.
#[derive(Debug, Default, Clone)]
pub struct PatrolCommand {
    /// The waypoints that this patrol should pass through.
    pub waypoints: ArrayVec<Location2, 10>,
    /// The objects to include in this formation.
    pub objects: ObjectsList,
}

impl PatrolCommand {
    /// Read a Patrol command from an input stream.
    ///
    /// The format always stores 10 waypoints: first their X coordinates, then their Y coordinates.
    /// Only the first `waypoint_count` of them are kept.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    ///
    /// # Panics
    ///
    /// Panics if the command says it has more than 10 waypoints.
    pub fn read_from(mut input: impl Read) -> Result<Self> {
        let mut command = Self::default();
        let selected_count = input.read_i8()?;
        let waypoint_count = input.read_u8()?;
        let _padding = input.read_u8()?;
        let mut raw_waypoints = [(0.0, 0.0); 10];
        for w in &mut raw_waypoints {
            w.0 = input.read_f32::<LE>()?;
        }
        for w in &mut raw_waypoints {
            w.1 = input.read_f32::<LE>()?;
        }
        command
            .waypoints
            .try_extend_from_slice(&raw_waypoints[0..usize::from(waypoint_count)])
            .unwrap();
        command.objects = ObjectsList::read_from(input, i32::from(selected_count))?;
        Ok(command)
    }

    /// Write this Patrol command to an output stream.
    ///
    /// Unused waypoint slots are written as (0.0, 0.0).
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails.
    ///
    /// # Panics
    ///
    /// Panics if there are more than 127 objects, the most the format's `i8` count holds.
    pub fn write_to<W: Write>(&self, output: &mut W) -> Result<()> {
        output.write_i8(self.objects.len().try_into().unwrap())?;
        output.write_u8(self.waypoints.len().try_into().unwrap())?;
        output.write_u8(0)?;
        for i in 0..10 {
            output.write_f32::<LE>(self.waypoints.get(i).copied().unwrap_or_default().0)?;
        }
        for i in 0..10 {
            output.write_f32::<LE>(self.waypoints.get(i).copied().unwrap_or_default().1)?;
        }
        self.objects.write_to(output)?;
        Ok(())
    }
}

/// Task a group of objects to form a formation.
#[derive(Debug, Default, Clone)]
pub struct FormFormationCommand {
    /// The ID of the player issuing this command.
    pub player_id: PlayerID,
    /// The type of formation to form.
    pub formation_type: i32,
    /// The objects to include in this formation.
    pub objects: ObjectsList,
}

impl FormFormationCommand {
    /// Read a Form Formation command from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    pub fn read_from(mut input: impl Read) -> Result<Self> {
        let mut command = Self::default();
        let selected_count = input.read_i8()?;
        command.player_id = input.read_u8()?.into();
        let _padding = input.read_u8()?;
        command.formation_type = input.read_i32::<LE>()?;
        command.objects = ObjectsList::read_from(input, i32::from(selected_count))?;
        Ok(command)
    }

    /// Write this Form Formation command to an output stream.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails.
    ///
    /// # Panics
    ///
    /// Panics if there are more than 127 objects, the most the format's `i8` count holds.
    pub fn write_to<W: Write>(&self, output: &mut W) -> Result<()> {
        output.write_i8(self.objects.len().try_into().unwrap())?;
        output.write_u8(self.player_id.into())?;
        output.write_u8(0)?;
        output.write_i32::<LE>(self.formation_type)?;
        self.objects.write_to(output)?;
        Ok(())
    }
}

/// Meta-command for `UserPatch`'s new AI commands.
#[derive(Debug, Default, Clone)]
pub struct UserPatchAICommand {
    /// The ID of the player issuing this command.
    pub player_id: PlayerID,
    /// 0: move to object
    /// 1: set unit ai state
    /// 2: ?
    /// 3: ?
    /// 4: stop unit group?
    /// 5: dropoff something?
    /// 6: dropoff something?
    /// 7: ?
    /// 8: set offensive target priority
    /// 9: reset offensive target priorities?
    /// 10: nothing?
    /// 11: stop unit group?
    /// 12: set gather point to garrison in self
    /// 13: set ai player name
    /// 14: unload object
    /// 15: nothing?
    pub ai_action: u8,
    /// The parameters of the AI action: up to 3 `u32` values, as many as the command's size
    /// leaves room for.
    pub params: ArrayVec<u32, 4>,
}

impl UserPatchAICommand {
    /// Read a `UserPatch` AI command of `size` bytes from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    ///
    /// # Panics
    ///
    /// Panics if `size` is below 4, or if it leaves room for more than 3 parameters (20 or
    /// more).
    pub fn read_from(mut input: impl Read, size: u32) -> Result<Self> {
        let num_params = (size - 4) / 4;
        assert!(
            num_params < 4,
            "UserPatchAICommand needs more room for {num_params} params"
        );
        let ai_action = input.read_u8()?;
        let player_id = input.read_u8()?.into();
        let _padding = input.read_u8()?;
        let mut params: ArrayVec<u32, 4> = ArrayVec::default();
        for _ in 0..num_params {
            params.push(input.read_u32::<LE>()?);
        }
        Ok(Self {
            player_id,
            ai_action,
            params,
        })
    }

    /// Write this `UserPatch` AI command to an output stream.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails.
    pub fn write_to<W: Write>(&self, output: &mut W) -> Result<()> {
        output.write_u8(self.ai_action)?;
        output.write_u8(self.player_id.into())?;
        output.write_u8(0)?;
        for p in &self.params {
            output.write_u32::<LE>(*p)?;
        }
        Ok(())
    }
}

/// Train a unit at a building (command `0x64`; aoc-mgz calls it the AI queue command).
#[derive(Debug, Default, Clone, Copy)]
pub struct MakeCommand {
    /// The ID of the player issuing this command.
    pub player_id: PlayerID,
    /// The building that trains the unit.
    pub building_id: ObjectID,
    /// The type of unit to train.
    pub unit_type_id: UnitTypeID,
    /// Not known yet; read as an optional object ID (-1 for none).
    pub target_id: Option<ObjectID>,
}

impl MakeCommand {
    /// Read a Make command from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    pub fn read_from(mut input: impl Read) -> Result<Self> {
        input.skip(3)?;
        let building_id = input.read_u32::<LE>()?.into();
        let player_id = input.read_u8()?.into();
        let _padding = input.read_u8()?;
        let unit_type_id = input.read_u16::<LE>()?.into();
        let target_id = read_opt_u32(&mut input)?;
        Ok(Self {
            player_id,
            building_id,
            unit_type_id,
            target_id,
        })
    }

    /// Write this Make command to an output stream.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails.
    ///
    /// # Panics
    ///
    /// Panics if the target ID is above `i32::MAX`.
    pub fn write_to<W: Write>(&self, output: &mut W) -> Result<()> {
        output.write_all(&[0, 0, 0])?;
        output.write_u32::<LE>(self.building_id.into())?;
        output.write_u8(self.player_id.into())?;
        output.write_u8(0)?;
        output.write_u16::<LE>(self.unit_type_id.into())?;
        output.write_i32::<LE>(self.target_id.map_or(-1, |id| id.try_into().unwrap()))?;
        Ok(())
    }
}

/// Start a research.
#[derive(Debug, Default, Clone, Copy)]
pub struct ResearchCommand {
    /// The ID of the player starting the research.
    pub player_id: PlayerID,
    /// The building where the research is taking place.
    pub building_id: ObjectID,
    /// The tech ID of the research.
    pub tech_id: TechID,
    /// Not known yet; read as an optional object ID (-1 for none).
    pub target_id: Option<ObjectID>,
}

impl ResearchCommand {
    /// Read a Research command from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    pub fn read_from(mut input: impl Read) -> Result<Self> {
        input.skip(3)?;
        let building_id = input.read_u32::<LE>()?.into();
        let player_id = input.read_u8()?.into();
        let _padding = input.read_u8()?;
        let tech_id = input.read_u16::<LE>()?.into();
        let target_id = read_opt_u32(&mut input)?;
        Ok(Self {
            player_id,
            building_id,
            tech_id,
            target_id,
        })
    }

    /// Write this Research command to an output stream.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails.
    ///
    /// # Panics
    ///
    /// Panics if the target ID is above `i32::MAX`.
    pub fn write_to<W: Write>(&self, output: &mut W) -> Result<()> {
        output.write_all(&[0, 0, 0])?;
        output.write_u32::<LE>(self.building_id.into())?;
        output.write_u8(self.player_id.into())?;
        output.write_u8(0)?;
        output.write_u16::<LE>(self.tech_id.into())?;
        output.write_i32::<LE>(self.target_id.map_or(-1, |id| id.try_into().unwrap()))?;
        Ok(())
    }
}

/// Place a building foundation and task a group of villagers to start building.
#[derive(Debug, Default, Clone)]
pub struct BuildCommand {
    /// The ID of the player issuing this command.
    pub player_id: PlayerID,
    /// The type of building to place.
    pub unit_type_id: UnitTypeID,
    /// The location of the new building foundation.
    pub location: Location2,
    /// The index of the frame to use, for buildings with multiple graphics like houses.
    pub frame: u8,
    /// The IDs of the villagers that are tasked to build this building.
    pub builders: ObjectsList,
    /// Unique ID for the _command_ (not building)? Used by AIs?
    unique_id: Option<u32>,
}

impl BuildCommand {
    /// Read a Build command from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    pub fn read_from(mut input: impl Read) -> Result<Self> {
        let mut command = Self::default();
        let selected_count = input.read_i8()?;
        command.player_id = input.read_u8()?.into();
        let _padding = input.read_u8()?;
        let x = input.read_f32::<LE>()?;
        let y = input.read_f32::<LE>()?;
        command.location = (x, y);
        command.unit_type_id = input.read_u16::<LE>()?.into();
        let _padding = input.read_u16::<LE>()?;
        command.unique_id = read_opt_u32(&mut input)?;
        command.frame = input.read_u8()?;
        input.skip(3)?;
        command.builders = ObjectsList::read_from(input, i32::from(selected_count))?;
        Ok(command)
    }
}

/// Commands affecting the game (command `0x67`), one variant per sub-command.
#[derive(Debug, Clone, Copy)]
pub enum GameCommand {
    /// Change the game speed (sub-command 1).
    SetGameSpeed {
        /// The player who sent the command.
        player_id: PlayerID,
        /// The new game speed.
        speed: f32,
    },
    /// Not used in game, but implemented.
    Inventory {
        /// The player who sent the command.
        player_id: PlayerID,
        /// The attribute (resource) to change.
        attribute_id: i16,
        /// The amount.
        amount: f32,
    },
    /// Not implemented in game.
    UpgradeTown {
        /// The player who sent the command.
        player_id: PlayerID,
    },
    /// The quick build cheat (sub-command 4).
    QuickBuild {
        /// The player who sent the command.
        player_id: PlayerID,
    },
    /// Set whether the player wants an allied victory (sub-command 5).
    AlliedVictory {
        /// The player who sent the command.
        player_id: PlayerID,
        /// Whether allied victory is turned on.
        status: bool,
    },
    /// A cheat code (sub-command 6).
    Cheat {
        /// The player who sent the command.
        player_id: PlayerID,
        /// The ID of the cheat.
        cheat_id: i16,
    },
    /// Not implemented in game.
    SharedLos {
        /// The player who sent the command.
        player_id: PlayerID,
    },
    /// Spies, or Treason in later versions (sub-command 0x0a): the player pays to reveal other
    /// players' units.
    Spies {
        /// The player who sent the command.
        player_id: PlayerID,
    },
    /// Set an AI strategic number (sub-command 0x0b), sent by AI players.
    SetStrategicNumber {
        /// The player who sent the command.
        player_id: PlayerID,
        /// The ID of the strategic number.
        strategic_number: i16,
        /// The new value of the strategic number.
        value: i32,
    },
    /// Appears to be unused.
    Unknown0x0c {
        /// The player who sent the command.
        player_id: PlayerID,
    },
    /// Queue farm reseeds at the mill (sub-command 0x0d).
    AddFarmReseedQueue {
        /// The player who sent the command.
        player_id: PlayerID,
        /// The number of reseeds to queue.
        amount: i16,
    },
    /// Remove queued farm reseeds (sub-command 0x0e).
    RemoveFarmReseedQueue {
        /// The player who sent the command.
        player_id: PlayerID,
        /// The number of reseeds to remove.
        amount: i16,
    },
    /// Toggle automatic farm reseeding (sub-command 0x10).
    FarmReseedAutoQueue {
        /// The player who sent the command.
        player_id: PlayerID,
        // TODO unknown vars
    },
}

#[derive(Debug)]
struct RawGameCommand {
    game_command: u8,
    var1: i16,
    var2: i16,
    var3: f32,
    var4: u32,
}

impl RawGameCommand {
    pub fn read_from(mut input: impl Read) -> Result<Self> {
        let game_command = input.read_u8()?;
        let var1 = input.read_i16::<LE>()?;
        let var2 = input.read_i16::<LE>()?;
        let _padding = input.read_u16::<LE>()?;
        let var3 = input.read_f32::<LE>()?;
        let var4 = input.read_u32::<LE>()?;
        Ok(Self {
            game_command,
            var1,
            var2,
            var3,
            var4,
        })
    }
}

impl GameCommand {
    /// Read a game command from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    ///
    /// # Panics
    ///
    /// Panics if the sub-command is not one listed in [`GameCommand`], if the player ID is
    /// negative or above 255, or if a strategic number's value is above `i32::MAX`.
    pub fn read_from(input: impl Read) -> Result<Self> {
        use GameCommand::{
            AddFarmReseedQueue, AlliedVictory, Cheat, FarmReseedAutoQueue, Inventory, QuickBuild,
            RemoveFarmReseedQueue, SetGameSpeed, SetStrategicNumber, SharedLos, Spies, Unknown0x0c,
            UpgradeTown,
        };

        let RawGameCommand {
            game_command,
            var1,
            var2,
            var3,
            var4,
        } = RawGameCommand::read_from(input)?;

        match game_command {
            0x01 => Ok(SetGameSpeed {
                player_id: var1.try_into().unwrap(),
                speed: var3,
            }),
            0x02 => Ok(Inventory {
                player_id: var1.try_into().unwrap(),
                attribute_id: var2,
                amount: var3,
            }),
            0x03 => Ok(UpgradeTown {
                player_id: var1.try_into().unwrap(),
            }),
            0x04 => Ok(QuickBuild {
                player_id: var1.try_into().unwrap(),
            }),
            0x05 => Ok(AlliedVictory {
                player_id: var1.try_into().unwrap(),
                status: var2 != 0,
            }),
            0x06 => Ok(Cheat {
                player_id: var1.try_into().unwrap(),
                cheat_id: var2,
            }),
            0x07 => Ok(SharedLos {
                player_id: var1.try_into().unwrap(),
            }),
            0x0a => Ok(Spies {
                player_id: var1.try_into().unwrap(),
            }),
            0x0b => Ok(SetStrategicNumber {
                player_id: var1.try_into().unwrap(),
                strategic_number: var2,
                value: var4.try_into().unwrap(),
            }),
            0x0c => Ok(Unknown0x0c {
                player_id: var1.try_into().unwrap(),
            }),
            0x0d => Ok(AddFarmReseedQueue {
                player_id: var1.try_into().unwrap(),
                amount: var2,
            }),
            0x0e => Ok(RemoveFarmReseedQueue {
                player_id: var1.try_into().unwrap(),
                amount: var2,
            }),
            0x10 => Ok(FarmReseedAutoQueue {
                player_id: var1.try_into().unwrap(),
            }),
            _ => panic!("unimplemented game command {game_command:#x}"),
        }
    }
}

/// Task a group of villagers to build a wall from point A to point B.
#[derive(Debug, Default, Clone)]
pub struct BuildWallCommand {
    /// The ID of the player issuing this command.
    pub player_id: PlayerID,
    /// The tile (x, y) where the wall starts.
    pub start: (u8, u8),
    /// The tile (x, y) where the wall ends.
    pub end: (u8, u8),
    /// The type of wall to build.
    pub unit_type_id: UnitTypeID,
    /// The villagers that are tasked to build the wall.
    pub builders: ObjectsList,
}

impl BuildWallCommand {
    fn read_from(mut input: impl Read) -> Result<Self> {
        let selected_count = input.read_i8()?;
        let player_id = input.read_u8()?.into();
        let start = (input.read_u8()?, input.read_u8()?);
        let end = (input.read_u8()?, input.read_u8()?);
        let _padding = input.read_u8()?;
        let unit_type_id = input.read_u16::<LE>()?.into();
        let _padding = input.read_u16::<LE>()?;
        assert_eq!(
            input.read_u32::<LE>()?,
            0xFFFF_FFFF,
            "check out what this is for"
        );
        let builders = if selected_count == -1 {
            ObjectsList::SameAsLast
        } else {
            let mut list = vec![0; selected_count.try_into().unwrap()];
            input.read_i32_into::<LE>(&mut list)?;
            if selected_count == 1 && list[0] == -1 {
                list.clear();
            }
            ObjectsList::List(list.into_iter().map(|id| id.try_into().unwrap()).collect())
        };
        Ok(Self {
            player_id,
            start,
            end,
            unit_type_id,
            builders,
        })
    }
}

/// Delete a building or cancel a building that's not fully built yet.
#[derive(Debug, Default, Clone, Copy)]
pub struct CancelBuildCommand {
    /// The ID of the player issuing this command.
    pub player_id: PlayerID,
    /// The ID of the building to cancel.
    pub building_id: ObjectID,
}

impl CancelBuildCommand {
    /// Read a Cancel Build command from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    ///
    /// # Panics
    ///
    /// Panics if the stored player ID is above 255.
    pub fn read_from(mut input: impl Read) -> Result<Self> {
        input.skip(3)?;
        let building_id = input.read_u32::<LE>()?.into();
        let player_id = input.read_u32::<LE>()?.try_into().unwrap();
        Ok(Self {
            player_id,
            building_id,
        })
    }

    /// Write this Cancel Build command to an output stream.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails.
    pub fn write_to<W: Write>(&self, output: &mut W) -> Result<()> {
        output.write_all(&[0, 0, 0])?;
        output.write_u32::<LE>(self.building_id.into())?;
        output.write_u32::<LE>(u32::from(self.player_id))?;
        Ok(())
    }
}

/// Task an object to attack ground.
#[derive(Debug, Default, Clone)]
pub struct AttackGroundCommand {
    /// The target location of this command.
    pub location: Location2,
    /// The objects being tasked.
    pub objects: ObjectsList,
}

impl AttackGroundCommand {
    /// Read a `AttackGround` command from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    pub fn read_from(mut input: impl Read) -> Result<Self> {
        let mut command = Self::default();
        let selected_count = i32::from(input.read_i8()?);
        input.skip(2)?;
        command.location = (input.read_f32::<LE>()?, input.read_f32::<LE>()?);
        command.objects = ObjectsList::read_from(input, selected_count)?;
        Ok(command)
    }

    /// Write this `AttackGround` command to an output stream.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails.
    ///
    /// # Panics
    ///
    /// Panics if there are more than 127 objects, the most the format's `i8` count holds.
    pub fn write_to<W: Write>(&self, output: &mut W) -> Result<()> {
        output.write_i8(self.objects.len().try_into().unwrap())?;
        output.write_all(&[0, 0])?;
        output.write_f32::<LE>(self.location.0)?;
        output.write_f32::<LE>(self.location.1)?;
        self.objects.write_to(output)?;
        Ok(())
    }
}

/// Task units to repair an object.
#[derive(Debug, Default, Clone)]
pub struct RepairCommand {
    /// The target object of this order.
    pub target_id: Option<ObjectID>,
    /// The objects this command applies to.
    pub repairers: ObjectsList,
}

impl RepairCommand {
    /// Read a Repair command from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    pub fn read_from(mut input: impl Read) -> Result<Self> {
        let mut command = Self::default();
        let selected_count = i32::from(input.read_u8()?);
        input.skip(2)?;
        command.target_id = read_opt_u32(&mut input)?;
        command.repairers = ObjectsList::read_from(input, selected_count)?;
        Ok(command)
    }

    /// Write a Repair command to an output stream.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails.
    ///
    /// # Panics
    ///
    /// Panics if there are more than 255 repairers (the format's `u8` count), or if the target
    /// ID is above `i32::MAX`.
    pub fn write_to<W: Write>(&self, output: &mut W) -> Result<()> {
        output.write_u8(self.repairers.len().try_into().unwrap())?;
        output.write_all(&[0, 0])?;
        output.write_i32::<LE>(self.target_id.map_or(-1, |id| id.try_into().unwrap()))?;
        self.repairers.write_to(output)?;
        Ok(())
    }
}

/// Ungarrison objects from a given list of objects.
#[derive(Debug, Default, Clone)]
pub struct UngarrisonCommand {
    /// Which objects to ungarrison. aoc-mgz names the values 0 (all), 3 (selected), 4 (same
    /// type), 5 (not selected) and 6 (inverse type).
    pub ungarrison_type: i8,
    /// The unit type to ungarrison, for the kinds that select by type; read as an optional ID
    /// (-1 for none).
    pub unit_type_id: Option<ObjectID>,
    /// Where the ungarrisoned objects go, if a location was given (it is stored as -1, -1 if
    /// not).
    pub location: Option<Location2>,
    /// The objects to ungarrison from.
    pub objects: ObjectsList,
}

impl UngarrisonCommand {
    fn read_from(mut input: impl Read) -> Result<Self> {
        let mut command = Self::default();
        let selected_count = input.read_i8()?;
        let _padding = input.read_u16::<LE>()?;
        let x = input.read_f32::<LE>()?;
        let y = input.read_f32::<LE>()?;
        command.location = if f32_neq!(x, -1.0) && f32_neq!(y, -1.0) {
            Some((x, y))
        } else {
            None
        };
        command.ungarrison_type = input.read_i8()?;
        input.skip(3)?;
        command.unit_type_id = read_opt_u32(&mut input)?;
        command.objects = ObjectsList::read_from(input, i32::from(selected_count))?;
        Ok(command)
    }
}

/// Send a flare at the given location.
#[derive(Debug, Default, Clone, Copy)]
pub struct FlareCommand {
    /// The ID of the player sending the flare.
    pub player_id: PlayerID,
    /// The multiplayer ID of the player sending the flare.
    pub comm_player_id: PlayerID,
    /// Which players see the flare, indexed by player ID.
    pub recipients: [bool; 9],
    /// The location of the flare.
    pub location: Location2,
}

impl FlareCommand {
    /// Read a Flare command from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    ///
    /// # Panics
    ///
    /// Panics if the unit ID stored before the recipients is not -1.
    pub fn read_from(mut input: impl Read) -> Result<Self> {
        let mut command = Self::default();
        input.skip(3)?;
        assert_eq!(
            input.read_i32::<LE>()?,
            -1,
            "found flare with unexpected unit id"
        );
        for receive in &mut command.recipients {
            *receive = input.read_u8()? != 0;
        }
        input.skip(3)?;
        command.location = (input.read_f32::<LE>()?, input.read_f32::<LE>()?);
        command.player_id = input.read_u8()?.into();
        command.comm_player_id = input.read_u8()?.into();
        input.skip(2)?;
        Ok(command)
    }
}

/// Give objects a unit order (command `0x75`), such as packing or unpacking a trebuchet.
#[derive(Debug, Default, Clone)]
pub struct UnitOrderCommand {
    /// The target object, if any.
    pub target_id: Option<ObjectID>,
    /// The order. aoc-mgz names the values 1 (pack a trebuchet), 2 (unpack a trebuchet) and 5
    /// (garrison).
    pub action: i8,
    /// A parameter of the order, if any (stored as -1 if not). aoc-mgz reads it as the index of
    /// the queue entry to cancel when a production queue is cancelled.
    pub param: Option<u8>,
    /// The target location, if one was given (it is stored as -1, -1 if not).
    pub location: Option<Location2>,
    /// An ID for the command (stored as -1 if absent); its use is not known yet.
    pub unique_id: Option<u32>,
    /// The objects receiving the order.
    pub objects: ObjectsList,
}

impl UnitOrderCommand {
    fn read_from(mut input: impl Read) -> Result<Self> {
        let mut command = Self::default();
        let selected_count = input.read_i8()?;
        let _padding = input.read_u16::<LE>()?;
        command.target_id = read_opt_u32(&mut input)?;
        command.action = input.read_i8()?;
        command.param = match input.read_i8()? {
            -1 => None,
            param => Some(param.cast_unsigned()),
        };
        let _padding = input.read_u16::<LE>()?;
        let x = input.read_f32::<LE>()?;
        let y = input.read_f32::<LE>()?;
        command.location = if f32_neq!(x, -1.0) && f32_neq!(y, -1.0) {
            Some((x, y))
        } else {
            None
        };
        command.unique_id = read_opt_u32(&mut input)?;
        command.objects = ObjectsList::read_from(input, i32::from(selected_count))?;
        Ok(command)
    }
}

/// Queue units of one type at a building (command `0x77`).
#[derive(Debug, Default, Clone, Copy)]
pub struct QueueCommand {
    /// The ID of the building where this unit is being queued.
    pub building_id: ObjectID,
    /// The ID of the unit type being queued.
    pub unit_type_id: UnitTypeID,
    /// The amount of units to queue.
    pub amount: u16,
}

impl QueueCommand {
    /// Read a Queue command from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    pub fn read_from(mut input: impl Read) -> Result<Self> {
        let mut command = Self::default();
        input.skip(3)?;
        command.building_id = input.read_u32::<LE>()?.into();
        command.unit_type_id = input.read_u16::<LE>()?.into();
        command.amount = input.read_u16::<LE>()?;
        Ok(command)
    }

    /// Write this Queue command to an output stream.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails.
    pub fn write_to<W: Write>(&self, output: &mut W) -> Result<()> {
        output.write_all(&[0, 0, 0])?;
        output.write_u32::<LE>(self.building_id.into())?;
        output.write_u16::<LE>(self.unit_type_id.into())?;
        output.write_u16::<LE>(self.amount)?;
        Ok(())
    }
}

/// Set the gather point of one or more buildings (command `0x78`).
#[derive(Debug, Default, Clone)]
pub struct SetGatherPointCommand {
    /// The IDs of the buildings whose gather points are being set.
    pub buildings: ObjectsList,
    /// The ID of the object being targeted, if the gather point is set to an object.
    pub target_id: Option<ObjectID>,
    /// The type ID of the unit being targeted, if the gather point is set to an object.
    pub target_type_id: Option<UnitTypeID>,
    /// The location of the new gather point, if the gather point is not set to an object.
    pub location: Option<Location2>,
}

impl SetGatherPointCommand {
    /// Read a Set Gather Point command from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    pub fn read_from(mut input: impl Read) -> Result<Self> {
        let mut command = Self::default();
        let selected_count = i32::from(input.read_i8()?);
        input.skip(2)?;
        command.target_id = read_opt_u32(&mut input)?;
        command.target_type_id = match input.read_u16::<LE>()? {
            0xFFFF => None,
            id => Some(id.into()),
        };
        input.skip(2)?;
        command.location = Some((input.read_f32::<LE>()?, input.read_f32::<LE>()?));
        command.buildings = ObjectsList::read_from(input, selected_count)?;
        Ok(command)
    }

    /// Write this Set Gather Point command to an output stream.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails.
    ///
    /// # Panics
    ///
    /// Panics if there are more than 255 buildings, the most the format's `u8` count holds.
    pub fn write_to<W: Write>(&self, output: &mut W) -> Result<()> {
        output.write_u8(self.buildings.len().try_into().unwrap())?;
        output.write_all(&[0, 0])?;
        output.write_u32::<LE>(self.target_id.map_or(0xFFFF_FFFF, Into::into))?;
        output.write_u16::<LE>(self.target_type_id.map_or(0xFFFF, Into::into))?;
        output.write_all(&[0, 0])?;
        if let Some((x, y)) = self.location {
            output.write_f32::<LE>(x)?;
            output.write_f32::<LE>(y)?;
        } else {
            output.write_f32::<LE>(0.0)?;
            output.write_f32::<LE>(0.0)?;
        }
        self.buildings.write_to(output)?;
        Ok(())
    }
}

/// Read and write impl for market buying/selling commands, which are different commands but have
/// the same shape.
macro_rules! buy_sell_impl {
    ($name:ident) => {
        impl $name {
            /// Read this market command from an input stream.
            ///
            /// # Errors
            ///
            /// Returns an error if reading from `input` fails.
            pub fn read_from(mut input: impl Read) -> Result<Self> {
                Ok(Self {
                    player_id: input.read_u8()?.into(),
                    resource: input.read_u8()?,
                    amount: input.read_i8()?,
                    market_id: input.read_u32::<LE>()?.into(),
                })
            }

            /// Write this market command to an output stream.
            ///
            /// # Errors
            ///
            /// Returns an error if writing to `output` fails.
            pub fn write_to<W: Write>(&self, output: &mut W) -> Result<()> {
                output.write_u8(self.player_id.into())?;
                output.write_u8(self.resource)?;
                output.write_i8(self.amount)?;
                output.write_u32::<LE>(self.market_id.into())?;
                Ok(())
            }
        }
    };
}

/// Sell a resource at the market.
#[derive(Debug, Default, Clone, Copy)]
pub struct SellResourceCommand {
    /// The ID of the player issuing this command.
    pub player_id: PlayerID,
    /// The resource being sold.
    pub resource: u8,
    /// The amount being sold, in 100s. Typically this is 1 for selling 100 of a resource, or 5 for
    /// selling 500 (with Shift-click).
    pub amount: i8,
    /// The ID of the building where this resource is being bought.
    pub market_id: ObjectID,
}

buy_sell_impl!(SellResourceCommand);

/// Buy a resource at the market.
#[derive(Debug, Default, Clone, Copy)]
pub struct BuyResourceCommand {
    /// The ID of the player issuing this command.
    pub player_id: PlayerID,
    /// The resource being bought.
    pub resource: u8,
    /// The amount being bought, in 100s. Typically this is 1 for buying 100 of a resource, or 5 for
    /// buying 500 (with Shift-click).
    pub amount: i8,
    /// The ID of the building where this resource is being bought.
    pub market_id: ObjectID,
}

buy_sell_impl!(BuyResourceCommand);

/// Command `0x7f`, which aoc-mgz and the DE engine name the town bell: ring or silence the town
/// bell of a town center.
#[derive(Debug, Default, Clone, Copy)]
pub struct Unknown7FCommand {
    /// The town center whose bell is rung.
    pub object_id: ObjectID,
    /// Whether the bell is turned on (non-zero) or off, as aoc-mgz reads it.
    pub value: u32,
}

impl Unknown7FCommand {
    /// Read a `0x7f` (town bell) command from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    pub fn read_from(mut input: impl Read) -> Result<Self> {
        input.skip(3)?;
        let object_id = input.read_u32::<LE>()?.into();
        let value = input.read_u32::<LE>()?;
        Ok(Self { object_id, value })
    }
}

/// Send villagers back to work after they've been garrisoned into the Town Center.
#[derive(Debug, Default, Clone, Copy)]
pub struct BackToWorkCommand {
    /// The building (typically a town center) the villagers leave.
    pub building_id: ObjectID,
}

impl BackToWorkCommand {
    /// Read a Back To Work command from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    pub fn read_from(mut input: impl Read) -> Result<Self> {
        input.skip(3)?;
        let building_id = input.read_u32::<LE>()?.into();
        Ok(Self { building_id })
    }
}

/// A player command.
#[derive(Debug, Clone)]
pub enum Command {
    /// Command `0x00`, see [`OrderCommand`].
    Order(OrderCommand),
    /// Command `0x01`, see [`StopCommand`].
    Stop(StopCommand),
    /// Command `0x02`, see [`WorkCommand`].
    Work(WorkCommand),
    /// Command `0x03`, see [`MoveCommand`].
    Move(MoveCommand),
    /// Command `0x04`, see [`CreateCommand`].
    Create(CreateCommand),
    /// Command `0x05`, see [`AddResourceCommand`].
    AddResource(AddResourceCommand),
    /// Command `0x0a`, see [`AIOrderCommand`].
    AIOrder(AIOrderCommand),
    /// Command `0x0b`, see [`ResignCommand`].
    Resign(ResignCommand),
    /// Command `0x10`, see [`GroupWaypointCommand`].
    GroupWaypoint(GroupWaypointCommand),
    /// Command `0x12`, see [`UnitAIStateCommand`].
    UnitAIState(UnitAIStateCommand),
    /// Command `0x13`, see [`GuardCommand`].
    Guard(GuardCommand),
    /// Command `0x14`, see [`FollowCommand`].
    Follow(FollowCommand),
    /// Command `0x15`, see [`PatrolCommand`].
    Patrol(PatrolCommand),
    /// Command `0x17`, see [`FormFormationCommand`].
    FormFormation(FormFormationCommand),
    /// Command `0x35`, see [`UserPatchAICommand`].
    UserPatchAI(UserPatchAICommand),
    /// Command `0x64`, see [`MakeCommand`].
    Make(MakeCommand),
    /// Command `0x65`, see [`ResearchCommand`].
    Research(ResearchCommand),
    /// Command `0x66`, see [`BuildCommand`].
    Build(BuildCommand),
    /// Command `0x67`, see [`GameCommand`].
    Game(GameCommand),
    /// Command `0x69`, see [`BuildWallCommand`].
    BuildWall(BuildWallCommand),
    /// Command `0x6a`, see [`CancelBuildCommand`].
    CancelBuild(CancelBuildCommand),
    /// Command `0x6b`, see [`AttackGroundCommand`].
    AttackGround(AttackGroundCommand),
    /// Command `0x6e`, see [`RepairCommand`].
    Repair(RepairCommand),
    /// Command `0x6f`, see [`UngarrisonCommand`].
    Ungarrison(UngarrisonCommand),
    /// Command `0x73`, see [`FlareCommand`].
    Flare(FlareCommand),
    /// Command `0x75`, see [`UnitOrderCommand`].
    UnitOrder(UnitOrderCommand),
    /// Command `0x77`, see [`QueueCommand`].
    Queue(QueueCommand),
    /// Command `0x78`, see [`SetGatherPointCommand`].
    SetGatherPoint(SetGatherPointCommand),
    /// Command `0x7a`, see [`SellResourceCommand`].
    SellResource(SellResourceCommand),
    /// Command `0x7b`, see [`BuyResourceCommand`].
    BuyResource(BuyResourceCommand),
    /// Command `0x7f`, see [`Unknown7FCommand`].
    Unknown7F(Unknown7FCommand),
    /// Command `0x80`, see [`BackToWorkCommand`].
    BackToWork(BackToWorkCommand),
}

impl Command {
    /// Read a command from an input stream: its length, its bytes, then the world time.
    ///
    /// Bytes of the command that its reader does not consume are skipped.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    ///
    /// # Panics
    ///
    /// Panics if the command type is not one listed in [`Command`], or if the command's own
    /// reader panics.
    pub fn read_from<R: Read>(input: &mut R) -> Result<Self> {
        let len = input.read_u32::<LE>()?;

        let mut cursor = input.by_ref().take(len.into());
        let command = match cursor.read_u8()? {
            0x00 => OrderCommand::read_from(&mut cursor).map(Command::Order),
            0x01 => StopCommand::read_from(&mut cursor).map(Command::Stop),
            0x02 => WorkCommand::read_from(&mut cursor).map(Command::Work),
            0x03 => MoveCommand::read_from(&mut cursor).map(Command::Move),
            0x04 => CreateCommand::read_from(&mut cursor).map(Command::Create),
            0x05 => AddResourceCommand::read_from(&mut cursor).map(Command::AddResource),
            0x0a => AIOrderCommand::read_from(&mut cursor).map(Command::AIOrder),
            0x0b => ResignCommand::read_from(&mut cursor).map(Command::Resign),
            0x10 => GroupWaypointCommand::read_from(&mut cursor).map(Command::GroupWaypoint),
            0x12 => UnitAIStateCommand::read_from(&mut cursor).map(Command::UnitAIState),
            0x13 => GuardCommand::read_from(&mut cursor).map(Command::Guard),
            0x14 => FollowCommand::read_from(&mut cursor).map(Command::Follow),
            0x15 => PatrolCommand::read_from(&mut cursor).map(Command::Patrol),
            0x17 => FormFormationCommand::read_from(&mut cursor).map(Command::FormFormation),
            0x35 => UserPatchAICommand::read_from(&mut cursor, len).map(Command::UserPatchAI),
            0x64 => MakeCommand::read_from(&mut cursor).map(Command::Make),
            0x65 => ResearchCommand::read_from(&mut cursor).map(Command::Research),
            0x66 => BuildCommand::read_from(&mut cursor).map(Command::Build),
            0x67 => GameCommand::read_from(&mut cursor).map(Command::Game),
            0x69 => BuildWallCommand::read_from(&mut cursor).map(Command::BuildWall),
            0x6a => CancelBuildCommand::read_from(&mut cursor).map(Command::CancelBuild),
            0x6b => AttackGroundCommand::read_from(&mut cursor).map(Command::AttackGround),
            0x6e => RepairCommand::read_from(&mut cursor).map(Command::Repair),
            0x6f => UngarrisonCommand::read_from(&mut cursor).map(Command::Ungarrison),
            0x73 => FlareCommand::read_from(&mut cursor).map(Command::Flare),
            0x75 => UnitOrderCommand::read_from(&mut cursor).map(Command::UnitOrder),
            0x77 => QueueCommand::read_from(&mut cursor).map(Command::Queue),
            0x78 => SetGatherPointCommand::read_from(&mut cursor).map(Command::SetGatherPoint),
            0x7a => SellResourceCommand::read_from(&mut cursor).map(Command::SellResource),
            0x7b => BuyResourceCommand::read_from(&mut cursor).map(Command::BuyResource),
            0x7f => Unknown7FCommand::read_from(&mut cursor).map(Command::Unknown7F),
            0x80 => BackToWorkCommand::read_from(&mut cursor).map(Command::BackToWork),
            id => panic!("unsupported command type {id:#x}"),
        };
        // Consume any excess bytes; how many there were does not matter.
        let _ = std::io::copy(&mut cursor, &mut std::io::sink())?;

        let _world_time = input.read_u32::<LE>()?;
        command
    }
}

/// A time step in the body: how much game time passed since the previous one.
#[derive(Debug, Default, Clone, Copy)]
// `time` is the public field name callers use.
#[allow(clippy::struct_field_names)]
pub struct Time {
    /// The game time that passed in this step, in milliseconds.
    pub time: u32,
    old_world_time: u32,
    unknown: u32,
}

impl Time {
    /// Read a time step from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    pub fn read_from<R: Read>(input: &mut R) -> Result<Self> {
        let mut time = Self {
            time: input.read_u32::<LE>()?,
            ..Default::default()
        };
        let is_old_record = false;
        if is_old_record {
            time.old_world_time = input.read_u32::<LE>()?;
            time.unknown = input.read_u32::<LE>()?;
        }
        Ok(time)
    }
}

/// A Sync message, used to ensure that all players agree on the game state by comparing checksums
/// and world time.
#[derive(Debug, Default, Clone, Copy)]
pub struct Sync {
    /// A checksum of the game state.
    pub checksum: u32,
    /// A checksum of the objects' positions.
    pub position_checksum: u32,
    /// A checksum of the commands. When it is non-zero, 332 bytes of further data follow; they
    /// are skipped.
    pub action_checksum: u32,
    /// The world time of the next sync, as the reader names it; not verified.
    pub next_world_time: u32,
}

impl Sync {
    /// Read a sync message from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    pub fn read_from<R: Read>(input: &mut R) -> Result<Self> {
        let mut sync = Self::default();
        let _always_zero = input.read_u32::<LE>()?;
        sync.checksum = input.read_u32::<LE>()?;
        sync.position_checksum = input.read_u32::<LE>()?;
        sync.action_checksum = input.read_u32::<LE>()?;

        if sync.action_checksum != 0 {
            // From happyleaves:
            // https://github.com/happyleavesaoc/aoc-mgz/blob/30079d29a1cb448b58f83fba5f639017fbd5a2b5/mgz/body/__init__.py#L98
            input.skip(332)?;
        }

        let _always_zero = input.read_u32::<LE>()?;
        sync.next_world_time = input.read_u32::<LE>()?;
        Ok(sync)
    }
}

/// Action at the start of the game, contains settings affecting the rec format.
#[derive(Debug, Default, Clone, Copy)]
pub struct Meta {
    /// The version of the action log format.
    /// `3` for `AoC` 1.0, `4` for `AoC` 1.0c and `UserPatch`.
    pub log_version: Option<u32>,
    /// How many time steps pass between two sync messages.
    pub checksum_interval: u32,
    /// Whether the game was a multiplayer game.
    pub is_multiplayer: bool,
    /// Whether every body operation is preceded by a one-byte sequence number.
    pub use_sequence_numbers: bool,
    /// The ID of the player who recorded the game.
    pub local_player_id: PlayerID,
    /// The fourth `u32` of the metadata. genie-rs reads it as a header position, aoc-mgz as the
    /// reveal-map setting; not verified.
    pub header_position: u32,
    /// The amount of saved chapters in this rec / save game. This is only set if the game version
    /// that generated the file supports saved chapters (i.e. The Conquerors and up).
    pub num_chapters: Option<u32>,
}

impl Meta {
    /// Read the chunk of recorded game body metadata that's the same across all versions.
    fn read_from_inner(mut input: impl Read) -> Result<Self> {
        let checksum_interval = input.read_u32::<LE>()?;
        let is_multiplayer = input.read_u32::<LE>()? != 0;
        let local_player_id = input.read_u32::<LE>()?.try_into().unwrap();
        let header_position = input.read_u32::<LE>()?;
        let use_sequence_numbers = input.read_u32::<LE>()? != 0;
        Ok(Self {
            checksum_interval,
            is_multiplayer,
            use_sequence_numbers,
            local_player_id,
            header_position,
            ..Default::default()
        })
    }

    /// Read recorded game body metadata in the `mgl` format used by Age of Empires 2: The
    /// Age Of Kings.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    ///
    /// # Panics
    ///
    /// Panics if the stored local player ID is above 255.
    pub fn read_from_mgl(mut input: impl Read) -> Result<Self> {
        let meta = Self::read_from_inner(&mut input)?;
        let _exe_file_size = input.read_u64::<LE>()?;
        let _unknown = input.read_f32::<LE>()?;
        let _unknown = input.read_f32::<LE>()?;

        // TODO if `is_multiplayer` flag contains 2 or 3, the `remaining_syncs_until_checksum`
        // value is stored here as u32

        Ok(meta)
    }

    /// Read recorded game body metadata in the `mgx` format used by Age of Empires 2: The
    /// Conquerors and all subsequent versions.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    ///
    /// # Panics
    ///
    /// Panics if the log version is not 3, 4 or 5, or if the stored local player ID is above 255.
    pub fn read_from_mgx(mut input: impl Read) -> Result<Self> {
        let log_version = input.read_u32::<LE>()?;
        assert!(matches!(log_version, 3..=5));
        let mut meta = Self::read_from_inner(&mut input)?;
        meta.log_version = Some(log_version);
        if log_version == 5 {
            // One of these is likely num_chapters, but not sure which.
            let _unknown = input.read_u32::<LE>()?;
            let _unknown = input.read_u32::<LE>()?;
        } else {
            meta.num_chapters = Some(input.read_u32::<LE>()?);
        }
        Ok(meta)
    }
}

/// A chat message sent during the game.
#[derive(Debug, Clone)]
pub struct Chat {
    #[allow(dead_code)]
    message: String,
}

impl Chat {
    /// Read a chat message from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails or the text is not valid WINDOWS-1252.
    ///
    /// # Panics
    ///
    /// Panics if the `i32` stored before the text is not -1.
    pub fn read_from<R: Read>(input: &mut R) -> Result<Self> {
        assert_eq!(input.read_i32::<LE>()?, -1);
        let message = input.read_u32_length_prefixed_str()?.unwrap_or_default();
        Ok(Self { message })
    }
}

/// One operation of the recorded game body.
#[derive(Debug, Clone)]
pub enum Action {
    /// A player command (operation 1).
    Command(Command),
    /// A time step (operation 2).
    Time(Time),
    /// A sync message (operation 0).
    Sync(Sync),
    /// A view lock (operation 3).
    ViewLock(ViewLock),
    /// A chat message (operation 4).
    Chat(Chat),
}
