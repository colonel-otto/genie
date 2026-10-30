use super::*;
use crate::combat::damage::Victim;
use crate::test_ids::object_id;

fn task(action_type: i16) -> Task {
    Task {
        action_type,
        ..Task::default()
    }
}

/// The Feitoria's first task (1021, task 0): food from attribute 205 at 1.6 a second.
fn feitoria_food() -> Task {
    Task {
        object_group: -1,
        attribute_type2: 205,
        attribute_type3: 0,
        work_val1: 1.6,
        combat_level_flag: 1,
        ..task(GENERATE)
    }
}

/// The Keshik's task 5 (1228): gold from attribute 213 at 0.01 while it fights class 0.
fn keshik_gold() -> Task {
    Task {
        object_group: 0,
        attribute_type2: 213,
        attribute_type3: 3,
        work_val1: 0.01,
        combat_level_flag: 1,
        ..task(GENERATE)
    }
}

#[test]
fn the_living_path_takes_at_least_1_hit_point_or_a_nan() {
    assert!(alive(1.0));
    assert!(alive(25.0));
    assert!(alive(f32::NAN));
    assert!(!alive(0.999_999_94));
    assert!(!alive(0.0));
}

#[test]
fn a_task_with_its_type_s_flag_bit_generates_always() {
    // bit 1 for an object of type 70, bit 2 for one of type 80
    let mut t = feitoria_food();
    t.combat_level_flag = 2;
    assert!(generates(&t, 70, 70, None, false));
    assert!(!generates(&t, 80, 80, None, false));
    t.combat_level_flag = 4;
    assert!(generates(&t, 80, 80, None, false));
    assert!(!generates(&t, 70, 70, None, false));
}

#[test]
fn a_building_generates_once_built() {
    let t = feitoria_food();
    assert!(!generates(&t, 80, 80, None, false));
    assert!(generates(&t, 80, 80, None, true));
    // a combat unit's master is not a building's
    assert!(!generates(&t, 70, 70, None, true));
}

#[test]
fn a_unit_generates_while_its_top_action_works_on_a_target_of_the_task_s_class() {
    let t = keshik_gold();
    let target = WorkTarget {
        hp: 45.0,
        storage: 0.0,
        class: 0,
    };
    let top = |kind, state, target| {
        Some(TopWork {
            kind,
            state,
            target,
        })
    };
    // the Attack (9), type 7, the Gather (5) and the Hunt (0x6e), in state 7
    for kind in [9, 7, 5, 0x6e] {
        assert!(generates(&t, 70, 70, top(kind, 7, Some(target)), false));
    }
    assert!(!generates(&t, 70, 70, top(1, 7, Some(target)), false));
    assert!(!generates(&t, 70, 70, top(9, 6, Some(target)), false));
    assert!(!generates(&t, 70, 70, top(9, 7, None), false));
    assert!(!generates(&t, 70, 70, None, false));
    // another class
    let other = WorkTarget { class: 6, ..target };
    assert!(!generates(&t, 70, 70, top(9, 7, Some(other)), false));
    // a dead target counts while its master stores some, never with a NaN
    let dead = WorkTarget {
        hp: 0.0,
        storage: 1.0,
        ..target
    };
    assert!(generates(&t, 70, 70, top(9, 7, Some(dead)), false));
    let empty = WorkTarget {
        hp: 0.0,
        storage: 0.0,
        ..target
    };
    assert!(!generates(&t, 70, 70, top(9, 7, Some(empty)), false));
    let nan = WorkTarget {
        hp: f32::NAN,
        storage: f32::NAN,
        ..target
    };
    assert!(!generates(&t, 70, 70, top(9, 7, Some(nan)), false));
}

#[test]
fn the_amount_is_dt_times_the_rate_times_the_attribute_in_that_order() {
    let t = feitoria_food();
    let dt = 0.044_f32;
    assert_eq!(generated(dt, &t, 1.0).to_bits(), (dt * 1.6 * 1.0).to_bits());
    let m = 0.3_f32;
    assert_eq!(generated(dt, &t, m).to_bits(), ((dt * 1.6) * m).to_bits());
    assert_eq!(generated(dt, &t, 0.0), 0.0);
}

