//! The live objects of a player (pattern: `Object` and the chain `StaticObject` … `Building`).
//!
//! Each kind extends the previous one: static, animated, moving, action, combat, building. From
//! the moving part to the combat object's attribute amounts the reader keeps the pattern's fields,
//! whose sizes add up but whose names are about 30 bytes off the binary's (the combat object's
//! kill-reward list lies in that range and is not read). The masters, the unit AI and the tails
//! after it follow the 158041 save functions.

use super::master::MasterObject;
use super::primitives::{DeString, peek_u8};
use binrw::{BinRead, binread};

/// The feature named by an `Unsupported` marker anywhere in a read error, with its position.
pub fn unsupported_in(e: &binrw::Error) -> Option<(&'static str, usize)> {
    let root = e.root_cause();
    let what = root.custom_err::<UnsupportedFeature>()?.0;
    let at = match root {
        binrw::Error::Custom { pos, .. } => *pos as usize,
        _ => 0,
    };
    Some((what, at))
}

/// One object of a player's live list.
#[derive(Debug, Clone, PartialEq)]
pub struct Object {
    pub kind: u8,
    pub player: u8,
    pub body: ObjectBody,
}

/// The kind-specific body; every kind contains the previous one.
#[derive(Debug, Clone, PartialEq)]
pub enum ObjectBody {
    /// Kinds 10 and 90.
    Static(StaticObject),
    /// Kinds 20 and 25.
    Animated(AnimatedObject),
    /// Kind 30. The larger kinds are boxed: Gaia's lists are mostly small static objects.
    Moving(Box<MovingObject>),
    /// Kind 40.
    Action(Box<ActionObject>),
    /// Kind 70.
    Combat(Box<CombatObject>),
    /// Kind 80.
    Building(Box<BuildingObject>),
}

impl Object {
    /// The fields every kind shares.
    pub fn static_part(&self) -> &StaticObject {
        match &self.body {
            ObjectBody::Static(s) => s,
            ObjectBody::Animated(a) => &a.base,
            ObjectBody::Moving(m) => &m.base.animated.base,
            ObjectBody::Action(a) => &a.moving.animated.base,
            ObjectBody::Combat(c) => &c.action.moving.animated.base,
            ObjectBody::Building(b) => &b.combat.action.moving.animated.base,
        }
    }

    /// The action list, for kinds that have one.
    pub fn actions(&self) -> Option<&[UnitAction]> {
        match &self.body {
            ObjectBody::Action(a) => Some(&a.actions),
            ObjectBody::Combat(c) => Some(&c.action.actions),
            ObjectBody::Building(b) => Some(&b.combat.action.actions),
            _ => None,
        }
    }

    /// The unit AI, for combat objects and buildings that have one.
    pub fn unit_ai(&self) -> Option<&UnitAi> {
        match &self.body {
            ObjectBody::Combat(c) => c.unit_ai.as_ref(),
            ObjectBody::Building(b) => b.combat.unit_ai.as_ref(),
            _ => None,
        }
    }
}

/// Marks a feature this port does not read yet, so callers can tell it from a parse error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnsupportedFeature(pub &'static str);

impl std::fmt::Display for UnsupportedFeature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unsupported: {}", self.0)
    }
}

pub(super) fn unsupported<R: binrw::io::Seek>(reader: &mut R, what: &'static str) -> binrw::Error {
    binrw::Error::Custom {
        pos: reader.stream_position().unwrap_or_default(),
        err: Box::new(UnsupportedFeature(what)),
    }
}

/// Objects until the next byte is 0 (pattern: `Object objects[while(... != 0x00)]`).
pub fn read_objects<R: binrw::io::Read + binrw::io::Seek>(
    r: &mut R,
    v: u32,
) -> binrw::BinResult<Vec<Object>> {
    let mut out = Vec::new();
    while peek_u8(r)? != 0 {
        if std::env::var_os("ZZ_TRACE").is_some() {
            eprintln!("OBJ {:#x}", r.stream_position()?);
        }
        out.push(read_object(r, v)?);
    }
    Ok(out)
}

