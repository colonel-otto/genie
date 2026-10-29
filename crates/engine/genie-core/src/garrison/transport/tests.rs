use super::*;

/// A map of zones by column (the tile's x over 10) and terrain flags by a closure; the shore
/// point the planner gives, recorded.
struct Coast {
    flags: fn(i16, i16) -> u8,
    near: bool,
    shore: Option<[f32; 2]>,
    asked: Vec<([f32; 2], [f32; 2])>,
}

impl Shore for Coast {
    fn zone(&mut self, x: i32, _y: i32) -> Result<u16, GenieError> {
        Ok(if x < 0 { 0 } else { (x / 10) as u16 + 1 })
    }

    fn near(&mut self, _zone: u8, _reach: f32) -> Result<bool, GenieError> {
        Ok(self.near)
    }

    fn flags(&self, x: i16, y: i16) -> u8 {
        (self.flags)(x, y)
    }

    fn shore_point(&mut self, from: [f32; 2], to: [f32; 2]) -> Option<[f32; 2]> {
        self.asked.push((from, to));
        self.shore
    }
}

fn coast(near: bool) -> Coast {
    Coast {
        flags: |x, _| if 20 <= x { LAND_FLAGS } else { 1 },
        near,
        shore: None,
        asked: Vec::new(),
    }
}

/// One update as the unit's runs it: the `MoveTo` (none walking here), then the state.
fn run(t: &mut TransportAction, s: &Ship, c: &mut Coast) -> Result<TransportStep, GenieError> {
    let walked = t.walk(None, PathAsk::default(), None)?;
    t.update(walked, s, c)
}

fn ship(loaded: bool) -> Ship {
    Ship {
        position: [12.5, 8.5, 0.0],
        radius: [0.5, 0.5],
        loaded,
        map: [40, 30],
        moves: true,
    }
}

#[test]
fn a_new_transport_sails_to_its_own_place_and_a_start_off_the_map_idles() {
    let s = ship(true);
    let mut c = coast(true);
    let mut t = TransportAction::new(s.position);
    assert_eq!((t.state, t.mode, t.retarget), (0, -1, false));
    // the list's add (first_in_stack with 1): set_state(11), its MoveTo to the shore point
    // on the way, none here, so the place itself
    let changed = t.start(true, &s, &mut c);
    assert_eq!(changed, Changed::default());
    assert_eq!((t.state, t.move_to), (SAIL, Some(MoveTo::Start)));
    assert_eq!(c.asked, [([12.5, 8.5], [12.5, 8.5])]);
    // a NaN point sails too (COMISS/JA, JBE)
    let mut nan = TransportAction::new([f32::NAN, 1.0, 0.0]);
    let _ = nan.start(true, &s, &mut c);
    assert_eq!(nan.state, SAIL);
    // below 0, or not first: idle with the standing sprite
    let mut off = TransportAction::new([3.0, -1.0, 0.0]);
    assert!(off.start(true, &s, &mut c).stands);
    assert_eq!((off.state, off.move_to), (IDLE, None));
    let mut later = TransportAction::new(s.position);
    let _ = later.start(false, &s, &mut c);
    assert_eq!(later.state, IDLE);
}

#[test]
fn a_sail_takes_the_shore_point_and_drops_its_old_move_to() {
    let s = ship(true);
    let mut c = coast(true);
    c.shore = Some([19.5, 8.5]);
    let mut t = TransportAction::new(s.position);
    let _ = t.start(true, &s, &mut c);
    t.point = [25.0, 8.0, 0.0];
    // set_state(4) again: the old MoveTo's teardown stops the unit, the new one starts
    let changed = t.set_state(SAIL_TO_UNLOAD, &s, &mut c);
    assert!(changed.stopped && !changed.stands);
    assert_eq!(t.point, [19.5, 8.5, 0.0]);
    assert_eq!(t.move_to, Some(MoveTo::Start));
    assert_eq!(c.asked[1], ([12.5, 8.5], [25.0, 8.0]));
    // a unit without speed: its quiet MoveTo fails at its start
    let still = Ship { moves: false, ..s };
    let _ = t.set_state(SAIL, &still, &mut c);
    assert_eq!(t.move_to, Some(MoveTo::Failed));
}

