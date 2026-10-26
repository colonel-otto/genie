//! What the comparison with the game's saves checks of an object ([`ObjectRow`]) and its
//! bit-exact equality ([`Same`]).

use crate::{listed, path_level, top_action, vec3};
use genie_core::turn::{Turns, Unit};
use genie_save::generated::world::Path;

/// The rows the comparison with the game's saves reads of a player's live objects in a slice.
#[must_use]
pub fn object_rows(turns: &Turns, player: usize) -> Vec<ObjectRow> {
    turns.players.get(player).map_or_else(Vec::new, |p| {
        p.objects
            .iter()
            .filter(|u| listed(u))
            .map(ObjectRow::of_unit)
            .collect()
    })
}

/// What the comparison with the game's saves checks of one live object: its master, state, hit
/// points, held amount and position in full f32, its top action (type, state) and its unit AI's
/// retarget timer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ObjectRow {
    /// The object's id.
    pub id: i32,
    /// Its master.
    pub master: i16,
    /// Its state.
    pub state: u8,
    /// Its hit points.
    pub hp: f32,
    /// What it holds.
    pub amount: f32,
    /// Its place.
    pub position: [f32; 3],
    /// Its top action's type and state.
    pub action: Option<(i16, u8)>,
    /// The top action's target (`+0x30`), where the slice holds the action.
    pub action_target: Option<i32>,
    /// Its unit AI's retarget timer.
    pub retarget: Option<i32>,
    /// A moving object's orientation, velocity data and path data (the level it walks first).
    pub movement: Option<MoveRow>,
    /// A building's construction: desolid, progress, builders.
    pub construction: Option<(bool, f32, u8)>,
    /// A Build action's `added_to_num_builders`.
    pub build_added: Option<bool>,
    /// The unit AI's order, action and target with their last values, and its timers.
    pub ai: Option<AiRow>,
    /// `+0x650`, the last human order time.
    pub human_order: u32,
    /// A building's gather point: x, y and its target.
    pub gather: Option<([f32; 2], i32)>,
    /// A combat object's charge, `mHitSomethingValidForChargeAttack` and hit point powerups,
    /// which its hits and the hits on it change.
    pub combat: Option<(f32, bool, [f32; 2])>,
    /// A building's gate lock.
    pub gate: Option<genie_core::turn::Gate>,
    /// A combat object's town-bell flag and back-to-work records.
    pub town_bell: Option<genie_core::town_bell::TownBell>,
    /// A combat object's `next_volley` (the missiles of its volley under way already fired) and
    /// `mReloadWorldtime` (when it may attack again).
    pub volley: Option<(u8, u32)>,
    /// Its unit group (`mGroupID`, `mGroupFlag`).
    pub group: genie_core::group::GroupLink,
    /// A building's production action (its make action or research): its state, which the
    /// checkpoint's word 9 reads as the building's action state, and its progress.
    pub making: Option<(u8, f32)>,
}

/// What an object's row compares of its unit AI (named as in the save).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AiRow {
    /// The fields, named as in the save, with their values.
    pub fields: Vec<(&'static str, String)>,
}

impl AiRow {
    pub(super) fn of(ai: &genie_core::unit_ai::UnitAi, under_attack: bool) -> Self {
        let f = |v: &dyn std::fmt::Debug| format!("{v:?}");
        Self {
            fields: vec![
                ("current_order", f(&ai.current_order)),
                ("current_order_priority", f(&ai.order_priority)),
                ("current_action", f(&ai.current_action)),
                ("current_target", f(&ai.current_target)),
                ("current_target_type", f(&ai.target_type)),
                ("current_target_point", f(&ai.target_point)),
                ("desired_target_distance", f(&ai.desired_distance)),
                ("defend_target", f(&ai.defend_target)),
                ("last_order", f(&ai.last_order)),
                ("last_action", f(&ai.last_action)),
                ("last_target", f(&ai.last_target)),
                ("last_target_type", f(&ai.last_target_type)),
                ("idle_timer", f(&ai.idle_timer)),
                ("adjusted_idle_timeout", f(&ai.adjusted_idle_timeout)),
                ("secondary_timer", f(&ai.secondary_timer)),
                ("look_around_timer", f(&ai.look_around_timer)),
                ("look_around_timeout", f(&ai.look_around_timeout)),
                ("last_retarget_time", f(&ai.last_retarget_time)),
                ("state_position", f(&ai.state_position)),
                ("last_update_time", f(&ai.last_update_time)),
                ("alert_mode", f(&ai.alert_mode)),
                ("time_since_enemy_sighting", f(&ai.time_since_enemy)),
                ("orders", f(&ai.orders)),
                ("notifies", f(&ai.notifies)),
                ("attackers", f(&ai.attackers)),
                ("under_attack", f(&under_attack)),
                ("furious", f(&ai.home.map(|h| h.furious))),
                ("formation_type", f(&ai.formation_type)),
            ],
        }
    }