/// One object: kind, player, then the body by kind (dispatched by hand).
pub fn read_object<R: binrw::io::Read + binrw::io::Seek>(
    r: &mut R,
    v: u32,
) -> binrw::BinResult<Object> {
    let e = binrw::Endian::Little;
    let kind = u8::read_options(r, e, ())?;
    let player = u8::read_options(r, e, ())?;
    let body = match kind {
        10 | 90 => ObjectBody::Static(StaticObject::read_options(r, e, ())?),
        20 | 25 => ObjectBody::Animated(AnimatedObject::read_options(r, e, ())?),
        30 => ObjectBody::Moving(Box::new(MovingObject::read_options(r, e, ())?)),
        40 => ObjectBody::Action(Box::new(ActionObject::read_options(r, e, ())?)),
        70 => ObjectBody::Combat(Box::new(CombatObject::read_options(r, e, (v,))?)),
        80 => ObjectBody::Building(Box::new(BuildingObject::read_options(r, e, (v,))?)),
        other => {
            return Err(binrw::Error::AssertFail {
                pos: r.stream_position()?,
                message: format!("unknown object kind {other}"),
            });
        }
    };
    Ok(Object { kind, player, body })
}

#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little)]
pub struct StaticObject {
    /// The main object (unit type) id.
    pub main_id: u16,
    pub sprite: u16,
    pub garrisoned_in_id: i32,
    pub hit_points: f32,
    pub state: u8,
    pub sleep: u8,
    pub doppelganger: u8,
    pub go_to_sleep: u8,
    pub id: u32,
    pub facet: u8,
    pub position: [f32; 3],
    pub screen_offset: [i16; 2],
    pub shadow_offset: [i16; 2],
    pub resource: i16,
    /// The amount held (checkpoint word 4).
    pub amount: f32,
    pub worker_count: u8,
    pub current_damage: u8,
    pub damaged_lately_timer: u8,
    pub under_attack: u8,
    pub unknown_de_66_1: [u8; 2],
    pub group_id: i32,
    pub roo_already_called: u8,
    pub unknown1: [u8; 17],
    pub unknown2: u8,
    /// `hasOwnMasterObject` and `hasSuccessorMasterObj` (158041 static save `0x1411985d0`, at
    /// `0x14119bbd2`); each set flag is followed by that master's full save. The pattern reads the
    /// pair as one u16 `has_object_props`.
    pub has_own_master: u8,
    pub has_successor_master: u8,
    #[br(if(has_own_master != 0))]
    pub own_master: Option<Box<MasterObject>>,
    #[br(if(has_successor_master != 0))]
    pub successor_master: Option<Box<MasterObject>>,
    #[br(parse_with = sprite_list)]
    pub sprites: Vec<Sprite>,
    pub particles: [Particle; 5],
    pub unknown3: [u8; 5],
    pub unknown4: [u8; 2],
    pub unknown5: [u8; 2],
    pub unknown6: DeString,
    pub unknown7: DeString,
    pub unknown8: [u8; 2],
    pub unknown_de_64: u8,
    pub unknown9: u8,
    pub unknown10: [u8; 18],
    pub unknown_de_66_2: [u8; 4],
}

/// `has_sprite_list`, then the sprites up to a 0 byte.
#[binrw::parser(reader, endian)]
fn sprite_list() -> binrw::BinResult<Vec<Sprite>> {
    let has_sprite_list = u8::read_options(reader, endian, ())?;
    let mut out = Vec::new();
    if has_sprite_list > 0 {
        while peek_u8(reader)? != 0 {
            out.push(Sprite::read_options(reader, endian, ())?);
        }
        u8::read_options(reader, endian, ())?; // the 0 that ends the list
    }
    Ok(out)
}

#[binread]
#[derive(Debug, Clone, Copy, PartialEq)]
#[br(little)]
pub struct Sprite {
    pub sprite_type: u8,
    pub id: u16,
    pub x: u32,
    pub y: u32,
    pub frame: u16,
    pub invisible: u8,
    #[br(if(sprite_type == 2))]
    pub animation: Option<SpriteAnimation>,
    pub order: u8,
    pub flag: u8,
    pub count: u8,
}

