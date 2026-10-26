use super::*;
use crate::masters::BuildingMaster;
use crate::turn::{Player, Unit};

pub(crate) const TC: i16 = 109;
pub(crate) const VILLAGER: i16 = 83;
pub(crate) const ARCHER: i16 = 4;
const SHIP: i16 = 545;
const GATE: i16 = 88;

fn task(action_type: i16, object_group: i16) -> Task {
    Task {
        action_type,
        object_group,
        ..Task::default()
    }
}

/// A town centre (garrison type 11, room for 15), a villager and an archer that may enter
/// buildings, a transport ship (object flag 0, room for 5) and a gate.
fn garrison_masters() -> Masters {
    Masters::new([
        Master {
            id: TC,
            kind: 80,
            class: 52,
            hit_points: 2400,
            garrison: 15,
            building: Some(BuildingMaster {
                garrison_type: 11,
                ..BuildingMaster::default()
            }),
            ..Master::default()
        },
        Master {
            id: VILLAGER,
            kind: 70,
            class: 4,
            copy_id: VILLAGER,
            tasks: vec![task(GARRISON, 3), task(GARRISON, 20)],
            ..Master::default()
        },
        Master {
            id: ARCHER,
            kind: 70,
            class: 0,
            tasks: vec![task(7, 3)],
            ..Master::default()
        },
        Master {
            id: SHIP,
            kind: 70,
            class: 20,
            object_flags: 1,
            garrison: 1,
            ..Master::default()
        },
        Master {
            id: GATE,
            kind: 80,
            class: 39,
            hit_points: 2750,
            garrison: 0,
            ..Master::default()
        },
    ])
}

pub(crate) fn unit(id: i32, master: i16, hp: f32) -> Unit {
    let mut u = Unit {
        id,
        pasture: -1,
        my_pasture: -1,
        ..Unit::default()
    };
    u.life.master_id = master;
    u.life.hp = hp;
    u.life.state = 2;
    u
}

/// Gaia and three players; players 1 and 2 allied both ways, player 3 an enemy. Player 1
/// holds town centre 100, ship 200 and gate 300 (an annex of the town centre).
fn garrison_world() -> Turns {
    let mut players: Vec<Player> = (0..4).map(|_| Player::default()).collect();
    players[1].relations = vec![1, 0, 0, 3];
    players[2].relations = vec![1, 0, 0, 3];
    players[3].relations = vec![1, 3, 3, 0];
    let mut t = Turns::new(0, 0, 0, vec![0; 4], players);
    t.masters = (0..4).map(|_| garrison_masters()).collect();
    t.players[1].objects.push(unit(100, TC, 2400.0));
    t.players[1].objects.push(unit(200, SHIP, 100.0));
    let mut gate = unit(300, GATE, 2750.0);
    gate.annex_of = Some(100);
    t.players[1].objects.push(gate);
    t
}

/// UNLOAD's class groups (flag 2) as `FUN_14118a610` tests them; group 3 is the bit mask
/// `0x200801000801000` over classes below 58.
#[test]
fn the_unload_groups() {
    let group = |g| {
        (0..64)
            .filter(|&c| unload_group_takes(g, c))
            .collect::<Vec<_>>()
    };
    assert_eq!(group(1), [4, 19]);
    assert_eq!(group(3), [12, 23, 36, 47, 57]);
    assert_eq!(group(6), [18, 42, 43]);
    assert_eq!(group(4), [0i16; 0]);
}

#[test]
fn an_enter_walks_to_its_building_unless_it_is_in_reach() {
    // MarvinBlue/426048731's 9171 garrisons the town centre at (104, 47) from 3.5 tiles off:
    // set_state(4)'s MoveTo to it (FUN_140aa0df0), quiet, starts in state 3
    let tc = EnterTarget {
        state: 2,
        allied: true,
        room: true,
        position: [104.0, 47.0, 0.0],
        in_reach: false,
        building: true,
    };
    let never = || -> Result<([f32; 2], u8), GenieError> { panic!("a building's point") };
    let mut far = EnterAction::new(9149, [104.0, 47.0, 0.0], 1.0, 3);
    assert_eq!(
        far.walk_again(&tc, 3.5, 0.8, &mut never.clone()),
        Ok(Walking::Walk(None))
    );
    assert_eq!((far.state, far.move_to), (WALK, Some(MoveTo::Start)));
    // within ARRIVED of its footprint the MoveTo starts arrived (state 1)
    let mut close = EnterAction::new(9149, [104.0, 47.0, 0.0], 1.0, 3);
    assert_eq!(
        close.walk_again(&tc, 0.0, 0.8, &mut never.clone()),
        Ok(Walking::Walk(None))
    );
    assert_eq!(close.move_to, Some(MoveTo::Arrived));
    // 10049 of update 4409 stands within the task's range: it enters at once
    let mut near = EnterAction::new(9149, [104.0, 47.0, 0.0], 1.0, 3);
    let in_reach = EnterTarget {
        in_reach: true,
        ..tc
    };
    assert_eq!(
        near.walk_again(&in_reach, 0.6, 0.8, &mut never.clone()),
        Ok(Walking::Enter)
    );
    assert_eq!(near.move_to, None);
    // the reach is the footprints' squared gap against the range squared (COMISS/JBE: a NaN
    // gap is in); a target with speed within a range of 1 (COMISS/JC: a NaN range is not
    // tested) asks the tiles about the two
    let no_tiles = || -> Result<bool, GenieError> { panic!("no tile test") };
    assert_eq!(EnterAction::in_reach(1.0, 1.0, 0.0, no_tiles), Ok(true));
    assert_eq!(EnterAction::in_reach(1.0, 1.01, 0.0, no_tiles), Ok(false));
    assert_eq!(
        EnterAction::in_reach(1.0, f32::NAN, 0.0, no_tiles),
        Ok(true)
    );
    assert_eq!(EnterAction::in_reach(1.5, 0.5, 0.8, no_tiles), Ok(true));
    assert_eq!(
        EnterAction::in_reach(f32::NAN, 0.5, 0.8, no_tiles),
        Ok(true)
    );
    assert_eq!(EnterAction::in_reach(1.0, 0.5, 0.8, || Ok(true)), Ok(false));
    assert_eq!(EnterAction::in_reach(1.0, 0.5, 0.8, || Ok(false)), Ok(true));
    assert_eq!(
        EnterAction::in_reach(1.0, 0.5, f32::NAN, no_tiles),
        Ok(true)
    );
}

#[test]
fn a_transport_s_enter_walks_to_it_and_sends_it_where_they_meet() {
    // FUN_140a87320 case 4 on a transport: the point and code of FUN_140a85f30; code 1 sends
    // the transport there at its own height, code 0 does not; the MoveTo goes to the
    // transport within 0
    let ship = EnterTarget {
        state: 2,
        allied: true,
        room: true,
        position: [79.5, 92.5, 0.25],
        in_reach: false,
        building: false,
    };
    let mut enter = EnterAction::new(7420, [79.5, 92.5, 0.0], 1.0, 20);
    let walking = enter.walk_again(&ship, 6.0, 0.8, &mut || Ok(([82.5, 89.5], 1)));
    assert_eq!(walking, Ok(Walking::Walk(Some([82.5, 89.5, 0.25]))));
    assert_eq!(
        (enter.point, enter.move_to),
        ([82.5, 89.5, 0.0], Some(MoveTo::Start))
    );
    assert_eq!(enter.walk_range(false), 0.0);
    assert_eq!(enter.walk_range(true), 1.0);
    let walking = enter.walk_again(&ship, 6.0, 0.8, &mut || Ok(([80.0, 91.0], 0)));
    assert_eq!(walking, Ok(Walking::Walk(None)));
    assert_eq!(enter.point, [80.0, 91.0, 0.0]);
    // a unit without speed beyond ARRIVED: the MoveTo's state 13, not modelled
    assert!(
        enter
            .walk_again(&ship, 6.0, 0.0, &mut || Ok(([80.0, 91.0], 0)))
            .is_err()
    );
}

/// A transport target, room for the unit, 1.0 tiles off its point on y.
fn moved_ship(y: f32) -> EnterTarget {
    EnterTarget {
        state: 2,
        allied: true,
        room: true,
        position: [82.5, y, 0.0],
        in_reach: false,
        building: false,
    }
}

/// A walk of unit 1 without movement state or world, as [`EnterAction::update`] takes it.
fn idle_walk(world: &mut crate::map::spatial::Spatial) -> Walk<'_> {
    Walk {
        mover: None,
        me: crate::movement::Me {
            id: 1,
            position: [82.5, 87.5, 0.0],
            action_state: 4,
            group: -1,
        },
        dt: 0.5,
        world,
        plan: None,
    }
}

#[test]
fn a_transport_that_moved_off_the_point_makes_the_enter_walk_again() {
    // FUN_140a87a80 state 4: FUN_140a7bbf0 (not on the point) and FUN_140a7bd10 (more than
    // half a tile off: COMISS/JA) set_state(4); then the new MoveTo's update; the transport's
    // order waits on slot 0x3d8
    let mut world = crate::map::spatial::Spatial::new([100, 100]);
    let point = [82.5, 89.5, 0.0];
    let mut enter = EnterAction {
        move_to: Some(MoveTo::Arrived),
        ..EnterAction::new(7420, point, 1.0, 20)
    };
    let reached = std::cell::Cell::new(None);
    let step = enter
        .update(
            idle_walk(&mut world),
            Some(moved_ship(90.5)),
            (0.0, 0.8),
            || Ok(([82.5, 89.5], 1)),
            |_, to| {
                reached.set(Some(to));
                Ok(true)
            },
            (false, 0),
        )
        .unwrap();
    assert_eq!(reached.get(), Some([82.5, 89.5, 0.0]));
    assert_eq!(step.order, Some([82.5, 89.5, 0.0]));
    // the new MoveTo starts arrived (gap 0): code 1 out of reach waits (set_state(2))
    assert_eq!((enter.state, enter.timer, step.waits), (WAIT, 1.0, true));
    // within half a tile on each axis it does not walk again; a NaN is not off
    for y in [90.0, f32::NAN] {
        let mut enter = EnterAction {
            move_to: Some(MoveTo::Arrived),
            ..EnterAction::new(7420, point, 1.0, 20)
        };
        let step = enter
            .update(
                idle_walk(&mut world),
                Some(moved_ship(y)),
                (0.0, 0.8),
                || panic!("no walk again"),
                |_, _| panic!("no order"),
                (false, 0),
            )
            .unwrap();
        assert_eq!(step.order, None);
    }
    // a transport that cannot reach the point: set_state(13), not modelled
    let mut enter = EnterAction {
        move_to: Some(MoveTo::Arrived),
        ..EnterAction::new(7420, point, 1.0, 20)
    };
    let failed = enter.update(
        idle_walk(&mut world),
        Some(moved_ship(90.5)),
        (0.0, 0.8),
        || Ok(([82.5, 89.5], 1)),
        |_, _| Ok(false),
        (false, 0),
    );
    assert_eq!(failed, Err(UNREACHED));
}

