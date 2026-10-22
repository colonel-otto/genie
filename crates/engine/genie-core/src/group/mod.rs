//! Unit groups and formations (FD-036): the game's `BUnitGroup` (`group.cpp`) with its motion
//! (`BMotion`) and formations (`BFormation`).
//!
//! The order splitter puts soldiers that move together into a group (`FUN_140fd76d0`); the group
//! moves as one (its own path and motion), keeps its members in a formation and gives each a
//! move of its own. The world holds the groups in a list (`world+0x50`, created by
//! `FUN_14152efc0`, looked up by id by `FUN_14152ff40`) and counts the ids it gave out
//! (`mUnitGroupCounter`, `world+0x5c`); a unit names its group (`mGroupID`, `+0x6d0`).
//!
//! This module holds the state, field for field as the save writes it (`FUN_1410321e0`; the
//! read is `FUN_14101d350`), each named by the game's member, so that a save of any build fills
//! it and the forced replay compares every field. The behaviour is ported routine by routine
//! from build 158041, whose addresses each port cites.

use crate::movement::PathData;

mod command;
mod create;
pub mod formation;
mod layout;
mod motion;
mod place;
mod stats;
mod steer;
mod update;

/// An object's link to its group: `mGroupID` (`+0x6d0`) and `mGroupFlag` (`+0x6d4`), -1 and -2
/// from the object's setup (`FUN_141196df0`) and in every object of a save in no group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GroupLink {
    /// `mGroupID`: the group's id, -1 for none.
    pub id: i32,
    /// `mGroupFlag`.
    pub flag: i32,
}

impl Default for GroupLink {
    fn default() -> Self {
        Self { id: -1, flag: -2 }
    }
}

/// The world's unit groups (`mNumberUnitGroups`, `mUnitGroupCounter`, `FUN_141536ef0`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UnitGroups {
    /// The groups in the world list's order.
    pub groups: Vec<UnitGroup>,
    /// The id the next group takes (`mUnitGroupCounter`, `world+0x5c`).
    pub counter: i32,
    /// The commands that ran the order splitter so far (`DAT_143f10134`, a process global no
    /// save holds): each group a command forms or orders takes it as `mDifferentiatorID`, then it
    /// counts up. A group's load raises it past the group's (`FUN_14101d350`).
    pub commands: i32,
    /// The scratch array of a group's unit types (`DAT_143f23b70`, 50 words, a process global):
    /// each count of a sub-formation's unit types (`FUN_1410116c0`) writes its types from the
    /// front and copies the whole array into the group's `mUnitTypes`, so the words past the
    /// types are an earlier count's. Empty is all zero, as the process starts.
    pub unit_types_scratch: Vec<u32>,
}

impl UnitGroups {
    /// The long pather's list for a unit in group `group` (`FUN_140f9eb70`: `+0x6d0`, the group
    /// by id `FUN_14152ff40`, then `FUN_14100bbe0` with 255): None for no group (-1) or one the
    /// world does not hold.
    #[must_use]
    pub fn group_units(&self, group: i32) -> Option<Vec<i32>> {
        /// The long pather's list holds this many (`0xff`).
        const LOWERED: usize = 255;
        self.get(group)
            .map(|g| self.differentiator_units(g, LOWERED))
    }

    /// `FUN_14100bbe0` of group `group` with `cap`: the members of each group of its
    /// differentiator groups the world holds (by id), in order, at most `cap`.
    #[must_use]
    pub fn differentiator_units(&self, group: &UnitGroup, cap: usize) -> Vec<i32> {
        let mut out = Vec::new();
        for &id in &group.differentiator_groups {
            let Some(g) = self.get(id as i32) else {
                continue;
            };
            for m in &g.members {
                out.push(m.id);
                if cap <= out.len() {
                    return out;
                }
            }
        }
        out
    }

    /// A command ran the order splitter to its end (`DAT_143f10134` up by one): MOVE
    /// (`FUN_140e67800`) every time, INTERACT (`FUN_140e68b90`) with a unit and a single unit
    /// held, FOLLOW (`FUN_14161c920`) with its target, BUILD (`FUN_140e7a400`) past its
    /// placement tests, after the groups took the old count as their differentiator; the shelter
    /// garrison (`FUN_141621220`) once per group its splitter makes. WORK (2), PATROL (0x15,
    /// 0x21, 0x22, `FUN_14161b970`), `FORM_FORMATION` (0x17, `FUN_14161dba0`), command 0x1f
    /// (`FUN_140e67150`) and REPAIR (0x6e, `FUN_140e7da90`) count too; make-way
    /// (`FUN_140f88750`) would, but returns at once while `DAT_143f01836` is 0, as nothing
    /// writes it.
    pub const fn count_command(&mut self) {
        self.commands = self.commands.wrapping_add(1);
    }

