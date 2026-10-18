use crate::Result;
use crate::element::{OptionalReadableElement, ReadableHeaderElement, WritableHeaderElement};
use crate::reader::RecordingHeaderReader;
use crate::unit_action::UnitAction;
use crate::unit_type::UnitBaseClass;
use crate::{ObjectID, PlayerID};
use arrayvec::ArrayVec;
use byteorder::{LE, ReadBytesExt, WriteBytesExt};
pub use genie_dat::sprite::SpriteID;
pub use genie_dat::terrain::TerrainID;
pub use genie_dat::unit_type::AttributeCost;
use genie_dat::unit_type::UnitType;
use genie_support::{ReadSkipExt, read_opt_u32};
pub use genie_support::{StringKey, UnitTypeID};
use std::convert::TryInto;
use std::io::{Read, Write};

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

/// The animation state of a sprite node.
#[derive(Debug, Default, Clone, Copy)]
pub struct SpriteNodeAnimation {
    /// The time between two animation frames.
    pub animate_interval: u32,
    /// The time of the last frame change.
    pub animate_last: u32,
    /// The frame shown last.
    pub last_frame: u16,
    /// Whether the frame changed at the last update.
    pub frame_changed: u8,
    /// Whether the animation looped at the last update.
    pub frame_looped: u8,
    /// Whether the sprite animates.
    pub animate_flag: u8,
    /// The animation speed at the last update.
    pub last_speed: f32,
}

impl ReadableHeaderElement for SpriteNodeAnimation {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        Ok(Self {
            animate_interval: input.read_u32::<LE>()?,
            animate_last: input.read_u32::<LE>()?,
            last_frame: input.read_u16::<LE>()?,
            frame_changed: input.read_u8()?,
            frame_looped: input.read_u8()?,
            animate_flag: input.read_u8()?,
            last_speed: input.read_f32::<LE>()?,
        })
    }
}

impl WritableHeaderElement for SpriteNodeAnimation {
    fn write_to<W: Write>(&self, output: &mut W) -> Result<()> {
        output.write_u32::<LE>(self.animate_interval)?;
        output.write_u32::<LE>(self.animate_last)?;
        output.write_u16::<LE>(self.last_frame)?;
        output.write_u8(self.frame_changed)?;
        output.write_u8(self.frame_looped)?;
        output.write_u8(self.animate_flag)?;
        output.write_f32::<LE>(self.last_speed)?;
        Ok(())
    }
}

/// One sprite drawn for an object; an object can carry several (a building and its damage
/// fires, for example).
#[derive(Debug, Default, Clone, Copy)]
pub struct SpriteNode {
    /// The sprite.
    pub id: SpriteID,
    /// The sprite's horizontal offset.
    pub x: u32,
    /// The sprite's vertical offset.
    pub y: u32,
    /// The frame shown.
    pub frame: u16,
    /// Whether the sprite is hidden.
    pub invisible: bool,
    /// The animation state, for an animated node.
    pub animation: Option<SpriteNodeAnimation>,
    /// The drawing order among the object's sprites.
    pub order: u8,
    /// Not known yet.
    pub flag: u8,
    /// Not known yet.
    pub count: u8,
}

impl OptionalReadableElement for SpriteNode {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Option<Self>> {
        let ty = input.read_u8()?;
        if ty == 0 {
            return Ok(None);
        }

        Ok(Some(Self {
            id: input.read_u16::<LE>()?.into(),
            x: input.read_u32::<LE>()?,
            y: input.read_u32::<LE>()?,
            frame: input.read_u16::<LE>()?,
            invisible: input.read_u8()? != 0,
            animation: if ty == 2 {
                Some(SpriteNodeAnimation::read_from(input)?)
            } else {
                None
            },
            order: input.read_u8()?,
            flag: input.read_u8()?,
            count: input.read_u8()?,
        }))
    }
}

