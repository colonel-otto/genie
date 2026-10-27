//! Read and write player AI state.
//!
//! A computer player's AI state is saved as a chain of modules (main, build, construction,
//! diplomacy, emotional, information, resource, strategy). The field names follow the genie-rs
//! reader and the AI's own terms; values whose meaning is not known say so.

use crate::element::ReadableHeaderElement;
use crate::reader::RecordingHeaderReader;
use crate::{ObjectID, Result};
use byteorder::{LE, ReadBytesExt};
use std::io::Read;

mod build;
mod information;
mod memories;
mod strategy;

pub use self::build::{BuildAI, BuildItem, ConstructionAI, ConstructionItem};
pub use self::information::InformationAI;
pub use self::memories::{
    AttackMemory, BuildingLot, ImportantObjectMemory, InfluenceMap, PerimeterWall, QuadrantLog,
    ResourceMemory, WallLine,
};
pub use self::strategy::{DiplomacyAI, EmotionalAI, ResourceAI, StrategyAI, TacticalAI};

/// The main AI module.
#[derive(Debug, Default, Clone)]
pub struct MainAI {
    /// The object IDs the main module lists.
    pub objects: Vec<ObjectID>,
}

impl ReadableHeaderElement for MainAI {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        let num_objects = input.read_u32::<LE>()?;
        let mut objects = vec![];
        for _ in 0..num_objects {
            objects.push(input.read_u32::<LE>()?.into());
        }
        Ok(Self { objects })
    }
}

/// The AI state of one computer player: its AI modules in file order.
#[derive(Debug, Default, Clone)]
pub struct PlayerAI {
    /// The main AI module.
    pub main_ai: MainAI,
    /// The build AI module.
    pub build_ai: BuildAI,
    /// The construction AI module.
    pub construction_ai: ConstructionAI,
    /// The diplomacy AI module.
    pub diplomacy_ai: DiplomacyAI,
    /// The emotional AI module.
    pub emotional_ai: EmotionalAI,
    /// The information AI module.
    pub information_ai: InformationAI,
    /// The resource AI module.
    pub resource_ai: ResourceAI,
    /// The strategy AI module.
    pub strategy_ai: StrategyAI,
}

impl ReadableHeaderElement for PlayerAI {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        let main_ai = MainAI::read_from(input)?;
        let build_ai = BuildAI::read_from(input)?;
        let construction_ai = ConstructionAI::read_from(input)?;
        let diplomacy_ai = DiplomacyAI::read_from(input)?;
        let emotional_ai = EmotionalAI::read_from(input)?;
        let information_ai = InformationAI::read_from(input)?;
        let resource_ai = ResourceAI::read_from(input)?;
        let strategy_ai = StrategyAI::read_from(input)?;

        Ok(Self {
            main_ai,
            build_ai,
            construction_ai,
            diplomacy_ai,
            emotional_ai,
            information_ai,
            resource_ai,
            strategy_ai,
        })
    }
}

fn read_id_list<T: From<u32>, R: Read>(input: &mut R) -> Result<Vec<T>> {
    let len = input.read_u32::<LE>()?;
    let mut ids = vec![];
    for _ in 0..len {
        ids.push(input.read_u32::<LE>()?.into());
    }
    Ok(ids)
}

#[cfg(test)]
mod tests;
