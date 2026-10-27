use super::*;

/// The object's own update knows its slot: it is taken while it still holds the object, and
/// the object is found by its id once the list moved or the slot is out of the lists; an
/// object asleep or gone has no live slot.
#[test]
fn an_object_s_own_slot_is_taken_while_it_holds_the_object() {
    let unit = |id| Unit {
        id,
        ..Unit::default()
    };
    let t = World::new(
        0,
        1,
        0,
        vec![0, 0],
        vec![
            crate::world::Player {
                objects: vec![unit(1), unit(2)],
                ..crate::world::Player::default()
            },
            crate::world::Player {
                objects: vec![unit(3)],
                sleeping: vec![unit(4)],
                ..crate::world::Player::default()
            },
        ],
    );
    assert_eq!(t.live_slot((1, 0, 3)), Some((1, 0)));
    assert_eq!(t.live_slot((0, 0, 2)), Some((0, 1)));
    assert_eq!(t.live_slot((5, 9, 3)), Some((1, 0)));
    assert_eq!(t.live_slot((1, 0, 4)), None);
    assert_eq!(t.live_slot((0, 0, 7)), None);
}

/// records/425906646's gate 27745 (masters 88 and 91, 1650 hit points each, three damage
/// sprites): unhurt, its share is 0 under either master; a resource's share is not modelled.
#[test]
fn a_damage_share_is_the_whole_less_the_hit_points_share() {
    assert_eq!(damage_share(1650.0, 1650, 39), Ok(0));
    assert_eq!(damage_share(825.0, 1650, 39), Ok(50));
    assert_eq!(damage_share(1.0, 1650, 39), Ok(99));
    assert_eq!(damage_share(2000.0, 1650, 39), Ok(0));
    assert!(damage_share(10.0, 100, 58).is_err());
}

/// records/425906646's update 1641: player 6's drop-off order reaches its goose 28766 (class
/// 58, 100 food, no drop sites, its default task on top): no table entry takes class 58, so
/// it does nothing. An entry of its class (taking food) leads to the nearest such building
/// of the player's tracker in state 2 (`FUN_141658e20`), but a goose (below type 70) does no
/// work on it.
#[test]
fn a_drop_off_order_leaves_a_livestock_without_a_drop_site_alone() {
    let goose = Unit {
        id: 28766,
        life: crate::objects::lifecycle::Lifecycle {
            master_id: 1243,
            ..crate::objects::lifecycle::Lifecycle::default()
        },
        ..Unit::default()
    };
    let mut t = World::new(
        0,
        1,
        0,
        vec![0],
        vec![crate::world::Player {
            objects: vec![goose],
            ..crate::world::Player::default()
        }],
    );
    t.masters = vec![crate::objects::masters::Masters::new([Master {
        id: 1243,
        class: 58,
        ..Master::default()
    }])];
    let tc = crate::unit_ai::task_finder::DropSite {
        building: 109,
        worker_class: 4,
        ..crate::unit_ai::task_finder::DropSite::default()
    };
    t.drop_sites = vec![tc.clone()];
    assert!(t.drop_off_unasked(0, 0, true).is_ok());
    t.drop_sites = vec![
        tc,
        crate::unit_ai::task_finder::DropSite {
            building: 1897,
            worker_class: 58,
            resource_types: vec![0],
            ..crate::unit_ai::task_finder::DropSite::default()
        },
    ];
    let pen = |id, x: f32, state| Unit {
        id,
        position: [x, 10.0, 0.0],
        life: crate::objects::lifecycle::Lifecycle {
            master_id: 1897,
            state,
            ..crate::objects::lifecycle::Lifecycle::default()
        },
        ..Unit::default()
    };
    t.masters[0] = crate::objects::masters::Masters::new([
        t.masters[0].get(1243).unwrap().clone(),
        Master {
            id: 1897,
            kind: 80,
            class: 3,
            ..Master::default()
        },
    ]);
    t.players[0].objects[0].life.attribute_type = 0;
    t.players[0].objects[0].position = [10.0, 10.0, 0.0];
    for (id, x, state) in [
        (500, 30.0, 2),
        (501, 12.0, 0),
        (502, 20.0, 2),
        (503, 15.0, 2),
    ] {
        t.players[0].objects.push(pen(id, x, state));
        t.track_add(0, id, 1897);
    }
    // the tracker's done list: the foundation 501 waits, 503 is the nearest standing one
    let key = t.tracker_key(0, 1897);
    for id in [503, 502, 500] {
        t.players[0].tracker.complete(id, key);
    }
    assert_eq!(t.table_drop_site(0, [10.0, 10.0], 0, 58), Some(503));
    assert_eq!(t.table_drop_site(0, [10.0, 10.0], 1, 58), None);
    let before = t.players[0].objects[0].clone();
    assert!(t.drop_off_unasked(0, 0, true).is_ok());
    assert_eq!(t.players[0].objects[0], before);
}

/// `UNIT_ORDER`'s drop-off on a builder carrying wood (`FUN_14161e370` on a Build: slot 0x78,
/// `FUN_140a78060`, then `set_state(8)`): it leaves its foundation's builders, takes the
/// drop-site table's nearest mill (`FUN_141658e20`) as its second target and walks to it
/// within the Build task's range (its master has no task on the mill). Without a mill in the
/// table it walks back to its foundation; carrying nothing it is not ordered at all.
#[test]
fn a_unit_order_drop_off_sends_a_carrying_builder_to_the_table_s_drop_site() {
    use crate::map::spatial::Placed;
    let placed = |position, master, radius| Placed {
        position,
        radius,
        class: 3,
        player: 0,
        master,
        obstruction_type: 2,
        buildable_on: 0,
        tall: true,
        speed: 0.0,
        unknown: None,
        obstruction: None,
    };
    let villager = Unit {
        id: 10,
        position: [10.0, 10.0, 0.0],
        life: crate::objects::lifecycle::Lifecycle {
            master_id: 83,
            state: 2,
            attribute_type: 1,
            amount: 5.0,
            ..crate::objects::lifecycle::Lifecycle::default()
        },
        mover: Some(crate::gathering::hunt::tests::walker()),
        build: Some(crate::construction::build::BuildAction {
            state: 6,
            added: true,
            ..crate::construction::build::BuildAction::start(
                600,
                [5.0, 10.0, 0.0],
                0.0,
                crate::construction::build::BuildTask {
                    range: 0.1,
                    work_value: 1.0,
                    work_rate: 1.0,
                    resource: -1,
                    id: -1,
                },
            )
        }),
        ..Unit::default()
    };
    let foundation = Unit {
        id: 600,
        position: [5.0, 10.0, 0.0],
        life: crate::objects::lifecycle::Lifecycle {
            master_id: 70,
            ..crate::objects::lifecycle::Lifecycle::default()
        },
        construction: Some(crate::construction::build::Construction {
            progress: 1.0,
            desolid: false,
            builders: 1,
            hit_points: 550,
            build_time: 25,
            record: crate::map::spatial::Obstruction { kind: 2, class: 2 },
            snow: false,
            sprite_override: false,
            started: false,
        }),
        ..Unit::default()
    };
    let mill = Unit {
        id: 503,
        position: [15.0, 10.0, 0.0],
        life: crate::objects::lifecycle::Lifecycle {
            master_id: 68,
            state: 2,
            ..crate::objects::lifecycle::Lifecycle::default()
        },
        ..Unit::default()
    };
    let world = || {
        let mut t = World::new(
            0,
            1,
            0,
            vec![0],
            vec![crate::world::Player {
                objects: vec![villager.clone(), foundation.clone(), mill.clone()],
                ..crate::world::Player::default()
            }],
        );
        t.masters = vec![crate::objects::masters::Masters::new([
            Master {
                id: 83,
                kind: 70,
                class: 4,
                radius: [0.2, 0.2],
                ..Master::default()
            },
            Master {
                id: 70,
                kind: 80,
                class: 3,
                radius: [1.0, 1.0],
                ..Master::default()
            },
            Master {
                id: 68,
                kind: 80,
                class: 3,
                radius: [1.0, 1.0],
                ..Master::default()
            },
        ])];
        t.spatial
            .insert(10, placed([10.0, 10.0, 0.0], 83, [0.2, 0.2]));
        t.spatial
            .insert(600, placed([5.0, 10.0, 0.0], 70, [1.0, 1.0]));
        t.spatial
            .insert(503, placed([15.0, 10.0, 0.0], 68, [1.0, 1.0]));
        t.track_add(0, 503, 68);
        let key = t.tracker_key(0, 68);
        t.players[0].tracker.complete(503, key);
        t
    };
    let mut t = world();
    t.drop_sites = vec![crate::unit_ai::task_finder::DropSite {
        building: 68,
        worker_class: 4,
        resource_types: vec![1],
        ..crate::unit_ai::task_finder::DropSite::default()
    }];
    t.unit_order(0, &[10], 8).unwrap();
    let build = t.players[0].objects[0].build.unwrap();
    assert_eq!(
        (build.state, build.site, build.walk_target, build.drop_range),
        (crate::construction::build::DROP, 503, 503, 0.1)
    );
    assert_eq!(
        (build.move_to, build.added),
        (Some(crate::movement::move_to::MoveTo::Start), false)
    );
    assert_eq!(
        t.players[0].objects[1].construction.map(|k| k.builders),
        Some(0)
    );
    assert_eq!(t.players[0].objects[2].workers, 1);
    // no table entry for wood: back to the foundation
    let mut t = world();
    t.unit_order(0, &[10], 8).unwrap();
    let build = t.players[0].objects[0].build.unwrap();
    assert_eq!((build.state, build.site, build.walk_target), (4, -1, 600));
    // carrying nothing: not ordered
    let mut t = world();
    t.players[0].objects[0].life.amount = 0.0;
    t.unit_order(0, &[10], 8).unwrap();
    assert_eq!(t.players[0].objects[0].build.map(|b| b.state), Some(6));
    // 7 asks the Build as 8 does; 6 does not ask it: the search alone (no drop site in the
    // table here, nothing to work on)
    let mut t = world();
    t.drop_sites = vec![crate::unit_ai::task_finder::DropSite {
        building: 68,
        worker_class: 4,
        resource_types: vec![1],
        ..crate::unit_ai::task_finder::DropSite::default()
    }];
    t.unit_order(0, &[10], 7).unwrap();
    assert_eq!(
        t.players[0].objects[0].build.map(|b| (b.state, b.site)),
        Some((crate::construction::build::DROP, 503))
    );
    let mut t = world();
    t.unit_order(0, &[10], 6).unwrap();
    assert_eq!(t.players[0].objects[0].build.map(|b| b.state), Some(6));
    assert!(t.unit_order(0, &[10], 3).is_err());
}

/// The work command's Attack (`0x140a7f8c0`) of a soldier that may move, touching Gaia's
/// wolf: the start ends in `set_state(4)`, whose `MoveTo` to the target starts arrived (state
/// 1 within 0.01, `FUN_140aa2100`; 426954810's p6 at 1730). Its `set_state(1)`
/// (`0x140aa29e0`, not quiet) shows the standing sprite (slot 0x798) at the standing speed and
/// clears `+0x7b8`; the Attack waits in state 4 for the `MoveTo`'s end.
#[test]
fn a_work_attack_on_a_touching_target_starts_its_move_to_arrived() {
    let (mut t, task) = work_attack(0);
    t.start_attack(1, 0, 50, &task, [10.4, 10.0, 0.0], true)
        .unwrap();
    let unit = &t.players[1].objects[0];
    let attack = unit.attack.as_ref().expect("the soldier's Attack");
    assert_eq!(
        (attack.state, attack.move_to, attack.target2),
        (
            crate::combat::attack::MOVE,
            Some(crate::movement::move_to::MoveTo::Arrived),
            50
        )
    );
    assert_eq!(unit.sprite.map(|s| s.graphic.id), Some(300));
    assert_eq!(unit.anim_speed, Some(0.0));
    assert_eq!(unit.mover.as_ref().map(|m| m.stats.speed), Some(0.0));
}

/// The attack task's aim (`FUN_141189b20`): Gaia's wolf aims at itself; a building annex at
/// its main building, with the annex's owner, class and fog visibility but the main building's
/// place and class; a target or a main building the slice does not hold fails.
#[test]
fn an_attack_task_aims_at_its_target_or_an_annex_s_main_building() {
    let (mut t, _) = work_attack(0);
    let aim = t.attack_aim(50).unwrap();
    assert_eq!((aim.owner, aim.class, aim.fog_visibility), (0, 10, 0));
    assert_eq!(
        (aim.aimed, aim.at, aim.aimed_class),
        (50, [10.4, 10.0, 0.0], 10)
    );
    annexed(&mut t);
    let aim = t.attack_aim(61).unwrap();
    assert_eq!((aim.owner, aim.class, aim.fog_visibility), (0, 52, 1));
    assert_eq!(
        (aim.aimed, aim.at, aim.aimed_class),
        (60, [20.5, 20.5, 0.0], 3)
    );
    let r = t.attack_aim(98);
    assert_eq!(
        r.err().map(|e| e.reason()),
        Some("an attack task's target the slice does not hold")
    );
    t.players[0].objects[2].annex_of = Some(99);
    let r = t.attack_aim(61);
    assert_eq!(
        r.err().map(|e| e.reason()),
        Some("an attack task's main building is gone")
    );
}

/// The attack task's word to its unit AI (`FUN_140c64b50`, `FUN_140c64c10`, `FUN_14106e4e0`
/// kind 9): the aimed main building becomes the soldier's last target, the annex its target
/// with its class at the work's point under order 700 and action 600; the last retarget time
/// and the order history take the world's time, and the annex counts the soldier as a worker.
/// A soldier without a unit AI only takes the target.
#[test]
fn an_attack_task_tells_its_unit_ai_the_order_the_action_and_the_target() {
    let (mut t, _) = work_attack(0);
    annexed(&mut t);
    t.world_time = 1730;
    let aim = t.attack_aim(61).unwrap();
    t.attack_notifies(1, 0, (61, &aim), [21.0, 21.0, 0.0], true)
        .unwrap();
    let ai = t.players[1].objects[0].ai.as_ref().unwrap();
    assert_eq!(
        (ai.current_order, ai.current_action, ai.current_target),
        (700, 600, 61)
    );
    assert_eq!((ai.target_type, ai.target_point), (52, [21.0, 21.0, 0.0]));
    assert_eq!(
        (ai.last_target, ai.last_target_type, ai.last_retarget_time),
        (60, 3, 1730)
    );
    let record = ai.order_history.last().expect("the order history");
    assert_eq!(
        (record.order, record.action, record.time, record.target),
        (700, 600, 1730, 61)
    );
    assert_eq!(t.players[0].objects[2].workers, 1);
    t.players[1].objects[0].ai = None;
    t.attack_notifies(1, 0, (61, &aim), [21.0, 21.0, 0.0], true)
        .unwrap();
    assert_eq!(t.players[0].objects[2].workers, 2);
}

/// The attack task's Attack of a soldier without a unit AI (it cannot move) touching the wolf
/// goes ready at its start (`set_state(6)`, [`super::attack::ready_at_start`]): with its reload
/// under way it waits ready on its standing sprite; with it done it swings at once on its
/// attack sprite.
#[test]
fn an_attack_task_of_a_unit_that_cannot_move_goes_ready_in_reach() {
    use crate::combat::attack::{READY, SWING};
    for (timer, state, pending, sprite) in [(0.5, READY, false, 300), (0.0, SWING, true, 301)] {
        let (mut t, task) = work_attack(0);
        let unit = &mut t.players[1].objects[0];
        unit.ai = None;
        unit.attack_timer = timer;
        t.start_attack(1, 0, 50, &task, [10.4, 10.0, 0.0], true)
            .unwrap();
        let unit = &t.players[1].objects[0];
        let attack = unit.attack.expect("the soldier's Attack");
        assert_eq!(
            (attack.state, attack.pending, attack.target2),
            (state, pending, 50),
            "reload timer {timer}"
        );
        assert_eq!(unit.sprite.map(|s| s.graphic.id), Some(sprite));
    }
}

/// [`super::attack::ready_at_start`] on an Attack of its own with neither a target nor a
/// point: at ready it posts the action's end (0x1fa), a part of the swing the start does not
/// model, and fails.
#[test]
fn an_attack_task_s_ready_start_fails_on_a_post_it_does_not_model() {
    use super::attack::{StartStrike, ready_at_start};
    let (mut t, task) = work_attack(0);
    let unit = &mut t.players[1].objects[0];
    unit.ai = None;
    unit.attack_timer = 0.5;
    t.start_attack(1, 0, 50, &task, [10.4, 10.0, 0.0], true)
        .unwrap();
    let unit = &mut t.players[1].objects[0];
    let mut attack = unit.attack.take().expect("the soldier's Attack");
    assert!(!attack.sub_action);
    attack.target = -1;
    attack.point = [crate::math::image::attack::NONE; 3];
    let start = StartStrike {
        standing: t.graphics[&300],
        volleys: false,
        reload: 2.0,
        hit_chance: 100,
        swing_sprites: crate::combat::attack::SwingSprites::default(),
        computer: false,
        world_time: 0,
        own: crate::combat::damage::Victim::default(),
    };
    let r = ready_at_start(
        &mut attack,
        unit,
        &mut t.spatial,
        &mut crate::math::rng::Well512::default(),
        start,
        &mut crate::combat::damage::Victim::default(),
    );
    assert_eq!(
        r.err().map(|e| e.reason()),
        Some("an attack task's swing at its start")
    );
}