#[binread]
#[derive(Debug, Clone, Copy, PartialEq)]
#[br(little)]
pub struct SpriteAnimation {
    pub animate_interval: f32,
    pub animate_last: u32,
    pub last_frame: u16,
    pub frame_changed: u8,
    pub frame_looped: u8,
    pub animate_flag: u8,
    pub last_speed: f32,
}

#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little)]
pub struct Particle {
    pub particle_type: u16,
    #[br(if(particle_type == 1))]
    pub data: Option<ParticleData>,
}

#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little)]
pub struct ParticleData {
    pub name: DeString,
    pub x: f32,
    pub y: f32,
    pub unknown1: [u8; 26],
}

#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little)]
pub struct AnimatedObject {
    pub base: StaticObject,
    pub turn_speed: f32,
}

#[binread]
#[derive(Debug, Clone, Copy, PartialEq)]
#[br(little)]
pub struct PathData {
    pub id: u32,
    pub linked_path_type: u32,
    pub waypoint_level: u32,
    pub path_id: u32,
    pub waypoint: u32,
    pub state: i32,
    pub range: f32,
    pub target_id: i32,
    pub pause_time: f32,
    pub continue_counter: u32,
    pub flags: u32,
}

/// The moving part shared by moving, action, combat and building objects (pattern:
/// `BaseMovingObject`).
#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little)]
pub struct BaseMoving {
    pub animated: AnimatedObject,
    pub trail_remainder: u32,
    pub velocity: [f32; 3],
    pub angle: f32,
    pub move_byte: u8,
    pub turn_towards_time: u32,
    pub waiting_to_move: u8,
    pub wait_delays_count: u8,
    pub on_ground: u8,
    #[br(temp)]
    num_path_data: u32,
    #[br(count = num_path_data)]
    pub path_data: Vec<PathData>,
    #[br(temp)]
    has_future_path_data: u32,
    #[br(if(has_future_path_data > 0))]
    pub future_path_data: Option<PathData>,
    #[br(temp)]
    has_movement_data: u32,
    /// Velocity and acceleration.
    #[br(if(has_movement_data > 0))]
    pub movement_data: Option<[[f32; 3]; 2]>,
    pub position: [f32; 3],
    pub orientation_forward: [f32; 3],
    pub orientation_right: [f32; 3],
    pub last_time_move: u32,
    #[br(temp)]
    num_user_waypoints: u32,
    #[br(count = num_user_waypoints)]
    pub user_waypoints: Vec<[f32; 3]>,
    pub has_substitute_position: u32,
    pub substitute_position: [f32; 3],
    pub consecutive_substitute_count: u32,
}

#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little)]
pub struct MovingObject {
    pub base: BaseMoving,
    pub unknown_moving_1: [u8; 21],
}

/// One entry of an action list; it holds its own sub-actions.
#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little)]
pub struct UnitAction {
    pub action_type: u16,
    pub target_object_pointer: i32,
    pub target_object_pointer2: i32,
    pub target_object_id: i32,
    pub target_object_id2: i32,
    pub position: [f32; 3],
    pub timer: f32,
    pub target_moved_state: u8,
    pub task_id: u16,
    pub sub_action_value: u8,
    #[br(parse_with = action_list)]
    pub sub_actions: Vec<UnitAction>,
    pub sprite_id: i16,
    #[br(parse_with = action_extra, args(action_type))]
    pub extra: ActionExtra,
}

/// The type-specific tail of an action. The pattern knows four types; the others carry no tail.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ActionExtra {
    None,
    /// Type 1.
    Move {
        range: f32,
    },
    /// Type 3.
    Enter {
        first_time: u32,
    },
    /// Type 9.
    Attack(AttackAction),
    /// Type 21.
    Make {
        work_timer: f32,
    },
}

#[binread]
#[derive(Debug, Clone, Copy, PartialEq)]
#[br(little)]
pub struct AttackAction {
    pub range: f32,
    pub min_range: f32,
    pub missile_id: u16,
    pub frame_delay: u16,
    pub need_to_attack: u16,
    pub was_same_owner: u16,
    pub indirect_fire_flag: u8,
    pub move_sprite_id: u16,
    pub fight_sprite_id: u16,
    pub wait_sprite_id: u16,
    pub last_target_position: [f32; 3],
}