impl WritableHeaderElement for SpriteNode {
    fn write_to<W: Write>(&self, output: &mut W) -> Result<()> {
        let ty = if self.animation.is_some() { 2 } else { 1 };
        output.write_u8(ty)?;
        output.write_u16::<LE>(self.id.into())?;
        output.write_u32::<LE>(self.x)?;
        output.write_u32::<LE>(self.y)?;
        output.write_u16::<LE>(self.frame)?;
        output.write_u8(u8::from(self.invisible))?;
        if let Some(animation) = &self.animation {
            animation.write_to(output)?;
        }
        output.write_u8(self.order)?;
        output.write_u8(self.flag)?;
        output.write_u8(self.count)?;
        Ok(())
    }
}

/// The sprites drawn for an object.
#[derive(Debug, Default, Clone)]
pub struct SpriteList {
    /// The sprite nodes, in drawing order.
    pub sprites: Vec<SpriteNode>,
}

impl ReadableHeaderElement for SpriteList {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        let mut sprites = vec![];
        while let Some(node) = SpriteNode::read_from(input)? {
            sprites.push(node);
        }
        Ok(Self { sprites })
    }
}

impl WritableHeaderElement for SpriteList {
    fn write_to<W: Write>(&self, output: &mut W) -> Result<()> {
        for sprite in &self.sprites {
            sprite.write_to(output)?;
        }
        output.write_u8(0)?;
        Ok(())
    }
}

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

/// A path a moving object follows.
#[derive(Debug, Default, Clone, Copy)]
pub struct PathData {
    /// The path's id.
    pub id: u32,
    /// Not known yet; the type of a linked path.
    pub linked_path_type: u32,
    /// Not known yet.
    pub waypoint_level: u32,
    /// Not known yet.
    pub path_id: u32,
    /// The waypoint the object is at.
    pub waypoint: u32,
    /// Disable flags; stored before save version 10.25 only.
    pub disable_flags: Option<u32>,
    /// Enable flags; stored in save versions 10.20 to 10.24 only.
    pub enable_flags: Option<u32>,
    /// The path's state.
    pub state: u32,
    /// How close the object has to get to the target.
    pub range: f32,
    /// The target object.
    pub target_id: u32,
    /// How long the object pauses.
    pub pause_time: f32,
    /// Not known yet.
    pub continue_counter: u32,
    /// Path flags; not known yet in detail.
    pub flags: u32,
}

impl ReadableHeaderElement for PathData {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        let mut path = Self {
            id: input.read_u32::<LE>()?,
            linked_path_type: input.read_u32::<LE>()?,
            waypoint_level: input.read_u32::<LE>()?,
            path_id: input.read_u32::<LE>()?,
            waypoint: input.read_u32::<LE>()?,
            ..Default::default()
        };
        if input.version() < 10.25 {
            path.disable_flags = Some(input.read_u32::<LE>()?);
            if input.version() >= 10.20 {
                path.enable_flags = Some(input.read_u32::<LE>()?);
            }
        }
        path.state = input.read_u32::<LE>()?;
        path.range = input.read_f32::<LE>()?;
        path.target_id = input.read_u32::<LE>()?;
        path.pause_time = input.read_f32::<LE>()?;
        path.continue_counter = input.read_u32::<LE>()?;
        path.flags = input.read_u32::<LE>()?;
        Ok(path)
    }
}

/// A moving object's velocity and acceleration.
#[derive(Debug, Default, Clone, Copy)]
pub struct MovementData {
    /// The velocity (x, y, z).
    pub velocity: (f32, f32, f32),
    /// The acceleration (x, y, z).
    pub acceleration: (f32, f32, f32),
}