/// Gaia's main building 60 (master 200, class 3) at (20.5, 20.5) and its annex 61 (master 201,
/// class 52) at (22, 20.5), both fog visible, after the wolf of [`work_attack`]'s world.
fn annexed(t: &mut World) {
    let wolf = t.masters[0].get(126).cloned().expect("the wolf's master");
    t.masters[0] = crate::objects::masters::Masters::new([
        wolf,
        Master {
            id: 200,
            kind: BUILDING,
            class: 3,
            fog_visibility: 1,
            ..Master::default()
        },
        Master {
            id: 201,
            kind: BUILDING,
            class: 52,
            fog_visibility: 1,
            ..Master::default()
        },
    ]);
    let building = |id, master_id, position, annex_of| Unit {
        id,
        position,
        annex_of,
        life: crate::objects::lifecycle::Lifecycle {
            master_id,
            state: 2,
            ..crate::objects::lifecycle::Lifecycle::default()
        },
        ..Unit::default()
    };
    t.players[0]
        .objects
        .push(building(60, 200, [20.5, 20.5, 0.0], None));
    t.players[0]
        .objects
        .push(building(61, 201, [22.0, 20.5, 0.0], Some(60)));
}

/// [`soldiers`]' soldier 11 (master 4, melee, reload 2) at (10, 10) on its feet with a walker
/// of speed 1.2 and a unit AI of `stance`, touching Gaia's wolf 50 (master 126, class 10, 25
/// hit points) at (10.4, 10), and the attack task (action type 7, no sprites of its own).
fn work_attack(stance: u8) -> (World, crate::objects::masters::Task) {
    use crate::map::spatial::Placed;
    use crate::math::rng::Well512;
    use crate::objects::sprite::Graphic;
    let standing = Graphic {
        id: 300,
        frames: 10,
        facets: 8,
        speed_multiplier: 1.0,
        frame_duration: 0.1,
        replay_delay: 0.0,
        flags: 1,
    };
    let mut t = soldiers(&[(11, 4, [10.0, 10.0])], &[]);
    t.graphics = std::collections::BTreeMap::from([
        (300, standing),
        (
            301,
            Graphic {
                id: 301,
                ..standing
            },
        ),
        (
            302,
            Graphic {
                id: 302,
                ..standing
            },
        ),
    ]);
    let mut walker = crate::gathering::hunt::tests::walker();
    walker.stats.speed = 1.2;
    walker.stats.walk_speed = 1.2;
    walker.stats.stand_speed = 0.0;
    let soldier = t.masters[1].get_mut(4).unwrap();
    soldier.standing_sprite = 300;
    soldier.attack_sprite = 301;
    soldier.move_sprite = 302;
    soldier.weapon_range = 0.0;
    soldier.reload = 2.0;
    soldier.moving = Some(walker.stats);
    t.masters[0] = crate::objects::masters::Masters::new([Master {
        id: 126,
        kind: 70,
        class: 10,
        ..Master::default()
    }]);
    t.players[0].objects.push(Unit {
        id: 50,
        position: [10.4, 10.0, 0.0],
        life: crate::objects::lifecycle::Lifecycle {
            master_id: 126,
            state: 2,
            hp: 25.0,
            ..crate::objects::lifecycle::Lifecycle::default()
        },
        ..Unit::default()
    });
    let placed = |x: f32, class, player, master| Placed {
        position: [x, 10.0, 0.0],
        radius: [0.2, 0.2],
        class,
        player,
        master,
        obstruction_type: 0,
        buildable_on: 0,
        tall: true,
        speed: 0.0,
        unknown: None,
        obstruction: None,
    };
    t.spatial = crate::map::spatial::Spatial::new([120, 120]);
    t.spatial.insert(11, placed(10.0, 0, 1, 4));
    t.spatial.insert(50, placed(10.4, 10, 0, 126));
    let mut rng = Well512::default();
    let unit = &mut t.players[1].objects[0];
    unit.mover = Some(walker);
    unit.sprite = Some(crate::objects::sprite::ActiveSprite::start(
        Graphic {
            id: 302,
            ..standing
        },
        &mut rng,
    ));
    unit.anim_speed = Some(1.0);
    unit.ai = Some(crate::unit_ai::UnitAi {
        ai_type: 6,
        current_order: -1,
        current_action: -1,
        current_target: -1,
        stance,
        ..crate::unit_ai::UnitAi::default()
    });
    let task = crate::objects::masters::Task {
        action_type: ATTACK_TASK,
        sprites: [-1; 4],
        ..crate::objects::masters::Task::default()
    };
    (t, task)
}

/// A world of player 1's soldiers (master 4, formation category 3) and villagers (master 83,
/// category 0) at `units`' places, auto formations on with an influence of 3 tiles, and one
/// group of id 5 holding `members`.
fn soldiers(units: &[(i32, i16, [f32; 2])], members: &[i32]) -> World {
    let objects = units
        .iter()
        .map(|&(id, master, [x, y])| Unit {
            id,
            position: [x, y, 0.0],
            group: crate::groups::GroupLink {
                id: if members.contains(&id) { 5 } else { -1 },
                flag: -2,
            },
            life: crate::objects::lifecycle::Lifecycle {
                master_id: master,
                state: 2,
                ..crate::objects::lifecycle::Lifecycle::default()
            },
            ..Unit::default()
        })
        .collect();
    let mut t = World::new(
        0,
        1,
        0,
        vec![0, 0],
        vec![
            crate::world::Player::default(),
            crate::world::Player {
                objects,
                formations: crate::world::Formations {
                    auto: true,
                    influence: 3.0,
                    by_speed: false,
                    ..crate::world::Formations::default()
                },
                ..crate::world::Player::default()
            },
        ],
    );
    let master = |id, formation_category| Master {
        id,
        kind: 70,
        formation_category,
        ..Master::default()
    };
    t.masters = vec![
        crate::objects::masters::Masters::new([]),
        crate::objects::masters::Masters::new([master(4, 3), master(83, 0)]),
    ];
    if !members.is_empty() {
        t.groups.groups.push(crate::groups::UnitGroup {
            id: 5,
            members: members
                .iter()
                .map(|&id| crate::groups::Member {
                    id,
                    ..crate::groups::Member::default()
                })
                .collect(),
            ..crate::groups::UnitGroup::default()
        });
    }
    t
}

/// `FUN_140fd76d0`: villagers move loose, soldiers within the influence distance of each
/// other form a group (the one with the most joiners first, by id), one too far moves loose;
/// with Ctrl held each soldier is a group of its own.
#[test]
fn the_splitter_groups_soldiers_near_each_other_and_leaves_villagers_loose() {
    let units = [
        (10, 83, [10.0, 10.0]),
        (11, 4, [20.0, 20.0]),
        (12, 4, [21.0, 20.0]),
        (13, 4, [23.5, 20.0]),
        (14, 4, [40.0, 20.0]),
    ];
    let mut t = soldiers(&units, &[]);
    let split = t.split(&[14, 13, 12, 11, 10], &|_| true, false).unwrap();
    // 12 has two to join it (11 and 13), 11 and 13 one each
    assert_eq!(split.groups, [vec![12, 11, 13]]);
    assert_eq!(split.loose, [10, 14]);
    let split = t.split(&[14, 13, 12, 11, 10], &|_| true, true).unwrap();
    assert_eq!(split.loose, [10]);
    assert_eq!(split.groups.len(), 4);
    // without auto formations every unit moves loose
    t.players[1].formations.auto = false;
    let split = t.split(&[14, 13, 12, 11, 10], &|_| true, false).unwrap();
    assert_eq!(split.loose, [10, 11, 12, 13, 14]);
    assert_eq!(split.groups, Vec::<Vec<i32>>::new());
}

/// `FUN_140fd76d0`'s first pass: units in a group the world holds keep their group with the
/// order's other members, auto formations or not; a member ordered alone moves loose.
#[test]
fn the_splitter_keeps_the_order_s_members_of_an_existing_group_together() {
    let units = [
        (11, 4, [20.0, 20.0]),
        (12, 4, [60.0, 20.0]),
        (13, 4, [21.0, 20.0]),
    ];
    let mut t = soldiers(&units, &[11, 12]);
    t.players[1].formations.auto = false;
    let split = t.split(&[13, 12, 11], &|_| true, false).unwrap();
    assert_eq!(split.groups, [vec![11, 12]]);
    assert_eq!(split.loose, [13]);
    let split = t.split(&[12, 13], &|_| true, false).unwrap();
    assert_eq!(split.loose, [12, 13]);
}

/// `FUN_140e55a30`: of the first 60 ids (500 for a computer) one the world lacks or another
/// player's (without the setting `+0x1074`) becomes -1; a human's command left without a unit
/// is refused.
#[test]
fn the_command_check_drops_gone_units_and_others_units_of_its_first_60() {
    let units = [(11, 4, [20.0, 20.0]), (12, 4, [21.0, 20.0])];
    let mut t = soldiers(&units, &[]);
    t.players[0].objects.push(Unit {
        id: 30,
        ..Unit::default()
    });
    assert_eq!(
        t.command_check(1, &[11, 30, 99, -1]),
        Some(vec![11, -1, -1, -1])
    );
    t.settings.cheats_enabled = true;
    assert_eq!(t.command_check(1, &[11, 30, 99]), Some(vec![11, 30, -1]));
    t.settings.cheats_enabled = false;
    assert_eq!(t.command_check(1, &[30, 99]), None);
    t.players[1].computer = true;
    assert_eq!(t.command_check(1, &[30, 99]), Some(vec![-1, -1]));
    t.players[1].computer = false;
    // past the first 60 nothing is checked
    let mut ids = vec![99; 60];
    ids.push(30);
    assert_eq!(t.command_check(1, &ids), None);
    ids[0] = 11;
    let checked = t.command_check(1, &ids).unwrap();
    assert_eq!((checked[0], checked[1], checked[60]), (11, -1, 30));
}

/// `FUN_140e6b750`: each id's object, live or sleeping, None for a gone one, in the ids' order.
#[test]
fn the_lookup_keeps_a_gone_unit_s_entry() {
    let units = [(11, 4, [20.0, 20.0])];
    let mut t = soldiers(&units, &[]);
    t.players[1].sleeping.push(Unit {
        id: 40,
        ..Unit::default()
    });
    assert_eq!(
        t.lookup(&[99, 40, 11, -1]),
        [
            None,
            Some(Place::Sleeping(1, 0)),
            Some(Place::Live(1, 0)),
            None
        ]
    );
}

/// `FUN_14161e200`: the units of a STOP leave their unit group unless they are exactly its
/// members (a gone unit's entry counts in the list); each unit AI's order queue empties and
/// its idle timer takes its adjusted timeout; another player's unit the check dropped and a
/// sleeping object take nothing.
#[test]
fn a_stop_takes_its_units_out_of_their_group_and_empties_their_orders() {
    let units = [(11, 4, [20.0, 20.0]), (12, 4, [21.0, 20.0])];
    let mut t = soldiers(&units, &[11, 12]);
    let order = crate::unit_ai::QueuedOrder {
        issuer: 1,
        order: MOVE_ORDER,
        priority: 100,
        target: -1,
        target_owner: -1,
        point: [1.0, 1.0, 0.0],
        range: 0.0,
        shown: [1.0, 1.0],
        ready: true,
        target_type: -1,
    };
    t.players[1].objects[0].ai = Some(crate::unit_ai::UnitAi {
        orders: vec![order],
        idle_timer: 5,
        adjusted_idle_timeout: 300,
        ..crate::unit_ai::UnitAi::default()
    });
    let ai = t.players[1].objects[0].ai.clone();
    t.players[0].objects.push(Unit {
        id: 30,
        ai,
        ..Unit::default()
    });
    t.players[1].sleeping.push(Unit {
        id: 40,
        ..Unit::default()
    });
    t.stop_command(1, &[11, 12, 99, 30, 40]).unwrap();
    let ai = t.players[1].objects[0].ai.as_ref().unwrap();
    assert_eq!((ai.orders.len(), ai.idle_timer), (0, 300));
    assert_eq!(t.players[1].objects[0].group.id, -1);
    assert_eq!(t.players[1].objects[1].group.id, -1);
    assert_eq!(t.groups.get(5).unwrap().members.len(), 0);
    assert_eq!(t.players[0].objects[0].ai.as_ref().unwrap().orders.len(), 1);
}

/// With the game setting `+0x1074` (`mCheatsEnabled`) the command's check passes another
/// player's unit, and MOVE and INTERACT (no owner test past it) order it as its own player's:
/// a queued order joins its unit AI's queue with the issuer as the order's.
#[test]
fn an_order_to_another_player_s_unit_goes_to_its_queue_with_the_setting() {
    let mut t = soldiers(&[(11, 83, [20.0, 20.0])], &[]);
    t.settings.cheats_enabled = true;
    t.players.push(crate::world::Player::default());
    t.player_time.push(0);
    t.masters.push(crate::objects::masters::Masters::new([]));
    let walker = crate::gathering::hunt::tests::walker();
    if let Some(m) = t.masters[1].get_mut(83) {
        m.moving = Some(walker.stats);
    }
    let unit = &mut t.players[1].objects[0];
    unit.mover = Some(walker);
    unit.ai = Some(crate::unit_ai::UnitAi::default());
    assert_eq!(t.checked_units(2, &[11]), [11]);
    let order = MoveOrder {
        target: -1,
        point: [30.0, 30.0],
        queued: true,
        direct: false,
        human: false,
        control: false,
    };
    t.move_order(2, &[11], &order).unwrap();
    let interact = Interact {
        target: -1,
        point: [25.0, 30.0],
        carry_test: false,
        queued: true,
        direct: false,
        human: false,
        filter: -1,
    };
    t.interact(2, &[11], &interact).unwrap();
    let ai = t.players[1].objects[0].ai.as_ref().unwrap();
    let queued: Vec<(i32, i32)> = ai.orders.iter().map(|o| (o.issuer, o.order)).collect();
    assert_eq!(
        queued,
        [(2, MOVE_ORDER), (2, crate::unit_ai::ORDER_ON_TARGET)]
    );
}

/// MOVE and INTERACT (`FUN_140e67800`, `FUN_140e68b90`): their loose units in a group the
/// world holds leave it (`FUN_14102c160`), one unit or several; a unit in no group, or in a
/// group the world does not hold, stays as it is.
#[test]
fn the_loose_units_of_a_move_or_an_interact_leave_their_groups() {
    let units = [(11, 83, [20.0, 20.0]), (12, 83, [40.0, 20.0])];
    let order = MoveOrder {
        target: -1,
        point: [30.0, 30.0],
        queued: false,
        direct: false,
        human: false,
        control: false,
    };
    let mut t = soldiers(&units, &[11]);
    t.move_order(1, &[11], &order).unwrap();
    assert_eq!(t.players[1].objects[0].group.id, -1);
    assert_eq!(t.groups.get(5).unwrap().members.len(), 0);
    // two groups' members ordered alone: both loose
    let two = || {
        let mut t = soldiers(&units, &[11]);
        t.groups.groups.push(crate::groups::UnitGroup {
            id: 6,
            members: vec![crate::groups::Member {
                id: 12,
                ..crate::groups::Member::default()
            }],
            ..crate::groups::UnitGroup::default()
        });
        t.players[1].objects[1].group.id = 6;
        t
    };
    let mut t = two();
    t.move_order(1, &[11, 12], &order).unwrap();
    assert_eq!(t.players[1].objects[0].group.id, -1);
    assert_eq!(t.players[1].objects[1].group.id, -1);
    assert_eq!(t.groups.get(5).unwrap().members.len(), 0);
    assert_eq!(t.groups.get(6).unwrap().members.len(), 0);
    // a group link the world does not hold
    let mut t = soldiers(&units, &[]);
    t.players[1].objects[0].group.id = 9;
    t.move_order(1, &[11], &order).unwrap();
    assert_eq!(t.players[1].objects[0].group.id, 9);
    let interact = Interact {
        target: -1,
        point: [30.0, 30.0],
        carry_test: false,
        queued: false,
        direct: false,
        human: false,
        filter: -1,
    };
    let mut t = two();
    for u in &mut t.players[1].objects {
        u.pasture = -1;
        u.my_pasture = -1;
    }
    t.interact(1, &[11, 12], &interact).unwrap();
    assert_eq!(t.players[1].objects[0].group.id, -1);
    assert_eq!(t.players[1].objects[1].group.id, -1);
}

