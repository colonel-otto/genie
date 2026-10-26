//! A unit's AI state ([`UnitAI`]): its orders, notifications, history, retargets and paths.

use crate::element::ReadableHeaderElement;
use crate::reader::RecordingHeaderReader;
use crate::{ObjectID, PlayerID, Result};
use byteorder::{LE, ReadBytesExt};
use genie_support::{ReadSkipExt, read_opt_u32};
use std::convert::TryInto;
use std::io::Read;

/// An order in a unit AI's order queue.
#[allow(dead_code)]
#[derive(Debug, Default, Clone, Copy)]
pub struct UnitAIOrder {
    pub(super) issuer: u32,
    pub(super) order_type: u32,
    pub(super) priority: u32,
    pub(super) target_id: ObjectID,
    pub(super) target_player: PlayerID,
    pub(super) target_location: (f32, f32, f32),
    pub(super) range: f32,
}

impl ReadableHeaderElement for UnitAIOrder {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        Ok(Self {
            issuer: input.read_u32::<LE>()?,
            order_type: input.read_u32::<LE>()?,
            priority: input.read_u32::<LE>()?,
            target_id: input.read_u32::<LE>()?.into(),
            target_player: input.read_u32::<LE>()?.try_into().unwrap(),
            target_location: (
                input.read_f32::<LE>()?,
                input.read_f32::<LE>()?,
                input.read_f32::<LE>()?,
            ),
            range: input.read_f32::<LE>()?,
        })
    }
}

/// A notification one unit AI sends another.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnitAINotification {
    /// The object that sent the notification.
    pub caller: u32,
    /// The object the notification is for.
    pub recipient: u32,
    /// The kind of notification.
    pub notification_type: u32,
    /// The notification's three parameters.
    pub params: (u32, u32, u32),
}

impl ReadableHeaderElement for UnitAINotification {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        Ok(Self {
            caller: input.read_u32::<LE>()?,
            recipient: input.read_u32::<LE>()?,
            notification_type: input.read_u32::<LE>()?,
            params: (
                input.read_u32::<LE>()?,
                input.read_u32::<LE>()?,
                input.read_u32::<LE>()?,
            ),
        })
    }
}

/// An entry of a unit AI's order history.
#[allow(dead_code)]
#[derive(Debug, Default, Clone, Copy)]
pub struct UnitAIOrderHistory {
    pub(super) order: u32,
    pub(super) action: u32,
    pub(super) time: u32,
    pub(super) position: (f32, f32, f32),
    pub(super) target_id: ObjectID,
    pub(super) target_attack_category: Option<u32>,
    pub(super) target_position: (f32, f32, f32),
}

impl ReadableHeaderElement for UnitAIOrderHistory {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        Ok(Self {
            order: input.read_u32::<LE>()?,
            action: input.read_u32::<LE>()?,
            time: input.read_u32::<LE>()?,
            position: (
                input.read_f32::<LE>()?,
                input.read_f32::<LE>()?,
                input.read_f32::<LE>()?,
            ),
            target_id: input.read_u32::<LE>()?.into(),
            target_attack_category: if input.version() >= 10.50 {
                read_opt_u32(input)?
            } else {
                None
            },
            target_position: (
                input.read_f32::<LE>()?,
                input.read_f32::<LE>()?,
                input.read_f32::<LE>()?,
            ),
        })
    }
}

/// A target a unit AI keeps track of for retargeting, with a timeout.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnitAIRetargetEntry {
    /// The target object.
    pub target_id: ObjectID,
    /// When the entry times out.
    pub retarget_timeout: u32,
}

impl ReadableHeaderElement for UnitAIRetargetEntry {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        let target_id = input.read_u32::<LE>()?.into();
        let retarget_timeout = input.read_u32::<LE>()?;
        Ok(Self {
            target_id,
            retarget_timeout,
        })
    }
}

/// A waypoint of a path.
#[derive(Debug, Default, Clone, Copy)]
pub struct Waypoint {
    /// The position (x, y, z).
    pub location: (f32, f32, f32),
    /// The facet (direction) towards the next waypoint.
    pub facet_to_next_waypoint: u8,
}

impl ReadableHeaderElement for Waypoint {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        let waypoint = Self {
            location: (
                input.read_f32::<LE>()?,
                input.read_f32::<LE>()?,
                input.read_f32::<LE>()?,
            ),
            facet_to_next_waypoint: input.read_u8()?,
        };
        let _padding = input.read_u8()?;
        let _padding = input.read_u8()?;
        let _padding = input.read_u8()?;
        Ok(waypoint)
    }
}

/// A patrol path. Placeholder: reading one is not implemented yet and panics.
#[derive(Debug, Default, Clone, Copy)]
pub struct PatrolPath {}

impl ReadableHeaderElement for PatrolPath {
    fn read_from<R: Read>(_: &mut RecordingHeaderReader<R>) -> Result<Self> {
        unimplemented!()
    }
}

