use super::*;

/// Slot 0x280 (`FUN_141082d60`) passes only a target id above -1 the world holds: a
/// recycled object's negative id gives -1 like none.
#[test]
fn a_top_target_below_minus_one_is_none() {
    use crate::objects::unit::Unit;
    use crate::world::Player;
    let hunter = |target: i32| Unit {
        id: 5,
        hunt: Some(crate::gathering::hunt::tests::hunting(target)),
        ..Unit::default()
    };
    let held = |id: i32| Unit {
        id,
        ..Unit::default()
    };
    let player = Player {
        objects: vec![held(7), held(-3)],
        ..Player::default()
    };
    let t = Turns::new(0, 0, 0, vec![0, 0], vec![Player::default(), player]);
    assert_eq!(t.top_target_object(&hunter(7)), 7);
    assert_eq!(t.top_target_object(&hunter(-3)), -1);
    assert_eq!(t.top_target_object(&hunter(9)), -1);
    assert_eq!(t.top_target_object(&held(1)), -1);
}

/// `FUN_14104c150` turns only when 1e-7 is below the yaw's size (an ordered compare): a NaN
/// yaw leaves the vectors and the motion flags alone.
#[test]
fn a_nan_yaw_does_not_turn() {
    let mut m = Motion::new();
    let before = m.clone();
    m.turn(f32::NAN);
    assert_eq!(m.flags, before.flags);
    assert_eq!(
        format!("{:?}", m.new_position),
        format!("{:?}", before.new_position)
    );
}
use crate::groups::UnitGroups;
use crate::math::rng::Well512;
use crate::pathing::Pathing;

fn level(level: u8, points: &[[f32; 2]], waypoint: i32, state: i32) -> PathData {
    PathData {
        level,
        points: points.iter().map(|&[x, y]| [x, 0.0, y]).collect(),
        has_path: true,
        path_flags: NEW_PATH_FLAGS,
        waypoint,
        state,
        ..PathData::without_path()
    }
}

/// `FUN_141046600` and `FUN_141046640`: a motion moves with any velocity, and waits to move
/// while its walked level wants a path (state 1 or 2).
#[test]
fn a_motion_moves_with_a_velocity_and_waits_for_a_path() {
    let mut m = Motion::new();
    assert!(!m.moving() && !m.waits_to_move());
    m.velocity = [0.0, 0.0, -0.5];
    assert!(m.moving());
    m.paths = vec![level(0, &[], 0, 2)];
    assert!(m.waits_to_move());
    m.paths[0].state = 3;
    assert!(!m.waits_to_move());
}

/// `FUN_1410481f0`: levels 0 and 1 below the top (2) go and the top walks from waypoint 1
/// (state 2, 10 tries); a top of another level resets; a lone level stays.
#[test]
fn the_walked_levels_below_the_high_level_go() {
    let mut m = Motion::new();
    let mut top = level(2, &[[0.0, 0.0], [5.0, 5.0], [9.0, 9.0]], 2, 4);
    top.continue_counter = 3;
    m.paths = vec![
        level(0, &[[1.0, 1.0]], 1, 3),
        level(1, &[[2.0, 2.0]], 1, 0),
        top,
    ];
    m.drop_to_high_level();
    assert_eq!(m.paths.len(), 1);
    let l = &m.paths[0];
    assert_eq!(
        (l.level, l.waypoint, l.state, l.continue_counter),
        (2, 1, 2, 10)
    );
    assert_eq!(l.points.len(), 3);
    m.paths = vec![level(0, &[[1.0, 1.0]], 1, 3), level(1, &[[2.0, 2.0]], 1, 0)];
    m.paths[0].repath_time = 4.0;
    m.drop_to_high_level();
    assert_eq!(m.paths, [PathData::without_path()]);
    let lone = vec![level(0, &[[1.0, 1.0]], 1, 3)];
    m.paths.clone_from(&lone);
    m.drop_to_high_level();
    assert_eq!(m.paths, lone);
}

/// `FUN_140f9b4d0`: a bit on or off the walked level; bit 2 also goes on for its parents.
#[test]
fn a_levels_flag_and_its_parents_bit_2() {
    let mut paths = vec![
        level(0, &[], 0, 3),
        level(1, &[], 0, 0),
        level(2, &[], 0, 0),
    ];
    set_flag(&mut paths, 0, true);
    assert_eq!(paths[0].flags, 1);
    set_flag(&mut paths, 2, false);
    assert_eq!((paths[0].flags, paths[1].flags, paths[2].flags), (1, 4, 4));
    set_flag(&mut paths, 0, false);
    assert_eq!(paths[0].flags, 0);
    assert_eq!(paths[1].flags, 4);
}

/// `FUN_140f98e80`: the walked level and its parents go; the level keeps its repath time.
#[test]
fn a_reset_level_drops_its_parents_and_keeps_its_repath_time() {
    let mut paths = vec![level(0, &[[1.0, 1.0]], 1, 3), level(2, &[], 0, 0)];
    paths[0].repath_time = 2.5;
    reset_level(&mut paths);
    assert_eq!(paths.len(), 1);
    assert_eq!(
        paths[0],
        PathData {
            repath_time: 2.5,
            ..PathData::without_path()
        }
    );
}

