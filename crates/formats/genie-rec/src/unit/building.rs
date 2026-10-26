//! The building base class's attributes: the gather point and the production queue.

use crate::element::ReadableHeaderElement;
use crate::reader::RecordingHeaderReader;
use crate::unit_action::UnitAction;
use crate::{ObjectID, Result};
use arrayvec::ArrayVec;
use byteorder::{LE, ReadBytesExt};
use genie_dat::terrain::TerrainID;
use genie_support::{UnitTypeID, read_opt_u32};
use std::convert::TryInto;
use std::io::Read;

/// Where a building sends the units it trains.
#[derive(Debug, Clone, Copy)]
pub enum GatherPoint {
    /// A position.
    Location {
        /// The x coordinate.
        x: f32,
        /// The y coordinate.
        y: f32,
        /// The z coordinate (height).
        z: f32,
    },
    /// An object, with its unit type.
    Object {
        /// The object.
        id: ObjectID,
        /// The object's unit type.
        unit_type: UnitTypeID,
    },
}

/// An entry of a building's production queue.
#[derive(Debug, Default, Clone, Copy)]
pub struct ProductionQueueEntry {
    /// The unit type to train.
    pub unit_type_id: UnitTypeID,
    /// How many to train.
    pub count: u16,
}

impl ReadableHeaderElement for ProductionQueueEntry {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        let unit_type_id = input.read_u16::<LE>()?.into();
        let count = input.read_u16::<LE>()?;
        Ok(Self {
            unit_type_id,
            count,
        })
    }
}

/// The attributes the building base class adds.
// The bools are the format's on/off flags.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Default, Clone)]
pub struct BuildingUnitAttributes {
    /// Is this building fully built?
    pub built: bool,
    /// Number of build points: how much work villagers have to do to build this building.
    pub build_points: f32,
    /// The build item ID for this building. Only used by AIs.
    pub unique_build_id: Option<u32>,
    /// The culture / architecture graphics set to use for this building.
    pub culture: u8,
    /// Is this building on fire?
    pub burning: u8,
    /// The time the building last burned.
    pub last_burn_time: u32,
    /// The time a unit last garrisoned in the building.
    pub last_garrison_time: u32,
    /// The number of relics currently stored inside this building.
    pub relic_count: u32,
    /// The number of "specific relics" currently stored inside this building(?).
    ///
    /// This specific relic count generates 2× as much gold as normal relics, but appears to be
    /// otherwise unused.
    pub specific_relic_count: u32,
    /// Gather point for units trained from this building.
    pub gather_point: Option<GatherPoint>,
    /// Whether the building has been made passable.
    pub desolid_flag: bool,
    /// Not known yet; stored from save version 10.54 on.
    pub pending_order: u32,
    /// The "owner" building, if this building object is part of a larger building like a Town
    /// Center.
    pub linked_owner: Option<ObjectID>,
    /// The IDs of the children of this building object, also known as "annex buildings".
    pub linked_children: ArrayVec<ObjectID, 4>,
    /// Not known yet; the name suggests a count of captured units.
    pub captured_unit_count: u8,
    /// Further actions; not known yet what for.
    pub extra_actions: Vec<UnitAction>,
    /// The research actions.
    pub research_actions: Vec<UnitAction>,
    /// The current active production queue.
    pub production_queue: Vec<ProductionQueueEntry>,
    /// Cumulative count of queued units.
    pub production_queue_total_units: u16,
    /// Whether the production queue is enabled.
    pub production_queue_enabled: bool,
    /// The actions currently in the production queue.
    pub production_queue_actions: Vec<UnitAction>,
    /// A gate end point (x, y, z). The file has two; the game reads both into one field, so
    /// the second wins.
    pub endpoint: (f32, f32, f32),
    /// Whether the gate is locked.
    pub gate_locked: u32,
    /// Not known yet; stored from save version 10.65 on.
    pub first_update: u32,
    /// The gate's close timer.
    pub close_timer: u32,
    /// A terrain, probably the one under the building; stored from save version 10.67 on.
    pub terrain_type: Option<TerrainID>,
    /// Whether the building is semi-asleep (updated less often); stored from save version
    /// 11.43 on.
    pub semi_asleep: bool,
    /// Should this building be rendered with the snow graphic?
    pub snow_flag: bool,
}