#[test]
fn the_store_gives_what_reached_1_and_starts_again() {
    let mut stores = vec![0.0_f32; 4];
    assert_eq!(store(0.6, 0, &mut stores), None);
    assert_eq!(stores[0], 0.6);
    // ADDSS: the amount plus the store
    let sum = 0.5_f32 + 0.6;
    assert_eq!(store(0.5, 0, &mut stores), Some(sum));
    assert_eq!(stores[0], 0.0);
    // nothing above 0 joins: the store stays, and nothing is given
    stores[1] = 0.9;
    assert_eq!(store(0.0, 1, &mut stores), None);
    assert_eq!(store(f32::NAN, 1, &mut stores), None);
    assert_eq!(store(-2.0, 1, &mut stores), None);
    assert_eq!(stores[1], 0.9);
    // a NaN store gives nothing (JC)
    stores[2] = f32::NAN;
    assert_eq!(store(0.5, 2, &mut stores), None);
    // a negative resource: no store and no give, however large
    assert_eq!(store(5.0, -1, &mut stores), None);
}

#[test]
fn the_movement_damage_timer_runs_down_to_0_and_then_hits() {
    let mut t = 0.5_f32;
    assert!(!movement_damage_due(&mut t, 0.2));
    assert_eq!(t, 0.5 - 0.2);
    assert!(movement_damage_due(&mut t, 0.4));
    assert_eq!(t, 0.0);
    // at 0 it does not run down and hits
    assert!(movement_damage_due(&mut t, 0.4));
    assert_eq!(t, 0.0);
    // to exactly 0
    let mut t = 0.25_f32;
    assert!(movement_damage_due(&mut t, 0.25));
    assert_eq!(t, 0.0);
    // below 0 (a timer the save holds so) hits as it is
    let mut t = -1.0_f32;
    assert!(movement_damage_due(&mut t, 0.1));
    assert_eq!(t, -1.0);
    // a NaN never hits and stays
    let mut t = f32::NAN;
    assert!(!movement_damage_due(&mut t, 0.1));
    assert!(t.is_nan());
}

#[test]
fn the_reach_is_the_truncated_range_about_the_truncated_place_within_the_map() {
    assert_eq!(
        reach([10.7, 20.2, 0.0], 2.9, [120, 120]),
        ([8, 18], [12, 22])
    );
    // from 0, and up to the map's size itself
    assert_eq!(
        reach([1.5, 118.9, 0.0], 3.0, [120, 120]),
        ([0, 115], [4, 120])
    );
    // a NaN range is the integer indefinite
    // (5 less it wraps to a negative value, 0; 5 plus it stays below the map's size): no
    // tile
    let (low, high) = reach([5.0, 5.0, 0.0], f32::NAN, [120, 120]);
    assert_eq!(low, [0, 0]);
    assert_eq!(high, [i32::MIN + 5; 2]);
}

fn hitter() -> Hitter {
    Hitter {
        id: 1,
        blast_level: 1,
        position: [10.0, 10.0, 0.0],
        radius: [0.4, 0.4],
    }
}

fn bystander() -> Bystander {
    Bystander {
        id: 2,
        player: 2,
        allied: false,
        kind: 70,
        class: 6,
        state: 2,
        blast_defense: 1,
        position: [11.5, 10.0, 0.0],
        radius: [0.2, 0.2],
    }
}

