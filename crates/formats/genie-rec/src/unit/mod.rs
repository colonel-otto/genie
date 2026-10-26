use crate::Result;
use crate::element::{OptionalReadableElement, ReadableHeaderElement, WritableHeaderElement};
use crate::reader::RecordingHeaderReader;
use crate::unit_type::UnitBaseClass;
use byteorder::{ReadBytesExt, WriteBytesExt};
pub use genie_dat::sprite::SpriteID;
pub use genie_dat::terrain::TerrainID;
pub use genie_dat::unit_type::AttributeCost;
pub use genie_support::{StringKey, UnitTypeID};
use std::convert::TryInto;
use std::io::{Read, Write};

mod building;
mod combat;
mod movers;
mod sprites;
mod statics;
mod unit_ai;

pub use self::building::{BuildingUnitAttributes, GatherPoint, ProductionQueueEntry};
pub use self::combat::{BaseCombatUnitAttributes, CombatUnitAttributes, MissileUnitAttributes};
pub use self::movers::{ActionUnitAttributes, MovementData, MovingUnitAttributes, PathData};
pub use self::sprites::{SpriteList, SpriteNode, SpriteNodeAnimation};
pub use self::statics::{AnimatedUnitAttributes, StaticUnitAttributes};
pub use self::unit_ai::{
    PatrolPath, UnitAI, UnitAINotification, UnitAIOrder, UnitAIOrderHistory, UnitAIRetargetEntry,
    Waypoint,
};

/// A game object (unit, building, resource …) as the recording header stores it, with the
/// attribute groups of its base class.
#[derive(Debug, Clone)]
pub struct Unit {
    /// The object's base class; it decides which of the attribute groups below are present.
    pub unit_base_class: UnitBaseClass,
    /// The attributes every object has.
    pub static_: StaticUnitAttributes,
    /// Present from the animated base class on.
    pub animated: Option<AnimatedUnitAttributes>,
    /// Present from the moving base class on.
    pub moving: Option<MovingUnitAttributes>,
    /// Present from the action base class on.
    pub action: Option<ActionUnitAttributes>,
    /// Present from the base combat base class on.
    pub base_combat: Option<BaseCombatUnitAttributes>,
    /// Present from the missile base class on.
    pub missile: Option<MissileUnitAttributes>,
    /// Present from the combat base class on.
    pub combat: Option<CombatUnitAttributes>,
    /// Present for buildings.
    pub building: Option<BuildingUnitAttributes>,
}

impl OptionalReadableElement for Unit {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Option<Self>> {
        let raw_class = input.read_u8()?;
        if raw_class == 0 {
            return Ok(None);
        }
        let unit_base_class = raw_class.try_into().unwrap();
        let static_ = StaticUnitAttributes::read_from(input)?;
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
            unit.animated = Some(AnimatedUnitAttributes::read_from(input)?);
        }
        if unit_base_class >= UnitBaseClass::Moving {
            unit.moving = Some(MovingUnitAttributes::read_from(input)?);
        }
        if unit_base_class >= UnitBaseClass::Action {
            unit.action = Some(ActionUnitAttributes::read_from(input)?);
        }
        if unit_base_class >= UnitBaseClass::BaseCombat {
            unit.base_combat = Some(BaseCombatUnitAttributes::read_from(input)?);
        }
        if unit_base_class >= UnitBaseClass::Missile {
            unit.missile = Some(MissileUnitAttributes::read_from(input)?);
        }
        if unit_base_class >= UnitBaseClass::Combat {
            unit.combat = Some(CombatUnitAttributes::read_from(input)?);
        }
        if unit_base_class >= UnitBaseClass::Building {
            unit.building = Some(BuildingUnitAttributes::read_from(input)?);
        }
        Ok(Some(unit))
    }
}

impl WritableHeaderElement for Unit {
    fn write_to<W: Write>(&self, output: &mut W) -> Result<()> {
        let raw_class = self.unit_base_class as u8;
        output.write_u8(raw_class)?;
        self.static_.write_to(output)?;
        if let Some(animated) = &self.animated {
            animated.write_to(output)?;
        }
        if let Some(moving) = &self.moving {
            moving.write_to(output)?;
        }
        if let Some(action) = &self.action {
            action.write_to(output)?;
        }
        if let Some(base_combat) = &self.base_combat {
            base_combat.write_to(output)?;
        }
        if let Some(missile) = &self.missile {
            missile.write_to(output)?;
        }
        if let Some(combat) = &self.combat {
            combat.write_to(output)?;
        }
        Ok(())
    }
}