/// A queued MOVE's new group (`FUN_140e67800` at `0x140e67b5d`): the first of its units the
/// world holds (99 is not), with a unit AI, names the target the group's first command takes
/// when the world holds it, at its place; a unit without a unit AI or a target the world
/// does not hold names none.
#[test]
fn a_queued_move_s_new_group_takes_its_first_unit_s_target() {
    let units = [(11, 4, [20.0, 20.0]), (13, 4, [21.0, 20.0])];
    let ai = |target| crate::unit_ai::UnitAi {
        current_order: -1,
        current_action: -1,
        current_target: target,
        ..crate::unit_ai::UnitAi::default()
    };
    let mut t = soldiers(&units, &[]);
    t.players[1].objects[0].ai = Some(ai(13));
    t.players[1].objects[1].ai = Some(ai(11));
    assert_eq!(t.first_unit_target(&[99, 11, 13]), Some((13, [21.0, 20.0])));
    assert_eq!(t.first_unit_target(&[13, 11]), Some((11, [20.0, 20.0])));
    t.players[1].objects[0].ai = Some(ai(98));
    assert_eq!(t.first_unit_target(&[11, 13]), None);
    t.players[1].objects[0].ai = None;
    assert_eq!(t.first_unit_target(&[11, 13]), None);
    assert_eq!(t.first_unit_target(&[99]), None);
}

/// GATE and `FUN_1411dba30`: player 1's building 50 toggles its lock, a lock setting its last
/// move time to the world time plus 1, an unlock leaving it; player 2's GATE of it does
/// nothing unless players may order each other's units; a unit (type 70) takes nothing.
#[test]
fn a_gate_command_toggles_the_lock_of_a_building_of_its_player() {
    let units = [(11, 4, [20.0, 20.0])];
    let mut t = soldiers(&units, &[]);
    t.world_time = 400;
    t.players.push(crate::world::Player::default());
    t.masters.push(crate::objects::masters::Masters::new([]));
    t.masters[1] = crate::objects::masters::Masters::new([
        Master {
            id: 4,
            kind: 70,
            ..Master::default()
        },
        Master {
            id: 79,
            kind: BUILDING,
            ..Master::default()
        },
    ]);
    t.players[1].objects.push(Unit {
        id: 50,
        gate: Some(crate::world::Gate::default()),
        mover: Some(crate::gathering::hunt::tests::walker()),
        life: crate::objects::lifecycle::Lifecycle {
            master_id: 79,
            state: 2,
            ..crate::objects::lifecycle::Lifecycle::default()
        },
        ..Unit::default()
    });
    let building = |t: &World| {
        let b = &t.players[1].objects[1];
        (b.gate.unwrap().locked, b.mover.as_ref().unwrap().last_move)
    };
    t.gate(1, 50).unwrap();
    assert_eq!(building(&t), (true, 401));
    t.world_time = 500;
    t.gate(2, 50).unwrap();
    assert_eq!(building(&t), (true, 401));
    t.settings.cheats_enabled = true;
    t.gate(2, 50).unwrap();
    assert_eq!(building(&t), (false, 401));
    t.gate(1, 11).unwrap();
    t.gate(1, 99).unwrap();
    assert_eq!(t.players[1].objects[0].gate, None);
}

/// `TRANSFORM_OBJECT` and `FUN_1411e6d90`: player 1's trophy 60 (master 1649) takes trophy
/// 1650, keeping its share of hit points, and in state 2 its build points take the new
/// master's build time; in state 1 they stay.
#[test]
fn a_transformed_building_takes_its_new_master_s_build_time_in_state_2() {
    let units = [(11, 4, [20.0, 20.0])];
    let mut t = soldiers(&units, &[]);
    let master = |id, build_time| Master {
        id,
        kind: BUILDING,
        hit_points: 100,
        build_time,
        volley_fire: f32::from(build_time / 15),
        ..Master::default()
    };
    t.masters[1] = crate::objects::masters::Masters::new([master(TROPHY, 10), master(1650, 30)]);
    let trophy = |state| Unit {
        id: 60,
        firepower: 7.0,
        construction: Some(crate::construction::build::Construction {
            progress: 10.0,
            desolid: false,
            builders: 0,
            hit_points: 100,
            build_time: 10,
            record: crate::map::spatial::Obstruction { kind: 2, class: 2 },
            snow: false,
            sprite_override: false,
            started: true,
        }),
        life: crate::objects::lifecycle::Lifecycle {
            master_id: TROPHY,
            state,
            hp: 50.0,
            ..crate::objects::lifecycle::Lifecycle::default()
        },
        ..Unit::default()
    };
    t.players[1].objects = vec![trophy(2)];
    t.transform_object(1, 60, 1650).unwrap();
    let unit = &t.players[1].objects[0];
    assert_eq!(unit.life.master_id, 1650);
    assert_eq!(unit.life.hp, 50.0);
    assert_eq!(unit.construction.unwrap().progress, 30.0);
    // FUN_141225b70 in state 2: the volleys are the new master's volley fire
    assert_eq!(unit.firepower, 2.0);
    t.players[1].objects = vec![trophy(1)];
    t.transform_object(1, 60, 1650).unwrap();
    assert_eq!(t.players[1].objects[0].construction.unwrap().progress, 10.0);
    assert_eq!(t.players[1].objects[0].firepower, 7.0);
}

/// INTERACT to an object that does not move (`FUN_140e68b90`): it takes the order at once
/// (slot 0x128); a queued order then goes on as a mover's and its unit AI queues it (order
/// 0x2db at the point), with no movement state to touch.
#[test]
fn a_queued_interact_to_an_object_that_does_not_move_also_queues_it() {
    let interact = |queued| Interact {
        target: -1,
        point: [25.0, 30.0],
        carry_test: false,
        queued,
        direct: false,
        human: false,
        filter: -1,
    };
    let run = |queued| {
        let mut t = soldiers(&[(11, 83, [20.0, 20.0])], &[]);
        if let Some(m) = t.masters[1].get_mut(83) {
            m.kind = BUILDING;
        }
        let unit = &mut t.players[1].objects[0];
        (unit.pasture, unit.my_pasture) = (-1, -1);
        unit.ai = Some(crate::unit_ai::UnitAi::default());
        t.interact(1, &[11], &interact(queued)).unwrap();
        let ai = t.players[1].objects[0].ai.clone().unwrap();
        ai.orders.iter().map(|o| o.order).collect::<Vec<_>>()
    };
    assert_eq!(run(false), Vec::<i32>::new());
    assert_eq!(run(true), [crate::unit_ai::ORDER_ON_TARGET]);
}

/// INTERACT neither queued nor direct to a unit AI holding an order not ready
/// (`FUN_140e68b90` at 0x140e6983a): the order goes into its queue not ready, the readiness
/// pass with 1 makes both ready, and its tail stops the unit (the object's slot 0x148,
/// `FUN_14108c050`, 0x140c42cd4): the AI's order ends and it forgets its targets.
#[test]
fn an_order_to_a_busy_unit_ai_readies_its_queue_and_stops_the_unit() {
    let mut t = soldiers(&[(11, 83, [20.0, 20.0])], &[]);
    let walker = crate::gathering::hunt::tests::walker();
    if let Some(m) = t.masters[1].get_mut(83) {
        m.moving = Some(walker.stats);
    }
    let unit = &mut t.players[1].objects[0];
    unit.mover = Some(walker);
    (unit.pasture, unit.my_pasture) = (-1, -1);
    unit.ai = Some(crate::unit_ai::UnitAi {
        current_order: MOVE_ORDER,
        current_action: crate::unit_ai::MOVE_ACTION,
        current_target: 99,
        orders: vec![crate::unit_ai::QueuedOrder {
            order: MOVE_ORDER,
            ready: false,
            ..crate::unit_ai::QueuedOrder::default()
        }],
        ..crate::unit_ai::UnitAi::default()
    });
    let interact = Interact {
        target: -1,
        point: [25.0, 30.0],
        carry_test: false,
        queued: false,
        direct: false,
        human: false,
        filter: -1,
    };
    t.interact(1, &[11], &interact).unwrap();
    let ai = t.players[1].objects[0].ai.clone().unwrap();
    let queue: Vec<_> = ai.orders.iter().map(|o| (o.order, o.ready)).collect();
    assert_eq!(
        queue,
        [(MOVE_ORDER, true), (crate::unit_ai::ORDER_ON_TARGET, true)]
    );
    assert_eq!((ai.current_order, ai.current_target), (-1, -1));
}

/// One unit of INTERACT (`FUN_140e68b90`): a unit the world does not hold takes nothing; a
/// queued order's unit AI queues it at the unit's offset from the centroid added to the order's
/// point, the order's own point shown, its ignore list the order's other units; a unit the order
/// targets itself takes it without a work command. A sleeping unit and a mover without movement
/// state are not modelled.
#[test]
fn an_interact_s_unit_queues_its_offset_point_and_takes_no_work_on_itself() {
    let interact = Interact {
        target: -1,
        point: [25.0, 30.0],
        carry_test: false,
        queued: true,
        direct: false,
        human: false,
        filter: -1,
    };
    let mover = || {
        let mut t = soldiers(&[(11, 83, [20.0, 20.0])], &[]);
        let walker = crate::gathering::hunt::tests::walker();
        if let Some(m) = t.masters[1].get_mut(83) {
            m.moving = Some(walker.stats);
        }
        let unit = &mut t.players[1].objects[0];
        unit.mover = Some(walker);
        (unit.pasture, unit.my_pasture) = (-1, -1);
        unit.ai = Some(crate::unit_ai::UnitAi::default());
        t.spatial.map = [100, 100];
        t
    };
    let mut t = mover();
    t.interact_unit(1, 99, &[], None, &interact).unwrap();
    assert_eq!(t, mover());
    t.interact_unit(1, 11, &[12], Some([21.0, 20.0]), &interact)
        .unwrap();
    let u = &t.players[1].objects[0];
    let m = u.mover.as_ref().unwrap();
    assert_eq!((m.waiting, &m.ignore_units[..]), (1, &[12][..]));
    let queued: Vec<_> =
        u.ai.as_ref()
            .unwrap()
            .orders
            .iter()
            .map(|o| (o.order, o.point, o.shown, o.ready))
            .collect();
    assert_eq!(
        queued,
        [(
            crate::unit_ai::ORDER_ON_TARGET,
            [24.0, 30.0, 0.0],
            [25.0, 30.0],
            false
        )]
    );
    let mut t = mover();
    let ai = t.players[1].objects[0].ai.as_mut().unwrap();
    ai.orders.push(crate::unit_ai::QueuedOrder::default());
    let itself = Interact {
        target: 11,
        queued: false,
        direct: true,
        ..interact
    };
    t.interact_unit(1, 11, &[], None, &itself).unwrap();
    let u = &t.players[1].objects[0];
    assert_eq!(u.ai.as_ref().unwrap().orders, []);
    assert_eq!(u.move_to, None);
    let mut t = mover();
    t.players[1].objects[0].mover = None;
    let reason = |t: &mut World| {
        t.interact_unit(1, 11, &[], None, &interact)
            .unwrap_err()
            .reason()
    };
    assert_eq!(reason(&mut t), "an ordered unit without movement state");
    let mut t = mover();
    let u = t.players[1].objects.remove(0);
    t.players[1].sleeping.push(u);
    assert_eq!(
        reason(&mut t),
        "an order to a unit the slice does not hold live"
    );
}

/// `FUN_140c42b60` with 1: from the last order to the first, an order not ready becomes
/// ready and a ready one leaves the queue, the others keeping their order; with 0 the ready
/// ones stay.
#[test]
fn the_readiness_pass_readies_the_unready_and_drops_the_ready() {
    let order = |order, ready| crate::unit_ai::QueuedOrder {
        order,
        ready,
        ..crate::unit_ai::QueuedOrder::default()
    };
    let mut orders = vec![
        order(1, true),
        order(2, false),
        order(3, true),
        order(4, false),
    ];
    let mut kept = orders.clone();
    readiness(&mut orders, true);
    assert_eq!(orders, [order(2, true), order(4, true)]);
    let mut none: Vec<crate::unit_ai::QueuedOrder> = Vec::new();
    readiness(&mut none, true);
    assert_eq!(none.len(), 0);
    // with 0 (BUILD's) the ready ones stay and every order is ready
    readiness(&mut kept, false);
    assert_eq!(
        kept,
        [
            order(1, true),
            order(2, true),
            order(3, true),
            order(4, true)
        ]
    );
}

/// `FUN_14106ff70`'s carried-resource test: a gather or hunt task of another resource than
/// the one held leaves the unit holding none of the task's (the food kinds 0 and 15 to 17
/// are one); an attack task (7 or 9) turns a unit's retargeting off unless it is a villager.
#[test]
fn the_carried_resource_test_drops_another_resource_and_stops_a_soldier_s_retargeting() {
    let task = |action_type, attribute_type| crate::objects::masters::Task {
        action_type,
        attribute_type,
        ..crate::objects::masters::Task::default()
    };
    let run = |master: i16, held: (i16, f32), task: crate::objects::masters::Task| {
        let mut t = soldiers(&[(11, master, [20.0, 20.0])], &[]);
        for (id, class) in [(83, 4), (4, 6)] {
            if let Some(m) = t.masters[1].get_mut(id) {
                m.class = class;
            }
        }
        let unit = &mut t.players[1].objects[0];
        (unit.life.attribute_type, unit.life.amount) = held;
        unit.ai = Some(crate::unit_ai::UnitAi::default());
        t.carried_resource_test(1, 0, &task).unwrap();
        let unit = &t.players[1].objects[0];
        (
            unit.life.attribute_type,
            unit.life.amount,
            unit.ai.as_ref().unwrap().retarget_disabled,
        )
    };
    // wood held, a gather of gold: none held, of gold
    assert_eq!(
        run(83, (1, 10.0), task(crate::gathering::gather::GATHER, 3)),
        (3, 0.0, 0)
    );
    // meat (17) held, a hunt of food (0): both food, kept
    assert_eq!(run(83, (17, 10.0), task(HUNT, 0)), (17, 10.0, 0));
    // the same resource: kept
    assert_eq!(
        run(83, (3, 4.0), task(crate::gathering::gather::GATHER, 3)),
        (3, 4.0, 0)
    );
    // an attack task: a soldier's retargeting goes off, a villager's stays on
    assert_eq!(run(4, (-1, 0.0), task(ATTACK_TASK, -1)), (-1, 0.0, 1));
    assert_eq!(run(83, (-1, 0.0), task(ATTACK_TASK, -1)), (-1, 0.0, 0));
}

/// `FUN_141216f10` to a point: the unit AI's order queue empties only with the unit's
/// `+0xa14`, which slot 0x128 with 0 clears while its work command runs; the move follows
/// either way.
#[test]
fn a_move_inside_a_work_command_with_0_keeps_the_order_queue() {
    let units = [(11, 4, [20.0, 20.0])];
    let mut t = soldiers(&units, &[]);
    let order = crate::unit_ai::QueuedOrder {
        issuer: 1,
        order: MOVE_ORDER,
        priority: 100,
        target: -1,
        target_owner: -1,
        point: [1.0, 1.0, 0.0],
        range: 0.0,
        shown: [1.0, 1.0],
        ready: true,
        target_type: -1,
    };
    t.players[1].objects[0].ai = Some(crate::unit_ai::UnitAi {
        orders: vec![order],
        ..crate::unit_ai::UnitAi::default()
    });
    t.move_point_order_with(1, 0, [30.0, 30.0, 0.0], false)
        .unwrap();
    let unit = &t.players[1].objects[0];
    assert_eq!(unit.ai.as_ref().unwrap().orders.len(), 1);
    assert_eq!(unit.move_to.unwrap().goal, Some([30.0, 30.0, 0.0]));
    t.move_point_order(1, 0, [31.0, 30.0, 0.0]).unwrap();
    assert_eq!(t.players[1].objects[0].ai.as_ref().unwrap().orders.len(), 0);
}

/// `FUN_140e666f0`: another player's scout takes the order as its owner's, a unit without
/// bit 4 nothing; of a human's ids only the first 60 count.
#[test]
fn an_auto_scout_reaches_any_player_s_scout_of_its_first_60_ids() {
    let units = [(11, 4, [20.0, 20.0])];
    let mut t = soldiers(&units, &[]);
    t.players.push(crate::world::Player::default());
    let scout = Master {
        id: 448,
        kind: 70,
        class: 12,
        object_flags: 0x10,
        ..Master::default()
    };
    t.masters
        .push(crate::objects::masters::Masters::new([scout.clone()]));
    t.masters[1] = crate::objects::masters::Masters::new([
        scout,
        Master {
            id: 4,
            kind: 70,
            ..Master::default()
        },
    ]);
    let ai = crate::unit_ai::UnitAi {
        current_order: -1,
        current_action: -1,
        ..crate::unit_ai::UnitAi::default()
    };
    let unit = |id| Unit {
        id,
        position: [20.0, 20.0, 0.0],
        ai: Some(ai.clone()),
        life: crate::objects::lifecycle::Lifecycle {
            master_id: 448,
            state: 2,
            ..crate::objects::lifecycle::Lifecycle::default()
        },
        ..Unit::default()
    };
    t.players[1].objects[0].ai = Some(ai.clone());
    t.players[1].objects.push(unit(12));
    t.players[2].objects.push(unit(30));
    let mut ids = vec![99; 59];
    ids.extend([30, 11, 12]);
    t.auto_scout(1, &ids).unwrap();
    let order =
        |t: &World, p: usize, i: usize| t.players[p].objects[i].ai.as_ref().unwrap().current_order;
    assert_eq!(order(&t, 2, 0), crate::unit_ai::AUTO_SCOUT_ORDER);
    assert_eq!(order(&t, 1, 0), -1);
    assert_eq!(order(&t, 1, 1), -1);
    t.auto_scout(1, &[12]).unwrap();
    assert_eq!(order(&t, 1, 1), crate::unit_ai::AUTO_SCOUT_ORDER);
}

