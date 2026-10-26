use crate::group::{Member, UnitGroup, UnitGroups};
use crate::masters::{Master, Masters, Task};
use crate::turn::{Player, Turns, Unit};
use crate::unit_ai::UnitAi;

/// A unit of master 4 at (`x`, `y`) in state 2 with 50 hit points and a unit AI.
fn soldier(id: i32, x: f32, y: f32) -> Unit {
    let mut u = Unit {
        id,
        position: [x, y, 0.0],
        ai: Some(UnitAi {
            current_order: -1,
            current_action: -1,
            current_target: -1,
            ..UnitAi::default()
        }),
        ..Unit::default()
    };
    u.life.master_id = 4;
    u.life.state = 2;
    u.life.hp = 50.0;
    u
}

/// Player 1 holds soldiers 10, 11 and 12 (master 4: a combat unit of class 6, 60 hit
/// points, line of sight 4, radius 0.2) in group 1; player 2, its enemy, holds 20 at (3, 3)
/// and player 3, its ally, 30 at (4, 4). An 8 by 8 map whose tiles player 1 sees none of.
fn world() -> Turns {
    let players = vec![
        Player::default(),
        Player {
            objects: vec![
                soldier(10, 1.0, 1.0),
                soldier(11, 2.0, 1.0),
                soldier(12, 1.0, 2.0),
            ],
            ..Player::default()
        },
        Player {
            objects: vec![soldier(20, 3.5, 3.5)],
            ..Player::default()
        },
        Player {
            objects: vec![soldier(30, 4.5, 4.5)],
            ..Player::default()
        },
    ];
    let mut t = Turns::new(0, 0, 0, vec![0; 4], players);
    let master = Master {
        id: 4,
        kind: 70,
        class: 6,
        hit_points: 60,
        line_of_sight: 4.0,
        search_radius: 3.0,
        radius: [0.2, 0.2],
        standing_sprite: -1,
        formation_category: 1,
        ..Master::default()
    };
    t.masters = vec![Masters::new([master]); 4];
    let mut s = crate::spatial::Spatial::new([8, 8]);
    s.relations = vec![
        vec![0, 3, 3, 3],
        vec![0, 0, 3, 0],
        vec![0, 3, 0, 3],
        vec![0, 0, 3, 0],
    ];
    s.unit_diplomacy = vec![
        vec![1, 4, 4, 4],
        vec![0, 1, 4, 2],
        vec![0, 4, 1, 4],
        vec![0, 2, 4, 1],
    ];
    s.sight = Some(crate::visibility::Visibility::new(
        8,
        8,
        Vec::new(),
        vec![0; 64],
        Vec::new(),
        0,
        Vec::new(),
    ));
    t.spatial = s;
    let member = |id| Member {
        id,
        ..Member::default()
    };
    let mut g = UnitGroup::new(1);
    g.owner = 1;
    g.members = vec![member(10), member(11), member(12)];
    for u in &mut t.players[1].objects {
        u.group.id = 1;
    }
    t.groups = UnitGroups {
        groups: vec![g],
        ..UnitGroups::default()
    };
    t
}

/// Player `p` sees tile (`x`, `y`) (`seen`) or only explored it.
fn light(t: &mut Turns, p: usize, x: usize, y: usize, seen: bool) {
    let v = t.spatial.sight.as_mut().unwrap();
    v.unified[y * 8 + x] |= if seen { 1 << p } else { 1 << (32 + p) };
}

/// The members' check of `FUN_14103d910`: 11 below 1 hit point, 12 of another player, 13
/// without a unit AI and 99 the world does not hold leave the group (their links clear and
/// the held ones stop, slot 0x5d0), which sets flags bit 16; 10 stays.
#[test]
fn lost_members_leave_the_group_and_stop() {
    let mut t = world();
    t.players[1].objects[1].life.hp = 0.5;
    let mut stray = soldier(12, 1.0, 2.0);
    stray.group.id = 1;
    let _ = t.players[1].objects.remove(2);
    t.players[2].objects.push(stray);
    let mut mute = soldier(13, 2.0, 2.0);
    mute.ai = None;
    mute.group.id = 1;
    t.players[1].objects.push(mute);
    for u in &mut t.players[1].objects {
        u.mover = Some(crate::hunt::tests::walker());
    }
    let g = t.groups.get_mut(1).unwrap();
    for id in [13, 99] {
        g.members.push(Member {
            id,
            ..Member::default()
        });
    }
    g.flags = 0;
    t.drop_lost_members(1).unwrap();
    let g = t.groups.get(1).unwrap();
    assert_eq!(g.members.iter().map(|m| m.id).collect::<Vec<_>>(), [10]);
    assert_eq!(g.flags & 0x1_0000, 0x1_0000);
    let unit = |t: &Turns, id| t.object(id).unwrap().1.clone();
    assert_eq!(unit(&t, 10).group.id, 1);
    for id in [11, 12, 13] {
        assert_eq!(unit(&t, id).group.id, -1, "{id}");
    }
    let stopped = unit(&t, 11).mover.unwrap();
    assert!(stopped.speed_stopped);
    assert_eq!(stopped.velocity.unwrap().velocity[2], 0.0);
    // a NaN's hit points stay (COMISS then JA)
    let mut t = world();
    t.players[1].objects[0].life.hp = f32::NAN;
    t.drop_lost_members(1).unwrap();
    assert_eq!(t.groups.get(1).unwrap().members.len(), 3);
    assert_eq!(t.groups.get(1).unwrap().flags & 0x1_0000, 0);
}

