use super::*;

/// No sprite shown: a test hunt's walk.
fn shown<'a>(
    sprite: &'a mut Option<crate::objects::sprite::ActiveSprite>,
    rng: &'a mut crate::math::rng::Well512,
) -> Shown<'a> {
    Shown {
        sprite,
        rng,
        graphic: None,
    }
}
use crate::movement::{MoveStats, Velocity};

/// Slot 0x78 (`FUN_140a78060`): a load of the task's resource, or any for a task of none,
/// asks for the drop-off; another resource (wood for a food task: kinds by
/// [`resource_kind`], meat 15 counting as food) does not, the caller's unit AI taking notify
/// 0x21c.
#[test]
fn a_load_of_another_resource_asks_no_drop_off() {
    assert!(asked_drop_off(0, 15, 300));
    assert!(asked_drop_off(-1, 1, 300));
    assert!(!asked_drop_off(0, 1, 300));
    assert!(!asked_drop_off(2, 3, 300));
}

/// A Hunt gathering `target` with the chopping task.
pub(crate) fn hunting(target: i32) -> HuntAction {
    HuntAction {
        target,
        state: GATHER,
        point: [0.0; 3],
        target_class: -1,
        task: CHOP,
        attack: None,
        move_to: None,
        walk_range: 0.0,
        drop_site: -1,
        carry_to: -1,
        timer: 0.0,
        move_wait: MoveWait::default(),
        lead: Lead::default(),
    }
}

/// `FUN_140ac44b0`: the decay task (`attribute_type4` 0x124) under attribute 292 turns a
/// decaying target's decay negative once; any other task or a player without it leaves it.
#[test]
fn a_decay_task_stops_its_target_s_decay_once() {
    let mut hunt = hunting(7);
    let decaying = || Victim {
        decay: 0.25,
        ..Victim::default()
    };
    let mut v = decaying();
    hunt.decay_check(true, &mut v);
    assert_eq!(v.decay, 0.25);
    hunt.task.kind4 = DECAY_TASK;
    hunt.decay_check(false, &mut v);
    assert_eq!(v.decay, 0.25);
    hunt.decay_check(true, &mut v);
    assert_eq!(v.decay, -0.25);
    hunt.decay_check(true, &mut v);
    assert_eq!(v.decay, -0.25);
    let mut none = Victim::default();
    hunt.decay_check(true, &mut none);
    assert_eq!(none.decay.to_bits(), 0);
}

/// A villager walking at 0.8 tiles a second, radius 0.2.
pub(crate) fn walker() -> Mover {
    Mover {
        forward: [-1.0, 0.0, 0.0],
        right: [0.0, 0.0, 1.0],
        velocity: Some(Velocity {
            velocity: [0.0, 0.0, 0.8],
            acceleration: [0.0, 0.0, 1.0],
        }),
        paths: vec![],
        factor: 1.0,
        stats: MoveStats {
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
        },
        waiting: 1,
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
    }
}

/// MarvinBlue/426048731's tree 9365 (Gaia, class 15, 100 wood) as 9181 meets it in update 748.
fn tree() -> Quarry {
    Quarry {
        id: 9365,
        position: [24.5, 97.5, 0.0],
        class: 15,
        kind: 10,
        state: 2,
        player: 0,
        amount: 100.0,
        allied: false,
        top_action: None,
        drop_site: false,
    }
}

fn engagement() -> Engagement {
    Engagement {
        gap_squared: 3.726 * 3.726,
        gap: 3.726,
        position: [20.774, 97.5, 0.0],
        target_position: [24.5, 97.5, 0.0],
        speed: 0.8,
        range: 0.0,
        min_range: 0.0,
        can_move: true,
        stance: 0,
        state_position: [0.0; 2],
        world_time: 0,
        master: 83,
        target_class: 15,
        same_owner: false,
    }
}

const ARMS: Arms = Arms {
    move_sprite: None,
    sprite: None,
    second_sprite: None,
    idle_sprite: None,
    idle_attack_sprite: None,
    missile: -1,
    fire_frame: 0,
    interrupt_frame: -1,
};

const CHOP: HuntTask = HuntTask {
    id: 0,
    range: 0.0,
    resource: 15,
    sprites: [-1, 1239, 1961, 1960],
    work_value: 1.0,
    productivity: -1,
    kind4: -1,
};

#[test]
fn a_villager_sent_to_a_tree_attacks_it_first_and_walks_there() {
    let mut mover = walker();
    let mut held = (-1, 0.0);
    let (hunt, _) = HuntAction::start(
        CHOP,
        &tree(),
        2,
        false,
        false,
        &mut held,
        &engagement(),
        ARMS,
        &mut mover,
    )
    .unwrap();
    assert_eq!(hunt.state, ATTACK);
    let attack = hunt.attack.unwrap();
    assert_eq!((attack.state, attack.move_to), (4, Some(MoveTo::Start)));
    // the first MoveTo's teardown stopped the unit
    let v = mover.velocity.unwrap();
    assert_eq!(
        (v.velocity[2], v.acceleration[2], mover.waiting),
        (0.0, 0.0, 0)
    );
    // nothing carried, a human player and settings that keep it: the held kind stays
    assert_eq!(held, (-1, 0.0));
    // the constructor's lead: the animal's start at (-1, -1)
    assert_eq!(hunt.lead, Lead::NEW);
    assert_eq!(hunt.lead.start, [-1.0, -1.0]);
}