#[test]
fn a_movement_damage_strikes_enemies_in_reach() {
    let h = hitter();
    let b = bystander();
    // the gap is 1.5 - 0.6 = 0.9: within 1 (0.81 at most 1), not within 0.5
    assert!(struck(&h, 1.0, &b));
    assert!(!struck(&h, 0.5, &b));
    assert!(struck(&h, f32::NAN, &b));
    assert!(!struck(&h, 1.0, &Bystander { id: 1, ..b }));
    assert!(!struck(&h, 1.0, &Bystander { allied: true, ..b }));
    assert!(!struck(&h, 1.0, &Bystander { player: 0, ..b }));
    assert!(!struck(&h, 1.0, &Bystander { state: 3, ..b }));
    // buildings only of classes 51 and 54
    let building = Bystander {
        kind: 80,
        class: 3,
        ..b
    };
    assert!(!struck(&h, 1.0, &building));
    assert!(struck(
        &h,
        1.0,
        &Bystander {
            class: 51,
            ..building
        }
    ));
    assert!(struck(
        &h,
        1.0,
        &Bystander {
            class: 54,
            ..building
        }
    ));
    // the blast levels
    let high = Hitter {
        blast_level: 3,
        ..h
    };
    assert!(!struck(&high, 1.0, &b));
    assert!(struck(
        &high,
        1.0,
        &Bystander {
            blast_defense: 2,
            ..b
        }
    ));
    assert!(struck(
        &Hitter {
            blast_level: 1,
            ..h
        },
        1.0,
        &Bystander {
            blast_defense: 1,
            ..b
        }
    ));
}

#[test]
fn a_hit_below_1_hit_point_leaves_0_and_holds_nothing() {
    let mut v = Victim {
        hp: 0.5,
        amount: 30.0,
        capacity: 100,
        ..Victim::default()
    };
    assert!(finish(&mut v));
    assert_eq!((v.hp, v.amount), (0.0, 0.0));
    // a capacity below 0 is taken as it is
    let mut v = Victim {
        hp: -3.0,
        amount: 30.0,
        capacity: -1,
        ..Victim::default()
    };
    assert!(finish(&mut v));
    assert_eq!((v.hp, v.amount), (0.0, -1.0));
    let mut v = Victim {
        hp: 1.0,
        amount: 30.0,
        ..Victim::default()
    };
    assert!(!finish(&mut v));
    assert_eq!(v.amount, 30.0);
    let mut v = Victim {
        hp: f32::NAN,
        ..Victim::default()
    };
    assert!(!finish(&mut v));
}

#[test]
fn the_terrain_mask_is_a_bitset_of_8() {
    // the longboats' -32: the mask 32
    assert_eq!(terrain_mask(-32), Some(32));
    assert_eq!(terrain_mask(-3), Some(3));
    // the negation's low byte
    assert_eq!(terrain_mask(-0x103), Some(3));
    assert_eq!(terrain_mask(i16::MIN), Some(0));
    assert_eq!(terrain_mask(1), Some(1));
    assert_eq!(terrain_mask(8), Some(0x80));
    assert_eq!(terrain_mask(9), Some(0));
    assert_eq!(terrain_mask(0), None);
}

/// The longboat's task 3 (250 in the current build): terrains of flag 32, sprites 19601,
/// −1, 19599, 19599, the dying and undead sprites 19598 and 19600.
fn longboat_look() -> Task {
    Task {
        terrain: -32,
        work_val1: 0.35,
        work_val2: 1.0,
        sprites: [19601, -1, 19599, 19599],
        sound_events: [19598, 19600],
        ..task(TERRAIN_LOOK)
    }
}

#[test]
fn a_terrain_look_gives_the_task_s_sprites_on_its_terrains() {
    let t = longboat_look();
    let all = |_| true;
    assert_eq!(
        look(&t, 32, 0x21, all),
        Some(Look {
            standing: Some(19599),
            walking: Some(19601),
            attacking: Some(19599),
            running: None,
            dying: Some(19598),
            undead: Some(19600),
        })
    );
    assert_eq!(look(&t, 32, 1, all), None);
    // a sprite the table lacks is a null one
    let some = |id: i32| id != 19600 && id != 19599;
    let l = look(&t, 32, 32, some).unwrap();
    assert_eq!((l.standing, l.undead, l.walking), (None, None, Some(19601)));
    // an event id past the table's ids is none
    let t = Task {
        sound_events: [0x62d4_b3ec, -5],
        ..longboat_look()
    };
    let l = look(&t, 32, 32, all).unwrap();
    assert_eq!((l.dying, l.undead), (None, None));
}