/// The Enter's waiting `MoveTo` (state 2, after the motion step gave up): the world's dt
/// onto its timer (code 0: nothing else happens); at its delay it asks again.
#[test]
fn an_enter_s_waiting_move_to_asks_again_at_its_delay() {
    let mut world = crate::map::spatial::Spatial::new([100, 100]);
    let ship = moved_ship(89.5);
    let mut enter = EnterAction {
        move_to: Some(MoveTo::Waiting),
        move_wait: crate::movement::move_to::MoveWait {
            state: 1,
            timer: 0.0,
            delay: 1.0,
        },
        ..EnterAction::new(7420, ship.position, 1.0, 20)
    };
    let dt = idle_walk(&mut world).dt;
    let step = enter
        .update(
            idle_walk(&mut world),
            Some(ship),
            (0.0, 0.8),
            || panic!("no walk again"),
            |_, _| panic!("no order"),
            (true, 0),
        )
        .unwrap();
    assert_eq!(step, EnterStep::default());
    assert_eq!(
        (enter.move_to, enter.move_wait.timer),
        (Some(MoveTo::Waiting), dt)
    );
    enter.move_wait.timer = 1.0;
    let _ = enter
        .update(
            idle_walk(&mut world),
            Some(ship),
            (0.0, 0.8),
            || panic!("no walk again"),
            |_, _| panic!("no order"),
            (true, 0),
        )
        .unwrap();
    assert_eq!(
        (enter.move_to, enter.move_wait.timer),
        (Some(MoveTo::Start), 0.0)
    );
}

#[test]
fn a_walk_again_into_reach_enters_and_the_update_enters_again() {
    // the set_state(4) of a moved transport finds the unit in reach: FUN_140a867b0, then the
    // update's code 1 (no MoveTo) in reach enters again
    let mut world = crate::map::spatial::Spatial::new([100, 100]);
    let mut enter = EnterAction {
        move_to: Some(MoveTo::Arrived),
        ..EnterAction::new(7420, [82.5, 89.5, 0.0], 1.0, 20)
    };
    let ship = EnterTarget {
        in_reach: true,
        ..moved_ship(90.5)
    };
    let step = enter
        .update(
            idle_walk(&mut world),
            Some(ship),
            (0.0, 0.8),
            || panic!("in reach: no point"),
            |_, _| panic!("no order"),
            (false, 0),
        )
        .unwrap();
    assert_eq!((step.enter, step.again, enter.move_to), (true, true, None));
    // without the walk again, code 1 in reach enters once
    let mut enter = EnterAction {
        move_to: Some(MoveTo::Arrived),
        ..EnterAction::new(7420, [82.5, 90.5, 0.0], 1.0, 20)
    };
    let step = enter
        .update(
            idle_walk(&mut world),
            Some(ship),
            (0.0, 0.8),
            || panic!("no walk again"),
            |_, _| panic!("no order"),
            (false, 0),
        )
        .unwrap();
    assert_eq!((step.enter, step.again), (true, false));
}

#[test]
fn a_failed_move_walks_again_once_then_waits_and_a_wait_walks_again() {
    // code 3: +0x68 lets one set_state(4); the next waits a second (set_state(2)), whose end
    // walks again
    let mut world = crate::map::spatial::Spatial::new([100, 100]);
    let ship = moved_ship(89.5);
    let mut enter = EnterAction {
        move_to: Some(MoveTo::Failed),
        ..EnterAction::new(7420, [82.5, 89.5, 0.0], 1.0, 20)
    };
    let run = |enter: &mut EnterAction, world: &mut crate::map::spatial::Spatial| {
        enter.update(
            idle_walk(world),
            Some(ship),
            (2.0, 0.8),
            || Ok(([82.5, 89.5], 0)),
            |_, _| panic!("code 0: no order"),
            (false, 0),
        )
    };
    let step = run(&mut enter, &mut world).unwrap();
    assert_eq!(
        (enter.retry, enter.move_to, step.waits),
        (false, Some(MoveTo::Start), false)
    );
    enter.move_to = Some(MoveTo::Failed);
    let step = run(&mut enter, &mut world).unwrap();
    assert_eq!((enter.state, enter.move_to, step.waits), (WAIT, None, true));
    // the wait counts down by dt (0.5): at 0 it walks again
    let step = run(&mut enter, &mut world).unwrap();
    assert_eq!((enter.state, enter.timer, step.waits), (WAIT, 0.5, false));
    let _ = run(&mut enter, &mut world).unwrap();
    assert_eq!((enter.state, enter.move_to), (WALK, Some(MoveTo::Start)));
}

#[test]
fn a_meeting_point_is_the_shore_on_the_way_else_a_beach_under_the_unit() {
    // FUN_140a85f30 on an idle transport with room and speed
    let ship = MeetingTarget {
        room: true,
        speed: 1.1,
        state: 0,
        class: 20,
        position: [79.5, 92.5],
        action_point: Err(UNPLACED),
    };
    let unit = [83.5, 86.25];
    let no_beach = || panic!("no beach test");
    assert_eq!(
        meeting(&ship, unit, || Ok(Some([81, 89])), no_beach),
        Ok(([81.5, 89.5], 1))
    );
    // no shore on the way: a beach under the unit gives its place rounded (floorf of x+0.5)
    assert_eq!(
        meeting(&ship, unit, || Ok(None), || true),
        Ok(([84.0, 86.0], 1))
    );
    assert_eq!(
        meeting(&ship, unit, || Ok(None), || false),
        Ok(([79.5, 92.5], 0))
    );
    // no room, no speed (or a negative one; a NaN has some): its place
    let at_place = |t: MeetingTarget| meeting(&t, unit, || panic!("no landing"), || true);
    for t in [
        MeetingTarget {
            room: false,
            ..ship.clone()
        },
        MeetingTarget {
            speed: 0.0,
            ..ship.clone()
        },
        MeetingTarget {
            speed: -1.0,
            ..ship.clone()
        },
    ] {
        assert_eq!(at_place(t), Ok(([79.5, 92.5], 0)));
    }
    let nan = MeetingTarget {
        speed: f32::NAN,
        ..ship.clone()
    };
    assert_eq!(
        meeting(&nan, unit, || Ok(Some([81, 89])), no_beach),
        Ok(([81.5, 89.5], 1))
    );
    // moving (state 4): where its action goes, but a siege weapon at its place; another
    // state at its place
    let moving = MeetingTarget {
        state: 4,
        action_point: Ok([60.0, 70.0]),
        ..ship
    };
    assert_eq!(at_place(moving.clone()), Ok(([60.0, 70.0], 0)));
    assert_eq!(
        at_place(MeetingTarget {
            class: 13,
            ..moving.clone()
        }),
        Ok(([79.5, 92.5], 0))
    );
    assert_eq!(
        at_place(MeetingTarget { state: 7, ..moving }),
        Ok(([79.5, 92.5], 0))
    );
}

#[test]
fn the_landing_walk_steps_twice_per_tile_of_the_longer_axis() {
    // FUN_141157840 from (79, 92) to (83, 86): the y axis is longer (6), 12 steps of
    // (1/3, -1/2) in floats (79 + 3 steps is 80.00001); the tiles in the order they are
    // first reached
    let mut seen = Vec::new();
    let found = landing_tile([79, 92], [83, 86], |tile, _| {
        seen.push(tile);
        Ok(false)
    });
    assert_eq!(found, Ok(None));
    assert_eq!(
        seen,
        [
            [79, 91],
            [80, 90],
            [80, 89],
            [81, 89],
            [81, 88],
            [82, 87],
            [82, 86],
            [83, 86],
        ]
    );
    // the first that lands is the meeting tile, with the float place reached there
    let found = landing_tile([79, 92], [83, 86], |tile, [x, y]| {
        if tile == [81, 89] {
            assert_eq!([x, y], [81.000_015, 89.0]);
        }
        Ok(tile == [81, 89])
    });
    assert_eq!(found, Ok(Some([81, 89])));
    // equal axes: x is the longer (`<=`); one tile: none
    let mut seen = Vec::new();
    let _ = landing_tile([0, 0], [-2, 2], |tile, _| {
        seen.push(tile);
        Ok(false)
    });
    assert_eq!(seen, [[0, 0], [-1, 1], [-2, 2]]);
    assert_eq!(
        landing_tile([5, 5], [5, 5], |_, _| panic!("no step")),
        Ok(None)
    );
}

#[test]
fn the_reach_tile_test_counts_buildings_where_the_unit_cannot_stand() {
    // FUN_140a85c30 for range <= 1 and a target with speed: the box of the two tiles (one
    // tile and the next when it spans more than one), the two tiles left out
    let mut asked = Vec::new();
    let walled = walled_off(
        [10.5, 10.5],
        [11.5, 11.5],
        [100, 100],
        |x, y| {
            asked.push((x, y));
            Ok(false)
        },
        |x, y| Ok(usize::from((x, y) == (10, 11) || (x, y) == (11, 10))),
    );
    assert_eq!(walled, Ok(true));
    assert_eq!(asked, [(10, 11), (11, 10)]);
    // a tile the unit could stand on does not count
    let walled = walled_off(
        [10.5, 10.5],
        [11.5, 11.5],
        [100, 100],
        |x, y| Ok((x, y) == (11, 10)),
        |_, _| Ok(1),
    );
    assert_eq!(walled, Ok(false));
    // far apart: the first tile of the box and the next, within the map
    let mut asked = Vec::new();
    let _ = walled_off(
        [98.5, 2.5],
        [95.5, 9.5],
        [97, 100],
        |x, y| {
            asked.push((x, y));
            Ok(true)
        },
        |_, _| Ok(0),
    );
    assert_eq!(asked, [(95, 2), (95, 3), (96, 2), (96, 3)]);
}

#[test]
fn a_town_center_takes_villagers_infantry_cavalry_and_monks() {
    // a town center's garrison_type 11: bits 0, 1 and 3
    assert!(garrison_type_takes(11, 4, 83, true));
    assert!(garrison_type_takes(11, 6, 74, true));
    assert!(!garrison_type_takes(11, 12, 38, true));
    assert!(garrison_type_takes(11, 18, 125, true));
    // master 1811 is no infantry, but a monk
    assert!(garrison_type_takes(11, 6, MASTER_1811, true));
    assert!(!garrison_type_takes(2, 6, MASTER_1811, true));
    // a tower's 3 takes no monk; the task finder takes no class 42
    assert!(!garrison_type_takes(3, 18, 125, true));
    assert!(garrison_type_takes(8, 42, 1, true));
    assert!(!garrison_type_takes(8, 42, 1, false));
}

