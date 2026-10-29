//! The orders, actions, timers and reaches the unit AI names, its factory types and stances.

use crate::GenieError;
use crate::math::rng::Well512;
use crate::unit_ai::{
    ARTIFACT_AI, ActionKind, AiMove, BOAR_AI, CIVILIAN, FISHING_AI, PRIEST_AI, QueuedOrder,
    SIEGE_AI, SIEGE_TOWER_AI, SOLDIER_AI, Surroundings, TOWER_AI, TRADE_AI,
};

/// What slot 73 does for a factory type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Retarget {
    /// The base slot 73: the retarget check.
    Base,
    /// An override that never reaches the check: livestock (19, `0x140c21f20`: 9).
    Never,
    /// The villagers' override (8, `0x140c21da0`, [`UnitAi::civilian_misc`]), which never
    /// reaches the check either.
    ///
    /// [`UnitAi::civilian_misc`]: crate::unit_ai::UnitAi::civilian_misc
    Civilian,
    /// The artifact's override (14, `FUN_140c21d40`, [`UnitAi::artifact_misc`]): its
    /// processIdle, no check.
    ///
    /// [`UnitAi::artifact_misc`]: crate::unit_ai::UnitAi::artifact_misc
    Artifact,
    /// The priest's override (9, `FUN_140c21f30`): the base's check but on the order on a target
    /// (`0x2db`) and for a computer player's priest, which return 9.
    Priest,
}

pub(super) const fn retarget_kind(ai_type: u32) -> Result<Retarget, GenieError> {
    match ai_type {
        // villagers (0x140c21da0) and livestock (0x140c21f20) return 9 from slot 73
        8 => Ok(Retarget::Civilian),
        19 => Ok(Retarget::Never),
        ARTIFACT_AI => Ok(Retarget::Artifact),
        PRIEST_AI => Ok(Retarget::Priest),
        // hunted animal (2, slot 72 FUN_140c1eea0), boar (4, slot 72 FUN_140c1e630), lion (5,
        // slot 72 FUN_140c1fb70), trade ship (0xa, slot 72 FUN_140c21930), transport ship (11,
        // slot 72 0x140c21d20), fishing ship (0xc, slot 72 FUN_140c1eab0), soldier (13, slot 72
        // FUN_140c20db0), tower (15, the base's slot 72 FUN_140c564c0), building (17, slot 72
        // 0x140c1e600, which does nothing), siege unit (0x10, slot 72 FUN_140c206b0), siege tower
        // (0x12, slot 72 FUN_140c20690)
        2 | 4 | 5 | TRADE_AI | 11 | FISHING_AI | 13 | 15 | SIEGE_AI | 17 | SIEGE_TOWER_AI => {
            Ok(Retarget::Base)
        }
        _ => Err(GenieError::unsupported("unit-AI type not modelled")),
    }
}

/// The base idle timeout (`+0x168`) the constructor sets; the adjusted one is drawn around it.
pub(super) const IDLE_TIMEOUT: u32 = 3000;
/// The tower's idle timeout (`FUN_140c12960`'s `FUN_140c41bc0` with 1000).
pub(super) const TOWER_IDLE_TIMEOUT: u32 = 1000;
/// The artifact's idle timeout (`FUN_140c11340`'s `FUN_140c41bc0` with 5000).
pub(super) const ARTIFACT_IDLE_TIMEOUT: u32 = 5000;
/// The desired target distance (`+0x5c`) the constructor sets.
pub(super) const DESIRED_DISTANCE: f32 = 2.0;

/// The boar's idle check (`FUN_140c1e630`): it stays within a draw of `0..81` plus `DAT_142eb5bbc`
/// (144) of its origin, squared; within `DAT_142eb0f48` (4) it wanders `-3..=2` tiles each way
/// (`(r % 6) - 3`), once in 7 checks; its moves come within `DAT_142e7f384` (0.5).
pub(super) const BOAR_LEASH_DRAW: u32 = 81;
pub(super) const BOAR_LEASH: f32 = 144.0;
pub(super) const BOAR_HOME: f32 = 4.0;
pub(super) const BOAR_WANDER: u32 = 7;
pub(super) const BOAR_STEP: u32 = 6;
pub(super) const BOAR_OFFSET: i32 = 3;
pub(super) const AI_MOVE_RANGE: f32 = 0.5;
/// The height of the explore's target point (`FUN_140c4d910`, `DAT_14330eb8c`).
pub(super) const EXPLORE_Z: f32 = crate::math::image::unit_ai::EXPLORE_Z;
/// The villager's reach test of the foundation its search found (slot 0x3d0 in
/// `FUN_140c22bf0`, `DAT_142eb5690`).
pub(super) const FOUNDATION_REACH: f32 = crate::math::image::unit_ai::FOUNDATION_REACH;