#[binrw::parser(reader, endian)]
fn action_extra(action_type: u16) -> binrw::BinResult<ActionExtra> {
    Ok(match action_type {
        1 => ActionExtra::Move {
            range: f32::read_options(reader, endian, ())?,
        },
        3 => ActionExtra::Enter {
            first_time: u32::read_options(reader, endian, ())?,
        },
        9 => ActionExtra::Attack(AttackAction::read_options(reader, endian, ())?),
        21 => ActionExtra::Make {
            work_timer: f32::read_options(reader, endian, ())?,
        },
        _ => ActionExtra::None,
    })
}

/// Actions until the next u16 is 0, then that `00 00`.
#[binrw::parser(reader, endian)]
fn action_list() -> binrw::BinResult<Vec<UnitAction>> {
    let mut out = Vec::new();
    loop {
        let next = u16::read_options(reader, endian, ())?;
        if next == 0 {
            return Ok(out);
        }
        reader.seek(binrw::io::SeekFrom::Current(-2))?;
        out.push(UnitAction::read_options(reader, endian, ())?);
    }
}

#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little)]
pub struct ActionObject {
    pub moving: BaseMoving,
    pub waiting: u8,
    pub command_flag: u8,
    pub selected_group_info: u16,
    #[br(parse_with = action_list)]
    pub actions: Vec<UnitAction>,
}

#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little, import(v: u32))]
pub struct CombatObject {
    pub action: ActionObject,
    pub formation_id: u8,
    pub formation_row: u8,
    pub formation_col: u8,
    pub attack_timer: f32,
    pub capture_flag: i8,
    pub multi_unified_points: i8,
    pub large_object_radius: i8,
    pub attack_count: u32,
    pub unknown_de1: [u8; 14],
    pub unknown_de_66_1: [u8; 4],
    pub unknown_de2: [u8; 16],
    pub unknown_de3: [u8; 4],
    pub unknown_de_64_2: [u8; 19],
    /// The last byte of the combat object save (`RGE_Combat_Object::save`, `0x1410870e0`). The
    /// pattern calls these three bytes `next_volley`, `using_special_animation` and `own_base`;
    /// the binary writes `attr_amt[6]` right after `using_special_attack_animation`
    /// (`TRIBE_Combat_Object::save`, `0x141218920`).
    pub processed_kill: u8,
    pub next_volley: u8,
    pub using_special_attack_animation: u8,
    pub attribute_amounts: [u16; 6],
    pub decay_timer: u16,
    pub raider_build_countdown: u32,
    pub locked_down_count: u32,
    pub inside_garrison_count: u8,
    pub has_ai: u32,
    #[br(if(has_ai > 0), args(v, has_ai))]
    pub unit_ai: Option<UnitAi>,
    #[br(if(v >= 6720))]
    pub unknown_de_67_2_1: Option<i8>,
    #[br(parse_with = optional_position_67_2, args(v))]
    pub unknown_vector_67_2: Option<([f32; 3], u8)>,
    pub tail: CombatTail,
}