/// `FUN_140f98bc0`: a loop goes round; a path back and forth turns at its ends.
#[test]
fn a_looping_path_goes_round_or_back_and_forth() {
    let mut l = level(1, &[[0.0, 0.0], [1.0, 0.0], [2.0, 0.0]], 0, 2);
    assert_eq!(looping_next(&mut l, 0), -1);
    l.path_flags = 1;
    assert_eq!(looping_next(&mut l, 1), 2);
    assert_eq!(looping_next(&mut l, 2), 0);
    l.path_flags = 1 | 2;
    assert_eq!(looping_next(&mut l, 1), 0);
    assert_eq!(looping_next(&mut l, 0), 1);
    assert_eq!(l.path_flags & 4, 4);
    assert_eq!(looping_next(&mut l, 1), 2);
    assert_eq!(looping_next(&mut l, 2), 1);
    assert_eq!(l.path_flags & 4, 0);
}

/// `FUN_141046130`: short of the waypoint, on it, or past it (or a step under 0.1).
#[test]
fn a_step_falls_short_of_reaches_or_passes_its_waypoint() {
    let w = [10.0, 0.0, 0.0];
    assert_eq!(passes_waypoint(w, [0.0; 3], [5.0, 0.0, 0.0]), 0);
    // a step that ends on the waypoint is not past it
    assert_eq!(passes_waypoint(w, [9.95, 0.0, 0.0], [10.0, 0.0, 0.0]), 0);
    // one past it by less than 0.1 is
    assert_eq!(passes_waypoint(w, [9.95, 0.0, 0.0], [10.02, 0.0, 0.0]), 2);
    assert_eq!(passes_waypoint(w, [9.0, 0.0, 0.0], [10.0, 0.0, 0.0]), 0);
    // past it, but no farther beyond than it was before: on it
    assert_eq!(passes_waypoint(w, [9.0, 0.0, 0.0], [10.5, 0.0, 0.0]), 1);
    assert_eq!(passes_waypoint(w, [9.0, 0.0, 0.0], [11.0, 0.0, 0.0]), 1);
    assert_eq!(passes_waypoint(w, [9.0, 0.0, 0.0], [12.0, 0.0, 0.0]), 2);
    // a NaN step is short (COMISS step, to the waypoint; JBE at 0x14104621d), where an
    // ordered compare would pass it on to the next tests
    assert_eq!(passes_waypoint(w, [9.0, 0.0, 0.0], [f32::NAN, 0.0, 0.0]), 0);
}

/// `FUN_1411570c0`'s ends held inside the map.
#[test]
fn a_querys_ends_stay_inside_the_map() {
    assert_eq!(hold([-1.0, 5.0, 3.0], [10, 20]), [0.0, 3.0]);
    assert_eq!(
        hold([10.0, 0.0, 25.0], [10, 20]),
        [10.0 - NEAR, 20.0 - NEAR]
    );
}