/// A villager's notify, or a part of one, the slice does not model.
pub(super) const VILLAGER_NOTIFY: GenieError =
    GenieError::unsupported("a villager's notify the slice does not model");

/// A villager's damage notify that reads its animation speed (slot 0x1f8) while the slice does
/// not know it ([`Surroundings::anim_speed`]).
pub(super) const NO_ANIM_SPEED: GenieError =
    GenieError::unsupported("a moving villager's damage notify without its animation speed");
/// The target point's x that marks it unset (`DAT_14330ec20`, −1), which the defend order fills.
pub(super) const INVALID_POINT: f32 = crate::math::image::unit_ai::INVALID_POINT;
/// What the defend order adds to its desired distance before it follows (`DAT_14330eb8c`, 1).
pub(super) const DEFEND_SLACK: f32 = crate::math::image::unit_ai::DEFEND_SLACK;
/// The master whose unit takes no defend order (slot 0x230's case 0x2bd: raw `0x611c`).
pub(super) const DEFENDLESS_MASTER: i16 = 128;

/// An order on an object run without the unit AI's update (its surroundings and side), as
/// [`UnitAi::take_order_now`] runs one.
///
/// [`UnitAi::take_order_now`]: crate::unit_ai::UnitAi::take_order_now
pub(super) const OUTSIDE_ORDER: GenieError =
    GenieError::unsupported("an order on an object run outside the unit AI's update");

/// The move an order to a point gives ([`UnitAi::process_order`]'s move and attack move): the
/// order's point and range, forced and walking.
///
/// [`UnitAi::process_order`]: crate::unit_ai::UnitAi::process_order
pub(super) const fn order_move(o: &QueuedOrder) -> AiMove {
    AiMove {
        point: o.point,
        range: o.range,
        force: true,
        walk: true,
    }
}
/// The trebuchet's classes (`-0xe79`, `-0xe76` raw: packed and unpacked), whose attack move on
/// a target attacks it.
pub(super) const TREBUCHET_CLASS: i16 = 51;
pub(super) const TREBUCHET_PACKED_CLASS: i16 = 54;
/// The follow order's target counts as moved when it is more than a tile off the AI's target
/// point on either axis (`0x140c567c0`: `DAT_14330ec20`, `DAT_14330eb8c`).
pub(super) const FOLLOW_SLACK_LOW: f32 = crate::math::image::unit_ai::FOLLOW_SLACK_LOW;
pub(super) const FOLLOW_SLACK_HIGH: f32 = crate::math::image::unit_ai::FOLLOW_SLACK_HIGH;

/// The hunted animal's idle check (`FUN_140c1eea0`): away from its origin by more than
/// `DAT_142eb0f48` (4, squared) it goes home once in 7 checks; otherwise it wanders 1 to 3 tiles
/// each way (`HUNTED_STEP`), in mode 1 once in 28 and by a tile. Its search reaches its line of
/// sight plus `DAT_14330eb8c` (1), or its search radius if that is as far, plus `+0x1e8` (unsaved,
/// 0 in every unit probed).
pub(super) const HUNTED_HOME: f32 = 4.0;
pub(super) const HUNTED_STEP: i32 = 3;
pub(super) const HUNTED_SIGHT_GROWTH: f32 = 1.0;
/// What an idle check lowers the unit's last human order time to (`FUN_140fe0d10`), and below
/// which the moving object's update counts it down.
pub const HUMAN_ORDER_CAP: u32 = 5000;
/// The master whose idle soldier does nothing (`FUN_140c20db0`, raw `0x635e`).
pub(super) const SOLDIER_IDLE_MASTER: i16 = 706;
/// The lion's idle timeout (`FUN_140c1fb70`, as its constructor `FUN_140c11bd0`).
pub(super) const LION_IDLE_TIMEOUT: u32 = 6000;
/// The lion's idle timeout once its attack completed (`FUN_140c266f0`).
pub(super) const LION_ATTACKED_TIMEOUT: u32 = 1000;
/// Outside mode 1 the lion attacks within this share of its line of sight (`FUN_140c1fb70`,
/// `DAT_142e7f384`).
pub(super) const LION_SIGHT_SHARE: f32 = crate::math::image::unit_ai::LION_SIGHT_SHARE;
/// Finding no villager outside mode 1 it attacks within its line of sight over this
/// (`FUN_140c1fb70`, `DAT_142eb1c98`).
pub(super) const LION_SIGHT_DIVISOR: f32 = crate::math::image::unit_ai::LION_SIGHT_DIVISOR;
/// The king's master that the branch passes over (raw `0x635c`).
pub(super) const KING_UNLED: i16 = 704;