/// Slot 0x48 on another tree: the hunt takes it and its Attack starts over; one within 0.01
/// starts arrived, which the caller shows (records/425906646's 31110 in update 7967). Its own
/// target it does not take.
#[test]
fn a_hunt_retargeted_to_a_tree_beside_it_starts_its_attack_arrived() {
    let mut mover = walker();
    let mut held = (-1, 0.0);
    let (mut hunt, _) = HuntAction::start(
        CHOP,
        &tree(),
        2,
        false,
        false,
        &mut held,
        &engagement(),
        ARMS,
        &mut mover,
    )
    .unwrap();
    let retarget = |hunt: &mut HuntAction, q: &Quarry, e: &Engagement, m: &mut Mover| {
        hunt.retarget(q, 2, 0.0, false, e, ARMS, m).unwrap()
    };
    assert_eq!(
        retarget(&mut hunt, &tree(), &engagement(), &mut mover),
        None
    );
    let beside = Quarry {
        id: 9366,
        position: [20.78, 97.5, 0.0],
        ..tree()
    };
    let close = Engagement {
        gap_squared: 0.005 * 0.005,
        gap: 0.005,
        target_position: beside.position,
        ..engagement()
    };
    assert_eq!(retarget(&mut hunt, &beside, &close, &mut mover), Some(true));
    let attack = hunt.attack.unwrap();
    assert_eq!(
        (hunt.target, hunt.state, attack.state, attack.move_to),
        (9366, ATTACK, 4, Some(MoveTo::Arrived))
    );
}

#[test]
fn a_hunter_keeps_another_food_it_carries_and_drops_the_rest_by_the_settings() {
    let start = |held: (i16, f32), carry_rule: bool| {
        let mut held = held;
        let _ = HuntAction::start(
            CHOP,
            &tree(),
            2,
            false,
            carry_rule,
            &mut held,
            &engagement(),
            ARMS,
            &mut walker(),
        )
        .unwrap();
        held
    };
    assert_eq!(start((0, 7.0), false), (15, 7.0));
    assert_eq!(start((2, 7.0), false), (2, 7.0));
    assert_eq!(start((2, 7.0), true), (15, 0.0));
}

fn victim(amount: f32) -> Victim {
    Victim {
        id: 9357,
        amount,
        state: 3,
        kind: 10,
        class: 15,
        storage: 100.0,
        ..Victim::default()
    }
}

#[test]
fn a_villager_gathers_work_rate_times_work_value_times_dt_until_its_load_is_full() {
    let hunt = HuntAction {
        target: 9357,
        state: GATHER,
        point: [24.5, 94.5, 0.0],
        target_class: 15,
        task: HuntTask {
            work_value: 0.39,
            resource: 1,
            ..CHOP
        },
        attack: None,
        move_to: None,
        walk_range: 0.0,
        drop_site: -1,
        carry_to: -1,
        timer: 0.0,
        move_wait: MoveWait::default(),
        lead: Lead::default(),
    };
    let (mut kind, mut held) = (1, 0.0f32);
    let gather = |held: &mut f32, kind: &mut i16, tree: &mut Victim| {
        let mut g = Gatherer {
            held_kind: kind,
            held,
            work_rate: 1.0,
            capacity: 10.0,
            productivity: 1.0,
            target_class: 15,
            target_storage: 100.0,
            stops_decay: false,
            ai_class: None,
            sprites: HuntSprites::default(),
            tree_searched: false,
            predator_hunts_end: false,
            searches: false,
            leads: None,
            attack_failed: false,
            let_go: None,
        };
        hunt.gather(0.02, tree, &mut g)
    };
    let mut tree = victim(100.0);
    assert_eq!(gather(&mut held, &mut kind, &mut tree), Ok(false));
    let w = (1.0f32 * 0.39) * 0.02;
    assert_eq!((held, tree.amount), (w, 100.0 - w));
    // a load 0.1 short of the capacity rounds up, and the full load asks for set_state(8)
    held = 9.95 - w;
    let mut tree = victim(50.0);
    assert_eq!(gather(&mut held, &mut kind, &mut tree), Ok(true));
    assert_eq!(held, 10.0);
    // a tree's last bit goes with a thousandth of its master's storage, the empty target
    // asking for set_state(8)
    held = 0.0;
    let mut tree = victim(0.05);
    assert_eq!(gather(&mut held, &mut kind, &mut tree), Ok(true));
    assert_eq!((held, tree.amount), (0.05 + 100.0 * 0.001, 0.0));
}

/// Case 6 without a target: a load is carried off (`set_state(8)`), none or a NaN load searches
/// (`set_state(3)`, `COMISS` with `JBE`).
#[test]
fn a_hunt_without_a_target_carries_its_load_off_or_searches() {
    let mut h = HuntAction {
        target: -1,
        state: GATHER,
        point: [0.0; 3],
        target_class: 15,
        task: CHOP,
        attack: None,
        move_to: None,
        walk_range: 0.0,
        drop_site: -1,
        carry_to: -1,
        timer: 0.0,
        move_wait: MoveWait::default(),
        lead: Lead::default(),
    };
    assert!(h.ends_without_target(0.5));
    assert_eq!(h.state, GATHER);
    for held in [0.0, -1.0, f32::NAN] {
        h.state = GATHER;
        assert!(!h.ends_without_target(held));
        assert_eq!(h.state, SEARCH);
    }
}

/// A live allied target fails the hunt at once (`set_state` 13, its timer cleared), as house
/// 9158 did MarvinBlue/426048731's 9181 after its drop-off in update 1897.
#[test]
fn a_hunt_on_a_live_ally_fails() {
    let ally = Quarry {
        allied: true,
        ..tree()
    };
    let mut held = (-1, 0.0);
    let (hunt, _) = HuntAction::start(
        CHOP,
        &ally,
        2,
        false,
        false,
        &mut held,
        &engagement(),
        ARMS,
        &mut walker(),
    )
    .expect("a hunt");
    assert_eq!(
        (hunt.state, hunt.timer, hunt.target),
        (FAILED, 0.0, ally.id)
    );
}

/// `set_state(4)` on a dead target, or on the hunter's own livestock, walks to it: a quiet
/// `MoveTo` within the task's range, or 1.5 to the livestock (MarvinBlue/426048731's 9171 to its
/// own sheep 9520 in update 1116).
#[test]
fn a_hunt_walks_to_a_dead_target_or_its_own_livestock() {
    let dead = Quarry { state: 3, ..tree() };
    let sheep = Quarry {
        class: LIVESTOCK,
        player: 2,
        ..tree()
    };
    for (q, range) in [(dead, CHOP.range), (sheep, 1.5)] {
        let mut held = (-1, 0.0);
        let (hunt, _) = HuntAction::start(
            CHOP,
            &q,
            2,
            false,
            false,
            &mut held,
            &engagement(),
            ARMS,
            &mut walker(),
        )
        .unwrap();
        assert_eq!(
            (hunt.state, hunt.move_to, hunt.walk_range, hunt.attack),
            (WALK, Some(MoveTo::Start), range, None)
        );
    }
}