/// A removal reward task: 10 gold (3) a removal, by attribute 7's rate.
fn bounty() -> Task {
    Task {
        attribute_type2: 7,
        attribute_type3: 3,
        work_val1: 10.0,
        ..task(REMOVAL_REWARD)
    }
}

#[test]
fn a_removal_reward_pays_the_rate_times_the_attribute() {
    let t = bounty();
    assert_eq!(
        reward(&t, 2, Some(1.5), &[]),
        Reward {
            player: 2,
            attribute: 3,
            amount: 15
        }
    );
    // truncated
    assert_eq!(reward(&t, 2, Some(0.25), &[]).amount, 2);
    // the player lacks the attribute, or the task names none
    assert_eq!(reward(&t, 2, None, &[]).amount, 10);
    let none = Task {
        attribute_type2: -1,
        ..bounty()
    };
    assert_eq!(reward(&none, 2, Some(3.0), &[]).amount, 10);
    // a NaN rate is the integer indefinite
    assert_eq!(reward(&t, 2, Some(f32::NAN), &[]).amount, i32::MIN);
}

#[test]
fn a_removal_reward_with_its_flag_pays_the_last_paid_cost_of_its_resource_times_the_rate() {
    let t = Task {
        combat_level_flag: 1,
        ..bounty()
    };
    let cost = [(3, 20, true), (0, 50, true), (3, 30, false)];
    assert_eq!(reward(&t, 1, Some(0.5), &cost).amount, 100);
    let cost = [(3, 20, true), (3, 30, true)];
    assert_eq!(reward(&t, 1, Some(1.0), &cost).amount, 300);
    assert_eq!(reward(&t, 1, Some(1.0), &[(0, 50, true)]).amount, 0);
}

use crate::map::spatial::{Placed, Spatial};
use crate::objects::masters::{Master, Masters};
use crate::objects::unit::Unit;
use crate::world::Player;

/// Gaia and two enemies, the world's dt half a second, a 20 by 20 map.
fn world() -> World {
    let player = |relations: Vec<u8>| Player {
        attributes: vec![0.0; 600],
        generated: vec![0.0; 600],
        relations,
        ..Player::default()
    };
    let players = vec![
        player(vec![0, 3, 3]),
        player(vec![3, 0, 3]),
        player(vec![3, 3, 0]),
    ];
    let mut t = World::new(0, 1, 0, vec![0; 3], players);
    t.dt_seconds = 0.5;
    t.spatial = Spatial::new([20, 20]);
    t.spatial.relations = t.players.iter().map(|p| p.relations.clone()).collect();
    t.masters = vec![Masters::default(), Masters::default(), Masters::default()];
    t
}

/// Master `id` of `kind` and `class` with `tasks`, given to every player.
fn give_master(t: &mut World, id: i16, kind: u8, class: i16, tasks: Vec<Task>) {
    let m = Master {
        id,
        kind,
        class,
        tasks,
        radius: [0.5, 0.5],
        hit_points: 100,
        ..Master::default()
    };
    for ms in &mut t.masters {
        let mut all: Vec<Master> = ms.iter().cloned().collect();
        all.push(m.clone());
        *ms = Masters::new(all);
    }
}

/// Object `id` of master `m` with `hp` hit points at (`x`, `y`) in player `p`'s list, placed
/// on its tile.
fn put(t: &mut World, p: usize, id: i32, m: i16, hp: f32, [x, y]: [f32; 2]) {
    let mut u = Unit {
        id: object_id(id),
        body: crate::objects::layers::Body::at([x, y, 0.0]),
        ..Unit::default()
    };
    u.life.master_id = m;
    u.life.hp = hp;
    u.life.state = 2;
    t.players[p].objects.push(u);
    let radius = t.masters[p].get(m).map_or([0.5; 2], |m| m.radius);
    t.spatial.insert(
        id,
        Placed {
            position: [x, y, 0.0],
            radius,
            class: 0,
            player: p,
            master: m,
            obstruction_type: 0,
            buildable_on: 0,
            tall: false,
            speed: 0.0,
            unknown: None,
            obstruction: None,
        },
    );
}