/// The rest of `TRIBE_Combat_Object::save` (`0x141218920`) after the unit AI, in write order.
#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little)]
pub struct CombatTail {
    pub town_bell_flag: u8,
    /// `mBackToWork1` and `mBackToWork2`.
    pub back_to_work: [BackToWork; 2],
    pub berserker_timer: f32,
    pub num_builders: u8,
    pub num_healers: u8,
    pub charge: f32,
    pub melee_armor_removed: i16,
    pub pierce_armor_removed: i16,
    pub movement_damage_timer: f32,
    pub attack_powerup: f32,
    pub reload_powerup: f32,
    pub powerup_update_offset: u32,
    pub speed_powerup: f32,
    pub work_rate_powerup: f32,
    pub regeneration_rate_powerup: f32,
    pub speed_stinger: f32,
    pub regeneration_stinger: f32,
    pub reload_stinger: f32,
    #[br(temp)]
    active_stinger_count: i32,
    /// `ActiveStingerData`, 32 bytes each; the fields are not mapped.
    #[br(count = active_stinger_count.max(0) as usize)]
    pub active_stingers: Vec<[u8; 32]>,
    pub hitpoints_powerup: f32,
    pub hitpoints_max_powerup: f32,
    pub max_range_powerup: f32,
    pub melee_armor_powerup: i32,
    pub pierce_armor_powerup: i32,
    #[br(temp)]
    local_attribute_count: u32,
    /// Attribute key and value. Towers usually hold one entry, which the pattern read as six
    /// extra bytes for unit-AI type 15.
    #[br(count = local_attribute_count)]
    pub local_attributes: Vec<(u16, f32)>,
    pub conversion_rate: f32,
    pub skip_volley_attack: u8,
}

/// Where a unit returns to work after the town bell.
#[binread]
#[derive(Debug, Clone, Copy, PartialEq)]
#[br(little)]
pub struct BackToWork {
    pub target_id: i32,
    pub target: [f32; 2],
    pub target_id2: i32,
    pub target_type: i32,
    pub action: i32,
}

fn position_follows<R: binrw::io::Read + binrw::io::Seek>(
    reader: &mut R,
) -> binrw::BinResult<bool> {
    let mut next = [0u8; 5];
    reader.read_exact(&mut next)?;
    reader.seek(binrw::io::SeekFrom::Current(-5))?;
    Ok(next != [0x00, 0xff, 0xff, 0xff, 0xff])
}

#[binrw::parser(reader, endian)]
fn optional_position_67_2(v: u32) -> binrw::BinResult<Option<([f32; 3], u8)>> {
    if v >= 6720 && position_follows(reader)? {
        Ok(Some(<([f32; 3], u8)>::read_options(reader, endian, ())?))
    } else {
        Ok(None)
    }
}

/// An `OrderEvent`, 52 bytes (`UnitAIModule::save` writes `0x34` per order); the fields are not
/// mapped. The pattern read 36 bytes per order.
pub type OrderEvent = [u8; 52];

/// A unit AI's patrol path (`0x140f98f00`).
#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little)]
pub struct PatrolPath {
    pub value_0: u32,
    #[br(temp)]
    waypoint_count: u32,
    #[br(count = waypoint_count)]
    pub waypoints: Vec<[f32; 3]>,
    /// The words at `+4` … `+0x10`.
    pub values: [u32; 4],
    #[br(temp)]
    id_count: i32,
    #[br(count = id_count.max(0) as usize)]
    pub ids: Vec<i32>,
}

#[binread]
#[derive(Debug, Clone, Copy, PartialEq)]
#[br(little)]
pub struct Notification {
    pub caller: u32,
    pub recipient: u32,
    pub notification_type: u32,
    pub params: [u32; 3],
}

#[binread]
#[derive(Debug, Clone, Copy, PartialEq)]
#[br(little)]
pub struct OrderHistory {
    pub order: u32,
    pub action: u32,
    pub time: u32,
    pub position: [f32; 3],
    pub target_id: u32,
    pub target_attack_category: u32,
    pub target_position: [f32; 3],
}