#[test]
fn a_move_sails_to_its_point_and_a_stop_idles() {
    let s = ship(true);
    let mut c = coast(true);
    c.shore = Some([19.5, 8.5]);
    let mut t = TransportAction {
        state: UNLOAD,
        mode: 1,
        ..TransportAction::new(s.position)
    };
    let changed = t.move_to([25.0, 8.0, 2.0], &s, &mut c);
    assert_eq!(changed, Changed::default());
    assert_eq!(
        (t.state, t.mode, t.point, t.move_to),
        (SAIL, 0, [19.5, 8.5, 2.0], Some(MoveTo::Start))
    );
    assert_eq!(c.asked, [([12.5, 8.5], [25.0, 8.0])]);
    let stopped = t.stop(&s, &mut c);
    assert!(stopped.stopped && stopped.stands);
    assert_eq!((t.state, t.move_to), (IDLE, None));
}

#[test]
fn a_work_order_near_its_zone_unloads_at_once_else_sails_there() {
    let s = ship(true);
    // within reach of the point's zone and 2 from the footprint (squared 4, at most 4)
    let mut c = coast(true);
    let mut t = TransportAction::new(s.position);
    let _ = t.work([15.0, 8.5, 1.0], true, &s, &mut c).unwrap();
    assert_eq!((t.state, t.mode, t.point), (UNLOAD, 1, [15.0, 8.5, 1.0]));
    // a bit further: sail there to unload, from the unit itself, else just sail
    let mut t = TransportAction::new(s.position);
    let _ = t.work([15.1, 8.5, 1.0], true, &s, &mut c).unwrap();
    assert_eq!((t.state, t.point), (SAIL_TO_UNLOAD, [15.1, 8.5, 1.0]));
    let mut t = TransportAction::new(s.position);
    let _ = t.work([15.1, 8.5, 1.0], false, &s, &mut c).unwrap();
    assert_eq!(t.state, SAIL);
    // not near the zone: sail
    let mut far = coast(false);
    let mut t = TransportAction::new(s.position);
    let _ = t.work([13.0, 8.5, 0.0], true, &s, &mut far).unwrap();
    assert_eq!(t.state, SAIL_TO_UNLOAD);
    // a point below 0 or NaN far away changes nothing but +0x34
    for bad in [[-1.0, 3.0, 0.0], [3.0, f32::NAN, 0.0]] {
        let mut t = TransportAction::new(s.position);
        assert_eq!(t.work(bad, true, &s, &mut far).unwrap(), Changed::default());
        assert_eq!((t.state, t.mode, t.point), (0, 1, s.position));
    }
    // a NaN gap stays NaN (MAXSS) and is not within (COMISS/JC): it sails
    let mut t = TransportAction::new(s.position);
    let nan_ship = Ship {
        radius: [f32::NAN, 0.5],
        ..s
    };
    let _ = t.work([12.5, 8.5, 0.0], true, &nan_ship, &mut c).unwrap();
    assert_eq!(t.state, SAIL_TO_UNLOAD);
}

#[test]
fn the_shore_search_looks_about_the_footprint_for_the_point_s_zone_or_land() {
    let mut c = coast(true);
    // the ship at x 12.5 with radius 0.5: tiles 10..=15; zone 2 is x 10..19
    let mut t = TransportAction::new(ship(true).position);
    t.point = [17.0, 3.0, 0.0];
    assert!(t.next_to_shore(&ship(true), &mut c).unwrap());
    // zone 3 (x 20..29) lies past x 15
    t.point = [25.0, 3.0, 0.0];
    assert!(!t.next_to_shore(&ship(true), &mut c).unwrap());
    // without a point: land (x 20 and up) is past x 15 too, but not from x 17.5
    t.point = [NONE, 0.0, 0.0];
    assert!(!t.next_to_shore(&ship(true), &mut c).unwrap());
    let east = Ship {
        position: [17.5, 8.5, 0.0],
        ..ship(true)
    };
    assert!(t.next_to_shore(&east, &mut c).unwrap());
    // the box stays on the map: a ship at the west edge reads no tile below 0
    let mut c = Coast {
        flags: |x, y| {
            assert!(0 <= x && 0 <= y, "({x}, {y})");
            0
        },
        ..coast(true)
    };
    let west = Ship {
        position: [0.5, 0.5, 0.0],
        ..ship(true)
    };
    assert!(!t.next_to_shore(&west, &mut c).unwrap());
}

