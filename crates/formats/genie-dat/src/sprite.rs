//! Types related to sprites and graphics.

use crate::sound::SoundID;
use arrayvec::ArrayVec;
use byteorder::{LE, ReadBytesExt, WriteBytesExt};
pub use genie_support::SpriteID;
use genie_support::{MapInto, fallible_try_into, infallible_try_into, read_opt_u16};
use std::convert::{TryFrom, TryInto};
use std::io::{Read, Result, Write};
use std::num::TryFromIntError;

/// An ID identifying a string resource.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct GraphicID(u32);

impl From<u16> for GraphicID {
    fn from(n: u16) -> Self {
        Self(n.into())
    }
}

impl From<u32> for GraphicID {
    fn from(n: u32) -> Self {
        Self(n)
    }
}

impl TryFrom<i32> for GraphicID {
    type Error = TryFromIntError;
    fn try_from(n: i32) -> std::result::Result<Self, Self::Error> {
        Ok(Self(n.try_into()?))
    }
}

fallible_try_into!(GraphicID, i16);
infallible_try_into!(GraphicID, u32);
fallible_try_into!(GraphicID, i32);

/// Another sprite drawn as part of this one, at an offset (e.g. the parts of a building).
#[derive(Debug, Default, Clone, Copy)]
pub struct SpriteDelta {
    /// The sprite to draw, or `None` (-1).
    pub sprite_id: Option<SpriteID>,
    /// Horizontal offset from this sprite's anchor, in pixels.
    pub offset_x: i16,
    /// Vertical offset from this sprite's anchor, in pixels.
    pub offset_y: i16,
    /// The facing angle at which the delta is drawn; -1 draws it at every angle.
    pub display_angle: i16,
}

/// A sound played during an attack animation, after a delay.
#[derive(Debug, Default, Clone, Copy)]
pub struct SoundProp {
    /// How long into the animation the sound starts.
    pub sound_delay: i16,
    /// The sound to play; 0xFFFF marks an unused slot.
    pub sound_id: SoundID,
    #[allow(dead_code)]
    wwise_sound_id: Option<u32>,
}

/// The attack sounds of one facing angle: up to three sounds with their delays.
#[derive(Debug, Default, Clone)]
pub struct SpriteAttackSound {
    /// The used sound slots; unused slots (sound 0xFFFF) are dropped when reading.
    pub sound_props: ArrayVec<SoundProp, 3>,
}

/// A sprite (graphic): an SLP image with its frames and angles, the sprites drawn with it, and
/// its sounds.
#[derive(Debug, Default, Clone)]
pub struct Sprite {
    /// ID of this sprite.
    pub id: SpriteID,
    /// Internal name (at most 21 bytes in the file).
    pub name: String,
    /// File name of the SLP (at most 13 bytes in the file).
    pub filename: String,
    /// The SLP resource ID for this sprite.
    pub slp_id: Option<GraphicID>,
    /// Whether the game has loaded the image: a runtime flag the file keeps unused.
    pub is_loaded: bool,
    /// If `Some(id)`, the sprite will always be rendered with this player colour.
    force_player_color: Option<u8>,
    /// The layer describes order of graphics being rendered.
    /// Possible values: 0 (lowest layer) to 40 (highest layer)
    /// Graphics on a higher layer will be rendered above graphics of a lower
    /// layer. If graphics share the same layer, graphics will be displayed
    /// dependend on their map positions.
    ///
    /// Draw Level
    /// ```txt
    /// 0   Terrain
    /// 5   Shadows, farms
    /// 6   Rubble
    /// 10   Constructions, corpses, shadows, flowers, ruins
    /// 11   Fish
    /// 19   Rugs, craters
    /// 20   Buildings, units, damage flames, mill animation
    /// 21   Blacksmith smoke
    /// 22   Hawk
    /// 30   Projectiles, explosions
    /// ```
    pub layer: u8,
    /// Two bytes read as one: genieutils reads them as the player colour the sprite is drawn
    /// with (`PlayerColor`) and a flag whether that colour changes with the owner (`Rainbow`).
    pub color_table: u16,
    /// genieutils' `TransparentSelection` flag; its exact effect is not known yet.
    pub transparent_selection: bool,
    /// The sprite's extent as four coordinates (genieutils' `Coordinates`).
    pub bounding_box: (i16, i16, i16, i16),
    /// Sound played with the animation, or `None`.
    pub sound_id: Option<SoundID>,
    /// Number of frames per angle animation
    pub num_frames: u16,
    /// Number of angles tored in slp and also the number of extra structures.
    /// If there are more than 1 angle, AngleCount/2 - 1 frames will be
    /// mirrored. That means angles starting from south going clockwise to
    /// north are stored and the others will be mirrored.
    pub num_angles: u16,
    /// If this is over 0, the speed of the unit will be replaced with this.
    pub base_speed: f32,
    /// Frame rate in seconds. (Delay between frames)
    pub frame_rate: f32,
    /// Time to wait until the animation sequence is started again.
    pub replay_delay: f32,
    /// Animation sequence type (genieutils' `SequenceType`); not decoded here.
    pub sequence_type: u8,
    /// How the angles that are not stored are mirrored (genieutils' `MirroringMode`).
    pub mirror_flag: i8,
    /// editor flag?
    other_flag: i8,
    /// Sprites drawn as part of this one.
    pub deltas: Vec<SpriteDelta>,
    /// Attack sounds, one entry per angle; empty if the sprite has none.
    pub attack_sounds: Vec<SpriteAttackSound>,
}

