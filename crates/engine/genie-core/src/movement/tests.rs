use super::*;
use crate::map::spatial::{Gait, Obstruction, Placed};
use crate::movement::move_to::Walk;
use crate::movement::request_plans::{NewPath, Way};

const DT: f32 = 0.02;

/// MarvinBlue/426048731's boar (master 48): a walking sprite of 16 facets at a turn speed of
/// 0.05 turns 2π / 0.8 a second, 11.25° in a tick of 0.025 s; a villager turns at once.
#[test]
fn a_master_s_turn_rate_comes_from_its_turn_speed_and_walking_facets() {
    let rate = turn_rate(0.05, Some(16));
    assert_eq!(rate, TWO_PI / (16.0f32 * 0.05));
    assert_eq!(rate.to_bits(), 7.853_982f32.to_bits());
    assert_eq!(turn_rate(0.0, Some(16)), f32::MAX);
    assert_eq!(turn_rate(0.05, None), f32::MAX);
}

/// `FUN_1410e3a70` writes all five turning values: a zero turn speed resets them, a walking
/// sprite sets the revolution and both rates, no sprite changes nothing.
#[test]
fn a_master_s_turning_values_follow_its_turn_speed_and_walking_sprite() {
    let before = Turning {
        radius: 1.0,
        radius_speed: 2.0,
        rate: 3.0,
        revolution: 4.0,
        revolution_rate: 5.0,
    };
    assert_eq!(turning(0.0, Some(16), before), (Turning::PRESET, false));
    assert_eq!(turning(0.05, None, before), (before, false));
    let (after, set) = turning(0.05, Some(16), before);
    assert!(set);
    let rate = TWO_PI / (16.0f32 * 0.05);
    assert_eq!(
        after,
        Turning {
            revolution: 16.0f32 * 0.05,
            revolution_rate: rate,
            rate,
            ..before
        }
    );
    // a sprite of no facets: a revolution of 0, the largest float
    assert_eq!(turning(0.05, Some(0), before).0.rate, f32::MAX);
}

fn stats() -> MoveStats {
    MoveStats {
        fly_mode: 0,
        speed: 0.8,
        turn_rate: f32::MAX,
        turn_radius: 0.0,
        turn_radius_speed: f32::MAX,
        radius: [0.2, 0.2, 0.5],
        restriction: 7,
        restriction_mask: 6,
        turn_time: 0.0,
        move_algorithm: 0,
        min_collision: 1.0,
        fast_turn: false,
        walk_speed: 1.0,
        stand_speed: 1.0,
        run_speed: 1.0,
        facets: Some(crate::objects::facing::SpriteFacets {
            count: 16,
            flag_1: true,
        }),
        weapon_range: 0.0,
    }
}

fn level(level: u8, points: &[[f32; 2]], state: i32) -> PathData {
    PathData {
        level,
        linked_type: i32::from(level == 0),
        points: points.iter().map(|&[x, y]| [x, 0.0, y]).collect(),
        has_path: true,
        path_flags: if level == 0 { 6 } else { 4 },
        waypoint: 1,
        state,
        range: 0.1,
        target: 10028,
        pause_time: 0.0,
        flags: 0,
        length: 0.0,
        created: 16360,
        continue_counter: 10,
        repath_time: 0.0,
        ignore: Vec::new(),
        ignored: Vec::new(),
    }
}

/// MarvinBlue/426048731's boar 9729 in update 4193: its Attack's `MoveTo` (`+0x61` clear)
/// walks to its path's end when its target, the hunter 10049, walks off; the walked level's
/// waypoint and the top path's end follow the hunter, in the paths' layout (x, z, y).
#[test]
fn a_path_s_end_follows_a_moved_target_in_the_paths_layout() {
    let mut m = builder_9177();
    m.paths[0].flags = 3;
    assert!(m.walks_to_path_end().unwrap());
    m.follow_path_end([99.515_114, 41.147_972, 0.0]);
    assert_eq!(m.paths[0].points[1], [99.515_114, 0.0, 41.147_972]);
    assert_eq!(m.paths[1].points[1], [99.515_114, 0.0, 41.147_972]);
    assert_eq!(m.paths[0].flags, 0);
    assert!(m.walks_to_path_end().unwrap());
}

/// MarvinBlue/426048731's builder 9177 as the game saved it after tick 679 (update 680): its
/// first path toward the house foundation 10028, facing along it, not yet moving.
pub(crate) fn builder_9177() -> Mover {
    Mover {
        forward: [0.894_427_2, 0.0, -0.447_213_62],
        right: [-0.447_213_62, 0.0, -0.894_427_2],
        velocity: Some(Velocity {
            velocity: [0.0; 3],
            acceleration: [0.0, 0.0, f32::MAX],
        }),
        paths: vec![
            level(0, &[[28.0, 89.0], [30.0, 88.0]], 3),
            level(2, &[[28.0, 89.0], [30.0, 88.0]], 0),
        ],
        factor: 1.0,
        stats: stats(),
        waiting: 0,
        wait_delays: 0,
        attack_moving: false,
        last_move: 0,
        moved_at: [28.0, 89.0, 0.0],
        substitute_count: 0,
        ignore_units: Vec::new(),
        user_waypoints: Vec::new(),
        last_turn: 0.0,
        angle: 0.0,
        move_to_point: None,
        speed_stopped: false,
        pauses: Vec::new(),
        in_the_way: Vec::new(),
        sprite_facets: None,
    }
}

/// `FUN_14114ece0`: the unit, its target, then its own list but itself when the walked
/// level's top path holds a path made this update.
#[test]
fn the_ignore_list_takes_the_own_list_when_the_top_path_is_this_update_s() {
    let world = world();
    let mut pathing = Pathing::default();
    let mut object = Well512::default();
    let groups = crate::groups::UnitGroups::default();
    let mut plan = Planning {
        pathing: &mut pathing,
        now: 16360,
        computer: false,
        object: &mut object,
        construction_rate: 1.0,
        groups: &groups,
    };
    let me = Me {
        id: 9177,
        position: [28.0, 89.0, 0.0],
        group: -1,
        action_state: 0,
    };
    let mut m = builder_9177();
    m.ignore_units = vec![5, 9177, 6];
    let house = *world.get(10028).unwrap();
    let line = ([28.0, 0.0], [30.0, 0.0]);
    let list = |m: &Mover, plan: &mut Planning<'_>| {
        m.ignore_list(&me, &world, plan, Some(&house), 10028, line)
    };
    assert_eq!(list(&m, &mut plan), Ok(vec![9177, 10028, 5, 6]));
    // a top level without a path, or a path of another update: none of the own list
    m.paths[1].has_path = false;
    assert_eq!(list(&m, &mut plan), Ok(vec![9177, 10028]));
    m.paths[1].has_path = true;
    m.paths[1].created = 16359;
    assert_eq!(list(&m, &mut plan), Ok(vec![9177, 10028]));
}

/// `FUN_140fa0110`: the drop-off walker's start test along its line, with its radius, its
/// weapon range and the target's radii: blocked deep in terrain its mask lacks, free on
/// terrain it has.
#[test]
fn the_drop_off_walker_s_start_is_blocked_in_terrain_its_mask_lacks() {
    let test = |class: u8| {
        let world = Spatial::with_tiles(
            [16, 16],
            crate::map::spatial::Tiles::new(16, 16, vec![class; 256]),
        );
        let mut pathing = Pathing::default();
        pathing.start_mesh(16);
        let mut object = Well512::default();
        let groups = crate::groups::UnitGroups::default();
        let mut plan = Planning {
            pathing: &mut pathing,
            now: 0,
            computer: false,
            object: &mut object,
            construction_rate: 1.0,
            groups: &groups,
        };
        let mut target = world_target();
        target.position = [12.5, 5.5, 0.0];
        let m = builder_9177();
        m.start_blocked(&world, &mut plan, &target, ([5.5, 5.5], [12.5, 5.5]))
    };
    // class 1 lies outside the villager's mask (6): every row of tiles is a shape
    assert_eq!(test(1), Ok(true));
    assert_eq!(test(2), Ok(false));
}

/// A 20 × 20 map whose tiles (6, 4..=6) are of a terrain the mask 6 lacks (a wall), with the
/// map's links, and player 2's unit 9171 of radius `radius` at (4.5, 5.5).
fn walled(radius: f32) -> Spatial {
    let mut classes = vec![4; 400];
    for y in 4..=6 {
        classes[y * 20 + 6] = 1;
    }
    let mut w = Spatial::with_tiles([20, 20], crate::map::spatial::Tiles::new(20, 20, classes));
    w.sight = Some(crate::vision::visibility::Visibility::new(
        20,
        20,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        0,
        Vec::new(),
    ));
    w.insert(
        9171,
        Placed {
            position: [4.5, 5.5, 0.0],
            radius: [radius, radius],
            class: 4,
            player: 2,
            master: 0,
            obstruction_type: 5,
            buildable_on: 0,
            tall: true,
            speed: 0.0,
            unknown: None,
            obstruction: Some(Obstruction { kind: 5, class: 2 }),
        },
    );
    w
}

/// `FUN_140f9e540`'s dispatch: `COMISS 0.5, radius; JC` (0x140f9e71f): a radius above half a
/// tile, or an unordered one, takes the old pather.
#[test]
fn a_radius_above_half_a_tile_or_unordered_takes_the_old_pather() {
    assert!(!old_pather_for(0.2));
    assert!(!old_pather_for(0.5));
    assert!(old_pather_for(f32::from_bits(0.5f32.to_bits() + 1)));
    assert!(old_pather_for(f32::NAN));
}

/// `FUN_140f9e540` for a unit wider than half a tile: the path empties, then the old pather
/// (`FUN_140f89e40` with the caller's ignore list, `FUN_140f85580` with the include list and
/// the owner's number) writes it, its waypoints at height 0: here around the wall to the
/// point (its area, grown once, holds 96 tiles). A unit of half a tile takes the new pather,
/// which needs the game's mesh.
#[test]
fn a_big_unit_s_short_path_takes_the_old_pather() {
    let world = walled(0.6);
    let mut pathing = Pathing::default();
    let mut object = Well512::default();
    let groups = crate::groups::UnitGroups::default();
    let mut plan = Planning {
        pathing: &mut pathing,
        now: 100,
        computer: false,
        object: &mut object,
        construction_rate: 1.0,
        groups: &groups,
    };
    let walker = Walker {
        id: 9171,
        radius: 0.6,
        restriction: 0,
        mask: 6,
    };
    let mut out = Path::default();
    let _ = out.push([1.0, 0.0, 1.0]);
    let goal = ([8.5, 0.0, 5.5], 0.0, -1);
    let r = short_path_for(
        walker,
        [4.5, 5.5],
        &world,
        &mut plan,
        goal,
        &[9171],
        &mut out,
    );
    let mut pather = HullPather::for_list(&world, &[9171], 6, 0.6, 2);
    let (result, path) = pather.find([4.5, 5.5], [8.5, 5.5], -1, 0.0).unwrap();
    assert_eq!((r, result), (Ok(1), 1), "{path:?}");
    let points: Vec<Vec3> = path.points.iter().map(|&[x, y]| [x, 0.0, y]).collect();
    assert_eq!(out.points, points);
    assert_eq!(out.points.len(), 4);
    // off the map: emptied too, 4
    let r = short_path_for(
        walker,
        [4.5, 5.5],
        &world,
        &mut plan,
        ([20.5, 0.0, 5.5], 0.0, -1),
        &[9171],
        &mut out,
    );
    assert_eq!((r, out.points.len()), (Ok(4), 0));
    let small = Walker {
        radius: 0.5,
        ..walker
    };
    let r = short_path_for(
        small,
        [4.5, 5.5],
        &world,
        &mut plan,
        goal,
        &[9171],
        &mut out,
    );
    assert_eq!(
        r,
        Err(GenieError::unsupported(
            "a short path without the game's mesh"
        ))
    );
}

