use super::*;
use crate::artifact::Artifact;
use crate::build::{BuildAction, BuildTask, Construction};
use crate::checkpoint::TopAction;
use crate::fly::Fly;
use crate::lifecycle::seconds;
use crate::lifecycle::{Class, Lifecycle, MasterData};
use crate::masters::{Master, Masters};
use crate::move_to::MoveTo;
use crate::rng::{Stream, Streams, Well512};
use crate::spatial::{Placed, Spatial};
use crate::tasks::{Actor, Diplomacy};
use crate::turn::command::{Command, GatherPoint, ResearchAt};
use crate::turn::player::Player;
use crate::turn::unit::{LIVESTOCK, Unit, give_load, loses_actions};
use crate::turn::view::{Foreign, Wait, patrol_information};
use crate::turn::world::{
    SideWorld, World, change_hands, count_class, search_origin, task_search, task_target_with,
    wait_progress,
};
use crate::unit_ai::{BUILD_ORDER, GenieError, Notify, OWNER_CHANGED, UnitAi};
use crate::visibility::{Seer, capture_kind};

fn player(objects: Vec<Option<UnitAi>>) -> Player {
    Player {
        objects: objects
            .into_iter()
            .map(|ai| Unit {
                ai,
                ..Unit::default()
            })
            .collect(),
        ..Player::default()
    }
}

fn building() -> UnitAi {
    UnitAi {
        ai_type: 17,
        current_order: -1,
        current_action: -1,
        current_target: -1,
        adjusted_idle_timeout: 3000,
        look_around_timeout: 1000,
        retarget_timer: 1,
        ..UnitAi::default()
    }
}

#[test]
fn the_turn_rotates_and_only_the_turn_player_updates() {
    let mut t = Turns::new(
        0,
        29910,
        1,
        vec![0, 0, 0],
        vec![
            player(vec![Some(building())]),
            player(vec![Some(building())]),
            player(vec![]),
        ],
    );
    t.tick(10);
    assert_eq!(t.player_turn, 2);
    assert_eq!(t.player_time, [10, 0, 10]);
    assert_eq!(
        t.players[0].objects[0]
            .ai
            .as_ref()
            .unwrap()
            .last_retarget_time,
        0
    );
    assert_eq!(
        t.players[1].objects[0]
            .ai
            .as_ref()
            .unwrap()
            .last_retarget_time,
        10
    );
    t.tick(10);
    t.tick(10);
    // turns 2 (no objects), then 0; back at player 1
    assert_eq!(t.player_turn, 1);
    assert_eq!(t.player_time, [0, 20, 10]);
    assert_eq!(
        t.players[0].objects[0]
            .ai
            .as_ref()
            .unwrap()
            .last_retarget_time,
        30
    );
}

#[test]
fn the_frame_seed_chains_through_two_world_draws() {
    let mut t = Turns::new(0, 29910, 0, vec![0], vec![player(vec![])]);
    t.tick(10);
    // SeedAll(29910); s1 = World.rand15 = 19550 (see rng); SeedAll(19550); World.rand15
    let mut s = Streams::default();
    s.seed_all(19550);
    assert_eq!(t.frame_seed, s.get(Stream::World).rand15());
}

#[test]
fn artifacts_of_every_player_start_updating_in_the_second_tick() {
    let animal = |gaia| Unit {
        artifact: Some(Artifact::new(gaia)),
        ..Unit::default()
    };
    let players = vec![
        Player {
            objects: vec![animal(true)],
            ..Player::default()
        },
        Player {
            objects: vec![animal(false)],
            ..Player::default()
        },
    ];
    let mut t = Turns::new(0, 1, 0, vec![0, 0], players);
    let state = |t: &Turns, p: usize| t.players[p].objects[0].artifact.unwrap().state;
    t.tick(50);
    assert_eq!((state(&t, 0), state(&t, 1)), (6, 6));
    t.tick(50);
    assert_eq!((state(&t, 0), state(&t, 1)), (10, 10));
}

#[test]
fn an_unsupported_unit_stops_and_is_recorded() {
    let mut odd = building();
    // a unit-AI type the slice does not model
    odd.ai_type = 99;
    let mut t = Turns::new(0, 1, 0, vec![0], vec![player(vec![Some(odd)])]);
    t.tick(10);
    t.tick(10);
    assert_eq!(
        t.players[0].unsupported,
        Some((1, 0, GenieError::unsupported("unit-AI type not modelled")))
    );
}

fn dead_building() -> Unit {
    Unit {
        id: 1,
        ai: Some(building()),
        life: Lifecycle {
            class: Class::Animated,
            hp: 0.0,
            master: Some(MasterData::default()),
            ..Lifecycle::default()
        },
        ..Unit::default()
    }
}

/// A live building whose retarget timer falls due in the fourth tick of 10 ms (world time 40).
fn live(id: i32) -> Unit {
    Unit {
        id,
        ai: Some(UnitAi {
            retarget_timer: 40,
            ..building()
        }),
        ..Unit::default()
    }
}

fn ids(t: &Turns) -> Vec<i32> {
    t.players[0].objects.iter().map(|o| o.id).collect()
}

fn retarget(t: &Turns) -> Vec<u32> {
    t.players[0]
        .objects
        .iter()
        .map(|o| o.ai.as_ref().unwrap().retarget_timer)
        .collect()
}

#[test]
fn a_removed_object_is_replaced_by_the_last_one_which_updates_in_the_same_tick() {
    // RGE_Object_List::update: the dead building leaves in its fourth update; the last object
    // takes its slot and updates there, so it draws before the object behind the slot
    let mut t = Turns::new(
        0,
        29910,
        0,
        vec![0],
        vec![Player {
            objects: vec![dead_building(), live(2), live(3)],
            ..Player::default()
        }],
    );
    for _ in 0..4 {
        t.tick(10);
    }
    assert_eq!(ids(&t), [3, 2]);
    // the same ticks with the two live buildings in that order from the start: their timers
    // fall due in the fourth tick, so its draws depend only on the order
    let mut reordered = Turns::new(
        0,
        29910,
        0,
        vec![0],
        vec![Player {
            objects: vec![live(3), live(2)],
            ..Player::default()
        }],
    );
    for _ in 0..4 {
        reordered.tick(10);
    }
    assert_eq!(retarget(&t), retarget(&reordered));
}

#[test]
fn a_dead_objects_unit_ai_draws_nothing() {
    let one = |objects| {
        let mut t = Turns::new(
            0,
            29910,
            0,
            vec![0],
            vec![Player {
                objects,
                ..Player::default()
            }],
        );
        t.tick(10);
        t
    };
    let with_dead = one(vec![dead_building(), live(2)]);
    let alone = one(vec![live(2)]);
    assert_eq!(with_dead.players[0].objects[0].life.state, 5);
    // its full update adds the world's dt to the time since the last enemy sighting, and
    // returns before anything else
    let dead_ai = dead_building().ai.map(|ai| UnitAi {
        time_since_enemy: ai.time_since_enemy + 0.01,
        ..ai
    });
    assert_eq!(
        with_dead.players[0].objects[0].ai, dead_ai,
        "the dead building's timers stay"
    );
    assert_eq!(
        with_dead.players[0].objects[1].ai,
        alone.players[0].objects[0].ai
    );
}

/// The masters of MarvinBlue/426048731's player 2 that its BUILD at tick 679 uses, shortened
/// from build 158041's save and `.dat`.
fn builder_masters() -> Masters {
    use crate::masters::{Master, Masters, Task};
    let task = |action_type, object_group| Task {
        action_type,
        object_group,
        ..Task::default()
    };
    let villager = |id, group, tasks: Vec<Task>| Master {
        id,
        kind: 70,
        class: 4,
        task_by_group: group,
        radius: [0.2, 0.2],
        tall: true,
        obstruction_type: 0,
        buildable_on: 0,
        tasks,
        cost: vec![],
        obstruction_class: 0,
        object_flags: 0,
        combat_level: 0,
        work_rate: 1.0,
        hit_points: 25,
        build_time: 0,
        // a villager moves (its master's slot 0xe8), which BUILD's last loop asks
        moving: Some(crate::movement::MoveStats {
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
            facets: None,
            weapon_range: 0.0,
        }),
        ..Master::default()
    };
    let base = || vec![task(7, -1), task(3, 3)];
    let builder = || vec![task(101, -1), task(3, 3)];
    Masters::new([
        villager(83, 1, base()),
        villager(118, 1, builder()),
        villager(212, 2, builder()),
        villager(293, 2, base()),
        Master {
            id: 70,
            kind: 80,
            class: 3,
            task_by_group: 0,
            radius: [1.0, 1.0],
            tall: true,
            obstruction_type: 0,
            buildable_on: 0,
            tasks: vec![],
            cost: vec![(1, 25, true)],
            obstruction_class: 2,
            object_flags: 0,
            combat_level: 0,
            work_rate: 0.0,
            hit_points: 550,
            build_time: 25,
            // a house's clearance, which BUILD's placement test reads
            clearance: [1.0, 1.0],
            // built by the villagers (the builders' test, FUN_141262270)
            trained_at: vec![118],
            // a house's building part: no annexes, no stack unit, not an annex
            building: Some(crate::masters::BuildingMaster {
                stack_unit: -1,
                foundation_terrain: -1,
                tech: -1,
                annexes: [-1; 4],
                head_unit: -1,
                transform_unit: -1,
                ..crate::masters::BuildingMaster::default()
            }),
            ..Master::default()
        },
    ])
}

