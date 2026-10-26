//! A unit type ([`UnitType`]): its base class and the parts it has, read and written.

use crate::unit_type::{
    ActionUnitTypeAttributes, AnimatedUnitTypeAttributes, BaseCombatUnitTypeAttributes,
    BuildingUnitTypeAttributes, CombatUnitTypeAttributes, MissileUnitTypeAttributes,
    MovingUnitTypeAttributes, StaticUnitTypeAttributes, UnitBaseClass,
};
use byteorder::{ReadBytesExt, WriteBytesExt};
use std::convert::TryInto;
use std::io::{Read, Result, Write};

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
    ///
    /// [`SpriteID`]: crate::sprite::SpriteID
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