#[test]
fn an_unload_out_of_reach_or_beside_no_shore_tells_the_owner_unless_over_water() {
    let s = ship(true);
    let mut t = TransportAction::new(s.position);
    t.state = UNLOAD;
    t.point = [25.0, 3.0, 0.0];
    t.retarget = true;
    // not near the point's zone: the owner hears it, +0x68 clears, the action idles
    let step = run(&mut t, &s, &mut coast(false)).unwrap();
    assert!(step.cannot_unload && !step.unload && step.changed.stands);
    assert_eq!((t.state, t.retarget, step.code), (IDLE, false, 0));
    // near, but zone 3 is not beside the footprint: over land at the point it tells the
    // owner, over water it waits silently
    let mut t = TransportAction {
        state: UNLOAD,
        point: [25.0, 3.0, 0.0],
        ..TransportAction::new(s.position)
    };
    let step = run(&mut t, &s, &mut coast(true)).unwrap();
    assert!(step.cannot_unload);
    let mut wet = coast(true);
    wet.flags = |_, _| 1;
    let mut t = TransportAction {
        state: UNLOAD,
        point: [25.0, 3.0, 0.0],
        ..TransportAction::new(s.position)
    };
    let step = run(&mut t, &s, &mut wet).unwrap();
    assert!(!step.cannot_unload && step.changed.stands);
    // empty: nothing to unload, the action idles
    let mut t = TransportAction {
        state: UNLOAD,
        point: [25.0, 3.0, 0.0],
        ..TransportAction::new(s.position)
    };
    let step = run(&mut t, &ship(false), &mut coast(true)).unwrap();
    assert!(!step.cannot_unload && !step.unload);
    assert_eq!(t.state, IDLE);
}

#[test]
fn an_unload_beside_the_shore_is_the_caller_s_then_idles_unless_it_sails_on() {
    let s = ship(true);
    let mut c = coast(true);
    let mut t = TransportAction {
        state: UNLOAD,
        point: [17.0, 3.0, 0.0],
        ..TransportAction::new(s.position)
    };
    let step = run(&mut t, &s, &mut c).unwrap();
    assert!(step.unload && !step.cannot_unload);
    assert_eq!(t.state, UNLOAD);
    assert!(t.unloaded(&s, &mut c).stands);
    assert_eq!(t.state, IDLE);
    // a retarget's set_state(4) stays
    t.state = SAIL_TO_UNLOAD;
    t.retarget = true;
    assert_eq!(t.unloaded(&s, &mut c), Changed::default());
    assert_eq!(t.state, SAIL_TO_UNLOAD);
}

#[test]
fn a_sail_ends_with_its_move_to_and_a_failed_one_returns_3() {
    let s = ship(true);
    let mut c = coast(true);
    // state 4 without a MoveTo: the list answers 1, so it unloads
    let mut t = TransportAction {
        state: SAIL_TO_UNLOAD,
        ..TransportAction::new(s.position)
    };
    let step = run(&mut t, &s, &mut c).unwrap();
    assert_eq!((t.state, step.code), (UNLOAD, 0));
    // state 11 with an ended MoveTo: idle, the MoveTo gone (its teardown stops the unit)
    let mut t = TransportAction {
        state: SAIL,
        move_to: Some(MoveTo::Arrived),
        ..TransportAction::new(s.position)
    };
    let step = run(&mut t, &s, &mut c).unwrap();
    assert_eq!(t.state, IDLE);
    assert!(step.changed.stopped && step.changed.stands);
    // a failed MoveTo (code 3) ends a sail to unload too
    let mut t = TransportAction {
        state: SAIL_TO_UNLOAD,
        move_to: Some(MoveTo::Failed),
        ..TransportAction::new(s.position)
    };
    let _ = run(&mut t, &s, &mut c).unwrap();
    assert_eq!(t.state, UNLOAD);
    // state 13: the unit hears it failed, the action idles and returns 3
    let mut t = TransportAction {
        state: FAILED,
        ..TransportAction::new(s.position)
    };
    let step = run(&mut t, &s, &mut c).unwrap();
    assert!(step.failed);
    assert_eq!((t.state, step.code), (IDLE, 3));
    // state 1 is done; an idle one does nothing
    t.state = DONE;
    assert_eq!(run(&mut t, &s, &mut c).unwrap().code, 1);
    t.state = IDLE;
    assert_eq!(run(&mut t, &s, &mut c).unwrap(), TransportStep::default());
}