/// The walk back's gap and range (`FUN_140c5e7d0`: `DAT_14330eb8c`, 1.0, whose image role in
/// the unit AI's table is [`RUN_ONE`]'s).
///
/// [`RUN_ONE`]: crate::unit_ai::RUN_ONE
pub(super) const WALK_BACK: f32 = crate::math::image::unit_ai::RUN_ONE;

/// The action values the full update resets once the unit's actions ended (`0x140c66b10`, the
/// unit's slot 0x430 false).
pub(super) const ENDED_ACTIONS: [ActionKind; 9] = [
    ActionKind::Attack,
    ActionKind::Gather,
    ActionKind::Move,
    ActionKind::Hunt,
    ActionKind::Enter,
    ActionKind::Make,
    ActionKind::Research,
    ActionKind::BoarLuring,
    ActionKind::MoveAside,
];
/// A human player's unit on the attack order with a range above this does not retarget
/// (processMisc).
pub(super) const RETARGET_RANGE: f32 = crate::math::image::unit_ai::RETARGET_RANGE;
/// The retarget's reference distance about a current target (`DAT_142eb9430`).
pub(super) const RETARGET_REACH: f32 = crate::math::image::unit_ai::RETARGET_REACH;

/// What the retarget's search reads of its checks ([`UnitAi::retarget_outcome`]): the current
/// target is a building's, wall's or trebuchet's (or one off the attack order), and the top
/// action is in an attack's work state (6 or 7).
///
/// [`UnitAi::retarget_outcome`]: crate::unit_ai::UnitAi::retarget_outcome
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct RetargetSearch {
    pub(super) building: bool,
    pub(super) work_state: bool,
}

/// The unit-AI type the factory (`FUN_141215a90`) gives a new object of a master of `class`
/// with the volley missile `missile` (`+0x8d4`, −1 for none) and the combat level
/// `combat_level` (`+0x62c`). The types are those `FUN_141215560`'s switch names for its
/// constructors: masters 1105 (`FUN_140c11fb0`, 0x12) and 1744, 1746 (`FUN_140c12040`, 0x10) by
/// their id; then none for classes 5, 7, 8, 11, 14, 15, 16, 30 to 34, 38, 42, 48 and 63; 0xe for
/// class 1; 0xa for 2 and 19; the building's (0x11) for classes 3, 27, 39, 49 and 60 without a
/// volley missile, the tower's (0xf) for those with one and for 52; the civilian's (8) for 4;
/// the hunted animal's (2) for 9 and 59; 0x10 for 13, 51, 54 and 55; 9 for 18, 43 and 53; 0xb for
/// 20; 0xc for 21; 0x13 for 29 and 58; the soldier's (0xd) for any other. Class 10 takes the
/// boar's (4, `FUN_140c11720`) when its master is of class 10 and combat level 1
/// (`FUN_140fde4f0`), else the lion's (5, `FUN_140c11bd0`).
#[must_use]
pub const fn factory_type(class: i16, master: i16, missile: i16, combat_level: u8) -> Option<u32> {
    match master {
        1105 => return Some(0x12),
        1744 | 1746 => return Some(0x10),
        _ => {}
    }
    Some(match class {
        5 | 7 | 8 | 11 | 14 | 15 | 16 | 30..=34 | 38 | 42 | 48 | 63 => return None,
        1 => 0xe,
        2 | 19 => 0xa,
        3 | 27 | 39 | 49 | 60 if missile < 0 => 17,
        3 | 27 | 39 | 49 | 52 | 60 => TOWER_AI,
        4 => CIVILIAN,
        9 | 59 => 2,
        // FUN_140fde4f0: the master's class (`+0x410`, 10 in this case) and its combat level 1
        10 if combat_level == 1 => BOAR_AI,
        10 => 5,
        13 | 51 | 54 | 55 => 0x10,
        18 | 43 | 53 => 9,
        20 => 0xb,
        21 => 0xc,
        29 | 58 => 0x13,
        _ => SOLDIER_AI,
    })
}