/// `FUN_140c64d50`: the stance and its place are set; `NoAttack` on an attacking unit AI
/// stops its action (keeping an order not of the action), asks for its stopAction and
/// forgets its target; any other stance leaves them.
#[test]
fn no_attack_stops_an_attacking_unit_ai_and_forgets_its_target() {
    let attacking = crate::unit_ai::UnitAi {
        current_order: 700,
        current_action: 600,
        current_target: 40,
        target_type: 6,
        ..crate::unit_ai::UnitAi::default()
    };
    let mut ai = attacking;
    let mut fx = Vec::new();
    set_stance(&mut ai, 1, ([3.0, 4.0], 6), &mut fx).unwrap();
    assert_eq!((ai.stance, ai.state_position), (1, [3.0, 4.0]));
    assert_eq!((ai.current_action, ai.current_target), (600, 40));
    assert_eq!(fx, []);
    set_stance(&mut ai, 3, ([3.0, 5.0], 6), &mut fx).unwrap();
    assert_eq!(ai.stance, 3);
    // order 700 is the action's (600 + 100): the stop ends it, the state position the place
    assert_eq!((ai.current_order, ai.current_action), (-1, -1));
    assert_eq!((ai.current_target, ai.last_target), (-1, 40));
    assert_eq!(fx, [crate::unit_ai::Effect::Stop]);
}

/// `FUN_140e6a7c0` after `FUN_140e55d30`: another player's unit takes the stance only while
/// players may order each other's; a class-54 unit takes no stance 1.
#[test]
fn a_stance_reaches_another_player_s_unit_only_under_the_setting() {
    let units = [(11, 4, [20.0, 20.0])];
    let mut t = soldiers(&units, &[]);
    t.players.push(crate::world::Player::default());
    t.masters
        .push(crate::objects::masters::Masters::new([Master {
            id: 4,
            kind: 70,
            class: 54,
            ..Master::default()
        }]));
    t.players[1].objects[0].ai = Some(crate::unit_ai::UnitAi::default());
    t.players[2].objects.push(Unit {
        id: 30,
        ai: Some(crate::unit_ai::UnitAi::default()),
        life: crate::objects::lifecycle::Lifecycle {
            master_id: 4,
            state: 2,
            ..crate::objects::lifecycle::Lifecycle::default()
        },
        ..Unit::default()
    });
    let stance = |t: &World, p: usize| t.players[p].objects[0].ai.as_ref().unwrap().stance;
    t.unit_ai_state(1, &[11, 30], 2).unwrap();
    assert_eq!((stance(&t, 1), stance(&t, 2)), (2, 0));
    t.settings.cheats_enabled = true;
    t.unit_ai_state(1, &[30], 1).unwrap();
    assert_eq!(stance(&t, 2), 0);
    t.unit_ai_state(1, &[30], 3).unwrap();
    assert_eq!(stance(&t, 2), 3);
}

/// `FUN_1411a6a70` on a unit inside a building (`+0x2f0` set, `+0x2e8` 0): no sight slots and
/// no tile, list or record change; its record (for its way out) takes the new master's values,
/// its hit points their share, its storage leaves and comes as anyone's.
#[test]
fn a_master_change_inside_a_building_keeps_to_its_record() {
    let old = Master {
        id: 4,
        kind: 70,
        class: 6,
        hit_points: 40,
        radius: [0.2, 0.2],
        standing_sprite: -1,
        ..Master::default()
    };
    let next = Master {
        id: 5,
        kind: 70,
        class: 6,
        hit_points: 60,
        radius: [0.3, 0.3],
        standing_sprite: -1,
        ..Master::default()
    };
    let mut spatial = crate::map::spatial::Spatial::new([10, 10]);
    spatial.insert(
        7,
        crate::map::spatial::Placed {
            position: [3.5, 3.5, 0.0],
            radius: [0.2, 0.2],
            class: 6,
            player: 1,
            master: 4,
            obstruction_type: 0,
            buildable_on: 0,
            tall: false,
            speed: 0.0,
            unknown: None,
            obstruction: None,
        },
    );
    spatial.go_inside(7, [5.0, 5.0, 0.0]);
    let mut unit = Unit {
        id: 7,
        position: [5.0, 5.0, 0.0],
        life: crate::objects::lifecycle::Lifecycle {
            hp: 20.0,
            master_id: 4,
            inside: Some(100),
            ..crate::objects::lifecycle::Lifecycle::default()
        },
        ..Unit::default()
    };
    let mut pathing = crate::pathing::Pathing::default();
    let graphics = std::collections::BTreeMap::new();
    let mut rng = crate::math::rng::Well512::default();
    let world = SwitchWorld {
        spatial: &mut spatial,
        pathing: &mut pathing,
        graphics: &graphics,
        sprite: &mut rng,
    };
    let switched = switch_master(&mut unit, &old, &next, world).unwrap();
    assert!(switched.promoted);
    assert_eq!((unit.life.master_id, unit.life.hp), (5, 30.0));
    let record = spatial.get(7).copied().unwrap();
    assert_eq!((record.master, record.radius), (5, [0.3, 0.3]));
    assert!(!spatial.tile_list(5, 5).contains(&7));
}

/// Slot 0x90's unit part ([`switch_master`]) of `unit`, at (3.5, 3.5) in the open, from `old`
/// to `next`, with the graphics `shown`: what it leaves to its owner, the unit after it and its
/// record.
fn switched_in_the_open(
    mut unit: Unit,
    old: &Master,
    next: &Master,
    shown: &[i16],
) -> Result<(actions::Switched, Unit, crate::map::spatial::Placed), GenieError> {
    let mut spatial = crate::map::spatial::Spatial::new([10, 10]);
    spatial.insert(
        unit.id,
        crate::map::spatial::Placed {
            position: [3.5, 3.5, 0.0],
            radius: old.radius,
            class: old.class,
            player: 1,
            master: old.id,
            obstruction_type: old.obstruction_type,
            buildable_on: 0,
            tall: false,
            speed: 0.0,
            unknown: None,
            obstruction: Some(crate::map::spatial::Obstruction { kind: 5, class: 2 }),
        },
    );
    let mut pathing = crate::pathing::Pathing::default();
    let graphics = shown
        .iter()
        .map(|&id| {
            let g = crate::objects::sprite::Graphic {
                id,
                frames: 10,
                facets: 8,
                speed_multiplier: 1.0,
                frame_duration: 0.1,
                replay_delay: 0.0,
                flags: 1,
            };
            (id, g)
        })
        .collect();
    let mut rng = crate::math::rng::Well512::default();
    let world = SwitchWorld {
        spatial: &mut spatial,
        pathing: &mut pathing,
        graphics: &graphics,
        sprite: &mut rng,
    };
    let switched = switch_master(&mut unit, old, next, world)?;
    let record = spatial.get(unit.id).copied().unwrap();
    Ok((switched, unit, record))
}

/// The two masters of [`switched_in_the_open`]'s tests: 4 (40 hit points, standing sprite 10)
/// and 5 (60, sprite 11, volley fire 3), each with storage of several flags.
fn switch_masters() -> (Master, Master) {
    let old = Master {
        id: 4,
        kind: 70,
        class: 6,
        hit_points: 40,
        radius: [0.2, 0.2],
        standing_sprite: 10,
        storage: vec![
            (4, 1.0, 2),
            (11, 1.0, 16),
            (12, 2.0, 32),
            (13, 3.0, 1),
            (-1, 1.0, 2),
        ],
        ..Master::default()
    };
    let next = Master {
        id: 5,
        kind: 70,
        class: 6,
        hit_points: 60,
        radius: [0.3, 0.3],
        standing_sprite: 11,
        storage: vec![(4, 1.0, 2), (14, 1.0, 16)],
        volley_fire: 3.0,
        obstruction_type: 3,
        obstruction_class: 1,
        ..Master::default()
    };
    (old, next)
}

/// `FUN_141225b70` with 0 on a unit in the open below state 3: the old master's storage of the
/// flags 2, 16 and 32 leaves, the new master's of flag 2 comes; the hit points keep their share
/// of the new master's (at most them, at least 1); its record and obstruction take the new
/// master's values; it shows the new standing sprite; in state 2 its volleys are the new
/// master's volley fire. In state 3 only the powerups go.
#[test]
fn a_master_change_in_the_open_moves_the_storage_and_shows_the_new_standing_sprite() {
    let (old, next) = switch_masters();
    let unit = |hp, state| Unit {
        id: 7,
        position: [3.5, 3.5, 0.0],
        life: crate::objects::lifecycle::Lifecycle {
            hp,
            state,
            master_id: 4,
            ..crate::objects::lifecycle::Lifecycle::default()
        },
        ..Unit::default()
    };
    let mut sprites = crate::math::rng::Well512::default();
    let showing = Unit {
        sprite: Some(crate::objects::sprite::ActiveSprite::start(
            crate::objects::sprite::Graphic {
                id: 10,
                frames: 10,
                facets: 8,
                speed_multiplier: 1.0,
                frame_duration: 0.1,
                replay_delay: 0.0,
                flags: 1,
            },
            &mut sprites,
        )),
        ..unit(20.0, 2)
    };
    let (switched, u, record) = switched_in_the_open(showing, &old, &next, &[10, 11]).unwrap();
    assert_eq!(switched.leaving, [(4, -1.0), (11, -1.0), (12, -2.0)]);
    assert_eq!(switched.coming, [(4, 1.0)]);
    assert!(switched.promoted);
    assert_eq!((u.life.master_id, u.life.hp, u.firepower), (5, 30.0, 3.0));
    assert_eq!(u.sprite.map(|s| s.graphic.id), Some(11));
    assert_eq!(
        (record.master, record.radius, record.obstruction_type),
        (5, [0.3, 0.3], 3)
    );
    assert_eq!(
        record.obstruction,
        Some(crate::map::spatial::Obstruction { kind: 3, class: 1 })
    );
    let hp = |hp| {
        let (_, u, _) = switched_in_the_open(unit(hp, 1), &old, &next, &[10, 11]).unwrap();
        (u.life.hp, u.firepower)
    };
    assert_eq!(hp(50.0), (60.0, 0.0));
    assert_eq!(hp(0.5), (1.0, 0.0));
    let (switched, u, record) = switched_in_the_open(unit(20.0, 3), &old, &next, &[]).unwrap();
    assert!(!switched.promoted && switched.leaving.is_empty());
    assert_eq!((u.life.master_id, record.master), (4, 4));
}

/// [`switch_master`]'s unmodelled parts: a share that changes the damage sprites, a moving
/// unit's new master that does not move and a new standing sprite the data file lacks.
#[test]
fn a_master_change_s_unmodelled_parts_fail() {
    let (old, next) = switch_masters();
    let unit = Unit {
        id: 7,
        life: crate::objects::lifecycle::Lifecycle {
            hp: 0.1,
            master_id: 4,
            ..crate::objects::lifecycle::Lifecycle::default()
        },
        ..Unit::default()
    };
    let damaged = Master {
        damage_sprites: true,
        ..old.clone()
    };
    let reason = |r: Result<_, GenieError>| r.unwrap_err().reason();
    assert_eq!(
        reason(switched_in_the_open(
            unit.clone(),
            &damaged,
            &next,
            &[10, 11]
        )),
        "a master change that changes the damage sprites (FUN_1411859a0)"
    );
    let walking = Unit {
        mover: Some(crate::gathering::hunt::tests::walker()),
        ..unit.clone()
    };
    assert_eq!(
        reason(switched_in_the_open(walking, &old, &next, &[10, 11])),
        "a moving unit's new master does not move"
    );
    assert_eq!(
        reason(switched_in_the_open(unit, &old, &next, &[10])),
        "a new master's standing sprite the data file does not have"
    );
}

/// `FUN_1412066b0`: the attack, reload, work rate, most hit points and armor powerups go, the
/// hit points powerup and the reload stinger stay.
#[test]
fn a_master_change_clears_the_powerups_but_the_hit_points_one() {
    let mut unit = Unit {
        attack_powerup: 2.0,
        reload_powerup: 0.5,
        reload_stinger: 0.25,
        work_rate_powerup: 0.15,
        hp_powerups: [10.0, 20.0],
        armor_powerups: [1, 2],
        ..Unit::default()
    };
    clear_powerups(&mut unit);
    assert_eq!(
        (
            unit.attack_powerup,
            unit.reload_powerup,
            unit.work_rate_powerup
        ),
        (0.0, 0.0, 0.0)
    );
    assert_eq!(
        (unit.hp_powerups, unit.armor_powerups),
        ([10.0, 0.0], [0, 0])
    );
    assert_eq!(unit.reload_stinger, 0.25);
}

/// `FUN_140a97730`: an empty list and a `MoveTo` head take no move; an Artifact head takes
/// the point (its goal); an action the slice does not hold is refused.
#[test]
fn the_head_action_takes_a_move_only_where_its_slot_0x40_does() {
    let units = [(11, 4, [20.0, 20.0])];
    let mut t = soldiers(&units, &[]);
    let point = [25.0, 21.0, 0.0];
    assert_eq!(t.head_takes_move(1, 0, None, point), Ok(false));
    t.players[1].objects[0].move_to = Some(crate::movement::move_to::MoveToAction {
        run: false,
        state: crate::movement::move_to::MoveTo::Start,
        goal: Some([30.0, 30.0, 0.0]),
        target: -1,
        range: 0.0,
        wait: crate::movement::move_to::MoveWait::default(),
    });
    assert_eq!(t.head_takes_move(1, 0, Some(11), point), Ok(false));
    t.players[1].objects[0].move_to = None;
    t.players[1].objects[0].artifact = Some(crate::ownership::artifact::Artifact::new(false));
    assert_eq!(t.head_takes_move(1, 0, None, point), Ok(true));
    let artifact = t.players[1].objects[0].artifact.unwrap();
    assert_eq!(artifact.goal, Some(point));
    t.players[1].objects[0].artifact = None;
    t.players[1].objects[0].unmodelled_action = Some(GenieError::unsupported("a test's action"));
    assert!(t.head_takes_move(1, 0, None, point).is_err());
}

/// `FUN_14161c920`'s box: each unit's x is set against its own y, the units before the last
/// fold into the running least and greatest; units side by side on a diagonal pass, two near
/// each other far off it do not.
#[test]
fn follow_s_box_sets_each_unit_s_x_against_its_y() {
    assert_eq!(
        follow_spread(&[[20.0, 20.0], [21.0, 20.0]]),
        Some([20.5, 20.0])
    );
    assert_eq!(follow_spread(&[[10.0, 30.0], [10.5, 30.0]]), None);
    // the last unit's spread: 8 passes, more does not
    assert_eq!(follow_spread(&[[10.0, 18.0]]), Some([10.0, 18.0]));
    assert_eq!(follow_spread(&[[10.0, 18.5]]), None);
    // the running spread holds the units before the last
    assert_eq!(
        follow_spread(&[[10.0, 10.0], [30.0, 30.0], [30.0, 30.0]]),
        None
    );
    assert_eq!(follow_spread(&[]), Some([0.0, 0.0]));
}

/// A FOLLOW offset stays on the map: below 0 it is 0, past the size less 0.01 that.
#[test]
fn a_follow_offset_stays_on_the_map() {
    assert_eq!(follow_axis(-0.5, 120), 0.0);
    assert_eq!(follow_axis(130.0, 120), 119.99);
    assert_eq!(follow_axis(60.25, 120), 60.25);
    assert!(follow_axis(f32::NAN, 120).is_nan());
}

