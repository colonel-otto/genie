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
    /// is the constructor's (`FUN_140fe1550`) and `owner`'s.
    pub fn create(&mut self, owner: u32) -> &mut UnitGroup {
        let id = self.counter;
        self.counter += 1;
        let at = self.groups.len();
        self.groups.push(UnitGroup {
            id,
            owner,
            ..UnitGroup::default()
        });
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

/// A unit's sub-formation in its group (`FUN_141011300`), from its master's type, `hero_flag`
/// (`+0x8d9`) and formation category (`+0x8bc`): none below type 70; a combat master (70) with
/// a hero flag of bit 0 or 4 but not both 4 and 7 is 5; categories 1 to 4 are themselves, 0 and
/// 5 are 5, 6 is 0, any other (255) none ("Invalid formation category"). The rams' rule (class
/// 13 of masters 1258, 35, 422 and 548 of a human, ordered at a building: 0) holds only while the
/// byte `DAT_143f01837` is 1, and no code writes it.
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
    /// The path levels (`numPathDatas`, `tempPathData`).
    pub paths: Vec<PathData>,
    /// `mMotionFlags`.
    pub flags: u32,
    /// `mMaximumVelocityForward`.
    pub maximum_velocity_forward: f32,
    /// `mNumberWaitDelays`.
    pub number_wait_delays: i32,
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
}

impl UnitGroup {
    /// Whether unit `id` is a member (`FUN_1410112b0`).
    #[must_use]
    pub fn has(&self, id: i32) -> bool {
        self.members.iter().any(|m| m.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