#[test]
fn a_villager_inside_a_town_center_adds_one_missile_to_its_volleys() {
    // MarvinBlue/426048731's town center 9149 (the tick oracle, update 4439): pierce 5 by its
    // volley missile 54 every 2 s; its 7 villagers made its firepower 1 + 7
    let arrow = Master {
        id: 54,
        kind: 60,
        weapons: vec![(3, 5)],
        ..Master::default()
    };
    let tc = Master {
        id: 109,
        kind: 80,
        class: 3,
        reload: 2.0,
        weapon_range: 6.0,
        weapons: vec![(3, 5)],
        volley_missile: Some(54),
        volley_fire: 1.0,
        max_volley: 11,
        ..Master::default()
    };
    let villager = Master {
        id: 83,
        kind: 70,
        class: 4,
        reload: 2.0,
        weapons: vec![(11, 3), (4, 3), (13, 6), (24, 0), (17, 0)],
        garrison_firepower: -2.5,
        ..Master::default()
    };
    let masters = Masters::new([arrow, tc.clone(), villager.clone()]);
    let rate = attack_rate(&tc, &masters, 0.0);
    assert_eq!(rate, 2.5);
    // a garrison firepower at most 0 is its magnitude
    assert_eq!(firepower(&villager, &masters, 0.0), 2.5);
    assert_eq!(share(&villager, &masters, 0.0, rate), Ok(1.0));
    assert_eq!(share(&villager, &masters, 0.0, 0.0), Ok(0.0));
    // one above 0 times the melee, pierce, class 31 and 38 amounts over the reload
    let armed = Master {
        garrison_firepower: 0.5,
        ..villager
    };
    assert_eq!(firepower(&armed, &masters, 0.0), 0.75);
}

/// A town centre's master as `a_villager_inside_a_town_center_adds_one_missile_to_its_volleys`
/// has it.
fn town_center() -> Master {
    Master {
        id: 109,
        kind: 80,
        class: 3,
        reload: 2.0,
        weapon_range: 6.0,
        weapons: vec![(3, 5)],
        volley_missile: Some(54),
        volley_fire: 1.0,
        ..Master::default()
    }
}

#[test]
fn an_attack_rate_needs_a_reload_and_a_range_that_compare_above_their_limits() {
    // FUN_14120dc10: COMISS reload, 0 and COMISS range, 1.0, each with JBE: an unordered
    // value gives no rate, as one at the limit does
    let arrow = Master {
        id: 54,
        kind: 60,
        weapons: vec![(3, 5), (4, 7), (38, 2)],
        ..Master::default()
    };
    let masters = Masters::new([arrow, town_center()]);
    assert_eq!(attack_rate(&town_center(), &masters, 0.0), 3.5);
    for (reload, range) in [(f32::NAN, 6.0), (2.0, f32::NAN), (0.0, 6.0), (2.0, 1.0)] {
        let tc = Master {
            reload,
            weapon_range: range,
            ..town_center()
        };
        assert_eq!(attack_rate(&tc, &masters, 0.0).to_bits(), 0.0f32.to_bits());
    }
    // classes 4 and 6 sum their own melee, pierce, 31 and 38 amounts, not the missile's
    let infantry = Master {
        class: 6,
        weapons: vec![(4, 4), (3, 1), (31, 2), (38, 1), (11, 9)],
        ..town_center()
    };
    assert_eq!(attack_rate(&infantry, &masters, 0.5), 8.0 / 1.5);
}

#[test]
fn a_volley_missile_slot_the_table_holds_is_taken_even_when_it_is_empty() {
    // FUN_14120e1a0: the volley missile when 0 <= id < the player's master count, else the
    // missile; a null slot or one without weapons adds nothing, with no fallback
    let missile = Master {
        id: 50,
        kind: 60,
        weapons: vec![(3, 6)],
        ..Master::default()
    };
    let volley = Master {
        id: 54,
        kind: 60,
        weapons: vec![(3, 2), (38, 1), (4, 9)],
        ..Master::default()
    };
    let unarmed = Master {
        id: 55,
        kind: 60,
        ..Master::default()
    };
    let archer = |volley_missile| Master {
        id: 4,
        kind: 70,
        reload: 2.0,
        weapons: vec![(3, 4)],
        missile: 50,
        garrison_firepower: 1.0,
        volley_fire: 3.0,
        volley_missile,
        ..Master::default()
    };
    let masters = Masters::new([missile, volley, unarmed]);
    // (2 + 1) * 2 extra missiles, plus its own 4, over its reload 2
    assert_eq!(firepower(&archer(Some(54)), &masters, 0.0), 5.0);
    // no volley missile: the missile's (6 * 2 + 4) / 2
    assert_eq!(firepower(&archer(None), &masters, 0.0), 8.0);
    // a volley missile without weapons adds nothing
    assert_eq!(firepower(&archer(Some(55)), &masters, 0.0), 2.0);
    // a null slot of the player's table (id 52 below its last master) adds nothing either
    assert_eq!(firepower(&archer(Some(52)), &masters, 0.0), 2.0);
    // a negative id is no slot: the missile's
    assert_eq!(firepower(&archer(Some(-2)), &masters, 0.0), 8.0);
}

#[test]
fn the_extra_missiles_truncate_and_multiply_as_the_game_does() {
    // FUN_14120e1a0: CVTTSS2SI R8D gives i32::MIN past the i32 range, and IMUL EAX, R8D
    // wraps: amount 1 adds -2^31, amount 2 adds 0
    let volley = |amount| Master {
        id: 54,
        kind: 60,
        weapons: vec![(3, amount)],
        ..Master::default()
    };
    let tower = Master {
        id: 79,
        kind: 70,
        reload: 1.0,
        garrison_firepower: 1.0,
        volley_fire: 1e10,
        volley_missile: Some(54),
        ..Master::default()
    };
    let masters = Masters::new([volley(1)]);
    assert_eq!(firepower(&tower, &masters, 0.0), -2_147_483_648.0);
    let masters = Masters::new([volley(2)]);
    assert_eq!(firepower(&tower, &masters, 0.0), 0.0);
    // the f32 truncation toward zero: 3.9 - 1 gives 2 extra missiles
    let masters = Masters::new([volley(3)]);
    let three = Master {
        volley_fire: 3.9,
        ..tower
    };
    assert_eq!(firepower(&three, &masters, 0.0), 6.0);
}

#[test]
fn a_share_is_at_least_zero_and_a_transforming_master_takes_its_other_form_as_its_floor() {
    // FUN_141213520: MAXSS(own / rate, floor), the floor 0.0 unless object flag 3 names (in
    // its upper half, +0x656) a master of type 70: that master's firepower over the rate
    let foot = Master {
        id: 1120,
        kind: 70,
        class: 0,
        reload: 2.0,
        weapons: vec![(3, 2)],
        garrison_firepower: 1.0,
        object_flags: 8 | (1122 << 16),
        ..Master::default()
    };
    let mounted = Master {
        id: 1122,
        kind: 70,
        class: 36,
        reload: 2.0,
        weapons: vec![(3, 6)],
        garrison_firepower: 1.0,
        ..Master::default()
    };
    let masters = Masters::new([foot.clone(), mounted.clone()]);
    // its own 1.0 against the other form's 3.0, over a rate of 2
    assert_eq!(share(&foot, &masters, 0.0, 2.0), Ok(1.5));
    // the other form's lower share leaves its own
    assert_eq!(share(&mounted, &masters, 0.0, 2.0), Ok(1.5));
    let reversed = Master {
        object_flags: 8 | (1120 << 16),
        ..mounted.clone()
    };
    assert_eq!(share(&reversed, &masters, 0.0, 2.0), Ok(1.5));
    // a named master not of type 70, or none, gives the floor 0.0; flag 3 clear names none
    let building = Master {
        kind: 80,
        ..mounted
    };
    let masters_80 = Masters::new([foot.clone(), building]);
    assert_eq!(share(&foot, &masters_80, 0.0, 2.0), Ok(0.5));
    let unflagged = Master {
        object_flags: 1122 << 16,
        ..foot.clone()
    };
    assert_eq!(share(&unflagged, &masters, 0.0, 2.0), Ok(0.5));
    // a negative quotient and a NaN one give the floor 0.0
    assert_eq!(share(&unflagged, &masters, 0.0, -2.0), Ok(0.0));
    assert_eq!(share(&unflagged, &masters, 0.0, f32::NAN), Ok(0.0));
    // _fdclass: a rate of ±0 gives +0.0; class 42 gives 1.0 whatever the rate
    assert_eq!(share(&foot, &masters, 0.0, -0.0).map(f32::to_bits), Ok(0));
    let relic_like = Master {
        class: WHOLE_SHARE_CLASS,
        ..foot
    };
    assert_eq!(share(&relic_like, &masters, 0.0, f32::NAN), Ok(1.0));
}

#[test]
fn a_garrison_task_compares_the_bytes_the_game_passes_widened() {
    // FUN_1415c9240: the task's ushorts +0xe and +0x10 against the byte arguments
    let tasks = [task(GARRISON, 20), task(GARRISON, -1)];
    // class 276 passes its low byte 20; a task of 276 never matches a byte
    assert!(has_garrison_task(&tasks[..1], GARRISON, 276));
    assert!(has_garrison_task(&tasks[..1], GARRISON, 20));
    assert!(!has_garrison_task(&[task(GARRISON, 276)], GARRISON, 276));
    // a class of -1 passes 0xff, which a task of 0xffff does not match
    assert!(!has_garrison_task(&tasks[1..], GARRISON, -1));
    assert!(has_garrison_task(&[task(GARRISON, 255)], GARRISON, -1));
    assert!(!has_garrison_task(&[task(7, 3)], GARRISON, 3));
}

#[test]
fn a_building_takes_a_unit_in_by_its_player_state_hit_points_room_and_garrison_type() {
    // FUN_14120eb10
    let mut t = garrison_world();
    let takes =
        |t: &Turns, b, p, m, others, unchecked| t.takes_in_as(b, p, m, others, unchecked).unwrap();
    assert!(takes(&t, 100, 1, VILLAGER, true, true));
    assert!(takes(&t, 100, 1, VILLAGER, true, false));
    // a master the player does not have, a building the world does not hold
    assert!(!takes(&t, 100, 1, 999, true, true));
    assert!(!takes(&t, 101, 1, VILLAGER, true, true));
    // an ally's building only with param_4 1; an enemy's never
    assert!(takes(&t, 100, 2, VILLAGER, true, true));
    assert!(!takes(&t, 100, 2, VILLAGER, false, true));
    assert!(!takes(&t, 100, 3, VILLAGER, true, true));
    // no garrison task on buildings; garrison type 11 has no bit for class 0
    assert!(!takes(&t, 100, 1, ARCHER, true, true));
    // the gate (an annex) asks its main building, but its own master must be of type 80
    assert!(takes(&t, 300, 1, VILLAGER, true, true));
    // a unit takes nothing in through this path: the archer is no transport
    assert!(!takes(&t, 200, 1, ARCHER, true, true));
    // hit points at most a fifth of the master's (COMISD, JBE: NaN too)
    for hp in [480.0, f32::NAN] {
        t.players[1].objects[0].life.hp = hp;
        assert!(!takes(&t, 100, 1, VILLAGER, true, true));
        assert!(!takes(&t, 300, 1, VILLAGER, true, true));
    }
    t.players[1].objects[0].life.hp = 480.5;
    assert!(takes(&t, 100, 1, VILLAGER, true, true));
    // state 2 only
    t.players[1].objects[0].life.state = 3;
    assert!(!takes(&t, 100, 1, VILLAGER, true, true));
    t.players[1].objects[0].life.state = 2;
    // full: only the room test (param_5 0) refuses
    for k in 0..15 {
        let mut v = unit(1000 + k, VILLAGER, 25.0);
        v.life.inside = Some(100);
        t.players[1].objects.push(v);
    }
    assert!(takes(&t, 100, 1, VILLAGER, true, true));
    assert!(!takes(&t, 100, 1, VILLAGER, true, false));
}