    /// The group of id `id` (`FUN_14152ff40`): none for -1.
    #[must_use]
    pub fn get(&self, id: i32) -> Option<&UnitGroup> {
        if id == -1 {
            return None;
        }
        self.groups.iter().find(|g| g.id == id)
    }

    /// The group of id `id`, to change (`FUN_14152ff40`).
    pub fn get_mut(&mut self, id: i32) -> Option<&mut UnitGroup> {
        if id == -1 {
            return None;
        }
        self.groups.iter_mut().find(|g| g.id == id)
    }

    /// A new group at the world list's end with the next id (`FUN_14152efc0` with 1: the
    /// constructor, `FUN_141039640`'s id, then the count and the counter up by one); its state
    /// is the constructor's ([`UnitGroup::new`]) and `owner`'s, which the order's handler sets
    /// (`+0xdc`).
    pub fn create(&mut self, owner: u32) -> &mut UnitGroup {
        let id = self.counter;
        self.counter += 1;
        let at = self.groups.len();
        let mut g = UnitGroup::new(id);
        g.owner = owner;
        self.groups.push(g);
        &mut self.groups[at]
    }
}

/// The group that every listed unit is in (`FUN_14152ece0`), from each unit's group link
/// (`group_of`, None for a unit the world does not hold, which is passed over): -1 for an empty
/// list, for a unit in no group, for two groups, and when the first unit is gone (the others
/// are then compared with no group).
pub fn common_group(ids: &[i32], group_of: impl Fn(i32) -> Option<i32>) -> i32 {
    let Some(&first) = ids.first() else {
        return -1;
    };
    let first_held = group_of(first).is_some();
    let mut common = -1;
    for &id in ids {
        let Some(group) = group_of(id) else {
            continue;
        };
        if group == -1 {
            return -1;
        }
        if id == first && first_held {
            common = group;
        } else if common != group {
            return -1;
        }
    }
    common
}

/// An object of the world's id table (`FUN_1415417a0`, `BaseWorld::object`): the objects of
/// every player's live, sleeping and doppelganger lists, which the player's add and removal
/// (`FUN_141668040`) keep in the table through the world's slots 16 and 17 (`FUN_14152cb60`,
/// `FUN_141546a90`). An object of a recyclable master (a missile, a ghost) has a negative id and
/// sits in the table's negative half (`world+0x178`, at the id's negation); one recycled out of
/// the game (`RGE_Static_Object::recycle_out_of_game`) leaves its player's list, so the table.
#[derive(Clone, Copy, Debug)]
pub(crate) enum WorldObject<'a> {
    /// An object of player `.0`'s live or sleeping list.
    Unit(usize, &'a crate::turn::Unit),
    /// A ghost of player `.0`'s doppelganger list (type 25, its master `243`).
    Ghost(usize, &'a crate::ghost::Ghost),
}

impl WorldObject<'_> {
    /// The player whose list holds it (`+0x138`).
    pub(crate) const fn player(&self) -> usize {
        match *self {
            Self::Unit(p, _) | Self::Ghost(p, _) => p,
        }
    }

    /// Its master's id (`+0xa0`'s): a ghost's is its player's [`crate::ghost::GHOST_MASTER`].
    pub(crate) const fn master_id(&self) -> i16 {
        match *self {
            Self::Unit(_, u) => u.life.master_id,
            Self::Ghost(..) => crate::ghost::GHOST_MASTER,
        }
    }

    /// Its id (`+0x8`).
    pub(crate) const fn id(&self) -> i32 {
        match *self {
            Self::Unit(_, u) => u.id,
            Self::Ghost(_, g) => g.id,
        }
    }

    /// Its state (`+0x640`): a ghost's own (2, then 7 and 8 once destroyed).
    pub(crate) const fn state(&self) -> u8 {
        match *self {
            Self::Unit(_, u) => u.life.state,
            Self::Ghost(_, g) => g.state,
        }
    }

    /// It is inside another object (`+0x2f0`, `FUN_141188da0`); a ghost never is.
    pub(crate) const fn inside(&self) -> bool {
        match *self {
            Self::Unit(_, u) => u.life.inside.is_some(),
            Self::Ghost(..) => false,
        }
    }

    /// Its place: x, y, z (`+0x3c8`, `+0x4c8`, `+0x5a8`).
    pub(crate) const fn position(&self) -> crate::fmath::Vec3 {
        match *self {
            Self::Unit(_, u) => u.position,
            Self::Ghost(_, g) => g.position,
        }
    }

    /// Whether it is a moving object (`RGE_Moving_Object` and its subclasses, types 30 and up,
    /// whose movement state the slice holds): their slot 0x270 is `FUN_141158940`, a static,
    /// animated or doppelganger object's `FUN_141071e30` (0).
    pub(crate) const fn moves(&self) -> bool {
        match *self {
            Self::Unit(_, u) => u.mover.is_some(),
            Self::Ghost(..) => false,
        }
    }

    /// Slot 0x648, its forward: a moving object's `+0x830` (`FUN_14106d500`), a static,
    /// animated or doppelganger object's (0, 0, 0) (`FUN_141071e10`).
    pub(crate) fn forward(&self) -> crate::fmath::Vec3 {
        match *self {
            Self::Unit(_, u) => u.mover.as_ref().map_or([0.0; 3], |m| m.forward),
            Self::Ghost(..) => [0.0; 3],
        }
    }

    /// Slot 0x520, where it last moved to as the game's vector (x, height, y): a moving
    /// object's `+0x824` ([`crate::group::motion::moved_vector`]), any other's place
    /// (`FUN_141196c10`, a ghost's too).
    pub(crate) fn moved_vector(&self) -> crate::fmath::Vec3 {
        match *self {
            Self::Unit(_, u) => motion::moved_vector(u),
            Self::Ghost(_, g) => [g.position[0], g.position[2], g.position[1]],
        }
    }
}

