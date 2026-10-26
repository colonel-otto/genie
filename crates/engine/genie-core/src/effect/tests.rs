use super::*;
use crate::masters::{Master, Masters};
use crate::turn::{Player, Unit};

/// A combat unit's charge after its master's maximum charge changes (`FUN_1412163c0`'s
/// `;`): a set and an add keep the charge's share of the maximum, without a maximum a set
/// takes the value and an add adds it, a multiply scales the charge.
#[test]
fn a_charge_keeps_its_share_of_a_changed_maximum() {
    assert_eq!(charge_after(10.0, 20.0, Change::Set, 40.0), 20.0);
    assert_eq!(charge_after(10.0, 0.0, Change::Set, 40.0), 40.0);
    assert_eq!(charge_after(10.0, 20.0, Change::Add, 20.0), 20.0);
    assert_eq!(charge_after(10.0, 0.0, Change::Add, 5.0), 15.0);
    assert_eq!(charge_after(10.0, 20.0, Change::Multiply, 1.5), 15.0);
}

/// An object has actions while it holds a top action or its production list holds one (a
/// building's Make); an empty queue or none at all is no action.
#[test]
fn an_object_has_actions_while_it_holds_one() {
    let mut u = Unit::default();
    assert!(!has_actions(&u));
    u.production = Some(crate::production::Production::default());
    assert!(!has_actions(&u));
    u.production.as_mut().unwrap().action = Some(crate::production::Making::Research(
        crate::production::MakeTech {
            state: 6,
            paid: false,
            tech: 22,
            master: -1,
            progress: 0.0,
        },
    ));
    assert!(has_actions(&u));
}

/// An Attack's slot 0x50 (`FUN_140a7f800`): its sprites take the master's unless they are
/// its task's (the walking sprite the task's moving one, the attack and standing sprites the
/// task's proceeding or working one), a sprite it lacks stays lacking, and only a combat
/// master's idle attack and second attack sprites replace its own.
#[test]
fn an_attack_takes_its_master_s_sprites_but_its_task_s() {
    let g = |id| crate::sprite::Graphic {
        id,
        frames: 10,
        facets: 1,
        speed_multiplier: 1.0,
        frame_duration: 0.1,
        replay_delay: 0.0,
        flags: 0,
    };
    let arms = crate::combat::attack::Arms {
        move_sprite: Some(g(1)),
        sprite: Some(g(2)),
        second_sprite: Some(g(3)),
        idle_sprite: Some(g(4)),
        idle_attack_sprite: None,
        missile: -1,
        fire_frame: 0,
        interrupt_frame: -1,
    };
    let sprites = MasterSprites {
        moving: Some(g(11)),
        attack: Some(g(12)),
        standing: Some(g(14)),
        idle_attack: Some(g(15)),
        second: Some(g(13)),
        combat: false,
    };
    let mut a = arms;
    follow_master(&mut a, None, &sprites);
    assert_eq!(
        [
            a.move_sprite,
            a.sprite,
            a.idle_sprite,
            a.second_sprite,
            a.idle_attack_sprite
        ],
        [Some(g(11)), Some(g(12)), Some(g(14)), Some(g(3)), None]
    );
    // the task's moving (1) and working (4, the standing sprite's) sprites stay
    let mut a = arms;
    follow_master(&mut a, Some([1, 9, 4, 9]), &sprites);
    assert_eq!(
        [a.move_sprite, a.sprite, a.idle_sprite],
        [Some(g(1)), Some(g(12)), Some(g(4))]
    );
    // a sprite it lacks stays lacking; a combat master's last two replace its own
    let mut a = crate::combat::attack::Arms {
        move_sprite: None,
        ..arms
    };
    follow_master(
        &mut a,
        None,
        &MasterSprites {
            combat: true,
            ..sprites
        },
    );
    assert_eq!(
        [a.move_sprite, a.second_sprite, a.idle_attack_sprite],
        [None, Some(g(13)), Some(g(15))]
    );
}

/// `FUN_1412163c0`'s population set (110) over the master's first three storage entries: the
/// headroom (4) takes back the old amount's magnitude and adds the value, the population (11)
/// and resource 19 take back the old amount and add the value's magnitude; other kinds and a
/// fourth entry change nothing.
#[test]
fn a_population_set_moves_each_storage_entry_to_the_new_value() {
    let mut t = Turns::new(
        0,
        0,
        0,
        vec![0],
        vec![Player {
            attributes: vec![0.0; 40],
            ..Player::default()
        }],
    );
    let master = Master {
        id: 70,
        storage: vec![
            (HEADROOM, -5.0, 0),
            (POPULATION, 3.0, 0),
            (7, 1.0, 0),
            (RESOURCE_19, 2.0, 0),
        ],
        ..Master::default()
    };
    t.population_storage(0, &master, -8.0).unwrap();
    let at = |i: i16| t.players[0].attributes[i as usize];
    assert_eq!(
        [at(HEADROOM), at(POPULATION), at(7), at(RESOURCE_19)],
        [-3.0, 5.0, 0.0, 0.0]
    );
}