/// Player 1's soldiers 11 and 12 (moving, each with a unit AI) and player 2's 30 at (40, 20),
/// a master with a footprint of class 6; auto formations off, a map of 120 tiles.
fn followers() -> World {
    let units = [(11, 4, [20.0, 20.0]), (12, 4, [21.0, 20.0])];
    let mut t = soldiers(&units, &[]);
    t.players[1].formations.auto = false;
    t.spatial.map = [120, 120];
    let walker = crate::gathering::hunt::tests::walker();
    t.masters[1] = crate::objects::masters::Masters::new([Master {
        id: 4,
        kind: 70,
        moving: Some(walker.stats),
        ..Master::default()
    }]);
    for u in &mut t.players[1].objects {
        u.mover = Some(walker.clone());
        u.ai = Some(crate::unit_ai::UnitAi {
            current_order: -1,
            current_action: -1,
            current_target: -1,
            ..crate::unit_ai::UnitAi::default()
        });
    }
    t.players.push(crate::world::Player::default());
    t.masters
        .push(crate::objects::masters::Masters::new([Master {
            id: 5,
            kind: 70,
            class: 6,
            radius: [0.5, 0.5],
            ..Master::default()
        }]));
    t.players[2].objects.push(Unit {
        id: 30,
        position: [40.0, 20.0, 0.0],
        life: crate::objects::lifecycle::Lifecycle {
            master_id: 5,
            state: 2,
            ..crate::objects::lifecycle::Lifecycle::default()
        },
        ..Unit::default()
    });
    t.spatial.insert(
        30,
        crate::map::spatial::Placed {
            position: [40.0, 20.0, 0.0],
            radius: [0.5, 0.5],
            class: 6,
            player: 2,
            master: 5,
            obstruction_type: 0,
            buildable_on: 0,
            tall: false,
            speed: 0.0,
            unknown: None,
            obstruction: None,
        },
    );
    t
}

/// GUARD (`FUN_14161dee0`): each unit's AI takes the defend order on the target at once,
/// run outside its update: it stops, forgets its target and defends the target within 2.5
/// (the defend action, the target at its place), the order processed and gone. The ready
/// flag is the low byte of the target's x: a whole x leaves the order queued, unprocessed.
#[test]
fn a_guard_gives_its_units_the_defend_order_at_once() {
    let mut t = followers();
    t.players[2].objects[0].position = [20.3, 20.0, 0.0];
    t.spatial.set_position(30, [20.3, 20.0, 0.0]);
    t.guard(1, &[11], 30).unwrap();
    let ai = t.players[1].objects[0].ai.as_ref().unwrap();
    assert_eq!(
        (ai.current_order, ai.current_action, ai.order_priority),
        (crate::unit_ai::DEFEND_ORDER, 0x259, 100)
    );
    assert_eq!(
        (ai.current_target, ai.defend_target, ai.desired_distance),
        (30, 30, GUARD_RANGE)
    );
    assert_eq!(ai.target_point, [20.3, 20.0, 0.0]);
    assert_eq!(ai.orders.len(), 0);
    // a whole x: the order waits in the queue, the unit stopped and without a target
    let mut t = followers();
    t.guard(1, &[12], 30).unwrap();
    let ai = t.players[1].objects[1].ai.as_ref().unwrap();
    assert_eq!((ai.current_order, ai.current_target), (-1, -1));
    assert_eq!(ai.orders.len(), 1);
    assert_eq!(
        (ai.orders[0].order, ai.orders[0].target, ai.orders[0].ready),
        (crate::unit_ai::DEFEND_ORDER, 30, false)
    );
    assert_eq!(
        (
            ai.orders[0].issuer,
            ai.orders[0].target_owner,
            ai.orders[0].target_type
        ),
        (1, 2, 5)
    );
}

/// `FUN_14161c920` for one unit: it follows the target (a `MoveTo` to it), its unit AI takes
/// the follow order and action and the target with its class at its place.
#[test]
fn a_follower_takes_the_follow_order_and_walks_to_its_target() {
    let mut t = followers();
    t.follow(1, &[11], 30).unwrap();
    let unit = &t.players[1].objects[0];
    let ai = unit.ai.as_ref().unwrap();
    assert_eq!(
        (ai.current_order, ai.current_action),
        (crate::unit_ai::FOLLOW_ORDER, crate::unit_ai::FOLLOW_ACTION)
    );
    assert_eq!((ai.current_target, ai.target_type), (30, 6));
    assert_eq!(ai.target_point, [40.0, 20.0, 0.0]);
    assert_eq!(unit.move_to.unwrap().target, 30);
    assert_eq!(t.players[1].objects[1].move_to, None);
}

/// `FUN_14161c920` for two units side by side: each moves to the target at its offset from
/// the two's mean, takes the move order and action, and ignores the other.
#[test]
fn followers_side_by_side_take_their_offsets_from_the_target() {
    let mut t = followers();
    t.follow(1, &[11, 12], 30).unwrap();
    for (i, (other, x)) in [(12, 39.5), (11, 40.5)].into_iter().enumerate() {
        let unit = &t.players[1].objects[i];
        let ai = unit.ai.as_ref().unwrap();
        assert_eq!(
            (ai.current_order, ai.current_action),
            (MOVE_ORDER, crate::unit_ai::MOVE_ACTION)
        );
        assert_eq!((ai.current_target, ai.target_point), (30, [x, 20.0, 0.0]));
        assert_eq!(unit.mover.as_ref().unwrap().ignore_units, [other]);
        assert_eq!(unit.move_to.unwrap().target, 30);
    }
}

/// FOLLOW's check: another player's unit takes nothing while players may not order each
/// other's units.
#[test]
fn a_follow_of_another_player_s_unit_does_nothing() {
    let mut t = followers();
    t.follow(2, &[11], 30).unwrap();
    assert_eq!(
        t.players[1].objects[0].ai.as_ref().unwrap().current_order,
        -1
    );
}

/// `FUN_140aaddb0`: a Build on an annex counts its main building's builders one less; a
/// foundation the world no longer holds counts nothing.
#[test]
fn a_builder_leaves_the_main_building_of_an_annex() {
    let units = [(11, 4, [20.0, 20.0])];
    let mut t = soldiers(&units, &[]);
    t.masters[1] = crate::objects::masters::Masters::new([Master {
        id: 79,
        kind: BUILDING,
        ..Master::default()
    }]);
    let construction = crate::construction::build::Construction {
        progress: 0.0,
        desolid: true,
        builders: 2,
        hit_points: 100,
        build_time: 10,
        record: crate::map::spatial::Obstruction { kind: 2, class: 2 },
        snow: false,
        sprite_override: false,
        started: true,
    };
    let building = |id, annex_of| Unit {
        id,
        annex_of,
        construction: Some(construction),
        life: crate::objects::lifecycle::Lifecycle {
            master_id: 79,
            ..crate::objects::lifecycle::Lifecycle::default()
        },
        ..Unit::default()
    };
    t.players[1].objects = vec![building(60, None), building(70, Some(60))];
    t.leave_builders(70).unwrap();
    let builders = |t: &World, i: usize| t.players[1].objects[i].construction.unwrap().builders;
    assert_eq!((builders(&t, 0), builders(&t, 1)), (1, 2));
    t.leave_builders(60).unwrap();
    t.leave_builders(99).unwrap();
    assert_eq!(builders(&t, 0), 0);
}

/// `FUN_140c4da50`: the first queued order with a target that the unit itself issued; an
/// order of another issuer or without a target does not count.
#[test]
fn a_unit_s_own_queued_order_names_its_target() {
    let order = |issuer, target| crate::unit_ai::QueuedOrder {
        issuer,
        order: MOVE_ORDER,
        priority: 100,
        target,
        target_owner: -1,
        point: [1.0, 1.0, 0.0],
        range: 0.0,
        shown: [1.0, 1.0],
        ready: true,
        target_type: -1,
    };
    let mut ai = crate::unit_ai::UnitAi {
        orders: vec![order(1, 50), order(11, -1), order(11, 60), order(11, 70)],
        ..crate::unit_ai::UnitAi::default()
    };
    assert_eq!(own_order_target(&ai, 11), 60);
    ai.orders.truncate(2);
    assert_eq!(own_order_target(&ai, 11), -1);
}

/// `FUN_140a7a230`: a new target is taken (its workers count one more) and the old one let
/// go; a quiet sub-action does neither; a target in state 7 or more is not taken; the same
/// target changes nothing.
#[test]
fn an_action_s_new_target_is_taken_and_its_old_one_let_go() {
    let units = [(11, 4, [20.0, 20.0])];
    let mut t = soldiers(&units, &[]);
    t.masters[0] = crate::objects::masters::Masters::new([Master {
        id: 66,
        kind: 70,
        ..Master::default()
    }]);
    let deer = |id, state, workers| Unit {
        id,
        workers,
        combat: true,
        life: crate::objects::lifecycle::Lifecycle {
            master_id: 66,
            state,
            ..crate::objects::lifecycle::Lifecycle::default()
        },
        ..Unit::default()
    };
    t.players[0].objects = vec![deer(40, 2, 1), deer(41, 2, 0), deer(42, 7, 0)];
    let workers =
        |t: &World| -> Vec<i32> { t.players[0].objects.iter().map(|u| u.workers).collect() };
    assert_eq!(t.retarget_action(1, 0, 40, 40, false), Ok(40));
    assert_eq!(workers(&t), [1, 0, 0]);
    assert_eq!(t.retarget_action(1, 0, 40, 41, true), Ok(41));
    assert_eq!(workers(&t), [1, 0, 0]);
    assert_eq!(t.retarget_action(1, 0, 40, 41, false), Ok(41));
    assert_eq!(workers(&t), [0, 1, 0]);
    assert_eq!(t.retarget_action(1, 0, 41, 42, false), Ok(-1));
    assert_eq!(workers(&t), [0, 0, 0]);
    assert_eq!(t.retarget_action(1, 0, 99, -1, false), Ok(-1));
}

/// `FUN_140ab0300`: a Build counted among its foundation's builders leaves them for a new
/// target, which it takes; the same target keeps the count.
#[test]
fn a_build_s_new_target_leaves_the_old_foundation_s_builders() {
    let units = [(11, 4, [20.0, 20.0])];
    let mut t = soldiers(&units, &[]);
    t.masters[1] = crate::objects::masters::Masters::new([
        Master {
            id: 4,
            kind: 70,
            ..Master::default()
        },
        Master {
            id: 79,
            kind: BUILDING,
            ..Master::default()
        },
    ]);
    let foundation = |id, builders| Unit {
        id,
        construction: Some(crate::construction::build::Construction {
            progress: 0.0,
            desolid: true,
            builders,
            hit_points: 100,
            build_time: 10,
            record: crate::map::spatial::Obstruction { kind: 2, class: 2 },
            snow: false,
            sprite_override: false,
            started: true,
        }),
        workers: 1,
        life: crate::objects::lifecycle::Lifecycle {
            master_id: 79,
            ..crate::objects::lifecycle::Lifecycle::default()
        },
        ..Unit::default()
    };
    t.players[1].objects.push(foundation(60, 1));
    t.players[1].objects.push(foundation(61, 0));
    t.players[1].objects[0].build = Some(crate::construction::build::BuildAction {
        target: 60,
        state: 6,
        move_to: None,
        goal: None,
        walk_target: -1,
        point: [-1.0; 3],
        task: crate::construction::build::BuildTask {
            range: 0.1,
            work_value: 1.0,
            work_rate: 1.0,
            resource: -1,
            id: -1,
        },
        added: true,
        timer: 0.0,
        move_wait: crate::movement::move_to::MoveWait::default(),
        site: -1,
        drop_range: 0.0,
    });
    assert_eq!(t.build_set_target(1, 0, (60, true), 60), Ok(60));
    assert!(t.players[1].objects[0].build.unwrap().added);
    assert_eq!(t.build_set_target(1, 0, (60, true), 61), Ok(61));
    let build = t.players[1].objects[0].build.unwrap();
    assert_eq!((build.target, build.added), (61, false));
    let site = |t: &World, i: usize| {
        let u = &t.players[1].objects[i];
        (u.construction.unwrap().builders, u.workers)
    };
    assert_eq!((site(&t, 1), site(&t, 2)), ((0, 0), (0, 2)));
}

/// `FUN_141211340` and `FUN_14118f580` in a gate that shut: a soldier dies (no hit points),
/// class 42 and livestock with food are spared, livestock with a NaN load is not, and a
/// sleeping object (below type 70) wakes to the end of the live list and dies.
#[test]
fn a_shutting_gate_kills_the_units_in_it_but_the_spared() {
    let units = [(11, 4, [20.0, 20.0])];
    let mut t = soldiers(&units, &[]);
    let master = |id, kind, class| Master {
        id,
        kind,
        class,
        ..Master::default()
    };
    t.masters[1] = crate::objects::masters::Masters::new([
        master(4, 70, 6),
        master(42, 70, 42),
        master(58, 70, LIVESTOCK),
        master(10, 10, 5),
    ]);
    let unit = |id, master, amount| Unit {
        id,
        life: crate::objects::lifecycle::Lifecycle {
            master_id: master,
            hp: 10.0,
            amount,
            state: 2,
            ..crate::objects::lifecycle::Lifecycle::default()
        },
        ..Unit::default()
    };
    t.players[1].objects = vec![
        unit(11, 4, 0.0),
        unit(12, 42, 0.0),
        unit(13, 58, 50.0),
        unit(14, 58, f32::NAN),
    ];
    t.players[1].sleeping = vec![unit(15, 10, 0.0)];
    for id in 11..=16 {
        t.shut_in_gate(id).unwrap();
    }
    let hp: Vec<(i32, f32)> = t.players[1]
        .objects
        .iter()
        .map(|u| (u.id, u.life.hp))
        .collect();
    assert_eq!(
        hp,
        [(11, 0.0), (12, 10.0), (13, 10.0), (14, 0.0), (15, 0.0)]
    );
    assert_eq!(t.players[1].sleeping.len(), 0);
}

/// `FUN_14106c6a0`: a villager's own master without a task for its player's foundation
/// takes the first master of its class, type and swap group with one, searched with any
/// filter, and becomes it only with the change; one with its own task keeps its master.
#[test]
fn slot_0x820_takes_the_first_master_of_the_swap_group_with_a_task() {
    let units = [(11, 83, [20.0, 20.0])];
    let mut t = soldiers(&units, &[]);
    let task = |action_type, object_id| crate::objects::masters::Task {
        action_type,
        object_group: -1,
        object_id,
        work_val1: 1.0,
        ..crate::objects::masters::Task::default()
    };
    let villager = |id, tasks| Master {
        id,
        kind: 70,
        class: 4,
        task_by_group: 1,
        hit_points: 25,
        tasks,
        ..Master::default()
    };
    t.masters[1] = crate::objects::masters::Masters::new([
        villager(83, vec![task(5, 53)]),
        villager(118, vec![task(101, -1)]),
        Master {
            id: 70,
            kind: BUILDING,
            class: 3,
            hit_points: 550,
            ..Master::default()
        },
    ]);
    t.players[1].objects[0].life.hp = 25.0;
    t.players[1].objects.push(Unit {
        id: 60,
        life: crate::objects::lifecycle::Lifecycle {
            master_id: 70,
            state: 0,
            hp: 550.0,
            ..crate::objects::lifecycle::Lifecycle::default()
        },
        ..Unit::default()
    });
    let found = t.find_work_task(1, 0, 60, GARRISON, false).unwrap();
    assert_eq!(found.map(|t| t.action_type), Some(101));
    assert_eq!(t.players[1].objects[0].life.master_id, 83);
    let found = t.find_work_task(1, 0, 60, -1, true).unwrap();
    assert_eq!(found.map(|t| t.action_type), Some(101));
    assert_eq!(t.players[1].objects[0].life.master_id, 118);
    // 118's own task: no change
    assert!(t.find_work_task(1, 0, 60, -1, true).unwrap().is_some());
    assert_eq!(t.players[1].objects[0].life.master_id, 118);
}

/// `0x140ac60e0` case 6 and `FUN_140ac4c60`: the hunt's own sheep goes to the drop site, and
/// so does the player's idle livestock in state 2 on a tile within 3 that the player sees,
/// then the livestock around that; another player's or one too far stays.
#[test]
fn a_led_sheep_takes_the_idle_livestock_about_it_to_the_drop_site() {
    let units = [(11, 4, [30.0, 30.0])];
    let mut t = soldiers(&units, &[]);
    t.spatial = crate::map::spatial::Spatial::new([40, 40]);
    let sight = crate::vision::visibility::Visibility::new(
        40,
        40,
        vec![],
        vec![u64::MAX; 1600],
        vec![],
        0,
        vec![],
    );
    t.spatial.sight = Some(sight);
    let walker = crate::gathering::hunt::tests::walker();
    let sheep_master = Master {
        id: 594,
        kind: 70,
        class: LIVESTOCK,
        radius: [0.2, 0.2],
        moving: Some(walker.stats),
        ..Master::default()
    };
    t.masters[1] = crate::objects::masters::Masters::new([
        sheep_master.clone(),
        Master {
            id: 4,
            kind: 70,
            radius: [1.5, 1.5],
            ..Master::default()
        },
    ]);
    t.players.push(crate::world::Player::default());
    t.masters
        .push(crate::objects::masters::Masters::new([sheep_master]));
    let sheep = |id, x: f32, y: f32| Unit {
        id,
        position: [x, y, 0.0],
        mover: Some(walker.clone()),
        life: crate::objects::lifecycle::Lifecycle {
            master_id: 594,
            state: 2,
            ..crate::objects::lifecycle::Lifecycle::default()
        },
        ..Unit::default()
    };
    t.players[1].objects.extend([
        sheep(20, 10.5, 10.5),
        sheep(21, 12.5, 11.5),
        sheep(22, 20.5, 10.5),
    ]);
    t.players[2].objects.push(sheep(23, 11.5, 10.5));
    let placed = |player, master, at: [f32; 2], radius| crate::map::spatial::Placed {
        position: [at[0], at[1], 0.0],
        radius: [radius, radius],
        class: LIVESTOCK,
        player,
        master,
        obstruction_type: 0,
        buildable_on: 0,
        tall: false,
        speed: 0.8,
        unknown: None,
        obstruction: None,
    };
    t.spatial.insert(11, placed(1, 4, [30.0, 30.0], 1.5));
    for (id, p, at) in [
        (20, 1, [10.5, 10.5]),
        (21, 1, [12.5, 11.5]),
        (22, 1, [20.5, 10.5]),
        (23, 2, [11.5, 10.5]),
    ] {
        t.spatial.insert(id, placed(p, 594, at, 0.2));
    }
    t.lead_herd(1, 20, 11).unwrap();
    let goes = |p: usize, id: i32| {
        t.players[p]
            .objects
            .iter()
            .find(|u| u.id == id)
            .and_then(|u| u.move_to)
            .map(|m| m.target)
    };
    assert_eq!(goes(1, 20), Some(11));
    assert_eq!(goes(1, 21), Some(11));
    assert_eq!(goes(1, 22), None);
    assert_eq!(goes(2, 23), None);
}