/// `calculateYawAngle` with its clamp: a quarter turn to the left is negative, and limited
/// by the yaw per second times dt; without the clamp (`param_5` 0) the whole angle.
#[test]
fn a_turn_is_limited_by_the_yaw_per_second() {
    let mut m = Motion::new();
    let (x, z) = ([1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
    let free = m.calculate_yaw_angle(x, z, 0.1, true);
    assert_eq!(free, -std::f32::consts::FRAC_PI_2);
    m.maximum_yaw_per_second_moving = 1.0;
    assert_eq!(m.calculate_yaw_angle(x, z, 0.1, true), -0.1);
    assert_eq!(m.calculate_yaw_angle(z, x, 0.1, true), 0.1);
    assert_eq!(
        m.calculate_yaw_angle(z, x, 0.1, false),
        std::f32::consts::FRAC_PI_2
    );
}

/// The clamp is `MINSS limit, angle` (0x1410463e2): a NaN limit leaves the angle, a NaN
/// angle stays NaN (an ordered `<=` would take the limit both times); a NaN cross product
/// keeps the sign (`COMISS`/`JBE`).
#[test]
fn the_yaw_clamp_keeps_the_angle_for_a_nan_operand() {
    let mut m = Motion::new();
    let (x, z) = ([1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
    m.maximum_yaw_per_second_moving = f32::NAN;
    assert_eq!(
        m.calculate_yaw_angle(x, z, 0.1, true),
        -std::f32::consts::FRAC_PI_2
    );
    m.maximum_yaw_per_second_moving = 1.0;
    let nan = m.calculate_yaw_angle([f32::NAN, 0.0, 0.0], z, 0.1, true);
    assert!(nan.is_nan(), "{nan}");
}

/// `FUN_14104c150` turns the new position's forward and right about its up; a turn of 1e-7
/// or less is none. The integration then moves the group along its forward and copies the
/// new position.
#[test]
fn the_motion_turns_then_moves_along_its_forward() {
    let mut m = Motion::new();
    m.flags = 0;
    m.turn(1e-8);
    assert_eq!((m.new_position, m.flags), (m.position, 0));
    m.turn(-std::f32::consts::FRAC_PI_2);
    assert_eq!(m.flags, 1);
    let f = m.new_position[1];
    assert!((f[2] - 1.0).abs() < 1e-6 && f[0].abs() < 1e-6, "{f:?}");
    m.flags = 0;
    m.velocity = [0.0, 0.0, 2.0];
    m.acceleration = [0.0, 0.0, 1.0];
    m.integrate(0.5);
    assert_eq!(m.velocity[2], 2.5);
    let p = m.position[0];
    assert!((p[2] - 1.25).abs() < 1e-6 && p[0].abs() < 1e-6, "{p:?}");
    assert_eq!(m.position, m.new_position);
    assert_eq!(m.flags, 1);
}

struct Fixture {
    spatial: Spatial,
    pathing: Pathing,
    object: Well512,
    groups: UnitGroups,
}

impl Fixture {
    fn new() -> Self {
        Self {
            spatial: Spatial::new([40, 40]),
            pathing: Pathing::default(),
            object: Well512::default(),
            groups: UnitGroups::default(),
        }
    }

    fn step(&mut self, m: &mut Motion, group: &GroupView, lead: &Lead) -> bool {
        let mut plan = Planning {
            pathing: &mut self.pathing,
            now: 100,
            computer: false,
            object: &mut self.object,
            construction_rate: 0.0,
            groups: &self.groups,
        };
        let mut world = World {
            spatial: &self.spatial,
            plan: &mut plan,
            dt: 0.1,
        };
        path_step(m, group, lead, &mut world, false).unwrap()
    }
}

fn lead() -> Lead {
    Lead {
        walker: Walker {
            id: 7,
            radius: 0.2,
            restriction: 0,
            mask: 0xff,
        },
        position: [5.0, 5.0, 0.0],
        radius: [0.2, 0.2],
        moves: true,
        solid: true,
        ignore_units: Vec::new(),
        player: 1,
        computer: false,
        group: None,
    }
}

/// A walked level heads for its waypoint: the group turns to it and takes its top speed.
#[test]
fn a_walked_level_turns_the_group_toward_its_waypoint() {
    let mut fx = Fixture::new();
    let mut m = Motion::new();
    m.position[0] = [5.0, 0.0, 5.0];
    m.new_position[0] = [5.0, 0.0, 5.0];
    m.maximum_velocity_forward = 1.2;
    m.paths = vec![level(0, &[[5.0, 5.0], [5.0, 9.0]], 1, 3)];
    let group = GroupView::default();
    assert!(fx.step(&mut m, &group, &lead()));
    assert_eq!(m.velocity[2], 1.2);
    let f = m.new_position[1];
    assert!((f[2] - 1.0).abs() < 1e-6 && f[0].abs() < 1e-6, "{f:?}");
}

/// A reached last waypoint of a top level without a parent arrives: the level resets and
/// the group stands.
#[test]
fn the_last_waypoint_of_the_top_level_ends_the_walk() {
    let mut fx = Fixture::new();
    let mut m = Motion::new();
    m.position[0] = [5.0, 0.0, 9.0];
    m.velocity = [0.0, 0.0, 1.2];
    let mut top = level(2, &[[5.0, 5.0], [5.0, 9.0]], 1, 3);
    top.flags = 1;
    m.paths = vec![top];
    assert!(fx.step(&mut m, &GroupView::default(), &lead()));
    assert_eq!(m.paths, vec![PathData::without_path()]);
    assert_eq!(m.velocity[2], 0.0);
}

/// An arrived walked level linked as found hands over to its parent, whose waypoint then
/// counts as reached; a pause makes the group stand until it ran out.
#[test]
fn an_arrived_level_hands_over_and_a_pause_waits() {
    let mut fx = Fixture::new();
    let mut m = Motion::new();
    m.position[0] = [5.0, 0.0, 9.0];
    let mut walked = level(0, &[[5.0, 5.0], [5.0, 9.0]], 1, 4);
    walked.linked_type = 1;
    let parent = level(2, &[[5.0, 5.0], [5.0, 9.0]], 1, 0);
    m.paths = vec![walked, parent];
    // the parent's reached last waypoint ends the walk in the same step
    assert!(fx.step(&mut m, &GroupView::default(), &lead()));
    assert_eq!(m.paths.len(), 1);
    assert_eq!(m.paths[0].state, 0);
    let mut m = Motion::new();
    m.velocity = [0.0, 0.0, 1.0];
    let mut paused = level(0, &[[5.0, 5.0], [5.0, 9.0]], 1, 3);
    paused.pause_time = 0.3;
    m.paths = vec![paused];
    assert!(!fx.step(&mut m, &GroupView::default(), &lead()));
    assert_eq!(m.velocity[2], 0.0);
    assert!((m.paths[0].pause_time - 0.2).abs() < 1e-6);
}