impl SpriteDelta {
    /// Read a sprite delta from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    pub fn read_from(mut input: impl Read) -> Result<Self> {
        let mut delta = Self {
            sprite_id: read_opt_u16(&mut input)?,
            ..Default::default()
        };
        let _padding = input.read_i16::<LE>()?;
        let _parent_sprite_pointer = input.read_i32::<LE>()?;
        delta.offset_x = input.read_i16::<LE>()?;
        delta.offset_y = input.read_i16::<LE>()?;
        delta.display_angle = input.read_i16::<LE>()?;
        let _padding = input.read_i16::<LE>()?;

        Ok(delta)
    }

    /// Write this sprite delta to an output stream.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails.
    pub fn write_to<W: Write>(&self, output: &mut W) -> Result<()> {
        output.write_u16::<LE>(self.sprite_id.map_into().unwrap_or(0xFFFF))?;
        // padding
        output.write_i16::<LE>(0)?;
        // pointer address to the parent sprite (overridden at load time by the game)
        output.write_i32::<LE>(0)?;
        output.write_i16::<LE>(self.offset_x)?;
        output.write_i16::<LE>(self.offset_y)?;
        output.write_i16::<LE>(self.display_angle)?;
        // padding
        output.write_i16::<LE>(0)?;

        Ok(())
    }
}

impl SoundProp {
    /// Read an attack sound slot from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    pub fn read_from<R: Read>(input: &mut R) -> Result<Self> {
        let sound_delay = input.read_i16::<LE>()?;
        let sound_id = input.read_u16::<LE>()?.into();
        Ok(Self {
            sound_delay,
            sound_id,
            wwise_sound_id: None,
        })
    }

    /// Write this attack sound slot to an output stream.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails.
    pub fn write_to<W: Write>(self, output: &mut W) -> Result<()> {
        output.write_i16::<LE>(self.sound_delay)?;
        output.write_u16::<LE>(self.sound_id.into())?;
        Ok(())
    }

    /// Write an unused attack sound slot (delay 0, sound 0xFFFF).
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails.
    pub fn write_empty<W: Write>(output: &mut W) -> Result<()> {
        output.write_u16::<LE>(0)?;
        output.write_u16::<LE>(0xFFFF)?;
        Ok(())
    }
}

impl SpriteAttackSound {
    /// Read the three attack sound slots of one angle from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails.
    pub fn read_from<R: Read>(input: &mut R) -> Result<Self> {
        let mut val = Self::default();
        for _ in 0..val.sound_props.capacity() {
            let prop = SoundProp::read_from(input)?;
            if u16::from(prop.sound_id) != 0xFFFF {
                val.sound_props.push(prop);
            }
        }
        Ok(val)
    }

    /// Write the three attack sound slots of one angle, padding with unused slots.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails.
    pub fn write_to<W: Write>(&self, output: &mut W) -> Result<()> {
        for index in 0..self.sound_props.capacity() {
            match self.sound_props.get(index) {
                Some(prop) => prop.write_to(output)?,
                None => SoundProp::write_empty(output)?,
            }
        }
        Ok(())
    }
}