/// 425906646's shepherd 31110 in update 3543: the carcass it walks to went past state 6, so
/// its walk ends and a new one to the hunt's point starts (state 11), the unit stopped; a
/// lumberjack (its unit AI's class 15) would search for another tree first.
#[test]
fn a_hunt_whose_target_went_walks_to_its_point() {
    let mut hunt = HuntAction {
        target: 28813,
        state: WALK,
        point: [177.272_75, 62.444_31, 0.0],
        target_class: LIVESTOCK,
        task: CHOP,
        attack: None,
        move_to: Some(MoveTo::Moving),
        walk_range: CHOP.range,
        drop_site: -1,
        carry_to: -1,
        timer: 0.0,
        move_wait: MoveWait::default(),
        lead: Lead::default(),
    };
    let mut mover = walker();
    let mut world = crate::map::spatial::Spatial::default();
    let (mut kind, mut held) = (-1, 0.0);
    let mut walk = Walk {
        mover: Some(&mut mover),
        me: crate::movement::Me {
            id: 31110,
            position: [171.888_89, 59.053_39, 0.0],
            group: -1,
            action_state: WALK,
        },
        dt: 0.028,
        world: &mut world,
        plan: None,
    };
    let mut gather = Gatherer {
        held_kind: &mut kind,
        held: &mut held,
        work_rate: 1.0,
        capacity: 10.0,
        productivity: 1.0,
        target_class: LIVESTOCK,
        target_storage: 0.0,
        stops_decay: false,
        ai_class: Some(15),
        sprites: HuntSprites::default(),
        tree_searched: false,
        predator_hunts_end: false,
        searches: false,
        leads: None,
        attack_failed: false,
        let_go: None,
    };
    assert!(hunt.clone().lost_on_the_way(&mut walk, &gather).is_err());
    // the caller's search for the next tree found none: the hunt walks to its point
    gather.tree_searched = true;
    hunt.clone().lost_on_the_way(&mut walk, &gather).unwrap();
    gather.tree_searched = false;
    gather.ai_class = Some(-1);
    hunt.lost_on_the_way(&mut walk, &gather).unwrap();
    assert_eq!(
        (hunt.state, hunt.move_to, hunt.timer, hunt.point[0]),
        (WALK_SEARCH, Some(MoveTo::Start), 0.0, 177.272_75)
    );
    assert_eq!(mover.velocity.unwrap().velocity[2], 0.0);
}

/// `set_state(6)` on a target of class 49 (429381190's villager 15295 hunting its pasture's
/// dead Ibex 15371 in update 1453): the hunter's own goes on to the hunt's moving sprite, its
/// timer 0; another player's (a farm changes hands, a pasture's livestock ends the hunt) is
/// not modelled.
#[test]
fn a_hunt_faces_its_own_pasture_s_dead_livestock() {
    use crate::combat::attack::tests::Rig;
    const MOVING: Graphic = Graphic {
        id: 1880,
        frames: 10,
        facets: 16,
        speed_multiplier: 1.0,
        frame_duration: 0.1,
        replay_delay: 0.0,
        flags: 3,
    };
    let ibex = Victim {
        id: 15371,
        class: CLASS_49,
        kind: 30,
        player: 1,
        state: 5,
        ..Victim::default()
    };
    let face = |victim: Victim| {
        let mut hunt = HuntAction {
            state: WALK,
            target_class: CLASS_49,
            timer: -3.0,
            ..hunting(15371)
        };
        let mut rig = Rig::new(victim);
        let mut mover = walker();
        let mut world = crate::map::spatial::Spatial::default();
        let walk = Walk {
            mover: Some(&mut mover),
            me: crate::movement::Me {
                id: 15295,
                position: [134.5, 70.5, 0.0],
                group: -1,
                action_state: WALK,
            },
            dt: 0.05,
            world: &mut world,
            plan: None,
        };
        let (mut kind, mut held) = (-1, 0.0);
        let mut gather = Gatherer {
            held_kind: &mut kind,
            held: &mut held,
            work_rate: 1.0,
            capacity: 10.0,
            productivity: 1.0,
            target_class: CLASS_49,
            target_storage: 0.0,
            stops_decay: false,
            ai_class: Some(-1),
            sprites: HuntSprites {
                moving: Some(MOVING),
                ..HuntSprites::default()
            },
            tree_searched: false,
            predator_hunts_end: false,
            searches: false,
            leads: None,
            attack_failed: false,
            let_go: None,
        };
        let r = hunt.set_face(walk, &mut rig.strike(), &mut gather, None);
        (r, hunt, rig.sprite.map(|s| s.graphic.id))
    };
    let (r, hunt, shown) = face(ibex.clone());
    assert_eq!(r, Ok(()));
    assert_eq!((hunt.state, hunt.timer, hunt.move_to), (FACE, 0.0, None));
    assert_eq!(shown, Some(1880));
    assert!(face(Victim { player: 2, ..ibex }).0.is_err());
}