/// `FUN_140a992b0` as a Transport runs it: a start asks for its path to the point (the
/// planner gives the walker its two paths), a move walks on; without the walk either is
/// an error, and a waiting `MoveTo` (only a villager's waits) is not a ship's.
#[test]
fn a_sail_s_move_to_asks_for_its_path_then_walks_on() {
    use crate::movement::tests::WalkRig;
    let sailing = |move_to| TransportAction {
        move_to,
        point: [35.5, 88.5, 0.0],
        ..TransportAction::new([28.0, 89.0, 0.0])
    };
    let ask = PathAsk::default();
    let mut rig = WalkRig::on_the_path();
    rig.mover_mut().paths.clear();
    let mut t = sailing(Some(MoveTo::Start));
    // its start shows its sprite before the motion step
    let graphic = crate::objects::sprite::Graphic {
        id: 3,
        frames: 10,
        facets: 16,
        speed_multiplier: 1.0,
        frame_duration: 0.1,
        replay_delay: 0.0,
        flags: 1,
    };
    let graphics = std::collections::BTreeMap::from([(3, graphic)]);
    let (mut sprite, mut rng) = (None, Well512::default());
    let mut sprites = crate::movement::move_to::Sprites {
        sprite: &mut sprite,
        rng: &mut rng,
        graphics: &graphics,
    };
    let shown = Some(sprites.shown(Some(3)).unwrap());
    assert_eq!(t.walk(Some(rig.walk(SAIL)), ask, shown), Ok(0));
    assert_eq!(t.move_to, Some(MoveTo::Moving));
    assert_eq!(rig.mover_mut().paths.len(), 2);
    assert_eq!(sprite.map(|s| s.graphic.id), Some(3));
    let mut rig = WalkRig::on_the_path();
    let mut t = sailing(Some(MoveTo::Moving));
    assert_eq!(t.walk(Some(rig.walk(SAIL)), ask, None), Ok(0));
    assert_eq!(t.move_to, Some(MoveTo::Moving));
    assert!(sailing(Some(MoveTo::Start)).walk(None, ask, None).is_err());
    assert!(sailing(Some(MoveTo::Moving)).walk(None, ask, None).is_err());
    let mut t = sailing(Some(MoveTo::Waiting));
    assert!(t.walk(Some(rig.walk(SAIL)), ask, None).is_err());
}

/// The sprite its `MoveTo` shows (`0x140aa42a0` gives `FUN_140aa0d30` one): its task's moving
/// sprite, without a task the master's walking sprite, for none the constructor's default; a
/// task the master lacks is an error.
#[test]
fn a_transport_s_move_to_shows_its_task_s_or_its_master_s_sprite() {
    use crate::objects::masters::{Master, Task, TaskId};
    let master = Master {
        move_sprite: 7,
        tasks: vec![
            Task {
                id: 4,
                sprites: [3, -1, -1, -1],
                ..Task::default()
            },
            Task {
                id: 5,
                ..Task::default()
            },
        ],
        ..Master::default()
    };
    let defaults = crate::movement::move_to::MoveDefaults {
        point: Some(20),
        object: Some(21),
    };
    let on = |task| TransportAction::on_task(task, [28.0, 89.0, 0.0]);
    assert_eq!(
        on(Some(TaskId(4))).move_shown(&master, defaults),
        Ok(Some(3))
    );
    assert_eq!(
        on(Some(TaskId(5))).move_shown(&master, defaults),
        Ok(Some(20))
    );
    assert_eq!(on(None).move_shown(&master, defaults), Ok(Some(7)));
    let walkless = Master {
        move_sprite: -1,
        ..master.clone()
    };
    assert_eq!(on(None).move_shown(&walkless, defaults), Ok(Some(20)));
    assert!(on(Some(TaskId(6))).move_shown(&master, defaults).is_err());
}

fn rng() -> Well512 {
    let mut r = Well512::default();
    r.set_seed(7);
    r
}