/// `FUN_1410395a0`: the old list goes, the new one is copied whole, an empty one too.
#[test]
fn a_group_s_differentiator_groups_are_the_list_given() {
    let mut g = crate::groups::UnitGroup {
        differentiator_groups: vec![1, 2, 3],
        ..crate::groups::UnitGroup::default()
    };
    set_differentiator_groups(&mut g, &[7, 8]);
    assert_eq!(g.differentiator_groups, [7, 8]);
    set_differentiator_groups(&mut g, &[]);
    assert_eq!(g.differentiator_groups, Vec::<u32>::new());
}

/// `FUN_14118c790`: each id joins the end of the ignore list, a repeated one again.
#[test]
fn an_ignored_unit_joins_the_end_of_the_ignore_list() {
    let mut ignore = vec![7];
    for id in [9, 8, 9] {
        ignore_unit(&mut ignore, id);
    }
    assert_eq!(ignore, [7, 9, 8, 9]);
}

/// `FUN_1410862c0`: too close below the least range (COMISS, SETA: NaN not), in reach at
/// most the weapon range (COMISS, JC: NaN out) and not too close.
#[test]
fn a_target_is_in_reach_within_the_weapon_range_and_not_below_the_least() {
    assert_eq!(work_reach(3.0, 5.0, 2.0), (true, false));
    assert_eq!(work_reach(5.0, 5.0, 2.0), (true, false));
    assert_eq!(work_reach(5.5, 5.0, 2.0), (false, false));
    assert_eq!(work_reach(1.0, 5.0, 2.0), (false, true));
    assert_eq!(work_reach(2.0, 5.0, 2.0), (true, false));
    assert_eq!(work_reach(f32::NAN, 5.0, 2.0), (false, false));
    assert_eq!(work_reach(0.0, 0.0, 0.0), (true, false));
}

/// Player 1's soldier 10 (master 4, ranges 2 to 5) at (10, 10) and player 2's unit 20 at
/// (`x`, 10), the two enemies, each with a unit AI in state 2.
fn soldier_and_enemy(x: f32, class: i16) -> World {
    let unit = |id, master, x: f32| Unit {
        id,
        position: [x, 10.0, 0.0],
        ai: Some(crate::unit_ai::UnitAi::default()),
        life: crate::objects::lifecycle::Lifecycle {
            master_id: master,
            state: 2,
            ..crate::objects::lifecycle::Lifecycle::default()
        },
        ..Unit::default()
    };
    let player = |objects, relations| crate::world::Player {
        objects,
        relations,
        ..crate::world::Player::default()
    };
    let mut t = World::new(
        0,
        1,
        0,
        vec![0, 0, 0],
        vec![
            player(vec![], vec![0, 3, 3]),
            player(vec![unit(10, 4, 10.0)], vec![3, 0, 3]),
            player(vec![unit(20, 5, x)], vec![3, 3, 0]),
        ],
    );
    let master = |id, class| Master {
        id,
        kind: 70,
        class,
        weapon_range: 5.0,
        min_range: 2.0,
        radius: [0.2, 0.2],
        ..Master::default()
    };
    t.masters = vec![
        crate::objects::masters::Masters::new([]),
        crate::objects::masters::Masters::new([master(4, class)]),
        crate::objects::masters::Masters::new([master(5, 6)]),
    ];
    for (id, player, x) in [(10, 1, 10.0), (20, 2, x)] {
        t.spatial.insert(
            id,
            crate::map::spatial::Placed {
                position: [x, 10.0, 0.0],
                radius: [0.2, 0.2],
                class,
                player,
                master: 4,
                obstruction_type: 0,
                buildable_on: 0,
                tall: false,
                speed: 0.0,
                unknown: None,
                obstruction: None,
            },
        );
    }
    t
}

/// `FUN_141227190` on an enemy nearer than the least range: the unit posts notify 0x1fe
/// with the target to its own unit AI (slot 0x328) and takes no work.
#[test]
fn an_order_on_an_enemy_below_the_least_range_notifies_the_unit_ai() {
    let mut t = soldier_and_enemy(11.0, 6);
    let work = Work {
        target: 20,
        point: [11.0, 10.0, 0.0],
        carry_test: false,
        filter: -1,
        notified: true,
    };
    t.order_work(1, 0, &work).unwrap();
    let unit = &t.players[1].objects[0];
    let ai = unit.ai.as_ref().unwrap();
    assert_eq!(
        ai.notifies,
        [crate::unit_ai::Notify::own(10, 0x1fe, [20, -1, -1])]
    );
    assert_eq!(unit.top_action_kind(), None);
}

/// `FUN_140fd9ae0` with 61 or more: the sorted list (a gone unit null, sorted first) splits
/// in runs of 60, each split on its own (`FUN_140fd76d0`) into the one split: its groups count
/// on, and an id repeated across two runs is in each.
#[test]
fn the_splitter_splits_an_order_to_more_than_60_units_in_runs_of_60() {
    let units: Vec<_> = (100..161).map(|id| (id, 4, [20.0, 20.0])).collect();
    let mut t = soldiers(&units, &[]);
    let ids: Vec<i32> = (100..161).rev().collect();
    let split = t.split(&ids, &|_| true, false).unwrap();
    assert_eq!(split.groups, [(100..160).collect::<Vec<_>>()]);
    assert_eq!(split.loose, [160]);
    // the gone 999 is the first run's first entry
    let gone: Vec<i32> = ids.iter().copied().chain([999]).collect();
    let split = t.split(&gone, &|_| true, false).unwrap();
    assert_eq!(
        split.groups,
        [(100..159).collect::<Vec<_>>(), vec![159, 160]]
    );
    assert_eq!(split.loose, Vec::<i32>::new());
    // 159 twice: the first run's last entry has no next one to repeat it
    let twice: Vec<i32> = (100..160).chain([159]).collect();
    let split = t.split(&twice, &|_| true, false).unwrap();
    assert_eq!(split.groups, [(100..160).collect::<Vec<_>>()]);
    assert_eq!(split.loose, [159]);
    // with Ctrl held the first run fills the 60 groups, the second's group is dropped
    let split = t.split(&ids, &|_| true, true).unwrap();
    assert_eq!(split.groups.len(), 60);
    assert_eq!(split.groups[59], [159]);
    assert_eq!(split.loose, Vec::<i32>::new());
}

/// MarvinBlue/426048731's MOVE of 1569: player 2's villagers 9175 and 9177 (by id) take a
/// grid of one cell a row, half a tile apart, from the order's point (their `MoveTos`' points in
/// the save after 1568).
#[test]
fn units_of_a_move_take_the_cells_of_a_grid() {
    let point = [29.78125, 92.489_586];
    assert_eq!(grid_point(0, 2, point, [120, 120]), [29.78125, 92.489_586]);
    assert_eq!(grid_point(1, 2, point, [120, 120]), [29.78125, 92.989_586]);
    // nine units: three a row around the point; off the map an axis keeps a tile inside
    assert_eq!(grid_point(0, 9, [10.0, 10.0], [120, 120]), [9.5, 9.5]);
    assert_eq!(grid_point(4, 9, [10.0, 10.0], [120, 120]), [10.0, 10.0]);
    assert_eq!(grid_point(8, 9, [119.75, 0.25], [120, 120]), [119.0, 0.75]);
    assert_eq!(grid_point(0, 9, [0.25, 0.25], [120, 120]), [1.0, 1.0]);
}

/// MarvinBlue/426048731's INTERACT of 927: player 1's builders 9171 and 9173 stand side by
/// side; the queued orders of the save after 927 hold their points around the order's.
#[test]
fn units_of_an_order_take_their_offsets_from_their_centroid() {
    let (a, b) = ([104.32466, 39.299_885], [103.67509, 39.299_885]);
    let c = centroid(&[a, b]).unwrap();
    let point = [108.37596, 43.91338];
    assert_eq!(
        offset_point(a, c, point, [120, 120]),
        [108.700_745, 43.91338]
    );
    assert_eq!(offset_point(b, c, point, [120, 120]), [108.05117, 43.91338]);
}

/// A unit left of the first one counted moves only the least x, so the greatest stays 0 and
/// the box passes however far apart the units stand; a unit 9 tiles right of the first does
/// not.
#[test]
fn the_box_of_an_order_moves_only_its_least_value_for_a_lower_one() {
    assert!(centroid(&[[50.0, 50.0], [30.0, 50.0]]).is_some());
    assert_eq!(centroid(&[[30.0, 50.0], [39.0, 50.0]]), None);
    assert_eq!(centroid(&[[30.0, 50.0], [38.0, 50.0]]), Some([34.0, 50.0]));
}

/// MarvinBlue/426048731's MOVE of 2979 (the save after 2977): player 2's livestock 9528 and
/// 9529 (formation category 0) stay loose, the scout 9185 has none to join it.
#[test]
fn the_splitter_joins_units_of_one_kind_within_the_influence_distance() {
    let f = crate::world::Formations {
        auto: true,
        influence: 3.0,
        by_speed: false,
        ..crate::world::Formations::default()
    };
    let unit = |id, grouped, position: [f32; 3]| Splittable {
        id,
        group: -1,
        kind: 70,
        grouped,
        flag_1: false,
        speed: 1.2,
        position,
        moved_at: [position[0], position[1]],
        state: 2,
        radius: 0.2,
        restriction: Some(1),
    };
    // no planner: the distance decides
    let mut none = |_: &Splittable, _: &Splittable| Ok(None);
    let mut away = std::collections::BTreeSet::new();
    let scout = unit(9185, true, [42.047_367, 94.338_486, 0.0]);
    let sheep = unit(9529, false, [45.5, 93.5, 0.0]);
    assert!(!splits_together(&scout, &sheep, f, false, &mut none, &mut away).unwrap());
    // nearer than 20 tiles the planner's path decides: its goal reached (1) within the
    // influence distance
    let other = unit(9186, true, [44.0, 94.0, 0.0]);
    let mut short = |_: &Splittable, _: &Splittable| Ok(Some((1, 2.5)));
    assert!(splits_together(&scout, &other, f, false, &mut short, &mut away).unwrap());
    let mut long = |_: &Splittable, _: &Splittable| Ok(Some((1, 3.5)));
    assert!(!splits_together(&scout, &other, f, false, &mut long, &mut away).unwrap());
    let mut nearest = |_: &Splittable, _: &Splittable| Ok(Some((2, 2.5)));
    assert!(!splits_together(&scout, &other, f, false, &mut nearest, &mut away).unwrap());
    // a path that did not join puts the unit among the far ones; a NaN length joins
    assert_eq!(away, [9186].into());
    let mut lost = |_: &Splittable, _: &Splittable| Ok(Some((1, f32::NAN)));
    assert!(splits_together(&scout, &other, f, false, &mut lost, &mut away).unwrap());
    // a NaN distance takes no path and is within the influence distance
    let nowhere = Splittable {
        moved_at: [f32::NAN, 94.0],
        ..other
    };
    assert!(splits_together(&scout, &nowhere, f, false, &mut short, &mut away).unwrap());
    let far = unit(9187, true, [80.0, 94.0, 0.0]);
    assert!(!splits_together(&scout, &far, f, false, &mut none, &mut away).unwrap());
    let wide = crate::world::Formations {
        influence: 40.0,
        ..f
    };
    assert!(splits_together(&scout, &far, wide, false, &mut none, &mut away).unwrap());
    // a member must stand in state 2 or more
    let born = Splittable { state: 0, ..far };
    assert!(!splits_together(&scout, &born, wide, true, &mut none, &mut away).unwrap());
}

#[test]
fn a_point_off_the_map_stays_inside_its_edge() {
    let c = [10.0, 10.0];
    assert_eq!(
        offset_point([5.0, 15.0], c, [2.0, 118.0], [120, 120]),
        [1.0, 119.0]
    );
}

/// `FUN_14161b970`'s timers: 750 ms for the first six entries of the list (the gone ones
/// count), the world's dt more for each six after.
#[test]
fn a_patrol_staggers_its_units_timeouts_by_sixes() {
    assert_eq!(patrol_timeout(0, 100), 750);
    assert_eq!(patrol_timeout(5, 100), 750);
    assert_eq!(patrol_timeout(6, 100), 850);
    assert_eq!(patrol_timeout(13, 33), 816);
    assert_eq!(patrol_timeout(499, 0), 750);
}

/// `FUN_14161b970`'s filter: a waypoint less than a tile from the last one kept on both x
/// and y is passed over; a NaN difference does not count (`COMISS`/`JNC`).
#[test]
fn a_patrol_passes_over_a_waypoint_within_a_tile_of_the_last() {
    let w = [
        [10.5, 10.5],
        [11.0, 10.0],
        [11.5, 10.9],
        [11.5, 9.0],
        [f32::NAN, 9.0],
        [12.5, 9.0],
    ];
    assert_eq!(
        waypoints_from([10.0, 10.0], &w),
        [[11.0, 10.0], [11.5, 9.0], [12.5, 9.0]]
    );
    assert_eq!(
        waypoints_from([10.0, 10.0], &w[..1]),
        Vec::<[f32; 2]>::new()
    );
}

/// Player 1's soldiers of `soldiers`, each walking and with a unit AI off any order, at a
/// world dt of 100 ms.
fn patrollers(units: &[(i32, i16, [f32; 2])], members: &[i32]) -> World {
    let mut t = soldiers(units, members);
    t.dt = 100;
    for u in &mut t.players[1].objects {
        u.mover = Some(crate::gathering::hunt::tests::walker());
        u.ai = Some(crate::unit_ai::UnitAi {
            ai_type: 13,
            current_order: -1,
            current_action: -1,
            current_target: -1,
            target_type: -1,
            idle_timer: 400,
            adjusted_idle_timeout: 2000,
            secondary_timer: 3000,
            patrol_path: Some(crate::unit_ai::PatrolPath::default()),
            patrol_waypoint: 2,
            ..crate::unit_ai::UnitAi::default()
        });
    }
    t
}

/// MarvinBlue/426049701's first PATROL (update 25): player 1's scout alone, one waypoint,
/// `+0x8` set. It moves loose (slot 0x340: a forced, walking `MoveTo` to the waypoint at its
/// height), waits to move with an empty ignore list and no waypoints of its own; its unit AI
/// takes the patrol order, the move action and no target at the waypoint, its timers 0 and
/// 750, its saved patrol gone.
#[test]
fn a_lone_unit_patrols_to_its_waypoint() {
    let mut t = patrollers(&[(11, 4, [20.0, 20.0])], &[]);
    t.players[1].objects[0].position[2] = 1.5;
    t.patrol(1, &[11], &[[30.0, 20.0]], true).unwrap();
    let u = &t.players[1].objects[0];
    let move_to = u.move_to.unwrap();
    assert_eq!(
        (move_to.goal, move_to.target, move_to.range, move_to.run),
        (Some([30.0, 20.0, 1.5]), -1, 0.0, false)
    );
    let m = u.mover.as_ref().unwrap();
    assert_eq!(
        (m.waiting, m.ignore_units.len(), m.user_waypoints.len()),
        (1, 0, 0)
    );
    let ai = u.ai.as_ref().unwrap();
    assert_eq!(
        (ai.current_order, ai.last_order, ai.current_action),
        (crate::unit_ai::PATROL, -1, crate::unit_ai::MOVE_ACTION)
    );
    assert_eq!(
        (ai.current_target, ai.target_type, ai.target_point),
        (-1, -1, [30.0, 20.0, 1.5])
    );
    assert_eq!(
        (ai.idle_timer, ai.adjusted_idle_timeout, ai.secondary_timer),
        (0, 750, 750)
    );
    assert_eq!((ai.patrol_path.is_none(), ai.patrol_waypoint), (true, 0));
    assert_eq!(t.groups.commands, 1);
}