/// A step whose one hit is the walked level's target counts its waypoint reached; that hit,
/// kept, is kept all the same: the first hit loop takes it into the include list and gives
/// it its temporary shape (0x1411547c0..0x141154855) before the test (0x1411549ac).
#[test]
fn a_kept_hit_on_the_target_alone_is_kept_and_reaches_the_waypoint() {
    let mut w = world();
    let mut close = *w.get(9175).unwrap();
    close.position = [28.3, 88.9, 0.0];
    w.insert(9175, close);
    w.set_gait(
        9175,
        Some(Gait {
            last_move: 16361,
            ..Gait::default()
        }),
    );
    let me = Me {
        id: 9177,
        position: [28.0, 89.0, 0.0],
        group: -1,
        action_state: 0,
    };
    let mut m = builder_9177();
    m.paths[0].target = 9175;
    let position = vector(m.moved_at);
    let waypoint = m.paths[0].point(1);
    let projected = m.projected(m.direction_to(position, waypoint), DT);
    let avoided = m.avoid(&me, (&w, None), &[9175], projected, DT, (16361, false));
    let avoided = avoided.unwrap();
    assert!(avoided.at_target);
    assert_eq!((avoided.kept, avoided.pauses), (vec![9175], Vec::new()));
}

/// The walk's kept hits ([`Mover::go_round`]): with a parent the walked level goes and its
/// parent asks for a path again (state 2); a walk without one (a group member's own path,
/// level 0 from slot 0x540) stops the unit (slot 0x5d0) and the step gives up (1).
#[test]
fn kept_hits_replan_the_parent_or_stop_a_walk_without_one() {
    let w = world();
    let me = Me {
        id: 9177,
        position: [28.0, 89.0, 0.0],
        group: -1,
        action_state: 0,
    };
    let mut m = builder_9177();
    assert_eq!(m.paths.len(), 2);
    assert!(matches!(
        m.go_round(&me, &w, &[9175], false),
        Ok(Next::Again)
    ));
    assert_eq!((m.paths.len(), m.paths[0].state), (1, 2));
    let mut lone = builder_9177();
    lone.paths.truncate(1);
    assert!(matches!(
        lone.go_round(&me, &w, &[9175], false),
        Ok(Next::Done(Motion::Done))
    ));
    assert!(lone.speed_stopped);
    assert_eq!(lone.velocity.map(|v| v.velocity[2]), Some(0.0));
}

/// A map edge's record (id `x << 12 | y | 0xff000000`, [`Spatial::edge_records`]) is a hit
/// without an object: the hit loops pass over it (`FUN_1415417a0` finds none,
/// 0x141154709), while it counts among the hits (`local_1758`'s count, 0x14115488a): the
/// walked level's target alone among two hits is no lone target.
#[test]
fn a_map_edge_s_record_is_a_hit_without_an_object() {
    let w = world();
    let me = Me {
        id: 9177,
        position: [28.0, 89.0, 0.0],
        group: -1,
        action_state: 0,
    };
    let mut m = builder_9177();
    m.paths[0].target = 9175;
    let position = vector(m.moved_at);
    let waypoint = m.paths[0].point(1);
    let projected = m.projected(m.direction_to(position, waypoint), DT);
    let edge = (5 << 12) | 3 | 0xff00_0000_u32 as i32;
    let avoided = m
        .avoid(&me, (&w, None), &[edge], projected, DT, (16361, false))
        .unwrap();
    assert_eq!(
        (avoided.kept, avoided.pauses, avoided.at_target),
        (Vec::new(), Vec::new(), false)
    );
    let avoided = m
        .avoid(
            &me,
            (&w, None),
            &[edge, 9175],
            projected,
            DT,
            (16361, false),
        )
        .unwrap();
    assert!(!avoided.at_target);
}

/// The walk's kept hits join the motion step's include list (`DAT_143f4a000`), at most 0x100
/// (`CMP RCX, 0x100; JGE` at 0x1411547cc): each it takes without a mesh shape of its own
/// gets a temporary one; one past the list's end gets neither.
#[test]
fn kept_hits_join_the_include_list_up_to_its_room() {
    let w = world();
    let mut pathing = Pathing::default();
    pathing.start_mesh(120);
    let mut include = vec![0; INCLUDED - 1];
    let shapes = |p: &Pathing| p.mesh().unwrap().shapes.len();
    let before = shapes(&pathing);
    assert_eq!(
        include_kept(&w, Some(&mut pathing), &[9175, 9177], &mut include),
        Ok(())
    );
    assert_eq!((include.len(), include.last()), (INCLUDED, Some(&9175)));
    assert_eq!(shapes(&pathing), before + 1);
    // an object with a shape of its own gets none
    pathing.place_shape(9177, w.get(9177).unwrap());
    let mut include = Vec::new();
    let before = shapes(&pathing);
    assert_eq!(
        include_kept(&w, Some(&mut pathing), &[9177], &mut include),
        Ok(())
    );
    assert_eq!((include, shapes(&pathing)), (vec![9177], before));
}

/// The motion step's path request of a unit wider than half a tile (master `+0x5d8` 0.6, no
/// planner: the straight way, too short): the short pather is the old one (`FUN_140f9e540`
/// at 0x1411524b6 -> `FUN_140f85580`) with the walk's kept hits as its include list
/// (`DAT_143f4a000`): a moving unit in the way counts only when included, and the new level
/// goes round it.
#[test]
fn a_big_unit_s_path_request_takes_the_old_pather_with_the_walk_s_kept_hits() {
    let mut w = Spatial::with_tiles(
        [20, 20],
        crate::map::spatial::Tiles::new(20, 20, vec![4; 400]),
    );
    let unit = |x: f32, r: f32| Placed {
        position: [x, 5.5, 0.0],
        radius: [r, r],
        class: 4,
        player: 2,
        master: 0,
        obstruction_type: 5,
        buildable_on: 0,
        tall: true,
        speed: 0.0,
        unknown: None,
        obstruction: Some(Obstruction { kind: 5, class: 2 }),
    };
    w.insert(9171, unit(4.5, 0.6));
    w.insert(50, unit(6.5, 0.3));
    w.set_gait(
        50,
        Some(Gait {
            moving: true,
            ..Gait::default()
        }),
    );
    let me = Me {
        id: 9171,
        position: [4.5, 5.5, 0.0],
        group: -1,
        action_state: 0,
    };
    let mut pathing = Pathing::default();
    let mut object = Well512::default();
    let groups = crate::groups::UnitGroups::default();
    let mut plan = Planning {
        pathing: &mut pathing,
        now: 100,
        computer: false,
        object: &mut object,
        construction_rate: 1.0,
        groups: &groups,
    };
    let goal = Goal {
        target: None,
        gap: FAR,
        to_end: FAR,
    };
    let mut request = |include: &[i32]| {
        let mut m = builder_9177();
        m.stats.radius = [0.6; 3];
        m.moved_at = [4.5, 5.5, 0.0];
        m.request_path([4.5, 5.5, 0.0], [8.5, 5.5, 0.0], 0.0, -1, 100);
        let mut plans = 0;
        assert!(
            m.request(&me, &w, &mut plan, goal, &mut plans, include)
                .is_ok()
        );
        m.paths[0].points.clone()
    };
    assert_eq!(request(&[]), [[4.5, 0.0, 5.5], [8.5, 0.0, 5.5]]);
    let points = request(&[50]);
    let [start, by, end] = points[..] else {
        panic!("{points:?}")
    };
    assert_eq!((start, end), ([4.5, 0.0, 5.5], [8.5, 0.0, 5.5]));
    let off = (0.3 + 0.6) * std::f32::consts::SQRT_2;
    assert!((by[0] - 6.5).abs() < 1e-5 && (by[2] - (5.5 - off)).abs() < 1e-5);
}

/// The house the builders of `world` walk to.
fn world_target() -> Placed {
    *world().get(10028).unwrap()
}

/// `FUN_14114ffc0` -> `FUN_140fa0370`: the reach test starts at +0x824, where the unit last
/// moved to: a unit whose +0x824 lies off the map reaches nothing, whatever its place.
#[test]
fn the_reach_test_starts_where_the_unit_last_moved_to() {
    let mut world = Spatial::with_tiles(
        [40, 40],
        crate::map::spatial::Tiles::new(40, 40, vec![4; 40 * 40]),
    );
    world.insert(
        9177,
        Placed {
            position: [5.5, 5.5, 0.0],
            radius: [0.2, 0.2],
            class: 4,
            player: 2,
            master: 83,
            obstruction_type: 0,
            buildable_on: 0,
            tall: true,
            speed: 0.8,
            unknown: None,
            obstruction: Some(Obstruction { kind: 5, class: 2 }),
        },
    );
    let mut pathing = Pathing::default();
    let mut object = Well512::default();
    let groups = crate::groups::UnitGroups::default();
    let mut plan = Planning {
        pathing: &mut pathing,
        now: 0,
        computer: false,
        object: &mut object,
        construction_rate: 1.0,
        groups: &groups,
    };
    let me = Me {
        id: 9177,
        position: [5.5, 5.5, 0.0],
        group: -1,
        action_state: 0,
    };
    let mut m = builder_9177();
    m.moved_at = [5.5, 5.5, 0.0];
    // 25 tiles away on open land: the long pather's answer
    let far = [30.5, 5.5];
    assert_eq!(m.reaches(&me, &world, &mut plan, far, 0.0, 83), Ok(true));
    // with ignore units the long pather lowers those that wait or move (none here)
    m.ignore_units = vec![5, 6];
    assert_eq!(m.reaches(&me, &world, &mut plan, far, 0.0, 83), Ok(true));
    m.moved_at = [-1.0, 5.5, 0.0];
    assert_eq!(m.reaches(&me, &world, &mut plan, far, 0.0, 83), Ok(false));
}