    pub(super) fn saved(
        ai: &genie_save::generated::world::UnitAiModule,
        elephant: Option<&genie_save::generated::world::TribeElephantUnitAiModule>,
        under_attack: bool,
    ) -> Self {
        let f = |v: &dyn std::fmt::Debug| format!("{v:?}");
        Self {
            fields: vec![
                ("current_order", f(&ai.current_order_value)),
                (
                    "current_order_priority",
                    f(&ai.current_order_priority_value),
                ),
                ("current_action", f(&ai.current_action_value)),
                ("current_target", f(&ai.current_target_value)),
                ("current_target_type", f(&ai.current_target_type_value)),
                (
                    "current_target_point",
                    f(&[
                        ai.current_target_xvalue,
                        ai.current_target_yvalue,
                        ai.current_target_zvalue,
                    ]),
                ),
                (
                    "desired_target_distance",
                    f(&ai.desired_target_distance_value),
                ),
                ("defend_target", f(&ai.defend_target_value)),
                ("last_order", f(&ai.last_order_value)),
                ("last_action", f(&ai.last_action_value)),
                ("last_target", f(&ai.last_target_value)),
                ("last_target_type", f(&ai.last_target_type_value)),
                ("idle_timer", f(&ai.idle_timer_value)),
                ("adjusted_idle_timeout", f(&ai.adjusted_idle_timeout_value)),
                ("secondary_timer", f(&ai.secondary_timer_value)),
                ("look_around_timer", f(&ai.look_around_timer_value)),
                ("look_around_timeout", f(&ai.look_around_timeout_value)),
                ("last_retarget_time", f(&ai.last_retarget_time)),
                (
                    "state_position",
                    f(&[ai.state_position_xvalue, ai.state_position_yvalue]),
                ),
                ("last_update_time", f(&ai.last_update_time_value)),
                ("alert_mode", f(&(ai.alert_mode_value != 0))),
                (
                    "time_since_enemy_sighting",
                    f(&ai.time_since_enemy_sighting),
                ),
                (
                    "orders",
                    f(&ai
                        .temp_order
                        .iter()
                        .map(genie_core::unit_ai::QueuedOrder::from_bytes)
                        .collect::<Vec<_>>()),
                ),
                (
                    "notifies",
                    f(&ai
                        .temp_notify
                        .iter()
                        .map(genie_core::unit_ai::Notify::from_bytes)
                        .collect::<Vec<_>>()),
                ),
                ("attackers", f(&ai.temp_int)),
                ("under_attack", f(&under_attack)),
                ("furious", f(&elephant.map(|e| e.furious != 0))),
                ("formation_type", f(&ai.formation_type)),
            ],
        }
    }
}

/// Equality to the bit (an `f32` by its bits): two values the same this way print the same, so a
/// row the same this way has no difference ([`object_differences`] compares the fields' texts).
///
/// [`object_differences`]: crate::differences::object_differences
pub(super) trait Same {
    fn same(&self, other: &Self) -> bool;
}

macro_rules! same_by_eq {
    ($($t:ty),*) => {
        $(impl Same for $t {
            fn same(&self, other: &Self) -> bool {
                self == other
            }
        })*
    };
}

same_by_eq!(
    i16,
    i32,
    u8,
    u32,
    bool,
    &'static str,
    String,
    genie_core::turn::Gate
);

impl Same for f32 {
    fn same(&self, other: &Self) -> bool {
        self.to_bits() == other.to_bits()
    }
}