/// A hunt's walk to its point (states 11 and 15, `0x140ac7a00` cases 0xa and 0xe): an ended
/// `MoveTo` (code 1) stops the unit, after which state 11 searches (`set_state` 3) and state 15
/// is done (`set_state` 2); a failed one (code 3) does the same for a human player, a computer
/// player's would end the hunt (`set_state` 13). A human player's villager whose motion step
/// gave up waits first (425906646's p4 villagers 27238, 31018, 31034 and 31042 in update 5724).
#[test]
fn a_walk_to_the_point_ends_in_a_search_or_done() {
    let hunt = |state, move_to| HuntAction {
        target: -1,
        state,
        point: [177.272_75, 62.444_31, 0.0],
        target_class: LIVESTOCK,
        task: CHOP,
        attack: None,
        move_to: Some(move_to),
        walk_range: CHOP.range,
        drop_site: -1,
        carry_to: -1,
        timer: 0.0,
        move_wait: MoveWait::default(),
        lead: Lead::default(),
    };
    let walker_at = PointWalker {
        computer: false,
        villager: true,
        stance: 1,
    };
    let run = |h: &mut HuntAction, who: PointWalker| {
        let mut mover = walker();
        let mut world = crate::map::spatial::Spatial::default();
        let (mut sprite, mut rng) = (None, crate::math::rng::Well512::default());
        let walk = Walk {
            mover: Some(&mut mover),
            me: crate::movement::Me {
                id: 31110,
                position: [171.888_89, 59.053_39, 0.0],
                group: -1,
                action_state: h.state,
            },
            dt: 0.5,
            world: &mut world,
            plan: None,
        };
        let got = h.update_point_walk(walk, who, shown(&mut sprite, &mut rng));
        (got, mover.velocity.unwrap().velocity[2])
    };
    let done = PointWalk::Done {
        target: -1,
        second: -1,
    };
    for (state, ends) in [(WALK_SEARCH, PointWalk::Search), (WALK_POINT, done)] {
        for code in [MoveTo::Arrived, MoveTo::Failed] {
            let mut h = hunt(state, code);
            let (got, speed) = run(&mut h, walker_at);
            assert_eq!((got.unwrap(), h.move_to, speed), (ends, None, 0.0));
            let after = if ends == PointWalk::Search {
                SEARCH
            } else {
                IDLE
            };
            assert_eq!(h.state, after);
        }
        // a computer player's failed walk ends the hunt (set_state 13)
        let computer = PointWalker {
            computer: true,
            ..walker_at
        };
        assert!(run(&mut hunt(state, MoveTo::Failed), computer).0.is_err());
    }
    // set_state(2) lets the target and the second target go, in that order
    let mut h = hunt(SEARCH, MoveTo::Arrived);
    (h.target, h.carry_to, h.timer) = (28813, 9156, -10.0);
    assert_eq!(h.set_done(), [28813, 9156]);
    assert_eq!(
        (h.state, h.target, h.carry_to, h.point, h.timer),
        (IDLE, -1, -1, [-1.0; 3], 0.0)
    );
    // the wait: a second for a stance below 2, then the MoveTo asks again
    let mut h = hunt(WALK_SEARCH, MoveTo::Waiting);
    h.move_wait = MoveWait {
        state: 1,
        timer: 0.0,
        delay: 1.0,
    };
    assert_eq!(run(&mut h, walker_at).0.unwrap(), PointWalk::Walking);
    assert_eq!((h.move_to, h.move_wait.timer), (Some(MoveTo::Waiting), 0.5));
    assert_eq!(run(&mut h, walker_at).0.unwrap(), PointWalk::Walking);
    assert_eq!((h.move_to, h.move_wait.timer), (Some(MoveTo::Start), 0.0));
    // the MoveTo's answer to the motion step giving up: a human's villager waits once
    let mut h = hunt(WALK_POINT, MoveTo::Moving);
    h.give_up_walk(walker_at);
    assert_eq!(
        (h.move_to, h.move_wait.state, h.move_wait.delay),
        (Some(MoveTo::Waiting), 1, 1.0)
    );
    h.give_up_walk(walker_at);
    assert_eq!(h.move_to, Some(MoveTo::Failed));
    let mut h = hunt(WALK_POINT, MoveTo::Moving);
    h.give_up_walk(PointWalker {
        villager: false,
        ..walker_at
    });
    assert_eq!(h.move_to, Some(MoveTo::Failed));
}