impl crate::turn::Turns {
    /// `FUN_1415417a0` of `id`: the object the world's table holds for it ([`WorldObject`]),
    /// none for `i32::MIN` (the routine's own test) and for an id no list holds. Only the
    /// negative half holds ghosts.
    pub(crate) fn world_object(&self, id: i32) -> Option<WorldObject<'_>> {
        if id == i32::MIN {
            return None;
        }
        if let Some((p, u)) = self.object(id) {
            return Some(WorldObject::Unit(p, u));
        }
        if id < 0 {
            for (p, player) in self.players.iter().enumerate() {
                if let Some(g) = player.ghosts.iter().find(|g| g.id == id) {
                    return Some(WorldObject::Ghost(p, g));
                }
            }
        }
        None
    }
}

/// A unit's sub-formation in its group (`FUN_141011300`, `Turns::unit_category` for a held
/// object), from its master's type, `hero_flag` (`+0x8d9`) and formation category
/// (`+0x8bc`): none below type 70 (unsigned); a combat master (70) with a hero flag of bit 0 or
/// 4 but not both 4 and 7 is 5; categories 1 to 4 are themselves, 0 and 5 are 5, 6 is 0, any
/// other (255) none ("Invalid formation category").
#[must_use]
pub const fn sub_formation(kind: u8, hero_flag: u8, formation_category: u8) -> Option<i32> {
    if kind < 70 {
        return None;
    }
    if kind == 70 && hero_flag & 0x11 != 0 && hero_flag & 0x90 != 0x90 {
        return Some(5);
    }
    match formation_category {
        0 | 5 => Some(5),
        c @ 1..=4 => Some(c as i32),
        6 => Some(0),
        _ => None,
    }
}