/// The abilities of object `id` of player `p` as `p`'s, which issue no command.
fn run(t: &mut World, p: usize, id: i32) {
    assert_eq!(rewards(t, p, id, false), Vec::new());
}

/// The abilities of object `id` of player `p` as `p`'s: the commands they issue.
fn rewards(t: &mut World, p: usize, id: i32, leaves: bool) -> Vec<Reward> {
    let i = t.players[p]
        .objects
        .iter()
        .position(|u| u.id == object_id(id))
        .unwrap();
    t.abilities((id, p, i), p, leaves).unwrap()
}

/// The players' sight with `id` a building of player `p` that is `built`.
fn building_sight(t: &mut World, p: usize, id: i32, built: bool) {
    let map = || crate::vision::visibility::VisibleMap {
        counts: vec![0; 400],
        explored: 0,
    };
    let mut v = crate::vision::visibility::Visibility::new(
        20,
        20,
        vec![map(), map(), map()],
        vec![0; 400],
        Vec::new(),
        0,
        vec![false; 3],
    );
    v.add_seer(
        id,
        crate::vision::visibility::Seer {
            player: p,
            master: 1021,
            los: 6.0,
            state: 2,
            explored: true,
            building: Some(crate::vision::visibility::BuildingSight {
                built,
                started: true,
            }),
            umv: 0,
            capture: 0,
            kind: 80,
        },
    );
    t.spatial.sight = Some(v);
}

#[test]
fn a_built_feitoria_trickles_food_through_its_player_s_store() {
    let mut t = world();
    // its explore task (6) does nothing here
    give_master(&mut t, 1021, 80, 3, vec![task(6), feitoria_food()]);
    put(&mut t, 1, 50, 1021, 2000.0, [5.5, 5.5]);
    building_sight(&mut t, 1, 50, true);
    t.players[1].attributes[205] = 1.0;
    run(&mut t, 1, 50);
    let made = 0.5_f32 * 1.6 * 1.0;
    assert_eq!(t.players[1].generated[0], made);
    assert_eq!(t.players[1].resources[0], 0.0);
    run(&mut t, 1, 50);
    let given = made + made;
    assert_eq!(t.players[1].resources[0], given);
    assert_eq!(t.players[1].attributes[166], given);
    assert_eq!(t.players[1].generated[0], 0.0);
    // unbuilt, nothing
    building_sight(&mut t, 1, 50, false);
    run(&mut t, 1, 50);
    assert_eq!(t.players[1].generated[0], 0.0);
    // the player's attribute 205 at 0: nothing joins the store
    building_sight(&mut t, 1, 50, true);
    t.players[1].attributes[205] = 0.0;
    run(&mut t, 1, 50);
    assert_eq!(t.players[1].generated[0], 0.0);
}

/// A Keshik of player 1 attacking object 77 in state 7.
fn keshik(t: &mut World) {
    give_master(t, 1228, 70, 12, vec![keshik_gold()]);
    put(t, 1, 60, 1228, 100.0, [5.5, 5.5]);
    let attack = crate::combat::attack::Attack {
        target: 77,
        target2: -1,
        state: 7,
        timer: 0.0,
        range: 0.0,
        min_range: 0.0,
        point: [-1.0; 3],
        seen: [-1.0; 3],
        move_to: None,
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
            fire_frame: 0,
            interrupt_frame: -1,
        },
        sub_action: false,
        on_target_moved: crate::movement::move_to::TargetMoved::Follows,
        same_owner: false,
    };
    t.players[1].objects[0].actions.set_attack(Some(attack));
    t.players[1].attributes[213] = 0.5;
    t.players[1].generated[3] = 0.999;
}