/// A hibernating group by its command: without one or on a move it runs its queued commands
/// when it has any; a build waits on its own target below state 2; a repair waits on an
/// allied target below its hit points (a NaN too) and runs the queue otherwise; an attack
/// waits while its target is pending; a follow (2) and a command above 106 but the relics'
/// go.
#[test]
fn a_hibernating_group_waits_runs_its_queue_or_goes() {
    let mut t = world();
    let set = |t: &mut Turns, command: i32, target: i32, queued: bool| {
        let g = t.groups.get_mut(1).unwrap();
        g.current_command = command;
        g.current_target.target = target;
        g.command_queue = if queued {
            vec![crate::group::GroupCommand::default()]
        } else {
            Vec::new()
        };
    };
    for command in [-1, 1] {
        set(&mut t, command, -1, false);
        assert_eq!(t.hibernation(1).unwrap(), Some(false));
        set(&mut t, command, -1, true);
        assert_eq!(t.hibernation(1).unwrap(), Some(true));
    }
    for command in [2, 3, 0x69, 0x6b, 0x2711] {
        set(&mut t, command, -1, false);
        assert_eq!(t.hibernation(1).unwrap(), None, "{command}");
    }
    // a build on the group's own 11 in state 1 waits, in state 2 or of another player not
    set(&mut t, 0x65, 11, false);
    t.players[1].objects[1].life.state = 1;
    assert_eq!(t.hibernation(1).unwrap(), Some(false));
    t.players[1].objects[1].life.state = 2;
    assert_eq!(t.hibernation(1).unwrap(), Some(true));
    set(&mut t, 0x65, 20, false);
    t.players[2].objects[0].life.state = 0;
    assert_eq!(t.hibernation(1).unwrap(), Some(true));
    // a repair of the ally's 30 below its 60 hit points waits; at them, or of the enemy's
    // 20, or of nothing, the queue runs
    set(&mut t, 0x6a, 30, false);
    assert_eq!(t.hibernation(1).unwrap(), Some(false));
    t.players[3].objects[0].life.hp = f32::NAN;
    assert_eq!(t.hibernation(1).unwrap(), Some(false));
    t.players[3].objects[0].life.hp = 60.0;
    assert_eq!(t.hibernation(1).unwrap(), Some(true));
    set(&mut t, 0x6a, 20, false);
    assert_eq!(t.hibernation(1).unwrap(), Some(true));
    set(&mut t, 0x6a, 77, false);
    assert_eq!(t.hibernation(1).unwrap(), Some(true));
    // an attack on the enemy's 20 waits while player 1 sees it
    set(&mut t, 9, 20, false);
    assert_eq!(t.hibernation(1).unwrap(), Some(true));
    light(&mut t, 1, 3, 3, true);
    assert_eq!(t.hibernation(1).unwrap(), Some(false));
}

/// `FUN_1410176b0`: a target the world holds, in state 2 at most (0 for a build), inside no
/// other object, of a player the owner does not count an ally, that the owner sees (or only
/// explored, for a master with `fog_visibility`).
#[test]
fn a_target_is_pending_while_seen_and_no_ally_s() {
    let mut t = world();
    let target = |t: &mut Turns, command: i32, id: i32| {
        let g = t.groups.get_mut(1).unwrap();
        g.current_command = command;
        g.current_target.target = id;
    };
    target(&mut t, 9, 20);
    assert!(!t.target_pending(1).unwrap());
    light(&mut t, 1, 3, 3, false);
    assert!(!t.target_pending(1).unwrap());
    for m in &mut t.masters {
        let mut master = m.get(4).unwrap().clone();
        master.fog_visibility = 1;
        *m = Masters::new([master]);
    }
    assert!(t.target_pending(1).unwrap());
    t.players[2].objects[0].life.state = 3;
    assert!(!t.target_pending(1).unwrap());
    t.players[2].objects[0].life.state = 2;
    t.players[2].objects[0].life.inside = Some(5);
    assert!(!t.target_pending(1).unwrap());
    t.players[2].objects[0].life.inside = None;
    assert!(t.target_pending(1).unwrap());
    // an ally's: not pending, but for a build in state 0
    light(&mut t, 1, 4, 4, true);
    target(&mut t, 9, 30);
    assert!(!t.target_pending(1).unwrap());
    target(&mut t, 0x65, 30);
    assert!(!t.target_pending(1).unwrap());
    t.players[3].objects[0].life.state = 0;
    assert!(t.target_pending(1).unwrap());
    // without members, none
    t.groups.get_mut(1).unwrap().members.clear();
    assert!(!t.target_pending(1).unwrap());
    assert!(!t.target_pending(2).unwrap());
}

/// `FUN_141017c70` and `FUN_141017850`: a member whose unit AI works on a relic (order
/// `0x2db` or action `0x277`) keeps a pick-up pending while its master picks up relics and
/// its target is a relic (class `0x2a`), or delivers them and it has a relic or no target;
/// a delivery pending on a monastery (master 104).
#[test]
fn relic_carriers_keep_their_command_pending() {
    let mut t = world();
    let relic = Master {
        id: 285,
        kind: 70,
        class: 0x2a,
        ..Master::default()
    };
    let monastery = Master {
        id: 104,
        kind: 80,
        class: 3,
        ..Master::default()
    };
    let task = |action| Task {
        action_type: action,
        ..Task::default()
    };
    let monk = |tasks: Vec<Task>| Master {
        id: 4,
        kind: 70,
        class: 18,
        tasks,
        ..Master::default()
    };
    let set_masters = |t: &mut Turns, tasks: Vec<Task>| {
        t.masters = vec![Masters::new([monk(tasks), relic.clone(), monastery.clone()]); 4];
    };
    let mut r = soldier(40, 6.0, 6.0);
    r.life.master_id = 285;
    let mut m = soldier(41, 7.0, 7.0);
    m.life.master_id = 104;
    t.players[0].objects.extend([r, m]);
    let ai = |t: &mut Turns, order: i32, action: i32, target: i32| {
        let a = t.players[1].objects[0].ai.as_mut().unwrap();
        a.current_order = order;
        a.current_action = action;
        a.current_target = target;
    };
    t.groups.get_mut(1).unwrap().current_command = 0x84;
    set_masters(&mut t, vec![task(0x84)]);
    ai(&mut t, 0x2db, -1, 40);
    assert!(t.target_pending(1).unwrap());
    ai(&mut t, -1, 0x277, 40);
    assert!(t.target_pending(1).unwrap());
    ai(&mut t, -1, -1, 40);
    assert!(!t.target_pending(1).unwrap());
    ai(&mut t, 0x2db, -1, -1);
    assert!(!t.target_pending(1).unwrap());
    set_masters(&mut t, vec![task(0x88)]);
    assert!(t.target_pending(1).unwrap());
    ai(&mut t, 0x2db, -1, 41);
    assert!(!t.target_pending(1).unwrap());
    // the delivery: on the monastery, with either task
    t.groups.get_mut(1).unwrap().current_command = 0x88;
    assert!(t.target_pending(1).unwrap());
    set_masters(&mut t, vec![task(0x84)]);
    assert!(t.target_pending(1).unwrap());
    set_masters(&mut t, vec![task(5)]);
    assert!(!t.target_pending(1).unwrap());
    set_masters(&mut t, vec![task(0x88)]);
    ai(&mut t, 0x2db, -1, 40);
    assert!(!t.target_pending(1).unwrap());
}