impl<T: Same> Same for Option<T> {
    fn same(&self, other: &Self) -> bool {
        match (self, other) {
            (Some(a), Some(b)) => a.same(b),
            (None, None) => true,
            _ => false,
        }
    }
}

impl<T: Same, const N: usize> Same for [T; N] {
    fn same(&self, other: &Self) -> bool {
        self.iter().zip(other).all(|(a, b)| a.same(b))
    }
}

impl<T: Same> Same for Vec<T> {
    fn same(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().zip(other).all(|(a, b)| a.same(b))
    }
}

impl<A: Same, B: Same> Same for (A, B) {
    fn same(&self, other: &Self) -> bool {
        self.0.same(&other.0) && self.1.same(&other.1)
    }
}

impl<A: Same, B: Same, C: Same> Same for (A, B, C) {
    fn same(&self, other: &Self) -> bool {
        self.0.same(&other.0) && self.1.same(&other.1) && self.2.same(&other.2)
    }
}

impl Same for genie_core::town_bell::BackToWork {
    fn same(&self, other: &Self) -> bool {
        self.target == other.target
            && self.target_type == other.target_type
            && self.action == other.action
            && self.target2 == other.target2
            && self.point.same(&other.point)
    }
}

impl Same for genie_core::town_bell::TownBell {
    fn same(&self, other: &Self) -> bool {
        self.flag == other.flag
            && self.work.same(&other.work)
            && self.before_building.same(&other.before_building)
    }
}

impl Same for genie_core::movement::PathData {
    fn same(&self, other: &Self) -> bool {
        self.level == other.level
            && self.linked_type == other.linked_type
            && self.points.same(&other.points)
            && self.has_path == other.has_path
            && self.path_flags == other.path_flags
            && self.waypoint == other.waypoint
            && self.state == other.state
            && self.range.same(&other.range)
            && self.target == other.target
            && self.pause_time.same(&other.pause_time)
            && self.flags == other.flags
            && self.length.same(&other.length)
            && self.created == other.created
            && self.continue_counter == other.continue_counter
            && self.repath_time.same(&other.repath_time)
            && self.ignore == other.ignore
            && self.ignored == other.ignored
    }
}

impl Same for MoveRow {
    fn same(&self, other: &Self) -> bool {
        self.forward.same(&other.forward)
            && self.right.same(&other.right)
            && self.velocity.same(&other.velocity)
            && self.paths.same(&other.paths)
            && self.waiting == other.waiting
            && self.wait_delays == other.wait_delays
            && self.attack_moving == other.attack_moving
            && self.last_move == other.last_move
            && self.substitute_count == other.substitute_count
            && self.ignore_units == other.ignore_units
            && self.angle.same(&other.angle)
            && self.last_turn.same(&other.last_turn)
    }
}

impl Same for AiRow {
    fn same(&self, other: &Self) -> bool {
        self.fields == other.fields
    }
}

impl Same for ObjectRow {
    fn same(&self, other: &Self) -> bool {
        self.id == other.id
            && self.master == other.master
            && self.state == other.state
            && self.hp.same(&other.hp)
            && self.amount.same(&other.amount)
            && self.position.same(&other.position)
            && self.action == other.action
            && self.action_target == other.action_target
            && self.retarget == other.retarget
            && self.movement.same(&other.movement)
            && self.construction.same(&other.construction)
            && self.build_added == other.build_added
            && self.ai.same(&other.ai)
            && self.human_order == other.human_order
            && self.gather.same(&other.gather)
            && self.combat.same(&other.combat)
            && self.gate == other.gate
            && self.town_bell.same(&other.town_bell)
            && self.volley == other.volley
            && self.group == other.group
            && self.making.same(&other.making)
    }
}