/// The drop-off's and the point walk's `MoveTo` on the way (`0x140ac7a00` cases 7, 0xa and
/// 0xe, `FUN_140a992b0`): it starts, asking for a path (to the drop site, to the point;
/// after a wait `+0x6c` goes from 1 to 2); moving, it walks on, the drop-off's starting again
/// once its site left its point; a waiting drop-off starts again past its delay, a failed
/// one waits 3 seconds with the unit's animation stopped, and the timer's wait ends in
/// another try. A drop-off or a point walk without its `MoveTo`, a drop site gone and a
/// computer player's ended walk are not modelled.
#[test]
fn a_hunter_walks_to_its_drop_site_and_to_its_point() {
    use crate::movement::tests::WalkRig;
    let who = PointWalker {
        computer: false,
        villager: true,
        stance: 0,
    };
    let drop = |h: &mut HuntAction, rig: &mut WalkRig, anim: &mut Option<f32>| {
        let (mut sprite, mut rng) = (None, crate::math::rng::Well512::default());
        let (mut kind, mut held) = (1, 3.0);
        let shown = shown(&mut sprite, &mut rng);
        let walk = rig.walk(DROP);
        h.update_drop(walk, &mut kind, &mut held, 230, None, shown, who, anim)
    };
    let dropping = |move_to| HuntAction {
        state: DROP,
        carry_to: 10028,
        move_to,
        ..hunting(-1)
    };
    let mut anim = Some(1.0);
    let mut rig = WalkRig::on_the_path();
    rig.mover.paths.clear();
    let mut h = dropping(Some(MoveTo::Start));
    assert_eq!(drop(&mut h, &mut rig, &mut anim), Ok(Dropped::Going));
    assert_eq!(h.move_to, Some(MoveTo::Moving));
    assert_eq!(rig.mover.paths.len(), 2);
    let mut rig = WalkRig::on_the_path();
    let mut h = dropping(Some(MoveTo::Moving));
    assert_eq!(drop(&mut h, &mut rig, &mut anim), Ok(Dropped::Going));
    assert_eq!(h.move_to, Some(MoveTo::Moving));
    rig.mover.move_to_point = Some([35.0, 88.0, 0.0]);
    assert_eq!(drop(&mut h, &mut rig, &mut anim), Ok(Dropped::Going));
    assert_eq!(h.move_to, Some(MoveTo::Start));
    let mut h = dropping(Some(MoveTo::Waiting));
    h.move_wait.delay = 0.05;
    assert_eq!(drop(&mut h, &mut rig, &mut anim), Ok(Dropped::Going));
    assert_eq!(h.move_to, Some(MoveTo::Start));
    let mut h = dropping(Some(MoveTo::Failed));
    assert_eq!(drop(&mut h, &mut rig, &mut anim), Ok(Dropped::Going));
    assert_eq!((h.timer, anim), (DROP_WAIT, Some(0.0)));
    h.timer = -0.05;
    assert_eq!(drop(&mut h, &mut rig, &mut anim), Ok(Dropped::Again));
    assert_eq!(h.timer, 0.0);
    assert!(drop(&mut dropping(None), &mut rig, &mut anim).is_err());
    let mut h = dropping(Some(MoveTo::Arrived));
    assert!(drop(&mut h, &mut rig, &mut anim).is_err());
    // the walk to a point
    let point_walk = |h: &mut HuntAction, rig: &mut WalkRig, who| {
        let (mut sprite, mut rng) = (None, crate::math::rng::Well512::default());
        let state = h.state;
        h.update_point_walk(rig.walk(state), who, shown(&mut sprite, &mut rng))
    };
    let walking = |move_to| HuntAction {
        state: WALK_POINT,
        point: [35.5, 88.5, 0.0],
        move_to,
        ..hunting(-1)
    };
    let mut rig = WalkRig::on_the_path();
    rig.mover.paths.clear();
    let mut h = walking(Some(MoveTo::Start));
    h.move_wait.state = 1;
    assert_eq!(point_walk(&mut h, &mut rig, who), Ok(PointWalk::Walking));
    assert_eq!((h.move_to, h.move_wait.state), (Some(MoveTo::Moving), 2));
    assert_eq!(rig.mover.paths.len(), 2);
    let mut rig = WalkRig::on_the_path();
    let mut h = walking(Some(MoveTo::Moving));
    assert_eq!(point_walk(&mut h, &mut rig, who), Ok(PointWalk::Walking));
    assert_eq!(h.move_to, Some(MoveTo::Moving));
    assert!(point_walk(&mut walking(None), &mut rig, who).is_err());
    let computer = PointWalker {
        computer: true,
        ..who
    };
    let mut h = walking(Some(MoveTo::Arrived));
    assert!(point_walk(&mut h, &mut rig, computer).is_err());
}

/// A hunter's gatherer of no load, hunting livestock for a human player.
fn herder<'a>(kind: &'a mut i16, held: &'a mut f32) -> Gatherer<'a> {
    Gatherer {
        held_kind: kind,
        held,
        work_rate: 1.0,
        capacity: 10.0,
        productivity: 1.0,
        target_class: LIVESTOCK,
        target_storage: 0.0,
        stops_decay: false,
        ai_class: Some(-1),
        sprites: HuntSprites::default(),
        tree_searched: false,
        predator_hunts_end: false,
        searches: false,
        leads: None,
        attack_failed: false,
        let_go: None,
    }
}

/// `set_state(4)` (`0x140ac60e0` case 4) toward house 10028 from 9177: a dead target is
/// walked to within the task's range, the hunter's own live livestock within
/// `OWN_LIVESTOCK_RANGE`, the point the target's place; a target the hunter touches already
/// starts arrived. Another live target and a unit without speed are not modelled.
#[test]
fn a_hunt_walks_to_its_target_s_place_within_its_range() {
    use crate::movement::tests::WalkRig;
    let set_walk = |h: &mut HuntAction, rig: &mut WalkRig, victim: &Victim| {
        let mut walk = rig.walk(GATHER);
        h.set_walk(&mut walk, victim, 1)
    };
    let dead = Victim {
        state: 3,
        ..Victim::default()
    };
    let own = Victim {
        class: LIVESTOCK,
        player: 1,
        state: 2,
        ..Victim::default()
    };
    let mut rig = WalkRig::on_the_path();
    let mut h = HuntAction {
        timer: -2.0,
        ..hunting(10028)
    };
    assert_eq!(set_walk(&mut h, &mut rig, &dead), Ok(()));
    assert_eq!(
        (h.state, h.timer, h.point, h.walk_range, h.move_to),
        (
            WALK,
            0.0,
            [30.0, 88.0, 0.0],
            CHOP.range,
            Some(MoveTo::Start)
        )
    );
    let mut h = hunting(10028);
    assert_eq!(set_walk(&mut h, &mut rig, &own), Ok(()));
    assert_eq!(h.walk_range, OWN_LIVESTOCK_RANGE);
    let mut h = hunting(9177);
    assert_eq!(set_walk(&mut h, &mut rig, &dead), Ok(()));
    assert_eq!(h.move_to, Some(MoveTo::Arrived));
    let alive = Victim { player: 2, ..own };
    assert!(set_walk(&mut hunting(10028), &mut rig, &alive).is_err());
    rig.mover.stats.speed = 0.0;
    assert!(set_walk(&mut hunting(10028), &mut rig, &dead).is_err());
}