impl Sprite {
    /// Read a sprite from an input stream.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails, or if an ID is out of range.
    ///
    /// # Panics
    ///
    /// Panics if the name or file name is not valid UTF-8, or if the SLP ID is negative but not
    /// -1.
    pub fn read_from(mut input: impl Read) -> Result<Self> {
        let mut sprite = Self::default();
        let mut name = [0u8; 21];
        input.read_exact(&mut name)?;
        sprite.name =
            String::from_utf8(name.iter().copied().take_while(|b| *b != 0).collect()).unwrap();
        let mut filename = [0u8; 13];
        input.read_exact(&mut filename)?;
        sprite.filename =
            String::from_utf8(filename.iter().copied().take_while(|b| *b != 0).collect()).unwrap();
        sprite.slp_id = {
            let num = input.read_i32::<LE>()?;
            if num == -1 {
                None
            } else {
                Some(num.try_into().unwrap())
            }
        };
        sprite.is_loaded = input.read_u8()? != 0;
        sprite.force_player_color = match input.read_u8()? {
            0xFF => None,
            id => Some(id),
        };
        sprite.layer = input.read_u8()?;
        sprite.color_table = input.read_u16::<LE>()?;
        sprite.transparent_selection = input.read_u8()? != 0;
        sprite.bounding_box = (
            input.read_i16::<LE>()?,
            input.read_i16::<LE>()?,
            input.read_i16::<LE>()?,
            input.read_i16::<LE>()?,
        );
        let num_deltas = input.read_u16::<LE>()?;
        sprite.sound_id = read_opt_u16(&mut input)?;
        let attack_sounds_used = input.read_u8()? != 0;
        sprite.num_frames = input.read_u16::<LE>()?;
        sprite.num_angles = input.read_u16::<LE>()?;
        sprite.base_speed = input.read_f32::<LE>()?;
        sprite.frame_rate = input.read_f32::<LE>()?;
        sprite.replay_delay = input.read_f32::<LE>()?;
        sprite.sequence_type = input.read_u8()?;
        sprite.id = input.read_u16::<LE>()?.into();
        sprite.mirror_flag = input.read_i8()?;
        sprite.other_flag = input.read_i8()?;

        for _ in 0..num_deltas {
            sprite.deltas.push(SpriteDelta::read_from(&mut input)?);
        }
        if attack_sounds_used {
            for _ in 0..sprite.num_angles {
                sprite
                    .attack_sounds
                    .push(SpriteAttackSound::read_from(&mut input)?);
            }
        }

        Ok(sprite)
    }

    /// Write this sprite to an output stream.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails, or if the name or file name is longer than
    /// its fixed-size field.
    ///
    /// # Panics
    ///
    /// Panics if there are attack sounds but not one per angle, if there are more than
    /// `u16::MAX` deltas, or if the SLP or sound ID does not fit the format's signed field.
    pub fn write_to<W: Write>(&self, output: &mut W) -> Result<()> {
        if !self.attack_sounds.is_empty() {
            assert_eq!(self.attack_sounds.len(), usize::from(self.num_angles));
        }
        let mut name = [0u8; 21];
        (&mut name[..]).write_all(self.name.as_bytes())?;
        let mut filename = [0u8; 13];
        (&mut filename[..]).write_all(self.filename.as_bytes())?;
        output.write_all(&name)?;
        output.write_all(&filename)?;
        output.write_i32::<LE>(self.slp_id.map_or(-1, |v| v.try_into().unwrap()))?;
        output.write_u8(u8::from(self.is_loaded))?;
        output.write_u8(self.force_player_color.unwrap_or(0xFF))?;
        output.write_u8(self.layer)?;
        output.write_u16::<LE>(self.color_table)?;
        output.write_u8(u8::from(self.transparent_selection))?;
        output.write_i16::<LE>(self.bounding_box.0)?;
        output.write_i16::<LE>(self.bounding_box.1)?;
        output.write_i16::<LE>(self.bounding_box.2)?;
        output.write_i16::<LE>(self.bounding_box.3)?;

        output.write_u16::<LE>(self.deltas.len().try_into().unwrap())?;
        output.write_i16::<LE>(self.sound_id.map_or(-1, |v| v.try_into().unwrap()))?;
        output.write_u8(u8::from(!self.attack_sounds.is_empty()))?;
        output.write_u16::<LE>(self.num_frames)?;
        output.write_u16::<LE>(self.num_angles)?;
        output.write_f32::<LE>(self.base_speed)?;
        output.write_f32::<LE>(self.frame_rate)?;
        output.write_f32::<LE>(self.replay_delay)?;
        output.write_u8(self.sequence_type)?;
        output.write_u16::<LE>(self.id.into())?;
        output.write_i8(self.mirror_flag)?;
        output.write_i8(self.other_flag)?;

        for delta in &self.deltas {
            delta.write_to(output)?;
        }
        for sound in &self.attack_sounds {
            sound.write_to(output)?;
        }
        Ok(())
    }
}