/// `FUN_141017f00`: a member is free by its top action's state and its unit AI; an object
/// the world does not hold is not.
#[test]
fn a_member_is_free_by_its_action_and_its_unit_ai() {
    let mut t = world();
    assert!(t.member_idle(10).unwrap());
    assert!(!t.member_idle(99).unwrap());
    let ai = |t: &mut Turns, order: i32, action: i32| {
        let a = t.players[1].objects[0].ai.as_mut().unwrap();
        a.current_order = order;
        a.current_action = action;
    };
    ai(&mut t, -1, 0x2c6);
    assert!(!t.member_idle(10).unwrap());
    ai(&mut t, 7, 0x2c0);
    assert!(t.member_idle(10).unwrap());
    ai(&mut t, -1, 0x25b);
    assert!(t.member_idle(10).unwrap());
    ai(&mut t, 0x2c6, 0x25b);
    assert!(!t.member_idle(10).unwrap());
    ai(&mut t, -1, -1);
    t.players[1].objects[0].ai = None;
    assert!(t.member_idle(10).unwrap());
    // an Attack in state 2 is not free, in states 3 and 13 it is; a state 4 is not
    let attack = |state| crate::attack::Attack {
        target: -1,
        target2: -1,
        state,
        timer: 0.0,
        range: 0.0,
        min_range: 0.0,
        point: [0.0; 3],
        seen: [0.0; 3],
        goto: [-1.0; 3],
        move_to: None,
        move_wait: crate::move_to::MoveWait::default(),
        pending: false,
        swing: 0.0,
        rest: 0.0,
        arms: crate::attack::Arms {
            move_sprite: None,
            sprite: None,
            second_sprite: None,
            idle_sprite: None,
            idle_attack_sprite: None,
            missile: -1,
            fire_frame: -1,
            interrupt_frame: -1,
        },
        sub_action: false,
        repaths: false,
        runs: false,
        same_owner: false,
    };
    for (state, free) in [(2, false), (3, true), (13, true), (14, true), (4, false)] {
        t.players[1].objects[0].attack = Some(attack(state));
        assert_eq!(t.member_idle(10).unwrap(), free, "{state}");
    }
    // a trade cart's state 2 is free, whatever its action
    t.players[1].objects[0].attack = Some(attack(2));
    for m in &mut t.masters {
        let mut master = m.get(4).unwrap().clone();
        master.class = 19;
        *m = Masters::new([master]);
    }
    assert!(t.member_idle(10).unwrap());
}

/// `FUN_141007a50`: a group rests without a command, its motion standing and no held
/// member moving.
#[test]
fn a_group_rests_without_command_motion_or_moving_member() {
    let mut t = world();
    t.groups.get_mut(1).unwrap().current_command = -1;
    assert!(t.at_rest(1));
    t.players[1].objects[1].mover = Some(crate::hunt::tests::walker());
    assert!(!t.at_rest(1));
    t.players[1].objects[1].mover = None;
    t.groups.get_mut(1).unwrap().motion.velocity = [0.0, 0.0, 1.0];
    assert!(!t.at_rest(1));
    t.groups.get_mut(1).unwrap().motion.velocity = [0.0; 3];
    t.groups.get_mut(1).unwrap().current_command = 1;
    assert!(!t.at_rest(1));
    assert!(!t.at_rest(2));
}

/// `FUN_141007ca0`: a kept-apart unit the world does not hold leaves the list; one that may
/// not be interrupted (its unit AI has queued orders) stays; any other leaves and stands,
/// as does an attacker with no member's target within its reach.
#[test]
fn kept_apart_units_leave_unless_busy() {
    let mut t = world();
    let mut apart = soldier(15, 6.0, 6.0);
    apart.mover = Some(crate::hunt::tests::walker());
    apart.ai.as_mut().unwrap().orders = vec![crate::unit_ai::QueuedOrder::default()];
    let mut free = soldier(16, 6.0, 6.0);
    free.mover = Some(crate::hunt::tests::walker());
    let mut attacker = soldier(17, 7.0, 7.0);
    attacker.ai.as_mut().unwrap().current_action = 600;
    t.players[1].objects.extend([apart, free, attacker]);
    t.groups.get_mut(1).unwrap().objects = vec![99, 15, 16, 17];
    t.keep_apart(1).unwrap();
    assert_eq!(t.groups.get(1).unwrap().objects, [15]);
    assert!(
        t.object(16)
            .unwrap()
            .1
            .mover
            .as_ref()
            .unwrap()
            .speed_stopped
    );
    assert!(
        !t.object(15)
            .unwrap()
            .1
            .mover
            .as_ref()
            .unwrap()
            .speed_stopped
    );
    // a member's group moving keeps the unit too
    t.players[1].objects[3].ai.as_mut().unwrap().orders.clear();
    t.players[1].objects[3].group.id = 1;
    t.groups.get_mut(1).unwrap().current_command = 1;
    t.keep_apart(1).unwrap();
    assert_eq!(t.groups.get(1).unwrap().objects, [15]);
    t.groups.get_mut(1).unwrap().current_command = -1;
    t.keep_apart(1).unwrap();
    assert_eq!(t.groups.get(1).unwrap().objects, Vec::<i32>::new());
}

/// The group's deletion: its held members whose link names it leave it; an idle one stands
/// and shows standing, a busy one walks on. A group without members goes at its update.
#[test]
fn a_deleted_group_lets_its_members_go() {
    let mut t = world();
    for u in &mut t.players[1].objects {
        u.mover = Some(crate::hunt::tests::walker());
    }
    t.players[1].objects[1].ai.as_mut().unwrap().current_action = 0x2c6;
    t.players[1].objects[2].group.id = 7;
    t.delete_group(1).unwrap();
    assert!(t.groups.get(1).is_none());
    let unit = |t: &Turns, id| t.object(id).unwrap().1.clone();
    assert_eq!([10, 11, 12].map(|id| unit(&t, id).group.id), [-1, -1, 7]);
    assert!(unit(&t, 10).mover.unwrap().speed_stopped);
    assert!(!unit(&t, 11).mover.unwrap().speed_stopped);
    assert!(!unit(&t, 12).mover.unwrap().speed_stopped);
    // the update of a group without members deletes it
    let mut t = world();
    t.groups.get_mut(1).unwrap().members.clear();
    assert!(!t.group_update(1).unwrap());
    t.update_groups(1).unwrap();
    assert!(t.groups.get(1).is_none());
}