impl MovementData {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        let velocity = (
            input.read_f32::<LE>()?,
            input.read_f32::<LE>()?,
            input.read_f32::<LE>()?,
        );
        let acceleration = (
            input.read_f32::<LE>()?,
            input.read_f32::<LE>()?,
            input.read_f32::<LE>()?,
        );
        Ok(Self {
            velocity,
            acceleration,
        })
    }

    /// Write the velocity and acceleration.
    ///
    /// # Errors
    ///
    /// Returns an error if writing fails.
    pub fn write_to(&self, mut output: impl Write) -> Result<()> {
        output.write_f32::<LE>(self.velocity.0)?;
        output.write_f32::<LE>(self.velocity.1)?;
        output.write_f32::<LE>(self.velocity.2)?;
        output.write_f32::<LE>(self.acceleration.0)?;
        output.write_f32::<LE>(self.acceleration.1)?;
        output.write_f32::<LE>(self.acceleration.2)?;
        Ok(())
    }
}

/// The attributes the moving base class adds: movement and paths.
#[derive(Debug, Default, Clone)]
pub struct MovingUnitAttributes {
    /// Not known yet.
    pub trail_remainder: u32,
    /// The velocity (x, y, z).
    pub velocity: (f32, f32, f32),
    /// The angle the object faces.
    pub angle: f32,
    /// Not known yet; a timing of its turn towards a target.
    pub turn_towards_time: u32,
    /// The turning timer.
    pub turn_timer: u32,
    /// Not known yet.
    pub continue_counter: u32,
    /// Two terrain exceptions; not known yet.
    pub current_terrain_exception: (Option<u32>, Option<u32>),
    /// Whether the object waits to move.
    pub waiting_to_move: u8,
    /// How often the object has waited for its move.
    pub wait_delays_count: u8,
    /// Whether the object is on the ground.
    pub on_ground: u8,
    /// The paths the object follows.
    pub path_data: Vec<PathData>,
    /// The next path, if one is planned.
    pub future_path_data: Option<PathData>,
    /// The velocity and acceleration, if stored.
    pub movement_data: Option<MovementData>,
    /// A position (x, y, z); not known yet how it differs from the static attributes' one.
    pub position: (f32, f32, f32),
    /// The forward direction vector.
    pub orientation_forward: (f32, f32, f32),
    /// The right direction vector.
    pub orientation_right: (f32, f32, f32),
    /// The time the object last moved.
    pub last_move_time: u32,
    /// The waypoints the player set.
    pub user_defined_waypoints: Vec<(f32, f32, f32)>,
    /// A substitute position, if set; not known yet what for.
    pub substitute_position: Option<(f32, f32, f32)>,
    /// How many substitute positions in a row the object has used.
    pub consecutive_substitute_count: u32,
}

impl ReadableHeaderElement for MovingUnitAttributes {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        let mut attrs = Self {
            trail_remainder: input.read_u32::<LE>()?,
            velocity: (
                input.read_f32::<LE>()?,
                input.read_f32::<LE>()?,
                input.read_f32::<LE>()?,
            ),
            angle: input.read_f32::<LE>()?,
            turn_towards_time: input.read_u32::<LE>()?,
            turn_timer: input.read_u32::<LE>()?,
            continue_counter: input.read_u32::<LE>()?,
            current_terrain_exception: (read_opt_u32(input)?, read_opt_u32(input)?),
            waiting_to_move: input.read_u8()?,
            wait_delays_count: input.read_u8()?,
            on_ground: input.read_u8()?,
            path_data: {
                let num_paths = input.read_u32::<LE>()?;
                let mut paths = vec![];
                for _ in 0..num_paths {
                    paths.push(PathData::read_from(input)?);
                }
                paths
            },
            ..Default::default()
        };
        if input.read_u32::<LE>()? != 0 {
            attrs.future_path_data = Some(PathData::read_from(input)?);
        }
        if input.read_u32::<LE>()? != 0 {
            attrs.movement_data = Some(MovementData::read_from(input)?);
        }
        attrs.position = (
            input.read_f32::<LE>()?,
            input.read_f32::<LE>()?,
            input.read_f32::<LE>()?,
        );
        attrs.orientation_forward = (
            input.read_f32::<LE>()?,
            input.read_f32::<LE>()?,
            input.read_f32::<LE>()?,
        );
        attrs.orientation_right = (
            input.read_f32::<LE>()?,
            input.read_f32::<LE>()?,
            input.read_f32::<LE>()?,
        );
        attrs.last_move_time = input.read_u32::<LE>()?;
        attrs.user_defined_waypoints = {
            let num_waypoints = input.read_i32::<LE>()?.max(0);
            let mut waypoints = vec![];
            for _ in 0..num_waypoints {
                waypoints.push((
                    input.read_f32::<LE>()?,
                    input.read_f32::<LE>()?,
                    input.read_f32::<LE>()?,
                ));
            }
            waypoints
        };
        attrs.substitute_position = {
            let exists = input.read_u32::<LE>()? != 0;
            let x = input.read_f32::<LE>()?;
            let y = input.read_f32::<LE>()?;
            let z = input.read_f32::<LE>()?;
            if exists { Some((x, y, z)) } else { None }
        };
        attrs.consecutive_substitute_count = input.read_u32::<LE>()?;
        Ok(attrs)
    }
}