#[test]
fn a_retry_draws_about_the_old_point_until_it_leaves_the_water() {
    // the draws as the game makes them: x then y, each r / 0xccc tiles
    let mut copy = rng();
    let draws: Vec<u32> = (0..6).map(|_| copy.rand15() / RETRY_STEP).collect();
    let point = [20.0, 20.0];
    let at = |i: usize| {
        [
            draws[2 * i] as f32 + 20.0 - 5.0,
            draws[2 * i + 1] as f32 + 20.0 - 5.0,
        ]
    };
    // land (terrain 0) everywhere: the first draw leaves the water
    let mut r = rng();
    assert_eq!(
        retry_point(point, [40, 40], |_, _| 0, &mut r),
        (at(0), true)
    );
    // water 22, 23 and 1 keep drawing: the third draw's tile is land
    let first = [at(0), at(1)].map(|p| [p[0] as i16, p[1] as i16]);
    let mut r = rng();
    let (found, sails) = retry_point(
        point,
        [40, 40],
        |x, y| match [x, y] {
            t if t == first[0] => 0x17,
            t if t == first[1] => 1,
            _ => 0,
        },
        &mut r,
    );
    assert_eq!((found, sails), (at(2), true));
    assert_eq!(r, copy);
    // off the map the last terrain stays: all water, 100 draws, the last one taken
    let mut r = rng();
    let (_, sails) = retry_point(point, [40, 40], |_, _| 0x16, &mut r);
    assert!(!sails);
    assert_eq!(r.draws, rng().draws + 200);
}

#[test]
fn the_99th_draw_leaving_the_water_comes_too_late() {
    // water until the 99th draw: the loop breaks with no draws to spare
    let mut r = rng();
    let count = std::cell::Cell::new(0);
    let (_, sails) = retry_point(
        [20.0, 20.0],
        [400, 400],
        |_, _| {
            count.set(count.get() + 1);
            if count.get() < 99 { 0x16 } else { 0 }
        },
        &mut r,
    );
    assert!(!sails);
    // the 98th: one to spare
    let mut r = rng();
    count.set(0);
    let (_, sails) = retry_point(
        [20.0, 20.0],
        [400, 400],
        |_, _| {
            count.set(count.get() + 1);
            if count.get() < 98 { 0x16 } else { 0 }
        },
        &mut r,
    );
    assert!(sails);
}

#[test]
fn a_drop_location_is_drawn_on_a_side_past_both_footprints() {
    let around = Around {
        center: [10.0, 10.0],
        radius: [1.0, 1.0],
    };
    // the first draw's side and place, as the game computes them
    let mut copy = rng();
    let (r1, r2) = (copy.rand15(), copy.rand15());
    let side = (r1 as i32 * 4) / 0x7fff;
    let along = ((r2 as f32 * 2.0) / 32767.0 - 1.0) + 10.0;
    let gap = 1.0 + (0.5 + DROP_GAP);
    let expected = match side {
        0 => [10.0 + gap, along],
        1 => [10.0 - gap, along],
        2 => [along, 10.0 + gap],
        _ => [along, 10.0 - gap],
    };
    let mut last = None;
    let mut r = rng();
    let found = find_drop_location(
        around,
        [0.5, 0.5],
        [40, 40],
        &mut last,
        &mut r,
        &mut |_| Ok(true),
        &mut |_| Ok(true),
    )
    .unwrap();
    assert_eq!(found, Some(expected));
    assert_eq!(last, Some(expected));
    assert_eq!(r, copy);
    // nothing free: 50 tries, the last place kept for the next call
    let mut r = rng();
    let mut tries = 0;
    let none = find_drop_location(
        around,
        [0.5, 0.5],
        [40, 40],
        &mut last,
        &mut r,
        &mut |_| Ok(true),
        &mut |_| {
            tries += 1;
            Ok(false)
        },
    )
    .unwrap();
    assert_eq!((none, tries), (None, 50));
    // off the map: neither test runs
    let mut r = rng();
    let edge = Around {
        center: [0.0, 0.0],
        radius: [5.0, 5.0],
    };
    let mut tested = 0;
    let _ = find_drop_location(
        edge,
        [0.5, 0.5],
        [4, 4],
        &mut None,
        &mut r,
        &mut |p| {
            tested += 1;
            assert!(0.0 <= p[0] && 0.0 <= p[1] && p[0] < 4.0 && p[1] < 4.0);
            Ok(true)
        },
        &mut |_| Ok(false),
    )
    .unwrap();
    assert!(tested < 50);
}