/// State 10's update (`0x140ac7a00` case 9, [`HuntAction::update_lead`]): without a target the
/// hunt searches; an animal standing where it was sent is slaughtered (`set_state(12)`, its
/// Attack); else the timer runs, and at its end an animal on the start is slaughtered too,
/// one within 1.5 tiles becomes the start again with the hunter facing it and the timer at
/// −1 s, and one farther is walked to (`set_state(4)`). A hunt leading unprepared is not
/// modelled.
#[test]
fn a_herder_slaughters_its_animal_once_it_stands_or_walks_after_it() {
    use crate::movement::tests::WalkRig;
    let herd = |idle| Herd {
        own: true,
        drop_site: None,
        engagement: engagement(),
        arms: ARMS,
        idle,
        special_graphic: None,
    };
    let animal = |position| Victim {
        id: 10028,
        class: LIVESTOCK,
        player: 1,
        state: 2,
        position,
        ..Victim::default()
    };
    let lead = |h: &mut HuntAction, victim: Victim, herd: Option<Herd>| {
        let mut rig = WalkRig::on_the_path();
        let mut strike = crate::combat::attack::tests::Rig::new(victim);
        let (mut kind, mut held) = (-1, 0.0);
        let mut gather = herder(&mut kind, &mut held);
        let r = h.update_lead(rig.walk(LEAD), &mut strike.strike(), &mut gather, herd);
        (r, gather.searches)
    };
    let leading = |timer| HuntAction {
        state: LEAD,
        timer,
        lead: Lead {
            start: [29.0, 89.0],
            ..Lead::default()
        },
        ..hunting(10028)
    };
    let mut h = HuntAction {
        target: -1,
        ..leading(0.0)
    };
    assert_eq!(
        lead(&mut h, animal([29.0, 89.0, 0.0]), None),
        (Ok(()), true)
    );
    assert_eq!(h.state, SEARCH);
    let mut h = leading(-1.0);
    assert!(lead(&mut h, animal([29.0, 89.0, 0.0]), None).0.is_err());
    let mut h = leading(-1.0);
    assert_eq!(
        lead(&mut h, animal([29.0, 89.0, 0.0]), Some(herd(true))).0,
        Ok(())
    );
    assert_eq!((h.state, h.attack.is_some()), (ATTACK, true));
    let mut h = leading(-1.0);
    assert_eq!(
        lead(&mut h, animal([29.0, 89.0, 0.0]), Some(herd(false))).0,
        Ok(())
    );
    assert_eq!((h.state, h.timer), (LEAD, -1.0 + 0.1));
    let mut h = leading(-0.05);
    assert_eq!(
        lead(&mut h, animal([29.0, 89.0, 0.0]), Some(herd(false))).0,
        Ok(())
    );
    assert_eq!(h.state, ATTACK);
    let mut h = leading(-0.05);
    assert_eq!(
        lead(&mut h, animal([29.5, 89.0, 0.0]), Some(herd(false))).0,
        Ok(())
    );
    assert_eq!(
        (h.state, h.timer, h.lead.start),
        (LEAD, LEAD_WAIT, [29.5, 89.0])
    );
    let mut h = leading(-0.05);
    assert_eq!(
        lead(&mut h, animal([35.0, 89.0, 0.0]), Some(herd(false))).0,
        Ok(())
    );
    assert_eq!(
        (h.state, h.walk_range, h.move_to),
        (WALK, OWN_LIVESTOCK_RANGE, Some(MoveTo::Start))
    );
}

/// One update (`0x140ac7a00`) by the hunt's state: a computer player's and a state without a
/// case are not modelled; a walk whose target went on the way walks to the point (state 11); a
/// walk that gave up waits, then walks there; a walk under way steps on, or asks again once its
/// target left the `MoveTo`'s point; a lead and a face on nothing search; a gather without a
/// target carries its load off (full) or searches.
#[test]
fn a_hunt_s_update_runs_the_step_of_its_state() {
    use crate::movement::tests::WalkRig;
    let update = |h: &mut HuntAction, rig: &mut WalkRig, computer: bool, held: f32| {
        let mut strike = crate::combat::attack::tests::Rig::new(Victim::default());
        let (mut kind, mut held) = (0, held);
        let mut gather = herder(&mut kind, &mut held);
        let state = h.state;
        let r = h.update(
            rig.walk(state),
            computer,
            &mut strike.strike(),
            &mut gather,
            None,
        );
        (r, gather.searches)
    };
    let mut rig = WalkRig::on_the_path();
    let reason = |r: Result<bool, crate::GenieError>| r.unwrap_err().reason();
    let (r, _) = update(&mut hunting(10028), &mut rig, true, 0.0);
    assert_eq!(reason(r), "a computer player's hunt (FUN_140fde4f0)");
    let mut h = HuntAction {
        state: 99,
        ..hunting(10028)
    };
    let (r, _) = update(&mut h, &mut rig, false, 0.0);
    assert_eq!(
        reason(r),
        "a Hunt action in a state the slice does not model"
    );
    // the target lost on the way
    let walking = |target, timer, move_to| HuntAction {
        state: WALK,
        timer,
        move_to: Some(move_to),
        point: [35.5, 88.5, 0.0],
        ..hunting(target)
    };
    let mut h = walking(-1, 0.0, MoveTo::Moving);
    assert_eq!(update(&mut h, &mut rig, false, 0.0), (Ok(false), false));
    assert_eq!((h.state, h.move_to), (WALK_SEARCH, Some(MoveTo::Start)));
    // a walk that gave up waits its time, then walks to the point
    let mut h = walking(10028, -1.0, MoveTo::Failed);
    assert_eq!(update(&mut h, &mut rig, false, 0.0), (Ok(false), false));
    assert_eq!((h.state, h.timer), (WALK, -1.0 + 0.1));
    let mut h = walking(10028, -0.05, MoveTo::Failed);
    assert_eq!(update(&mut h, &mut rig, false, 0.0), (Ok(false), false));
    assert_eq!((h.state, h.timer), (WALK_SEARCH, 0.0));
    // a walk under way to the house at the MoveTo's point steps on; off it, it asks again
    let mut h = walking(10028, 0.0, MoveTo::Moving);
    assert_eq!(update(&mut h, &mut rig, false, 0.0), (Ok(false), false));
    assert_eq!((h.state, h.move_to), (WALK, Some(MoveTo::Moving)));
    rig.mover.move_to_point = Some([35.0, 88.0, 0.0]);
    let mut h = walking(10028, 0.0, MoveTo::Moving);
    assert_eq!(update(&mut h, &mut rig, false, 0.0), (Ok(false), false));
    assert_eq!((h.state, h.move_to), (WALK, Some(MoveTo::Start)));
    // a lead without a target and a face on an empty target search
    for state in [LEAD, FACE] {
        let mut h = HuntAction {
            state,
            ..hunting(-1)
        };
        assert_eq!(update(&mut h, &mut rig, false, 0.0), (Ok(false), true));
        assert_eq!(h.state, SEARCH, "state {state}");
    }
    // a gather without a target: empty it searches, loaded it is full
    let mut h = HuntAction {
        target: -1,
        ..hunting(-1)
    };
    assert_eq!(update(&mut h, &mut rig, false, 0.0), (Ok(false), true));
    assert_eq!(h.state, SEARCH);
    let mut h = hunting(-1);
    assert_eq!(update(&mut h, &mut rig, false, 5.0), (Ok(true), false));
    assert_eq!(h.state, GATHER);
}