#[test]
fn a_transport_takes_a_combat_unit_with_a_task_on_its_class_and_room() {
    // FUN_14120eb10's object-flag-0 branch: FUN_1415c9240 with 3 and the transport's class
    let mut t = garrison_world();
    let takes = |t: &Turns, m, unchecked| t.takes_in_as(200, 1, m, true, unchecked);
    assert_eq!(takes(&t, VILLAGER, false), Ok(true));
    assert_eq!(takes(&t, ARCHER, true), Ok(false));
    let mut v = unit(1000, VILLAGER, 25.0);
    v.life.inside = Some(200);
    t.players[1].objects.push(v);
    assert_eq!(takes(&t, VILLAGER, false), Ok(false));
    assert_eq!(takes(&t, VILLAGER, true), Ok(true));
    // a unit of another type with no class flags 0x10002 (FUN_1411205e0) goes on as for a
    // building: the transport is of type 70, so it takes none
    assert_eq!(takes(&t, TC, true), Ok(false));
    // a unit of a type below 70 whose class has them (13: 0x3) takes the transport's branch
    let mut masters = garrison_masters();
    masters = Masters::new(masters.iter().cloned().chain([Master {
        id: 1500,
        kind: 60,
        class: 13,
        tasks: vec![task(GARRISON, 20)],
        ..Master::default()
    }]));
    t.masters = (0..4).map(|_| masters.clone()).collect();
    assert_eq!(takes(&t, 1500, true), Ok(true));
    // full: no room for one more
    assert_eq!(takes(&t, 1500, false), Ok(false));
}

#[test]
fn the_class_flags_are_the_images_table_by_class() {
    // FUN_1411205e0 with DAT_1439e37b0: villagers (4, 0x10163), infantry (6, 0x10103) and
    // siege (13, 0x3) garrison a transport; ships (20, 21, 22) do not
    for class in [0, 4, 6, 13, 18, 44, 58] {
        assert!(class_has_flags(class, TRANSPORTED), "class {class}");
    }
    for class in [2, 20, 21, 22, 52, 60, 63] {
        assert!(!class_has_flags(class, TRANSPORTED), "class {class}");
    }
    // other flags of the same words; a class outside the table has none (JS, JGE)
    assert!(class_has_flags(17, 0x10));
    assert!(class_has_flags(21, 0x100));
    assert!(!class_has_flags(-1, u32::MAX));
    assert!(!class_has_flags(64, u32::MAX));
    assert!(class_has_flags(61, u32::MAX));
}

#[test]
fn a_room_test_counts_the_unit_and_what_it_holds_or_one_for_none() {
    // FUN_1412169b0: slot 0x498 of the building plus that of the unit and 1
    let mut t = garrison_world();
    t.players[1].objects.push(unit(1000, VILLAGER, 25.0));
    let room = |t: &Turns, unit| {
        let (bp, b) = t.object(200).unwrap();
        t.has_room(bp, b, unit).unwrap()
    };
    assert!(room(&t, None));
    assert!(room(&t, Some(1000)));
    // a unit holding one more needs room for two
    let mut held = unit(1001, ARCHER, 25.0);
    held.life.inside = Some(1000);
    t.players[1].objects.push(held);
    assert!(!room(&t, Some(1000)));
    assert!(room(&t, None));
}

#[test]
fn unload_picks_the_units_its_flag_names_of_any_player() {
    // FUN_14118a610 on the units inside town centre 100: player 1's villager 1000, archer
    // 1001 and the class-42 1002, and player 2's (an ally's) villager 2000
    let mut t = garrison_world();
    let mut relic = garrison_masters();
    relic = Masters::new(relic.iter().cloned().chain([Master {
        id: 285,
        kind: 70,
        class: WHOLE_SHARE_CLASS,
        ..Master::default()
    }]));
    t.masters = (0..4).map(|_| relic.clone()).collect();
    for (p, id, m) in [
        (1, 1000, VILLAGER),
        (1, 1001, ARCHER),
        (1, 1002, 285),
        (2, 2000, VILLAGER),
        (3, 3000, ARCHER),
    ] {
        let mut u = unit(id, m, 25.0);
        u.life.inside = (p != 3).then_some(100);
        t.players[p].objects.push(u);
    }
    let picks = |t: &Turns, flag, param| {
        [1000, 1001, 1002, 2000]
            .into_iter()
            .filter(|&id| {
                let (p, u) = t.object(id).unwrap();
                t.unload_picks(p, u, flag, param).unwrap()
            })
            .collect::<Vec<i32>>()
    };
    assert_eq!(picks(&t, 0, -1), [1000, 1001, 2000]);
    assert_eq!(picks(&t, 1, 2), [2000]);
    assert_eq!(picks(&t, 2, 1), [1000, 2000]);
    assert_eq!(picks(&t, 2, 6), [1002]);
    assert_eq!(picks(&t, 3, 1001), [1001]);
    // flag 4: the named object's master (a villager names every villager of its player)
    assert_eq!(picks(&t, 4, 1000), [1000]);
    assert_eq!(picks(&t, 4, 2000), [2000]);
    assert_eq!(picks(&t, 4, 1001), [1001]);
    assert_eq!(picks(&t, 4, 9999), [0i32; 0]);
    // flag 5: every unit but it and class 42, of its player or with no named object
    assert_eq!(picks(&t, 5, 1001), [1000]);
    assert_eq!(picks(&t, 5, 9999), [1000, 1001, 2000]);
    // flag 6: not of its master, never class 42, a villager not for a villager
    assert_eq!(picks(&t, 6, 1001), [1000]);
    assert_eq!(picks(&t, 6, 1000), [1001]);
    assert_eq!(picks(&t, 6, 3000), [0i32; 0]);
    assert_eq!(picks(&t, 7, 1000), [0i32; 0]);
}

/// A unit's movement as the spot test reads it: terrain restriction 0, no speed.
fn standing() -> crate::movement::MoveStats {
    crate::movement::MoveStats {
        fly_mode: 0,
        speed: 0.0,
        walk_speed: 0.0,
        run_speed: 0.0,
        stand_speed: 0.0,
        turn_rate: 0.0,
        turn_radius: 0.0,
        turn_radius_speed: f32::MAX,
        radius: [0.2, 0.2, 2.0],
        restriction: 0,
        restriction_mask: 255,
        turn_time: 0.0,
        move_algorithm: 0,
        min_collision: 0.0,
        fast_turn: false,
        facets: None,
        weapon_range: 0.0,
    }
}

/// [`garrison_world`] on a flat 20×20 map every player sees, the town centre at (10, 10)
/// with its footprint on the map, the villager master able to stand beside it.
pub(crate) fn let_out_world() -> Turns {
    sized_world(20, 20)
}

/// [`let_out_world`] on a map of `w` × `h` tiles.
pub(crate) fn sized_world(w: i32, h: i32) -> Turns {
    let mut t = garrison_world();
    let mut masters = garrison_masters();
    if let Some(tc) = masters.get_mut(TC) {
        tc.radius = [2.0, 2.0];
        tc.tall = true;
        tc.volley_fire = 1.0;
        // no stack unit, head or annexes: the tracker files it under its own id
        if let Some(b) = tc.building.as_mut() {
            b.stack_unit = -1;
            b.head_unit = -1;
            b.annexes = [-1; 4];
        }
    }
    if let Some(v) = masters.get_mut(VILLAGER) {
        v.radius = [0.2, 0.2];
        v.tall = true;
        v.moving = Some(standing());
    }
    t.masters = (0..4).map(|_| masters.clone()).collect();
    let n = (w * h) as usize;
    t.spatial = crate::map::spatial::Spatial::new([w, h]);
    let mut terrain = crate::map::terrain::Terrain::new(
        w,
        h,
        vec![(0, 255, -1); n],
        vec![
            crate::map::terrain::TerrainKind {
                shown: true,
                flags: 0,
                forest: false,
            };
            1
        ],
        vec![vec![1.0]],
    );
    terrain.set_elevations(vec![0; n]);
    t.spatial.terrain = Some(terrain);
    let seen = (0..4).fold(0u64, |b, p| b | (1 << p) | (1 << (32 + p)));
    t.spatial.sight = Some(crate::vision::visibility::Visibility::new(
        w,
        h,
        (0..4)
            .map(|_| crate::vision::visibility::VisibleMap {
                counts: vec![0; n],
                explored: 0,
            })
            .collect(),
        vec![seen; n],
        Vec::new(),
        0,
        vec![false; 4],
    ));
    let tc = &mut t.players[1].objects[0];
    tc.position = [10.0, 10.0, 0.0];
    tc.firepower = 1.0;
    t.spatial.insert(
        100,
        crate::map::spatial::Placed {
            position: [10.0, 10.0, 0.0],
            radius: [2.0, 2.0],
            class: 52,
            player: 1,
            master: TC,
            obstruction_type: 2,
            buildable_on: 0,
            tall: true,
            speed: 0.0,
            unknown: None,
            obstruction: Some(crate::map::spatial::Obstruction { kind: 2, class: 2 }),
        },
    );
    t
}

/// `FUN_1411921d0`, the enter itself: a member of a unit group (`+0x6d0`) leaves the group
/// the world holds (removeUnit with the building and 0, `FUN_14102c160`) and stands (slot
/// 0x560) between leaving the mesh and joining the building.
/// A new unit whose building's gather point is the building itself (`FUN_140ace9c0`): with
/// fewer than 10 inside, or fewer than its capacity, its slot 0x1a0 forced (1:
/// `FUN_1411921d0` passes over the room test, slot 0x198) takes it in at once: no notify,
/// no state change, no move; the building's sight takes its circle of 5 and the villager's
/// town-bell record its work.
#[test]
fn a_gather_point_on_its_own_building_takes_the_new_unit_in() {
    let mut t = let_out_world();
    walker(&mut t, 1, 1000, VILLAGER, [12.5, 10.0]);
    let idx = t.players[1].objects.len() - 1;
    let state = t.players[1].objects[idx].life.state;
    let g = crate::turn::GatherPoint {
        point: [10.0, 10.0, 0.0],
        target: 100,
        target_master: TC,
    };
    t.gather(1, idx, g, 100, true, true).unwrap();
    let v = &t.players[1].objects[idx];
    assert_eq!(
        (
            v.life.inside,
            v.life.state,
            v.move_to.is_some(),
            v.enter.is_some()
        ),
        (Some(100), state, false, false)
    );
    assert_eq!(v.town_bell.work.target, 100);
    assert_eq!(t.object(100).unwrap().1.garrison, [1000]);
}

