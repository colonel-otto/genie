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

mod group_list;
mod objects;

pub use self::group_list::{GroupLink, UnitGroups, common_group};
pub(crate) use self::objects::WorldObject;
pub use self::objects::sub_formation;

mod command;
mod create;

/// What `commandMoveTo` is told besides the target and the point ([`crate::world::World::command_move_to`]).
pub use command::MoveTo as CommandMoveTo;
pub mod formation;
mod layout;
mod motion;
mod place;
mod stats;
mod steer;
mod update;

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
    let d = crate::math::fmath::sub(a.point, b.point);
    crate::math::fmath::sqrt(d[0] * d[0] + d[1] * d[1] + d[2] * d[2])
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
    /// The objects the save lists after the queues (`objCount`, `objId`): the units the group
    /// keeps apart (`+0x458`, size `+0x468`).
    pub objects: Vec<i32>,
    /// What the members' searches found (`+0x49c`, at most 70, its count `+0x5b4`; not saved):
    /// the statistics refill it ([`crate::world::World::statistics_only`]), and nothing else in
    /// the game reads or writes it.
    pub sighted: Vec<i32>,
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

    /// Unit `id`'s place among the members (`FUN_1410175f0`: its index in `+0xf0`), −1 when it is
    /// not one.
    #[must_use]
    pub fn index_of(&self, id: i32) -> i32 {
        self.members
            .iter()
            .position(|m| m.id == id)
            .and_then(|i| i32::try_from(i).ok())
            .unwrap_or(-1)
    }
}

#[cfg(test)]
mod tests;