/// Slots 0x660, 0x658 and 0x650: a unit with the sprite takes its speed (and the sprite,
/// where the slice holds the unit's), one without (-1) shows nothing; a trade cart's
/// standing and walking sprites are its goods', which the slice does not hold, its running
/// one the master's.
#[test]
fn a_unit_shows_its_standing_walking_or_running_sprite() {
    let mut t = world();
    let mut walker = crate::hunt::tests::walker();
    walker.stats.walk_speed = 0.9;
    walker.stats.run_speed = 1.5;
    walker.stats.stand_speed = 0.1;
    t.players[1].objects[0].mover = Some(walker);
    let speed = |t: &Turns| t.object(10).unwrap().1.mover.as_ref().unwrap().stats.speed;
    let master = |t: &mut Turns, class: i16, sprites: [i16; 3]| {
        let m = Master {
            id: 4,
            kind: 70,
            class,
            hit_points: 60,
            radius: [0.2, 0.2],
            standing_sprite: sprites[0],
            move_sprite: sprites[1],
            run_sprite: sprites[2],
            formation_category: 1,
            ..Master::default()
        };
        t.masters = vec![Masters::new([m]); 4];
    };
    master(&mut t, 6, [-1; 3]);
    assert_eq!(t.show_walking(10), Ok(false));
    assert_eq!(t.show_running(10), Ok(false));
    t.show_standing(10).unwrap();
    assert_eq!(speed(&t), 0.8);
    master(&mut t, 6, [6, 7, 8]);
    assert_eq!(t.show_walking(10), Ok(true));
    assert_eq!(speed(&t), 0.9);
    assert_eq!(t.show_running(10), Ok(true));
    assert_eq!(speed(&t), 1.5);
    t.show_standing(10).unwrap();
    assert_eq!(speed(&t), 0.1);
    // a held sprite needs the graphic
    let graphic = crate::sprite::Graphic {
        id: 1,
        frames: 1,
        facets: 8,
        speed_multiplier: 1.0,
        frame_duration: 0.0,
        replay_delay: 0.0,
        flags: 0,
    };
    t.players[1].objects[0].sprite = Some(crate::sprite::ActiveSprite {
        graphic,
        frame: 0,
        interval: 0.0,
        last: 0.0,
        last_frame: 0,
        changed: false,
        looped: false,
        animate: 0,
        speed: 0.0,
    });
    assert!(t.show_walking(10).is_err());
    master(&mut t, crate::group::update::TRADE_CART, [6, 7, 8]);
    let trade = crate::group::update::TRADE_CART_SPRITES;
    assert_eq!(t.show_walking(10), Err(trade.clone()));
    assert_eq!(t.show_standing(10), Err(trade));
    t.players[1].objects[0].sprite = None;
    assert_eq!(t.show_running(10), Ok(true));
    assert_eq!(t.show_walking(99), Ok(false));
}

/// The formation's turn: past π/4 (not a NaN), or with motion flags bit 1 or flags bit 14.
#[test]
fn the_formation_turns_past_a_quarter_turn() {
    let mut t = world();
    let g = t.groups.get_mut(1).unwrap();
    g.flags = 0;
    g.motion.flags = 0;
    g.motion.position[1] = [1.0, 0.0, 0.0];
    assert!(!t.turns_formation(1, [0.72, 0.0, 0.693_974_1]));
    assert!(t.turns_formation(1, [0.70, 0.0, 0.714_142_8]));
    assert!(t.turns_formation(1, [0.0, 0.0, -1.0]));
    assert!(!t.turns_formation(1, [f32::NAN, 0.0, 0.0]));
    t.groups.get_mut(1).unwrap().flags = 0x4000;
    assert!(t.turns_formation(1, [1.0, 0.0, 0.0]));
    let g = t.groups.get_mut(1).unwrap();
    g.flags = 0;
    g.motion.flags = 2;
    assert!(t.turns_formation(1, [1.0, 0.0, 0.0]));
}

/// `movePositionToFrontUsingCentroid`: the members' mean place (x, height, y) plus the
/// forward times 0, as the group's place and new place.
#[test]
fn the_centroid_front_is_the_members_mean_place() {
    let mut t = world();
    t.players[1].objects[2].position = [1.0, 4.0, 3.0];
    t.groups.get_mut(1).unwrap().motion.position[1] = [1.0, 0.0, 0.0];
    t.front_by_centroid(1);
    let g = t.groups.get(1).unwrap();
    let mean = [4.0 / 3.0, 1.0, 2.0];
    assert_eq!(g.motion.position[0], mean);
    assert_eq!(g.motion.new_position[0], mean);
    // an infinite forward makes the place NaN (inf times 0)
    t.groups.get_mut(1).unwrap().motion.position[1] = [f32::INFINITY, 0.0, 0.0];
    t.front_by_centroid(1);
    assert!(t.groups.get(1).unwrap().motion.position[0][0].is_nan());
}

/// The formation lines' attack ranges: each its slots' units' least weapon range (a combat
/// master's; 0 for another type), `f32::MAX` without a held one; a spare slot's index reads
/// the spare slot; a place without a slot is not modelled.
#[test]
fn each_line_takes_its_units_least_range() {
    let mut t = world();
    let combat = |id: i16, kind: u8, range: f32| Master {
        id,
        kind,
        weapon_range: range,
        ..Master::default()
    };
    t.masters = vec![Masters::new([combat(4, 70, 3.0), combat(5, 70, 1.5), combat(6, 30, 2.0)]); 4];
    t.players[1].objects[1].life.master_id = 5;
    t.players[1].objects[2].life.master_id = 6;
    let slot = |unit| crate::group::Slot {
        unit_id: unit,
        ..crate::group::Slot::default()
    };
    let line = |indices: Vec<u32>| crate::group::FormationLine {
        indices,
        ..crate::group::FormationLine::default()
    };
    t.groups.get_mut(1).unwrap().current_formation = Some(crate::group::Formation {
        slots: vec![slot(10), slot(11), slot(99)],
        spare_slots: vec![slot(12)],
        lines: vec![
            line(vec![0, 1]),
            line(vec![2]),
            line(vec![3, 0]),
            line(vec![0]),
        ],
        number_lines: 3,
        ..crate::group::Formation::default()
    });
    t.line_ranges(1).unwrap();
    let f = t.groups.get(1).unwrap().current_formation.clone().unwrap();
    // the fourth line is not in use: it keeps its range
    assert_eq!(
        f.lines.iter().map(|l| l.attack_range).collect::<Vec<_>>(),
        [1.5, f32::MAX, 0.0, 0.0]
    );
    t.groups
        .get_mut(1)
        .unwrap()
        .current_formation
        .as_mut()
        .unwrap()
        .lines[1]
        .indices = vec![u32::MAX];
    assert!(t.line_ranges(1).is_err());
}