#[test]
fn a_keshik_makes_gold_while_it_fights_a_target_of_its_task_s_class() {
    let mut t = world();
    keshik(&mut t);
    give_master(&mut t, 4, 70, 0, Vec::new());
    put(&mut t, 2, 77, 4, 30.0, [6.5, 5.5]);
    run(&mut t, 1, 60);
    let given = 0.5_f32 * 0.01 * 0.5 + 0.999;
    assert_eq!(t.players[1].resources[3], given);
    assert_eq!(t.players[1].attributes[49], given);
    assert_eq!(t.players[1].attributes[169], given);
    assert_eq!(t.players[1].generated[3], 0.0);
    // in another state, nothing
    t.players[1].objects[0].actions.attack_mut().unwrap().state = 6;
    run(&mut t, 1, 60);
    assert_eq!(t.players[1].generated[3], 0.0);
}

/// A combat unit of master 9 at (5.5, 5.5) moving east, with a movement damage of range 2,
/// 5 hit points and a timer of 3 s.
fn trampler(t: &mut World) {
    let trample = Task {
        work_range: 2.0,
        work_val1: 5.0,
        work_val2: 3.0,
        ..task(MOVEMENT_DAMAGE)
    };
    give_master(t, 9, 70, 12, vec![trample]);
    put(t, 1, 10, 9, 100.0, [5.5, 5.5]);
    let mut mover = crate::gathering::hunt::tests::walker();
    mover.velocity = Some(crate::movement::Velocity {
        velocity: [1.0, 0.0, 0.0],
        acceleration: [0.0; 3],
    });
    t.players[1].objects[0].body.put_mover(Some(mover));
}

#[test]
fn a_moving_unit_s_movement_damage_hits_the_enemies_about_it() {
    let mut t = world();
    trampler(&mut t);
    give_master(&mut t, 4, 70, 6, Vec::new());
    // an enemy in reach, one out of it, one of its own player, one of Gaia
    put(&mut t, 2, 20, 4, 30.0, [6.5, 5.5]);
    put(&mut t, 2, 21, 4, 30.0, [9.5, 5.5]);
    put(&mut t, 1, 30, 4, 30.0, [5.5, 6.5]);
    put(&mut t, 0, 40, 4, 30.0, [4.5, 5.5]);
    run(&mut t, 1, 10);
    let hp = |t: &World, id| t.object(id).unwrap().1.life.hp;
    assert_eq!(hp(&t, 20), 25.0);
    assert_eq!(hp(&t, 21), 30.0);
    assert_eq!(hp(&t, 30), 30.0);
    assert_eq!(hp(&t, 40), 30.0);
    assert_eq!(t.players[1].objects[0].abilities.movement_damage_timer, 3.0);
    // the timer runs down first: no hit for 3 s
    run(&mut t, 1, 10);
    assert_eq!(hp(&t, 20), 25.0);
    assert_eq!(t.players[1].objects[0].abilities.movement_damage_timer, 2.5);
    // standing, no hit even when due
    t.players[1].objects[0].abilities.movement_damage_timer = 0.0;
    t.players[1].objects[0].body.mover_mut().unwrap().velocity = None;
    run(&mut t, 1, 10);
    assert_eq!(hp(&t, 20), 25.0);
}

#[test]
fn a_movement_damage_kill_leaves_0_hit_points_and_counts_the_kill() {
    let mut t = world();
    trampler(&mut t);
    give_master(&mut t, 4, 70, 6, Vec::new());
    put(&mut t, 2, 20, 4, 3.0, [6.5, 5.5]);
    t.players[2].objects[0].life.amount = 10.0;
    run(&mut t, 1, 10);
    let (_, u) = t.object(20).unwrap();
    assert_eq!((u.life.hp, u.life.amount), (0.0, 0.0));
    // FUN_1412005b0: player 1 a kill (20) of player 2 (302), player 2 a loss (154)
    assert_eq!(t.players[1].attributes[20], 1.0);
    assert_eq!(t.players[1].attributes[302], 1.0);
    assert_eq!(t.players[2].attributes[154], 1.0);
    assert_eq!(t.players[2].attributes[326], 1.0);
}