#[test]
fn a_grouped_unit_that_enters_a_building_leaves_its_group() {
    let mut t = let_out_world();
    let mut v = unit(400, VILLAGER, 25.0);
    v.position = [12.5, 10.0, 0.0];
    let mut walker = crate::gathering::hunt::tests::walker();
    walker.stats = standing();
    v.mover = Some(walker);
    v.group.id = 7;
    v.town_bell.flag = true;
    t.players[1].objects.push(v);
    t.spatial.insert(
        400,
        crate::map::spatial::Placed {
            position: [12.5, 10.0, 0.0],
            radius: [0.2, 0.2],
            class: 4,
            player: 1,
            master: VILLAGER,
            obstruction_type: 0,
            buildable_on: 0,
            tall: true,
            speed: 0.0,
            unknown: None,
            obstruction: None,
        },
    );
    let member = |id| crate::group::Member {
        id,
        ..crate::group::Member::default()
    };
    t.groups.groups.push(crate::group::UnitGroup {
        id: 7,
        owner: 1,
        members: vec![member(401), member(400)],
        ..crate::group::UnitGroup::default()
    });
    let idx = t.players[1].objects.len() - 1;
    let garrison = Task {
        work_range: 1.0,
        ..task(GARRISON, 3)
    };
    t.start_enter(1, idx, 100, &garrison, [10.0, 10.0, 0.0], false)
        .unwrap();
    let v = &t.players[1].objects[idx];
    assert_eq!((v.life.inside, v.group.id), (Some(100), -1));
    assert!(v.mover.as_ref().unwrap().speed_stopped);
    let members: Vec<i32> = t.groups.groups[0].members.iter().map(|m| m.id).collect();
    assert_eq!(members, [401]);
}

/// Puts unit `id` of player `p` and master `m` inside the town centre.
pub(crate) fn inside_tc(t: &mut Turns, p: usize, id: i32, m: i16) {
    let mut u = unit(id, m, 25.0);
    u.position = [10.0, 10.0, 0.0];
    u.life.inside = Some(100);
    t.players[p].objects.push(u);
    t.spatial.insert(
        id,
        crate::map::spatial::Placed {
            position: [10.0, 10.0, 0.0],
            radius: [0.2, 0.2],
            class: 4,
            player: p,
            master: m,
            obstruction_type: 0,
            buildable_on: 0,
            tall: true,
            speed: 0.0,
            unknown: None,
            obstruction: None,
        },
    );
    t.spatial.go_inside(id, [10.0, 10.0, 0.0]);
    t.players[1].objects[0].garrison.push(id);
}

#[test]
fn back_to_work_lets_an_allys_villager_out_toward_its_record() {
    // FUN_141625590 -> FUN_1412081e0 -> FUN_1411e7220 on player 2's villager 2000 inside
    // player 1's town centre: its own player's master and orders, the town bell cleared
    let mut t = let_out_world();
    inside_tc(&mut t, 2, 2000, VILLAGER);
    let v = &mut t.players[2].objects[0];
    v.town_bell.flag = true;
    v.town_bell.work = crate::town_bell::BackToWork {
        point: [4.5, 4.5],
        ..crate::town_bell::BackToWork::NONE
    };
    // the building's last unit inside: the volleys go back to the master's volley fire
    t.players[1].objects[0].firepower = 2.0;
    t.go_back_to_work(1, &[100]).unwrap();
    let v = &t.players[2].objects[0];
    assert_eq!(v.life.inside, None);
    assert!(!v.town_bell.flag);
    assert_eq!(t.players[1].objects[0].garrison.len(), 0);
    assert_eq!(t.players[1].objects[0].firepower, 1.0);
    assert!(t.spatial.get_known(2000).unwrap().is_some());
    // FUN_141217910's reset: COMISS volley, firepower; JBE keeps it while units stay inside
    // and the firepower is not below the volley fire (a NaN one too)
    let mut t = let_out_world();
    inside_tc(&mut t, 1, 1000, VILLAGER);
    inside_tc(&mut t, 2, 2000, VILLAGER);
    t.players[2].objects[0].town_bell.work.point = [4.5, 4.5];
    t.players[1].objects[0].firepower = f32::NAN;
    assert!(t.back_to_work(Some((1, 0)), 2000).unwrap());
    assert!(t.players[1].objects[0].firepower.is_nan());
    assert_eq!(t.players[1].objects[0].garrison, [1000]);
    t.players[1].objects[0].firepower = 0.5;
    t.object_mut(1000).unwrap().1.town_bell.work.point = [4.5, 15.5];
    assert!(t.back_to_work(Some((1, 0)), 1000).unwrap());
    assert_eq!(t.players[1].objects[0].firepower, 1.0);
    // a unit not a villager stays inside
    let mut t = let_out_world();
    inside_tc(&mut t, 2, 2001, ARCHER);
    t.go_back_to_work(1, &[100]).unwrap();
    assert_eq!(t.players[2].objects[0].life.inside, Some(100));
}

#[test]
fn a_unit_ais_test_takes_a_building_or_transport_that_takes_its_unit_in() {
    // FUN_140c40e00: type below 70 none, else FUN_14120eb10 with 1, 1
    let mut t = garrison_world();
    let mut masters = garrison_masters();
    masters = Masters::new(masters.iter().cloned().chain([Master {
        id: 59,
        kind: 60,
        class: 3,
        garrison: 5,
        ..Master::default()
    }]));
    t.masters = (0..4).map(|_| masters.clone()).collect();
    t.players[1].objects.push(unit(400, 59, 10.0));
    let takes = |t: &Turns, b, p, m| {
        let (bp, bu) = t.object(b).unwrap();
        t.ai_takes(bp, bu, p, m)
    };
    assert_eq!(takes(&t, 100, 1, VILLAGER), Ok(true));
    assert_eq!(takes(&t, 100, 2, VILLAGER), Ok(true));
    assert_eq!(takes(&t, 100, 1, ARCHER), Ok(false));
    // the transport (type 70) by its own branch
    assert_eq!(takes(&t, 200, 1, VILLAGER), Ok(true));
    // an object below type 70
    assert_eq!(takes(&t, 400, 1, VILLAGER), Ok(false));
}

/// A villager of player `p` with a unit AI and speed at (`x`, `y`).
pub(crate) fn walker(t: &mut Turns, p: usize, id: i32, m: i16, at: [f32; 2]) {
    let mut v = unit(id, m, 25.0);
    v.position = [at[0], at[1], 0.0];
    v.ai = Some(crate::unit_ai::UnitAi {
        ai_type: 8,
        current_order: -1,
        current_action: -1,
        current_target: -1,
        ..crate::unit_ai::UnitAi::default()
    });
    v.mover = Some(crate::movement::Mover {
        forward: [1.0, 0.0, 0.0],
        right: [0.0, 0.0, -1.0],
        velocity: None,
        paths: vec![],
        factor: 1.0,
        stats: crate::movement::MoveStats {
            speed: 0.8,
            ..standing()
        },
        waiting: 0,
        wait_delays: 0,
        attack_moving: false,
        last_move: 0,
        moved_at: [0.0; 3],
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
    });
    t.players[p].objects.push(v);
    t.spatial.insert(
        id,
        crate::map::spatial::Placed {
            position: [at[0], at[1], 0.0],
            radius: [0.2, 0.2],
            class: 4,
            player: p,
            master: m,
            obstruction_type: 0,
            buildable_on: 0,
            tall: true,
            speed: 0.8,
            unknown: None,
            obstruction: None,
        },
    );
}

/// A building of player 1 of master `m` (radius 2) at `at`, `annex_of` its main building.
pub(crate) fn building_at(t: &mut Turns, id: i32, m: i16, at: [f32; 2], annex_of: Option<i32>) {
    let mut b = unit(id, m, 2400.0);
    b.position = [at[0], at[1], 0.0];
    b.annex_of = annex_of;
    t.players[1].objects.push(b);
    t.spatial.insert(
        id,
        crate::map::spatial::Placed {
            position: [at[0], at[1], 0.0],
            radius: [2.0, 2.0],
            class: 52,
            player: 1,
            master: m,
            obstruction_type: 2,
            buildable_on: 0,
            tall: true,
            speed: 0.0,
            unknown: None,
            obstruction: Some(crate::map::spatial::Obstruction { kind: 2, class: 2 }),
        },
    );
}

/// Player 1's tracker files `ids` under master `m`'s key, done, in that order.
pub(crate) fn tracked(t: &mut Turns, m: i16, ids: &[i32]) {
    let k = t.tracker_key(1, m);
    for &id in ids {
        t.players[1].tracker.add(id, k);
        t.players[1].tracker.complete(id, k);
    }
}

#[test]
fn the_shelters_near_a_target_follow_its_owners_tracker_and_count_annexes_and_gates() {
    // FUN_14161e370 with +0x1e: the town centres' list (109) in the tracker's order; 102 is
    // beyond ten tiles, annex 104 is no building but pushes its count (one unit inside)
    let mut t = sized_world(40, 40);
    building_at(&mut t, 101, TC, [15.0, 10.0], None);
    building_at(&mut t, 102, TC, [30.0, 30.0], None);
    building_at(&mut t, 104, TC, [10.0, 15.0], Some(101));
    let mut inside = unit(500, VILLAGER, 25.0);
    inside.life.inside = Some(104);
    t.players[1].objects.push(inside);
    tracked(&mut t, TC, &[104, 102, 101, 100]);
    assert_eq!(
        t.shelters_near(1, 100, TC).unwrap(),
        Shelters {
            buildings: vec![101, 100],
            transports: Vec::new(),
            counts: vec![1, 0, 0],
        }
    );
    // the gates' list: no gate is a building, each pushes its count
    building_at(&mut t, 301, GATE, [10.0, 6.0], None);
    t.spatial.insert(300, *t.spatial.get(301).unwrap());
    tracked(&mut t, GATE, &[300, 301]);
    assert_eq!(
        t.shelters_near(1, 301, GATE).unwrap(),
        Shelters {
            buildings: Vec::new(),
            transports: Vec::new(),
            counts: vec![0, 0],
        }
    );
}