/// The relayout's choice: a changed count sets flags bit 7 at a spacing of at most 6 (a NaN
/// too) and relays out above it; an attack's kind-6 formation refills instead; bit 7 above 6
/// relays out and clears bits 7 and 1; bit 1 above 6 relays out an idle group.
#[test]
fn the_relayout_follows_the_count_and_the_spacing() {
    let mut t = world();
    let set = |t: &mut Turns, flags: u32, spacing: f32| {
        let g = t.groups.get_mut(1).unwrap();
        g.flags = flags;
        g.total_spacing = spacing;
    };
    set(&mut t, 0, 6.0);
    assert_eq!(t.relayout_choice(1, 3), (false, false));
    assert_eq!(t.relayout_choice(1, 4), (false, false));
    assert_eq!(t.groups.get(1).unwrap().flags, 0x80);
    set(&mut t, 0, f32::NAN);
    assert_eq!(t.relayout_choice(1, 4), (false, false));
    assert_eq!(t.groups.get(1).unwrap().flags, 0x80);
    set(&mut t, 0, 6.5);
    assert_eq!(t.relayout_choice(1, 4), (true, false));
    set(&mut t, 0x82, 6.5);
    assert_eq!(t.relayout_choice(1, 3), (true, false));
    assert_eq!(t.groups.get(1).unwrap().flags, 0);
    set(&mut t, 0x2, 6.5);
    assert_eq!(t.relayout_choice(1, 3), (true, false));
    assert_eq!(t.groups.get(1).unwrap().flags, 0);
    t.groups.get_mut(1).unwrap().motion.velocity = [0.0, 0.0, 1.0];
    set(&mut t, 0x2, 6.5);
    assert_eq!(t.relayout_choice(1, 3), (false, false));
    assert_eq!(t.groups.get(1).unwrap().flags, 2);
    let g = t.groups.get_mut(1).unwrap();
    g.current_command = 9;
    g.current_formation = Some(crate::group::Formation {
        kind: 6,
        ..crate::group::Formation::default()
    });
    set(&mut t, 0, 6.0);
    assert_eq!(t.relayout_choice(1, 4), (false, true));
    assert_eq!(t.groups.get(1).unwrap().flags, 0);
}

/// A follow: its target near where the group's target was keeps the group (flags bit 12);
/// far, the group moves to it (`commandMoveTo`: the members' seconds since attacking go to
/// `f32::MAX`); without the target it ends as a finished move: deleted without queued
/// commands, else hibernating.
#[test]
fn a_follow_moves_to_its_target_when_it_strays() {
    let mut t = world();
    let g = t.groups.get_mut(1).unwrap();
    g.current_command = 2;
    g.current_target.target = 20;
    g.current_target.original_position = [3.5, 0.0, 4.5];
    g.flags = 0;
    let mut relayout = false;
    // 1 from the target: within half the lead's line of sight (2)
    assert!(t.command_update(1, &mut relayout).unwrap());
    let g = t.groups.get(1).unwrap();
    assert_eq!(
        (g.flags, g.members[0].seconds_since_attacking),
        (0x1000, 0.0)
    );
    t.groups
        .get_mut(1)
        .unwrap()
        .current_target
        .original_position = [3.5, 0.0, 5.5];
    assert!(t.command_update(1, &mut relayout).unwrap());
    let g = t.groups.get(1).unwrap();
    assert_eq!(
        (g.flags, g.members[0].seconds_since_attacking),
        (0x1000, f32::MAX)
    );
    assert_eq!(g.motion.lead, 12);
    t.groups.get_mut(1).unwrap().current_target.target = 77;
    assert!(!t.command_update(1, &mut relayout).unwrap());
    t.groups.get_mut(1).unwrap().command_queue = vec![crate::group::GroupCommand::default()];
    assert!(t.command_update(1, &mut relayout).unwrap());
    assert_eq!(t.groups.get(1).unwrap().hibernating, 1);
    assert!(!relayout);
    t.groups.get_mut(1).unwrap().current_command = 3;
    assert!(t.command_update(1, &mut relayout).is_err());
}

/// `FUN_141041420`: motion flags bit 1 and a moving group ask for a relayout; with flags bit
/// 12 a strayed target (a squared 1 from the top path's end) turns the move into a follow;
/// wait delays and a group waiting for a path do nothing.
#[test]
fn a_moving_group_s_move_waits_or_follows() {
    let mut t = world();
    let g = t.groups.get_mut(1).unwrap();
    g.current_command = 1;
    g.flags = 0;
    g.motion.flags = 2;
    let mut relayout = false;
    assert!(!t.move_update(1, &mut relayout).unwrap());
    assert!(relayout);
    let g = t.groups.get_mut(1).unwrap();
    g.motion.flags = 0;
    g.motion.velocity = [0.0, 0.0, 1.0];
    let mut relayout = false;
    assert!(!t.move_update(1, &mut relayout).unwrap());
    assert!(relayout);
    // a follow's target 20 at (3.5, 3.5) and the top path's end at (3.5, h, 3.5): no change
    let g = t.groups.get_mut(1).unwrap();
    g.flags = 0x1000;
    g.current_target.target = 20;
    let mut path = crate::movement::PathData::without_path();
    path.has_path = true;
    path.points = vec![[0.0; 3], [3.5, 0.0, 3.5]];
    g.motion.paths = vec![path];
    let mut relayout = false;
    assert!(!t.move_update(1, &mut relayout).unwrap());
    assert_eq!(
        (relayout, t.groups.get(1).unwrap().current_command),
        (true, 1)
    );
    t.groups.get_mut(1).unwrap().motion.paths[0].points[1] = [3.5, 0.0, 4.5];
    let mut relayout = false;
    assert!(!t.move_update(1, &mut relayout).unwrap());
    assert_eq!(
        (relayout, t.groups.get(1).unwrap().current_command),
        (false, 2)
    );
    // standing with bit 12: a follow at once; else wait delays or a level waiting do nothing
    let g = t.groups.get_mut(1).unwrap();
    g.current_command = 1;
    g.motion.velocity = [0.0; 3];
    assert!(!t.move_update(1, &mut relayout).unwrap());
    assert_eq!(t.groups.get(1).unwrap().current_command, 2);
    let g = t.groups.get_mut(1).unwrap();
    g.current_command = 1;
    g.flags = 0;
    g.motion.number_wait_delays = 1;
    assert!(!t.move_update(1, &mut relayout).unwrap());
    let g = t.groups.get_mut(1).unwrap();
    g.motion.number_wait_delays = 0;
    g.motion.paths[0].state = 2;
    assert!(!t.move_update(1, &mut relayout).unwrap());
    assert_eq!(t.groups.get(1).unwrap().current_command, 1);
}