/// What a moving object's row compares of its movement.
#[derive(Clone, Debug, PartialEq)]
pub struct MoveRow {
    /// `+0x830`: where it heads.
    pub forward: genie_core::math::fmath::Vec3,
    /// `+0x83c`: its right.
    pub right: genie_core::math::fmath::Vec3,
    /// Velocity and acceleration, when the object has velocity data.
    pub velocity: Option<(genie_core::math::fmath::Vec3, genie_core::math::fmath::Vec3)>,
    /// Its path data, the walked level first.
    pub paths: Vec<genie_core::movement::PathData>,
    /// `+0x7f9`, `+0x7fa`, `+0x7fb`, `+0x870`, `+0x874`.
    pub waiting: u8,
    /// `+0x7fa`: the walk's waits.
    pub wait_delays: u8,
    /// `+0x7fb`: it moves to attack.
    pub attack_moving: bool,
    /// `+0x870`: its last move time.
    pub last_move: u32,
    /// `+0x874`: the substitutes counted.
    pub substitute_count: i32,
    /// The unit's ignore list (`+0x670`).
    pub ignore_units: Vec<i32>,
    /// `+0x7e8`, where it faces.
    pub angle: f32,
    /// `+0x7ec`, `turn_towards_time`: its last facet step toward a target.
    pub last_turn: f32,
}

impl ObjectRow {
    pub(super) fn of_unit(unit: &Unit) -> Self {
        Self {
            id: unit.id,
            master: unit.life.master_id,
            state: unit.life.state,
            hp: unit.life.hp,
            amount: unit.life.amount,
            position: unit.position,
            action: top_action(unit).map(|a| (a.kind, a.state)),
            action_target: unit.top_target(),
            retarget: unit.ai.as_ref().map(|ai| ai.retarget_timer as i32),
            movement: unit.mover.as_ref().map(|m| MoveRow {
                forward: m.forward,
                right: m.right,
                velocity: m.velocity.as_ref().map(|v| (v.velocity, v.acceleration)),
                // the paths' ignore units, which the save does not hold, are left out
                paths: m
                    .paths
                    .iter()
                    .map(|l| genie_core::movement::PathData {
                        ignored: Vec::new(),
                        ..l.clone()
                    })
                    .collect(),
                waiting: m.waiting,
                wait_delays: m.wait_delays,
                attack_moving: m.attack_moving,
                last_move: m.last_move,
                substitute_count: m.substitute_count,
                ignore_units: m.ignore_units.clone(),
                angle: m.angle,
                last_turn: m.last_turn,
            }),
            construction: unit
                .construction
                .map(|c| (c.desolid, c.progress, c.builders)),
            build_added: unit.build.map(|b| b.added),
            ai: unit.ai.as_ref().map(|ai| AiRow::of(ai, unit.under_attack)),
            human_order: unit.last_human_order,
            gather: unit
                .gather_point
                .map(|g| ([g.point[0], g.point[1]], g.target)),
            combat: unit
                .combat
                .then_some((unit.charge, unit.charge_hit, unit.hp_powerups)),
            gate: unit.gate,
            town_bell: unit.combat.then_some(unit.town_bell),
            volley: unit
                .combat
                .then_some((unit.next_volley, unit.reload_worldtime)),
            group: unit.group,
            making: unit
                .production
                .as_ref()
                .and_then(|p| p.action.as_ref())
                .map(|a| match a {
                    genie_core::production::Making::Unit(m) => (m.state, m.progress),
                    genie_core::production::Making::Research(t) => (t.state, t.progress),
                }),
        }
    }
}

/// A saved moving object's [`MoveRow`]; a level whose path the world's table lacks is left out.
pub(super) fn saved_movement(object: &genie_save::Object, paths: &[Path]) -> Option<MoveRow> {
    let m = object.moving_part()?;
    let s = object.static_part()?;
    Some(MoveRow {
        forward: vec3(&m.orientation_forward_value),
        right: vec3(&m.orientation_right_value),
        velocity: (m.have_movement == 1).then(|| {
            (
                m.velocity.as_ref().map_or([0.0; 3], vec3),
                m.acceleration.as_ref().map_or([0.0; 3], vec3),
            )
        }),
        paths: m
            .temp_path_data
            .iter()
            .rev()
            .filter_map(|d| path_level(d, paths).ok())
            .collect(),
        waiting: m.waiting_to_move_value,
        wait_delays: m.number_wait_delays,
        attack_moving: m.attack_moving != 0,
        last_move: m.last_move_time_value,
        substitute_count: m.consecutive_substitute_count,
        ignore_units: s.temp_int.clone(),
        angle: m.angle,
        last_turn: m.turn_towards_time,
    })
}