/// The unit AI of a combat object.
#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little, import(v: u32, ai_type: u32))]
pub struct UnitAi {
    pub mood: i32,
    pub current_order: i32,
    pub current_order_priority: i32,
    pub current_action: i32,
    pub current_target: i32,
    pub current_target_type: i16,
    pub unknown1: i16,
    pub current_target_location: [f32; 3],
    pub desired_target_distance: f32,
    pub last_action: i32,
    pub last_order: i32,
    pub last_target: i32,
    pub last_target_type: i32,
    pub last_update_type: i32,
    pub idle_timer: u32,
    pub idle_timeout: u32,
    pub adjusted_idle_timeout: u32,
    pub secondary_timer: u32,
    pub look_around_timer: u32,
    pub look_around_timeout: u32,
    pub defend_target: i32,
    pub defense_buffer: f32,
    /// Position, facet to the next waypoint, three unknown bytes.
    pub last_world_position: ([f32; 3], u8, [u8; 3]),
    #[br(temp)]
    num_orders: u32,
    #[br(count = num_orders)]
    pub orders: Vec<OrderEvent>,
    #[br(temp)]
    num_notifications: u32,
    #[br(count = num_notifications)]
    pub notifications: Vec<Notification>,
    #[br(temp)]
    num_attacking_units: u32,
    #[br(count = num_attacking_units)]
    pub attacking_units: Vec<u32>,
    pub stop_after_target_killed: u8,
    pub state: u8,
    pub state_position: [f32; 2],
    pub time_since_enemy_sighting: u32,
    pub alert_mode: u8,
    pub alert_mode_object_id: i32,
    pub has_patrol_path: u32,
    #[br(if(has_patrol_path != 0))]
    pub patrol_path: Option<PatrolPath>,
    pub patrol_current_waypoint: u32,
    #[br(temp)]
    num_order_history: u32,
    #[br(count = num_order_history)]
    pub order_history: Vec<OrderHistory>,
    pub last_retarget_time: u32,
    /// The retarget timer of checkpoint word 9 (INFERRED, FD-008-D07).
    pub randomized_target_timer: u32,
    #[br(temp)]
    num_retarget_entries: u32,
    /// Target id and retarget timeout.
    #[br(count = num_retarget_entries)]
    pub retarget_entries: Vec<[u32; 2]>,
    pub best_unit_to_attack: i32,
    pub formation_type: u8,
    pub time_since_last_search: u32,
    pub disable_retargeting: u8,
    /// Tower and building unit AIs (types 15 and 17; `0x140c2b580`, `0x140c2aa00`): their
    /// order queue, a count and the orders.
    #[br(if(ai_type == 15 || ai_type == 17))]
    pub building_order_queue: Option<OrderQueue>,
    #[br(if(v >= 6720))]
    pub unknown_de4: Option<i8>,
    /// Elephant unit AI (type 4, `0x140c2ae70`): the words at `+0x1f0`, `+0x1f4`, `+0x1f8` and
    /// the byte at `+0x1fc`. The words read as a map position. The pattern found these bytes
    /// with a look-ahead.
    #[br(if(ai_type == 4))]
    pub elephant_position: Option<([f32; 3], u8)>,
}

/// A count and that many orders.
#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little)]
pub struct OrderQueue {
    #[br(temp)]
    count: i32,
    #[br(count = count.max(0) as usize)]
    pub orders: Vec<OrderEvent>,
}

#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little, import(v: u32))]
pub struct BuildingObject {
    #[br(args(v))]
    pub combat: CombatObject,
    pub built: u8,
    pub build_points: f32,
    pub unique_building_id: i32,
    pub culture: u8,
    pub burning: u8,
    pub last_burn_time: u32,
    pub last_garrison_time: u32,
    pub relic_count: u32,
    pub specific_relic_count: u32,
    /// Exists, position, object id, unit type id.
    pub gatherpoint: (u32, [f32; 3], i32, i16),
    pub desolid_flag: u8,
    pub pending_order: u32,
    pub linked_owner: i32,
    pub linked_children: [i32; 3],
    pub captured_unit_count: u8,
    #[br(parse_with = action_list)]
    pub extra_actions: Vec<UnitAction>,
    #[br(temp)]
    capacity: u16,
    #[br(count = capacity)]
    pub production_queue: Vec<ProductionQueueEntry>,
    pub size: u16,
    pub production_queue_total_units: u16,
    pub production_queue_enabled: u8,
    #[br(parse_with = action_list)]
    pub production_queue_actions: Vec<UnitAction>,
    pub endpoint: [f32; 3],
    pub endpoint_2: [f32; 3],
    pub gate_locked: u32,
    pub first_update: u32,
    pub close_timer: u32,
    pub tail: BuildingTail,
}