/// A lone unit's waypoints but the last become its own (slot 0x448, a whole coordinate moved
/// to the tile's middle); the move goes to the last one kept, the unit AI's point is the
/// command's last even where the filter left it out. Without `+0x8` the saved patrol stays.
#[test]
fn a_lone_unit_s_waypoints_but_the_last_become_its_own() {
    let mut t = patrollers(&[(11, 4, [20.0, 20.0])], &[]);
    let w = [[25.0, 20.0], [25.5, 20.5], [30.0, 20.0], [30.5, 20.5]];
    t.patrol(1, &[11], &w, false).unwrap();
    let u = &t.players[1].objects[0];
    assert_eq!(u.move_to.unwrap().goal, Some([30.0, 20.0, 0.0]));
    assert_eq!(
        u.mover.as_ref().unwrap().user_waypoints,
        [[25.5, 0.0, 20.5]]
    );
    let ai = u.ai.as_ref().unwrap();
    assert_eq!(ai.target_point, [30.5, 20.5, 0.0]);
    assert!(ai.patrol_path.is_some());
}

/// `FUN_14106d340`'s `FUN_140c5dc10` with a completion of the unit AI's current action
/// (no order) pending: the action stops (slot 0xb0 with 1: its value the last, none now) and
/// the unit's stopAction runs at once, before the move.
#[test]
fn a_move_runs_the_stop_of_a_pending_completion_first() {
    let mut t = patrollers(&[(11, 4, [20.0, 20.0])], &[]);
    let ai = t.players[1].objects[0].ai.as_mut().unwrap();
    ai.current_action = crate::unit_ai::MOVE_ACTION;
    ai.notifies = vec![crate::unit_ai::Notify::own(
        11,
        crate::unit_ai::ACTION_COMPLETED,
        [crate::unit_ai::MOVE_ACTION, 0, 0],
    )];
    assert_eq!(t.move_to_point(1, 0, [30.0, 20.0, 0.0]), Ok(false));
    let u = &t.players[1].objects[0];
    let ai = u.ai.as_ref().unwrap();
    assert_eq!(
        (ai.current_action, ai.last_action, ai.notifies.len()),
        (-1, crate::unit_ai::MOVE_ACTION, 0)
    );
    assert_eq!(u.move_to.unwrap().goal, Some([30.0, 20.0, 0.0]));
}

/// The attack move of 428506340's update 940 (`+0x8` and `+0xa` set): player 1's lone unit
/// first resets its AI as for an order (`FUN_141631040`: its saved patrol goes even without
/// `+0x8`), then moves loose as a patrol's unit but unforced (a `MoveTo` already to the point
/// stands), its unit AI on the attack move order (`0x2dd`), the move action and no target at
/// the waypoint.
#[test]
fn a_lone_unit_attack_moves_to_its_waypoint() {
    let sweep = Sweep {
        patrol: false,
        reset: false,
        queued: false,
        ignore_pending: true,
    };
    let mut t = patrollers(&[(11, 4, [20.0, 20.0])], &[]);
    // the reset redraws the adjusted idle timeout from it
    t.players[1].objects[0].ai.as_mut().unwrap().idle_timeout = 3000;
    t.attack_move(1, &[11], &[[30.0, 20.0]], sweep).unwrap();
    let u = &t.players[1].objects[0];
    assert_eq!(u.move_to.unwrap().goal, Some([30.0, 20.0, 0.0]));
    let ai = u.ai.as_ref().unwrap();
    assert_eq!(
        (ai.current_order, ai.current_action, ai.target_point),
        (
            crate::unit_ai::ATTACK_MOVE,
            crate::unit_ai::MOVE_ACTION,
            [30.0, 20.0, 0.0]
        )
    );
    assert_eq!(
        (ai.idle_timer, ai.adjusted_idle_timeout, ai.secondary_timer),
        (0, 750, 750)
    );
    assert!(ai.patrol_path.is_none());
    // unforced: the MoveTo to the same point stands
    t.players[1].objects[0].move_to.as_mut().unwrap().range = 0.25;
    t.attack_move(1, &[11], &[[30.0, 20.0]], sweep).unwrap();
    assert_eq!(t.players[1].objects[0].move_to.unwrap().range, 0.25);
    // a patrol's forced move makes a new one
    t.patrol(1, &[11], &[[30.0, 20.0]], false).unwrap();
    assert_eq!(t.players[1].objects[0].move_to.unwrap().range, 0.0);
}

/// An attack move through the unit AI's queue (`+0x9`, or without `+0xa` an order in the
/// queue not ready) leaves the record's shown point and ready flag to the stack, which is
/// not modelled; with `+0xa` an unready order does not send it there.
#[test]
fn an_attack_move_through_the_unit_ai_s_queue_is_not_modelled() {
    let sweep = |queued, ignore_pending| Sweep {
        patrol: false,
        reset: true,
        queued,
        ignore_pending,
    };
    let patrollers = |units: &[(i32, i16, [f32; 2])]| {
        let mut t = patrollers(units, &[]);
        t.players[1].objects[0].ai.as_mut().unwrap().idle_timeout = 3000;
        t
    };
    let mut t = patrollers(&[(11, 4, [20.0, 20.0])]);
    assert_eq!(
        t.attack_move(1, &[11], &[[30.0, 20.0]], sweep(true, true)),
        Err(ATTACK_MOVE_QUEUED)
    );
    let unready = crate::unit_ai::QueuedOrder {
        issuer: 1,
        order: MOVE_ORDER,
        priority: 100,
        target: -1,
        target_owner: -1,
        point: [25.0, 20.0, 0.0],
        range: 0.0,
        shown: [0.0; 2],
        ready: false,
        target_type: -1,
    };
    let mut t = patrollers(&[(11, 4, [20.0, 20.0])]);
    t.players[1].objects[0].ai.as_mut().unwrap().orders = vec![unready];
    assert_eq!(
        t.attack_move(1, &[11], &[[30.0, 20.0]], sweep(false, false)),
        Err(ATTACK_MOVE_QUEUED)
    );
    let mut t = patrollers(&[(11, 4, [20.0, 20.0])]);
    t.players[1].objects[0].ai.as_mut().unwrap().orders = vec![unready];
    assert_eq!(
        t.attack_move(1, &[11], &[[30.0, 20.0]], sweep(false, true)),
        Ok(())
    );
}

/// Loose units of a PATROL (villagers, category 0) list each other in their ignore lists;
/// one whose waypoints all lie within a tile of it stays where it is, its order unchanged.
#[test]
fn loose_patrollers_ignore_each_other_and_one_at_its_waypoint_stays() {
    let mut t = patrollers(&[(11, 83, [20.0, 20.0]), (12, 83, [29.5, 20.0])], &[]);
    t.patrol(1, &[11, 12], &[[30.0, 20.0]], true).unwrap();
    let [a, b] = &t.players[1].objects[..] else {
        panic!("two units")
    };
    assert_eq!(a.mover.as_ref().unwrap().ignore_units, [12]);
    assert_eq!(b.mover.as_ref().unwrap().ignore_units, [11]);
    assert!(a.move_to.is_some() && b.move_to.is_none());
    assert_eq!(b.ai.as_ref().unwrap().current_order, -1);
    assert_eq!(b.mover.as_ref().unwrap().waiting, 1);
}

/// `FUN_140fd9f40`: the splitter keeps group 5 (the order's 11 and 12 are all of it); the
/// loose 13 of type 70 with a formation category within the owner's influence of the
/// group's place joins it (flags bit 17), the villager does not, and a unit among the far
/// ones does not either.
#[test]
fn a_kept_group_takes_the_loose_soldiers_about_it() {
    let units = [
        (11, 4, [20.0, 20.0]),
        (12, 4, [21.0, 20.0]),
        (13, 4, [22.0, 20.0]),
        (14, 83, [21.0, 21.0]),
        (15, 4, [21.0, 19.0]),
    ];
    let mut t = soldiers(&units, &[11, 12]);
    for u in &mut t.players[1].objects {
        let mut m = crate::gathering::hunt::tests::walker();
        m.moved_at = u.position;
        u.mover = Some(m);
    }
    t.groups.get_mut(5).unwrap().motion.position[0] = [21.0, 0.0, 20.0];
    let mut split = Split {
        owner: Some(1),
        loose: vec![13, 14, 15],
        groups: vec![vec![11, 12]],
        far: [15].into(),
    };
    t.keep_group(&mut split, 5).unwrap();
    assert_eq!(split.loose, [14, 15]);
    let g = t.groups.get(5).unwrap();
    assert!(g.has(13));
    assert_eq!(g.flags & 0x2_0000, 0x2_0000);
    // the unit after one that joins is passed over: 13 joins, 15 slides into its place
    let mut t = soldiers(&units, &[11, 12]);
    t.groups.get_mut(5).unwrap().motion.position[0] = [21.0, 0.0, 20.0];
    let mut split = Split {
        owner: Some(1),
        loose: vec![13, 15],
        ..Split::default()
    };
    t.keep_group(&mut split, 5).unwrap();
    assert_eq!(split.loose, [15]);
}

/// [`patrollers`]' two soldiers near each other in a world whose statistics search works.
fn two_patrollers() -> World {
    searchable(patrollers(
        &[(11, 4, [20.0, 20.0]), (12, 4, [21.0, 20.0])],
        &[],
    ))
}

/// `t` on a 60 by 60 map of open land, its sight's registry for the searches.
fn searchable(mut t: World) -> World {
    // the group's statistics search the members' surroundings
    let mut s = crate::map::spatial::Spatial::with_tiles(
        [60, 60],
        crate::map::spatial::Tiles::new(60, 60, vec![1; 3600]),
    );
    let kind = crate::map::terrain::TerrainKind {
        shown: true,
        flags: 1,
        forest: false,
    };
    s.terrain = Some(crate::map::terrain::Terrain::new(
        60,
        60,
        vec![(0, 255, -1); 3600],
        vec![kind],
        vec![vec![1.0]],
    ));
    s.relations = vec![vec![0, 3], vec![0, 0]];
    s.unit_diplomacy = vec![vec![1, 4], vec![0, 1]];
    let mut v =
        crate::vision::visibility::Visibility::new(60, 60, vec![], vec![], vec![], 0, vec![]);
    v.registry = Some(crate::vision::registry::Registry::new(2, 65));
    s.sight = Some(v);
    t.spatial = s;
    t
}

/// An attack move of two soldiers near each other: their group's move to the last waypoint
/// holds (`commandMoveTo` with `param_6` 0, `param_8` 1): its path does not loop (bit 0)
/// and its flags bit 11 is set, where a patrol's loops and leaves it clear.
#[test]
fn an_attack_moving_group_holds_its_path_to_the_last_waypoint() {
    let sweep = Sweep {
        patrol: false,
        reset: true,
        queued: false,
        ignore_pending: true,
    };
    let mut t = two_patrollers();
    for u in &mut t.players[1].objects {
        u.ai.as_mut().unwrap().idle_timeout = 3000;
    }
    t.attack_move(1, &[11, 12], &[[25.0, 25.0], [30.0, 20.0]], sweep)
        .unwrap();
    let g = &t.groups.groups[0];
    let top = g.motion.paths.last().unwrap();
    assert_eq!((top.path_flags & 1, g.flags & 0x800), (0, 0x800));
    let mut t = two_patrollers();
    t.patrol(1, &[11, 12], &[[25.0, 25.0], [30.0, 20.0]], true)
        .unwrap();
    let g = &t.groups.groups[0];
    let top = g.motion.paths.last().unwrap();
    assert_eq!((top.path_flags & 1, g.flags & 0x800), (1, 0));
}

/// A PATROL of two soldiers near each other: the splitter's group becomes a new group of
/// player 1, listed (its differentiators the command count before and itself), with the
/// waypoints but the last as its own and a looping path to the last, back and forth from 5
/// tiles; the command count goes on.
#[test]
fn a_patrolling_group_walks_a_loop_through_its_waypoints() {
    let mut t = two_patrollers();
    let w = [[25.0, 25.0], [30.0, 20.0]];
    t.patrol(1, &[11, 12], &w, true).unwrap();
    assert_eq!(t.groups.groups.len(), 1);
    let g = &t.groups.groups[0];
    assert_eq!((g.owner, g.has(11), g.has(12)), (1, true, true));
    assert_eq!((g.differentiator_id, g.current_command), (0, 1));
    let top = g.motion.paths.last().unwrap();
    assert_eq!(top.path_flags & 3, 3);
    assert_eq!(top.points.len(), 3);
    assert_eq!(top.points[1..], [[25.0, 0.0, 25.0], [30.0, 0.0, 20.0]]);
    assert_eq!(t.groups.commands, 1);
    let ai = t.players[1].objects[0].ai.as_ref().unwrap();
    assert_eq!(
        (ai.adjusted_idle_timeout, ai.patrol_path.is_none()),
        (750, true)
    );
}

/// MOVE (`FUN_140e67800`) of three soldiers in no group: the splitter groups them (each has
/// two joiners: the first by id leads), and a new group of their owner (`FUN_14152efc0`)
/// takes them, its statistics, its place and the command; with Ctrl held each is a group of
/// its own (its formation the line's lock) and the three list each other as their
/// differentiator groups.
#[test]
fn a_move_of_soldiers_in_no_group_forms_new_groups() {
    let units = [
        (11, 4, [20.0, 20.0]),
        (12, 4, [21.0, 20.0]),
        (13, 4, [22.0, 20.0]),
    ];
    let order = MoveOrder {
        target: -1,
        point: [40.0, 40.0],
        queued: false,
        direct: false,
        human: false,
        control: false,
    };
    let world = || searchable(patrollers(&units, &[]));
    let mut t = world();
    t.move_order(1, &[11, 12, 13], &order).unwrap();
    let g = &t.groups.groups;
    assert_eq!(g.len(), 1);
    let ids: Vec<i32> = g[0].members.iter().map(|m| m.id).collect();
    assert_eq!(ids, [11, 12, 13]);
    assert_eq!(t.groups.commands, 1);
    let mut t = world();
    let ctrl = MoveOrder {
        control: true,
        ..order
    };
    t.move_order(1, &[11, 12, 13], &ctrl).unwrap();
    let g = &t.groups.groups;
    assert_eq!(g.len(), 3);
    for g in g {
        assert_eq!(g.members.len(), 1);
        assert_eq!(g.differentiator_groups, [0, 1, 2]);
    }
}

/// MOVE (`FUN_140e67800`) of the whole of group 5 and a loose soldier beside it: the
/// splitter keeps the group (`FUN_140fd9f40`), which takes the soldier and the command
/// (`BUnitGroup::command`); no new group is made. With a target the world holds the kept group's range test is not
/// modelled.
#[test]
fn a_move_of_a_whole_group_keeps_it() {
    let units = [
        (11, 4, [20.0, 20.0]),
        (12, 4, [21.0, 20.0]),
        (13, 4, [22.0, 20.0]),
    ];
    let order = MoveOrder {
        target: -1,
        point: [40.0, 40.0],
        queued: false,
        direct: false,
        human: false,
        control: false,
    };
    let kept = || {
        let mut t = soldiers(&units, &[11, 12]);
        t.groups.get_mut(5).unwrap().motion.position[0] = [21.0, 0.0, 20.0];
        t
    };
    let mut t = kept();
    t.move_order(1, &[11, 12, 13], &order).unwrap();
    assert_eq!(t.groups.groups.len(), 1);
    let g = t.groups.get(5).unwrap();
    assert!(g.has(13));
    // the keep sets flags bit 17, which the group's command then clears
    assert_eq!((g.flags & 0x2_0000, g.differentiator_id), (0, 0));
    assert_eq!(t.groups.commands, 1);
    let mut t = kept();
    let at_12 = MoveOrder {
        target: 12,
        ..order
    };
    assert!(t.move_order(1, &[11, 12, 13], &at_12).is_err());
}