/// A group's motion (`BMotion`, read by `FUN_141046660`): where the group stands and faces, its
/// path and how fast it may move and turn.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Motion {
    /// `mPosition`: the point, then the forward, up and right vectors.
    pub position: [[f32; 3]; 4],
    /// `mNewPosition`, the same four vectors.
    pub new_position: [[f32; 3]; 4],
    /// `mVelocity`.
    pub velocity: [f32; 3],
    /// `mAcceleration`.
    pub acceleration: [f32; 3],
    /// `mAngularVelocity`.
    pub angular_velocity: [f32; 3],
    /// `mAngularAcceleration`.
    pub angular_acceleration: [f32; 3],
    /// `mTurnRadius`.
    pub turn_radius: f32,
    /// `mTurnRadiusSpeed`.
    pub turn_radius_speed: f32,
    /// `mMaximumYawPerSecondMoving`.
    pub maximum_yaw_per_second_moving: f32,
    /// `mStationaryYawRevolutionTime` (its 32 bits).
    pub stationary_yaw_revolution_time: u32,
    /// `mMaximumYawPerSecondStationary`.
    pub maximum_yaw_per_second_stationary: f32,
    /// The path levels (`numPathDatas`, `tempPathData`): the level the group walks first, each
    /// next one its parent (`+8`), the top level last, as a unit's (the save writes them top
    /// first).
    pub paths: Vec<PathData>,
    /// `mMotionFlags`.
    pub flags: u32,
    /// `mMaximumVelocityForward`.
    pub maximum_velocity_forward: f32,
    /// `mNumberWaitDelays`.
    pub number_wait_delays: i32,
    /// `+0xc4` (not saved): the group's lead unit (-1 from the constructor), which a formation's
    /// transform tests the slots' places with (`FUN_1410465d0`).
    pub lead: i32,
}

/// A member's own move (`mSpecificMoves[i]`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SpecificMove {
    /// `mActive`.
    pub active: u32,
    /// `mPathID`.
    pub path_id: i32,
    /// `mReason`.
    pub reason: i32,
    /// `mGoal`.
    pub goal: [f32; 3],
    /// `mTimer`.
    pub timer: f32,
    /// `mConsecutiveMoveFailures`.
    pub consecutive_move_failures: i32,
    /// `mDistance`.
    pub distance: f32,
    /// `mLastPosition`.
    pub last_position: [f32; 3],
}

/// A member (`mUnits[i]`, with its move and `mSecondsSinceAttacking[i]`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Member {
    /// The unit's id.
    pub id: i32,
    /// Its own move.
    pub specific_move: SpecificMove,
    /// `mSecondsSinceAttacking[i]`.
    pub seconds_since_attacking: f32,
}

/// A formation's slot (`mSlots[i]`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Slot {
    /// `mUnitID`.
    pub unit_id: i32,
    /// `mOffset`.
    pub offset: [f32; 3],
    /// `mTransformPosition`.
    pub transform_position: [f32; 3],
    /// `mTime`.
    pub time: u32,
    /// `mState`.
    pub state: i32,
    /// `mDepth`.
    pub depth: i32,
    /// `mDesiredTransformPosition`.
    pub desired_transform_position: [f32; 3],
    /// `mPositionFilled`.
    pub position_filled: u32,
    /// `mDesiredCategory`.
    pub desired_category: i32,
    /// `mActive`.
    pub active: u32,
    /// `mDesiredSubCategory`.
    pub desired_sub_category: i32,
    /// `+0x44` (not saved): the formation's transform took the slot.
    pub transformed: u32,
}

/// A formation's line (`FUN_14101b290`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FormationLine {
    /// `tempDepth`.
    pub depth: i32,
    /// `mLineIndices` (as many as `tempWidth`).
    pub indices: Vec<u32>,
    /// `mAttackRange`.
    pub attack_range: f32,
    /// `mDistanceBack`.
    pub distance_back: f32,
}

/// A formation (`BFormation`, read by `FUN_1410180b0`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Formation {
    /// `mType`.
    pub kind: i32,
    /// `mSlots` (as many as `mNumberSlots`).
    pub slots: Vec<Slot>,
    /// `mNumberTransformedSlots`.
    pub number_transformed_slots: i32,
    /// `mMaximumWidth`.
    pub maximum_width: i32,
    /// `mMaximumDepth`.
    pub maximum_depth: i32,
    /// `mAboutFaced`.
    pub about_faced: u32,
    /// The lines (`tempMaxLines` of them).
    pub lines: Vec<FormationLine>,
    /// `mTransformType`.
    pub transform_type: i32,
    /// `mNumberLines`.
    pub number_lines: i32,
    /// `mTransformOrientation`.
    pub transform_orientation: [f32; 3],
    /// The slots' array past `mNumberSlots` (not saved): its capacity (`+0x8`) beyond the slots
    /// in use, which a pass over the whole array reads too.
    pub spare_slots: Vec<Slot>,
}

/// A point of the stored positions (`mPoint`, `mForward`, `mUp`, `mRight`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LinePoint {
    /// `mPoint`.
    pub point: [f32; 3],
    /// `mForward`.
    pub forward: [f32; 3],
    /// `mUp`.
    pub up: [f32; 3],
    /// `mRight`.
    pub right: [f32; 3],
}