impl WritableHeaderElement for MovingUnitAttributes {}

/// The attributes the action base class adds: the action stack.
#[derive(Debug, Default, Clone)]
pub struct ActionUnitAttributes {
    /// Whether the unit is waiting.
    pub waiting: bool,
    /// Not known yet; stored from save version 6.5 on.
    pub command_flag: u8,
    /// Control group information; stored from save version 11.58 on.
    pub selected_group_info: u16,
    /// The unit's action stack.
    pub actions: Vec<UnitAction>,
}

impl ReadableHeaderElement for ActionUnitAttributes {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        let mut attrs = Self {
            waiting: input.read_u8()? != 0,
            ..Default::default()
        };

        if input.version() >= 6.5 {
            attrs.command_flag = input.read_u8()?;
        }
        if input.version() >= 11.58 {
            attrs.selected_group_info = input.read_u16::<LE>()?;
        }
        attrs.actions = UnitAction::read_list_from(input)?;
        Ok(attrs)
    }
}

impl WritableHeaderElement for ActionUnitAttributes {}

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

/// An order in a unit AI's order queue.
#[allow(dead_code)]
#[derive(Debug, Default, Clone, Copy)]
pub struct UnitAIOrder {
    issuer: u32,
    order_type: u32,
    priority: u32,
    target_id: ObjectID,
    target_player: PlayerID,
    target_location: (f32, f32, f32),
    range: f32,
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
    order: u32,
    action: u32,
    time: u32,
    position: (f32, f32, f32),
    target_id: ObjectID,
    target_attack_category: Option<u32>,
    target_position: (f32, f32, f32),
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
    mood: Option<u32>,
    current_order: Option<u32>,
    current_order_priority: Option<u32>,
    current_action: Option<u32>,
    current_target: Option<u32>,
    current_target_type: Option<u32>,
    current_target_location: (f32, f32, f32),
    desired_target_distance: f32,
    last_action: Option<u32>,
    last_order: Option<u32>,
    last_target: Option<u32>,
    last_target_type: Option<u32>,
    last_update_type: Option<u32>,
    idle_timer: u32,
    idle_timeout: u32,
    adjusted_idle_timeout: u32,
    secondary_timer: u32,
    lookaround_timer: u32,
    lookaround_timeout: u32,
    defend_target: Option<ObjectID>,
    defense_buffer: f32,
    last_world_position: Waypoint,
    orders: Vec<UnitAIOrder>,
    notifications: Vec<UnitAINotification>,
    attacking_units: Vec<ObjectID>,
    stop_after_target_killed: bool,
    state: u8,
    state_position: (f32, f32),
    time_since_enemy_sighting: u32,
    alert_mode: u8,
    alert_mode_object_id: Option<ObjectID>,
    patrol_path: Option<PatrolPath>,
    patrol_current_waypoint: u32,
    order_history: Vec<UnitAIOrderHistory>,
    last_retarget_time: u32,
    randomized_retarget_timer: u32,
    retarget_entries: Vec<UnitAIRetargetEntry>,
    best_unit_to_attack: Option<u32>,
    formation_type: u8,
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