/// The Gather's slot 0x48 on a building (`0x140a903a0`, [`World::retarget_gather`]): its
/// player's farm in state 2 (class 49) it takes, walking there; its player's farm foundation
/// (the task finder's build), a building at work (packing) and an annex whose main building is
/// gone are not modelled; another building it does not take (false), so the work command makes
/// a new action.
#[test]
fn a_gather_takes_a_farm_but_no_other_building() {
    let mut t = crate::world::tests::forage_world();
    t.gather_command(1, 0, 600).unwrap();
    t.masters[1] =
        crate::objects::masters::Masters::new(t.masters[1].iter().cloned().chain([Master {
            id: 50,
            kind: BUILDING,
            class: crate::gathering::gather::FARM_CLASS,
            radius: [1.5, 1.5],
            ..Master::default()
        }]));
    let farm = |id, state, annex_of| Unit {
        id,
        position: [26.5, 89.5, 0.0],
        annex_of,
        life: crate::objects::lifecycle::Lifecycle {
            state,
            master_id: 50,
            ..crate::objects::lifecycle::Lifecycle::default()
        },
        pasture: -1,
        my_pasture: -1,
        ..Unit::default()
    };
    t.players[1].objects.push(farm(800, 2, None));
    t.players[1].objects.push(farm(801, 0, None));
    t.players[1].objects.push(farm(802, 2, Some(4444)));
    let world = t.clone();
    assert_eq!(t.retarget_gather(1, 0, 800), Ok(true));
    let g = t.players[1].objects[0].gather.unwrap();
    assert_eq!((g.target, g.state, g.point), (800, 4, [26.5, 89.5, 0.0]));
    assert_eq!(
        (
            t.players[0].objects[0].workers,
            t.players[1].objects[3].workers
        ),
        (0, 1)
    );
    let mut t = world.clone();
    assert!(t.retarget_gather(1, 0, 801).is_err());
    assert!(t.retarget_gather(1, 0, 802).is_err());
    t.players[1].objects[3].unmodelled_action = Some(GenieError::unsupported("a pack"));
    assert!(t.retarget_gather(1, 0, 800).is_err());
    let mut t = world;
    assert_eq!(t.retarget_gather(1, 0, 700), Ok(false));
    assert!(t.retarget_gather(1, 0, 4444).is_err());
}

/// [`crate::world::tests::forage_world`]'s forager 9175 carrying 6 berries (kind 17), its
/// Gather on bush 600 from a work command, its player holding 10 attributes.
fn foraging() -> World {
    let mut t = crate::world::tests::forage_world();
    let work = Work {
        target: 600,
        point: [32.5, 89.5, 0.0],
        carry_test: false,
        filter: -1,
        notified: true,
    };
    t.order_work(1, 0, &work).unwrap();
    t.players[1].attributes = vec![0.0; 10];
    let v = &mut t.players[1].objects[0];
    (v.life.attribute_type, v.life.amount) = (17, 6.0);
    t
}

/// The drop-off order's ask (`FUN_14161e370`): the orders 7 and 8 ask a top Gather, Build or
/// Hunt, whose task's resource (berries, kind 0) asks for a load of its kind and not for wood
/// (kind 1); the order 8 to an action without a target or a point is unaimed; the order 6
/// and a unit without such an action ask nothing.
#[test]
fn a_drop_off_order_asks_the_top_gather_build_or_hunt() {
    use super::unit_order::{DropOffAsk, DropOffBy};
    let mut t = foraging();
    let ask = |by, asked, unaimed| Some(DropOffAsk { by, asked, unaimed });
    assert_eq!(
        t.drop_off_ask(1, 0, (true, true)),
        ask(DropOffBy::Gather, true, false)
    );
    assert_eq!(t.drop_off_ask(1, 0, (false, true)), None);
    t.players[1].objects[0].life.attribute_type = 1;
    assert_eq!(
        t.drop_off_ask(1, 0, (true, false)),
        ask(DropOffBy::Gather, false, false)
    );
    let g = t.players[1].objects[0].gather.as_mut().unwrap();
    (g.target, g.point) = (-1, [NO_POINT, NO_POINT, 0.0]);
    assert_eq!(
        t.drop_off_ask(1, 0, (true, true)),
        ask(DropOffBy::Gather, false, true)
    );
    assert_eq!(
        t.drop_off_ask(1, 0, (true, false)),
        ask(DropOffBy::Gather, false, false)
    );
    // a Hunt and a Build of a wood task, aimed
    let mut hunt = crate::gathering::hunt::tests::hunting(600);
    hunt.task.resource = 1;
    let v = &mut t.players[1].objects[0];
    v.gather = None;
    v.hunt = Some(hunt);
    assert_eq!(
        t.drop_off_ask(1, 0, (true, true)),
        ask(DropOffBy::Hunt, true, false)
    );
    let task = crate::construction::build::BuildTask {
        range: 0.1,
        work_value: 1.0,
        work_rate: 1.0,
        resource: 1,
        id: -1,
    };
    let v = &mut t.players[1].objects[0];
    v.build = Some(crate::construction::build::BuildAction::start(
        10028,
        [10.5, 10.5, 0.0],
        5.0,
        task,
    ));
    assert_eq!(
        t.drop_off_ask(1, 0, (true, true)),
        ask(DropOffBy::Build, true, false)
    );
    let v = &mut t.players[1].objects[0];
    (v.build, v.hunt) = (None, None);
    assert_eq!(t.drop_off_ask(1, 0, (true, true)), None);
}

/// The drop-off order 7 to the forager's Gather: it carries its berries to the town centre
/// 700 (state 8), which counts it as a worker; a load of wood instead posts the other load
/// (0x21c) and leaves the Gather; the order 8 to an unaimed Gather records its work for the
/// town bell, drops off and queues the way back.
#[test]
fn a_drop_off_order_takes_a_gather_to_its_site_or_posts_the_other_load() {
    let mut t = foraging();
    t.unit_order(1, &[9175], 7).unwrap();
    let g = t.players[1].objects[0].gather.expect("the Gather");
    assert_eq!((g.carry_to, g.state), (700, 8));
    assert_eq!(t.players[1].objects[2].workers, 1);
    let mut t = foraging();
    t.players[1].objects[0].life.attribute_type = 1;
    let before = t.players[1].objects[0].gather;
    t.unit_order(1, &[9175], 7).unwrap();
    let v = &t.players[1].objects[0];
    assert_eq!(v.gather, before);
    let ai = v.ai.as_ref().expect("the forager's unit AI");
    assert!(
        ai.notifies
            .iter()
            .any(|n| n.kind == crate::unit_ai::OTHER_LOAD),
        "{:?}",
        ai.notifies
    );
    let mut t = foraging();
    let g = t.players[1].objects[0].gather.as_mut().unwrap();
    (g.target, g.point) = (-1, [NO_POINT, NO_POINT, 0.0]);
    t.unit_order(1, &[9175], 8).unwrap();
    let v = &t.players[1].objects[0];
    assert_eq!(v.gather.map(|g| (g.carry_to, g.state)), Some((700, 8)));
    assert_eq!(v.ai.as_ref().map(|ai| ai.orders.len()), Some(1));
}

/// The drop-off order 7 to a Hunt carrying its load: it goes to the town centre 700 (state 8),
/// which counts it as a worker.
#[test]
fn a_drop_off_order_takes_a_hunt_to_its_site() {
    let mut t = foraging();
    let v = &mut t.players[1].objects[0];
    v.gather = None;
    let mut hunt = crate::gathering::hunt::tests::hunting(600);
    hunt.task.resource = 17;
    v.hunt = Some(hunt);
    t.unit_order(1, &[9175], 7).unwrap();
    let h = t.players[1].objects[0].hunt.expect("the Hunt");
    assert_eq!((h.carry_to, h.state), (700, 8));
    assert_eq!(t.players[1].objects[2].workers, 1);
}

/// The drop-off orders fail on a sleeping object and on a unit whose action the slice does not
/// hold; they pass over a unit not in state 2, below type 40 or carrying nothing.
#[test]
fn a_drop_off_order_fails_on_what_it_does_not_hold() {
    let mut t = foraging();
    t.players[1].objects[0].unmodelled_action = Some(GenieError::unsupported("a test's action"));
    assert_eq!(
        t.unit_order(1, &[9175], 7).err().map(|e| e.reason()),
        Some("a UNIT_ORDER drop-off of an action not held")
    );
    let mut t = foraging();
    t.players[1].objects[0].life.amount = 0.0;
    let before = t.players[1].objects[0].clone();
    t.unit_order(1, &[9175, 12345], 7).unwrap();
    assert_eq!(t.players[1].objects[0], before);
}

/// The attack command's answer settled ([`super::attack`]'s `settle_command`): readyToAttack's
/// failure first runs the animation at speed 1; the start's notifies go to the unit AI; a
/// failure out of reach fails the Attack; a ready unit without a unit AI waits ready on its
/// reload; a `MoveTo` started arrived shows the standing sprite at the standing speed; with
/// none of them the Attack and the unit stay.
#[test]
fn an_attack_task_settles_what_its_command_answered() {
    use crate::combat::attack::{Commanded, FAILED, READY};
    use crate::combat::damage::Victim;
    fn shown_sprite(t: &World) -> Option<i16> {
        t.players[1].objects[0].sprite.map(|s| s.graphic.id)
    }
    let settled = |f: fn(&mut Commanded), ai: bool| {
        let (mut t, task) = work_attack(0);
        if !ai {
            t.players[1].objects[0].ai = None;
        }
        t.players[1].objects[0].attack_timer = 0.5;
        t.start_attack(1, 0, 50, &task, [10.4, 10.0, 0.0], true)
            .unwrap();
        let unit = &mut t.players[1].objects[0];
        let attack = unit.attack.take().expect("the soldier's Attack");
        unit.anim_speed = Some(0.5);
        let mut commanded = Commanded {
            attack,
            arrived: false,
            posted: Vec::new(),
            ready: false,
            failed: false,
            failed_first: false,
        };
        f(&mut commanded);
        let start = attack::StartStrike {
            standing: t.graphics[&300],
            volleys: false,
            reload: 2.0,
            hit_chance: 100,
            swing_sprites: crate::combat::attack::SwingSprites::default(),
            computer: false,
            world_time: 0,
            own: Victim::default(),
        };
        let attack = t
            .settle_command(1, 0, commanded, start, &mut Victim::default())
            .unwrap();
        let unit = &t.players[1].objects[0];
        let notifies = unit.ai.as_ref().map(|ai| ai.notifies.len());
        (attack, shown_sprite(&t), unit.anim_speed, notifies)
    };
    let (plain, sprite, speed, notifies) = settled(|_| {}, true);
    assert_eq!((sprite, speed), (Some(300), Some(0.5)));
    assert_eq!(settled(|c| c.failed_first = true, true).2, Some(1.0));
    let posted = settled(|c| c.posted = vec![(0x1fd, [600, 0, 0])], true).3;
    assert_eq!(posted, notifies.map(|n| n + 1));
    assert_eq!(settled(|c| c.failed = true, true).0.state, FAILED);
    assert_eq!(settled(|c| c.ready = true, false).0.state, READY);
    let (arrived, sprite, speed, _) = settled(|c| c.arrived = true, true);
    assert_eq!((arrived, sprite, speed), (plain, Some(300), Some(0.0)));
}

/// The work command's task (`FUN_14106ff70`): none on a point; the forager's Gather task 2 on
/// Gaia's bush (its master 120); none on its town centre, which no task of it names.
#[test]
fn a_work_command_finds_its_master_and_task_on_a_target() {
    let t = crate::world::tests::forage_world();
    assert_eq!(t.work_task(1, 0, -1, -1).unwrap(), None);
    let (with, task) = t
        .work_task(1, 0, 600, -1)
        .unwrap()
        .expect("the Gather task");
    assert_eq!(
        (with, task.id, task.action_type),
        (120, 2, crate::gathering::gather::GATHER)
    );
    assert_eq!(t.work_task(1, 0, 700, -1).unwrap(), None);
}

/// A work command on the other bush retargets the forager's Gather on the same task, and on
/// the other deer the hunter's Hunt: the unit AI hears the gather or hunt order on it.
#[test]
fn a_work_command_retargets_a_gather_or_a_hunt_on_its_task() {
    use crate::unit_ai::{GATHER_ACTION, HUNT_ACTION, HUNT_ORDER, ai_kinds::ORDER_GATHER};
    let mut t = crate::world::tests::forage_world();
    t.gather_command(1, 0, 600).unwrap();
    let took = t
        .retarget_gather_work(1, 0, (601, [36.5, 89.5, 0.0], true))
        .unwrap();
    let v = &t.players[1].objects[0];
    assert_eq!((took, v.gather.map(|g| g.target)), (true, Some(601)));
    let ai = v.ai.as_ref().unwrap();
    assert_eq!(
        (ai.current_order, ai.current_action, ai.current_target),
        (ORDER_GATHER, GATHER_ACTION, 601)
    );
    let mut t = crate::world::tests::hunt_world();
    t.hunt_command(1, 0, 500).unwrap();
    t.retarget_hunt_work(1, 0, (501, [36.5, 89.5, 0.0], true))
        .unwrap();
    let v = &t.players[1].objects[0];
    assert_eq!(v.hunt.map(|h| h.target), Some(501));
    let ai = v.ai.as_ref().unwrap();
    assert_eq!(
        (ai.current_order, ai.current_action, ai.current_target),
        (HUNT_ORDER, HUNT_ACTION, 501)
    );
}

/// A Gather or a Hunt sent on to an object of its own player's master 84 is not modelled.
#[test]
fn a_work_command_s_retarget_to_its_player_s_master_84_fails() {
    let with_84 = |mut t: World| {
        for masters in &mut t.masters {
            let m84 = Master {
                id: 84,
                kind: 70,
                class: 9,
                ..Master::default()
            };
            *masters = crate::objects::masters::Masters::new(masters.iter().cloned().chain([m84]));
        }
        t.players[1].objects.push(Unit {
            id: 800,
            position: [33.5, 89.5, 0.0],
            life: crate::objects::lifecycle::Lifecycle {
                master_id: 84,
                state: 2,
                hp: 5.0,
                ..crate::objects::lifecycle::Lifecycle::default()
            },
            ..Unit::default()
        });
        t
    };
    let mut t = with_84(crate::world::tests::forage_world());
    t.gather_command(1, 0, 600).unwrap();
    let r = t.retarget_gather_work(1, 0, (800, [33.5, 89.5, 0.0], true));
    assert_eq!(
        r.err().map(|e| e.reason()),
        Some("a gather sent to its player's master 84")
    );
    let mut t = with_84(crate::world::tests::hunt_world());
    t.hunt_command(1, 0, 500).unwrap();
    let r = t.retarget_hunt_work(1, 0, (800, [33.5, 89.5, 0.0], true));
    assert_eq!(
        r.err().map(|e| e.reason()),
        Some("a hunt sent to its player's master 84")
    );
}

/// The retake of the work's task by the unit's top action: a Gather on the same task keeps
/// the work (retargeted); a base action type on top of the task's type answers 0 (the factory
/// follows); another current task, or a Build of the unit's former master, is not modelled;
/// no action on top answers 0.
#[test]
fn a_work_command_s_task_is_retaken_by_the_action_on_top() {
    use crate::objects::masters::Task;
    let mut t = crate::world::tests::forage_world();
    let task = *t.masters[1].get(120).unwrap().tasks.first().unwrap();
    let at = (601, [36.5, 89.5, 0.0], true);
    assert_eq!(t.retake_task(1, 0, at, &task, true), Ok(false));
    t.gather_command(1, 0, 600).unwrap();
    assert_eq!(t.retake_task(1, 0, at, &task, true), Ok(true));
    let other = Task { id: 9, ..task };
    assert_eq!(
        t.retake_task(1, 0, at, &other, true)
            .err()
            .map(|e| e.reason()),
        Some("a work command on the unit's current task")
    );
    let mut t = crate::world::tests::forage_world();
    t.players[1].objects[0].move_to = Some(crate::movement::move_to::MoveToAction {
        run: false,
        state: crate::movement::move_to::MoveTo::Moving,
        goal: Some([40.0, 89.5, 0.0]),
        target: -1,
        range: 0.0,
        wait: crate::movement::move_to::MoveWait::default(),
    });
    let moving = Task {
        action_type: crate::movement::move_to::MOVE_TO,
        ..task
    };
    assert_eq!(t.retake_task(1, 0, at, &moving, true), Ok(false));
    let mut t = crate::world::tests::forage_world();
    let build = crate::construction::build::BuildTask {
        range: 0.1,
        work_value: 1.0,
        work_rate: 1.0,
        resource: -1,
        id: 3,
    };
    t.players[1].objects[0].build = Some(crate::construction::build::BuildAction::start(
        10028,
        [30.0, 88.0, 0.0],
        1.0,
        build,
    ));
    let building = Task {
        action_type: crate::construction::build::BUILD,
        ..task
    };
    assert_eq!(
        t.retake_task(1, 0, at, &building, false)
            .err()
            .map(|e| e.reason()),
        Some("a work command on the Build task of the unit's former master")
    );
}

/// The action list's factory for the work's task: the Gather task starts a Gather on the
/// bush; a task of an action type the slice does not make is not modelled.
#[test]
fn a_work_command_s_factory_starts_the_task_s_action() {
    use crate::objects::masters::Task;
    let mut t = crate::world::tests::forage_world();
    let task = *t.masters[1].get(120).unwrap().tasks.first().unwrap();
    t.start_task(1, 0, (600, [32.5, 89.5, 0.0]), &task, (true, true))
        .unwrap();
    assert_eq!(
        t.players[1].objects[0].gather.map(|g| (g.target, g.state)),
        Some((600, 4))
    );
    let unknown = Task {
        action_type: 0x55,
        ..task
    };
    assert_eq!(
        t.start_task(1, 0, (600, [32.5, 89.5, 0.0]), &unknown, (true, true))
            .err()
            .map(|e| e.reason()),
        Some("the action factory for a task the slice does not model")
    );
}