/// The group's stored positions (`mStoredPositions`, a point line, `FUN_14101bb60`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PointLine {
    /// The points (`mNumberPoints`).
    pub points: Vec<LinePoint>,
    /// `mLength`.
    pub length: f32,
    /// `mDesiredLength`.
    pub desired_length: f32,
}

impl PointLine {
    /// `FUN_14102bae0`: the points go; the length stays.
    pub fn clear_points(&mut self) {
        self.points.clear();
    }

    /// `AddPointAtFront` (`FUN_141008040`): `p` with its height 0 goes before the newest point;
    /// the first point's length is 0, any other adds its distance to the point after it. A line
    /// of two points or more longer than its desired length then drops its oldest points while
    /// it stays longer without the last stretch.
    pub fn add_at_front(&mut self, mut p: LinePoint) {
        p.point[1] = 0.0;
        self.points.insert(0, p);
        if self.points.len() == 1 {
            self.length = 0.0;
        } else {
            self.length += stretch(&self.points[0], &self.points[1]);
        }
        if self.desired_length < self.length && 1 < self.points.len() {
            let n = self.points.len();
            let mut d = stretch(&self.points[n - 1], &self.points[n - 2]);
            while self.desired_length < self.length - d {
                if self.points.len() < 2 {
                    break;
                }
                let _ = self.points.pop();
                self.length -= d;
                // one point left: the game's next pass breaks at once
                let n = self.points.len();
                if n < 2 {
                    break;
                }
                d = stretch(&self.points[n - 1], &self.points[n - 2]);
            }
        }
    }
}

/// The 3D distance between two stored points (`FUN_14087e560`, `FUN_14099ef70`).
fn stretch(a: &LinePoint, b: &LinePoint) -> f32 {
    let d = crate::fmath::sub(a.point, b.point);
    crate::fmath::sqrt(d[0] * d[0] + d[1] * d[1] + d[2] * d[2])
}

/// The members of one formation category (`FUN_141023570`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UnitTypeStatistic {
    /// `mFormationCategory`.
    pub formation_category: i32,
    /// `mUnits` (as many as `mNumberUnits`).
    pub units: Vec<i32>,
    /// `mUsed`.
    pub used: u32,
    /// `mAttackRange`.
    pub attack_range: f32,
    /// `mDesiredSpacing`.
    pub desired_spacing: f32,
    /// `mNumberUnitTypes`.
    pub number_unit_types: i32,
    /// `mUnitTypes`: the 200 bytes the save writes (`+0x38`), as 50 words.
    pub unit_types: Vec<u32>,
    /// `mUnitTypeCounts`: the 200 bytes the save writes (`+0x100`), as 50 words.
    pub unit_type_counts: Vec<u32>,
}

/// The group's target (`mCurrentTarget`, `FUN_14101c7d0`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TargetData {
    /// `mTarget`.
    pub target: i32,
    /// `mType`.
    pub kind: i32,
    /// `mOriginalPosition`.
    pub original_position: [f32; 3],
    /// `mRange`.
    pub range: f32,
    /// `mTargetRadius`.
    pub target_radius: f32,
    /// `mDisableRetargeting`.
    pub disable_retargeting: u8,
    /// `mCertainAction`.
    pub certain_action: i16,
}

/// An order of the group's queue (`newOrder`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GroupOrder {
    /// `mCommand`.
    pub command: i32,
    /// `mTargetID`.
    pub target_id: i32,
    /// `mParm`.
    pub parm: f32,
    /// `mPoint`.
    pub point: [f32; 3],
    /// `mExecuteTime`.
    pub execute_time: u32,
}

/// A command of the group's queue (`newCommand`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GroupCommand {
    /// `mCommand`.
    pub command: i32,
    /// `mTargetID`.
    pub target_id: i32,
    /// `mRange`.
    pub range: f32,
    /// `mPoint`.
    pub point: [f32; 3],
    /// `mApproved`.
    pub approved: u8,
    /// `mDisableRetargeting`.
    pub disable_retargeting: u8,
    /// The 16 bits the save writes after them (read as `mApproved` again).
    pub approved_2: u16,
}