/// `FUN_141041420` for a standing group: without a path walked and no formation the move
/// finishes (command -1, flags bit 2 off and bit 5 on); with one walked it goes on; on a
/// patrol it does not finish; a formation whose slots are all in their places lets their
/// units go and ends the command; one with none in place locks to its kind.
#[test]
fn a_standing_group_s_move_ends_or_goes_on() {
    let mut t = world();
    let g = t.groups.get_mut(1).unwrap();
    g.current_command = 1;
    g.flags = 4;
    g.motion.flags = 0;
    let mut relayout = false;
    assert!(t.move_update(1, &mut relayout).unwrap());
    let g = t.groups.get(1).unwrap();
    assert_eq!((g.current_command, g.flags), (-1, 0x20));
    let walked = || {
        let mut path = crate::movement::PathData::without_path();
        path.has_path = true;
        path.points = vec![[0.0; 3], [3.5, 0.0, 3.5]];
        path
    };
    let g = t.groups.get_mut(1).unwrap();
    g.current_command = 1;
    g.motion.paths = vec![walked()];
    assert!(!t.move_update(1, &mut relayout).unwrap());
    assert_eq!(t.groups.get(1).unwrap().current_command, 1);
    // on a patrol (flags bit 11) without a path walked: the command ends, unfinished
    let g = t.groups.get_mut(1).unwrap();
    g.motion.paths[0].has_path = false;
    g.flags = 0x800;
    assert!(!t.move_update(1, &mut relayout).unwrap());
    assert_eq!(t.groups.get(1).unwrap().current_command, -1);
    // every slot in place: its units leave and the command ends
    let mut t = world();
    let slot = |unit, state| crate::group::Slot {
        unit_id: unit,
        state,
        ..crate::group::Slot::default()
    };
    let g = t.groups.get_mut(1).unwrap();
    g.current_command = 1;
    g.motion.flags = 0;
    g.current_formation = Some(crate::group::Formation {
        kind: 1,
        slots: vec![slot(10, 3), slot(11, 3)],
        ..crate::group::Formation::default()
    });
    assert!(t.move_update(1, &mut relayout).unwrap());
    let g = t.groups.get(1).unwrap();
    assert_eq!(g.members.iter().map(|m| m.id).collect::<Vec<_>>(), [12]);
    assert_eq!(g.current_command, -1);
    // none in place but no free unit: the formation locks to its kind, the command ends
    let mut t = world();
    for u in &mut t.players[1].objects {
        u.ai.as_mut().unwrap().current_action = 0x2c6;
    }
    let g = t.groups.get_mut(1).unwrap();
    g.current_command = 1;
    g.motion.flags = 0;
    g.current_formation = Some(crate::group::Formation {
        kind: 1,
        slots: vec![slot(10, 0)],
        ..crate::group::Formation::default()
    });
    assert!(!t.move_update(1, &mut relayout).unwrap());
    assert_eq!(t.groups.get(1).unwrap().current_command, -1);
}

/// `AutomaticMoveUnitsToDesiredPositions`: a slot without a unit, of a unit the world does
/// not hold or that is busy, or in its place (on no attack), is passed; a free unit out of
/// its place gets a path to it, not modelled.
#[test]
fn units_out_of_their_places_get_paths() {
    let mut t = world();
    let slot = |unit, state| crate::group::Slot {
        unit_id: unit,
        state,
        ..crate::group::Slot::default()
    };
    t.players[1].objects[1].ai.as_mut().unwrap().current_action = 0x2c6;
    t.groups.get_mut(1).unwrap().current_formation = Some(crate::group::Formation {
        slots: vec![slot(-1, 0), slot(99, 0), slot(11, 0), slot(10, 3)],
        ..crate::group::Formation::default()
    });
    t.move_to_desired_positions(1, 1, -1).unwrap();
    t.move_to_desired_positions(1, 9, -1).unwrap();
    assert!(t.move_to_desired_positions(1, 9, 20).is_err());
    t.groups
        .get_mut(1)
        .unwrap()
        .current_formation
        .as_mut()
        .unwrap()
        .slots[3]
        .state = 0;
    assert!(t.move_to_desired_positions(1, 1, -1).is_err());
}