/// A flat map of terrain 0 that the first terrain restriction lets anything stand on, every
/// tile seen by every player: BUILD's placement test passes anywhere free.
fn placeable(t: &mut Turns) {
    let (w, h) = (120, 120);
    let n = (w * h) as usize;
    t.spatial.map = [w, h];
    let mut terrain = crate::terrain::Terrain::new(
        w,
        h,
        vec![(0, 255, -1); n],
        vec![
            crate::terrain::TerrainKind {
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
    let players = t.players.len();
    let seen = (0..players).fold(0u64, |b, p| b | (1 << p) | (1 << (32 + p)));
    t.spatial.sight = Some(crate::visibility::Visibility::new(
        w,
        h,
        (0..players)
            .map(|_| crate::visibility::VisibleMap {
                counts: vec![0; n],
                explored: 0,
            })
            .collect(),
        vec![seen; n],
        Vec::new(),
        0,
        vec![false; players],
    ));
}

/// `Process_New_Tiles`: player 1's newly explored tiles, in order, give the objects of its
/// masters that gather by themselves to its lists by group, at their places.
#[test]
fn a_player_s_new_tiles_give_their_resources_to_its_lists() {
    use crate::masters::{Master, Masters};
    use crate::visibility::{VisibleResource, VisibleResources};
    let object = |id, master, x, y| Unit {
        id,
        position: [x, y, 0.0],
        life: Lifecycle {
            class: Class::Static,
            state: 2,
            master_id: master,
            ..Lifecycle::default()
        },
        ..Unit::default()
    };
    let mut t = Turns::new(
        0,
        1,
        1,
        vec![0, 0],
        vec![
            Player {
                sleeping: vec![
                    object(501, 411, 10.5, 10.25),
                    object(502, 412, 10.5, 10.5),
                    object(503, 413, 11.5, 10.75),
                ],
                ..Player::default()
            },
            Player::default(),
        ],
    );
    let master = |id, auto_gather, gather_group| Master {
        id,
        kind: 10,
        auto_gather,
        gather_group,
        ..Master::default()
    };
    t.masters = vec![
        Masters::new([
            master(411, true, 0),
            master(412, false, 0),
            master(413, true, 4),
        ]),
        Masters::default(),
    ];
    t.spatial = Spatial::with_tiles(
        [120, 120],
        crate::spatial::Tiles::new(120, 120, vec![0; 120 * 120]),
    );
    placeable(&mut t);
    for u in &t.players[0].sleeping {
        t.spatial.insert(
            u.id,
            Placed {
                position: u.position,
                radius: [0.5, 0.5],
                class: 15,
                player: 0,
                master: u.life.master_id,
                obstruction_type: 0,
                buildable_on: 0,
                tall: false,
                speed: 0.0,
                unknown: None,
                obstruction: None,
            },
        );
    }
    let v = t.spatial.sight.as_mut().unwrap();
    v.resources[1] = VisibleResources {
        capacities: vec![32; 5],
        lists: vec![Vec::new(); 5],
    };
    v.new_tiles[1] = vec![[11, 10], [10, 10]];
    t.process_new_tiles(1).unwrap();
    let rec = |id, x, y| VisibleResource {
        id,
        x,
        y,
        ..VisibleResource::default()
    };
    let v = t.spatial.sight.as_ref().unwrap();
    assert_eq!(v.resources[1].lists[0], [rec(501, 10.5, 10.25)]);
    assert_eq!(v.resources[1].lists[4], [rec(503, 11.5, 10.75)]);
    assert_eq!(v.resources[0], VisibleResources::default());
}

/// `FUN_1411980b0`: a static resource that runs out (state 3 to 7) leaves the players'
/// visible-resource lists at once; an animated one (state 5 to 7, not below 4) only when it
/// is deleted (state 8 leaving its list, `FUN_14116b5e0`).
#[test]
fn a_resource_leaves_the_lists_when_it_runs_out_or_is_deleted() {
    use crate::lifecycle::MasterData;
    use crate::masters::{Master, Masters};
    use crate::visibility::VisibleResources;
    let dead = |id, class, state, master| Unit {
        id,
        position: [10.5, 10.5, 0.0],
        life: Lifecycle {
            class,
            state,
            master_id: master,
            attribute_type: -1,
            master: Some(MasterData {
                decay: 0.0,
                undead: false,
            }),
            ..Lifecycle::default()
        },
        ..Unit::default()
    };
    let mut t = Turns::new(
        0,
        1,
        1,
        vec![0, 0],
        vec![
            Player {
                objects: vec![
                    dead(501, Class::Static, 3, 411),
                    dead(502, Class::Animated, 5, 412),
                ],
                ..Player::default()
            },
            Player::default(),
        ],
    );
    let master = |id, gather_group| Master {
        id,
        kind: 10,
        auto_gather: true,
        gather_group,
        ..Master::default()
    };
    t.masters = vec![
        Masters::new([master(411, 0), master(412, 1)]),
        Masters::default(),
    ];
    placeable(&mut t);
    let v = t.spatial.sight.as_mut().unwrap();
    for p in 0..2 {
        v.resources[p] = VisibleResources {
            capacities: vec![32; 5],
            lists: vec![Vec::new(); 5],
        };
        v.resources[p].add(501, 0, 10.5, 10.5);
        v.resources[p].add(502, 1, 10.5, 10.5);
    }
    let held = |t: &Turns| -> Vec<(usize, usize)> {
        let v = t.spatial.sight.as_ref().unwrap();
        (0..2)
            .map(|p| (v.resources[p].lists[0].len(), v.resources[p].lists[1].len()))
            .collect()
    };
    t.tick(10);
    assert_eq!(t.players[0].objects[0].life.state, 7);
    assert_eq!(held(&t), [(0, 1), (0, 1)]);
    t.tick(10);
    assert_eq!(held(&t), [(0, 1), (0, 1)]);
    t.tick(10);
    assert_eq!(t.players[0].objects, []);
    assert_eq!(held(&t), [(0, 0), (0, 0)]);
}

/// `FUN_14162ce00`: a new object of a master that gathers by itself joins the lists of the
/// players that explored its tile; one of another master joins none.
#[test]
fn a_new_resource_joins_the_lists_of_the_players_that_explored_its_tile() {
    use crate::masters::{Master, Masters};
    use crate::visibility::{VisibleResource, VisibleResources};
    let made = |id, master| Unit {
        id,
        position: [20.75, 30.25, 0.0],
        life: Lifecycle {
            state: 2,
            master_id: master,
            ..Lifecycle::default()
        },
        ..Unit::default()
    };
    let mut t = Turns::new(
        0,
        1,
        1,
        vec![0, 0, 0],
        vec![Player::default(), Player::default(), Player::default()],
    );
    let master = |id, auto_gather| Master {
        id,
        kind: 30,
        auto_gather,
        gather_group: 2,
        ..Master::default()
    };
    t.masters = vec![Masters::new([master(53, true), master(54, false)]); 3];
    placeable(&mut t);
    let v = t.spatial.sight.as_mut().unwrap();
    // players 0 and 2 explored the tile, player 1 did not
    v.unified[30 * 120 + 20] = 1 << 32 | 1 << 34;
    for r in &mut v.resources {
        *r = VisibleResources {
            capacities: vec![32; 5],
            lists: vec![Vec::new(); 5],
        };
    }
    t.players[0].objects.push(made(700, 53));
    t.made_new_object(0).unwrap();
    t.players[0].objects.push(made(701, 54));
    t.made_new_object(0).unwrap();
    let fish = VisibleResource {
        id: 700,
        x: 20.75,
        y: 30.25,
        ..VisibleResource::default()
    };
    let v = t.spatial.sight.as_ref().unwrap();
    let lists: Vec<_> = v.resources.iter().map(|r| r.lists[2].clone()).collect();
    assert_eq!(lists, [vec![fish], vec![], vec![fish]]);
}

/// `FUN_141226060`: the class counters by class and master.
#[test]
fn a_combat_object_counts_in_its_player_s_class_attributes() {
    use crate::masters::Master;
    let counted = |class, id, task_by_group| {
        let mut a = vec![0.0f32; 240];
        let m = Master {
            id,
            class,
            task_by_group,
            ..Master::default()
        };
        count_class(&mut a, &m, 1.0);
        count_class(&mut a, &m, 1.0);
        [a[37], a[38], a[40], a[235]]
    };
    assert_eq!(counted(6, 4, 0), [0.0, 0.0, 2.0, 0.0]);
    assert_eq!(counted(13, 1663, 0), [0.0, 0.0, 2.0, 2.0]);
    assert_eq!(counted(57, 1697, 0), [0.0, 0.0, 2.0, 2.0]);
    assert_eq!(counted(4, 83, 1), [2.0, 2.0, 0.0, 0.0]);
    assert_eq!(counted(4, 83, 2), [2.0, 2.0, 0.0, 0.0]);
    assert_eq!(counted(4, 83, 3), [2.0, 0.0, 0.0, 0.0]);
    assert_eq!(counted(19, 128, 0), [2.0, 0.0, 0.0, 0.0]);
    assert_eq!(counted(58, 1663, 1), [0.0; 4]);
    assert_eq!(counted(3, 109, 0), [0.0; 4]);
    let mut a = vec![5.0f32; 41];
    count_class(
        &mut a,
        &Master {
            class: 6,
            ..Master::default()
        },
        -1.0,
    );
    assert_eq!(a[40], 4.0);
}

/// `0x141071880`'s idle swap: a standing sprite that looped draws on the Sprite stream and,
/// above 0x3fff, swaps to the other standing sprite at its first frame; a sprite that did not
/// loop, another sprite or a master without a second standing sprite draws nothing.
#[test]
fn an_idle_object_swaps_its_standing_sprite_on_a_high_draw() {
    use crate::sprite::{ActiveSprite, Graphic};
    let g = |id| Graphic {
        id,
        frames: 10,
        facets: 8,
        speed_multiplier: 1.0,
        frame_duration: 0.1,
        replay_delay: 0.0,
        flags: 1,
    };
    let graphics: std::collections::BTreeMap<i16, Graphic> =
        [(100, g(100)), (101, g(101))].into_iter().collect();
    let master = Master {
        standing_sprite: 100,
        standing_sprite_2: Some(101),
        ..Master::default()
    };
    let shown = |id, looped| ActiveSprite {
        frame: 4,
        looped,
        ..ActiveSprite::at_rest(g(id), 4)
    };
    // a seed whose first draw is above 0x3fff, another whose first is not
    let rng = |seed| {
        let mut r = Well512::default();
        r.set_seed(seed);
        r
    };
    let high = (0..100u32)
        .find(|&s| 0x3fff < rng(s).rand15())
        .expect("a high draw");
    let low = (0..100u32)
        .find(|&s| rng(s).rand15() <= 0x3fff)
        .expect("a low draw");
    let mut u = Unit {
        sprite: Some(shown(100, true)),
        ..Unit::default()
    };
    let mut r = rng(high);
    u.idle_swap(&master, &graphics, &mut r);
    assert_eq!(u.idle_variant, 1);
    let s = u.sprite.unwrap();
    assert_eq!((s.graphic.id, s.frame), (101, 0));
    assert_eq!(r.draws, 1 + rng(high).draws);
    // a low draw keeps the sprite
    let mut u = Unit {
        sprite: Some(shown(100, true)),
        ..Unit::default()
    };
    u.idle_swap(&master, &graphics, &mut rng(low));
    assert_eq!((u.idle_variant, u.sprite.unwrap().graphic.id), (0, 100));
    // no loop, another sprite, no second standing sprite: no draw
    for (sprite, m) in [
        (shown(100, false), master.clone()),
        (shown(7, true), master.clone()),
        (
            shown(100, true),
            Master {
                standing_sprite_2: None,
                ..master.clone()
            },
        ),
    ] {
        let mut u = Unit {
            sprite: Some(sprite),
            ..Unit::default()
        };
        let mut r = rng(high);
        u.idle_swap(&m, &graphics, &mut r);
        assert_eq!(r.draws, rng(high).draws);
    }
}

#[test]
fn a_build_pays_the_cost_and_tasks_the_builders_of_its_own_player() {
    let villager = |id, master, x, y| Unit {
        id,
        position: [x, y, 0.0],
        life: Lifecycle {
            class: Class::Animated,
            state: 2,
            hp: 25.0,
            master_id: master,
            ..Lifecycle::default()
        },
        // a villager's unit AI (type 8) without an order or action
        ai: Some(UnitAi {
            ai_type: 8,
            current_order: -1,
            current_action: -1,
            current_target: -1,
            ..UnitAi::default()
        }),
        ..Unit::default()
    };
    let mut t = Turns::new(
        0,
        29910,
        1,
        vec![0, 0],
        vec![
            Player::default(),
            Player {
                objects: vec![
                    villager(9175, 83, 30.0, 89.0),
                    villager(9181, 293, 29.0, 99.0),
                ],
                resources: [200.0, 100.0, 200.0, 100.0],
                attributes: vec![0.0; 250],
                ..Player::default()
            },
        ],
    );
    t.next_object_id = 10028;
    t.masters = vec![Masters::default(), builder_masters()];
    placeable(&mut t);
    t.tick_with(
        50,
        &[Command::Build {
            player: 1,
            master: 70,
            unit_ai: Ok(None),
            position: [30.0, 88.0, 0.0],
            builders: vec![9175, 9181],
            queued: false,
            commits: false,
        }],
    );
    let p = &t.players[1];
    assert_eq!(p.resources, [200.0, 75.0, 200.0, 100.0]);
    // the house's paid cost, as a building with a train location, in attribute 247
    assert_eq!(p.attributes[247], 25.0);
    let foundation = p.objects.last().unwrap();
    assert_eq!(
        (
            foundation.id,
            foundation.life.master_id,
            foundation.position
        ),
        (10028, 70, [30.0, 88.0, 0.0])
    );
    // an animated object's constructor (FUN_141070cf0) sets its animation speed (+0x7b8) to 0:
    // its sprite's clock runs from its first update (a dying sprite plays out)
    assert_eq!(foundation.anim_speed, Some(0.0));
    let builder = |i: usize| {
        (
            p.objects[i].life.master_id,
            p.objects[i].build.map(|b| b.state),
        )
    };
    // the builder at the foundation works in its first update; the other asks for a path, for
    // which these villagers lack their movement state
    assert_eq!((builder(0), builder(1)), ((118, Some(6)), (212, Some(4))));
    // each idle villager's town-bell record: no work, at its own place
    assert_eq!(
        p.objects[1].town_bell.work,
        crate::town_bell::BackToWork {
            point: [29.0, 99.0],
            ..crate::town_bell::BackToWork::NONE
        }
    );
    assert_eq!(
        p.unsupported,
        Some((
            1,
            1,
            GenieError::unsupported("a walker without movement state")
        ))
    );
}

/// census v5's 257 stops "a builder of another player or a sleeping one": BUILD's last
/// loop (`FUN_140e7a400`) takes the packet's units the world holds whatever their player: a
/// queued BUILD puts the build order into another player's villager's queue, issued by its
/// own player and naming the BUILD's player as the target's; a unit of a master that does
/// not move is passed over unless the BUILD is queued.
#[test]
fn a_queued_build_queues_its_order_for_another_player_s_villager() {
    let villager = |id, master, x, y| Unit {
        id,
        position: [x, y, 0.0],
        life: Lifecycle {
            class: Class::Animated,
            state: 2,
            hp: 25.0,
            master_id: master,
            ..Lifecycle::default()
        },
        ai: Some(UnitAi {
            ai_type: 8,
            current_order: -1,
            current_action: -1,
            current_target: -1,
            ..UnitAi::default()
        }),
        ..Unit::default()
    };
    let mut t = Turns::new(
        0,
        29910,
        1,
        vec![0, 0, 0],
        vec![
            Player::default(),
            Player {
                objects: vec![villager(9175, 83, 30.0, 89.0)],
                resources: [200.0, 100.0, 200.0, 100.0],
                attributes: vec![0.0; 250],
                ..Player::default()
            },
            Player {
                objects: vec![villager(9618, 293, 29.0, 99.0)],
                attributes: vec![0.0; 250],
                ..Player::default()
            },
        ],
    );
    t.next_object_id = 10028;
    t.masters = vec![Masters::default(), builder_masters(), builder_masters()];
    placeable(&mut t);
    // the command phase alone (the units' updates would run the orders)
    t.run(&Command::Build {
        player: 1,
        master: 70,
        unit_ai: Ok(None),
        position: [30.0, 88.0, 0.0],
        builders: vec![9618, 9175],
        queued: true,
        commits: false,
    });
    let order = |p: usize| {
        t.players[p].objects[0]
            .ai
            .as_ref()
            .and_then(|ai| ai.orders.first().copied())
            .map(|o| (o.issuer, o.order, o.target, o.target_owner))
    };
    assert_eq!(order(2), Some((2, BUILD_ORDER, 10028, 1)));
    assert_eq!(order(1), Some((1, BUILD_ORDER, 10028, 1)));
}

/// A world of player 1's villager 9175 (master 83, which has no Build task) beside its
/// house's foundation 10028 (master 70, state 0, no build points yet), with the unit AI's
/// queue `orders`.
fn sent_off_world(orders: Vec<crate::unit_ai::QueuedOrder>) -> Turns {
    let villager = Unit {
        id: 9175,
        position: [29.5, 89.5, 0.0],
        life: Lifecycle {
            class: Class::Animated,
            state: 2,
            hp: 25.0,
            master_id: 83,
            ..Lifecycle::default()
        },
        ai: Some(UnitAi {
            ai_type: crate::unit_ai::CIVILIAN,
            current_order: -1,
            current_action: -1,
            current_target: -1,
            last_target: -1,
            orders,
            ..UnitAi::default()
        }),
        pasture: -1,
        my_pasture: -1,
        ..Unit::default()
    };
    let foundation = Unit {
        id: 10028,
        position: [30.0, 88.0, 0.0],
        life: Lifecycle {
            class: Class::Animated,
            state: 0,
            hp: 1.0,
            master_id: 70,
            ..Lifecycle::default()
        },
        construction: Some(Construction {
            progress: 0.0,
            desolid: true,
            builders: 0,
            hit_points: 550,
            build_time: 25,
            record: crate::spatial::Obstruction { kind: 2, class: 2 },
            snow: false,
            sprite_override: false,
            started: false,
        }),
        pasture: -1,
        my_pasture: -1,
        ..Unit::default()
    };
    let mut t = Turns::new(
        0,
        29910,
        1,
        vec![0, 0],
        vec![
            Player::default(),
            Player {
                objects: vec![villager, foundation],
                attributes: vec![0.0; 250],
                relations: vec![1, 0],
                ..Player::default()
            },
        ],
    );
    t.masters = vec![builder_masters(), builder_masters()];
    t
}

fn task(action_type: i16) -> crate::masters::Task {
    crate::masters::Task {
        action_type,
        object_group: -1,
        ..crate::masters::Task::default()
    }
}

/// Slot 0x820's search (`FUN_14106c6a0`): the own master's list first; without a task there
/// the player's masters by id of the unit's class, type and swap group, with any filter,
/// the first with a task giving it with the master to switch to; none without a group.
#[test]
fn slot_0x820_s_search_takes_the_own_task_else_the_first_master_of_the_swap_group() {
    let t = sent_off_world(vec![]);
    let masters = &t.masters[1];
    let find = |id: i32| t.object(id);
    let site = task_target_with(10028, 1, &find, &t.masters)
        .unwrap()
        .unwrap();
    let world = World {
        players: &t.players,
        drop_sites: &[],
    };
    let search = |own: &Master| {
        let actor = Actor {
            id: 9175,
            master: own,
            player: 1,
            held: (-1, 0.0),
            attribute_95: 0.0,
            civ: 0,
        };
        task_search(masters, &actor, &site, &site, -1, &world)
            .unwrap()
            .map(|(task, switch)| (task.action_type, switch.map(|m| m.id)))
    };
    // a villager of master 83 builds as the builder 118, the first of its group with a task
    assert_eq!(search(masters.get(83).unwrap()), Some((101, Some(118))));
    // the builders find it in their own lists
    assert_eq!(search(masters.get(118).unwrap()), Some((101, None)));
    assert_eq!(search(masters.get(212).unwrap()), Some((101, None)));
    // without a swap group there is no other list to try
    let alone = Master {
        task_by_group: 0,
        ..masters.get(83).unwrap().clone()
    };
    assert_eq!(search(&alone), None);
}

/// The task finder's target (`Turns::task_target`'s reading): an annex stands for its main
/// building, once per step; an object not held is none.
#[test]
fn a_task_target_reads_an_annex_as_its_main_building() {
    let mut t = sent_off_world(vec![]);
    t.players[1].objects[0].annex_of = Some(10028);
    let find = |id: i32| t.object(id);
    let at = |steps| {
        task_target_with(9175, steps, &find, &t.masters)
            .unwrap()
            .map(|x| (x.id, x.master.id, x.state))
    };
    assert_eq!(at(0), Some((9175, 83, 2)));
    assert_eq!(at(1), Some((10028, 70, 0)));
    assert_eq!(at(2), Some((10028, 70, 0)));
    assert!(
        task_target_with(77, 1, &find, &t.masters)
            .unwrap()
            .is_none()
    );
}

/// The unit's slot 0x388 after its unit AI's update (`FUN_141226fc0`): the switch to the
/// master slot 0x820's search gave (slot 0x90), the actions cleared and the wait (type 0x9a,
/// state 10) the only one, its target −1 (the foundation is at its `+0x68`).
#[test]
fn a_wait_for_a_foundation_switches_the_villager_and_holds_the_task() {
    let mut t = sent_off_world(vec![]);
    let wait = Wait {
        target: 10028,
        task: task(101),
        switch: Some(118),
    };
    t.wait_for_foundation(1, 0, wait).unwrap();
    let v = &t.players[1].objects[0];
    assert_eq!(v.life.master_id, 118);
    assert_eq!(
        v.wait_solidify,
        Some(crate::wait_solidify::WaitSolidify::new(task(101), 10028))
    );
    assert_eq!(
        v.top_action(),
        Some(TopAction {
            kind: 0x9a,
            state: 10
        })
    );
    assert_eq!(v.top_target(), Some(-1));
    // without a switch the master stays
    let mut kept = sent_off_world(vec![]);
    kept.wait_for_foundation(
        1,
        0,
        Wait {
            switch: None,
            ..wait
        },
    )
    .unwrap();
    assert_eq!(kept.players[1].objects[0].life.master_id, 83);
}

/// While a player's list is out for its unit's update, the others are read around it, and
/// the task finder reads the owner's diplomacy from the update and another player's from its
/// own record.
#[test]
fn a_unit_s_side_reads_the_other_players_relations_around_its_own() {
    let players: Vec<Player> = (0..3)
        .map(|q| Player {
            relations: vec![q as u8; 3],
            ..Player::default()
        })
        .collect();
    let foreign = Foreign {
        before: &players[..1],
        after: &players[2..],
        left: None,
    };
    assert_eq!(foreign.player(0).map(|p| p.relations[0]), Some(0));
    assert!(foreign.player(1).is_none());
    assert_eq!(foreign.player(2).map(|p| p.relations[0]), Some(2));
    assert!(foreign.player(3).is_none());
    // the owner (1) counts player 2 as an ally (0), player 2 counts the owner an enemy (2)
    let world = SideWorld {
        player: 1,
        relations: &[1, 1, 0],
        foreign,
        drop_sites: &[],
    };
    assert!(world.allied(1, 2, false));
    assert!(!world.allied(2, 1, false));
    assert!(world.allied(1, 1, false));
}

/// The wait's foundation as its update reads it: its build points among the owner's other
/// objects or the other players', none when the world does not hold it.
#[test]
fn a_wait_reads_its_foundation_s_build_points_wherever_it_is() {
    let t = sent_off_world(vec![]);
    let (none, players) = (&t.players[..0], &t.players[..]);
    let own = Foreign {
        before: none,
        after: none,
        left: None,
    };
    assert_eq!(wait_progress(10028, &t.players[1].objects, own), Some(0.0));
    assert_eq!(wait_progress(77, &t.players[1].objects, own), None);
    // another player's (player 0 updating, player 1 after it)
    let other = Foreign {
        before: none,
        after: &players[1..],
        left: None,
    };
    assert_eq!(wait_progress(10028, &[], other), Some(0.0));
}

/// `FUN_1411cc430` on an annex whose main building is in state 0 with a hit point (the
/// builder's work deferred to the caller, [`crate::build::Worked::Main`]): the main
/// building's annexes start first (the annex itself among them, desolid), then the main
/// building starts and takes the work; the annex takes none.
#[test]
fn the_work_on_an_annex_goes_to_its_main_building_after_its_annexes_start() {
    let mut t = sent_off_world(vec![]);
    let annex = Unit {
        id: 10031,
        annex_of: Some(10028),
        ..t.players[1].objects[1].clone()
    };
    t.players[1].objects.push(annex);
    t.spatial = Spatial::with_tiles(
        [120, 120],
        crate::spatial::Tiles::new(120, 120, vec![4; 14400]),
    );
    for (id, x) in [(10028, 30.0), (10031, 32.0)] {
        t.spatial.insert(
            id,
            Placed {
                position: [x, 88.0, 0.0],
                radius: [1.0, 1.0],
                class: 3,
                player: 1,
                master: 70,
                obstruction_type: 2,
                buildable_on: 0,
                tall: true,
                speed: 0.0,
                unknown: None,
                obstruction: None,
            },
        );
    }
    t.spatial.set_annex(10031, Some(10028));
    t.work_building(10031, 0.021).unwrap();
    let c = |id: i32| t.object(id).unwrap().1.construction.unwrap();
    let started = f32::from_bits(0x2edb_e6ff);
    assert!(!c(10031).desolid);
    assert_eq!(c(10031).progress, started);
    assert!(!c(10028).desolid);
    assert_eq!(c(10028).progress, started + 0.021);
    assert_eq!(t.object(10031).unwrap().1.life.hp, 1.0);
    assert!(1.0 < t.object(10028).unwrap().1.life.hp);
}

/// The whole of it: a villager the foundation sent off (its queue the foundation's order)
/// runs the order on its turn (slot 0x108 forced: its slot 0x388 finds the builder's task by
/// the swap group), becomes the builder with the wait as its only action, and keeps waiting
/// while the foundation has no build points; once they pass 0.01 its next update drops the
/// wait, the unit AI's action value staying 0x27a.
#[test]
fn a_villager_sent_off_a_foundation_waits_for_it_as_a_builder() {
    use crate::unit_ai::{FOUNDATION_ORDER, QueuedOrder, WAIT_SOLIDIFY_ACTION};
    let order = QueuedOrder {
        issuer: 1,
        order: FOUNDATION_ORDER,
        priority: 100,
        target: 10028,
        target_owner: 1,
        point: [30.0, 0.0, 0.0],
        range: 0.0,
        shown: [30.0, 0.0],
        ready: true,
        target_type: 70,
    };
    let mut t = sent_off_world(vec![order]);
    t.tick(50);
    let v = &t.players[1].objects[0];
    assert!(!v.stopped, "{:?}", t.players[1].objects[0]);
    assert_eq!(v.life.master_id, 118);
    assert_eq!(v.top_action_kind(), Some(0x9a));
    let ai = v.ai.as_ref().unwrap();
    assert_eq!(
        (ai.current_action, ai.last_action, ai.current_order),
        (WAIT_SOLIDIFY_ACTION, WAIT_SOLIDIFY_ACTION, FOUNDATION_ORDER)
    );
    assert_eq!(ai.orders, vec![]);
    // player 0's update, then player 1's: the wait reads 0 build points and stays
    t.tick(50);
    t.tick(50);
    assert_eq!(t.players[1].objects[0].top_action_kind(), Some(0x9a));
    t.players[1].objects[1]
        .construction
        .as_mut()
        .unwrap()
        .progress = 0.5;
    t.tick(50);
    t.tick(50);
    assert_eq!(t.players[1].freezes, vec![]);
    let v = &t.players[1].objects[0];
    assert_eq!(v.wait_solidify, None);
    assert_eq!(
        v.ai.as_ref().map(|ai| ai.current_action),
        Some(WAIT_SOLIDIFY_ACTION)
    );
}

/// Slot 0x230's build order (`0x2be`): slot 0xf0 builds the foundation (the builder's task by
/// the swap group), then the tail makes the order the AI's with its priority, leaving a patrol
/// order: its secondary timer capped at 750 and an adjusted idle timeout of 750 redrawn.
#[test]
fn a_queued_build_order_builds_and_leaves_a_patrol_order() {
    use crate::unit_ai::{BUILD_ACTION, PATROL, QueuedOrder};
    let order = QueuedOrder {
        issuer: 1,
        order: BUILD_ORDER,
        priority: 100,
        target: 10028,
        target_owner: 1,
        point: [30.0, 88.0, 0.0],
        range: 0.0,
        shown: [30.0, 88.0],
        ready: true,
        target_type: 70,
    };
    let mut t = sent_off_world(vec![]);
    {
        let ai = t.players[1].objects[0].ai.as_mut().unwrap();
        ai.current_order = PATROL;
        ai.secondary_timer = 900;
        ai.adjusted_idle_timeout = 750;
        ai.idle_timeout = 3000;
    }
    let draws = t.streams.get(Stream::Ai).draws;
    t.build_order(1, 0, order).unwrap();
    let v = &t.players[1].objects[0];
    assert_eq!(v.life.master_id, 118);
    assert!(v.build.is_some());
    let ai = v.ai.as_ref().unwrap();
    assert_eq!(
        (
            ai.current_order,
            ai.order_priority,
            ai.current_action,
            ai.current_target
        ),
        (BUILD_ORDER, 100, BUILD_ACTION, 10028)
    );
    assert_eq!(ai.secondary_timer, 750);
    assert_ne!(ai.adjusted_idle_timeout, 750);
    assert_eq!(t.streams.get(Stream::Ai).draws, draws + 1);
}

#[test]
fn a_build_creates_a_foundation_whose_unit_ai_draws_before_the_turn_players_units() {
    // MarvinBlue/426048731 tick 281: player 1's BUILD draws three AI values in the command
    // phase, so player 2's units, on their turn, get the draws after them (a computer's
    // BUILD here: it needs no builder that can build it)
    let world = |objects: Vec<Unit>| {
        let mut t = Turns::new(
            0,
            29910,
            1,
            vec![0, 0],
            vec![
                Player {
                    resources: [0.0, 100.0, 0.0, 0.0],
                    attributes: vec![0.0; 250],
                    computer: true,
                    ..Player::default()
                },
                Player {
                    objects,
                    ..Player::default()
                },
            ],
        );
        t.next_object_id = 100;
        t.masters = vec![builder_masters(), builder_masters()];
        placeable(&mut t);
        t
    };
    let mut built = world(vec![live(2)]);
    built.tick_with(
        40,
        &[Command::Build {
            player: 0,
            master: 70,
            unit_ai: Ok(Some(17)),
            position: [10.0, 10.0, 0.0],
            builders: vec![],
            queued: false,
            commits: false,
        }],
    );
    let foundation = &built.players[0].objects[0];
    assert_eq!(
        (
            foundation.id,
            foundation.life.state,
            foundation.life.master_id
        ),
        (100, 0, 70)
    );
    assert_eq!(built.next_object_id, 101);
    // the same tick without the BUILD, with three AI draws taken by hand after the reseed
    let mut by_hand = world(vec![live(2)]);
    let mut streams = Streams::default();
    streams.seed_all(29910);
    let ai = streams.get(Stream::Ai);
    let new = crate::unit_ai::NewObject {
        position: [10.0, 10.0, 0.0],
        ..crate::unit_ai::NewObject::default()
    };
    let first = UnitAi::constructed(17, &new, ai).unwrap();
    assert_eq!(foundation.ai, Some(first));
    let next = 750 + ai.rand15() % 500;
    by_hand.tick(40);
    assert_eq!(
        built.players[1].objects[0]
            .ai
            .as_ref()
            .unwrap()
            .retarget_timer,
        next
    );
    assert_ne!(
        by_hand.players[1].objects[0]
            .ai
            .as_ref()
            .unwrap()
            .retarget_timer,
        next
    );
}

#[test]
fn an_order_resets_the_idle_timeouts_of_its_units_wherever_they_are() {
    // MOVE: one AI draw per listed unit with a unit AI, in list order, before the turn player's
    // units update; the ordering player's lane stops
    let ordered = |id| Unit {
        id,
        ai: Some(UnitAi {
            idle_timeout: 3000,
            adjusted_idle_timeout: 3000,
            ..building()
        }),
        ..Unit::default()
    };
    let mut t = Turns::new(
        0,
        29910,
        1,
        vec![0, 0],
        vec![
            Player {
                objects: vec![ordered(7), ordered(8)],
                ..Player::default()
            },
            Player {
                objects: vec![live(2)],
                ..Player::default()
            },
        ],
    );
    t.tick_with(
        40,
        &[Command::Order {
            player: 0,
            units: vec![8, 99, 7],
            order: None,
        }],
    );
    let mut streams = Streams::default();
    streams.seed_all(29910);
    let ai = streams.get(Stream::Ai);
    let (eight, seven) = (ai.rand15() % 1500 + 2250, ai.rand15() % 1500 + 2250);
    let next = 750 + ai.rand15() % 500;
    let timeout = |t: &Turns, i: usize| {
        t.players[0].objects[i]
            .ai
            .as_ref()
            .unwrap()
            .adjusted_idle_timeout
    };
    assert_eq!((timeout(&t, 1), timeout(&t, 0)), (eight, seven));
    assert_eq!(
        t.players[1].objects[0].ai.as_ref().unwrap().retarget_timer,
        next
    );
    assert!(t.players[0].unsupported.is_some());
}

#[test]
fn an_unmodelled_action_stops_its_player_unless_the_object_dies_first() {
    let task = GenieError::unsupported("a default task the slice does not model");
    let with_task = |life| Unit {
        id: 5,
        life,
        unmodelled_action: Some(task.clone()),
        ..Unit::default()
    };
    let run = |unit| {
        let mut t = Turns::new(0, 1, 0, vec![0], vec![player(vec![])]);
        t.players[0].objects.push(unit);
        t.tick(10);
        t.players[0].unsupported.clone()
    };
    assert_eq!(
        run(with_task(Lifecycle::default())),
        Some((1, 0, task.clone()))
    );
    let dying = Lifecycle {
        class: Class::Animated,
        hp: 0.0,
        master: Some(MasterData::default()),
        ..Lifecycle::default()
    };
    assert_eq!(run(with_task(dying)), None);
}

/// Build 158041's farm (master 50): its default task 0 is a Make of the farm food kind 16
/// at `work_val1` 1.0, its work rate 0.4 a second, its capacity 15.
fn farm_master() -> Master {
    Master {
        id: 50,
        kind: 80,
        class: crate::gather::FARM_CLASS,
        work_rate: 0.4,
        capacity: 15,
        default_task: Some(0),
        tasks: vec![crate::masters::Task {
            action_type: crate::make::MAKE,
            attribute_type: crate::make::FARM_FOOD_KIND,
            attribute_type2: 0,
            work_val1: 1.0,
            ..crate::masters::Task::default()
        }],
        ..Master::default()
    }
}

#[test]
fn a_farm_starts_its_make_in_its_first_update_and_fills_from_the_next() {
    let farm = Unit {
        id: 40,
        life: Lifecycle {
            master_id: 50,
            attribute_type: 0,
            amount: 15.0,
            ..Lifecycle::default()
        },
        default_make: true,
        ..Unit::default()
    };
    let mut attributes = vec![0.0; 100];
    attributes[crate::make::FARM_FOOD] = 175.0;
    let mut t = Turns::new(
        0,
        1,
        0,
        vec![0],
        vec![Player {
            objects: vec![farm],
            attributes,
            ..Player::default()
        }],
    );
    t.masters = vec![Masters::new([farm_master()])];
    t.tick(104);
    let f = &t.players[0].objects[0];
    let make = f.make.unwrap();
    assert_eq!((make.state, make.remaining, make.total), (6, 175.0, 175.0));
    assert_eq!((f.life.attribute_type, f.life.amount), (16, 0.0));
    assert_eq!(
        f.top_action(),
        Some(TopAction {
            kind: crate::make::MAKE,
            state: 6
        })
    );
    t.tick(104);
    let f = &t.players[0].objects[0];
    let made = 0.4f32 * 1.0 * seconds(104);
    assert_eq!(
        (f.life.amount, f.make.unwrap().remaining),
        (made, 175.0 - made)
    );
    assert!(t.players[0].unsupported.is_none());
}

#[test]
fn a_built_farm_s_master_starts_a_make_and_another_default_task_is_reported() {
    assert_eq!(farm_master().default_start(), (true, None));
    let mut other = farm_master();
    other.tasks[0].action_type = crate::fly::FLY;
    assert!(other.default_start().1.is_some());
    // a type the factories make nothing of starts nothing (a wild animal's Explore, 6)
    other.tasks[0].action_type = 6;
    assert_eq!(other.default_start(), (false, None));
    other.default_task = None;
    assert_eq!(other.default_start(), (false, None));
}

/// `TRIBE_Building_Object::update`'s whole seconds (`0x1411c95a2`): a built building's
/// `damaged_lately_timer` (`+0x30d`) loses one in an update that passes whole seconds, however
/// many, and stays at 0.
#[test]
fn a_building_s_damaged_lately_timer_loses_one_an_update_with_whole_seconds() {
    let house = Unit {
        id: 40,
        life: Lifecycle {
            master_id: 70,
            hp: 550.0,
            state: 2,
            ..Lifecycle::default()
        },
        damaged_lately: 2,
        ..Unit::default()
    };
    let mut t = Turns::new(
        0,
        1,
        0,
        vec![0],
        vec![Player {
            objects: vec![house],
            attributes: vec![0.0; 300],
            ..Player::default()
        }],
    );
    t.masters = vec![Masters::new([Master {
        id: 70,
        kind: 80,
        class: 3,
        hit_points: 550,
        building: Some(crate::masters::BuildingMaster::default()),
        ..Master::default()
    }])];
    let timer = |t: &Turns| t.players[0].objects[0].damaged_lately;
    t.tick(500);
    assert_eq!(timer(&t), 2);
    t.tick(2600);
    assert_eq!(timer(&t), 1);
    t.tick(1000);
    assert_eq!(timer(&t), 0);
    t.tick(1000);
    assert_eq!(timer(&t), 0);
    assert!(t.players[0].unsupported.is_none());
}

fn town_centre(id: i32) -> Unit {
    Unit {
        id,
        ..Unit::default()
    }
}

fn queue(building: i32, unit: i16, amount: u16) -> Command {
    Command::Queue {
        player: 0,
        buildings: vec![building],
        unit,
        amount,
        button: -1,
    }
}

/// A town centre of master 109 ([`one_player`]'s).
fn centre(id: i32) -> Unit {
    let mut tc = town_centre(id);
    tc.life.master_id = 109;
    tc
}

#[test]
fn a_unit_inside_heals_up_to_its_hit_points() {
    // records/425906646's update 2191: villager 27260 inside TC 27150 heals by the TC's
    // garrison heal rate once a second
    let master = Master {
        id: 592,
        class: 4,
        hit_points: 25,
        ..Master::default()
    };
    let mut v = Unit {
        id: 27260,
        life: Lifecycle {
            hp: 11.0,
            ..Lifecycle::default()
        },
        ..Unit::default()
    };
    v.heal(&master, 0.1).unwrap();
    assert_eq!(v.life.hp, 11.1);
    // a heal past the hit points stops at them, the powerup's room taking the rest
    v.life.hp = 24.95;
    v.hp_powerups = [1.0, 2.0];
    v.heal(&master, 0.1).unwrap();
    assert_eq!((v.life.hp, v.hp_powerups), (24.95f32 + 0.1, [1.0, 2.0]));
    // past them with the room: the overflow comes off the powerup, then a full unit holds
    // exactly its master's hit points
    v.life.hp = 26.95;
    v.heal(&master, 0.1).unwrap();
    assert_eq!(v.life.hp, 25.0);
    assert_eq!(v.hp_powerups[0], 1.0 - (26.95f32 + 0.1 - 27.0));
    // a paid class heals at a cost: not modelled
    let ship = Master {
        class: 22,
        ..master
    };
    assert!(v.heal(&ship, 0.1).is_err());
}

/// `FUN_140e6cfc0` resets every listed unit the world holds, a sleeping one too, and passes
/// over an id it does not hold.
#[test]
fn an_order_resets_its_units_sleeping_ones_too() {
    let unit = |id: i32| Unit {
        id,
        ai: Some(UnitAi {
            patrol_path: Some(crate::unit_ai::PatrolPath::default()),
            patrol_waypoint: 2,
            adjusted_idle_timeout: 100,
            secondary_timer: 900,
            ..UnitAi::default()
        }),
        ..Unit::default()
    };
    let player = Player {
        objects: vec![unit(1)],
        sleeping: vec![unit(2)],
        ..Player::default()
    };
    let mut t = Turns::new(0, 0, 0, vec![0, 0], vec![Player::default(), player]);
    t.reset_order_units(&[2, 9, 1]);
    for u in t.players[1].objects.iter().chain(&t.players[1].sleeping) {
        let ai = u.ai.as_ref().unwrap();
        assert_eq!((ai.patrol_path.is_none(), ai.secondary_timer), (true, 750));
    }
}

#[test]
fn a_load_goes_to_its_player_alone_unless_the_player_shares() {
    // records/425906646's update 818: player 6, with allies, does not share (+0x1426 0)
    let relations = vec![vec![0, 3, 3], vec![3, 0, 0], vec![3, 0, 0]];
    let mut adds = Vec::new();
    give_load((&relations, false), 1, &mut adds, 0, 10.0).unwrap();
    assert_eq!(adds, [(0, 10.0), (166, 10.0)]);
    assert!(give_load((&relations, true), 1, &mut adds, 3, 10.0).is_err());
}

/// A gate's slot 0x678 (`FUN_1411d4a30`): a standing, unlocked object of class 39 whose
/// master transforms (`+0x9ba` not -1), between allies; any other object answers false.
#[test]
fn only_a_standing_unlocked_transforming_gate_lets_an_ally_pass() {
    let object = |id: i32, master: i16, state: u8, locked: bool| Unit {
        id,
        life: Lifecycle {
            master_id: master,
            state,
            ..Lifecycle::default()
        },
        gate: Some(Gate {
            locked,
            ..Gate::default()
        }),
        ..Unit::default()
    };
    let player = |objects: Vec<Unit>| Player {
        objects,
        relations: vec![0, 0],
        attributes: vec![0.0; 250],
        ..Player::default()
    };
    let mut t = Turns::new(
        0,
        1,
        0,
        vec![0; 2],
        vec![
            player(Vec::new()),
            player(vec![
                object(1, 64, 2, false),
                object(2, 65, 2, false),
                object(3, 66, 2, false),
                object(4, 64, 3, false),
                object(5, 64, 2, true),
            ]),
        ],
    );
    let master = |id: i16, class: i16, transform_unit: i16| Master {
        id,
        class,
        building: Some(crate::masters::BuildingMaster {
            transform_unit,
            ..crate::masters::BuildingMaster::default()
        }),
        ..Master::default()
    };
    let masters = Masters::new([master(64, 39, 63), master(65, 39, -1), master(66, 4, 63)]);
    t.masters = vec![masters.clone(), masters];
    let passes: Vec<bool> = (0..5).map(|i| t.gate_passes(1, i, 1)).collect();
    assert_eq!(passes, [true, false, false, false, false]);
}

#[test]
fn a_gate_opens_for_its_allies_while_they_update() {
    // records/425906646's update 636: player 2's gate 27687 (master 64, class 39 with a
    // transform unit) has no record, marks or shape while its team updates, so 27274's plan
    // passes it; the next player, an enemy, finds it closed again
    let gate = Unit {
        id: 27687,
        life: Lifecycle {
            master_id: 64,
            state: 2,
            ..Lifecycle::default()
        },
        gate: Some(Gate::default()),
        ..Unit::default()
    };
    let player = |objects: Vec<Unit>, relations: Vec<u8>| Player {
        objects,
        relations,
        attributes: vec![0.0; 250],
        ..Player::default()
    };
    let mut t = Turns::new(
        0,
        1,
        0,
        vec![0; 4],
        vec![
            player(Vec::new(), vec![0, 3, 3, 3]),
            player(Vec::new(), vec![3, 0, 0, 3]),
            player(vec![gate], vec![3, 0, 0, 3]),
            player(Vec::new(), vec![3, 3, 3, 0]),
        ],
    );
    let master = Master {
        id: 64,
        class: 39,
        obstruction_type: 2,
        obstruction_class: 5,
        building: Some(crate::masters::BuildingMaster {
            transform_unit: 63,
            ..crate::masters::BuildingMaster::default()
        }),
        ..Master::default()
    };
    t.masters = (0..4).map(|_| Masters::new([master.clone()])).collect();
    t.spatial = Spatial::with_tiles([20, 20], crate::spatial::Tiles::new(20, 20, vec![1; 400]));
    t.spatial.relations = t.players.iter().map(|p| p.relations.clone()).collect();
    let record = crate::spatial::Obstruction { kind: 2, class: 5 };
    t.spatial.insert(
        27687,
        Placed {
            position: [10.0, 10.5, 0.0],
            radius: [1.0, 0.5],
            class: 39,
            player: 2,
            master: 64,
            obstruction_type: 2,
            buildable_on: 0,
            tall: true,
            speed: 0.0,
            unknown: None,
            obstruction: Some(record),
        },
    );
    let blocked = |t: &Turns| {
        t.spatial
            .tiles()
            .word(9, 10)
            .is_some_and(|w| w & crate::spatial::BLOCKED != 0)
    };
    assert!(blocked(&t));
    let mut open = vec![false; 4];
    // player 1, an ally, opens both its own and player 2's gates
    t.open_gates(1, &mut open).unwrap();
    assert_eq!(open, [false, true, true, false]);
    assert!(t.spatial.get(27687).unwrap().obstruction.is_none());
    assert!(!blocked(&t));
    // player 2 is next and an ally of both: they stay open
    t.close_gates(1, &mut open);
    assert_eq!(open, [false, true, true, false]);
    t.open_gates(2, &mut open).unwrap();
    // player 3, next, is an enemy: they close
    t.close_gates(2, &mut open);
    assert_eq!(open, [false; 4]);
    assert_eq!(t.spatial.get(27687).unwrap().obstruction, Some(record));
    assert!(blocked(&t));
    t.open_gates(3, &mut open).unwrap();
    assert_eq!(open, [false, false, false, true]);
    assert!(blocked(&t));
    // a locked gate stays closed for its allies
    t.close_gates(3, &mut open);
    t.players[2].objects[0].gate = Some(Gate {
        locked: true,
        ..Gate::default()
    });
    t.open_gates(1, &mut open).unwrap();
    assert!(blocked(&t));
}

/// records/425906646's gate 27745 of player 3 (annexes 27746 and 27747) in update 2865: an
/// ally walking between the posts opens it; a standing Gaia animal does not count, an enemy
/// counts but is no ally, a wall next to it neither.
#[test]
fn a_gate_counts_the_moving_units_between_its_posts() {
    let mut s = Spatial::with_tiles([40, 40], crate::spatial::Tiles::new(40, 40, vec![1; 1600]));
    s.relations = vec![vec![0, 3, 3], vec![3, 0, 3], vec![3, 3, 0]];
    let placed = |x: f32, class, player, speed, kind| Placed {
        position: [x, 20.0, 0.0],
        radius: [0.2, 0.2],
        class,
        player,
        master: 0,
        obstruction_type: kind,
        buildable_on: 0,
        tall: true,
        speed,
        unknown: None,
        obstruction: Some(crate::spatial::Obstruction { kind, class: 2 }),
    };
    s.insert(27745, placed(20.0, 39, 1, 0.0, 2));
    s.insert(27746, placed(18.5, 39, 1, 0.0, 2));
    s.insert(27747, placed(21.5, 39, 1, 0.0, 2));
    s.insert(1, placed(19.6, 4, 1, 0.8, 5));
    let (units, allied) = gate_units(&s, 27745, &[27746, 27747]).unwrap();
    assert_eq!((units, allied), (vec![1], 1));
    s.insert(2, placed(20.4, 10, 0, 0.0, 5));
    s.insert(3, placed(20.8, 6, 2, 0.9, 5));
    s.insert(4, placed(21.0, 27, 1, 0.0, 2));
    let (units, allied) = gate_units(&s, 27745, &[27746, 27747]).unwrap();
    assert_eq!((units, allied), (vec![1, 3], 1));
    assert!(gate_units(&s, 27745, &[27746]).is_err());
}

#[test]
fn the_forced_replay_takes_the_game_s_state_for_what_froze() {
    // a frozen unit takes the game's (and runs again), a frozen unit the game no longer
    // holds goes, a unit that ran keeps the slice's, a stopped player takes its resources
    let mut t = one_player(vec![live(7), live(8), live(9)], [0.0; 4]);
    t.players[0].objects[0].stopped = true;
    t.players[0].objects[2].stopped = true;
    t.players[0].stopped = false;
    let game_unit = |id, timer| Unit {
        id,
        ai: Some(UnitAi {
            retarget_timer: timer,
            ..building()
        }),
        ..Unit::default()
    };
    let game = GameState {
        frame_seed: 77,
        players: vec![vec![game_unit(7, 900), game_unit(8, 901)]],
        resources: vec![[1.0, 2.0, 3.0, 4.0]],
        ..GameState::default()
    };
    assert_eq!(t.force(&game), vec![]);
    assert_eq!(t.frame_seed, 77);
    assert_eq!(ids(&t), vec![7, 8]);
    assert_eq!(retarget(&t), vec![900, 40]);
    assert!(t.players[0].objects.iter().all(|u| !u.stopped));
    // a stopped player runs again with the game's resources; what differed is forced too
    t.players[0].stopped = true;
    let game = GameState {
        resync: vec![8],
        ..game
    };
    let _ = t.force(&game);
    assert_eq!(
        (t.players[0].stopped, t.players[0].resources),
        (false, [1.0, 2.0, 3.0, 4.0])
    );
    assert_eq!(retarget(&t), vec![900, 901]);
    // a game object the slice never made is named, not added
    let made = GameState {
        players: vec![vec![game_unit(7, 900), game_unit(8, 901), game_unit(10, 5)]],
        ..GameState::default()
    };
    assert_eq!(
        t.force(&made),
        vec![(
            0,
            10,
            GenieError::unsupported("an object the game made and the slice did not")
        )]
    );
}

/// The forced replay's removal of a frozen object the game no longer holds is the leave of an
/// object whose update returned 1 ([`Turns::leave_world`]): its owner's watched list lets it go
/// (`FUN_141091310`) and a recyclable master's missile goes to the pool of type 60.
#[test]
fn the_forced_replay_s_removal_leaves_as_an_update_s_leave_does() {
    let missile = Unit {
        id: 9,
        stopped: true,
        missile: Some(crate::missile::Missile {
            max_range: 0.0,
            fired_from: -1,
            charge: false,
            flying: true,
            step: [0.0; 3],
            attack_count: 0,
            action: None,
        }),
        life: Lifecycle {
            master_id: 54,
            ..Lifecycle::default()
        },
        ..Unit::default()
    };
    let mut t = one_player(vec![live(7), missile], [0.0; 4]);
    t.masters = vec![Masters::new([Master {
        id: 54,
        kind: 60,
        recyclable: true,
        ..Master::default()
    }])];
    t.players[0].watched.push(crate::ghost::Watched {
        id: 9,
        cell: crate::ghost::Cell::Tile(0),
        bits: 0,
    });
    let game = GameState {
        players: vec![vec![live(7)]],
        ..GameState::default()
    };
    assert_eq!(t.force(&game), vec![]);
    assert_eq!(ids(&t), vec![7]);
    assert_eq!(t.players[0].watched, Vec::new());
    assert_eq!(t.recycling.missiles, [(9, true)]);
}

/// A player with the villagers' masters (a population of one each) and room for more.
fn one_player(objects: Vec<Unit>, resources: [f32; 4]) -> Turns {
    let mut attributes = vec![0.0; 250];
    (attributes[4], attributes[11], attributes[32]) = (5.0, 4.0, 10.0);
    let mut t = Turns::new(
        0,
        29910,
        0,
        vec![0],
        vec![Player {
            objects,
            resources,
            attributes,
            ..Player::default()
        }],
    );
    // the town centre (109, work rate 1); the villagers (46 food, a population of one, 25
    // seconds at the centre); master 99 without costs or population, and 97 with a population
    // of one, made in 2 seconds
    let trained = |id, time| Master {
        id,
        kind: 70,
        available: true,
        train_locations: vec![crate::masters::TrainLocation {
            location: 109,
            time,
            button: 1,
            hotkey: 0,
        }],
        ..Master::default()
    };
    let villager = |id| Master {
        class: 4,
        cost: vec![(0, 46, true), (4, 1, false), (-1, 0, false)],
        storage: vec![(4, -1.0, 2), (11, 1.0, 2)],
        ..trained(id, 25)
    };
    let centre = Master {
        id: 109,
        kind: 80,
        work_rate: 1.0,
        ..Master::default()
    };
    let quick = Master {
        storage: vec![(4, -1.0, 2), (11, 1.0, 2)],
        ..trained(97, 2)
    };
    t.masters = vec![Masters::new([
        villager(83),
        villager(293),
        trained(99, 25),
        quick,
        centre,
    ])];
    t
}

#[test]
fn a_queue_pays_its_cost_and_the_building_starts_making() {
    let mut t = one_player(vec![centre(9)], [200.0; 4]);
    t.tick_with(104, &[queue(9, 83, 1)]);
    let p = &t.players[0];
    assert_eq!(p.resources, [154.0, 200.0, 200.0, 200.0]);
    let production = p.objects[0].production.as_ref().unwrap();
    // the make action works in the tick it is created in
    let Some(crate::production::Making::Unit(make)) = production.action else {
        panic!("a make action");
    };
    assert_eq!(
        (make.progress, make.state, make.master),
        (0.104, 6, production.queue.slots[0].master)
    );
    let q = &production.queue;
    assert_eq!(
        (q.count, q.total, q.slots[0].train_time, q.slots[0].costs[0]),
        (1, 1, 25, (0, 46.0))
    );
    assert!(p.unsupported.is_none());
}

#[test]
fn a_queued_villager_is_male_or_female_by_an_action_stream_draw() {
    let mut t = one_player(vec![centre(9)], [200.0; 4]);
    t.tick_with(104, &[queue(9, 83, 1)]);
    let mut streams = Streams::default();
    streams.seed_all(29910);
    let r = streams.get(Stream::Action).raw64();
    let expected = if r.wrapping_mul(100) % 100 < 50 {
        83
    } else {
        293
    };
    assert_eq!(
        t.players[0].objects[0]
            .production
            .as_ref()
            .unwrap()
            .queue
            .slots[0]
            .master,
        expected
    );
}

#[test]
fn a_queue_the_player_cannot_pay_or_at_a_missing_building_queues_nothing() {
    // too little food for the villager (46 here): no entry, no cost, no draw (records/
    // 425906646's update 31: player 5's refused QUEUE leaves player 6's draws alone)
    let mut poor = one_player(vec![centre(9)], [40.0, 0.0, 0.0, 0.0]);
    poor.tick_with(104, &[queue(9, 83, 1)]);
    assert!(poor.players[0].unsupported.is_none());
    assert_eq!(poor.players[0].resources[0], 40.0);
    let tc = &poor.players[0].objects[0];
    assert!(
        tc.production
            .as_ref()
            .is_none_or(|p| p.queue.entries().is_empty())
    );
    let mut idle = one_player(vec![centre(9)], [40.0, 0.0, 0.0, 0.0]);
    idle.tick_with(104, &[]);
    assert_eq!(
        poor.streams.get(Stream::Action).draws,
        idle.streams.get(Stream::Action).draws
    );
    // the packet's check drops a building the world lacks and refuses the packet
    let mut missing = one_player(vec![centre(9)], [200.0; 4]);
    missing.tick_with(104, &[queue(9 - 1, 83, 1)]);
    assert!(missing.players[0].unsupported.is_none());
    assert_eq!(missing.players[0].resources, [200.0; 4]);
}

/// `FUN_141614450` over two town centres: unit by unit the one with room and the least load
/// (`FUN_1411d0e90`) takes it, the last pick kept on a tie: five of master 99 (25 seconds each)
/// go to 9, 10, 10, 9, 9.
#[test]
fn a_queue_at_several_buildings_shares_its_units_out_by_load() {
    let several = |amount| Command::Queue {
        player: 0,
        buildings: vec![9, 10],
        unit: 99,
        amount,
        button: -1,
    };
    let total = |t: &Turns, k: usize| {
        t.players[0].objects[k]
            .production
            .as_ref()
            .map_or(0, |p| p.queue.total)
    };
    let mut t = one_player(vec![centre(9), centre(10)], [200.0; 4]);
    t.tick_with(104, &[several(5)]);
    assert_eq!((total(&t, 0), total(&t, 1)), (3, 2));
    // a centre with work queued: 25 seconds less the work done the update gave it
    let mut t = one_player(vec![centre(9), centre(10)], [200.0; 4]);
    t.tick_with(104, &[queue(9, 99, 1)]);
    let Some(crate::production::Making::Unit(make)) = t.players[0].objects[0]
        .production
        .as_ref()
        .and_then(|p| p.action)
    else {
        panic!("a make action");
    };
    assert_eq!(t.production_load(0, 0), 25.0 - make.progress);
    assert_eq!(t.production_load(0, 1), 0.0);
    // the empty one takes the first, then the loaded one the second (25 above its load)
    t.tick_with(104, &[several(2)]);
    assert_eq!((total(&t, 0), total(&t, 1)), (2, 1));
}

/// RESEARCH's building (`0x140e7e270`): with more than one listed, the least loaded of the
/// packet's and the listed ones the world holds; a tie keeps the one taken; one alone is the
/// packet's.
#[test]
fn a_research_at_several_buildings_takes_the_least_loaded() {
    let mut t = one_player(vec![centre(9), centre(10)], [200.0; 4]);
    t.tick_with(104, &[queue(9, 99, 1)]);
    let at = |building, count, buildings: Vec<i32>| ResearchAt {
        building,
        count,
        buildings,
        tech: 0,
        target: -1,
        shift: false,
    };
    assert_eq!(t.least_loaded(0, &at(9, 3, vec![9, 77, 10])), 10);
    assert_eq!(t.least_loaded(0, &at(10, 2, vec![10, 9])), 10);
    assert_eq!(t.least_loaded(0, &at(9, 1, vec![9, 10])), 9);
    assert_eq!(t.least_loaded(0, &at(77, 1, vec![])), -1);
}

/// `FUN_1411cbc40`: a queue adds its units' population to attribute 80 (the queued units),
/// which keeps it until the unit spawns; the make action's start counts the one it makes in
/// 81.
#[test]
fn a_queue_counts_its_units_population_as_queued() {
    let mut t = one_player(vec![centre(9)], [1000.0; 4]);
    t.tick_with(104, &[queue(9, 83, 3)]);
    let a = &t.players[0].attributes;
    assert_eq!((a[80], a[81]), (3.0, 1.0));
    // a unit without population (master 99, no storage of kind 4) adds none
    let mut other = one_player(vec![centre(9)], [1000.0; 4]);
    other.tick_with(104, &[queue(9, 99, 2)]);
    assert_eq!(other.players[0].attributes[80], 0.0);
}

#[test]
fn a_queue_takes_as_many_as_the_room_and_the_resources_allow() {
    // food for two of three: two entries, two draws
    let mut two = one_player(vec![centre(9)], [100.0, 0.0, 0.0, 0.0]);
    two.tick_with(104, &[queue(9, 83, 3)]);
    let mut one = one_player(vec![centre(9)], [100.0, 0.0, 0.0, 0.0]);
    one.tick_with(104, &[queue(9, 83, 1)]);
    let queued = |t: &Turns| {
        t.players[0].objects[0]
            .production
            .as_ref()
            .unwrap()
            .queue
            .total
    };
    assert_eq!(queued(&two), 2);
    assert_eq!(two.players[0].resources[0], 100.0 - 2.0 * 46.0);
    assert_eq!(
        two.streams.get(Stream::Action).draws,
        one.streams.get(Stream::Action).draws + 1
    );
    // 14 queued: room for one of five
    let mut full = one_player(vec![centre(9)], [1000.0; 4]);
    full.tick_with(104, &[queue(9, 99, 14), queue(9, 99, 5)]);
    assert_eq!(queued(&full), 15);
    // a computer player fills to 2
    let mut computer = one_player(vec![centre(9)], [1000.0; 4]);
    computer.players[0].computer = true;
    computer.tick_with(104, &[queue(9, 99, 5)]);
    assert_eq!(queued(&computer), 2);
}

/// `FUN_1411cbc40` joins units of the last entry's master and button whose costs are the same;
/// another button opens an entry with that button's train time; a full buffer of ten grows by
/// ten, the serials numbering the entries.
#[test]
fn queued_units_join_the_last_entry_and_a_full_buffer_grows() {
    let with = |amount, button| Command::Queue {
        player: 0,
        buildings: vec![9],
        unit: 99,
        amount,
        button,
    };
    let mut t = one_player(vec![centre(9)], [1000.0; 4]);
    t.tick_with(104, &[with(2, -1), with(3, -1), with(1, 1)]);
    let q = &t.players[0].objects[0].production.as_ref().unwrap().queue;
    assert_eq!((q.count, q.total, q.changes, q.next_serial), (2, 6, 3, 2));
    assert_eq!(
        (q.slots[0].count, q.slots[1].count, q.slots[1].button),
        (5, 1, 1)
    );
    assert_eq!((q.slots[1].train_time, q.slots[1].serial), (25, 1));
    assert_eq!(t.players[0].tracker.queued, 6);
    let mut t = one_player(vec![centre(9)], [1000.0; 4]);
    let alternating: Vec<_> = (0..11).map(|k| with(1, k % 2)).collect();
    t.tick_with(104, &alternating);
    let q = &t.players[0].objects[0].production.as_ref().unwrap().queue;
    assert_eq!((q.slots.len(), q.count, q.total), (20, 11, 11));
    let serials: Vec<_> = q.entries().iter().map(|e| e.serial).collect();
    assert_eq!(serials, (0..11).collect::<Vec<i64>>());
}

/// `FUN_1411e7dc0`: a computer player's housed building cancels its queue
/// (`FUN_1411cff00`): the villager's 46 food come back and attribute 80 empties; a disabled
/// queue (`+0xbf6` clear) makes nothing.
#[test]
fn a_computer_player_s_housed_building_cancels_its_queue() {
    let mut t = one_player(vec![centre(9)], [1000.0; 4]);
    t.players[0].computer = true;
    t.players[0].attributes[4] = 0.5;
    t.tick_with(104, &[queue(9, 83, 1)]);
    let p = &t.players[0];
    assert_eq!(
        (p.resources[0], p.attributes[80], p.attributes[81]),
        (1000.0, 0.0, 0.0)
    );
    let production = p.objects[0].production.as_ref().unwrap();
    assert_eq!((production.queue.count, production.queue.total), (0, 0));
    assert!(production.action.is_none());
    let mut t = one_player(vec![centre(9)], [1000.0; 4]);
    t.players[0].objects[0].production = Some(crate::production::Production {
        queue: crate::production::Queue {
            enabled: false,
            ..crate::production::Queue::default()
        },
        ..crate::production::Production::default()
    });
    t.tick_with(104, &[queue(9, 83, 1)]);
    let p = &t.players[0];
    assert_eq!((p.attributes[80], p.attributes[81]), (1.0, 0.0));
    assert!(p.objects[0].production.as_ref().unwrap().action.is_none());
}

/// Game mode 16 with attribute 6 at most 1: a military unit (master 99, class 0) is queued
/// one at a time, the player's flag (`+0x1378`) set; a second QUEUE queues nothing.
#[test]
fn game_mode_16_queues_one_military_unit_at_a_time() {
    let mut t = one_player(vec![centre(9)], [1000.0; 4]);
    t.settings.game_mode = 16;
    t.tick_with(104, &[queue(9, 99, 3)]);
    t.tick_with(104, &[queue(9, 99, 2)]);
    let q = &t.players[0].objects[0].production.as_ref().unwrap().queue;
    assert_eq!(q.total, 1);
    assert!(t.players[0].one_military_queued);
}

#[test]
fn an_unmodelled_command_stops_its_player() {
    let mut t = one_player(vec![town_centre(9)], [200.0; 4]);
    let why = GenieError::unsupported("QUEUE at two buildings");
    t.tick_with(
        104,
        &[Command::Unmodelled {
            player: 0,
            why: why.clone(),
        }],
    );
    assert_eq!(t.players[0].unsupported, Some((1, 0, why)));
}

/// The masters of the ownership changes' tests, by player: Gaia's (0), player 1's and
/// player 2's archer 4 (class 0, its dead archer 3 of type 10; line of sight 6, 7, 8; fog
/// visibility 0, 1, 2; hit points 30, 35, 40) and sheep 705 (class 58; line of sight 2, 3,
/// 4; fog visibility 0, 1, 2; food 100); player 2 lacks the dead archer, and the dead
/// archer of player 1 is of type 25 (a doppelganger's, which the copy leaves).
fn hands_masters() -> Vec<Masters> {
    let table = |p: u8| {
        let archer = Master {
            id: 4,
            kind: 70,
            class: 0,
            line_of_sight: 6.0 + f32::from(p),
            fog_visibility: p,
            hit_points: 30 + 5 * i16::from(p),
            death_spawns: [3, -1],
            storage: vec![(4, 1.0, 2), (11, 0.0, 0), (-1, 0.0, 0)],
            ..Master::default()
        };
        let dead = Master {
            id: 3,
            kind: if p == 1 { 25 } else { 10 },
            class: 0,
            hit_points: 9 + i16::from(p),
            ..Master::default()
        };
        let sheep = Master {
            id: 705,
            kind: 70,
            class: LIVESTOCK,
            line_of_sight: 2.0 + f32::from(p),
            fog_visibility: p,
            storage: vec![(16, 100.0, 1)],
            ..Master::default()
        };
        if p == 2 {
            Masters::new([archer, sheep])
        } else {
            Masters::new([archer, dead, sheep])
        }
    };
    vec![table(0), table(1), table(2)]
}

/// A 40 × 40 world of three players' sight with the registry, holding object `id` of
/// player `p`, class `class` and master `master` at (10.5, 10.5) in player `p`'s registry
/// list, its sight on.
fn hands_world(id: i32, p: usize, class: i16, master: i16, los: f32) -> Spatial {
    let n = 40 * 40;
    let mut s = Spatial::new([40, 40]);
    let mut v = crate::visibility::Visibility::new(
        40,
        40,
        (0..3)
            .map(|_| crate::visibility::VisibleMap {
                counts: vec![-1; n],
                explored: 0,
            })
            .collect(),
        vec![0; n],
        Vec::new(),
        0,
        vec![false; 3],
    );
    let mut r = crate::registry::Registry::new(3, 65);
    r.entries.entry(id).or_default().mask = 1 << p;
    r.update_unit(id, p as u8, [10.5, 10.5], [0.2, 0.2], class, 0, 1 << p)
        .unwrap();
    v.registry = Some(r);
    let at = [10.5, 10.5, 0.0];
    v.add_seer(
        id,
        Seer {
            player: p,
            master,
            los,
            state: 2,
            explored: true,
            building: None,
            umv: 0,
            capture: capture_kind(p, class, master),
            kind: 70,
        },
    );
    let _ = v.sight_on(id, at, [0.2, 0.2], false, false);
    s.sight = Some(v);
    s.insert(
        id,
        Placed {
            position: at,
            radius: [0.2, 0.2],
            class,
            player: p,
            master,
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

/// A live unit `id` of master `master` with a unit AI holding a notify.
fn hands_unit(id: i32, master: i16) -> Unit {
    let mut ai = UnitAi::default();
    ai.notifies
        .push(Notify::own(id, crate::unit_ai::DAMAGED, [1, 2, 3]));
    Unit {
        id,
        position: [10.5, 10.5, 0.0],
        life: Lifecycle {
            master_id: master,
            ..Lifecycle::default()
        },
        ai: Some(ai),
        combat: true,
        ..Unit::default()
    }
}

/// `FUN_14120f800` on MarvinBlue/426048681's Gaia archer 21712 (master 4) that player 1
/// takes over: its records and place take player 1, its sight moves from Gaia's map to
/// player 1's with Gaia's line of sight, it takes a master of its own (a copy of Gaia's,
/// with player 1's fog visibility) and a copy of Gaia's dead archer, its unit AI's notifies
/// go with the world time and it tells its unit AI it changed hands.
#[test]
fn a_taken_gaia_archer_keeps_a_copy_of_gaia_s_master_with_its_new_owner_s_fog() {
    let masters = hands_masters();
    let mut s = hands_world(21712, 0, 0, 4, 6.0);
    let mut u = hands_unit(21712, 4);
    assert_eq!(
        change_hands(&mut u, (0, 1), &masters, &mut s, 1250),
        Ok(true)
    );
    let own = u.own_master.as_deref().unwrap();
    assert_eq!(
        (own.hit_points, own.line_of_sight, own.fog_visibility),
        (30, 6.0, 1)
    );
    assert_eq!(u.dead_master.as_deref().map(|d| d.hit_points), Some(9));
    assert_eq!(s.get(21712).map(|p| p.player), Some(1));
    let v = s.sight.as_ref().unwrap();
    let r = v.registry.as_ref().unwrap();
    assert_eq!(r.lists[0][0][0].owner, 1);
    let seer = v.seer(21712).copied().unwrap();
    assert_eq!((seer.player, seer.los), (1, 6.0));
    let tile = |x: usize, y: usize| y * 40 + x;
    assert_eq!(v.maps[0].counts[tile(10, 10)], 0);
    assert_eq!(v.maps[1].counts[tile(16, 10)], 1);
    let ai = u.ai.as_ref().unwrap();
    assert_eq!(ai.last_update_time, 1250);
    assert_eq!(ai.notifies, [Notify::own(21712, OWNER_CHANGED, [1, 0, 0])]);
}

/// `FUN_14120f800` on a sheep: its master of its own takes the new owner's line of sight
/// (class 58), which its sight on the new owner's map reads; its master has no death spawn.
/// A master of its own it had stays (with the new fog visibility), and so does a dead
/// master; a dead master of a type the copy leaves (25) gives none.
#[test]
fn a_taken_sheep_sees_with_its_new_owner_s_line_of_sight() {
    let masters = hands_masters();
    let mut s = hands_world(9520, 1, LIVESTOCK, 705, 3.0);
    let mut u = hands_unit(9520, 705);
    assert_eq!(change_hands(&mut u, (1, 2), &masters, &mut s, 0), Ok(true));
    let own = u.own_master.as_deref().unwrap();
    assert_eq!((own.line_of_sight, own.fog_visibility), (4.0, 2));
    assert_eq!(u.dead_master, None);
    let v = s.sight.as_ref().unwrap();
    assert_eq!(v.seer(9520).map(|s| s.los), Some(4.0));
    assert_eq!(v.maps[2].counts[10 * 40 + 14], 1);
    // back to player 1: the copy stays, with player 1's fog and line of sight
    assert_eq!(change_hands(&mut u, (2, 1), &masters, &mut s, 0), Ok(true));
    let own = u.own_master.as_deref().unwrap();
    assert_eq!((own.line_of_sight, own.fog_visibility), (3.0, 1));
    // player 1's archer: its dead archer is of type 25, and none is copied
    let mut s = hands_world(7, 1, 0, 4, 7.0);
    let mut a = hands_unit(7, 4);
    assert_eq!(change_hands(&mut a, (1, 0), &masters, &mut s, 0), Ok(true));
    assert_eq!(a.dead_master, None);
    // a dead master it holds stays
    let held = Box::new(Master {
        id: 3,
        hit_points: 99,
        ..Master::default()
    });
    a.dead_master = Some(held.clone());
    assert_eq!(change_hands(&mut a, (0, 1), &masters, &mut s, 0), Ok(true));
    assert_eq!(a.dead_master, Some(held));
    assert_eq!(a.own_master.as_deref().map(|m| m.hit_points), Some(35));
}

/// `FUN_14120f800` for a new owner whose table lacks the master writes the new owner into
/// the records and changes nothing else; one inside another (or not in state 2) changes
/// hands without moving its sight.
#[test]
fn an_ownership_change_without_the_master_writes_the_records_alone() {
    let mut masters = hands_masters();
    masters[2] = Masters::new([]);
    let mut s = hands_world(21712, 0, 0, 4, 6.0);
    let before = s.sight.as_ref().unwrap().maps.clone();
    let mut u = hands_unit(21712, 4);
    assert_eq!(change_hands(&mut u, (0, 2), &masters, &mut s, 9), Ok(false));
    let v = s.sight.as_ref().unwrap();
    assert_eq!(v.registry.as_ref().unwrap().lists[0][0][0].owner, 2);
    assert_eq!(v.maps, before);
    assert_eq!(v.seer(21712).map(|s| s.player), Some(0));
    assert_eq!(s.get(21712).map(|p| p.player), Some(0));
    assert_eq!(
        (u.own_master.is_none(), u.dead_master.is_none()),
        (true, true)
    );
    assert_eq!(u.ai.as_ref().unwrap().notifies.len(), 1);
    u.life.inside = Some(100);
    assert_eq!(change_hands(&mut u, (0, 1), &masters, &mut s, 9), Ok(true));
    let v = s.sight.as_ref().unwrap();
    assert_eq!(v.maps, before);
    assert_eq!(v.seer(21712).map(|s| (s.player, s.los)), Some((1, 6.0)));
    assert_eq!(s.get(21712).map(|p| p.player), Some(1));
}

/// `RGE_Combat_Object::update` on MarvinBlue/426048681's Gaia archer 21712 in update 1: its
/// mask now holds player 1's bit, so player 1 takes it over (slot 0x70) and `+0x924` goes 0;
/// it leaves Gaia's list for the end of player 1's, its class count with it (attribute 40).
/// A player whose table lacks the master ends the loop without taking it; outside the
/// standard modes a computer player that does not allow it is passed over.
#[test]
fn a_gaia_archer_a_player_sees_goes_to_that_player() {
    let world = |masters: Vec<Masters>, computer: bool, mode: i32| {
        let mut t = Turns::new(
            0,
            1,
            0,
            vec![0, 0, 0],
            vec![
                Player {
                    objects: vec![Unit {
                        ai: None,
                        capture_flag: 1,
                        ..hands_unit(21712, 4)
                    }],
                    attributes: vec![0.0; 300],
                    ..Player::default()
                },
                Player {
                    objects: vec![live(500)],
                    attributes: vec![0.0; 300],
                    computer,
                    kind: if computer { 3 } else { 1 },
                    ..Player::default()
                },
                Player {
                    attributes: vec![0.0; 300],
                    ..Player::default()
                },
            ],
        );
        t.settings.game_mode = mode;
        t.players[0].attributes[40] = 1.0;
        t.masters = masters;
        t.spatial = hands_world(21712, 0, 0, 4, 6.0);
        let v = t.spatial.sight.as_mut().unwrap();
        v.unified[10 * 40 + 10] = 0b111;
        t
    };
    let mut t = world(hands_masters(), false, 0);
    t.tick(100);
    assert_eq!(t.players[0].objects.len(), 0);
    let ids: Vec<i32> = t.players[1].objects.iter().map(|u| u.id).collect();
    assert_eq!(ids, [500, 21712]);
    let archer = &t.players[1].objects[1];
    assert_eq!(archer.capture_flag, 0);
    assert_eq!(
        archer.own_master.as_deref().map(|m| m.fog_visibility),
        Some(1)
    );
    let v = t.spatial.sight.as_ref().unwrap();
    assert_eq!(v.seer(21712).map(|s| (s.player, s.capture)), Some((1, 0)));
    assert_eq!(
        [t.players[0].attributes[40], t.players[1].attributes[40]],
        [0.0, 1.0]
    );
    // player 1 lacks the master: the loop ends with it, and Gaia keeps the archer
    let mut masters = hands_masters();
    masters[1] = Masters::new([]);
    let mut t = world(masters, false, 0);
    t.tick(100);
    assert_eq!(t.players[0].objects[0].capture_flag, 1);
    assert_eq!(t.spatial.get(21712).map(|p| p.player), Some(0));
    // a computer player 1 in a scenario passes it to player 2
    let mut t = world(hands_masters(), true, 3);
    t.tick(100);
    assert_eq!(t.players[2].objects.len(), 1);
    assert_eq!(t.spatial.get(21712).map(|p| p.player), Some(2));
    // unless it allows taking Gaia's units
    let mut t = world(hands_masters(), true, 3);
    t.allow_gaia_capture = vec![false, true, false];
    t.tick(100);
    assert_eq!(t.spatial.get(21712).map(|p| p.player), Some(1));
}

/// A world for the teardowns: player 1 holds the sheep 20 (master 705), the bird 30 (96),
/// the villager 40 (83) and town centres 9149 and 9150 (109, one worker each); player 2 (an
/// ally) holds the foundation 10028 (70) with one builder.
fn teardown_world() -> Turns {
    let walker = crate::hunt::tests::walker();
    let unit = |id, master| Unit {
        id,
        position: [10.5, 10.5, 0.0],
        mover: Some(walker.clone()),
        life: Lifecycle {
            master_id: master,
            ..Lifecycle::default()
        },
        ..Unit::default()
    };
    let tc = |id| Unit {
        id,
        position: [20.0, 20.0, 0.0],
        workers: 1,
        combat: true,
        life: Lifecycle {
            master_id: 109,
            ..Lifecycle::default()
        },
        ..Unit::default()
    };
    let foundation = Unit {
        id: 10028,
        position: [30.0, 30.0, 0.0],
        workers: 1,
        life: Lifecycle {
            state: 0,
            master_id: 70,
            ..Lifecycle::default()
        },
        construction: Some(Construction {
            progress: 0.0,
            desolid: false,
            builders: 1,
            hit_points: 550,
            build_time: 25,
            record: crate::spatial::Obstruction { kind: 2, class: 2 },
            snow: false,
            sprite_override: false,
            started: true,
        }),
        ..Unit::default()
    };
    let mut t = Turns::new(
        0,
        1,
        0,
        vec![0, 0, 0],
        vec![
            Player::default(),
            Player {
                objects: vec![
                    unit(20, 705),
                    unit(30, 96),
                    unit(40, 83),
                    tc(9149),
                    tc(9150),
                ],
                relations: vec![0, 1, 2],
                ..Player::default()
            },
            Player {
                objects: vec![foundation],
                relations: vec![0, 2, 1],
                ..Player::default()
            },
        ],
    );
    let master = |id, kind, class| Master {
        id,
        kind,
        class,
        ..Master::default()
    };
    let table = Masters::new([
        master(705, 70, LIVESTOCK),
        master(96, 70, 11),
        master(83, 70, 4),
        master(109, 80, 52),
        master(70, 80, 3),
    ]);
    t.masters = vec![table.clone(), table.clone(), table];
    t
}

/// The action list's clearing (`FUN_140a98620`): the Artifact's teardown (`FUN_140aab0d0`)
/// lets its town centre go and its `MoveTo`'s stops the sheep; the Fly's (`FUN_140a84750`)
/// gives the bird its top speed as its animation speed, then its `MoveTo`'s stops it; the
/// Enter's lets its building go; a counted builder's Build leaves another player's
/// foundation's builders (`Turns::leave_builders`) and lets it go; a wait for a foundation
/// goes.
#[test]
fn clearing_the_actions_tears_the_artifact_fly_enter_and_build_down() {
    let mut t = teardown_world();
    t.players[1].objects[0].artifact = Some(Artifact {
        target: 9149,
        goal: Some([20.0, 20.0, 0.0]),
        walk_to: Some((crate::move_to::Goal::Target(9149), 1.0)),
        move_to: Some(MoveTo::Moving),
        ..Artifact::new(false)
    });
    t.clear_actions(1, 0).unwrap();
    let sheep = &t.players[1].objects[0];
    assert_eq!(sheep.artifact, None);
    assert!(sheep.mover.as_ref().unwrap().speed_stopped);
    assert_eq!(t.players[1].objects[3].workers, 0);
    let bird = &mut t.players[1].objects[1];
    bird.fly = Some(Fly {
        state: 3,
        timer: 1.0,
        move_to: Some(MoveTo::Moving),
        goal: Some([12.0, 12.0, 0.0]),
        home: [10.0, 10.0],
        task: crate::masters::Task::default(),
    });
    bird.mover.as_mut().unwrap().stats.speed = 1.3;
    t.clear_actions(1, 1).unwrap();
    let bird = &t.players[1].objects[1];
    assert_eq!((bird.fly, bird.anim_speed), (None, Some(1.3)));
    assert!(bird.mover.as_ref().unwrap().speed_stopped);
    let villager = &mut t.players[1].objects[2];
    villager.enter = Some(crate::garrison::EnterAction {
        target: 9150,
        state: 3,
        move_to: None,
        point: [20.0, 20.0, 0.0],
        range: 0.0,
        timer: 0.0,
        retry: true,
        task_class: 0,
    });
    villager.wait_solidify = Some(crate::wait_solidify::WaitSolidify::new(
        crate::masters::Task::default(),
        10028,
    ));
    t.clear_actions(1, 2).unwrap();
    let villager = &t.players[1].objects[2];
    assert_eq!((villager.enter, villager.wait_solidify), (None, None));
    assert!(!villager.mover.as_ref().unwrap().speed_stopped);
    assert_eq!(t.players[1].objects[4].workers, 0);
    t.players[1].objects[2].build = Some(BuildAction {
        target: 10028,
        state: 3,
        move_to: None,
        goal: None,
        task: BuildTask {
            range: 0.0,
            work_value: 0.0,
            work_rate: 1.0,
        },
        added: true,
        timer: 0.0,
    });
    t.clear_actions(1, 2).unwrap();
    let foundation = &t.players[2].objects[0];
    assert_eq!(foundation.workers, 0);
    assert_eq!(foundation.construction.map(|k| k.builders), Some(0));
}

/// A world for the deaths: Gaia holds the tree 50 (master 349, type 15, one worker) and the boar
/// 60 (master 48, type 70, one worker, a unit AI that counts the hunter 41 among its attackers);
/// player 1 holds the villagers 40 (gathering the tree), 41 (hunting the boar) and 42 (a counted
/// builder of its foundation 10028, one builder, one worker), each with hit points 0.
fn deaths_world() -> Turns {
    let dead = |id| Unit {
        id,
        position: [10.5, 10.5, 0.0],
        life: Lifecycle {
            class: Class::Animated,
            hp: 0.0,
            master_id: 83,
            master: Some(MasterData::default()),
            ..Lifecycle::default()
        },
        pasture: -1,
        my_pasture: -1,
        ..Unit::default()
    };
    let chop = crate::hunt::tests::hunting(50).task;
    let tree = crate::gather::Resource {
        id: 50,
        position: [12.5, 10.5, 0.0],
        class: 15,
        master: 349,
        player: 0,
        workers: 0,
        drop_site: false,
        gap: 1.0,
        radius: [0.5, 0.5],
    };
    let rate = crate::gather::StopRate {
        work_rate: 1.0,
        capacity: 10.0,
        productivity: 1.0,
    };
    let gather = crate::gather::GatherAction::new(chop, &tree, false, rate).unwrap();
    let mut villagers = vec![dead(40), dead(41), dead(42)];
    villagers[0].gather = Some(crate::gather::GatherAction { state: 7, ..gather });
    villagers[1].hunt = Some(crate::hunt::tests::hunting(60));
    villagers[2].build = Some(BuildAction {
        target: 10028,
        state: 7,
        move_to: None,
        goal: None,
        task: BuildTask {
            range: 0.0,
            work_value: 1.0,
            work_rate: 1.0,
        },
        added: true,
        timer: 0.0,
    });
    let gaia = vec![
        Unit {
            id: 50,
            position: [12.5, 10.5, 0.0],
            workers: 1,
            life: Lifecycle {
                master_id: 349,
                ..Lifecycle::default()
            },
            ..Unit::default()
        },
        Unit {
            id: 60,
            position: [14.5, 10.5, 0.0],
            workers: 1,
            combat: true,
            ai: Some(UnitAi {
                attackers: vec![41],
                ..building()
            }),
            life: Lifecycle {
                class: Class::Animated,
                master_id: 48,
                ..Lifecycle::default()
            },
            ..Unit::default()
        },
    ];
    let foundation = Unit {
        id: 10028,
        position: [30.0, 30.0, 0.0],
        workers: 1,
        life: Lifecycle {
            state: 0,
            master_id: 70,
            ..Lifecycle::default()
        },
        construction: Some(Construction {
            progress: 0.0,
            desolid: false,
            builders: 1,
            hit_points: 550,
            build_time: 25,
            record: crate::spatial::Obstruction { kind: 2, class: 2 },
            snow: false,
            sprite_override: false,
            started: true,
        }),
        ..Unit::default()
    };
    // player 1's turn: the boar's unit AI keeps its attackers
    let mut t = Turns::new(
        0,
        1,
        1,
        vec![0, 0],
        vec![
            Player {
                objects: gaia,
                ..Player::default()
            },
            Player {
                objects: villagers.into_iter().chain([foundation]).collect(),
                relations: vec![0, 1],
                attributes: vec![0.0; 250],
                ..Player::default()
            },
        ],
    );
    // no dying graphic (-1): slot 0x7a0 gives the master's, which these masters lack
    let master = |id, kind, class| Master {
        id,
        kind,
        class,
        death_sprite: -1,
        ..Master::default()
    };
    let table = Masters::new([
        master(349, 15, 15),
        master(48, 70, 9),
        master(83, 70, 4),
        master(70, 80, 3),
    ]);
    t.masters = vec![table.clone(), table];
    t
}

/// `RGE_Action_Object::update` (`0x14106ac90`) deletes a dead unit's action list in the update
/// of its death (`FUN_140a98620`), before any action's update: the Gather lets its tree go (no
/// worker left, it goes to sleep), the Hunt lets its boar go (notify 699 from the hunter, its
/// class 4), the Build takes its builder out of the foundation's builders and lets it go.
#[test]
fn a_dead_unit_s_actions_go_and_let_their_targets_go_in_the_update_of_its_death() {
    let mut t = deaths_world();
    t.tick(100);
    assert_eq!(
        (
            t.players[0].unsupported.clone(),
            t.players[1].unsupported.clone()
        ),
        (None, None)
    );
    let villagers = &t.players[1].objects;
    for v in &villagers[..3] {
        assert!(5 <= v.life.state, "{} died", v.id);
        assert_eq!(v.top_action(), None, "{} lost its actions", v.id);
    }
    let tree = &t.players[0].objects[0];
    assert_eq!((tree.workers, tree.life.goto_sleep), (0, true));
    let boar = &t.players[0].objects[1];
    assert_eq!(boar.workers, 0);
    let notify = boar.ai.as_ref().unwrap().notifies.last().copied();
    assert_eq!(
        notify.map(|n| (n.caller, n.kind, n.p[0])),
        Some((41, crate::unit_ai::NO_LONGER_ON_ME, 4))
    );
    let foundation = &villagers[3];
    assert_eq!(foundation.workers, 0);
    assert_eq!(foundation.construction.map(|k| k.builders), Some(0));
}

/// `FUN_140aaddb0`: a Build counts its builder in a building annex's main building (`+0xbb8`),
/// in any other object itself, and in none once the world no longer holds its target; a dead
/// builder of the annex leaves the main building's builders.
#[test]
fn a_build_on_an_annex_counts_its_builder_in_the_main_building() {
    let mut t = deaths_world();
    let mut main = t.players[1].objects[3].clone();
    main.id = 10030;
    t.players[1].objects[3].annex_of = Some(10030);
    t.players[1].objects[3]
        .construction
        .as_mut()
        .unwrap()
        .builders = 0;
    t.players[1].objects.push(main);
    assert_eq!(t.counted_building(10028), Ok(Some(10030)));
    assert_eq!(t.counted_building(50), Ok(Some(50)));
    assert_eq!(t.counted_building(999), Ok(None));
    t.tick(100);
    assert_eq!(
        (
            t.players[0].unsupported.clone(),
            t.players[1].unsupported.clone()
        ),
        (None, None)
    );
    let builders = |id| {
        t.players[1]
            .objects
            .iter()
            .find(|u| u.id == id)
            .and_then(|u| u.construction)
            .map(|k| k.builders)
    };
    assert_eq!((builders(10028), builders(10030)), (Some(0), Some(0)));
}

/// A `MoveTo` of its own to an object takes the object (`FUN_140aa0df0` with quiet 0, then
/// `FUN_140a7a230`: slot 0x178 wakes the sleeping tree 50) and lets it go at every end: another
/// move to an object takes the new one before the clearing's teardown (`0x140aa1f30`, then the
/// base teardown `FUN_140a73fc0`) lets the old one go (no worker left on the tree: it is flagged
/// to sleep), and a clearing of the list lets the town centre go.
#[test]
fn a_move_to_an_object_takes_it_and_lets_it_go_when_it_is_replaced_or_cleared() {
    let mut t = teardown_world();
    let tree = Unit {
        id: 50,
        position: [12.5, 10.5, 0.0],
        life: Lifecycle {
            master_id: 349,
            ..Lifecycle::default()
        },
        ..Unit::default()
    };
    t.players[0].sleeping.push(tree);
    for m in &mut t.masters {
        *m = Masters::new(m.iter().cloned().chain([Master {
            id: 349,
            kind: 15,
            class: 15,
            ..Master::default()
        }]));
    }
    let placed = |x: f32, class, master| Placed {
        position: [x, 10.5, 0.0],
        radius: [0.5, 0.5],
        class,
        player: 0,
        master,
        obstruction_type: 0,
        buildable_on: 0,
        tall: true,
        speed: 0.0,
        unknown: None,
        obstruction: None,
    };
    t.spatial.insert(50, placed(12.5, 15, 349));
    t.spatial.insert(9149, placed(20.0, 52, 109));
    t.move_to_object(1, 2, 50, [12.5, 10.5, 0.0]).unwrap();
    assert_eq!(t.players[0].sleeping.len(), 0);
    let tree = &t.players[0].objects[0];
    assert_eq!((tree.id, tree.workers), (50, 1));
    assert_eq!(t.players[1].objects[2].move_to.map(|m| m.target), Some(50));
    t.move_to_object(1, 2, 9149, [20.0, 10.5, 0.0]).unwrap();
    let tree = &t.players[0].objects[0];
    assert_eq!((tree.workers, tree.life.goto_sleep), (0, true));
    assert_eq!(t.players[1].objects[3].workers, 2);
    t.clear_actions(1, 2).unwrap();
    assert_eq!(t.players[1].objects[3].workers, 1);
}

/// A `MoveTo` of its own that ends (its update in state 1 returns 1, the action list removes it,
/// `FUN_140a98e20`) lets its target go (the base teardown, slot 0x180); one that dies with the
/// unit does too (`FUN_140a98620`).
#[test]
fn a_move_to_an_object_lets_it_go_when_it_ends_or_its_unit_dies() {
    let mut t = deaths_world();
    t.players[0].objects[0].workers = 2;
    let mover = crate::hunt::tests::walker();
    let walker = |id| Unit {
        id,
        position: [11.5, 10.5, 0.0],
        mover: Some(mover.clone()),
        move_to: Some(crate::move_to::MoveToAction {
            state: MoveTo::Arrived,
            goal: None,
            target: 50,
            range: 0.0,
            run: false,
            wait: crate::move_to::MoveWait::default(),
        }),
        pasture: -1,
        my_pasture: -1,
        life: Lifecycle {
            class: Class::Animated,
            master_id: 83,
            master: Some(MasterData::default()),
            ..Lifecycle::default()
        },
        ..Unit::default()
    };
    let dead = Unit {
        life: Lifecycle {
            hp: 0.0,
            ..walker(43).life
        },
        ..walker(43)
    };
    t.players[1].objects = vec![walker(44), dead];
    t.spatial.insert(
        50,
        Placed {
            position: [12.5, 10.5, 0.0],
            radius: [0.5, 0.5],
            class: 15,
            player: 0,
            master: 349,
            obstruction_type: 0,
            buildable_on: 0,
            tall: true,
            speed: 0.0,
            unknown: None,
            obstruction: None,
        },
    );
    t.tick(100);
    assert_eq!(
        (
            t.players[0].unsupported.clone(),
            t.players[1].unsupported.clone()
        ),
        (None, None)
    );
    assert_eq!(t.players[1].objects[0].move_to, None);
    assert_eq!(t.players[1].objects[1].move_to, None);
    let tree = &t.players[0].objects[0];
    assert_eq!((tree.workers, tree.life.goto_sleep), (0, true));
}

/// The hunt's `set_state(8)` (`0x140ac60e0` case 8): without a second target its search
/// (`FUN_1411671e0`; without planners by distance) finds the town centre, which becomes the
/// second target (`FUN_140a7a190`): the caller takes it (slot 0x178). With the drop site held
/// already it walks there and takes nothing new.
#[test]
fn a_hunter_s_drop_off_takes_the_drop_site_it_found() {
    let mut t = Turns::new(
        0,
        1,
        0,
        vec![0, 0],
        vec![Player::default(), Player::default()],
    );
    placeable(&mut t);
    let mut walker = crate::hunt::tests::walker();
    walker.stats.restriction = 0;
    let mut hunter = Unit {
        id: 40,
        position: [10.5, 10.5, 0.0],
        mover: Some(walker),
        hunt: Some(crate::hunt::tests::hunting(-1)),
        life: Lifecycle {
            master_id: 83,
            attribute_type: 0,
            amount: 10.0,
            ..Lifecycle::default()
        },
        ..Unit::default()
    };
    let tc = Unit {
        id: 9149,
        position: [20.0, 20.0, 0.0],
        life: Lifecycle {
            master_id: 109,
            ..Lifecycle::default()
        },
        ..Unit::default()
    };
    let masters = Masters::new([
        Master {
            id: 83,
            kind: 70,
            class: 4,
            radius: [0.2, 0.2],
            drop_sites: vec![109, -1],
            ..Master::default()
        },
        Master {
            id: 109,
            kind: 80,
            class: 52,
            radius: [2.0, 2.0],
            ..Master::default()
        },
    ]);
    let placed = |x: f32, radius: f32, class, master| Placed {
        position: [x, x, 0.0],
        radius: [radius, radius],
        class,
        player: 1,
        master,
        obstruction_type: 0,
        buildable_on: 0,
        tall: true,
        speed: 0.0,
        unknown: None,
        obstruction: None,
    };
    t.spatial.insert(40, placed(10.5, 0.2, 4, 83));
    t.spatial.insert(9149, placed(20.0, 2.0, 52, 109));
    let others = [tc];
    let drop_off = |hunter: &mut Unit, t: &mut Turns| {
        let world = DropWorld {
            masters: Some(&masters),
            spatial: &t.spatial,
            zones: &mut t.zones,
            pathing: &mut t.pathing,
            object: t.streams.get(Stream::Object),
            world_time: 0,
            computer: false,
            construction_rate: 1.0,
            groups: &t.groups,
        };
        hunter.drop_off_in(world, &others)
    };
    assert_eq!(drop_off(&mut hunter, &mut t), Ok(Some(9149)));
    let hunt = hunter.hunt.unwrap();
    assert_eq!((hunt.state, hunt.carry_to), (crate::hunt::DROP, 9149));
    assert_eq!(drop_off(&mut hunter, &mut t), Ok(None));
}

/// The Hunt's update (`0x140ac7a00`) first checks its second target: a drop site past state 6
/// is let go (`FUN_140a7a190` with none: slot 0x180) and dropped, one the world's table no longer
/// holds only dropped; the teardown (`FUN_140a73fc0`) of a dead hunter's Hunt and of a failed
/// one (state 13, removed by `FUN_140a98e20`) lets its drop site go too.
#[test]
fn a_hunt_lets_its_drop_site_go_when_it_dies_or_the_hunter_does() {
    let mut t = deaths_world();
    let site = |state| Unit {
        id: 9149,
        position: [20.0, 20.0, 0.0],
        workers: 1,
        life: Lifecycle {
            state,
            master_id: 70,
            ..Lifecycle::default()
        },
        ..Unit::default()
    };
    t.players[1].objects.push(site(2));
    // the dead hunter 41's Hunt walks its load to the town centre
    t.players[1].objects[1].hunt.as_mut().unwrap().carry_to = 9149;
    t.tick(100);
    assert_eq!(
        (
            t.players[0].unsupported.clone(),
            t.players[1].unsupported.clone()
        ),
        (None, None)
    );
    let workers = |t: &Turns| {
        t.players[1]
            .objects
            .iter()
            .find(|u| u.id == 9149)
            .unwrap()
            .workers
    };
    assert_eq!(workers(&t), 0);
    // a live hunter waiting to search (state 3) whose drop site is past state 6 (the world's
    // table holds it still); one whose drop site the table no longer holds only drops it
    let mut t = deaths_world();
    t.players[1].objects.push(site(7));
    t.spatial.insert(
        9149,
        Placed {
            position: [20.0, 20.0, 0.0],
            radius: [2.0, 2.0],
            class: 3,
            player: 1,
            master: 70,
            obstruction_type: 2,
            buildable_on: 0,
            tall: true,
            speed: 0.0,
            unknown: None,
            obstruction: None,
        },
    );
    let hunter = &mut t.players[1].objects[1];
    hunter.life.hp = 1.0;
    hunter.hunt = Some(crate::hunt::HuntAction {
        state: crate::hunt::SEARCH,
        target: -1,
        carry_to: 9149,
        ..crate::hunt::tests::hunting(-1)
    });
    let mut gone = t.clone();
    gone.spatial = Spatial::default();
    t.tick(100);
    assert_eq!(
        (
            t.players[0].unsupported.clone(),
            t.players[1].unsupported.clone()
        ),
        (None, None)
    );
    assert_eq!(workers(&t), 0);
    assert_eq!(t.players[1].objects[1].hunt.map(|h| h.carry_to), Some(-1));
    gone.tick(100);
    assert_eq!(workers(&gone), 1);
    assert_eq!(
        gone.players[1].objects[1].hunt.map(|h| h.carry_to),
        Some(-1)
    );
    // a failed hunt (state 13) is removed: its teardown lets the drop site go too
    let mut failed = deaths_world();
    failed.players[1].objects.push(site(2));
    failed.spatial = t.spatial.clone();
    let hunter = &mut failed.players[1].objects[1];
    hunter.life.hp = 1.0;
    hunter.hunt = Some(crate::hunt::HuntAction {
        state: crate::hunt::FAILED,
        target: -1,
        carry_to: 9149,
        ..crate::hunt::tests::hunting(-1)
    });
    failed.tick(100);
    assert_eq!(failed.players[1].unsupported, None);
    assert_eq!(workers(&failed), 0);
    assert_eq!(failed.players[1].objects[1].hunt, None);
}

/// A Gather's and a Hunt's second target at the start of their updates (`0x140a8e1d0`,
/// `0x140ac7a00`): a drop site past state 6 is let go and goes, one the world's table lacks only
/// goes, one in state 2 (or 6) stays.
#[test]
fn a_second_target_past_state_6_is_let_go_and_one_gone_dropped() {
    use crate::turn::unit::second_target_checked;
    assert_eq!(second_target_checked(9149, true, Some(7)), (-1, Some(9149)));
    assert_eq!(second_target_checked(9149, true, Some(6)), (9149, None));
    assert_eq!(second_target_checked(9149, true, Some(2)), (9149, None));
    assert_eq!(second_target_checked(9149, false, None), (-1, None));
    assert_eq!(second_target_checked(-1, false, None), (-1, None));
}

/// A foundation's attacking units (`+0xce0`): slot 0x178 of a building (`FUN_1411dba70`) below
/// state 2 whose master has a stack unit (`+0x986`) lists each taker of a player its owner is
/// not allied with, once per take; slot 0x180 (`FUN_1411d6020`) drops the releaser's first entry,
/// the others keeping their order. An ally's take, a built building's and one of a master
/// without a stack unit list nothing. Both still count the workers.
#[test]
fn a_foundation_lists_its_unallied_takers_and_drops_them_when_they_let_it_go() {
    let unit = |id, master| Unit {
        id,
        life: Lifecycle {
            master_id: master,
            ..Lifecycle::default()
        },
        ..Unit::default()
    };
    let foundation = |id, master| Unit {
        life: Lifecycle {
            state: 0,
            ..unit(id, master).life
        },
        combat: true,
        ..unit(id, master)
    };
    let mut t = Turns::new(
        0,
        1,
        0,
        vec![0, 0, 0, 0],
        vec![
            Player::default(),
            Player {
                objects: vec![foundation(10028, 621), foundation(10029, 70)],
                relations: vec![0, 0, 2, 4],
                ..Player::default()
            },
            Player {
                objects: vec![unit(40, 83), unit(41, 83)],
                relations: vec![0, 2, 0, 2],
                ..Player::default()
            },
            Player {
                objects: vec![unit(50, 83)],
                relations: vec![0, 4, 2, 0],
                ..Player::default()
            },
        ],
    );
    let building = |id, stack_unit| Master {
        id,
        kind: 80,
        class: 3,
        building: Some(crate::masters::BuildingMaster {
            stack_unit,
            ..crate::masters::BuildingMaster::default()
        }),
        ..Master::default()
    };
    let table = Masters::new([
        building(621, 109),
        building(70, -1),
        Master {
            id: 83,
            kind: 70,
            class: 4,
            ..Master::default()
        },
    ]);
    t.masters = vec![table.clone(), table.clone(), table.clone(), table];
    let by = |t: &Turns, p, i| t.releaser(p, i);
    let (enemy, other, ally) = (by(&t, 2, 0), by(&t, 2, 1), by(&t, 3, 0));
    for taker in [enemy, other, enemy, ally] {
        t.take(10028, taker);
        t.take(10029, taker);
    }
    let site = |t: &Turns| t.players[1].objects[0].clone();
    assert_eq!(site(&t).attacking_units, [40, 41, 40]);
    assert_eq!(site(&t).workers, 4);
    assert_eq!(t.players[1].objects[1].attacking_units, Vec::<i32>::new());
    t.release(10028, enemy).unwrap();
    assert_eq!(
        (site(&t).attacking_units, site(&t).workers),
        (vec![41, 40], 3)
    );
    t.release(10028, ally).unwrap();
    assert_eq!(site(&t).attacking_units, [41, 40]);
    t.players[1].objects[0].life.state = 2;
    t.take(10028, other);
    t.release(10028, enemy).unwrap();
    assert_eq!(site(&t).attacking_units, [41, 40]);
}

/// Slot 0x180 by class: the static object's (`FUN_141197d00`, types 10 and 15) flags an object
/// alive without workers left to sleep (not one of NaN hit points: `COMISS`, `JNC`); the animated
/// object's (`FUN_141071ea0`, types 20 to 40) only counts the worker less. A sleeping object
/// (`FUN_1415417a0` finds it) counts one less and stays asleep without the flag.
#[test]
fn only_a_static_object_let_go_by_its_last_worker_goes_to_sleep() {
    let by = Releaser {
        id: 40,
        class: 4,
        player: 1,
    };
    let let_go = |kind, hp| {
        let mut u = Unit {
            workers: 1,
            life: Lifecycle {
                hp,
                ..Lifecycle::default()
            },
            ..Unit::default()
        };
        released(&mut u, kind, false, by).unwrap();
        (u.workers, u.life.goto_sleep)
    };
    assert_eq!(let_go(10, 1.0), (0, true));
    assert_eq!(let_go(15, 1.0), (0, true));
    assert_eq!(let_go(20, 1.0), (0, false));
    assert_eq!(let_go(25, 1.0), (0, false));
    assert_eq!(let_go(10, f32::NAN), (0, false));
    let mut t = deaths_world();
    let mut tree = t.players[0].objects.remove(0);
    tree.workers = 2;
    t.players[0].sleeping.push(tree);
    t.release(50, by).unwrap();
    let tree = &t.players[0].sleeping[0];
    assert_eq!((tree.workers, tree.life.goto_sleep), (1, false));
}

/// The Wonder (master 276) keeps its action list up to state 5 (`0x14106ac90`'s threshold 5,
/// any other object's 2).
#[test]
fn only_the_wonder_keeps_its_actions_past_state_2() {
    let life = |master_id, state| Lifecycle {
        state,
        master_id,
        ..Lifecycle::default()
    };
    assert!(!loses_actions(&life(83, 2)));
    assert!(loses_actions(&life(83, 3)));
    assert!(!loses_actions(&life(276, 5)));
    assert!(loses_actions(&life(276, 6)));
}

/// The Artifact's move to an object (slot 8 with an object, `FUN_140aaca30`): the old target
/// is let go, then the new one taken (slot 0x68 twice, `+0x60` 0).
#[test]
fn an_artifact_s_move_to_an_object_lets_the_old_one_go_and_takes_the_new() {
    let mut t = teardown_world();
    for (id, at) in [(9149, [20.0, 20.0]), (9150, [25.0, 20.0])] {
        t.spatial.insert(
            id,
            Placed {
                position: [at[0], at[1], 0.0],
                radius: [2.0, 2.0],
                class: 52,
                player: 1,
                master: 109,
                obstruction_type: 2,
                buildable_on: 0,
                tall: true,
                speed: 0.0,
                unknown: None,
                obstruction: None,
            },
        );
    }
    t.players[1].objects[0].artifact = Some(Artifact::new(false));
    t.artifact_take_object(1, 0, 9149).unwrap();
    assert_eq!(t.players[1].objects[3].workers, 2);
    t.artifact_take_object(1, 0, 9150).unwrap();
    assert_eq!(
        [
            t.players[1].objects[3].workers,
            t.players[1].objects[4].workers
        ],
        [1, 2]
    );
    let a = t.players[1].objects[0].artifact.unwrap();
    assert_eq!((a.target, a.goal), (9150, Some([25.0, 20.0, 0.0])));
    assert!(
        t.players[1].objects[0]
            .mover
            .as_ref()
            .unwrap()
            .speed_stopped
    );
}

/// The artifact's unit AI in a tick, with Gaia's torch 7 (master 854, class 1, a combat
/// unit): on Gaia's turn its processMisc (`FUN_140c21d40`) finds player 1's villager 500,
/// the one player's object in its lists, and asks its unit to change hands (slot 0x70); the
/// torch leaves Gaia's list for the end of player 1's.
#[test]
fn a_gaia_torch_goes_to_the_one_player_near_it() {
    let master = |id, class, los| Master {
        id,
        kind: 70,
        class,
        line_of_sight: los,
        radius: [0.2, 0.2],
        ..Master::default()
    };
    let table = Masters::new([master(854, 1, 4.0), master(83, 4, 4.0)]);
    let torch = Unit {
        ai: Some(UnitAi {
            ai_type: crate::unit_ai::ARTIFACT_AI,
            current_order: -1,
            current_action: -1,
            current_target: -1,
            idle_timeout: 5000,
            adjusted_idle_timeout: 4000,
            retarget_timer: 1000,
            ..UnitAi::default()
        }),
        ..hands_unit(7, 854)
    };
    let villager = Unit {
        position: [11.5, 10.5, 0.0],
        ai: None,
        ..hands_unit(500, 83)
    };
    let mut t = Turns::new(
        0,
        1,
        0,
        vec![0, 0, 0],
        vec![
            Player {
                objects: vec![torch],
                attributes: vec![0.0; 300],
                ..Player::default()
            },
            Player {
                objects: vec![villager],
                attributes: vec![0.0; 300],
                ..Player::default()
            },
            Player {
                attributes: vec![0.0; 300],
                ..Player::default()
            },
        ],
    );
    t.masters = vec![table.clone(), table.clone(), table];
    t.spatial = hands_world(7, 0, 1, 854, 4.0);
    t.spatial.unit_diplomacy = vec![vec![1, 4, 4], vec![4, 1, 4], vec![4, 4, 1]];
    t.spatial.insert(
        500,
        Placed {
            position: [11.5, 10.5, 0.0],
            radius: [0.2, 0.2],
            class: 4,
            player: 1,
            master: 83,
            obstruction_type: 0,
            buildable_on: 0,
            tall: true,
            speed: 0.0,
            unknown: None,
            obstruction: None,
        },
    );
    let v = t.spatial.sight.as_mut().unwrap();
    v.add_seer(
        500,
        Seer {
            player: 1,
            master: 83,
            los: 4.0,
            state: 2,
            explored: true,
            building: None,
            umv: 0,
            capture: 0,
            kind: 70,
        },
    );
    let r = v.registry.as_mut().unwrap();
    r.entries.entry(500).or_default().mask = 0b11;
    r.update_unit(500, 1, [11.5, 10.5], [0.2, 0.2], 4, 0, 0b11)
        .unwrap();
    t.tick(100);
    assert_eq!(t.players[0].objects.len(), 0);
    let ids: Vec<i32> = t.players[1].objects.iter().map(|u| u.id).collect();
    assert_eq!(ids, [500, 7]);
    assert_eq!(t.spatial.get(7).map(|p| p.player), Some(1));
    let torch = &t.players[1].objects[1];
    assert_eq!(torch.own_master.as_deref().map(|m| m.id), Some(854));
    let ai = torch.ai.as_ref().unwrap();
    assert_eq!(ai.notifies, [Notify::own(7, OWNER_CHANGED, [1, 0, 0])]);
    assert_eq!(ai.idle_timer, 0);
}

/// The livestock capture in a tick (`FUN_140aab570` from the Artifact's check in the second
/// update): player 1's villager stands beside Gaia's sheep 9520, which stops (its Artifact's
/// slot 0x38: the standing sprite's speed) and goes to player 1 (slot 0x70: a master of its
/// own with player 1's line of sight), the end of player 1's list, with its food (storage
/// flag 1 to the new owner).
#[test]
fn a_gaia_sheep_a_player_stands_by_goes_to_it() {
    let mut walker = crate::hunt::tests::walker();
    walker.velocity = None;
    walker.stats.speed = 0.8;
    walker.stats.stand_speed = 0.5;
    let sheep = Unit {
        artifact: Some(Artifact::new(true)),
        mover: Some(walker),
        ai: None,
        ..hands_unit(9520, 705)
    };
    let villager = Unit {
        position: [11.5, 10.5, 0.0],
        ai: None,
        ..hands_unit(500, 83)
    };
    let mut t = Turns::new(
        0,
        1,
        0,
        vec![0, 0, 0],
        vec![
            Player {
                objects: vec![sheep],
                attributes: vec![0.0; 300],
                ..Player::default()
            },
            Player {
                objects: vec![villager],
                attributes: vec![0.0; 300],
                ..Player::default()
            },
            Player {
                attributes: vec![0.0; 300],
                ..Player::default()
            },
        ],
    );
    let mut masters = hands_masters();
    for m in &mut masters {
        *m = Masters::new(m.iter().cloned().chain([Master {
            id: 83,
            kind: 70,
            class: 4,
            radius: [0.2, 0.2],
            ..Master::default()
        }]));
    }
    t.masters = masters;
    t.spatial = hands_world(9520, 0, LIVESTOCK, 705, 2.0);
    t.spatial.relations = vec![vec![1, 4, 4], vec![4, 1, 4], vec![4, 4, 1]];
    t.spatial.insert(
        500,
        Placed {
            position: [11.5, 10.5, 0.0],
            radius: [0.2, 0.2],
            class: 4,
            player: 1,
            master: 83,
            obstruction_type: 0,
            buildable_on: 0,
            tall: true,
            speed: 0.0,
            unknown: None,
            obstruction: None,
        },
    );
    t.spatial.sight.as_mut().unwrap().add_seer(
        500,
        Seer {
            player: 1,
            master: 83,
            los: 4.0,
            state: 2,
            explored: true,
            building: None,
            umv: 0,
            capture: 0,
            kind: 70,
        },
    );
    t.tick(100);
    assert_eq!(t.players[0].objects.len(), 1);
    t.tick(100);
    assert_eq!(t.players[0].objects.len(), 0);
    let ids: Vec<i32> = t.players[1].objects.iter().map(|u| u.id).collect();
    assert_eq!(ids, [500, 9520]);
    let sheep = &t.players[1].objects[1];
    assert_eq!(
        sheep.own_master.as_deref().map(|m| m.line_of_sight),
        Some(3.0)
    );
    assert_eq!(sheep.mover.as_ref().map(|m| m.stats.speed), Some(0.5));
    assert_eq!(sheep.artifact.map(|a| a.gaia), Some(false));
    assert_eq!(t.players[1].attributes[16], 100.0);
    assert_eq!(t.spatial.get(9520).map(|p| p.player), Some(1));
}

/// `FUN_141302450`: the scenario modes' descriptors have `+0x9c` 0; a mode above 16 reads the
/// first descriptor's.
#[test]
fn the_standard_modes_are_all_but_the_scenario_ones() {
    let standard: Vec<i32> = (0..=17).filter(|&m| standard_mode(m)).collect();
    assert_eq!(standard, [0, 1, 2, 6, 7, 8, 9, 10, 11, 12, 13, 16, 17]);
    assert!(standard_mode(-1));
}

/// The attributes an ownership change moves with its object, by its master of its own: the
/// old owner loses storage flags 2, 16 and 32 and counts one archer less (40), the new one
/// counts one more and gains flags 1, 2 and 32.
#[test]
fn an_object_changing_hands_takes_its_storage_and_class_counts_along() {
    let mut t = Turns::new(
        0,
        1,
        0,
        vec![0, 0],
        vec![
            Player {
                attributes: vec![0.0; 300],
                ..Player::default()
            },
            Player {
                attributes: vec![0.0; 300],
                ..Player::default()
            },
        ],
    );
    t.players[0].attributes[4] = 5.0;
    t.players[0].attributes[40] = 2.0;
    let mut u = hands_unit(1, 4);
    t.hand_over_attributes(0, 1, &u).unwrap();
    assert_eq!(t.players[0].attributes[4], 5.0);
    u.own_master = hands_masters()[0].get(4).cloned().map(Box::new);
    t.hand_over_attributes(0, 1, &u).unwrap();
    assert_eq!(
        [t.players[0].attributes[4], t.players[0].attributes[40]],
        [4.0, 1.0]
    );
    assert_eq!(
        [t.players[1].attributes[4], t.players[1].attributes[40]],
        [1.0, 1.0]
    );
    assert_eq!(t.players[1].attributes[11], 0.0);
}

/// MarvinBlue/426048731's update 1250: 10029's second hit kills its sheep 9520 (2 hit points,
/// 12 damage): it has none, Gaia takes it (its list, its storage, its unit AI's notifies),
/// and the players count the kill (300 + 1 and 325 + 1; a sheep costs nothing).
#[test]
fn a_killed_sheep_goes_to_gaia() {
    let sheep = Unit {
        id: 9520,
        life: Lifecycle {
            master_id: 705,
            hp: 2.0,
            ..Lifecycle::default()
        },
        ai: Some(UnitAi::default()),
        ..Unit::default()
    };
    let mut t = Turns::new(
        0,
        1,
        0,
        vec![0, 0],
        vec![
            Player {
                attributes: vec![0.0; 500],
                ..Player::default()
            },
            Player {
                objects: vec![town_centre(10029), sheep],
                attributes: vec![0.0; 500],
                ..Player::default()
            },
        ],
    );
    let master = Master {
        id: 705,
        class: 58,
        storage: vec![(16, 100.0, 1)],
        ..Master::default()
    };
    t.masters = vec![Masters::new([master.clone()]), Masters::new([master])];
    t.spatial.insert(
        9520,
        Placed {
            position: [107.0, 48.0, 0.0],
            radius: [0.35, 0.35],
            class: 58,
            player: 1,
            master: 705,
            obstruction_type: 0,
            buildable_on: 0,
            tall: true,
            speed: 0.0,
            unknown: None,
            obstruction: None,
        },
    );
    let mut v = t.victim(9520, 1).unwrap();
    v.hp = 0.0;
    v.damaged = Some((10029, [10029, -10, 14]));
    v.killed = Some(crate::damage::Kill {
        attacker: 10029,
        attacker_player: 1,
        to_gaia: true,
    });
    t.set_victim(&v).unwrap();
    assert_eq!(t.players[1].objects.len(), 1);
    let gone = &t.players[0].objects[0];
    assert_eq!((gone.id, gone.life.hp), (9520, 0.0));
    assert_eq!(t.spatial.get(9520).map(|p| p.player), Some(0));
    assert_eq!(t.players[1].attributes[301], 1.0);
    assert_eq!(t.players[1].attributes[326], 1.0);
    assert_eq!(t.players[0].attributes[16], 100.0);
}

/// A hit on player 2's castle 30 (master 82, 650 stone paid) with its annexes 31 and 32:
/// `Turns::victim` reads its annexes from the world's links, its main building for an annex,
/// its paid cost and that it is no trebuchet. The write-back of a razing hit (`FUN_1411cea90`):
/// the castle's `damaged_lately_timer`, its annexes' hit points, its kill (`FUN_1412005b0`: a
/// building's class counts no kill, both players count it against the other and its cost),
/// then the razing's history event (7, `data1` the owner) on the attacker's player and its adds.
#[test]
fn a_razed_building_s_write_back_reaches_its_annexes_history_and_counts() {
    let building = |id: i32, master_id: i16, annex_of: Option<i32>| Unit {
        id,
        life: Lifecycle {
            master_id,
            hp: 100.0,
            state: 2,
            ..Lifecycle::default()
        },
        annex_of,
        ..Unit::default()
    };
    let player = |objects| Player {
        objects,
        attributes: vec![0.0; 500],
        ..Player::default()
    };
    let mut t = Turns::new(
        0,
        1,
        0,
        vec![0, 0, 0],
        vec![
            player(vec![]),
            player(vec![]),
            player(vec![
                building(30, 82, None),
                building(31, 1, Some(30)),
                building(32, 1, Some(30)),
            ]),
        ],
    );
    let castle = Master {
        id: 82,
        kind: 80,
        class: 3,
        cost: vec![(2, 650, true), (0, 10, false)],
        weapons: vec![(3, 11)],
        ..Master::default()
    };
    let annex = Master {
        id: 1,
        kind: 80,
        class: 3,
        ..Master::default()
    };
    let masters = Masters::new([castle, annex]);
    t.masters = vec![masters.clone(), masters.clone(), masters];
    t.spatial.set_annex(31, Some(30));
    t.spatial.set_annex(32, Some(30));
    // a history adds events once it has an entry
    t.players[1].history.entries = vec![[0, 0]];
    let mut v = t.victim(30, 1).unwrap();
    let b = v.building.clone().unwrap();
    assert_eq!(
        (b.main, b.annexes, b.cost, b.trebuchet),
        (None, vec![31, 32], 650.0, false)
    );
    let a = t.victim(31, 1).unwrap();
    assert_eq!(a.building.and_then(|b| b.main), Some(30));
    v.hp = 0.0;
    v.killed = Some(crate::damage::Kill {
        attacker: 9,
        attacker_player: 1,
        to_gaia: false,
    });
    let b = v.building.as_mut().unwrap();
    (b.struck, b.damaged_lately) = (true, 60);
    b.razed.push(crate::damage::Razed {
        attacker_player: 1,
        owner: 2,
        event: Some(crate::history::CASTLE_DESTROYED),
        adds: vec![(1, 43, 1.0), (2, 155, 1.0), (1, 170, 650.0)],
    });
    t.set_victim(&v).unwrap();
    let hp: Vec<f32> = t.players[2].objects.iter().map(|u| u.life.hp).collect();
    assert_eq!(hp, [0.0, 0.0, 0.0]);
    assert_eq!(t.players[2].objects[0].damaged_lately, 60);
    let (attacker, owner) = (&t.players[1].attributes, &t.players[2].attributes);
    assert_eq!([attacker[20], owner[154]], [0.0, 0.0]);
    assert_eq!(
        [attacker[302], owner[326], attacker[402]],
        [1.0, 1.0, 650.0]
    );
    assert_eq!([attacker[43], owner[155]], [1.0, 1.0]);
    assert_eq!([attacker[170], owner[152]], [1300.0, 650.0]);
    let event = t.players[1]
        .history
        .event_of_kind(crate::history::CASTLE_DESTROYED)
        .unwrap();
    assert_eq!(event.data1, 2.0);
}

/// `FUN_140fde380`: a master of type 70 and up, of class 51 or 54, with a weapon (`+0x810`
/// above 0) is a trebuchet; the paid cost (`+0x880`) sums the entries with a resource and the
/// paid flag, as floats in their order.
#[test]
fn a_trebuchet_is_a_master_of_class_51_or_54_with_a_weapon() {
    use crate::turn::world::{is_trebuchet, paid_cost};
    let m = |kind, class, weapons: Vec<(i16, i16)>| Master {
        kind,
        class,
        weapons,
        ..Master::default()
    };
    assert!(is_trebuchet(&m(80, 54, vec![(4, 200)])));
    assert!(is_trebuchet(&m(80, 51, vec![(4, 1)])));
    assert!(is_trebuchet(&m(70, 54, vec![(4, 1)])));
    assert!(!is_trebuchet(&m(80, 51, vec![])));
    assert!(!is_trebuchet(&m(60, 54, vec![(4, 1)])));
    assert!(!is_trebuchet(&m(80, 3, vec![(3, 11)])));
    let paid = Master {
        cost: vec![(1, 200, true), (3, 200, true), (-1, 5, true), (2, 7, false)],
        ..Master::default()
    };
    assert_eq!(paid_cost(&paid), 400.0);
}

/// MarvinBlue/426048731's 1086: SET GATHER POINT on TC 9149 at (107.2451, 46.075863) on the
/// sheep 9520 (master 705).
#[test]
fn set_gather_point_takes_the_point_and_target_or_clears_it() {
    let mut tc = town_centre(9149);
    tc.life.master_id = 109;
    let mut t = one_player(vec![tc], [200.0; 4]);
    t.masters = vec![Masters::new([Master {
        id: 109,
        garrison: 15,
        ..Master::default()
    }])];
    let set = |target, point, own| Command::GatherPoint {
        player: 0,
        ids: vec![9149],
        point,
        target,
        target_master: 705,
        own,
    };
    t.tick_with(25, &[set(9520, [107.2451, 46.075_863], false)]);
    assert_eq!(t.players[0].unsupported, None);
    assert_eq!(
        t.players[0].objects[0].gather_point,
        Some(GatherPoint {
            point: [107.2451, 46.075_863, 0.0],
            target: 9520,
            target_master: 705,
        })
    );
    t.tick_with(25, &[set(-1, [-1.0, -1.0], false)]);
    assert_eq!(t.players[0].objects[0].gather_point, None);
    t.tick_with(25, &[set(-1, [-1.0, -1.0], true)]);
    assert_eq!(
        t.players[0].objects[0].gather_point.map(|g| g.target),
        Some(9149)
    );
}

/// `BUILDING_COMMAND_QUEUE` on a town centre with a gather point: the gather point goes
/// into the queue first and clears, then the packet's step; a second packet adds its step
/// alone; SET GATHER POINT empties the queue.
#[test]
fn a_command_queue_takes_the_gather_point_first_then_the_steps() {
    let mut tc = town_centre(9149);
    tc.life.master_id = 109;
    tc.ai = Some(UnitAi {
        ai_type: 17,
        current_order: -1,
        current_action: -1,
        current_target: -1,
        last_target: -1,
        ..UnitAi::default()
    });
    tc.gather_point = Some(GatherPoint {
        point: [30.5, 40.5, 0.0],
        target: -1,
        target_master: -1,
    });
    let mut t = one_player(vec![tc], [200.0; 4]);
    t.masters = vec![Masters::new([Master {
        id: 109,
        kind: 80,
        garrison: 15,
        ..Master::default()
    }])];
    let step = |point| Command::CommandQueue {
        player: 0,
        ids: vec![9149],
        target: -1,
        point,
    };
    t.tick_with(25, &[step([50.0, 60.0])]);
    t.tick_with(25, &[step([70.0, 80.0])]);
    assert_eq!(t.players[0].unsupported, None);
    let tc = &t.players[0].objects[0];
    assert_eq!(tc.gather_point, None);
    let steps: Vec<_> = tc
        .ai
        .as_ref()
        .unwrap()
        .building_orders
        .iter()
        .map(|o| (o.order, o.target, o.point, o.priority, o.ready))
        .collect();
    let at = |x, y| (crate::unit_ai::MOVE_ORDER, -1, [x, y, -1.0], -1, false);
    assert_eq!(steps, vec![at(30.5, 40.5), at(50.0, 60.0), at(70.0, 80.0)]);
    t.tick_with(
        25,
        &[Command::GatherPoint {
            player: 0,
            ids: vec![9149],
            point: [-1.0, -1.0],
            target: -1,
            target_master: -1,
            own: false,
        }],
    );
    let left = &t.players[0].objects[0].ai.as_ref().unwrap().building_orders;
    assert_eq!(left.len(), 0);
}

#[test]
fn a_unit_due_to_spawn_stops_the_player() {
    let mut t = one_player(vec![centre(9)], [200.0; 4]);
    let quick = queue(9, 97, 1);
    t.tick_with(1000, &[quick]);
    // the building makes the make action in its update: one unit in production
    assert_eq!(t.players[0].unsupported, None);
    assert_eq!(t.players[0].attributes[81], 1.0);
    t.tick(1000);
    assert_eq!(
        t.players[0].freezes.first().map(|f| f.2.clone()),
        Some(GenieError::unsupported(
            "a spawn without the players' sight"
        ))
    );
}

/// Slot 0x468 (`FUN_141157c50`): the top level's path, its points, length and flags, and
/// that level's waypoint; nothing without a path at the top or without movement state.
#[test]
fn a_unit_s_patrol_information_is_its_top_path_and_waypoint() {
    let mut m = crate::hunt::tests::walker();
    let mut walked = crate::movement::PathData::without_path();
    walked.has_path = true;
    walked.points = vec![[1.0, 0.0, 1.0], [2.0, 0.0, 1.0]];
    let mut top = crate::movement::PathData::without_path();
    top.has_path = true;
    top.points = vec![[20.0, 0.0, 20.0], [30.0, 0.0, 20.0]];
    top.length = 10.0;
    top.path_flags = 7;
    top.waypoint = 1;
    m.paths = vec![walked, top];
    let (saved, waypoint) = patrol_information(Some(&m)).unwrap();
    assert_eq!(saved.path.points, [[20.0, 0.0, 20.0], [30.0, 0.0, 20.0]]);
    assert_eq!((saved.path.length, saved.flags, waypoint), (10.0, 7, 1));
    m.paths[1].has_path = false;
    assert_eq!(patrol_information(Some(&m)), None);
    assert_eq!(patrol_information(None), None);
}

/// `restorePatrolInformation`'s unit part: the own waypoints become the saved patrol's
/// points (slot 0x448, a whole coordinate moved to the tile's middle), and a forced, running
/// `MoveTo` goes to the saved waypoint; the old `MoveTo` to an object lets it go (its teardown's
/// slot 0x180, the caller's).
#[test]
fn a_restored_patrol_takes_its_points_and_runs_to_its_waypoint() {
    let saved = crate::unit_ai::PatrolPath {
        path: crate::tile_path::Path {
            points: vec![[20.0, 0.5, 20.0], [30.0, 0.5, 20.25]],
            length: 10.0,
        },
        flags: 7,
    };
    let mut u = Unit {
        mover: Some(crate::hunt::tests::walker()),
        ai: Some(UnitAi {
            patrol_path: Some(saved),
            patrol_waypoint: 1,
            ..UnitAi::default()
        }),
        move_to: Some(crate::move_to::MoveToAction {
            state: MoveTo::Moving,
            goal: None,
            target: 77,
            range: 0.0,
            run: false,
            wait: crate::move_to::MoveWait::default(),
        }),
        ..Unit::default()
    };
    let run = u.restore_patrol().expect("the saved patrol's move");
    assert_eq!(
        (run.point, run.range, run.force, run.walk),
        ([30.0, 20.25, 0.5], 0.0, true, false)
    );
    assert_eq!(
        u.mover.as_ref().unwrap().user_waypoints,
        [[20.5, 0.5, 20.5], [30.5, 0.5, 20.25]]
    );
    assert_eq!(ai_move_of(&mut u, run, 6), Ok(vec![77]));
    let move_to = u.move_to.unwrap();
    assert_eq!(
        (move_to.goal, move_to.range, move_to.run),
        (Some([30.0, 20.25, 0.5]), 0.0, true)
    );
}

/// `RGE_Combat_Object::update`: the attack timer runs down by dt times the object's rate
/// (`+0x87c`), which a terrain look sets ([`crate::abilities`]).
#[test]
fn the_attack_timer_runs_down_at_the_object_s_rate() {
    let unit = Unit {
        id: 1,
        attack_timer: 1.0,
        abilities: crate::abilities::Abilities {
            attack_rate: 0.5,
            ..crate::abilities::Abilities::default()
        },
        ..Unit::default()
    };
    let mut t = Turns::new(
        0,
        1,
        0,
        vec![0],
        vec![Player {
            objects: vec![unit],
            ..Player::default()
        }],
    );
    t.tick(100);
    assert_eq!(t.players[0].objects[0].attack_timer, 1.0 - 0.1 * 0.5);
}

/// records/425984813: a barracks (12) that dies short of its build time plays its destruction
/// sprite (389), not its death sprite (2573) (`RGE_Static_Object::update`'s sprite after the
/// death), a foundation (state 0) as a building in state 2.
#[test]
fn a_building_short_of_its_build_time_that_dies_plays_its_destruction_sprite() {
    for state in [0, 2] {
        building_short_of_its_build_time_dies(state);
    }
}

fn building_short_of_its_build_time_dies(state: u8) {
    use crate::sprite::{ActiveSprite, Graphic};
    let g = |id| Graphic {
        id,
        frames: 10,
        facets: 1,
        speed_multiplier: 1.0,
        frame_duration: 0.1,
        replay_delay: 0.0,
        flags: 1,
    };
    let barracks = Master {
        id: 12,
        kind: 80,
        class: 3,
        standing_sprite: 207,
        death_sprite: 2573,
        build_time: 50,
        hit_points: 1200,
        building: Some(crate::masters::BuildingMaster {
            destruction_sprite: 389,
            ..crate::masters::BuildingMaster::default()
        }),
        ..Master::default()
    };
    let foundation = Unit {
        id: 5,
        life: Lifecycle {
            class: Class::Animated,
            state,
            hp: 0.5,
            master_id: 12,
            master: Some(MasterData::default()),
            dying_sprite: Some(2573),
            ..Lifecycle::default()
        },
        construction: Some(Construction {
            progress: 10.0,
            desolid: false,
            builders: 0,
            hit_points: 1200,
            build_time: 50,
            record: crate::spatial::Obstruction { kind: 2, class: 0 },
            snow: false,
            sprite_override: false,
            started: false,
        }),
        sprite: Some(ActiveSprite::at_rest(g(207), 0)),
        pasture: -1,
        my_pasture: -1,
        ..Unit::default()
    };
    let mut t = Turns::new(
        0,
        1,
        0,
        vec![0],
        vec![Player {
            objects: vec![foundation],
            attributes: vec![0.0; 300],
            ..Player::default()
        }],
    );
    t.masters = vec![Masters::new([barracks])];
    t.graphics = [(207, g(207)), (389, g(389)), (2573, g(2573))]
        .into_iter()
        .collect();
    t.tick(10);
    assert_eq!(t.players[0].unsupported, None);
    let u = &t.players[0].objects[0];
    assert_eq!(u.life.state, 4);
    assert_eq!(u.sprite.unwrap().graphic.id, 389);
}

/// GAME's cheat code (sub-command 6, `FUN_1415c92c0`): without the game's cheats it does
/// nothing; with them the cheat would run, which the slice does not model: its player stops.
#[test]
fn a_cheat_code_does_nothing_without_the_game_s_cheats() {
    let mut t = sent_off_world(vec![]);
    let before = t.players.clone();
    let cheat = Command::Game(GameCommand {
        issuer: 1,
        sub: 6,
        player: 1,
        a: 4,
        ..GameCommand::default()
    });
    t.run(&cheat);
    assert_eq!(t.players, before);
    t.settings.cheats_enabled = true;
    t.run(&cheat);
    assert!(t.players[1].stopped);
}

/// A unit of the AI-move tests: the walker of the hunt tests at (10.5, 10.5), master 83, on no
/// pasture.
fn mover_unit(id: i32) -> Unit {
    Unit {
        id,
        position: [10.5, 10.5, 0.0],
        mover: Some(crate::hunt::tests::walker()),
        my_pasture: -1,
        life: Lifecycle {
            master_id: 83,
            ..Lifecycle::default()
        },
        ..Unit::default()
    }
}

/// A unit's own Attack on `target` (second target `target2`) in `state`, without a `MoveTo`.
fn own_attack(target: i32, target2: i32, state: u8) -> crate::attack::Attack {
    crate::attack::Attack {
        target,
        target2,
        state,
        timer: 0.0,
        range: 0.0,
        min_range: 0.0,
        point: [-1.0; 3],
        seen: [0.0; 3],
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
        runs: true,
        same_owner: false,
    }
}

/// The unit's slot 0x340 ([`Unit::ai_move_with`]) for the unit of class `class` of player 1:
/// what it let go, or why it stopped.
fn ai_move_of(u: &mut Unit, m: crate::unit_ai::AiMove, class: i16) -> Result<Vec<i32>, GenieError> {
    let by = Releaser {
        id: u.id,
        class,
        player: 1,
    };
    let (mut releases, mut leaves) = (Vec::new(), Vec::new());
    let mut rng = Well512::default();
    u.ai_move_with(
        m,
        (class, 1),
        (&mut releases, &mut leaves, by),
        (None, &std::collections::BTreeMap::new(), &mut rng),
    )?;
    assert!(releases.iter().all(|&(_, b)| b == by));
    Ok(releases.into_iter().map(|(t, _)| t).collect())
}

const AI_POINT: [f32; 3] = [14.5, 12.0, 0.0];

fn forced(walk: bool) -> crate::unit_ai::AiMove {
    crate::unit_ai::AiMove {
        point: AI_POINT,
        range: 0.5,
        force: true,
        walk,
    }
}

/// `FUN_1410866e0` on a unit whose Attack is done (state 1, 426391614's scout 123 in update
/// 1052): the Attack's slot 0x40 is the base one (`FUN_140a78490`, 0), so a `MoveTo` to the point
/// within the move's range, running unless the move walks, replaces the list; the Attack's
/// teardown lets both its targets go. The `MoveTo` starts (state 3: no target, so never within
/// 0.01, and the unit has speed).
#[test]
fn an_ai_move_replaces_a_done_attack_with_a_move_to_the_point() {
    let mut u = mover_unit(123);
    u.attack = Some(own_attack(9149, 9150, crate::attack::DONE));
    assert_eq!(ai_move_of(&mut u, forced(false), 47), Ok(vec![9149, 9150]));
    assert_eq!(u.attack, None);
    let mt = u.move_to.expect("the AI's MoveTo");
    assert_eq!(
        (mt.state, mt.goal, mt.target, mt.range, mt.run),
        (MoveTo::Start, Some(AI_POINT), -1, 0.5, true)
    );
    // walking: the master's walking sprite
    let mut u = mover_unit(123);
    assert_eq!(ai_move_of(&mut u, forced(true), 47), Ok(vec![]));
    assert!(!u.move_to.unwrap().run);
}

/// The Gather's slot 0x40 (`0x140a8b740`) takes a move to a point: the target goes (slot 0x68
/// with none, the caller's release), the point is the move's and `set_state(15)` walks there; the
/// unit's list keeps the Gather (425780724's villager in update 5526, its Gather idle).
#[test]
fn an_ai_move_hands_the_point_to_the_gather() {
    let task = crate::hunt::HuntTask {
        id: 2,
        range: 0.25,
        resource: 17,
        sprites: [-1; 4],
        work_value: 1.0,
        productivity: -1,
        kind4: -1,
    };
    let bush = crate::gather::Resource {
        id: 28222,
        position: [23.5, 93.5, 0.0],
        class: 7,
        master: 59,
        player: 0,
        workers: 0,
        drop_site: false,
        gap: 1.2,
        radius: [0.5, 0.5],
    };
    let rate = crate::gather::StopRate {
        work_rate: 1.0,
        capacity: 10.0,
        productivity: 1.0,
    };
    let mut gather = crate::gather::GatherAction::new(task, &bush, false, rate).unwrap();
    gather.state = crate::gather::IDLE;
    let mut u = mover_unit(31139);
    u.gather = Some(gather);
    assert_eq!(ai_move_of(&mut u, forced(false), 4), Ok(vec![28222]));
    let g = u.gather.expect("the Gather stays");
    assert_eq!(
        (g.state, g.target, g.point, g.walk_to),
        (crate::gather::MOVED, -1, AI_POINT, Some(AI_POINT))
    );
    assert_eq!(g.move_to, Some(MoveTo::Start));
    assert_eq!(u.move_to, None);
}

/// The Hunt's slot 0x40 (`FUN_140ac52c0`) takes a point unless its task has a work range: its
/// target goes, `+0x68` is −1 and `set_state(15)` walks to the point (431193960's villager in
/// update 1465, its Hunt idle). A task with a range answers 0: the `MoveTo` replaces the list and
/// the Hunt's teardown lets its target and its drop site go.
#[test]
fn an_ai_move_hands_the_point_to_a_hunt_without_a_work_range() {
    let mut u = mover_unit(47);
    let mut hunt = crate::hunt::tests::hunting(9149);
    hunt.state = crate::hunt::IDLE;
    hunt.carry_to = 9150;
    u.hunt = Some(hunt);
    assert_eq!(ai_move_of(&mut u, forced(false), 4), Ok(vec![9149]));
    let h = u.hunt.expect("the Hunt stays");
    assert_eq!(
        (h.state, h.target, h.carry_to, h.point, h.target_class),
        (crate::hunt::WALK_POINT, -1, 9150, AI_POINT, -1)
    );
    assert_eq!(u.move_to, None);
    let mut u = mover_unit(47);
    hunt.task.range = 1.0;
    u.hunt = Some(hunt);
    assert_eq!(ai_move_of(&mut u, forced(false), 4), Ok(vec![9149, 9150]));
    assert_eq!(u.hunt, None);
    assert_eq!(u.move_to.map(|m| m.goal), Some(Some(AI_POINT)));
}

/// Without the force a head `MoveTo` whose point (`+0x38`) is the move's stands, a point one or
/// an object one (the target's place, the mover's `move_to_point`); forced, or to another point,
/// the new `MoveTo` replaces it (its teardown stops the unit and lets its target go).
#[test]
fn an_unforced_ai_move_leaves_a_move_to_the_same_point() {
    let unforced = crate::unit_ai::AiMove {
        force: false,
        ..forced(false)
    };
    let old = crate::move_to::MoveToAction::to_point(AI_POINT, 0.0, false, None);
    let mut u = mover_unit(5);
    u.move_to = Some(old);
    assert_eq!(ai_move_of(&mut u, unforced, 47), Ok(vec![]));
    assert_eq!(u.move_to, Some(old));
    let to_object = crate::move_to::MoveToAction {
        goal: None,
        target: 9149,
        ..old
    };
    u.move_to = Some(to_object);
    u.mover.as_mut().unwrap().move_to_point = Some(AI_POINT);
    assert_eq!(ai_move_of(&mut u, unforced, 47), Ok(vec![]));
    assert_eq!(u.move_to, Some(to_object));
    assert_eq!(ai_move_of(&mut u, forced(false), 47), Ok(vec![9149]));
    assert_eq!(u.move_to.map(|m| (m.target, m.range)), Some((-1, 0.5)));
    u.move_to = Some(old);
    let elsewhere = crate::unit_ai::AiMove {
        point: [1.0, 2.0, 0.0],
        ..unforced
    };
    assert_eq!(ai_move_of(&mut u, elsewhere, 47), Ok(vec![]));
    assert_eq!(u.move_to.unwrap().goal, Some([1.0, 2.0, 0.0]));
}

/// `FUN_141216c50`: an unpacked trebuchet (class 54) packs instead (not modelled); a packed one
/// (class 51) moves as any unit. The base slot 0x40 of an Explore and of a wait for a foundation
/// takes nothing; an action the slice does not hold stops it.
#[test]
fn an_ai_move_of_a_trebuchet_packs_and_the_base_slot_takes_nothing() {
    let mut u = mover_unit(7);
    assert!(ai_move_of(&mut u, forced(false), 54).is_err());
    assert_eq!(u.move_to, None);
    assert_eq!(ai_move_of(&mut u, forced(false), 51), Ok(vec![]));
    assert_eq!(u.move_to.map(|m| m.goal), Some(Some(AI_POINT)));
    let mut u = mover_unit(7);
    u.wait_solidify = Some(crate::wait_solidify::WaitSolidify::new(
        crate::masters::Task::default(),
        10028,
    ));
    assert_eq!(u.head_takes_point(AI_POINT, 1), Ok(PointMove::Declined));
    let mut u = mover_unit(7);
    u.unmodelled_action = Some(GenieError::unsupported("a test's action"));
    assert!(ai_move_of(&mut u, forced(false), 47).is_err());
}

/// A unit without speed: the `MoveTo`'s start fails it (state 13), whose `set_state` (not
/// quiet) shows the master's standing sprite: the mover's speed is the standing sprite's and
/// the unit's `+0x7b8` clears.
#[test]
fn an_ai_move_of_a_unit_without_speed_fails_at_its_start_standing() {
    let mut u = mover_unit(8);
    let m = u.mover.as_mut().unwrap();
    m.stats.speed = 0.0;
    m.stats.stand_speed = 0.0;
    u.anim_speed = Some(1.0);
    assert_eq!(ai_move_of(&mut u, forced(false), 47), Ok(vec![]));
    assert_eq!(u.move_to.map(|m| m.state), Some(MoveTo::Failed));
    assert_eq!(u.anim_speed, Some(0.0));
}

/// The unit AI's attack command ([`crate::unit_ai::UnitSide::attack`]) of the scout 123 (master
/// 448, class 47, 426049998's) on Gaia's wolf 50 half a tile east, its attack timer `timer`, inside
/// `inside` if any: the scout and what the command answered and took.
fn scout_attack(timer: f32, inside: Option<i32>) -> (Unit, crate::unit_ai::Attacked, Vec<i32>) {
    use crate::unit_ai::UnitSide;
    let standing = Graphic {
        id: 300,
        frames: 10,
        facets: 8,
        speed_multiplier: 1.0,
        frame_duration: 0.1,
        replay_delay: 0.0,
        flags: 1,
    };
    let graphics = std::collections::BTreeMap::from([
        (300, standing),
        (
            301,
            Graphic {
                id: 301,
                ..standing
            },
        ),
    ]);
    let scout = Master {
        id: 448,
        kind: 70,
        class: 47,
        standing_sprite: 300,
        attack_sprite: 301,
        weapon_range: 1.0,
        reload: 2.0,
        ..Master::default()
    };
    let wolf = Master {
        id: 126,
        kind: 70,
        class: 10,
        ..Master::default()
    };
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
    let mut spatial = Spatial::new([120, 120]);
    spatial.insert(123, placed(10.0, 47, 1, 448));
    spatial.insert(50, placed(10.5, 10, 0, 126));
    let gaia = Player {
        objects: vec![Unit {
            id: 50,
            position: [10.5, 10.0, 0.0],
            life: Lifecycle {
                master_id: 126,
                ..Lifecycle::default()
            },
            ..Unit::default()
        }],
        ..Player::default()
    };
    let players = [gaia];
    let mut rng = Well512::default();
    let mut walker = crate::hunt::tests::walker();
    walker.stats.speed = 1.2;
    let mut unit = Unit {
        id: 123,
        position: [10.0, 10.0, 0.0],
        mover: Some(walker),
        sprite: Some(crate::sprite::ActiveSprite::start(standing, &mut rng)),
        attack_timer: timer,
        life: Lifecycle {
            master_id: 448,
            inside,
            ..Lifecycle::default()
        },
        ..Unit::default()
    };
    let ai = UnitAi {
        stance: 2,
        current_order: -1,
        ai_type: 13,
        ..UnitAi::default()
    };
    let masters = Masters::new([scout]);
    let every = [Masters::new([wolf]), masters.clone()];
    let (mut pathing, mut zones) = (Pathing::default(), crate::zones::Zones::default());
    let groups = crate::group::UnitGroups::default();
    let (mut object, mut sprite) = (Well512::default(), Well512::default());
    let (mut takes, mut releases, mut leaves, mut waits) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let attacked = {
        let mut side = view::UnitView {
            unit: &mut unit,
            attacker: crate::unit_ai::Attacker {
                ai_type: 13,
                player: 1,
                computer: false,
                state: 2,
                held: -1,
                top_action: None,
                master: 448,
                kind: 70,
                class: 47,
                attack_priority: 0,
                missile: -1,
                range: 1.0,
                min_range: 0.0,
                charge_type: 0,
                area_effect: 0.0,
                blast_level: 0,
                los: 4.0,
                position: [10.0, 10.0, 0.0],
                radius: [0.2, 0.2],
                speed: 1.2,
                scenario: false,
                firepower: 0.0,
                capture: 0,
            },
            player: 1,
            computer: false,
            spatial: &mut spatial,
            pathing: &mut pathing,
            groups: &groups,
            object: &mut object,
            sprite: &mut sprite,
            world_time: 0,
            construction_rate: 1.0,
            others: &[],
            zones: &mut zones,
            foreign: Foreign {
                before: &players,
                after: &[],
                left: None,
            },
            every: &every,
            masters: Some(&masters),
            graphics: &graphics,
            takes: &mut takes,
            releases: &mut releases,
            builder_leaves: &mut leaves,
            waits: &mut waits,
            relations: &[0, 0],
            attributes: &[],
            civ: 0,
            drop_sites: &[],
            mode_680: 0,
        };
        side.attack(50, false, &ai).unwrap()
    };
    (unit, attacked, takes.into_iter().map(|(t, _)| t).collect())
}

/// `FUN_141081a80`'s start (`0x140a7f8c0`) of a unit that cannot move (stance 2, no order) in
/// reach: `set_state(6)` at once (426049998's scout 123 in update 1222). Its readyToAttack finds
/// the target on the point; facing it with its attack timer run out the scout swings
/// (`set_state(7)`: the attack sprite, a hit pending); with time left it waits in state 6 on the
/// standing sprite. The constructor takes the target either way. A unit inside another fails at
/// the start (`set_state(13)`: speed 1, the standing sprite).
#[test]
fn an_attack_command_of_a_unit_that_cannot_move_goes_ready_and_swings_at_once() {
    let (unit, attacked, takes) = scout_attack(0.0, None);
    assert_eq!((attacked.attacks, attacked.posted), (true, vec![]));
    assert_eq!(takes, vec![50]);
    let attack = unit.attack.expect("the scout's Attack");
    assert_eq!(
        (attack.state, attack.pending, attack.point),
        (crate::attack::SWING, true, [10.5, 10.0, 0.0])
    );
    assert_eq!(unit.sprite.map(|s| s.graphic.id), Some(301));
    let (unit, _, _) = scout_attack(0.5, None);
    let attack = unit.attack.expect("the scout's Attack");
    assert_eq!(
        (attack.state, attack.pending),
        (crate::attack::READY, false)
    );
    assert_eq!(unit.sprite.map(|s| s.graphic.id), Some(300));
    let (unit, attacked, takes) = scout_attack(0.0, Some(9149));
    assert_eq!((attacked.attacks, takes), (true, vec![50]));
    assert_eq!(unit.attack.map(|a| a.state), Some(crate::attack::FAILED));
    assert_eq!(unit.anim_speed, Some(1.0));
}

/// A house's movement values: it never moves (speed 0).
const fn house_moves() -> crate::movement::MoveStats {
    crate::movement::MoveStats {
        fly_mode: 0,
        speed: 0.0,
        turn_rate: f32::MAX,
        turn_radius: 0.0,
        turn_radius_speed: f32::MAX,
        radius: [1.0, 1.0, 1.0],
        restriction: 7,
        restriction_mask: 6,
        turn_time: 0.0,
        move_algorithm: 0,
        min_collision: 1.0,
        fast_turn: false,
        walk_speed: 0.0,
        stand_speed: 0.0,
        run_speed: 0.0,
        facets: None,
        weapon_range: 0.0,
    }
}

/// BUILD's foundation is a moving object (type 80): the moving setup (`FUN_1411580a0`) gives it
/// its movement state, though it never moves (speed 0), and the position setter makes its place
/// a move at the world time (`+0x870`), which the walk reads of a building it meets (slot 0x5e8,
/// `FUN_14106d2d0`): a foundation made after a unit's path was can be kept as a hit.
#[test]
fn a_build_command_s_foundation_holds_its_movement_state() {
    let mut masters = builder_masters();
    let house = masters.get_mut(70).unwrap();
    house.moving = Some(house_moves());
    let mut t = Turns::new(
        0,
        29910,
        1,
        vec![0, 0],
        vec![
            Player::default(),
            Player {
                objects: vec![Unit {
                    id: 9175,
                    position: [30.0, 89.0, 0.0],
                    life: Lifecycle {
                        class: Class::Animated,
                        state: 2,
                        hp: 25.0,
                        master_id: 83,
                        ..Lifecycle::default()
                    },
                    ai: Some(UnitAi {
                        ai_type: 8,
                        current_order: -1,
                        current_action: -1,
                        current_target: -1,
                        ..UnitAi::default()
                    }),
                    ..Unit::default()
                }],
                resources: [200.0, 100.0, 200.0, 100.0],
                attributes: vec![0.0; 250],
                ..Player::default()
            },
        ],
    );
    t.next_object_id = 10028;
    t.world_time = 72890;
    t.masters = vec![Masters::default(), masters];
    placeable(&mut t);
    t.run(&Command::Build {
        player: 1,
        master: 70,
        unit_ai: Ok(None),
        position: [30.0, 88.0, 0.0],
        builders: vec![9175],
        queued: false,
        commits: false,
    });
    assert_eq!(t.players[1].unsupported, None);
    let foundation = t.players[1].objects.last().unwrap();
    assert_eq!(foundation.id, 10028);
    let m = foundation
        .mover
        .as_ref()
        .expect("a building's movement state");
    assert_eq!(
        (m.forward, m.waiting, m.last_move, m.moved_at, m.paths.len()),
        ([1.0, 0.0, 0.0], 0, 72890, [30.0, 88.0, 0.0], 0)
    );
    t.sync_world();
    let gait = t
        .spatial
        .gait(10028)
        .expect("the walk's view of the building");
    assert_eq!(
        (gait.moving, gait.last_move, gait.pause),
        (false, 72890, None)
    );
}

/// BUILD's foundation takes the setups' draws a new unit takes (`FUN_1411aca00`'s combat setup
/// `FUN_14121df60` and the setups it runs first), with its master's standing sprite current
/// (slot 0x798, `FUN_1412029f0`): a sprite of several facets with sequence flag 2 draws the
/// facet the building faces (the moving setup `FUN_141161450`, "setup" line 0x2c3), before the
/// powerup offset (line 0x1c1). 429730863's palisade 15472 (standing sprite 588: 5 facets,
/// sequence type 3) faces facet 4, 5.0265484.
#[test]
fn a_build_command_s_foundation_draws_its_facing_before_its_powerup_offset() {
    let build = |flags: u8| {
        let mut masters = builder_masters();
        let house = masters.get_mut(70).unwrap();
        house.standing_sprite = 588;
        house.moving = Some(house_moves());
        let mut t = Turns::new(
            0,
            29910,
            1,
            vec![0, 0],
            vec![
                Player::default(),
                Player {
                    objects: vec![Unit {
                        id: 9175,
                        position: [30.0, 89.0, 0.0],
                        life: Lifecycle {
                            class: Class::Animated,
                            state: 2,
                            hp: 25.0,
                            master_id: 83,
                            ..Lifecycle::default()
                        },
                        ai: Some(UnitAi {
                            ai_type: 8,
                            current_order: -1,
                            current_action: -1,
                            current_target: -1,
                            ..UnitAi::default()
                        }),
                        ..Unit::default()
                    }],
                    resources: [200.0, 100.0, 200.0, 100.0],
                    attributes: vec![0.0; 250],
                    ..Player::default()
                },
            ],
        );
        t.next_object_id = 10028;
        t.masters = vec![Masters::default(), masters];
        t.streams.seed_all(0x5eed);
        let _ = t.graphics.insert(
            588,
            Graphic {
                id: 588,
                frames: 1,
                facets: 5,
                speed_multiplier: 1.0,
                frame_duration: 0.0,
                replay_delay: 0.0,
                flags,
            },
        );
        placeable(&mut t);
        let before = t.streams.get(Stream::Object).clone();
        t.run(&Command::Build {
            player: 1,
            master: 70,
            unit_ai: Ok(None),
            position: [30.0, 88.0, 0.0],
            builders: vec![9175],
            queued: false,
            commits: false,
        });
        assert_eq!(t.players[1].unsupported, None);
        (t, before)
    };
    // sequence type 3: the facing's draw, then the powerup offset's
    let (mut t, mut before) = build(3);
    let facing = before.rand15();
    let _ = before.rand15();
    assert_eq!(t.streams.get(Stream::Object), &before);
    let facets = crate::facing::SpriteFacets {
        count: 5,
        flag_1: true,
    };
    let foundation = t.players[1].objects.last().unwrap();
    let angle = crate::facing::drawn_angle(facing, facets);
    assert_ne!(angle, 0.0);
    assert_eq!(foundation.mover.as_ref().map(|m| m.angle), Some(angle));
    // without sequence flag 2 only the powerup offset's
    let (mut t, mut before) = build(1);
    let _ = before.rand15();
    assert_eq!(t.streams.get(Stream::Object), &before);
    let foundation = t.players[1].objects.last().unwrap();
    assert_eq!(foundation.mover.as_ref().map(|m| m.angle), Some(0.0));
}

/// [`builder_masters`] with the farmer (259, of the villagers' group 1, which builds as 118)
/// and the farm (50: class 49, its own base, available, 60 wood, a building part of its own).
fn farm_masters() -> Masters {
    let builders = builder_masters();
    let farmer = Master {
        id: 259,
        ..builders.get(83).unwrap().clone()
    };
    let farm = Master {
        id: 50,
        base_id: 50,
        class: 49,
        available: true,
        radius: [1.5, 1.5],
        clearance: [1.5, 1.5],
        tall: false,
        cost: vec![(1, 60, true)],
        hit_points: 480,
        build_time: 15,
        ..builders.get(70).unwrap().clone()
    };
    Masters::new(builders.iter().cloned().chain([farmer, farm]))
}

/// Player 1's farmer 9175 back at its farm spot (30.5, 88.5), its walk back (state 0xf2)
/// ended; the player holds 100 wood.
fn reseed_world() -> Turns {
    let task = crate::hunt::HuntTask {
        id: 2,
        range: 0.25,
        resource: 15,
        sprites: [-1; 4],
        work_value: 1.0,
        productivity: -1,
        kind4: -1,
    };
    let farm = crate::gather::Resource {
        id: 12557,
        position: [30.5, 88.5, 0.0],
        class: 49,
        master: 50,
        player: 1,
        workers: 0,
        drop_site: false,
        gap: 0.0,
        radius: [1.5, 1.5],
    };
    let rate = crate::gather::StopRate {
        work_rate: 1.0,
        capacity: 10.0,
        productivity: 1.0,
    };
    let gather = crate::gather::GatherAction {
        state: crate::gather::RESEED_WALK,
        target: -1,
        worked: true,
        ..crate::gather::GatherAction::new(task, &farm, false, rate).unwrap()
    };
    let farmer = Unit {
        id: 9175,
        position: [30.5, 88.5, 0.0],
        life: Lifecycle {
            class: Class::Animated,
            state: 2,
            hp: 25.0,
            master_id: 259,
            ..Lifecycle::default()
        },
        gather: Some(gather),
        ai: Some(UnitAi {
            ai_type: 8,
            current_order: -1,
            current_action: -1,
            current_target: -1,
            ..UnitAi::default()
        }),
        ..Unit::default()
    };
    let mut t = Turns::new(
        0,
        29910,
        1,
        vec![0, 0],
        vec![
            Player::default(),
            Player {
                objects: vec![farmer],
                resources: [0.0, 100.0, 0.0, 0.0],
                attributes: vec![0.0; 250],
                ..Player::default()
            },
        ],
    );
    t.next_object_id = 10028;
    t.masters = vec![Masters::default(), farm_masters()];
    placeable(&mut t);
    t
}

/// `FUN_140a8aa60` with the automatic farm queue: the farmer pays a farm (60 wood), its
/// foundation goes on the farm spot and the farmer builds it (as 118, its Gather gone).
#[test]
fn a_farmer_whose_farm_ran_out_reseeds_it_with_the_automatic_queue() {
    let mut t = reseed_world();
    t.players[1].auto_farm_queue = true;
    t.reseed_after_walk(1, 0, true).unwrap();
    let p = &t.players[1];
    assert_eq!(p.resources[1], 40.0);
    assert_eq!(p.farm_queue, 0);
    let foundation = p.objects.last().unwrap();
    assert_eq!(
        (
            foundation.id,
            foundation.life.master_id,
            foundation.position
        ),
        (10028, 50, [30.5, 88.5, 0.0])
    );
    let farmer = &p.objects[0];
    assert_eq!(farmer.life.master_id, 118);
    assert_eq!(farmer.build.map(|b| b.target), Some(10028));
    assert!(farmer.gather.is_none());
}

/// Without the automatic queue the farm comes from the mill's queue (one less) unpaid; with
/// it but too little wood, from the queue too.
#[test]
fn a_reseed_takes_a_queued_farm() {
    for (auto, wood) in [(false, 100.0), (true, 59.0)] {
        let mut t = reseed_world();
        t.players[1].auto_farm_queue = auto;
        t.players[1].resources[1] = wood;
        t.players[1].farm_queue = 2;
        t.reseed_after_walk(1, 0, true).unwrap();
        let p = &t.players[1];
        assert_eq!((p.resources[1], p.farm_queue), (wood, 1));
        assert_eq!(p.objects.last().unwrap().life.master_id, 50);
    }
}

/// Without a farm to make (nothing queued, no automatic queue, or no wood) the reseed makes
/// nothing: a human's farmer idles, a computer's is done.
#[test]
fn without_a_farm_to_make_the_farmer_idles() {
    let mut t = reseed_world();
    t.reseed_after_walk(1, 0, true).unwrap();
    let p = &t.players[1];
    assert_eq!(p.objects.len(), 1);
    assert_eq!(
        p.objects[0].gather.map(|g| g.state),
        Some(crate::gather::IDLE)
    );
    let mut t = reseed_world();
    t.players[1].auto_farm_queue = true;
    t.players[1].resources[1] = 59.0;
    t.players[1].computer = true;
    t.reseed_after_walk(1, 0, true).unwrap();
    let p = &t.players[1];
    assert_eq!((p.objects.len(), p.resources[1]), (1, 59.0));
    assert_eq!(
        p.objects[0].gather.map(|g| g.state),
        Some(crate::gather::DONE)
    );
}

/// A committing BUILD (`+0x1f` 0) to a builder whose unit AI holds an order not ready
/// (`FUN_140c4dfa0`): the build order joins its queue, then the readiness pass with 0
/// (`FUN_140c42b60` at 0x140e7b252) makes every queued order ready, the ready ones kept, and
/// the unit is not stopped.
#[test]
fn a_committing_build_or_wall_to_a_busy_builder_readies_its_queue() {
    let unready = crate::unit_ai::QueuedOrder {
        order: crate::unit_ai::MOVE_ORDER,
        ready: false,
        ..crate::unit_ai::QueuedOrder::default()
    };
    let mut t = Turns::new(
        0,
        29910,
        1,
        vec![0, 0],
        vec![
            Player::default(),
            Player {
                objects: vec![Unit {
                    id: 9175,
                    position: [30.0, 89.0, 0.0],
                    life: Lifecycle {
                        class: Class::Animated,
                        state: 2,
                        hp: 25.0,
                        master_id: 83,
                        ..Lifecycle::default()
                    },
                    ai: Some(UnitAi {
                        ai_type: 8,
                        current_order: crate::unit_ai::MOVE_ORDER,
                        current_action: -1,
                        current_target: -1,
                        orders: vec![unready],
                        ..UnitAi::default()
                    }),
                    ..Unit::default()
                }],
                resources: [200.0, 100.0, 200.0, 100.0],
                attributes: vec![0.0; 250],
                ..Player::default()
            },
        ],
    );
    t.next_object_id = 10028;
    t.masters = vec![Masters::default(), builder_masters()];
    placeable(&mut t);
    let mut wall = t.clone();
    t.run(&Command::Build {
        player: 1,
        master: 70,
        unit_ai: Ok(None),
        position: [30.0, 88.0, 0.0],
        builders: vec![9175],
        queued: false,
        commits: true,
    });
    assert_eq!(t.players[1].unsupported, None);
    let ai = t.players[1].objects[0].ai.as_ref().unwrap();
    let queue: Vec<_> = ai.orders.iter().map(|o| (o.order, o.ready)).collect();
    assert_eq!(
        queue,
        [(crate::unit_ai::MOVE_ORDER, true), (BUILD_ORDER, true)]
    );
    assert_eq!(ai.current_order, crate::unit_ai::MOVE_ORDER);
    // BUILD_WALL's line of one tile the same (0x140e7be4b)
    wall.run(&Command::BuildWall(crate::wall::Wall {
        player: 1,
        master: 70,
        unit_ai: Ok(None),
        from: [30, 88],
        to: [30, 88],
        target: -1,
        builders: vec![9175],
        queued: false,
        commits: true,
    }));
    assert_eq!(wall.players[1].unsupported, None);
    let ai = wall.players[1].objects[0].ai.as_ref().unwrap();
    let queue: Vec<_> = ai.orders.iter().map(|o| (o.order, o.ready)).collect();
    assert_eq!(
        queue,
        [(crate::unit_ai::MOVE_ORDER, true), (BUILD_ORDER, true)]
    );
}

/// `ClosestResourceObject`'s point (`FUN_140c49760`): without a site the search starts from the
/// point at the unit's height; (−1, −1) exactly, or a site, keeps the unit's place; one part −1
/// or not a number is a point.
#[test]
fn a_resource_search_starts_from_its_point_unless_it_is_none_or_has_a_site() {
    let unit = [5.0, 6.0, 0.5];
    assert_eq!(search_origin([12.5, 30.5], false, unit), [12.5, 30.5, 0.5]);
    assert_eq!(search_origin(crate::unit_ai::NO_POINT, false, unit), unit);
    assert_eq!(search_origin([12.5, 30.5], true, unit), unit);
    assert_eq!(search_origin([-1.0, 3.0], false, unit), [-1.0, 3.0, 0.5]);
    assert!(search_origin([f32::NAN, -1.0], false, unit)[0].is_nan());
}

/// Where a stop lands its error names what was running: a command's stop the command, an
/// object's freeze the object, its master, its top action and its unit AI's order.
#[test]
fn a_stop_s_error_names_the_command_or_the_object_it_happened_on() {
    let why = GenieError::unsupported("QUEUE at two buildings");
    let mut t = one_player(vec![town_centre(9)], [200.0; 4]);
    t.tick_with(104, &[Command::Unmodelled { player: 0, why }]);
    let (_, _, stopped) = t.players[0].unsupported.clone().unwrap();
    assert_eq!(
        stopped.context().collect::<Vec<_>>(),
        [("command", "Unmodelled")]
    );
    let task = GenieError::unsupported("a default task the slice does not model");
    let mut t = Turns::new(0, 1, 0, vec![0], vec![player(vec![])]);
    t.players[0].objects.push(Unit {
        id: 5,
        unmodelled_action: Some(task),
        life: Lifecycle {
            master_id: 83,
            ..Lifecycle::default()
        },
        ..Unit::default()
    });
    t.tick(10);
    let (_, _, frozen) = t.players[0].freezes[0].clone();
    assert_eq!(
        frozen.context().collect::<Vec<_>>(),
        [("object", "5"), ("master", "83")]
    );
}