/// A unit group (`BUnitGroup`, 0x5c0 bytes, constructed by `FUN_140fe1550`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UnitGroup {
    /// Its motion.
    pub motion: Motion,
    /// `mID` (`+0xd8`).
    pub id: i32,
    /// `mOwner` (its 32 bits).
    pub owner: u32,
    /// `mState`.
    pub state: i32,
    /// The members (`mNumberUnits` at `+0xe4`, `mUnits` at `+0xf0`).
    pub members: Vec<Member>,
    /// `mPreviousFormation`, when it has one.
    pub previous_formation: Option<Formation>,
    /// `mCurrentFormation`, when it has one.
    pub current_formation: Option<Formation>,
    /// The word the save writes after the formations (`garbage`).
    pub garbage: u32,
    /// `mFormationLockedType`.
    pub formation_locked_type: i32,
    /// `mGroupFlags`.
    pub flags: u32,
    /// `mCentroid`.
    pub centroid: [f32; 3],
    /// `mDesiredHeight`.
    pub desired_height: f32,
    /// `mLargestUnitRadius`.
    pub largest_unit_radius: f32,
    /// `mSmallestUnitRadius`.
    pub smallest_unit_radius: f32,
    /// `mDesiredSpacing`.
    pub desired_spacing: f32,
    /// `mSmallestUnitRange`.
    pub smallest_unit_range: f32,
    /// `mSmallestRangedUnitRange`.
    pub smallest_ranged_unit_range: f32,
    /// `mLargestUnitRange`.
    pub largest_unit_range: f32,
    /// `mLargestRangedUnitRange`.
    pub largest_ranged_unit_range: f32,
    /// `mStoredPositions`.
    pub stored_positions: PointLine,
    /// `mUnitTypes`: the members by formation category.
    pub unit_types: Vec<UnitTypeStatistic>,
    /// `mWaypoints`.
    pub waypoints: Vec<[f32; 3]>,
    /// `mCurrentCommand`.
    pub current_command: i32,
    /// `mCurrentTarget`.
    pub current_target: TargetData,
    /// `mAttackPlanType`.
    pub attack_plan_type: i32,
    /// `mIdleTimer`.
    pub idle_timer: u32,
    /// The order queue.
    pub order_queue: Vec<GroupOrder>,
    /// `mHibernating`.
    pub hibernating: u8,
    /// The command queue.
    pub command_queue: Vec<GroupCommand>,
    /// The objects the save lists after the queues (`objCount`, `objId`).
    pub objects: Vec<i32>,
    /// `mDifferentiatorID`.
    pub differentiator_id: i32,
    /// `mSecondsNotUnderAttack`.
    pub seconds_not_under_attack: f32,
    /// `mDifferentiatorGroups`.
    pub differentiator_groups: Vec<u32>,
    /// `mTotalSpacing`.
    pub total_spacing: f32,
    /// `mNonRetargetUnits`.
    pub non_retarget_units: Vec<u32>,
    /// The members' `mSecondsSinceAttacking` slots past the members (not saved): the array's
    /// capacity beyond its count, which the next members take in order (`AddUnit`'s
    /// `FUN_14102d710`).
    pub spare_seconds: Vec<f32>,
    /// `+0x13d` (not saved; the constructor's 1): the group's last command was not queued
    /// (`BUnitGroup::command` sets it), so its next move empties the command queue first;
    /// `checkQueuedOrders` (`FUN_14100d460`) clears it.
    pub replaces_queue: u8,
}

impl Motion {
    /// `BMotion`'s constructor (`FUN_141041850`): at the origin facing x (forward (1, 0, 0), up
    /// (0, 1, 0), right their cross product (0, 0, -1)), no speed, the turn limits at `f32::MAX`
    /// (the constructor's `0x7f7fffff`), one level of path data without a path (`FUN_140f8f670`),
    /// flags 3.
    #[must_use]
    pub fn new() -> Self {
        let frame = [[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0]];
        Self {
            position: frame,
            new_position: frame,
            turn_radius_speed: f32::MAX,
            maximum_yaw_per_second_moving: f32::MAX,
            maximum_yaw_per_second_stationary: f32::MAX,
            paths: vec![PathData::without_path()],
            flags: 3,
            lead: -1,
            ..Self::default()
        }
    }
}