/// `FUN_14114f630`: an angle at most the turn's most, or a NaN most (an infinite turn rate
/// times dt 0), turns the whole angle and is not clipped (JBE on unordered).
#[test]
fn a_nan_most_turns_the_whole_angle_unclipped() {
    let mut m = builder_9177();
    m.forward = [1.0, 0.0, 0.0];
    m.right = [0.0, 0.0, -1.0];
    m.stats.turn_rate = f32::INFINITY;
    assert_eq!(m.steer([0.0, 0.0, 1.0], 0.0), Ok(false));
    assert!(m.forward.iter().all(|c| c.is_finite()));
    assert!(0.99 < m.forward[2]);
    // a small most clips the turn
    m.forward = [1.0, 0.0, 0.0];
    m.stats.turn_rate = 1.0;
    assert_eq!(m.steer([0.0, 0.0, 1.0], 0.02), Ok(true));
    assert!(m.forward[2].abs() < 0.03);
}

/// Slot 195 (`FUN_14114f630`) from a forward vector it is given: a quarter turn to the left
/// of +x (toward +z) is negative, clipped to the turn rate times dt; the fast turn's dt is a
/// hundred times longer.
#[test]
fn a_turn_toward_a_direction_is_signed_and_clipped_by_the_turn_rate() {
    let mut m = builder_9177();
    m.stats.turn_rate = 1.0;
    let (x, z) = ([1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
    assert_eq!(m.turn_toward(x, z, 0.5), (-0.5, true));
    assert_eq!(m.turn_toward(z, x, 0.5), (0.5, true));
    let (turn, clipped) = m.turn_toward(x, z, 2.0);
    assert!(!clipped);
    assert_eq!(turn, -std::f32::consts::FRAC_PI_2);
    // a NaN most turns the whole angle (JBE on unordered)
    assert!(!m.turn_toward(x, z, f32::NAN).1);
    assert_eq!(m.turn_dt(0.02), 0.02);
    m.stats.fast_turn = true;
    assert_eq!(m.turn_dt(0.02), 0.02 * 100.0);
}

/// Slot 184 (`FUN_141163140`): the yaw turns the forward vector about the up axis and
/// rebuilds the right one (slot 173); the angle is the new facet's. Without facets it stops
/// after the vectors.
#[test]
fn a_yaw_turns_forward_and_right_and_takes_the_facet_s_angle() {
    let mut m = builder_9177();
    m.forward = [1.0, 0.0, 0.0];
    m.right = [0.0, 0.0, -1.0];
    let facets = crate::objects::facing::SpriteFacets {
        count: 16,
        flag_1: true,
    };
    m.yaw(-std::f32::consts::FRAC_PI_2, Some(facets)).unwrap();
    assert!(m.forward[0].abs() < 1e-6 && (m.forward[2] - 1.0).abs() < 1e-6);
    assert_eq!(m.right, fmath::normalize(fmath::cross(UP, m.forward)));
    let facet = crate::objects::facing::facet_of_forward(m.forward, facets);
    assert_eq!(
        m.angle,
        crate::objects::facing::angle_of_facet(facet, facets)
    );
    let before = m.angle;
    assert!(m.yaw(0.1, None).is_err());
    assert_eq!(m.angle, before);
    assert!(m.forward[0] != 0.0);
}

/// Slot 173 (`FUN_14115d150`) sets the forward vector and the right one from the given up
/// vector, the angle left; slot 0x598 (`FUN_14115d050`) makes zeroed velocity data for a
/// speed other than 0 (a NaN too) and sets its forward speed; slot 0x600 (`FUN_14114d3b0`)
/// reads the walked level's pause.
#[test]
fn the_orientation_speed_and_pause_slots() {
    let mut m = builder_9177();
    m.angle = 1.0;
    m.set_orientation([0.0, 0.0, 1.0], [0.0, 1.0, 0.0]);
    assert_eq!(m.forward, [0.0, 0.0, 1.0]);
    assert_eq!(m.right, [1.0, 0.0, 0.0]);
    assert_eq!(m.angle, 1.0);
    m.velocity = None;
    m.set_speed(0.0);
    assert_eq!(m.velocity, None);
    m.set_speed(f32::NAN);
    assert!(m.velocity.unwrap().velocity[2].is_nan());
    m.velocity = None;
    m.set_speed(1.2);
    assert_eq!(
        m.velocity,
        Some(Velocity {
            velocity: [0.0, 0.0, 1.2],
            acceleration: [0.0; 3],
        })
    );
    m.velocity.as_mut().unwrap().acceleration = [0.0, 0.0, 5.0];
    m.set_speed(0.0);
    assert_eq!(m.velocity.unwrap().velocity[2], 0.0);
    assert_eq!(m.velocity.unwrap().acceleration[2], 5.0);
    m.paths = Vec::new();
    assert!(!m.paused());
    m.paths = vec![PathData::without_path()];
    assert!(!m.paused());
    m.paths[0].pause_time = 0.5;
    assert!(m.paused());
    m.paths[0].pause_time = f32::NAN;
    assert!(!m.paused());
}

/// `LAB_141152ec1`: a waypoint a tile away, or a target a tile from the unit, or a near one of
/// master 45, is the long pather's.
#[test]
fn a_far_waypoint_or_a_target_of_master_45_takes_the_long_pather() {
    assert!(far_waypoint(1.0, None) && !far_waypoint(0.5, None));
    assert!(far_waypoint(0.0, Some((1.0, 4))));
    assert!(!far_waypoint(30.0, Some((0.5, 4))));
    assert!(far_waypoint(30.0, Some((0.5, WIDE_MASTER))));
    assert!(!far_waypoint(30.0, Some((f32::NAN, 4))));
}

/// `FUN_141161950`: a moving unit's footprint stays 0.01 inside the map, its y by the map's
/// width as its x (the game reads `+0x18` for both).
#[test]
fn the_position_setter_holds_y_by_the_map_s_width() {
    let m = builder_9177();
    // a map 40 wide and 120 high: y 100 lies beyond the width less the radius and the gap
    let at = m.set_position([10.0, 100.0, 0.0], [40, 120]);
    assert_eq!(at[1], (40.0f32 - 0.2) - EDGE_GAP);
    // a unit that does not move is not held
    let mut still = builder_9177();
    still.stats.speed = 0.0;
    assert_eq!(still.set_position([10.0, 100.0, 0.0], [40, 120])[1], 100.0);
}

/// `FUN_140f98bc0` with 1: a loop's next point or 0 past its end; a back-and-forth path turns
/// at its ends and flips bit 2; a path that does not loop has none.
#[test]
fn a_looping_path_s_next_waypoint_goes_round_or_back_and_forth() {
    let mut l = level(1, &[[0.0, 0.0], [1.0, 0.0], [2.0, 0.0]], 2);
    l.path_flags = 0;
    assert_eq!(looping_next(&mut l, 1), -1);
    l.path_flags = 1;
    assert_eq!((looping_next(&mut l, 1), looping_next(&mut l, 2)), (2, 0));
    l.path_flags = 3;
    assert_eq!(looping_next(&mut l, 1), 0);
    assert_eq!((looping_next(&mut l, 0), l.path_flags), (1, 7));
    assert_eq!(looping_next(&mut l, 1), 2);
    assert_eq!((looping_next(&mut l, 2), l.path_flags), (1, 3));
}

/// `LAB_141151b6d`: an arrived level of link 2 hands its looping parent its next waypoint;
/// one of another link stays and the step walks on.
#[test]
fn an_arrived_level_hands_a_looping_parent_its_next_waypoint() {
    let goal = Goal {
        target: None,
        gap: FAR,
        to_end: FAR,
    };
    let mut m = builder_9177();
    m.paths[0].linked_type = 2;
    m.paths[1].path_flags = 1;
    m.paths[1].waypoint = 1;
    assert!(matches!(m.arrived(goal), Ok(Next::Again)));
    assert_eq!(
        (m.paths.len(), m.paths[0].state, m.paths[0].waypoint),
        (1, 2, 0)
    );
    let mut m = builder_9177();
    m.paths[0].linked_type = 0;
    assert!(matches!(m.arrived(goal), Ok(Next::Dispatch)));
    assert_eq!(m.paths.len(), 2);
}

/// `LAB_14115136c` on a loop (the path's flags bit 0): the level moves on from the waypoint
/// reached (`FUN_140f98bc0` with 1) and waits for a path (state 2), on to its state; back at
/// its first waypoint the step ends (`LAB_141155637`): the unit turns toward the direction
/// the step's walk last took (zero without one) and, its level in state 2, stops.
#[test]
fn a_loop_s_reached_waypoint_moves_on_and_its_first_ends_the_step() {
    let looping = |waypoint| {
        let mut m = builder_9177();
        m.paths = vec![level(2, &[[28.0, 89.0], [30.0, 88.0], [32.0, 89.0]], 3)];
        let l = &mut m.paths[0];
        l.target = -1;
        l.path_flags = 1;
        l.waypoint = waypoint;
        l.flags = 1;
        m
    };
    let mut world = Spatial::new([120, 120]);
    let mut me = Me {
        id: 9177,
        position: [30.0, 88.0, 0.0],
        group: -1,
        action_state: 0,
    };
    let goal = Goal {
        target: None,
        gap: FAR,
        to_end: FAR,
    };
    let mut m = looping(1);
    let next = m.reached(&mut me, &mut world, None, goal, 0);
    assert!(matches!(next, Ok(Next::Dispatch)));
    assert_eq!((m.paths[0].waypoint, m.paths[0].state), (2, 2));
    let mut m = looping(2);
    let next = m.reached(&mut me, &mut world, None, goal, 0);
    assert!(matches!(next, Ok(Next::LoopEnd)));
    assert_eq!((m.paths[0].waypoint, m.paths[0].state), (0, 2));
    let mut m = looping(2);
    me.position = [32.0, 89.0, 0.0];
    m.moved_at = me.position;
    // no walk ran in the step: the direction it turns toward is zero, at π/2 (426048706's
    // scout 7227 in update 5429 turns by π/2 there)
    let mut expect = m.clone();
    let mut flat = expect.forward;
    flat[1] = 0.0;
    let flat = fmath::normalize(flat);
    let (turn, _) = expect.turn_toward(flat, [0.0; 3], expect.turn_dt(DT));
    expect.yaw(turn, expect.facets()).unwrap();
    assert_ne!(turn, 0.0);
    assert_eq!(m.motion(&mut me, DT, &mut world, None), Ok(Motion::Moving));
    assert_eq!((m.paths[0].waypoint, m.paths[0].state), (0, 2));
    assert_eq!(m.velocity.unwrap().velocity, [0.0; 3]);
    assert_eq!(m.forward, expect.forward);
}

/// `0x141150f00`: a level whose target is gone walks as to a point (`FUN_1415417a0` finds
/// none, `LAB_1411512df`), as one with no target does.
#[test]
fn a_walk_whose_target_is_gone_walks_as_to_a_point() {
    let walk = |target: i32| {
        let mut m = builder_9177();
        for l in &mut m.paths {
            l.target = target;
            l.range = 0.0;
        }
        let mut me = Me {
            id: 9926,
            position: [28.0, 89.0, 0.0],
            group: -1,
            action_state: 0,
        };
        let motion = m.motion(&mut me, DT, &mut Spatial::new([120, 120]), None);
        (motion, m.forward, m.velocity)
    };
    assert_eq!(walk(77), walk(-1));
    assert_eq!(walk(77).0, Ok(Motion::Moving));
}

/// The motion step's gate gaps (0x1411512b3, 0x141151a62, 0x141154f22): a gate's gap is the
/// least of its own and its first two annexes' through the site's terms (`MINSS`); any other
/// target's is its own.
#[test]
fn a_gate_s_gap_is_the_least_over_it_and_its_annexes() {
    let mut world = Spatial::new([40, 40]);
    let wall = |x: f32, class: i16| Placed {
        position: [x, 10.0, 0.0],
        radius: [0.5, 0.5],
        class,
        player: 1,
        master: 0,
        obstruction_type: 2,
        buildable_on: 0,
        tall: true,
        speed: 0.0,
        unknown: None,
        obstruction: Some(Obstruction { kind: 2, class: 1 }),
    };
    world.insert(100, wall(10.0, GATE));
    world.insert(101, wall(12.0, 27));
    world.insert(102, wall(8.0, 27));
    world.set_annex(101, Some(100));
    world.set_annex(102, Some(100));
    let near = ([13.5, 10.0, 0.0], [0.2, 0.2]);
    let gate = *world.get(100).unwrap();
    let own = footprint_gap(near.0, near.1, gate.position, gate.radius);
    assert!((own - 2.8).abs() < 1e-6);
    // annex 101 lies 0.8 away, less 0.1
    let gap = gate_gap(&world, near, (100, &gate), own, |g| g - ANNEX_GAP);
    let annex = footprint_gap(near.0, near.1, [12.0, 10.0, 0.0], [0.5, 0.5]);
    assert!((annex - 0.8).abs() < 1e-6);
    assert_eq!(gap, annex - ANNEX_GAP);
    // not a gate: its own
    let mut post = gate;
    post.class = 27;
    assert_eq!(gate_gap(&world, near, (100, &post), own, |g| g - 1.0), own);
}

/// `FUN_140f98d50` and `FUN_140f98dd0`: a point goes, or the path ends before it and is
/// measured again; an index outside the path, or an empty path, changes nothing.
#[test]
fn a_path_loses_a_point_or_its_end_only_for_an_index_inside_it() {
    let mut points = vec![[0.0; 3], [1.0, 0.0, 0.0], [1.0, 0.0, 2.0]];
    assert!(!remove_point(&mut points, -1));
    assert!(!remove_point(&mut points, 3));
    assert!(remove_point(&mut points, 1));
    assert_eq!(points, [[0.0; 3], [1.0, 0.0, 2.0]]);
    let mut path = Path {
        points: vec![[0.0; 3], [3.0, 0.0, 0.0], [3.0, 0.0, 4.0], [0.0; 3]],
        length: 12.0,
    };
    assert!(!cut_path(&mut path, -1));
    assert!(!cut_path(&mut path, 4));
    assert_eq!(path.length, 12.0);
    assert!(cut_path(&mut path, 3));
    assert_eq!((path.points.len(), path.length), (3, 7.0));
    assert!(cut_path(&mut path, 0));
    assert_eq!((path.points.len(), path.length), (0, 0.0));
    assert!(!cut_path(&mut path, 0));
}

/// `FUN_140f96050`: the top level decides, not the walked one; a level without a path has
/// no more; else `count - 1 <= waypoint` in 32 bits says none.
#[test]
fn the_top_level_has_more_waypoints_unless_its_count_less_one_reaches_its_index() {
    let mut m = builder_9177();
    // the top level's two points, its waypoint 1: none after it, whatever the walked level has
    m.paths[0].waypoint = 0;
    assert!(!m.top_has_more());
    m.paths[1].waypoint = 0;
    assert!(m.top_has_more());
    // without a path: none
    m.paths[1].has_path = false;
    assert!(!m.top_has_more());
    // an empty path: -1 <= waypoint, but a waypoint below -1 has more
    m.paths[1].has_path = true;
    m.paths[1].points.clear();
    assert!(!m.top_has_more());
    m.paths[1].waypoint = -2;
    assert!(m.top_has_more());
    m.paths.clear();
    assert!(!m.top_has_more());
}

/// `FUN_140f9b8f0` compares `i` with the point count less one, signed: an empty path or the
/// last point has no stretch to split; any other stretch gets its midpoint.
#[test]
fn a_split_takes_the_midpoint_of_a_stretch_and_skips_an_empty_path() {
    let mut empty = Path::default();
    split(&mut empty, 0);
    assert_eq!(empty.points, Vec::<Vec3>::new());
    let mut path = Path {
        points: vec![[0.0, 0.0, 0.0], [4.0, 0.0, 0.0]],
        length: 4.0,
    };
    split(&mut path, 1);
    assert_eq!(path.points.len(), 2);
    split(&mut path, 0);
    assert_eq!(
        path.points,
        [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [4.0, 0.0, 0.0]]
    );
}

/// Slot 196 (`FUN_14114f400`) at 0x14114f520..0x14114f52d: a NaN distance to the waypoint
/// goes past the first test and stops at the second (JBE on unordered): short.
#[test]
fn a_nan_waypoint_distance_is_short() {
    let w = [f32::NAN, 0.0, 5.0];
    assert!(matches!(reach(w, [0.0; 3], [1.0, 0.0, 0.0]), Reach::Short));
    assert!(matches!(
        reach([5.0, 0.0, 0.0], [0.0; 3], [1.0, 0.0, 0.0]),
        Reach::Short
    ));
    assert!(matches!(
        reach([5.0, 0.0, 0.0], [0.0; 3], [5.0, 0.0, 0.0]),
        Reach::Reached
    ));
}

/// The ground direction (`FUN_14114f210`): a NaN length divides (JBE on unordered), so the
/// direction is NaN, not the forward vector.
#[test]
fn a_direction_of_a_nan_length_is_nan() {
    let m = crate::gathering::hunt::tests::walker();
    let d = m.direction_to([f32::NAN, 0.0, 0.0], [1.0, 0.0, 1.0]);
    assert!(d[0].is_nan() && d[2].is_nan());
    let d = m.direction_to([0.0, 0.0, 0.0], [3.0, 0.0, 4.0]);
    assert_eq!(d, [0.6, 0.0, 0.8]);
}

#[test]
fn two_boxes_overlap_by_the_lesser_of_their_axes() {
    // the cow 9517 at (21.8, 92.2) and Gaia's cow 9519 at (22.5, 91.5), radii 0.5 and 0.5
    let a = [21.8, 0.0, 92.2];
    let b = [22.5, 0.0, 91.5];
    let d = overlap(a, [0.5, 0.5], b, [0.5, 0.5]);
    assert_eq!(d, (21.8f32 + 0.5) - (22.5 - 0.5));
    assert_eq!(overlap(a, [0.2, 0.2], b, [0.2, 0.2]), 0.0);
    // a NaN goes past the four tests (COMISS/JNC) into MINSS/MAXSS, whose second operand wins
    let nan = overlap([f32::NAN, 0.0, 92.2], [0.5, 0.5], b, [0.5, 0.5]);
    assert_eq!(
        nan,
        fmath::minss(
            (22.5f32 + 0.5) - (22.5 - 0.5),
            (91.5f32 + 0.5) - (92.2 - 0.5)
        )
    );
}

#[test]
fn level_0_looks_ahead_to_the_farthest_parent_waypoint_it_walks_to_straight() {
    // a cow at its level 0 waypoint (21.8, 92.2), its level 1 parent's waypoint 1 of three
    let mut m = builder_9177();
    m.moved_at = [21.8, 92.2, 0.0];
    m.paths = vec![
        level(0, &[[21.5, 91.5], [21.8, 92.2], [25.15, 92.85]], 3),
        level(
            1,
            &[[21.5, 91.5], [28.5, 93.5], [28.9, 94.1], [40.0, 94.0]],
            0,
        ),
    ];
    m.paths[1].waypoint = 1;
    let me = Me {
        id: 9517,
        position: [21.8, 92.2, 0.0],
        group: -1,
        action_state: 0,
    };
    // land of a terrain class the unit's mask (6) has
    let mut w = Spatial::with_tiles(
        [120, 120],
        crate::map::spatial::Tiles::new(120, 120, vec![2; 14400]),
    );
    // on open ground: waypoint 3 is beyond 10 tiles, waypoint 2 within, so it takes 2
    assert_eq!(m.look_ahead(&me, &w), Ok(true));
    assert_eq!(m.paths.len(), 1);
    assert_eq!(
        (m.paths[0].level, m.paths[0].waypoint, m.paths[0].state),
        (1, 2, 2)
    );
    // a tree in the way of both leaves the levels as they are
    let mut m = builder_9177();
    m.moved_at = [21.8, 92.2, 0.0];
    m.paths = vec![
        level(0, &[[21.5, 91.5], [21.8, 92.2]], 3),
        level(1, &[[21.5, 91.5], [28.5, 93.5], [28.9, 94.1]], 0),
    ];
    m.paths[1].waypoint = 1;
    w.insert(
        9357,
        Placed {
            position: [25.0, 93.0, 0.0],
            radius: [0.5, 0.5],
            class: 15,
            player: 0,
            master: 0,
            obstruction_type: 2,
            buildable_on: 0,
            tall: true,
            speed: 0.0,
            unknown: None,
            obstruction: Some(Obstruction { kind: 2, class: 1 }),
        },
    );
    assert_eq!(m.look_ahead(&me, &w), Ok(false));
    assert_eq!(m.paths.len(), 2);
}

/// A trade cart plans only within its owner's budget; any unit at most three times a step.
#[test]
fn a_trade_cart_plans_within_its_owners_budget() {
    let spent = PlanBudget {
        attempt_cap: 6,
        delay_cap: 2,
        plans: 6,
    };
    assert!(!may_plan(true, Some(spent), 0, 0));
    // a villager is not held to the budget
    assert!(may_plan(false, Some(spent), 0, 0));
    // past the delay cap the budget lets the cart plan again
    assert!(may_plan(true, Some(spent), 3, 0));
    assert!(may_plan(true, Some(PlanBudget { plans: 5, ..spent }), 0, 0));
    assert!(may_plan(true, None, 0, 0));
    assert!(!may_plan(false, None, 0, PLANS));
}

/// `FUN_140f9eb70`'s list for `FUN_140f60440`: a walker in no group lists itself and its
/// ignore units that move (not one that stands, not the target, not one the world lacks),
/// then the target; a walker in a group lists its group's differentiator units instead.
#[test]
fn the_long_pather_lowers_the_walker_or_its_groups_units_and_the_target() {
    let mut s = world();
    let moving = Gait {
        moving: true,
        ..Gait::default()
    };
    s.set_gait(9175, Some(moving));
    s.set_gait(10028, Some(moving));
    let ignore = [9175, 9177, 10028, 4242];
    let mut walker = LongWalker {
        id: 9177,
        position: [28.0, 89.0, 0.0],
        radius: [0.2, 0.2],
        mask: 1,
        moves: true,
        ignore_units: &ignore,
        group_units: None,
    };
    // 10028 moves but is the target: it comes last, once
    assert_eq!(lowered_units(&walker, &s, Some(10028)), [9177, 9175, 10028]);
    assert_eq!(lowered_units(&walker, &s, None), [9177, 9175, 10028]);
    walker.group_units = Some(vec![9177, 31, 32]);
    assert_eq!(
        lowered_units(&walker, &s, Some(10028)),
        [9177, 31, 32, 10028]
    );
    // a group whose differentiator groups are gone lists none, not even the walker
    walker.group_units = Some(Vec::new());
    assert_eq!(lowered_units(&walker, &s, None), Vec::<i32>::new());
}

pub(crate) fn world() -> Spatial {
    let mut s = Spatial::new([120, 120]);
    let unit = |x, y| Placed {
        position: [x, y, 0.0],
        radius: [0.2, 0.2],
        class: 4,
        player: 2,
        master: 0,
        obstruction_type: 0,
        buildable_on: 0,
        tall: true,
        speed: 0.0,
        unknown: None,
        obstruction: Some(Obstruction { kind: 5, class: 2 }),
    };
    s.insert(9175, unit(30.0, 89.0));
    s.insert(9177, unit(28.0, 89.0));
    s.insert(
        10028,
        Placed {
            position: [30.0, 88.0, 0.0],
            radius: [1.0, 1.0],
            class: 3,
            player: 2,
            master: 0,
            obstruction_type: 0,
            buildable_on: 0,
            tall: true,
            speed: 0.0,
            unknown: None,
            obstruction: None,
        },
    );
    s
}

/// [`world`]'s objects on a 120 × 120 map of tiles of `classes` (row by row), which the
/// pather reads.
pub(crate) fn tiled_world(classes: Vec<u8>) -> Spatial {
    let objects = world();
    let tiles = crate::map::spatial::Tiles::new(120, 120, classes);
    let mut s = Spatial::with_tiles([120, 120], tiles);
    for id in [9175, 9177, 10028] {
        s.insert(id, *objects.get(id).expect("in the world"));
    }
    s
}

/// `FUN_14114cc60` from where the unit last moved (`+0x824`) to `p` (x, height, y): a clear
/// line closes in; a point off the map, a line through terrain the unit's mask lacks, or one
/// through another unit (9175 at (30, 89)) does not, but for the target, which the query
/// ignores.
#[test]
fn a_unit_closes_in_along_a_clear_straight_line() {
    let m = builder_9177();
    let me = Me {
        id: 9177,
        position: [28.0, 89.0, 0.0],
        group: -1,
        action_state: 0,
    };
    let open = tiled_world(vec![4; 120 * 120]);
    assert_eq!(m.close_in(&me, [28.0, 0.0, 87.0], -1, &open), Ok(true));
    assert_eq!(m.close_in(&me, [32.0, 0.0, 89.0], -1, &open), Ok(false));
    assert_eq!(m.close_in(&me, [32.0, 0.0, 89.0], 9175, &open), Ok(true));
    assert_eq!(m.close_in(&me, [-1.0, 0.0, 89.0], -1, &open), Ok(false));
    assert_eq!(m.close_in(&me, [28.0, 0.0, 200.0], -1, &open), Ok(false));
    let mut classes = vec![4; 120 * 120];
    for x in 26..31 {
        classes[88 * 120 + x] = 1;
    }
    let walled = tiled_world(classes);
    assert_eq!(m.close_in(&me, [28.0, 0.0, 87.0], -1, &walled), Ok(false));
}

/// The long path's stretches (`0x141150f00` after `FUN_140f9eb70`): one whose query meets a
/// wall, a gate, a tower or a building that does not move ends the path before it, or, the
/// first, empties it (false); a moving building and a clear path leave it; a villager's path
/// is never cut.
#[test]
fn a_long_path_ends_before_its_first_walled_stretch() {
    let m = builder_9177();
    let me = Me {
        id: 9177,
        position: [28.0, 89.0, 0.0],
        group: -1,
        action_state: 0,
    };
    let path = || Path {
        points: vec![[28.0, 0.0, 89.0], [28.0, 0.0, 85.0], [28.0, 0.0, 80.0]],
        length: 9.0,
    };
    let obstacle = |class, speed, y| Placed {
        position: [28.0, y, 0.0],
        radius: [0.5, 0.5],
        class,
        player: 1,
        master: 72,
        obstruction_type: 0,
        buildable_on: 0,
        tall: true,
        speed,
        unknown: None,
        obstruction: Some(Obstruction { kind: 5, class: 2 }),
    };
    let walled = |soldier: bool, block: Option<Placed>| {
        let mut world = tiled_world(vec![4; 120 * 120]);
        if soldier {
            let mut me = *world.get(9177).unwrap();
            me.class = 6;
            world.insert(9177, me);
        }
        if let Some(b) = block {
            world.insert(500, b);
        }
        let mut p = path();
        let r = m.walled_off(&me, &world, -1, &mut p);
        (r, p.points.len())
    };
    assert_eq!(walled(true, None), (Ok(true), 3));
    assert_eq!(
        walled(true, Some(obstacle(WALL_CLASS, 0.0, 82.0))),
        (Ok(true), 2)
    );
    assert_eq!(
        walled(true, Some(obstacle(GATE, 0.0, 87.0))),
        (Ok(false), 0)
    );
    assert_eq!(
        walled(true, Some(obstacle(TOWER_CLASS, 0.0, 82.0))),
        (Ok(true), 2)
    );
    let still = obstacle(BUILDING_CLASS, 0.0, 82.0);
    assert_eq!(walled(true, Some(still)), (Ok(true), 2));
    let moving = obstacle(BUILDING_CLASS, 1.0, 82.0);
    assert_eq!(walled(true, Some(moving)), (Ok(true), 3));
    let wall = obstacle(WALL_CLASS, 0.0, 87.0);
    assert_eq!(walled(false, Some(wall)), (Ok(true), 3));
}

/// The long pather's plan for a far waypoint (`0x141150f00` at `LAB_141152f9c`) from 9177's
/// tile: on open tiles it reaches the waypoint's tile (true); a waypoint on its own tile
/// gives no path, which stops the unit (None); a waypoint in a band its mask lacks (rows 70
/// to 74) ends at the band's edge (false), and farther from the waypoint than `way` allows the
/// path empties and the gap of its end comes back (3.54 here).
#[test]
fn a_long_plan_reaches_its_waypoint_stops_short_or_ends_at_a_band() {
    let mut band = vec![4; 120 * 120];
    for y in 70..75 {
        for x in 20..40 {
            band[y * 120 + x] = 1;
        }
    }
    let plan = |classes: &[u8], waypoint: Vec3, way: f32| {
        let world = tiled_world(classes.to_vec());
        let mut pathing = Pathing::default();
        pathing.start_mesh(120);
        let mut object = Well512::default();
        let groups = crate::groups::UnitGroups::default();
        let plan = Planning {
            pathing: &mut pathing,
            now: 0,
            computer: false,
            object: &mut object,
            construction_rate: 1.0,
            groups: &groups,
        };
        let me = Me {
            id: 9177,
            position: [28.0, 89.0, 0.0],
            group: -1,
            action_state: 0,
        };
        let mut m = builder_9177();
        let mut path = Path::default();
        let r = m.long_plan(&me, &world, &plan, waypoint, way, 0.0, None, &mut path);
        (r, path.points, path.length)
    };
    let open = vec![4; 120 * 120];
    assert_eq!(
        plan(&open, [28.0, 0.0, 60.0], 100.0),
        (
            Ok(Some((true, None))),
            vec![[28.5, 0.0, 89.5], [28.5, 0.0, 60.5]],
            29.0
        )
    );
    assert_eq!(plan(&open, [28.0, 0.0, 89.0], 100.0).0, Ok(None));
    assert_eq!(
        plan(&band, [28.0, 0.0, 72.0], 100.0),
        (
            Ok(Some((false, None))),
            vec![[28.5, 0.0, 89.5], [28.5, 0.0, 75.5]],
            14.0
        )
    );
    let short = plan(&band, [28.0, 0.0, 72.0], 0.5);
    assert_eq!(short.0, Ok(Some((false, Some(12.5f32.sqrt())))));
    assert_eq!((short.1.len(), short.2), (0, 0.0));
}

/// The new level's path is no path (`0x141150f00` after the plans), on 9177's walked level:
/// a found path in range of its end has its waypoint marked reached on a looping level (else
/// it is not modelled); not found, or out of range, off its last waypoint the waypoint is
/// marked too; at its last waypoint while its continue counter runs the unit pauses for
/// `STUCK_PAUSE` and stops; with the counter out a level 0 under a top path with more waits
/// at its end (state 5); a looping level, an attack move, a level above 1 and a top path
/// without more are not modelled. With a target, bit 2 moves from the level to its parents.
#[test]
fn a_level_without_a_path_marks_its_waypoint_pauses_or_waits() {
    let target = *world().get(10028).unwrap();
    let mut m = builder_9177();
    m.paths[0].flags = 4;
    assert!(matches!(
        m.no_path(false, Some(&target), 0.5),
        Ok(Step::Paused)
    ));
    assert_eq!(m.paths[0].pause_time, STUCK_PAUSE);
    assert_eq!((m.paths[0].flags, m.paths[1].flags), (0, 4));
    let out = |edit: &dyn Fn(&mut Mover)| {
        let mut m = builder_9177();
        m.paths[0].continue_counter = 0;
        edit(&mut m);
        let r = m.no_path(false, None, 0.5);
        (r, m)
    };
    let (r, m) = out(&|m| m.paths[1].waypoint = 0);
    assert!(matches!(r, Ok(Step::Again)));
    assert_eq!(m.paths[0].state, 5);
    assert!(out(&|_| {}).0.is_err());
    assert!(out(&|m| m.paths[0].path_flags |= 1).0.is_err());
    assert!(out(&|m| m.attack_moving = true).0.is_err());
    assert!(out(&|m| m.paths[0].level = 2).0.is_err());
    let (r, m) = out(&|m| m.paths[0].waypoint = 0);
    assert!(matches!(r, Ok(Step::Again)));
    assert_eq!(m.paths[0].flags, 1);
    let mut m = builder_9177();
    assert!(m.no_path(true, None, 0.05).is_err());
    m.paths[0].path_flags |= 1;
    assert!(matches!(m.no_path(true, None, 0.05), Ok(Step::Again)));
    assert_eq!(m.paths[0].flags, 1);
}

/// A unit's walk for the actions' tests: MarvinBlue/426048731's builder 9177 on its first
/// path toward house 10028 ([`builder_9177`]), its `MoveTo`'s point the house's, on open
/// tiles ([`tiled_world`]) with the pather's mesh, a fresh Object stream and no unit groups.
pub(crate) struct WalkRig {
    pub(crate) mover: Mover,
    pub(crate) world: Spatial,
    pathing: Pathing,
    object: Well512,
    groups: crate::groups::UnitGroups,
}

impl WalkRig {
    pub(crate) fn on_the_path() -> Self {
        let mut mover = builder_9177();
        mover.move_to_point = Some([30.0, 88.0, 0.0]);
        let mut pathing = Pathing::default();
        pathing.start_mesh(120);
        Self {
            mover,
            world: tiled_world(vec![4; 120 * 120]),
            pathing,
            object: Well512::default(),
            groups: crate::groups::UnitGroups::default(),
        }
    }

    /// The walk of unit 9177 at (28, 89) in action state `state`, a tenth of a second long,
    /// with the world's planners.
    pub(crate) fn walk(&mut self, state: u8) -> Walk<'_> {
        Walk {
            mover: Some(&mut self.mover),
            me: Me {
                id: 9177,
                position: [28.0, 89.0, 0.0],
                group: -1,
                action_state: state,
            },
            dt: 0.1,
            world: &mut self.world,
            plan: Some(Planning {
                pathing: &mut self.pathing,
                now: 0,
                computer: false,
                object: &mut self.object,
                construction_rate: 1.0,
                groups: &self.groups,
            }),
        }
    }
}

/// One tick of the walker: the velocity step, then the motion step from where it got.
fn tick(m: &mut Mover, at: [f32; 3], world: &mut Spatial) -> ([f32; 3], Motion) {
    let at = m.step_velocity(at, DT, world.map, None, 0);
    world.set_position(9177, at);
    let motion = m
        .motion(
            &mut Me {
                id: 9177,
                position: at,
                group: -1,
                action_state: 0,
            },
            DT,
            world,
            None,
        )
        .unwrap();
    (at, motion)
}

/// `LAB_141153bd1`'s waypoint tests: a unit on its waypoint, or one whose step would lead
/// away from it, counts it reached and the motion step starts again (on the path's last
/// waypoint the walk ends: arrived, the levels reset); with more than 2 substitutes a short
/// step counts it reached too (flags 3) and the walk goes on.
#[test]
fn a_walk_on_or_past_its_waypoint_counts_it_reached() {
    let me = |position| Me {
        id: 9177,
        position,
        group: -1,
        action_state: 0,
    };
    let levels = |m: &Mover| {
        m.paths
            .iter()
            .map(|l| (l.waypoint, l.flags, l.state))
            .collect::<Vec<_>>()
    };
    for at in [[30.0, 88.0, 0.0], [31.0, 87.5, 0.0]] {
        let mut w = world();
        let mut m = builder_9177();
        m.moved_at = at;
        let motion = m.motion(&mut me(at), DT, &mut w, None).unwrap();
        assert_eq!(motion, Motion::Arrived);
        assert_eq!(levels(&m), [(0, 0, 0)]);
    }
    let mut w = world();
    let mut m = builder_9177();
    m.substitute_count = 3;
    let (_, motion) = tick(&mut m, [28.0, 89.0, 0.0], &mut w);
    assert_eq!(motion, Motion::Moving);
    assert_eq!(levels(&m), [(1, 3, 3), (1, 0, 0)]);
}

#[test]
fn the_first_step_reaches_top_speed_and_moves_along_the_saved_forward_vector() {
    let mut m = builder_9177();
    let mut w = world();
    let (at, motion) = tick(&mut m, [28.0, 89.0, 0.0], &mut w);
    // the tick oracle's positions after update 681 (snapshot 680)
    assert_eq!(at, [28.01431, 88.99284, 0.0]);
    assert_eq!(motion, Motion::Moving);
    assert_eq!(m.velocity.as_ref().unwrap().velocity, [0.0, 0.0, 0.8]);
    assert_eq!(
        m.velocity.as_ref().unwrap().acceleration,
        [0.0, 0.0, f32::MAX]
    );
    assert_eq!(m.forward, [0.894_427_2, 0.0, -0.447_213_62]);
}

/// `moveForward` (`FUN_141158490`), slot 199 (`FUN_141161860`) and slot 198
/// (`FUN_141150a80`) start from `+0x824`, where the unit last moved to, not from the place
/// the caller holds.
#[test]
fn the_step_and_the_speed_control_start_where_the_unit_last_moved_to() {
    let mut m = builder_9177();
    m.forward = [1.0, 0.0, 0.0];
    m.moved_at = [10.0, 20.0, 0.0];
    m.velocity.as_mut().unwrap().velocity = [0.0, 0.0, 0.5];
    m.velocity.as_mut().unwrap().acceleration = [0.0; 3];
    // the step: 0.5 tiles a second for a second, from (10, 20) whatever the caller holds
    let at = m.step_velocity([40.0, 40.0, 0.0], 1.0, [120, 120], None, 0);
    assert_eq!(at, [10.5, 20.0, 0.0]);
    assert_eq!(m.moved_at, [10.5, 20.0, 0.0]);
    // the projection along +x: 0.5 tiles from (10.5, 20)
    assert_eq!(m.projected([1.0, 0.0, 0.0], 1.0), [11.0, 0.0, 20.0]);
    // the speed control's turning circle around (10.5, 20) grown to the right by 1: a
    // waypoint inside it brakes, one beyond it speeds up
    m.stats.turn_radius = 2.0;
    m.stats.turn_radius_speed = 1.0;
    m.right = [0.0, 0.0, -1.0];
    m.control_speed([10.5, 0.0, 19.2], false);
    assert_eq!(m.velocity.as_ref().unwrap().acceleration[2], -f32::MAX);
    m.velocity.as_mut().unwrap().acceleration = [0.0; 3];
    m.control_speed([12.0, 0.0, 17.0], false);
    assert_eq!(m.velocity.as_ref().unwrap().acceleration[2], f32::MAX);
}

#[test]
fn the_second_tick_turns_toward_the_waypoint_from_where_the_first_step_ended() {
    let mut m = builder_9177();
    let mut w = world();
    let (at, _) = tick(&mut m, [28.0, 89.0, 0.0], &mut w);
    let (at, _) = tick(&mut m, at, &mut w);
    assert_eq!(at, [28.028_622, 88.98569, 0.0]);
    // after update 682 the game's forward vector points from there to (30, 88)
    assert_eq!(m.forward, [0.894_581_56, 0.0, -0.446_904_78]);
    assert_eq!(m.right, [-0.446_904_78, 0.0, -0.894_581_56]);
    let (at, _) = tick(&mut m, at, &mut w);
    assert_eq!(at, [28.042_934, 88.97854, 0.0]);
}

#[test]
fn the_gap_is_the_distance_between_the_footprints() {
    // MarvinBlue/426048731: a house at (30, 88), builders at (30, 89), (28, 89), (29, 99)
    let (villager, house) = ([0.2, 0.2], [1.0, 1.0]);
    let gap = |x, y| footprint_gap([x, y, 0.0], villager, [30.0, 88.0, 0.0], house);
    assert_eq!(gap(30.0, 89.0), 0.0);
    assert_eq!(gap(28.0, 89.0), 2.0f32 - 1.2);
    assert_eq!(gap(29.0, 99.0), (11.0f32 - 1.2).hypot(0.0));
}

/// MarvinBlue/426048731's hawk 9926 after tick 679: a flight to a point, no range.
#[test]
fn a_walk_to_a_point_goes_on_until_its_range() {
    let mut hawk = builder_9177();
    for l in &mut hawk.paths {
        l.target = -1;
        l.range = 0.0;
    }
    let mut me = Me {
        id: 9926,
        position: [28.0, 89.0, 0.0],
        group: -1,
        action_state: 0,
    };
    assert_eq!(
        hawk.motion(&mut me, DT, &mut Spatial::new([120, 120]), None),
        Ok(Motion::Moving)
    );
    // a deer's walk with range 0.5 whose top path ends within it
    let mut deer = builder_9177();
    for l in &mut deer.paths {
        l.target = -1;
        l.range = 0.5;
    }
    deer.paths[1].points = vec![[28.0, 0.0, 89.0], [28.6, 0.0, 89.0]];
    deer.velocity.as_mut().unwrap().velocity = [0.0, 0.0, 0.8];
    assert_eq!(
        deer.motion(&mut me, DT, &mut Spatial::new([120, 120]), None),
        Ok(Motion::Arrived)
    );
    // the walk ends: one level, reset, and the deer stops
    assert_eq!(deer.paths.len(), 1);
    assert_eq!(
        (
            deer.paths[0].state,
            deer.paths[0].target,
            &deer.paths[0].points
        ),
        (0, -1, &vec![])
    );
    assert_eq!(
        deer.velocity
            .as_ref()
            .map(|v| (v.velocity[2], v.acceleration[2])),
        Some((0.0, 0.0))
    );
}

/// `0x141154f8a`: a top level (2) at its last waypoint, short of its target and off the
/// waypoint it left: a walker without a weapon range whose target moved after the path was
/// made lets the path go and arrives (2); one whose target did not, or a walker with a weapon
/// range, takes state 6 and is done (1); on a tie of times the higher player, then the higher
/// id yields (427222982, 428355643, 428376971).
#[test]
fn a_top_level_short_of_its_moved_target_arrives() {
    let walk = |weapon_range: f32| {
        let mut m = builder_9177();
        m.paths = vec![level(2, &[[28.0, 89.0], [30.0, 88.0]], 3)];
        let l = &mut m.paths[0];
        l.flags = 1;
        l.target = 500;
        l.created = 100;
        m.stats.weapon_range = weapon_range;
        m
    };
    let run = |m: &mut Mover, last_move: u32, target_player: usize, target_id: i32| {
        let mut world = Spatial::new([120, 120]);
        let placed = Placed {
            position: [40.0, 88.0, 0.0],
            radius: [0.5, 0.5],
            class: 6,
            player: target_player,
            master: 74,
            obstruction_type: 0,
            buildable_on: 0,
            tall: false,
            speed: 1.0,
            unknown: None,
            obstruction: None,
        };
        world.insert(target_id, placed);
        world.set_gait(
            target_id,
            Some(Gait {
                last_move,
                ..Gait::default()
            }),
        );
        let mut me = Me {
            id: 9177,
            position: [30.0, 88.0, 0.0],
            group: -1,
            action_state: 0,
        };
        m.paths[0].target = target_id;
        let goal = Goal {
            target: Some(&placed),
            gap: 9.0,
            to_end: 9.0,
        };
        m.reached(&mut me, &mut world, None, goal, 0)
    };
    let mut m = walk(0.0);
    assert!(matches!(
        run(&mut m, 150, 0, 500),
        Ok(Next::Done(Motion::Arrived))
    ));
    assert_eq!(m.paths.len(), 1);
    assert_eq!(m.paths[0].created, 0);
    let mut m = walk(0.0);
    assert!(matches!(
        run(&mut m, 50, 0, 500),
        Ok(Next::Done(Motion::Done))
    ));
    assert_eq!(m.paths[0].state, 6);
    let mut m = walk(2.0);
    assert!(matches!(
        run(&mut m, 150, 0, 500),
        Ok(Next::Done(Motion::Done))
    ));
    // a tie: a target of a higher id yields, a lower one does not
    let mut m = walk(0.0);
    assert!(matches!(
        run(&mut m, 100, 0, 9500),
        Ok(Next::Done(Motion::Arrived))
    ));
    let mut m = walk(0.0);
    assert!(matches!(
        run(&mut m, 100, 0, 500),
        Ok(Next::Done(Motion::Done))
    ));
}

/// `LAB_14115136c` on a level 1 or 2 whose next waypoint lies within reach (its squared
/// distance below [`TARGET_SHORT_REACH`]): the manager's point query at the waypoint it moves
/// on to (`FUN_141051900`, flags 0x20) drops that waypoint (`FUN_140f98d50`) when an object's
/// record holds it, unless the manager's ignore list as the last `FUN_1410562e0` left it
/// holds the object (426916909's p0 in update 4036, 428265662's in 4931).
#[test]
fn a_next_waypoint_an_object_stands_on_goes() {
    let walk = || {
        let mut m = builder_9177();
        m.paths = vec![level(
            2,
            &[[28.0, 89.0], [30.0, 88.0], [31.0, 88.0], [32.0, 88.0]],
            3,
        )];
        let l = &mut m.paths[0];
        l.target = -1;
        l.flags = 1;
        m
    };
    let mut world = Spatial::new([120, 120]);
    world.insert(
        500,
        Placed {
            position: [31.0, 88.5, 0.0],
            radius: [0.5, 0.5],
            class: 3,
            player: 2,
            master: 70,
            obstruction_type: 2,
            buildable_on: 0,
            tall: true,
            speed: 0.0,
            unknown: None,
            obstruction: Some(Obstruction { kind: 2, class: 2 }),
        },
    );
    let mut me = Me {
        id: 9177,
        position: [30.0, 88.0, 0.0],
        group: -1,
        action_state: 0,
    };
    let goal = Goal {
        target: None,
        gap: FAR,
        to_end: FAR,
    };
    let mut m = walk();
    let _ = m.reached(&mut me, &mut world, None, goal, 0).unwrap();
    assert_eq!(
        m.paths[0].points,
        vec![[28.0, 0.0, 89.0], [30.0, 0.0, 88.0], [32.0, 0.0, 88.0]]
    );
    assert_eq!(m.paths[0].waypoint, 2);
    // the house in the manager's ignore list: the waypoint stays
    world.set_manager_ignore(&[500]);
    let mut m = walk();
    let _ = m.reached(&mut me, &mut world, None, goal, 0).unwrap();
    assert_eq!(m.paths[0].points.len(), 4);
    // the next waypoint out of reach: no query
    world.set_manager_ignore(&[]);
    let mut m = walk();
    m.paths[0].points[3] = [40.0, 0.0, 88.0];
    let _ = m.reached(&mut me, &mut world, None, goal, 0).unwrap();
    assert_eq!(m.paths[0].points.len(), 4);
}

/// MarvinBlue/426048731's 9175 in update 724..726: the step that reaches its waypoint marks it
/// reached; the next motion step puts the unit onto it, the level arrives and hands over to the
/// top level, which arrives too: the walk ends.
#[test]
fn a_reached_waypoint_ends_the_walk_at_the_end_of_the_path() {
    let mut m = builder_9177();
    m.paths[0].points = vec![[30.0, 0.0, 89.0], [29.5, 0.0, 89.5]];
    m.paths[1].points = vec![[30.0, 0.0, 89.0], [29.5, 0.0, 89.5]];
    for l in &mut m.paths {
        l.target = -1;
        l.range = 0.0;
    }
    m.forward = [
        -std::f32::consts::FRAC_1_SQRT_2,
        0.0,
        std::f32::consts::FRAC_1_SQRT_2,
    ];
    m.velocity.as_mut().unwrap().velocity = [0.0, 0.0, 0.8];
    let mut world = Spatial::new([120, 120]);
    m.moved_at = [29.51, 89.49, 0.0];
    let mut me = Me {
        id: 9175,
        position: [29.51, 89.49, 0.0],
        group: -1,
        action_state: 0,
    };
    assert_eq!(m.motion(&mut me, DT, &mut world, None), Ok(Motion::Moving));
    assert_eq!(m.paths[0].flags, 3);
    assert_eq!(m.motion(&mut me, DT, &mut world, None), Ok(Motion::Arrived));
    assert_eq!(me.position, [29.5, 89.5, 0.0]);
    assert_eq!(m.paths.len(), 1);
    assert_eq!(m.paths[0].state, 0);
}

/// MarvinBlue/426048731's 9175 in update 682: the path request of its `MoveTo` off the house
/// footprint, which the game saves as the top level of its path data.
#[test]
fn a_path_request_makes_the_top_level_from_the_unit_to_its_goal() {
    let mut m = builder_9177();
    m.request_path([30.0, 89.0, 0.0], [29.5, 89.5, 0.0], 0.0, -1, 16400);
    assert_eq!(m.paths.len(), 1);
    let top = &m.paths[0];
    assert_eq!(
        (
            top.level,
            top.path_flags,
            top.waypoint,
            top.state,
            top.target
        ),
        (2, 4, 1, 2, -1)
    );
    assert_eq!(top.points, vec![[30.0, 0.0, 89.0], [29.5, 0.0, 89.5]]);
    // the motion step then plans the path to walk, which the slice does not model
    let mut me = Me {
        id: 9175,
        position: [30.0, 89.0, 0.0],
        group: -1,
        action_state: 0,
    };
    assert_eq!(
        m.motion(&mut me, DT, &mut Spatial::new([120, 120]), None),
        Err(GenieError::unsupported(
            "a path request without the world's planners"
        ))
    );
}

#[test]
fn a_unit_with_no_speed_or_no_path_is_done() {
    let mut m = builder_9177();
    m.stats.speed = 0.0;
    let mut me = Me {
        id: 9177,
        position: [28.0, 89.0, 0.0],
        group: -1,
        action_state: 0,
    };
    assert_eq!(m.motion(&mut me, DT, &mut world(), None), Ok(Motion::Done));
    let mut m = builder_9177();
    m.paths.clear();
    assert_eq!(m.motion(&mut me, DT, &mut world(), None), Ok(Motion::Done));
}

/// The 4v4's hawk 30594 in update 2007: its level 0 arrived under a top level without a path
/// (a bird's); the parent takes the reached mark and the step ends there, the unit flying on.
#[test]
fn an_arrived_level_under_one_without_a_path_ends_the_step() {
    let mut m = builder_9177();
    m.velocity.as_mut().unwrap().velocity = [0.0, 0.0, 1.32];
    m.paths = vec![
        level(0, &[[28.0, 89.0], [30.0, 88.0]], 4),
        PathData {
            has_path: false,
            path_flags: 0,
            created: 0,
            ..level(2, &[], 0)
        },
    ];
    for l in &mut m.paths {
        l.target = -1;
    }
    let mut me = Me {
        id: 9177,
        position: [30.0, 88.0, 0.0],
        group: -1,
        action_state: 0,
    };
    assert_eq!(m.motion(&mut me, DT, &mut world(), None), Ok(Motion::Done));
    assert_eq!(m.paths.len(), 1);
    assert_eq!((m.paths[0].flags, m.paths[0].state), (1, 0));
    assert_eq!(m.velocity.as_ref().unwrap().velocity, [0.0, 0.0, 1.32]);
}

#[test]
fn a_human_walker_pauses_a_met_unit_of_an_older_human_order() {
    // 9177 meets 9175 of its own player: ordered at the same time, the lower id outranks
    // and 9177 walks on; once 9175's last human order is the older, 9177 pauses it for dt
    // more than its pause (slot 0x5f0)
    let mut w = world();
    let mut close = *w.get(9175).unwrap();
    close.position = [28.3, 88.9, 0.0];
    w.insert(9175, close);
    let me = Me {
        id: 9177,
        position: [28.0, 89.0, 0.0],
        group: -1,
        action_state: 0,
    };
    let walker = |human_order| Gait {
        human_order,
        ..Gait::default()
    };
    w.set_gait(9177, Some(walker(17000)));
    w.set_gait(
        9175,
        Some(Gait {
            pause: Some(0.0),
            ..walker(17000)
        }),
    );
    let mut walker_9177 = builder_9177();
    assert!(
        walker_9177
            .motion(&mut me.clone(), DT, &mut w, None)
            .is_ok()
    );
    assert_eq!(walker_9177.pauses, Vec::<(i32, f32)>::new());
    w.set_gait(
        9175,
        Some(Gait {
            pause: Some(0.0),
            ..walker(16000)
        }),
    );
    let mut walker_9177 = builder_9177();
    assert!(
        walker_9177
            .motion(&mut me.clone(), DT, &mut w, None)
            .is_ok()
    );
    assert_eq!(walker_9177.pauses, [(9175, 0.0 + DT)]);
}

/// `0x141150f00`'s pause loop between two members of one unit group (`+0x6d0`) ordered at
/// the same time: the group's member order decides (`FUN_1410175f0`), not the ids. 9177,
/// the group's first member, pauses 9175, the second; listed the other way round it walks
/// on; a group the world does not hold (`FUN_14152ff40`) pauses neither, as two units of no
/// group rank by id (the higher id 9177 walks on).
#[test]
fn two_members_of_one_group_rank_by_their_places_in_it() {
    let mut w = world();
    let mut close = *w.get(9175).unwrap();
    close.position = [28.3, 88.9, 0.0];
    w.insert(9175, close);
    let walker = |group| Gait {
        human_order: 17000,
        pause: Some(0.0),
        group,
        ..Gait::default()
    };
    w.set_gait(9177, Some(walker(Some(4))));
    w.set_gait(9175, Some(walker(Some(4))));
    let me = Me {
        id: 9177,
        position: [28.0, 89.0, 0.0],
        group: 4,
        action_state: 0,
    };
    let groups = |ids: &[i32]| crate::groups::UnitGroups {
        groups: vec![crate::groups::UnitGroup {
            id: 4,
            members: ids
                .iter()
                .map(|&id| crate::groups::Member {
                    id,
                    ..crate::groups::Member::default()
                })
                .collect(),
            ..crate::groups::UnitGroup::default()
        }],
        ..crate::groups::UnitGroups::default()
    };
    let pauses = |w: &Spatial, me: &Me, gs: Option<&crate::groups::UnitGroups>| {
        let m = builder_9177();
        let position = vector(m.moved_at);
        let waypoint = m.paths[0].point(1);
        let projected = m.projected(m.direction_to(position, waypoint), DT);
        m.avoid(me, (w, gs), &[9175], projected, DT, (16361, false))
            .unwrap()
            .pauses
    };
    let first = groups(&[9177, 9175]);
    assert_eq!(pauses(&w, &me, Some(&first)), [(9175, DT)]);
    let second = groups(&[9175, 9177]);
    assert_eq!(pauses(&w, &me, Some(&second)), []);
    let none = crate::groups::UnitGroups::default();
    assert_eq!(pauses(&w, &me, Some(&none)), []);
    w.set_gait(9175, Some(walker(None)));
    let loose = Me { group: -1, ..me };
    assert_eq!(pauses(&w, &loose, Some(&first)), []);
}

/// `FUN_14114e920` (slot 0x448): a unit's own waypoint is held as (x, height, y); a
/// coordinate less than 1e-7 above a whole number moves to the tile's middle, the height
/// stays; a NaN stays (`COMISS`, `JBE`). Slot 0x450 (`FUN_141070ce0`) empties the list.
#[test]
fn a_user_waypoint_on_a_tile_edge_moves_to_the_tile_s_middle() {
    let mut m = builder_9177();
    m.add_user_waypoint(25.0, 20.0, 1.0);
    m.add_user_waypoint(25.25, 20.000_000_1, 0.0);
    m.add_user_waypoint(f32::NAN, 7.75, 2.0);
    assert_eq!(m.user_waypoints[0], [25.5, 1.0, 20.5]);
    // 20.000_000_1 is 20.0 as an f32: on the edge
    assert_eq!(m.user_waypoints[1], [25.25, 0.0, 20.5]);
    assert!(m.user_waypoints[2][0].is_nan());
    assert_eq!(m.user_waypoints[2][1..], [2.0, 7.75]);
    m.clear_user_waypoints();
    assert_eq!(m.user_waypoints, Vec::<Vec3>::new());
}

/// `FUN_14114e410`: the unit's own waypoints come between its place and the goal and go;
/// a goal less than 1e-7 on the ground from the last of them is left out. A patrol's path
/// loops (flags bit 0), a far one goes back and forth (bit 1); an attack move (1) clears bit
/// 0 and sets `+0x7fb`, 0 clears it, none leaves it.
#[test]
fn a_patrol_s_path_request_walks_the_own_waypoints_and_takes_the_patrol_flags() {
    let mut m = builder_9177();
    m.user_waypoints = vec![[25.5, 0.0, 20.5], [26.5, 0.0, 20.5]];
    let ask = PathAsk {
        patrol: true,
        back_and_forth: true,
        attack_move: Some(false),
        ..PathAsk::default()
    };
    m.attack_moving = true;
    m.request_path_asked([20.0, 20.0, 0.0], [30.0, 20.0, 0.0], 0.0, -1, 500, ask);
    let top = &m.paths[0];
    assert_eq!(
        top.points,
        vec![
            [20.0, 0.0, 20.0],
            [25.5, 0.0, 20.5],
            [26.5, 0.0, 20.5],
            [30.0, 0.0, 20.0]
        ]
    );
    assert_eq!((top.path_flags, top.waypoint, top.state), (7, 1, 2));
    assert_eq!(m.user_waypoints, Vec::<Vec3>::new());
    assert!(!m.attack_moving);
    // the goal on the last waypoint is left out; an attack move does not loop
    m.user_waypoints = vec![[30.0, 0.0, 20.0]];
    let ask = PathAsk {
        patrol: true,
        attack_move: Some(true),
        ..PathAsk::default()
    };
    m.request_path_asked([20.0, 20.0, 0.0], [30.0, 20.0, 0.0], 0.0, -1, 500, ask);
    assert_eq!(
        m.paths[0].points,
        vec![[20.0, 0.0, 20.0], [30.0, 0.0, 20.0]]
    );
    assert_eq!(m.paths[0].path_flags, 4);
    assert!(m.attack_moving);
    // none leaves the attack move as it was
    m.request_path([20.0, 20.0, 0.0], [30.0, 20.0, 0.0], 0.0, -1, 500);
    assert!(m.attack_moving);
}

/// `FUN_14114e410` with a saved patrol path's waypoint (`param_9`): the path is the own
/// waypoints alone and the level walks on from that waypoint; without any, the unit's place
/// and the goal on the ground (height 0).
#[test]
fn a_resumed_patrol_s_path_is_its_waypoints_from_the_saved_one() {
    let mut m = builder_9177();
    m.user_waypoints = vec![[20.0, 0.0, 20.0], [25.5, 0.0, 20.5], [30.0, 0.0, 20.0]];
    let ask = PathAsk {
        patrol: true,
        back_and_forth: true,
        resume: 2,
        ..PathAsk::default()
    };
    m.request_path_asked([22.0, 21.0, 0.0], [30.0, 20.0, 1.0], 0.0, -1, 500, ask);
    assert_eq!(m.paths[0].points.len(), 3);
    assert_eq!((m.paths[0].waypoint, m.paths[0].path_flags), (2, 7));
    m.request_path_asked([22.0, 21.0, 0.0], [30.0, 20.0, 1.0], 0.0, -1, 500, ask);
    assert_eq!(
        m.paths[0].points,
        vec![[22.0, 0.0, 21.0], [30.0, 0.0, 20.0]]
    );
}

/// [`builder_9177`]'s place as `Me`, at `position`.
const fn me_9177(position: [f32; 3]) -> Me {
    Me {
        id: 9177,
        position,
        group: -1,
        action_state: 0,
    }
}

/// The path request's first exits ([`Mover::in_reach`]): a target within the walked level's
/// range (at least `RANGE_FLOOR` for a range of 0) puts the level in state 4; a unit on its
/// waypoint marks it reached (flag 1), and the waypoint itself (flag 2) without a target or
/// away from the target's footprint. Both go again; elsewhere the request goes on.
#[test]
fn a_request_in_reach_of_its_target_or_on_its_waypoint_goes_again() {
    let house = world_target();
    let me = me_9177([28.0, 89.0, 0.0]);
    let start = [28.0, 0.0, 89.0];
    let near = Goal {
        target: Some(&house),
        gap: 0.1,
        to_end: FAR,
    };
    let mut m = builder_9177();
    assert_eq!(m.in_reach(&me, near, start), Some(Step::Again));
    assert_eq!(m.paths[0].state, 4);
    let mut m = builder_9177();
    m.paths[0].range = 0.0;
    let floor = Goal {
        gap: RANGE_FLOOR,
        ..near
    };
    assert_eq!(m.in_reach(&me, floor, start), Some(Step::Again));
    assert_eq!(m.paths[0].state, 4);
    let far = Goal {
        target: None,
        gap: FAR,
        to_end: FAR,
    };
    let mut m = builder_9177();
    assert_eq!(m.in_reach(&me, far, start), None);
    assert_eq!((m.paths[0].state, m.paths[0].flags), (3, 0));
    let waypoint = [30.0, 0.0, 88.0];
    assert_eq!(m.in_reach(&me, far, waypoint), Some(Step::Again));
    assert_eq!(m.paths[0].flags, 3);
    let mut m = builder_9177();
    let at_house = me_9177(house.position);
    let on_house = Goal { gap: FAR, ..near };
    assert_eq!(m.in_reach(&at_house, on_house, waypoint), Some(Step::Again));
    assert_eq!(m.paths[0].flags, 1);
}

/// The request's try ([`Mover::spend_try`]): a try left goes on, one fewer; a looping top
/// path or an attack move spends none. The last try of level 0 under a top with more points
/// takes state 5; without more the level resets to state 6 and the unit stops (it gives up),
/// as does a level 2 on its last waypoint; a level 2 short of it gets its tries again
/// (`CONTINUE`) and goes on (flag 1).
#[test]
fn a_request_spends_a_try_and_gives_up_or_goes_on_without_more() {
    let mut m = builder_9177();
    assert_eq!(m.spend_try(), Ok(None));
    assert_eq!(m.paths[0].continue_counter, 9);
    let mut looping = builder_9177();
    looping.paths[1].path_flags |= 1;
    let mut attacking = builder_9177();
    attacking.attack_moving = true;
    for mut m in [looping, attacking] {
        assert_eq!(m.spend_try(), Ok(None));
        assert_eq!(m.paths[0].continue_counter, 10);
    }
    let last_try = || {
        let mut m = builder_9177();
        m.paths[0].continue_counter = 1;
        m
    };
    let mut m = last_try();
    m.paths[1].waypoint = 0;
    assert_eq!(m.spend_try(), Ok(Some(Step::Again)));
    assert_eq!(m.paths[0].state, 5);
    let mut m = last_try();
    assert_eq!(m.spend_try(), Ok(Some(Step::GaveUp)));
    assert_eq!((m.paths.len(), m.paths[0].state), (1, 6));
    assert!(m.speed_stopped);
    let mut m = last_try();
    m.paths[0].level = 2;
    assert_eq!(m.spend_try(), Ok(Some(Step::GaveUp)));
    let mut m = last_try();
    m.paths[0].level = 2;
    m.paths[0].waypoint = 0;
    assert_eq!(m.spend_try(), Ok(Some(Step::Again)));
    assert_eq!(
        (m.paths[0].flags, m.paths[0].continue_counter),
        (1, CONTINUE)
    );
}

/// The request's way from where [`builder_9177`] last moved to (28, 89) to its waypoint
/// (30, 88), toward `goal` within `range`.
fn way_9177(goal: Goal<'_>, range: f32) -> Way<'_> {
    Way {
        goal,
        include: &[],
        position: [28.0, 0.0, 89.0],
        waypoint: [30.0, 0.0, 88.0],
        distance: ground([30.0, 0.0, 88.0], [28.0, 0.0, 89.0]),
        radius: 0.2,
        level: 0,
        range,
        target_id: -1,
        trade_cart: false,
    }
}

/// A short path of no length ([`Mover::waypoint_in_range`]): within the range (at least
/// `RANGE_FLOOR` for a target and no range) the level takes state 4; beyond it the level goes
/// on (flag 1), but on the last waypoint of a path made this update it resets to state 6 and
/// the unit stops (it gives up).
#[test]
fn a_waypoint_in_range_ends_the_level_or_gives_up_on_a_fresh_path() {
    let point = |gap| Goal {
        target: None,
        gap,
        to_end: FAR,
    };
    let mut m = builder_9177();
    assert_eq!(
        m.waypoint_in_range(&way_9177(point(0.05), 0.1), 16360),
        Ok(Step::Again)
    );
    assert_eq!(m.paths[0].state, 4);
    let mut m = builder_9177();
    assert_eq!(
        m.waypoint_in_range(&way_9177(point(0.5), 0.1), 16000),
        Ok(Step::Again)
    );
    assert_eq!((m.paths[0].state, m.paths[0].flags), (3, 1));
    let mut m = builder_9177();
    assert_eq!(
        m.waypoint_in_range(&way_9177(point(0.5), 0.1), 16360),
        Ok(Step::GaveUp)
    );
    assert_eq!((m.paths.len(), m.paths[0].state), (1, 6));
    let house = world_target();
    let target = Goal {
        target: Some(&house),
        gap: RANGE_FLOOR,
        to_end: FAR,
    };
    let mut m = builder_9177();
    assert_eq!(
        m.waypoint_in_range(&way_9177(target, 0.0), 16360),
        Ok(Step::Again)
    );
    assert_eq!(m.paths[0].state, 4);
}

/// A unit without a footprint takes the straight way ([`Mover::straight_way`]) to its
/// waypoint, found; past the level's last waypoint the way has only its start.
#[test]
fn a_unit_without_a_footprint_takes_the_straight_way() {
    let m = builder_9177();
    let start = [28.0, 0.0, 89.0];
    let mut np = NewPath::new(0);
    m.straight_way(start, &mut np);
    assert_eq!(np.path.points, [start, [30.0, 0.0, 88.0]]);
    assert!(np.found);
    let mut m = builder_9177();
    m.paths[0].waypoint = 2;
    let mut np = NewPath::new(0);
    m.straight_way(start, &mut np);
    assert_eq!((np.path.points, np.path.length), (vec![start], 0.0));
}