/// What a unit AI's constructor reads besides its type (`FUN_141215a90`'s and
/// `FUN_141215560`'s constructors): the new object's place, its master's fields and its
/// owner's.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NewObject {
    /// The object's place (`+0x3c8`, `+0x4c8`, `+0x5a8`): the state position, the hunted
    /// animal's and the boar's origin.
    pub position: [f32; 3],
    /// Its master's id (`+0x58`).
    pub master: i16,
    /// Its master's `copy_id` (`+0x158`).
    pub copy_id: i16,
    /// Its master's class (`+0x410`).
    pub class: i16,
    /// Its master's `object_flags` (`+0x654`).
    pub object_flags: u32,
    /// Its master's `hero_flag` (`+0x8d9`).
    pub hero_flag: u8,
    /// The object's slot 0x7b0 (`+0xb74`, which the action object's constructor set before
    /// through slot 0x8a8, `FUN_141206d50`: the work range of its master's last task of action
    /// type 6, else 0): the hunted animal's leash.
    pub hunt_range: f32,
    /// The owner is a computer player (`FUN_141659cf0`: `+0x10` 3).
    pub computer: bool,
    /// The owner's posture (`+0x71c`, its slot 0x90 `FUN_1416224f0`).
    pub posture: i32,
    /// The owner's attribute 6, the current age (`player+0x100` at `+0x30`).
    pub age: f32,
}

impl NewObject {
    /// The object at `position` of master `made` (id `master`), its owner's fields left at a
    /// human player's in the Dark Age with posture 0 ([`NewObject::computer`],
    /// [`NewObject::posture`], [`NewObject::age`] are the caller's).
    #[must_use]
    pub fn of(position: [f32; 3], master: i16, made: &crate::objects::masters::Master) -> Self {
        Self {
            position,
            master,
            copy_id: made.copy_id,
            class: made.class,
            object_flags: made.object_flags,
            hero_flag: made.hero_flag,
            // FUN_141206d50: each task of action type 6 in turn sets `+0xb74`
            hunt_range: made
                .tasks
                .iter()
                .rev()
                .find(|t| t.action_type == HUNT_RANGE_TASK)
                .map_or(0.0, |t| t.work_range),
            computer: false,
            posture: 0,
            age: 0.0,
        }
    }
}

/// The action type of the task whose work range a new object keeps at `+0xb74`
/// (`FUN_141206d50`).
pub(super) const HUNT_RANGE_TASK: i16 = 6;
/// The hunted animal's idle timeout (`FUN_140c11920`'s `FUN_140c41bc0` with 4000).
pub(super) const HUNTED_IDLE_TIMEOUT: u32 = 4000;
/// The priest's idle timeout (`FUN_140c11e30`'s `FUN_140c41bc0` with 500).
pub(super) const PRIEST_IDLE_TIMEOUT: u32 = 500;
/// The soldier's idle timeout (`FUN_140c12300`'s `FUN_140c41bc0`): 500, a computer player's
/// 1500.
pub(super) const SOLDIER_IDLE_TIMEOUT: u32 = 500;
pub(super) const COMPUTER_SOLDIER_IDLE_TIMEOUT: u32 = 1500;
/// The siege unit's idle timeout (`FUN_140c12040`'s `FUN_140c41bc0` with 1000).
pub(super) const SIEGE_IDLE_TIMEOUT: u32 = 1000;
/// The masters whose soldier is defensive (`FUN_140c12300`, raw `0x62ab`, `0x62ac`, `0x64ec`).
pub(super) const DEFENSIVE_MASTERS: [i16; 3] = [527, 528, 1104];
/// The classes of a human player's soldier in `NoAttack` (`FUN_140c12300`: `-0xe89`, `-0xe6c`
/// raw).
pub(super) const NO_ATTACK_CLASSES: [i16; 2] = [35, 64];
/// The class whose soldier is in `NoAttack` unless a hero (`FUN_140c12300`: `-0xe6f` raw).
pub(super) const NO_ATTACK_CLASS: i16 = 61;
/// The master object flag of the soldier that stands its ground in the Dark Age
/// (`FUN_140c12300`: `+0x654` bit 4).
pub(super) const STAND_GROUND_FLAG: u32 = 0x10;