impl UnitGroup {
    /// `BUnitGroup`'s constructor (`FUN_140fe1550` after [`Motion::new`]) with id `id`
    /// (`FUN_141039640`): the owner -1 (the 64-bit -1 over `+0xd8` and `+0xdc`, whose id the
    /// id's setter then writes; [`UnitGroups::create`] sets the owner after), state -1, no
    /// formation, flags `0x100`, no command (-1), no target (-1, type -1), differentiator -1,
    /// `+0x13d` 1 (the 16-bit `0x100` at `+0x13c`, `mHibernating` 0); every other field 0 or
    /// empty, the attack plan (`+0x3b0`, `FUN_140fd75c0`) too.
    ///
    /// The constructor's other values are no group state: `+0xc8` and `+0xd0` point at the
    /// group itself; `+0x1e0` (-1) is the category of the unit types list's sentinel node
    /// (`+0x1d0`), which every walk of the list stops at before reading; the -1 at `+0x3e8` and
    /// `+0x420` are the order and command queues' sentinels; `+0x470` (0x20) is the growth step
    /// of the array at `+0x458` (data, capacity `+0x460`, size `+0x468`), which sets its
    /// capacity only.
    #[must_use]
    pub fn new(id: i32) -> Self {
        Self {
            motion: Motion::new(),
            id,
            owner: u32::MAX,
            state: -1,
            flags: 0x100,
            current_command: -1,
            current_target: TargetData {
                target: -1,
                kind: -1,
                ..TargetData::default()
            },
            differentiator_id: -1,
            replaces_queue: 1,
            ..Self::default()
        }
    }

    /// `FUN_14102bb40`: the command queue empties (`+0x420`, its size `+0x450`); a hibernating
    /// group (`mHibernating`, `+0x13c`, which `commandMoveTo`'s queue path sets for a group
    /// without a command) wakes, with flags `0x4c000` when it was 1.
    pub fn clear_command_queue(&mut self) {
        self.command_queue.clear();
        if self.hibernating != 0 {
            if self.hibernating == 1 {
                self.flags |= 0x4_c000;
            }
            self.hibernating = 0;
        }
    }