/// The map's terrain: every tile of terrain 1 but (5, 6) of terrain 2, whose flag byte has
/// 32 (deep water).
fn waters(t: &mut World) {
    let mut tiles = vec![(1u8, 255u8, -1i16); 400];
    tiles[6 * 20 + 5] = (2, 255, -1);
    let kind = |flags| crate::map::terrain::TerrainKind {
        shown: true,
        flags,
        forest: false,
    };
    t.spatial.terrain = Some(crate::map::terrain::Terrain::new(
        20,
        20,
        tiles,
        vec![kind(0), kind(1), kind(0x21)],
        Vec::new(),
    ));
    let g = |id| crate::objects::sprite::Graphic {
        id,
        frames: 10,
        facets: 8,
        speed_multiplier: 1.0,
        frame_duration: 0.1,
        replay_delay: 0.0,
        flags: 1,
    };
    for id in [19598, 19599, 19600, 19601] {
        let _ = t.graphics.insert(id, g(id));
    }
}

#[test]
fn a_longboat_takes_its_task_s_look_on_its_terrains_and_its_master_s_off_them() {
    let mut t = world();
    waters(&mut t);
    give_master(&mut t, 250, 70, 22, vec![longboat_look()]);
    put(&mut t, 1, 70, 250, 100.0, [5.5, 6.5]);
    let mut mover = crate::gathering::hunt::tests::walker();
    mover.moved_at = [5.5, 6.5, 0.0];
    t.players[1].objects[0].body.put_mover(Some(mover));
    run(&mut t, 1, 70);
    let u = &t.players[1].objects[0];
    assert_eq!(u.body.mover().unwrap().factor, 0.35);
    assert_eq!(u.abilities.attack_rate, 1.0);
    assert_eq!(
        u.abilities.look,
        Some(Look {
            standing: Some(19599),
            walking: Some(19601),
            attacking: Some(19599),
            running: None,
            dying: Some(19598),
            undead: Some(19600),
        })
    );
    // where it last moved to decides, not its place
    t.players[1].objects[0].body.mover_mut().unwrap().moved_at = [5.5, 7.5, 0.0];
    run(&mut t, 1, 70);
    let u = &t.players[1].objects[0];
    assert_eq!(u.body.mover().unwrap().factor, 1.0);
    assert_eq!((u.abilities.attack_rate, u.abilities.look), (1.0, None));
}

#[test]
fn a_dead_object_that_leaves_issues_its_removal_rewards() {
    let mut t = world();
    let far = Task {
        work_range: 1.0,
        ..bounty()
    };
    give_master(&mut t, 5, 70, 6, vec![bounty(), far, bounty()]);
    put(&mut t, 1, 80, 5, 0.0, [5.5, 5.5]);
    t.players[1].attributes[7] = 2.0;
    assert_eq!(rewards(&mut t, 1, 80, false), Vec::new());
    let r = Reward {
        player: 1,
        attribute: 3,
        amount: 20,
    };
    assert_eq!(rewards(&mut t, 1, 80, true), vec![r, r]);
    // alive, it issues none
    t.players[1].objects[0].life.hp = 1.0;
    assert_eq!(rewards(&mut t, 1, 80, true), Vec::new());
}

#[test]
fn an_object_of_another_type_has_no_abilities() {
    let mut t = world();
    give_master(&mut t, 7, 30, 6, vec![bounty()]);
    put(&mut t, 1, 90, 7, 0.0, [5.5, 5.5]);
    assert_eq!(rewards(&mut t, 1, 90, true), Vec::new());
}

/// `FUN_1416625b0`: an attribute below the player's count, the resources' from the slice's
/// own (food, wood, stone and gold); none past it.
#[test]
fn a_player_s_attribute_reads_the_resources_first() {
    let p = Player {
        resources: [1.0, 2.0, 3.0, 4.0],
        attributes: vec![9.0; 6],
        ..Player::default()
    };
    assert_eq!(p.attribute(2), Some(3.0));
    assert_eq!(p.attribute(5), Some(9.0));
    assert_eq!(p.attribute(6), None);
}
