use super::*;
use crate::objects::ObjectId;
use crate::objects::masters::class;
use crate::objects::unit::Unit;
use crate::world::World;

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

/// `FUN_141017590`: a member's own move's reason (`+0xf8`'s record at the member's index,
/// its `+8`), whatever its place in the list; −1 for a unit that is no member.
#[test]
fn a_member_s_reason_is_its_own_move_s() {
    let mut group = UnitGroup::new(1);
    let with_reason = |id, reason| Member {
        id,
        specific_move: SpecificMove {
            reason,
            ..SpecificMove::default()
        },
        ..Member::default()
    };
    group.members = vec![with_reason(10, 2), with_reason(11, 0)];
    assert_eq!(group.member_reason(10), 2);
    assert_eq!(group.member_reason(11), 0);
    assert_eq!(group.member_reason(12), -1);
}

/// `FUN_1410177f0`: the first slot of the current formation (`+0x128`) that holds the unit
/// decides: in place when its state (`+0x2c`) is 3. A unit without a slot, and a group without
/// a current formation, is not in place; the previous formation is not read.
#[test]
fn a_unit_is_in_place_when_its_first_slot_in_the_current_formation_is() {
    let slot = |unit_id, state| Slot {
        unit_id,
        state,
        ..Slot::default()
    };
    let mut group = UnitGroup::new(1);
    group.previous_formation = Some(Formation {
        slots: vec![slot(10, 3)],
        ..Formation::default()
    });
    assert!(!group.slot_in_place(10));
    group.current_formation = Some(Formation {
        slots: vec![slot(11, 3), slot(10, 2), slot(10, 3)],
        ..Formation::default()
    });
    assert!(group.slot_in_place(11));
    assert!(!group.slot_in_place(10));
    assert!(!group.slot_in_place(12));
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

/// A world of player 1's unit 11 (master 83, a villager: type 70, class 4, swap group 1, state 2,
/// 25 hit points) and a building 60 (master 70, state 0) of player `owner`, the masters
/// `masters` for both players, player 1's attribute 90 at `owner_range`.
fn tasked_world(masters: Vec<crate::objects::masters::Master>, owner_range: f32) -> World {
    use crate::objects::lifecycle::Lifecycle;
    let unit = |id, master, state, hp| Unit {
        id,
        life: Lifecycle {
            master_id: master,
            state,
            hp,
            ..Lifecycle::default()
        },
        ..Unit::default()
    };
    let mut attributes = vec![0.0; 100];
    attributes[90] = owner_range;
    let mut t = World::new(
        0,
        1,
        0,
        vec![0, 0],
        vec![
            crate::world::Player::default(),
            crate::world::Player {
                objects: vec![unit(11, 83, 2, 25.0), unit(60, 70, 0, 550.0)].into(),
                attributes,
                ..crate::world::Player::default()
            },
        ],
    );
    t.masters = vec![
        crate::objects::masters::Masters::new([]),
        crate::objects::masters::Masters::new(masters),
    ];
    t
}

fn task_master(
    id: i16,
    tasks: Vec<crate::objects::masters::Task>,
) -> crate::objects::masters::Master {
    crate::objects::masters::Master {
        id,
        kind: 70,
        class: 4,
        task_by_group: 1,
        hit_points: 25,
        tasks,
        ..crate::objects::masters::Master::default()
    }
}

fn building_master() -> crate::objects::masters::Master {
    crate::objects::masters::Master {
        id: 70,
        kind: 80,
        class: 3,
        hit_points: 550,
        ..crate::objects::masters::Master::default()
    }
}

fn plain_task(action_type: i16, range: f32) -> crate::objects::masters::Task {
    crate::objects::masters::Task {
        action_type,
        work_range: range,
        work_val1: 1.0,
        ..crate::objects::masters::Task::default()
    }
}

fn query_on(target: i32) -> task::TaskQuery {
    task::TaskQuery {
        target: ObjectId::try_from(target).unwrap(),
        point: [20.0, 20.0],
        filter: -1,
        queued: false,
        direct: true,
    }
}

fn member_11() -> ObjectId {
    ObjectId::try_from(11).unwrap()
}

/// `FUN_14106b8b0`: the unit's own master's task is the command, with the task's range.
#[test]
fn a_member_s_own_task_is_the_command_with_the_task_s_range() {
    let t = tasked_world(
        vec![
            task_master(83, vec![plain_task(101, 2.5)]),
            building_master(),
        ],
        9.0,
    );
    let got = t.member_task(member_11(), &query_on(60)).unwrap();
    assert_eq!(
        got,
        Some(task::TargetTask {
            command: 101,
            range: Some(2.5)
        })
    );
}

/// The slot answers none (-1) for a target that is the member itself.
#[test]
fn a_member_has_no_task_on_itself() {
    let t = tasked_world(
        vec![
            task_master(83, vec![plain_task(101, 2.5)]),
            building_master(),
        ],
        9.0,
    );
    assert_eq!(t.member_task(member_11(), &query_on(11)).unwrap(), None);
}

/// Task 0x69's range is the target owner's attribute 90; task 0x68's is the unit's slot 0x230,
/// which the command never reads (none here); any other task's is its own. (The finder does not
/// find 0x68 and 0x69 yet, so the answer is built from a task given.)
#[test]
fn the_ranges_of_tasks_0x69_and_0x68_do_not_come_from_the_task() {
    let t = tasked_world(vec![task_master(83, vec![]), building_master()], 9.0);
    let target = t.task_target(60, 1).unwrap();
    let range = |action| {
        t.task_range(&plain_task(action, 2.5), &target)
            .map(|g| (g.command, g.range))
    };
    assert_eq!(range(0x69).unwrap(), (0x69, Some(9.0)));
    assert_eq!(range(0x68).unwrap(), (0x68, None));
    assert_eq!(range(5).unwrap(), (5, Some(2.5)));
}

/// A master without a task for the target takes the first master of its class, type and swap
/// group that has one, searched with the same filter.
#[test]
fn a_member_without_a_task_takes_the_swap_group_s_first_with_one() {
    let t = tasked_world(
        vec![
            task_master(83, vec![]),
            task_master(118, vec![plain_task(101, 1.5)]),
            task_master(119, vec![plain_task(101, 1.0)]),
            building_master(),
        ],
        9.0,
    );
    let got = t.member_task(member_11(), &query_on(60)).unwrap();
    assert_eq!(got.map(|g| (g.command, g.range)), Some((101, Some(1.5))));
}

/// A master of a swap group 0 does not search the others; none found is none.
#[test]
fn a_master_without_a_swap_group_has_no_other_to_try() {
    let mut own = task_master(83, vec![]);
    own.task_by_group = 0;
    let t = tasked_world(
        vec![
            own,
            task_master(118, vec![plain_task(101, 1.5)]),
            building_master(),
        ],
        9.0,
    );
    assert_eq!(t.member_task(member_11(), &query_on(60)).unwrap(), None);
}

/// A master of type 0x28 or above that holds a board task (0x84) asks the player's commands
/// when it finds no task, which is not modelled; a type below does not.
#[test]
fn a_master_with_a_board_task_and_no_task_found_is_not_modelled() {
    let mut own = task_master(83, vec![plain_task(0x84, 1.0)]);
    own.task_by_group = 0;
    // its own list finds the board task for any target, so give it one for another object
    own.tasks[0].object_id = 999;
    let t = tasked_world(vec![own.clone(), building_master()], 9.0);
    assert!(t.member_task(member_11(), &query_on(60)).is_err());
    own.kind = 0x27;
    let t = tasked_world(vec![own, building_master()], 9.0);
    assert_eq!(t.member_task(member_11(), &query_on(60)).unwrap(), None);
}

/// Player 1's group 1 of the members `members` (`(id, master, x, y)`, 25 hit points, state 2) and
/// player 2's building 60 (master 70, type 80, class 3, radii 2, attackable, state 2) at
/// (40, 45); master 4 is a combat unit of weapon range 4, 5 one of class 46 and 329 a listed
/// master. The group's top speed is 1 and its smallest unit range 2.5.
fn attack_world(members: &[(i32, i16, f32, f32)]) -> World {
    use crate::objects::lifecycle::Lifecycle;
    use crate::objects::masters::{Master, Masters};
    let objects: Vec<Unit> = members
        .iter()
        .map(|&(id, master, x, y)| Unit {
            id,
            body: crate::objects::layers::Body::at([x, y, 0.0]),
            group: GroupLink { id: 1, flag: -2 },
            life: Lifecycle {
                master_id: master,
                state: 2,
                hp: 25.0,
                ..Lifecycle::default()
            },
            ..Unit::default()
        })
        .collect();
    let target = Unit {
        id: 60,
        body: crate::objects::layers::Body::at([40.0, 45.0, 0.0]),
        attackable: true,
        life: Lifecycle {
            master_id: 70,
            state: 2,
            hp: 550.0,
            ..Lifecycle::default()
        },
        ..Unit::default()
    };
    let mut t = World::new(
        50,
        0,
        0,
        vec![0, 0, 0],
        vec![
            crate::world::Player::default(),
            crate::world::Player {
                objects: objects.into(),
                ..crate::world::Player::default()
            },
            crate::world::Player {
                objects: vec![target].into(),
                ..crate::world::Player::default()
            },
        ],
    );
    let soldier = |id, class| Master {
        id,
        kind: 70,
        class,
        formation_category: 1,
        weapon_range: 4.0,
        radius: [0.3, 0.3],
        ..Master::default()
    };
    let building = Master {
        radius: [2.0, 2.0],
        ..building_master()
    };
    let masters = Masters::new([
        soldier(4, 6),
        soldier(5, class::CLASS_46),
        soldier(329, 6),
        building,
    ]);
    t.masters = vec![Masters::new([]), masters.clone(), masters];
    let mut g = UnitGroup::new(1);
    g.owner = 1;
    g.members = members
        .iter()
        .map(|&(id, ..)| Member {
            id,
            ..Member::default()
        })
        .collect();
    g.motion.flags = 2;
    g.motion.maximum_velocity_forward = 1.0;
    g.smallest_unit_range = 2.5;
    t.groups = UnitGroups {
        groups: vec![g],
        ..UnitGroups::default()
    };
    t
}

fn attack_on(filter: i16) -> attack::AttackOrder {
    attack::AttackOrder {
        disable_retargeting: false,
        filter,
    }
}

fn building_60() -> ObjectId {
    ObjectId::try_from(60).unwrap()
}

/// `FUN_14100f7f0`: a group commanded on a target out of its members' reach resets the target's
/// data, takes the largest weapon range as its range, then sets the target (kind 1, the place the
/// target last moved to as x, height, y, the range, the radii's length, the order's bytes) and
/// moves to it: command 9, flags bit 3 off and bit 4 on.
#[test]
fn a_group_commanded_on_a_target_out_of_reach_moves_to_it() {
    let mut t = attack_world(&[(7, 4, 10.0, 10.0), (8, 4, 11.0, 10.0)]);
    assert!(
        t.command_attack(1, building_60(), attack_on(0x1234))
            .unwrap()
    );
    let g = t.groups.get(1).unwrap();
    let radius = crate::math::fmath::sqrt(8.0);
    assert_eq!(
        g.current_target,
        TargetData {
            target: 60,
            kind: 1,
            original_position: [40.0, 0.0, 45.0],
            range: 4.0,
            target_radius: radius,
            disable_retargeting: 0,
            certain_action: 0x1234,
        }
    );
    assert_eq!(g.current_command, 9);
    assert_eq!((g.flags & 8, g.flags & 0x10), (0, 0x10));
    assert!(
        g.members
            .iter()
            .all(|m| m.seconds_since_attacking == f32::MAX)
    );
}

/// A group already on the target whose members all work on it, without flags bit 17, only notes
/// where the target is (false); with bit 17, or a member on another target, it commands again.
#[test]
fn a_group_already_on_its_target_only_notes_the_place() {
    let mut t = attack_world(&[(7, 4, 10.0, 10.0)]);
    assert!(t.command_attack(1, building_60(), attack_on(-1)).unwrap());
    t.players[2].objects[0].body = crate::objects::layers::Body::at([41.0, 45.0, 0.0]);
    assert!(!t.command_attack(1, building_60(), attack_on(-1)).unwrap());
    assert_eq!(
        t.groups.get(1).unwrap().current_target.original_position,
        [41.0, 0.0, 45.0]
    );
    t.groups.get_mut(1).unwrap().flags |= 0x2_0000;
    assert!(t.command_attack(1, building_60(), attack_on(-1)).unwrap());
}

/// With two commands queued the members' pass does not run: the range is the group's smallest unit
/// range, and a member within reach attacks nothing here.
#[test]
fn a_group_with_two_queued_commands_takes_its_smallest_unit_range() {
    let mut t = attack_world(&[(7, 4, 38.0, 45.0)]);
    t.groups.get_mut(1).unwrap().command_queue =
        vec![GroupCommand::default(), GroupCommand::default()];
    assert!(t.command_attack(1, building_60(), attack_on(-1)).unwrap());
    assert_eq!(t.groups.get(1).unwrap().current_target.range, 2.5);
}

/// `FUN_14102bbd0`: a member within its weapon's reach of the target and not nearer than its
/// minimum range is not modelled; the pass raises the range to the largest a member works at.
#[test]
fn the_reach_pass_raises_the_range_and_stops_at_a_member_in_reach() {
    let t = attack_world(&[(7, 4, 10.0, 10.0), (8, 4, 38.0, 45.0)]);
    let mut range = -1.0;
    assert!(
        t.reach_pass(1, 60, attack_on(-1), &mut range).is_err(),
        "member 8 is at the target"
    );
    assert_eq!(range, 4.0);
    let t = attack_world(&[(7, 4, 10.0, 10.0)]);
    let mut range = -1.0;
    t.reach_pass(1, 60, attack_on(-1), &mut range).unwrap();
    assert_eq!(range, 4.0);
}

/// `FUN_1415cd0d0`: no filter is -1; a filter takes the range of the master's task of that
/// action type (the conversion's adds attribute 280); no such task -1; a master the player does
/// not have 0.
#[test]
fn a_filter_takes_the_range_of_the_members_task_of_that_type() {
    let mut t = attack_world(&[(7, 4, 10.0, 10.0)]);
    for master in t.masters.iter_mut().filter_map(|m| m.get_mut(4)) {
        master.tasks = vec![plain_task(5, 6.0), plain_task(0x68, 1.5)];
    }
    t.players[1].attributes = vec![0.0; 300];
    t.players[1].attributes[280] = 2.0;
    assert_eq!(t.task_reach(1, 4, -1).unwrap(), -1.0);
    assert_eq!(t.task_reach(1, 4, 5).unwrap(), 6.0);
    assert_eq!(t.task_reach(1, 4, 0x68).unwrap(), 3.5);
    assert_eq!(t.task_reach(1, 4, 9).unwrap(), -1.0);
    assert_eq!(t.task_reach(1, 99, 5).unwrap(), 0.0);
    // the range is the task's with the filter: the pass raises the group's range to it
    let mut range = -1.0;
    t.reach_pass(1, 60, attack_on(5), &mut range).unwrap();
    assert_eq!(range, 6.0);
}

/// `FUN_14100c6c0`: a group of fewer than five members lists one special unit, of five or more
/// four. A target of class 24 (and 57, 12, 47) takes the listed masters' members then the class
/// 46 ones; a target of master 6 or 7 the class 46 ones only; a target of a villager's class the
/// units by formation category; any other target none.
#[test]
fn the_special_units_follow_the_target_s_class() {
    let units = [
        (7, 4, 0.0, 0.0),
        (8, 5, 0.0, 0.0),
        (9, 329, 0.0, 0.0),
        (10, 329, 0.0, 0.0),
        (11, 5, 0.0, 0.0),
    ];
    let special = |target_class: i16, target_master: i16, take: usize| {
        let mut t = attack_world(&units[..take]);
        for b in t.masters.iter_mut().filter_map(|m| m.get_mut(70)) {
            b.class = target_class;
        }
        let _ = target_master;
        t.refill_special_units(1, 60).unwrap();
        t.groups.get(1).unwrap().special_units.clone()
    };
    assert_eq!(special(class::CLASS_24, 70, 5), [9, 10, 8, 11]);
    assert_eq!(special(class::CLASS_24, 70, 4), [9]);
    assert_eq!(special(3, 70, 5), Vec::<i32>::new());
}

/// `FUN_141007ba0`: each formation category's attack range rises to the largest range its members
/// have against the target (never down); a category without units stays.
#[test]
fn the_categories_attack_ranges_rise_to_their_members_ranges() {
    let mut t = attack_world(&[(7, 4, 0.0, 0.0), (8, 4, 0.0, 0.0)]);
    t.groups.get_mut(1).unwrap().unit_types = vec![
        UnitTypeStatistic {
            formation_category: 1,
            units: vec![7, 8],
            attack_range: 1.0,
            ..UnitTypeStatistic::default()
        },
        UnitTypeStatistic {
            formation_category: 2,
            units: vec![7],
            attack_range: 9.0,
            ..UnitTypeStatistic::default()
        },
        UnitTypeStatistic {
            formation_category: 3,
            attack_range: 0.5,
            ..UnitTypeStatistic::default()
        },
    ];
    t.raise_attack_ranges(1, 60).unwrap();
    let ranges: Vec<f32> = t
        .groups
        .get(1)
        .unwrap()
        .unit_types
        .iter()
        .map(|s| s.attack_range)
        .collect();
    assert_eq!(ranges, [4.0, 9.0, 0.5]);
}

/// An attack task for a building of another player (task 7, owner type 2, class 3).
fn attack_task() -> crate::objects::masters::Task {
    crate::objects::masters::Task {
        action_type: 7,
        owner_type: 2,
        object_group: 3,
        work_range: 1.5,
        ..crate::objects::masters::Task::default()
    }
}

/// [`attack_world`] whose soldiers (master 4) have the attack task.
fn attack_world_with_task(members: &[(i32, i16, f32, f32)]) -> World {
    let mut t = attack_world(members);
    for master in t.masters.iter_mut().filter_map(|m| m.get_mut(4)) {
        master.tasks = vec![attack_task()];
    }
    t
}

fn command_args(queued: bool, direct: bool) -> CommandArgs {
    CommandArgs {
        queued,
        direct,
        disable_retargeting: false,
        filter: -1,
    }
}

/// `FUN_14100efd0`: a group's order on a building of another player runs the attack path (task 7):
/// the group moves to it in command 9 and the move stops the members.
#[test]
fn a_group_ordered_on_an_enemy_building_runs_the_attack_path() {
    let mut t = attack_world_with_task(&[(7, 4, 10.0, 10.0), (8, 4, 11.0, 10.0)]);
    let moved = t
        .group_command(1, 60, [40.0, 45.0], command_args(false, true))
        .unwrap();
    assert!(moved);
    let g = t.groups.get(1).unwrap();
    assert_eq!((g.current_command, g.current_target.target), (9, 60));
    assert_eq!(g.flags & 0x2_0000, 0);
}

/// A target the world does not hold is a null object for the command: the plain move.
#[test]
fn a_group_ordered_on_an_object_the_world_lacks_moves_to_the_point() {
    let mut t = attack_world_with_task(&[(7, 4, 10.0, 10.0)]);
    assert!(
        t.group_command(1, 99, [40.0, 45.0], command_args(false, true))
            .unwrap()
    );
    let g = t.groups.get(1).unwrap();
    assert_eq!((g.current_command, g.current_target.target), (1, -1));
}

/// A queued attack joins the command queue (approved with `direct`, its range the task's, the
/// retargeting byte and the filter's 16 bits kept) and nothing moves; a group without a command
/// that is told directly hibernates.
#[test]
fn a_queued_attack_joins_the_command_queue() {
    let mut t = attack_world_with_task(&[(7, 4, 10.0, 10.0)]);
    let args = CommandArgs {
        queued: true,
        direct: true,
        disable_retargeting: true,
        filter: -1,
    };
    assert!(!t.group_command(1, 60, [40.0, 45.0], args).unwrap());
    let g = t.groups.get(1).unwrap();
    assert_eq!(
        g.command_queue,
        [GroupCommand {
            command: 7,
            target_id: 60,
            range: 1.5,
            point: [40.0, 0.0, 45.0],
            approved: 1,
            disable_retargeting: 1,
            approved_2: 0xffff,
        }]
    );
    assert_eq!(
        (g.current_command, g.hibernating, g.replaces_queue),
        (-1, 1, 0)
    );
}

/// An order neither queued nor direct behind queued commands joins them unapproved; the queue is
/// approved and run: the first attack on the target (held) is the group's command and ends the run,
/// the second stays queued.
#[test]
fn an_indirect_attack_behind_queued_commands_runs_the_queue() {
    let mut t = attack_world_with_task(&[(7, 4, 10.0, 10.0)]);
    t.groups.get_mut(1).unwrap().command_queue = vec![GroupCommand {
        command: 7,
        target_id: 60,
        range: 1.5,
        point: [40.0, 0.0, 45.0],
        approved: 0,
        disable_retargeting: 0,
        approved_2: 0xffff,
    }];
    assert!(
        t.group_command(1, 60, [40.0, 45.0], command_args(false, false))
            .unwrap()
    );
    let g = t.groups.get(1).unwrap();
    assert_eq!(g.command_queue.len(), 1);
    assert_eq!((g.current_command, g.current_target.target), (9, 60));
}

/// `FUN_14100d460`: an approved queued attack on a target the world holds is the group's
/// command (`FUN_14100f7f0` with the node's retargeting byte and filter) and ends the run
/// without the move to the last command's point.
#[test]
fn a_queued_attack_on_a_held_target_ends_the_run_with_the_attack() {
    let mut t = attack_world(&[(7, 4, 10.0, 10.0)]);
    t.groups.get_mut(1).unwrap().command_queue = vec![GroupCommand {
        command: 7,
        target_id: 60,
        range: 1.5,
        point: [40.0, 0.0, 45.0],
        approved: 1,
        disable_retargeting: 1,
        approved_2: 0x0102,
    }];
    t.check_queued_orders(1).unwrap();
    let g = t.groups.get(1).unwrap();
    assert_eq!(g.command_queue, []);
    assert_eq!(g.current_command, 9);
    assert_eq!(
        (
            g.current_target.disable_retargeting,
            g.current_target.certain_action
        ),
        (1, 0x0102)
    );
}

/// `FUN_14100c6c0`: a target of master 6 or 7 takes the class 46 members only; a target of class 4
/// the members by formation category in the group's statistics (up to the limit); one of class 57
/// the masters' list first.
#[test]
fn the_special_units_of_masters_6_and_7_and_of_the_category_classes() {
    let units = [
        (7, 4, 0.0, 0.0),
        (8, 5, 0.0, 0.0),
        (9, 329, 0.0, 0.0),
        (10, 5, 0.0, 0.0),
        (11, 4, 0.0, 0.0),
    ];
    let world = |target_class: i16, target_master: i16| {
        let mut t = attack_world(&units);
        t.players[2].objects[0].life.master_id = target_master;
        for masters in &mut t.masters {
            let mut b = building_master();
            b.id = target_master;
            b.class = target_class;
            *masters = crate::objects::masters::Masters::new(
                masters.iter().filter(|m| m.id != 70).cloned().chain([b]),
            );
        }
        t
    };
    let mut t = world(3, 7);
    t.refill_special_units(1, 60).unwrap();
    assert_eq!(t.groups.get(1).unwrap().special_units, [8, 10]);
    let mut t = world(class::VILLAGER, 70);
    t.groups.get_mut(1).unwrap().unit_types = vec![
        UnitTypeStatistic {
            formation_category: 2,
            units: vec![9, 10],
            ..UnitTypeStatistic::default()
        },
        UnitTypeStatistic {
            formation_category: 1,
            units: vec![7, 8, 11],
            ..UnitTypeStatistic::default()
        },
    ];
    t.refill_special_units(1, 60).unwrap();
    // category 1 first, then 2, up to four
    assert_eq!(t.groups.get(1).unwrap().special_units, [7, 8, 11, 9]);
    let mut t = world(class::CLASS_57, 70);
    t.refill_special_units(1, 60).unwrap();
    assert_eq!(t.groups.get(1).unwrap().special_units, [9, 8, 10]);
}