    /// Whether unit `id` is a member (`FUN_1410112b0`).
    #[must_use]
    pub fn has(&self, id: i32) -> bool {
        self.members.iter().any(|m| m.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `BMotion`'s constructor (`FUN_141041850`): the 0x7f7fffff turn limits at `+0x9c`,
    /// `+0xa0`, `+0xa8`, zero speeds and accelerations, the lead (`+0xc4`) -1, flags 3, the
    /// frame at the origin facing x, one level without a path.
    #[test]
    fn a_new_motion_holds_the_constructor_s_values() {
        let m = Motion::new();
        assert_eq!(
            (
                m.turn_radius_speed,
                m.maximum_yaw_per_second_moving,
                m.maximum_yaw_per_second_stationary
            ),
            (f32::MAX, f32::MAX, f32::MAX)
        );
        assert_eq!((m.flags, m.lead, m.number_wait_delays), (3, -1, 0));
        assert_eq!((m.velocity, m.acceleration), ([0.0; 3], [0.0; 3]));
        let frame = [[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0]];
        assert_eq!((m.position, m.new_position), (frame, frame));
        assert_eq!(m.paths, vec![PathData::without_path()]);
    }

    /// `FUN_14152ece0` over units 1, 2 (group 7), 3 (group 8), 4 (no group), 9 (gone).
    #[test]
    fn the_common_group_needs_every_held_unit_in_one_group_and_the_first_held() {
        let group_of = |id| match id {
            1 | 2 => Some(7),
            3 => Some(8),
            4 => Some(-1),
            _ => None,
        };
        assert_eq!(common_group(&[1, 2], group_of), 7);
        assert_eq!(common_group(&[1, 9, 2], group_of), 7);
        assert_eq!(common_group(&[1, 3], group_of), -1);
        assert_eq!(common_group(&[1, 4], group_of), -1);
        // the first unit gone: the others are compared with no group
        assert_eq!(common_group(&[9, 1, 2], group_of), -1);
        assert_eq!(common_group(&[], group_of), -1);
    }

    fn member(id: i32) -> Member {
        Member {
            id,
            ..Member::default()
        }
    }

    /// `FUN_14100bbe0`: group 1's differentiator groups 1, 2 and 9 (gone) list 10, 11 and 12, the
    /// gone one nothing; the cap stops the list.
    #[test]
    fn the_differentiator_units_are_the_members_of_the_listed_groups() {
        let mut one = UnitGroup::new(1);
        one.members = vec![member(10), member(11)];
        one.differentiator_groups = vec![1, 2, 9];
        let mut two = UnitGroup::new(2);
        two.members = vec![member(12)];
        let groups = UnitGroups {
            groups: vec![one.clone(), two],
            ..UnitGroups::default()
        };
        assert_eq!(groups.differentiator_units(&one, 255), [10, 11, 12]);
        assert_eq!(groups.differentiator_units(&one, 2), [10, 11]);
        let alone = UnitGroup::new(3);
        assert_eq!(groups.differentiator_units(&alone, 255), Vec::<i32>::new());
    }

    /// `AddPointAtFront` (`FUN_141008040`) on a line of desired length 2: the newest point goes
    /// first with its height 0, the length grows by each stretch, and past the desired length the
    /// oldest points go while the line without its last stretch stays longer.
    /// `FUN_14102bae0`: the points go, the lengths stay.
    #[test]
    fn clearing_a_lines_points_keeps_its_lengths() {
        let mut line = PointLine {
            points: vec![LinePoint::default(); 3],
            length: 2.5,
            desired_length: 4.0,
        };
        line.clear_points();
        assert_eq!(
            line,
            PointLine {
                points: Vec::new(),
                length: 2.5,
                desired_length: 4.0,
            }
        );
    }

    #[test]
    fn the_stored_positions_keep_their_desired_length() {
        let at = |x: f32, h: f32| LinePoint {
            point: [x, h, 0.0],
            ..LinePoint::default()
        };
        let mut line = PointLine {
            desired_length: 2.0,
            ..PointLine::default()
        };
        line.add_at_front(at(0.0, 0.0));
        assert_eq!((line.points.len(), line.length), (1, 0.0));
        line.add_at_front(at(1.0, 5.0));
        assert_eq!(line.points[0].point, [1.0, 0.0, 0.0]);
        assert_eq!(line.length, 1.0);
        line.add_at_front(at(2.0, 0.0));
        // 3 is longer than 2, but not without its last stretch (2): all stay
        line.add_at_front(at(3.0, 0.0));
        assert_eq!((line.points.len(), line.length), (4, 3.0));
        // 4 less 1 is 3: the oldest goes; 3 less 1 is 2: the rest stay
        line.add_at_front(at(4.0, 0.0));
        let xs: Vec<f32> = line.points.iter().map(|p| p.point[0]).collect();
        assert_eq!(xs, [4.0, 3.0, 2.0, 1.0]);
        assert_eq!(line.length, 3.0);
    }

    #[test]
    fn a_unit_s_sub_formation_follows_its_category_and_its_hero_flag() {
        assert_eq!(sub_formation(60, 0, 3), None);
        assert_eq!(sub_formation(70, 0, 3), Some(3));
        assert_eq!(sub_formation(70, 0, 0), Some(5));
        assert_eq!(sub_formation(70, 0, 6), Some(0));
        assert_eq!(sub_formation(70, 0, 255), None);
        assert_eq!(sub_formation(70, 1, 3), Some(5));
        assert_eq!(sub_formation(70, 0x90, 3), Some(3));
        assert_eq!(sub_formation(80, 1, 3), Some(3));
    }

    /// `FUN_140fe1550`: every value the constructor writes that the group holds.
    #[test]
    fn a_new_group_holds_the_constructor_s_values() {
        let g = UnitGroup::new(7);
        assert_eq!((g.id, g.owner, g.state), (7, u32::MAX, -1));
        assert_eq!(
            (g.flags, g.current_command, g.differentiator_id),
            (0x100, -1, -1)
        );
        assert_eq!((g.current_target.target, g.current_target.kind), (-1, -1));
        assert_eq!(
            (
                g.hibernating,
                g.replaces_queue,
                g.attack_plan_type,
                g.idle_timer
            ),
            (0, 1, 0, 0)
        );
        assert_eq!((g.previous_formation, g.current_formation), (None, None));
        assert!(g.members.is_empty() && g.unit_types.is_empty() && g.command_queue.is_empty());
        assert_eq!((g.formation_locked_type, g.desired_spacing), (0, 0.0));
        assert_eq!(g.motion, Motion::new());
    }

    #[test]
    fn a_new_group_takes_the_next_id_at_the_list_s_end() {
        let mut groups = UnitGroups {
            counter: 4,
            ..UnitGroups::default()
        };
        assert_eq!(groups.create(2).id, 4);
        assert_eq!(groups.create(1).id, 5);
        assert_eq!(groups.counter, 6);
        assert_eq!(
            groups
                .groups
                .iter()
                .map(|g| (g.id, g.owner))
                .collect::<Vec<_>>(),
            [(4, 2), (5, 1)]
        );
    }
}