/// A unit's AI state: orders, targets, timers and notifications.
#[allow(dead_code)]
#[derive(Debug, Default, Clone)]
pub struct UnitAI {
    pub(super) mood: Option<u32>,
    pub(super) current_order: Option<u32>,
    pub(super) current_order_priority: Option<u32>,
    pub(super) current_action: Option<u32>,
    pub(super) current_target: Option<u32>,
    pub(super) current_target_type: Option<u32>,
    pub(super) current_target_location: (f32, f32, f32),
    pub(super) desired_target_distance: f32,
    pub(super) last_action: Option<u32>,
    pub(super) last_order: Option<u32>,
    pub(super) last_target: Option<u32>,
    pub(super) last_target_type: Option<u32>,
    pub(super) last_update_type: Option<u32>,
    pub(super) idle_timer: u32,
    pub(super) idle_timeout: u32,
    pub(super) adjusted_idle_timeout: u32,
    pub(super) secondary_timer: u32,
    pub(super) lookaround_timer: u32,
    pub(super) lookaround_timeout: u32,
    pub(super) defend_target: Option<ObjectID>,
    pub(super) defense_buffer: f32,
    pub(super) last_world_position: Waypoint,
    pub(super) orders: Vec<UnitAIOrder>,
    pub(super) notifications: Vec<UnitAINotification>,
    pub(super) attacking_units: Vec<ObjectID>,
    pub(super) stop_after_target_killed: bool,
    pub(super) state: u8,
    pub(super) state_position: (f32, f32),
    pub(super) time_since_enemy_sighting: u32,
    pub(super) alert_mode: u8,
    pub(super) alert_mode_object_id: Option<ObjectID>,
    pub(super) patrol_path: Option<PatrolPath>,
    pub(super) patrol_current_waypoint: u32,
    pub(super) order_history: Vec<UnitAIOrderHistory>,
    pub(super) last_retarget_time: u32,
    pub(super) randomized_retarget_timer: u32,
    pub(super) retarget_entries: Vec<UnitAIRetargetEntry>,
    pub(super) best_unit_to_attack: Option<u32>,
    pub(super) formation_type: u8,
}

impl ReadableHeaderElement for UnitAI {
    // Reads the unit AI's fields in file order.
    #[allow(clippy::too_many_lines)]
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        let mut ai = Self {
            mood: read_opt_u32(input)?,
            current_order: read_opt_u32(input)?,
            current_order_priority: read_opt_u32(input)?,
            current_action: read_opt_u32(input)?,
            current_target: read_opt_u32(input)?,
            current_target_type: match input.read_u16::<LE>()? {
                0xFFFF => None,
                id => Some(id.into()),
            },
            ..Default::default()
        };
        input.skip(2)?;
        ai.current_target_location = (
            input.read_f32::<LE>()?,
            input.read_f32::<LE>()?,
            input.read_f32::<LE>()?,
        );
        ai.desired_target_distance = input.read_f32::<LE>()?;
        ai.last_action = read_opt_u32(input)?;
        ai.last_order = read_opt_u32(input)?;
        ai.last_target = read_opt_u32(input)?;
        ai.last_target_type = read_opt_u32(input)?;
        ai.last_update_type = read_opt_u32(input)?;
        ai.idle_timer = input.read_u32::<LE>()?;
        ai.idle_timeout = input.read_u32::<LE>()?;
        ai.adjusted_idle_timeout = input.read_u32::<LE>()?;
        ai.secondary_timer = input.read_u32::<LE>()?;
        ai.lookaround_timer = input.read_u32::<LE>()?;
        ai.lookaround_timeout = input.read_u32::<LE>()?;
        ai.defend_target = read_opt_u32(input)?;
        ai.defense_buffer = input.read_f32::<LE>()?;
        ai.last_world_position = Waypoint::read_from(input)?;
        ai.orders = {
            let num_orders = input.read_u32::<LE>()?;
            let mut orders = vec![];
            for _ in 0..num_orders {
                orders.push(UnitAIOrder::read_from(input)?);
            }
            orders
        };
        ai.notifications = {
            let num_notifications = input.read_u32::<LE>()?;
            let mut notifications = vec![];
            for _ in 0..num_notifications {
                notifications.push(UnitAINotification::read_from(input)?);
            }
            notifications
        };
        ai.attacking_units = {
            let num_units = input.read_u32::<LE>()?;
            let mut units = vec![];
            for _ in 0..num_units {
                units.push(input.read_u32::<LE>()?.into());
            }
            units
        };
        ai.stop_after_target_killed = input.read_u8()? != 0;
        ai.state = input.read_u8()?;
        ai.state_position = (input.read_f32::<LE>()?, input.read_f32::<LE>()?);
        ai.time_since_enemy_sighting = input.read_u32::<LE>()?;
        ai.alert_mode = input.read_u8()?;
        ai.alert_mode_object_id = read_opt_u32(input)?;
        ai.patrol_path = {
            let has_path = input.read_u32::<LE>()? != 0;
            if has_path {
                Some(PatrolPath::read_from(input)?)
            } else {
                None
            }
        };
        ai.patrol_current_waypoint = input.read_u32::<LE>()?;
        if input.version() >= 10.48 {
            ai.order_history = {
                let num_orders = input.read_u32::<LE>()?;
                let mut orders = vec![];
                for _ in 0..num_orders {
                    orders.push(UnitAIOrderHistory::read_from(input)?);
                }
                orders
            };
        }
        if input.version() >= 10.50 {
            ai.last_retarget_time = input.read_u32::<LE>()?;
        }
        if input.version() >= 11.04 {
            ai.randomized_retarget_timer = input.read_u32::<LE>()?;
        }
        if input.version() >= 11.05 {
            ai.retarget_entries = {
                let num_entries = input.read_u32::<LE>()?;
                let mut entries = vec![];
                for _ in 0..num_entries {
                    entries.push(UnitAIRetargetEntry::read_from(input)?);
                }
                entries
            };
        }
        if input.version() >= 11.14 {
            ai.best_unit_to_attack = read_opt_u32(input)?;
        }
        if input.version() >= 11.44 {
            ai.formation_type = input.read_u8()?;
        }
        Ok(ai)
    }
}