/// State 12's Attack and its end (`0x140ac7a00` case 0xb): under way (code 0) the hunt stays;
/// ended (code 1) on its target the hunt walks to it, without one it searches, and on a predator
/// of a unit AI with orders queued under the game's setting it would end (not modelled); failed
/// (3) it tells the caller, whose search for the next tree follows; its target inside another (4)
/// goes and the hunt walks to its point. A hunt in state 12 without its Attack is not modelled.
#[test]
fn a_hunt_s_attack_ends_into_a_walk_a_search_or_its_point() {
    use crate::combat::attack::tests::{Rig, own_attack};
    use crate::movement::tests::WalkRig;
    let sheep = || Victim {
        id: 10028,
        class: LIVESTOCK,
        player: 1,
        state: 2,
        position: [30.0, 88.0, 0.0],
        ..Victim::default()
    };
    let attack = |h: &mut HuntAction, state, victim, queue_empty, ends| {
        let mut a = own_attack(10028, state, 1.0);
        (a.sub_action, a.seen) = (true, [30.0, 88.0, 0.0]);
        a.arms.idle_sprite = Some(Graphic {
            id: 1,
            frames: 10,
            facets: 8,
            speed_multiplier: 1.0,
            frame_duration: 0.1,
            replay_delay: 0.0,
            flags: 0,
        });
        h.attack = Some(a);
        let mut rig = WalkRig::on_the_path();
        let mut struck = Rig::new(victim);
        let mut strike = struck.strike();
        strike.queue_empty = queue_empty;
        let (mut kind, mut held) = (-1, 0.0);
        let mut gather = herder(&mut kind, &mut held);
        gather.predator_hunts_end = ends;
        let r = h.update(rig.walk(ATTACK), false, &mut strike, &mut gather, None);
        (r, gather.searches, gather.attack_failed, gather.let_go)
    };
    let fresh = |target| HuntAction {
        state: ATTACK,
        ..hunting(target)
    };
    let mut h = fresh(10028);
    assert_eq!(
        attack(&mut h, 14, sheep(), true, false),
        (Ok(false), false, false, None)
    );
    assert_eq!((h.state, h.attack.map(|a| a.state)), (ATTACK, Some(14)));
    let done = crate::combat::attack::DONE;
    let mut h = fresh(10028);
    assert_eq!(
        attack(&mut h, done, sheep(), true, false),
        (Ok(false), false, false, None)
    );
    assert_eq!(
        (h.state, h.walk_range, h.move_to.is_some()),
        (WALK, OWN_LIVESTOCK_RANGE, true)
    );
    let mut h = fresh(-1);
    assert_eq!(
        attack(&mut h, done, sheep(), true, false),
        (Ok(false), true, false, None)
    );
    assert_eq!((h.state, h.attack), (SEARCH, None));
    let wolf = Victim {
        class: PREDATOR,
        ..sheep()
    };
    let (r, ..) = attack(&mut fresh(10028), done, wolf, false, true);
    assert_eq!(
        r.unwrap_err().reason(),
        "a hunt that ends with its Attack on a predator (set_state 1)"
    );
    let mut h = fresh(10028);
    let failed = crate::combat::attack::FAILED;
    assert_eq!(
        attack(&mut h, failed, sheep(), true, false),
        (Ok(false), false, true, None)
    );
    assert_eq!(h.state, ATTACK);
    let inside = Victim {
        inside: true,
        ..sheep()
    };
    let mut h = fresh(10028);
    assert_eq!(
        attack(&mut h, done, inside, true, false),
        (Ok(false), false, false, Some(10028))
    );
    assert_eq!((h.state, h.target), (WALK_SEARCH, -1));
    let mut rig = WalkRig::on_the_path();
    let mut struck = Rig::new(sheep());
    let (mut kind, mut held) = (-1, 0.0);
    let mut gather = herder(&mut kind, &mut held);
    let r = fresh(10028).update(
        rig.walk(ATTACK),
        false,
        &mut struck.strike(),
        &mut gather,
        None,
    );
    assert_eq!(
        r.unwrap_err().reason(),
        "a Hunt in state 12 without its Attack"
    );
}