/// One entry of a building's production queue, 36 bytes (`TRIBE_Building_Object::save`,
/// `0x1411d72c0`, writes `+0`, `+2`, `+4`, `+0x2c`, `+0x28`, `+8`).
#[binread]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[br(little)]
pub struct ProductionQueueEntry {
    pub master_unit_id: i16,
    pub unit_count: i16,
    /// `mTechId` and `mTrainTime`; which is which is not mapped.
    pub value_04: i16,
    pub value_2c: i16,
    pub button_location_id: i32,
    /// `mCosts`.
    pub costs: [u8; 24],
}

/// The rest of `TRIBE_Building_Object::save` (`0x1411d72c0`) after the close timer, in write order.
/// The pattern read it as 36 fixed bytes, which holds while every list is empty and the building
/// has no open/closed master.
#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little)]
pub struct BuildingTail {
    pub terrain_type: u8,
    pub semi_asleep: u8,
    pub snow_flag: u8,
    pub original_owner: i8,
    /// The id of the master at `+0xc90`, or -1.
    pub master_id_c90: i16,
    pub livestock_summation: f32,
    #[br(temp)]
    pasture_livestock_count: i32,
    #[br(count = pasture_livestock_count.max(0) as usize)]
    pub pasture_livestock: Vec<i32>,
    #[br(temp)]
    pasture_worker_count: i32,
    #[br(count = pasture_worker_count.max(0) as usize)]
    pub pasture_workers: Vec<i32>,
    pub has_open_closed_master: u8,
    /// A gate's open or closed master, saved in full.
    #[br(if(has_open_closed_master != 0))]
    pub open_closed_master: Option<Box<MasterObject>>,
    #[br(temp)]
    attacking_unit_count: i32,
    #[br(count = attacking_unit_count.max(0) as usize)]
    pub attacking_units: Vec<i32>,
    #[br(temp)]
    disabled_tech_count: u32,
    #[br(count = disabled_tech_count)]
    pub disabled_techs: Vec<i16>,
    #[br(temp)]
    available_unit_count: u32,
    #[br(count = available_unit_count)]
    pub available_units: Vec<i16>,
    #[br(temp)]
    researched_tech_count: u32,
    #[br(count = researched_tech_count)]
    pub researched_techs: Vec<i16>,
    pub is_on_mangrove_shallows: u8,
}

#[cfg(test)]
mod tests {
    use super::super::master::tests::static_part;
    use super::super::test_bytes::Bytes;
    use super::*;
    use binrw::io::Cursor;

    /// A unit AI up to its type-specific part: `orders` 52-byte orders, an optional patrol path
    /// with two waypoints and one id, `retarget_timer` as the randomized retarget timer.
    fn unit_ai_base(orders: usize, patrol: bool, retarget_timer: u32) -> Bytes {
        let mut b = Bytes::default()
            .zeros(4 * 5 + 2 + 2 + 12 + 4) // mood … desired target distance
            .zeros(4 * 5 + 4 * 6 + 4 + 4 + 16) // last action … last world position
            .u32(orders as u32)
            .zeros(52 * orders)
            .u32(1) // one notification
            .zeros(24)
            .u32(2) // two attacking units
            .u32(11)
            .u32(12)
            .zeros(1 + 1 + 8 + 4 + 1 + 4) // stop after kill … alert mode object
            .u32(patrol as u32);
        if patrol {
            b = b
                .u32(3)
                .u32(2) // two waypoints
                .f32(1.0)
                .f32(2.0)
                .f32(0.0)
                .f32(3.0)
                .f32(4.0)
                .f32(0.0)
                .zeros(16)
                .u32(1) // one id
                .i32(77);
        }
        b.u32(0) // patrol current waypoint
            .u32(1) // one order history entry
            .zeros(44)
            .u32(100) // last retarget time
            .u32(retarget_timer)
            .u32(1) // one retarget entry
            .zeros(8)
            .i32(-1) // best unit to attack
            .u8(0) // formation type
            .u32(5) // time since last search
            .u8(1) // disable retargeting
    }

    fn read_unit_ai(bytes: &Bytes, ai_type: u32) -> (UnitAi, u64) {
        let mut r = Cursor::new(&bytes.0[..]);
        let ai = UnitAi::read_le_args(&mut r, (6660, ai_type)).unwrap();
        (ai, r.position())
    }