/// The soldier's stance (`+0x1a9`) its constructor (`FUN_140c12300`) gives a new object: 2
/// (stand ground) to a human player's object of a master with object flag 0x10 that is its own
/// copy (`copy_id`, `+0x158`, its id) while the player's current age (attribute 6) is 0; 1
/// (defensive) to a human player's of masters 527, 528 and 1104; 3 (no attack) to a human
/// player's of classes 35 and 64; then 1 to a hero (`hero_flag` with bit 0 or 3, not both 3
/// and 7); 3 to class 61; else the owner's posture (slot 0x90, `+0x71c`, its low byte).
pub(super) fn soldier_stance(new: &NewObject) -> u8 {
    // UCOMISS with JP, JNZ: the age equals 0 (a NaN does not)
    if !new.computer
        && new.object_flags & STAND_GROUND_FLAG != 0
        && new.copy_id == new.master
        && new.age == 0.0
    {
        return 2;
    }
    if !new.computer && DEFENSIVE_MASTERS.contains(&new.master) {
        return 1;
    }
    if !new.computer && NO_ATTACK_CLASSES.contains(&new.class) {
        return 3;
    }
    // TEST 9, then a signed byte at least 0 or bit 3 clear
    let hero = new.hero_flag & 9 != 0 && (new.hero_flag & 0x80 == 0 || new.hero_flag & 8 == 0);
    if hero {
        return 1;
    }
    if new.class == NO_ATTACK_CLASS {
        return 3;
    }
    new.posture as u8
}

/// The siege unit's stance (`+0x1a9`) its constructor (`FUN_140c12040`) gives a new object: 0
/// for a computer player's and for classes 51 and 54 (`-0xe79`, `-0xe76` raw); the owner's
/// posture (slot 0x90, its low byte) for class 55 (`-0xe75`); 1 for any other.
pub(super) const fn siege_stance(new: &NewObject) -> u8 {
    if new.computer || matches!(new.class, 51 | 54) {
        return 0;
    }
    if new.class == 55 {
        return new.posture as u8;
    }
    1
}

/// A ship's draw on a missile's warning (`0x20f`, `processNotify`): `r * 100 / 0x7fff` below
/// its owner's `chance_to_dodge_missiles` ([`Surroundings::dodge_chance`]) dodges it.
pub(super) fn dodges(at: &Surroundings, ai: &mut Well512) -> bool {
    // IMUL by 100, then IDIV by 0x7fff: a 15-bit draw stays positive
    let r = ai.rand15() as i32;
    r * 100 / 0x7fff < i32::from(at.dodge_chance)
}

/// The owner's slot 0x2e8 with an alert of its unit's (0x201 under attack, 0x209 dead; 0x206
/// and 0x207 from a transport, units left inside or no unload there; `FUN_14162dab0`): a human
/// player's alert list or interface message and the old pather's hulled-area reset, not held;
/// a computer player's AI hears it (slot 0x3c8), which is not modelled.
pub(crate) const fn alert_owner(computer: bool) -> Result<(), GenieError> {
    if computer {
        return Err(GenieError::unsupported(
            "a computer player's AI hearing its unit's alert (slot 0x2e8, then slot 0x3c8)",
        ));
    }
    Ok(())
}

/// `AskForHelp`'s helper that passes every test ([`crate::unit_ai::world_view::UnitView::helper_free`]):
/// its reach to the attacker (`FUN_14104c220` within its weapon range), its own
/// `bestUnitToAttack` (`FUN_140c44680` from its place) and its `AttackObject` (slot 0xb8, any)
/// on the nearer of that and the attacker, which run as the helper's, are not modelled.
pub(crate) const fn helper_answers() -> Result<(), GenieError> {
    Err(GenieError::unsupported(
        "a helper that answers a call for help (its reach, bestUnitToAttack and AttackObject)",
    ))
}

/// The unit AI's slot 0x90 (`closestUndiscoveredTile`, `FUN_140c4bd40`) for an explorer other
/// than the auto scout, then its explore there (slot 0x100); not modelled.
pub(super) const fn closest_undiscovered_tile() -> Result<(), GenieError> {
    Err(GenieError::unsupported(
        "an explorer's search for the closest undiscovered tile (slot 0x90)",
    ))
}

/// The state of the unit's top action (slot 0x2b0) as [`Surroundings::action_state`] holds it,
/// for an action the slice holds.
pub(super) const fn held_action_state(state: Option<u8>) -> Result<u8, GenieError> {
    match state {
        Some(s) => Ok(s),
        None => Err(GenieError::unsupported(
            "an action the slice does not hold (its state, slot 0x2b0)",
        )),
    }
}