/// `set_state(6)` on livestock (`0x140ac60e0` case 6): the hunter's own live animal more
/// than 3 tiles from its drop site and nearer than 30 is led there (`set_state(10)`, an
/// animal that stands sent first), and once led it is led on however far; a nearer or a
/// farther one is slaughtered (`set_state(12)`). A dead animal out of the task's range plus
/// `GAP` is walked to (`set_state(4)`); one at hand is faced. A live one unprepared is not
/// modelled.
#[test]
fn a_hunter_facing_livestock_leads_slaughters_or_walks_to_it() {
    use crate::movement::tests::WalkRig;
    let herd = |site: Option<(i32, f32)>, idle| Herd {
        own: true,
        drop_site: site,
        engagement: engagement(),
        arms: ARMS,
        idle,
        special_graphic: None,
    };
    let sheep = |state| Victim {
        id: 10028,
        class: LIVESTOCK,
        player: 1,
        state,
        position: [29.5, 89.0, 0.0],
        ..Victim::default()
    };
    let face = |h: &mut HuntAction, victim: Victim, herd: Option<Herd>| {
        let mut rig = WalkRig::on_the_path();
        let mut strike = crate::combat::attack::tests::Rig::new(victim);
        let (mut kind, mut held) = (-1, 0.0);
        let mut gather = herder(&mut kind, &mut held);
        let r = h.set_face(rig.walk(WALK), &mut strike.strike(), &mut gather, herd);
        (r, gather.leads)
    };
    let walking = |target, moved| HuntAction {
        state: WALK,
        lead: Lead {
            moved,
            ..Lead::default()
        },
        ..hunting(target)
    };
    assert!(face(&mut walking(10028, false), sheep(2), None).0.is_err());
    let mut h = walking(10028, false);
    let led = face(&mut h, sheep(2), Some(herd(Some((500, 16.0)), true)));
    assert_eq!(led, (Ok(()), Some(500)));
    assert_eq!(
        (h.state, h.drop_site, h.lead.moved, h.lead.start, h.timer),
        (LEAD, 500, true, [29.5, 89.0], LEAD_WAIT)
    );
    let mut h = walking(10028, false);
    let led = face(&mut h, sheep(2), Some(herd(Some((500, 16.0)), false)));
    assert_eq!((led, h.state), ((Ok(()), None), LEAD));
    for (gap_squared, moved, state) in [
        (4.0, false, ATTACK),
        (HERD_FAR, false, ATTACK),
        (HERD_FAR, true, LEAD),
    ] {
        let mut h = walking(10028, moved);
        let herd = herd(Some((500, gap_squared)), false);
        assert_eq!(face(&mut h, sheep(2), Some(herd)).0, Ok(()));
        assert_eq!(h.state, state, "{gap_squared} {moved}");
    }
    let mut h = walking(10028, false);
    assert_eq!(face(&mut h, sheep(2), Some(herd(None, false))).0, Ok(()));
    assert_eq!(h.state, ATTACK);
    let mut h = walking(10028, false);
    assert_eq!(face(&mut h, sheep(3), None).0, Ok(()));
    assert_eq!((h.state, h.move_to), (WALK, Some(MoveTo::Start)));
    let mut h = walking(9177, false);
    assert_eq!(face(&mut h, sheep(3), None).0, Ok(()));
    assert_eq!((h.state, h.move_to, h.timer), (FACE, None, 0.0));
}

/// MarvinBlue/426048731's lumberjack 9181 in update 1897: its `MoveTo` to the TC 9156 ended
/// (state 1); the TC takes wood within a tile, so the load goes to player 2 as wood (1) and
/// the unit carries nothing after; the hunt has no target to go back to.
#[test]
fn a_drop_off_gives_the_load_to_the_player() {
    fn walk<'a>(mover: &'a mut Mover, world: &'a mut crate::map::spatial::Spatial) -> Walk<'a> {
        Walk {
            mover: Some(mover),
            me: crate::movement::Me {
                id: 9181,
                position: [26.820_372, 94.718_056, 0.0],
                group: -1,
                action_state: DROP,
            },
            dt: 0.028,
            world,
            plan: None,
        }
    }
    let mut hunt = HuntAction {
        target: -1,
        state: DROP,
        point: [-1.0; 3],
        target_class: 3,
        task: CHOP,
        attack: None,
        move_to: Some(MoveTo::Arrived),
        walk_range: CHOP.range,
        drop_site: -1,
        carry_to: 9156,
        timer: 0.0,
        move_wait: MoveWait::default(),
        lead: Lead::default(),
    };
    let mut mover = walker();
    let mut world = crate::map::spatial::Spatial::default();
    let (mut sprite, mut rng) = (None, crate::math::rng::Well512::default());
    let (mut kind, mut held) = (1, 7.057_053_6);
    let site = DropOff {
        takes: true,
        gap_squared: 0.5,
    };
    let who = PointWalker {
        computer: false,
        villager: true,
        stance: 0,
    };
    let mut anim = Some(1.0);
    let got = hunt
        .update_drop(
            walk(&mut mover, &mut world),
            &mut kind,
            &mut held,
            230,
            Some(site),
            shown(&mut sprite, &mut rng),
            who,
            &mut anim,
        )
        .unwrap();
    assert_eq!(got, Dropped::Load(1, 7.057_053_6));
    assert_eq!((kind, held, hunt.move_to), (-1, 0.0, None));
    // carried food of a carcass (15) goes as food; a site that does not take it stops
    let mut kind = 15;
    let mut held = 2.0;
    hunt.move_to = Some(MoveTo::Arrived);
    let got = hunt
        .update_drop(
            walk(&mut mover, &mut world),
            &mut kind,
            &mut held,
            230,
            Some(site),
            shown(&mut sprite, &mut rng),
            who,
            &mut anim,
        )
        .unwrap();
    assert_eq!(got, Dropped::Load(0, 2.0));
    hunt.move_to = Some(MoveTo::Arrived);
    let refused = DropOff {
        takes: false,
        ..site
    };
    assert!(
        hunt.update_drop(
            walk(&mut mover, &mut world),
            &mut kind,
            &mut held,
            230,
            Some(refused),
            shown(&mut sprite, &mut rng),
            who,
            &mut anim,
        )
        .is_err()
    );
    // a site out of a tile's reach: the hunt waits 3 seconds, the unit's animation stopped;
    // at the wait's end it tries again
    hunt.move_to = Some(MoveTo::Arrived);
    let far = DropOff {
        gap_squared: 1.5,
        ..site
    };
    let step = |hunt: &mut HuntAction, mover: &mut Mover, anim: &mut Option<f32>| {
        let mut world = crate::map::spatial::Spatial::default();
        let (mut sprite, mut rng) = (None, crate::math::rng::Well512::default());
        hunt.update_drop(
            walk(mover, &mut world),
            &mut 15,
            &mut 2.0,
            230,
            Some(far),
            shown(&mut sprite, &mut rng),
            who,
            anim,
        )
        .unwrap()
    };
    assert_eq!(step(&mut hunt, &mut mover, &mut anim), Dropped::Going);
    assert_eq!((hunt.timer, anim), (DROP_WAIT, Some(0.0)));
    hunt.timer = -0.01;
    assert_eq!(step(&mut hunt, &mut mover, &mut anim), Dropped::Again);
    assert_eq!(hunt.timer, 0.0);
}
