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