#[test]
fn a_garrison_near_its_target_reads_the_count_list_at_its_pairs_index() {
    // the town centres take one unit; the list [104, 101, 100] gives the buildings
    // [101, 100] and the counts [1, 0, 0]: the villager's nearest pair (101, index 0) reads
    // the annex's count 1, which is full, so it goes to 100 (index 1, count 0)
    let world = |with_annex: bool| {
        let mut t = sized_world(40, 40);
        for m in &mut t.masters {
            if let Some(tc) = m.get_mut(TC) {
                tc.garrison = 1;
            }
        }
        building_at(&mut t, 101, TC, [16.0, 10.0], None);
        building_at(&mut t, 104, TC, [10.0, 15.0], Some(101));
        let mut inside = unit(500, VILLAGER, 25.0);
        inside.life.inside = Some(104);
        t.players[1].objects.push(inside);
        let listed: &[i32] = if with_annex {
            &[104, 101, 100]
        } else {
            &[101, 100]
        };
        tracked(&mut t, TC, listed);
        walker(&mut t, 1, 1000, VILLAGER, [19.5, 10.5]);
        t.garrison_order(1, &[1000], 100, false, true).unwrap();
        t.object(1000).unwrap().1.enter.map(|e| e.target)
    };
    assert_eq!(world(true), Some(100));
    assert_eq!(world(false), Some(101));
}

#[test]
fn a_garrison_through_the_unit_ais_queues_their_enter_orders() {
    // FUN_141621220 with param_7: slot 0x30 into slot 0x28 with 0x2cd, issued by the unit
    let mut t = let_out_world();
    walker(&mut t, 1, 1000, VILLAGER, [5.5, 5.5]);
    t.garrison_order(1, &[1000], 100, true, false).unwrap();
    let v = t.object(1000).unwrap().1;
    assert_eq!(v.enter, None);
    assert_eq!(
        v.ai.as_ref().unwrap().orders,
        [crate::unit_ai::QueuedOrder {
            issuer: 1000,
            order: ENTER_ORDER,
            priority: 100,
            target: 100,
            target_owner: 1,
            point: [10.0, 10.0, 0.0],
            range: 0.0,
            shown: [10.0, 10.0],
            ready: true,
            target_type: i32::from(TC),
        }]
    );
}

/// [`let_out_world`] with ship 200 at (`x`, `y`) on the map.
fn ship_at(t: &mut Turns, at: [f32; 2]) {
    let ship = t.object_mut(200).expect("the ship").1;
    ship.position = [at[0], at[1], 0.0];
    t.spatial.insert(
        200,
        crate::map::spatial::Placed {
            position: [at[0], at[1], 0.0],
            radius: [0.5, 0.5],
            class: 20,
            player: 1,
            master: SHIP,
            obstruction_type: 0,
            buildable_on: 0,
            tall: true,
            speed: 1.0,
            unknown: None,
            obstruction: None,
        },
    );
}

