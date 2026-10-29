use super::*;
use crate::GenieError;
use crate::commands::game::GameCommand;
use crate::commands::{
    AllBackToWorkCommand, AttackMoveCommand, AutoScoutCommand, BuildCommand, BuildWallCommand,
    BuildingQueueCommand, Command, DeleteCommand, FlareCommand, GateCommand, GatherPoint,
    GatherPointCommand, GoBackToWork2Command, Order, OrderCommand, PatrolCommand, QueueCommand,
    ResearchCommand, ResignCommand, SeekShelterCommand, StopCommand, TownBellCommand, TradeCommand,
    TransformObjectCommand, TributeCommand, UnitAiStateCommand, UnitList, UnitOrderAction,
    UnitOrderCommand, UnloadCommand, UnmodelledCommand, Waypoints,
};
use crate::construction::build::{BuildAction, BuildTask, Construction};
use crate::construction::gates::gate_units;
use crate::gathering::closest_resource::search_origin;
use crate::gathering::gather::unit::give_load;
use crate::map::Point;
use crate::map::spatial::{Placed, Spatial};
use crate::math::rng::{Stream, Streams, Well512};
use crate::movement::ai_moves::PointMove;
use crate::movement::fly::Fly;
use crate::movement::move_to::{MoveTo, MoveToAction};
use crate::objects::ObjectId;
use crate::objects::actions::ActionList;
use crate::objects::lifecycle::death_parts::{count_class, storage_leaves};
use crate::objects::lifecycle::{Class, Lifecycle, MasterData, seconds};
use crate::objects::masters::{Master, Masters};
use crate::objects::object_list::ObjectList;
use crate::objects::release::{Releaser, released};
use crate::objects::sprite::Graphic;
use crate::objects::unit::{Unit, loses_actions};
use crate::ownership::artifact::Artifact;
use crate::ownership::capture::LIVESTOCK;
use crate::ownership::change::change_hands;
use crate::test_ids::master_id;
use crate::test_ids::object_id;
use crate::test_ids::object_ids;
use crate::test_ids::player_id;
use crate::test_ids::unit_list;
use crate::unit_ai::ActionKind;
use crate::unit_ai::OrderKind;
use crate::unit_ai::task_finder::world::{
    SideWorld, TaskWorld, task_search, task_target_with, wait_progress,
};
use crate::unit_ai::task_finder::{Actor, Diplomacy};
use crate::unit_ai::world_view::{DropWorld, Foreign, Wait, patrol_information};
use crate::unit_ai::{Notify, OWNER_CHANGED, UnitAi};
use crate::vision::visibility::{Seer, capture_kind};
use crate::world::checkpoint::TopAction;
use crate::world::lets::Lets;
use crate::world::others::{Others, OthersRef};
use crate::world::player::Player;

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
        current_order: None,
        current_action: None,
        current_target: -1,
        adjusted_idle_timeout: 3000,
        look_around_timeout: 1000,
        retarget_timer: 1,
        ..UnitAi::default()
    }
}

#[test]
fn the_turn_rotates_and_only_the_turn_player_updates() {
    let mut t = World::new(
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
    let mut t = World::new(0, 29910, 0, vec![0], vec![player(vec![])]);
    t.tick(10);
    // SeedAll(29910); s1 = World.rand15 = 19550 (see rng); SeedAll(19550); World.rand15
    let mut s = Streams::default();
    s.seed_all(19550);
    assert_eq!(t.frame_seed, s.get(Stream::World).rand15());
}

#[test]
fn artifacts_of_every_player_start_updating_in_the_second_tick() {
    let animal = |gaia| Unit {
        actions: ActionList::default().with_artifact(Some(Artifact::new(gaia, None))),
        ..Unit::default()
    };
    let players = vec![
        Player {
            objects: vec![animal(true)].into(),
            ..Player::default()
        },
        Player {
            objects: vec![animal(false)].into(),
            ..Player::default()
        },
    ];
    let mut t = World::new(0, 1, 0, vec![0, 0], players);
    let state = |t: &World, p: usize| {
        t.players[p].objects[0]
            .actions
            .artifact()
            .copied()
            .unwrap()
            .state
    };
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
    let mut t = World::new(0, 1, 0, vec![0], vec![player(vec![Some(odd)])]);
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

fn ids(t: &World) -> Vec<i32> {
    t.players[0].objects.iter().map(|o| o.id).collect()
}

fn retarget(t: &World) -> Vec<u32> {
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
    let mut t = World::new(
        0,
        29910,
        0,
        vec![0],
        vec![Player {
            objects: vec![dead_building(), live(2), live(3)].into(),
            ..Player::default()
        }],
    );
    for _ in 0..4 {
        t.tick(10);
    }
    assert_eq!(ids(&t), [3, 2]);
    // the same ticks with the two live buildings in that order from the start: their timers
    // fall due in the fourth tick, so its draws depend only on the order
    let mut reordered = World::new(
        0,
        29910,
        0,
        vec![0],
        vec![Player {
            objects: vec![live(3), live(2)].into(),
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
        let mut t = World::new(
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
    let with_dead = one(vec![dead_building(), live(2)].into());
    let alone = one(vec![live(2)].into());
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
pub(crate) fn builder_masters() -> Masters {
    use crate::objects::masters::{Master, Masters, Task};
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
            base: 0.8,
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
        // no sprites (the world's sprites' table is empty)
        standing_sprite: -1,
        move_sprite: -1,
        run_sprite: -1,
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
            building: Some(crate::objects::masters::BuildingMaster {
                stack_unit: -1,
                foundation_terrain: -1,
                tech: -1,
                annexes: [-1; 4],
                head_unit: -1,
                transform_unit: -1,
                ..crate::objects::masters::BuildingMaster::default()
            }),
            ..Master::default()
        },
    ])
}

/// A flat map of terrain 0 that the first terrain restriction lets anything stand on, every
/// tile seen by every player: BUILD's placement test passes anywhere free.
pub(crate) fn placeable(t: &mut World) {
    let (w, h) = (120, 120);
    let n = (w * h) as usize;
    t.spatial.map = [w, h];
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
    let players = t.players.len();
    let seen = (0..players).fold(0u64, |b, p| b | (1 << p) | (1 << (32 + p)));
    t.spatial.sight = Some(crate::vision::visibility::Visibility::new(
        w,
        h,
        (0..players)
            .map(|_| crate::vision::visibility::VisibleMap {
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

/// `FUN_140c42080` after a notify of a new action: a unit AI with a target the world holds
/// remembers the order with the world's time, both places as slot 0x520 gives them (a
/// [`GameVector`]; these hold no movement state) and the target's attack category by its master
/// (an archer, class 0: 3); one whose target is gone, or on a point, remembers none.
///
/// [`GameVector`]: crate::math::game_vector::GameVector
#[test]
fn an_order_on_a_target_joins_the_unit_ai_s_history() {
    use crate::math::game_vector::GameVector;
    let master = |id, class| Master {
        id,
        kind: 70,
        class,
        ..Master::default()
    };
    let unit = |id, position, target| Unit {
        id,
        body: crate::objects::layers::Body::at(position),
        ai: Some(UnitAi {
            current_target: target,
            ..UnitAi::default()
        }),
        life: Lifecycle {
            master_id: if id == 1 { 74 } else { 4 },
            ..Lifecycle::default()
        },
        ..Unit::default()
    };
    let player = |objects| Player {
        objects,
        attributes: vec![0.0; 300],
        ..Player::default()
    };
    let mut t = World::new(
        0,
        1,
        0,
        vec![0, 0, 0],
        vec![
            player(vec![].into()),
            player(vec![unit(1, [10.0, 11.0, 0.0], 2), unit(3, [1.0, 1.0, 0.0], 99)].into()),
            player(vec![unit(2, [20.0, 21.0, 1.0], -1)].into()),
        ],
    );
    let table = Masters::new([master(74, 6), master(4, 0)]);
    t.masters = vec![table.clone(), table.clone(), table];
    t.world_time = 4321;
    t.remember_order(1, 0, (Some(OrderKind::Attack), Some(ActionKind::Attack)))
        .unwrap();
    t.remember_order(1, 1, (Some(OrderKind::Attack), Some(ActionKind::Attack)))
        .unwrap();
    let history = |i: usize| {
        t.players[1].objects[i]
            .ai
            .as_ref()
            .unwrap()
            .order_history
            .clone()
    };
    assert_eq!(
        history(0),
        [crate::unit_ai::OrderRecord {
            order: Some(OrderKind::Attack),
            action: Some(ActionKind::Attack),
            time: 4321,
            position: GameVector::new(10.0, 0.0, 11.0),
            target: 2,
            category: 3,
            target_position: GameVector::new(20.0, 1.0, 21.0),
        }]
    );
    assert_eq!(history(1), []);
}

/// FLARE (`0x140e7bfd0`) at a point without a target: each player's flare comes at the point
/// with height 0, which the static `new_position` (slot 0x58, slot 0x60 `FUN_141193a10`) puts
/// on the ground, as 426048706's flare of update 2486 stands at 4 on raised ground.
#[test]
fn a_flare_at_a_point_stands_on_the_ground() {
    let player = || Player {
        attributes: vec![0.0; 300],
        ..Player::default()
    };
    let mut t = World::new(0, 1, 0, vec![0, 0, 0], vec![player(), player(), player()]);
    placeable(&mut t);
    if let Some(terrain) = t.spatial.terrain.as_mut() {
        terrain.set_elevations(vec![4; 120 * 120]);
    }
    let flare = Master {
        id: 274,
        kind: crate::objects::masters::master_type::ANIMATED,
        standing_sprite: -1,
        ..Master::default()
    };
    t.masters = (0..3).map(|_| Masters::new([flare.clone()])).collect();
    t.next_object_id = 100;
    t.flare(None, Point::new(19.114_586, 29.197_92), 3).unwrap();
    for p in 1..3 {
        let at: Vec<[f32; 3]> = t.players[p].objects.iter().map(Unit::position).collect();
        assert_eq!(at, [[19.114_586, 29.197_92, 4.0]]);
    }
}

/// A hit on player 2's object makes player 2 a flare (master 112) at the attacker's place
/// (`FUN_14120c0a0`'s slot 0x1c0), whose amount (`+0x310`, its time) a trebuchet's hit doubles.
#[test]
fn a_hit_s_flare_lasts_twice_as_long_from_a_trebuchet() {
    let player = || Player {
        attributes: vec![0.0; 300],
        ..Player::default()
    };
    let mut t = World::new(0, 1, 0, vec![0, 0, 0], vec![player(), player(), player()]);
    placeable(&mut t);
    let flare = Master {
        id: 112,
        kind: crate::objects::masters::master_type::ANIMATED,
        standing_sprite: -1,
        storage: vec![(12, 1.0, 0)],
        ..Master::default()
    };
    t.masters = (0..3).map(|_| Masters::new([flare.clone()])).collect();
    t.next_object_id = 100;
    t.make_flare(2, [10.5, 12.5, 0.0], false).unwrap();
    t.make_flare(2, [10.5, 12.5, 0.0], true).unwrap();
    let flares: Vec<(i16, f32)> = t.players[2]
        .objects
        .iter()
        .map(|u| (u.life.master_id, u.life.amount))
        .collect();
    assert_eq!(flares, [(112, 1.0), (112, 2.0)]);
}

/// Player 1 with 300 attributes, `livestock` of them at a pasture 50 (master 1897, radius 2) at
/// `at` and 300 food, on a 120-tile map, its masters `livestock_masters` of the two livestock
/// (moving, one hit point, a turning idle sprite of 16 facets with a random first one, 100 food
/// held).
fn pasture_game(at: [f32; 3], livestock: f32, livestock_masters: &[i16]) -> World {
    use crate::objects::masters::master_type::{BUILDING, MOVING};
    use crate::objects::sprite::Graphic;
    let player = || Player {
        attributes: vec![0.0; 300],
        ..Player::default()
    };
    let mut t = World::new(0, 1, 0, vec![0, 0], vec![player(), player()]);
    t.spatial = Spatial::new([120, 120]);
    placeable(&mut t);
    t.players[1].attributes[289] = livestock;
    t.players[1].attributes[288] = 300.0;
    let idle = |id| Graphic {
        id,
        frames: 60,
        facets: 16,
        speed_multiplier: 0.0,
        frame_duration: 0.1,
        replay_delay: 0.0,
        flags: 7,
    };
    t.graphics = std::collections::BTreeMap::from([(13124, idle(13124)), (13170, idle(13170))]);
    let pasture = Master {
        id: 1897,
        kind: BUILDING,
        class: 49,
        radius: [2.0, 2.0],
        ..Master::default()
    };
    let animal = |id, standing_sprite| Master {
        id,
        kind: MOVING,
        class: 49,
        hit_points: 1,
        radius: [0.2, 0.2],
        standing_sprite,
        storage: vec![(0, 100.0, 0)],
        moving: Some(crate::gathering::hunt::tests::walker().stats),
        ..Master::default()
    };
    let mut masters = vec![pasture];
    for &m in livestock_masters {
        masters.push(animal(m, if m == 1899 { 13124 } else { 13170 }));
    }
    t.masters = (0..2).map(|_| Masters::new(masters.clone())).collect();
    t.players[1].objects.push(Unit {
        id: 50,
        body: crate::objects::layers::Body::at(at),
        life: Lifecycle {
            master_id: 1897,
            ..Lifecycle::default()
        },
        pasture: -1,
        my_pasture: -1,
        ..Unit::default()
    });
    t.next_object_id = 100;
    t
}

/// `FUN_1411c8410` (429381190's pasture 15342 of update 1385 made three): per animal two draws of
/// the Object stream pick its master and its point along the edges, a point on a tile that holds
/// one of the pasture's livestock goes and another is drawn; the new object's setups draw its
/// facet and its angle, then the routine its first facet. Each is the pasture's, holds its share
/// of the food, no unit targets it, and the pasture lists it.
#[test]
fn a_completed_pasture_puts_its_livestock_on_its_edges() {
    use crate::gathering::pasture::{LIVESTOCK, edge_points};
    let mut t = pasture_game([20.0, 20.0, 0.0], 3.0, &[1899, 1900]);
    t.streams.get(Stream::Object).set_seed(1385);
    let mut e = t.streams.get(Stream::Object).clone();
    let mut points = edge_points([20.0, 20.0, 0.0], 2.0);
    let mut tiles = Vec::new();
    let mut expected = Vec::new();
    for _ in 0..3 {
        let kind = LIVESTOCK[e.rand15() as usize % 2];
        let mut i = e.rand15() as usize % points.len();
        let tile = |p: [f32; 2]| (p[0] as i32, p[1] as i32);
        while tiles.contains(&tile(points[i])) {
            let _ = points.remove(i);
            i = e.rand15() as usize % points.len();
        }
        tiles.push(tile(points[i]));
        expected.push((kind, points.remove(i)));
        // the static setup's facet, the moving setup's angle, the routine's facet
        for _ in 0..3 {
            let _ = e.rand15();
        }
    }
    t.setup_pasture(1, 50).unwrap();
    let made: Vec<(i16, [f32; 2])> = t.players[1].objects[1..]
        .iter()
        .map(|u| (u.life.master_id, [u.position()[0], u.position()[1]]))
        .collect();
    assert_eq!(made, expected);
    assert_eq!(t.players[1].objects[0].livestock, [100, 101, 102]);
    for u in &t.players[1].objects[1..] {
        assert_eq!(
            (
                u.pasture,
                u.life.amount,
                u.life.attribute_type,
                u.untargetable
            ),
            (50, 100.0, 0, true)
        );
        assert_eq!(u.life.state, 2);
        assert!(u.sprite.is_some() && u.body.mover().is_some());
    }
    assert_eq!(*t.streams.get(Stream::Object), e);
}

/// One livestock a tile: a pasture at (20, 20) has twelve tiles under its sixteen points, so
/// sixteen asked give twelve and the points run out; at the map's left edge the points off it
/// (x below 0 after CVTTSS2SI: −1.5, not −0.6) go too, leaving six tiles.
#[test]
fn a_pasture_puts_one_animal_a_tile_on_the_map() {
    let mut t = pasture_game([20.0, 20.0, 0.0], 16.0, &[1899, 1900]);
    t.setup_pasture(1, 50).unwrap();
    let tiles = |t: &World| {
        let mut v: Vec<(i32, i32)> = t.players[1].objects[1..]
            .iter()
            .map(|u| (u.position()[0] as i32, u.position()[1] as i32))
            .collect();
        v.sort_unstable();
        v
    };
    let mut twelve = tiles(&t);
    assert_eq!(twelve.len(), 12);
    twelve.dedup();
    assert_eq!(twelve.len(), 12);
    let mut t = pasture_game([0.0, 10.0, 0.0], 16.0, &[1899, 1900]);
    t.setup_pasture(1, 50).unwrap();
    assert_eq!(
        tiles(&t),
        [(0, 8), (0, 11), (1, 8), (1, 9), (1, 10), (1, 11)]
    );
}

/// A master the owner has not got makes nothing but counts as an attempt, its point staying; no
/// livestock asked draws nothing; the food is the pasture's over the count (DIVSS).
#[test]
fn a_pasture_counts_each_attempt_and_shares_its_food() {
    use crate::gathering::pasture::{LIVESTOCK, edge_points};
    let mut t = pasture_game([20.0, 20.0, 0.0], 3.0, &[1899]);
    t.streams.get(Stream::Object).set_seed(7);
    let mut e = t.streams.get(Stream::Object).clone();
    let mut points = edge_points([20.0, 20.0, 0.0], 2.0);
    let mut expected = Vec::new();
    for _ in 0..3 {
        let kind = LIVESTOCK[e.rand15() as usize % 2];
        let i = e.rand15() as usize % points.len();
        let tile = |p: [f32; 2]| (p[0] as i32, p[1] as i32);
        assert!(
            !expected.iter().any(|&q| tile(q) == tile(points[i])),
            "the seed's points"
        );
        if kind == 1899 {
            expected.push(points.remove(i));
            for _ in 0..3 {
                let _ = e.rand15();
            }
        }
    }
    assert!(expected.len() < 3, "the seed draws an Ibex");
    t.setup_pasture(1, 50).unwrap();
    let made: Vec<[f32; 2]> = t.players[1].objects[1..]
        .iter()
        .map(|u| [u.position()[0], u.position()[1]])
        .collect();
    assert_eq!(made, expected);
    assert_eq!(*t.streams.get(Stream::Object), e);
    let mut t = pasture_game([20.0, 20.0, 0.0], 0.0, &[1899, 1900]);
    t.setup_pasture(1, 50).unwrap();
    assert_eq!(
        (
            t.players[1].objects.len(),
            t.streams.get(Stream::Object).draws
        ),
        (1, 0)
    );
    let mut t = pasture_game([20.0, 20.0, 0.0], 7.0, &[1899, 1900]);
    t.setup_pasture(1, 50).unwrap();
    let food = 300.0f32 / 7.0;
    assert!(
        t.players[1].objects[1..]
            .iter()
            .all(|u| u.life.amount == food)
    );
}

/// A moving object of type 30 that dies (429381190's Ibex 15371 in update 1426) shows its
/// master's dying sprite (slot 0x7a0, `FUN_1411887e0`, through slot 0x68) and goes on to state 4.
/// Its top speed (slot 65, `FUN_14114d1d0`) follows the sprite: the master's speed, times the
/// sprite's speed factor above 0.
#[test]
fn a_dying_livestock_shows_its_dying_sprite() {
    for (factor, speed) in [(0.0, 0.737), (2.0, 1.474)] {
        let u = dying_animal(crate::objects::masters::master_type::MOVING, factor);
        assert_eq!(u.sprite.map(|s| s.graphic.id), Some(13171));
        assert_eq!(u.life.state, 4);
        assert_eq!(u.body.mover().map(|m| m.stats.speed), Some(speed));
    }
}

/// A livestock of [`pasture_game`] of master type `kind` (speed 0.737, the dying sprite 13171 of
/// speed factor `factor`) running at 1.4003 when it dies, two updates later (the deer 9655 of
/// MarvinBlue/426048731, a combat object, likewise in update 3147: the Gaia pin of
/// `oracle_snapshots`).
fn dying_animal(kind: u8, factor: f32) -> Unit {
    let mut t = pasture_game([20.0, 20.0, 0.0], 1.0, &[1899, 1900]);
    let dying = Graphic {
        id: 13171,
        frames: 10,
        facets: 16,
        speed_multiplier: factor,
        frame_duration: 0.1,
        replay_delay: 0.0,
        flags: 7,
    };
    let _ = t.graphics.insert(13171, dying);
    for m in &mut t.masters {
        let mut all: Vec<Master> = m.iter().cloned().collect();
        for a in all
            .iter_mut()
            .filter(|a| a.kind == crate::objects::masters::master_type::MOVING)
        {
            a.kind = kind;
            a.death_sprite = 13171;
            a.max_speed = 0.737;
        }
        *m = Masters::new(all);
    }
    t.setup_pasture(1, 50).unwrap();
    let animal = t.players[1].objects[1].id;
    t.players[1].objects[1].life.hp = 0.0;
    if let Some(m) = t.players[1].objects[1].body.mover_mut() {
        m.stats.speed = 1.4003;
    }
    for _ in 0..2 {
        t.tick(50);
    }
    t.object(animal).unwrap().1.clone()
}

/// [`pasture_game`] with its three livestock made and player 1's villager 60 (class 4, radius
/// 0.2) at `at`, the owner's [`crate::gathering::pasture::PASTURE_WORKERS`] `room`.
fn tended_pasture(at: [f32; 3], room: f32) -> World {
    let mut t = pasture_game([20.0, 20.0, 0.0], 3.0, &[1899, 1900]);
    t.setup_pasture(1, 50).unwrap();
    t.players[1].attributes[290] = room;
    let villager = Master {
        id: 83,
        kind: 70,
        class: 4,
        radius: [0.2, 0.2],
        ..Master::default()
    };
    for m in &mut t.masters {
        let mut all: Vec<Master> = m.iter().cloned().collect();
        all.push(villager.clone());
        *m = Masters::new(all);
    }
    t.players[1].objects.push(Unit {
        id: 60,
        body: crate::objects::layers::Body::at(at),
        life: Lifecycle {
            master_id: 83,
            ..Lifecycle::default()
        },
        ai: Some(UnitAi::default()),
        pasture: -1,
        my_pasture: -1,
        ..Unit::default()
    });
    t
}

/// `FUN_1411c7670`: the pasture's livestock sorted by their footprints' gap to the villager;
/// the first without workers, else the first whose workers are below the owner's attribute
/// 290, else none; a unit of another player gets none. Slot 0x780 sums the livestock's workers.
#[test]
fn a_villager_tends_the_nearest_livestock_with_room() {
    let livestock = |t: &World| t.players[1].objects[0].livestock.clone();
    let place = |t: &World, id: i32| t.object(id).unwrap().1.position();
    let t = tended_pasture([20.0, 20.0, 0.0], 2.0);
    let ids = livestock(&t);
    let near = place(&t, ids[0]);
    let mut t = tended_pasture([near[0], near[1], 0.0], 2.0);
    let villager = t.players[1].objects.len() - 1;
    assert_eq!(t.pasture_pick(50, 1, villager), Ok(ids[0]));
    let mut gaps: Vec<(f32, i32)> = ids
        .iter()
        .map(|&id| {
            let p = place(&t, id);
            let g = crate::movement::footprint_gap_squared(p, [0.2, 0.2], near, [0.2, 0.2]);
            (g, id)
        })
        .collect();
    gaps.sort_by(|a, b| a.0.total_cmp(&b.0));
    let by_gap: Vec<i32> = gaps.iter().map(|g| g.1).collect();
    let worked = |t: &mut World, id: i32, n: i32| t.object_mut(id).unwrap().1.workers = n;
    worked(&mut t, by_gap[0], 1);
    assert_eq!(t.pasture_pick(50, 1, villager), Ok(by_gap[1]));
    worked(&mut t, by_gap[1], 1);
    worked(&mut t, by_gap[2], 1);
    assert_eq!(t.livestock_workers(50), 3);
    // all worked: the nearest below the room
    assert_eq!(t.pasture_pick(50, 1, villager), Ok(by_gap[0]));
    worked(&mut t, by_gap[0], 2);
    assert_eq!(t.pasture_pick(50, 1, villager), Ok(by_gap[1]));
    for &id in &by_gap {
        worked(&mut t, id, 2);
    }
    assert_eq!(t.pasture_pick(50, 1, villager), Ok(-1));
    let mut other = tended_pasture(near, 2.0);
    let v = other.players[1].objects.pop().unwrap();
    other.players.push(Player {
        attributes: vec![0.0; 300],
        objects: vec![v].into(),
        ..Player::default()
    });
    other.masters.push(other.masters[1].clone());
    assert_eq!(other.pasture_pick(50, 2, 0), Ok(-1));
}

/// Slot 0x7e8 of a villager on a pasture: with room (the livestock's workers below attribute
/// 290) it works the livestock `FUN_1411c7670` picks, at its place, and tends the pasture; a
/// full one has its unit AI take a search (0x202 with the hunt action, class 49, −1) and it
/// tends none.
#[test]
fn a_villager_ordered_on_a_pasture_works_its_livestock() {
    let order = |target| crate::orders::Work {
        target,
        point: [20.0, 20.0, 0.0],
        carry_test: true,
        filter: -1,
        notified: false,
    };
    let mut t = tended_pasture([18.0, 18.0, 0.0], 2.0);
    let v = t.players[1].objects.len() - 1;
    t.order_work(1, v, &order(50)).unwrap();
    assert_eq!(t.players[1].objects[v].my_pasture, 50);
    assert_eq!(t.players[1].objects[0].pasture_workers, [60]);
    let mut full = tended_pasture([18.0, 18.0, 0.0], 0.0);
    let v = full.players[1].objects.len() - 1;
    full.players[1].objects[v].my_pasture = 50;
    full.players[1].objects[0].pasture_workers = vec![60];
    full.order_work(1, v, &order(50)).unwrap();
    let u = &full.players[1].objects[v];
    assert_eq!(
        u.ai.as_ref().unwrap().notifies,
        [Notify::own(
            60,
            crate::unit_ai::REQUIRES_SEARCH,
            [0x265, 49, -1]
        )]
    );
    assert_eq!(u.my_pasture, -1);
    assert_eq!(
        full.players[1].objects[0].pasture_workers,
        Vec::<i32>::new()
    );
}

/// Slot 0x7e8 of a unit in state 3 or later ignores the order: nothing of the pasture's or the
/// unit's changes.
#[test]
fn a_unit_past_state_2_ignores_a_work_order() {
    let order = crate::orders::Work {
        target: 50,
        point: [20.0, 20.0, 0.0],
        carry_test: true,
        filter: -1,
        notified: false,
    };
    let mut t = tended_pasture([18.0, 18.0, 0.0], 2.0);
    let v = t.players[1].objects.len() - 1;
    t.players[1].objects[v].life.state = 3;
    let before = t.players[1].objects.clone();
    t.order_work(1, v, &order).unwrap();
    assert_eq!(t.players[1].objects, before);
}

/// Slot 0x7e8 of a villager on a pasture with room but no livestock: `FUN_1411c7670` gives
/// none and the order ends there, before `FUN_141189ca0`: the pasture the villager tended stays
/// its own.
#[test]
fn a_villager_ordered_on_a_pasture_without_livestock_tends_on() {
    let order = crate::orders::Work {
        target: 50,
        point: [20.0, 20.0, 0.0],
        carry_test: true,
        filter: -1,
        notified: false,
    };
    let mut t = tended_pasture([18.0, 18.0, 0.0], 2.0);
    let v = t.players[1].objects.len() - 1;
    t.players[1].objects[0].livestock.clear();
    t.players[1].objects[v].my_pasture = 50;
    t.players[1].objects[0].pasture_workers = vec![60];
    t.order_work(1, v, &order).unwrap();
    assert_eq!(t.players[1].objects[v].my_pasture, 50);
    assert_eq!(t.players[1].objects[0].pasture_workers, [60]);
}

/// `FUN_141189ca0`: a unit tending a pasture joins its workers once; tending none drops it
/// from the pasture it tended (`FUN_1411cafa0`); an object that is no building keeps no list.
#[test]
fn a_unit_joins_and_leaves_a_pasture_s_workers() {
    let mut t = tended_pasture([18.0, 18.0, 0.0], 2.0);
    let v = t.players[1].objects.len() - 1;
    t.tend_pasture(1, v, 50).unwrap();
    t.tend_pasture(1, v, 50).unwrap();
    assert_eq!(t.players[1].objects[0].pasture_workers, [60]);
    assert_eq!(t.players[1].objects[v].my_pasture, 50);
    t.tend_pasture(1, v, -1).unwrap();
    assert_eq!(t.players[1].objects[0].pasture_workers, Vec::<i32>::new());
    assert_eq!(t.players[1].objects[v].my_pasture, -1);
    let animal = t.players[1].objects[1].id;
    t.tend_pasture(1, v, animal).unwrap();
    assert_eq!(t.players[1].objects[1].pasture_workers, Vec::<i32>::new());
    assert_eq!(t.players[1].objects[v].my_pasture, animal);
}

/// `Process_New_Tiles`: player 1's newly explored tiles, in order, give the objects of its
/// masters that gather by themselves to its lists by group, at their places.
#[test]
fn a_player_s_new_tiles_give_their_resources_to_its_lists() {
    use crate::objects::masters::{Master, Masters};
    use crate::vision::visibility::{VisibleResource, VisibleResources};
    let object = |id, master, x, y| Unit {
        id,
        body: crate::objects::layers::Body::at([x, y, 0.0]),
        life: Lifecycle {
            class: Class::Static,
            state: 2,
            master_id: master,
            ..Lifecycle::default()
        },
        ..Unit::default()
    };
    let mut t = World::new(
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
                ]
                .into(),
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
        crate::map::spatial::Tiles::new(120, 120, vec![0; 120 * 120]),
    );
    placeable(&mut t);
    for u in &t.players[0].sleeping {
        t.spatial.insert(
            u.id,
            Placed {
                position: u.position(),
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
    use crate::objects::lifecycle::MasterData;
    use crate::objects::masters::{Master, Masters};
    use crate::vision::visibility::VisibleResources;
    let dead = |id, class, state, master| Unit {
        id,
        body: crate::objects::layers::Body::at([10.5, 10.5, 0.0]),
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
    let mut t = World::new(
        0,
        1,
        1,
        vec![0, 0],
        vec![
            Player {
                objects: vec![
                    dead(501, Class::Static, 3, 411),
                    dead(502, Class::Animated, 5, 412),
                ]
                .into(),
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
    let held = |t: &World| -> Vec<(usize, usize)> {
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
    use crate::objects::masters::{Master, Masters};
    use crate::vision::visibility::{VisibleResource, VisibleResources};
    let made = |id, master| Unit {
        id,
        body: crate::objects::layers::Body::at([20.75, 30.25, 0.0]),
        life: Lifecycle {
            state: 2,
            master_id: master,
            ..Lifecycle::default()
        },
        ..Unit::default()
    };
    let mut t = World::new(
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
    use crate::objects::masters::Master;
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
    use crate::objects::sprite::{ActiveSprite, Graphic};
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
    u.idle_swap(
        master.standing_sprite,
        master.standing_sprite_2,
        &graphics,
        &mut r,
    );
    assert_eq!(u.idle_variant, 1);
    let s = u.sprite.unwrap();
    assert_eq!((s.graphic.id, s.frame), (101, 0));
    assert_eq!(r.draws, 1 + rng(high).draws);
    // a low draw keeps the sprite
    let mut u = Unit {
        sprite: Some(shown(100, true)),
        ..Unit::default()
    };
    u.idle_swap(
        master.standing_sprite,
        master.standing_sprite_2,
        &graphics,
        &mut rng(low),
    );
    assert_eq!((u.idle_variant, u.sprite.unwrap().graphic.id), (0, 100));
    // no loop, another sprite, no second standing sprite: no draw
    for (sprite, m) in [
        (shown(100, false), master.clone()),
        (shown(7, true), master.clone()),
        (
            shown(100, true),
            Master {
                standing_sprite_2: None,
                ..master
            },
        ),
    ] {
        let mut u = Unit {
            sprite: Some(sprite),
            ..Unit::default()
        };
        let mut r = rng(high);
        u.idle_swap(m.standing_sprite, m.standing_sprite_2, &graphics, &mut r);
        assert_eq!(r.draws, rng(high).draws);
    }
}

/// Player 1's two idle villagers (9175 at the house's place, 9181 away), its resources and
/// the builders' masters, the house placeable: the BUILD tests' world.
fn build_world() -> World {
    let villager = |id, master, x, y| Unit {
        id,
        body: crate::objects::layers::Body::at([x, y, 0.0]),
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
            current_order: None,
            current_action: None,
            current_target: -1,
            ..UnitAi::default()
        }),
        ..Unit::default()
    };
    let mut t = World::new(
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
                ]
                .into(),
                resources: [200.0, 100.0, 200.0, 100.0],
                attributes: vec![0.0; 250],
                ..Player::default()
            },
        ],
    );
    t.next_object_id = 10028;
    t.masters = vec![Masters::default(), builder_masters()];
    placeable(&mut t);
    t
}

/// BUILD's handler (`FUN_140e7a400`) does nothing for a human's packet with a target (`+0x14`);
/// a house of civ 12 keeps the packet's sprite override (`+0x18`, 0x2c9 to 0x2ce), any other
/// none.
#[test]
fn a_human_build_with_a_target_places_nothing_and_a_house_keeps_its_sprite_override() {
    let build = |target, sprite_override| {
        Command::Build(BuildCommand {
            player: player_id(1),
            master: master_id(70),
            position: Point::new(30.0, 88.0),
            q_id: target,
            sprite_override,
            builders: unit_list(object_ids([9175, 9181])),
            queued: false,
            commits: false,
            frame: 0,
        })
    };
    let mut t = build_world();
    let before = t.players[1].objects.len();
    t.run(&build(3, -1));
    assert_eq!(t.players[1].objects.len(), before);
    let overridden = |civ, sprite_override| {
        let mut t = build_world();
        t.players[1].civ = civ;
        t.run(&build(-1, sprite_override));
        let foundation = t.players[1].objects.last().unwrap();
        foundation.construction.as_ref().unwrap().sprite_override
    };
    assert!(overridden(12, 0x2c9));
    assert!(!overridden(12, 0x2cf));
    assert!(!overridden(11, 0x2c9));
    assert!(!overridden(12, -1));
}

#[test]
fn a_build_pays_the_cost_and_tasks_the_builders_of_its_own_player() {
    let mut t = build_world();
    t.tick_with(
        50,
        &[Command::Build(BuildCommand {
            player: player_id(1),
            master: master_id(70),
            position: Point::new(30.0, 88.0),
            q_id: -1,
            sprite_override: -1,
            builders: unit_list(object_ids([9175, 9181])),
            queued: false,
            commits: false,
            frame: 0,
        })],
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
            foundation.position()
        ),
        (10028, 70, [30.0, 88.0, 0.0])
    );
    // an animated object's constructor (FUN_141070cf0) sets its animation speed (+0x7b8) to 0:
    // its sprite's clock runs from its first update (a dying sprite plays out)
    assert_eq!(foundation.body.anim_speed(), Some(0.0));
    let builder = |i: usize| {
        (
            p.objects[i].life.master_id,
            p.objects[i].actions.build().map(|b| b.state),
        )
    };
    // the builder at the foundation works in its first update; the other asks for a path, for
    // which these villagers lack their movement state
    assert_eq!((builder(0), builder(1)), ((118, Some(6)), (212, Some(4))));
    // each idle villager's town-bell record: no work, at its own place
    assert_eq!(
        p.objects[1].town_bell.work,
        crate::orders::town_bell::BackToWork {
            point: [29.0, 99.0],
            ..crate::orders::town_bell::BackToWork::NONE
        }
    );
    assert_eq!(p.unsupported, Some((1, 1, WALKER_UNMOVING)));
}

/// BUILD's direct path is the builder's slot 0x128 with 1 (`FUN_141211b80`, a combat object's
/// and a building's): its unit AI's order queue empties (`FUN_140c5e010`) before the work goes
/// on. 430020666's villager 6676 drops the move it queued in update 40 when BUILD sends it to
/// another house in update 119.
#[test]
fn a_direct_build_empties_its_builder_s_order_queue() {
    let mut t = build_world();
    let queued_move = crate::unit_ai::QueuedOrder {
        issuer: 1,
        order: OrderKind::Move,
        priority: 100,
        target: -1,
        target_owner: -1,
        point: [19.760_416, 101.677_086, 0.0],
        range: 0.0,
        shown: [19.760_416, 102.177_086],
        ready: true,
        target_type: -1,
    };
    for u in &mut t.players[1].objects {
        u.ai.as_mut().unwrap().orders.push(queued_move);
    }
    // the command phase alone (the units' updates would run the orders)
    t.run(&Command::Build(BuildCommand {
        player: player_id(1),
        master: master_id(70),
        position: Point::new(30.0, 88.0),
        q_id: -1,
        sprite_override: -1,
        builders: unit_list(object_ids([9175])),
        queued: false,
        commits: false,
        frame: 0,
    }));
    let unit = |id: i32| t.object(id).map(|(_, u)| u).unwrap();
    let orders = |id: i32| unit(id).ai.as_ref().map_or(0, |ai| ai.orders.len());
    assert!(unit(9175).actions.build().is_some());
    // the builder's queue empties; the villager BUILD did not name keeps its own
    assert_eq!((orders(9175), orders(9181)), (0, 1));
}

/// census v5's 257 stops "a builder of another player or a sleeping one": BUILD's last
/// loop (`FUN_140e7a400`) takes the packet's units the world holds whatever their player: a
/// queued BUILD puts the build order into another player's villager's queue, issued by its
/// own player and naming the BUILD's player as the target's; a unit of a master that does
/// not move is passed over unless the BUILD is queued. Another player's unit reaches the
/// handler only while players may order each other's (`do_command`'s check, the setting
/// `+0x1074`).
#[test]
fn a_queued_build_queues_its_order_for_another_player_s_villager() {
    let villager = |id, master, x, y| Unit {
        id,
        body: crate::objects::layers::Body::at([x, y, 0.0]),
        life: Lifecycle {
            class: Class::Animated,
            state: 2,
            hp: 25.0,
            master_id: master,
            ..Lifecycle::default()
        },
        ai: Some(UnitAi {
            ai_type: 8,
            current_order: None,
            current_action: None,
            current_target: -1,
            ..UnitAi::default()
        }),
        ..Unit::default()
    };
    let mut t = World::new(
        0,
        29910,
        1,
        vec![0, 0, 0],
        vec![
            Player::default(),
            Player {
                objects: vec![villager(9175, 83, 30.0, 89.0)].into(),
                resources: [200.0, 100.0, 200.0, 100.0],
                attributes: vec![0.0; 250],
                ..Player::default()
            },
            Player {
                objects: vec![villager(9618, 293, 29.0, 99.0)].into(),
                attributes: vec![0.0; 250],
                ..Player::default()
            },
        ],
    );
    t.next_object_id = 10028;
    t.masters = vec![Masters::default(), builder_masters(), builder_masters()];
    placeable(&mut t);
    t.settings.cheats_enabled = true;
    // the command phase alone (the units' updates would run the orders)
    t.run(&Command::Build(BuildCommand {
        player: player_id(1),
        master: master_id(70),
        position: Point::new(30.0, 88.0),
        q_id: -1,
        sprite_override: -1,
        builders: unit_list(object_ids([9618, 9175])),
        queued: true,
        commits: false,
        frame: 2,
    }));
    // the foundation keeps the command's frame (+0xc54)
    assert_eq!(t.object(10028).map(|(_, u)| u.build_frame), Some(2));
    let order = |p: usize| {
        t.players[p].objects[0]
            .ai
            .as_ref()
            .and_then(|ai| ai.orders.first().copied())
            .map(|o| (o.issuer, o.order, o.target, o.target_owner))
    };
    assert_eq!(order(2), Some((2, OrderKind::Build, 10028, 1)));
    assert_eq!(order(1), Some((1, OrderKind::Build, 10028, 1)));
}

/// A world of player 1's villager 9175 (master 83, which has no Build task) beside its
/// house's foundation 10028 (master 70, state 0, no build points yet), with the unit AI's
/// queue `orders`.
pub(crate) fn sent_off_world(orders: Vec<crate::unit_ai::QueuedOrder>) -> World {
    let villager = Unit {
        id: 9175,
        body: crate::objects::layers::Body::at([29.5, 89.5, 0.0]),
        life: Lifecycle {
            class: Class::Animated,
            state: 2,
            hp: 25.0,
            master_id: 83,
            ..Lifecycle::default()
        },
        ai: Some(UnitAi {
            ai_type: crate::unit_ai::CIVILIAN,
            current_order: None,
            current_action: None,
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
        body: crate::objects::layers::Body::at([30.0, 88.0, 0.0]),
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
            record: crate::map::spatial::Obstruction { kind: 2, class: 2 },
            snow: false,
            sprite_override: false,
            started: false,
        }),
        pasture: -1,
        my_pasture: -1,
        ..Unit::default()
    };
    let mut t = World::new(
        0,
        29910,
        1,
        vec![0, 0],
        vec![
            Player::default(),
            Player {
                objects: vec![villager, foundation].into(),
                attributes: vec![0.0; 250],
                relations: vec![1, 0],
                ..Player::default()
            },
        ],
    );
    t.masters = vec![builder_masters(), builder_masters()];
    t
}

fn task(action_type: i16) -> crate::objects::masters::Task {
    crate::objects::masters::Task {
        action_type,
        object_group: -1,
        ..crate::objects::masters::Task::default()
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
    let world = TaskWorld {
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
        task_search(masters, &actor, &site, &site, None, -1, &world)
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

/// The task finder's target (`World::task_target`'s reading): an annex stands for its main
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
        v.actions.wait_solidify().copied(),
        Some(crate::construction::wait_solidify::WaitSolidify::new(
            task(101),
            10028
        ))
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
    let own = OthersRef::new(1, &t.players[1].objects, own);
    assert_eq!(wait_progress(10028, own), Some(0.0));
    assert_eq!(wait_progress(77, own), None);
    // another player's (player 0 updating, player 1 after it)
    let other = Foreign {
        before: none,
        after: &players[1..],
        left: None,
    };
    assert_eq!(
        wait_progress(10028, OthersRef::new(0, &ObjectList::default(), other)),
        Some(0.0)
    );
}

/// `FUN_1411cc430` on an annex whose main building is in state 0 with a hit point (the
/// builder's work deferred to the caller, [`crate::construction::build::Worked::Main`]): the main
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
        crate::map::spatial::Tiles::new(120, 120, vec![4; 14400]),
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

/// The world of [`sent_off_world`] with the house built and damaged (300 of its 550 hit
/// points), the villager a builder (118) with a Repair task on any object (work value 0.5) and
/// a weapon range of 0.5.
fn repair_world() -> World {
    let mut t = sent_off_world(vec![]);
    let v = &mut t.players[1].objects[0];
    v.life.master_id = 118;
    let house = &mut t.players[1].objects[1];
    house.life.state = 2;
    house.life.hp = 300.0;
    let c = house.construction.as_mut().unwrap();
    (c.progress, c.desolid) = (1.0, false);
    for masters in &mut t.masters {
        let builder = masters.get_mut(118).unwrap();
        builder.weapon_range = 0.5;
        builder.tasks.push(crate::objects::masters::Task {
            id: 4,
            work_val1: 0.5,
            ..task(crate::construction::repair::REPAIR)
        });
    }
    t
}

/// The work command on the player's damaged house (`FUN_14106fd00`, then the Repair's start):
/// the task finder's Repair task (106), the target taken, the unit AI told order 0x2ce with
/// action 0x26a and the house's class, the Repair walking (state 4) with its master's weapon
/// range. The same command again while it faces the house (state 6) keeps the action; a
/// computer player's repairer walking to it keeps it too.
#[test]
fn a_work_command_on_a_damaged_building_starts_a_repair() {
    let mut t = repair_world();
    let work = crate::orders::Work {
        target: 10028,
        point: [30.0, 88.0, 0.0],
        carry_test: false,
        filter: -1,
        notified: true,
    };
    t.order_work(1, 0, &work).unwrap();
    let v = &t.players[1].objects[0];
    assert_eq!(
        v.top_action_kind(),
        Some(crate::construction::repair::REPAIR)
    );
    let r = v.actions.repair().copied().unwrap();
    assert_eq!(
        (r.target, r.state, r.range, r.task.id, r.task.work_value),
        (10028, 4, 0.5, 4, 0.5)
    );
    let ai = v.ai.as_ref().unwrap();
    assert_eq!(
        (ai.current_order, ai.current_action, ai.current_target),
        (Some(OrderKind::Repair), Some(ActionKind::Repair), 10028)
    );
    t.players[1].objects[0].actions.repair_mut().unwrap().state = 6;
    t.order_work(1, 0, &work).unwrap();
    assert_eq!(
        t.players[1].objects[0]
            .actions
            .repair()
            .copied()
            .unwrap()
            .state,
        6
    );
}

/// A work command on another damaged house while repairing (`FUN_140adb480`'s slot 0x68): a
/// repairer counted among the first house's repairers leaves them, takes the new one and walks
/// to it again (state 4, not counted).
#[test]
fn a_work_command_on_another_building_moves_the_repair_there() {
    let mut t = repair_world();
    let other = Unit {
        id: 10031,
        body: crate::objects::layers::Body::at([34.0, 88.0, 0.0]),
        ..t.players[1].objects[1].clone()
    };
    t.players[1].objects.push(other);
    let work = |target| crate::orders::Work {
        target,
        point: [30.0, 88.0, 0.0],
        carry_test: false,
        filter: -1,
        notified: true,
    };
    t.order_work(1, 0, &work(10028)).unwrap();
    {
        let r = t.players[1].objects[0].actions.repair_mut().unwrap();
        (r.state, r.counted) = (7, true);
    }
    t.players[1].objects[1]
        .construction
        .as_mut()
        .unwrap()
        .builders = 1;
    t.order_work(1, 0, &work(10031)).unwrap();
    let r = t.players[1].objects[0].actions.repair().copied().unwrap();
    assert_eq!(
        (r.target, r.state, r.counted, r.point),
        (10031, 4, false, [34.0, 88.0, 0.0])
    );
    assert_eq!(t.players[1].objects[1].construction.unwrap().builders, 0);
}

/// The work command on the player's foundation (`FUN_14106ff70`, the factory's case 0x65, then
/// `FUN_140aadd80`): the villager (83) becomes the builder 118 of its swap group, takes the
/// foundation (one worker), tells its unit AI order 702 with action 602 on it and walks to it
/// (state 4, its `MoveTo` starting: the footprints lie 0.3 apart) with the task's range, work
/// value and the builder's work rate.
#[test]
fn a_work_command_on_a_foundation_starts_a_build_as_its_builder() {
    let mut t = sent_off_world(vec![]);
    let work = crate::orders::Work {
        target: 10028,
        point: [30.0, 88.0, 0.0],
        carry_test: false,
        filter: -1,
        notified: true,
    };
    t.order_work(1, 0, &work).unwrap();
    let v = &t.players[1].objects[0];
    assert_eq!(v.life.master_id, 118);
    assert_eq!(v.top_action_kind(), Some(crate::construction::build::BUILD));
    let b = v.actions.build().copied().unwrap();
    assert_eq!(
        (
            b.target,
            b.state,
            b.walk_target,
            b.point,
            b.move_to,
            b.added
        ),
        (
            10028,
            4,
            10028,
            [30.0, 88.0, 0.0],
            Some(MoveTo::Start),
            false
        )
    );
    assert_eq!(
        b.task,
        BuildTask {
            range: 0.0,
            work_value: 0.0,
            work_rate: 1.0,
            resource: -1,
            id: 0,
            moving: None,
        }
    );
    let ai = v.ai.as_ref().unwrap();
    assert_eq!(
        (ai.current_order, ai.current_action, ai.current_target),
        (Some(OrderKind::Build), Some(ActionKind::Build), 10028)
    );
    assert_eq!(t.players[1].objects[1].workers, 1);
}

/// A builder with a master of its own (a captured villager's copy) builds as that master: its
/// Build's `MoveTo` starts with the own master's walking sprite (`FUN_140aa0df0`'s default),
/// not the one of its player's master of its id, which has none.
#[test]
fn a_builder_with_a_master_of_its_own_walks_with_its_own_master_s_sprite() {
    use crate::map::spatial::Placed;
    use crate::objects::sprite::Graphic;
    let mut t = sent_off_world(vec![]);
    open_map(&mut t);
    t.spatial.relations = vec![vec![0, 3], vec![3, 0]];
    let placed = |position, class, master, radius| Placed {
        position,
        radius,
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
    let at = [26.5, 89.5, 0.0];
    t.spatial.insert(9175, placed(at, 4, 83, [0.2, 0.2]));
    t.spatial
        .insert(10028, placed([30.0, 88.0, 0.0], 3, 70, [1.0, 1.0]));
    let v = &mut t.players[1].objects[0];
    v.place_for_test(at);
    let mut mover = crate::gathering::hunt::tests::walker();
    mover.moved_at = at;
    v.body.put_mover(Some(mover));
    // a walker of sixteen facets (its yaw reads them)
    for table in &mut t.masters {
        if let Some(m) = table.get_mut(118).and_then(|m| m.moving.as_mut()) {
            m.facets = Some(crate::objects::facing::SpriteFacets {
                count: 16,
                flag_1: true,
            });
        }
    }
    t.build_mesh();
    let work = crate::orders::Work {
        target: 10028,
        point: [30.0, 88.0, 0.0],
        carry_test: false,
        filter: -1,
        notified: true,
    };
    t.order_work(1, 0, &work).unwrap();
    let walking = Graphic {
        id: 7,
        frames: 10,
        facets: 16,
        speed_multiplier: 1.0,
        frame_duration: 0.1,
        replay_delay: 0.0,
        flags: 1,
    };
    let _ = t.graphics.insert(7, walking);
    let shown_after_update = |t: &mut World| {
        in_update(t, (1, 0), 50, |unit, c| {
            let me = crate::movement::Me {
                id: unit.id,
                group: unit.group.id,
                action_state: unit.top_action().map_or(0, |a| a.state),
            };
            let _ = unit.build_part(c, me)?;
            Ok(unit.sprite.map(|s| s.graphic.id))
        })
        .unwrap()
    };
    // the player's master 118 has no sprites: the MoveTo shows none
    let mut table = t.clone();
    assert_eq!(shown_after_update(&mut table), None);
    let own = Master {
        move_sprite: 7,
        ..t.masters[1].get(118).unwrap().clone()
    };
    t.players[1].objects[0].own_master = Some(Box::new(own));
    assert_eq!(shown_after_update(&mut t), Some(7));
}

/// The work command again on the Build's own foundation (`FUN_140ab1b30`): a Build at work
/// (state 6) keeps on; one counted among the builders that waits (state 5) leaves them and
/// walks again (state 4), its `MoveTo` arrived where the footprints touch.
#[test]
fn a_work_command_on_the_build_s_own_foundation_keeps_the_work_or_walks_again() {
    let mut t = sent_off_world(vec![]);
    let work = crate::orders::Work {
        target: 10028,
        point: [30.0, 88.0, 0.0],
        carry_test: false,
        filter: -1,
        notified: true,
    };
    t.order_work(1, 0, &work).unwrap();
    t.players[1].objects[0].actions.build_mut().unwrap().state = 6;
    t.order_work(1, 0, &work).unwrap();
    assert_eq!(
        t.players[1].objects[0].actions.build().map(|b| b.state),
        Some(6)
    );
    {
        let b = t.players[1].objects[0].actions.build_mut().unwrap();
        (b.state, b.added, b.move_to) = (5, true, None);
    }
    t.players[1].objects[1]
        .construction
        .as_mut()
        .unwrap()
        .builders = 1;
    t.players[1].objects[0].place_for_test([29.0, 88.0, 0.0]);
    t.order_work(1, 0, &work).unwrap();
    let b = t.players[1].objects[0].actions.build().copied().unwrap();
    assert_eq!(
        (b.state, b.added, b.move_to, b.target),
        (4, false, Some(MoveTo::Arrived), 10028)
    );
    assert_eq!(
        t.players[1].objects[1].construction.map(|k| k.builders),
        Some(0)
    );
    // the same unit's work command on itself does nothing
    let before = t.players[1].objects[0].clone();
    t.order_work(
        1,
        0,
        &crate::orders::Work {
            target: 9175,
            ..work
        },
    )
    .unwrap();
    assert_eq!(t.players[1].objects[0], before);
}

/// The work command on another foundation of the player's while building (`FUN_140ab0300`):
/// a counted builder leaves the first foundation's builders, lets it go and takes the new
/// one, walking to it again (state 4).
#[test]
fn a_work_command_on_another_foundation_moves_the_build_there() {
    let mut t = sent_off_world(vec![]);
    let other = Unit {
        id: 10031,
        body: crate::objects::layers::Body::at([34.0, 88.0, 0.0]),
        ..t.players[1].objects[1].clone()
    };
    t.players[1].objects.push(other);
    let work = |target| crate::orders::Work {
        target,
        point: [30.0, 88.0, 0.0],
        carry_test: false,
        filter: -1,
        notified: true,
    };
    t.order_work(1, 0, &work(10028)).unwrap();
    {
        let b = t.players[1].objects[0].actions.build_mut().unwrap();
        (b.state, b.added) = (5, true);
    }
    t.players[1].objects[1]
        .construction
        .as_mut()
        .unwrap()
        .builders = 1;
    t.order_work(1, 0, &work(10031)).unwrap();
    let b = t.players[1].objects[0].actions.build().copied().unwrap();
    assert_eq!(
        (b.target, b.state, b.added, b.point, b.walk_target),
        (10031, 4, false, [34.0, 88.0, 0.0], 10031)
    );
    let site = |i: usize| {
        let o = &t.players[1].objects[i];
        (o.workers, o.construction.map(|k| k.builders))
    };
    assert_eq!((site(1), site(2)), ((0, Some(0)), (1, Some(0))));
    assert_eq!(
        t.players[1].objects[0]
            .ai
            .as_ref()
            .map(|ai| ai.current_target),
        Some(10031)
    );
}

/// The unit AI's queued order on a target (`FUN_140c5c170` case `0x2db`): the unit's work
/// command on its foundation at the foundation's place (the villager becomes the builder and
/// starts the Build), then the tail with the order the work gave the AI (the build order) and
/// the queued order's priority. An order on the unit itself works on nothing but still takes
/// the tail; one on a target that is gone, or for a unit without a unit AI, is not modelled.
#[test]
fn a_queued_order_on_a_target_works_on_it_with_the_order_s_priority() {
    use crate::unit_ai::QueuedOrder;
    let order = |target| QueuedOrder {
        issuer: 1,
        order: OrderKind::OnTarget,
        priority: 3,
        target,
        target_owner: 1,
        point: [0.0; 3],
        range: 0.0,
        shown: [0.0; 2],
        ready: true,
        target_type: 80,
    };
    let mut t = sent_off_world(vec![]);
    t.order_on_target(1, 0, order(10028)).unwrap();
    let v = &t.players[1].objects[0];
    assert_eq!(v.life.master_id, 118);
    assert_eq!(
        v.actions.build().map(|b| (b.target, b.state)),
        Some((10028, 4))
    );
    let ai = v.ai.as_ref().unwrap();
    assert_eq!(
        (ai.current_order, ai.order_priority, ai.current_target),
        (Some(OrderKind::Build), 3, 10028)
    );
    let mut t = sent_off_world(vec![]);
    t.order_on_target(1, 0, order(9175)).unwrap();
    let v = &t.players[1].objects[0];
    assert_eq!(v.actions.build().copied(), None);
    assert_eq!(
        v.ai.as_ref()
            .map(|ai| (ai.current_order, ai.order_priority)),
        Some((None, 3))
    );
    assert!(t.order_on_target(1, 0, order(4444)).is_err());
    t.players[1].objects[0].ai = None;
    assert!(t.order_on_target(1, 0, order(9175)).is_err());
}

/// [`placeable`]'s map with its tiles (class 4, open), on which the walker's restriction 7
/// lets a unit stand anywhere: the spatial table starts empty.
pub(crate) fn open_map(t: &mut World) {
    t.spatial = Spatial::with_tiles(
        [120, 120],
        crate::map::spatial::Tiles::new(120, 120, vec![4; 120 * 120]),
    );
    placeable(t);
    if let Some(terrain) = t.spatial.terrain.as_mut() {
        terrain.restrictions = vec![vec![1.0]; 8];
    }
}

/// [`sent_off_world`] with its villager the forager 120 (a Gather task on class 7 for food,
/// dropping at the town centre 109, as the drop-site table has it), Gaia's berry bushes 600 and
/// 601 (master 59, class 7) and player 1's town centre 700, all placed on [`open_map`]; the
/// villager can move.
pub(crate) fn forage_world() -> World {
    let mut t = sent_off_world(vec![]);
    for masters in &mut t.masters {
        let forager = Master {
            id: 120,
            capacity: 10,
            drop_sites: vec![109],
            tasks: vec![crate::objects::masters::Task {
                id: 2,
                action_type: crate::gathering::gather::GATHER,
                object_group: 7,
                attribute_type: 17,
                work_range: 0.25,
                work_val1: 0.31,
                ..crate::objects::masters::Task::default()
            }],
            ..masters.get(83).unwrap().clone()
        };
        let bush = Master {
            id: 59,
            kind: 30,
            class: 7,
            radius: [0.5, 0.5],
            ..Master::default()
        };
        let centre = Master {
            id: 109,
            kind: 80,
            class: 3,
            radius: [2.0, 2.0],
            ..Master::default()
        };
        *masters = Masters::new(masters.iter().cloned().chain([forager, bush, centre]));
    }
    let resource = |id, x| Unit {
        id,
        body: crate::objects::layers::Body::at([x, 89.5, 0.0]),
        life: Lifecycle {
            state: 2,
            hp: 1.0,
            master_id: 59,
            attribute_type: 17,
            amount: 125.0,
            ..Lifecycle::default()
        },
        pasture: -1,
        my_pasture: -1,
        ..Unit::default()
    };
    t.players[0].objects = vec![resource(600, 32.5), resource(601, 36.5)].into();
    t.players[1].objects.push(Unit {
        id: 700,
        life: Lifecycle {
            state: 2,
            hp: 2400.0,
            master_id: 109,
            ..Lifecycle::default()
        },
        // a building is an animated object (type 80)
        body: crate::objects::layers::Body::new(
            [20.0, 89.5, 0.0],
            Some(crate::objects::layers::Animated::default()),
            None,
        ),
        ..Unit::default()
    });
    t.players[1].objects[0]
        .body
        .put_mover(Some(crate::gathering::hunt::tests::walker()));
    t.players[1].objects[0].life.master_id = 120;
    open_map(&mut t);
    let placed = |x: f32, class, player, master, radius| Placed {
        position: [x, 89.5, 0.0],
        radius,
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
    t.spatial.insert(9175, placed(29.5, 4, 1, 83, [0.2, 0.2]));
    t.spatial.insert(600, placed(32.5, 7, 0, 59, [0.5, 0.5]));
    t.spatial.insert(601, placed(36.5, 7, 0, 59, [0.5, 0.5]));
    t.spatial.insert(700, placed(20.0, 3, 1, 109, [2.0, 2.0]));
    // the drop-site table: villagers drop berries at the town centre
    t.drop_sites = vec![DropSite {
        building: 109,
        worker_class: 4,
        targets: vec![crate::unit_ai::task_finder::DropTarget {
            class: 7,
            workers: -1,
            state: -1,
            attribute: 17,
        }],
        ..DropSite::default()
    }];
    t
}

/// The work command on Gaia's berry bush (the factory's case 5, `FUN_140a97e00`): the forager's
/// Gather takes the bush (one worker), the unit
/// AI hears order 0x2c5 with action 0x261 on it and the Gather walks to it (state 4) with the
/// task's range. The command on the other bush while it walks (slot 0x48, `0x140a903a0`) moves
/// the Gather there: the first bush is let go, the second taken.
#[test]
fn a_work_command_on_a_bush_starts_a_gather_and_another_moves_it() {
    let mut t = forage_world();
    let work = |target| crate::orders::Work {
        target,
        point: [32.5, 89.5, 0.0],
        carry_test: false,
        filter: -1,
        notified: true,
    };
    t.order_work(1, 0, &work(600)).unwrap();
    let v = &t.players[1].objects[0];
    assert_eq!(v.life.master_id, 120);
    assert_eq!(v.top_action_kind(), Some(crate::gathering::gather::GATHER));
    let g = v.actions.gather().copied().unwrap();
    assert_eq!(
        (
            g.target,
            g.carry_to,
            g.state,
            g.point,
            g.walk_range,
            g.task.id
        ),
        (600, -1, 4, [32.5, 89.5, 0.0], 0.25, 2)
    );
    let ai = v.ai.as_ref().unwrap();
    assert_eq!(
        (ai.current_order, ai.current_action, ai.current_target),
        (Some(OrderKind::Gather), Some(ActionKind::Gather), 600)
    );
    let workers = |t: &World| {
        (
            t.players[0].objects[0].workers,
            t.players[0].objects[1].workers,
        )
    };
    assert_eq!(workers(&t), (1, 0));
    t.order_work(1, 0, &work(601)).unwrap();
    let g = t.players[1].objects[0].actions.gather().copied().unwrap();
    assert_eq!((g.target, g.state, g.point), (601, 4, [36.5, 89.5, 0.0]));
    assert_eq!(workers(&t), (0, 1));
    assert_eq!(
        t.players[1].objects[0]
            .ai
            .as_ref()
            .map(|ai| ai.current_target),
        Some(601)
    );
    // at work on it (state 7) the same command keeps the Gather as it is
    t.players[1].objects[0].actions.gather_mut().unwrap().state = 7;
    let before = t.players[1].objects[0].actions.gather().copied();
    t.order_work(1, 0, &work(601)).unwrap();
    assert_eq!(t.players[1].objects[0].actions.gather().copied(), before);
    assert_eq!(workers(&t), (0, 1));
}

/// The unit AI's gather command (`FUN_141085030`, forced): a new Gather on the bush replaces
/// the forager's actions without the unit AI's notify (the AI set its order itself); the command on the other bush while the Gather on the same task is on
/// top retargets it (slot 0x48): the first bush let go, the second taken.
#[test]
fn the_unit_ai_s_gather_command_starts_a_gather_or_retargets_its_own() {
    let mut t = forage_world();
    t.gather_command(1, 0, 600).unwrap();
    let v = &t.players[1].objects[0];
    assert_eq!(v.life.master_id, 120);
    let g = v.actions.gather().copied().unwrap();
    assert_eq!((g.target, g.state, g.task.id), (600, 4, 2));
    assert_eq!(v.ai.as_ref().map(|ai| ai.current_order), Some(None));
    t.gather_command(1, 0, 601).unwrap();
    assert_eq!(
        t.players[1].objects[0].actions.gather().map(|g| g.target),
        Some(601)
    );
    assert_eq!(
        (
            t.players[0].objects[0].workers,
            t.players[0].objects[1].workers
        ),
        (0, 1)
    );
    // an object the world does not hold fails the command
    assert!(t.gather_command(1, 0, 4444).is_err());
}

/// A loaded forager's work command on its own town centre, a drop site of its master: the task
/// finder's drop-off gives the Gather task for what it carries, and the Gather started on the
/// centre carries the load there (`0x140a8b3a0`: state 8, the centre its second target, no
/// target or point); the centre, taken as the target and again as the second target, then let
/// go as the first, counts one worker. A Gather at work on its bush sent there (slot 0x48 on a
/// building that is no farm answers 0) starts the same new Gather.
#[test]
fn a_loaded_forager_sent_to_its_town_centre_carries_its_load_there() {
    let work = crate::orders::Work {
        target: 700,
        point: [20.0, 89.5, 0.0],
        carry_test: false,
        filter: -1,
        notified: true,
    };
    let loaded = || {
        let mut t = forage_world();
        let v = &mut t.players[1].objects[0];
        (v.life.attribute_type, v.life.amount) = (17, 6.0);
        t
    };
    let carries = |t: &World| {
        let v = &t.players[1].objects[0];
        let g = v.actions.gather().copied().unwrap();
        (
            (g.target, g.carry_to, g.state, g.point),
            (v.life.attribute_type, v.life.amount),
            t.players[1].objects[2].workers,
        )
    };
    let carrying = ((-1, 700, 8, [-1.0; 3]), (17, 6.0), 1);
    let mut t = loaded();
    t.order_work(1, 0, &work).unwrap();
    assert_eq!(carries(&t), carrying);
    let mut t = loaded();
    t.gather_command(1, 0, 600).unwrap();
    t.players[1].objects[0].actions.gather_mut().unwrap().state = 7;
    t.order_work(1, 0, &work).unwrap();
    assert_eq!(carries(&t), carrying);
    assert_eq!(t.players[0].objects[0].workers, 0);
}

/// The work command without a task (`FUN_14106ff70`'s move): the forager sent to its town
/// centre empty-handed (no task on class 3) is told of the move (order 0x2c6, the centre with
/// its class) and walks to the centre; sent to a point it walks there; in state 1 it is told of
/// no order (−1) and stays; a master that does not move does nothing.
#[test]
fn a_work_command_without_a_task_moves_the_unit_there() {
    let work = |target| crate::orders::Work {
        target,
        point: [20.0, 89.5, 0.0],
        carry_test: false,
        filter: -1,
        notified: true,
    };
    let mut t = forage_world();
    t.order_work(1, 0, &work(700)).unwrap();
    let v = &t.players[1].objects[0];
    assert!(v.actions.move_to().is_some());
    let ai = v.ai.as_ref().unwrap();
    assert_eq!(
        (ai.current_order, ai.current_action, ai.current_target),
        (Some(OrderKind::Move), Some(ActionKind::Move), 700)
    );
    let mut t = forage_world();
    t.order_work(1, 0, &work(-1)).unwrap();
    let v = &t.players[1].objects[0];
    assert!(v.actions.move_to().is_some());
    assert_eq!(
        v.ai.as_ref().map(|ai| ai.current_order),
        Some(Some(OrderKind::Move))
    );
    let mut t = forage_world();
    t.players[1].objects[0].life.state = 1;
    t.order_work(1, 0, &work(700)).unwrap();
    let v = &t.players[1].objects[0];
    assert_eq!(v.actions.move_to().copied(), None);
    assert_eq!(v.ai.as_ref().map(|ai| ai.current_order), Some(None));
    let mut t = forage_world();
    t.masters[1].get_mut(120).unwrap().moving = None;
    let before = t.players[1].objects[0].clone();
    t.order_work(1, 0, &work(700)).unwrap();
    assert_eq!(
        t.players[1].objects[0].actions.move_to().copied(),
        before.actions.move_to().copied()
    );
}

/// A move toward an object while gathering (`FUN_140a97730`, the Gather's slot 0x40,
/// `0x140a8b740`): the Gather takes the move; toward the other bush it walks to the move's point
/// (state 15) without a target, the first bush let go. Loaded and moved toward its town centre,
/// a drop site, it carries the load there (state 8, the centre its second target, taken).
#[test]
fn a_gather_takes_a_move_toward_an_object_as_its_point_or_drop_site() {
    let mut t = forage_world();
    t.gather_command(1, 0, 600).unwrap();
    t.players[1].objects[0].actions.gather_mut().unwrap().state = 7;
    let point = [36.0, 89.0, 0.0];
    assert_eq!(t.move_to_object_with(1, 0, 601, point, false), Ok(true));
    let g = t.players[1].objects[0].actions.gather().copied().unwrap();
    assert_eq!(
        (g.target, g.carry_to, g.state, g.point, g.walk_to),
        (-1, -1, 15, point, Some(point))
    );
    assert_eq!(
        (
            t.players[0].objects[0].workers,
            t.players[0].objects[1].workers
        ),
        (0, 0)
    );
    let v = &mut t.players[1].objects[0];
    (v.life.attribute_type, v.life.amount) = (17, 6.0);
    assert_eq!(
        t.move_to_object_with(1, 0, 700, [20.0, 89.5, 0.0], false),
        Ok(true)
    );
    let g = t.players[1].objects[0].actions.gather().copied().unwrap();
    assert_eq!(
        (g.target, g.carry_to, g.state, g.point),
        (-1, 700, 8, [-1.0; 3])
    );
    assert_eq!(t.players[1].objects[2].workers, 1);
    // a move toward the drop site it already carries to changes nothing
    let before = t.players[1].objects[0].clone();
    assert_eq!(
        t.move_to_object_with(1, 0, 700, [20.0, 89.5, 0.0], false),
        Ok(true)
    );
    assert_eq!(t.players[1].objects[0], before);
    assert_eq!(t.players[1].objects[2].workers, 1);
}

/// `UNIT_ORDER`'s drop-off 6 of a loaded forager without an action (`FUN_14161e370`, then
/// [`World::drop_off_unasked`]): its master drops at the town centre, so it seeks its drop site
/// from itself (`FUN_1411671e0`: its zone, the planner; by the squared distance) and works on the
/// nearer of the player's two centres in state 2, its Gather carrying the load there. A master
/// dropping at the dock (45) too searches without a seeker; with no drop site found it does
/// nothing.
#[test]
fn a_drop_off_order_takes_a_forager_to_its_nearest_drop_site() {
    let mut t = forage_world();
    let far = Unit {
        id: 701,
        body: crate::objects::layers::Body::at([60.0, 89.5, 0.0]),
        ..t.players[1].objects[2].clone()
    };
    t.players[1].objects.push(far);
    let v = &mut t.players[1].objects[0];
    (v.life.attribute_type, v.life.amount) = (17, 6.0);
    let world = t.clone();
    t.unit_order(player_id(1), &object_ids([9175]), 6).unwrap();
    let v = &t.players[1].objects[0];
    let g = v
        .actions
        .gather()
        .copied()
        .expect("a Gather to the drop site");
    assert_eq!((g.carry_to, g.state), (700, 8));
    assert_eq!(t.players[1].objects[2].workers, 1);
    // dropping at the dock too: the search without a seeker finds the same centre
    let mut t = world.clone();
    for masters in &mut t.masters {
        masters.get_mut(120).unwrap().drop_sites = vec![45, 109];
    }
    t.unit_order(player_id(1), &object_ids([9175]), 6).unwrap();
    assert_eq!(
        t.players[1].objects[0]
            .actions
            .gather()
            .map(|g| (g.carry_to, g.state)),
        Some((700, 8))
    );
    // no centre standing (state 2): no drop site, no table entry for berries by class
    let mut t = world;
    for i in [2, 3] {
        t.players[1].objects[i].life.state = 1;
    }
    t.unit_order(player_id(1), &object_ids([9175]), 6).unwrap();
    assert_eq!(t.players[1].objects[0].actions.gather().copied(), None);
}

/// The Empire Wars start (`BaseWorld::update`'s first update) under the farm setting: the idle
/// forager standing on its player's farm 800 works it (the first of class 49 on its tile), its
/// Gather on the farm at the farm's place; the foundation, the town centre and the farm take
/// nothing. A villager with orders waits; a fishing ship without a fish trap under it, and the
/// pass without the farm setting, are not modelled.
#[test]
fn the_empire_wars_start_puts_a_villager_on_its_farm_to_work() {
    let mut t = forage_world();
    t.settings.walkable_farms = true;
    for masters in &mut t.masters {
        masters
            .get_mut(120)
            .unwrap()
            .tasks
            .push(crate::objects::masters::Task {
                id: 3,
                action_type: crate::gathering::gather::GATHER,
                object_group: 49,
                attribute_type: 0,
                work_range: 0.25,
                ..crate::objects::masters::Task::default()
            });
        let farm = Master {
            id: 50,
            kind: 80,
            class: 49,
            radius: [1.5, 1.5],
            ..Master::default()
        };
        let fishing = Master {
            id: 13,
            class: 21,
            ..masters.get(120).unwrap().clone()
        };
        *masters = Masters::new(masters.iter().cloned().chain([farm, fishing]));
    }
    t.players[1].objects.push(Unit {
        id: 800,
        body: crate::objects::layers::Body::at([29.5, 89.5, 0.0]),
        life: Lifecycle {
            state: 2,
            hp: 480.0,
            master_id: 50,
            attribute_type: 0,
            amount: 175.0,
            ..Lifecycle::default()
        },
        pasture: -1,
        my_pasture: -1,
        ..Unit::default()
    });
    t.spatial.insert(
        800,
        Placed {
            position: [29.5, 89.5, 0.0],
            radius: [1.5, 1.5],
            class: 49,
            player: 1,
            master: 50,
            obstruction_type: 0,
            buildable_on: 0,
            tall: false,
            speed: 0.0,
            unknown: None,
            obstruction: None,
        },
    );
    let world = t.clone();
    t.start_work(1).unwrap();
    let g = t.players[1].objects[0]
        .actions
        .gather()
        .copied()
        .expect("the farm's Gather");
    assert_eq!((g.target, g.task.id, g.point), (800, 3, [29.5, 89.5, 0.0]));
    assert_eq!(t.players[1].objects[3].workers, 1);
    let others = |t: &World| {
        t.players[1].objects[1..]
            .iter()
            .map(Unit::top_action_kind)
            .collect::<Vec<_>>()
    };
    assert_eq!(others(&t), [None, None, None]);
    // a villager with an order in its queue waits for it
    let mut t = world.clone();
    t.players[1].objects[0]
        .ai
        .as_mut()
        .unwrap()
        .orders
        .push(crate::unit_ai::QueuedOrder {
            issuer: 1,
            order: OrderKind::Move,
            priority: 100,
            target: -1,
            target_owner: -1,
            point: [10.0, 10.0, 0.0],
            range: 0.0,
            shown: [10.0, 10.0],
            ready: true,
            target_type: -1,
        });
    t.start_work(1).unwrap();
    assert_eq!(t.players[1].objects[0].actions.gather().copied(), None);
    // a fishing ship finds no fish trap on its tile
    let mut t = world.clone();
    t.players[1].objects[0].life.master_id = 13;
    assert!(t.start_work(1).is_err());
    let mut t = world;
    t.settings.walkable_farms = false;
    assert!(t.start_work(1).is_err());
}

/// The forager's unit AI run outside its update giving `effects` ([`World::run_unit_ai`]):
/// they apply in order ([`Unit::apply`], then the orders [`World::finish_object`] runs).
fn with_effects(t: &mut World, effects: Vec<crate::unit_ai::Effect>) -> Result<(), GenieError> {
    t.run_unit_ai((1, 0), move |_, _, _, _, _| Ok(effects))
}

/// The unit AI's explore (slot 0x398, `FUN_141084db0`) of the idle forager: a new Explore to
/// the point at its height walks there (state 11, its `MoveTo` starting); a point off the map's
/// quarter for the Explore on top makes it wait (state 3), its walk stopped, and ask its unit
/// AI for a point (notify 0x202 with the explore's action). An Explore replacing a Gather is
/// not modelled.
#[test]
fn the_unit_ai_s_explore_walks_to_its_point_or_asks_for_one() {
    use crate::movement::explore::{WAIT, WALK, WANTS_POINT};
    use crate::unit_ai::Effect;
    let mut t = forage_world();
    with_effects(&mut t, vec![Effect::Explore([40.0, 90.0])]).unwrap();
    let e = t.players[1].objects[0].actions.explore().copied().unwrap();
    assert_eq!((e.state, e.point), (WALK, [40.0, 90.0, 0.0]));
    assert!(e.move_to.is_some());
    with_effects(&mut t, vec![Effect::Explore([-1.0, 90.0])]).unwrap();
    let v = &t.players[1].objects[0];
    let e = v.actions.explore().copied().unwrap();
    assert_eq!((e.state, e.move_to), (WAIT, None));
    let notifies = &v.ai.as_ref().unwrap().notifies;
    assert_eq!(
        notifies.last().map(|n| (n.kind, n.p)),
        Some((WANTS_POINT, [ActionKind::Explore.value(), 0, 0]))
    );
    let mut t = forage_world();
    t.gather_command(1, 0, 600).unwrap();
    assert!(with_effects(&mut t, vec![Effect::Explore([40.0, 90.0])]).is_err());
}

/// The unit AI's move to an object (slot 0x348, `FUN_141086440`): the forager's Gather goes (its
/// bush let go), a `MoveTo` to the other bush replaces it within the range given; the unit AI's
/// cap of its last human order (slot 0x300) and its works follow in order: a work on the bush
/// (slot 0x7e8 with the carried-resource test) starts a Gather there.
#[test]
fn the_unit_ai_s_moves_and_works_apply_in_order() {
    use crate::unit_ai::Effect;
    let mut t = forage_world();
    t.gather_command(1, 0, 600).unwrap();
    t.players[1].objects[0].last_human_order = 900;
    with_effects(
        &mut t,
        vec![
            Effect::CapHumanOrder(300),
            Effect::MoveToObject {
                target: 601,
                state: 2,
                range: 0.5,
            },
        ],
    )
    .unwrap();
    let v = &t.players[1].objects[0];
    assert_eq!(
        (v.actions.gather().copied(), v.last_human_order),
        (None, 300)
    );
    assert!(v.actions.move_to().is_some());
    assert_eq!(t.players[0].objects[0].workers, 0);
    with_effects(
        &mut t,
        vec![Effect::WorkOn {
            target: 601,
            point: [36.5, 89.5, 0.0],
        }],
    )
    .unwrap();
    let v = &t.players[1].objects[0];
    assert_eq!(
        v.actions.gather().map(|g| (g.target, g.state)),
        Some((601, 4))
    );
    assert_eq!(v.actions.move_to().copied(), None);
    // a unit that does not move cannot be moved to an object
    t.players[1].objects[0].body.put_mover(None);
    assert!(
        with_effects(
            &mut t,
            vec![Effect::MoveToObject {
                target: 600,
                state: 2,
                range: 0.5,
            }],
        )
        .is_err()
    );
}

/// The unit AI's hunt and build commands and its order on a target, as its effects give them
/// after the run: `HuntObject` hunts the deer (the hunt command, `FUN_1412151f0`); `BuildObject`
/// sends the villager to build its foundation (the build command, slot 0x380, as the builder);
/// `OrderOnTarget` works on the foundation with the order's priority. A tail after an order the
/// AI did not take, and an AI move of a master the slice does not have, are not modelled; a
/// restored patrol without one saved does nothing, and the stop stops.
#[test]
fn the_unit_ai_s_hunt_build_and_target_orders_apply_after_its_run() {
    use crate::unit_ai::{AfterOrder, AiMove, Effect, QueuedOrder};
    let mut t = hunt_world();
    with_effects(&mut t, vec![Effect::HuntObject(500)]).unwrap();
    assert_eq!(
        t.players[1].objects[0].actions.hunt().map(|h| h.target),
        Some(500)
    );
    with_effects(&mut t, vec![Effect::Stop]).unwrap();
    assert_eq!(t.players[1].objects[0].top_action_kind(), None);
    let mut t = sent_off_world(vec![]);
    with_effects(&mut t, vec![Effect::BuildObject(10028)]).unwrap();
    let v = &t.players[1].objects[0];
    assert_eq!(v.life.master_id, 118);
    assert_eq!(v.actions.build().map(|b| b.target), Some(10028));
    let mut t = sent_off_world(vec![]);
    let order = QueuedOrder {
        issuer: 1,
        order: OrderKind::OnTarget,
        priority: 3,
        target: 10028,
        target_owner: 1,
        point: [0.0; 3],
        range: 0.0,
        shown: [0.0; 2],
        ready: true,
        target_type: 80,
    };
    with_effects(&mut t, vec![Effect::OrderOnTarget(order)]).unwrap();
    let v = &t.players[1].objects[0];
    assert_eq!(v.actions.build().map(|b| b.target), Some(10028));
    assert_eq!(v.ai.as_ref().map(|ai| ai.order_priority), Some(3));
    let mut t = sent_off_world(vec![]);
    let after = AfterOrder {
        busy: false,
        group: None,
    };
    assert!(with_effects(&mut t, vec![Effect::AfterOrder(after)]).is_err());
    assert!(
        with_effects(
            &mut t,
            vec![Effect::OrderOnTarget(order), Effect::OrderOnTarget(order)]
        )
        .is_err()
    );
    let mut t = sent_off_world(vec![]);
    t.players[1].objects[0].life.master_id = 4444;
    let walk = AiMove {
        point: [20.0, 20.0, 0.0],
        range: 0.0,
        force: false,
        walk: false,
    };
    assert!(with_effects(&mut t, vec![Effect::Move(walk)]).is_err());
    let mut t = sent_off_world(vec![]);
    let before = t.players[1].objects[0].clone();
    with_effects(&mut t, vec![Effect::RestorePatrol]).unwrap();
    assert_eq!(t.players[1].objects[0], before);
}

/// The base handler's case 0x216 from the forager's side ([`UnitSide::off_footprint`],
/// `FUN_1411566b0` with 0): with the forager's height and class, its player, the foundation by
/// its master and the Gather's bush as the action's target; without the foundation placed no
/// spot, placed at (30, 88) the flood's first empty tile beside its footprint, (30, 89).
///
/// [`UnitSide::off_footprint`]: crate::unit_ai::UnitSide::off_footprint
#[test]
fn a_unit_sent_off_a_foundation_finds_the_spot_beside_it() {
    let mut t = forage_world();
    t.gather_command(1, 0, 600).unwrap();
    let mut found = None;
    t.run_unit_ai((1, 0), |_, _, side, _, _| {
        found = Some(side.off_footprint(10028, [1.0, 1.0])?);
        Ok(Vec::new())
    })
    .unwrap();
    let off = found.unwrap();
    assert_eq!(
        (off.spot, off.class, off.player, off.computer),
        (None, 4, 1, false)
    );
    assert_eq!(
        off.site.map(|s| (s.id, s.master, s.owner)),
        Some((10028, 70, 1))
    );
    assert_eq!(
        off.action_target.map(|s| (s.id, s.master, s.owner)),
        Some((600, 59, 0))
    );
    t.spatial.insert(
        10028,
        Placed {
            position: [30.0, 88.0, 0.0],
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
    let mut found = None;
    t.run_unit_ai((1, 0), |_, _, side, _, _| {
        found = Some(side.off_footprint(10028, [1.0, 1.0])?);
        Ok(Vec::new())
    })
    .unwrap();
    assert_eq!(found.unwrap().spot, Some([30.5, 89.5]));
}

/// The auto scout's search from the forager's side ([`UnitSide::seek_unexplored`]): on a map
/// its player explored whole, neither the area about its town centre nor the unit AI's own
/// search (slot 0x90) finds a tile.
///
/// [`UnitSide::seek_unexplored`]: crate::unit_ai::UnitSide::seek_unexplored
#[test]
fn an_explored_map_leaves_the_auto_scout_nothing_to_seek() {
    let mut t = forage_world();
    let mut found = Some(None);
    t.run_unit_ai((1, 0), |ai, _, side, _, rng| {
        found = Some(side.seek_unexplored(ai, rng)?);
        Ok(Vec::new())
    })
    .unwrap();
    assert_eq!(found, Some(None));
}

/// The unit AI's `ClosestResourceObject` from the forager's side (`FUN_140c49760`,
/// [`UnitSide::closest_resource`]): of the bushes (gather group 2, gathering by themselves) the
/// nearer, 600; passing over its current target the other; from its town centre as the drop
/// site the one nearer the centre; none below the state asked for.
///
/// [`UnitSide::closest_resource`]: crate::unit_ai::UnitSide::closest_resource
#[test]
fn the_unit_ai_s_resource_search_finds_the_nearest_bush_it_reaches() {
    use crate::unit_ai::{NO_POINT, ResourceSearch};
    let mut t = forage_world();
    for masters in &mut t.masters {
        let bush = masters.get_mut(59).unwrap();
        (bush.auto_gather, bush.gather_group) = (true, 2);
        // the walk reaches the forager's line of sight plus 5 tiles: 601 lies 7 away
        masters.get_mut(120).unwrap().line_of_sight = 4.0;
    }
    let search = |t: &mut World, s: ResourceSearch| {
        let mut found = None;
        t.run_unit_ai((1, 0), |_, _, side, _, _| {
            found = Some(side.closest_resource(s)?);
            Ok(Vec::new())
        })
        .unwrap();
        found.unwrap().map(|g| (g.id, g.class))
    };
    let any = ResourceSearch {
        kind: 2,
        cap: -1,
        site: -1,
        current: -1,
        point: NO_POINT,
        min_state: 2,
    };
    assert_eq!(search(&mut t, any), Some((600, 7)));
    assert_eq!(
        search(
            &mut t,
            ResourceSearch {
                current: 600,
                ..any
            }
        ),
        Some((601, 7))
    );
    assert_eq!(
        search(&mut t, ResourceSearch { site: 700, ..any }),
        Some((600, 7))
    );
    assert_eq!(
        search(&mut t, ResourceSearch { site: 4444, ..any }),
        Some((600, 7))
    );
    assert_eq!(
        search(
            &mut t,
            ResourceSearch {
                min_state: 3,
                ..any
            }
        ),
        None
    );
}

/// [`forage_world`] with player 1's mule cart 750 (master 1808: a building of a task of action
/// type 0x99 on class 7, berries) at (24.5, 89.5), moving and with a unit AI; the bushes gather
/// by themselves as group 1.
fn cart_world() -> World {
    let mut t = forage_world();
    for masters in &mut t.masters {
        let bush = masters.get_mut(59).unwrap();
        (bush.auto_gather, bush.gather_group) = (true, 1);
        let cart = Master {
            id: 1808,
            kind: 80,
            class: 3,
            radius: [0.5, 0.5],
            line_of_sight: 4.0,
            moving: Some(crate::gathering::hunt::tests::walker().stats),
            tasks: vec![crate::objects::masters::Task {
                action_type: crate::construction::build::CART_TASK,
                object_group: 7,
                ..crate::objects::masters::Task::default()
            }],
            ..Master::default()
        };
        *masters = Masters::new(masters.iter().cloned().chain([cart]));
    }
    t.players[1].objects.push(Unit {
        id: 750,
        life: Lifecycle {
            state: 2,
            hp: 100.0,
            master_id: 1808,
            ..Lifecycle::default()
        },
        body: crate::objects::layers::Body::new(
            [24.5, 89.5, 0.0],
            None,
            Some(crate::gathering::hunt::tests::walker()),
        ),
        ai: Some(UnitAi {
            current_order: None,
            current_action: None,
            current_target: -1,
            ..UnitAi::default()
        }),
        pasture: -1,
        my_pasture: -1,
        ..Unit::default()
    });
    t.spatial.insert(
        750,
        Placed {
            position: [24.5, 89.5, 0.0],
            radius: [0.5, 0.5],
            class: 3,
            player: 1,
            master: 1808,
            obstruction_type: 0,
            buildable_on: 0,
            tall: true,
            speed: 0.0,
            unknown: None,
            obstruction: None,
        },
    );
    t
}

/// The mule cart's own search (slot 0x1e8, `FUN_140c16240`): its task's class 7 names gather
/// group 1, searched from the cart as the drop site; the nearer bush by the footprints' gap
/// with its place. Without its movement state the search is not modelled; without a task of
/// a known class it finds none (its own place).
#[test]
fn a_mule_cart_finds_the_nearest_resource_of_its_task_s_class() {
    let mut t = cart_world();
    assert_eq!(t.cart_find(1, 3), Ok((600, [32.5, 89.5, 0.0])));
    let mut t = cart_world();
    t.masters[1].get_mut(1808).unwrap().tasks[0].object_group = 99;
    assert_eq!(t.cart_find(1, 3), Ok((-1, [24.5, 89.5, 0.0])));
    let mut t = cart_world();
    t.players[1].objects[3].body.put_mover(None);
    assert!(t.cart_find(1, 3).is_err());
}

/// [`forage_world`] with the house's foundation 10028 placed (radii 1) and the forager standing
/// on its footprint at (30, 88.5).
fn footprint_world() -> World {
    let mut t = forage_world();
    t.spatial.insert(
        10028,
        Placed {
            position: [30.0, 88.0, 0.0],
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
    t.players[1].objects[0].place_for_test([30.0, 88.5, 0.0]);
    t.spatial.set_position(9175, [30.0, 88.5, 0.0]);
    t
}

/// `FUN_14123bb20`'s sending off of the units on a new foundation's footprint: the idle forager
/// standing there hears notify 0x216 with the foundation master's radii in hundredths and the
/// foundation; a bush (without a unit AI), an object the world does not hold, a unit at work
/// on the foundation, one walking and one moving off already are passed over.
#[test]
fn the_units_on_a_new_foundation_are_sent_off_it() {
    let sent = |t: &World| {
        t.players[1].objects[0]
            .ai
            .as_ref()
            .unwrap()
            .notifies
            .iter()
            .filter(|n| n.kind == crate::unit_ai::SENT_OFF)
            .map(|n| n.p)
            .collect::<Vec<_>>()
    };
    let mut t = footprint_world();
    t.evacuate(10028, &[9175, 600, 4444]).unwrap();
    assert_eq!(sent(&t), [[100, 100, 10028]]);
    let mut t = footprint_world();
    t.players[1].objects[0].body.set_anim_speed(1.0);
    t.evacuate(10028, &[9175]).unwrap();
    assert_eq!(sent(&t), Vec::<[i32; 3]>::new());
    let mut t = footprint_world();
    t.players[1].objects[0].ai.as_mut().unwrap().current_action = Some(ActionKind::MoveAside);
    t.evacuate(10028, &[9175]).unwrap();
    assert_eq!(sent(&t), Vec::<[i32; 3]>::new());
    let mut t = footprint_world();
    t.order_work(
        1,
        0,
        &crate::orders::Work {
            target: 10028,
            point: [30.0, 88.0, 0.0],
            carry_test: false,
            filter: -1,
            notified: true,
        },
    )
    .unwrap();
    t.evacuate(10028, &[9175]).unwrap();
    assert_eq!(sent(&t), Vec::<[i32; 3]>::new());
    assert!(t.evacuate(4444, &[9175]).is_err());
}

/// `FUN_14123bb20`'s owner's units of a formation category on the footprint go together
/// (`FUN_1411566b0` with 1, then `FUN_1416123a0`): the farthest free spot beside the footprint
/// from the first, and the forager the splitter leaves loose takes the action of moving off a
/// footprint (`0x2bf`) and moves there, so it hears no notify `0x21b`.
#[test]
fn a_formation_s_units_on_a_new_foundation_move_off_together() {
    let mut t = footprint_world();
    for masters in &mut t.masters {
        masters.get_mut(120).unwrap().formation_category = 1;
    }
    t.evacuate(10028, &[9175]).unwrap();
    let v = &t.players[1].objects[0];
    let ai = v.ai.as_ref().unwrap();
    assert_eq!(ai.current_action, Some(ActionKind::MoveAside));
    assert!(
        ai.notifies
            .iter()
            .all(|n| n.kind != crate::unit_ai::SENT_OFF_GROUP)
    );
    assert!(v.actions.move_to().is_some());
}

/// A built castle's death outside its update (`FUN_1411dbb70` to state 3 from 2,
/// [`World::set_state`]): the population headroom its storage gave leaves its owner (mode 4),
/// the castle count (attribute 134) falls by one, not below 0, and it goes to state 3. A
/// computer player's building, or one with units inside, is not modelled.
#[test]
fn a_built_castle_s_death_takes_its_headroom_and_its_count() {
    let castle = Master {
        id: 82,
        kind: 80,
        class: 3,
        radius: [2.0, 2.0],
        hit_points: 4800,
        storage: vec![(4, 20.0, 4)],
        building: Some(crate::objects::masters::BuildingMaster {
            tech: -1,
            foundation_terrain: -1,
            stack_unit: -1,
            head_unit: -1,
            transform_unit: -1,
            annexes: [-1; 4],
            ..crate::objects::masters::BuildingMaster::default()
        }),
        ..Master::default()
    };
    let world = |count: f32| {
        let mut attributes = vec![0.0; 250];
        attributes[4] = 25.0;
        attributes[134] = count;
        let mut t = World::new(
            0,
            1,
            0,
            vec![0, 0],
            vec![
                Player::default(),
                Player {
                    objects: vec![Unit {
                        id: 900,
                        body: crate::objects::layers::Body::at([50.0, 50.0, 0.0]),
                        life: Lifecycle {
                            state: 2,
                            hp: 4800.0,
                            master_id: 82,
                            ..Lifecycle::default()
                        },
                        pasture: -1,
                        my_pasture: -1,
                        ..Unit::default()
                    }]
                    .into(),
                    attributes,
                    ..Player::default()
                },
            ],
        );
        t.masters = vec![Masters::default(), Masters::new([castle.clone()])];
        t
    };
    let mut t = world(1.0);
    t.set_state(1, 900, 3).unwrap();
    let p = &t.players[1];
    assert_eq!((p.attributes[4], p.attributes[134]), (5.0, 0.0));
    assert_eq!(p.objects[0].life.state, 3);
    let mut t = world(0.0);
    t.set_state(1, 900, 3).unwrap();
    assert_eq!(t.players[1].attributes[134], 0.0);
    let mut t = world(1.0);
    t.players[1].computer = true;
    assert!(t.set_state(1, 900, 3).is_err());
    let mut t = world(1.0);
    t.players[1].objects[0].garrison = vec![5];
    assert!(t.set_state(1, 900, 3).is_err());
}

/// The load's shape mesh (`RGE_Static_Object::rehook` per object): a standing house of a tall
/// master with an obstruction kind and class takes its shape; a dead one (state 7), one inside
/// another, a desolid foundation, an object the spatial table lacks and one whose master lacks
/// a kind or class take none.
#[test]
fn the_load_s_mesh_takes_the_shapes_of_the_standing_tall_objects() {
    let house = |id, state, inside, desolid: bool, master| Unit {
        id,
        body: crate::objects::layers::Body::at([10.5, 10.5, 0.0]),
        life: Lifecycle {
            state,
            master_id: master,
            inside,
            ..Lifecycle::default()
        },
        construction: desolid.then_some(Construction {
            progress: 0.0,
            desolid: true,
            builders: 0,
            hit_points: 550,
            build_time: 25,
            record: crate::map::spatial::Obstruction { kind: 2, class: 2 },
            snow: false,
            sprite_override: false,
            started: false,
        }),
        ..Unit::default()
    };
    let mut t = World::new(
        0,
        1,
        0,
        vec![0],
        vec![Player {
            objects: vec![
                house(1, 2, None, false, 70),
                house(2, 7, None, false, 70),
                house(3, 2, Some(1), false, 70),
                house(4, 0, None, true, 70),
                house(5, 2, None, false, 70),
                house(6, 2, None, false, 71),
            ]
            .into(),
            ..Player::default()
        }],
    );
    let master = |id, obstruction_class| Master {
        id,
        kind: 80,
        class: 3,
        radius: [1.0, 1.0],
        tall: true,
        obstruction_type: 2,
        obstruction_class,
        ..Master::default()
    };
    t.masters = vec![Masters::new([master(70, 2), master(71, 0)])];
    t.spatial = Spatial::new([40, 40]);
    for id in [1, 2, 3, 4, 6] {
        t.spatial.insert(
            id,
            Placed {
                position: [10.5, 10.5, 0.0],
                radius: [1.0, 1.0],
                class: 3,
                player: 0,
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
    t.build_mesh();
    let shaped: Vec<i32> = (1..=6)
        .filter(|&id| t.pathing.shape(id).is_some())
        .collect();
    assert_eq!(shaped, [1]);
}

/// [`sent_off_world`] with its villager the hunter 122 (task 110 on class 9 within 0.5,
/// dropping at the town centre 109), Gaia's deer 500 and 501 (master 65, class 9) and player 1's
/// town centre 700, all placed on [`open_map`]; the villager can move.
pub(crate) fn hunt_world() -> World {
    let mut t = sent_off_world(vec![]);
    for masters in &mut t.masters {
        let hunter = Master {
            id: 122,
            drop_sites: vec![109],
            tasks: vec![crate::objects::masters::Task {
                action_type: 110,
                object_group: 9,
                work_range: 0.5,
                ..crate::objects::masters::Task::default()
            }],
            ..masters.get(83).unwrap().clone()
        };
        let deer = Master {
            id: 65,
            kind: 70,
            class: 9,
            radius: [0.4, 0.4],
            ..Master::default()
        };
        let centre = Master {
            id: 109,
            kind: 80,
            class: 3,
            radius: [2.0, 2.0],
            ..Master::default()
        };
        *masters = Masters::new(masters.iter().cloned().chain([hunter, deer, centre]));
    }
    let deer = |id, x| Unit {
        id,
        body: crate::objects::layers::Body::at([x, 89.5, 0.0]),
        life: Lifecycle {
            state: 2,
            hp: 5.0,
            master_id: 65,
            ..Lifecycle::default()
        },
        pasture: -1,
        my_pasture: -1,
        ..Unit::default()
    };
    t.players[0].objects = vec![deer(500, 32.5), deer(501, 36.5)].into();
    t.players[1].objects.push(Unit {
        id: 700,
        body: crate::objects::layers::Body::at([20.0, 89.5, 0.0]),
        life: Lifecycle {
            state: 2,
            hp: 2400.0,
            master_id: 109,
            ..Lifecycle::default()
        },
        pasture: -1,
        my_pasture: -1,
        ..Unit::default()
    });
    open_map(&mut t);
    let placed = |x: f32, class, player, master, radius| Placed {
        position: [x, 89.5, 0.0],
        radius,
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
    t.spatial.insert(9175, placed(29.5, 4, 1, 122, [0.2, 0.2]));
    t.spatial.insert(500, placed(32.5, 9, 0, 65, [0.4, 0.4]));
    t.spatial.insert(501, placed(36.5, 9, 0, 65, [0.4, 0.4]));
    t.spatial.insert(700, placed(20.0, 3, 1, 109, [2.0, 2.0]));
    let v = &mut t.players[1].objects[0];
    v.life.master_id = 122;
    v.body
        .put_mover(Some(crate::gathering::hunt::tests::walker()));
    t
}

/// The work command on the other deer while hunting the first on the same task (the Hunt's slot
/// 9, `0x140ac97c0`): the Hunt takes it and starts over on it (its Attack walking there), the
/// first deer let go and the second taken; the unit AI hears the hunt order on it.
#[test]
fn a_work_command_on_another_deer_moves_the_hunt_there() {
    let mut t = hunt_world();
    t.hunt_command(1, 0, 500).unwrap();
    let work = crate::orders::Work {
        target: 501,
        point: [36.5, 89.5, 0.0],
        carry_test: false,
        filter: -1,
        notified: true,
    };
    t.order_work(1, 0, &work).unwrap();
    let v = &t.players[1].objects[0];
    let h = v.actions.hunt().copied().unwrap();
    assert_eq!((h.target, h.carry_to, h.target_class), (501, -1, 9));
    assert_eq!(
        (h.state, h.attack.map(|a| a.target)),
        (crate::gathering::hunt::ATTACK, Some(501))
    );
    let ai = v.ai.as_ref().unwrap();
    assert_eq!(
        (ai.current_order, ai.current_action, ai.current_target),
        (Some(OrderKind::Hunt), Some(ActionKind::Hunt), 501)
    );
    assert_eq!(
        (
            t.players[0].objects[0].workers,
            t.players[0].objects[1].workers
        ),
        (0, 1)
    );
}

/// A move toward an object while hunting (the Hunt's slot 0x40, `FUN_140ac52c0`): toward the
/// other deer it walks to the move's point (state 15) and lets the first go; loaded and moved
/// toward its town centre, a drop site, it carries the load there (state 8, the centre its second
/// target, taken).
#[test]
fn a_hunt_takes_a_move_toward_an_object_as_its_point_or_drop_site() {
    let mut t = hunt_world();
    t.hunt_command(1, 0, 500).unwrap();
    let point = [36.0, 89.0, 0.0];
    assert_eq!(t.move_to_object_with(1, 0, 501, point, false), Ok(true));
    let h = t.players[1].objects[0].actions.hunt().copied().unwrap();
    assert_eq!(
        (h.target, h.carry_to, h.state, h.point),
        (-1, -1, 15, point)
    );
    assert_eq!(
        (
            t.players[0].objects[0].workers,
            t.players[0].objects[1].workers
        ),
        (0, 0)
    );
    let v = &mut t.players[1].objects[0];
    (v.life.attribute_type, v.life.amount) = (15, 6.0);
    assert_eq!(
        t.move_to_object_with(1, 0, 700, [20.0, 89.5, 0.0], false),
        Ok(true)
    );
    let h = t.players[1].objects[0].actions.hunt().copied().unwrap();
    assert_eq!(
        (h.target, h.carry_to, h.state, h.point, h.target_class),
        (-1, 700, 8, [-1.0; 3], 3)
    );
    assert_eq!(t.players[1].objects[2].workers, 1);
}

/// Runs `f` on object `i` of player `p` with its `Step` as its update has it (dt `dt`, outside
/// a turn), the unit staying in its list, then what the `Step` gathered
/// ([`World::step_object`]): `f`'s answer, or the first error.
pub(crate) fn in_update<R>(
    t: &mut World,
    (p, i): (usize, usize),
    dt: u32,
    f: impl FnOnce(&mut Unit, &mut step::Step<'_>) -> Result<R, GenieError>,
) -> Result<R, GenieError> {
    let mut answer = None;
    let _ = t.step_object((p, i), (dt, None, false), None, |u, c| {
        answer = Some(f(u, c)?);
        Ok(crate::objects::unit::Outcome::Stay)
    })?;
    Ok(answer.unwrap())
}

/// [`hunt_world`]'s hunter on deer 500 (the hunt command's), its hunt then set by `set`.
fn hunter_in(set: impl FnOnce(&mut crate::gathering::hunt::HuntAction)) -> World {
    let mut t = hunt_world();
    t.hunt_command(1, 0, 500).unwrap();
    set(t.players[1].objects[0].actions.hunt_mut().unwrap());
    t
}

/// The hunter's notifies of `kind` (their parameters).
fn notified(t: &World, kind: i32) -> Vec<[i32; 3]> {
    t.players[1].objects[0]
        .ai
        .as_ref()
        .unwrap()
        .notifies
        .iter()
        .filter(|n| n.kind == kind)
        .map(|n| n.p)
        .collect()
}

/// The hunter's part of the update in its hunt's state ([`Unit::hunt_part`]) with `dt`, the
/// hunter as a combat object and its lost tree as the update reads them.
fn hunt_part_of(t: &mut World, dt: u32) -> Result<(), GenieError> {
    in_update(t, (1, 0), dt, |unit, c| {
        let me = crate::movement::Me {
            id: unit.id,
            group: unit.group.id,
            action_state: unit.top_action().map_or(0, |a| a.state),
        };
        let mut own = unit.own_victim(c);
        let tree_lost = unit.tree_lost();
        let code = unit.hunt_part(c, me, &mut own, tree_lost)?;
        unit.end_head_update(c, code)
    })
}

/// The drop-off's wait (state 8, its timer below 0, `0x140ac7a00` case 7): the timer runs by
/// dt; at its end the drop site goes and `set_state(8)` searches again, finding the town centre
/// and walking there.
#[test]
fn a_hunter_s_drop_off_waits_then_searches_its_drop_site_again() {
    use crate::gathering::hunt::DROP;
    let wait = |timer| {
        hunter_in(|h| {
            (h.state, h.timer, h.target, h.carry_to) = (DROP, timer, -1, 700);
            h.attack = None;
        })
    };
    let mut t = wait(-0.5);
    t.players[1].objects[2].workers = 1;
    assert_eq!(hunt_part_of(&mut t, 50), Ok(()));
    let h = t.players[1].objects[0].actions.hunt().copied().unwrap();
    assert_eq!((h.state, h.carry_to), (DROP, 700));
    assert!((h.timer + 0.45).abs() < 1e-6);
    let mut t = wait(-0.01);
    t.players[1].objects[0].life.attribute_type = 15;
    t.players[1].objects[0].life.amount = 5.0;
    t.players[1].objects[2].workers = 1;
    assert_eq!(hunt_part_of(&mut t, 50), Ok(()));
    let h = t.players[1].objects[0].actions.hunt().copied().unwrap();
    assert_eq!((h.state, h.carry_to, h.timer), (DROP, 700, 0.0));
    // the centre let go, then taken again as the new drop site
    assert_eq!(t.players[1].objects[2].workers, 1);
}

/// The drop-off's walk (state 8, case 7): arrived at the town centre within a tile, where the
/// drop-site table takes meat, the hunter's load goes to its player as food and it carries
/// nothing; the hunt goes back to its deer (`set_state(4)`: the live deer's chase, state 12).
/// A failed walk waits 3
/// seconds and stops the animation; a centre that does not take the load is not modelled.
#[test]
fn a_hunter_drops_its_meat_at_the_town_centre_and_goes_back() {
    use crate::gathering::hunt::{ATTACK, DROP};
    use crate::movement::move_to::MoveTo;
    let dropping = |move_to| {
        let mut t = hunter_in(|h| {
            (h.state, h.timer, h.carry_to, h.move_to) = (DROP, 0.0, 700, Some(move_to));
            h.attack = None;
        });
        let v = &mut t.players[1].objects[0];
        (v.life.attribute_type, v.life.amount) = (15, 5.0);
        v.place_for_test([22.3, 89.5, 0.0]);
        t.spatial.set_position(9175, [22.3, 89.5, 0.0]);
        t.drop_sites = vec![DropSite {
            building: 109,
            worker_class: 4,
            targets: vec![crate::unit_ai::task_finder::DropTarget {
                class: 9,
                workers: -1,
                state: -1,
                attribute: 15,
            }],
            ..DropSite::default()
        }];
        t
    };
    let mut t = dropping(MoveTo::Arrived);
    assert_eq!(hunt_part_of(&mut t, 50), Ok(()));
    let v = &t.players[1].objects[0];
    assert_eq!(v.life.amount, 0.0);
    assert_eq!(t.players[1].resources[0], 5.0);
    let h = v.actions.hunt().copied().unwrap();
    assert_eq!((h.state, h.target, h.carry_to), (ATTACK, 500, -1));
    let mut t = dropping(MoveTo::Failed);
    assert_eq!(hunt_part_of(&mut t, 50), Ok(()));
    let v = &t.players[1].objects[0];
    assert_eq!(v.body.anim_speed(), Some(0.0));
    assert!(v.actions.hunt().is_some_and(|h| h.timer < 0.0));
    let mut t = dropping(MoveTo::Arrived);
    t.drop_sites.clear();
    assert!(hunt_part_of(&mut t, 50).is_err());
}

/// The failed hunt (state 13, case 0xc): notify 0x1f9 with the hunt action; the hunt goes and
/// its deer, another player's, is let go after the update. With a sub-action it is not
/// modelled; without a hunt the part does nothing.
#[test]
fn a_failed_hunt_tells_its_unit_ai_and_lets_its_target_go() {
    use crate::gathering::hunt::FAILED;
    let mut t = hunter_in(|h| {
        h.state = FAILED;
        h.attack = None;
    });
    assert_eq!(t.players[0].objects[0].workers, 1);
    assert_eq!(hunt_part_of(&mut t, 50), Ok(()));
    assert_eq!(t.players[1].objects[0].actions.hunt().copied(), None);
    assert_eq!(
        notified(&t, crate::unit_ai::ACTION_FAILED),
        [[ActionKind::Hunt.value(), 0, 0]]
    );
    assert_eq!(t.players[0].objects[0].workers, 0);
    assert_eq!(hunt_part_of(&mut t, 50), Ok(()));
    let mut t = hunter_in(|h| h.state = FAILED);
    assert!(hunt_part_of(&mut t, 50).is_err());
}

/// The search (state 3, case 2): the timer runs by dt; at its end it starts again at −10 s and
/// the unit AI hears notify 0x202 with the hunt action, the hunt's class (9) and its drop site,
/// the town centre the search finds. Without its target a lead (state 10, case 9) searches at
/// once; a done hunt (state 2) waits, but of a tree's class it is not modelled.
#[test]
fn a_searching_hunt_asks_its_unit_ai_again_after_ten_seconds() {
    use crate::gathering::hunt::{IDLE, LEAD, SEARCH};
    let search = [ActionKind::Hunt.value(), 9, 700];
    let mut t = hunter_in(|h| {
        (h.state, h.timer) = (SEARCH, -0.5);
        h.attack = None;
    });
    assert_eq!(hunt_part_of(&mut t, 50), Ok(()));
    assert_eq!(notified(&t, SEARCH_NEEDED), [[0; 3]; 0]);
    t.players[1].objects[0].actions.hunt_mut().unwrap().timer = -0.01;
    assert_eq!(hunt_part_of(&mut t, 50), Ok(()));
    assert_eq!(notified(&t, SEARCH_NEEDED), [search]);
    assert_eq!(
        t.players[1].objects[0].actions.hunt().map(|h| h.timer),
        Some(SEARCH_AGAIN)
    );
    let mut t = hunter_in(|h| {
        (h.state, h.target) = (LEAD, -1);
        h.attack = None;
    });
    assert_eq!(hunt_part_of(&mut t, 50), Ok(()));
    assert_eq!(
        t.players[1].objects[0].actions.hunt().map(|h| h.state),
        Some(SEARCH)
    );
    assert_eq!(notified(&t, SEARCH_NEEDED), [search]);
    let mut t = hunter_in(|h| {
        h.state = IDLE;
        h.attack = None;
    });
    let before = t.players[1].objects[0].actions.hunt().copied();
    assert_eq!(hunt_part_of(&mut t, 50), Ok(()));
    assert_eq!(t.players[1].objects[0].actions.hunt().copied(), before);
    t.players[1].objects[0]
        .actions
        .hunt_mut()
        .unwrap()
        .target_class = 15;
    assert!(hunt_part_of(&mut t, 50).is_err());
}

/// The walks to a point (states 15 and 11): arrived, state 15 is done (`set_state(2)`: the
/// target goes and is let go) and state 11 searches (`set_state(3)`: notify 0x202); a waiting
/// walk asks again after its delay. Waiting after a given-up `MoveTo` without its target (state
/// 4, its timer below 0) the timer runs, then the walk to the point (state 11).
#[test]
fn a_hunt_s_walk_to_a_point_ends_done_or_in_a_search() {
    use crate::gathering::hunt::{IDLE, SEARCH, WALK, WALK_POINT, WALK_SEARCH};
    use crate::movement::move_to::MoveTo;
    let walked = |state| {
        hunter_in(|h| {
            (h.state, h.move_to) = (state, Some(MoveTo::Arrived));
            h.attack = None;
        })
    };
    let mut t = walked(WALK_POINT);
    assert_eq!(hunt_part_of(&mut t, 50), Ok(()));
    let h = t.players[1].objects[0].actions.hunt().copied().unwrap();
    assert_eq!((h.state, h.target, h.move_to), (IDLE, -1, None));
    assert_eq!(t.players[0].objects[0].workers, 0);
    let mut t = walked(WALK_SEARCH);
    assert_eq!(hunt_part_of(&mut t, 50), Ok(()));
    assert_eq!(
        t.players[1].objects[0].actions.hunt().map(|h| h.state),
        Some(SEARCH)
    );
    assert_eq!(notified(&t, SEARCH_NEEDED).len(), 1);
    let mut t = hunter_in(|h| {
        (h.state, h.move_to) = (WALK_POINT, Some(MoveTo::Waiting));
        h.move_wait.delay = 1.0;
        h.attack = None;
    });
    assert_eq!(hunt_part_of(&mut t, 50), Ok(()));
    let h = t.players[1].objects[0].actions.hunt().copied().unwrap();
    assert_eq!((h.move_to, h.state), (Some(MoveTo::Waiting), WALK_POINT));
    let mut t = hunter_in(|h| {
        (h.state, h.target, h.timer, h.move_to) = (WALK, -1, -0.01, None);
        h.point = [36.0, 89.5, 0.0];
        h.attack = None;
    });
    assert_eq!(hunt_part_of(&mut t, 50), Ok(()));
    assert_eq!(
        t.players[1].objects[0].actions.hunt().map(|h| h.state),
        Some(WALK_SEARCH)
    );
}

/// The hunt's strike (`0x140ac7a00`'s states 12 and 7, [`Unit::hunt_strike`]): the hunt
/// command's Attack on the live deer walks to it (state 4, its `MoveTo` moving on a path).
/// Gathering the dead deer (work rate 1 by work value 1 a second) the hunter gains 0.05 in
/// 50 ms and the deer loses it; with less room than that it is full and carries its load to the
/// town centre (state 8) without taking it.
#[test]
fn a_hunter_strikes_its_live_deer_and_gathers_the_dead_one_until_full() {
    use crate::gathering::hunt::{ATTACK, DROP, GATHER};
    use crate::movement::move_to::MoveTo;
    let mut t = hunter_in(|_| {});
    assert_eq!(hunt_part_of(&mut t, 50), Ok(()));
    let v = &t.players[1].objects[0];
    let h = v.actions.hunt().copied().unwrap();
    assert_eq!(
        (h.state, h.attack.map(|a| (a.state, a.move_to))),
        (ATTACK, Some((4, Some(MoveTo::Moving))))
    );
    assert!(v.body.mover().is_some_and(|m| !m.paths.is_empty()));
    let gathering = |held: f32| {
        let mut t = hunter_in(|h| {
            (h.state, h.attack, h.move_to) = (GATHER, None, None);
            (h.task.work_value, h.task.resource) = (1.0, 15);
        });
        for masters in &mut t.masters {
            let hunter = masters.get_mut(122).unwrap();
            (hunter.capacity, hunter.work_rate) = (10, 1.0);
        }
        let deer = &mut t.players[0].objects[0];
        (deer.life.state, deer.life.hp) = (3, 0.0);
        (deer.life.attribute_type, deer.life.amount) = (15, 100.0);
        let v = &mut t.players[1].objects[0];
        (v.life.attribute_type, v.life.amount) = (15, held);
        v.place_for_test([31.9, 89.5, 0.0]);
        t.spatial.set_position(9175, [31.9, 89.5, 0.0]);
        t
    };
    let mut t = gathering(5.0);
    assert_eq!(hunt_part_of(&mut t, 50), Ok(()));
    let (gained, left) = (
        t.players[1].objects[0].life.amount,
        t.players[0].objects[0].life.amount,
    );
    assert!((gained - 5.05).abs() < 1e-5 && (left - 99.95).abs() < 1e-4);
    assert_eq!(
        t.players[1].objects[0].actions.hunt().map(|h| h.state),
        Some(GATHER)
    );
    let mut t = gathering(9.99);
    assert_eq!(hunt_part_of(&mut t, 50), Ok(()));
    let h = t.players[1].objects[0].actions.hunt().copied().unwrap();
    assert_eq!((h.state, h.carry_to), (DROP, 700));
    assert_eq!(t.players[1].objects[0].life.amount, 9.99);
    assert_eq!(t.players[0].objects[0].life.amount, 100.0);
}

/// A hunter's work on its dead deer reaches the deer where it ran (the hit's write-back,
/// [`crate::combat::victims::write_back`]): in the same update the deer holds 0.05 less.
#[test]
fn a_hunter_s_work_reaches_its_deer_in_the_same_update() {
    use crate::gathering::hunt::GATHER;
    let mut t = hunter_in(|h| {
        (h.state, h.attack, h.move_to) = (GATHER, None, None);
        (h.task.work_value, h.task.resource) = (1.0, 15);
    });
    for masters in &mut t.masters {
        let hunter = masters.get_mut(122).unwrap();
        (hunter.capacity, hunter.work_rate) = (10, 1.0);
    }
    let deer = &mut t.players[0].objects[0];
    (deer.life.state, deer.life.hp) = (3, 0.0);
    (deer.life.attribute_type, deer.life.amount) = (15, 100.0);
    let v = &mut t.players[1].objects[0];
    (v.life.attribute_type, v.life.amount) = (15, 5.0);
    v.place_for_test([31.9, 89.5, 0.0]);
    t.spatial.set_position(9175, [31.9, 89.5, 0.0]);
    let left = in_update(&mut t, (1, 0), 50, |unit, c| {
        let me = crate::movement::Me {
            id: unit.id,
            group: unit.group.id,
            action_state: unit.top_action().map_or(0, |a| a.state),
        };
        let mut own = unit.own_victim(c);
        let tree_lost = unit.tree_lost();
        let code = unit.hunt_part(c, me, &mut own, tree_lost)?;
        unit.end_head_update(c, code)?;
        Ok(OthersRef::of(&c.w.players, c.player)
            .find(500)
            .map(|(_, deer)| deer.life.amount))
    });
    assert!(
        left.as_ref()
            .is_ok_and(|l| l.is_some_and(|l| (l - 99.95).abs() < 1e-4)),
        "{left:?}"
    );
}

/// A gathering hunt (state 7) whose deer left the world drops it at its update's start and
/// ends without a target in the same update: holding nothing, it searches (state 3), reading no
/// target.
#[test]
fn a_gathering_hunt_whose_target_left_searches_without_reading_one() {
    use crate::gathering::hunt::{GATHER, SEARCH};
    let mut t = hunter_in(|h| (h.state, h.attack, h.move_to) = (GATHER, None, None));
    let _ = t.players[0].objects.remove(0);
    let _ = t.spatial.leave(500);
    let r = in_update(&mut t, (1, 0), 50, |unit, c| {
        unit.hunt_targets(c)?;
        let me = crate::movement::Me {
            id: unit.id,
            group: unit.group.id,
            action_state: unit.top_action().map_or(0, |a| a.state),
        };
        let mut own = unit.own_victim(c);
        let code = unit.hunt_part(c, me, &mut own, false)?;
        unit.end_head_update(c, code)
    });
    assert_eq!(r, Ok(()));
    let h = t.players[1].objects[0].actions.hunt().copied().unwrap();
    assert_eq!((h.state, h.target), (SEARCH, -1));
}

/// [`hunt_world`] with player 1's own sheep 510 (master 594, class 58, moving) at `x` beside
/// the hunter, its hunter's task on livestock, the drop-site table's town centre taking
/// livestock; the hunt command's hunt faces it (state 4, arrived).
fn shepherd_world(x: f32) -> World {
    use crate::movement::move_to::MoveTo;
    let mut t = hunt_world();
    for masters in &mut t.masters {
        let sheep = Master {
            id: 594,
            kind: 70,
            class: 58,
            radius: [0.2, 0.2],
            moving: Some(crate::gathering::hunt::tests::walker().stats),
            ..Master::default()
        };
        *masters = Masters::new(masters.iter().cloned().chain([sheep]));
        masters
            .get_mut(122)
            .unwrap()
            .tasks
            .push(crate::objects::masters::Task {
                action_type: 110,
                object_group: 58,
                work_range: 0.5,
                ..crate::objects::masters::Task::default()
            });
    }
    t.players[1].objects.push(Unit {
        id: 510,
        life: Lifecycle {
            state: 2,
            hp: 7.0,
            master_id: 594,
            attribute_type: 15,
            amount: 100.0,
            ..Lifecycle::default()
        },
        body: crate::objects::layers::Body::new(
            [x, 89.5, 0.0],
            None,
            Some(crate::gathering::hunt::tests::walker()),
        ),
        pasture: -1,
        my_pasture: -1,
        ..Unit::default()
    });
    t.spatial.insert(
        510,
        Placed {
            position: [x, 89.5, 0.0],
            radius: [0.2, 0.2],
            class: 58,
            player: 1,
            master: 594,
            obstruction_type: 0,
            buildable_on: 0,
            tall: true,
            speed: 0.0,
            unknown: None,
            obstruction: None,
        },
    );
    t.drop_sites = vec![DropSite {
        building: 109,
        worker_class: 4,
        livestock: true,
        ..DropSite::default()
    }];
    t.hunt_command(1, 0, 510).unwrap();
    let h = t.players[1].objects[0].actions.hunt_mut().unwrap();
    (h.move_to, h.attack) = (Some(MoveTo::Arrived), None);
    t
}

/// A hunt facing its own live sheep (`set_state(6)` on livestock with its herd,
/// [`Unit::herd_for`]): the sheep stands more than 3 tiles from the drop-site table's nearest
/// town centre, so the hunter leads it there (state 10, the centre its drop site) and the sheep
/// walks; the lead goes on in the next update. A sheep beside the centre is slaughtered at once
/// (state 12, the Attack).
#[test]
fn a_hunter_leads_its_far_sheep_to_the_town_centre_and_slaughters_a_near_one() {
    use crate::gathering::hunt::{ATTACK, LEAD};
    let mut t = shepherd_world(30.2);
    assert_eq!(hunt_part_of(&mut t, 50), Ok(()));
    let h = t.players[1].objects[0].actions.hunt().copied().unwrap();
    assert_eq!((h.state, h.target, h.drop_site), (LEAD, 510, 700));
    assert!(t.players[1].objects[3].actions.move_to().is_some());
    assert_eq!(hunt_part_of(&mut t, 50), Ok(()));
    assert_eq!(
        t.players[1].objects[0].actions.hunt().map(|h| h.state),
        Some(LEAD)
    );
    let mut t = shepherd_world(22.6);
    t.players[1].objects[0].place_for_test([23.2, 89.5, 0.0]);
    t.spatial.set_position(9175, [23.2, 89.5, 0.0]);
    assert_eq!(hunt_part_of(&mut t, 50), Ok(()));
    let h = t.players[1].objects[0].actions.hunt().copied().unwrap();
    assert_eq!((h.state, h.attack.map(|a| a.target)), (ATTACK, Some(510)));
}

/// [`hunt_world`] with Gaia's trees 520 and 521 (master 349, class 15, 100 wood, gathered by
/// themselves as group 0) at x `near` and 40, the hunter's task on trees, its unit AI targeting
/// class 15 and its hunt without a target at its own place.
fn lumber_world(near: f32) -> World {
    let mut t = hunt_world();
    for masters in &mut t.masters {
        let tree = Master {
            id: 349,
            kind: 10,
            class: 15,
            radius: [0.5, 0.5],
            auto_gather: true,
            gather_group: 0,
            ..Master::default()
        };
        *masters = Masters::new(masters.iter().cloned().chain([tree]));
        let hunter = masters.get_mut(122).unwrap();
        hunter.line_of_sight = 4.0;
        hunter.tasks.push(crate::objects::masters::Task {
            action_type: 110,
            object_group: 15,
            work_range: 0.5,
            ..crate::objects::masters::Task::default()
        });
    }
    let tree = |id, x: f32| Unit {
        id,
        body: crate::objects::layers::Body::at([x, 89.5, 0.0]),
        life: Lifecycle {
            state: 2,
            hp: 20.0,
            master_id: 349,
            attribute_type: 1,
            amount: 100.0,
            ..Lifecycle::default()
        },
        pasture: -1,
        my_pasture: -1,
        ..Unit::default()
    };
    t.players[0]
        .objects
        .extend([tree(520, near), tree(521, 40.0)]);
    for (id, x) in [(520, near), (521, 40.0)] {
        t.spatial.insert(
            id,
            Placed {
                position: [x, 89.5, 0.0],
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
    }
    t.spatial.relations = vec![vec![0, 3], vec![3, 0]];
    t.hunt_command(1, 0, 500).unwrap();
    let v = &mut t.players[1].objects[0];
    let here = v.position();
    let h = v.actions.hunt_mut().unwrap();
    (h.target, h.target_class, h.attack, h.point) = (-1, 15, None, here);
    v.ai.as_mut().unwrap().target_type = 15;
    t
}

/// [`lumber_world`] with its tree 521 asleep and a Gaia foundation 530 (state 0) of a building
/// master with a stack unit (master 99, which the game never uses) that is an annex of tree
/// 520.
fn lets_world() -> World {
    let mut t = lumber_world(31.5);
    let asleep = t.players[0].objects.swap_remove(3);
    t.players[0].sleeping.push(asleep);
    for masters in &mut t.masters {
        let annex = Master {
            id: 99,
            kind: crate::orders::BUILDING,
            building: Some(crate::objects::masters::BuildingMaster {
                stack_unit: 5,
                ..crate::objects::masters::BuildingMaster::default()
            }),
            ..Master::default()
        };
        *masters = Masters::new(masters.iter().cloned().chain([annex]));
    }
    t.players[0].objects.push(Unit {
        id: 530,
        annex_of: Some(520),
        life: Lifecycle {
            state: 0,
            master_id: 99,
            ..Lifecycle::default()
        },
        pasture: -1,
        my_pasture: -1,
        ..Unit::default()
    });
    t
}

/// An update's take through its view (slot 0x178, [`crate::world::lets::take`]): a live tree
/// counts its worker; a sleeping one counts its worker and wakes, last in its player's live
/// list and tracker; an unallied player's foundation with a stack unit lists its taker, and
/// Gaia's own taker is allied; one the world does not hold is not taken; the updating unit
/// itself is the world's after the step.
#[test]
fn an_update_s_take_counts_its_worker_and_wakes_a_sleeping_object() {
    use crate::world::lets::take;
    let mut t = lets_world();
    let gaia = Releaser {
        id: 500,
        class: 9,
        player: 0,
    };
    let deferred = in_update(&mut t, (1, 0), 50, |unit, c| {
        let by = unit.releaser(c);
        for id in [520, 521, 530, 9999, unit.id] {
            take(c, id, by);
        }
        take(c, 530, gaia);
        Ok(c.takes.iter().map(|&(id, _)| id).collect::<Vec<_>>())
    });
    assert_eq!(deferred, Ok(vec![t.players[1].objects[0].id]));
    let workers = |t: &World, id| t.object(id).map(|(_, u)| u.workers);
    assert_eq!(workers(&t, 520), Some(1));
    assert_eq!(workers(&t, 521), Some(1));
    assert_eq!(t.players[0].objects.iter().last().map(|u| u.id), Some(521));
    assert!(t.players[0].sleeping.by_id(521).is_none());
    let k = crate::players::tracker::key(t.masters.first(), &t.lines, 349);
    assert_eq!(t.players[0].tracker.list(k.master).last(), Some(&521));
    let villager = t.players[1].objects[0].id;
    assert_eq!(workers(&t, villager), Some(1));
    let annex = t
        .object(530)
        .map(|(_, u)| (u.workers, u.attacking_units.clone()));
    assert_eq!(annex, Some((2, vec![villager])));
}

/// An update's release through its view (slot 0x180, [`crate::world::lets::release`]): a live
/// tree no one works on any more goes to sleep; a sleeping one counts a worker less and stays
/// asleep; one the world does not hold is not let go; the updating unit itself is the world's
/// after the step. Then a counted builder's leave ([`crate::world::lets::Lets::leave_builders`]): an
/// object counts one builder less, an annex's main building for the annex; the teardown's
/// pair applies leaves before releases ([`crate::world::lets::torn_down`]).
#[test]
fn an_update_s_release_and_builder_leave_change_the_objects_at_once() {
    use crate::world::lets::{leave_builders, release, torn_down};
    let mut t = lets_world();
    for id in [520, 521] {
        let u = t.object_mut(id).unwrap().1;
        (u.workers, u.builders) = (2, 3);
    }
    let deferred = in_update(&mut t, (1, 0), 50, |unit, c| {
        let by = unit.releaser(c);
        for id in [520, 521, 9999, unit.id] {
            release(c, id, by)?;
        }
        leave_builders(c, 520)?;
        leave_builders(c, 530)?;
        torn_down(c, (vec![521], vec![(520, by)]))?;
        Ok(c.releases.iter().map(|&(id, _)| id).collect::<Vec<_>>())
    });
    assert_eq!(deferred, Ok(vec![t.players[1].objects[0].id]));
    let tree = |t: &World, id| {
        t.object(id)
            .map(|(_, u)| (u.workers, u.builders, u.life.goto_sleep))
    };
    assert_eq!(tree(&t, 520), Some((0, 1, true)));
    assert_eq!(tree(&t, 521), Some((1, 2, false)));
    assert!(t.players[0].sleeping.by_id(521).is_some());
}

/// Outside an update a take, a release and a builder's leave run the update's calls on a view
/// of the whole world ([`World::with_lets`]): the sleeping tree wakes to the end of Gaia's live
/// list and tracker, a release lets it sleep again, and the annex's main building counts a
/// builder less.
#[test]
fn outside_an_update_the_lets_run_on_the_whole_world() {
    let mut t = lets_world();
    let by = Releaser {
        id: t.players[1].objects[0].id,
        class: 4,
        player: 1,
    };
    t.take(521, by);
    assert_eq!(t.players[0].objects.iter().last().map(|u| u.id), Some(521));
    let k = crate::players::tracker::key(t.masters.first(), &t.lines, 349);
    assert_eq!(t.players[0].tracker.list(k.master).last(), Some(&521));
    assert_eq!(t.object(521).map(|(_, u)| u.workers), Some(1));
    t.release(521, by).unwrap();
    let tree = t.object(521).map(|(_, u)| (u.workers, u.life.goto_sleep));
    assert_eq!(tree, Some((0, true)));
    t.object_mut(520).unwrap().1.builders = 2;
    t.leave_builders(530).unwrap();
    assert_eq!(t.object(520).map(|(_, u)| u.builders), Some(1));
}

/// A player's live object out of its slot ([`World::take_out_own`]) is out of the world's
/// lookups as an updating one is, its stand-in in its place, until it goes back
/// ([`World::put_back_own`]); the sleeping object wakes once ([`World::wake_sleeper`]).
#[test]
fn an_own_object_taken_out_leaves_the_world_until_it_goes_back() {
    let mut t = lets_world();
    let id = t.players[1].objects[0].id;
    assert!(t.take_out_own(0, id).is_none(), "another player's");
    let mut u = t.take_out_own(1, id).expect("the owner's");
    assert_eq!(u.id, id);
    assert!(t.object(id).is_none() && t.locate(id).is_none());
    assert_eq!(t.players[1].objects[0].id, crate::objects::unit::TAKEN_OUT);
    u.life.hp = 8.0;
    assert_eq!(t.put_back_own(1, u).ok(), Some(0));
    assert_eq!(
        t.locate(id).map(|(q, u, s)| (q, u.life.hp, s)),
        Some((1, 8.0, false))
    );
    assert_eq!(t.locate(521).map(|(q, _, s)| (q, s)), Some((0, true)));
    assert!(t.wake_sleeper(521));
    assert!(!t.wake_sleeper(521), "awake already");
    assert_eq!(t.locate(521).map(|(q, _, s)| (q, s)), Some((0, false)));
}

/// The unit AI's stop (`Effect::Stop`, [`Unit::stop_action`]): the forager's Gather goes and
/// its bush counts a worker less within the update, through the view, not after the step.
#[test]
fn a_unit_ai_s_stop_lets_its_target_go_within_the_update() {
    let mut t = forage_world();
    let work = crate::orders::Work {
        target: 600,
        point: [32.5, 89.5, 0.0],
        carry_test: false,
        filter: -1,
        notified: true,
    };
    t.order_work(1, 0, &work).unwrap();
    assert_eq!(t.players[0].objects[0].workers, 1);
    let seen = in_update(&mut t, (1, 0), 50, |unit, c| {
        unit.stop_action(c)?;
        let bush = OthersRef::of(&c.w.players, c.player)
            .find(600)
            .map(|(_, b)| b.workers);
        Ok((bush, c.releases.len(), unit.actions.gather().is_some()))
    });
    assert_eq!(seen, Ok((Some(0), 0, false)));
    assert_eq!(t.players[0].objects[0].workers, 0);
}

/// The hunt's search for the next tree (`FUN_140ac4670`, [`Unit::hunt_tree_search`]): the
/// unit AI's `ClosestResourceObject` of group 0 names the nearer tree, which the hunt takes
/// (slot 0x48: its Attack on the tree) and the tree counts the hunter. A hunt farther than
/// `within` from its point, a tree farther than `near` from the unit, or no tree, find none.
#[test]
fn a_hunt_that_lost_its_tree_takes_the_next_one() {
    let search = |t: &mut World, within, near| {
        in_update(t, (1, 0), 50, |unit, c| {
            unit.hunt_tree_search(c, within, near)
        })
    };
    let mut t = lumber_world(31.5);
    assert_eq!(search(&mut t, Some(8.0), None), Ok(true));
    let h = t.players[1].objects[0].actions.hunt().copied().unwrap();
    assert_eq!((h.target, h.attack.map(|a| a.target)), (520, Some(520)));
    assert_eq!(t.players[0].objects[2].workers, 1);
    let mut t = lumber_world(31.5);
    t.players[1].objects[0].actions.hunt_mut().unwrap().point = [40.0, 89.5, 0.0];
    assert_eq!(search(&mut t, Some(8.0), None), Ok(false));
    assert_eq!(search(&mut t, None, Some(1.5)), Ok(false));
    let mut t = lumber_world(31.5);
    for i in [2, 3] {
        t.players[0].objects[i].life.amount = 0.0;
    }
    assert_eq!(search(&mut t, None, None), Ok(false));
}

/// A hunt's end of an Attack that failed (`FUN_140ac4670` with -1 and 5,
/// [`Unit::hunt_attack_failed`]): a hunter whose unit AI targets trees takes the next tree
/// within 5 of it; with none so near, or a unit AI that targets another class, the hunt
/// searches (state 3, its timer at -10 s) with its target kept.
#[test]
fn a_hunt_whose_attack_failed_takes_a_near_tree_or_searches() {
    let failed = |t: &mut World| in_update(t, (1, 0), 50, Unit::hunt_attack_failed);
    let mut t = lumber_world(31.5);
    assert_eq!(failed(&mut t), Ok(()));
    let h = t.players[1].objects[0].actions.hunt().copied().unwrap();
    assert_eq!((h.target, h.attack.map(|a| a.target)), (520, Some(520)));
    for (near, target_type) in [(39.0, 15), (31.5, 99)] {
        let mut t = lumber_world(near);
        t.players[1].objects[0].ai.as_mut().unwrap().target_type = target_type;
        assert_eq!(failed(&mut t), Ok(()));
        let h = t.players[1].objects[0].actions.hunt().copied().unwrap();
        assert_eq!(
            (h.state, h.timer, h.target),
            (crate::gathering::hunt::SEARCH, SEARCH_AGAIN, -1)
        );
    }
}

/// The unit AI's repair order on the player's damaged house (`0x2ce`, slot 0x378 forced,
/// `FUN_141217c00`): the villager (83, task swap group 1) becomes its player's repairer (156,
/// slot 0x90), whose task list gives the Repair task; after the update the Repair walks to the
/// house (state 4) and the AI holds the repair action on it. Without the repairer master the
/// villager's own list must give the task, which it does not: the order is not taken.
#[test]
fn the_unit_ai_s_repair_order_switches_a_villager_to_the_repairer() {
    use crate::unit_ai::QueuedOrder;
    let order = QueuedOrder {
        issuer: 1,
        order: OrderKind::Repair,
        priority: 100,
        target: 10028,
        target_owner: 1,
        point: [30.0, 88.0, 0.0],
        range: 0.0,
        shown: [30.0, 88.0],
        ready: true,
        target_type: 80,
    };
    let world = |repairer: bool| {
        let mut t = sent_off_world(vec![order]);
        let house = &mut t.players[1].objects[1];
        house.life.state = 2;
        house.life.hp = 300.0;
        let c = house.construction.as_mut().unwrap();
        (c.progress, c.desolid) = (1.0, false);
        if repairer {
            for masters in &mut t.masters {
                let mut m = masters.get(83).unwrap().clone();
                m.id = 156;
                m.weapon_range = 0.5;
                m.tasks.push(crate::objects::masters::Task {
                    id: 4,
                    work_val1: 0.5,
                    ..task(crate::construction::repair::REPAIR)
                });
                *masters = Masters::new(masters.iter().cloned().chain([m]));
            }
        }
        t
    };
    let mut t = world(true);
    t.tick(50);
    let v = &t.players[1].objects[0];
    assert_eq!(v.life.master_id, 156);
    let r = v.actions.repair().copied().expect("the Repair");
    assert_eq!((r.target, r.state, r.range, r.task.id), (10028, 4, 0.5, 4));
    let ai = v.ai.as_ref().unwrap();
    assert_eq!(
        (ai.current_action, ai.current_target),
        (Some(ActionKind::Repair), 10028)
    );
    let mut t = world(false);
    t.tick(50);
    let v = &t.players[1].objects[0];
    assert_eq!((v.life.master_id, v.actions.repair().copied()), (83, None));
}

/// The world of [`sent_off_world`] with the queue `orders`, the house built and whole, taking
/// villagers in with room for `room`, both placed and the villager moving.
fn enter_world(orders: Vec<crate::unit_ai::QueuedOrder>, room: u8) -> World {
    let mut t = sent_off_world(orders);
    let house = &mut t.players[1].objects[1];
    house.life.state = 2;
    house.life.hp = 550.0;
    let c = house.construction.as_mut().unwrap();
    (c.progress, c.desolid) = (1.0, false);
    for masters in &mut t.masters {
        let house = masters.get_mut(70).unwrap();
        house.garrison = room;
        // it takes villagers in (bit 0)
        house.building.as_mut().unwrap().garrison_type = 1;
    }
    t.players[1].objects[0]
        .body
        .put_mover(Some(crate::gathering::hunt::tests::walker()));
    t.spatial = Spatial::with_tiles(
        [120, 120],
        crate::map::spatial::Tiles::new(120, 120, vec![4; 14400]),
    );
    t.spatial.insert(
        10028,
        Placed {
            position: [30.0, 88.0, 0.0],
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
    t.spatial.insert(
        9175,
        Placed {
            position: [29.5, 89.5, 0.0],
            radius: [0.2, 0.2],
            class: 4,
            player: 1,
            master: 83,
            obstruction_type: 0,
            buildable_on: 0,
            tall: true,
            speed: 0.8,
            unknown: None,
            obstruction: None,
        },
    );
    t
}

/// Slot 0x3a0 (`FUN_1410847e0`) itself: a target the world does not hold is 0; a task found
/// gives the unit its Enter (1); unforced, a unit entering the target already keeps its Enter
/// (1), forced it takes a new one.
#[test]
fn the_enter_command_enters_or_keeps_the_enter_on_its_target() {
    let mut t = enter_world(vec![], 5);
    // the villager out of reach, so that it walks
    t.players[1].objects[0].place_for_test([20.0, 80.0, 0.0]);
    t.spatial.set_position(9175, [20.0, 80.0, 0.0]);
    assert_eq!(t.enter_command(1, 0, 4242, true), Ok(false));
    assert_eq!(t.players[1].objects[0].actions.enter().copied(), None);
    assert_eq!(t.enter_command(1, 0, 10028, true), Ok(true));
    let first = t.players[1].objects[0]
        .actions
        .enter()
        .copied()
        .expect("the Enter");
    assert_eq!((first.target, first.state), (10028, 4));
    t.players[1].objects[0].actions.enter_mut().unwrap().timer = 7.0;
    assert_eq!(t.enter_command(1, 0, 10028, false), Ok(true));
    assert_eq!(
        t.players[1].objects[0]
            .actions
            .enter()
            .copied()
            .unwrap()
            .timer,
        7.0
    );
    assert_eq!(t.enter_command(1, 0, 10028, true), Ok(true));
    assert_eq!(
        t.players[1].objects[0]
            .actions
            .enter()
            .copied()
            .unwrap()
            .timer,
        first.timer
    );
}

/// The Enter's start (`FUN_140a86f20`) of a unit inside an object: `set_state(13)`, its
/// standing sprite and notify 0x1f9 with the enter's action value to its unit AI; the update
/// then returns 3 and the list removes it.
#[test]
fn an_enter_of_a_unit_inside_an_object_fails_at_its_start() {
    let mut t = enter_world(vec![], 5);
    t.players[1].objects[0].life.inside = Some(4242);
    assert_eq!(t.enter_command(1, 0, 10028, true), Ok(true));
    let v = &t.players[1].objects[0];
    assert_eq!(
        v.actions.enter().map(|e| e.state),
        Some(crate::garrison::FAILED)
    );
    assert_eq!(
        v.ai.as_ref().unwrap().notifies,
        [Notify::own(
            9175,
            crate::unit_ai::ACTION_FAILED,
            [ActionKind::Enter.value(), 0, 0]
        )]
    );
}

/// [`enter_world`] whose villager is a sheep under its Artifact, the house taking herdables in.
fn livestock_enter_world() -> World {
    let mut t = enter_world(vec![], 5);
    let sheep = Master {
        id: 594,
        kind: 70,
        class: 58,
        radius: [0.2, 0.2],
        tasks: vec![crate::objects::masters::Task {
            id: 1,
            action_type: crate::garrison::ENTER,
            object_group: 3,
            ..crate::objects::masters::Task::default()
        }],
        moving: Some(crate::gathering::hunt::tests::walker().stats),
        ..Master::default()
    };
    for masters in &mut t.masters {
        *masters = Masters::new(masters.iter().cloned().chain([sheep.clone()]));
        // the house takes herdables in (bit 4)
        masters
            .get_mut(70)
            .unwrap()
            .building
            .as_mut()
            .unwrap()
            .garrison_type = 1 << 4;
    }
    let v = &mut t.players[1].objects[0];
    v.life.master_id = 594;
    v.actions.set_artifact(Some(Artifact::new(false, None)));
    t
}

/// The enter command of livestock under its Artifact (`FUN_1410847e0`): the Enter becomes the
/// Artifact's sub-action (its list emptied, `FUN_140a98e20`, then `FUN_140a977e0`), the Artifact
/// staying the top action; the next update runs it through the Artifact's.
#[test]
fn livestock_s_enter_command_becomes_its_artifact_s_sub_action() {
    let mut t = livestock_enter_world();
    assert_eq!(t.enter_command(1, 0, 10028, false), Ok(true));
    let v = &t.players[1].objects[0];
    assert_eq!(v.actions.enter().copied(), None);
    assert_eq!(
        v.top_action_kind(),
        Some(crate::ownership::artifact::ARTIFACT_TYPE)
    );
    let sub = v.actions.artifact().copied().unwrap().enter;
    assert!(
        sub.is_some_and(|e| e.target == 10028) || v.life.inside == Some(10028),
        "{v:?}"
    );
}

/// The Enter's `MoveTo` shows its sprite as it starts (`FUN_140a87320` case 4 gives its
/// constructor the task's moving sprite, `0x140aa29e0`'s `set_state(4)` shows it): the task's,
/// else `FUN_140aa0df0`'s default toward a building, the master's walking sprite.
#[test]
fn an_enter_s_move_to_shows_its_sprite_as_it_starts() {
    for (moving, shown) in [(Some(30), 30), (None, 31)] {
        let mut t = enter_world(vec![], 5);
        t.pathing = Pathing::new([120, 120], vec![0; 120 * 120], vec![vec![1.0]; 8], 0);
        t.build_mesh();
        for id in [30, 31] {
            let g = Graphic {
                id,
                frames: 10,
                facets: 16,
                speed_multiplier: 1.0,
                frame_duration: 0.1,
                replay_delay: 0.0,
                flags: 1,
            };
            let _ = t.graphics.insert(id, g);
        }
        t.masters[1].get_mut(83).unwrap().move_sprite = 31;
        let v = &mut t.players[1].objects[0];
        let here = v.position();
        v.body.mover_mut().unwrap().moved_at = here;
        let task = crate::garrison::tests::enter_task(0.0, 3);
        v.actions.set_enter(Some(crate::garrison::EnterAction {
            move_to: Some(MoveTo::Start),
            moving,
            ..crate::garrison::EnterAction::new(10028, [30.0, 88.0, 0.0], &task)
        }));
        let _ = in_update(&mut t, (1, 0), 50, Unit::update).unwrap();
        let v = &t.players[1].objects[0];
        assert_eq!(
            v.actions.enter().map(|e| e.move_to),
            Some(Some(MoveTo::Moving))
        );
        assert_eq!(v.sprite.map(|s| s.graphic.id), Some(shown));
    }
}

/// A second enter command of livestock walking under its Artifact (430061417's pig 7920 in
/// update 32, INTERACT on the transport again): the new Enter takes the house (slot 0x178), then
/// `FUN_140a98e20` pops the old one, whose teardown lets the house go (slot 0x180) and whose
/// `MoveTo`'s stops the unit: its speed and acceleration 0, so the update's velocity step does
/// not move it.
#[test]
fn livestock_s_second_enter_command_stops_the_walk_and_lets_the_old_enter_s_target_go() {
    let mut t = livestock_enter_world();
    let mut walking = crate::garrison::EnterAction::new(
        10028,
        [30.0, 88.0, 0.0],
        &crate::garrison::tests::enter_task(0.0, 3),
    );
    walking.move_to = Some(MoveTo::Moving);
    let v = &mut t.players[1].objects[0];
    v.actions.artifact_mut().unwrap().enter = Some(walking);
    v.body.mover_mut().unwrap().velocity = Some(crate::movement::Velocity {
        velocity: [0.0, 0.0, 0.7],
        acceleration: [0.0, 0.0, f32::MAX],
    });
    // the old Enter took the house
    t.players[1].objects[1].workers = 1;
    assert_eq!(t.enter_command(1, 0, 10028, true), Ok(true));
    let v = &t.players[1].objects[0];
    let velocity = v.body.mover().unwrap().velocity.unwrap();
    assert_eq!((velocity.velocity[2], velocity.acceleration[2]), (0.0, 0.0));
    assert_eq!(t.players[1].objects[1].workers, 1);
}

/// The inside update (`FUN_140a98790`) of a villager inside the house deletes its top action
/// whatever its kind: a `MoveTo` left from before the enter goes (its destructor stops the
/// unit), and the update after has nothing left to delete (426027790-style villagers in a town
/// bell: 427445417's p3, 429184426's p4).
#[test]
fn an_inside_unit_s_update_deletes_its_top_action() {
    let mut t = enter_world(vec![], 5);
    t.players[1].objects[0].life.inside = Some(10028);
    t.spatial.go_inside(9175, [30.0, 88.0, 0.0]);
    t.players[1].objects[0]
        .actions
        .set_move_to(Some(MoveToAction::to_point(
            [40.0, 80.0, 0.0],
            0.0,
            None,
            None,
        )));
    t.tick(50);
    let v = &t.players[1].objects[0];
    assert_eq!(v.actions.move_to().copied(), None);
    assert_eq!(v.top_action_kind(), None);
    assert_eq!(t.players[1].freezes, vec![]);
}

/// An Enter whose walk arrived in reach gets its unit in in its update (`FUN_140a867b0`): the
/// building adds the unit to its list in the same update, its own player's and an ally's
/// (player 2's) alike.
#[test]
fn an_enter_s_update_adds_its_unit_to_its_building_own_or_an_ally_s() {
    let entered = |ally: bool| {
        let mut t = enter_world(vec![], 5);
        if ally {
            let house = t.players[1].objects.remove(1);
            let mut ally = Player {
                attributes: vec![0.0; 250],
                ..Player::default()
            };
            ally.objects.push(house);
            t.players.push(ally);
            t.masters.push(t.masters[1].clone());
            let mut placed = *t.spatial.get(10028).unwrap();
            placed.player = 2;
            t.spatial.insert(10028, placed);
            t.players[1].relations = vec![3, 0, 0];
            t.players[2].relations = vec![3, 0, 0];
            t.spatial.relations = vec![vec![0, 3, 3], vec![3, 0, 0], vec![3, 0, 0]];
        }
        let mut e = crate::garrison::EnterAction::new(
            10028,
            [30.0, 88.0, 0.0],
            &crate::garrison::tests::enter_task(2.0, 3),
        );
        e.move_to = Some(MoveTo::Arrived);
        let v = &mut t.players[1].objects[0];
        v.place_for_test([29.5, 89.5, 0.0]);
        v.actions.set_enter(Some(e));
        in_update(&mut t, (1, 0), 50, |unit, c| {
            let me = crate::movement::Me {
                id: unit.id,
                group: unit.group.id,
                action_state: unit.top_action().map_or(0, |a| a.state),
            };
            let code = unit.enter_update(c, me)?;
            unit.end_head_update(c, code)?;
            let (_, house) = OthersRef::of(&c.w.players, c.player).find(10028).unwrap();
            Ok((unit.life.inside, house.garrison.clone()))
        })
    };
    assert_eq!(entered(false), Ok((Some(10028), vec![9175])));
    assert_eq!(entered(true), Ok((Some(10028), vec![9175])));
}

/// An Enter that got the unit in, the unit let out again before its next update: that update's
/// 1 has the action list remove it (its teardown lets the building go).
#[test]
fn an_entered_enter_of_a_unit_let_out_goes_at_its_update() {
    let mut t = enter_world(vec![], 5);
    let mut e = crate::garrison::EnterAction::new(
        10028,
        [30.0, 88.0, 0.0],
        &crate::garrison::tests::enter_task(0.5, 3),
    );
    e.state = crate::garrison::ENTERED;
    t.players[1].objects[0].actions.set_enter(Some(e));
    t.tick(50);
    t.tick(50);
    assert_eq!(t.players[1].objects[0].actions.enter().copied(), None);
    assert_eq!(t.players[1].freezes, vec![]);
}

/// The unit AI's enter order on the player's house (`0x2cb`, slot 0x3a0 forced,
/// `FUN_1410847e0`): its slot 0x820 with the garrison filter finds the villager's Enter task on
/// buildings (action 3, class 3); after the update the Enter is the unit's (or the unit inside
/// the house), and the AI holds the enter action on it. A house without room gives no task:
/// the order is not taken.
#[test]
fn the_unit_ai_s_enter_order_gives_the_unit_an_enter() {
    use crate::unit_ai::QueuedOrder;
    let order = QueuedOrder {
        issuer: 1,
        order: OrderKind::Enter,
        priority: 100,
        target: 10028,
        target_owner: 1,
        point: [30.0, 88.0, 0.0],
        range: 0.0,
        shown: [30.0, 88.0],
        ready: true,
        target_type: 80,
    };
    let mut t = enter_world(vec![order], 5);
    t.tick(50);
    let v = &t.players[1].objects[0];
    assert!(
        v.actions.enter().is_some_and(|e| e.target == 10028) || v.life.inside == Some(10028),
        "{:?}",
        t.players[1].freezes
    );
    let ai = v.ai.as_ref().unwrap();
    assert_eq!(
        (ai.current_action, ai.current_target),
        (Some(ActionKind::Enter), 10028)
    );
    let mut t = enter_world(vec![order], 0);
    t.tick(50);
    let v = &t.players[1].objects[0];
    assert_eq!(
        (t.players[1].freezes.len(), v.actions.enter().copied()),
        (0, None)
    );
    assert_ne!(
        v.ai.as_ref().unwrap().current_action,
        Some(ActionKind::Enter)
    );
}

/// The whole of it: a villager the foundation sent off (its queue the foundation's order)
/// runs the order on its turn (slot 0x108 forced: its slot 0x388 finds the builder's task by
/// the swap group), becomes the builder with the wait as its only action, and keeps waiting
/// while the foundation has no build points; once they pass 0.01 its next update drops the
/// wait, the unit AI's action value staying 0x27a.
#[test]
fn a_villager_sent_off_a_foundation_waits_for_it_as_a_builder() {
    use crate::unit_ai::QueuedOrder;
    let order = QueuedOrder {
        issuer: 1,
        order: OrderKind::Foundation,
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
        (
            Some(ActionKind::WaitSolidify),
            Some(ActionKind::WaitSolidify),
            Some(OrderKind::Foundation)
        )
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
    assert_eq!(v.actions.wait_solidify().copied(), None);
    assert_eq!(
        v.ai.as_ref().and_then(|ai| ai.current_action),
        Some(ActionKind::WaitSolidify)
    );
}

/// Slot 0x230's build order (`0x2be`): slot 0xf0 builds the foundation (the builder's task by
/// the swap group), then the tail makes the order the AI's with its priority, leaving a patrol
/// order: its secondary timer capped at 750 and an adjusted idle timeout of 750 redrawn.
#[test]
fn a_queued_build_order_builds_and_leaves_a_patrol_order() {
    use crate::unit_ai::QueuedOrder;
    let order = QueuedOrder {
        issuer: 1,
        order: OrderKind::Build,
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
        ai.current_order = Some(OrderKind::Patrol);
        ai.secondary_timer = 900;
        ai.adjusted_idle_timeout = 750;
        ai.idle_timeout = 3000;
    }
    let draws = t.streams.get(Stream::Ai).draws;
    // the unit AI runs the order (slot 0x230) and gives the build command, which runs after it
    t.run_unit_ai((1, 0), |ai, at, side, _, rng| {
        let mut fx = Vec::new();
        let r = ai.process_order(&order, Some((at, side, false)), None, rng, &mut fx)?;
        assert_eq!(r, 1);
        assert_eq!(fx, [crate::unit_ai::Effect::BuildObject(10028)]);
        Ok(fx)
    })
    .unwrap();
    let v = &t.players[1].objects[0];
    assert_eq!(v.life.master_id, 118);
    assert!(v.actions.build().is_some());
    let ai = v.ai.as_ref().unwrap();
    assert_eq!(
        (
            ai.current_order,
            ai.order_priority,
            ai.current_action,
            ai.current_target
        ),
        (Some(OrderKind::Build), 100, Some(ActionKind::Build), 10028)
    );
    assert_eq!(ai.secondary_timer, 750);
    assert_ne!(ai.adjusted_idle_timeout, 750);
    assert_eq!(t.streams.get(Stream::Ai).draws, draws + 1);
}

/// Slot 0x380 (`FUN_14120d410`) answers 0 and does nothing for a foundation the world does not
/// hold and for a unit without a task on it (its own master's list and its swap group's); the
/// Gather's `set_state(6)` ignores that 0.
#[test]
fn a_build_command_without_its_foundation_or_a_task_does_nothing() {
    let without_build_tasks = |t: &mut World| {
        for id in [118, 212] {
            let m = t.masters[1].get_mut(id).unwrap();
            m.tasks.retain(|k| k.action_type != 101);
        }
    };
    let mut t = sent_off_world(vec![]);
    let before = t.players[1].objects[0].clone();
    assert_eq!(t.build_command(1, 0, 99_999), Ok(false));
    without_build_tasks(&mut t);
    assert_eq!(t.build_command(1, 0, 10028), Ok(false));
    assert_eq!(t.players[1].objects[0], before);
    // with a task it builds as the builder 118
    let mut t = sent_off_world(vec![]);
    assert_eq!(t.build_command(1, 0, 10028), Ok(true));
    assert_eq!(t.players[1].objects[0].life.master_id, 118);
}

/// The unit AI's view answers slot 0x380 before `BuildObject` takes the build
/// (`UnitSide::builds`): 0 for a foundation the world does not hold or no task for it, 1 with a
/// task or a Build on top (its slot 0x48), whatever the master's lists then hold.
#[test]
fn the_unit_ai_s_view_answers_the_build_command_before_it_runs() {
    let answers = |t: &mut World, foundation| {
        let mut answer = None;
        t.run_unit_ai((1, 0), |_, _, side, _, _| {
            answer = Some(side.builds(foundation)?);
            Ok(Vec::new())
        })
        .unwrap();
        answer.unwrap()
    };
    let mut t = sent_off_world(vec![]);
    assert!(answers(&mut t, 10028));
    assert!(!answers(&mut t, 99_999));
    // the build command on top: a Build answers 1 without a task in the lists
    assert_eq!(t.build_command(1, 0, 10028), Ok(true));
    for id in [83, 118, 212] {
        let m = t.masters[1].get_mut(id).unwrap();
        m.tasks.retain(|k| k.action_type != 101);
    }
    assert!(answers(&mut t, 10028));
    let mut t = sent_off_world(vec![]);
    for id in [118, 212] {
        let m = t.masters[1].get_mut(id).unwrap();
        m.tasks.retain(|k| k.action_type != 101);
    }
    assert!(!answers(&mut t, 10028));
}

/// The build command at once from the unit AI's run (`UnitSide::build_now`: a requeued build
/// order's, which a following attack command replaces) does what the build command after the
/// update does: the switch to the builder (118), the Build on the foundation, which it takes.
#[test]
fn the_build_command_at_once_matches_the_one_after_the_update() {
    let mut later = sent_off_world(vec![]);
    let mut now = later.clone();
    let _ = later.build_command(1, 0, 10028).unwrap();
    now.run_unit_ai((1, 0), |_, _, side, _, _| {
        side.build_now(10028)?;
        Ok(Vec::new())
    })
    .unwrap();
    let (a, b) = (&later.players[1].objects[0], &now.players[1].objects[0]);
    assert_eq!(b.life.master_id, 118);
    assert!(b.actions.build().is_some_and(|build| build.target == 10028));
    assert_eq!(
        (b.life.master_id, b.actions.build().copied(), &b.sprite),
        (a.life.master_id, a.actions.build().copied(), &a.sprite)
    );
    assert_eq!(now.players[1].attributes, later.players[1].attributes);
    assert_eq!(now.streams, later.streams);
    let taken = |t: &World| t.object(10028).map(|(_, f)| f.clone());
    assert_eq!(taken(&now), taken(&later));
}

/// `FUN_141205ac0`, the way back after a drop-off: the order on a target `0x325` in front of the
/// unit AI's queue, on the town-bell record's object the world holds (its master the target
/// type, its owner the target's, its place the point), else on the record's target as recorded
/// (owner −1, the record's point); issued by the unit's owner, priority 100, ready.
#[test]
fn the_way_back_after_a_drop_off_queues_the_order_on_the_recorded_work() {
    use crate::orders::town_bell::BackToWork;
    let mut t = sent_off_world(vec![]);
    let work = BackToWork {
        target: 10028,
        target_type: 3,
        action: Some(ActionKind::Build),
        target2: -1,
        point: [1.0, 2.0],
    };
    t.players[1].objects[0].town_bell.work = work;
    t.queue_way_back(1, 0).unwrap();
    let ai = t.players[1].objects[0].ai.as_ref().unwrap();
    assert_eq!(
        ai.orders,
        [crate::unit_ai::QueuedOrder {
            issuer: 1,
            order: OrderKind::OnTarget2,
            priority: 100,
            target: 10028,
            target_owner: 1,
            point: [30.0, 88.0, 0.0],
            range: 0.0,
            shown: [30.0, 88.0],
            ready: true,
            target_type: 70,
        }]
    );
    // a recorded target the world no longer holds: the record as it is, in front
    t.players[1].objects[0].town_bell.work = BackToWork {
        target: 999,
        ..work
    };
    t.queue_way_back(1, 0).unwrap();
    let first = t.players[1].objects[0].ai.as_ref().unwrap().orders[0];
    assert_eq!(
        (
            first.target,
            first.target_type,
            first.target_owner,
            first.point
        ),
        (999, 3, -1, [1.0, 2.0, 0.0])
    );
}

/// `UNIT_ORDER`'s drop-off (action 8, `FUN_14161e370`) runs on any player's units its command
/// names: a carrying villager without a gather action, of a master without drop sites and a
/// class the drop-site table does not list, does nothing, whoever commands it.
#[test]
fn a_unit_order_drop_off_takes_any_player_s_unit() {
    let mut t = sent_off_world(vec![]);
    let v = &mut t.players[1].objects[0];
    (v.life.attribute_type, v.life.amount) = (1, 5.0);
    let before = t.players[1].objects[0].clone();
    t.unit_order(player_id(1), &object_ids([9175]), 8).unwrap();
    t.unit_order(player_id(0), &object_ids([9175]), 8).unwrap();
    assert_eq!(t.players[1].objects[0], before);
}

/// The unit AI's move at once (`UnitSide::ai_move_now`, ahead of a stop and an attack command
/// in the same run) does what the move after the update does (slot 0x340: the actions go, a
/// `MoveTo` to the point).
#[test]
fn the_ai_move_at_once_matches_the_one_after_the_update() {
    let mut later = sent_off_world(vec![]);
    later.players[1].objects[0]
        .body
        .put_mover(Some(crate::gathering::hunt::tests::walker()));
    let mut now = later.clone();
    let m = crate::unit_ai::AiMove {
        point: [35.0, 80.0, 0.0],
        range: 0.0,
        force: true,
        walk: false,
    };
    later.move_aside(1, 0, m).unwrap();
    now.run_unit_ai((1, 0), |_, _, side, _, _| {
        side.ai_move_now(m)?;
        Ok(Vec::new())
    })
    .unwrap();
    let (a, b) = (&later.players[1].objects[0], &now.players[1].objects[0]);
    assert_eq!(
        b.actions.move_to().copied().and_then(|mt| mt.goal),
        Some([35.0, 80.0, 0.0])
    );
    assert_eq!(
        (b.actions.move_to().copied(), &b.body, &b.sprite),
        (a.actions.move_to().copied(), &a.body, &a.sprite)
    );
    assert_eq!(now.streams, later.streams);
}

/// The hunt command at once from the unit AI's run (`UnitSide::hunt_now`: a requeued hunt
/// order's, which a following attack command replaces) does what `HuntObject`'s unit part after
/// the update does: the stopAction's standing sprite, the switch to the hunter (122) whose task
/// hunts deer (class 9), the Hunt on the deer, which it takes.
#[test]
fn the_hunt_command_at_once_matches_the_one_after_the_update() {
    let mut later = sent_off_world(vec![]);
    for masters in &mut later.masters {
        let hunter = Master {
            id: 122,
            tasks: vec![crate::objects::masters::Task {
                action_type: 110,
                object_group: 9,
                work_range: 0.5,
                ..crate::objects::masters::Task::default()
            }],
            ..masters.get(83).unwrap().clone()
        };
        let deer = Master {
            id: 65,
            kind: 70,
            class: 9,
            radius: [0.4, 0.4],
            ..Master::default()
        };
        *masters = Masters::new(masters.iter().cloned().chain([hunter, deer]));
    }
    later.players[0].objects.push(Unit {
        id: 500,
        body: crate::objects::layers::Body::at([32.5, 89.5, 0.0]),
        life: Lifecycle {
            state: 2,
            hp: 5.0,
            master_id: 65,
            ..Lifecycle::default()
        },
        ..Unit::default()
    });
    let placed = |x: f32, class, player, master| Placed {
        position: [x, 89.5, 0.0],
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
    later.spatial.insert(9175, placed(29.5, 4, 1, 83));
    later.spatial.insert(500, placed(32.5, 9, 0, 65));
    later.players[1].objects[0]
        .body
        .put_mover(Some(crate::gathering::hunt::tests::walker()));
    let mut now = later.clone();
    later.hunt_command(1, 0, 500).unwrap();
    let ai = now.players[1].objects[0].ai.clone().unwrap();
    now.run_unit_ai((1, 0), |_, _, side, _, _| {
        side.hunt_now(500, &ai)?;
        Ok(Vec::new())
    })
    .unwrap();
    let (a, b) = (&later.players[1].objects[0], &now.players[1].objects[0]);
    assert_eq!(b.life.master_id, 122);
    assert!(b.actions.hunt().is_some_and(|h| h.target == 500));
    assert_eq!(
        (
            b.life.master_id,
            b.actions.hunt().copied(),
            &b.sprite,
            &b.body
        ),
        (
            a.life.master_id,
            a.actions.hunt().copied(),
            &a.sprite,
            &a.body
        )
    );
    assert_eq!(now.players[1].attributes, later.players[1].attributes);
    assert_eq!(now.streams, later.streams);
    assert_eq!(now.players[0].objects, later.players[0].objects);
}

#[test]
fn a_build_creates_a_foundation_whose_unit_ai_draws_before_the_turn_players_units() {
    // MarvinBlue/426048731 tick 281: player 1's BUILD draws three AI values in the command
    // phase, so player 2's units, on their turn, get the draws after them (a computer's
    // BUILD here: it needs no builder that can build it)
    let world = |objects: Vec<Unit>| {
        let mut t = World::new(
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
                    objects: objects.into(),
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
        &[Command::Build(BuildCommand {
            player: player_id(0),
            master: master_id(70),
            position: Point::new(10.0, 10.0),
            q_id: -1,
            sprite_override: -1,
            builders: unit_list(vec![]),
            queued: false,
            commits: false,
            frame: 0,
        })],
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
    let mut t = World::new(
        0,
        29910,
        1,
        vec![0, 0],
        vec![
            Player {
                objects: vec![ordered(7), ordered(8)].into(),
                ..Player::default()
            },
            Player {
                objects: vec![live(2)].into(),
                ..Player::default()
            },
        ],
    );
    t.tick_with(
        40,
        &[Command::Order(OrderCommand {
            player: player_id(0),
            units: unit_list(object_ids([8, 99, 7])),
            order: Order::Retreat(crate::orders::Interact::default()),
        })],
    );
    let mut streams = Streams::default();
    streams.seed_all(29910);
    let ai = streams.get(Stream::Ai);
    let (eight, seven) = (ai.rand15() % 1500 + 2250, ai.rand15() % 1500 + 2250);
    let next = 750 + ai.rand15() % 500;
    let timeout = |t: &World, i: usize| {
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
        let mut t = World::new(0, 1, 0, vec![0], vec![player(vec![])]);
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
        class: crate::gathering::gather::FARM_CLASS,
        work_rate: 0.4,
        capacity: 15,
        default_task: Some(0),
        tasks: vec![crate::objects::masters::Task {
            action_type: crate::gathering::make::MAKE,
            attribute_type: crate::gathering::make::FARM_FOOD_KIND,
            attribute_type2: 0,
            work_val1: 1.0,
            ..crate::objects::masters::Task::default()
        }],
        ..Master::default()
    }
}

/// Livestock whose action list is empty in state 2 (out of a building, its Artifact gone)
/// starts its default task's Artifact again (slot 0x7f0), Gaia's waiting half as long; its first
/// update comes in the next tick, as a Fly's.
#[test]
fn livestock_without_actions_starts_its_artifact_again() {
    let sheep = Unit {
        id: 40,
        life: Lifecycle {
            master_id: 594,
            state: 2,
            hp: 7.0,
            ..Lifecycle::default()
        },
        default_artifact: true,
        ..Unit::default()
    };
    let mut t = World::new(
        0,
        1,
        0,
        vec![0, 0],
        vec![
            Player::default(),
            Player {
                objects: vec![sheep].into(),
                attributes: vec![0.0; 100],
                ..Player::default()
            },
        ],
    );
    t.masters = vec![
        Masters::default(),
        Masters::new([Master {
            id: 594,
            kind: 70,
            class: 58,
            ..Master::default()
        }]),
    ];
    t.tick(50);
    let a = t.players[1].objects[0]
        .actions
        .artifact()
        .copied()
        .expect("the Artifact again");
    assert_eq!(a, Artifact::new(false, None));
    // its first update checks for a new owner, finds none and waits a second
    t.tick(50);
    let a = t.players[1].objects[0].actions.artifact().copied().unwrap();
    assert_eq!((a.state, a.timer), (10, 1.0));
}

/// [`forage_world`]'s villager 9175 (master 120) with `set` run on its master and on itself,
/// a master 99 for the remains, then `part` on it in an update: what the part left in the
/// Step, the attribute changes and the remains.
fn life_part(
    set: impl FnOnce(&mut Master, &mut Unit),
    part: impl FnOnce(&mut Unit, &mut step::Step<'_>) -> Result<(), GenieError>,
) -> Result<LifeLeft, GenieError> {
    let mut t = forage_world();
    let mut master = t.masters[1].get(120).unwrap().clone();
    set(&mut master, &mut t.players[1].objects[0]);
    // the remains and death spawn the tests name (99, a master the game does not have)
    let left = Master {
        id: 99,
        kind: 20,
        ..Master::default()
    };
    let ms = &mut t.masters[1];
    *ms = Masters::new(
        ms.iter()
            .filter(|m| m.id != 120)
            .cloned()
            .chain([master, left]),
    );
    in_update(&mut t, (1, 0), 50, |unit, c| {
        let before = attribute_values(c.w, 1);
        let ids: Vec<i32> = c.w.players[1].objects.iter().map(|u| u.id).collect();
        part(unit, c)?;
        let own = c.w.players[1].objects.iter();
        let made = own
            .filter(|u| !ids.contains(&u.id))
            .map(|u| (u.life.master_id, u.position()));
        Ok((gained(c.w, 1, &before), made.collect()))
    })
}

/// Player `p`'s food, wood, stone and gold, then its attributes from 4 on: every attribute's
/// value in attribute order.
fn attribute_values(t: &World, p: usize) -> Vec<f32> {
    let owner = &t.players[p];
    owner
        .resources
        .iter()
        .chain(owner.attributes.iter().skip(4))
        .copied()
        .collect()
}

/// What player `p`'s attributes changed by since `before` ([`attribute_values`]): each changed
/// attribute and its change, in attribute order.
fn gained(t: &World, p: usize, before: &[f32]) -> Vec<(i16, f32)> {
    let now = attribute_values(t, p);
    let changed = now
        .iter()
        .zip(before)
        .enumerate()
        .filter(|(_, (a, b))| a != b);
    changed.map(|(i, (a, b))| (i as i16, a - b)).collect()
}

/// What [`life_part`] gives within the update: the owner's attribute changes and the objects
/// it made (master, place).
type LifeLeft = (Vec<(i16, f32)>, Vec<(i16, [f32; 3])>);

/// The life cycle's `set_state(3)` storage (`FUN_1411a56c0` with 0): a live object below one
/// hit point gives back each storage entry of kind 2 or 16 with an attribute (the amount
/// negated); one at a hit point, or at a NaN one, gives back nothing.
#[test]
fn a_dying_object_gives_back_its_storage_of_kinds_2_and_16() {
    let storage = |hp: f32| {
        life_part(
            |m, v| {
                m.storage = vec![(4, 1.0, 2), (11, 5.0, 16), (9, 1.0, 1), (-1, 1.0, 2)];
                (v.life.state, v.life.hp) = (2, hp);
            },
            |u, c| u.dying_storage_leaves(c),
        )
        .unwrap()
        .0
    };
    assert_eq!(storage(0.5), [(4, -1.0), (11, -5.0)]);
    assert_eq!(storage(1.0), []);
    assert_eq!(storage(f32::NAN), []);
}

/// The dying sprite's end (`RGE_Animated_Object::update`, [`Unit::dying_sprite_ended`]): the
/// animation stops and a unit (type below 80) leaves its remains (here 99) at its place within
/// its update (the dead unit lent to the world); without remains (−1) or as a building none. A
/// dead object's death spawn ([`Unit::death_spawn_comes`]) comes the same way. A master the
/// player lacks fails.
#[test]
fn a_dead_object_leaves_its_remains_and_its_death_spawn() {
    let ended = |kind: u8, remains: i16| {
        life_part(
            |m, v| {
                m.kind = kind;
                v.life.remains = remains;
            },
            Unit::dying_sprite_ended,
        )
    };
    let v = forage_world().players[1].objects[0].clone();
    assert_eq!(ended(70, 99).unwrap().1, [(99, v.position())]);
    assert_eq!(ended(70, -1).unwrap().1, []);
    assert_eq!(ended(80, 99).unwrap().1, []);
    let spawned = life_part(|m, _| m.death_spawns = [99, -1], Unit::death_spawn_comes);
    assert_eq!(spawned.unwrap().1, [(99, v.position())]);
    let lacking = life_part(|_, v| v.life.master_id = 999, Unit::dying_sprite_ended);
    assert_eq!(
        lacking.unwrap_err().reason(),
        "a dead object of a master the slice does not have"
    );
}

/// The Artifact's handover to Gaia (state 100, [`Unit::artifact_handover`]): player 1's sheep
/// stops and changes hands to Gaia (slot 0x70), and its Artifact checks again (state 6). The
/// world's first update runs no Artifact (`0x140aad250` from the second on).
#[test]
fn an_artifact_in_state_100_hands_its_sheep_to_gaia() {
    let master = Master {
        id: 594,
        kind: 70,
        class: 58,
        ..Master::default()
    };
    let sheep = Unit {
        id: 40,
        life: Lifecycle {
            master_id: 594,
            state: 2,
            hp: 7.0,
            ..Lifecycle::default()
        },
        actions: ActionList::default().with_artifact(Some(Artifact {
            state: crate::ownership::artifact::TO_GAIA,
            ..Artifact::new(false, None)
        })),
        ..Unit::default()
    };
    let player = |objects| Player {
        objects,
        attributes: vec![0.0; 250],
        ..Player::default()
    };
    let mut t = World::new(
        0,
        1,
        0,
        vec![0, 0],
        vec![player(vec![].into()), player(vec![sheep].into())],
    );
    t.masters = vec![Masters::new([master.clone()]), Masters::new([master])];
    placeable(&mut t);
    let placed = Placed {
        position: [10.5, 10.5, 0.0],
        radius: [0.2, 0.2],
        class: 58,
        player: 1,
        master: 594,
        obstruction_type: 0,
        buildable_on: 0,
        tall: true,
        speed: 0.0,
        unknown: None,
        obstruction: None,
    };
    t.spatial.insert(40, placed);
    t.tick(50);
    assert_eq!(t.players[1].objects.len(), 1);
    t.tick(50);
    assert_eq!(
        (
            t.players[0].unsupported.clone(),
            t.players[1].unsupported.clone()
        ),
        (None, None)
    );
    assert_eq!(t.players[1].objects.len(), 0);
    let sheep = &t.players[0].objects[0];
    assert_eq!(sheep.id, 40);
    assert_eq!(
        sheep.actions.artifact().map(|a| a.state),
        Some(crate::ownership::artifact::CHECK)
    );
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
    attributes[crate::gathering::make::FARM_FOOD] = 175.0;
    let mut t = World::new(
        0,
        1,
        0,
        vec![0],
        vec![Player {
            objects: vec![farm].into(),
            attributes,
            ..Player::default()
        }],
    );
    t.masters = vec![Masters::new([farm_master()])];
    t.tick(104);
    let f = &t.players[0].objects[0];
    let make = f.actions.make().copied().unwrap();
    assert_eq!((make.state, make.remaining, make.total), (6, 175.0, 175.0));
    assert_eq!((f.life.attribute_type, f.life.amount), (16, 0.0));
    assert_eq!(
        f.top_action(),
        Some(TopAction {
            kind: crate::gathering::make::MAKE,
            state: 6
        })
    );
    t.tick(104);
    let f = &t.players[0].objects[0];
    let made = 0.4f32 * 1.0 * seconds(104);
    assert_eq!(
        (f.life.amount, f.actions.make().copied().unwrap().remaining),
        (made, 175.0 - made)
    );
    assert!(t.players[0].unsupported.is_none());
}

#[test]
fn a_built_farm_s_master_starts_a_make_and_another_default_task_is_reported() {
    assert_eq!(farm_master().default_start(), (true, None));
    let mut other = farm_master();
    other.tasks[0].action_type = crate::movement::fly::FLY;
    assert!(other.default_start().1.is_some());
    // a type the factories make nothing of starts nothing (a wild animal's Explore, 6)
    other.tasks[0].action_type = 6;
    assert_eq!(other.default_start(), (false, None));
    other.default_task = None;
    assert_eq!(other.default_start(), (false, None));
}

/// `TRIBE_Building_Object::update`'s whole seconds (`0x1411c95a2`): a built building outside
/// the walls' branch (a barracks) has its `damaged_lately_timer` (`+0x30d`) lose one in an
/// update that passes whole seconds, however many, and stay at 0.
#[test]
fn a_building_s_damaged_lately_timer_loses_one_an_update_with_whole_seconds() {
    let barracks = Unit {
        id: 40,
        life: Lifecycle {
            master_id: 12,
            hp: 550.0,
            state: 2,
            ..Lifecycle::default()
        },
        damaged_lately: 2,
        ..Unit::default()
    };
    let mut t = World::new(
        0,
        1,
        0,
        vec![0],
        vec![Player {
            objects: vec![barracks].into(),
            attributes: vec![0.0; 300],
            ..Player::default()
        }],
    );
    t.masters = vec![Masters::new([Master {
        id: 12,
        kind: 80,
        class: 3,
        hit_points: 550,
        building: Some(crate::objects::masters::BuildingMaster::default()),
        ..Master::default()
    }])];
    let timer = |t: &World| t.players[0].objects[0].damaged_lately;
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

fn queue(building: i32, unit: i16, amount: i16) -> Command {
    Command::Queue(QueueCommand {
        player: player_id(0),
        buildings: unit_list(vec![Some(object_id(building))]),
        building_master: master_id(109),
        unit: master_id(unit),
        amount,
        button: -1,
    })
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
    v.heal(master.class, master.hit_points, 0.1).unwrap();
    assert_eq!(v.life.hp, 11.1);
    // a heal past the hit points stops at them, the powerup's room taking the rest
    v.life.hp = 24.95;
    v.hp_powerups = [1.0, 2.0];
    v.heal(master.class, master.hit_points, 0.1).unwrap();
    assert_eq!((v.life.hp, v.hp_powerups), (24.95f32 + 0.1, [1.0, 2.0]));
    // past them with the room: the overflow comes off the powerup, then a full unit holds
    // exactly its master's hit points
    v.life.hp = 26.95;
    v.heal(master.class, master.hit_points, 0.1).unwrap();
    assert_eq!(v.life.hp, 25.0);
    assert_eq!(v.hp_powerups[0], 1.0 - (26.95f32 + 0.1 - 27.0));
    // a paid class heals at a cost: not modelled
    let ship = Master {
        class: 22,
        ..master
    };
    assert!(v.heal(ship.class, ship.hit_points, 0.1).is_err());
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
        objects: vec![unit(1)].into(),
        sleeping: vec![unit(2)].into(),
        ..Player::default()
    };
    let mut t = World::new(0, 0, 0, vec![0, 0], vec![Player::default(), player]);
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
        objects: objects.into(),
        relations: vec![0, 0],
        attributes: vec![0.0; 250],
        ..Player::default()
    };
    let mut t = World::new(
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
        building: Some(crate::objects::masters::BuildingMaster {
            transform_unit,
            ..crate::objects::masters::BuildingMaster::default()
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
        objects: objects.into(),
        relations,
        attributes: vec![0.0; 250],
        ..Player::default()
    };
    let mut t = World::new(
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
        building: Some(crate::objects::masters::BuildingMaster {
            transform_unit: 63,
            ..crate::objects::masters::BuildingMaster::default()
        }),
        ..Master::default()
    };
    t.masters = (0..4).map(|_| Masters::new([master.clone()])).collect();
    t.spatial = Spatial::with_tiles(
        [20, 20],
        crate::map::spatial::Tiles::new(20, 20, vec![1; 400]),
    );
    t.spatial.relations = t.players.iter().map(|p| p.relations.clone()).collect();
    let record = crate::map::spatial::Obstruction { kind: 2, class: 5 };
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
    let blocked = |t: &World| {
        t.spatial
            .tiles()
            .word(9, 10)
            .is_some_and(|w| w & crate::map::spatial::BLOCKED != 0)
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
    let mut s = Spatial::with_tiles(
        [40, 40],
        crate::map::spatial::Tiles::new(40, 40, vec![1; 1600]),
    );
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
        obstruction: Some(crate::map::spatial::Obstruction { kind, class: 2 }),
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

/// records/425906646's gate 27745 of player 1 (annexes 27746 and 27747, master 88: open, no
/// height, its `transform_unit` the shut 91 of height 2; 92 transforms into nothing) at (20, 20)
/// with the moving soldier 1 (player 1's object 3) between its posts, at world time 400.
fn gate_world() -> World {
    let gate_master = |id, transform_unit, height| Master {
        id,
        kind: 80,
        class: 39,
        height,
        hit_points: 1650,
        // a gate moves (its master's slot 0xe8): its last move time is the mover's
        moving: Some(crate::gathering::hunt::tests::walker().stats),
        building: Some(crate::objects::masters::BuildingMaster {
            transform_unit,
            stack_unit: -1,
            head_unit: -1,
            foundation_terrain: -1,
            tech: -1,
            annexes: [-1; 4],
            ..crate::objects::masters::BuildingMaster::default()
        }),
        ..Master::default()
    };
    let soldier = Master {
        id: 4,
        kind: 70,
        class: 6,
        ..Master::default()
    };
    let unit = |id, master, x: f32, annex_of| Unit {
        id,
        body: crate::objects::layers::Body::at([x, 20.0, 0.0]),
        annex_of,
        life: Lifecycle {
            master_id: master,
            state: 2,
            hp: 10.0,
            ..Lifecycle::default()
        },
        pasture: -1,
        my_pasture: -1,
        ..Unit::default()
    };
    let mut gate = unit(27745, 88, 20.0, None);
    gate.gate = Some(Gate::default());
    gate.body
        .put_mover(Some(crate::gathering::hunt::tests::walker()));
    let mut t = World::new(
        0,
        1,
        0,
        vec![0, 0],
        vec![
            Player::default(),
            Player {
                objects: vec![
                    gate,
                    unit(27746, 88, 18.5, Some(27745)),
                    unit(27747, 88, 21.5, Some(27745)),
                    unit(1, 4, 19.6, None),
                ]
                .into(),
                ..Player::default()
            },
        ],
    );
    t.world_time = 400;
    t.masters = vec![
        Masters::default(),
        Masters::new([
            gate_master(88, 91, 0.0),
            gate_master(91, 88, 2.0),
            gate_master(92, -1, 0.0),
            soldier,
        ]),
    ];
    t.spatial = Spatial::with_tiles(
        [40, 40],
        crate::map::spatial::Tiles::new(40, 40, vec![1; 1600]),
    );
    t.spatial.relations = vec![vec![0, 3], vec![3, 0]];
    let placed = |x: f32, class, speed, kind| Placed {
        position: [x, 20.0, 0.0],
        radius: [0.2, 0.2],
        class,
        player: 1,
        master: 0,
        obstruction_type: kind,
        buildable_on: 0,
        tall: true,
        speed,
        unknown: None,
        obstruction: Some(crate::map::spatial::Obstruction { kind, class: 2 }),
    };
    t.spatial.insert(27745, placed(20.0, 39, 0.0, 2));
    t.spatial.insert(27746, placed(18.5, 39, 0.0, 2));
    t.spatial.insert(27747, placed(21.5, 39, 0.0, 2));
    t.spatial.insert(1, placed(19.6, 6, 0.8, 5));
    t
}

/// The player's gate toggle (`FUN_14162f8c0`) on records/425906646's gate 27745 (annexes 27746
/// and 27747): the open gate (master 88) becomes its `transform_unit`, the shut 91 (a height),
/// which kills the moving soldier between its posts and takes the world time plus 1 as its last
/// move time; the shut one opens again (no height) without either. A gate without a
/// `transform_unit` stays; one with masters of its own is not modelled.
#[test]
fn a_gate_toggle_shuts_on_the_units_between_its_posts_or_opens() {
    let mut t = gate_world();
    let state = |t: &World| {
        let g = &t.players[1].objects[0];
        (
            g.life.master_id,
            g.body.mover().map(|m| m.last_move),
            t.players[1].objects[3].life.hp,
        )
    };
    t.toggle_gate(1, 0).unwrap();
    assert_eq!(state(&t), (91, Some(401), 0.0));
    t.players[1].objects[3].life.hp = 10.0;
    t.world_time = 500;
    t.toggle_gate(1, 0).unwrap();
    assert_eq!(state(&t), (88, Some(401), 10.0));
    t.players[1].objects[0].life.master_id = 92;
    t.toggle_gate(1, 0).unwrap();
    assert_eq!(t.players[1].objects[0].life.master_id, 92);
    t.players[1].objects[0].gate = Some(Gate {
        other_masters: true,
        ..Gate::default()
    });
    assert!(t.toggle_gate(1, 0).is_err());
}

/// [`Unit::gate_update`] of [`gate_world`]'s gate as master `master` with its gate state `gate`:
/// the gate's master within the update after it (its toggle's, [`World::toggle_gate`]), its
/// state after, or the error.
fn gate_update_of(
    master: i16,
    gate: Option<Gate>,
    setup: impl FnOnce(&mut World),
) -> Result<(i16, Option<Gate>), GenieError> {
    let mut t = gate_world();
    let g = &mut t.players[1].objects[0];
    (g.life.master_id, g.gate) = (master, gate);
    setup(&mut t);
    let master = in_update(&mut t, (1, 0), 50, |unit, c| {
        unit.gate_update(c)?;
        Ok(unit.life.master_id)
    })?;
    Ok((master, t.players[1].objects[0].gate))
}

/// The gate's update (`FUN_1411e7730`): a shut (height 2, master 91), unlocked gate with an ally
/// moving between its posts opens within its update (its player's toggle: master 88), its close
/// timer 0; a locked one stays shut. An open gate (no height) with nothing in it starts its close
/// timer at 0.5 s, runs it down by the world's dt (0.05 s), and at 0 shuts within its update.
/// Its first update clears every time it runs; a gate whose master transforms into nothing is
/// passed over, and one without its gate state is not modelled.
#[test]
fn a_gate_opens_for_an_ally_inside_and_shuts_after_its_close_timer() {
    let gate = |locked, close_timer| {
        Some(Gate {
            locked,
            close_timer,
            first_update: true,
            ..Gate::default()
        })
    };
    let ran = |locked, close_timer| {
        Some(Gate {
            first_update: false,
            ..gate(locked, close_timer).unwrap()
        })
    };
    assert_eq!(
        gate_update_of(91, gate(false, 0.3), |_| ()),
        Ok((88, ran(false, 0.0)))
    );
    assert_eq!(
        gate_update_of(91, gate(true, 0.3), |_| ()),
        Ok((91, ran(true, 0.3)))
    );
    // the soldier stands aside: nothing in the open gate
    let aside = |t: &mut World| {
        let p = *t.spatial.get(1).unwrap();
        t.spatial.insert(
            1,
            Placed {
                position: [20.0, 30.0, 0.0],
                ..p
            },
        );
    };
    assert_eq!(
        gate_update_of(88, gate(false, 0.0), aside),
        Ok((88, ran(false, GATE_CLOSE_DELAY)))
    );
    assert_eq!(
        gate_update_of(88, gate(false, 0.5), aside),
        Ok((88, ran(false, 0.5 - 0.05)))
    );
    assert_eq!(
        gate_update_of(88, gate(false, 0.05), aside),
        Ok((91, ran(false, 0.0)))
    );
    // the soldier in the open gate holds its timer
    assert_eq!(
        gate_update_of(88, gate(false, 0.5), |_| ()),
        Ok((88, ran(false, 0.5)))
    );
    assert_eq!(
        gate_update_of(92, gate(false, 0.5), |_| ()),
        Ok((92, gate(false, 0.5)))
    );
    let err = gate_update_of(91, None, |_| ()).unwrap_err();
    assert_eq!(err.reason(), "a gate without its gate state");
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
/// object whose update returned 1 ([`World::leave_world`]): its owner's watched list lets it go
/// (`FUN_141091310`) and a recyclable master's missile goes to the pool of type 60.
#[test]
fn the_forced_replay_s_removal_leaves_as_an_update_s_leave_does() {
    let missile = Unit {
        id: 9,
        stopped: true,
        missile: Some(crate::combat::missile::Missile {
            max_range: 0.0,
            fired_from: -1,
            charge: false,
            flying: true,
            step: [0.0; 3],
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
    t.players[0].watched.push(crate::vision::ghost::Watched {
        id: 9,
        cell: crate::vision::ghost::Cell::Tile(0),
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
fn one_player(objects: Vec<Unit>, resources: [f32; 4]) -> World {
    let mut attributes = vec![0.0; 250];
    (attributes[4], attributes[11], attributes[32]) = (5.0, 4.0, 10.0);
    let mut t = World::new(
        0,
        29910,
        0,
        vec![0],
        vec![Player {
            objects: objects.into(),
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
        trained_at: vec![109],
        train_locations: vec![crate::objects::masters::TrainLocation {
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
    let several = |amount| {
        Command::Queue(QueueCommand {
            player: player_id(0),
            buildings: unit_list(object_ids([9, 10])),
            building_master: master_id(109),
            unit: master_id(99),
            amount,
            button: -1,
        })
    };
    let total = |t: &World, k: usize| {
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
    assert_eq!(
        t.production_load(0, &t.players[0].objects[0]),
        25.0 - make.progress
    );
    assert_eq!(t.production_load(0, &t.players[0].objects[1]), 0.0);
    // the empty one takes the first, then the loaded one the second (25 above its load)
    t.tick_with(104, &[several(2)]);
    assert_eq!((total(&t, 0), total(&t, 1)), (2, 1));
}

/// `FUN_141614450` gives a building room and load only where the unit trains (`FUN_141263610`,
/// [`World::trains_at`]): a building of another master keeps room −1 and takes none.
/// 427647190's players queued villagers at their trophy (master 1649, which trains none) and
/// the game queued nothing there.
#[test]
fn a_queue_at_a_building_that_does_not_train_the_unit_queues_nothing_there() {
    let trophy = |id| {
        let mut u = town_centre(id);
        u.life.master_id = 1649;
        u
    };
    let with_trophy = |objects| {
        let mut t = one_player(objects, [200.0; 4]);
        let master = Master {
            id: 1649,
            kind: 80,
            class: 3,
            ..Master::default()
        };
        t.masters = vec![Masters::new(t.masters[0].iter().cloned().chain([master]))];
        t
    };
    let total = |t: &World, k: usize| {
        t.players[0].objects[k]
            .production
            .as_ref()
            .map_or(0, |p| p.queue.total)
    };
    let both = Command::Queue(QueueCommand {
        player: player_id(0),
        buildings: unit_list(object_ids([9, 10])),
        building_master: master_id(109),
        unit: master_id(99),
        amount: 3,
        button: -1,
    });
    let mut t = with_trophy(vec![trophy(9), centre(10)]);
    t.tick_with(104, &[both]);
    assert!(t.players[0].unsupported.is_none());
    assert_eq!((total(&t, 0), total(&t, 1)), (0, 3));
    // the trophy alone: no room, nothing queued, nothing paid
    let mut alone = with_trophy(vec![trophy(9)]);
    alone.tick_with(104, &[queue(9, 83, 1)]);
    assert!(alone.players[0].unsupported.is_none());
    assert_eq!(alone.players[0].resources, [200.0; 4]);
    assert!(alone.players[0].objects[0].production.is_none());
}

/// `FUN_141614450` with players ordering each other's (`+0x1074`): a building of another
/// player takes part as its own, and its part (`FUN_1411cbc40`) queues and pays for its owner
/// (`+0x138`); the cost check before reads the ordering player's stock. Without the setting the
/// packet's check drops it.
#[test]
fn a_queue_at_another_player_s_building_queues_and_pays_for_its_owner() {
    let two = |cheats| {
        let mut t = one_player(vec![], [200.0; 4]);
        let mut other = t.players[0].clone();
        other.objects = vec![centre(10)].into();
        other.resources = [100.0, 0.0, 0.0, 0.0];
        t.players.push(other);
        t.player_time.push(0);
        t.masters.push(t.masters[0].clone());
        t.settings.cheats_enabled = cheats;
        t
    };
    let mut t = two(true);
    t.tick_with(104, &[queue(10, 83, 1)]);
    assert!(t.players.iter().all(|p| p.unsupported.is_none()));
    assert_eq!(t.players[0].resources, [200.0; 4]);
    assert_eq!(t.players[1].resources[0], 54.0);
    let production = t.players[1].objects[0].production.as_ref().unwrap();
    assert_eq!(production.queue.total, 1);
    let mut t = two(false);
    t.tick_with(104, &[queue(10, 83, 1)]);
    assert_eq!(t.players[1].resources[0], 100.0);
    assert!(t.players[1].objects[0].production.is_none());
}

/// `FUN_141614450` reads a listed object live or sleeping (`FUN_140e6b750`): a sleeping one the
/// unit does not train at keeps room −1, and the town centre beside it takes the unit.
#[test]
fn a_queue_passes_over_a_sleeping_object_the_unit_does_not_train_at() {
    let mut t = one_player(vec![centre(9)], [200.0; 4]);
    let mut asleep = town_centre(11);
    asleep.life.master_id = 99;
    t.players[0].sleeping.push(asleep);
    let both = Command::Queue(QueueCommand {
        player: player_id(0),
        buildings: unit_list(object_ids([11, 9])),
        building_master: master_id(109),
        unit: master_id(99),
        amount: 2,
        button: -1,
    });
    t.tick_with(104, &[both]);
    assert!(t.players[0].unsupported.is_none());
    let production = t.players[0].objects[0].production.as_ref().unwrap();
    assert_eq!(production.queue.total, 2);
    assert!(t.players[0].sleeping[0].production.is_none());
}

/// RESEARCH's building (`0x140e7e270`): with more than one listed, the least loaded of the
/// packet's and the listed ones the world holds; a tie keeps the one taken; one alone is the
/// packet's.
#[test]
fn a_research_at_several_buildings_takes_the_least_loaded() {
    let mut t = one_player(vec![centre(9), centre(10)], [200.0; 4]);
    t.tick_with(104, &[queue(9, 99, 1)]);
    let at = |player, building, count, buildings: Vec<i32>| ResearchCommand {
        player,
        building,
        count,
        buildings: object_ids(buildings),
        tech: 0,
        q_id: -1,
        shift: false,
    };
    let loaded = |r: ResearchCommand| t.least_loaded(&r, &r.buildings);
    assert_eq!(
        loaded(at(player_id(0), Some(object_id(9)), 3, vec![9, 77, 10])),
        10
    );
    assert_eq!(
        loaded(at(player_id(0), Some(object_id(10)), 2, vec![10, 9])),
        10
    );
    assert_eq!(
        loaded(at(player_id(0), Some(object_id(9)), 1, vec![9, 10])),
        9
    );
    assert_eq!(loaded(at(player_id(0), Some(object_id(77)), 1, vec![])), -1);
}

/// `do_command`'s case 0x65 (`FUN_140e6f470`), RESEARCH's check before its handler: listing
/// none is refused; the packet's building, when the world holds it, must be the player's own
/// (any while players may order each other's), and one listed alone must be held; of more
/// listed, another player's or one the world lacks becomes −1, and a human's left without one
/// is refused.
#[test]
fn a_research_s_check_refuses_another_player_s_buildings() {
    let mut t = one_player(vec![centre(9), centre(10)], [200.0; 4]);
    t.players.push(Player::default());
    let at = |player, building, count, buildings: Vec<i32>| ResearchCommand {
        player,
        building,
        count,
        buildings: object_ids(buildings),
        tech: 0,
        q_id: -1,
        shift: false,
    };
    // its own: listed alone, or with the others checked
    assert_eq!(
        t.research_check(&at(player_id(0), Some(object_id(9)), 1, vec![])),
        Some(vec![])
    );
    assert_eq!(
        t.research_check(&at(player_id(0), Some(object_id(9)), 3, vec![9, 77, 10])),
        Some(object_ids([9, -1, 10]))
    );
    assert_eq!(
        t.research_check(&at(player_id(0), Some(object_id(9)), 0, vec![])),
        None
    );
    assert_eq!(
        t.research_check(&at(player_id(0), Some(object_id(9)), -2, vec![])),
        None
    );
    // player 1's: another's building, one alone the world lacks, a list of none of its own
    assert_eq!(
        t.research_check(&at(player_id(1), Some(object_id(9)), 1, vec![])),
        None
    );
    assert_eq!(
        t.research_check(&at(player_id(1), Some(object_id(9)), 2, vec![9, 10])),
        None
    );
    assert_eq!(
        t.research_check(&at(player_id(1), Some(object_id(77)), 1, vec![])),
        None
    );
    assert_eq!(
        t.research_check(&at(player_id(1), Some(object_id(77)), 2, vec![9, 10])),
        None
    );
    // a building the world lacks with a list skips the packet's own test
    assert_eq!(
        t.research_check(&at(player_id(0), Some(object_id(77)), 2, vec![77, 10])),
        Some(object_ids([-1, 10]))
    );
    // a computer's list left without one still runs
    t.players[1].computer = true;
    assert_eq!(
        t.research_check(&at(player_id(1), Some(object_id(77)), 2, vec![9, 10])),
        Some(object_ids([-1, -1]))
    );
    // while players may order each other's
    t.settings.cheats_enabled = true;
    t.players[1].computer = false;
    assert_eq!(
        t.research_check(&at(player_id(1), Some(object_id(9)), 1, vec![])),
        Some(vec![])
    );
}

/// RESEARCH (`0x140e7e270`) of Castle Age (0x66) at the town centre: queued as any technology;
/// only in game mode 16 does it mark the world's treaty monitor (`+0x820`'s `+0x17`), which the
/// slice does not hold.
#[test]
fn a_research_of_castle_age_queues_it_but_in_game_mode_16() {
    use crate::technology::{Location, TechDef, TechState};
    let castle = |mode| {
        let mut t = one_player(vec![centre(9)], [1000.0; 4]);
        t.settings.game_mode = mode;
        t.techs = vec![TechDef::default(); 0x67];
        let mut techs = vec![TechState::default(); 0x67];
        techs[0x66] = TechState {
            state: 1,
            research_cap: -1,
            building_master: -1,
            locations: vec![Location {
                building: 109,
                time: 160,
                button: 1,
                adjustment: 0.0,
            }],
            ..TechState::default()
        };
        t.players[0].techs = techs;
        let at = ResearchCommand {
            player: player_id(0),
            building: Some(object_id(9)),
            count: 1,
            buildings: vec![],
            tech: 0x66,
            q_id: -1,
            shift: false,
        };
        let r = research_listed(&mut t, &at);
        let queued = t.players[0].objects[0]
            .production
            .as_ref()
            .map_or(0, |p| p.queue.total);
        (r, queued)
    };
    let (r, queued) = castle(0);
    assert_eq!((r.is_ok(), queued), (true, 1));
    assert!(castle(16).0.is_err());
}

/// [`one_player`]'s town centre 9 with a unit AI and technology 5, available (state 1) at the
/// centre's master 109, its first cost 50 food.
fn research_world() -> World {
    use crate::technology::{Cost, Location, TechDef, TechState};
    let mut t = one_player(vec![centre(9)], [1000.0; 4]);
    t.players[0].objects[0].ai = Some(UnitAi::default());
    t.techs = vec![TechDef::default(); 6];
    t.techs[5].costs[0] = Cost {
        attribute: 0,
        amount: 50,
        paid: 1,
    };
    let mut techs = vec![TechState::default(); 6];
    techs[5] = TechState {
        state: 1,
        research_cap: -1,
        building_master: -1,
        overrides: [(0, -1, -2), (0, -1, 0), (0, -1, 0)],
        locations: vec![Location {
            building: 109,
            time: 30,
            button: 1,
            adjustment: 0.0,
        }],
        ..TechState::default()
    };
    t.players[0].techs = techs;
    t
}

/// RESEARCH of technology `tech` at player 0's building 9 with the target `target`.
fn research_at(tech: i16, target: i32) -> ResearchCommand {
    ResearchCommand {
        player: player_id(0),
        building: Some(object_id(9)),
        count: 1,
        buildings: vec![],
        tech,
        q_id: target,
        shift: false,
    }
}

/// RESEARCH (`0x140e7e270`): its listed costs come off the pending debit, and an alive building
/// at one of the technology's locations queues it, its unit AI on the action value 0x26c; a
/// target, a technology the tree lacks, one neither available nor repeatable, a building
/// nowhere, a dead one or one at no location do nothing past the debit.
#[test]
fn a_research_queues_at_an_alive_location_after_its_costs_come_off_the_debit() {
    let queued = |t: &World| {
        t.players[0].objects[0]
            .production
            .as_ref()
            .map_or(0, |p| p.queue.total)
    };
    let mut t = research_world();
    research_listed(&mut t, &research_at(5, -1)).unwrap();
    assert_eq!(queued(&t), 1);
    assert_eq!(t.players[0].pending_debit, [-50.0, 0.0, 0.0, 0.0]);
    let ai = t.players[0].objects[0].ai.as_ref().unwrap();
    assert_eq!(ai.current_action, Some(ActionKind::Research));
    let untouched = research_world();
    for (tech, target) in [(5, 9), (6, -1), (-1, -1)] {
        let mut t = research_world();
        research_listed(&mut t, &research_at(tech, target)).unwrap();
        assert_eq!(t, untouched);
    }
    let refused = |change: &dyn Fn(&mut World)| {
        let mut t = research_world();
        change(&mut t);
        research_listed(&mut t, &research_at(5, -1)).unwrap();
        (queued(&t), t.players[0].pending_debit[0])
    };
    assert_eq!(refused(&|t| t.players[0].techs[5].state = 2), (0, -50.0));
    assert_eq!(refused(&|t| t.players[0].objects[0].id = 8), (0, -50.0));
    assert_eq!(
        refused(&|t| t.players[0].objects[0].life.state = 3),
        (0, -50.0)
    );
    assert_eq!(
        refused(&|t| t.players[0].techs[5].locations[0].building = 110),
        (0, -50.0)
    );
}

/// RESEARCH's unmodelled parts: a technology the data file lacks, the shared kind, a linked
/// technology, a building of another player, a computer player's research and a building with
/// an unmodelled action.
#[test]
fn a_research_s_unmodelled_parts_fail() {
    let fails = |change: &dyn Fn(&mut World)| {
        let mut t = research_world();
        change(&mut t);
        research_listed(&mut t, &research_at(5, -1))
            .unwrap_err()
            .reason()
    };
    assert_eq!(
        fails(&|t| t.techs.truncate(5)),
        "a technology the data file has not got"
    );
    assert_eq!(
        fails(&|t| t.techs[5].kind = crate::technology::SHARED_KIND),
        "RESEARCH of the shared kind (FUN_1411c3fd0 over the packet's buildings)"
    );
    assert_eq!(
        fails(&|t| {
            let _ = t.linked_techs.insert(5, crate::technology::Linked::Cycle);
        }),
        "RESEARCH of a linked technology (FUN_140f3a0f0, FUN_140f39e80)"
    );
    assert_eq!(
        fails(&|t| {
            let tc = t.players[0].objects.remove(0);
            t.players.push(Player {
                objects: vec![tc].into(),
                ..Player::default()
            });
            t.masters.push(t.masters[0].clone());
        }),
        "RESEARCH at another player's object"
    );
    assert_eq!(
        fails(&|t| t.players[0].computer = true),
        "a computer player's RESEARCH (FUN_1411d5520, player+0x7a8)"
    );
    assert_eq!(
        fails(&|t| {
            t.players[0].objects[0].unmodelled_action = Some(GenieError::unsupported("an action"));
        }),
        "RESEARCH at a building with an unmodelled action"
    );
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
    let queued = |t: &World| {
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
    let with = |amount, button| {
        Command::Queue(QueueCommand {
            player: player_id(0),
            buildings: unit_list(object_ids([9])),
            building_master: master_id(109),
            unit: master_id(99),
            amount,
            button,
        })
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
        &[Command::Unmodelled(UnmodelledCommand {
            player: player_id(0),
            packet: Vec::new(),
            why: why.clone(),
        })],
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
    let mut v = crate::vision::visibility::Visibility::new(
        40,
        40,
        (0..3)
            .map(|_| crate::vision::visibility::VisibleMap {
                counts: vec![-1; n],
                explored: 0,
            })
            .collect(),
        vec![0; n],
        Vec::new(),
        0,
        vec![false; 3],
    );
    let mut r = crate::vision::registry::Registry::new(3, 65);
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
        body: crate::objects::layers::Body::at([10.5, 10.5, 0.0]),
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

/// A unit of player `p` at (`x`, 10.5) for the owner search of [`hands_world`]'s object: a
/// combat-type seer in state 2 on its tile's list.
fn owner_search_unit(s: &mut Spatial, id: i32, p: usize, x: f32) {
    s.sight.as_mut().unwrap().add_seer(
        id,
        Seer {
            player: p,
            master: 4,
            los: 6.0,
            state: 2,
            explored: true,
            building: None,
            umv: 0,
            capture: capture_kind(p, 0, 4),
            kind: 70,
        },
    );
    s.insert(
        id,
        Placed {
            position: [x, 10.5, 0.0],
            radius: [0.2, 0.2],
            class: 0,
            player: p,
            master: 4,
            obstruction_type: 0,
            buildable_on: 0,
            tall: true,
            speed: 0.0,
            unknown: None,
            obstruction: None,
        },
    );
}

/// Players of attribute 97 `attribute_97` and game status `status`, as an Artifact's owner
/// search reads them.
fn artifact_players(attribute_97: &[f32], status: &[u8]) -> Vec<Player> {
    status
        .iter()
        .enumerate()
        .map(|(q, &game_status)| {
            let mut attributes = vec![0.0; 98];
            attributes[97] = attribute_97.get(q).copied().unwrap_or(0.0);
            Player {
                attributes,
                game_status,
                ..Player::default()
            }
        })
        .collect()
}

/// The Artifact action's owner search (`FUN_140aabc40`) passes over the units of a player that
/// is out (`+0x89` not 0) and over such a player as a taker: 428265662's Gaia sheep 27530 in
/// update 642 stays Gaia's next to the units of player 7, who resigned in update 428.
#[test]
fn an_owner_search_passes_over_a_player_out_of_the_game() {
    let unit = owner_search_unit;
    let mut s = hands_world(27530, 0, LIVESTOCK, 594, 2.0);
    unit(&mut s, 1, 1, 11.5);
    let none = [0.0; 3];
    assert_eq!(
        s.artifact_owner(27530, &artifact_players(&none, &[0, 0, 0])),
        Ok(Some(1))
    );
    assert_eq!(
        s.artifact_owner(27530, &artifact_players(&none, &[0, 2, 0])),
        Ok(None)
    );
    // of two players, the nearest takes it: player 2 once player 1 is out
    unit(&mut s, 2, 2, 12.5);
    assert_eq!(
        s.artifact_owner(27530, &artifact_players(&none, &[0, 0, 0])),
        Ok(Some(1))
    );
    assert_eq!(
        s.artifact_owner(27530, &artifact_players(&none, &[0, 2, 0])),
        Ok(Some(2))
    );
    assert_eq!(
        s.artifact_owner(27530, &artifact_players(&none, &[0, 2, 1])),
        Ok(None)
    );
}

/// The owner search's own owner and attribute 97 (`FUN_140aabc40`): a unit of the animal's own
/// owner keeps it, and so does that owner's attribute 97 above 0 for livestock; another
/// player's attribute 97 above 0 counts its units 100 more, so it takes the animal from its
/// owner. A unit of a class that never counts (a building's, 3) and a Gaia unit with `+0x924`
/// set do not count; an animal inside another object searches about its record there.
#[test]
fn an_owner_search_keeps_the_owner_s_animal_unless_attribute_97_takes_it() {
    let unit = owner_search_unit;
    let none = [0.0; 3];
    let mut s = hands_world(27530, 1, LIVESTOCK, 594, 2.0);
    unit(&mut s, 1, 1, 11.5);
    unit(&mut s, 2, 2, 11.5);
    assert_eq!(
        s.artifact_owner(27530, &artifact_players(&none, &[0, 0, 0])),
        Ok(None)
    );
    assert_eq!(
        s.artifact_owner(27530, &artifact_players(&[0.0, 0.0, 1.0], &[0, 0, 0])),
        Ok(Some(2))
    );
    assert_eq!(
        s.artifact_owner(27530, &artifact_players(&[0.0, 1.0, 1.0], &[0, 0, 0])),
        Ok(None)
    );
    let mut s = hands_world(27530, 1, LIVESTOCK, 594, 2.0);
    unit(&mut s, 0, 0, 11.5);
    unit(&mut s, 3, 2, 12.5);
    let building = Placed {
        class: 3,
        ..*s.get(3).unwrap()
    };
    s.insert(3, building);
    assert_eq!(
        s.artifact_owner(27530, &artifact_players(&none, &[0, 0, 0])),
        Ok(None)
    );
    let mut s = hands_world(27530, 0, LIVESTOCK, 594, 2.0);
    unit(&mut s, 1, 1, 11.5);
    s.go_inside(27530, [10.5, 10.5, 0.0]);
    assert_eq!(
        s.artifact_owner(27530, &artifact_players(&none, &[0, 0, 0])),
        Ok(Some(1))
    );
}

/// The owner search's unmodelled parts: an animal the world does not hold, a unit of a player
/// beyond the lists, and a stopped unit that counts and may have moved onto the tiles.
#[test]
fn an_owner_search_s_unmodelled_parts_fail() {
    let none = [0.0; 3];
    let reason = |s: &Spatial, me, status: &[u8]| {
        s.artifact_owner(me, &artifact_players(&none, status))
            .unwrap_err()
            .reason()
    };
    let s = hands_world(27530, 0, LIVESTOCK, 594, 2.0);
    assert_eq!(
        reason(&s, 99, &[0, 0, 0]),
        "an owner search of an object the world does not hold"
    );
    let mut s = hands_world(27530, 0, LIVESTOCK, 594, 2.0);
    owner_search_unit(&mut s, 1, 3, 11.5);
    assert_eq!(
        reason(&s, 27530, &[0, 0, 0]),
        "an owner search's unit of a player beyond the list"
    );
    let mut s = hands_world(27530, 0, LIVESTOCK, 594, 2.0);
    owner_search_unit(&mut s, 1, 1, 20.5);
    let stopped = Placed {
        unknown: Some(10.0),
        ..*s.get(1).unwrap()
    };
    s.insert(1, stopped);
    assert_eq!(
        reason(&s, 27530, &[0, 0, 0]),
        "an owner search near a stopped unit that counts"
    );
}

/// The owner search walks the tiles' lists (`+0x28`, `FUN_140aabc40`), as the game does: a unit
/// on a full tile (`0xfa` objects: `FUN_140f5f0a0` files no more) is not on its list and does not
/// count, one on the next tile does.
#[test]
fn an_owner_search_counts_only_the_units_on_the_tiles_lists() {
    let mut s = hands_world(27530, 0, LIVESTOCK, 594, 2.0);
    for id in 1000..1250 {
        s.insert(
            id,
            Placed {
                position: [11.5, 10.5, 0.0],
                radius: [0.2, 0.2],
                class: 0,
                player: 0,
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
    owner_search_unit(&mut s, 1, 1, 11.5);
    let none = [0.0; 3];
    assert_eq!(
        s.artifact_owner(27530, &artifact_players(&none, &[0, 0, 0])),
        Ok(None)
    );
    owner_search_unit(&mut s, 2, 1, 12.5);
    assert_eq!(
        s.artifact_owner(27530, &artifact_players(&none, &[0, 0, 0])),
        Ok(Some(1))
    );
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
        let mut t = World::new(
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
                    }]
                    .into(),
                    attributes: vec![0.0; 300],
                    ..Player::default()
                },
                Player {
                    objects: vec![live(500)].into(),
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
fn teardown_world() -> World {
    let walker = crate::gathering::hunt::tests::walker();
    let unit = |id, master| Unit {
        id,
        body: crate::objects::layers::Body::new([10.5, 10.5, 0.0], None, Some(walker.clone())),
        life: Lifecycle {
            master_id: master,
            ..Lifecycle::default()
        },
        ..Unit::default()
    };
    let tc = |id| Unit {
        id,
        body: crate::objects::layers::Body::at([20.0, 20.0, 0.0]),
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
        body: crate::objects::layers::Body::at([30.0, 30.0, 0.0]),
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
            record: crate::map::spatial::Obstruction { kind: 2, class: 2 },
            snow: false,
            sprite_override: false,
            started: true,
        }),
        ..Unit::default()
    };
    let mut t = World::new(
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
                ]
                .into(),
                relations: vec![0, 1, 2],
                ..Player::default()
            },
            Player {
                objects: vec![foundation].into(),
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
/// gives the bird its top speed as its animation speed, then the base teardown's `MoveTo`
/// stops it (`+0x7b8` 0 at once; without a `MoveTo` the top speed stays); the
/// Enter's lets its building go; a counted builder's Build leaves another player's
/// foundation's builders (`World::leave_builders`) and lets it go; a wait for a foundation
/// goes.
#[test]
fn clearing_the_actions_tears_the_artifact_fly_enter_and_build_down() {
    let mut t = teardown_world();
    t.players[1].objects[0].actions.set_artifact(Some(Artifact {
        target: 9149,
        goal: Some([20.0, 20.0, 0.0]),
        walk_to: Some((crate::movement::move_to::Goal::Target(9149), 1.0)),
        move_to: Some(MoveTo::Moving),
        ..Artifact::new(false, None)
    }));
    t.clear_actions(1, 0).unwrap();
    let sheep = &t.players[1].objects[0];
    assert_eq!(sheep.actions.artifact().copied(), None);
    assert_eq!(sheep.body.mover().unwrap().animated.speed, 0.0);
    assert_eq!(t.players[1].objects[3].workers, 0);
    let bird = &mut t.players[1].objects[1];
    bird.actions.set_fly(Some(Fly {
        state: 3,
        timer: 1.0,
        move_to: Some(MoveTo::Moving),
        goal: Some([12.0, 12.0, 0.0]),
        home: [10.0, 10.0],
        task: crate::objects::masters::Task::default(),
    }));
    bird.body.mover_mut().unwrap().stats.speed = 1.3;
    let flying = bird.actions.fly().copied();
    t.clear_actions(1, 1).unwrap();
    let bird = &mut t.players[1].objects[1];
    assert_eq!(
        (bird.actions.fly().copied(), bird.body.anim_speed()),
        (None, Some(0.0))
    );
    bird.actions
        .set_fly(flying.map(|f| Fly { move_to: None, ..f }));
    t.clear_actions(1, 1).unwrap();
    let bird = &t.players[1].objects[1];
    assert_eq!(
        (bird.actions.fly().copied(), bird.body.anim_speed()),
        (None, Some(1.3))
    );
    let villager = &mut t.players[1].objects[2];
    villager
        .actions
        .set_enter(Some(crate::garrison::EnterAction {
            target: 9150,
            state: 3,
            move_to: None,
            point: [20.0, 20.0, 0.0],
            range: 0.0,
            timer: 0.0,
            retry: true,
            task_class: 0,
            move_wait: crate::movement::move_to::MoveWait::default(),
            moving: None,
        }));
    // a walking villager: an Enter without a MoveTo does not stop it
    villager.body.set_anim_speed(1.0);
    t.clear_actions(1, 2).unwrap();
    let villager = &mut t.players[1].objects[2];
    assert_eq!(villager.actions.enter().copied(), None);
    assert_ne!(villager.body.mover().unwrap().animated.speed, 0.0);
    villager.actions.set_wait_solidify(Some(
        crate::construction::wait_solidify::WaitSolidify::new(
            crate::objects::masters::Task::default(),
            10028,
        ),
    ));
    t.clear_actions(1, 2).unwrap();
    let villager = &t.players[1].objects[2];
    assert_eq!(villager.actions.wait_solidify().copied(), None);
    assert_eq!(t.players[1].objects[4].workers, 0);
    t.players[1].objects[2].actions.set_build(Some(BuildAction {
        target: 10028,
        state: 3,
        move_to: None,
        goal: None,
        walk_target: -1,
        point: [-1.0; 3],
        task: BuildTask {
            range: 0.0,
            work_value: 0.0,
            work_rate: 1.0,
            resource: -1,
            id: -1,
            moving: None,
        },
        added: true,
        timer: 0.0,
        move_wait: crate::movement::move_to::MoveWait::default(),
        move_sprite: None,
        site: -1,
        drop_range: 0.0,
    }));
    t.clear_actions(1, 2).unwrap();
    let foundation = &t.players[2].objects[0];
    assert_eq!(foundation.workers, 0);
    assert_eq!(foundation.construction.map(|k| k.builders), Some(0));
}

/// `FUN_1411e63f0` then `FUN_1411e6580` at a built building's death: each storage entry of mode
/// 4 with an attribute and an amount above 0 leaves its owner (negated), then the population
/// the object holds itself (attribute 4: it holds nothing after); each entry of mode 16 (a flag
/// bit 0x10) leaves its own store when the store holds its attribute.
#[test]
fn a_built_building_s_storage_leaves_its_owner_and_its_store() {
    let master = Master {
        id: 109,
        kind: 80,
        storage: vec![
            (4, 5.0, 4),
            (-1, 5.0, 4),
            (7, 0.0, 4),
            (11, 2.0, 16),
            (12, 3.0, 0x30),
            (13, 1.0, 16),
        ],
        ..Master::default()
    };
    let mut u = Unit {
        id: 1,
        store: vec![(11, 6.0), (12, 4.0)],
        ..Unit::default()
    };
    u.life.attribute_type = 4;
    u.life.amount = 3.0;
    let adds = storage_leaves(&mut u, &master);
    assert_eq!(adds, [(4, -5.0), (4, -3.0)]);
    assert_eq!((u.life.attribute_type, u.life.amount), (-1, 0.0));
    assert_eq!(u.store, [(11, 4.0), (12, 1.0)]);
    // holding no population: the master's entries alone
    let adds = storage_leaves(&mut u, &master);
    assert_eq!(adds, [(4, -5.0)]);
    assert_eq!(u.store, [(11, 2.0), (12, -2.0)]);
}

/// A world for the deaths: Gaia holds the tree 50 (master 349, type 15, one worker) and the boar
/// 60 (master 48, type 70, one worker, a unit AI that counts the hunter 41 among its attackers);
/// player 1 holds the villagers 40 (gathering the tree), 41 (hunting the boar) and 42 (a counted
/// builder of its foundation 10028, one builder, one worker), each with hit points 0.
fn deaths_world() -> World {
    let dead = |id| Unit {
        id,
        body: crate::objects::layers::Body::at([10.5, 10.5, 0.0]),
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
    let chop = crate::gathering::hunt::tests::hunting(50).task;
    let tree = crate::gathering::gather::Resource {
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
    let rate = crate::gathering::gather::StopRate {
        work_rate: 1.0,
        capacity: 10.0,
        productivity: 1.0,
    };
    let gather = crate::gathering::gather::GatherAction::new(chop, &tree, false, rate).unwrap();
    let mut villagers = vec![dead(40), dead(41), dead(42)];
    villagers[0]
        .actions
        .set_gather(Some(crate::gathering::gather::GatherAction {
            state: 7,
            ..gather
        }));
    villagers[1]
        .actions
        .set_hunt(Some(crate::gathering::hunt::tests::hunting(60)));
    villagers[2].actions.set_build(Some(BuildAction {
        target: 10028,
        state: 7,
        move_to: None,
        goal: None,
        walk_target: -1,
        point: [-1.0; 3],
        task: BuildTask {
            range: 0.0,
            work_value: 1.0,
            work_rate: 1.0,
            resource: -1,
            id: -1,
            moving: None,
        },
        added: true,
        timer: 0.0,
        move_wait: crate::movement::move_to::MoveWait::default(),
        move_sprite: None,
        site: -1,
        drop_range: 0.0,
    }));
    let gaia = vec![
        Unit {
            id: 50,
            body: crate::objects::layers::Body::at([12.5, 10.5, 0.0]),
            workers: 1,
            life: Lifecycle {
                master_id: 349,
                ..Lifecycle::default()
            },
            ..Unit::default()
        },
        Unit {
            id: 60,
            body: crate::objects::layers::Body::at([14.5, 10.5, 0.0]),
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
        body: crate::objects::layers::Body::at([30.0, 30.0, 0.0]),
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
            record: crate::map::spatial::Obstruction { kind: 2, class: 2 },
            snow: false,
            sprite_override: false,
            started: true,
        }),
        ..Unit::default()
    };
    // player 1's turn: the boar's unit AI keeps its attackers
    let mut t = World::new(
        0,
        1,
        1,
        vec![0, 0],
        vec![
            Player {
                objects: gaia.into(),
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
        body: crate::objects::layers::Body::at([12.5, 10.5, 0.0]),
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
    assert_eq!(
        t.players[1].objects[2].actions.move_to().map(|m| m.target),
        Some(50)
    );
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
    let mover = crate::gathering::hunt::tests::walker();
    let walker = |id| Unit {
        id,
        body: crate::objects::layers::Body::new([11.5, 10.5, 0.0], None, Some(mover.clone())),
        actions: ActionList::default().with_move_to(Some(MoveToAction {
            state: MoveTo::Arrived,
            goal: None,
            target: 50,
            range: 0.0,
            sprite: None,
            wait: crate::movement::move_to::MoveWait::default(),
        })),
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
    t.players[1].objects = vec![walker(44), dead].into();
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
    assert_eq!(t.players[1].objects[0].actions.move_to().copied(), None);
    assert_eq!(t.players[1].objects[1].actions.move_to().copied(), None);
    let tree = &t.players[0].objects[0];
    assert_eq!((tree.workers, tree.life.goto_sleep), (0, true));
}

/// The hunt's `set_state(8)` (`0x140ac60e0` case 8): without a second target its search
/// (`FUN_1411671e0`; without planners by distance) finds the town centre, which becomes the
/// second target (`FUN_140a7a190`): the caller takes it (slot 0x178). With the drop site held
/// already it walks there and takes nothing new.
#[test]
fn a_hunter_s_drop_off_takes_the_drop_site_it_found() {
    let mut t = World::new(
        0,
        1,
        0,
        vec![0, 0],
        vec![Player::default(), Player::default()],
    );
    placeable(&mut t);
    let mut walker = crate::gathering::hunt::tests::walker();
    walker.stats.restriction = 0;
    let mut hunter = Unit {
        id: 40,
        body: crate::objects::layers::Body::new([10.5, 10.5, 0.0], None, Some(walker)),
        actions: ActionList::default().with_hunt(Some(crate::gathering::hunt::tests::hunting(-1))),
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
        body: crate::objects::layers::Body::at([20.0, 20.0, 0.0]),
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
    let others = ObjectList::from(vec![tc]);
    let drop_off = |hunter: &mut Unit, t: &mut World| {
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
        hunter.drop_off_in(world, OthersRef::alone(1, &others))
    };
    assert_eq!(drop_off(&mut hunter, &mut t), Ok(Some(9149)));
    let hunt = hunter.actions.hunt().copied().unwrap();
    assert_eq!(
        (hunt.state, hunt.carry_to),
        (crate::gathering::hunt::DROP, 9149)
    );
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
        body: crate::objects::layers::Body::at([20.0, 20.0, 0.0]),
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
    t.players[1].objects[1].actions.hunt_mut().unwrap().carry_to = 9149;
    t.tick(100);
    assert_eq!(
        (
            t.players[0].unsupported.clone(),
            t.players[1].unsupported.clone()
        ),
        (None, None)
    );
    let workers = |t: &World| {
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
    hunter
        .actions
        .set_hunt(Some(crate::gathering::hunt::HuntAction {
            state: crate::gathering::hunt::SEARCH,
            target: -1,
            carry_to: 9149,
            ..crate::gathering::hunt::tests::hunting(-1)
        }));
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
    assert_eq!(
        t.players[1].objects[1].actions.hunt().map(|h| h.carry_to),
        Some(-1)
    );
    gone.tick(100);
    assert_eq!(workers(&gone), 1);
    assert_eq!(
        gone.players[1].objects[1]
            .actions
            .hunt()
            .map(|h| h.carry_to),
        Some(-1)
    );
    // a failed hunt (state 13) is removed: its teardown lets the drop site go too
    let mut failed = deaths_world();
    failed.players[1].objects.push(site(2));
    failed.spatial = t.spatial.clone();
    let hunter = &mut failed.players[1].objects[1];
    hunter.life.hp = 1.0;
    hunter
        .actions
        .set_hunt(Some(crate::gathering::hunt::HuntAction {
            state: crate::gathering::hunt::FAILED,
            target: -1,
            carry_to: 9149,
            ..crate::gathering::hunt::tests::hunting(-1)
        }));
    failed.tick(100);
    assert_eq!(failed.players[1].unsupported, None);
    assert_eq!(workers(&failed), 0);
    assert_eq!(failed.players[1].objects[1].actions.hunt().copied(), None);
}

/// A Gather's and a Hunt's second target at the start of their updates (`0x140a8e1d0`,
/// `0x140ac7a00`): a drop site past state 6 is let go and goes, one the world's table lacks only
/// goes, one in state 2 (or 6) stays.
#[test]
fn a_second_target_past_state_6_is_let_go_and_one_gone_dropped() {
    use crate::objects::unit::second_target_checked;
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
    let mut t = World::new(
        0,
        1,
        0,
        vec![0, 0, 0, 0],
        vec![
            Player::default(),
            Player {
                objects: vec![foundation(10028, 621), foundation(10029, 70)].into(),
                relations: vec![0, 0, 2, 4],
                ..Player::default()
            },
            Player {
                objects: vec![unit(40, 83), unit(41, 83)].into(),
                relations: vec![0, 2, 0, 2],
                ..Player::default()
            },
            Player {
                objects: vec![unit(50, 83)].into(),
                relations: vec![0, 4, 2, 0],
                ..Player::default()
            },
        ],
    );
    let building = |id, stack_unit| Master {
        id,
        kind: 80,
        class: 3,
        building: Some(crate::objects::masters::BuildingMaster {
            stack_unit,
            ..crate::objects::masters::BuildingMaster::default()
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
    let by = |t: &World, p, i| t.releaser(p, i);
    let (enemy, other, ally) = (by(&t, 2, 0), by(&t, 2, 1), by(&t, 3, 0));
    for taker in [enemy, other, enemy, ally] {
        t.take(10028, taker);
        t.take(10029, taker);
    }
    let site = |t: &World| t.players[1].objects[0].clone();
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
    t.players[1].objects[0]
        .actions
        .set_artifact(Some(Artifact::new(false, None)));
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
    let a = t.players[1].objects[0].actions.artifact().copied().unwrap();
    assert_eq!((a.target, a.goal), (9150, Some([25.0, 20.0, 0.0])));
    assert_eq!(t.players[1].objects[0].body.anim_speed(), Some(0.0));
}

/// A building's slot 0x70 (`FUN_1411cd860`), as Aftermath's trade workshops change hands: Gaia's
/// workshop 7 (master 1378, class 1, the artifact's unit AI) finds player 1's villager 500 and
/// asks to change hands; after the update its whole change runs. Built, it gives its storage
/// of flag 4 (5 of attribute 4) from Gaia to player 1, leaves Gaia's watched list for player
/// 1's, completes in player 1's tracker, loses its gather point and its actions, and comes to
/// the end of player 1's list with its record again.
#[test]
fn a_gaia_building_with_the_artifact_s_unit_ai_changes_hands_whole() {
    let master = |id, class, los| Master {
        id,
        kind: 70,
        class,
        line_of_sight: los,
        radius: [0.2, 0.2],
        ..Master::default()
    };
    let workshop = Master {
        kind: 80,
        build_time: 10,
        storage: vec![(4, 5.0, 4), (-1, 0.0, 0), (-1, 0.0, 0)],
        obstruction_type: 2,
        obstruction_class: 2,
        building: Some(crate::objects::masters::BuildingMaster {
            stack_unit: -1,
            foundation_terrain: -1,
            tech: -1,
            head_unit: -1,
            transform_unit: -1,
            ..crate::objects::masters::BuildingMaster::default()
        }),
        ..master(1378, 1, 4.0)
    };
    let table = Masters::new([workshop, master(83, 4, 4.0)]);
    let torch = Unit {
        ai: Some(UnitAi {
            ai_type: crate::unit_ai::ARTIFACT_AI,
            current_order: None,
            current_action: None,
            current_target: -1,
            idle_timeout: 5000,
            adjusted_idle_timeout: 4000,
            retarget_timer: 1000,
            ..UnitAi::default()
        }),
        life: Lifecycle {
            master_id: 1378,
            state: 2,
            hp: 20000.0,
            ..Lifecycle::default()
        },
        construction: Some(Construction {
            progress: 10.0,
            desolid: false,
            builders: 0,
            hit_points: 20000,
            build_time: 10,
            record: crate::map::spatial::Obstruction { kind: 2, class: 2 },
            snow: false,
            sprite_override: false,
            started: true,
        }),
        gather_point: Some(GatherPoint {
            point: [12.0, 12.0, 0.0],
            target: -1,
            target_master: -1,
        }),
        ..hands_unit(7, 1378)
    };
    let villager = Unit {
        body: crate::objects::layers::Body::at([11.5, 10.5, 0.0]),
        ai: None,
        ..hands_unit(500, 83)
    };
    let mut t = World::new(
        0,
        1,
        0,
        vec![0, 0, 0],
        vec![
            Player {
                objects: vec![torch].into(),
                attributes: vec![0.0; 300],
                ..Player::default()
            },
            Player {
                objects: vec![villager].into(),
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
    t.spatial = hands_world(7, 0, 1, 1378, 4.0);
    t.players[0].attributes[4] = 5.0;
    t.players[0].watched.push(crate::vision::ghost::Watched {
        id: 7,
        cell: crate::vision::ghost::Cell::Tile(10 * 40 + 10),
        bits: 0,
    });
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
    assert_eq!(t.players[0].unsupported, None);
    assert_eq!(t.players[0].objects.len(), 0);
    let ids: Vec<i32> = t.players[1].objects.iter().map(|u| u.id).collect();
    assert_eq!(ids, [500, 7]);
    assert_eq!(t.spatial.get(7).map(|p| p.player), Some(1));
    assert_eq!(
        [t.players[0].attributes[4], t.players[1].attributes[4]],
        [0.0, 5.0]
    );
    let watched = |p: usize| t.players[p].watched.iter().any(|w| w.id == 7);
    assert_eq!([watched(0), watched(1)], [false, true]);
    let shop = &t.players[1].objects[1];
    assert_eq!(shop.gather_point, None);
    assert_eq!(shop.own_master.as_deref().map(|m| m.id), Some(1378));
    assert!(t.spatial.get(7).is_some_and(|p| p.obstruction.is_some()));
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
            current_order: None,
            current_action: None,
            current_target: -1,
            idle_timeout: 5000,
            adjusted_idle_timeout: 4000,
            retarget_timer: 1000,
            ..UnitAi::default()
        }),
        ..hands_unit(7, 854)
    };
    let villager = Unit {
        body: crate::objects::layers::Body::at([11.5, 10.5, 0.0]),
        ai: None,
        ..hands_unit(500, 83)
    };
    let mut t = World::new(
        0,
        1,
        0,
        vec![0, 0, 0],
        vec![
            Player {
                objects: vec![torch].into(),
                attributes: vec![0.0; 300],
                ..Player::default()
            },
            Player {
                objects: vec![villager].into(),
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
    let mut walker = crate::gathering::hunt::tests::walker();
    walker.velocity = None;
    walker.stats.speed = 0.8;
    walker.stats.stand_speed = 0.5;
    let sheep = Unit {
        actions: ActionList::default().with_artifact(Some(Artifact::new(true, None))),
        body: crate::objects::layers::Body::Moving(walker),
        ai: None,
        ..hands_unit(9520, 705)
    };
    let villager = Unit {
        body: crate::objects::layers::Body::at([11.5, 10.5, 0.0]),
        ai: None,
        ..hands_unit(500, 83)
    };
    let mut t = World::new(
        0,
        1,
        0,
        vec![0, 0, 0],
        vec![
            Player {
                objects: vec![sheep].into(),
                attributes: vec![0.0; 300],
                ..Player::default()
            },
            Player {
                objects: vec![villager].into(),
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
    assert_eq!(sheep.body.mover().map(|m| m.stats.speed), Some(0.5));
    assert_eq!(sheep.actions.artifact().map(|a| a.gaia), Some(false));
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
    let mut t = World::new(
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

/// `FUN_141082e80` before the kill's counts: a killed villager (class 4) counts for its owner an
/// animal attacker (class 9 or 10) in attribute 227, and by the attacker's player type a human's
/// in 229, Gaia's in 226, a computer's in 228 (type 0 none), the attacker's class as the hit
/// saw it (an attacker the world does not hold then, out for its update or gone, counts too); a
/// computer player's killed object (its AI is told) is not modelled.
#[test]
fn a_killed_villager_counts_its_attacker_s_kind_and_player_type() {
    let unit = |id, master_id| Unit {
        id,
        life: Lifecycle {
            master_id,
            ..Lifecycle::default()
        },
        ..Unit::default()
    };
    let player = |kind, objects| Player {
        objects,
        kind,
        attributes: vec![0.0; 500],
        ..Player::default()
    };
    let world = |attacker_kind| {
        let mut t = World::new(
            0,
            1,
            0,
            vec![0; 3],
            vec![
                player(2, vec![unit(50, 126)].into()),
                player(1, vec![unit(40, 83), unit(41, 83)].into()),
                player(attacker_kind, vec![unit(60, 4)].into()),
            ],
        );
        let masters = Masters::new([
            Master {
                id: 83,
                class: 4,
                ..Master::default()
            },
            Master {
                id: 126,
                class: 9,
                ..Master::default()
            },
            Master {
                id: 4,
                class: 0,
                ..Master::default()
            },
        ]);
        t.masters = vec![masters; 3];
        t
    };
    let by = |attacker, attacker_player, attacker_class| crate::combat::damage::Kill {
        attacker,
        attacker_player,
        attacker_class,
        to_gaia: false,
    };
    let counts = |t: &World| [226, 227, 228, 229].map(|i| t.players[1].attributes[i]);
    // Gaia's wolf: an animal, Gaia's type
    let mut t = world(1);
    t.killed(1, 0, by(50, 0, 9)).unwrap();
    assert_eq!(counts(&t), [1.0, 1.0, 0.0, 0.0]);
    // the archers of a human, a computer and the editor's phantom
    for (kind, seen) in [
        (1, [0.0, 0.0, 0.0, 1.0]),
        (3, [0.0, 0.0, 1.0, 0.0]),
        (0, [0.0; 4]),
    ] {
        let mut t = world(kind);
        t.killed(1, 1, by(60, 2, 0)).unwrap();
        assert_eq!(counts(&t), seen);
    }
    let mut t = world(1);
    t.killed(1, 0, by(99, 2, 10)).unwrap();
    assert_eq!(counts(&t), [0.0, 1.0, 0.0, 1.0]);
    t.players[1].computer = true;
    assert!(t.killed(1, 0, by(60, 2, 0)).is_err());
}

/// [`a_killed_sheep_goes_to_gaia`]'s world: player 1's town centre 10029 (`killer_last`: the
/// list's last object) and its sheep 9520, and the hit that kills the sheep for Gaia.
fn killed_sheep(killer_last: bool) -> (World, crate::combat::damage::Victim) {
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
    let objects = if killer_last {
        vec![sheep, town_centre(10029)]
    } else {
        vec![town_centre(10029), sheep]
    }
    .into();
    let mut t = World::new(
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
                objects,
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
    v.damaged = vec![(10029, [10029, -10, 14])];
    v.killed = Some(crate::combat::damage::Kill {
        attacker: 10029,
        attacker_player: 1,
        attacker_class: 80,
        to_gaia: true,
    });
    (t, v)
}

/// MarvinBlue/426048731's update 1250: 10029's second hit kills its sheep 9520 (2 hit points,
/// 12 damage): it has none, Gaia takes it (its list, its storage, its unit AI's notifies),
/// and the players count the kill (300 + 1 and 325 + 1; a sheep costs nothing).
#[test]
fn a_killed_sheep_goes_to_gaia() {
    let (mut t, v) = killed_sheep(false);
    write_hit(&mut t, (1, 0), v);
    assert_eq!(t.players[1].objects.len(), 1);
    let gone = &t.players[0].objects[0];
    assert_eq!((gone.id, gone.life.hp), (9520, 0.0));
    assert_eq!(t.spatial.get(9520).map(|p| p.player), Some(0));
    assert_eq!(t.players[1].attributes[301], 1.0);
    assert_eq!(t.players[1].attributes[326], 1.0);
    assert_eq!(t.players[0].attributes[16], 100.0);
}

/// The update lends its object back to its slot for a world call that may reach it
/// ([`step::Step::lend`], the game's world holding the object while its update runs):
/// the world holds it as the update left it, what the call changes on it comes back, and its
/// slot holds the stand-in again after; a removal in the call that moves the object is
/// followed. One the call takes from its list fails the update, the stand-in in nobody's list.
#[test]
fn a_lent_object_is_the_world_s_for_the_call() {
    let mut t = house_builder();
    let last = t.players[1].objects.len() - 1;
    let id = t.players[1].objects[last].id;
    let seen = in_update(&mut t, (1, last), 0, |unit, c| {
        unit.life.hp = 7.0;
        let during = c.lend(unit, |w| {
            let hp = w.object(id).map(|(_, u)| u.life.hp);
            w.object_mut(id).expect("lent").1.life.amount = 3.0;
            let _ = w.players[1].objects.swap_remove(0);
            Ok(hp)
        })?;
        let out = c.w.object(id).is_none();
        Ok((during, unit.life.amount, out, unit.id))
    });
    assert_eq!(
        seen.map_err(|e| e.reason().to_owned()),
        Ok((Some(7.0), 3.0, true, id))
    );
    assert_eq!(t.players[1].objects.position_of(id), Some(0));
    let mut t = house_builder();
    let failed = in_update(&mut t, (1, last), 0, |unit, c| {
        c.lend(unit, |w| {
            let _ = w.players[1].objects.pop();
            Ok(())
        })
    });
    assert_eq!(
        failed.map_err(|e| e.kind()),
        Err(crate::ErrorKind::Unreachable)
    );
    let ids: Vec<i32> = t.players[1].objects.iter().map(|u| u.id).collect();
    assert!(!ids.contains(&crate::objects::unit::TAKEN_OUT), "{ids:?}");
}

/// The hit's kill gives Gaia the sheep in slot 0 of its owner's list where the hit runs
/// (`FUN_1411664a0`: the list's last object takes its slot): the town centre that steps, the
/// last, moves into it while it is out, goes back there, and the step reports that slot for the
/// list's loop to file it at.
#[test]
fn a_kill_in_the_step_reports_the_object_s_new_slot() {
    let (mut t, mut v) = killed_sheep(true);
    let stepped = t
        .step_object((1, 1), (0, None, false), None, |_, c| {
            if let Some(k) = crate::combat::victims::write_back(c, &mut v)? {
                crate::combat::victims::file_kill(c, k)?;
            }
            Ok(crate::objects::unit::Outcome::Stay)
        })
        .unwrap();
    assert_eq!(stepped.slot, 0);
    assert_eq!(t.players[1].objects.position_of(10029), Some(0));
    assert_eq!(t.players[0].objects[0].id, 9520);
}

/// A hit's kill reaches the world where the hit runs (the damage slot `FUN_1412108e0` kills and
/// counts at once: `FUN_141082e80`, slot 0x8b0): the update's later parts see the sheep Gaia's
/// and the kill counted. The updating unit's own kill (its target's retaliation) waits for the
/// step's end, its slot holding the stand-in.
#[test]
fn a_hit_s_kill_reaches_the_world_where_the_hit_runs() {
    use crate::combat::victims::{KillLeft, file_kill};
    let (mut t, mut v) = killed_sheep(false);
    let seen = in_update(&mut t, (1, 0), 0, |_, c| {
        if let Some(k) = crate::combat::victims::write_back(c, &mut v)? {
            file_kill(c, k)?;
        }
        let gaia_s = c.w.players[0].objects.position_of(9520).is_some();
        let counted = c.w.players[1].attributes[301];
        let kill = crate::combat::damage::Kill {
            attacker: 9520,
            attacker_player: 0,
            attacker_class: 58,
            to_gaia: false,
        };
        file_kill(c, KillLeft::own(c.me, kill))?;
        let waits = c.kills.len();
        c.kills.clear();
        Ok((gaia_s, counted, waits))
    });
    assert_eq!(seen.unwrap(), (true, 1.0, 1));
}

/// A world call in the step that moves the stepping object's stand-in (a removal from its list,
/// the stand-in its last object) puts the object back where the stand-in went; one whose
/// stand-in left the list goes to its end, and the step fails.
#[test]
fn a_step_puts_its_object_back_where_its_stand_in_went() {
    use crate::objects::unit::Outcome;
    let ids = |t: &World| {
        t.players[1]
            .objects
            .iter()
            .map(|u| u.id)
            .collect::<Vec<_>>()
    };
    let mut t = house_builder();
    let before = ids(&t);
    let last = before.len() - 1;
    let stepped = t
        .step_object((1, last), (50, None, false), None, |_, c| {
            let _ = c.w.players[1].objects.swap_remove(0);
            Ok(Outcome::Stay)
        })
        .unwrap();
    assert_eq!(stepped.slot, 0);
    assert_eq!(ids(&t)[0], before[last]);
    let mut t = house_builder();
    let failed = t
        .step_object((1, 0), (50, None, false), None, |_, c| {
            c.w.players[1].objects.truncate(0);
            Ok(Outcome::Stay)
        })
        .err()
        .unwrap();
    assert_eq!(failed.kind(), crate::ErrorKind::Unreachable);
    assert_eq!(ids(&t), [before[0]]);
}

/// The list's loop goes on by its own count (`FUN_141166550`): past an object that stays,
/// sleeps or changes hands, and again at the same count after one that leaves, whose slot the
/// list's last object took.
#[test]
fn the_list_s_loop_runs_its_count_again_only_after_a_leave() {
    use crate::objects::unit::Outcome;
    let mut t = house_builder();
    let n = t.players[1].objects.len();
    assert!(t.file_step(1, 0, Outcome::Stay));
    assert!(t.file_step(1, 0, Outcome::Sleep));
    assert_eq!(t.players[1].objects.len(), n - 1);
    assert!(!t.file_step(1, 0, Outcome::Leave));
    assert_eq!(t.players[1].objects.len(), n - 2);
}

/// A hit on player 2's castle 30 (master 82, 650 stone paid) with its annexes 31 and 32:
/// `World::victim` reads its annexes from the world's links, its main building for an annex,
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
    let mut t = World::new(
        0,
        1,
        0,
        vec![0, 0, 0],
        vec![
            player(vec![].into()),
            player(vec![building(9, 82, None)].into()),
            player(
                vec![
                    building(30, 82, None),
                    building(31, 1, Some(30)),
                    building(32, 1, Some(30)),
                ]
                .into(),
            ),
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
    v.killed = Some(crate::combat::damage::Kill {
        attacker: 9,
        attacker_player: 1,
        attacker_class: 0,
        to_gaia: false,
    });
    let b = v.building.as_mut().unwrap();
    (b.struck, b.damaged_lately) = (true, 60);
    b.razed.push(crate::combat::damage::Razed {
        attacker_player: 1,
        owner: 2,
        event: Some(crate::players::history::CASTLE_DESTROYED),
        adds: vec![(1, 43, 1.0), (2, 155, 1.0), (1, 170, 650.0)],
    });
    write_hit(&mut t, (1, 0), v);
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
        .event_of_kind(crate::players::history::CASTLE_DESTROYED)
        .unwrap();
    assert_eq!(event.data1, 2.0);
}

/// The hit `v` written back where it ran, in the update of object `i` of player `p`
/// ([`crate::combat::victims::write_back`]), its kill and razings with it
/// ([`crate::combat::victims::file_kill`]).
fn write_hit(t: &mut World, (p, i): (usize, usize), mut v: crate::combat::damage::Victim) {
    in_update(t, (p, i), 0, |_, c| {
        if let Some(k) = crate::combat::victims::write_back(c, &mut v)? {
            crate::combat::victims::file_kill(c, k)?;
        }
        Ok(())
    })
    .unwrap();
}

/// A hit on a sleeping object (a tree a blast reached) written back where it ran: it takes
/// its hit points and what it holds and sleeps on, still flagged, at 1 hit point or more;
/// below 1 it wakes to the end of its player's live list and takes the rest, the hit's cleared
/// sleep flag among it.
#[test]
fn a_hit_wakes_a_sleeping_object_only_below_1_hit_point() {
    let tree = |id| Unit {
        id,
        life: Lifecycle {
            master_id: 349,
            hp: 20.0,
            amount: 100.0,
            state: 2,
            goto_sleep: true,
            ..Lifecycle::default()
        },
        ..Unit::default()
    };
    let hit = |hp: f32| {
        let mut t = World::new(
            0,
            1,
            0,
            vec![0, 0],
            vec![
                Player {
                    objects: vec![tree(51)].into(),
                    sleeping: vec![tree(50)].into(),
                    attributes: vec![0.0; 500],
                    ..Player::default()
                },
                Player {
                    objects: vec![tree(9)].into(),
                    attributes: vec![0.0; 500],
                    ..Player::default()
                },
            ],
        );
        let master = Master {
            id: 349,
            kind: 10,
            class: 15,
            ..Master::default()
        };
        t.masters = vec![Masters::new([master.clone()]), Masters::new([master])];
        let mut v = t.victim(50, 1).unwrap();
        (v.hp, v.amount, v.goto_sleep) = (hp, 90.0, false);
        write_hit(&mut t, (1, 0), v);
        let gaia = &t.players[0];
        let ids = |l: &ObjectList| -> Vec<i32> { l.iter().map(|u| u.id).collect() };
        let (_, u) = t.object(50).unwrap();
        (
            ids(&gaia.objects),
            ids(&gaia.sleeping),
            (u.life.hp, u.life.amount, u.life.goto_sleep),
        )
    };
    assert_eq!(hit(5.0), (vec![51], vec![50], (5.0, 90.0, true)));
    assert_eq!(hit(0.5), (vec![51, 50], vec![], (0.5, 90.0, false)));
}

/// `FUN_140fde380`: a master of type 70 and up, of class 51 or 54, with a weapon (`+0x810`
/// above 0) is a trebuchet; the paid cost (`+0x880`) sums the entries with a resource and the
/// paid flag, as floats in their order.
#[test]
fn a_trebuchet_is_a_master_of_class_51_or_54_with_a_weapon() {
    use crate::combat::victims::is_trebuchet;
    use crate::combat::victims::paid_cost;
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
    let set = |target, point, own| {
        Command::GatherPoint(GatherPointCommand {
            player: player_id(0),
            units: unit_list(object_ids([9149])),
            point,
            target,
            target_master: Some(master_id(705)),
            own,
        })
    };
    t.tick_with(
        25,
        &[set(
            Some(object_id(9520)),
            Point::new(107.2451, 46.075_863),
            false,
        )],
    );
    assert_eq!(t.players[0].unsupported, None);
    assert_eq!(
        t.players[0].objects[0].gather_point,
        Some(GatherPoint {
            point: [107.2451, 46.075_863, 0.0],
            target: 9520,
            target_master: 705,
        })
    );
    t.tick_with(25, &[set(None, Point::new(-1.0, -1.0), false)]);
    assert_eq!(t.players[0].objects[0].gather_point, None);
    t.tick_with(25, &[set(None, Point::new(-1.0, -1.0), true)]);
    assert_eq!(
        t.players[0].objects[0].gather_point.map(|g| g.target),
        Some(9149)
    );
}

/// SET GATHER POINT's test (`FUN_140e7e9a0`): a building whose master does not garrison takes
/// the gather point when the player trains a unit there (`FUN_141625a20`'s count above 0); one
/// that neither garrisons nor trains ignores it, and the player goes on.
#[test]
fn set_gather_point_takes_a_building_that_trains_without_garrisoning() {
    let mut barracks = town_centre(10908);
    barracks.life.master_id = 12;
    let mut house = town_centre(10909);
    house.life.master_id = 70;
    let mut t = one_player(vec![barracks, house], [200.0; 4]);
    t.masters = vec![Masters::new([
        Master {
            id: 12,
            kind: 80,
            ..Master::default()
        },
        Master {
            id: 70,
            kind: 80,
            ..Master::default()
        },
        Master {
            id: 74,
            kind: 70,
            available: true,
            trained_at: vec![12],
            ..Master::default()
        },
    ])];
    let set = Command::GatherPoint(GatherPointCommand {
        player: player_id(0),
        units: unit_list(object_ids([10908, 10909])),
        point: Point::new(20.5, 30.5),
        target: None,
        target_master: None,
        own: false,
    });
    t.tick_with(25, &[set]);
    assert_eq!(t.players[0].unsupported, None);
    assert_eq!(
        t.players[0].objects[0].gather_point,
        Some(GatherPoint {
            point: [20.5, 30.5, 0.0],
            target: -1,
            target_master: -1,
        })
    );
    assert_eq!(t.players[0].objects[1].gather_point, None);
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
        current_order: None,
        current_action: None,
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
    let step = |point| {
        Command::CommandQueue(BuildingQueueCommand {
            player: player_id(0),
            units: unit_list(object_ids([9149])),
            target: None,
            point,
        })
    };
    t.tick_with(25, &[step(Point::new(50.0, 60.0))]);
    t.tick_with(25, &[step(Point::new(70.0, 80.0))]);
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
    let at = |x, y| (OrderKind::Move, -1, [x, y, -1.0], -1, false);
    assert_eq!(steps, vec![at(30.5, 40.5), at(50.0, 60.0), at(70.0, 80.0)]);
    t.tick_with(
        25,
        &[Command::GatherPoint(GatherPointCommand {
            player: player_id(0),
            units: unit_list(object_ids([9149])),
            point: Point::new(-1.0, -1.0),
            target: None,
            target_master: None,
            own: false,
        })],
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
    let mut m = crate::gathering::hunt::tests::walker();
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
        path: crate::pathing::tile_path::Path {
            points: vec![[20.0, 0.5, 20.0], [30.0, 0.5, 20.25]],
            length: 10.0,
        },
        flags: 7,
    };
    let mut u = Unit {
        body: crate::objects::layers::Body::new(
            [0.0; 3],
            None,
            Some(crate::gathering::hunt::tests::walker()),
        ),
        ai: Some(UnitAi {
            patrol_path: Some(saved),
            patrol_waypoint: 1,
            ..UnitAi::default()
        }),
        actions: ActionList::default().with_move_to(Some(MoveToAction {
            state: MoveTo::Moving,
            goal: None,
            target: 77,
            range: 0.0,
            sprite: None,
            wait: crate::movement::move_to::MoveWait::default(),
        })),
        ..Unit::default()
    };
    let run = u.restore_patrol().expect("the saved patrol's move");
    assert_eq!(
        (run.point, run.range, run.force, run.walk),
        ([30.0, 20.25, 0.5], 0.0, true, false)
    );
    assert_eq!(
        u.body.mover().unwrap().user_waypoints,
        [[20.5, 0.5, 20.5], [30.5, 0.5, 20.25]]
    );
    assert_eq!(ai_move_of(&mut u, run, 6), Ok(vec![77]));
    let move_to = u.actions.move_to().copied().unwrap();
    assert_eq!(
        (move_to.goal, move_to.range, move_to.sprite.map(|g| g.id)),
        (Some([30.0, 20.25, 0.5]), 0.0, None)
    );
}

/// `TRIBE_Combat_Object::update` (`0x141208440`) before `RGE_Combat_Object::update`: a unit
/// of class 35 (the petard; also 64 and the masters 527, 528, 1104, 2133 to 2135) below 1 hit
/// point in state 2 blasts about its place (slot 0x848, `FUN_14107f690`, factor 1, itself left
/// out, no main target), then dies. 426048706 (Aftermath): Gaia's petards 7948.. placed at −1
/// hit points blast the players' walls in update 1.
#[test]
fn a_petard_below_1_hit_point_blasts_about_itself_then_dies() {
    use crate::map::spatial::Placed;
    let petard = Master {
        id: 706,
        kind: 70,
        class: 35,
        hit_points: 45,
        weapons: vec![(4, 100)],
        area_effect: 2.0,
        area_damage: 1.0,
        missile: -1,
        death_sprite: 1751,
        ..Master::default()
    };
    let wall = Master {
        id: 117,
        kind: 80,
        class: 27,
        hit_points: 1800,
        armors: vec![(4, 0)],
        terrain_bonus: -1,
        ..Master::default()
    };
    let unit = |id, master, hp, position| Unit {
        id,
        body: crate::objects::layers::Body::at(position),
        pasture: -1,
        my_pasture: -1,
        life: Lifecycle {
            master_id: master,
            state: 2,
            hp,
            ..Lifecycle::default()
        },
        ..Unit::default()
    };
    let mut t = World::new(
        0,
        1,
        0,
        vec![0, 0],
        vec![
            Player {
                objects: vec![unit(7948, 706, -1.0, [101.5, 158.5, 0.0])].into(),
                attributes: vec![0.0; 250],
                ..Player::default()
            },
            Player {
                objects: vec![unit(7231, 117, 1080.0, [102.5, 158.5, 0.0])].into(),
                attributes: vec![0.0; 250],
                ..Player::default()
            },
        ],
    );
    let tree = Master {
        id: 1250,
        kind: 10,
        class: 15,
        hit_points: 500,
        ..Master::default()
    };
    t.masters = vec![Masters::new([petard, tree]), Masters::new([wall])];
    // a sleeping tree beside it (426048706's 8858): the blast walks the sleeping too
    t.players[0].sleeping.push(Unit {
        life: Lifecycle {
            class: Class::Static,
            ..unit(8858, 1250, 500.0, [100.5, 158.5, 0.0]).life
        },
        ..unit(8858, 1250, 500.0, [100.5, 158.5, 0.0])
    });
    t.graphics = std::collections::BTreeMap::from([(
        1751,
        Graphic {
            id: 1751,
            frames: 10,
            facets: 1,
            speed_multiplier: 1.0,
            frame_duration: 0.1,
            replay_delay: 0.0,
            flags: 1,
        },
    )]);
    let placed = |x: f32, class, player, master| Placed {
        position: [x, 158.5, 0.0],
        radius: [0.5, 0.5],
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
    t.spatial = crate::map::spatial::Spatial::new([200, 200]);
    t.spatial.insert(7948, placed(101.5, 35, 0, 706));
    t.spatial.insert(7231, placed(102.5, 27, 1, 117));
    t.spatial.insert(8858, placed(100.5, 15, 0, 1250));
    // a sleeping tree of id 0 out of the blast's reach but among the objects about it
    // (428491175's oak 0 at (107.5, 0.5), 9 tiles from the petard 5342): the updating petard's
    // place in its list holds no object of that id, so the tree is read as itself and sleeps on
    let far = unit(0, 1250, 500.0, [95.5, 158.5, 0.0]);
    t.players[0].sleeping.push(Unit {
        life: Lifecycle {
            class: Class::Static,
            ..far.life
        },
        ..far
    });
    t.spatial.insert(0, placed(95.5, 15, 0, 1250));
    t.tick(100);
    assert_eq!(t.players[0].unsupported, None);
    assert_eq!(t.players[1].objects[0].life.hp, 980.0);
    // the sleeping tree is among the objects the blast walks; a petard's blast (its weapon
    // range at most 1) spares its own player's objects, so Gaia's tree sleeps on untouched
    assert_eq!(
        t.players[0]
            .sleeping
            .iter()
            .map(|u| (u.id, u.life.hp))
            .collect::<Vec<_>>(),
        [(8858, 500.0), (0, 500.0)]
    );
    assert!(3 <= t.players[0].objects[0].life.state);
}

/// A dying building lets its annexes go with its hit points (the static update's death,
/// `0x14118ad50`): a gate (426048706's 7228, master 88) at no hit points dies (3) and goes on to
/// its dying sprite (the animated update's `set_state(4)`); its annex (7229, master 95, holding
/// nothing), after it in the list, dies in its own update of the same tick and reaches state 4
/// too, no longer an annex.
#[test]
fn an_annex_follows_its_dying_main_building_into_state_4() {
    let g = Graphic {
        id: 1751,
        frames: 10,
        facets: 1,
        speed_multiplier: 1.0,
        frame_duration: 0.1,
        replay_delay: 0.0,
        flags: 1,
    };
    let gate = |id| Master {
        id,
        kind: 80,
        class: 39,
        hit_points: 1650,
        standing_sprite: 1751,
        death_sprite: 1751,
        building: Some(crate::objects::masters::BuildingMaster {
            stack_unit: -1,
            foundation_terrain: -1,
            tech: -1,
            head_unit: -1,
            transform_unit: -1,
            ..crate::objects::masters::BuildingMaster::default()
        }),
        ..Master::default()
    };
    let building = |id, master, hp| Unit {
        id,
        body: crate::objects::layers::Body::at([124.5, 174.5, 0.0]),
        pasture: -1,
        my_pasture: -1,
        sprite: Some(crate::objects::sprite::ActiveSprite::start(
            g,
            &mut Well512::default(),
        )),
        life: Lifecycle {
            class: Class::Animated,
            master_id: master,
            state: 2,
            hp,
            dying_sprite: Some(1751),
            master: Some(MasterData::default()),
            ..Lifecycle::default()
        },
        construction: Some(Construction {
            progress: 100.0,
            desolid: false,
            builders: 0,
            hit_points: 1650,
            build_time: 100,
            record: crate::map::spatial::Obstruction { kind: 2, class: 2 },
            snow: false,
            sprite_override: false,
            started: false,
        }),
        ..Unit::default()
    };
    let mut t = World::new(
        0,
        1,
        0,
        vec![0, 0],
        vec![
            Player {
                attributes: vec![0.0; 250],
                ..Player::default()
            },
            Player {
                objects: vec![
                    building(7228, 88, -58.0),
                    Unit {
                        annex_of: Some(7228),
                        ..building(7229, 95, 1650.0)
                    },
                ]
                .into(),
                attributes: vec![0.0; 250],
                ..Player::default()
            },
        ],
    );
    t.masters = vec![Masters::new([]), Masters::new([gate(88), gate(95)])];
    t.graphics = std::collections::BTreeMap::from([(1751, g)]);
    t.tick(100);
    assert_eq!(t.players[1].unsupported, None);
    let states: Vec<(i32, u8)> = t.players[1]
        .objects
        .iter()
        .map(|u| (u.id, u.life.state))
        .collect();
    assert_eq!(states, [(7228, 4), (7229, 4)]);
    assert_eq!(t.players[1].objects[1].annex_of, None);
    assert_eq!(t.spatial.annexes_of(7228), []);
}

/// 428799090's palisade gate foundation (34600, master 800) dies at no hit points in update 2674
/// while its two annexes (34601, 34602, master 1192) come before it in the list: they stay
/// foundations with the gate's hit points (0) and no main building, then die in their own
/// update of the next tick on their own sprites, not the gate's states.
#[test]
fn annexes_before_their_dying_foundation_die_in_the_next_update() {
    let g = |id, flags| Graphic {
        id,
        frames: 10,
        facets: 1,
        speed_multiplier: 1.0,
        frame_duration: 0.1,
        replay_delay: 0.0,
        flags,
    };
    let master = |id, death_sprite, destruction_sprite| Master {
        id,
        kind: 80,
        class: 39,
        hit_points: 10,
        build_time: 70,
        standing_sprite: 1751,
        death_sprite,
        building: Some(crate::objects::masters::BuildingMaster {
            stack_unit: -1,
            foundation_terrain: -1,
            tech: -1,
            head_unit: -1,
            transform_unit: -1,
            destruction_sprite,
            construction_sprite: -1,
            ..crate::objects::masters::BuildingMaster::default()
        }),
        ..Master::default()
    };
    let foundation = |id, m, hp, annex_of| Unit {
        id,
        pasture: -1,
        my_pasture: -1,
        annex_of,
        sprite: Some(crate::objects::sprite::ActiveSprite::at_rest(g(1751, 0), 0)),
        body: crate::objects::layers::Body::new(
            [150.0, 46.0, 0.0],
            Some(crate::objects::layers::Animated::default()),
            None,
        ),
        life: Lifecycle {
            class: Class::Animated,
            master_id: m,
            state: 0,
            hp,
            dying_sprite: Some(38),
            master: Some(MasterData::default()),
            ..Lifecycle::default()
        },
        construction: Some(Construction {
            progress: 10.0,
            desolid: false,
            builders: 0,
            hit_points: 10,
            build_time: 70,
            record: crate::map::spatial::Obstruction { kind: 2, class: 2 },
            snow: false,
            sprite_override: false,
            started: true,
        }),
        ..Unit::default()
    };
    let mut t = World::new(
        0,
        1,
        0,
        vec![0, 0],
        vec![
            Player {
                attributes: vec![0.0; 250],
                ..Player::default()
            },
            Player {
                objects: vec![
                    foundation(34602, 1192, 1.0, Some(34600)),
                    foundation(34601, 1192, 1.0, Some(34600)),
                    foundation(34600, 800, 0.0, None),
                ]
                .into(),
                attributes: vec![0.0; 250],
                ..Player::default()
            },
        ],
    );
    t.spatial.set_annex(34602, Some(34600));
    t.spatial.set_annex(34601, Some(34600));
    t.masters = vec![
        Masters::new([]),
        Masters::new([master(800, 38, -1), master(1192, 37, 112)]),
    ];
    t.graphics = std::collections::BTreeMap::from([
        (1751, g(1751, 0)),
        (38, g(38, 9)),
        (37, g(37, 9)),
        (112, g(112, 9)),
    ]);
    let looks = |t: &World| -> Vec<(i32, u8, f32, Option<i32>)> {
        t.players[1]
            .objects
            .iter()
            .map(|u| (u.id, u.life.state, u.life.hp, u.annex_of))
            .collect()
    };
    t.tick(100);
    assert_eq!(t.players[1].unsupported, None);
    assert_eq!(
        looks(&t),
        [
            (34602, 0, 0.0, None),
            (34601, 0, 0.0, None),
            (34600, 4, 0.0, None)
        ]
    );
    t.tick(100);
    assert_eq!(t.players[1].unsupported, None);
    let states: Vec<(i32, u8)> = looks(&t).iter().map(|l| (l.0, l.1)).collect();
    assert_eq!(states, [(34602, 4), (34601, 4), (34600, 4)]);
    let sprites: Vec<Option<i16>> = t.players[1]
        .objects
        .iter()
        .map(|u| u.sprite.map(|s| s.graphic.id))
        .collect();
    assert_eq!(sprites, [Some(112), Some(112), Some(38)]);
}

/// The regeneration's end: an object at its most hit points after it (slot 0xc0 at least slot
/// 200) has the armor its charge's hits took back (`+0x9e4`/`+0x9e6` zeroed); below its most it
/// keeps the loss.
#[test]
fn a_unit_regenerated_to_its_most_has_its_armor_back() {
    let berserk = Master {
        id: 692,
        kind: 70,
        hit_points: 54,
        regeneration: 20.0,
        ..Master::default()
    };
    let unit = |hp| Unit {
        id: 1,
        life: Lifecycle {
            state: 2,
            hp,
            ..Lifecycle::default()
        },
        armor_removed: [2, 1],
        ..Unit::default()
    };
    let mut healed = unit(53.0);
    healed.regenerate(&berserk, 3.0);
    assert_eq!((healed.life.hp, healed.armor_removed), (54.0, [0, 0]));
    let mut hurt = unit(40.0);
    hurt.regenerate(&berserk, 3.0);
    assert_eq!((hurt.life.hp, hurt.armor_removed), (41.0, [2, 1]));
}

/// `TRIBE_Combat_Object::update`'s regeneration (`0x141208440`): the rate (slot 0x260,
/// `FUN_1412033a0`) is the stinger plus (the powerup plus (a hero's 30 plus the master's)); a
/// whole hit point comes per 60 / |rate| seconds of the world's dt plus the time held back
/// (`+0xa10`), the fraction kept as time. A positive rate heals up to the most (slot 0xb8); a
/// negative one takes off at most the hit points (slot 0xb0); below 1 hit point, out of state
/// 2 or below 1.2e-7 in size nothing changes.
#[test]
fn a_unit_regenerates_whole_hit_points_and_keeps_the_rest_as_time() {
    use crate::objects::hit_points::regeneration_rate;
    let berserk = Master {
        id: 692,
        kind: 70,
        hit_points: 54,
        regeneration: 20.0,
        ..Master::default()
    };
    let unit = |hp| Unit {
        id: 1,
        life: Lifecycle {
            state: 2,
            hp,
            ..Lifecycle::default()
        },
        ..Unit::default()
    };
    // 20 a minute: a point per 3 s; 2 s of dt plus 1.5 held is one point, the rest held back
    // (in f32: 3.5 / 3 less its whole point, times 3)
    let mut u = unit(30.0);
    u.regen_timer = 1.5;
    u.regenerate(&berserk, 2.0);
    assert_eq!(
        (u.life.hp, u.regen_timer),
        (31.0, ((3.5f32 / 3.0) % 1.0) * 3.0)
    );
    // at the most nothing is added, and the time still runs
    let mut full = unit(54.0);
    full.regenerate(&berserk, 3.0);
    assert_eq!((full.life.hp, full.regen_timer), (54.0, 0.0));
    // a hero's 30 and the powerups: stinger + (powerup + (30 + master))
    let hero = Master {
        hero_flag: 1,
        ..berserk.clone()
    };
    let mut powered = unit(10.0);
    powered.regen_powerups = [5.0, 5.0];
    assert_eq!(regeneration_rate(&hero, &powered), 60.0);
    let not_hero = Master {
        hero_flag: 0x84,
        ..berserk.clone()
    };
    assert_eq!(regeneration_rate(&not_hero, &powered), 30.0);
    // a gate of Aftermath's: -1000 a minute takes 1 hit point per 0.06 s, at most what it has
    let gate = Master {
        hit_points: 1650,
        regeneration: -1000.0,
        kind: 80,
        ..Master::default()
    };
    let mut g = unit(1650.0);
    g.regenerate(&gate, 0.104);
    assert_eq!(g.life.hp, 1649.0);
    let mut low = unit(1.0);
    low.regenerate(&gate, 1.0);
    assert_eq!(low.life.hp, 0.0);
    // below 1 hit point or out of state 2: nothing
    let mut dead = unit(0.5);
    dead.regenerate(&gate, 1.0);
    assert_eq!((dead.life.hp, dead.regen_timer), (0.5, 0.0));
}

/// `TRIBE_Combat_Object::update`'s hit point powerup check: a powerup above its room comes off
/// the hit points, at least 1 left (`MAXSS`), and takes the room; at most the room, nothing.
#[test]
fn a_hit_point_powerup_above_its_room_comes_off_the_hit_points() {
    let with = |hp, up, room| Unit {
        life: Lifecycle {
            hp,
            ..Lifecycle::default()
        },
        hp_powerups: [up, room],
        ..Unit::default()
    };
    let mut u = with(100.0, 10.0, 4.0);
    u.clamp_hp_powerup();
    assert_eq!((u.life.hp, u.hp_powerups), (94.0, [4.0, 4.0]));
    let mut low = with(3.0, 10.0, 4.0);
    low.clamp_hp_powerup();
    assert_eq!((low.life.hp, low.hp_powerups), (1.0, [4.0, 4.0]));
    let mut nan = with(f32::NAN, 10.0, 4.0);
    nan.clamp_hp_powerup();
    assert_eq!(nan.life.hp, 1.0);
    let mut kept = with(100.0, 4.0, 4.0);
    kept.clamp_hp_powerup();
    assert_eq!((kept.life.hp, kept.hp_powerups), (100.0, [4.0, 4.0]));
}

/// `RGE_Combat_Object::update`: the attack timer runs down by dt times the object's rate
/// (`+0x87c`), which a terrain look sets ([`crate::combat::abilities`]).
#[test]
fn the_attack_timer_runs_down_at_the_object_s_rate() {
    let unit = Unit {
        id: 1,
        attack_timer: 1.0,
        abilities: crate::combat::abilities::Abilities {
            attack_rate: 0.5,
            ..crate::combat::abilities::Abilities::default()
        },
        ..Unit::default()
    };
    let mut t = World::new(
        0,
        1,
        0,
        vec![0],
        vec![Player {
            objects: vec![unit].into(),
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
    use crate::objects::sprite::{ActiveSprite, Graphic};
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
        building: Some(crate::objects::masters::BuildingMaster {
            destruction_sprite: 389,
            construction_sprite: -1,
            ..crate::objects::masters::BuildingMaster::default()
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
            record: crate::map::spatial::Obstruction { kind: 2, class: 0 },
            snow: false,
            sprite_override: false,
            started: false,
        }),
        sprite: Some(ActiveSprite::at_rest(g(207), 0)),
        pasture: -1,
        my_pasture: -1,
        ..Unit::default()
    };
    let mut t = World::new(
        0,
        1,
        0,
        vec![0],
        vec![Player {
            objects: vec![foundation].into(),
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
        issuer: player_id(1),
        sub: 6,
        player: 1,
        a: 4,
        speed: 0.0,
        value: 0,
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
        body: crate::objects::layers::Body::new(
            [10.5, 10.5, 0.0],
            None,
            Some(crate::gathering::hunt::tests::walker()),
        ),
        my_pasture: -1,
        life: Lifecycle {
            master_id: 83,
            ..Lifecycle::default()
        },
        ..Unit::default()
    }
}

/// A unit's own Attack on `target` (second target `target2`) in `state`, without a `MoveTo`.
fn own_attack(target: i32, target2: i32, state: u8) -> crate::combat::attack::Attack {
    crate::combat::attack::Attack {
        target,
        target2,
        state,
        timer: 0.0,
        range: 0.0,
        min_range: 0.0,
        point: [-1.0; 3],
        seen: [0.0; 3],
        goto: [-1.0; 3],
        move_to: None,
        move_wait: crate::movement::move_to::MoveWait::default(),
        pending: false,
        swing: 0.0,
        rest: 0.0,
        arms: crate::combat::attack::Arms {
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
    let (masters, graphics) = scout_sprites();
    let _ = u.ai_move_with(
        m,
        (class, 1),
        (&mut releases, &mut leaves, by),
        (Some(&masters), &graphics, &mut rng),
    )?;
    assert!(releases.iter().all(|&(_, b)| b == by));
    Ok(releases.into_iter().map(|(t, _)| t).collect())
}

/// The master 83 of [`mover_unit`] with its running (13154), walking (13153) and standing
/// (13151) sprites, and their table.
fn scout_sprites() -> (Masters, std::collections::BTreeMap<i16, Graphic>) {
    let master = Master {
        id: 83,
        run_sprite: 13154,
        move_sprite: 13153,
        standing_sprite: 13151,
        ..Master::default()
    };
    let graphics = [13154, 13153, 13151]
        .into_iter()
        .map(|id| {
            let g = Graphic {
                id,
                frames: 10,
                facets: 16,
                speed_multiplier: 1.0,
                frame_duration: 0.1,
                replay_delay: 0.0,
                flags: 1,
            };
            (id, g)
        })
        .collect();
    (Masters::new([master]), graphics)
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
    u.actions
        .set_attack(Some(own_attack(9149, 9150, crate::combat::attack::DONE)));
    assert_eq!(ai_move_of(&mut u, forced(false), 47), Ok(vec![9149, 9150]));
    assert_eq!(u.actions.attack().copied(), None);
    let mt = u.actions.move_to().copied().expect("the AI's MoveTo");
    assert_eq!(
        (
            mt.state,
            mt.goal,
            mt.target,
            mt.range,
            mt.sprite.map(|g| g.id)
        ),
        (MoveTo::Start, Some(AI_POINT), -1, 0.5, Some(13154))
    );
    // walking: the master's walking sprite
    let mut u = mover_unit(123);
    assert_eq!(ai_move_of(&mut u, forced(true), 47), Ok(vec![]));
    let walked = u.actions.move_to().copied().unwrap().sprite;
    assert_eq!(walked.map(|g| g.id), Some(13153));
}

/// The Gather's slot 0x40 (`0x140a8b740`) takes a move to a point: the target goes (slot 0x68
/// with none, the caller's release), the point is the move's and `set_state(15)` walks there; the
/// unit's list keeps the Gather (425780724's villager in update 5526, its Gather idle).
#[test]
fn an_ai_move_hands_the_point_to_the_gather() {
    let task = crate::gathering::hunt::HuntTask {
        id: 2,
        range: 0.25,
        resource: 17,
        sprites: [-1; 4],
        work_value: 1.0,
        productivity: -1,
        kind4: -1,
    };
    let bush = crate::gathering::gather::Resource {
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
    let rate = crate::gathering::gather::StopRate {
        work_rate: 1.0,
        capacity: 10.0,
        productivity: 1.0,
    };
    let mut gather = crate::gathering::gather::GatherAction::new(task, &bush, false, rate).unwrap();
    gather.state = crate::gathering::gather::IDLE;
    let mut u = mover_unit(31139);
    u.actions.set_gather(Some(gather));
    assert_eq!(ai_move_of(&mut u, forced(false), 4), Ok(vec![28222]));
    let g = u.actions.gather().copied().expect("the Gather stays");
    assert_eq!(
        (g.state, g.target, g.point, g.walk_to),
        (
            crate::gathering::gather::MOVED,
            -1,
            AI_POINT,
            Some(AI_POINT)
        )
    );
    assert_eq!(g.move_to, Some(MoveTo::Start));
    assert_eq!(u.actions.move_to().copied(), None);
}

/// The Hunt's slot 0x40 (`FUN_140ac52c0`) takes a point unless its task has a work range: its
/// target goes, `+0x68` is −1 and `set_state(15)` walks to the point (431193960's villager in
/// update 1465, its Hunt idle). A task with a range answers 0: the `MoveTo` replaces the list and
/// the Hunt's teardown lets its target and its drop site go.
#[test]
fn an_ai_move_hands_the_point_to_a_hunt_without_a_work_range() {
    let mut u = mover_unit(47);
    let mut hunt = crate::gathering::hunt::tests::hunting(9149);
    hunt.state = crate::gathering::hunt::IDLE;
    hunt.carry_to = 9150;
    u.actions.set_hunt(Some(hunt));
    assert_eq!(ai_move_of(&mut u, forced(false), 4), Ok(vec![9149]));
    let h = u.actions.hunt().copied().expect("the Hunt stays");
    assert_eq!(
        (h.state, h.target, h.carry_to, h.point, h.target_class),
        (crate::gathering::hunt::WALK_POINT, -1, 9150, AI_POINT, -1)
    );
    assert_eq!(u.actions.move_to().copied(), None);
    let mut u = mover_unit(47);
    hunt.task.range = 1.0;
    u.actions.set_hunt(Some(hunt));
    assert_eq!(ai_move_of(&mut u, forced(false), 4), Ok(vec![9149, 9150]));
    assert_eq!(u.actions.hunt().copied(), None);
    assert_eq!(u.actions.move_to().map(|m| m.goal), Some(Some(AI_POINT)));
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
    let old = MoveToAction::to_point(AI_POINT, 0.0, None, None);
    let mut u = mover_unit(5);
    u.actions.set_move_to(Some(old));
    assert_eq!(ai_move_of(&mut u, unforced, 47), Ok(vec![]));
    assert_eq!(u.actions.move_to().copied(), Some(old));
    let to_object = MoveToAction {
        goal: None,
        target: 9149,
        ..old
    };
    u.actions.set_move_to(Some(to_object));
    u.body.mover_mut().unwrap().move_to_point = Some(AI_POINT);
    assert_eq!(ai_move_of(&mut u, unforced, 47), Ok(vec![]));
    assert_eq!(u.actions.move_to().copied(), Some(to_object));
    assert_eq!(ai_move_of(&mut u, forced(false), 47), Ok(vec![9149]));
    assert_eq!(
        u.actions.move_to().map(|m| (m.target, m.range)),
        Some((-1, 0.5))
    );
    u.actions.set_move_to(Some(old));
    let elsewhere = crate::unit_ai::AiMove {
        point: [1.0, 2.0, 0.0],
        ..unforced
    };
    assert_eq!(ai_move_of(&mut u, elsewhere, 47), Ok(vec![]));
    assert_eq!(
        u.actions.move_to().copied().unwrap().goal,
        Some([1.0, 2.0, 0.0])
    );
}

/// `FUN_141216c50`: an unpacked trebuchet (class 54) packs instead (not modelled); a packed one
/// (class 51) moves as any unit. The base slot 0x40 of an Explore and of a wait for a foundation
/// takes nothing; an action the slice does not hold stops it.
#[test]
fn an_ai_move_of_a_trebuchet_packs_and_the_base_slot_takes_nothing() {
    let mut u = mover_unit(7);
    assert!(ai_move_of(&mut u, forced(false), 54).is_err());
    assert_eq!(u.actions.move_to().copied(), None);
    assert_eq!(ai_move_of(&mut u, forced(false), 51), Ok(vec![]));
    assert_eq!(u.actions.move_to().map(|m| m.goal), Some(Some(AI_POINT)));
    let mut u = mover_unit(7);
    u.actions
        .set_wait_solidify(Some(crate::construction::wait_solidify::WaitSolidify::new(
            crate::objects::masters::Task::default(),
            10028,
        )));
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
    let m = u.body.mover_mut().unwrap();
    m.stats.speed = 0.0;
    m.stats.stand_speed = 0.0;
    u.body.set_anim_speed(1.0);
    assert_eq!(ai_move_of(&mut u, forced(false), 47), Ok(vec![]));
    assert_eq!(u.actions.move_to().map(|m| m.state), Some(MoveTo::Failed));
    assert_eq!(u.body.anim_speed(), Some(0.0));
}

/// The motion step gave up on a unit's own `MoveTo` (`LAB_140aa320c`): any unit but a human's
/// villager fails it (`set_state(13)`), whose `set_state` (`0x140aa29e0`, not quiet) shows the
/// standing sprite (slot 0x68 with slot 0x798's): the mover's speed becomes the standing one
/// (426049848's deer 14064 in update 2077, which then evaded at 0.737, not 1.4) and `+0x7b8`
/// clears. A human's villager waits once (`set_state(2)`), which shows nothing.
#[test]
fn an_own_move_to_that_gives_up_fails_standing_unless_a_villager_waits() {
    let standing = Graphic {
        id: 764,
        frames: 10,
        facets: 8,
        speed_multiplier: 1.0,
        frame_duration: 0.1,
        replay_delay: 0.0,
        flags: 1,
    };
    let walking = Graphic {
        id: 765,
        ..standing
    };
    let graphics = std::collections::BTreeMap::from([(764, standing), (765, walking)]);
    let masters = Masters::new([Master {
        id: 65,
        standing_sprite: 764,
        ..Master::default()
    }]);
    let mut rng = Well512::default();
    let deer = |rng: &mut Well512| {
        let mut u = mover_unit(14064);
        u.life.master_id = 65;
        let m = u.body.mover_mut().unwrap();
        // the deer's speed 0.737 with its standing sprite's factor 1, its walking one's 1.9
        m.stats.base = 0.737;
        m.stats.speed = 1.4003;
        m.stats.walk_speed = 1.4003;
        m.stats.stand_speed = 0.737;
        u.sprite = Some(crate::objects::sprite::ActiveSprite::start(walking, rng));
        u.body.set_anim_speed(1.0);
        u.actions.set_move_to(Some(MoveToAction::to_point(
            [119.5, 172.5, 0.0],
            0.0,
            None,
            None,
        )));
        u
    };
    let mut u = deer(&mut rng);
    u.own_move_gave_up(false, 0, (Some(&masters), &graphics, &mut rng))
        .unwrap();
    assert_eq!(u.actions.move_to().map(|m| m.state), Some(MoveTo::Failed));
    assert_eq!(u.body.mover().map(|m| m.stats.speed), Some(0.737));
    assert_eq!(u.sprite.map(|s| s.graphic.id), Some(764));
    assert_eq!(u.body.anim_speed(), Some(0.0));
    let mut v = deer(&mut rng);
    v.own_move_gave_up(true, 0, (Some(&masters), &graphics, &mut rng))
        .unwrap();
    assert_eq!(v.actions.move_to().map(|m| m.state), Some(MoveTo::Waiting));
    assert_eq!(v.body.mover().map(|m| m.stats.speed), Some(1.4003));
    assert_eq!(v.sprite.map(|s| s.graphic.id), Some(765));
}

/// The unit AI's attack command ([`crate::unit_ai::UnitSide::attack`]) of the scout 123 (master
/// 448, class 47, 426049998's) on Gaia's wolf 50 half a tile east, its attack timer `timer`, inside
/// `inside` if any: the scout and what the command answered and took.
fn scout_attack(timer: f32, inside: Option<i32>) -> (Unit, crate::unit_ai::Attacked, Vec<i32>) {
    scout_attack_on(timer, inside, None, false)
}

/// [`scout_attack`] of a scout whose own Attack is `own`, the command `force`d or not.
fn scout_attack_on(
    timer: f32,
    inside: Option<i32>,
    own: Option<crate::combat::attack::Attack>,
    force: bool,
) -> (Unit, crate::unit_ai::Attacked, Vec<i32>) {
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
            body: crate::objects::layers::Body::at([10.5, 10.0, 0.0]),
            life: Lifecycle {
                master_id: 126,
                ..Lifecycle::default()
            },
            ..Unit::default()
        }]
        .into(),
        ..Player::default()
    };
    let mut players = [gaia];
    let mut rng = Well512::default();
    let mut walker = crate::gathering::hunt::tests::walker();
    walker.stats.speed = 1.2;
    let mut unit = Unit {
        id: 123,
        body: crate::objects::layers::Body::new([10.0, 10.0, 0.0], None, Some(walker)),
        sprite: Some(crate::objects::sprite::ActiveSprite::start(
            standing, &mut rng,
        )),
        attack_timer: timer,
        actions: ActionList::default().with_attack(own),
        life: Lifecycle {
            master_id: 448,
            inside,
            ..Lifecycle::default()
        },
        ..Unit::default()
    };
    let ai = UnitAi {
        stance: 2,
        current_order: None,
        ai_type: 13,
        ..UnitAi::default()
    };
    let masters = Masters::new([scout]);
    let every = [Masters::new([wolf]), masters];
    let (mut pathing, mut zones) = (Pathing::default(), crate::pathing::zones::Zones::default());
    let groups = crate::groups::UnitGroups::default();
    let (mut object, mut sprite) = (Well512::default(), Well512::default());
    let (mut takes, mut releases, mut leaves, mut waits) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut adds = Vec::new();
    let (mut none, mut asleep) = (ObjectList::default(), ObjectList::default());
    let mut view = Others::new((1, 123), (&mut none, &mut asleep), (&mut players, &mut []));
    let attacked = {
        let mut side = crate::unit_ai::world_view::UnitView {
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
            others: &mut view,
            new_owner: None,
            zones: &mut zones,
            every: &every,
            graphics: &graphics,
            lets: Lets {
                player: 1,
                relations: &[0, 0],
                masters: &every,
                lines: &crate::objects::lines::UnitLines::default(),
                tracker: &mut crate::players::tracker::Tracker::default(),
                takes: &mut takes,
                releases: &mut releases,
                builder_leaves: &mut leaves,
            },
            starts: &mut waits,
            attribute_adds: &mut adds,
            relations: &[0, 0],
            attributes: &[],
            civ: 0,
            ai_settings: AiSettings::default(),
            drop_sites: &[],
            mode_680: 0,
            carry_rule: false,
        };
        side.attack(50, force, &ai).unwrap()
    };
    let _ = takes;
    // the takes reach Gaia's objects through the view: each counts a worker
    let taken = players[0].objects.iter();
    let takes = taken.flat_map(|u| std::iter::repeat_n(u.id, u.workers as usize));
    (unit, attacked, takes.collect())
}

/// The attack command of unit 123 of master `who` (in a table with `also`) on Gaia's wolf 50, as
/// [`scout_attack_on`], the unit in its owner's done tracker list of its master: the unit, the
/// answer, the attribute changes and its owner's tracker after it.
fn attack_by(
    who: Master,
    also: Vec<Master>,
) -> (
    Unit,
    crate::unit_ai::Attacked,
    Vec<(i16, f32)>,
    crate::players::tracker::Tracker,
) {
    use crate::unit_ai::UnitSide;
    let (timer, inside, own, force) = (0.0, None, None, true);
    let attacker_master = who.id;
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
            body: crate::objects::layers::Body::at([10.5, 10.0, 0.0]),
            life: Lifecycle {
                master_id: 126,
                ..Lifecycle::default()
            },
            ..Unit::default()
        }]
        .into(),
        ..Player::default()
    };
    let mut players = [gaia];
    let mut rng = Well512::default();
    let mut walker = crate::gathering::hunt::tests::walker();
    walker.stats.speed = 1.2;
    let mut unit = Unit {
        id: 123,
        body: crate::objects::layers::Body::new([10.0, 10.0, 0.0], None, Some(walker)),
        sprite: Some(crate::objects::sprite::ActiveSprite::start(
            standing, &mut rng,
        )),
        attack_timer: timer,
        actions: ActionList::default().with_attack(own),
        life: Lifecycle {
            master_id: attacker_master,
            inside,
            ..Lifecycle::default()
        },
        ..Unit::default()
    };
    let ai = UnitAi {
        stance: 2,
        current_order: None,
        ai_type: 13,
        ..UnitAi::default()
    };
    let _ = scout;
    let masters = Masters::new(std::iter::once(who).chain(also));
    let every = [Masters::new([wolf]), masters];
    let (mut pathing, mut zones) = (Pathing::default(), crate::pathing::zones::Zones::default());
    let groups = crate::groups::UnitGroups::default();
    let (mut object, mut sprite) = (Well512::default(), Well512::default());
    let (mut takes, mut releases, mut leaves, mut waits) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut adds = Vec::new();
    let (mut none, mut asleep) = (ObjectList::default(), ObjectList::default());
    let lines = crate::objects::lines::UnitLines::default();
    let mut tracker = crate::players::tracker::Tracker::default();
    tracker.add(
        123,
        crate::players::tracker::key(every.get(1), &lines, attacker_master),
    );
    let mut view = Others::new((1, 123), (&mut none, &mut asleep), (&mut players, &mut []));
    let attacked = {
        let mut side = crate::unit_ai::world_view::UnitView {
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
            others: &mut view,
            new_owner: None,
            zones: &mut zones,
            every: &every,
            graphics: &graphics,
            lets: Lets {
                player: 1,
                relations: &[0, 0],
                masters: &every,
                lines: &lines,
                tracker: &mut tracker,
                takes: &mut takes,
                releases: &mut releases,
                builder_leaves: &mut leaves,
            },
            starts: &mut waits,
            attribute_adds: &mut adds,
            relations: &[0, 0],
            attributes: &[],
            civ: 0,
            ai_settings: AiSettings::default(),
            drop_sites: &[],
            mode_680: 0,
            carry_rule: false,
        };
        side.attack(50, force, &ai).unwrap()
    };
    let _ = takes;
    (unit, attacked, adds, tracker)
}

/// A villager's attack command (`FUN_141081a80`): a builder (118, task swap group 1) switches to
/// its player's villager 83 first (slot 0x90 with 0: its storage leaves and the new one's comes,
/// the tracker promotes it), then attacks with the new master.
#[test]
fn a_villager_s_attack_command_switches_it_to_the_villager_first() {
    let villager = |id, storage| Master {
        id,
        kind: 70,
        class: 4,
        task_by_group: 1,
        hit_points: 25,
        standing_sprite: 300,
        attack_sprite: 301,
        weapon_range: 0.5,
        reload: 2.0,
        storage,
        moving: Some(crate::gathering::hunt::tests::walker().stats),
        ..Master::default()
    };
    let (unit, attacked, adds, tracker) = attack_by(
        villager(118, vec![(4, 1.0, 2)]),
        vec![villager(83, vec![(4, 1.0, 2)])],
    );
    assert_eq!(unit.life.master_id, 83);
    assert!(attacked.attacks);
    assert!(unit.actions.attack().is_some_and(|a| a.target == 50));
    assert_eq!(adds, [(4, -1.0), (4, 1.0)]);
    assert_eq!((tracker.list(118), tracker.list(83)), (&[][..], &[123][..]));
}

/// The scout 124 (player 1, master 448, at (10.3, 10), its unit AI `helper`, its own Attack
/// `own`), a helper against Gaia's wolf 50 asked by the scout 123.
fn helper_scout(helper: UnitAi, own: Option<crate::combat::attack::Attack>) -> Unit {
    Unit {
        id: 124,
        body: crate::objects::layers::Body::at([10.3, 10.0, 0.0]),
        actions: ActionList::default().with_attack(own),
        ai: Some(helper),
        life: Lifecycle {
            master_id: 448,
            state: 2,
            ..Lifecycle::default()
        },
        ..Unit::default()
    }
}

/// `AskForHelp`'s world: the scout 123's view (player 1, master 448, at (10, 10)) with the
/// `helper` among its owner's other objects, Gaia's wolf 50 at (10.5, 10), the lists the last
/// search left (`searched`, none before any) and the world's unit groups `groups`; `f` runs on
/// the view.
fn help_view<R>(
    helper: Unit,
    searched: Option<[Vec<crate::vision::registry::Record>; 5]>,
    groups: &crate::groups::UnitGroups,
    f: impl FnOnce(&mut crate::unit_ai::world_view::UnitView<'_, '_>) -> R,
) -> R {
    let scout = Master {
        id: 448,
        kind: 70,
        class: 47,
        weapon_range: 1.0,
        weapons: vec![(4, 3)],
        ..Master::default()
    };
    let wolf = Master {
        id: 126,
        kind: 70,
        class: 10,
        combat_level: 4,
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
    spatial.insert(124, placed(10.3, 47, 1, 448));
    spatial.insert(50, placed(10.5, 10, 0, 126));
    let gaia = Player {
        objects: vec![Unit {
            id: 50,
            body: crate::objects::layers::Body::at([10.5, 10.0, 0.0]),
            life: Lifecycle {
                master_id: 126,
                ..Lifecycle::default()
            },
            ..Unit::default()
        }]
        .into(),
        ..Player::default()
    };
    let mut players = [gaia];
    if let Some(lists) = searched {
        spatial.set_searched(lists);
    }
    let (mut own, mut asleep) = (ObjectList::from(vec![helper]), ObjectList::default());
    let mut unit = Unit {
        id: 123,
        body: crate::objects::layers::Body::at([10.0, 10.0, 0.0]),
        life: Lifecycle {
            master_id: 448,
            ..Lifecycle::default()
        },
        ..Unit::default()
    };
    let masters = Masters::new([scout]);
    let every = [Masters::new([wolf]), masters];
    let (mut pathing, mut zones) = (Pathing::default(), crate::pathing::zones::Zones::default());
    let (mut object, mut sprite) = (Well512::default(), Well512::default());
    let (mut takes, mut releases, mut leaves, mut waits) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut adds = Vec::new();
    let graphics = std::collections::BTreeMap::new();
    let mut view = Others::new((1, 123), (&mut own, &mut asleep), (&mut players, &mut []));
    let mut side = crate::unit_ai::world_view::UnitView {
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
        groups,
        object: &mut object,
        sprite: &mut sprite,
        world_time: 0,
        construction_rate: 1.0,
        others: &mut view,
        new_owner: None,
        zones: &mut zones,
        every: &every,
        graphics: &graphics,
        lets: Lets {
            player: 1,
            relations: &[0, 0],
            masters: &every,
            lines: &crate::objects::lines::UnitLines::default(),
            tracker: &mut crate::players::tracker::Tracker::default(),
            takes: &mut takes,
            releases: &mut releases,
            builder_leaves: &mut leaves,
        },
        starts: &mut waits,
        attribute_adds: &mut adds,
        relations: &[0, 0],
        attributes: &[],
        civ: 0,
        ai_settings: AiSettings::default(),
        drop_sites: &[],
        mode_680: 0,
        carry_rule: false,
    };
    f(&mut side)
}

/// `AskForHelp`'s helper test ([`UnitView::helper_free`]) of [`helper_scout`] against the wolf.
///
/// [`UnitView::helper_free`]: crate::unit_ai::world_view::UnitView::helper_free
fn helper_free(helper: UnitAi, own: Option<crate::combat::attack::Attack>) -> bool {
    helper_free_of(
        1,
        helper_scout(helper, own),
        &crate::groups::UnitGroups::default(),
    )
}

/// [`helper_free`] of the unit `helper` of player `owner` with the world's unit groups `groups`.
fn helper_free_of(owner: usize, helper: Unit, groups: &crate::groups::UnitGroups) -> bool {
    help_view(helper, None, groups, |side| {
        let scout = crate::objects::masters::player_master(side.every, side.player, 448)
            .unwrap()
            .clone();
        let helper = side.others.view().own_iter().next().unwrap();
        side.helper_free(owner, helper, &scout, 50).unwrap()
    })
}

/// `AskForHelp` (`FUN_140c42ce0`) asks only an idle helper in no group (no action value, action,
/// order, queued order or target) whose attack check lets it attack the attacker; a helper whose
/// top action's target lies within its range keeps it.
#[test]
fn ask_for_help_asks_only_an_idle_helper_that_may_attack() {
    let idle = UnitAi {
        ai_type: 13,
        current_order: None,
        current_action: None,
        current_target: -1,
        stance: 0,
        ..UnitAi::default()
    };
    assert!(helper_free(idle.clone(), None));
    let busy = [
        UnitAi {
            current_action: Some(ActionKind::Attack),
            ..idle.clone()
        },
        UnitAi {
            current_order: Some(OrderKind::Attack),
            ..idle.clone()
        },
        UnitAi {
            current_target: 50,
            ..idle.clone()
        },
    ];
    for ai in busy {
        assert!(!helper_free(ai, None));
    }
    // an action of its own makes it busy
    assert!(!helper_free(idle, Some(own_attack(50, -1, 4))));
}

/// `AskForHelp`'s helper in a group answers whatever it does while its group patrols, and not
/// while it does not; a patrolling helper whose own Attack's target lies within its range keeps
/// it. A unit without a unit AI never answers; the helper of another player than the asker's
/// answers as one of its own.
#[test]
fn a_helper_in_a_group_answers_only_while_its_group_patrols() {
    let idle = UnitAi {
        ai_type: 13,
        current_order: None,
        current_action: None,
        current_target: -1,
        stance: 0,
        ..UnitAi::default()
    };
    let busy = UnitAi {
        current_order: Some(OrderKind::Attack),
        current_action: Some(ActionKind::Attack),
        ..idle.clone()
    };
    let groups = crate::groups::UnitGroups {
        groups: vec![
            crate::groups::UnitGroup {
                flags: 0x800,
                ..crate::groups::UnitGroup::new(7)
            },
            crate::groups::UnitGroup::new(8),
        ],
        ..crate::groups::UnitGroups::default()
    };
    let in_group = |group, own| {
        let mut u = helper_scout(busy.clone(), own);
        u.group.id = group;
        u
    };
    assert!(helper_free_of(1, in_group(7, None), &groups));
    assert!(!helper_free_of(1, in_group(8, None), &groups));
    assert!(!helper_free_of(
        1,
        in_group(7, Some(own_attack(50, -1, 4))),
        &groups
    ));
    let mut thoughtless = helper_scout(idle.clone(), None);
    thoughtless.ai = None;
    assert!(!helper_free_of(1, thoughtless, &groups));
    assert!(helper_free_of(2, helper_scout(idle, None), &groups));
}

/// `FUN_141081a80` on the target of the unit's own Attack: it answers true; forced, the Attack's
/// slot 0x48 (`FUN_140a841d0`) runs on the target first. In state 1 the scout, which may not move
/// (stance 2, no order), keeps the wolf in its reach: where it saw it becomes its place, the
/// target the same (nothing taken). In state 4 the Attack keeps it as it is; unforced nothing
/// changes either.
#[test]
fn an_attack_command_on_its_own_attack_s_target_asks_that_attack_again() {
    let own = |state: u8| crate::combat::attack::Attack {
        state,
        seen: [-1.0; 3],
        ..own_attack(50, -1, state)
    };
    let (unit, attacked, takes) = scout_attack_on(0.0, None, Some(own(1)), true);
    assert_eq!((attacked.attacks, takes), (true, vec![]));
    let a = unit.actions.attack().copied().expect("the scout's Attack");
    assert_eq!((a.state, a.target, a.seen), (1, 50, [10.5, 10.0, 0.0]));
    let (unit, attacked, _) = scout_attack_on(0.0, None, Some(own(4)), true);
    assert!(attacked.attacks);
    assert_eq!(unit.actions.attack().map(|a| a.seen), Some([-1.0; 3]));
    let (unit, _, _) = scout_attack_on(0.0, None, Some(own(1)), false);
    assert_eq!(unit.actions.attack().map(|a| a.seen), Some([-1.0; 3]));
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
    let attack = unit.actions.attack().copied().expect("the scout's Attack");
    assert_eq!(
        (attack.state, attack.pending, attack.point),
        (crate::combat::attack::SWING, true, [10.5, 10.0, 0.0])
    );
    assert_eq!(unit.sprite.map(|s| s.graphic.id), Some(301));
    let (unit, _, _) = scout_attack(0.5, None);
    let attack = unit.actions.attack().copied().expect("the scout's Attack");
    assert_eq!(
        (attack.state, attack.pending),
        (crate::combat::attack::READY, false)
    );
    assert_eq!(unit.sprite.map(|s| s.graphic.id), Some(300));
    let (unit, attacked, takes) = scout_attack(0.0, Some(9149));
    assert_eq!((attacked.attacks, takes), (true, vec![50]));
    assert_eq!(
        unit.actions.attack().map(|a| a.state),
        Some(crate::combat::attack::FAILED)
    );
    assert_eq!(unit.body.anim_speed(), Some(1.0));
}

/// A house's movement values: it never moves (speed 0).
const fn house_moves() -> crate::movement::MoveStats {
    crate::movement::MoveStats {
        fly_mode: 0,
        speed: 0.0,
        base: 0.0,
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
    let mut t = World::new(
        0,
        29910,
        1,
        vec![0, 0],
        vec![
            Player::default(),
            Player {
                objects: vec![Unit {
                    id: 9175,
                    body: crate::objects::layers::Body::at([30.0, 89.0, 0.0]),
                    life: Lifecycle {
                        class: Class::Animated,
                        state: 2,
                        hp: 25.0,
                        master_id: 83,
                        ..Lifecycle::default()
                    },
                    ai: Some(UnitAi {
                        ai_type: 8,
                        current_order: None,
                        current_action: None,
                        current_target: -1,
                        ..UnitAi::default()
                    }),
                    ..Unit::default()
                }]
                .into(),
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
    t.run(&Command::Build(BuildCommand {
        player: player_id(1),
        master: master_id(70),
        position: Point::new(30.0, 88.0),
        q_id: -1,
        sprite_override: -1,
        builders: unit_list(object_ids([9175])),
        queued: false,
        commits: false,
        frame: 0,
    }));
    assert_eq!(t.players[1].unsupported, None);
    let foundation = t.players[1].objects.last().unwrap();
    assert_eq!(foundation.id, 10028);
    let m = foundation
        .body
        .mover()
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
        let mut t = World::new(
            0,
            29910,
            1,
            vec![0, 0],
            vec![
                Player::default(),
                Player {
                    objects: vec![Unit {
                        id: 9175,
                        body: crate::objects::layers::Body::at([30.0, 89.0, 0.0]),
                        life: Lifecycle {
                            class: Class::Animated,
                            state: 2,
                            hp: 25.0,
                            master_id: 83,
                            ..Lifecycle::default()
                        },
                        ai: Some(UnitAi {
                            ai_type: 8,
                            current_order: None,
                            current_action: None,
                            current_target: -1,
                            ..UnitAi::default()
                        }),
                        ..Unit::default()
                    }]
                    .into(),
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
        t.run(&Command::Build(BuildCommand {
            player: player_id(1),
            master: master_id(70),
            position: Point::new(30.0, 88.0),
            q_id: -1,
            sprite_override: -1,
            builders: unit_list(object_ids([9175])),
            queued: false,
            commits: false,
            frame: 0,
        }));
        assert_eq!(t.players[1].unsupported, None);
        (t, before)
    };
    // sequence type 3: the facing's draw, then the powerup offset's
    let (mut t, mut before) = build(3);
    let facing = before.rand15();
    let _ = before.rand15();
    assert_eq!(t.streams.get(Stream::Object), &before);
    let facets = crate::objects::facing::SpriteFacets {
        count: 5,
        flag_1: true,
    };
    let foundation = t.players[1].objects.last().unwrap();
    let angle = crate::objects::facing::drawn_angle(facing, facets);
    assert_ne!(angle, 0.0);
    assert_eq!(foundation.body.mover().map(|m| m.angle), Some(angle));
    // without sequence flag 2 only the powerup offset's
    let (mut t, mut before) = build(1);
    let _ = before.rand15();
    assert_eq!(t.streams.get(Stream::Object), &before);
    let foundation = t.players[1].objects.last().unwrap();
    assert_eq!(foundation.body.mover().map(|m| m.angle), Some(0.0));
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
fn reseed_world() -> World {
    let task = crate::gathering::hunt::HuntTask {
        id: 2,
        range: 0.25,
        resource: 15,
        sprites: [-1; 4],
        work_value: 1.0,
        productivity: -1,
        kind4: -1,
    };
    let farm = crate::gathering::gather::Resource {
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
    let rate = crate::gathering::gather::StopRate {
        work_rate: 1.0,
        capacity: 10.0,
        productivity: 1.0,
    };
    let gather = crate::gathering::gather::GatherAction {
        state: crate::gathering::gather::RESEED_WALK,
        target: -1,
        worked: true,
        ..crate::gathering::gather::GatherAction::new(task, &farm, false, rate).unwrap()
    };
    let farmer = Unit {
        id: 9175,
        body: crate::objects::layers::Body::at([30.5, 88.5, 0.0]),
        life: Lifecycle {
            class: Class::Animated,
            state: 2,
            hp: 25.0,
            master_id: 259,
            ..Lifecycle::default()
        },
        actions: ActionList::default().with_gather(Some(gather)),
        ai: Some(UnitAi {
            ai_type: 8,
            current_order: None,
            current_action: None,
            current_target: -1,
            ..UnitAi::default()
        }),
        ..Unit::default()
    };
    let mut t = World::new(
        0,
        29910,
        1,
        vec![0, 0],
        vec![
            Player::default(),
            Player {
                objects: vec![farmer].into(),
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
            foundation.position()
        ),
        (10028, 50, [30.5, 88.5, 0.0])
    );
    let farmer = &p.objects[0];
    assert_eq!(farmer.life.master_id, 118);
    assert_eq!(farmer.actions.build().map(|b| b.target), Some(10028));
    assert!(farmer.actions.gather().is_none());
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
        p.objects[0].actions.gather().map(|g| g.state),
        Some(crate::gathering::gather::IDLE)
    );
    let mut t = reseed_world();
    t.players[1].auto_farm_queue = true;
    t.players[1].resources[1] = 59.0;
    t.players[1].computer = true;
    t.reseed_after_walk(1, 0, true).unwrap();
    let p = &t.players[1];
    assert_eq!((p.objects.len(), p.resources[1]), (1, 59.0));
    assert_eq!(
        p.objects[0].actions.gather().map(|g| g.state),
        Some(crate::gathering::gather::DONE)
    );
}

/// [`reseed_world`] with one farm queued, `setup` run on it, then the reseed itself
/// ([`World::reseed`]): its answer and the count of player 1's objects after it.
fn reseeded(setup: impl FnOnce(&mut World)) -> (Result<bool, GenieError>, usize) {
    let mut t = reseed_world();
    t.players[1].farm_queue = 1;
    setup(&mut t);
    let made = t.reseed(1, 0);
    (made, t.players[1].objects.len())
}

/// `FUN_140a8aa60` makes nothing for a gatherer without a Gather or without a farm spot, for a
/// villager with only fish traps queued, for a fishing ship (class 21) with only farms queued and
/// for a farm the player cannot make (its master unavailable).
#[test]
fn a_reseed_without_a_farm_to_make_makes_nothing() {
    let none: [fn(&mut World); 5] = [
        |t| t.players[1].objects[0].actions.set_gather(None),
        |t| {
            let g = t.players[1].objects[0].actions.gather_mut().unwrap();
            g.farm = [-1.0, -1.0];
        },
        |t| (t.players[1].farm_queue, t.players[1].fish_trap_queue) = (0, 1),
        |t| t.masters[1].get_mut(259).unwrap().class = 21,
        |t| t.masters[1].get_mut(50).unwrap().available = false,
    ];
    for setup in none {
        assert_eq!(reseeded(setup), (Ok(false), 1));
    }
}

/// A fishing ship (class 21) reseeds a fish trap (199) from the dock's queue (one less), with
/// the automatic fish trap queue paying it (60 wood) instead.
#[test]
fn a_fishing_ship_reseeds_a_fish_trap() {
    let ship = |t: &mut World| {
        let trap = Master {
            id: crate::gathering::farm_queue::FISH_TRAP,
            base_id: crate::gathering::farm_queue::FISH_TRAP,
            ..t.masters[1].get(50).unwrap().clone()
        };
        let ms = &mut t.masters[1];
        // a ship builds as itself: its own Build task, no group
        let tasks = ms.get(118).unwrap().tasks.clone();
        let ship = ms.get_mut(259).unwrap();
        (ship.class, ship.task_by_group, ship.tasks) = (21, 0, tasks);
        *ms = Masters::new(ms.iter().cloned().chain([trap]));
        t.players[1].farm_queue = 0;
    };
    let mut t = reseed_world();
    ship(&mut t);
    t.players[1].fish_trap_queue = 2;
    assert_eq!(t.reseed(1, 0), Ok(true));
    let p = &t.players[1];
    assert_eq!((p.resources[1], p.fish_trap_queue), (100.0, 1));
    let trap = p.objects.last().unwrap();
    assert_eq!(
        (trap.life.master_id, trap.position()),
        (crate::gathering::farm_queue::FISH_TRAP, [30.5, 88.5, 0.0])
    );
    let mut t = reseed_world();
    ship(&mut t);
    t.players[1].auto_fishtrap_queue = true;
    assert_eq!(t.reseed(1, 0), Ok(true));
    let p = &t.players[1];
    assert_eq!((p.resources[1], p.fish_trap_queue), (40.0, 0));
}

/// The farm's placement test passes over the gather's old target (the spent farm 12557 on the
/// farm spot, a building): the new foundation goes over it. The same farm when it is not the
/// target fails the test, and nothing is made.
#[test]
fn a_reseed_s_placement_passes_over_the_old_farm() {
    let old_farm = |target: i32| {
        move |t: &mut World| {
            let farm = Unit {
                id: 12557,
                body: crate::objects::layers::Body::at([30.5, 88.5, 0.0]),
                life: Lifecycle {
                    class: Class::Animated,
                    state: 2,
                    hp: 480.0,
                    master_id: 50,
                    ..Lifecycle::default()
                },
                ..Unit::default()
            };
            t.players[1].objects.push(farm);
            open_map(t);
            let at = Placed {
                position: [30.5, 88.5, 0.0],
                radius: [1.5, 1.5],
                class: 49,
                player: 1,
                master: 50,
                obstruction_type: 2,
                buildable_on: 0,
                tall: true,
                speed: 0.0,
                unknown: None,
                obstruction: None,
            };
            t.spatial.insert(12557, at);
            t.players[1].objects[0].actions.gather_mut().unwrap().target = target;
        }
    };
    assert_eq!(reseeded(old_farm(12557)), (Ok(true), 3));
    assert_eq!(reseeded(old_farm(-1)), (Ok(false), 2));
}

/// A committing BUILD (`+0x1f` 0) to a builder whose unit AI holds an order not ready
/// (`FUN_140c4dfa0`): the build order joins its queue, then the readiness pass with 0
/// (`FUN_140c42b60` at 0x140e7b252) makes every queued order ready, the ready ones kept, and
/// the unit is not stopped.
#[test]
fn a_committing_build_or_wall_to_a_busy_builder_readies_its_queue() {
    let unready = crate::unit_ai::QueuedOrder {
        order: OrderKind::Move,
        ready: false,
        ..crate::unit_ai::QueuedOrder::default()
    };
    let mut t = World::new(
        0,
        29910,
        1,
        vec![0, 0],
        vec![
            Player::default(),
            Player {
                objects: vec![Unit {
                    id: 9175,
                    body: crate::objects::layers::Body::at([30.0, 89.0, 0.0]),
                    life: Lifecycle {
                        class: Class::Animated,
                        state: 2,
                        hp: 25.0,
                        master_id: 83,
                        ..Lifecycle::default()
                    },
                    ai: Some(UnitAi {
                        ai_type: 8,
                        current_order: Some(OrderKind::Move),
                        current_action: None,
                        current_target: -1,
                        orders: vec![unready],
                        ..UnitAi::default()
                    }),
                    ..Unit::default()
                }]
                .into(),
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
    t.run(&Command::Build(BuildCommand {
        player: player_id(1),
        master: master_id(70),
        position: Point::new(30.0, 88.0),
        q_id: -1,
        sprite_override: -1,
        builders: unit_list(object_ids([9175])),
        queued: false,
        commits: true,
        frame: 0,
    }));
    assert_eq!(t.players[1].unsupported, None);
    let ai = t.players[1].objects[0].ai.as_ref().unwrap();
    let queue: Vec<_> = ai.orders.iter().map(|o| (o.order, o.ready)).collect();
    assert_eq!(queue, [(OrderKind::Move, true), (OrderKind::Build, true)]);
    assert_eq!(ai.current_order, Some(OrderKind::Move));
    // BUILD_WALL's line of one tile the same (0x140e7be4b)
    wall.run(&Command::BuildWall(BuildWallCommand {
        player: player_id(1),
        master: master_id(70),
        from: [30, 88],
        to: [30, 88],
        q_id: -1,
        builders: unit_list(object_ids([9175])),
        queued: false,
        commits: true,
    }));
    assert_eq!(wall.players[1].unsupported, None);
    let ai = wall.players[1].objects[0].ai.as_ref().unwrap();
    let queue: Vec<_> = ai.orders.iter().map(|o| (o.order, o.ready)).collect();
    assert_eq!(queue, [(OrderKind::Move, true), (OrderKind::Build, true)]);
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
    t.tick_with(
        104,
        &[Command::Unmodelled(UnmodelledCommand {
            player: player_id(0),
            packet: Vec::new(),
            why,
        })],
    );
    let (_, _, stopped) = t.players[0].unsupported.clone().unwrap();
    assert_eq!(
        stopped.context().collect::<Vec<_>>(),
        [("command", "Unmodelled")]
    );
    let task = GenieError::unsupported("a default task the slice does not model");
    let mut t = World::new(0, 1, 0, vec![0], vec![player(vec![])]);
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

/// `FUN_1411671e0`'s planner results: a path to a drop site or into its range counts (0 and 1),
/// a partial one never (2); one the planner does not reach (3 and up) counts only for a fishing
/// ship seeking by path whose footprint gap to it is within 100 squared (429521544's p3 in
/// update 5601).
#[test]
fn a_fishing_ship_counts_a_near_drop_site_its_path_does_not_reach() {
    use crate::gathering::drop_sites::counted;
    assert!(counted(0, true, false, || false));
    assert!(counted(1, false, false, || false));
    assert!(!counted(2, true, true, || true));
    assert!(!counted(3, true, false, || true));
    assert!(counted(3, true, true, || true));
    assert!(!counted(3, true, true, || false));
    assert!(!counted(3, false, true, || true));
    assert!(counted(4, true, true, || true));
}

/// `TRIBE_Building_Object::update`'s farm stage (`0x1411ca2ef`): a farm lays 29 to 31 by its
/// build points over its build time (times 3, at most 2); a rice farm or a pasture on mangrove
/// or rice 65 to 67, elsewhere its foundation terrain plus 2 (63 and 117: 65.., 119..); another
/// master none (427445417's p4 in update 16981, 427557935's p4 in 22813).
#[test]
fn a_farm_foundation_lays_its_stage_terrain() {
    use crate::construction::building_update::farm_stage_terrain;
    assert_eq!(farm_stage_terrain(50, (false, 7), 0.0, 15), Some(29));
    assert_eq!(farm_stage_terrain(50, (false, 7), 7.5, 15), Some(30));
    assert_eq!(farm_stage_terrain(50, (false, 7), 15.0, 15), Some(31));
    assert_eq!(farm_stage_terrain(1187, (true, 63), 5.0, 15), Some(66));
    assert_eq!(farm_stage_terrain(1187, (false, 63), 0.0, 15), Some(65));
    assert_eq!(farm_stage_terrain(1889, (false, 117), 15.0, 15), Some(121));
    assert_eq!(farm_stage_terrain(1889, (false, -1), 0.0, 15), Some(1));
    assert_eq!(farm_stage_terrain(70, (false, -1), 0.0, 15), None);
}

/// `RGE_Moving_Object::update`'s count-down of the last human order time (`+0x650`, at most
/// `FUN_140fe0d10`'s 5000, by the world's dt `+0x7c`, not below 0) is the world's view at once:
/// the unit's own walk later in the update ranks a met unit by the counted-down time
/// (431096717's goat 12959 in update 78: 2001 against villager 16025's 2034, so it does not
/// pause it).
#[test]
fn a_unit_s_human_order_count_down_is_the_walk_s_view_at_once() {
    let mut t = enter_world(vec![], 5);
    let mut counted = |before: u32| {
        let v = &mut t.players[1].objects[0];
        v.last_human_order = before;
        let id = v.id;
        t.spatial.set_gait(id, v.gait());
        v.count_down_human_order(33, &mut t.spatial);
        let seen = t.spatial.gait(id).map(|g| g.human_order);
        (t.players[1].objects[0].last_human_order, seen)
    };
    assert_eq!(counted(2034), (2001, Some(2001)));
    assert_eq!(counted(5000), (4967, Some(4967)));
    assert_eq!(counted(20), (0, Some(0)));
    assert_eq!(counted(5001), (5001, Some(5001)));
}

/// `RGE_Action_Object::update` runs the actions of a unit not inside another, else its inside
/// update (`FUN_140a98790`), never both: livestock whose Artifact's Enter got it in keeps the
/// Artifact to its next update, whose inside update deletes it (429705045's sheep 10555 and
/// 10556 entering the transport 10579 in update 137, the Artifact gone after 138); an animal
/// inside at the update's start runs no Artifact step.
#[test]
fn livestock_got_in_by_its_artifact_s_enter_keeps_the_artifact_to_its_next_update() {
    let mut t = livestock_enter_world();
    // the sheep out of reach of the house, standing, so that its Enter walks
    let place = |t: &mut World, at: [f32; 3]| {
        let v = &mut t.players[1].objects[0];
        v.place_for_test(at);
        let m = v.body.mover_mut().unwrap();
        m.moved_at = at;
        m.velocity = Some(crate::movement::Velocity {
            velocity: [0.0; 3],
            acceleration: [0.0; 3],
        });
        t.spatial.set_position(9175, at);
    };
    place(&mut t, [30.0, 91.0, 0.0]);
    assert_eq!(t.enter_command(1, 0, 10028, false), Ok(true));
    assert_eq!(t.players[1].objects[0].life.inside, None);
    // the first update runs no Artifact (its first update is in the second)
    t.tick(50);
    // its walk arrived at the house's edge: the next update of the Enter gets it in
    place(&mut t, [30.0, 89.1, 0.0]);
    let art = t.players[1].objects[0].actions.artifact_mut().unwrap();
    art.enter.as_mut().unwrap().move_to = Some(MoveTo::Arrived);
    t.tick(50);
    let v = &t.players[1].objects[0];
    assert_eq!(v.life.inside, Some(10028));
    assert_eq!(
        v.top_action_kind(),
        Some(crate::ownership::artifact::ARTIFACT_TYPE)
    );
    // inside at the next update's start: no Artifact step, the inside update deletes it
    let before = v.actions.artifact().map(|a| (a.state, a.timer));
    t.tick(50);
    let v = &t.players[1].objects[0];
    assert_eq!(
        (v.actions.artifact().copied(), v.top_action_kind()),
        (None, None)
    );
    assert_eq!(t.players[1].freezes, vec![]);
    assert!(before.is_some());
}

/// `RGE_Action_Object::update` with an empty list makes the default task and does no inside
/// update in the same update: livestock inside another object gets its Artifact in one update
/// and loses it in the next, by turns (431108946's sheep 11628 in the transport 11638 from update
/// 61: 107 in state 6 after 63, 65, …, none after 62, 64, …).
#[test]
fn livestock_inside_gets_its_default_artifact_and_loses_it_by_turns() {
    let mut t = livestock_enter_world();
    let v = &mut t.players[1].objects[0];
    v.default_artifact = true;
    v.actions.set_artifact(None);
    v.life.inside = Some(10028);
    t.spatial.go_inside(9175, [30.0, 88.0, 0.0]);
    let mut kinds = Vec::new();
    for _ in 0..4 {
        t.tick(50);
        let v = &t.players[1].objects[0];
        kinds.push(v.actions.artifact().map(|a| a.state));
    }
    assert_eq!(kinds, [Some(6), None, Some(6), None]);
    assert_eq!(t.players[1].freezes, vec![]);
}

/// `TRIBE_Command::do_command`'s case 0x66 checks BUILD's builders before its handler
/// ([`World::command_check`]): another player's becomes −1, and a human's BUILD left without one
/// is refused, placing and paying nothing (429712917's player 8 in update 96, its BUILD naming
/// player 5's villagers 9618 and 9620). With one of its own the foundation goes down, and only
/// that one builds.
#[test]
fn a_build_of_another_player_s_builders_is_refused_and_theirs_are_left_out() {
    let mut t = build_world();
    t.masters[0] = builder_masters();
    let theirs = t.players[1].objects.remove(1);
    t.players[0].objects.push(theirs);
    let build = |builders: Vec<i32>| {
        Command::Build(BuildCommand {
            player: player_id(1),
            master: master_id(70),
            position: Point::new(30.0, 88.0),
            q_id: -1,
            sprite_override: -1,
            builders: unit_list(object_ids(builders)),
            queued: false,
            commits: false,
            frame: 0,
        })
    };
    t.tick_with(50, &[build(vec![9181])]);
    let p = &t.players[1];
    assert_eq!(
        (p.resources, p.objects.len()),
        ([200.0, 100.0, 200.0, 100.0], 1)
    );
    assert_eq!(t.next_object_id, 10028);
    assert_eq!(p.freezes, vec![]);
    t.tick_with(50, &[build(vec![9181, 9175])]);
    let p = &t.players[1];
    assert_eq!(p.resources, [200.0, 75.0, 200.0, 100.0]);
    assert_eq!(p.objects.last().map(|f| f.id), Some(10028));
    assert_eq!(p.objects[0].actions.build().map(|b| b.target), Some(10028));
    assert_eq!(t.players[0].objects[0].actions.build().copied(), None);
}

/// The archer's master (4, 2 s reload, range 4, its missile 363 a tile up), its arrow (363,
/// speed 7, straight) and a deer (65, 5 hit points, 1.5 high) for [`archer_world`].
fn archer_masters(hit_chance: i16, max_volley: u8, spread: f32) -> Masters {
    let archer = Master {
        id: 4,
        kind: 70,
        class: 0,
        hit_points: 30,
        radius: [0.2, 0.2],
        standing_sprite: 300,
        attack_sprite: 301,
        missile: 363,
        weapon_offset: [0.0, 0.0, 1.0],
        weapon_range: 4.0,
        reload: 2.0,
        hit_chance,
        max_volley,
        volley_spread: [spread, spread, 0.0],
        moving: Some(crate::gathering::hunt::tests::walker().stats),
        ..Master::default()
    };
    let arrow = Master {
        id: 363,
        kind: 60,
        class: 27,
        hit_points: 1,
        radius: [0.1, 0.1],
        standing_sprite: -1,
        death_sprite: -1,
        death_spawns: [-1, -1],
        life: Some(MasterData::default()),
        recyclable: true,
        moving: Some(crate::movement::MoveStats {
            speed: 7.0,
            base: 7.0,
            ..crate::gathering::hunt::tests::walker().stats
        }),
        missile_stats: Some(crate::combat::missile::MissileStats {
            speed: 7.0,
            radius: [0.1, 0.1, 0.0],
            ..crate::combat::missile::MissileStats::default()
        }),
        ..Master::default()
    };
    let deer = Master {
        id: 65,
        kind: 70,
        class: 10,
        hit_points: 5,
        radius: [0.2, 0.2],
        height: 1.5,
        terrain_bonus: -1,
        ..Master::default()
    };
    Masters::new([archer, arrow, deer])
}

/// A flat 40-tile map: player 1's archer 123 at (10, 10) in its Attack's READY on Gaia's deer 50
/// at (13, 10); its swing fires at frame 5 of 0.1 s. The masters as [`archer_masters`]; the next
/// reusable id −1.
fn archer_world(hit_chance: i16, max_volley: u8, spread: f32) -> World {
    use crate::garrison::tests::{unit, walker};
    let mut players: Vec<Player> = (0..2).map(|_| Player::default()).collect();
    players[0].relations = vec![0, 3];
    players[1].relations = vec![3, 0];
    let mut t = World::new(0, 29910, 1, vec![0; 2], players);
    t.spatial = Spatial::new([40, 40]);
    let mut terrain = crate::map::terrain::Terrain::new(
        40,
        40,
        vec![(0, 0, -1); 40 * 40],
        vec![crate::map::terrain::TerrainKind {
            shown: true,
            flags: 0,
            forest: false,
        }],
        Vec::new(),
    );
    terrain.set_elevations(vec![0; 40 * 40]);
    t.spatial.terrain = Some(terrain);
    let sprite = |id| Graphic {
        id,
        frames: 10,
        facets: 8,
        speed_multiplier: 1.0,
        frame_duration: 0.1,
        replay_delay: 0.0,
        flags: 1,
    };
    t.graphics = std::collections::BTreeMap::from([(300, sprite(300)), (301, sprite(301))]);
    let masters = archer_masters(hit_chance, max_volley, spread);
    t.masters = vec![masters.clone(), masters];
    walker(&mut t, 1, 123, 4, [10.0, 10.0]);
    let archer = &mut t.players[1].objects[0];
    if let Some(m) = archer.body.mover_mut() {
        m.sprite_facets = Some(crate::objects::facing::SpriteFacets {
            count: 8,
            flag_1: false,
        });
    }
    archer.firepower = f32::from(max_volley);
    let mut attack = own_attack(50, -1, crate::combat::attack::READY);
    attack.range = 4.0;
    attack.arms.missile = 363;
    attack.arms.fire_frame = 5;
    attack.arms.sprite = Some(sprite(301));
    attack.arms.idle_sprite = Some(sprite(300));
    archer.actions.set_attack(Some(attack));
    let mut deer = unit(50, 65, 5.0);
    deer.place_for_test([13.0, 10.0, 0.0]);
    t.players[0].objects.push(deer);
    t.spatial.insert(
        50,
        Placed {
            position: [13.0, 10.0, 0.0],
            radius: [0.2, 0.2],
            class: 10,
            player: 0,
            master: 65,
            obstruction_type: 2,
            buildable_on: 0,
            tall: true,
            speed: 0.0,
            unknown: None,
            obstruction: Some(crate::map::spatial::Obstruction { kind: 2, class: 0 }),
        },
    );
    t.recycling.next_id = -1;
    t.next_object_id = 200;
    t
}

/// A hit's flare (master 112, the target's player's slot 0x1c0) is made where the volley runs,
/// within the shooter's update: here for the shooter's own player, whose list holds the
/// shooter's stand-in.
#[test]
fn a_hit_s_flare_is_made_where_the_volley_runs() {
    let mut t = archer_world(100, 1, 0.0);
    let flare = Master {
        id: 112,
        kind: crate::objects::masters::master_type::ANIMATED,
        standing_sprite: -1,
        storage: vec![(12, 1.0, 0)],
        ..Master::default()
    };
    t.masters[1] = Masters::new(t.masters[1].iter().cloned().chain([flare]));
    t.players[1].attributes = vec![0.0; 300];
    let aimed = crate::combat::volley::Aimed {
        target: 50,
        position: [13.0, 10.0, 0.0],
        radius: [0.2, 0.2, 1.0],
        class: 10,
        player: 1,
    };
    let seen = in_update(&mut t, (1, 0), 0, |unit, c| {
        unit.volley(c, aimed)?;
        let own = &c.w.players[1].objects;
        Ok(own.iter().filter(|u| u.life.master_id == 112).count())
    });
    assert_eq!(seen.map_err(|e| e.reason().to_owned()), Ok(1));
    assert_eq!(
        t.players[1].objects[0].id, 123,
        "the shooter is back in its slot"
    );
}

/// The missiles in player 1's list: id, master, place.
fn missiles(t: &World) -> Vec<(i32, i16, [f32; 3])> {
    t.players[1]
        .objects
        .iter()
        .filter(|u| u.missile.is_some())
        .map(|u| (u.id, u.life.master_id, u.position()))
        .collect()
}

/// The deer's hit points in [`archer_world`].
fn deer_hp(t: &World) -> f32 {
    t.players[0].objects[0].life.hp
}

/// `a` within 1e-5 of `b` on each axis.
fn assert_near(a: [f32; 3], b: [f32; 3]) {
    assert!(
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-5),
        "{a:?} against {b:?}"
    );
}

/// Slot 0x858 (`FUN_14120c0a0`) and `FUN_141081020`: the archer's swing fires at its firing
/// frame (frame 5 of 0.1 s: the sixth update) its master's missile (363) with the next reusable
/// id (−1), a tile up (its weapon offset) and aimed at the deer a third of its height up; each
/// update the missile flies its speed's share (0.7 of a tile) along that line. It hits the deer
/// in the update its sweep, grown by its radius, meets the deer's footprint: 1 hit point (the
/// least damage, as neither has attacks) and its action done. The dead missile leaves the list
/// four updates after the hit for the pool of type 60, whose id the next swing takes again 20
/// updates after the first (the 2 s reload).
#[test]
fn a_ranged_swing_fires_its_missile_which_flies_to_the_target_and_hits_it() {
    let mut t = archer_world(100, 0, 0.0);
    for _ in 0..5 {
        t.tick(100);
    }
    assert_eq!(missiles(&t), vec![]);
    let way = [3.0f32, 0.0, -0.5];
    let length = (way[0] * way[0] + way[2] * way[2]).sqrt();
    let flown = |k: f32| {
        [
            10.0 + k * 0.7 * way[0] / length,
            10.0,
            1.0 + k * 0.7 * way[2] / length,
        ]
    };
    for k in 1..=3 {
        t.tick(100);
        let m = missiles(&t);
        assert_eq!((m.len(), m[0].0, m[0].1), (1, -1, 363));
        assert_near(m[0].2, flown(k as f32));
        assert_eq!(deer_hp(&t), 5.0);
    }
    // the fourth step ends 0.1 short of the deer's footprint (12.8): its sweep meets it
    t.tick(100);
    assert_near(missiles(&t)[0].2, flown(4.0));
    assert_eq!(deer_hp(&t), 4.0);
    let action = t.players[1].objects[1]
        .actions
        .missile()
        .map(|a| (a.state, a.result));
    assert_eq!(action, Some((crate::combat::missile::DONE, 1)));
    for _ in 0..3 {
        t.tick(100);
    }
    assert_eq!(missiles(&t).len(), 1);
    t.tick(100);
    assert_eq!(missiles(&t), vec![]);
    assert_eq!(t.recycling.missiles, vec![(-1, true)]);
    for _ in 0..12 {
        t.tick(100);
    }
    assert_eq!(missiles(&t), vec![]);
    t.tick(100);
    assert_eq!(missiles(&t).iter().map(|m| m.0).collect::<Vec<_>>(), [-1]);
    assert_eq!(t.recycling.missiles, vec![]);
}

/// The missile a swing fires joins the end of its owner's live list at its making (slot
/// 0x2d0, which files it in the tracker), inside the archer's update: what the update does after
/// the fire reaches it by id, as the game's table does.
#[test]
fn a_fired_missile_joins_its_owner_s_list_at_its_making() {
    let mut t = archer_world(100, 0, 0.0);
    for _ in 0..5 {
        t.tick(100);
    }
    let made = in_update(&mut t, (1, 0), 100, |unit, c| {
        let _ = unit.update(c)?;
        let k = crate::players::tracker::key(c.w.masters.get(c.player), &c.w.lines, 363);
        let listed = OthersRef::of(&c.w.players, c.player)
            .own(-1)
            .map(|m| m.life.master_id);
        Ok((
            listed,
            c.w.players[c.player].tracker.list(k.master).contains(&-1),
        ))
    });
    assert_eq!(made, Ok((Some(363), true)));
    let ids: Vec<i32> = t.players[1].objects.iter().map(|u| u.id).collect();
    assert_eq!(ids.last(), Some(&-1));
}

/// `FUN_140a9c600`'s miss: a hit chance of 0 strays the missile by up to the ground distance
/// times the master's missed spread; it flies past the deer, lands on the ground (its height 0,
/// `FUN_140a9c890`) and goes to the pool as landed (not flying).
#[test]
fn a_missed_missile_strays_and_lands_on_the_ground() {
    let mut t = archer_world(0, 0, 0.0);
    for m in &mut t.masters {
        m.get_mut(4).unwrap().missed_spread = 2.0;
    }
    for _ in 0..6 {
        t.tick(100);
    }
    let at = missiles(&t)[0].2;
    assert_ne!(at[1], 10.0);
    for _ in 0..4 {
        t.tick(100);
    }
    assert_eq!(missiles(&t)[0].2[2], 0.0);
    for _ in 0..2 {
        t.tick(100);
    }
    assert_eq!(missiles(&t), vec![]);
    assert_eq!(t.recycling.missiles, vec![(-1, false)]);
    assert_eq!(deer_hp(&t), 5.0);
    // the next shot takes the landed missile again: on the ground by its fly mode, a tile up
    // again once it is placed (its new position's height is the weapon offset's)
    for _ in 0..14 {
        t.tick(100);
    }
    assert_eq!(missiles(&t).iter().map(|m| m.0).collect::<Vec<_>>(), [-1]);
    assert_eq!(t.recycling.missiles, vec![]);
}

/// A volley (`max_attacks_in_volley` 3, firepower 3): with a spread, a swing fires 1 to 3 of its
/// missiles (an Object-stream draw), the first the master's missile on the line and the others
/// scattered off it, fired before it; `next_volley` counts the fired, the rest follow at the next
/// swing and then it is 0. Each missile that meets the deer takes a hit point. Without a spread
/// one missile a swing.
#[test]
fn a_volley_fires_its_missiles_over_swings_scattered_by_its_spread() {
    let mut t = archer_world(100, 3, 0.5);
    for _ in 0..6 {
        t.tick(100);
    }
    let first = missiles(&t);
    let fired = t.players[1].objects[0].next_volley;
    assert!((1..3).contains(&fired), "{fired}");
    assert_eq!(first.len(), usize::from(fired));
    let (main, scattered) = first.split_last().unwrap();
    assert_eq!(main.0, -i32::from(fired));
    assert!(scattered.iter().all(|m| m.2 != main.2));
    for _ in 0..3 {
        t.tick(100);
    }
    assert_eq!(deer_hp(&t), 5.0 - f32::from(fired));
    // the next swing, six updates after the first, fires the rest
    t.tick(100);
    t.tick(100);
    assert_eq!(t.players[1].objects[0].next_volley, fired);
    t.tick(100);
    assert_eq!(t.players[1].objects[0].next_volley, 0);
    let mut one = archer_world(100, 3, 0.0);
    for _ in 0..6 {
        one.tick(100);
    }
    assert_eq!(
        (missiles(&one).len(), one.players[1].objects[0].next_volley),
        (1, 1)
    );
    // the missile's own +0x900 is its archer's attack count when fired
    let counts: Vec<i32> = one.players[1].objects[1..]
        .iter()
        .filter(|u| u.missile.is_some())
        .map(|u| u.attack_count)
        .collect();
    assert_eq!(counts, [one.players[1].objects[0].attack_count]);
    assert_ne!(counts, [0]);
}

/// `FUN_141226690` with the master's own volley missile (`volley_missile`, 364): the extra
/// missiles are shooterless, each hits by its own master (`FUN_140a9e6e0`'s slot 0x860, the
/// missile as the attacker: 1 hit point), the first stays the master's missile with the shooter.
#[test]
fn a_volley_s_own_missiles_hit_without_the_shooter() {
    let mut t = archer_world(100, 3, 0.5);
    for m in &mut t.masters {
        let mut all: Vec<Master> = m.iter().cloned().collect();
        let own = Master {
            id: 364,
            ..m.get(363).unwrap().clone()
        };
        all.push(own);
        m.get_mut(4).unwrap().volley_missile = Some(364);
        all.retain(|x| x.id != 4);
        all.push(m.get(4).unwrap().clone());
        *m = Masters::new(all);
    }
    for _ in 0..6 {
        t.tick(100);
    }
    let fired = t.players[1].objects[0].next_volley;
    let first = missiles(&t);
    let masters: Vec<i16> = first.iter().map(|m| m.1).collect();
    let mut expected = vec![364; usize::from(fired) - 1];
    expected.push(363);
    assert_eq!(masters, expected);
    let shooters: Vec<i32> = t.players[1].objects[1..]
        .iter()
        .filter_map(|u| u.actions.missile().map(|a| a.fired_from))
        .collect();
    let mut expected = vec![-1; usize::from(fired) - 1];
    expected.push(123);
    assert_eq!(shooters, expected);
    for _ in 0..3 {
        t.tick(100);
    }
    assert_eq!(deer_hp(&t), 5.0 - f32::from(fired));
}

/// `FUN_1410d5920`'s setups for a new missile draw as a new object's: an animated standing sprite
/// (flag 1) with a random first frame (flag 4) starts on the Sprite stream (two draws), then its
/// facet and, as it turns (flag 2) with several facets, its angle on the Object stream; the
/// missile's angle is then its flight's. A missile without a standing sprite draws none of them
/// (the draw counters count each step; the streams' indexes restart at each seeding).
#[test]
fn a_new_missile_s_sprite_draws_as_a_new_object_s() {
    let mut plain = archer_world(100, 0, 0.0);
    let mut drawn = archer_world(100, 0, 0.0);
    let sprite = Graphic {
        id: 302,
        frames: 10,
        facets: 8,
        speed_multiplier: 1.0,
        frame_duration: 0.1,
        replay_delay: 0.0,
        flags: 7,
    };
    let _ = drawn.graphics.insert(302, sprite);
    for m in &mut drawn.masters {
        m.get_mut(363).unwrap().standing_sprite = 302;
    }
    for _ in 0..6 {
        plain.tick(100);
        drawn.tick(100);
    }
    assert_eq!(missiles(&plain), missiles(&drawn));
    let mut more = |stream| drawn.streams.get(stream).draws - plain.streams.get(stream).draws;
    assert_eq!((more(Stream::Sprite), more(Stream::Object)), (2, 2));
}

/// The missile chain's cases the slice does not model stop the shooter's player with their
/// reason, in the update the swing fires (or the missile flies): a charged hit, a target of a
/// computer player (its notify 0x214), a missile master that cannot be pooled, moved or flown,
/// a smart missile and a projectile of type 1.
#[test]
fn the_missile_chain_s_unmodelled_cases_stop_the_player_with_their_reason() {
    type Change = fn(&mut World);
    let cases: [(Change, &str); 7] = [
        (
            |t| {
                for m in &mut t.masters {
                    m.get_mut(4).unwrap().charge_type = 6;
                }
            },
            "a charged hit",
        ),
        (
            |t| t.players[0].computer = true,
            "a hit on a computer player's object",
        ),
        (
            |t| {
                for m in &mut t.masters {
                    m.get_mut(363).unwrap().recyclable = false;
                }
            },
            "a missile of a master that is not recyclable",
        ),
        (
            |t| {
                for m in &mut t.masters {
                    m.get_mut(363).unwrap().moving = None;
                }
            },
            "a missile master that does not move",
        ),
        (
            |t| {
                for m in &mut t.masters {
                    m.get_mut(363).unwrap().missile_stats = None;
                }
            },
            "a missile master without its flight",
        ),
        (
            |t| {
                for m in &mut t.masters {
                    if let Some(s) = m.get_mut(363).unwrap().missile_stats.as_mut() {
                        s.smart_mode = 1;
                    }
                }
            },
            "a smart missile leading its target",
        ),
        (
            |t| {
                for m in &mut t.masters {
                    if let Some(s) = m.get_mut(363).unwrap().missile_stats.as_mut() {
                        s.projectile_type = 1;
                    }
                }
            },
            "a missile of projectile type 1 or 2",
        ),
    ];
    for (change, reason) in cases {
        let mut t = archer_world(100, 0, 0.0);
        change(&mut t);
        for _ in 0..6 {
            t.tick(100);
        }
        let stopped = t.players[1].unsupported.as_ref().map(|u| u.2.to_string());
        assert!(
            stopped.as_deref().is_some_and(|s| s.starts_with(reason)),
            "{reason}: {stopped:?}"
        );
    }
}

/// `FUN_140a9e6e0`'s start: a missile whose target died (state 3) lets it go (its release) and
/// flies on without one: past the place it would have hit the live deer, down to the ground, and
/// into the pool as landed.
#[test]
fn a_missile_whose_target_died_flies_on_without_it_and_lands() {
    let mut t = archer_world(100, 0, 0.0);
    for _ in 0..7 {
        t.tick(100);
    }
    t.players[0].objects[0].life.state = 3;
    t.tick(100);
    let target = t.players[1].objects[1].actions.missile().map(|a| a.target);
    assert_eq!(target, Some(-1));
    // the update the live deer took its hit in passes it by
    t.tick(100);
    assert_eq!(deer_hp(&t), 5.0);
    t.tick(100);
    assert_eq!(missiles(&t)[0].2[2], 0.0);
    t.tick(100);
    t.tick(100);
    assert_eq!(missiles(&t), vec![]);
    assert_eq!(t.recycling.missiles, vec![(-1, false)]);
    assert_eq!(deer_hp(&t), 5.0);
}

/// QUEUE's pick (`FUN_141614450`): the building with room and the least load, the last pick holding
/// against an equal load; a comparison with a NaN load replaces nothing; with none with room, none.
#[test]
fn a_queued_unit_goes_to_the_least_loaded_building_with_room() {
    use crate::commands::least_loaded;
    assert_eq!(least_loaded(&[2, 2, 2], &[5.0, 3.0, 4.0], None), Some(1));
    assert_eq!(least_loaded(&[2, 0, 2], &[5.0, 3.0, 4.0], None), Some(2));
    assert_eq!(least_loaded(&[2, 2], &[3.0, 3.0], Some(1)), Some(1));
    assert_eq!(least_loaded(&[2, 2], &[f32::NAN, 3.0], None), Some(0));
    assert_eq!(
        least_loaded(&[2, 2, 2], &[4.0, f32::NAN, 3.0], None),
        Some(2)
    );
    assert_eq!(least_loaded(&[2, 2], &[3.0, f32::NAN], None), Some(0));
    assert_eq!(least_loaded(&[0, 2], &[1.0, 3.0], Some(0)), Some(0));
    assert_eq!(least_loaded(&[0, 0], &[3.0, 1.0], None), None);
    assert_eq!(least_loaded(&[], &[], None), None);
}

/// [`forage_world`]'s forager on bush 600 (the unit AI's gather command), its last move at its
/// place, the load's mesh built.
fn foraging() -> World {
    let mut t = forage_world();
    t.players[1].objects[0].body.mover_mut().unwrap().moved_at = [29.5, 89.5, 0.0];
    t.gather_command(1, 0, 600).unwrap();
    t.build_mesh();
    t
}

/// `n` of the forager's whole updates (50 ms each), up to the first after which `stop` holds.
fn forage_until(t: &mut World, n: usize, stop: impl Fn(&World) -> bool) {
    for _ in 0..n {
        let _ = in_update(t, (1, 0), 50, Unit::update).unwrap();
        if stop(t) {
            return;
        }
    }
}

/// The forager's Gather, as its update runs it (`0x140a8e1d0`): it walks to its bush (state 4),
/// arrives (6) and works (7) until it holds its master's capacity of 10 berries; then it carries
/// them to its town centre (8, the centre its second target), whose drop gives the player 10
/// food, and walks back to the bush (4), arrives and works again.
#[test]
fn a_forager_fills_up_carries_its_berries_home_and_comes_back() {
    let mut t = foraging();
    let mut states = vec![4];
    for _ in 0..1200 {
        forage_until(&mut t, 1, |_| false);
        let v = &t.players[1].objects[0];
        let g = v.actions.gather().copied().unwrap();
        if states.last() != Some(&g.state) {
            states.push(g.state);
            if g.state == crate::gathering::gather::DROP {
                assert_eq!((v.life.attribute_type, v.life.amount), (17, 10.0));
                assert_eq!((g.target, g.carry_to), (600, 700));
            }
        }
    }
    assert_eq!(states, [4, 6, 7, 8, 4, 6, 7]);
    assert_eq!(t.players[1].resources[0], 10.0);
    let g = t.players[1].objects[0].actions.gather().copied().unwrap();
    assert_eq!((g.target, g.carry_to), (600, -1));
}

/// A bush that dies under its forager (past state 6) is let go (its worker leaves) and the
/// Gather searches (state 3): the unit AI hears 0x202 with the gather action and the target
/// type 7.
#[test]
fn a_forager_whose_bush_dies_lets_it_go_and_asks_for_a_search() {
    let mut t = foraging();
    let working = |t: &World| t.players[1].objects[0].actions.gather().map(|g| g.state) == Some(7);
    forage_until(&mut t, 200, working);
    assert_eq!(t.players[0].objects[0].workers, 1);
    t.players[0].objects[0].life.state = 7;
    forage_until(&mut t, 1, |_| false);
    let v = &t.players[1].objects[0];
    assert_eq!(
        v.actions.gather().map(|g| (g.state, g.target)),
        Some((3, -1))
    );
    assert_eq!(t.players[0].objects[0].workers, 0);
    let heard: Vec<_> =
        v.ai.as_ref()
            .unwrap()
            .notifies
            .iter()
            .map(|n| (n.kind, n.p[0], n.p[1]))
            .collect();
    assert_eq!(
        heard,
        [(
            crate::unit_ai::REQUIRES_SEARCH,
            ActionKind::Gather.value(),
            7
        )]
    );
}

/// A forager whose bush went while it carried its load drops the load (the player's food) and,
/// without a target, takes the nearest bush its unit AI's search finds (`ClosestResourceObject`
/// of the Gather's target type's group, 1 for bushes, within its sight plus 5: bush 601 beside
/// the centre, state 4); with none it walks back to where it worked to search again (state 11).
#[test]
fn a_forager_without_its_bush_after_the_drop_takes_the_next_one() {
    let carrying = |near: bool| {
        let mut t = foraging();
        for masters in &mut t.masters {
            let bush = masters.get_mut(59).unwrap();
            (bush.auto_gather, bush.gather_group) = (true, 1);
            masters.get_mut(120).unwrap().line_of_sight = 4.0;
        }
        t.players[1].objects[0].ai.as_mut().unwrap().target_type = -1;
        if near {
            t.players[0].objects[1].place_for_test([25.5, 89.5, 0.0]);
            let p = *t.spatial.get(601).unwrap();
            t.spatial.insert(
                601,
                Placed {
                    position: [25.5, 89.5, 0.0],
                    ..p
                },
            );
        }
        let carries =
            |t: &World| t.players[1].objects[0].actions.gather().map(|g| g.state) == Some(8);
        forage_until(&mut t, 800, carries);
        let _ = t.players[0].objects.remove(0);
        t.spatial.remove(600);
        let dropped =
            |t: &World| t.players[1].objects[0].actions.gather().map(|g| g.state) != Some(8);
        forage_until(&mut t, 400, dropped);
        assert_eq!(t.players[1].resources[0], 10.0);
        t.players[1].objects[0]
            .actions
            .gather()
            .map(|g| (g.state, g.target))
    };
    assert_eq!(carrying(true), Some((4, 601)));
    assert_eq!(carrying(false), Some((11, -1)));
}

/// [`foraging`] up to the forager's drop at its town centre without a target (bush 600 gone
/// on the way, `setup` run before), after which the Gather's state follows.
fn dropped_without_a_bush(setup: impl FnOnce(&mut World)) -> World {
    let mut t = foraging();
    let carries = |t: &World| t.players[1].objects[0].actions.gather().map(|g| g.state) == Some(8);
    forage_until(&mut t, 800, carries);
    let _ = t.players[0].objects.remove(0);
    t.spatial.remove(600);
    setup(&mut t);
    let dropped = |t: &World| t.players[1].objects[0].actions.gather().map(|g| g.state) != Some(8);
    forage_until(&mut t, 400, dropped);
    assert_eq!(t.players[1].resources[0], 10.0);
    t
}

/// After the drop without a target and nothing found: a Gather without its point idles for a
/// human player (state 2, its targets and point cleared, the centre let go) and is done for a
/// computer player; with orders queued (on a target the world holds) the unit AI ends its
/// order and the Gather is done. An order on a target gone leaves the queue first.
#[test]
fn a_forager_with_nothing_found_after_the_drop_idles_or_is_done() {
    let pointless = |t: &mut World| {
        t.players[1].objects[0].actions.gather_mut().unwrap().point = [-1.0; 3];
    };
    let t = dropped_without_a_bush(pointless);
    let g = t.players[1].objects[0].actions.gather().copied().unwrap();
    assert_eq!(
        (g.state, g.target, g.carry_to, g.point),
        (2, -1, -1, [-1.0; 3])
    );
    assert_eq!(t.players[1].objects[1].workers, 0);
    let t = dropped_without_a_bush(|t| {
        pointless(t);
        t.players[1].computer = true;
    });
    assert!(t.players[1].objects[0].actions.gather().is_none());
    let order = |target| crate::unit_ai::QueuedOrder {
        issuer: 1,
        order: OrderKind::Gather,
        priority: 100,
        target,
        target_owner: 0,
        point: [36.5, 89.5, 0.0],
        range: 0.0,
        shown: [36.5, 89.5],
        ready: true,
        target_type: 7,
    };
    // the Gather's state 8 after the deposit, its bush gone, two orders queued
    let mut t = foraging();
    let _ = t.players[0].objects.remove(0);
    t.spatial.remove(600);
    let v = &mut t.players[1].objects[0];
    v.actions.gather_mut().unwrap().target = -1;
    v.ai.as_mut().unwrap().orders = vec![order(600), order(601)];
    in_update(&mut t, (1, 0), 50, |unit, c| {
        let code = unit.gather_after_drop(c)?;
        unit.end_head_update(c, code)
    })
    .unwrap();
    let v = &t.players[1].objects[0];
    assert!(v.actions.gather().is_none());
    let ai = v.ai.as_ref().unwrap();
    assert_eq!(
        ai.orders.iter().map(|o| o.target).collect::<Vec<_>>(),
        [601]
    );
    assert_eq!((ai.current_order, ai.current_action), (None, None));
}

/// The forager's Gather update alone (`gather_update`, 50 ms).
fn gather_step(t: &mut World) -> Result<(), GenieError> {
    in_update(t, (1, 0), 50, |unit, c| {
        let me = crate::movement::Me {
            id: unit.id,
            group: unit.group.id,
            action_state: unit.top_action().map_or(0, |a| a.state),
        };
        let code = unit.gather_update(c, me)?;
        unit.end_head_update(c, code)
    })
}

/// What follows the Gather's update ([`Unit::gather_next`]) for the forager on bush 600 with
/// its town centre 700, each worked by one: the end lets both go and removes the Gather; a
/// target the update changed lets the old one go, where the update does; a build command asks
/// for the Gather's target to be built after the update (a second in one update is not
/// modelled); the reseed after the walk back to the farm, whether it arrived.
#[test]
fn what_follows_the_gather_s_update_lets_go_builds_or_reseeds() {
    use crate::gathering::gather::Next;
    type Followed = (Result<(), GenieError>, Vec<i32>, Option<i32>, Option<bool>);
    let follow = |next: Next, old: (i32, i32), built: Option<i32>| -> (World, Followed) {
        let mut t = foraging();
        t.players[1].objects[0]
            .actions
            .gather_mut()
            .unwrap()
            .carry_to = 700;
        for id in [600, 700] {
            t.object_mut(id).unwrap().1.workers = 1;
        }
        let seen = in_update(&mut t, (1, 0), 50, |unit, c| {
            let by = unit.releaser(c);
            c.build_command = built;
            let r = unit
                .gather_next(c, next, old, by)
                .and_then(|code| unit.end_head_update(c, code));
            // the objects let go, as the update leaves them
            let released = [600, 700]
                .into_iter()
                .filter(|&id| {
                    OthersRef::of(&c.w.players, c.player)
                        .find(id)
                        .is_some_and(|(_, u)| u.workers == 0)
                })
                .collect();
            Ok((r, released, c.build_command.take(), c.reseed.take()))
        })
        .unwrap();
        (t, seen)
    };
    let (t, seen) = follow(Next::Done, (600, 700), None);
    assert_eq!(seen, (Ok(()), vec![600, 700], None, None));
    assert!(t.players[1].objects[0].actions.gather().is_none());
    let (t, seen) = follow(Next::Stay, (700, -1), None);
    assert_eq!(seen, (Ok(()), vec![700], None, None));
    assert!(t.players[1].objects[0].actions.gather().is_some());
    let (_, seen) = follow(Next::Stay, (600, 600), None);
    assert_eq!(seen, (Ok(()), vec![600], None, None));
    let (_, seen) = follow(Next::Build, (600, 700), None);
    assert_eq!(seen, (Ok(()), vec![], Some(600), None));
    let (_, (r, ..)) = follow(Next::Build, (600, 700), Some(5));
    assert_eq!(r.unwrap_err().reason(), "two build commands in one update");
    let (_, seen) = follow(Next::Reseed { arrived: true }, (600, 700), None);
    assert_eq!(seen, (Ok(()), vec![], None, Some(true)));
}

/// The Gather's other states (`0x140a8e1d0`): done, it goes and lets its bush go; the search's
/// wait (case 2) at its end asks the unit AI again (0x202) and waits 10 seconds more; the wait
/// after a failed walk (case 3) at its end walks back to the Gather's point to search there
/// (state 11).
#[test]
fn the_gather_s_update_ends_asks_again_and_walks_back() {
    use crate::gathering::gather::{DONE, SEARCH, WALK, WALK_BACK};
    let set = |state, timer| {
        let mut t = foraging();
        let g = t.players[1].objects[0].actions.gather_mut().unwrap();
        (g.state, g.timer) = (state, timer);
        t
    };
    let mut t = set(DONE, 0.0);
    assert_eq!(t.players[0].objects[0].workers, 1);
    gather_step(&mut t).unwrap();
    assert!(t.players[1].objects[0].actions.gather().is_none());
    assert_eq!(t.players[0].objects[0].workers, 0);
    let mut t = set(SEARCH, -0.02);
    gather_step(&mut t).unwrap();
    let v = &t.players[1].objects[0];
    assert_eq!(v.actions.gather().map(|g| g.timer), Some(-10.0));
    let heard: Vec<i32> =
        v.ai.as_ref()
            .unwrap()
            .notifies
            .iter()
            .map(|n| n.kind)
            .collect();
    assert_eq!(heard, [crate::unit_ai::REQUIRES_SEARCH]);
    let mut t = set(SEARCH, -0.2);
    gather_step(&mut t).unwrap();
    assert!(
        (t.players[1].objects[0]
            .actions
            .gather()
            .copied()
            .unwrap()
            .timer
            + 0.15)
            .abs()
            < 1e-6
    );
    let mut t = set(WALK, -0.02);
    gather_step(&mut t).unwrap();
    assert_eq!(
        t.players[1].objects[0].actions.gather().map(|g| g.state),
        Some(WALK_BACK)
    );
}

/// [`forage_world`]'s forager (player 1's object 0) building house 10028 (master 70, at (30, 88),
/// player 1's object 2) by the build command; the town centre 700 is complete in its tracker
/// and the drop-site table's town centre takes the four basic resources.
fn house_builder() -> World {
    let mut t = forage_world();
    let p = *t.spatial.get(700).unwrap();
    let site = Placed {
        position: [30.0, 88.0, 0.0],
        radius: [1.0, 1.0],
        class: 80,
        master: 70,
        ..p
    };
    t.spatial.insert(10028, site);
    let _ = t.build_command(1, 0, 10028).unwrap();
    let k = crate::players::tracker::key(t.masters.get(1), &t.lines, 109);
    t.players[1].tracker.add(700, k);
    t.players[1].tracker.complete(700, k);
    t.drop_sites[0].resource_types = vec![0, 1, 2, 3];
    t
}

/// The Build's drop-off (`set_state(8)` from its search, [`BuildAction::drop_again`]) of the
/// villager building foundation 10028 while it carries `held`, its town centre a done drop site
/// of the table (resources 0 to 3 and berries): Ok with the Build's state, its site and range,
/// the centre's workers and what the unit AI heard.
fn drop_again_with(held: (i16, f32)) -> (u8, i32, f32, i32, Vec<i32>) {
    let mut t = house_builder();
    let v = &mut t.players[1].objects[0];
    (v.life.attribute_type, v.life.amount) = held;
    in_update(&mut t, (1, 0), 50, |unit, c| {
        let mut b = *unit.actions.build().unwrap();
        let by = unit.releaser(c);
        let dropped = b.drop_again(c, unit, by);
        *unit.actions.build_mut().unwrap() = b;
        dropped
    })
    .unwrap();
    let v = &t.players[1].objects[0];
    let b = v.actions.build().copied().unwrap();
    let heard =
        v.ai.as_ref()
            .unwrap()
            .notifies
            .iter()
            .map(|n| n.kind)
            .collect();
    (
        b.state,
        b.site,
        b.drop_range,
        t.players[1].objects[2].workers,
        heard,
    )
}

/// The Build's drop-off: a load the table's town centre takes makes it the Build's site (taken,
/// one worker) with the range of the task it gives (the forager's Gather on berries, 0.25; none
/// for stone: the Build task's, 0); an empty builder walks back to its foundation (state 4); a
/// load no drop site takes leaves no site and tells the unit AI (0x203).
#[test]
fn a_loaded_builder_s_drop_off_finds_its_town_centre() {
    use crate::construction::build::DROP;
    assert_eq!(drop_again_with((17, 6.0)), (DROP, 700, 0.25, 1, vec![]));
    assert_eq!(drop_again_with((2, 6.0)), (DROP, 700, 0.0, 1, vec![]));
    assert_eq!(drop_again_with((-1, 0.0)), (4, -1, 0.0, 0, vec![]));
    assert_eq!(
        drop_again_with((5, 6.0)),
        (4, -1, 0.0, 0, vec![crate::unit_ai::NO_DROP_SITE])
    );
}

/// [`forage_world`]'s forager 9175 (at 29.5) seeking a drop site from itself
/// ([`Unit::drop_site_with`] with the search's arguments 10, 9 and 8 `args`) on a map with a tile
/// planner, a second town centre 701 at 45.0 beside centre 700 at 20.0; `setup` changes the
/// world first.
fn drop_site_sought(
    setup: impl FnOnce(&mut World),
    args: (bool, bool, i16),
) -> Result<Option<i32>, GenieError> {
    let mut t = forage_world();
    let mut far = t.players[1].objects[2].clone();
    (far.id, far.position()[0]) = (701, 45.0);
    t.players[1].objects.push(far);
    let p = *t.spatial.get(700).unwrap();
    t.spatial.insert(
        701,
        Placed {
            position: [45.0, 89.5, 0.0],
            ..p
        },
    );
    t.pathing = Pathing::new([120, 120], vec![0; 120 * 120], vec![vec![1.0]; 8], 0);
    t.build_mesh();
    setup(&mut t);
    let mut world = DropWorld {
        masters: t.masters.get(1),
        spatial: &t.spatial,
        zones: &mut t.zones,
        pathing: &mut t.pathing,
        object: t.streams.get(Stream::Object),
        world_time: 0,
        computer: false,
        construction_rate: 1.0,
        groups: &t.groups,
    };
    let objects = OthersRef::of(&t.players, 1);
    t.players[1].objects[0].drop_site_with(&mut world, objects, 0, args)
}

/// The drop-site search from a unit (`FUN_1411671e0`): the nearer town centre wins by path (its
/// length within 10 tiles measured by the short path too) and by the squared distance of the
/// centres; a centre inside another object is passed over when the search keeps outside, one at
/// work on the searched action type always (an object of type 40 up), so the far centre wins.
/// A drop site of a master the player does not hold fails.
#[test]
fn a_drop_site_search_from_a_unit_takes_the_nearest_it_reaches() {
    use crate::gathering::make::{MAKE, MakeAction};
    let keep = |_: &mut World| ();
    assert_eq!(drop_site_sought(keep, (false, true, -1)), Ok(Some(700)));
    assert_eq!(drop_site_sought(keep, (false, false, -1)), Ok(Some(700)));
    let inside = |t: &mut World| t.players[1].objects[2].life.inside = Some(9175);
    assert_eq!(drop_site_sought(inside, (true, true, -1)), Ok(Some(701)));
    assert_eq!(drop_site_sought(inside, (false, true, -1)), Ok(Some(700)));
    let making = |t: &mut World| {
        t.players[1].objects[2].actions.set_make(Some(MakeAction {
            state: 2,
            remaining: 0.0,
            total: 0.0,
            task: crate::objects::masters::Task::default(),
        }));
    };
    assert_eq!(drop_site_sought(making, (false, true, MAKE)), Ok(Some(701)));
    assert_eq!(drop_site_sought(making, (false, true, -1)), Ok(Some(700)));
    let unknown = |t: &mut World| {
        for masters in &mut t.masters {
            masters.get_mut(120).unwrap().drop_sites = vec![109, 999];
        }
        t.players[1].objects[2].life.master_id = 999;
    };
    assert_eq!(
        drop_site_sought(unknown, (false, true, -1))
            .unwrap_err()
            .reason(),
        OBJECT_MASTER.reason()
    );
}

/// [`forage_world`]'s villager 9175 with its own `MoveTo` `m` on a map with a tile planner.
fn own_move_world(m: MoveToAction) -> World {
    let mut t = forage_world();
    t.pathing = Pathing::new([120, 120], vec![0; 120 * 120], vec![vec![1.0]; 8], 0);
    t.build_mesh();
    t.players[1].objects[0].actions.set_move_to(Some(m));
    t
}

/// What [`own_move`] gives: the part's result, the `MoveTo` after it, what the unit AI heard
/// and bush 600's workers after the step's releases.
type OwnMoved = (
    Result<(), GenieError>,
    Option<MoveToAction>,
    Vec<(i32, [i32; 3])>,
    i32,
);

/// The villager's update's `MoveTo` part ([`Unit::move_to_part`]) in `t`.
fn own_move(t: &mut World) -> OwnMoved {
    let r = in_update(t, (1, 0), 50, |unit, c| {
        let me = crate::movement::Me {
            id: unit.id,
            group: unit.group.id,
            action_state: unit.actions.move_to().map_or(0, |m| m.state.number()),
        };
        let code = unit.move_to_part(c, me)?;
        unit.end_head_update(c, code)
    });
    let v = &t.players[1].objects[0];
    let heard = v.ai.as_ref().map_or_else(Vec::new, |ai| {
        ai.notifies.iter().map(|n| (n.kind, n.p)).collect()
    });
    (
        r,
        v.actions.move_to().copied(),
        heard,
        t.players[0].objects[0].workers,
    )
}

/// The unit's own `MoveTo` in its update (`0x140aa2a70`): a start toward bush 600 asks for its
/// path and moves; one that arrived tells the unit AI (0x1fa with the move action value, which
/// an AI on another action ignores), lets its target go (one worker less) and goes; a failed
/// one tells 0x1f9 and goes. A target that
/// left the point the walk goes to (more than half a tile) makes it tell 0x1fc.
#[test]
fn a_unit_s_own_move_to_walks_tells_its_end_and_lets_its_target_go() {
    use crate::unit_ai::{ACTION_COMPLETED, ACTION_FAILED, TARGET_MOVED};
    let to_bush = |state| MoveToAction {
        state,
        goal: None,
        target: 600,
        range: 0.25,
        sprite: None,
        wait: crate::movement::move_to::MoveWait::default(),
    };
    let mut t = own_move_world(to_bush(MoveTo::Start));
    let (r, m, heard, _) = own_move(&mut t);
    assert_eq!(
        (r, m.map(|m| m.state), heard),
        (Ok(()), Some(MoveTo::Moving), vec![])
    );
    // the bush moves two tiles off the point the walk goes to
    t.spatial.set_position(600, [32.5, 91.5, 0.0]);
    let (r, _, heard, _) = own_move(&mut t);
    assert_eq!(r, Ok(()));
    assert_eq!(heard, [(TARGET_MOVED, [0, 0, 0])]);
    let arrived = |action| {
        let mut t = own_move_world(to_bush(MoveTo::Arrived));
        t.players[0].objects[0].workers = 1;
        t.players[1].objects[0].ai.as_mut().unwrap().current_action = action;
        own_move(&mut t)
    };
    let (r, m, heard, workers) = arrived(Some(ActionKind::Move));
    assert_eq!((r, m, workers), (Ok(()), None, 0));
    assert_eq!(
        heard,
        [(ACTION_COMPLETED, [ActionKind::Move.value(), 0, 0])]
    );
    // the unit AI on another action ignores the move's completion (FUN_140c556d0)
    assert_eq!(arrived(Some(ActionKind::Gather)).2, []);
    let to_point = MoveToAction {
        goal: Some([40.5, 89.5, 0.0]),
        target: -1,
        ..to_bush(MoveTo::Failed)
    };
    let (r, m, heard, _) = own_move(&mut own_move_world(to_point));
    assert_eq!((r, m), (Ok(()), None));
    assert_eq!(heard, [(ACTION_FAILED, [ActionKind::Move.value(), 0, 0])]);
}

/// The result of a bird's Fly part, its Fly after it and what its unit AI heard.
type Flown = (Result<(), GenieError>, Option<Fly>, Vec<(i32, [i32; 3])>);

/// [`forage_world`]'s villager 9175 as a bird flying (state 4) toward (40.5, 89.5) with its
/// `MoveTo` in `move_to`, two seconds of flight left and a rest of two seconds; `setup`
/// changes the unit, then its update's Fly part runs ([`Unit::fly_part`]): the result, the Fly
/// after it and what its unit AI heard.
fn flown(move_to: MoveTo, setup: impl FnOnce(&mut Unit)) -> Flown {
    let mut t = forage_world();
    let task = crate::objects::masters::Task {
        wait_time: 2.0,
        ..crate::objects::masters::Task::default()
    };
    let v = &mut t.players[1].objects[0];
    v.actions.set_fly(Some(Fly {
        state: 4,
        timer: 2.0,
        move_to: Some(move_to),
        goal: Some([40.5, 89.5, 0.0]),
        home: [29.5, 89.5],
        task,
    }));
    setup(v);
    let r = in_update(&mut t, (1, 0), 50, |unit, c| {
        let me = crate::movement::Me {
            id: unit.id,
            group: unit.group.id,
            action_state: 4,
        };
        unit.fly_part(c, me)
    });
    let v = &t.players[1].objects[0];
    let heard = v.ai.as_ref().map_or_else(Vec::new, |ai| {
        ai.notifies.iter().map(|n| (n.kind, n.p)).collect()
    });
    (r, v.actions.fly().copied(), heard)
}

/// A bird's Fly in its update: a failed `MoveTo` ends the flight, its failure told to the unit
/// AI (0x1f9 with the move action value); the bird stops and rests (state 2) for the flight's
/// time left and the rest; a bird without movement state is not modelled.
#[test]
fn a_bird_s_failed_flight_tells_its_unit_ai_and_rests() {
    use crate::unit_ai::ACTION_FAILED;
    let (r, fly, heard) = flown(MoveTo::Failed, |_| ());
    assert_eq!(r, Ok(()));
    let fly = fly.unwrap();
    assert_eq!((fly.state, fly.move_to), (2, None));
    assert_eq!(fly.timer, 2.0 - seconds(50) + 2.0);
    assert_eq!(heard, [(ACTION_FAILED, [ActionKind::Move.value(), 0, 0])]);
    let (r, _, _) = flown(MoveTo::Failed, |v| v.body.put_mover(None));
    assert_eq!(r.unwrap_err().reason(), "a bird without movement state");
}

/// [`forage_world`]'s villager 9175 with an Explore toward (40.5, 89.5) on a map with a tile
/// planner, its Explore changed by `set`, through its update's Explore part
/// ([`Unit::explore_part`]): the result, the Explore after it and what its unit AI heard.
fn explored(set: impl FnOnce(&mut crate::movement::explore::ExploreAction)) -> Explored {
    let mut t = forage_world();
    t.pathing = Pathing::new([120, 120], vec![0; 120 * 120], vec![vec![1.0]; 8], 0);
    t.build_mesh();
    let (mut explore, _) = crate::movement::explore::ExploreAction::new([40.5, 89.5, 0.0]);
    set(&mut explore);
    t.players[1].objects[0].actions.set_explore(Some(explore));
    let r = in_update(&mut t, (1, 0), 50, |unit, c| {
        let me = crate::movement::Me {
            id: unit.id,
            group: unit.group.id,
            action_state: crate::movement::explore::WALK,
        };
        unit.explore_part(c, me)
    });
    let v = &t.players[1].objects[0];
    let heard = v.ai.as_ref().map_or_else(Vec::new, |ai| {
        ai.notifies.iter().map(|n| (n.kind, n.p)).collect()
    });
    (r, v.actions.explore().copied(), heard)
}

/// The Explore's `MoveTo` shows its sprite as it starts (`0x140a87e30` gives `FUN_140aa0d30`
/// the master's walking sprite, an Explore having no task; `0x140aa29e0`'s `set_state(4)` shows
/// it before the motion step).
#[test]
fn an_explore_s_move_to_shows_the_master_s_walking_sprite_as_it_starts() {
    let mut t = forage_world();
    t.pathing = Pathing::new([120, 120], vec![0; 120 * 120], vec![vec![1.0]; 8], 0);
    t.build_mesh();
    let walking = Graphic {
        id: 31,
        frames: 10,
        facets: 16,
        speed_multiplier: 1.0,
        frame_duration: 0.1,
        replay_delay: 0.0,
        flags: 1,
    };
    let _ = t.graphics.insert(31, walking);
    let master = t.players[1].objects[0].life.master_id;
    t.masters[1].get_mut(master).unwrap().move_sprite = 31;
    let (explore, _) = crate::movement::explore::ExploreAction::new([40.5, 89.5, 0.0]);
    t.players[1].objects[0].actions.set_explore(Some(explore));
    let r = in_update(&mut t, (1, 0), 50, |unit, c| {
        let me = crate::movement::Me {
            id: unit.id,
            group: unit.group.id,
            action_state: crate::movement::explore::WALK,
        };
        unit.explore_part(c, me)
    });
    assert_eq!(r, Ok(()));
    let v = &t.players[1].objects[0];
    assert_eq!(
        v.actions.explore().map(|e| e.move_to),
        Some(Some(MoveTo::Moving))
    );
    assert_eq!(v.sprite.map(|s| s.graphic.id), Some(31));
}

/// What [`explored`] gives: the part's result, the Explore after it and the notifies heard.
type Explored = (
    Result<(), GenieError>,
    Option<crate::movement::explore::ExploreAction>,
    Vec<(i32, [i32; 3])>,
);

/// The Explore's part of the update (`0x140a880c0`): in state 11 its quiet `MoveTo` to the
/// point starts (code 0: it walks on). A `MoveTo` that ended, or one that failed (code 1 or 3),
/// makes it wait for a new point (state 3): the unit stops and its unit AI hears 0x202 with the
/// explore action value 605. A waiting Explore does nothing; a done (1) or failed (13) one is
/// not modelled.
#[test]
fn an_explore_walks_to_its_point_then_asks_for_the_next() {
    use crate::movement::explore::{WAIT, WALK, WANTS_POINT};
    let (r, e, heard) = explored(|_| ());
    assert_eq!(r, Ok(()));
    let e = e.unwrap();
    assert_eq!((e.state, e.move_to), (WALK, Some(MoveTo::Moving)));
    assert_eq!(heard, []);
    let asked = vec![(WANTS_POINT, [ActionKind::Explore.value(), 0, 0])];
    for end in [MoveTo::Arrived, MoveTo::Failed] {
        let (r, e, heard) = explored(|e| e.move_to = Some(end));
        assert_eq!(r, Ok(()));
        let e = e.unwrap();
        assert_eq!((e.state, e.move_to), (WAIT, None));
        assert_eq!(heard, asked);
    }
    let (r, e, heard) = explored(|e| {
        let _ = e.set_state(WAIT);
    });
    assert_eq!((r, e.map(|e| e.state), heard), (Ok(()), Some(WAIT), vec![]));
    for (state, reason) in [
        (
            crate::movement::explore::DONE,
            "a done Explore (the action list after it)",
        ),
        (
            crate::movement::explore::FAILED,
            "a failed Explore (notify 0x1f9, set_state 2)",
        ),
    ] {
        let (r, _, _) = explored(|e| e.state = state);
        assert_eq!(r.unwrap_err().reason(), reason);
    }
}

/// A hunter back from its drop-off without a deer (`hunt.target` -1, the deer it hunted let
/// go): both deer seen by player 1, the hunter with a sight of 4, the mesh built. `setup` changes
/// the hunter before [`Unit::hunt_after_drop`] runs.
fn hunted_without_a_deer(setup: impl FnOnce(&mut Unit)) -> World {
    let mut t = hunt_world();
    t.spatial.relations = vec![vec![0, 3], vec![3, 0]];
    t.spatial.unit_diplomacy = vec![vec![1, 4], vec![4, 1]];
    for masters in &mut t.masters {
        let m = masters.get_mut(122).unwrap();
        (m.line_of_sight, m.search_radius) = (4.0, 4.0);
    }
    t.hunt_command(1, 0, 500).unwrap();
    t.build_mesh();
    let mut r = crate::vision::registry::Registry::new(2, 65);
    for (id, x) in [(500, 32.5), (501, 36.5)] {
        r.entries.entry(id).or_default().mask = 1 << 1;
        r.update_unit(id, 0, [x, 89.5], [0.2, 0.2], 9, 0, 1 << 1)
            .unwrap();
    }
    let sight = t.spatial.sight.as_mut().unwrap();
    sight.registry = Some(r);
    for id in [500, 501] {
        let deer = Seer {
            player: 0,
            master: 65,
            los: 2.0,
            state: 2,
            explored: false,
            building: None,
            umv: 0,
            capture: 0,
            kind: 70,
        };
        sight.add_seer(id, deer);
    }
    t.players[0].objects[0].workers = 0;
    let v = &mut t.players[1].objects[0];
    let at = v.position();
    v.body.mover_mut().unwrap().moved_at = at;
    v.actions.hunt_mut().unwrap().target = -1;
    v.ai.as_mut().unwrap().target_type = -1;
    setup(v);
    in_update(&mut t, (1, 0), 50, |unit, c| {
        let code = unit.hunt_after_drop(c)?;
        unit.end_head_update(c, code)
    })
    .unwrap();
    t
}

/// The hunt's state, target and drop site after [`hunted_without_a_deer`], and the two deer's
/// workers.
fn hunt_after(t: &World) -> (Option<(u8, i32, i32)>, [i32; 2]) {
    let h = t.players[1].objects[0].actions.hunt().copied();
    let deer = &t.players[0].objects;
    (
        h.map(|h| (h.state, h.target, h.carry_to)),
        [deer[0].workers, deer[1].workers],
    )
}

/// Without a deer after the drop the hunter's `ClosestObject` over its search (class 9, every
/// relation) finds the nearer deer, which the hunt takes and attacks (`FUN_140ac7a00`,
/// `LAB_140ac9509`'s other way).
#[test]
fn a_hunter_without_its_deer_after_the_drop_takes_the_closest_one() {
    let t = hunted_without_a_deer(|_| ());
    assert_eq!(
        hunt_after(&t),
        (Some((crate::gathering::hunt::ATTACK, 500, -1)), [1, 0])
    );
}

/// A queued order on a target the world still holds ends the hunt after the drop, so the unit AI
/// runs it; one on a target gone leaves the queue (`FUN_140c5de60`) and the search runs.
#[test]
fn a_hunter_with_a_queued_order_after_the_drop_ends_its_hunt() {
    let order = |target: i32| crate::unit_ai::QueuedOrder {
        issuer: 1,
        order: OrderKind::Hunt,
        priority: 100,
        target,
        target_owner: 0,
        point: [36.5, 89.5, 0.0],
        range: 0.0,
        shown: [36.5, 89.5],
        ready: true,
        target_type: 9,
    };
    let held = hunted_without_a_deer(|v| v.ai.as_mut().unwrap().orders = vec![order(501)]);
    assert_eq!(hunt_after(&held), (None, [0, 0]));
    let gone = hunted_without_a_deer(|v| v.ai.as_mut().unwrap().orders = vec![order(999)]);
    assert_eq!(
        gone.players[1].objects[0].ai.as_ref().unwrap().orders.len(),
        0
    );
    assert_eq!(
        hunt_after(&gone),
        (Some((crate::gathering::hunt::ATTACK, 500, -1)), [1, 0])
    );
}

/// A hunt of class 10 searches nothing after the drop, and a wood search finds no tree: either
/// walks back to its point to search again (`set_state(11)`).
#[test]
fn a_hunter_that_finds_nothing_after_the_drop_walks_to_its_point() {
    for class in [10, 15] {
        let t = hunted_without_a_deer(|v| v.actions.hunt_mut().unwrap().target_class = class);
        assert_eq!(
            hunt_after(&t),
            (Some((crate::gathering::hunt::WALK_SEARCH, -1, -1)), [0, 0]),
            "class {class}"
        );
        let h = t.players[1].objects[0].actions.hunt().copied().unwrap();
        assert_eq!(h.point, [32.5, 89.5, 0.0]);
    }
}

/// How often player `p` watches object `id`.
fn watches(t: &World, p: usize, id: i32) -> usize {
    t.players[p].watched.iter().filter(|w| w.id == id).count()
}

/// A site's state, hit points and build points.
fn site_of(u: &Unit) -> (u8, f32, Option<f32>) {
    (u.life.state, u.life.hp, u.construction.map(|c| c.progress))
}

/// [`house_builder`]'s house 10028 with an annex: a second house foundation 10030 (player 1's
/// last object) in its owner's pending tracker list, linked to the house as the game links an
/// annex (`+0xbc0`, `+0xbb8`).
fn annexed_house() -> World {
    let mut t = house_builder();
    let mut annex = t.players[1].objects.by_id(10028).unwrap().clone();
    annex.id = 10030;
    annex.annex_of = Some(10028);
    t.players[1].objects.push(annex);
    let placed = *t.spatial.get(10028).unwrap();
    t.spatial.insert(10030, placed);
    t.spatial.set_annex(10030, Some(10028));
    let k = crate::players::tracker::key(t.masters.get(1), &t.lines, 70);
    t.players[1].tracker.add(10030, k);
    t
}

/// Where houses `a` and `b` stand in their owner's done tracker list of master 70.
fn done_at(t: &World, a: i32, b: i32) -> (Option<usize>, Option<usize>) {
    let k = crate::players::tracker::key(t.masters.get(1), &t.lines, 70);
    let done = t.players[1].tracker.list(k.master);
    let at = |id| done.iter().position(|&d| d == id);
    (at(a), at(b))
}

/// A building that completes in its own update takes its annexes to state 2 first (its
/// `set_state(2)`, `FUN_1411dbb70`: their slot 0xa0 before its own completion): the annex's
/// completion reaches the world at once, its tracker part before the building's.
#[test]
fn a_building_s_own_completion_completes_its_annexes_first() {
    let mut t = annexed_house();
    let h = t.players[1].objects.by_id_mut(10028).unwrap();
    let k = h.construction.as_mut().unwrap();
    k.progress = f32::from(k.build_time);
    let i = t.players[1].objects.position_of(10028).unwrap();
    let seen = in_update(&mut t, (1, i), 50, |unit, c| {
        unit.complete_on_update(c)?;
        Ok(c.w.object(10030).map(|(_, a)| a.life.state))
    });
    assert_eq!(seen.map_err(|e| e.reason().to_owned()), Ok(Some(2)));
    let (annex, house) = done_at(&t, 10030, 10028);
    assert!(annex.is_some() && annex < house, "{annex:?} {house:?}");
}

/// [`house_builder`]'s forager after it built town centre 700 (its Build's target in state 224,
/// the end of the work, its `MoveTo` gone; the centre built) on a map with a tile planner, its
/// update with `setup` run first: the work it found (slot 0x7e8's target and point, the
/// caller's after the update) and the notifies its unit AI heard.
fn built_the_centre(setup: impl FnOnce(&mut World)) -> (World, BuildSearched) {
    let mut t = house_builder();
    t.pathing = Pathing::new([120, 120], vec![0; 120 * 120], vec![vec![1.0]; 8], 0);
    t.build_mesh();
    for masters in &mut t.masters {
        let bush = masters.get_mut(59).unwrap();
        (bush.auto_gather, bush.gather_group) = (true, 1);
        masters.get_mut(120).unwrap().line_of_sight = 4.0;
        masters.get_mut(109).unwrap().line_of_sight = 16.0;
    }
    t.drop_sites[0].auto_work = true;
    let b = t.players[1].objects[0].actions.build_mut().unwrap();
    (b.target, b.move_to, b.state) = (700, None, crate::construction::build::FINISHED);
    let centre = &mut t.players[1].objects[2];
    centre.workers = 1;
    centre.construction = Some(Construction {
        progress: 25.0,
        desolid: false,
        builders: 0,
        hit_points: 2400,
        build_time: 25,
        record: crate::map::spatial::Obstruction { kind: 2, class: 2 },
        snow: false,
        sprite_override: false,
        started: false,
    });
    setup(&mut t);
    let r = in_update(&mut t, (1, 0), 50, |unit, c| {
        let me = crate::movement::Me {
            id: unit.id,
            group: unit.group.id,
            action_state: unit.top_action().map_or(0, |a| a.state),
        };
        let _ = unit.build_part(c, me)?;
        let heard = unit.ai.as_ref().unwrap().notifies.iter();
        Ok((
            c.work_order.take().map(|w| (w.target, w.point)),
            heard.map(|n| n.kind).collect(),
        ))
    });
    (t, r)
}

/// What [`built_the_centre`]'s search left: the work command (its target and point) and the
/// notifies' kinds.
type BuildSearched = Result<(Option<(i32, [f32; 3])>, Vec<i32>), GenieError>;

/// The search for more work around the town centre a builder finished (`0x140ab0b43`,
/// [`Unit::more_work`], within the Build's update): the table's entry for the centre (auto
/// work) takes berries, within the centre's line of sight (16 tiles) the bush at 32.5 is the
/// nearest, and the unit AI's `ClosestResourceObject` names it: the work command on it at its
/// place. An entry without auto work, or a line of sight below the floor (the reach 4 tiles,
/// short of the bushes), finds nothing: the Build waits without a target, lets the centre go and
/// asks its unit AI for a search.
#[test]
fn a_builder_finds_more_work_around_the_building_it_finished() {
    let (t, found) = built_the_centre(|_| ());
    assert_eq!(found, Ok((Some((600, [32.5, 89.5, 0.0])), vec![])));
    assert_eq!(
        t.players[1].objects[0].actions.build().map(|b| b.target),
        Some(700)
    );
    let off: fn(&mut World) = |t| t.drop_sites[0].auto_work = false;
    let short: fn(&mut World) = |t| {
        for masters in &mut t.masters {
            masters.get_mut(109).unwrap().line_of_sight = 0.0;
        }
    };
    for setup in [off, short] {
        let (t, found) = built_the_centre(setup);
        assert_eq!(found, Ok((None, vec![SEARCH_NEEDED])));
        let b = t.players[1].objects[0].actions.build().copied().unwrap();
        assert_eq!((b.state, b.target), (crate::construction::build::IDLE, -1));
        assert_eq!(t.players[1].objects[2].workers, 0);
    }
}

/// [`house_builder`]'s forager at work on foundation `site` (the Build's state 7, counted among
/// its builders, with a task of rate and value 1), the house counting one builder, `setup` run
/// first: one update of `dt` milliseconds.
fn worked(t: &mut World, site: i32, dt: u32) -> Result<(), GenieError> {
    let b = t.players[1].objects[0].actions.build_mut().unwrap();
    (b.target, b.state, b.added, b.move_to) = (site, crate::construction::build::PLACE, true, None);
    b.task = BuildTask {
        range: 0.1,
        work_value: 1.0,
        work_rate: 1.0,
        resource: -1,
        id: -1,
        moving: None,
    };
    t.players[1]
        .objects
        .by_id_mut(10028)
        .and_then(|h| h.construction.as_mut())
        .unwrap()
        .builders = 1;
    in_update(t, (1, 0), dt, |unit, c| {
        let me = crate::movement::Me {
            id: unit.id,
            group: unit.group.id,
            action_state: unit.top_action().map_or(0, |a| a.state),
        };
        unit.build_part(c, me).map(drop)
    })
}

/// The first work on a desolid foundation starts its construction (`FUN_1411e5670`): its owner
/// watches it once, within the builder's update, and the start's flag clears.
#[test]
fn a_builder_s_first_work_starts_the_site_its_owner_watches() {
    let mut t = house_builder();
    assert_eq!(
        worked(&mut t, 10028, 50).map_err(|e| e.reason().to_owned()),
        Ok(())
    );
    let k = t.players[1]
        .objects
        .by_id(10028)
        .unwrap()
        .construction
        .unwrap();
    assert_eq!((k.desolid, k.started), (false, false));
    assert_eq!(watches(&t, 1, 10028), 1);
}

/// The work on an annex (started) whose main building (in state 0, 1 hit point) takes it
/// (`FUN_1411cc430`'s recursion, [`World::work_building`] with the builder lent): the house gets
/// the 2.5 seconds, its build points and its share of hit points, within the builder's update.
#[test]
fn a_builder_s_work_on_an_annex_goes_to_its_main_building() {
    let mut t = annexed_house();
    // the annex started: it clears no foundation under it (FUN_1411cd0d0) on the house's start
    let annex = t.players[1].objects.by_id_mut(10030).unwrap();
    annex.construction.as_mut().unwrap().desolid = false;
    assert_eq!(
        worked(&mut t, 10030, 2500).map_err(|e| e.reason().to_owned()),
        Ok(())
    );
    let house = t.players[1].objects.by_id(10028).unwrap();
    assert_eq!(site_of(house), (0, 56.0, Some(2.5)));
    let annex = t.players[1].objects.by_id(10030).unwrap();
    assert_eq!(site_of(annex).2, Some(0.0));
}

/// The work that completes the house (`Construction::complete`) gives its parts at once within
/// the builder's update: the house stands in state 2, leaves its pending tracker list for its
/// done one, its owner gains its paid cost (attributes 98 and 165, `World::built`); then the
/// builder's work ends (`set_state(224)`), out of the builders.
#[test]
fn a_builder_s_completion_reaches_the_world_within_its_update() {
    let mut t = house_builder();
    let h = t.players[1].objects.by_id_mut(10028).unwrap();
    h.life.hp = 549.0;
    let k = h.construction.as_mut().unwrap();
    (k.progress, k.desolid) = (24.99, false);
    assert_eq!(
        worked(&mut t, 10028, 50).map_err(|e| e.reason().to_owned()),
        Ok(())
    );
    let house = t.players[1].objects.by_id(10028).unwrap();
    assert_eq!((house.life.state, house.repairers()), (2, 0));
    let k = crate::players::tracker::key(t.masters.get(1), &t.lines, 70);
    assert!(t.players[1].tracker.list(k.master).contains(&10028));
    let attributes = &t.players[1].attributes;
    assert_eq!((attributes[98], attributes[165]), (25.0, 25.0));
    let b = t.players[1].objects[0].actions.build().copied().unwrap();
    assert_eq!(
        (b.state, b.added),
        (crate::construction::build::FINISHED, false)
    );
}

/// A builder's completion of its foundation takes the foundation's annexes to state 2 where it
/// runs, before the foundation's tracker part (`FUN_1411dbb70`).
#[test]
fn a_builder_s_completion_completes_the_foundation_s_annexes_where_it_runs() {
    let mut t = annexed_house();
    let h = t.players[1].objects.by_id_mut(10028).unwrap();
    let k = h.construction.as_mut().unwrap();
    (k.progress, k.desolid) = (24.99, false);
    assert_eq!(
        worked(&mut t, 10028, 50).map_err(|e| e.reason().to_owned()),
        Ok(())
    );
    let annex = t.players[1].objects.by_id(10030).unwrap();
    assert_eq!(annex.life.state, 2);
    let (annex, house) = done_at(&t, 10030, 10028);
    assert!(annex.is_some() && annex < house, "{annex:?} {house:?}");
}

/// A building's `set_state` past 3 takes its annexes along where it runs (`FUN_1411dbb70`,
/// [`Unit::annexes_follow`]): the annex holds the state within the building's update.
#[test]
fn a_building_s_later_state_reaches_its_annexes_where_it_runs() {
    let mut t = annexed_house();
    let i = t.players[1].objects.position_of(10028).unwrap();
    let seen = in_update(&mut t, (1, i), 0, |unit, c| {
        unit.annexes_follow(c, 3, 7)?;
        Ok(c.w.object(10030).map(|(_, a)| a.life.state))
    });
    assert_eq!(seen.map_err(|e| e.reason().to_owned()), Ok(Some(7)));
}

/// What [`completed_house`]'s completion left within the update: the attributes its owner
/// gained and how often it watches the house.
type Completed = (Vec<(i16, f32)>, usize);

/// [`house_builder`]'s house 10028 (player 1's object 1) with its build points at its build
/// time, changed by `setup`, after its own update's completion test
/// ([`Unit::complete_on_update`]): the attributes its owner gains and how often it watches the
/// house within the update.
fn completed_house(setup: fn(&mut Unit)) -> (World, Completed) {
    let mut t = house_builder();
    let h = &mut t.players[1].objects[1];
    let k = h.construction.as_mut().unwrap();
    k.progress = f32::from(k.build_time);
    setup(h);
    let r = in_update(&mut t, (1, 1), 50, |unit, c| {
        let before = attribute_values(c.w, 1);
        unit.complete_on_update(c)?;
        Ok((gained(c.w, 1, &before), watches(c.w, 1, 10028)))
    })
    .unwrap();
    (t, r)
}

/// A building in state 0 whose build points reached its build time completes in its own update
/// (`0x1411c9000`, [`Unit::complete_on_update`]): the house goes to state 2, solid, into its
/// owner's done tracker list; its owner gains the attributes its master gives, and its
/// construction starts in the completion (its owner watches it). Short of its build time it
/// stays a foundation, and an annex outside the walls' branch (class 27, master 70) waits for
/// its main building.
#[test]
fn a_foundation_whose_build_points_reached_its_build_time_completes() {
    let (t, r) = completed_house(|_| ());
    assert_eq!(r, (vec![(98, 25.0), (165, 25.0)], 1));
    let h = &t.players[1].objects[1];
    let k = h.construction.unwrap();
    assert_eq!((h.life.state, k.desolid, k.started), (2, false, false));
    let key = crate::players::tracker::key(t.masters.get(1), &t.lines, 70);
    assert_eq!(t.players[1].tracker.list(key.master), [10028]);
    let short: fn(&mut Unit) = |h| h.construction.as_mut().unwrap().progress = 24.0;
    let annex: fn(&mut Unit) = |h| (h.life.master_id, h.annex_of) = (109, Some(700));
    for setup in [short, annex] {
        let (t, r) = completed_house(setup);
        assert_eq!(r, (vec![], 0));
        assert_eq!(t.players[1].objects[1].life.state, 0);
    }
}

/// A building completing in its own update leaves its owner what the builder's completion
/// leaves (`World::built`, the building lent to the world), within the update: a master that
/// disappears when built is destroyed at once (state 7), and a castle's completion records its
/// history event (castle built, the owner's id as data1).
#[test]
fn a_building_completing_in_its_own_update_leaves_what_a_builder_s_completion_leaves() {
    let finish = |t: &mut World| {
        let h = &mut t.players[1].objects[1];
        let k = h.construction.as_mut().unwrap();
        k.progress = f32::from(k.build_time);
        in_update(t, (1, 1), 50, |unit, c| {
            unit.complete_on_update(c)?;
            let events: Vec<(u8, f32)> = c.w.players[1]
                .history
                .events
                .iter()
                .map(|e| (e.kind, e.data1))
                .collect();
            Ok((unit.life.state, events))
        })
        .map_err(|e| e.reason().to_owned())
    };
    let mut t = house_builder();
    for masters in &mut t.masters {
        let b = masters.get_mut(70).unwrap().building.as_mut().unwrap();
        b.disappears_when_built = true;
    }
    assert_eq!(finish(&mut t), Ok((7, vec![])));
    let mut t = house_builder();
    t.players[1].history.entries.push([0, 0]);
    t.players[1].attributes.resize(250, 0.0);
    for masters in &mut t.masters {
        let castle = Master {
            id: 82,
            ..masters.get(70).unwrap().clone()
        };
        *masters = Masters::new(masters.iter().cloned().chain([castle]));
    }
    t.players[1].objects[1].life.master_id = 82;
    let castle_built = crate::players::history::CASTLE_BUILT;
    assert_eq!(finish(&mut t), Ok((2, vec![(castle_built, 1.0)])));
}

/// A desolid foundation that dies (`FUN_1411dbb70` to state 3 from state 0) starts its
/// construction ([`Unit::foundation_died`], [`Construction::begin`]): its owner watches it at
/// once, the foundation lent to the world within its own update, and the start's flag clears.
#[test]
fn a_dying_desolid_foundation_s_owner_watches_it_within_its_update() {
    let mut t = house_builder();
    for masters in &mut t.masters {
        let house = masters.get_mut(70).unwrap();
        house.death_sprite = -1;
        if let Some(b) = house.building.as_mut() {
            b.destruction_sprite = -1;
        }
    }
    let k = t.players[1].objects[1].construction.as_mut().unwrap();
    k.desolid = true;
    let seen = in_update(&mut t, (1, 1), 0, |unit, c| {
        unit.life.state = 3;
        unit.died(c, 0)?;
        let started = unit.construction.map(|k| k.started);
        Ok((watches(c.w, 1, 10028), started))
    });
    assert_eq!(
        seen.map_err(|e| e.reason().to_owned()),
        Ok((1, Some(false)))
    );
}

/// [`house_builder`]'s town centre 700 (master 109: 2400 hit points, a garrison heal rate of 2)
/// at world time 3000 with the forager (master 120: 25 hit points, at 10) inside, changed by
/// `setup`, after the centre's whole seconds ([`Unit::building_seconds`]): the forager's hit
/// points and the centre's garrison time.
fn centre_seconds(setup: impl FnOnce(&mut World)) -> Result<(f32, u32), GenieError> {
    use crate::objects::masters::BuildingMaster;
    let mut t = house_builder();
    for masters in &mut t.masters {
        let centre = masters.get_mut(109).unwrap();
        centre.hit_points = 2400;
        centre.building = Some(BuildingMaster {
            garrison_heal_rate: 2.0,
            ..BuildingMaster::default()
        });
        masters.get_mut(120).unwrap().hit_points = 25;
    }
    t.world_time = 3000;
    t.players[1].objects[2].garrison = vec![9175];
    t.players[1].objects[0].life.hp = 10.0;
    setup(&mut t);
    in_update(&mut t, (1, 2), 50, Unit::building_seconds)?;
    let hp = t.players[1].objects[0].life.hp;
    Ok((hp, t.players[1].objects[2].garrison_time))
}

/// A built building's whole seconds (`0x1411c9000`): since its garrison time, which moves on by
/// them, the units inside below their master's hit points heal by its garrison heal rate; no
/// whole second, a unit at its hit points, or a building not built heals nothing. Not modelled:
/// a building at most a fifth of its hit points (it lets its garrison out), one attribute 255
/// names, a unit inside whose raider countdown runs, and villagers inside repairing a damaged
/// building.
#[test]
fn a_building_heals_its_garrison_each_whole_second() {
    let reason = |r: Result<(f32, u32), GenieError>| r.unwrap_err().reason();
    assert_eq!(centre_seconds(|_| ()), Ok((12.0, 3000)));
    assert_eq!(centre_seconds(|t| t.world_time = 999), Ok((10.0, 0)));
    assert_eq!(
        centre_seconds(|t| t.players[1].objects[0].life.hp = 25.0),
        Ok((25.0, 3000))
    );
    assert_eq!(
        centre_seconds(|t| t.players[1].objects[2].life.state = 0),
        Ok((10.0, 0))
    );
    assert_eq!(
        reason(centre_seconds(|t| t.players[1].objects[2].life.hp = 480.0)),
        "a damaged building lets its garrison out (a fifth of its hit points)"
    );
    assert_eq!(
        reason(centre_seconds(|t| {
            t.players[1].attributes.resize(256, 0.0);
            t.players[1].attributes[255] = 109.0;
        })),
        "a building whose master attribute 255 names (FUN_1411c39b0)"
    );
    assert_eq!(
        reason(centre_seconds(
            |t| t.players[1].objects[0].raider_countdown = 1
        )),
        "a unit inside whose raider_build_countdown runs (FUN_1411e7220)"
    );
    let repairs = |t: &mut World| {
        for masters in &mut t.masters {
            let b = masters.get_mut(109).unwrap().building.as_mut().unwrap();
            b.garrison_repair_rate = 1.0;
        }
        t.players[1].objects[2].life.hp = 2000.0;
    };
    assert_eq!(
        reason(centre_seconds(repairs)),
        "a building its villagers inside repair (FUN_1411d33e0)"
    );
}

/// A wall's or a house's update takes the walls' branch (`0x1411c9000`, [`walls_branch`]): no
/// whole seconds, so its garrison time stays and nothing inside heals.
///
/// [`walls_branch`]: crate::construction::building_update::walls_branch
#[test]
fn a_wall_or_a_house_counts_no_whole_seconds() {
    use crate::construction::building_update::walls_branch;
    let wall = |t: &mut World| {
        for masters in &mut t.masters {
            masters.get_mut(109).unwrap().class = 27;
        }
    };
    assert_eq!(centre_seconds(wall), Ok((10.0, 0)));
    let master = |id, class| Master {
        id,
        class,
        ..Master::default()
    };
    assert!(walls_branch(&master(70, 3)));
    assert!(walls_branch(&master(117, 27)));
    assert!(!walls_branch(&master(109, 3)));
}

/// [`repair_world`] on [`open_map`] with the repairer 9175 (a mover) at `at` and the house
/// 10028 at (30, 88) placed, the player holding 100 of each resource, the mesh built.
pub(crate) fn repairing(at: [f32; 3]) -> World {
    let mut t = repair_world();
    open_map(&mut t);
    t.spatial.relations = vec![vec![0, 3], vec![3, 0]];
    let placed = |position, class, master, radius| Placed {
        position,
        radius,
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
    t.spatial.insert(9175, placed(at, 4, 118, [0.2, 0.2]));
    t.spatial
        .insert(10028, placed([30.0, 88.0, 0.0], 3, 70, [1.0, 1.0]));
    let v = &mut t.players[1].objects[0];
    v.place_for_test(at);
    let mut mover = crate::gathering::hunt::tests::walker();
    mover.moved_at = at;
    v.body.put_mover(Some(mover));
    t.players[1].resources = [100.0; 4];
    t.build_mesh();
    t
}

/// The work command on the house, then `n` of the repairer's repair parts
/// ([`Unit::repair_part`], 50 ms each): the Repair's states in turn (None once it ended).
fn repair_for(t: &mut World, n: usize) -> Vec<Option<u8>> {
    let work = crate::orders::Work {
        target: 10028,
        point: [30.0, 88.0, 0.0],
        carry_test: false,
        filter: -1,
        notified: true,
    };
    t.order_work(1, 0, &work).unwrap();
    let mut states = vec![t.players[1].objects[0].actions.repair().map(|r| r.state)];
    for _ in 0..n {
        in_update(t, (1, 0), 50, |unit, c| {
            let me = crate::movement::Me {
                id: unit.id,
                group: unit.group.id,
                action_state: unit.top_action().map_or(0, |a| a.state),
            };
            let code = unit.repair_part(c, me)?;
            unit.end_head_update(c, code)
        })
        .unwrap();
        let state = t.players[1].objects[0].actions.repair().map(|r| r.state);
        if states.last() != Some(&state) {
            states.push(state);
        }
    }
    states
}

/// A repairer that arrives out of the Repair's reach of the house (0.3 tiles off it) cannot
/// repair: the reach test fails and the quick path finds no way closer, so the Repair fails
/// (state 13), lets the house go, tells its unit AI (0x1f9 with the repair action) and ends; the
/// house keeps its 300 hit points and nothing is paid. Moved within reach (its footprint touching
/// the house's), the same work command repairs: it faces the house (6), joins its repairers
/// and repairs (7), the house gaining hit points.
#[test]
fn a_repairer_out_of_reach_fails_and_one_within_reach_repairs() {
    let mut t = repairing([29.5, 89.5, 0.0]);
    assert_eq!(repair_for(&mut t, 10), [Some(4), Some(13), None]);
    let house = &t.players[1].objects[1];
    assert_eq!(
        (house.life.hp, house.repairers(), house.workers),
        (300.0, 0, 0)
    );
    let heard = &t.players[1].objects[0].ai.as_ref().unwrap().notifies;
    assert_eq!(
        heard.iter().map(|n| (n.kind, n.p)).collect::<Vec<_>>(),
        [(
            crate::unit_ai::ACTION_FAILED,
            [ActionKind::Repair.value(), 0, 0]
        )]
    );
    let at = [30.0, 89.2, 0.0];
    let p = *t.spatial.get(9175).unwrap();
    t.spatial.insert(9175, Placed { position: at, ..p });
    let v = &mut t.players[1].objects[0];
    v.place_for_test(at);
    v.body.mover_mut().unwrap().moved_at = at;
    assert_eq!(repair_for(&mut t, 400), [Some(4), Some(6), Some(7)]);
    let house = &t.players[1].objects[1];
    assert_eq!(
        (house.life.hp, house.repairers(), house.workers),
        (309.922_58, 1, 1)
    );
    assert_eq!(t.players[1].resources, [100.0; 4]);
}

/// A repair pays from what its owner holds when it pays, within the update (`FUN_141618db0` on
/// the player's attributes): wood spent earlier in the same update is not there to pay with, so
/// the house gains no hit points and the wood stays at 0.
#[test]
fn a_repair_pays_from_what_its_owner_holds_at_the_payment() {
    let mut t = repairing([30.0, 89.2, 0.0]);
    t.players[1].attributes.resize(300, 0.0);
    t.players[1].attributes[crate::construction::repair::BUILDING_REPAIR_COST] = 1.0;
    assert_eq!(repair_for(&mut t, 100), [Some(4), Some(6), Some(7)]);
    let hp = t.players[1].objects[1].life.hp;
    in_update(&mut t, (1, 0), 50, |unit, c| {
        c.w.players[1].resources[1] = 0.0;
        let me = crate::movement::Me {
            id: unit.id,
            group: unit.group.id,
            action_state: unit.top_action().map_or(0, |a| a.state),
        };
        let code = unit.repair_part(c, me)?;
        unit.end_head_update(c, code)
    })
    .unwrap();
    let house = &t.players[1].objects[1];
    assert_eq!((house.life.hp, t.players[1].resources[1]), (hp, 0.0));
}

/// The Repair's payment, its end and another player's target ([`Unit::repair_part`]): with a
/// building repair cost (attribute 271) the repair pays the share of the building's costs its
/// hit points make (the house's in wood, a castle's in stone); a repair that
/// takes the house to its hit points ends (state 1, then the action goes) with the house's armor
/// back, the house let go and out of its repairers, the unit AI told 0x1fa; an ally's house takes
/// its hit points and its repairer at once, through the update's view ([`Others::get_mut`]).
#[test]
fn a_repair_pays_ends_at_full_hit_points_and_reaches_an_ally_s_house() {
    let near = [30.0, 89.2, 0.0];
    let mut t = repairing(near);
    t.players[1].attributes.resize(300, 0.0);
    t.players[1].attributes[crate::construction::repair::BUILDING_REPAIR_COST] = 1.0;
    assert_eq!(repair_for(&mut t, 100), [Some(4), Some(6), Some(7)]);
    assert_eq!(t.players[1].objects[1].life.hp, 302.424_4);
    assert_eq!(t.players[1].resources, [100.0, 99.889_73, 100.0, 100.0]);
    // a castle (650 stone, 4800 hit points) pays its share in stone
    let mut t = repairing(near);
    for masters in &mut t.masters {
        let house = masters.get(70).unwrap().clone();
        let castle = Master {
            id: 82,
            hit_points: 4800,
            cost: vec![(2, 650, true)],
            ..house
        };
        *masters = Masters::new(masters.iter().cloned().chain([castle]));
    }
    let castle = &mut t.players[1].objects[1];
    (castle.life.master_id, castle.life.hp) = (82, 3000.0);
    let p = *t.spatial.get(10028).unwrap();
    t.spatial.insert(10028, Placed { master: 82, ..p });
    t.players[1].attributes.resize(300, 0.0);
    t.players[1].attributes[crate::construction::repair::BUILDING_REPAIR_COST] = 1.0;
    assert_eq!(repair_for(&mut t, 100), [Some(4), Some(6), Some(7)]);
    assert_eq!(t.players[1].objects[1].life.hp, 3_002.415_5);
    assert_eq!(t.players[1].resources, [100.0, 100.0, 99.671_42, 100.0]);
    let mut t = repairing(near);
    let house = &mut t.players[1].objects[1];
    (house.life.hp, house.armor_removed) = (549.0, [3, 4]);
    let states = repair_for(&mut t, 100);
    assert_eq!(states, [Some(4), Some(6), Some(7), Some(1), None]);
    let house = &t.players[1].objects[1];
    assert_eq!(
        (
            house.life.hp,
            house.armor_removed,
            house.repairers(),
            house.workers
        ),
        (550.0, [0, 0], 0, 0)
    );
    let heard = &t.players[1].objects[0].ai.as_ref().unwrap().notifies;
    assert_eq!(
        heard.iter().map(|n| (n.kind, n.p)).collect::<Vec<_>>(),
        [(0x1fa, [ActionKind::Repair.value(), 0, 0])]
    );
    // player 0's house, the players allied: repaired, then repaired to full
    let allied = |hp: f32| {
        let mut t = repairing(near);
        let mut house = t.players[1].objects.remove(1);
        (house.life.hp, house.armor_removed, house.workers) = (hp, [3, 4], 0);
        t.players[0].objects.push(house);
        t.players[0].attributes = vec![0.0; 300];
        t.spatial.relations = vec![vec![0, 0], vec![0, 0]];
        for p in &mut t.players {
            p.relations = vec![0, 0];
        }
        let p = *t.spatial.get(10028).unwrap();
        t.spatial.insert(10028, Placed { player: 0, ..p });
        let states = repair_for(&mut t, 100);
        let h = &t.players[0].objects[0];
        (states, h.life.hp, h.armor_removed, h.repairers(), h.workers)
    };
    assert_eq!(
        allied(300.0),
        (vec![Some(4), Some(6), Some(7)], 302.424_4, [3, 4], 1, 1)
    );
    assert_eq!(
        allied(549.0),
        (
            vec![Some(4), Some(6), Some(7), Some(1), None],
            550.0,
            [0, 0],
            0,
            0
        )
    );
}

/// A repair of an annex (house 10028 of the main building 10029, both at 300 hit points) is its
/// main building's (`FUN_1411d37d0`): the main building repairs its annexes by the amount first,
/// then itself, so both gain it; the repairer counts among the main building's repairers
/// (`FUN_140ad9050`).
#[test]
fn a_repair_of_an_annex_repairs_its_main_building_and_counts_there() {
    let mut t = repairing([30.0, 89.2, 0.0]);
    let mut main = t.players[1].objects[1].clone();
    main.id = 10029;
    main.place_for_test([34.0, 88.0, 0.0]);
    t.players[1].objects.push(main);
    t.players[1].objects[1].annex_of = Some(10029);
    t.spatial.set_annex(10028, Some(10029));
    let p = *t.spatial.get(10028).unwrap();
    t.spatial.insert(
        10029,
        Placed {
            position: [34.0, 88.0, 0.0],
            ..p
        },
    );
    assert_eq!(repair_for(&mut t, 100), [Some(4), Some(6), Some(7)]);
    let [annex, main] = [&t.players[1].objects[1], &t.players[1].objects[2]];
    assert_eq!(
        (
            annex.life.hp,
            main.life.hp,
            annex.repairers(),
            main.repairers()
        ),
        (302.424_4, 302.424_4, 0, 1)
    );
}

/// [`repairing`] with a building repair cost and `n` repairers on the house (9175, then copies
/// 9185 and 9195 at its east and west sides, all within reach), each repairing in turn: the
/// house's hit points and the player's wood after 10 updates (every repairer at work) and after
/// 100 more.
fn repaired_by(n: usize) -> [(f32, f32); 2] {
    let mut t = repairing([30.0, 89.2, 0.0]);
    t.players[1].attributes.resize(300, 0.0);
    t.players[1].attributes[crate::construction::repair::BUILDING_REPAIR_COST] = 1.0;
    for (k, at) in [[31.2, 88.0, 0.0], [28.8, 88.0, 0.0]]
        .into_iter()
        .enumerate()
        .take(n - 1)
    {
        let mut v = t.players[1].objects[0].clone();
        v.id = 9185 + 10 * k as i32;
        v.place_for_test(at);
        v.body.mover_mut().unwrap().moved_at = at;
        let p = *t.spatial.get(9175).unwrap();
        t.spatial.insert(v.id, Placed { position: at, ..p });
        t.players[1].objects.push(v);
    }
    let repairers: Vec<usize> = std::iter::once(0).chain(2..=n).collect();
    let work = crate::orders::Work {
        target: 10028,
        point: [30.0, 88.0, 0.0],
        carry_test: false,
        filter: -1,
        notified: true,
    };
    for &i in &repairers {
        t.order_work(1, i, &work).unwrap();
    }
    let update = |t: &mut World| {
        for &i in &repairers {
            in_update(t, (1, i), 50, |unit, c| {
                let me = crate::movement::Me {
                    id: unit.id,
                    group: unit.group.id,
                    action_state: unit.top_action().map_or(0, |a| a.state),
                };
                let code = unit.repair_part(c, me)?;
                unit.end_head_update(c, code)
            })
            .unwrap();
        }
    };
    let seen = |t: &World| (t.players[1].objects[1].life.hp, t.players[1].resources[1]);
    (0..10).for_each(|_| update(&mut t));
    let before = seen(&t);
    assert_eq!(usize::from(t.players[1].objects[1].repairers()), n);
    (0..100).for_each(|_| update(&mut t));
    [before, seen(&t)]
}

/// More repairers repair faster, but not in proportion (`0x140adaa20` case 6): each of `n`
/// repairs `((n − 1) × 0.5 + 1) / n` of one's work, so two repair 1.5 times as fast as one and
/// three twice as fast. The cost (`FUN_1411d37d0`: the building repair cost times the amount over
/// the master's hit points, of the master's costs) follows the hit points repaired: more
/// repairers pay more wood each second, the same for each hit point (the house's 25 wood over
/// its 550 hit points).
#[test]
fn more_repairers_repair_faster_but_not_in_proportion_and_pay_by_the_hit_point() {
    let gained = |[(hp0, wood0), (hp1, wood1)]: [(f32, f32); 2]| (hp1 - hp0, wood0 - wood1);
    let one = gained(repaired_by(1));
    let two = gained(repaired_by(2));
    let three = gained(repaired_by(3));
    assert_eq!(one, (2.499_389_6, 0.113_677_98));
    assert_eq!(two, (3.747_558_6, 0.170_898_44));
    assert_eq!(three, (4.998_779_3, 0.226_593_02));
    let near = |a: f32, b: f32| (a - b).abs() < 1e-3;
    assert!(near(two.0 / one.0, 1.5) && near(three.0 / one.0, 2.0));
    for (hp, wood) in [one, two, three] {
        assert!(
            near(wood / hp, 25.0 / 550.0),
            "{wood} wood for {hp} hit points"
        );
    }
}

/// A builder's whole Build of house 10028 ([`house_builder`] with the Build's work value 1, a
/// tile planner and the builder's sprites of 8 facets) through its updates (`0x140ab0370`): it
/// walks to the house (state 4, its `MoveTo` moving, then arrived), faces it (6) and works (7)
/// a build point a second until the house's 25 complete it (state 2; the Build's 224); then its
/// search for more work finds nothing around a house (no drop-site entry), so the Build waits
/// without a target (3), lets the house go and asks its unit AI for a search (0x202).
#[test]
fn a_builder_walks_to_its_house_builds_it_and_waits_after() {
    let mut t = house_builder();
    t.pathing = Pathing::new([120, 120], vec![0; 120 * 120], vec![vec![1.0]; 8], 0);
    t.build_mesh();
    let v = &mut t.players[1].objects[0];
    let here = v.position();
    let m = v.body.mover_mut().unwrap();
    m.moved_at = here;
    m.stats.facets = Some(crate::objects::facing::SpriteFacets {
        count: 8,
        flag_1: false,
    });
    v.actions.build_mut().unwrap().task.work_value = 1.0;
    let mut seen = Vec::new();
    for n in 0..520 {
        let _ = in_update(&mut t, (1, 0), 50, Unit::update).unwrap();
        let b = t.players[1].objects[0].actions.build().copied().unwrap();
        let house = t.players[1].objects[1].life.state;
        let now = (b.state, b.move_to, house);
        if seen.last().is_none_or(|&(_, last)| last != now) {
            seen.push((n, now));
        }
    }
    assert_eq!(
        seen,
        [
            (0, (4, Some(MoveTo::Moving), 0)),
            (8, (4, Some(MoveTo::Arrived), 0)),
            (9, (6, None, 0)),
            (10, (7, None, 0)),
            (511, (224, None, 2)),
            (512, (crate::construction::build::IDLE, None, 2)),
        ]
    );
    let house = &t.players[1].objects[1];
    assert_eq!(
        (house.construction.unwrap().progress, house.workers),
        (25.0, 0)
    );
    let v = &t.players[1].objects[0];
    assert_eq!(v.actions.build().copied().unwrap().target, -1);
    let heard = v.ai.as_ref().unwrap().notifies.iter().map(|n| n.kind);
    assert_eq!(heard.collect::<Vec<_>>(), [SEARCH_NEEDED]);
}

/// A loaded builder's drop-off through its updates (`0x140ab0370` case 7, then
/// [`BuildAction::update`]'s state 4): from [`house_builder`]'s house it walks to the town centre
/// its drop-off found (state 8, its quiet `MoveTo` moving, then arrived), gives the player its 6
/// food there and carries nothing, lets the centre go and walks back to the house (4), faces it
/// (6) and builds again (7).
///
/// [`BuildAction::update`]: crate::construction::build::BuildAction::update
#[test]
fn a_loaded_builder_drops_its_food_at_the_centre_and_builds_on() {
    let mut t = house_builder();
    t.pathing = Pathing::new([120, 120], vec![0; 120 * 120], vec![vec![1.0]; 8], 0);
    t.build_mesh();
    let v = &mut t.players[1].objects[0];
    (v.life.attribute_type, v.life.amount) = (17, 6.0);
    let here = v.position();
    let m = v.body.mover_mut().unwrap();
    m.moved_at = here;
    m.stats.facets = Some(crate::objects::facing::SpriteFacets {
        count: 8,
        flag_1: false,
    });
    in_update(&mut t, (1, 0), 50, |unit, c| {
        let mut b = *unit.actions.build().unwrap();
        let by = unit.releaser(c);
        let dropped = b.drop_again(c, unit, by);
        *unit.actions.build_mut().unwrap() = b;
        dropped
    })
    .unwrap();
    let mut seen = Vec::new();
    for n in 0..350 {
        let _ = in_update(&mut t, (1, 0), 50, Unit::update).unwrap();
        let v = &t.players[1].objects[0];
        let b = v.actions.build().copied().unwrap();
        let now = (b.state, b.move_to, b.site, v.life.amount);
        if seen.last().is_none_or(|&(_, last)| last != now) {
            seen.push((n, now));
        }
    }
    let drop = crate::construction::build::DROP;
    assert_eq!(
        seen,
        [
            (0, (drop, Some(MoveTo::Moving), 700, 6.0)),
            (177, (drop, Some(MoveTo::Arrived), 700, 6.0)),
            (178, (4, Some(MoveTo::Start), -1, 0.0)),
            (179, (4, Some(MoveTo::Moving), -1, 0.0)),
            (342, (4, Some(MoveTo::Arrived), -1, 0.0)),
            (343, (6, None, -1, 0.0)),
            (344, (7, None, -1, 0.0)),
        ]
    );
    assert_eq!(t.players[1].resources[0], 6.0);
    assert_eq!(t.players[1].objects[0].life.attribute_type, -1);
}

/// Object `i` of player `p`'s step (dt 50, outside a turn) whose only part is `step`: what it
/// leaves in the `Step`, and its result, which what follows it sees ([`World::step_object`]).
fn finished_with(
    t: &mut World,
    (p, i): (usize, usize),
    step: impl FnOnce(&mut step::Step<'_>) -> Result<crate::objects::unit::Outcome, GenieError>,
) -> Result<crate::objects::unit::Outcome, GenieError> {
    t.step_object((p, i), (50, None, false), None, |_, c| step(c))
        .map(|s| s.outcome)
}

/// What a step leaves in its `Step` for what follows it.
type Orders<'a> = dyn Fn(&mut step::Step<'_>) + 'a;

/// A unit that leaves its list (`Outcome::Leave`) cannot take the orders its unit AI gave in the
/// step: each kind fails with its own gap, and a step without orders just leaves.
#[test]
fn a_unit_that_leaves_its_list_takes_none_of_its_unit_ai_s_orders() {
    use crate::objects::unit::Outcome;
    use crate::unit_ai::world_view::AiWork;
    let work = crate::orders::Work {
        target: 600,
        point: [32.5, 89.5, 0.0],
        carry_test: false,
        filter: -1,
        notified: true,
    };
    let order = crate::unit_ai::QueuedOrder {
        issuer: 1,
        order: OrderKind::OnTarget,
        priority: 100,
        target: 600,
        target_owner: 0,
        point: [32.5, 89.5, 0.0],
        range: 0.0,
        shown: [32.5, 89.5],
        ready: true,
        target_type: 7,
    };
    let aside = crate::unit_ai::AiMove {
        point: [25.0, 89.5, 0.0],
        range: 0.0,
        force: false,
        walk: false,
    };
    let leaving = |orders: &Orders<'_>| {
        let mut t = house_builder();
        finished_with(&mut t, (1, 0), |c| {
            orders(c);
            Ok(Outcome::Leave)
        })
        .map_err(|e| e.reason().to_owned())
    };
    assert_eq!(leaving(&|_| ()), Ok(Outcome::Leave));
    let gap = |reason: &str| Err(reason.to_owned());
    let cases: [(&Orders<'_>, _); 6] = [
        (
            &|c| c.work_order = Some(work),
            gap("a builder's work order of a unit that leaves its list"),
        ),
        (
            &|c| c.works.push(AiWork::Gather(600)),
            gap("a HuntObject or GatherObject of a unit that leaves its list"),
        ),
        (
            &|c| c.move_aside.push(aside),
            gap("a move off a footprint of a unit that leaves its list"),
        ),
        (
            &|c| c.build_command = Some(10028),
            gap(LEAVES_ITS_LIST.reason()),
        ),
        (&|c| c.reseed = Some(true), gap(LEAVES_ITS_LIST.reason())),
        (
            &|c| c.order_on_target = Some((order, None)),
            gap("an order on a target of a unit that leaves its list"),
        ),
    ];
    for (orders, failed) in cases {
        assert_eq!(leaving(orders), failed);
    }
}

/// A dying building lets its annexes go at once (the static update's death, `0x14118ad50`): in
/// the same update its annex, here the town centre, holds no main building and the building's
/// hit points, and the world's annex slots are clear.
#[test]
fn a_dying_building_lets_its_annexes_go_in_its_own_update() {
    use crate::objects::sprite::{Graphic, building_death};
    let mut t = house_builder();
    t.spatial.set_annex(700, Some(10028));
    t.players[1].objects[2].annex_of = Some(10028);
    let house = &t.players[1].objects[1];
    let master = t.masters[1].get(house.life.master_id).unwrap();
    let progress = house.construction.unwrap().progress;
    let id = building_death(master, None, progress).unwrap();
    let shown = Graphic {
        id,
        frames: 10,
        facets: 1,
        speed_multiplier: 1.0,
        frame_duration: 0.1,
        replay_delay: 0.0,
        flags: 0,
    };
    let _ = t.graphics.insert(id, shown);
    let seen = in_update(&mut t, (1, 1), 0, |unit, c| {
        unit.life.hp = 3.0;
        unit.died(c, 1)?;
        let (_, annex) = OthersRef::of(&c.w.players, c.player).find(700).unwrap();
        Ok((annex.annex_of, annex.life.hp, c.w.spatial.annexes_of(10028)))
    });
    assert_eq!(
        seen.map_err(|e| e.reason().to_owned()),
        Ok((None, 3.0, vec![]))
    );
}

/// The unit AI's orders of a step reach the unit and the units they name: a `GatherObject`'s
/// gather command takes the bush, a reseed without a farm lets it go, a build command gives the
/// builder its Build, a drop elsewhere sends the unit to the drop site with the order back to its
/// target queued.
#[test]
fn the_unit_ai_s_orders_reach_the_unit_and_the_units_they_name() {
    use crate::objects::unit::Outcome;
    use crate::unit_ai::world_view::AiWork;
    let stay = |t: &mut World, orders: &Orders<'_>| {
        finished_with(t, (1, 0), |c| {
            orders(c);
            Ok(Outcome::Stay)
        })
        .map_err(|e| e.reason().to_owned())
    };
    let mut t = house_builder();
    assert_eq!(
        stay(&mut t, &|c| c.works.push(AiWork::Gather(600))),
        Ok(Outcome::Stay)
    );
    let gatherer = &t.players[1].objects[0];
    assert_eq!(
        (
            gatherer.top_action_kind(),
            gatherer.actions.gather().map(|g| (g.target, g.state))
        ),
        (Some(crate::gathering::gather::GATHER), Some((600, 4)))
    );
    assert_eq!(t.players[0].objects[0].workers, 1);
    assert_eq!(stay(&mut t, &|c| c.reseed = Some(true)), Ok(Outcome::Stay));
    let gather = t.players[1].objects[0].actions.gather().copied().unwrap();
    assert_eq!((gather.target, gather.state), (-1, 2));
    assert_eq!(t.players[0].objects[0].workers, 0);

    let mut t = house_builder();
    t.players[1].objects[0].actions.set_build(None);
    assert_eq!(
        stay(&mut t, &|c| c.build_command = Some(10028)),
        Ok(Outcome::Stay)
    );
    let builder = &t.players[1].objects[0];
    assert_eq!(
        (
            builder.top_action_kind(),
            builder.actions.build().map(|b| b.target)
        ),
        (Some(crate::construction::build::BUILD), Some(10028))
    );

    let mut t = house_builder();
    let drop = AiWork::DropElsewhere(700, Some(600));
    assert_eq!(stay(&mut t, &|c| c.works.push(drop)), Ok(Outcome::Stay));
    let ai = t.players[1].objects[0].ai.as_ref().unwrap();
    let queued: Vec<_> = ai.orders.iter().map(|o| (o.order, o.target)).collect();
    assert_eq!(
        (ai.current_target, queued),
        (700, vec![(OrderKind::OnTarget, 600)])
    );
}

/// An Enter's transport moves to the meeting point within the entering unit's update (slot
/// 0x340, [`World::ship_move`], the unit lent to the world): the transport's `MoveTo` is there
/// before the update ends; one the world does not hold cannot move. Its sprite is
/// [`an_ai_move_replaces_a_done_attack_with_a_move_to_the_point`]'s.
#[test]
fn an_enter_s_transport_moves_to_the_meeting_point_where_the_unit_runs() {
    let mut t = house_builder();
    let mut ship = t.players[1].objects[0].clone();
    ship.id = 9176;
    ship.actions = ActionList::default();
    t.players[1].objects.push(ship);
    let placed = *t.spatial.get(9175).unwrap();
    t.spatial.insert(9176, placed);
    let meet = [25.0, 89.5, 0.0];
    let moved = |t: &mut World, ship: i32| {
        in_update(t, (1, 0), 50, |unit, c| {
            c.lend(unit, |w| w.ship_move(ship, meet))?;
            let (_, s) = c.w.object(9176).expect("held");
            Ok(s.actions.move_to().map(|m| (m.goal, m.range)))
        })
        .map_err(|e| e.reason().to_owned())
    };
    assert_eq!(moved(&mut t, 9176), Ok(Some((Some(meet), 0.0))));
    let unreached = crate::garrison::UNREACHED.reason().to_owned();
    assert_eq!(moved(&mut t, 4242), Err(unreached));
}

/// The updating unit's own kill with what a razing hit counted: player 1's attribute 98 by
/// each of `adds` (attribute, amount).
fn own_razing(adds: &[(i16, f32)]) -> crate::combat::victims::KillLeft {
    crate::combat::victims::KillLeft {
        id: 9175,
        kill: None,
        razed: vec![crate::combat::damage::Razed {
            attacker_player: 1,
            owner: 1,
            event: None,
            adds: adds.iter().map(|&(a, v)| (1, a, v)).collect(),
        }],
    }
}

/// A step that fails stops what follows it: what its `Step` gathered does not apply (its own
/// kill's razing counts), and the first effect that fails stops the ones after it (a count of
/// a negative attribute stops the next one); a step that holds applies it.
#[test]
fn a_step_that_fails_stops_what_follows_it() {
    use crate::objects::unit::Outcome;
    let mut t = house_builder();
    t.players[1].attributes[98] = 10.0;
    let r = finished_with(&mut t, (1, 0), |c| {
        c.kills.push(own_razing(&[(98, -5.0)]));
        Err(GenieError::unsupported("the step's own gap"))
    });
    assert_eq!(
        r.map_err(|e| e.reason().to_owned()),
        Err("the step's own gap".to_owned())
    );
    assert_eq!(t.players[1].attributes[98], 10.0);

    let mut t = house_builder();
    t.players[1].attributes[98] = 10.0;
    let r = finished_with(&mut t, (1, 0), |c| {
        c.kills.push(own_razing(&[(-1, 5.0), (98, -5.0)]));
        Ok(Outcome::Stay)
    });
    assert_eq!(
        r.map_err(|e| e.reason().to_owned()),
        Err("a negative attribute".to_owned())
    );
    assert_eq!(t.players[1].attributes[98], 10.0);
    let mut t = house_builder();
    t.players[1].attributes[98] = 10.0;
    let r = finished_with(&mut t, (1, 0), |c| {
        c.kills.push(own_razing(&[(98, -5.0)]));
        Ok(Outcome::Stay)
    });
    assert_eq!(r.map_err(|e| e.reason().to_owned()), Ok(Outcome::Stay));
    assert_eq!(t.players[1].attributes[98], 5.0);
}

/// What a completion leaves its owner ([`World::built`]): a pasture puts its livestock on its
/// edges, a building's stack unit is made at its place, a completion's technology is
/// researched, and a building that disappears when built is destroyed. A completed mule cart
/// with nothing to find keeps its state.
#[test]
fn a_completion_s_parts_reach_their_owner() {
    use crate::construction::build::{Built, StackUnit};
    use crate::gathering::pasture::LIVESTOCK;
    use crate::technology::{TechDef, TechState};
    let stay = |t: &mut World, id: i32, built: Built| {
        t.built(1, id, built).map_err(|e| e.reason().to_owned())
    };
    let made = |t: &World| -> Vec<(i32, i16, [f32; 3])> {
        t.players[1].objects[1..]
            .iter()
            .map(|u| (u.id, u.life.master_id, u.position()))
            .collect()
    };
    let mut t = pasture_game([60.0, 60.0, 0.0], 2.0, &LIVESTOCK);
    let pasture = Built {
        pasture: true,
        ..Built::default()
    };
    assert_eq!(stay(&mut t, 50, pasture), Ok(()));
    assert_eq!(
        made(&t),
        [
            (100, 1899, [58.5, 58.5, 0.0]),
            (101, 1899, [59.4, 58.5, 0.0])
        ]
    );
    let mut t = pasture_game([60.0, 60.0, 0.0], 2.0, &LIVESTOCK);
    let stack = StackUnit {
        master: 1899,
        at: [60.0, 60.0, 0.0],
        hp_lost: 0.0,
    };
    let stacked = Built {
        stack: Some(stack),
        ..Built::default()
    };
    assert_eq!(stay(&mut t, 50, stacked), Ok(()));
    assert_eq!(made(&t), [(100, 1899, [60.0, 60.0, 0.0])]);

    let mut t = house_builder();
    t.techs = vec![TechDef::default()];
    t.players[1].techs = vec![TechState {
        state: 1,
        research_cap: -1,
        building_master: -1,
        ..TechState::default()
    }];
    let tech = Built {
        tech: Some(0),
        ..Built::default()
    };
    assert_eq!(stay(&mut t, 10028, tech), Ok(()));
    assert_eq!(t.players[1].techs[0].state, 3);
    let mut t = house_builder();
    let vanishes = Built {
        vanishes: true,
        ..Built::default()
    };
    assert_eq!(stay(&mut t, 10028, vanishes), Ok(()));
    let house = &t.players[1].objects[1].life;
    assert_eq!((house.state, house.hp), (7, 0.0));
    let mut t = house_builder();
    let cart = Built {
        cart: Some(2),
        ..Built::default()
    };
    assert_eq!(stay(&mut t, 9175, cart), Ok(()));
    let cart = &t.players[1].objects[0];
    assert_eq!(
        (cart.life.state, cart.ai.as_ref().map(|a| a.current_target)),
        (2, Some(-1))
    );
}

/// A built building's death in its own update (`FUN_1411dbb70`, state 3 from 2), each part
/// within the update in the game's order: its owner's watched list lets it go, a castle's count
/// (attribute 134, held at 0) and its paid cost (98 alone for a castle, the combat object's
/// `FUN_14121cc40`) leave the owner, and as the last building of its master it withdraws the
/// master's technology from state 3 (`FUN_141166ab0`'s scan over the owner's objects, the
/// castle lent to the world). Another castle standing keeps the technology.
#[test]
fn a_built_building_s_death_reaches_its_owner_within_its_update() {
    use crate::technology::{TechDef, TechState};
    let died = |another: bool| {
        let mut t = enter_world(vec![], 5);
        for masters in &mut t.masters {
            let mut castle = Master {
                id: 82,
                death_sprite: -1,
                cost: vec![(2, 650, true)],
                ..masters.get(70).unwrap().clone()
            };
            let b = castle.building.as_mut().unwrap();
            (b.tech, b.destruction_sprite) = (0, -1);
            *masters = Masters::new(masters.iter().cloned().chain([castle]));
        }
        t.players[1].objects[1].life.master_id = 82;
        if another {
            let mut other = t.players[1].objects[1].clone();
            other.id = 10031;
            t.players[1].objects.push(other);
        }
        t.techs = vec![TechDef::default()];
        t.players[1].techs = vec![TechState {
            state: 3,
            ..TechState::default()
        }];
        let owner = &mut t.players[1];
        owner.attributes.resize(250, 0.0);
        (
            owner.attributes[134],
            owner.attributes[98],
            owner.attributes[165],
        ) = (0.5, 1000.0, 1000.0);
        t.players[1].watched.push(crate::vision::ghost::Watched {
            id: 10028,
            cell: crate::vision::ghost::Cell::Tile(0),
            bits: 0,
        });
        in_update(&mut t, (1, 1), 0, |unit, c| {
            unit.life.state = 3;
            unit.died(c, 2)?;
            let a = &c.w.players[1].attributes;
            let withdrawn = c.w.players[1].techs[0].state != 3;
            Ok((watches(c.w, 1, 10028), [a[134], a[98], a[165]], withdrawn))
        })
        .map_err(|e| e.reason().to_owned())
    };
    assert_eq!(died(false), Ok((0, [0.0, 350.0, 1000.0], true)));
    assert_eq!(died(true), Ok((0, [0.0, 350.0, 1000.0], false)));
}

/// The world's update runs the builder of the walk-and-build test the same way the object's
/// own update does (4 walking, 6 and 7 at the house, 224 at its completion, then idle), and
/// the completion's stop before its unit AI resumes it in the same update: nothing freezes.
#[test]
fn a_builder_s_house_completes_in_the_world_s_update_and_its_unit_ai_resumes() {
    let mut t = house_builder();
    t.pathing = Pathing::new([120, 120], vec![0; 120 * 120], vec![vec![1.0]; 8], 0);
    t.build_mesh();
    let v = &mut t.players[1].objects[0];
    let here = v.position();
    let m = v.body.mover_mut().unwrap();
    m.moved_at = here;
    m.stats.facets = Some(crate::objects::facing::SpriteFacets {
        count: 8,
        flag_1: false,
    });
    v.actions.build_mut().unwrap().task.work_value = 1.0;
    let mut seen = Vec::new();
    for n in 0..520 {
        t.tick(50);
        let b = t.players[1].objects[0].actions.build().copied().unwrap();
        let house = t.players[1].objects[1].life.state;
        let now = (b.state, b.move_to, house);
        if seen.last().is_none_or(|&(_, last)| last != now) {
            seen.push((n, now));
        }
    }
    assert_eq!(
        seen,
        [
            (0, (4, Some(MoveTo::Moving), 0)),
            (8, (4, Some(MoveTo::Arrived), 0)),
            (9, (6, None, 0)),
            (10, (7, None, 0)),
            (511, (224, None, 2)),
            (512, (crate::construction::build::IDLE, None, 2)),
        ]
    );
    assert_eq!(t.players.iter().map(|p| p.freezes.len()).sum::<usize>(), 0);
}

/// [`World::run`] on `command` against its handler called by hand on a copy (`by_hand`, whose
/// error stops `player`'s lane): the same world after. Whether the command changed the world.
fn routes(
    t: &World,
    command: &Command,
    player: usize,
    by_hand: impl FnOnce(&mut World) -> Result<(), GenieError>,
) -> bool {
    let mut ran = t.clone();
    ran.run(command);
    let mut hand = t.clone();
    if let Err(e) = by_hand(&mut hand) {
        hand.stop_player(player, e);
    }
    assert!(ran == hand, "{command:?}");
    ran != *t
}

/// The command phase runs each unit command by its handler ([`World::run`]), on
/// [`forage_world`]'s villager 9175 and town centre 700; an error stops the issuer's lane. A
/// human player's `UNIT_ORDER` takes the world time as its units' last human order time first.
#[test]
fn the_command_phase_runs_each_unit_command_by_its_handler() {
    let t = forage_world();
    let v = object_ids([9175]);
    let way = vec![Point::new(40.5, 89.5)];
    let sweep = crate::orders::Sweep {
        patrol: false,
        reset: true,
        queued: false,
        ignore_pending: false,
    };
    let unit_order = |action, human| {
        Command::UnitOrder(UnitOrderCommand {
            player: player_id(1),
            units: unit_list(v.clone()),
            target: Some(object_id(700)),
            point: Point::new(-1.0, -1.0),
            q_id: -1,
            through_ai: true,
            nearby: false,
            action: UnitOrderAction::of(action),
            param: 0,
            human,
        })
    };
    let changed = [
        routes(
            &t,
            &Command::Stop(StopCommand {
                player: player_id(1),
                units: unit_list(v.clone()),
            }),
            1,
            |w| w.stop_command(player_id(1), &v),
        ),
        routes(
            &t,
            &Command::Patrol(PatrolCommand {
                player: player_id(1),
                units: unit_list(v.clone()),
                waypoints: Waypoints::of(&way),
                reset: true,
            }),
            1,
            |w| w.patrol(player_id(1), &v, &way, true),
        ),
        routes(
            &t,
            &Command::AttackMove(AttackMoveCommand {
                player: player_id(1),
                units: unit_list(v.clone()),
                waypoints: Waypoints::of(&way),
                reset: true,
                queued: false,
                ignore_pending: false,
            }),
            1,
            |w| w.attack_move(player_id(1), &v, &way, sweep),
        ),
        routes(
            &t,
            &Command::SeekShelter(SeekShelterCommand {
                player: player_id(1),
                units: unit_list(v.clone()),
            }),
            1,
            |w| w.seek_shelter(player_id(1), &v),
        ),
        routes(
            &t,
            &Command::UnitAiState(UnitAiStateCommand {
                player: player_id(1),
                units: unit_list(v.clone()),
                state: 1,
            }),
            1,
            |w| w.unit_ai_state(player_id(1), &v, 1),
        ),
        routes(
            &t,
            &Command::AutoScout(AutoScoutCommand {
                player: player_id(1),
                units: unit_list(v.clone()),
            }),
            1,
            |w| w.auto_scout(player_id(1), &v),
        ),
        routes(
            &t,
            &Command::Unload(UnloadCommand {
                player: player_id(1),
                units: unit_list(object_ids([700])),
                point: Point::new(24.5, 89.5),
                flag: 0,
                unit_type: -1,
                sound: false,
            }),
            1,
            |w| {
                UnloadCommand {
                    player: player_id(1),
                    units: unit_list(object_ids([700])),
                    point: Point::new(24.5, 89.5),
                    flag: 0,
                    unit_type: -1,
                    sound: false,
                }
                .run(w)
            },
        ),
        routes(
            &t,
            &Command::GoBackToWork2(GoBackToWork2Command {
                player: player_id(1),
                units: unit_list(v.clone()),
            }),
            1,
            |w| w.go_back_to_work(player_id(1), &v),
        ),
        routes(
            &t,
            &Command::Delete(DeleteCommand {
                player: player_id(1),
                id: object_id(9175),
            }),
            1,
            |w| w.delete(player_id(1), object_id(9175)),
        ),
        routes(
            &t,
            &Command::TransformObject(TransformObjectCommand {
                player: player_id(1),
                id: object_id(9175),
                master: master_id(120),
            }),
            1,
            |w| w.transform_object(player_id(1), object_id(9175), master_id(120)),
        ),
        routes(&t, &unit_order(5, false), 1, |w| {
            w.garrison_order(player_id(1), &v, Some(object_id(700)), true, false)
        }),
        routes(&t, &unit_order(4, false), 1, |w| {
            w.cancel_order(&v, 0, true)
        }),
        routes(&t, &unit_order(9, true), 1, |w| {
            w.human_order(&ObjectId::fields(&v));
            w.unit_order(player_id(1), &v, 9)
        }),
    ];
    assert_eq!(
        changed,
        [
            true, true, true, false, true, false, false, false, true, false, false, false, true
        ]
    );
}

/// The command phase runs each player's command by its handler ([`World::run`]): a
/// tribute, a resignation, a flare, a market trade (which cannot fail), a GAME sub-command
/// (what the slice cannot model of it stops the player it is for) and an unmodelled command
/// (its player stops).
#[test]
fn the_command_phase_runs_each_player_command_by_its_handler() {
    use crate::players::tribute::Gift;
    let t = forage_world();
    let gifts = [Gift {
        attribute: 0,
        amount: 10.0,
        fee: 0.0,
    }; 4];
    let game = |sub| GameCommand {
        issuer: player_id(1),
        sub,
        player: 0,
        a: 0,
        speed: 0.0,
        value: 0,
    };
    let why = GenieError::unsupported("a command the slice does not model");
    let changed = [
        routes(
            &t,
            &Command::Tribute(TributeCommand {
                player: player_id(1),
                to: player_id(0),
                gifts,
            }),
            1,
            |w| w.tribute(player_id(1), player_id(0), &gifts),
        ),
        routes(
            &t,
            &Command::Resign(ResignCommand {
                player: player_id(1),
                dropped: false,
            }),
            1,
            |w| w.resign(player_id(1), false),
        ),
        routes(
            &t,
            &Command::Flare(FlareCommand {
                player: player_id(1),
                target: None,
                point: Point::new(40.5, 89.5),
                players: 2,
                shown: Vec::new(),
            }),
            1,
            |w| w.flare(None, Point::new(40.5, 89.5), 2),
        ),
        routes(
            &t,
            &Command::Trade(TradeCommand {
                player: player_id(1),
                attribute: 1,
                times: 1,
                market: None,
                buy: true,
            }),
            1,
            |w| {
                w.buy(player_id(1), 1, 1);
                Ok(())
            },
        ),
        routes(
            &t,
            &Command::Trade(TradeCommand {
                player: player_id(1),
                attribute: 1,
                times: 1,
                market: None,
                buy: false,
            }),
            1,
            |w| {
                w.sell(player_id(1), 1, 1);
                Ok(())
            },
        ),
        routes(&t, &Command::Game(game(0)), 0, |w| w.game(&game(0))),
        routes(
            &t,
            &Command::Unmodelled(UnmodelledCommand {
                player: player_id(1),
                packet: Vec::new(),
                why: why.clone(),
            }),
            1,
            |_| Err(why.clone()),
        ),
    ];
    assert_eq!(changed, [false, true, true, false, false, false, true]);
    // with the game's cheats a cheat code is not modelled: it stops the player it is for, or
    // its issuer for a player outside the game
    let mut cheats = t;
    cheats.settings.cheats_enabled = true;
    let cheat = |player| GameCommand {
        issuer: player_id(1),
        sub: crate::commands::game::sub::CHEAT,
        player,
        ..game(0)
    };
    assert!(routes(&cheats, &Command::Game(cheat(0)), 0, |w| w.game(&cheat(0))));
    assert!(routes(&cheats, &Command::Game(cheat(9)), 1, |w| w.game(&cheat(9))));
}

/// An order of units runs after its command's check ([`World::run`]): a MOVE of the villager
/// leaves it as the last commanded units, resets it for the order, takes the world time as its
/// last human order time and moves it; a human player's MOVE left without a unit (Gaia's bush)
/// does nothing, as does a FOLLOW of a target the world lacks; a GUARD takes the builders'
/// check; a RETREAT is not modelled (after the reset).
#[test]
fn the_command_phase_runs_an_order_after_its_check() {
    use crate::commands::Order;
    let t = forage_world();
    let v = vec![9175];
    let walk = crate::orders::MoveOrder {
        target: None,
        point: Point::new(40.5, 89.5),
        queued: false,
        direct: false,
        human: true,
        control: false,
    };
    let order = |units: Vec<i32>, order| {
        Command::Order(OrderCommand {
            player: player_id(1),
            units: unit_list(object_ids(units)),
            order,
        })
    };
    assert!(routes(&t, &order(v.clone(), Order::Move(walk)), 1, |w| {
        w.set_last_commanded(1, v.clone());
        w.reset_order_units(&v);
        w.human_order(&v);
        w.move_order(1, &v, &walk)
    }));
    let none_left = order(vec![600], Order::Move(walk));
    assert!(!routes(&t, &none_left, 1, |_| Ok(())));
    let lost = order(v.clone(), Order::Follow(object_id(4444)));
    assert!(!routes(&t, &lost, 1, |_| Ok(())));
    assert!(routes(
        &t,
        &order(v.clone(), Order::Guard(object_id(700))),
        1,
        |w| {
            w.reset_order_units(&v);
            w.human_order(&v);
            w.guard(1, &v, 700)
        }
    ));
    assert!(routes(
        &t,
        &order(
            v.clone(),
            Order::Retreat(crate::orders::Interact::default())
        ),
        1,
        |w| {
            w.set_last_commanded(1, v.clone());
            w.reset_order_units(&v);
            Err(GenieError::unsupported("an order moves its units"))
        }
    ));
}

/// The last commanded units (`FUN_140e6b290`): a MOVE of its own list that its check lets
/// run leaves that list as its player's last commanded units, −1s and all; a refused MOVE
/// leaves them. A count of `0xffff` orders them, and its check writes its −1s back, refused
/// or not.
#[test]
fn an_order_of_the_last_commanded_units_reads_what_the_last_run_order_left() {
    use crate::commands::{Order, UnitList};
    let walk = crate::orders::MoveOrder {
        target: None,
        point: Point::new(40.5, 89.5),
        queued: false,
        direct: false,
        human: true,
        control: false,
    };
    let order = |units| {
        Command::Order(OrderCommand {
            player: player_id(1),
            units,
            order: Order::Move(walk),
        })
    };
    let mut t = forage_world();
    t.run(&order(unit_list(object_ids([9175, 600]))));
    assert_eq!(t.last_commanded.get(1), Some(&vec![9175, -1]));
    t.run(&order(unit_list(object_ids([600]))));
    assert_eq!(t.last_commanded.get(1), Some(&vec![9175, -1]));

    let mut cached = forage_world();
    cached.set_last_commanded(1, vec![600, 9175]);
    let checked = vec![-1, 9175];
    assert!(routes(
        &cached,
        &order(UnitList::last_commanded()),
        1,
        |w| {
            w.set_last_commanded(1, checked.clone());
            w.reset_order_units(&checked);
            w.human_order(&checked);
            w.move_order(1, &checked, &walk)
        }
    ));
    cached.set_last_commanded(1, vec![600]);
    let mut refused = cached.clone();
    refused.run(&order(UnitList::last_commanded()));
    cached.set_last_commanded(1, vec![-1]);
    assert_eq!(refused, cached);
}

/// An order's units (`FUN_140e6b290`): comm player 0's and one past the world's are the
/// packet's as they are, with no last commanded units; any other's are the first 60, 500 for
/// a computer, or its last commanded units, none before its first.
#[test]
fn an_order_s_units_are_capped_unless_its_comm_player_is_outside_the_world() {
    use UnitList;
    let many: Vec<i32> = (0..600).collect();
    let mut t = forage_world();
    let listed = unit_list(object_ids(many.clone()));
    assert_eq!(t.command_units(0, &listed), many);
    assert_eq!(t.command_units(t.players.len(), &listed), many);
    assert_eq!(t.command_units(1, &listed), many[..60]);
    assert_eq!(
        t.command_units(1, &UnitList::last_commanded()),
        Vec::<i32>::new()
    );
    t.players[1].computer = true;
    assert_eq!(t.command_units(1, &listed), many[..500]);
    assert_eq!(t.most_units(t.players.len()), 0);
    t.set_last_commanded(0, vec![5]);
    t.set_last_commanded(t.players.len(), vec![6]);
    t.set_last_commanded(1, vec![7]);
    assert_eq!(t.last_commanded, [vec![], vec![7]]);
    assert_eq!(
        t.command_units(0, &UnitList::last_commanded()),
        Vec::<i32>::new()
    );
    assert_eq!(t.command_units(1, &UnitList::last_commanded()), [7]);
}

/// The command phase runs each building command by its handler ([`World::run`]) on
/// [`forage_world`]'s town centre 700: a queue, a research after its buildings' check, a
/// gather point, the command queue, a gate, the town bell and its all-back-to-work.
#[test]
fn the_command_phase_runs_each_building_command_by_its_handler() {
    let t = forage_world();
    let tc = object_ids([700]);
    let research = || ResearchCommand {
        player: player_id(1),
        building: Some(object_id(700)),
        count: 1,
        buildings: tc.clone(),
        tech: 22,
        q_id: -1,
        shift: false,
    };
    let queue = QueueCommand {
        player: player_id(1),
        buildings: unit_list(tc.clone()),
        building_master: master_id(109),
        unit: master_id(83),
        amount: 1,
        button: 0,
    };
    let changed = [
        routes(
            &t,
            &Command::Queue(QueueCommand {
                player: player_id(1),
                buildings: unit_list(tc.clone()),
                building_master: master_id(109),
                unit: master_id(83),
                amount: 1,
                button: 0,
            }),
            1,
            |w| queue.run(w),
        ),
        routes(
            &t,
            &Command::Research(ResearchCommand {
                player: player_id(1),
                building: Some(object_id(700)),
                count: 1,
                buildings: tc.clone(),
                tech: 22,
                q_id: -1,
                shift: false,
            }),
            1,
            |w| {
                let Some(buildings) = w.research_check(&research()) else {
                    return Ok(());
                };
                w.research(&research(), &buildings)
            },
        ),
        routes(
            &t,
            &Command::GatherPoint(GatherPointCommand {
                player: player_id(1),
                units: unit_list(tc.clone()),
                point: Point::new(30.5, 85.5),
                target: None,
                target_master: None,
                own: false,
            }),
            1,
            |w| {
                GatherPointCommand {
                    player: player_id(1),
                    units: unit_list(tc.clone()),
                    point: Point::new(30.5, 85.5),
                    target: None,
                    target_master: None,
                    own: false,
                }
                .run(w)
            },
        ),
        routes(
            &t,
            &Command::CommandQueue(BuildingQueueCommand {
                player: player_id(1),
                units: unit_list(tc.clone()),
                target: None,
                point: Point::new(30.5, 85.5),
            }),
            1,
            |w| w.command_queue(player_id(1), &tc, None, Point::new(30.5, 85.5)),
        ),
        routes(
            &t,
            &Command::Gate(GateCommand {
                player: player_id(1),
                id: object_id(700),
            }),
            1,
            |w| w.gate(player_id(1), object_id(700)),
        ),
        routes(
            &t,
            &Command::TownBell(TownBellCommand {
                player: player_id(1),
                id: object_id(700),
                activate: true,
            }),
            1,
            |w| w.town_bell(player_id(1), object_id(700), true),
        ),
        routes(
            &t,
            &Command::AllBackToWork(AllBackToWorkCommand {
                player: player_id(1),
                id: object_id(700),
            }),
            1,
            |w| w.all_back_to_work(player_id(1), object_id(700)),
        ),
    ];
    // the town centre's master is bare: only the gate changes it (any building of type 80
    // toggles its lock)
    assert_eq!(changed, [false, false, false, false, true, false, false]);
}

/// `AskForHelp`'s walk of the last search's own and friends' lists
/// ([`UnitSide::call_for_help`] of the world's view): the asking unit passes itself; a live
/// helper with a unit AI within 3 tiles that is free ([`helper_free`]) answers, which is not
/// modelled; a busy, dead, unthinking or distant one does not. Without a search before it
/// the registry's last answer is not held.
///
/// [`UnitSide::call_for_help`]: crate::unit_ai::UnitSide::call_for_help
#[test]
fn a_call_for_help_walks_the_lists_for_a_free_helper_in_reach() {
    use crate::unit_ai::UnitSide;
    use crate::vision::registry::Record;
    let idle = UnitAi {
        ai_type: 13,
        current_order: None,
        current_action: None,
        current_target: -1,
        stance: 0,
        ..UnitAi::default()
    };
    let record = |id, x| Record {
        id,
        distance: f32::MAX,
        owner: 1,
        x,
        y: 10.0,
        rx: 0.2,
        ry: 0.2,
    };
    let lists = || {
        [
            Vec::new(),
            vec![record(123, 10.0), record(124, 10.3)],
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ]
    };
    let groups = crate::groups::UnitGroups::default();
    let call = |helper: Unit, searched| {
        help_view(helper, searched, &groups, |side| side.call_for_help(50))
    };
    assert_eq!(
        call(helper_scout(idle.clone(), None), Some(lists())),
        crate::unit_ai::helper_answers()
    );
    let busy = UnitAi {
        current_order: Some(OrderKind::Attack),
        ..idle.clone()
    };
    assert_eq!(call(helper_scout(busy, None), Some(lists())), Ok(()));
    let mut dead = helper_scout(idle.clone(), None);
    dead.life.state = 3;
    assert_eq!(call(dead, Some(lists())), Ok(()));
    let mut unthinking = helper_scout(idle.clone(), None);
    unthinking.ai = None;
    assert_eq!(call(unthinking, Some(lists())), Ok(()));
    let mut far = helper_scout(idle.clone(), None);
    far.place_for_test([14.0, 10.0, 0.0]);
    assert_eq!(call(far, Some(lists())), Ok(()));
    assert_eq!(
        call(helper_scout(idle, None), None),
        Err(GenieError::unsupported(
            "a search that reads the registry's last answer (before any query)"
        ))
    );
}

/// RESEARCH's handler on its listed buildings, every one kept by its check.
fn research_listed(t: &mut World, r: &ResearchCommand) -> Result<(), GenieError> {
    t.research(r, &r.buildings)
}