/// The members' own moves: a member moved aside (reason 2) that is free stops, counts its
/// timer down and, at 0, drops its move and stands; a busy one or one without an active
/// move is passed; another reason's step is not modelled.
#[test]
fn a_member_moved_aside_waits_out_its_timer() {
    let mut t = world();
    t.dt_seconds = 0.5;
    for u in &mut t.players[1].objects {
        u.mover = Some(crate::hunt::tests::walker());
    }
    let aside = crate::group::SpecificMove {
        active: 1,
        path_id: 4,
        reason: 2,
        timer: 0.75,
        ..crate::group::SpecificMove::default()
    };
    let g = t.groups.get_mut(1).unwrap();
    g.members[0].specific_move = aside;
    g.members[1].specific_move = aside;
    t.players[1].objects[1].ai.as_mut().unwrap().current_action = 0x2c6;
    t.specific_moves_step(1).unwrap();
    let g = t.groups.get(1).unwrap();
    assert_eq!(g.members[0].specific_move.timer, 0.25);
    assert_eq!(g.members[1].specific_move, aside);
    assert!(
        t.object(10)
            .unwrap()
            .1
            .mover
            .as_ref()
            .unwrap()
            .speed_stopped
    );
    assert!(
        !t.object(11)
            .unwrap()
            .1
            .mover
            .as_ref()
            .unwrap()
            .speed_stopped
    );
    t.specific_moves_step(1).unwrap();
    let g = t.groups.get(1).unwrap();
    assert_eq!(
        g.members[0].specific_move,
        crate::group::SpecificMove {
            path_id: -1,
            reason: -1,
            ..crate::group::SpecificMove::default()
        }
    );
    // a NaN timer keeps the move
    let g = t.groups.get_mut(1).unwrap();
    g.members[0].specific_move = crate::group::SpecificMove {
        timer: f32::NAN,
        ..aside
    };
    t.specific_moves_step(1).unwrap();
    assert_eq!(t.groups.get(1).unwrap().members[0].specific_move.active, 1);
    // any other reason takes the unit's own motion step: without a path it is done at
    // once, and the move goes
    t.groups.get_mut(1).unwrap().members[0].specific_move.reason = 1;
    t.specific_moves_step(1).unwrap();
    assert_eq!(t.groups.get(1).unwrap().members[0].specific_move.active, 0);
}

/// A member's own move of the way round (reason 0) steps the unit along its path (slot
/// 0x608): moving on (the step turns it toward its waypoint; the velocity update moves it
/// later), the move's distance loses the ground distance from its last position to where the
/// unit last moved to (here 1 tile short of it on y), which becomes its last position.
#[test]
fn a_member_s_own_move_walks_its_path() {
    let mut t = world();
    t.dt_seconds = 0.5;
    let mut walker = crate::hunt::tests::walker();
    walker.moved_at = [1.0, 1.0, 0.0];
    walker.take_record_path(vec![[1.0, 0.0, 1.0], [5.0, 0.0, 1.0]], 4.0, 0, false);
    t.players[1].objects[0].mover = Some(walker);
    t.spatial.insert(
        10,
        crate::spatial::Placed {
            position: [1.0, 1.0, 0.0],
            radius: [0.2, 0.2],
            class: 6,
            player: 1,
            master: 4,
            obstruction_type: 0,
            buildable_on: 0,
            tall: false,
            speed: 0.8,
            unknown: None,
            obstruction: None,
        },
    );
    t.groups.get_mut(1).unwrap().members[0].specific_move = crate::group::SpecificMove {
        active: 1,
        path_id: -1,
        reason: 0,
        timer: 3.0,
        distance: 3.0,
        last_position: [1.0, 0.0, 0.0],
        ..crate::group::SpecificMove::default()
    };
    t.specific_moves_step(1).unwrap();
    let (_, u) = t.object(10).unwrap();
    let at = crate::group::motion::moved_vector(u);
    let own = t.groups.get(1).unwrap().members[0].specific_move;
    assert_eq!(own.active, 1);
    assert_eq!((at, own.last_position), ([1.0, 0.0, 1.0], [1.0, 0.0, 1.0]));
    assert_eq!(own.distance, 2.0);
}

/// `FUN_14100d940`: the orders whose time has come go; later ones stay.
#[test]
fn due_orders_leave_the_queue() {
    let mut t = world();
    t.world_time = 100;
    let order = |execute_time| crate::group::GroupOrder {
        execute_time,
        ..crate::group::GroupOrder::default()
    };
    t.groups.get_mut(1).unwrap().order_queue = vec![order(99), order(101), order(100)];
    t.drop_due_orders(1);
    assert_eq!(t.groups.get(1).unwrap().order_queue, [order(101)]);
}

/// The idle timer: an idle group adds the update's milliseconds and draws anew past 3000; a
/// busy one draws anew. The milliseconds come back exactly from the update's seconds.
#[test]
fn the_idle_timer_counts_while_idle() {
    let mut t = world();
    t.dt_seconds = crate::lifecycle::seconds(69);
    t.groups.get_mut(1).unwrap().idle_timer = 2931;
    t.idle_timer_step(1);
    assert_eq!(t.groups.get(1).unwrap().idle_timer, 3000);
    let mut streams = t.streams.clone();
    let draw = streams.get(crate::rng::Stream::World).rand15();
    t.idle_timer_step(1);
    assert_eq!(t.groups.get(1).unwrap().idle_timer, draw % 750);
    t.groups.get_mut(1).unwrap().motion.velocity = [0.0, 0.0, 1.0];
    t.groups.get_mut(1).unwrap().idle_timer = 5;
    t.idle_timer_step(1);
    assert!(t.groups.get(1).unwrap().idle_timer < 750);
    for dt in 0..=100_000 {
        t.dt_seconds = crate::lifecycle::seconds(dt);
        assert_eq!(t.dt_ms(), dt);
    }
}

/// The awake update's ends: an idle group (no command, none queued) whose formation's slots
/// are all in state 3 goes, one with flags bit 6 goes, and one whose lead the world does
/// not hold goes; with every member kept apart and nothing pending, each leaves the group.
#[test]
fn an_idle_group_goes() {
    let mut t = world();
    let g = t.groups.get_mut(1).unwrap();
    g.current_command = -1;
    g.flags = 0;
    g.current_formation = Some(crate::group::Formation {
        kind: 1,
        slots: vec![crate::group::Slot {
            state: 3,
            ..crate::group::Slot::default()
        }],
        ..crate::group::Formation::default()
    });
    assert!(!t.group_update(1).unwrap());
    let mut t = world();
    t.groups.get_mut(1).unwrap().flags = 0x40;
    assert!(!t.group_update(1).unwrap());
    // every member kept apart: each leaves, then the group has none
    let mut t = world();
    let g = t.groups.get_mut(1).unwrap();
    g.flags = 0;
    g.objects = vec![10, 11, 12];
    g.current_command = 1;
    assert!(!t.group_update(1).unwrap());
    assert_eq!(t.groups.get(1).unwrap().members, Vec::new());
    assert_eq!(t.groups.get(1).unwrap().objects, Vec::<i32>::new());
    assert_eq!(t.object(11).unwrap().1.group.id, -1);
}