    #[test]
    fn a_unit_ai_reads_52_byte_orders_and_its_patrol_path() {
        let bytes = unit_ai_base(2, true, 989);
        let (ai, end) = read_unit_ai(&bytes, 13);
        assert_eq!(end, bytes.0.len() as u64);
        assert_eq!(ai.orders.len(), 2);
        let path = ai.patrol_path.as_ref().unwrap();
        assert_eq!(path.waypoints, vec![[1.0, 2.0, 0.0], [3.0, 4.0, 0.0]]);
        assert_eq!(path.ids, vec![77]);
        assert_eq!(ai.attacking_units, vec![11, 12]);
        assert_eq!(ai.randomized_target_timer, 989);
        assert_eq!((ai.time_since_last_search, ai.disable_retargeting), (5, 1));
    }

    #[test]
    fn a_tower_unit_ai_reads_its_order_queue() {
        let bytes = unit_ai_base(0, false, 0).u32(1).zeros(52);
        let (ai, end) = read_unit_ai(&bytes, 15);
        assert_eq!(end, bytes.0.len() as u64);
        assert_eq!(ai.building_order_queue.map(|q| q.orders.len()), Some(1));
    }

    #[test]
    fn an_elephant_unit_ai_reads_13_more_bytes() {
        let bytes = unit_ai_base(0, false, 0).f32(42.5).f32(40.5).f32(0.0).u8(0);
        let (ai, end) = read_unit_ai(&bytes, 4);
        assert_eq!(end, bytes.0.len() as u64);
        assert_eq!(ai.elephant_position, Some(([42.5, 40.5, 0.0], 0)));
    }

    #[test]
    fn a_combat_tail_reads_stingers_and_local_attributes() {
        let bytes = Bytes::default()
            .u8(0) // town bell flag
            .zeros(24 * 2) // back to work 1 and 2
            .f32(0.0) // berserker timer
            .zeros(1 + 1 + 4 + 2 + 2 + 4) // builders … movement damage timer
            .zeros(4 * 6 + 4 * 3) // powerups, stingers
            .i32(1) // one active stinger
            .zeros(32)
            .zeros(4 * 5) // hit point … pierce armour powerups
            .u32(1) // one local attribute: key, value
            .u16(9)
            .f32(2.5)
            .f32(1.0) // conversion rate
            .u8(1); // skip volley attack
        let mut r = Cursor::new(&bytes.0[..]);
        let tail = CombatTail::read_le(&mut r).unwrap();
        assert_eq!(r.position(), bytes.0.len() as u64);
        assert_eq!(tail.active_stingers.len(), 1);
        assert_eq!(tail.local_attributes, vec![(9, 2.5)]);
        assert_eq!((tail.conversion_rate, tail.skip_volley_attack), (1.0, 1));
    }

    #[test]
    fn a_building_tail_reads_its_lists_and_the_gate_master() {
        let bytes = Bytes::default()
            .zeros(1 + 1 + 1 + 1) // terrain type … original owner
            .i16(-1) // master id at +0xc90
            .f32(0.0) // livestock summation
            .i32(1) // one pastured animal
            .i32(31)
            .i32(0) // no pasture workers
            .u8(1) // an open/closed master
            .bytes(&static_part(10, 88, b"GATE").0)
            .i32(1) // one attacking unit
            .i32(41)
            .u32(2) // two disabled techs
            .i16(5)
            .i16(6)
            .u32(0) // no available units
            .u32(1) // one researched tech
            .i16(7)
            .u8(0); // on mangrove shallows
        let mut r = Cursor::new(&bytes.0[..]);
        let tail = BuildingTail::read_le(&mut r).unwrap();
        assert_eq!(r.position(), bytes.0.len() as u64);
        assert_eq!(tail.pasture_livestock, vec![31]);
        assert_eq!(tail.open_closed_master.as_ref().map(|m| m.id), Some(88));
        assert_eq!(tail.attacking_units, vec![41]);
        assert_eq!(tail.disabled_techs, vec![5, 6]);
        assert_eq!(tail.researched_techs, vec![7]);
    }
}