/// `FUN_14118c540`: an object below state 7, not inside another, of a master with a height
/// takes its master's record again (kind and class not 0); one dying, inside or of a flat
/// master keeps none.
#[test]
fn an_object_takes_its_record_again_unless_dying_inside_or_flat() {
    let unit = |id, state: u8, inside: Option<i32>| {
        let mut u = Unit {
            id,
            position: [5.5, 5.5, 0.0],
            ..Unit::default()
        };
        u.life.master_id = 70;
        u.life.state = state;
        u.life.inside = inside;
        u
    };
    let mut t = Turns::new(
        0,
        0,
        0,
        vec![0],
        vec![Player {
            objects: vec![unit(1, 2, None), unit(2, 7, None), unit(3, 2, Some(9))],
            ..Player::default()
        }],
    );
    t.spatial = crate::spatial::Spatial::new([10, 10]);
    for id in 1..=3 {
        t.spatial.insert(
            id,
            crate::spatial::Placed {
                position: [5.5, 5.5, 0.0],
                radius: [0.5, 0.5],
                class: 0,
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
    let master = Master {
        id: 70,
        height: 2.0,
        obstruction_type: 2,
        obstruction_class: 2,
        ..Master::default()
    };
    for id in 1..=3 {
        t.give_record(id, &master);
    }
    let record = |t: &Turns, id| t.spatial.get(id).and_then(|p| p.obstruction);
    let wall = Some(crate::spatial::Obstruction { kind: 2, class: 2 });
    assert_eq!(
        [record(&t, 1), record(&t, 2), record(&t, 3)],
        [wall, None, None]
    );
    // a flat master gives none
    t.spatial.drop_record(1);
    t.give_record(
        1,
        &Master {
            height: 0.0,
            ..master
        },
    );
    assert_eq!(record(&t, 1), None);
}

/// `FUN_1416136f0` (slot 0x1a8) of villagers (0x53) at player 1's town centre: each draws
/// its sex from the Player stream (above 49 a female, 0x125), the owner's slot 0x1c0 makes it
/// at the centre's place, it goes inside (the centre has room) and, attribute 283 at 0, comes
/// out at once toward no point (no gather point: it stays beside the centre). A master the
/// player lacks, Gaia lacking it too, spawns nothing.
#[test]
fn an_effect_spawns_its_villagers_at_each_built_town_centre() {
    use crate::garrison::tests::{TC, VILLAGER, let_out_world};
    let mut t = let_out_world();
    let standing = crate::sprite::Graphic {
        id: 300,
        frames: 10,
        facets: 1,
        speed_multiplier: 1.0,
        frame_duration: 0.1,
        replay_delay: 0.0,
        flags: 1,
    };
    let _ = t.graphics.insert(300, standing);
    for ms in &mut t.masters {
        ms.get_mut(VILLAGER).unwrap().standing_sprite = 300;
        let female = Master {
            id: 0x125,
            ..ms.get(VILLAGER).unwrap().clone()
        };
        *ms = Masters::new(ms.iter().cloned().chain([female]));
        ms.get_mut(TC).unwrap().garrison = 15;
    }
    t.players[1].attributes = vec![0.0; 600];
    let before = t.players[1].objects.len();
    let mut stream = t.streams.get(Stream::Player).clone();
    t.spawn_at(1, VILLAGER, TC, 2).unwrap();
    let made: Vec<(i16, Option<i32>)> = t.players[1].objects[before..]
        .iter()
        .map(|u| (u.life.master_id, u.life.inside))
        .collect();
    let sex = |r: u32| if 49 < r as i32 % 100 { 0x125 } else { VILLAGER };
    let expected = [sex(stream.rand15()), sex(stream.rand15())];
    assert_eq!(made, vec![(expected[0], None), (expected[1], None)]);
    assert_eq!(t.object(100).unwrap().1.garrison, Vec::<i32>::new());
    // a master neither the player nor Gaia has
    let mut t = let_out_world();
    let before = t.players[1].objects.len();
    t.spawn_at(1, 999, TC, 2).unwrap();
    assert_eq!(t.players[1].objects.len(), before);
}

/// Loom's effect (22) on player 2 of MarvinBlue/426048731 in update 4588: villagers (class 4)
/// of 25 hit points take 15 more, one melee and two pierce armor.
#[test]
fn loom_adds_hit_points_to_its_villagers_and_their_master_and_armor_to_the_master() {
    let villager = |id, hp| {
        let mut u = Unit {
            id,
            ..Unit::default()
        };
        u.life.master_id = 83;
        u.life.hp = hp;
        u
    };
    let player = Player {
        objects: vec![villager(9175, 25.0), villager(9177, 10.0)],
        ..Player::default()
    };
    let mut t = Turns::new(0, 0, 0, vec![0], vec![player]);
    t.masters = vec![Masters::new([Master {
        id: 83,
        kind: 70,
        class: 4,
        hit_points: 25,
        armors: vec![(4, 0), (3, 0)],
        ..Master::default()
    }])];
    let loom = [
        EffectCommand {
            kind: 4,
            a: -1,
            b: 4,
            c: 0,
            d: 15.0,
        },
        EffectCommand {
            kind: 4,
            a: -1,
            b: 4,
            c: 8,
            d: 1025.0,
        },
        EffectCommand {
            kind: 4,
            a: -1,
            b: 4,
            c: 8,
            d: 770.0,
        },
    ];
    // the load files the objects in the players' trackers
    t.load_trackers();
    t.do_effect(0, &loom, None).unwrap();
    let hp: Vec<f32> = t.players[0].objects.iter().map(|u| u.life.hp).collect();
    // (25 + 15) × 10 / 25 = 16
    assert_eq!(hp, [40.0, 16.0]);
    let m = t.masters[0].get(83).unwrap();
    assert_eq!((m.hit_points, m.armors.clone()), (40, vec![(4, 1), (3, 2)]));
}

/// A unit of `master` with `hp` hit points.
/// A building's construction whose build points reached its build time: built, so the load
/// completes it in its player's tracker ([`Turns::load_trackers`]).
fn built() -> crate::build::Construction {
    crate::build::Construction {
        progress: 1.0,
        desolid: false,
        builders: 0,
        hit_points: 1,
        build_time: 1,
        record: crate::spatial::Obstruction { kind: 0, class: 0 },
        snow: false,
        sprite_override: false,
        started: false,
    }
}

fn unit_of(id: i32, master: i16, hp: f32) -> Unit {
    let mut u = Unit {
        id,
        ..Unit::default()
    };
    u.life.master_id = master;
    u.life.hp = hp;
    u.life.state = 2;
    u
}

fn combat(id: i16, class: i16, hit_points: i16) -> Master {
    Master {
        id,
        kind: 70,
        class,
        hit_points,
        ..Master::default()
    }
}

fn attribute(kind: u8, a: i16, b: i16, c: i16, d: f32) -> EffectCommand {
    EffectCommand { kind, a, b, c, d }
}

/// `FUN_14165a400` (a master within the table, kind 0 with its id): only that master's
/// objects and the master change; a master id the table holds no master at (a null slot)
/// changes nothing, the class besides.
#[test]
fn an_effect_on_one_master_changes_its_objects_and_it_alone() {
    let player = Player {
        objects: vec![unit_of(1, 4, 30.0), unit_of(2, 5, 30.0)],
        ..Player::default()
    };
    let mut t = Turns::new(0, 0, 0, vec![0], vec![player]);
    t.masters = vec![Masters::new([
        combat(4, 6, 60),
        combat(5, 6, 60),
        combat(7, 6, 60),
    ])];
    // set master 4's hit points to 90: its unit keeps its share, 30 × 90 / 60 = 45
    // the load files the objects in the players' trackers
    t.load_trackers();
    t.do_effect(0, &[attribute(0, 4, 6, 0, 90.0)], None)
        .unwrap();
    let hp: Vec<f32> = t.players[0].objects.iter().map(|u| u.life.hp).collect();
    assert_eq!(hp, [45.0, 30.0]);
    let hps: Vec<i16> = t.masters[0].iter().map(|m| m.hit_points).collect();
    assert_eq!(hps, [90, 60, 60]);
    // master 6 lies within the table (up to 7) but has no master: nothing, not the class
    t.do_effect(0, &[attribute(4, 6, 6, 0, 10.0)], None)
        .unwrap();
    let hps: Vec<i16> = t.masters[0].iter().map(|m| m.hit_points).collect();
    assert_eq!(hps, [90, 60, 60]);
    // past the table the class changes: (60 + 10) × 30 / 60 = 35
    t.do_effect(0, &[attribute(4, 100, 6, 0, 10.0)], None)
        .unwrap();
    let hps: Vec<i16> = t.masters[0].iter().map(|m| m.hit_points).collect();
    assert_eq!(hps, [100, 70, 70]);
    assert_eq!(t.players[0].objects[1].life.hp, 35.0);
}

/// `FUN_141614f40`'s hooks: a building's volleys lose the shares of the combat units inside
/// before the change and take the new shares after (`FUN_141217ac0`, `FUN_14120b910`); the
/// change of the archers' attack moves an archer's share.
#[test]
fn a_change_moves_the_shares_of_the_units_inside_a_building() {
    let mut tower = unit_of(10, 79, 1000.0);
    tower.garrison = vec![11];
    tower.firepower = 2.0;
    let mut archer = unit_of(11, 4, 30.0);
    archer.life.inside = Some(10);
    let player = Player {
        objects: vec![tower, archer],
        ..Player::default()
    };
    let mut t = Turns::new(0, 0, 0, vec![0], vec![player]);
    let mut tower_master = combat(79, 52, 1020);
    tower_master.kind = 80;
    tower_master.reload = 2.0;
    tower_master.weapon_range = 8.0;
    tower_master.weapons = vec![(3, 5)];
    let mut archer_master = combat(4, 0, 30);
    archer_master.reload = 2.0;
    archer_master.weapons = vec![(3, 4)];
    archer_master.garrison_firepower = 1.0;
    t.masters = vec![Masters::new([archer_master, tower_master])];
    // the tower's rate: 5 / 2 = 2.5; the archer's share 4 / 2 / 2.5 = 0.8, then 6 / 2 / 2.5
    t.do_effect(0, &[attribute(4, -1, 0, 9, f32::from(0x302_i16))], None)
        .unwrap();
    assert_eq!(t.masters[0].get(4).unwrap().weapons, [(3, 6)]);
    assert_eq!(t.players[0].objects[0].firepower, 2.0 - 0.8 + 1.2);
}

/// The class path (`FUN_14165a000`): a building of the class takes the change, then its
/// annexes of the class, which the tracker files under no class; a foundation of the class
/// (pending) takes it too.
#[test]
fn a_class_effect_reaches_buildings_their_annexes_and_foundations() {
    let mut main = unit_of(20, 30, 100.0);
    main.life.master_id = 30;
    main.construction = Some(built());
    let mut annex = unit_of(21, 31, 50.0);
    annex.annex_of = Some(20);
    annex.construction = Some(built());
    let mut foundation = unit_of(22, 30, 10.0);
    foundation.life.state = 0;
    let player = Player {
        objects: vec![main, annex, foundation],
        ..Player::default()
    };
    let mut t = Turns::new(0, 0, 0, vec![0], vec![player]);
    let building = |id, hit_points, head_unit, annexes| Master {
        id,
        kind: 80,
        class: 3,
        hit_points,
        building: Some(crate::masters::BuildingMaster {
            stack_unit: -1,
            head_unit,
            annexes,
            ..crate::masters::BuildingMaster::default()
        }),
        ..Master::default()
    };
    t.masters = vec![Masters::new([
        building(30, 200, -1, [31, -1, -1, -1]),
        building(31, 100, 30, [-1; 4]),
    ])];
    // multiply the class's hit points by 1.5: every object's hit points × 1.5 + 0.5
    // the load files the objects in the players' trackers
    t.load_trackers();
    t.do_effect(0, &[attribute(5, -1, 3, 0, 1.5)], None)
        .unwrap();
    let hp: Vec<f32> = t.players[0].objects.iter().map(|u| u.life.hp).collect();
    assert_eq!(hp, [150.0, 75.0, 15.0]);
    let hps: Vec<i16> = t.masters[0].iter().map(|m| m.hit_points).collect();
    assert_eq!(hps, [300, 150]);
}

/// The allies' block (14: add) reaches the player's mutual allies, itself among them, and
/// Gaia's (44) Gaia alone.
#[test]
fn a_team_effect_reaches_the_mutual_allies() {
    let player = |relations: Vec<u8>| Player {
        relations,
        ..Player::default()
    };
    let mut t = Turns::new(
        0,
        0,
        0,
        vec![0; 4],
        vec![
            player(vec![0, 3, 3, 3]),
            player(vec![3, 0, 0, 3]),
            player(vec![3, 0, 0, 3]),
            player(vec![3, 3, 3, 0]),
        ],
    );
    t.masters = vec![Masters::new([combat(4, 6, 60)]); 4];
    t.do_effect(1, &[attribute(14, -1, 6, 0, 5.0)], None)
        .unwrap();
    let hps: Vec<i16> = t
        .masters
        .iter()
        .map(|m| m.get(4).unwrap().hit_points)
        .collect();
    assert_eq!(hps, [60, 65, 65, 60]);
    t.do_effect(1, &[attribute(44, -1, 6, 0, 5.0)], None)
        .unwrap();
    let hps: Vec<i16> = t
        .masters
        .iter()
        .map(|m| m.get(4).unwrap().hit_points)
        .collect();
    assert_eq!(hps, [65, 65, 65, 60]);
    // the enemies' block (24): player 3 counts player 1 an enemy and is counted one
    t.do_effect(1, &[attribute(24, -1, 6, 0, 5.0)], None)
        .unwrap();
    let hps: Vec<i16> = t
        .masters
        .iter()
        .map(|m| m.get(4).unwrap().hit_points)
        .collect();
    assert_eq!(hps, [65, 65, 65, 65]);
}

/// A set of a resource storage (`FUN_141195a50`'s 21): each object's player loses the old
/// amount and gains the new (slot 0xf8 twice), unless the master keeps it (`'@'`).
#[test]
fn a_storage_set_moves_the_player_s_attribute_by_each_object() {
    let player = Player {
        objects: vec![unit_of(1, 70, 550.0), unit_of(2, 70, 550.0)],
        attributes: vec![0.0; 40],
        ..Player::default()
    };
    let mut t = Turns::new(0, 0, 0, vec![0], vec![player]);
    let mut house = combat(70, 3, 550);
    house.storage = vec![(4, 5.0, 4), (-1, 0.0, 0), (-1, 0.0, 0)];
    t.masters = vec![Masters::new([house])];
    t.players[0].attributes[4] = 10.0;
    // the load files the objects in the players' trackers
    t.load_trackers();
    t.do_effect(0, &[attribute(0, 70, -1, 21, 15.0)], None)
        .unwrap();
    assert_eq!(t.players[0].attributes[4], 30.0);
    assert_eq!(t.masters[0].get(70).unwrap().storage[0].1, 15.0);
}

/// `FUN_141195a50`'s line of sight on a static object (kind 10) on the map: its old circle
/// (the seer's own, 2) goes and the new one is explored (a set: the value; an add: the
/// master's plus it; a multiply: the master's times it, truncated); off the map (state 0)
/// nothing. A set of the second storage (26) moves its attribute as the first does.
#[test]
fn a_static_object_s_line_of_sight_and_second_storage_take_the_change() {
    let (w, h) = (20, 20);
    let n = (w * h) as usize;
    let sight = |t: &Turns| {
        let map = &t.spatial.sight.as_ref().unwrap().maps[0];
        map.counts.iter().filter(|&&c| c > 0).count()
    };
    let world = || {
        let player = Player {
            objects: vec![unit_of(1, 50, 10.0)],
            attributes: vec![0.0; 40],
            ..Player::default()
        };
        let mut t = Turns::new(0, 0, 0, vec![0], vec![player]);
        let stone = Master {
            id: 50,
            kind: 10,
            class: 11,
            hit_points: 10,
            line_of_sight: 2.0,
            storage: vec![(-1, 0.0, 0), (4, 5.0, 0), (-1, 0.0, 0)],
            ..Master::default()
        };
        t.masters = vec![Masters::new([stone])];
        t.load_trackers();
        let mut v = crate::vision::visibility::Visibility::new(
            w,
            h,
            vec![crate::vision::visibility::VisibleMap {
                counts: vec![0; n],
                explored: 0,
            }],
            vec![0; n],
            Vec::new(),
            0,
            vec![false],
        );
        let seer = crate::vision::visibility::Seer {
            player: 0,
            master: 50,
            los: 2.0,
            state: 2,
            explored: true,
            building: None,
            umv: 0,
            capture: 0,
            kind: 10,
        };
        v.add_seer(1, seer);
        let position = [10.5, 10.5, 0.0];
        let radius = [0.5, 0.5];
        let _ = v.sight_on(1, position, radius, false, false);
        t.spatial.sight = Some(v);
        t.spatial.insert(
            1,
            crate::spatial::Placed {
                position,
                radius,
                class: 11,
                player: 0,
                master: 50,
                obstruction_type: 0,
                buildable_on: 0,
                tall: false,
                speed: 0.0,
                unknown: None,
                obstruction: None,
            },
        );
        t
    };
    // the tiles a circle of `los` explores, on a map of its own
    let circle = |los| {
        let mut t = world();
        let v = t.spatial.sight.as_mut().unwrap();
        v.sight_off(1, [10.5, 10.5, 0.0], [0.5, 0.5], false, false);
        let _ = v.sight_on_los(1, [10.5, 10.5, 0.0], [0.5, 0.5], false, los, false);
        sight(&t)
    };
    let (two, four, five, six) = (circle(2), circle(4), circle(5), circle(6));
    assert!(two < four && four < five && five < six);
    let mut t = world();
    assert_eq!(sight(&t), two);
    // a set to 4.9: truncated to 4
    t.do_effect(0, &[attribute(0, -1, 11, 1, 4.9)], None)
        .unwrap();
    assert_eq!(sight(&t), four);
    // an add of 3: the master's 2 (the clone before its own change) plus 3
    let mut t = world();
    t.do_effect(0, &[attribute(4, -1, 11, 1, 3.0)], None)
        .unwrap();
    assert_eq!(sight(&t), five);
    // a multiply by 3: the master's 2 times 3
    let mut t = world();
    t.do_effect(0, &[attribute(5, -1, 11, 1, 3.0)], None)
        .unwrap();
    assert_eq!(sight(&t), six);
    // off the map: the circle stays
    let mut t = world();
    t.players[0].objects[0].life.state = 0;
    t.do_effect(0, &[attribute(0, -1, 11, 1, 6.0)], None)
        .unwrap();
    assert_eq!(sight(&t), two);
    // the second storage: its attribute 4 loses the old 5 and takes the new 8
    let mut t = world();
    t.players[0].attributes[4] = 10.0;
    t.do_effect(0, &[attribute(0, -1, 11, 26, 8.0)], None)
        .unwrap();
    assert_eq!(t.players[0].attributes[4], 13.0);
}

/// What the slice keeps beside a master follows its change: its objects' (live and asleep)
/// dying sprite (none below 0), death spawn (a first spawn of 0 or more) and remains (the
/// second), a foundation's build time and record, a placed object's height test and a
/// seer's line of sight; another master's objects and a master the player lacks keep theirs.
#[test]
fn an_object_s_copies_of_its_master_follow_its_change() {
    use crate::masters::attribute as a;
    let mut site = unit_of(1, 4, 30.0);
    site.construction = Some(built());
    let player = Player {
        objects: vec![site, unit_of(3, 5, 30.0)],
        sleeping: vec![unit_of(2, 4, 30.0)],
        ..Player::default()
    };
    let mut t = Turns::new(0, 0, 0, vec![0], vec![player]);
    let mut m = combat(4, 6, 60);
    (m.death_sprite, m.death_spawns) = (7, [0, 9]);
    (m.build_time, m.obstruction_type, m.obstruction_class) = (33, 5, 2);
    (m.line_of_sight, m.tall) = (6.0, true);
    t.masters = vec![Masters::new([m, combat(5, 6, 60)])];
    for id in [1, 3] {
        t.spatial.insert(
            id,
            crate::spatial::Placed {
                position: [5.5, 5.5, 0.0],
                radius: [0.5, 0.5],
                class: 6,
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
    let mut v = crate::vision::visibility::Visibility::new(
        10,
        10,
        Vec::new(),
        vec![0; 100],
        Vec::new(),
        0,
        Vec::new(),
    );
    let seer = crate::vision::visibility::Seer {
        player: 0,
        master: 4,
        los: 2.0,
        state: 2,
        explored: true,
        building: None,
        umv: 0,
        capture: 0,
        kind: 70,
    };
    v.add_seer(1, seer);
    v.add_seer(3, seer);
    t.spatial.sight = Some(v);
    let all = [
        a::DYING_GRAPHIC,
        a::DEAD_UNIT_ID,
        a::BLOOD_UNIT,
        a::TRAIN_TIME,
        a::OBSTRUCTION_TYPE,
        a::SIZE_Z,
        a::LINE_OF_SIGHT,
    ];
    for attribute in all {
        t.master_followers(0, 4, attribute);
        t.master_followers(0, 99, attribute);
    }
    let follows = |u: &Unit| (u.life.dying_sprite, u.life.death_spawn, u.life.remains);
    let p = &t.players[0];
    assert_eq!(follows(&p.objects[0]), (Some(7), true, 9));
    assert_eq!(follows(&p.sleeping[0]), (Some(7), true, 9));
    assert_eq!(follows(&p.objects[1]), (None, false, -1));
    let c = p.objects[0].construction.unwrap();
    assert_eq!(c.build_time, 33);
    assert_eq!(c.record, crate::spatial::Obstruction { kind: 5, class: 2 });
    let tall = |id| t.spatial.get(id).unwrap().tall;
    assert_eq!((tall(1), tall(3)), (true, false));
    let los = |id| t.spatial.sight.as_ref().unwrap().seer(id).unwrap().los;
    assert_eq!((los(1), los(3)), (6.0, 2.0));
    // a dying sprite below 0 is none
    t.masters[0].get_mut(4).unwrap().death_sprite = -1;
    t.master_followers(0, 4, a::DYING_GRAPHIC);
    assert_eq!(t.players[0].objects[0].life.dying_sprite, None);
}

/// A speed's change reaches what a unit of the master moves with (the game reads its master).
#[test]
fn a_speed_change_reaches_the_units_movement() {
    let mut walker = unit_of(1, 4, 25.0);
    let mover = crate::hunt::tests::walker();
    let stats = mover.stats;
    walker.mover = Some(mover);
    let player = Player {
        objects: vec![walker],
        ..Player::default()
    };
    let mut t = Turns::new(0, 0, 0, vec![0], vec![player]);
    let mut villager = combat(4, 4, 25);
    villager.max_speed = 0.8;
    villager.moving = Some(stats);
    t.masters = vec![Masters::new([villager])];
    t.do_effect(0, &[attribute(5, -1, 4, 5, 1.1)], None)
        .unwrap();
    let s = t.players[0].objects[0].mover.as_ref().unwrap().stats;
    // no graphics: the sprites' factors are 0, the speeds the master's
    let speed = 0.8_f32 * 1.1;
    assert_eq!(
        (s.speed, s.walk_speed, s.stand_speed),
        (speed, speed, speed)
    );
    // a speed the loader left at 0 under the moving values stops the change first
    t.masters[0].get_mut(4).unwrap().max_speed = 0.0;
    assert!(
        t.do_effect(0, &[attribute(5, -1, 4, 5, 1.1)], None)
            .is_err()
    );
    let stats = t.players[0].objects[0].mover.as_ref().unwrap().stats;
    assert_eq!(stats.speed, speed);
}

/// The resource commands (kind 1, `FUN_141620c70`): mode 0 sets the attribute, any other adds
/// to it, scaled by the attribute the third argument names; kind 6 multiplies it
/// (`FUN_1416614c0`).
#[test]
fn the_resource_commands_set_add_and_multiply_an_attribute() {
    let player = Player {
        attributes: vec![0.0; 300],
        ..Player::default()
    };
    let mut t = Turns::new(0, 0, 0, vec![0], vec![player]);
    t.players[0].attributes[69] = 2.0;
    t.do_effect(0, &[attribute(1, 78, 0, -1, 0.15)], None)
        .unwrap();
    // Horse Collar: 75 × attribute 69 added to 36
    t.do_effect(0, &[attribute(1, 36, 1, 69, 75.0)], None)
        .unwrap();
    t.do_effect(0, &[attribute(6, 78, -1, -1, 2.0)], None)
        .unwrap();
    // gold, a resource, lives beside the attributes
    t.do_effect(0, &[attribute(1, 3, 1, -1, 50.0)], None)
        .unwrap();
    let a = &t.players[0].attributes;
    assert_eq!(
        (a[78], a[36], t.players[0].resources[3]),
        (0.3, 150.0, 50.0)
    );
}

/// A change of the farms' food (resource 0x24) grows the food each farm still makes:
/// `(f / before)^exponent × amount + f − held` with `f = held + still to make`
/// (`FUN_141615270`), the exponent attribute 31.
#[test]
fn a_farm_food_change_grows_the_farms_food() {
    let mut farm = unit_of(5, 50, 480.0);
    farm.life.amount = 75.0;
    farm.construction = Some(built());
    farm.make = Some(crate::make::MakeAction {
        state: 1,
        remaining: 100.0,
        total: 175.0,
        task: crate::masters::Task::default(),
    });
    let player = Player {
        objects: vec![farm],
        attributes: vec![0.0; 300],
        ..Player::default()
    };
    let mut t = Turns::new(0, 0, 0, vec![0], vec![player]);
    let mut master = combat(50, 49, 480);
    master.kind = 80;
    t.masters = vec![Masters::new([master])];
    t.players[0].attributes[31] = 1.0;
    t.players[0].attributes[36] = 175.0;
    // the load files the objects in the players' trackers
    t.load_trackers();
    t.do_effect(0, &[attribute(1, 36, 1, -1, 75.0)], None)
        .unwrap();
    assert_eq!(t.players[0].attributes[36], 250.0);
    // (175 / 175)^1 × 75 + 175 − 75
    assert_eq!(t.players[0].objects[0].make.unwrap().remaining, 175.0);
}

/// Cartography (resource 50, added): the allied vision turns on and the masks are rebuilt
/// before the attribute takes its sum (`FUN_141616820`); Treason's set (0xb7) rebuilds them
/// after; resource 0xc4 spreads to the players at most 0 unless the player's is above 0; the
/// scripts' effect function (0x21 above 0) is not modelled.
#[test]
fn the_resource_commands_side_effects() {
    let player = |relations: Vec<u8>| Player {
        relations,
        attributes: vec![0.0; 300],
        ..Player::default()
    };
    let mut t = Turns::new(
        0,
        0,
        0,
        vec![0; 3],
        vec![
            player(vec![0, 3, 3]),
            player(vec![3, 0, 0]),
            player(vec![3, 0, 0]),
        ],
    );
    t.spatial.vision = vec![[0; 2]; 3];
    t.do_effect(1, &[attribute(1, 50, 1, -1, 1.0)], None)
        .unwrap();
    assert!(t.players[1].allied_vision);
    assert_eq!(t.players[1].attributes[50], 1.0);
    assert_eq!(t.spatial.vision[1][0] & 0b110, 0b110);
    t.do_effect(2, &[attribute(1, 0xc4, 0, -1, 7.0)], None)
        .unwrap();
    let spread: Vec<f32> = t.players.iter().map(|p| p.attributes[0xc4]).collect();
    assert_eq!(spread, [0.0, 7.0, 7.0]);
    t.do_effect(1, &[attribute(1, 0xc4, 0, -1, 9.0)], None)
        .unwrap();
    assert_eq!(t.players[1].attributes[0xc4], 7.0);
    t.do_effect(1, &[attribute(1, 0xb7, 0, -1, 1.0)], None)
        .unwrap();
    assert_eq!(t.spatial.vision[1][0] & 0b110, 0b110);
    assert!(
        t.do_effect(1, &[attribute(1, 0x21, 0, -1, 7.0)], None)
            .is_err()
    );
}

/// A technology at the barracks (master 12, 30 seconds in the `.dat`) in state 1 with 20
/// seconds of work done.
fn barracks_tech() -> Turns {
    use crate::tech::{Location, TechDef, TechState};
    let state = TechState {
        state: 1,
        research_done: 20.0,
        locations: vec![Location {
            building: 12,
            time: 30,
            button: 3,
            adjustment: 0.0,
        }],
        building_master: -1,
        ..TechState::default()
    };
    let player = Player {
        techs: vec![state],
        attributes: vec![0.0; 300],
        ..Player::default()
    };
    let mut t = Turns::new(0, 0, 0, vec![0], vec![player]);
    t.techs = vec![TechDef {
        required: [-1; 6],
        locations: vec![(12, 30)],
        ..TechDef::default()
    }];
    t
}

/// `FUN_1413d1210`: mode 0 sets the time (the adjustment is the value less the `.dat`'s
/// time), mode 1 adds the truncated value, mode 2 adds `(v − 1) × time` rounded half away
/// from 1; once the work done reaches the new time a technology in state 1 completes.
#[test]
fn the_research_time_modifier_changes_the_adjustment_and_may_complete() {
    let mut t = barracks_tech();
    t.do_effect(0, &[attribute(b'g', 0, -1, 1, 4.9)], None)
        .unwrap();
    assert_eq!(t.players[0].techs[0].locations[0].adjustment, 4.0);
    t.do_effect(0, &[attribute(b'g', 0, -1, 2, 0.8)], None)
        .unwrap();
    // (0.8 − 1) × 30 − 0.5 = −6.5, truncated −6, plus 4
    assert_eq!(t.players[0].techs[0].locations[0].adjustment, -2.0);
    assert_eq!(t.players[0].techs[0].state, 1);
    // set to 20: the adjustment −10, and 20 seconds are done
    t.do_effect(0, &[attribute(b'g', 0, -1, 0, 20.0)], None)
        .unwrap();
    assert_eq!(t.players[0].techs[0].locations[0].adjustment, -10.0);
    assert_eq!(t.players[0].techs[0].state, 3);
}

/// Kind 8 (`FUN_1415c3f20`'s technology change): a cost mode changes a cost (0x4002: add to
/// stone), 4 a location's building to a master the player has, 5 its button, 10 the research
/// cap, 12 the state, −1 the research time (with the dependents' update).
#[test]
fn a_technology_change_reaches_its_costs_locations_and_state() {
    use crate::tech::Cost;
    let mut t = barracks_tech();
    t.techs[0].costs[0] = Cost {
        attribute: 2,
        amount: 100,
        paid: 1,
    };
    t.masters = vec![Masters::new([combat(87, 3, 1000)])];
    t.do_effect(0, &[attribute(8, 0, 0x4002, -1, 25.0)], None)
        .unwrap();
    assert_eq!(t.players[0].techs[0].cost_adj, [25, 0, 0]);
    // a building the player lacks leaves the location
    t.do_effect(0, &[attribute(8, 0, 4, -1, 88.0)], None)
        .unwrap();
    t.do_effect(0, &[attribute(8, 0, 4, -1, 87.0)], None)
        .unwrap();
    t.do_effect(0, &[attribute(8, 0, 5, -1, 7.0)], None)
        .unwrap();
    t.do_effect(0, &[attribute(8, 0, 10, -1, 2.0)], None)
        .unwrap();
    let s = &t.players[0].techs[0];
    assert_eq!((s.locations[0].building, s.locations[0].button), (87, 7));
    assert_eq!(s.research_cap, 2);
    // a location past the list adds a copy of the last, without a building
    t.do_effect(0, &[attribute(8, 0, 5, 3, 9.0)], None).unwrap();
    let s = &t.players[0].techs[0];
    assert_eq!(s.locations.len(), 2);
    assert_eq!((s.locations[1].building, s.locations[1].button), (-1, 9));
    t.do_effect(0, &[attribute(8, 0, 12, -1, 0.0)], None)
        .unwrap();
    assert_eq!(t.players[0].techs[0].state, -1);
    t.do_effect(0, &[attribute(8, 0, 12, -1, 2.0)], None)
        .unwrap();
    assert_eq!(
        (t.players[0].techs[0].state, t.players[0].techs[0].enabled),
        (1, true)
    );
    // state 3 completes it (its first location has a building and a button: it counts);
    // once researched, again gives its effect and counts whatever its location
    t.do_effect(0, &[attribute(8, 0, 12, -1, 3.0)], None)
        .unwrap();
    assert_eq!(t.players[0].techs[0].state, 3);
    assert_eq!(t.players[0].attributes[0x15], 1.0);
    t.do_effect(0, &[attribute(8, 0, 12, -1, 3.0)], None)
        .unwrap();
    assert_eq!(t.players[0].attributes[0x15], 2.0);
}

/// Kind 8's other modes: a technology past the player's tree changes nothing; a location
/// within the list changes it alone (−2, a research time's add: at its building); one past
/// the list of a technology without locations changes nothing; 9 the stacking, 12 with 1
/// makes it unavailable; 18 the effect override (a researched technology's new effect would
/// run from the world's effect table, not modelled).
#[test]
fn a_technology_change_s_location_stacking_and_effect_override() {
    use crate::tech::Location;
    let mut t = barracks_tech();
    t.players[0].techs[0].locations.push(Location {
        building: 13,
        time: 40,
        button: 4,
        adjustment: 0.0,
    });
    t.do_effect(0, &[attribute(8, 5, 9, -1, 1.0)], None)
        .unwrap();
    assert!(!t.players[0].techs[0].stack);
    t.do_effect(0, &[attribute(8, 0, 9, -1, 1.0)], None)
        .unwrap();
    assert!(t.players[0].techs[0].stack);
    t.do_effect(0, &[attribute(8, 0, 5, 1, 9.0)], None).unwrap();
    t.do_effect(0, &[attribute(8, 0, -2, 1, 5.9)], None)
        .unwrap();
    let s = &t.players[0].techs[0];
    let at = |i: usize| (s.locations[i].button, s.locations[i].adjustment);
    assert_eq!([at(0), at(1)], [(3, 0.0), (9, 5.0)]);
    t.do_effect(0, &[attribute(8, 0, 12, -1, 1.0)], None)
        .unwrap();
    assert_eq!(t.players[0].techs[0].state, 0);
    t.do_effect(0, &[attribute(8, 0, 18, -1, 7.0)], None)
        .unwrap();
    assert_eq!(t.players[0].techs[0].effect_override, 7);
    t.players[0].techs[0].state = 3;
    assert!(
        t.do_effect(0, &[attribute(8, 0, 18, -1, 8.0)], None)
            .is_err()
    );
    // no locations: a location past the list changes nothing
    let mut t = barracks_tech();
    t.players[0].techs[0].locations.clear();
    t.do_effect(0, &[attribute(8, 0, 5, 2, 9.0)], None).unwrap();
    assert_eq!(t.players[0].techs[0].locations.len(), 0);
}

/// Kind 3 (`FUN_14160e410`, mode −1): the old master takes the new one's values but keeps its
/// id, class, availability and disabling (`Master::copy_from`); its objects first take the
/// new one through their slot 0x98 ([`Turns::upgrade_object`]).
#[test]
fn an_upgrade_copies_the_new_master_into_the_old() {
    let player = Player::default();
    let mut t = Turns::new(0, 0, 0, vec![0], vec![player]);
    let mut militia = combat(74, 6, 40);
    militia.available = true;
    let mut man_at_arms = combat(75, 6, 45);
    man_at_arms.weapons = vec![(4, 6)];
    man_at_arms.available = false;
    t.masters = vec![Masters::new([militia, man_at_arms])];
    t.do_effect(0, &[attribute(3, 74, 75, -1, 0.0)], None)
        .unwrap();
    let m = t.masters[0].get(74).unwrap();
    assert_eq!(
        (m.id, m.hit_points, m.weapons.clone()),
        (74, 45, vec![(4, 6)])
    );
    assert!(m.available);
    // mode 1 leaves the master
    t.masters[0].get_mut(75).unwrap().hit_points = 50;
    t.do_effect(0, &[attribute(3, 74, 75, 1, 0.0)], None)
        .unwrap();
    assert_eq!(t.masters[0].get(74).unwrap().hit_points, 45);
    // with an object: its slot 0x98 keeps its share of the hit points (45 of 45: 50 of 50)
    t.players[0].objects.push(unit_of(1, 74, 45.0));
    t.do_effect(0, &[attribute(3, 74, 75, -1, 0.0)], None)
        .unwrap();
    let u = &t.players[0].objects[0];
    assert_eq!((u.life.master_id, u.life.hp), (74, 50.0));
    assert_eq!(t.masters[0].get(74).unwrap().hit_points, 50);
}

/// Kind 7 (`FUN_1416136f0`) makes objects at the built buildings of a master, which this file
/// leaves to the spawning; with none built it does nothing. Kind 5 on a cost by a negative
/// amount removes (−2) or adds (−1) a cost entry (`FUN_1415c1f70`).
#[test]
fn spawns_and_cost_entries() {
    let mut centre = unit_of(1, 109, 2400.0);
    centre.life.state = 0;
    let player = Player {
        objects: vec![centre],
        ..Player::default()
    };
    let mut t = Turns::new(0, 0, 0, vec![0], vec![player]);
    let mut tc = combat(109, 80, 2400);
    tc.kind = 80;
    let mut serjeant = combat(1658, 6, 60);
    serjeant.cost = vec![(0, 60, true), (2, 35, true)];
    t.masters = vec![Masters::new([tc, serjeant])];
    t.do_effect(0, &[attribute(7, 1658, 109, 5, 0.0)], None)
        .unwrap();
    t.players[0].objects[0].life.state = 2;
    assert!(
        t.do_effect(0, &[attribute(7, 1658, 109, 5, 0.0)], None)
            .is_err()
    );
    t.do_effect(0, &[attribute(5, 1658, -1, 106, -2.0)], None)
        .unwrap();
    assert_eq!(t.masters[0].get(1658).unwrap().cost, [(0, 60, true)]);
    t.do_effect(0, &[attribute(5, 1658, -1, 105, -1.0)], None)
        .unwrap();
    assert_eq!(
        t.masters[0].get(1658).unwrap().cost,
        [(0, 60, true), (3, -1, true)]
    );
}

/// The objects' kinds: 200 to 202 change the objects with masters of their own (none held);
/// 203 to 206 need the effect's object, which a research has not.
#[test]
fn the_object_kinds_without_an_object_do_nothing() {
    let mut t = Turns::new(0, 0, 0, vec![0], vec![Player::default()]);
    t.masters = vec![Masters::new([combat(82, 52, 4800)])];
    for kind in 200..=206 {
        t.do_effect(0, &[attribute(kind, 82, -1, 8, 770.0)], None)
            .unwrap();
    }
    assert!(
        t.do_effect(0, &[attribute(204, 82, -1, 0, 1500.0)], Some(5))
            .is_err()
    );
    assert_eq!(t.masters[0].get(82).unwrap().hit_points, 4800);
}

/// The enable command (`FUN_1416665b0`, kind 2): `available` takes the byte unless the master
/// is disabled; master 0x26d is never enabled in game mode 11.
#[test]
fn the_enable_command_sets_available_unless_disabled() {
    let mut t = Turns::new(0, 0, 0, vec![0], vec![Player::default()]);
    let mut locked = combat(5, 6, 10);
    locked.disabled = true;
    t.masters = vec![Masters::new([
        combat(4, 6, 10),
        locked,
        combat(0x26d, 3, 10),
    ])];
    t.do_effect(
        0,
        &[attribute(2, 4, 1, -1, 0.0), attribute(2, 5, 1, -1, 0.0)],
        None,
    )
    .unwrap();
    let available: Vec<bool> = t.masters[0].iter().map(|m| m.available).collect();
    assert_eq!(available, [true, false, false]);
    t.settings.game_mode = 11;
    t.do_effect(0, &[attribute(2, 0x26d, 1, -1, 0.0)], None)
        .unwrap();
    assert!(!t.masters[0].get(0x26d).unwrap().available);
    t.settings.game_mode = 0;
    assert!(
        t.do_effect(0, &[attribute(2, 0x26d, 1, -1, 0.0)], None)
            .is_err()
    );
    t.do_effect(0, &[attribute(2, 4, 0, -1, 0.0)], None)
        .unwrap();
    assert!(!t.masters[0].get(4).unwrap().available);
}

/// A research whose effect reaches its building (the object `FUN_1413cdcd0` passes, which its
/// callers do not yet) stops before it changes anything; one with an effect override too.
#[test]
fn a_research_whose_effect_needs_its_building_stops() {
    use crate::tech::{TechDef, TechState};
    let state = TechState {
        state: 2,
        building_master: -1,
        ..TechState::default()
    };
    let player = Player {
        techs: vec![state.clone(), state],
        attributes: vec![0.0; 300],
        ..Player::default()
    };
    let mut t = Turns::new(0, 0, 0, vec![0], vec![player]);
    let def = |effect| TechDef {
        required: [-1; 6],
        effect_id: 7,
        effect,
        ..TechDef::default()
    };
    t.techs = vec![
        def(vec![attribute(204, 82, -1, 0, 1500.0)]),
        def(vec![attribute(4, -1, 4, 0, 15.0)]),
    ];
    assert!(t.complete_research(0, 0, None).is_err());
    assert_eq!(t.players[0].techs[0].state, 2);
    t.players[0].techs[1].effect_override = 9;
    assert!(t.complete_research(0, 1, None).is_err());
    t.players[0].techs[1].effect_override = 7;
    assert_eq!(t.complete_research(0, 1, None), Ok(true));
}

/// An age (101 to 103) completes as any research and adds its event (0 to 2) to the
/// player's history at the world's time (`FUN_1413cdcd0`'s end); another research adds none.
#[test]
fn an_age_adds_its_event_to_the_history() {
    use crate::tech::{TechDef, TechState};
    let state = TechState {
        state: 2,
        building_master: -1,
        ..TechState::default()
    };
    let player = Player {
        techs: vec![state; 104],
        attributes: vec![0.0; 300],
        ..Player::default()
    };
    let mut t = Turns::new(0, 0, 0, vec![0], vec![player]);
    t.techs = vec![
        TechDef {
            required: [-1; 6],
            ..TechDef::default()
        };
        104
    ];
    t.world_time = 612_345;
    t.players[0].history.entries = vec![[3, 0]; 20];
    assert_eq!(t.complete_research(0, 100, None), Ok(true));
    assert_eq!(t.players[0].history.events, []);
    assert_eq!(t.complete_research(0, 102, None), Ok(true));
    assert_eq!(t.players[0].techs[102].state, 3);
    assert_eq!(
        t.players[0].history.events,
        [crate::history::Event {
            kind: crate::history::CASTLE_AGE,
            time_slice: 20,
            world_time: 612_345,
            ..crate::history::Event::default()
        }]
    );
}

/// A technology whose requirement is researched and whose research takes no time at a
/// location (`FUN_1413cce70`'s call of `FUN_1413cdcd0`) is done at once, and so are those
/// that depend on it in turn; one still missing a requirement stays unavailable.
#[test]
fn a_dependent_technology_done_already_completes_with_its_requirement() {
    use crate::tech::{Location, TechDef, TechState};
    let def = |required: i16, count| TechDef {
        required: [required, -1, -1, -1, -1, -1],
        required_count: count,
        ..TechDef::default()
    };
    let free = TechState {
        locations: vec![Location {
            building: 109,
            ..Location::default()
        }],
        building_master: -1,
        ..TechState::default()
    };
    let player = Player {
        techs: vec![TechState::default(), free.clone(), free.clone(), free],
        ..Player::default()
    };
    let mut t = Turns::new(0, 0, 0, vec![0], vec![player]);
    t.techs = vec![def(-1, 0), def(0, 1), def(1, 1), def(0, 2)];
    assert_eq!(t.complete_research(0, 0, None), Ok(true));
    let states: Vec<i16> = t.players[0].techs.iter().map(|s| s.state).collect();
    assert_eq!(states, [3, 3, 3, 0]);
}

/// `FUN_1413cce70`'s civilisation test: a dependent of another civilisation turns −1; one of
/// its own of mode 0 does too under the full tech tree (`+0x63`), not one of mode 1; one for
/// every civilisation (civ 0) is available either way.
#[test]
fn the_full_tech_tree_disables_a_civ_s_own_technologies_but_those_of_mode_1() {
    use crate::tech::{TechDef, TechState};
    let def = |required: i16, civ: i16, mode: i16| TechDef {
        required: [required, -1, -1, -1, -1, -1],
        required_count: i16::from(required != -1),
        civ,
        mode,
        ..TechDef::default()
    };
    let states = |full_tech_tree: bool| {
        let player = Player {
            civ: 5,
            techs: vec![TechState::default(); 5],
            ..Player::default()
        };
        let mut t = Turns::new(0, 0, 0, vec![0], vec![player]);
        t.settings.full_tech_tree = full_tech_tree;
        t.techs = vec![
            def(-1, 0, 0),
            def(0, 5, 0),
            def(0, 5, 1),
            def(0, 7, 0),
            def(0, 0, 0),
        ];
        assert_eq!(t.complete_research(0, 0, None), Ok(true));
        t.players[0]
            .techs
            .iter()
            .map(|s| s.state)
            .collect::<Vec<i16>>()
    };
    assert_eq!(states(false), [3, 1, 1, -1, 1]);
    assert_eq!(states(true), [3, -1, 1, -1, 1]);
}

/// The technology commands of an effect: the disable command (`'f'`, MarvinBlue/426048737's
/// civ effect disabling technology 238 in update 1) sets a technology to −1 but a researched
/// one, one of type 0x21 to 1, and leaves one of the shared kind (a building's own); the cost
/// modifier (`'e'`) adds to, sets or scales a paid cost entry's adjustment.
#[test]
fn the_technology_commands_disable_and_adjust_costs() {
    use crate::tech::{Cost, TechDef, TechState};
    let state = |s: i16| TechState {
        state: s,
        ..TechState::default()
    };
    let player = Player {
        techs: vec![state(1), state(3), state(0), state(1)],
        ..Player::default()
    };
    let mut t = Turns::new(0, 0, 0, vec![0], vec![player]);
    let kind = |k: i16| TechDef {
        kind: k,
        costs: [
            Cost {
                attribute: 0,
                amount: 150,
                paid: 1,
            },
            Cost {
                attribute: 3,
                amount: 50,
                paid: 1,
            },
            Cost::default(),
        ],
        ..TechDef::default()
    };
    t.techs = vec![kind(0), kind(0), kind(0x21), kind(crate::tech::SHARED_KIND)];
    let command = |kind: u8, a: i16, b: i16, c: i16, d: f32| EffectCommand { kind, a, b, c, d };
    for tech in 0..4 {
        t.do_effect(0, &[command(b'f', -1, -1, -1, tech as f32)], None)
            .unwrap();
    }
    let states: Vec<i16> = t.players[0].techs.iter().map(|s| s.state).collect();
    assert_eq!(states, [-1, 3, 1, 1]);
    // gold of technology 0 costs 10 more; food set to 100; every paid cost scaled by 0.5
    t.do_effect(0, &[command(b'e', 0, 3, 1, 10.0)], None)
        .unwrap();
    assert_eq!(t.players[0].techs[0].cost_adj, [0, 10, 0]);
    t.do_effect(0, &[command(b'e', 0, 0, 0, 100.0)], None)
        .unwrap();
    assert_eq!(t.players[0].techs[0].cost_adj, [-50, 10, 0]);
    t.do_effect(0, &[command(b'e', 1, -1, 2, 0.5)], None)
        .unwrap();
    assert_eq!(t.players[0].techs[1].cost_adj, [-75, -25, 0]);
    // the research time modifier changes a location's adjustment; a technology without
    // locations has none to change
    t.do_effect(0, &[command(b'g', 0, -1, 1, 5.0)], None)
        .unwrap();
}

/// `FUN_1413cdb00` reads the id as a short (`MOVZX EDX, BX` in `FUN_14163a500`): 65538 is
/// technology 2, 65535 is −1, every technology.
#[test]
fn the_disable_command_reads_its_id_s_low_16_bits() {
    use crate::tech::{TechDef, TechState};
    let state = |s: i16| TechState {
        state: s,
        ..TechState::default()
    };
    let player = Player {
        techs: vec![state(0), state(0), state(0)],
        ..Player::default()
    };
    let mut t = Turns::new(0, 0, 0, vec![0], vec![player]);
    t.techs = vec![TechDef::default(); 3];
    let disable = |d: f32| EffectCommand {
        kind: b'f',
        a: -1,
        b: -1,
        c: -1,
        d,
    };
    t.do_effect(0, &[disable(65538.0)], None).unwrap();
    let states: Vec<i16> = t.players[0].techs.iter().map(|s| s.state).collect();
    assert_eq!(states, [0, 0, -1]);
    t.do_effect(0, &[disable(65535.0)], None).unwrap();
    let states: Vec<i16> = t.players[0].techs.iter().map(|s| s.state).collect();
    assert_eq!(states, [-1, -1, -1]);
}

/// `FUN_1413d10f0` converts with `CVTTSS2SI` and adds the low word: an amount of 2^31 or
/// more adds 0 (0x80000000's low word), where a saturating cast would add −1.
#[test]
fn a_cost_change_past_the_i32_range_adds_nothing() {
    use crate::tech::{Cost, TechDef, TechState};
    let player = Player {
        techs: vec![TechState::default()],
        ..Player::default()
    };
    let mut t = Turns::new(0, 0, 0, vec![0], vec![player]);
    t.techs = vec![TechDef {
        costs: [
            Cost {
                attribute: 0,
                amount: 150,
                paid: 1,
            },
            Cost::default(),
            Cost::default(),
        ],
        ..TechDef::default()
    }];
    let change = EffectCommand {
        kind: b'e',
        a: 0,
        b: 0,
        c: 1,
        d: 3.0e9,
    };
    t.do_effect(0, &[change], None).unwrap();
    assert_eq!(t.players[0].techs[0].cost_adj, [0, 0, 0]);
    // mode 2 at 1.5: (1.5 − 1) × 150 + 0.5 = 75.5, truncated to 75
    let scale = EffectCommand {
        c: 2,
        d: 1.5,
        ..change
    };
    t.do_effect(0, &[scale], None).unwrap();
    assert_eq!(t.players[0].techs[0].cost_adj, [75, 0, 0]);
}

/// A dead building's master technology goes back to 0 (`FUN_1413d4ec0`) when no other
/// building of the master stands, and only from a state above 0.
#[test]
fn the_last_building_of_a_master_withdraws_its_technology() {
    use crate::tech::{TechDef, TechState};
    let building = |id, state| {
        let mut u = Unit {
            id,
            ..Unit::default()
        };
        u.life.master_id = 1640;
        u.life.state = state;
        u
    };
    let player = Player {
        objects: vec![building(1, 3), building(2, 2)],
        techs: vec![TechState {
            state: 3,
            ..TechState::default()
        }],
        ..Player::default()
    };
    let mut t = Turns::new(0, 0, 0, vec![0], vec![player]);
    t.techs = vec![TechDef {
        required: [-1; 6],
        ..TechDef::default()
    }];
    t.withdraw_tech(0, 1640, 0).unwrap();
    assert_eq!(t.players[0].techs[0].state, 3);
    t.players[0].objects.truncate(1);
    t.withdraw_tech(0, 1640, 0).unwrap();
    assert_eq!(t.players[0].techs[0].state, 0);
}