/// `BaseWorld::update` runs a player's groups (`FUN_14103d910` at `0x14155179b`) before the
/// player's own update (slot 0x100 at `0x1415518f0`, which updates its objects): a group
/// whose member leaves it (no unit AI) stops the member, which then does not move in the
/// same update.
#[test]
fn a_player_s_groups_update_before_its_objects() {
    let mut t = world();
    let walker = crate::hunt::tests::walker();
    for u in &mut t.players[1].objects {
        u.mover = Some(walker.clone());
        u.mover.as_mut().unwrap().moved_at = u.position;
        u.ai = None;
    }
    let before = t.object(10).unwrap().1.position;
    t.tick(69);
    let u = t.object(10).unwrap().1;
    assert_eq!(u.group.id, -1);
    assert_eq!(u.position, before);
    assert_eq!(u.mover.as_ref().unwrap().velocity.unwrap().velocity[2], 0.0);
}

/// `FUN_141040b50`: a motion that moved (bit 0) stores its new position at the front; bit 1
/// moves to the group's flags, and both motion bits clear. A motion that did not move keeps
/// its stored positions.
#[test]
fn the_update_end_stores_a_moved_group_s_new_position() {
    let mut g = UnitGroup::new(1);
    g.motion.new_position[0] = [3.0, 0.0, 4.0];
    g.motion.flags = 3;
    g.flags = 0;
    g.update_end();
    assert_eq!((g.motion.flags, g.flags & 2), (0, 2));
    assert_eq!(
        g.stored_positions.points.first().map(|p| p.point),
        Some([3.0, 0.0, 4.0])
    );
    let mut still = UnitGroup::new(2);
    still.motion.flags = 0;
    let before = still.stored_positions.points.len();
    still.update_end();
    assert_eq!(still.stored_positions.points.len(), before);
}

/// `FUN_141016870`: the lead is the held member nearest (in 3D, where it last moved to) the
/// target's place, the first of equals; -1 without a held member.
#[test]
fn the_lead_is_the_member_nearest_the_targets_place() {
    let at = |id, x: f32, y: f32| Unit {
        id,
        position: [x, y, 0.0],
        ..Unit::default()
    };
    let player = Player {
        objects: vec![at(10, 5.0, 5.0), at(11, 2.0, 1.0), at(12, 1.0, 2.0)],
        ..Player::default()
    };
    let mut t = Turns::new(0, 1, 0, vec![0], vec![player]);
    let member = |id| Member {
        id,
        ..Member::default()
    };
    let mut g = UnitGroup::new(1);
    // 99 is not held; 11 and 12 are as near (1, 1) as each other: 11 comes first
    g.members = vec![member(99), member(10), member(11), member(12)];
    g.current_target.original_position = [1.0, 0.0, 1.0];
    t.groups = UnitGroups {
        groups: vec![g],
        ..UnitGroups::default()
    };
    assert_eq!(t.nearest_to_target(1), 11);
    t.groups.groups[0].members = vec![member(99)];
    assert_eq!(t.nearest_to_target(1), -1);
    assert_eq!(t.nearest_to_target(2), -1);
}

/// `FUN_141017630`, `resetIdleTimer` (`FUN_14102d370`) and `IsIdle` (`FUN_141017ad0`).
#[test]
fn a_group_holds_members_draws_its_idle_timer_and_tells_when_it_is_idle() {
    let held = Unit {
        id: 10,
        ..Unit::default()
    };
    let player = Player {
        objects: vec![held],
        ..Player::default()
    };
    let mut t = Turns::new(0, 1, 0, vec![0], vec![player]);
    let member = |id| Member {
        id,
        ..Member::default()
    };
    let mut g = UnitGroup::new(1);
    g.members = vec![member(99), member(10)];
    t.groups = UnitGroups {
        groups: vec![g],
        ..UnitGroups::default()
    };
    assert!(t.any_member_held(1));
    t.groups.groups[0].members = vec![member(99)];
    assert!(!t.any_member_held(1));
    assert!(!t.any_member_held(2));
    // the World stream's next draw, modulo 750
    let mut streams = t.streams.clone();
    let draw = streams.get(crate::rng::Stream::World).rand15();
    t.reset_idle_timer(1);
    assert_eq!(t.groups.groups[0].idle_timer, draw % 750);
    assert_eq!(t.streams, streams);
    // idle: standing; not while moving or waiting for a path; idle again with bit 5, or on a
    // move with bit 11
    assert!(t.is_idle(1));
    t.groups.groups[0].motion.velocity = [0.0, 0.0, 1.0];
    assert!(!t.is_idle(1));
    t.groups.groups[0].current_command = 1;
    t.groups.groups[0].flags |= 0x800;
    assert!(t.is_idle(1));
    t.groups.groups[0].flags = 0x20;
    assert!(t.is_idle(1));
    assert!(!t.is_idle(2));
}

/// `FUN_14102bb40`: the command queue empties; a hibernating group (1) wakes with flags
/// `0x4c000`, any other hibernation value wakes without them.
#[test]
fn emptying_the_command_queue_wakes_a_hibernating_group() {
    let mut g = UnitGroup::new(1);
    g.command_queue = vec![crate::group::GroupCommand::default()];
    g.hibernating = 1;
    g.clear_command_queue();
    assert_eq!(g.command_queue, Vec::new());
    assert_eq!((g.hibernating, g.flags), (0, 0x100 | 0x4_c000));
    let mut g = UnitGroup::new(1);
    g.hibernating = 2;
    g.clear_command_queue();
    assert_eq!((g.hibernating, g.flags), (0, 0x100));
}

/// The long pather's list of a unit in group 1 (whose differentiator groups are 1 and 2) is
/// 10, 11 and 12; a unit of no group, or of a group the world does not hold, has none.
#[test]
fn a_grouped_units_list_is_its_groups_differentiator_units() {
    let member = |id| Member {
        id,
        ..Member::default()
    };
    let mut one = UnitGroup::new(1);
    one.members = vec![member(10), member(11)];
    one.differentiator_groups = vec![1, 2];
    let mut two = UnitGroup::new(2);
    two.members = vec![member(12)];
    let groups = UnitGroups {
        groups: vec![one, two],
        ..UnitGroups::default()
    };
    assert_eq!(groups.group_units(1), Some(vec![10, 11, 12]));
    assert_eq!(groups.group_units(2), Some(Vec::new()));
    assert_eq!(groups.group_units(9), None);
    assert_eq!(groups.group_units(-1), None);
}