#[test]
fn a_garrison_into_a_transport_pairs_the_units_whose_class_has_the_flags() {
    // FUN_141621220: a villager (class 4, flags 0x10163) with a garrison task on the ship's
    // class 20 pairs with it and, through the unit AIs, queues its enter order on it; the
    // archer (class 0) has no such task, and the ship itself (class 20) no class flags
    let mut t = let_out_world();
    ship_at(&mut t, [5.5, 12.5]);
    walker(&mut t, 1, 1000, VILLAGER, [5.5, 5.5]);
    walker(&mut t, 1, 1001, ARCHER, [6.5, 5.5]);
    t.garrison_order(1, &[1000, 1001, 200], 200, true, false)
        .unwrap();
    let orders = |t: &Turns, id| {
        t.object(id).unwrap().1.ai.as_ref().map(|ai| {
            ai.orders
                .iter()
                .map(|o| (o.order, o.target))
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(orders(&t, 1000), Some(vec![(ENTER_ORDER, 200)]));
    assert_eq!(orders(&t, 1001), Some(vec![]));
    // a transport beyond the town bell's reach (param_6 1) pairs nothing
    let mut t = sized_world(40, 40);
    ship_at(&mut t, [5.5, 35.5]);
    walker(&mut t, 1, 1000, VILLAGER, [5.5, 5.5]);
    let shelters = Shelters {
        buildings: Vec::new(),
        transports: vec![200],
        counts: vec![0],
    };
    t.shelter(1, &[1000], &shelters, true, true).unwrap();
    assert_eq!(orders(&t, 1000), Some(vec![]));
    t.shelter(1, &[1000], &shelters, false, true).unwrap();
    assert_eq!(orders(&t, 1000), Some(vec![(ENTER_ORDER, 200)]));
    // with no unit the transport is never asked
    assert_eq!(t.garrison_order(1, &[], 200, false, false), Ok(()));
}

/// [`sized_world`] 40 × 40 with a beach row (terrain 1, flags 0x10) at y = 17 and water
/// (terrain 2) from y = 18 on, every terrain passable for restriction 0; the villager's
/// garrison task on transports (class 20) has range 1; ship 200 (speed 1.1, type 70) at
/// `ship` and villager 1000 at `at`, each where it last moved to its place.
fn shore_world(at: [f32; 2], ship: [f32; 2]) -> Turns {
    let mut t = sized_world(40, 40);
    for m in &mut t.masters {
        if let Some(v) = m.get_mut(VILLAGER) {
            v.tasks = vec![
                task(GARRISON, 3),
                Task {
                    work_range: 1.0,
                    ..task(GARRISON, 20)
                },
            ];
        }
    }
    let tiles = (0..40 * 40)
        .map(|i| match i / 40 {
            17 => (1, 255, -1),
            y if 18 <= y => (2, 255, -1),
            _ => (0, 255, -1),
        })
        .collect();
    let kind = |flags| crate::map::terrain::TerrainKind {
        shown: true,
        flags,
        forest: false,
    };
    let mut terrain = crate::map::terrain::Terrain::new(
        40,
        40,
        tiles,
        vec![kind(0x20), kind(0x10), kind(1)],
        vec![vec![1.0, 1.0, 1.0]],
    );
    terrain.set_elevations(vec![0; 40 * 40]);
    t.spatial.terrain = Some(terrain);
    ship_at(&mut t, ship);
    let mut mover = crate::gathering::hunt::tests::walker();
    mover.stats = crate::movement::MoveStats {
        speed: 1.1,
        ..standing()
    };
    mover.moved_at = [ship[0], ship[1], 0.0];
    t.object_mut(200).unwrap().1.mover = Some(mover);
    walker(&mut t, 1, 1000, VILLAGER, at);
    t.object_mut(1000)
        .unwrap()
        .1
        .mover
        .as_mut()
        .unwrap()
        .moved_at = [at[0], at[1], 0.0];
    t
}

#[test]
fn a_villager_sent_into_a_ship_meets_it_at_the_shore_on_the_way() {
    // FUN_14161e370 (no +0x1e) -> FUN_141621220's transports -> slot 0x7e8 with filter 3 ->
    // the finder's LAB_1412796b4 -> the Enter: FUN_140a85f30 walks from the ship's tile
    // (5, 30) to the villager's (5, 12) and meets it on the beach tile (5, 17), code 1: the
    // ship reaches within 1 of (5, 17) (slot 0x3d8, the long pather's answer) and moves
    // there (slot 0x340: a MoveTo, running, range 0)
    let mut t = shore_world([5.5, 12.5], [5.5, 30.5]);
    t.garrison_order(1, &[1000], 200, false, false).unwrap();
    let ship = t.object(200).unwrap().1.move_to.unwrap();
    assert_eq!(
        (ship.goal, ship.range, ship.run, ship.target),
        (Some([5.5, 17.5, 0.0]), 0.0, true, -1)
    );
    let enter = t.object(1000).unwrap().1.enter.unwrap();
    assert_eq!(
        (enter.target, enter.point, enter.state, enter.move_to),
        (200, [5.5, 17.5, 0.0], WALK, Some(MoveTo::Start))
    );
    // the unit AI holds the enter order and action, the ship its target
    let ai = t.object(1000).unwrap().1.ai.clone().unwrap();
    assert_eq!(
        (ai.current_order, ai.current_action, ai.current_target),
        (ENTER_ORDER, ENTER_ACTION, 200)
    );
}

#[test]
fn a_villager_meets_a_moving_ship_where_it_goes() {
    // FUN_140a85f30 on a ship under a MoveTo (state 3): its goal, code 0, no order
    let mut t = shore_world([5.5, 12.5], [5.5, 30.5]);
    let ship = t.object_mut(200).unwrap().1;
    ship.move_to = Some(crate::movement::move_to::MoveToAction {
        run: false,
        state: MoveTo::Start,
        goal: Some([9.5, 19.5, 0.0]),
        target: -1,
        range: 0.0,
        wait: crate::movement::move_to::MoveWait::default(),
    });
    t.garrison_order(1, &[1000], 200, false, false).unwrap();
    let enter = t.object(1000).unwrap().1.enter.unwrap();
    assert_eq!(
        (enter.point, enter.move_to),
        ([9.5, 19.5, 0.0], Some(MoveTo::Start))
    );
    assert_eq!(
        t.object(200).unwrap().1.move_to.unwrap().goal,
        Some([9.5, 19.5, 0.0])
    );
    // the same order again: the work command finds the Enter's own task, whose slot 9 (the
    // base's, 0x140a7bf40) takes no new target, so a new Enter replaces it
    t.object_mut(1000).unwrap().1.enter.as_mut().unwrap().retry = false;
    t.garrison_order(1, &[1000], 200, false, false).unwrap();
    let enter = t.object(1000).unwrap().1.enter.unwrap();
    assert_eq!((enter.target, enter.retry), (200, true));
}

#[test]
fn a_villager_meets_a_sailing_transport_where_it_sails() {
    // FUN_140a85f30 on a ship whose Transport sails (state 11): the Transport's point (its
    // +0x38, +0x3c, FUN_140a7be40 and FUN_140a7be50), code 0
    let mut t = shore_world([5.5, 12.5], [5.5, 30.5]);
    let ship = t.object_mut(200).unwrap().1;
    ship.transport = Some(transport::TransportAction {
        state: transport::SAIL,
        point: [9.5, 19.5, 0.0],
        move_to: Some(MoveTo::Start),
        ..transport::TransportAction::new([5.5, 30.5, 0.0])
    });
    t.garrison_order(1, &[1000], 200, false, false).unwrap();
    let enter = t.object(1000).unwrap().1.enter.unwrap();
    assert_eq!(
        (enter.point, enter.move_to),
        ([9.5, 19.5, 0.0], Some(MoveTo::Start))
    );
}

/// A 30 × 20 map, water (terrain 1, flags 1) west of x = 15 and land (terrain 0, flags 8)
/// east of it, with the planners and zone maps of land units (restriction 0) and ships
/// (restriction 1). The ship 200 (a transport's unit AI, type 11; speed 1.1) floats at
/// `ship` with villager 1000 inside; the streams are seeded.
fn coast_world(ship: [f32; 2]) -> Turns {
    let mut t = sized_world(30, 20);
    let sails = crate::movement::MoveStats {
        speed: 1.1,
        stand_speed: 1.1,
        radius: [0.5, 0.5, 2.0],
        restriction: 1,
        ..standing()
    };
    for m in &mut t.masters {
        if let Some(s) = m.get_mut(SHIP) {
            s.terrain_restriction = 1;
            s.radius = [0.5, 0.5];
            s.tall = true;
            s.garrison = 5;
            s.moving = Some(sails);
        }
        if let Some(v) = m.get_mut(VILLAGER) {
            v.terrain_restriction = 0;
        }
    }
    let tiles: Vec<(u8, u8, i16)> = (0..30 * 20)
        .map(|i| {
            if i % 30 < 15 {
                (1, 255, -1)
            } else {
                (0, 255, -1)
            }
        })
        .collect();
    let kinds: Vec<u8> = tiles.iter().map(|t| t.0).collect();
    let kind = |flags| crate::map::terrain::TerrainKind {
        shown: true,
        flags,
        forest: false,
    };
    let restrictions = vec![vec![1.0, 0.0], vec![0.0, 1.0]];
    let mut terrain = crate::map::terrain::Terrain::new(
        30,
        20,
        tiles,
        vec![kind(8), kind(1)],
        restrictions.clone(),
    );
    terrain.set_elevations(vec![0; 30 * 20]);
    t.spatial.terrain = Some(terrain);
    t.pathing = crate::pathing::Pathing::new([30, 20], kinds, restrictions, 0);
    t.streams.seed_all(7);
    ship_at(&mut t, ship);
    let mut mover = crate::gathering::hunt::tests::walker();
    mover.stats = sails;
    mover.moved_at = [ship[0], ship[1], 0.0];
    let s = t.object_mut(200).unwrap().1;
    s.mover = Some(mover);
    s.ai = Some(crate::unit_ai::UnitAi {
        ai_type: 11,
        current_order: -1,
        current_action: -1,
        current_target: -1,
        ..crate::unit_ai::UnitAi::default()
    });
    s.garrison = vec![1000];
    walker(&mut t, 1, 1000, VILLAGER, ship);
    t.object_mut(1000).unwrap().1.life.inside = Some(200);
    t.spatial.go_inside(1000, [ship[0], ship[1], 0.0]);
    t
}

/// The first place about `center` within `radius` (`find_drop_location`'s draws on a copy of
/// the Object stream) where a unit of radius 0.2 stands on the land (x 15 and up) and clear
/// of the ship.
fn ashore(t: &mut Turns, center: [f32; 2], radius: f32) -> [f32; 2] {
    let mut r = t.streams.get(crate::math::rng::Stream::Object).clone();
    let gap = radius + (0.2 + 0.01);
    let along = |r: u32, c: f32| ((r as f32 * (radius + radius)) / 32767.0 - radius) + c;
    let mut place = [0.0; 2];
    loop {
        place = match (r.rand15() as i32 * 4) / 0x7fff {
            0 => [center[0] + gap, along(r.rand15(), center[1])],
            1 => [center[0] - gap, along(r.rand15(), center[1])],
            2 => [along(r.rand15(), center[0]), center[1] + gap],
            3 => [along(r.rand15(), center[0]), center[1] - gap],
            _ => place,
        };
        if 15.0 <= place[0] - 0.2 {
            return place;
        }
    }
}

/// UNLOAD of a ship off the shore (`FUN_14161f410`'s last branch): its unit AI takes the
/// unload order on itself at once (0x2d1 -> slot 0x118 -> slot 0x3a8 `FUN_14108d050`): a new
/// Transport (`FUN_140aa3ab0`) at the ship's place starts (`set_state(11)`), then its work at
/// the point: no tile of the point's zone (the land, on the ship's zone map) lies within 2 of
/// the ship's tile, so it sails there to unload (`set_state(4)`), toward the last water tile of
/// the planner's path to the point (`FUN_140f7c390`), its `MoveTo` starting; the unit AI holds
/// the unload's action value with the ship itself as its target.
#[test]
fn an_unload_command_sends_a_transport_toward_the_shore() {
    let mut t = coast_world([10.5, 10.5]);
    t.unload(1, &[200], [17.5, 10.5], 0, -1, false).unwrap();
    let ship = t.object(200).unwrap().1;
    let tr = ship.transport.expect("the Transport");
    assert_eq!(
        (tr.state, tr.mode, tr.point, tr.move_to, tr.retarget),
        (
            transport::SAIL_TO_UNLOAD,
            1,
            [14.5, 10.5, 0.0],
            Some(MoveTo::Start),
            false
        )
    );
    let ai = ship.ai.as_ref().unwrap();
    assert_eq!(
        (ai.current_order, ai.current_action, ai.current_target),
        (crate::ai_kinds::ORDER_UNLOAD, 0x26d, 200)
    );
    // the villager inside stays there
    assert_eq!(t.object(1000).unwrap().1.life.inside, Some(200));
    // a second unload: the immediate order stops the unit AI's action first (slot 0xb0, the
    // unit's stop tears the Transport down), so the command makes a new one alike
    t.unload(1, &[200], [17.5, 10.5], 0, -1, false).unwrap();
    let again = t.object(200).unwrap().1.transport.unwrap();
    assert_eq!((again.state, again.point), (tr.state, tr.point));
    // slot 0x3a8 with the Transport on top: "Telling existing action to work" at the ship's
    // own place, whose zone lies under the ship: it unloads there (set_state(6)), its MoveTo
    // gone
    let idx = t.players[1]
        .objects
        .iter()
        .position(|u| u.id == 200)
        .unwrap();
    t.transport_order(1, idx, [17.5, 10.5, 0.0]).unwrap();
    let tr = t.object(200).unwrap().1.transport.unwrap();
    assert_eq!(
        (tr.state, tr.point, tr.move_to),
        (transport::UNLOAD, [10.5, 10.5, 0.0], None)
    );
}

/// A ship beside the shore takes the unload order and unloads at once (`set_state(6)`: the
/// land within 2 of its tile, the point within 2 of its footprint); its unload
/// (`FUN_140aa3ba0` -> `FUN_141191b40`) puts the villager ashore east of it (the Object
/// stream's `find_drop_location`), out of the ship's list, its unit AI's state position the
/// spot; with none left inside the owner hears nothing, and the Transport idles.
#[test]
fn a_transport_beside_the_shore_puts_its_units_ashore() {
    let mut t = coast_world([14.5, 10.5]);
    t.unload(1, &[200], [15.5, 10.5], 0, -1, false).unwrap();
    let tr = t.object(200).unwrap().1.transport.unwrap();
    assert_eq!(
        (tr.state, tr.point, tr.move_to),
        (transport::UNLOAD, [15.5, 10.5, 0.0], None)
    );
    let spot = ashore(&mut t, [14.5, 10.5], 0.5);
    let idx = t.players[1]
        .objects
        .iter()
        .position(|u| u.id == 200)
        .unwrap();
    t.transport_unload(1, idx).unwrap();
    let v = t.object(1000).unwrap().1;
    assert_eq!(v.life.inside, None);
    assert_eq!(v.position, [spot[0], spot[1], 0.0]);
    assert_eq!(v.ai.as_ref().unwrap().state_position, spot);
    assert_eq!(
        t.spatial.get(1000).map(|p| p.position),
        Some([spot[0], spot[1], 0.0])
    );
    let ship = t.object(200).unwrap().1;
    assert_eq!(ship.garrison, [0; 0]);
    assert_eq!(ship.transport.map(|t| t.state), Some(transport::IDLE));
    // a move to a point: the idle Transport takes it (its slot 8, 0x140aa3e40) and sails
    // there, its MoveTo starting; no MoveTo of the unit's own
    let taken = t.move_to_point(1, idx, [5.5, 5.5, 0.0]).unwrap();
    let ship = t.object(200).unwrap().1;
    let tr = ship.transport.unwrap();
    assert!(taken);
    assert_eq!(
        (tr.state, tr.mode, tr.point, tr.move_to, ship.move_to),
        (
            transport::SAIL,
            0,
            [5.5, 5.5, 0.0],
            Some(MoveTo::Start),
            None
        )
    );
}

/// No drop location on the land: the point moves (the Action stream's "Unload" draws) until
/// a draw leaves the water, and the Transport sails there to unload (`set_state(4)`, `+0x68`):
/// its update's end leaves it so.
#[test]
fn a_transport_without_a_drop_location_sails_on_to_a_new_point() {
    let mut t = coast_world([14.5, 10.5]);
    // the villager's master stands nowhere (restriction 0 refuses both terrains)
    if let Some(terrain) = t.spatial.terrain.as_mut() {
        terrain.restrictions[0] = vec![0.0, 0.0];
    }
    t.unload(1, &[200], [15.5, 10.5], 0, -1, false).unwrap();
    let idx = t.players[1]
        .objects
        .iter()
        .position(|u| u.id == 200)
        .unwrap();
    let mut action = t.streams.get(crate::math::rng::Stream::Action).clone();
    t.transport_unload(1, idx).unwrap();
    assert_eq!(t.object(1000).unwrap().1.life.inside, Some(200));
    // the first draws: from the old point (15.5, 10.5), r / 0xccc tiles less 5
    let x = (action.rand15() / 0xccc) as f32 + 15.5 - 5.0;
    let y = (action.rand15() / 0xccc) as f32 + 10.5 - 5.0;
    let tr = t.object(200).unwrap().1.transport.unwrap();
    assert!(tr.retarget);
    assert_eq!(tr.state, transport::SAIL_TO_UNLOAD);
    if 15.0 <= x {
        // the first draw is on land: the MoveTo's point is the shore on the way to it
        assert_eq!(tr.move_to, Some(MoveTo::Start));
        assert!(tr.point[0] < 15.0, "{:?} from ({x}, {y})", tr.point);
    }
}

/// UNLOAD of the villager inside the ship (`FUN_14161f410` with a container of type 70): it
/// goes ashore about the ship within twice its radii (the Object stream's
/// `find_drop_location`, any zone), leaves it for the place and takes its stance again
/// there; the ship's own Transport stays untouched.
#[test]
fn a_unit_inside_a_transport_unloaded_goes_ashore_about_it() {
    let mut t = coast_world([14.5, 10.5]);
    let spot = ashore(&mut t, [14.5, 10.5], 1.0);
    t.unload(1, &[1000], [-1.0, -1.0], 0, -1, false).unwrap();
    let v = t.object(1000).unwrap().1;
    assert_eq!(v.life.inside, None);
    assert_eq!(v.position, [spot[0], spot[1], 0.0]);
    assert_eq!(v.ai.as_ref().unwrap().state_position, spot);
    assert!(!v.town_bell.flag);
    let ship = t.object(200).unwrap().1;
    assert_eq!(ship.garrison, [0; 0]);
    assert_eq!(ship.transport, None);
    // flag 3 names the unit inside: the ship lets it out the same way
    let mut t = coast_world([14.5, 10.5]);
    let spot = ashore(&mut t, [14.5, 10.5], 1.0);
    t.unload(1, &[200], [-1.0, -1.0], 3, 1000, false).unwrap();
    assert_eq!(t.object(1000).unwrap().1.position, [spot[0], spot[1], 0.0]);
    // a named unit elsewhere stays
    let mut t = coast_world([14.5, 10.5]);
    t.unload(1, &[200], [-1.0, -1.0], 3, 100, false).unwrap();
    assert_eq!(t.object(1000).unwrap().1.life.inside, Some(200));
}

#[test]
fn a_villager_beside_a_ship_enters_it_at_once() {
    // in reach (gap 0.8, range 1) of a ship with speed: the tiles (5, 19) and (5, 20) are
    // the two left out, no building between; FUN_140a867b0 -> FUN_1411921d0 -> the ship's
    // slot 0x1c0 (FUN_14120b790): inside at the ship's place; FUN_1412067f0 with 3 records
    // the Enter (the ship at its place, the enter action) and leaves the flag clear
    let mut t = shore_world([5.5, 19.0], [5.5, 20.5]);
    t.garrison_order(1, &[1000], 200, false, false).unwrap();
    let v = t.object(1000).unwrap().1;
    assert_eq!(v.life.inside, Some(200));
    assert_eq!([v.position[0], v.position[1]], [5.5, 20.5]);
    assert_eq!(v.enter.map(|e| e.state), Some(ENTERED));
    assert!(!v.town_bell.flag);
    assert_eq!(
        v.town_bell.work,
        crate::town_bell::BackToWork {
            target: 200,
            target_type: -1,
            action: ENTER_ACTION,
            target2: -1,
            point: [5.5, 20.5],
        }
    );
    assert_eq!(t.object(200).unwrap().1.garrison, [1000]);
    // the transport's count takes it: no room for another
    let (bp, b) = t.object(200).unwrap();
    assert!(!t.has_room(bp, b, None).unwrap());
}

#[test]
fn a_villager_enters_an_allys_ship_with_its_owners_masters() {
    // player 2's ship beside player 1's villager: FUN_14120eb10 lets an ally's transport take
    // it (both ways allied), and the enter adds the villager to the ship's list
    let mut t = shore_world([5.5, 19.0], [5.5, 20.5]);
    let k = t.players[1]
        .objects
        .iter()
        .position(|u| u.id == 200)
        .unwrap();
    let ship = t.players[1].objects.remove(k);
    t.players[2].objects.push(ship);
    let mut placed = *t.spatial.get(200).unwrap();
    placed.player = 2;
    t.spatial.insert(200, placed);
    t.garrison_order(1, &[1000], 200, false, false).unwrap();
    assert_eq!(t.object(1000).unwrap().1.life.inside, Some(200));
    assert_eq!(t.object(200).unwrap().1.garrison, [1000]);
    // an enemy's ship takes no one in (player 3)
    let mut t = shore_world([5.5, 19.0], [5.5, 20.5]);
    let k = t.players[1]
        .objects
        .iter()
        .position(|u| u.id == 200)
        .unwrap();
    let ship = t.players[1].objects.remove(k);
    t.players[3].objects.push(ship);
    t.garrison_order(1, &[1000], 200, false, false).unwrap();
    assert_eq!(t.object(1000).unwrap().1.enter, None);
}

#[test]
fn seek_shelter_sends_the_players_villagers_and_checked_units_to_its_buildings() {
    // FUN_140e6a3a0 -> FUN_141622450 -> FUN_141621220 with no transports: the command's
    // check (FUN_140e558b0) drops another player's unit unless players may order each
    // other's (+0x1074); a villager walks to the town centre's Enter
    let mut t = let_out_world();
    walker(&mut t, 1, 1000, VILLAGER, [5.5, 5.5]);
    walker(&mut t, 2, 2000, VILLAGER, [5.5, 6.5]);
    walker(&mut t, 1, 1001, ARCHER, [6.5, 5.5]);
    t.seek_shelter(1, &[1000, 2000, 1001, 4242]).unwrap();
    let enter = |t: &Turns, id| t.object(id).unwrap().1.enter.map(|e| e.target);
    assert_eq!(enter(&t, 1000), Some(100));
    // the ally's villager was dropped by the check; the archer is no villager
    assert_eq!((enter(&t, 2000), enter(&t, 1001)), (None, None));
    // with +0x1074 the ally's villager is the player's to send; its own player's town
    // centres are not asked, only the commanding player's
    let mut t = let_out_world();
    t.settings.cheats_enabled = true;
    walker(&mut t, 2, 2000, VILLAGER, [5.5, 6.5]);
    t.seek_shelter(1, &[2000]).unwrap();
    assert_eq!(enter(&t, 2000), Some(100));
    // a building that takes no villagers (garrison type bit 0) is not asked
    let mut t = let_out_world();
    for m in &mut t.masters {
        if let Some(b) = m.get_mut(TC).and_then(|tc| tc.building.as_mut()) {
            b.garrison_type = 2;
        }
    }
    walker(&mut t, 1, 1000, VILLAGER, [5.5, 5.5]);
    t.seek_shelter(1, &[1000]).unwrap();
    assert_eq!(enter(&t, 1000), None);
}

/// UNLOAD of a building (`FUN_14161f410`'s last branch): each unit inside comes out toward
/// the point, its orders gone and, a unit (type 70), no longer under the town bell; an id the
/// world no longer holds is passed over. A building with orders of its own to hand on is not
/// modelled.
#[test]
fn unload_of_a_building_lets_every_unit_inside_out() {
    let mut t = let_out_world();
    inside_tc(&mut t, 1, 1000, VILLAGER);
    inside_tc(&mut t, 1, 1001, VILLAGER);
    t.players[1].objects[0].garrison.insert(0, 1500);
    for id in [1000, 1001] {
        let v = t.object_mut(id).unwrap().1;
        v.town_bell.flag = true;
        v.ai = Some(crate::unit_ai::UnitAi {
            orders: vec![crate::unit_ai::QueuedOrder::default()],
            ..crate::unit_ai::UnitAi::default()
        });
    }
    t.unload(1, &[100], [4.5, 4.5], 0, -1, false).unwrap();
    assert_eq!(t.players[1].objects[0].garrison, [1500]);
    for id in [1000, 1001] {
        let v = t.object(id).unwrap().1;
        assert_eq!(v.life.inside, None, "{id}");
        assert!(!v.town_bell.flag, "{id}");
        assert!(v.ai.as_ref().unwrap().orders.is_empty(), "{id}");
    }
    let mut t = let_out_world();
    inside_tc(&mut t, 1, 1000, VILLAGER);
    t.players[1].objects[0].ai = Some(crate::unit_ai::UnitAi {
        building_orders: vec![crate::unit_ai::QueuedOrder::default()],
        ..crate::unit_ai::UnitAi::default()
    });
    assert!(t.unload(1, &[100], [4.5, 4.5], 0, -1, false).is_err());
}

#[test]
fn unload_lets_a_unit_out_of_the_building_it_is_inside() {
    // FUN_14161f410 on a unit inside a building: FUN_1411e7220 toward the point, +0x9dc
    // cleared, its orders gone; the building's own unload of the others is not run
    let mut t = let_out_world();
    inside_tc(&mut t, 1, 1000, VILLAGER);
    inside_tc(&mut t, 1, 1001, VILLAGER);
    let v = t.object_mut(1000).unwrap().1;
    v.town_bell.flag = true;
    v.ai = Some(crate::unit_ai::UnitAi::default());
    t.unload(1, &[1000], [4.5, 4.5], 0, -1, false).unwrap();
    let v = t.object(1000).unwrap().1;
    assert_eq!(v.life.inside, None);
    assert!(!v.town_bell.flag);
    assert_eq!(t.players[1].objects[0].garrison, [1001]);
}

/// `FUN_1411e7220` toward no point from a building without a gather point (nor a command
/// queue): the unit comes out, its unit AI stops (slot 0xb0 with 1: its stopAction), forgets
/// its target and takes its stance again at its new place (slot 0x1c0 with `+0x1a9`).
#[test]
fn a_unit_let_out_toward_nothing_stops_and_keeps_its_stance() {
    let mut t = let_out_world();
    inside_tc(&mut t, 1, 1000, VILLAGER);
    let v = t.object_mut(1000).unwrap().1;
    v.ai = Some(crate::unit_ai::UnitAi {
        current_action: 0x261,
        current_target: 77,
        stance: 1,
        ..crate::unit_ai::UnitAi::default()
    });
    assert_eq!(
        t.let_out(1, 0, 1000, [NO_POINT, NO_POINT], (None, false)),
        Ok(true)
    );
    let v = t.object(1000).unwrap().1;
    assert_eq!(v.life.inside, None);
    let ai = v.ai.as_ref().unwrap();
    assert_eq!(
        (
            ai.current_action,
            ai.current_target,
            ai.stance,
            ai.last_action
        ),
        (-1, -1, 1, 0x261)
    );
    assert_eq!(ai.state_position, [v.position[0], v.position[1]]);
}

/// `FUN_1411e7220` on a unit that is not inside (an effect's new unit the building could not
/// take in, `FUN_1416136f0`): slot 0x1a8 does nothing (the building's list and volleys
/// unchanged), slot 0x58 moves it from where it stands to a spot beside the building.
#[test]
fn a_unit_on_the_map_let_out_moves_beside_the_building() {
    let mut t = let_out_world();
    let tc_at = t.players[1].objects[0].position;
    walker(&mut t, 1, 1000, VILLAGER, [tc_at[0], tc_at[1]]);
    let firepower = t.players[1].objects[0].firepower;
    assert_eq!(
        t.let_out(1, 0, 1000, [NO_POINT, NO_POINT], (None, false)),
        Ok(true)
    );
    let v = t.object(1000).unwrap().1;
    assert_ne!([v.position[0], v.position[1]], [tc_at[0], tc_at[1]]);
    assert_eq!(t.spatial.get(1000).unwrap().position, v.position);
    let tc = &t.players[1].objects[0];
    assert_eq!((tc.garrison.len(), tc.firepower), (0, firepower));
}

#[test]
fn a_buildings_count_takes_relics_in_and_leaves_raiders_unmodelled() {
    // FUN_1411d0ce0 = FUN_141213500 (the +0x2f8 count and +0x9a8) less +0xc2c, which counts
    // the units of type 70 that came in with +0x9a0 set (FUN_1411cb850)
    let mut t = garrison_world();
    let mut masters = garrison_masters();
    masters = Masters::new(masters.iter().cloned().chain([Master {
        id: 285,
        kind: 20,
        class: 1,
        ..Master::default()
    }]));
    t.masters = (0..4).map(|_| masters.clone()).collect();
    for (p, id, m) in [(0, 900, 285), (1, 1000, VILLAGER), (2, 2000, VILLAGER)] {
        let mut u = unit(id, m, 25.0);
        u.life.inside = Some(100);
        t.players[p].objects.push(u);
    }
    assert_eq!(t.garrisoned(100), Ok(3));
    // a raider inside a building: its +0xc2c share is not kept
    t.object_mut(1000).unwrap().1.raider_countdown = 20;
    assert!(t.garrisoned(100).is_err());
    // a transport has no +0xc2c term
    let mut u = unit(3000, VILLAGER, 25.0);
    u.life.inside = Some(200);
    u.raider_countdown = 20;
    t.players[1].objects.push(u);
    assert_eq!(t.garrisoned(200), Ok(1));
}
