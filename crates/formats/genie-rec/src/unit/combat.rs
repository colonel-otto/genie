//! The base combat, missile and combat base classes' attributes.

use crate::element::{ReadableHeaderElement, WritableHeaderElement};
use crate::reader::RecordingHeaderReader;
use crate::unit::UnitAI;
use crate::{ObjectID, Result};
use byteorder::{LE, ReadBytesExt};
use genie_dat::unit_type::UnitType;
use genie_support::read_opt_u32;
use std::io::Read;

/// The attributes the base combat base class adds.
#[derive(Debug, Default, Clone, Copy)]
pub struct BaseCombatUnitAttributes {
    /// The formation the unit is in.
    pub formation_id: u8,
    /// The unit's row in the formation.
    pub formation_row: u8,
    /// The unit's column in the formation.
    pub formation_column: u8,
    /// The time until the next attack.
    pub attack_timer: f32,
    /// Not known yet; a capture flag.
    pub capture_flag: u8,
    /// Not known yet.
    pub multi_unified_points: u8,
    /// Not known yet.
    pub large_object_radius: u8,
    /// The number of attacks the unit has made.
    pub attack_count: u32,
}

impl ReadableHeaderElement for BaseCombatUnitAttributes {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        let mut attrs = Self::default();
        if input.version() >= 9.05 {
            attrs.formation_id = input.read_u8()?;
            attrs.formation_row = input.read_u8()?;
            attrs.formation_column = input.read_u8()?;
        }
        attrs.attack_timer = input.read_f32::<LE>()?;
        if input.version() >= 2.01 {
            attrs.capture_flag = input.read_u8()?;
        }
        if input.version() >= 9.09 {
            attrs.multi_unified_points = input.read_u8()?;
            attrs.large_object_radius = input.read_u8()?;
        }
        if input.version() >= 10.02 {
            attrs.attack_count = input.read_u32::<LE>()?;
        }
        Ok(attrs)
    }
}

impl WritableHeaderElement for BaseCombatUnitAttributes {}

/// The attributes the missile (projectile) base class adds.
#[derive(Debug, Default, Clone)]
pub struct MissileUnitAttributes {
    /// The furthest the projectile flies.
    pub max_range: f32,
    /// The object that fired the projectile.
    pub fired_from_id: ObjectID,
    /// The object's own copy of its unit type, if it has one.
    pub own_base: Option<UnitType>,
}

impl ReadableHeaderElement for MissileUnitAttributes {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        Ok(Self {
            max_range: input.read_f32::<LE>()?,
            fired_from_id: input.read_u32::<LE>()?.into(),
            own_base: {
                if input.read_u8()? == 0 {
                    None
                } else {
                    let version = input.version();
                    Some(UnitType::read_from(&mut *input, version)?)
                }
            },
        })
    }
}

impl WritableHeaderElement for MissileUnitAttributes {}

/// The attributes the combat base class adds.
#[derive(Debug, Default, Clone)]
pub struct CombatUnitAttributes {
    /// Not known yet; tracks the next volley.
    pub next_volley: u8,
    /// Whether the unit uses its special attack animation.
    pub using_special_attack_animation: u8,
    /// The object's own copy of its unit type, if it has one.
    pub own_base: Option<UnitType>,
    /// Six attribute amounts; not known yet which.
    pub attribute_amounts: [u16; 6],
    /// The decay timer; stored from save version 9.16 on.
    pub decay_timer: u16,
    /// Not known yet; stored from save version 9.61 on.
    pub raider_build_countdown: u32,
    /// Not known yet; stored from save version 9.65 on.
    pub locked_down_count: u32,
    /// The number of units garrisoned inside; stored from save version 11.56 on.
    pub inside_garrison_count: u8,
    /// The unit's AI, if it has one.
    pub unit_ai: Option<UnitAI>,
    /// The town bell state; stored from save version 10.30 on.
    pub town_bell_flag: i8,
    /// The object the town bell sent the unit to, if any.
    pub town_bell_target_id: Option<ObjectID>,
    /// The position the town bell sent the unit to, if any.
    pub town_bell_target_location: Option<(f32, f32)>,
    /// A second town bell target object; stored from save version 11.71 on.
    pub town_bell_target_id_2: Option<ObjectID>,
    /// The town bell target's type; stored from save version 11.71 on.
    pub town_bell_target_type: u32,
    /// The town bell action; stored from save version 11.74 on.
    pub town_bell_action: u32,
    /// The hit point regeneration timer (as Berserkers have); stored from save version
    /// 10.42 on.
    pub berserker_timer: f32,
    /// The number of units building the object; stored from save version 10.46 on.
    pub num_builders: u8,
    /// The number of units healing the object; stored from save version 11.69 on.
    pub num_healers: u8,
}

impl ReadableHeaderElement for CombatUnitAttributes {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        let mut attrs = Self {
            next_volley: input.read_u8()?,
            using_special_attack_animation: input.read_u8()?,
            own_base: {
                if input.read_u8()? == 0 {
                    None
                } else {
                    let version = input.version();
                    Some(UnitType::read_from(&mut *input, version)?)
                }
            },
            ..Default::default()
        };
        for amount in &mut attrs.attribute_amounts {
            *amount = input.read_u16::<LE>()?;
        }
        if input.version() >= 9.16 {
            attrs.decay_timer = input.read_u16::<LE>()?;
        }
        if input.version() >= 9.61 {
            attrs.raider_build_countdown = input.read_u32::<LE>()?;
        }
        if input.version() >= 9.65 {
            attrs.locked_down_count = input.read_u32::<LE>()?;
        }
        if input.version() >= 11.56 {
            attrs.inside_garrison_count = input.read_u8()?;
        }
        attrs.unit_ai = {
            let has_ai = input.read_u32::<LE>()? != 0;
            if has_ai {
                Some(UnitAI::read_from(input)?)
            } else {
                None
            }
        };
        if input.version() >= 10.30 {
            attrs.town_bell_flag = input.read_i8()?;
            attrs.town_bell_target_id = read_opt_u32(input)?;
            attrs.town_bell_target_location = {
                let location = (input.read_f32::<LE>()?, input.read_f32::<LE>()?);
                if location.0 >= 0.0 {
                    Some(location)
                } else {
                    None
                }
            };
        }
        if input.version() >= 11.71 {
            attrs.town_bell_target_id_2 = read_opt_u32(input)?;
            attrs.town_bell_target_type = input.read_u32::<LE>()?;
        }
        if input.version() >= 11.74 {
            attrs.town_bell_action = input.read_u32::<LE>()?;
        }
        if input.version() >= 10.42 {
            attrs.berserker_timer = input.read_f32::<LE>()?;
        }
        if input.version() >= 10.46 {
            attrs.num_builders = input.read_u8()?;
        }
        if input.version() >= 11.69 {
            attrs.num_healers = input.read_u8()?;
        }
        Ok(attrs)
    }
}

impl WritableHeaderElement for CombatUnitAttributes {}