impl ReadableHeaderElement for BuildingUnitAttributes {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        let mut attrs = Self {
            built: input.read_u8()? != 0,
            build_points: input.read_f32::<LE>()?,
            unique_build_id: read_opt_u32(input)?,
            culture: input.read_u8()?,
            burning: input.read_u8()?,
            last_burn_time: input.read_u32::<LE>()?,
            last_garrison_time: input.read_u32::<LE>()?,
            relic_count: input.read_u32::<LE>()?,
            specific_relic_count: input.read_u32::<LE>()?,
            gather_point: {
                let exists = input.read_u32::<LE>()? != 0;
                let location = GatherPoint::Location {
                    x: input.read_f32::<LE>()?,
                    y: input.read_f32::<LE>()?,
                    z: input.read_f32::<LE>()?,
                };
                let object_id = input.read_i32::<LE>()?;
                let unit_type_id = input.read_i16::<LE>()?;
                match (exists, object_id, unit_type_id) {
                    (false, _, _) => None,
                    (true, -1, -1) => Some(location),
                    (true, id, unit_type_id) => Some(GatherPoint::Object {
                        id: id.try_into().unwrap(),
                        unit_type: unit_type_id.try_into().unwrap(),
                    }),
                }
            },
            desolid_flag: input.read_u8()? != 0,
            ..Default::default()
        };
        if input.version() >= 10.54 {
            attrs.pending_order = input.read_u32::<LE>()?;
        }
        attrs.linked_owner = read_opt_u32(input)?;
        attrs.linked_children = {
            let mut children: ArrayVec<ObjectID, 4> = ArrayVec::new();
            for _ in 0..4 {
                let id = input.read_i32::<LE>()?;
                if id != -1 {
                    children.push(id.try_into().unwrap());
                }
            }
            children
        };
        attrs.captured_unit_count = input.read_u8()?;
        attrs.extra_actions = UnitAction::read_list_from(input)?;
        attrs.research_actions = UnitAction::read_list_from(input)?;
        attrs.production_queue = {
            let capacity = input.read_u16::<LE>()?;
            let mut queue = vec![ProductionQueueEntry::default(); capacity as usize];
            for entry in &mut queue {
                *entry = ProductionQueueEntry::read_from(input)?;
            }
            let _size = input.read_u16::<LE>()?;
            queue
        };
        attrs.production_queue_total_units = input.read_u16::<LE>()?;
        attrs.production_queue_enabled = input.read_u8()? != 0;
        attrs.production_queue_actions = UnitAction::read_list_from(input)?;
        if input.version() >= 10.65 {
            // game reads into the same value twice, while there are two separate fields of this
            // type. likely a bug, but it doesn't appear to cause issues? is this unused?
            attrs.endpoint = (
                input.read_f32::<LE>()?,
                input.read_f32::<LE>()?,
                input.read_f32::<LE>()?,
            );
            attrs.endpoint = (
                input.read_f32::<LE>()?,
                input.read_f32::<LE>()?,
                input.read_f32::<LE>()?,
            );
            attrs.gate_locked = input.read_u32::<LE>()?;
            attrs.first_update = input.read_u32::<LE>()?;
            attrs.close_timer = input.read_u32::<LE>()?;
        }
        if input.version() >= 10.67 {
            attrs.terrain_type = Some(input.read_u8()?.into());
        }
        if input.version() >= 11.43 {
            attrs.semi_asleep = input.read_u8()? != 0;
        }
        if input.version() >= 11.54 {
            attrs.snow_flag = input.read_u8()? != 0;
        }
        Ok(attrs)
    }
}
